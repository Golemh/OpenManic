//! Primary process composition for the first Windows tracking vertical slice.
//!
//! The composition root owns process lifetime and wires accepted boundaries together. SQLite,
//! tracking reduction, projection work, and Windows callbacks never execute in an egui frame.

use std::{
    collections::{BTreeMap, VecDeque},
    error::Error,
    fmt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use openmanic_application::{
    AppEvent, ApplicationError, ApplicationPort, CommandEnvelope, CommandId, CommandReceipt,
    DataRevision, EventEnvelope, LaneCapacities, LaneReceive, LaneSubmit, LatestMailbox,
    LatestMailboxReceiver, MailboxReceive, OrderingKey, PortFailureReason, ProjectionContextKey,
    ProjectionRequest, ProjectionSlot, RuntimeLaneReceiver, RuntimeLanes, RuntimeSupervisor,
    RuntimeWorker, SchemaRevision, ShutdownCoordinator, ShutdownPhase, ShutdownStep,
    SnapshotEnvelope, ThreadRoot, TimelineApplication, TimelineContext, TimelineProjector,
    TimelineRawIntervalId, TimelineSnapshot, TimelineSourceActivity, TrackingCommand,
    TrackingEvidence, TrackingPersistenceIntent, TrackingPersistencePort,
    TrackingPersistenceSubmit, TrackingService, WorkLane, bounded_runtime_lanes, latest_mailbox,
};
use openmanic_domain::{
    ActivityState, Application, ApplicationId, ApplicationName, HalfOpenInterval, TrackerRunId,
    UtcMicros,
};
#[cfg(windows)]
use openmanic_platform::{
    ActivationCommandDecode, InstanceAcquisition, LocalActivationCommand, WindowsActivationServer,
    WindowsControlAdapter, WindowsInstanceOwner,
};
use openmanic_platform::{
    EvidencePublishResult, TrackingEvidenceSink, WindowsPlatformAction, WindowsTrayController,
};
use openmanic_storage_sqlite::{
    RecoveryOutcome, SqliteStore, StoreOpenOptions, TrackerRunRegistration,
};
use openmanic_ui_egui::timeline::{TimelineRenderAction, TimelineRenderer};
use openmanic_ui_egui::{
    ApplicationUsage, ApplicationUsageSnapshot, CommandDispatcher, InboundMessage, MutationStatus,
    OpenManicApp, TodayController, TodayTrackingRequest, TrackingControlAction, UiAction,
    UiController, UiModel, render_distribution_snapshot, render_usage_snapshot,
};

use crate::bootstrap::{BootstrapDisposition, BootstrapError, BootstrapState, bootstrap};
use crate::cli::{CliError, parse_process_cli};
use crate::data_root::{LocalDataRootValidator, RejectKnownNetworkShares};

const UI_INBOUND_CAPACITY: usize = 64;
const UI_OUTBOUND_CAPACITY: usize = 32;
const UI_EVENT_CAPACITY: usize = 64;
const WRITER_CRITICAL_CAPACITY: usize = 64;
const WRITER_NORMAL_CAPACITY: usize = 64;
const WRITER_OPTIONAL_CAPACITY: usize = 16;
const UI_POLL_INTERVAL: Duration = Duration::from_millis(250);
const WORKER_IDLE_INTERVAL: Duration = Duration::from_millis(10);
const PLATFORM_PUMP_INTERVAL: Duration = Duration::from_millis(50);

/// Startup failures reported without exposing paths, titles, or raw platform errors.
#[derive(Debug)]
pub enum CompositionError {
    /// Bootstrap input was malformed.
    Cli(CliError),
    /// Data-root bootstrap did not finish successfully.
    Bootstrap(BootstrapError),
    /// A user directory choice is required before a store may open.
    DirectoryChoiceRequired,
    /// The local SQLite store could not open safely.
    Storage,
    /// A bounded queue, worker, or UI controller could not be configured.
    Runtime,
    /// Windows instance coordination could not start safely.
    Instance,
    /// The native Windows platform worker could not start.
    Platform,
    /// The native UI host could not start or complete normally.
    NativeUi,
}

impl CompositionError {
    /// Returns a privacy-safe startup summary suitable for the initial process boundary.
    #[must_use]
    pub const fn safe_summary(&self) -> &'static str {
        match self {
            Self::Cli(error) => error.safe_summary(),
            Self::Bootstrap(error) => error.code(),
            Self::DirectoryChoiceRequired => "Choose a local writable data directory to continue.",
            Self::Storage => "The local activity store could not be opened safely.",
            Self::Runtime => "The local activity runtime could not be started safely.",
            Self::Instance => "OpenManic could not coordinate the current Windows session.",
            Self::Platform => "Windows tracking controls could not be started safely.",
            Self::NativeUi => "The application window could not be started.",
        }
    }
}

impl fmt::Display for CompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.safe_summary())
    }
}

impl Error for CompositionError {}

/// The one correlated immutable value delivered to the composed Today screen.
#[derive(Clone, Debug)]
struct TodaySnapshot {
    timeline: TimelineSnapshot,
    usage: ApplicationUsageSnapshot,
    distribution: openmanic_ui_egui::DistributionSnapshot,
}

impl TodaySnapshot {
    const fn timeline(&self) -> &TimelineSnapshot {
        &self.timeline
    }

    const fn usage(&self) -> &ApplicationUsageSnapshot {
        &self.usage
    }

    const fn distribution(&self) -> &openmanic_ui_egui::DistributionSnapshot {
        &self.distribution
    }
}

/// A writer-lane item. UI commands are distinguished only so their authoritative outcomes can be
/// returned to the UI; platform evidence follows the same reducer and persistence path.
#[derive(Debug)]
enum WriterWork {
    CatalogForeground {
        application: Application,
        command: CommandEnvelope<TrackingCommand>,
    },
    System(CommandEnvelope<TrackingCommand>),
    Ui(CommandEnvelope<TrackingCommand>),
}

impl WriterWork {
    fn into_parts(self) -> (CommandEnvelope<TrackingCommand>, bool) {
        match self {
            Self::System(command) | Self::CatalogForeground { command, .. } => (command, false),
            Self::Ui(command) => (command, true),
        }
    }
}

/// Exclusive-worker control which is deliberately separate from ordinary ingress.
enum WriterControl {
    Checkpoint(SyncSender<bool>),
    Close(SyncSender<bool>),
}

/// Bounded UI-side work which must retain authoritative acknowledgements in arrival order.
#[derive(Debug)]
enum UiIngress {
    Pending(CommandId),
    Event(EventEnvelope<AppEvent>),
}

#[derive(Debug)]
struct UiInbox {
    queue: Mutex<VecDeque<UiIngress>>,
    capacity: usize,
}

impl UiInbox {
    fn new(capacity: usize) -> Self {
        Self {
            queue: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    fn try_push(&self, message: UiIngress) -> bool {
        let Ok(mut queue) = self.queue.lock() else {
            return false;
        };
        if queue.len() == self.capacity {
            return false;
        }
        queue.push_back(message);
        true
    }

    fn remove_pending(&self, command_id: CommandId) {
        let Ok(mut queue) = self.queue.lock() else {
            return;
        };
        let Some(index) = queue.iter().position(
            |message| matches!(message, UiIngress::Pending(candidate) if *candidate == command_id),
        ) else {
            return;
        };
        let _ = queue.remove(index);
    }

    fn drain_into(
        &self,
        controller: &mut UiController<TrackingCommand, TodaySnapshot>,
        limit: usize,
    ) {
        for _ in 0..limit {
            if !self.drain_one(controller) {
                break;
            }
        }
    }

    fn drain_one(&self, controller: &mut UiController<TrackingCommand, TodaySnapshot>) -> bool {
        let Ok(mut queue) = self.queue.lock() else {
            return false;
        };
        let Some(message) = queue.pop_front() else {
            return false;
        };
        if let UiIngress::Pending(command_id) = message {
            controller.record_external_pending(command_id);
            return true;
        }
        let UiIngress::Event(event) = message else {
            return false;
        };
        if controller.inbound_len() >= UI_INBOUND_CAPACITY {
            queue.push_front(UiIngress::Event(event));
            return false;
        }
        let _ = controller.try_enqueue_inbound(InboundMessage::Event(event));
        true
    }
}

/// A nonblocking sink from Windows normalization into the writer's critical ingress.
#[derive(Clone)]
struct RuntimeEvidenceSink {
    lanes: Arc<RuntimeLanes<WriterWork>>,
    identifiers: Arc<CommandIdentifiers>,
    accepting: Arc<AtomicBool>,
}

impl TrackingEvidenceSink for RuntimeEvidenceSink {
    fn try_publish(&self, evidence: TrackingEvidence) -> EvidencePublishResult {
        if !self.accepting.load(Ordering::Acquire) {
            return EvidencePublishResult::Closed;
        }
        let command = self
            .identifiers
            .command(TrackingCommand::Evidence(evidence));
        let work = match evidence {
            TrackingEvidence::Foreground {
                application_id,
                observed_at_utc,
                ..
            } => {
                let Ok(name) = ApplicationName::try_new("Tracked application") else {
                    return EvidencePublishResult::Closed;
                };
                let Ok(application) = Application::try_new(
                    application_id,
                    name,
                    None,
                    observed_at_utc,
                    observed_at_utc,
                ) else {
                    return EvidencePublishResult::Closed;
                };
                WriterWork::CatalogForeground {
                    application,
                    command,
                }
            }
            _ => WriterWork::System(command),
        };
        match self.lanes.try_submit(WorkLane::Critical, work) {
            LaneSubmit::Enqueued => EvidencePublishResult::Accepted,
            LaneSubmit::Retained { reason, .. } => match reason {
                openmanic_application::LaneRetentionReason::Full => EvidencePublishResult::Full,
                openmanic_application::LaneRetentionReason::Closed => EvidencePublishResult::Closed,
            },
            LaneSubmit::Dropped { .. } => EvidencePublishResult::Full,
        }
    }
}

/// Stable identifiers allocated by the process boundary for local commands and evidence.
#[derive(Debug, Default)]
struct CommandIdentifiers {
    next: AtomicU64,
}

impl CommandIdentifiers {
    fn command(&self, payload: TrackingCommand) -> CommandEnvelope<TrackingCommand> {
        let (command_id, ordering_key, submitted_at_utc) = self.next_metadata();
        CommandEnvelope::new(
            SchemaRevision::new(1),
            command_id,
            ordering_key,
            None,
            submitted_at_utc,
            payload,
        )
    }

    fn tracking_request(&self, action: TrackingControlAction) -> TodayTrackingRequest {
        let (command_id, ordering_key, submitted_at_utc) = self.next_metadata();
        TodayTrackingRequest::new(action, command_id, ordering_key, submitted_at_utc)
    }

    fn next_metadata(&self) -> (CommandId, OrderingKey, UtcMicros) {
        let value = self.next.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        (
            CommandId::new(value),
            OrderingKey::new(value),
            UtcMicros::new(utc_now_micros()),
        )
    }
}

/// Routes retained platform actions only after their native callback has returned.
#[derive(Clone)]
struct PlatformActionRouter {
    lanes: Arc<RuntimeLanes<WriterWork>>,
    ui_inbox: Arc<UiInbox>,
    identifiers: Arc<CommandIdentifiers>,
    accepting: Arc<AtomicBool>,
    restore_requested: Arc<AtomicBool>,
    quit_requested: Arc<AtomicBool>,
}

impl PlatformActionRouter {
    fn route(&self, action: WindowsPlatformAction) {
        match action {
            WindowsPlatformAction::Open => {
                self.restore_requested.store(true, Ordering::Release);
            }
            WindowsPlatformAction::PauseTracking => self.route_tracking(TrackingCommand::Pause),
            WindowsPlatformAction::ResumeTracking => self.route_tracking(TrackingCommand::Resume),
            WindowsPlatformAction::Quit => {
                self.quit_requested.store(true, Ordering::Release);
            }
            // Focus is intentionally unavailable until its own accepted service exists. The tray
            // action remains bounded and cannot fabricate a focus mutation in this vertical slice.
            WindowsPlatformAction::StartFocusSession => {}
        }
    }

    fn route_tracking(&self, payload: TrackingCommand) {
        if !self.accepting.load(Ordering::Acquire) {
            return;
        }
        let command = self.identifiers.command(payload);
        let command_id = command.command_id();
        if !self.ui_inbox.try_push(UiIngress::Pending(command_id)) {
            return;
        }
        if !matches!(
            self.lanes
                .try_submit(WorkLane::Critical, WriterWork::Ui(command)),
            LaneSubmit::Enqueued
        ) {
            self.ui_inbox.remove_pending(command_id);
        }
    }
}

/// The UI's nonblocking outbound dispatcher into the named writer worker.
struct RuntimeCommandDispatcher {
    lanes: Arc<RuntimeLanes<WriterWork>>,
    accepting: Arc<AtomicBool>,
}

impl CommandDispatcher<TrackingCommand> for RuntimeCommandDispatcher {
    fn try_dispatch(
        &mut self,
        command: CommandEnvelope<TrackingCommand>,
    ) -> Result<CommandReceipt, ApplicationError> {
        if !self.accepting.load(Ordering::Acquire) {
            return Err(ApplicationError::port_failure(
                ApplicationPort::Command,
                PortFailureReason::Unavailable,
            ));
        }
        let command_id = command.command_id();
        match self
            .lanes
            .try_submit(WorkLane::Critical, WriterWork::Ui(command))
        {
            LaneSubmit::Enqueued => Ok(CommandReceipt::accepted(command_id)),
            LaneSubmit::Retained { reason, .. } => Err(ApplicationError::port_failure(
                ApplicationPort::Command,
                match reason {
                    openmanic_application::LaneRetentionReason::Full => PortFailureReason::Busy,
                    openmanic_application::LaneRetentionReason::Closed => {
                        PortFailureReason::Unavailable
                    }
                },
            )),
            LaneSubmit::Dropped { .. } => Err(ApplicationError::port_failure(
                ApplicationPort::Command,
                PortFailureReason::Busy,
            )),
        }
    }
}

/// The only persistence adapter used by the tracking service on the writer thread.
struct WriterPersistence {
    store: Arc<Mutex<SqliteStore>>,
    first_write: bool,
}

impl TrackingPersistencePort for WriterPersistence {
    fn try_persist(&mut self, intent: TrackingPersistenceIntent) -> TrackingPersistenceSubmit {
        let Ok(mut store) = self.store.lock() else {
            return persistence_failure(PortFailureReason::Unavailable);
        };
        if !self.first_write {
            return store.writer().try_persist(intent);
        }

        let checkpoint = intent.checkpoint();
        let Ok(registration) = TrackerRunRegistration::try_new(
            checkpoint.tracker_run_id(),
            checkpoint.open_start_utc(),
            "windows-control-v1",
        ) else {
            return persistence_failure(PortFailureReason::Failed);
        };
        match store.writer().recover_unclean_exit(&intent, &registration) {
            Ok(RecoveryOutcome::Recovered { revision, .. }) => {
                self.first_write = false;
                TrackingPersistenceSubmit::Committed(revision)
            }
            Ok(RecoveryOutcome::NoCheckpoint) => {
                let result = register_tracker_run_and_persist(&mut store, &registration, intent);
                self.first_write = !matches!(result, TrackingPersistenceSubmit::Committed(_));
                result
            }
            Err(_) => persistence_failure(PortFailureReason::Failed),
        }
    }
}

fn register_tracker_run_and_persist(
    store: &mut SqliteStore,
    registration: &TrackerRunRegistration,
    intent: TrackingPersistenceIntent,
) -> TrackingPersistenceSubmit {
    match store.writer().register_tracker_run(registration) {
        Ok(_) => store.writer().try_persist(intent),
        Err(_) => persistence_failure(PortFailureReason::Failed),
    }
}

fn persistence_failure(reason: PortFailureReason) -> TrackingPersistenceSubmit {
    TrackingPersistenceSubmit::Failed(ApplicationError::port_failure(
        ApplicationPort::Command,
        reason,
    ))
}

/// All process-owned workers and their bounded ingress boundaries.
struct RuntimeResources {
    lanes: Arc<RuntimeLanes<WriterWork>>,
    projection_requests: LatestMailbox<ProjectionRequest<TimelineContext>>,
    ui_inbox: Arc<UiInbox>,
    writer_control: SyncSender<WriterControl>,
    writer_handle: Option<JoinHandle<()>>,
    accepting: Arc<AtomicBool>,
    identifiers: Arc<CommandIdentifiers>,
    restore_requested: Arc<AtomicBool>,
    quit_requested: Arc<AtomicBool>,
    supervisor: RuntimeSupervisor,
    #[cfg(windows)]
    platform_stop: Option<mpsc::Sender<()>>,
    #[cfg(windows)]
    platform_handle: Option<JoinHandle<()>>,
    #[cfg(windows)]
    activation_stop: Arc<AtomicBool>,
    #[cfg(windows)]
    activation_handle: Option<JoinHandle<()>>,
}

impl RuntimeResources {
    fn start(
        store: SqliteStore,
    ) -> Result<(Self, LatestMailboxReceiver<SnapshotEnvelope<TodaySnapshot>>), CompositionError>
    {
        let capacities = LaneCapacities::try_new(
            WRITER_CRITICAL_CAPACITY,
            WRITER_NORMAL_CAPACITY,
            WRITER_OPTIONAL_CAPACITY,
        )
        .map_err(|_| CompositionError::Runtime)?;
        let (lanes, lane_receiver) = bounded_runtime_lanes(capacities);
        let lanes = Arc::new(lanes);
        let (projection_requests, projection_receiver) = latest_mailbox();
        let (snapshots, snapshot_receiver) = latest_mailbox();
        let (writer_control, control_receiver) = mpsc::sync_channel(2);
        let ui_inbox = Arc::new(UiInbox::new(UI_EVENT_CAPACITY));
        let accepting = Arc::new(AtomicBool::new(true));
        let identifiers = Arc::new(CommandIdentifiers::default());
        let worker_ui_inbox = Arc::clone(&ui_inbox);
        let writer_handle = thread::Builder::new()
            .name(ThreadRoot::new(RuntimeWorker::Writer).name().to_owned())
            .spawn(move || {
                run_writer_worker(
                    store,
                    lane_receiver,
                    control_receiver,
                    projection_receiver,
                    snapshots,
                    worker_ui_inbox,
                );
            })
            .map_err(|_| CompositionError::Runtime)?;

        Ok((
            Self {
                lanes,
                projection_requests,
                ui_inbox,
                writer_control,
                writer_handle: Some(writer_handle),
                accepting,
                identifiers,
                restore_requested: Arc::new(AtomicBool::new(false)),
                quit_requested: Arc::new(AtomicBool::new(false)),
                supervisor: RuntimeSupervisor::new([
                    ThreadRoot::new(RuntimeWorker::Supervisor),
                    ThreadRoot::new(RuntimeWorker::Writer),
                    ThreadRoot::new(RuntimeWorker::ProjectionReader),
                    ThreadRoot::new(RuntimeWorker::BulkWorker),
                    ThreadRoot::new(RuntimeWorker::PlatformObservation),
                ]),
                #[cfg(windows)]
                platform_stop: None,
                #[cfg(windows)]
                platform_handle: None,
                #[cfg(windows)]
                activation_stop: Arc::new(AtomicBool::new(false)),
                #[cfg(windows)]
                activation_handle: None,
            },
            snapshot_receiver,
        ))
    }

    fn evidence_sink(&self) -> RuntimeEvidenceSink {
        RuntimeEvidenceSink {
            lanes: self.lanes.clone(),
            identifiers: Arc::clone(&self.identifiers),
            accepting: Arc::clone(&self.accepting),
        }
    }

    fn action_router(&self) -> PlatformActionRouter {
        PlatformActionRouter {
            lanes: self.lanes.clone(),
            ui_inbox: Arc::clone(&self.ui_inbox),
            identifiers: Arc::clone(&self.identifiers),
            accepting: Arc::clone(&self.accepting),
            restore_requested: Arc::clone(&self.restore_requested),
            quit_requested: Arc::clone(&self.quit_requested),
        }
    }

    fn reject_nonessential_work(&self) {
        self.accepting.store(false, Ordering::Release);
    }

    fn checkpoint_writer(&self) -> bool {
        let (reply, receive) = mpsc::sync_channel(1);
        match self
            .writer_control
            .try_send(WriterControl::Checkpoint(reply))
        {
            Ok(()) => receive
                .recv_timeout(Duration::from_secs(5))
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    fn close_writer(&mut self) -> bool {
        let (reply, receive) = mpsc::sync_channel(1);
        if self
            .writer_control
            .try_send(WriterControl::Close(reply))
            .is_err()
            || !receive
                .recv_timeout(Duration::from_secs(5))
                .unwrap_or(false)
        {
            return false;
        }
        self.writer_handle
            .take()
            .is_none_or(|handle| handle.join().is_ok())
    }

    #[cfg(windows)]
    fn start_windows(&mut self, owner: &WindowsInstanceOwner) -> Result<(), CompositionError> {
        let activation_server = owner
            .activation_server()
            .map_err(|_| CompositionError::Instance)?;
        let activation_handle = spawn_activation_listener(
            activation_server,
            self.action_router(),
            Arc::clone(&self.activation_stop),
        )?;
        let (stop_sender, platform_handle, ready_receiver) =
            spawn_platform_worker(self.action_router(), self.evidence_sink())?;
        if let Ok(Ok(())) = ready_receiver.recv_timeout(Duration::from_secs(5)) {
            self.activation_handle = Some(activation_handle);
            self.platform_stop = Some(stop_sender);
            self.platform_handle = Some(platform_handle);
            return Ok(());
        }

        self.activation_stop.store(true, Ordering::Release);
        let _ = stop_sender.send(());
        let _ = activation_handle.join();
        let _ = platform_handle.join();
        Err(CompositionError::Platform)
    }

    #[cfg(windows)]
    fn stop_windows(&mut self, owner: &WindowsInstanceOwner) -> bool {
        self.activation_stop.store(true, Ordering::Release);
        let _ = owner.wake_activation_listener();
        if let Some(stop) = self.platform_stop.take() {
            let _ = stop.send(());
        }
        let platform_joined = self
            .platform_handle
            .take()
            .is_none_or(|handle| handle.join().is_ok());
        let activation_joined = self
            .activation_handle
            .take()
            .is_none_or(|handle| handle.join().is_ok());
        platform_joined && activation_joined
    }
}

#[cfg(windows)]
fn spawn_activation_listener(
    server: WindowsActivationServer,
    action_router: PlatformActionRouter,
    stop: Arc<AtomicBool>,
) -> Result<JoinHandle<()>, CompositionError> {
    thread::Builder::new()
        .name("openmanic-activation".to_owned())
        .spawn(move || run_activation_listener(server, action_router, stop))
        .map_err(|_| CompositionError::Runtime)
}

#[cfg(windows)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the listener thread owns the router and stop signal for its complete lifetime"
)]
fn run_activation_listener(
    mut server: WindowsActivationServer,
    action_router: PlatformActionRouter,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Acquire) {
        let Ok(decoded) = server.receive_next() else {
            break;
        };
        let Some(action) = activation_action(decoded) else {
            continue;
        };
        if stop.load(Ordering::Acquire) {
            break;
        }
        action_router.route(action);
    }
}

#[cfg(windows)]
const fn activation_action(decoded: ActivationCommandDecode) -> Option<WindowsPlatformAction> {
    match decoded {
        ActivationCommandDecode::Known(LocalActivationCommand::Activate) => {
            Some(WindowsPlatformAction::Open)
        }
        ActivationCommandDecode::Known(LocalActivationCommand::PauseTracking) => {
            Some(WindowsPlatformAction::PauseTracking)
        }
        ActivationCommandDecode::Known(LocalActivationCommand::ResumeTracking) => {
            Some(WindowsPlatformAction::ResumeTracking)
        }
        ActivationCommandDecode::IgnoredUnknown | ActivationCommandDecode::Rejected => None,
    }
}

#[cfg(windows)]
type PlatformWorkerStart = (mpsc::Sender<()>, JoinHandle<()>, Receiver<Result<(), ()>>);

#[cfg(windows)]
fn spawn_platform_worker(
    action_router: PlatformActionRouter,
    evidence_sink: RuntimeEvidenceSink,
) -> Result<PlatformWorkerStart, CompositionError> {
    let (stop_sender, stop_receiver) = mpsc::channel();
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    let handle = thread::Builder::new()
        .name(
            ThreadRoot::new(RuntimeWorker::PlatformObservation)
                .name()
                .to_owned(),
        )
        .spawn(move || {
            if run_platform_worker(action_router, evidence_sink, stop_receiver, &ready_sender)
                .is_err()
            {
                let _ = ready_sender.send(Err(()));
            }
        })
        .map_err(|_| CompositionError::Runtime)?;
    Ok((stop_sender, handle, ready_receiver))
}

#[cfg(windows)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the platform thread owns its router, evidence sink, and stop receiver until it exits"
)]
fn run_platform_worker(
    action_router: PlatformActionRouter,
    evidence_sink: RuntimeEvidenceSink,
    stop_receiver: Receiver<()>,
    ready_sender: &SyncSender<Result<(), ()>>,
) -> Result<(), openmanic_platform::WindowsControlError> {
    let mut adapter = WindowsControlAdapter::new();
    let mut control = adapter.install_control_window()?;
    let mut tray = control.install_tray()?;
    let _ = ready_sender.send(Ok(()));
    loop {
        control.pump_available_with_tray(&mut adapter, &evidence_sink, &mut tray)?;
        while let Some(action) = tray.take_next_action() {
            action_router.route(action);
        }
        if matches!(
            stop_receiver.recv_timeout(PLATFORM_PUMP_INTERVAL),
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected)
        ) {
            return Ok(());
        }
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the named worker exclusively owns all receivers and publishers for its lifetime"
)]
fn run_writer_worker(
    store: SqliteStore,
    lanes: RuntimeLaneReceiver<WriterWork>,
    control: Receiver<WriterControl>,
    projection_requests: LatestMailboxReceiver<ProjectionRequest<TimelineContext>>,
    snapshots: LatestMailbox<SnapshotEnvelope<TodaySnapshot>>,
    ui_inbox: Arc<UiInbox>,
) {
    let store = Arc::new(Mutex::new(store));
    let run_id = tracker_run_id();
    let persistence = WriterPersistence {
        store: Arc::clone(&store),
        first_write: true,
    };
    let mut tracking = TrackingService::new(run_id, persistence);
    let mut current_projection = None;
    let mut running = true;

    while running {
        while let MailboxReceive::Latest(request) = projection_requests.try_receive() {
            current_projection = Some(request);
            if let Some(request) = current_projection.as_ref() {
                publish_today_snapshot(&store, request, &snapshots);
            }
        }

        match control.try_recv() {
            Ok(WriterControl::Checkpoint(reply)) => {
                let event = tracking.handle(system_checkpoint());
                let persisted = event.committed_data_revision().is_some()
                    || tracking.pending_intent().is_none();
                if persisted && let Some(request) = current_projection.as_ref() {
                    publish_today_snapshot(&store, request, &snapshots);
                }
                let _ = reply.send(persisted);
                continue;
            }
            Ok(WriterControl::Close(reply)) => {
                drain_writer_lanes(
                    &lanes,
                    &mut tracking,
                    &store,
                    &mut current_projection,
                    &snapshots,
                    &ui_inbox,
                );
                let event = tracking.handle(system_checkpoint());
                let persisted = event.committed_data_revision().is_some()
                    || tracking.pending_intent().is_none();
                let _ = reply.send(persisted);
                running = false;
                continue;
            }
            Err(TryRecvError::Disconnected) => {
                running = false;
                continue;
            }
            Err(TryRecvError::Empty) => {}
        }

        match lanes.try_receive() {
            LaneReceive::Work { work, .. } => process_writer_work(
                work,
                &mut tracking,
                &store,
                &mut current_projection,
                &snapshots,
                &ui_inbox,
            ),
            LaneReceive::Empty => thread::park_timeout(WORKER_IDLE_INTERVAL),
            LaneReceive::Closed => running = false,
        }
    }
}

fn drain_writer_lanes(
    lanes: &RuntimeLaneReceiver<WriterWork>,
    tracking: &mut TrackingService<WriterPersistence>,
    store: &Arc<Mutex<SqliteStore>>,
    current_projection: &mut Option<ProjectionRequest<TimelineContext>>,
    snapshots: &LatestMailbox<SnapshotEnvelope<TodaySnapshot>>,
    ui_inbox: &UiInbox,
) {
    while let LaneReceive::Work { work, .. } = lanes.try_receive() {
        process_writer_work(
            work,
            tracking,
            store,
            current_projection,
            snapshots,
            ui_inbox,
        );
    }
}

fn process_writer_work(
    work: WriterWork,
    tracking: &mut TrackingService<WriterPersistence>,
    store: &Arc<Mutex<SqliteStore>>,
    current_projection: &mut Option<ProjectionRequest<TimelineContext>>,
    snapshots: &LatestMailbox<SnapshotEnvelope<TodaySnapshot>>,
    ui_inbox: &UiInbox,
) {
    if let WriterWork::CatalogForeground {
        application,
        command,
    } = work
    {
        let Ok(mut writer_store) = store.lock() else {
            return;
        };
        if writer_store
            .writer()
            .upsert_application(&application)
            .is_err()
        {
            return;
        }
        let Ok(excluded) = writer_store
            .writer()
            .application_is_excluded(application.id())
        else {
            return;
        };
        drop(writer_store);
        process_tracking_command(
            foreground_command_with_exclusion(command, excluded),
            false,
            tracking,
            store,
            current_projection,
            snapshots,
            ui_inbox,
        );
        return;
    }
    let (command, from_ui) = work.into_parts();
    process_tracking_command(
        command,
        from_ui,
        tracking,
        store,
        current_projection,
        snapshots,
        ui_inbox,
    );
}

fn foreground_command_with_exclusion(
    command: CommandEnvelope<TrackingCommand>,
    excluded: bool,
) -> CommandEnvelope<TrackingCommand> {
    if !excluded {
        return command;
    }
    let TrackingCommand::Evidence(TrackingEvidence::Foreground {
        sequence,
        observed_at_utc,
        ..
    }) = *command.payload()
    else {
        return command;
    };
    CommandEnvelope::new(
        command.schema_revision(),
        command.command_id(),
        command.ordering_key(),
        command.expected_entity_revision(),
        command.submitted_at_utc(),
        TrackingCommand::Evidence(TrackingEvidence::ExcludedForeground {
            sequence,
            observed_at_utc,
        }),
    )
}

fn process_tracking_command(
    command: CommandEnvelope<TrackingCommand>,
    from_ui: bool,
    tracking: &mut TrackingService<WriterPersistence>,
    store: &Arc<Mutex<SqliteStore>>,
    current_projection: &mut Option<ProjectionRequest<TimelineContext>>,
    snapshots: &LatestMailbox<SnapshotEnvelope<TodaySnapshot>>,
    ui_inbox: &UiInbox,
) {
    let event = tracking.handle(command);
    let changed = event.committed_data_revision().is_some();
    if from_ui {
        let _ = ui_inbox.try_push(UiIngress::Event(event));
    }
    if changed && let Some(request) = current_projection.as_ref() {
        publish_today_snapshot(store, request, snapshots);
    }
}

fn system_checkpoint() -> CommandEnvelope<TrackingCommand> {
    let now = UtcMicros::new(utc_now_micros());
    CommandEnvelope::new(
        SchemaRevision::new(1),
        CommandId::new(u64::MAX),
        OrderingKey::new(u64::MAX),
        None,
        now,
        TrackingCommand::Checkpoint,
    )
}

fn publish_today_snapshot(
    store: &Arc<Mutex<SqliteStore>>,
    request: &ProjectionRequest<TimelineContext>,
    snapshots: &LatestMailbox<SnapshotEnvelope<TodaySnapshot>>,
) {
    let Ok(store) = store.lock() else {
        return;
    };
    let Ok(mut reader) = store.open_read_session() else {
        return;
    };
    drop(store);
    let Ok(read) = reader.snapshot() else {
        return;
    };
    let Ok(snapshot) = build_today_snapshot(request, &read) else {
        return;
    };
    let _ = snapshots.publish(snapshot);
}

fn build_today_snapshot(
    request: &ProjectionRequest<TimelineContext>,
    read: &openmanic_storage_sqlite::ReadSnapshot,
) -> Result<SnapshotEnvelope<TodaySnapshot>, ()> {
    let activities = read
        .activities()
        .iter()
        .map(|record| {
            TimelineSourceActivity::new(
                TimelineRawIntervalId::new(record.raw_id()),
                record.interval(),
            )
        })
        .collect::<Vec<_>>();
    let applications = read
        .applications()
        .iter()
        .map(|record| {
            TimelineApplication::new(
                record.application().id(),
                record.application().category_id(),
            )
        })
        .collect::<Vec<_>>();
    let source = openmanic_application::TimelineProjectionSource::new(
        read.revision(),
        &activities,
        &applications,
    );
    let timeline = TimelineProjector::build(*request.payload(), source).map_err(|_| ())?;
    let (usage, distribution) = build_summaries(read, request.payload().visible_range())?;
    Ok(SnapshotEnvelope::new(
        request.request_id(),
        request.slot(),
        request.context_key(),
        read.revision(),
        openmanic_application::TIMELINE_SNAPSHOT_SCHEMA_REVISION,
        TodaySnapshot {
            timeline,
            usage,
            distribution,
        },
    ))
}

#[expect(
    clippy::too_many_lines,
    reason = "one sequential pass preserves the correlation between a read snapshot and both Today summaries"
)]
fn build_summaries(
    read: &openmanic_storage_sqlite::ReadSnapshot,
    range: HalfOpenInterval,
) -> Result<
    (
        ApplicationUsageSnapshot,
        openmanic_ui_egui::DistributionSnapshot,
    ),
    (),
> {
    let application_data = read
        .applications()
        .iter()
        .map(|record| {
            (
                record.application().id(),
                (
                    record.application().name().as_str().to_owned(),
                    record.application().category_id(),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let category_names = read
        .categories()
        .iter()
        .map(|record| {
            (
                record.category().id(),
                record.category().name().as_str().to_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut application_totals = BTreeMap::<ApplicationId, u64>::new();
    let mut category_totals = BTreeMap::<String, (String, u64)>::new();

    for record in read.activities() {
        let interval = record.interval();
        if interval.state() != ActivityState::Active || !interval.range().overlaps(range) {
            continue;
        }
        let start = interval.range().start().max(range.start());
        let end = interval.range().end().min(range.end());
        let Ok(clipped) = HalfOpenInterval::try_new(start, end) else {
            continue;
        };
        let duration = clipped.duration_us();
        let Some(application_id) = interval.application_id() else {
            continue;
        };
        let total = application_totals.entry(application_id).or_default();
        *total = total.saturating_add(duration);
        let (category_key, category_label) = application_data
            .get(&application_id)
            .and_then(|(_, category_id)| {
                category_id.map(|id| (id_label(&id.as_bytes()), category_names.get(&id).cloned()))
            })
            .map_or_else(
                || ("uncategorized".to_owned(), "Uncategorized".to_owned()),
                |(key, label)| {
                    (
                        format!("category-{key}"),
                        label.unwrap_or_else(|| "Uncategorized".to_owned()),
                    )
                },
            );
        let entry = category_totals
            .entry(category_key)
            .or_insert((category_label, 0));
        entry.1 = entry.1.saturating_add(duration);
    }

    let usage = application_totals
        .into_iter()
        .map(|(application_id, duration_us)| {
            let display_name = application_data.get(&application_id).map_or_else(
                || "Unresolved application".to_owned(),
                |(name, _)| name.clone(),
            );
            ApplicationUsage::new(application_id, display_name, duration_us)
        })
        .collect();
    let contributions = category_totals.into_iter().map(|(key, (label, duration))| {
        openmanic_ui_egui::DistributionContribution::new(key, label, duration)
    });
    let distribution = openmanic_ui_egui::DistributionSnapshot::try_from_contributions(
        openmanic_ui_egui::DistributionGrouping::Category,
        contributions,
    )
    .map_err(|_| ())?;
    Ok((
        ApplicationUsageSnapshot::new(range_label(range), usage),
        distribution,
    ))
}

fn range_label(range: HalfOpenInterval) -> String {
    format!("{}–{} UTC", range.start().get(), range.end().get())
}

const fn tracking_status_label(status: &MutationStatus) -> &'static str {
    match status {
        MutationStatus::Pending => "Tracking request pending…",
        MutationStatus::Confirmed { .. } => "Tracking request confirmed.",
        MutationStatus::Rejected { .. } => "Tracking request was not saved.",
    }
}

fn id_label(bytes: &[u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut label = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        label.push(char::from(HEX[usize::from(byte >> 4)]));
        label.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    label
}

static TRACKER_RUN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn tracker_run_id() -> TrackerRunId {
    let time = utc_now_micros().to_be_bytes();
    let sequence = TRACKER_RUN_SEQUENCE
        .fetch_add(1, Ordering::Relaxed)
        .to_be_bytes();
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&time);
    bytes[8..].copy_from_slice(&sequence);
    TrackerRunId::from_bytes(bytes)
}

/// Owns the composed vertical-slice resources until coordinated explicit quit completes.
pub struct VerticalSlice {
    bootstrap: BootstrapState,
    ui: UiController<TrackingCommand, TodaySnapshot>,
    snapshots: LatestMailboxReceiver<SnapshotEnvelope<TodaySnapshot>>,
    runtime: RuntimeResources,
    shutdown: ShutdownCoordinator,
    #[cfg(windows)]
    instance_owner: WindowsInstanceOwner,
}

struct VerticalSliceApp {
    bootstrap: BootstrapState,
    app: OpenManicApp<TrackingCommand, TodaySnapshot>,
    snapshots: LatestMailboxReceiver<SnapshotEnvelope<TodaySnapshot>>,
    runtime: RuntimeResources,
    shutdown: ShutdownCoordinator,
    today: TodayController,
    timeline: TimelineRenderer,
    close_to_tray: WindowsTrayController,
    projection_sequence: u64,
    requested_range: Option<HalfOpenInterval>,
    #[cfg(windows)]
    instance_owner: WindowsInstanceOwner,
}

impl VerticalSlice {
    /// Creates the accepted component graph after bootstrap has exclusively locked the data root.
    fn new(
        bootstrap: BootstrapState,
        #[cfg(windows)] instance_owner: WindowsInstanceOwner,
    ) -> Result<Self, CompositionError> {
        let store_path = bootstrap.data_root().path().join("openmanic.sqlite3");
        let store = SqliteStore::open(
            &store_path,
            &StoreOpenOptions::new(
                store_identity(bootstrap.data_root().path()),
                utc_now_micros(),
                env!("CARGO_PKG_VERSION"),
            ),
        )
        .map_err(|_| CompositionError::Storage)?;
        let mut ui = UiController::try_new(
            UiModel::default(),
            UI_INBOUND_CAPACITY,
            UI_OUTBOUND_CAPACITY,
        )
        .map_err(|_| CompositionError::Runtime)?;
        let (mut runtime, snapshots) = RuntimeResources::start(store)?;
        #[cfg(windows)]
        runtime.start_windows(&instance_owner)?;
        let Some(initial_range) = day_range(0) else {
            return Err(CompositionError::Runtime);
        };
        let request = projection_request(1, initial_range);
        ui.begin_projection(&request);
        let _ = runtime.projection_requests.publish(request);
        Ok(Self {
            bootstrap,
            ui,
            snapshots,
            runtime,
            shutdown: ShutdownCoordinator::new(),
            #[cfg(windows)]
            instance_owner,
        })
    }

    /// Returns the coordinated quit state machine owned by the process boundary.
    #[must_use]
    pub const fn shutdown(&self) -> ShutdownCoordinator {
        self.shutdown
    }

    /// Retains process-owned resources that must outlive all platform and storage work.
    #[must_use]
    pub fn retained_resource_count(&self) -> usize {
        let _ = (&self.bootstrap, &self.runtime, self.shutdown);
        3
    }

    fn into_native_app(self) -> VerticalSliceApp {
        VerticalSliceApp {
            bootstrap: self.bootstrap,
            app: OpenManicApp::new(self.ui),
            snapshots: self.snapshots,
            runtime: self.runtime,
            shutdown: self.shutdown,
            today: TodayController::new(),
            timeline: TimelineRenderer::new(),
            close_to_tray: WindowsTrayController::new(),
            projection_sequence: 1,
            requested_range: None,
            #[cfg(windows)]
            instance_owner: self.instance_owner,
        }
    }
}

impl VerticalSliceApp {
    fn drain_worker_ingress(&mut self) {
        self.runtime
            .ui_inbox
            .drain_into(self.app.controller_mut(), UI_INBOUND_CAPACITY / 2);
        if let MailboxReceive::Latest(snapshot) = self.snapshots.try_receive() {
            let _ = self
                .app
                .controller_mut()
                .try_enqueue_inbound(InboundMessage::Snapshot(snapshot));
        }
    }

    fn publish_projection(&mut self, range: HalfOpenInterval) {
        if self.requested_range == Some(range) {
            return;
        }
        self.projection_sequence = self.projection_sequence.saturating_add(1);
        let request = projection_request(self.projection_sequence, range);
        self.app.controller_mut().begin_projection(&request);
        let _ = self.runtime.projection_requests.publish(request);
        self.requested_range = Some(range);
    }

    fn publish_day_projection(&mut self) {
        let offset = self
            .app
            .controller()
            .model()
            .today_view_context()
            .selected_day_offset();
        let Some(range) = day_range(offset) else {
            return;
        };
        self.publish_projection(range);
    }

    fn render_today_dashboard(&mut self, ui: &mut eframe::egui::Ui) {
        if self.app.controller().model().route() != openmanic_ui_egui::Route::Today {
            return;
        }
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui.button("Pause tracking").clicked() {
                self.queue_tracking_control(TrackingControlAction::Pause);
            }
            if ui.button("Resume tracking").clicked() {
                self.queue_tracking_control(TrackingControlAction::Resume);
            }
            if let Some(acknowledgement) = self
                .today
                .tracking_acknowledgement(self.app.controller().model())
            {
                ui.label(tracking_status_label(acknowledgement.status()));
            }
        });
        let data = self
            .app
            .controller()
            .model()
            .data()
            .visible_value()
            .cloned();
        let context = self.app.controller().model().today_view_context().clone();
        let Some(snapshot) = data else {
            return;
        };

        ui.separator();
        ui.heading("Timeline");
        let output = self
            .timeline
            .show_snapshot(ui, snapshot.timeline(), &context, false);
        for action in output.actions().iter().copied() {
            match action {
                TimelineRenderAction::Today(action) => {
                    self.app
                        .controller_mut()
                        .reduce_local(UiAction::Today(action));
                }
                TimelineRenderAction::ViewRangeChanged { range } => self.publish_projection(range),
                TimelineRenderAction::ScheduleRequested { .. } => {}
            }
        }
        ui.add_space(12.0);
        ui.heading("Application usage");
        render_usage_snapshot(ui, snapshot.usage());
        ui.add_space(12.0);
        ui.heading("Time distribution");
        render_distribution_snapshot(ui, snapshot.distribution());
    }

    fn queue_tracking_control(&mut self, action: TrackingControlAction) {
        let request = self.runtime.identifiers.tracking_request(action);
        let _ = self
            .today
            .queue_tracking(self.app.controller_mut(), request);
    }

    fn dispatch_ui_commands(&mut self) {
        let mut dispatcher = RuntimeCommandDispatcher {
            lanes: self.runtime.lanes.clone(),
            accepting: Arc::clone(&self.runtime.accepting),
        };
        let _ = self
            .app
            .controller_mut()
            .drain_dispatcher(&mut dispatcher, UI_OUTBOUND_CAPACITY);
    }

    fn handle_close_request(&mut self, context: &eframe::egui::Context) {
        if !context.input(|input| input.viewport().close_requested()) {
            return;
        }
        match self.close_to_tray.on_main_window_close(true) {
            openmanic_platform::CloseToTrayDisposition::HideToTray { .. } => {
                context.send_viewport_cmd(eframe::egui::ViewportCommand::CancelClose);
                context.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(false));
            }
            openmanic_platform::CloseToTrayDisposition::BeginCoordinatedQuit => {
                self.runtime.quit_requested.store(true, Ordering::Release);
            }
        }
    }

    fn stop_platform(&mut self) -> bool {
        #[cfg(windows)]
        {
            self.runtime.stop_windows(&self.instance_owner)
        }
        #[cfg(not(windows))]
        {
            true
        }
    }

    fn begin_shutdown(&mut self, context: &eframe::egui::Context) {
        if matches!(self.shutdown.phase(), ShutdownPhase::Running) && self.shutdown.begin().is_err()
        {
            return;
        }
        while let ShutdownPhase::Executing(step) = self.shutdown.phase() {
            let succeeded = match step {
                ShutdownStep::RejectNonessentialWork => {
                    self.runtime.reject_nonessential_work();
                    true
                }
                ShutdownStep::CancelSafeReads
                | ShutdownStep::FlushSettings
                | ShutdownStep::JoinReadersAndWorkers => true,
                ShutdownStep::CheckpointCriticalActivity => self.runtime.checkpoint_writer(),
                ShutdownStep::CloseWriter => self.runtime.close_writer(),
                ShutdownStep::StopPlatform => self.stop_platform(),
                ShutdownStep::JoinSupervisor => {
                    let _ = &self.runtime.supervisor;
                    true
                }
            };
            if succeeded {
                let _ = self.shutdown.complete(step);
                continue;
            }
            let _ = self.shutdown.fail_critical(step);
            break;
        }
        if matches!(self.shutdown.phase(), ShutdownPhase::Complete) {
            context.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for VerticalSliceApp {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut eframe::Frame) {
        // The bootstrap data-root lock, instance owner, worker handles, and supervisor remain
        // process-owned while the viewport is hidden or stalled.
        let _ = (&self.bootstrap, &self.runtime.supervisor, &*frame);
        self.drain_worker_ingress();
        eframe::App::ui(&mut self.app, ui, frame);
        self.render_today_dashboard(ui);
        self.publish_day_projection();
        self.dispatch_ui_commands();
        let context = ui.ctx().clone();
        self.handle_close_request(&context);
        if self.runtime.restore_requested.swap(false, Ordering::AcqRel) {
            context.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(true));
            context.send_viewport_cmd(eframe::egui::ViewportCommand::Focus);
        }
        if self.runtime.quit_requested.swap(false, Ordering::AcqRel) {
            self.begin_shutdown(&context);
        }
        context.request_repaint_after(UI_POLL_INTERVAL);
    }
}

/// Runs the safe pre-worker composition sequence.
///
/// # Errors
///
/// Returns a privacy-safe startup failure when command-line parsing, instance coordination,
/// data-root bootstrap, local store initialization, bounded UI setup, or the native host cannot
/// complete.
pub fn run_process() -> Result<(), CompositionError> {
    let cli = parse_process_cli().map_err(CompositionError::Cli)?;
    #[cfg(windows)]
    let instance_owner =
        match WindowsInstanceOwner::acquire().map_err(|_| CompositionError::Instance)? {
            InstanceAcquisition::Primary(owner) => owner,
            InstanceAcquisition::ExistingInstance(existing) => {
                let _ = existing.send(LocalActivationCommand::Activate);
                return Ok(());
            }
        };
    let artifact_directory = artifact_directory()?;
    let validator = LocalDataRootValidator::new(RejectKnownNetworkShares);
    let environment_root = std::env::var_os("OPENMANIC_DATA_DIR").map(PathBuf::from);
    let disposition = bootstrap(
        &cli,
        environment_root,
        None,
        &artifact_directory,
        &validator,
    )
    .map_err(CompositionError::Bootstrap)?;
    let BootstrapDisposition::Ready(bootstrap) = disposition else {
        return Err(CompositionError::DirectoryChoiceRequired);
    };
    let slice = VerticalSlice::new(
        bootstrap,
        #[cfg(windows)]
        instance_owner,
    )?;
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1_280.0, 860.0])
            .with_min_inner_size([720.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "OpenManic",
        native_options,
        Box::new(move |_| Ok(Box::new(slice.into_native_app()))),
    )
    .map_err(|_| CompositionError::NativeUi)
}

fn artifact_directory() -> Result<PathBuf, CompositionError> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .ok_or(CompositionError::Storage)
}

fn utc_now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
        })
}

fn day_range(offset_days: i32) -> Option<HalfOpenInterval> {
    const DAY_US: i64 = 86_400_000_000;
    let now = utc_now_micros();
    let day_start = now.div_euclid(DAY_US).saturating_mul(DAY_US);
    let offset = i64::from(offset_days).saturating_mul(DAY_US);
    let start = day_start.saturating_add(offset);
    HalfOpenInterval::try_new(
        UtcMicros::new(start),
        UtcMicros::new(start.saturating_add(DAY_US)),
    )
    .ok()
}

fn projection_request(
    sequence: u64,
    range: HalfOpenInterval,
) -> ProjectionRequest<TimelineContext> {
    let context_key = ProjectionContextKey::new(sequence);
    ProjectionRequest::new(
        openmanic_application::RequestId::new(sequence),
        ProjectionSlot::new(1),
        context_key,
        DataRevision::new(0),
        TimelineContext::new(context_key, range),
    )
}

fn store_identity(root: &Path) -> [u8; 16] {
    let mut first = 0xcbf2_9ce4_8422_2325_u64;
    let mut second = 0x9e37_79b9_7f4a_7c15_u64;
    for byte in root.as_os_str().to_string_lossy().as_bytes() {
        first = (first ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        second = second.rotate_left(5) ^ u64::from(*byte);
        second = second.wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    let mut identity = [0_u8; 16];
    identity[..8].copy_from_slice(&first.to_be_bytes());
    identity[8..].copy_from_slice(&second.to_be_bytes());
    identity
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use openmanic_application::{
        LaneCapacities, LaneReceive, TrackingCommand, TrackingEvidence, WorkLane,
        bounded_runtime_lanes,
    };
    use openmanic_domain::{ApplicationId, UtcMicros};
    use openmanic_platform::WindowsPlatformAction;

    use super::{
        CommandIdentifiers, PlatformActionRouter, UiInbox, UiIngress, day_range, store_identity,
    };

    #[test]
    fn writer_ingress_is_bounded_and_retains_critical_tracking_commands() {
        let capacities = LaneCapacities::try_new(1, 1, 1).expect("nonzero fixture lanes");
        let (lanes, receiver) = bounded_runtime_lanes(capacities);
        let identifiers = Arc::new(CommandIdentifiers::default());
        let first = identifiers.command(openmanic_application::TrackingCommand::Pause);
        let second = identifiers.command(openmanic_application::TrackingCommand::Resume);
        assert!(matches!(
            lanes.try_submit(WorkLane::Critical, super::WriterWork::Ui(first)),
            openmanic_application::LaneSubmit::Enqueued
        ));
        assert!(matches!(
            lanes.try_submit(WorkLane::Critical, super::WriterWork::Ui(second)),
            openmanic_application::LaneSubmit::Retained { .. }
        ));
        assert!(matches!(receiver.try_receive(), LaneReceive::Work { .. }));
    }

    #[test]
    fn tray_actions_route_pause_resume_and_quit_without_invoking_storage() {
        let capacities = LaneCapacities::try_new(4, 1, 1).expect("nonzero fixture lanes");
        let (lanes, receiver) = bounded_runtime_lanes(capacities);
        let inbox = Arc::new(UiInbox::new(4));
        let router = PlatformActionRouter {
            lanes: Arc::new(lanes),
            ui_inbox: Arc::clone(&inbox),
            identifiers: Arc::new(CommandIdentifiers::default()),
            accepting: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            restore_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            quit_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        router.route(WindowsPlatformAction::PauseTracking);
        router.route(WindowsPlatformAction::ResumeTracking);
        router.route(WindowsPlatformAction::Quit);
        let command_id = match receiver.try_receive() {
            LaneReceive::Work {
                work: super::WriterWork::Ui(command),
                ..
            } => Some(command.command_id()),
            _ => None,
        };
        assert!(matches!(
            inbox.queue.lock().expect("fixture inbox").front(),
            Some(UiIngress::Pending(pending)) if Some(*pending) == command_id
        ));
        assert!(matches!(receiver.try_receive(), LaneReceive::Work { .. }));
        assert!(
            router
                .quit_requested
                .load(std::sync::atomic::Ordering::Acquire)
        );
        assert!(matches!(
            inbox.queue.lock().expect("fixture inbox").front(),
            Some(UiIngress::Pending(_))
        ));
    }

    #[test]
    fn rejected_tray_submission_does_not_leave_a_pending_acknowledgement() {
        let capacities = LaneCapacities::try_new(1, 1, 1).expect("nonzero fixture lanes");
        let (lanes, _receiver) = bounded_runtime_lanes(capacities);
        let identifiers = Arc::new(CommandIdentifiers::default());
        let initial = identifiers.command(openmanic_application::TrackingCommand::Checkpoint);
        assert!(matches!(
            lanes.try_submit(WorkLane::Critical, super::WriterWork::System(initial)),
            openmanic_application::LaneSubmit::Enqueued
        ));
        let inbox = Arc::new(UiInbox::new(1));
        let router = PlatformActionRouter {
            lanes: Arc::new(lanes),
            ui_inbox: Arc::clone(&inbox),
            identifiers,
            accepting: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            restore_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            quit_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };

        router.route(WindowsPlatformAction::PauseTracking);

        assert!(inbox.queue.lock().expect("fixture inbox").is_empty());
    }

    #[test]
    fn shutdown_coordinator_requires_writer_before_platform_stop() {
        let mut shutdown = openmanic_application::ShutdownCoordinator::new();
        let _ = shutdown.begin().expect("explicit quit begins");
        for step in [
            openmanic_application::ShutdownStep::RejectNonessentialWork,
            openmanic_application::ShutdownStep::CancelSafeReads,
            openmanic_application::ShutdownStep::CheckpointCriticalActivity,
            openmanic_application::ShutdownStep::FlushSettings,
            openmanic_application::ShutdownStep::JoinReadersAndWorkers,
        ] {
            let _ = shutdown
                .complete(step)
                .expect("ordered prerequisite completes");
        }
        assert!(matches!(
            shutdown.phase(),
            openmanic_application::ShutdownPhase::Executing(
                openmanic_application::ShutdownStep::CloseWriter
            )
        ));
        let _ = shutdown
            .complete(openmanic_application::ShutdownStep::CloseWriter)
            .expect("writer closes before platform work");
        assert!(matches!(
            shutdown.phase(),
            openmanic_application::ShutdownPhase::Executing(
                openmanic_application::ShutdownStep::StopPlatform
            )
        ));
    }

    #[test]
    fn store_identity_and_day_range_are_stable() {
        assert_eq!(
            store_identity(std::path::Path::new("local-data")),
            store_identity(std::path::Path::new("local-data"))
        );
        assert_eq!(
            day_range(0)
                .expect("the current UTC day has a positive range")
                .duration_us(),
            86_400_000_000
        );
    }

    #[test]
    fn excluded_foreground_translation_preserves_command_metadata_and_redacts_application() {
        let identifiers = CommandIdentifiers::default();
        let command =
            identifiers.command(TrackingCommand::Evidence(TrackingEvidence::Foreground {
                sequence: 7,
                observed_at_utc: UtcMicros::new(123_456),
                application_id: ApplicationId::from_bytes([9; 16]),
            }));
        let metadata = (
            command.schema_revision(),
            command.command_id(),
            command.ordering_key(),
            command.expected_entity_revision(),
            command.submitted_at_utc(),
        );

        let translated = super::foreground_command_with_exclusion(command, true);

        assert_eq!(translated.schema_revision(), metadata.0);
        assert_eq!(translated.command_id(), metadata.1);
        assert_eq!(translated.ordering_key(), metadata.2);
        assert_eq!(translated.expected_entity_revision(), metadata.3);
        assert_eq!(translated.submitted_at_utc(), metadata.4);
        assert!(matches!(
            translated.payload(),
            TrackingCommand::Evidence(TrackingEvidence::ExcludedForeground {
                sequence: 7,
                observed_at_utc,
            }) if *observed_at_utc == UtcMicros::new(123_456)
        ));
    }
}
