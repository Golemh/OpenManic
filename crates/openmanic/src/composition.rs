//! Primary process composition for the first Windows tracking vertical slice.
//!
//! The composition root owns process lifetime and wires accepted boundaries together. SQLite,
//! tracking reduction, projection work, and Windows callbacks never execute in an egui frame.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    error::Error,
    fmt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use openmanic_application::{
    AppEvent, ApplicationError, ApplicationPort, CommandEnvelope, CommandId, CommandReceipt,
    CatalogCommand, CatalogPersistence, CatalogPersistenceError, CatalogService,
    DataRevision, EventEnvelope, LaneCapacities, LaneReceive, LaneSubmit, LatestMailbox,
    LatestMailboxReceiver, MailboxReceive, OrderingKey, PortFailureReason, ProjectionContextKey,
    ProjectionRequest, ProjectionSlot, RuntimeLaneReceiver, RuntimeLanes, RuntimeSupervisor,
    RuntimeWorker, SchemaRevision, ShutdownCoordinator, ShutdownPhase, ShutdownStep,
    EntityRevision, FocusCommand, FocusNotificationError, FocusNotificationPort, FocusPersistence,
    FocusKind, FocusPersistenceError, FocusService, FocusSnapshot, MutationOutcome,
    ApplicationIconCache, ApplicationIconCacheLimits, ApplicationIconLookup, ApplicationIconResult,
    RecurringOccurrenceOverride, RecurringScheduleEdit, RecurringScheduleRuleChange,
    ScheduleCommand, ScheduleId,
    ScheduleOccurrenceId, SchedulePersistence, SchedulePersistenceError,
    ScheduleService, ScheduleSnapshot,
    SnapshotEnvelope, ThreadRoot, TimelineApplication, TimelineContext, TimelineProjector,
    TimelineRawIntervalId, TimelineSnapshot, TimelineSourceActivity, TrackingCommand,
    TrackingEvidence, TrackingPersistenceIntent, TrackingPersistencePort,
    TrackingPersistenceSubmit, TrackingService, WorkLane, bounded_runtime_lanes, latest_mailbox,
};
use openmanic_domain::{
    ActivityState, Application, ApplicationId, ApplicationName, Category, CategoryId,
    CategoryName, HalfOpenInterval,
    FocusSessionId, FocusSessionState, OneTimeScheduleId, ScheduleEditScope, ScheduleRule,
    ScheduleSeriesId, TrackerRunId, UtcMicros,
};
#[cfg(windows)]
use openmanic_platform::{
    ActivationCommandDecode, InstanceAcquisition, LocalActivationCommand, WindowsActivationServer,
    WindowsApplicationMetadataRequest, WindowsControlAdapter, WindowsInstanceOwner,
};
use openmanic_platform::{
    EvidencePublishResult, TrackingEvidenceSink, WindowsPlatformAction, WindowsTrayController,
};
use openmanic_storage_sqlite::{
    RecoveryOutcome, SqliteStore, StorageWriter, StoreOpenOptions, TrackerRunRegistration,
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
#[cfg(windows)]
const APPLICATION_METADATA_REQUEST_CAPACITY: usize = 32;
#[cfg(windows)]
const APPLICATION_ICON_CACHE_ENTRIES: usize = 128;
#[cfg(windows)]
const APPLICATION_ICON_CACHE_BYTES: usize = 16 * 1024 * 1024;

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
    schedules: Vec<ScheduleSnapshot>,
    applications: Vec<(Application, bool)>,
    categories: Vec<Category>,
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

    fn schedules(&self) -> &[ScheduleSnapshot] {
        &self.schedules
    }

    fn applications(&self) -> &[(Application, bool)] {
        &self.applications
    }

    fn categories(&self) -> &[Category] {
        &self.categories
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
    Catalog(CommandEnvelope<CatalogCommand>),
    Schedule(CommandEnvelope<ScheduleCommand>),
    Focus(CommandEnvelope<FocusCommand>),
}

impl WriterWork {
    fn into_parts(self) -> (CommandEnvelope<TrackingCommand>, bool) {
        match self {
            Self::System(command) | Self::CatalogForeground { command, .. } => (command, false),
            Self::Ui(command) => (command, true),
            Self::Catalog(_) | Self::Schedule(_) | Self::Focus(_) => {
                unreachable!("typed commands use their own application service")
            }
        }
    }
}

/// The stateful application services which are exclusively owned by the writer worker.
struct WriterServices {
    tracking: TrackingService<WriterPersistence>,
    catalog: CatalogService<WriterPersistence>,
    schedules: ScheduleService<WriterPersistence>,
    focus: FocusService<WriterPersistence, UnavailableFocusNotifications>,
    ui_event_sequence: u64,
}

/// Completion notifications are deliberately honest until the Windows notification adapter lands.
struct UnavailableFocusNotifications {
    latest_error: Arc<Mutex<Option<FocusNotificationError>>>,
}

impl FocusNotificationPort for UnavailableFocusNotifications {
    fn notify_completed(&mut self, _: &FocusSnapshot) -> Result<(), FocusNotificationError> {
        let error = FocusNotificationError::Unavailable;
        if let Ok(mut latest) = self.latest_error.lock() {
            *latest = Some(error);
        }
        Err(error)
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

#[derive(Clone, Copy)]
enum FocusLifecycleAction {
    Pause,
    Resume,
    Complete,
    Cancel,
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

    fn catalog_command(&self, payload: CatalogCommand) -> CommandEnvelope<CatalogCommand> {
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

    fn catalog_create_category(&self, name: &str) -> Option<CommandEnvelope<CatalogCommand>> {
        let (command_id, ordering_key, submitted_at_utc) = self.next_metadata();
        let category_name = CategoryName::try_new(name).ok()?;
        let mut id_bytes = [0_u8; 16];
        id_bytes[..8].copy_from_slice(&submitted_at_utc.get().to_be_bytes());
        id_bytes[8..].copy_from_slice(&command_id.get().to_be_bytes());
        Some(CommandEnvelope::new(
            SchemaRevision::new(1),
            command_id,
            ordering_key,
            None,
            submitted_at_utc,
            CatalogCommand::CreateCategory {
                category: Category::new(CategoryId::from_bytes(id_bytes), category_name),
                observed_at_utc: submitted_at_utc,
            },
        ))
    }

    fn tracking_request(&self, action: TrackingControlAction) -> TodayTrackingRequest {
        let (command_id, ordering_key, submitted_at_utc) = self.next_metadata();
        TodayTrackingRequest::new(action, command_id, ordering_key, submitted_at_utc)
    }

    fn schedule_create(
        &self,
        range: HalfOpenInterval,
        label: &str,
        repeats: bool,
        weekday_mask: u8,
    ) -> Result<CommandEnvelope<ScheduleCommand>, ScheduleDraftValidationError> {
        let (command_id, ordering_key, submitted_at_utc) = self.next_metadata();
        let mut id_bytes = [0_u8; 16];
        id_bytes[..8].copy_from_slice(&submitted_at_utc.get().to_be_bytes());
        id_bytes[8..].copy_from_slice(&command_id.get().to_be_bytes());
        let (id, rule) = if repeats {
            (
                ScheduleId::Series(ScheduleSeriesId::from_bytes(id_bytes)),
                recurring_schedule_rule(label, range, weekday_mask)?,
            )
        } else {
            (
                ScheduleId::OneTime(OneTimeScheduleId::from_bytes(id_bytes)),
                ScheduleRule::one_time(label, None, range, "Etc/UTC")
                    .map_err(|_| ScheduleDraftValidationError::CannotRepresent)?,
            )
        };
        let snapshot = ScheduleSnapshot::try_new(
            id,
            rule,
            EntityRevision::new(0),
            submitted_at_utc,
        )
        .map_err(|_| ScheduleDraftValidationError::CannotRepresent)?;
        Ok(CommandEnvelope::new(
            SchemaRevision::new(1),
            command_id,
            ordering_key,
            None,
            submitted_at_utc,
            ScheduleCommand::Create(snapshot),
        ))
    }

    fn schedule_delete(
        &self,
        snapshot: &ScheduleSnapshot,
    ) -> CommandEnvelope<ScheduleCommand> {
        let (command_id, ordering_key, submitted_at_utc) = self.next_metadata();
        CommandEnvelope::new(
            SchemaRevision::new(1),
            command_id,
            ordering_key,
            Some(snapshot.entity_revision()),
            submitted_at_utc,
            ScheduleCommand::Delete(snapshot.id()),
        )
    }

    fn schedule_replace(
        &self,
        snapshot: ScheduleSnapshot,
    ) -> CommandEnvelope<ScheduleCommand> {
        let (command_id, ordering_key, submitted_at_utc) = self.next_metadata();
        CommandEnvelope::new(
            SchemaRevision::new(1),
            command_id,
            ordering_key,
            Some(snapshot.entity_revision()),
            submitted_at_utc,
            ScheduleCommand::Replace(snapshot),
        )
    }

    fn schedule_delete_occurrence(
        &self,
        series_id: ScheduleSeriesId,
        anchor_date: i32,
        scope: ScheduleEditScope,
        expected_revision: EntityRevision,
    ) -> CommandEnvelope<ScheduleCommand> {
        let (command_id, ordering_key, submitted_at_utc) = self.next_metadata();
        CommandEnvelope::new(
            SchemaRevision::new(1),
            command_id,
            ordering_key,
            Some(expected_revision),
            submitted_at_utc,
            ScheduleCommand::DeleteOccurrence {
                series_id,
                anchor_date,
                scope,
            },
        )
    }

    fn schedule_edit_occurrence(
        &self,
        series_id: ScheduleSeriesId,
        anchor_date: i32,
        scope: ScheduleEditScope,
        expected_revision: EntityRevision,
        edit: RecurringScheduleEdit,
    ) -> CommandEnvelope<ScheduleCommand> {
        let (command_id, ordering_key, submitted_at_utc) = self.next_metadata();
        CommandEnvelope::new(
            SchemaRevision::new(1), command_id, ordering_key, Some(expected_revision),
            submitted_at_utc, ScheduleCommand::EditOccurrence { series_id, anchor_date, scope, edit },
        )
    }

    fn focus_draft(&self) -> Option<(FocusSessionId, CommandEnvelope<FocusCommand>)> {
        const FOCUS_DURATION_US: i64 = 25 * 60 * 1_000_000;
        let (command_id, ordering_key, submitted_at_utc) = self.next_metadata();
        let mut id_bytes = [0_u8; 16];
        id_bytes[..8].copy_from_slice(&submitted_at_utc.get().to_be_bytes());
        id_bytes[8..].copy_from_slice(&command_id.get().to_be_bytes());
        let session_id = FocusSessionId::from_bytes(id_bytes);
        let snapshot = FocusSnapshot::try_new(
            session_id,
            FocusKind::Focus,
            Some("Focus session".to_owned()),
            FOCUS_DURATION_US,
            None,
            EntityRevision::new(0),
        )
        .ok()?;
        Some((
            session_id,
            CommandEnvelope::new(
                SchemaRevision::new(1),
                command_id,
                ordering_key,
                None,
                submitted_at_utc,
                FocusCommand::CreateDraft(snapshot),
            ),
        ))
    }

    fn focus_start(&self, session_id: FocusSessionId) -> CommandEnvelope<FocusCommand> {
        let (command_id, ordering_key, submitted_at_utc) = self.next_metadata();
        CommandEnvelope::new(
            SchemaRevision::new(1),
            command_id,
            ordering_key,
            None,
            submitted_at_utc,
            FocusCommand::Start {
                session_id,
                started_at: submitted_at_utc,
            },
        )
    }

    fn focus_lifecycle(
        &self,
        session_id: FocusSessionId,
        action: FocusLifecycleAction,
    ) -> CommandEnvelope<FocusCommand> {
        let (command_id, ordering_key, submitted_at_utc) = self.next_metadata();
        let payload = match action {
            FocusLifecycleAction::Pause => FocusCommand::Pause {
                session_id,
                paused_at: submitted_at_utc,
            },
            FocusLifecycleAction::Resume => FocusCommand::Resume {
                session_id,
                resumed_at: submitted_at_utc,
            },
            FocusLifecycleAction::Complete => FocusCommand::Complete {
                session_id,
                completed_at: submitted_at_utc,
            },
            FocusLifecycleAction::Cancel => FocusCommand::Cancel {
                session_id,
                cancelled_at: submitted_at_utc,
            },
        };
        CommandEnvelope::new(
            SchemaRevision::new(1),
            command_id,
            ordering_key,
            None,
            submitted_at_utc,
            payload,
        )
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
            WindowsPlatformAction::StartFocusSession => self.route_focus(),
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

    fn route_focus(&self) {
        if !self.accepting.load(Ordering::Acquire) {
            return;
        }
        let Some((session_id, draft)) = self.identifiers.focus_draft() else {
            return;
        };
        let start = self.identifiers.focus_start(session_id);
        if !self.submit_focus(draft) {
            return;
        }
        let _ = self.submit_focus(start);
    }

    fn submit_focus(&self, command: CommandEnvelope<FocusCommand>) -> bool {
        let command_id = command.command_id();
        if !self.ui_inbox.try_push(UiIngress::Pending(command_id)) {
            return false;
        }
        if matches!(
            self.lanes
                .try_submit(WorkLane::Critical, WriterWork::Focus(command)),
            LaneSubmit::Enqueued
        ) {
            return true;
        }
        self.ui_inbox.remove_pending(command_id);
        false
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

impl CatalogPersistence for WriterPersistence {
    fn create_category(
        &mut self,
        category: &Category,
        observed_at_utc: UtcMicros,
    ) -> Result<DataRevision, CatalogPersistenceError> {
        let Ok(mut store) = self.store.lock() else {
            return Err(CatalogPersistenceError::Failed);
        };
        <StorageWriter as CatalogPersistence>::create_category(
            store.writer(),
            category,
            observed_at_utc,
        )
    }

    fn rename_category(
        &mut self,
        category_id: CategoryId,
        name: &CategoryName,
        observed_at_utc: UtcMicros,
    ) -> Result<DataRevision, CatalogPersistenceError> {
        let Ok(mut store) = self.store.lock() else {
            return Err(CatalogPersistenceError::Failed);
        };
        <StorageWriter as CatalogPersistence>::rename_category(
            store.writer(),
            category_id,
            name,
            observed_at_utc,
        )
    }

    fn delete_category(
        &mut self,
        category_id: CategoryId,
    ) -> Result<DataRevision, CatalogPersistenceError> {
        let Ok(mut store) = self.store.lock() else {
            return Err(CatalogPersistenceError::Failed);
        };
        <StorageWriter as CatalogPersistence>::delete_category(store.writer(), category_id)
    }

    fn assign_applications(
        &mut self,
        application_ids: &[ApplicationId],
        category_id: Option<CategoryId>,
    ) -> Result<DataRevision, CatalogPersistenceError> {
        let Ok(mut store) = self.store.lock() else {
            return Err(CatalogPersistenceError::Failed);
        };
        <StorageWriter as CatalogPersistence>::assign_applications(
            store.writer(),
            application_ids,
            category_id,
        )
    }

    fn set_applications_excluded(
        &mut self,
        application_ids: &[ApplicationId],
        excluded: bool,
    ) -> Result<DataRevision, CatalogPersistenceError> {
        let Ok(mut store) = self.store.lock() else {
            return Err(CatalogPersistenceError::Failed);
        };
        <StorageWriter as CatalogPersistence>::set_applications_excluded(
            store.writer(),
            application_ids,
            excluded,
        )
    }
}

impl SchedulePersistence for WriterPersistence {
    fn load_schedule(
        &mut self,
        schedule_id: ScheduleId,
    ) -> Result<Option<ScheduleSnapshot>, SchedulePersistenceError> {
        let Ok(mut store) = self.store.lock() else {
            return Err(SchedulePersistenceError::Failed);
        };
        store.writer().load_schedule(schedule_id)
    }

    fn create_schedule(
        &mut self,
        snapshot: &ScheduleSnapshot,
    ) -> Result<DataRevision, SchedulePersistenceError> {
        let Ok(mut store) = self.store.lock() else {
            return Err(SchedulePersistenceError::Failed);
        };
        store.writer().create_schedule(snapshot)
    }

    fn replace_schedule(
        &mut self,
        snapshot: &ScheduleSnapshot,
        expected_revision: openmanic_application::EntityRevision,
    ) -> Result<DataRevision, SchedulePersistenceError> {
        let Ok(mut store) = self.store.lock() else {
            return Err(SchedulePersistenceError::Failed);
        };
        store.writer().replace_schedule(snapshot, expected_revision)
    }

    fn delete_schedule(
        &mut self,
        schedule_id: openmanic_application::ScheduleId,
        expected_revision: openmanic_application::EntityRevision,
    ) -> Result<DataRevision, SchedulePersistenceError> {
        let Ok(mut store) = self.store.lock() else {
            return Err(SchedulePersistenceError::Failed);
        };
        store.writer().delete_schedule(schedule_id, expected_revision)
    }
}

impl FocusPersistence for WriterPersistence {
    fn load_focus(
        &mut self,
        session_id: FocusSessionId,
    ) -> Result<Option<FocusSnapshot>, FocusPersistenceError> {
        let Ok(mut store) = self.store.lock() else {
            return Err(FocusPersistenceError::Failed);
        };
        store.writer().load_focus(session_id)
    }

    fn load_active_focus(&mut self) -> Result<Option<FocusSnapshot>, FocusPersistenceError> {
        let Ok(mut store) = self.store.lock() else {
            return Err(FocusPersistenceError::Failed);
        };
        store.writer().load_active_focus()
    }

    fn create_focus(
        &mut self,
        snapshot: &FocusSnapshot,
    ) -> Result<DataRevision, FocusPersistenceError> {
        let Ok(mut store) = self.store.lock() else {
            return Err(FocusPersistenceError::Failed);
        };
        store.writer().create_focus(snapshot)
    }

    fn replace_focus(
        &mut self,
        snapshot: &FocusSnapshot,
    ) -> Result<(DataRevision, EntityRevision), FocusPersistenceError> {
        let Ok(mut store) = self.store.lock() else {
            return Err(FocusPersistenceError::Failed);
        };
        store.writer().replace_focus(snapshot)
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
    focus_notification_error: Arc<Mutex<Option<FocusNotificationError>>>,
    focus_snapshot: Arc<Mutex<Option<FocusSnapshot>>>,
    restore_requested: Arc<AtomicBool>,
    quit_requested: Arc<AtomicBool>,
    supervisor: RuntimeSupervisor,
    #[cfg(windows)]
    platform_stop: Option<mpsc::Sender<()>>,
    #[cfg(windows)]
    platform_handle: Option<JoinHandle<()>>,
    #[cfg(windows)]
    metadata_stop: Option<mpsc::Sender<()>>,
    #[cfg(windows)]
    metadata_handle: Option<JoinHandle<()>>,
    #[cfg(windows)]
    application_icons: Arc<Mutex<ApplicationIconCache>>,
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
        let focus_notification_error = Arc::new(Mutex::new(None));
        let focus_snapshot = Arc::new(Mutex::new(None));
        let worker_ui_inbox = Arc::clone(&ui_inbox);
        let worker_focus_notification_error = Arc::clone(&focus_notification_error);
        let worker_focus_snapshot = Arc::clone(&focus_snapshot);
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
                    worker_focus_notification_error,
                    worker_focus_snapshot,
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
                focus_notification_error,
                focus_snapshot,
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
                metadata_stop: None,
                #[cfg(windows)]
                metadata_handle: None,
                #[cfg(windows)]
                application_icons: Arc::new(Mutex::new(ApplicationIconCache::new(
                    ApplicationIconCacheLimits::try_new(
                        APPLICATION_ICON_CACHE_ENTRIES,
                        APPLICATION_ICON_CACHE_BYTES,
                    )
                    .map_err(|_| CompositionError::Runtime)?,
                ))),
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

    #[cfg(windows)]
    fn application_icon(&self, application_id: ApplicationId) -> Option<openmanic_application::ApplicationIcon> {
        let Ok(mut cache) = self.application_icons.lock() else {
            return None;
        };
        match cache.lookup_application(application_id) {
            ApplicationIconLookup::Ready(icon) => Some(icon.clone()),
            ApplicationIconLookup::Fallback => None,
        }
    }

    fn try_submit_schedule(&self, command: CommandEnvelope<ScheduleCommand>) -> Option<CommandId> {
        if !self.accepting.load(Ordering::Acquire) {
            return None;
        }
        let command_id = command.command_id();
        if !self.ui_inbox.try_push(UiIngress::Pending(command_id)) {
            return None;
        }
        if matches!(
            self.lanes
                .try_submit(WorkLane::Normal, WriterWork::Schedule(command)),
            LaneSubmit::Enqueued
        ) {
            return Some(command_id);
        }
        self.ui_inbox.remove_pending(command_id);
        None
    }

    fn try_submit_catalog(&self, command: CommandEnvelope<CatalogCommand>) -> Option<CommandId> {
        if !self.accepting.load(Ordering::Acquire) {
            return None;
        }
        let command_id = command.command_id();
        if !self.ui_inbox.try_push(UiIngress::Pending(command_id)) {
            return None;
        }
        if matches!(
            self.lanes
                .try_submit(WorkLane::Normal, WriterWork::Catalog(command)),
            LaneSubmit::Enqueued
        ) {
            return Some(command_id);
        }
        self.ui_inbox.remove_pending(command_id);
        None
    }

    fn try_submit_focus(&self, command: CommandEnvelope<FocusCommand>) -> Option<CommandId> {
        if !self.accepting.load(Ordering::Acquire) {
            return None;
        }
        let command_id = command.command_id();
        if !self.ui_inbox.try_push(UiIngress::Pending(command_id)) {
            return None;
        }
        if matches!(
            self.lanes
                .try_submit(WorkLane::Normal, WriterWork::Focus(command)),
            LaneSubmit::Enqueued
        ) {
            return Some(command_id);
        }
        self.ui_inbox.remove_pending(command_id);
        None
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
        let (metadata_requests, metadata_stop, metadata_handle) = spawn_metadata_worker(
            Arc::clone(&self.application_icons),
        )?;
        let (stop_sender, platform_handle, ready_receiver) = spawn_platform_worker(
            self.action_router(),
            self.evidence_sink(),
            metadata_requests,
        )?;
        if let Ok(Ok(())) = ready_receiver.recv_timeout(Duration::from_secs(5)) {
            self.activation_handle = Some(activation_handle);
            self.platform_stop = Some(stop_sender);
            self.platform_handle = Some(platform_handle);
            self.metadata_stop = Some(metadata_stop);
            self.metadata_handle = Some(metadata_handle);
            return Ok(());
        }

        self.activation_stop.store(true, Ordering::Release);
        let _ = stop_sender.send(());
        let _ = metadata_stop.send(());
        let _ = activation_handle.join();
        let _ = platform_handle.join();
        let _ = metadata_handle.join();
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
        if let Some(stop) = self.metadata_stop.take() {
            let _ = stop.send(());
        }
        let metadata_joined = self
            .metadata_handle
            .take()
            .is_none_or(|handle| handle.join().is_ok());
        let activation_joined = self
            .activation_handle
            .take()
            .is_none_or(|handle| handle.join().is_ok());
        platform_joined && metadata_joined && activation_joined
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
    metadata_requests: SyncSender<WindowsApplicationMetadataRequest>,
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
            if run_platform_worker(
                action_router,
                evidence_sink,
                metadata_requests,
                stop_receiver,
                &ready_sender,
            )
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
    metadata_requests: SyncSender<WindowsApplicationMetadataRequest>,
    stop_receiver: Receiver<()>,
    ready_sender: &SyncSender<Result<(), ()>>,
) -> Result<(), openmanic_platform::WindowsControlError> {
    let mut adapter = WindowsControlAdapter::new().with_metadata_requests(metadata_requests);
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

#[cfg(windows)]
fn spawn_metadata_worker(
    application_icons: Arc<Mutex<ApplicationIconCache>>,
) -> Result<(SyncSender<WindowsApplicationMetadataRequest>, mpsc::Sender<()>, JoinHandle<()>), CompositionError> {
    let (request_sender, request_receiver) = mpsc::sync_channel(APPLICATION_METADATA_REQUEST_CAPACITY);
    let (stop_sender, stop_receiver) = mpsc::channel();
    let handle = thread::Builder::new()
        .name(ThreadRoot::new(RuntimeWorker::BulkWorker).name().to_owned())
        .spawn(move || run_metadata_worker(request_receiver, stop_receiver, application_icons))
        .map_err(|_| CompositionError::Runtime)?;
    Ok((request_sender, stop_sender, handle))
}

#[cfg(windows)]
fn run_metadata_worker(
    requests: Receiver<WindowsApplicationMetadataRequest>,
    stop: Receiver<()>,
    application_icons: Arc<Mutex<ApplicationIconCache>>,
) {
    loop {
        if matches!(stop.try_recv(), Ok(()) | Err(TryRecvError::Disconnected)) {
            return;
        }
        let request = match requests.recv_timeout(PLATFORM_PUMP_INTERVAL) {
            Ok(request) => request,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        let result: ApplicationIconResult = openmanic_platform::extract_application_icon(request);
        if let Ok(mut cache) = application_icons.lock() {
            let _ = result.apply_to(&mut cache);
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
    focus_notification_error: Arc<Mutex<Option<FocusNotificationError>>>,
    focus_snapshot: Arc<Mutex<Option<FocusSnapshot>>>,
) {
    let store = Arc::new(Mutex::new(store));
    let mut services = writer_services(&store, focus_notification_error);
    reconcile_focus_snapshot(&mut services, &focus_snapshot);
    let mut current_projection = None;
    let mut last_focus_reconciliation = Instant::now();
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
                let event = services.tracking.handle(system_checkpoint());
                let persisted = event.committed_data_revision().is_some()
                    || services.tracking.pending_intent().is_none();
                if persisted && let Some(request) = current_projection.as_ref() {
                    publish_today_snapshot(&store, request, &snapshots);
                }
                let _ = reply.send(persisted);
                continue;
            }
            Ok(WriterControl::Close(reply)) => {
                drain_writer_lanes(
                    &lanes,
                    &mut services,
                    &store,
                    &mut current_projection,
                    &snapshots,
                    &ui_inbox,
                    &focus_snapshot,
                );
                let event = services.tracking.handle(system_checkpoint());
                let persisted = event.committed_data_revision().is_some()
                    || services.tracking.pending_intent().is_none();
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
                &mut services,
                &store,
                &mut current_projection,
                &snapshots,
                &ui_inbox,
                &focus_snapshot,
            ),
            LaneReceive::Empty => thread::park_timeout(WORKER_IDLE_INTERVAL),
            LaneReceive::Closed => running = false,
        }
        if last_focus_reconciliation.elapsed() >= Duration::from_secs(1) {
            reconcile_focus_snapshot(&mut services, &focus_snapshot);
            last_focus_reconciliation = Instant::now();
        }
    }
}

fn writer_services(
    store: &Arc<Mutex<SqliteStore>>,
    focus_notification_error: Arc<Mutex<Option<FocusNotificationError>>>,
) -> WriterServices {
    let tracking = TrackingService::new(
        tracker_run_id(),
        WriterPersistence {
            store: Arc::clone(store),
            first_write: true,
        },
    );
    let schedules = ScheduleService::new(WriterPersistence {
        store: Arc::clone(store),
        first_write: false,
    });
    let catalog = CatalogService::new(WriterPersistence {
        store: Arc::clone(store),
        first_write: false,
    });
    let focus = FocusService::new(
        WriterPersistence {
            store: Arc::clone(store),
            first_write: false,
        },
        UnavailableFocusNotifications {
            latest_error: focus_notification_error,
        },
    );
    WriterServices {
        tracking,
        catalog,
        schedules,
        focus,
        ui_event_sequence: 0,
    }
}

fn drain_writer_lanes(
    lanes: &RuntimeLaneReceiver<WriterWork>,
    services: &mut WriterServices,
    store: &Arc<Mutex<SqliteStore>>,
    current_projection: &mut Option<ProjectionRequest<TimelineContext>>,
    snapshots: &LatestMailbox<SnapshotEnvelope<TodaySnapshot>>,
    ui_inbox: &UiInbox,
    focus_snapshot: &Arc<Mutex<Option<FocusSnapshot>>>,
) {
    while let LaneReceive::Work { work, .. } = lanes.try_receive() {
        process_writer_work(
            work,
            services,
            store,
            current_projection,
            snapshots,
            ui_inbox,
            focus_snapshot,
        );
    }
}

fn process_writer_work(
    work: WriterWork,
    services: &mut WriterServices,
    store: &Arc<Mutex<SqliteStore>>,
    current_projection: &mut Option<ProjectionRequest<TimelineContext>>,
    snapshots: &LatestMailbox<SnapshotEnvelope<TodaySnapshot>>,
    ui_inbox: &UiInbox,
    focus_snapshot: &Arc<Mutex<Option<FocusSnapshot>>>,
) {
    if let WriterWork::Focus(command) = work {
        process_focus_command(
            &command,
            services,
            store,
            current_projection,
            snapshots,
            ui_inbox,
            focus_snapshot,
        );
        return;
    }
    if let WriterWork::Schedule(command) = work {
        process_schedule_command(
            &command,
            services,
            store,
            current_projection,
            snapshots,
            ui_inbox,
        );
        return;
    }
    if let WriterWork::Catalog(command) = work {
        process_catalog_command(
            &command,
            services,
            store,
            current_projection,
            snapshots,
            ui_inbox,
        );
        return;
    }
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
            services,
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
        services,
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
    services: &mut WriterServices,
    store: &Arc<Mutex<SqliteStore>>,
    current_projection: &mut Option<ProjectionRequest<TimelineContext>>,
    snapshots: &LatestMailbox<SnapshotEnvelope<TodaySnapshot>>,
    ui_inbox: &UiInbox,
) {
    let event = services.tracking.handle(command);
    let changed = event.committed_data_revision().is_some();
    if from_ui {
        let _ = ui_inbox.try_push(UiIngress::Event(resequence_ui_event(
            event,
            &mut services.ui_event_sequence,
        )));
    }
    if changed && let Some(request) = current_projection.as_ref() {
        publish_today_snapshot(store, request, snapshots);
    }
}

fn process_schedule_command(
    command: &CommandEnvelope<ScheduleCommand>,
    services: &mut WriterServices,
    store: &Arc<Mutex<SqliteStore>>,
    current_projection: &mut Option<ProjectionRequest<TimelineContext>>,
    snapshots: &LatestMailbox<SnapshotEnvelope<TodaySnapshot>>,
    ui_inbox: &UiInbox,
) {
    let mutation = services.schedules.handle(command);
    let outcome = mutation.outcome().clone();
    let committed_revision = match &outcome {
        MutationOutcome::Confirmed(confirmation) => Some(confirmation.committed_data_revision()),
        MutationOutcome::Rejected(_) => None,
    };
    let event = EventEnvelope::new(
        command.schema_revision(),
        next_ui_event_sequence(&mut services.ui_event_sequence),
        Some(command.command_id()),
        committed_revision,
        command.submitted_at_utc(),
        AppEvent::Mutation(outcome),
    );
    let _ = ui_inbox.try_push(UiIngress::Event(event));
    if committed_revision.is_some()
        && let Some(request) = current_projection.as_ref()
    {
        publish_today_snapshot(store, request, snapshots);
    }
}

fn process_catalog_command(
    command: &CommandEnvelope<CatalogCommand>,
    services: &mut WriterServices,
    store: &Arc<Mutex<SqliteStore>>,
    current_projection: &mut Option<ProjectionRequest<TimelineContext>>,
    snapshots: &LatestMailbox<SnapshotEnvelope<TodaySnapshot>>,
    ui_inbox: &UiInbox,
) {
    let newly_excluded = match command.payload() {
        CatalogCommand::SetApplicationsExcluded {
            application_ids,
            excluded: true,
        } => Some(application_ids.as_slice()),
        _ => None,
    };
    let outcome = services.catalog.handle(command);
    let committed_revision = match &outcome {
        MutationOutcome::Confirmed(confirmation) => Some(confirmation.committed_data_revision()),
        MutationOutcome::Rejected(_) => None,
    };
    if committed_revision.is_some() && let Some(application_ids) = newly_excluded {
        for application_id in application_ids {
            let _ = services.tracking.reconcile_active_application_excluded(
                command.schema_revision(),
                command.command_id(),
                command.submitted_at_utc(),
                *application_id,
            );
        }
    }
    let event = EventEnvelope::new(
        command.schema_revision(),
        next_ui_event_sequence(&mut services.ui_event_sequence),
        Some(command.command_id()),
        committed_revision,
        command.submitted_at_utc(),
        AppEvent::Mutation(outcome),
    );
    let _ = ui_inbox.try_push(UiIngress::Event(event));
    if committed_revision.is_some()
        && let Some(request) = current_projection.as_ref()
    {
        publish_today_snapshot(store, request, snapshots);
    }
}

fn process_focus_command(
    command: &CommandEnvelope<FocusCommand>,
    services: &mut WriterServices,
    store: &Arc<Mutex<SqliteStore>>,
    current_projection: &mut Option<ProjectionRequest<TimelineContext>>,
    snapshots: &LatestMailbox<SnapshotEnvelope<TodaySnapshot>>,
    ui_inbox: &UiInbox,
    focus_snapshot: &Arc<Mutex<Option<FocusSnapshot>>>,
) {
    let mutation = services.focus.handle(command);
    if let Some(snapshot) = mutation.snapshot()
        && let Ok(mut latest) = focus_snapshot.lock()
    {
        *latest = Some(snapshot.clone());
    }
    let outcome = mutation.outcome().clone();
    let committed_revision = match &outcome {
        MutationOutcome::Confirmed(confirmation) => Some(confirmation.committed_data_revision()),
        MutationOutcome::Rejected(_) => None,
    };
    let event = EventEnvelope::new(
        command.schema_revision(),
        next_ui_event_sequence(&mut services.ui_event_sequence),
        Some(command.command_id()),
        committed_revision,
        command.submitted_at_utc(),
        AppEvent::Mutation(outcome),
    );
    let _ = ui_inbox.try_push(UiIngress::Event(event));
    if committed_revision.is_some()
        && let Some(request) = current_projection.as_ref()
    {
        publish_today_snapshot(store, request, snapshots);
    }
}

fn reconcile_focus_snapshot(
    services: &mut WriterServices,
    focus_snapshot: &Arc<Mutex<Option<FocusSnapshot>>>,
) {
    if let Ok(snapshot) = services
        .focus
        .reconcile_after_restart(UtcMicros::new(utc_now_micros()))
        && let Ok(mut latest) = focus_snapshot.lock()
    {
        *latest = snapshot;
    }
}

fn resequence_ui_event(
    event: EventEnvelope<AppEvent>,
    ui_event_sequence: &mut u64,
) -> EventEnvelope<AppEvent> {
    EventEnvelope::new(
        event.schema_revision(),
        next_ui_event_sequence(ui_event_sequence),
        event.causation_command_id(),
        event.committed_data_revision(),
        event.occurred_at_utc(),
        event.into_payload(),
    )
}

fn next_ui_event_sequence(ui_event_sequence: &mut u64) -> u64 {
    *ui_event_sequence = ui_event_sequence.saturating_add(1);
    *ui_event_sequence
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
    let schedules = read
        .schedules()
        .iter()
        .map(|record| record.snapshot().clone())
        .collect::<Vec<_>>();
    let catalog_applications = read
        .applications()
        .iter()
        .map(|record| (record.application().clone(), record.excluded()))
        .collect::<Vec<_>>();
    let categories = read
        .categories()
        .iter()
        .map(|record| record.category().clone())
        .collect::<Vec<_>>();
    let source = openmanic_application::TimelineProjectionSource::new(
        read.revision(),
        &activities,
        &applications,
    )
    .with_schedules(&schedules);
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
            schedules,
            applications: catalog_applications,
            categories,
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

fn recurring_schedule_rule(
    label: &str,
    range: HalfOpenInterval,
    weekday_mask: u8,
) -> Result<ScheduleRule, ScheduleDraftValidationError> {
    const DAY_US: i64 = 86_400_000_000;
    const SECOND_US: i64 = 1_000_000;
    if weekday_mask == 0 {
        return Err(ScheduleDraftValidationError::NoWeekdays);
    }
    if range.start().get().rem_euclid(SECOND_US) != 0
        || range.end().get().rem_euclid(SECOND_US) != 0
    {
        return Err(ScheduleDraftValidationError::WholeSecondTimes);
    }
    let start_second = u32::try_from(range.start().get().rem_euclid(DAY_US) / SECOND_US)
        .map_err(|_| ScheduleDraftValidationError::CannotRepresent)?;
    let end_second = u32::try_from(range.end().get().rem_euclid(DAY_US) / SECOND_US)
        .map_err(|_| ScheduleDraftValidationError::CannotRepresent)?;
    if start_second == end_second {
        return Err(ScheduleDraftValidationError::FullDayRecurrence);
    }
    let effective_start_date = i32::try_from(range.start().get().div_euclid(DAY_US))
        .map_err(|_| ScheduleDraftValidationError::CannotRepresent)?;
    ScheduleRule::repeating(
        label,
        None,
        weekday_mask,
        start_second,
        end_second,
        effective_start_date,
        None,
        "Etc/UTC",
    )
    .map_err(|_| ScheduleDraftValidationError::CannotRepresent)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScheduleDraftValidationError {
    NoWeekdays,
    WholeSecondTimes,
    FullDayRecurrence,
    QueueUnavailable,
    CannotRepresent,
}

impl ScheduleDraftValidationError {
    const fn message(self) -> &'static str {
        match self {
            Self::NoWeekdays => "Choose at least one day for a repeating schedule.",
            Self::WholeSecondTimes => {
                "Repeating schedules currently require start and end times on whole seconds."
            }
            Self::FullDayRecurrence => {
                "A repeating schedule cannot currently run for exactly 24 hours."
            }
            Self::QueueUnavailable => "The schedule could not be queued. Try saving it again.",
            Self::CannotRepresent => "This schedule cannot be saved with the current time settings.",
        }
    }
}

const fn tracking_status_label(status: &MutationStatus) -> &'static str {
    match status {
        MutationStatus::Pending => "Tracking request pending…",
        MutationStatus::Confirmed { .. } => "Tracking request confirmed.",
        MutationStatus::Rejected { .. } => "Tracking request was not saved.",
    }
}

const fn schedule_status_label(status: &MutationStatus) -> &'static str {
    match status {
        MutationStatus::Pending => "Saving schedule…",
        MutationStatus::Confirmed { .. } => "Schedule saved.",
        MutationStatus::Rejected { .. } => "Schedule was not saved. Check that it does not overlap another schedule.",
    }
}

const fn catalog_status_label(status: &MutationStatus) -> &'static str {
    match status {
        MutationStatus::Pending => "Saving category change…",
        MutationStatus::Confirmed { .. } => "Category change saved.",
        MutationStatus::Rejected { .. } => "Category change was not saved.",
    }
}

const fn focus_status_label(status: &MutationStatus) -> &'static str {
    match status {
        MutationStatus::Pending => "Focus request pending…",
        MutationStatus::Confirmed { .. } => "Focus request saved.",
        MutationStatus::Rejected { .. } => "Focus request was not accepted.",
    }
}

fn focus_remaining_us(state: FocusSessionState, now: UtcMicros) -> Option<i64> {
    match state {
        FocusSessionState::Running { deadline, .. } => {
            Some(deadline.get().saturating_sub(now.get()).max(0))
        }
        FocusSessionState::Paused { remaining_us, .. } => Some(remaining_us),
        FocusSessionState::Ready
        | FocusSessionState::Planned { .. }
        | FocusSessionState::Completed { .. }
        | FocusSessionState::Cancelled { .. } => None,
    }
}

fn focus_remaining_label(remaining_us: i64) -> String {
    let seconds = remaining_us.saturating_add(999_999) / 1_000_000;
    format!(
        "{minutes:02}:{seconds:02} remaining",
        minutes = seconds / 60,
        seconds = seconds % 60
    )
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
    create_schedule_mode: bool,
    schedule_draft: Option<ScheduleDraft>,
    schedule_delete_request: Option<ScheduleDeleteRequest>,
    latest_schedule_command: Option<CommandId>,
    latest_catalog_command: Option<CommandId>,
    new_category_name: String,
    selected_category_id: Option<CategoryId>,
    selected_category_applications: BTreeSet<ApplicationId>,
    application_search: String,
    application_filter_category: Option<Option<CategoryId>>,
    show_excluded_applications_only: bool,
    editing_category_id: Option<CategoryId>,
    category_rename_name: String,
    category_delete_confirmation: Option<CategoryId>,
    latest_focus_command: Option<CommandId>,
    latest_focus_session: Option<FocusSessionId>,
    #[cfg(windows)]
    application_icon_textures: BTreeMap<ApplicationId, eframe::egui::TextureHandle>,
    #[cfg(windows)]
    instance_owner: WindowsInstanceOwner,
}

#[derive(Clone)]
struct ScheduleDeleteRequest {
    snapshot: ScheduleSnapshot,
    anchor_date: Option<i32>,
    scope: ScheduleEditScope,
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
            create_schedule_mode: false,
            schedule_draft: None,
            schedule_delete_request: None,
            latest_schedule_command: None,
            latest_catalog_command: None,
            new_category_name: String::new(),
            selected_category_id: None,
            selected_category_applications: BTreeSet::new(),
            application_search: String::new(),
            application_filter_category: None,
            show_excluded_applications_only: false,
            editing_category_id: None,
            category_rename_name: String::new(),
            category_delete_confirmation: None,
            latest_focus_command: None,
            latest_focus_session: None,
            #[cfg(windows)]
            application_icon_textures: BTreeMap::new(),
            #[cfg(windows)]
            instance_owner: self.instance_owner,
        }
    }
}

impl VerticalSliceApp {
    #[cfg(windows)]
    fn render_application_icon(&mut self, ui: &mut eframe::egui::Ui, application_id: ApplicationId) {
        let Some(icon) = self.runtime.application_icon(application_id) else {
            self.application_icon_textures.remove(&application_id);
            ui.label("□");
            return;
        };
        let texture = self.application_icon_textures.entry(application_id).or_insert_with(|| {
            let image = eframe::egui::ColorImage::from_rgba_unmultiplied(
                [icon.width() as usize, icon.height() as usize],
                icon.rgba(),
            );
            ui.ctx().load_texture(
                format!("application-icon-{}", id_label(&application_id.as_bytes())),
                image,
                eframe::egui::TextureOptions::LINEAR,
            )
        });
        ui.image((texture.id(), eframe::egui::vec2(20.0, 20.0)));
    }

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
        self.render_focus_controls(ui);
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
        ui.horizontal(|ui| {
            ui.heading("Timeline");
            ui.toggle_value(&mut self.create_schedule_mode, "Create schedule");
            if self.create_schedule_mode {
                ui.label("Drag on the timeline to choose exact start and end times.");
            }
        });
        let output = self
            .timeline
            .show_snapshot(ui, snapshot.timeline(), &context, self.create_schedule_mode);
        for action in output.actions().iter().copied() {
            match action {
                TimelineRenderAction::Today(action) => {
                    self.app
                        .controller_mut()
                        .reduce_local(UiAction::Today(action));
                }
                TimelineRenderAction::ViewRangeChanged { range } => self.publish_projection(range),
                TimelineRenderAction::ScheduleRequested { range } => {
                    self.schedule_draft = Some(ScheduleDraft::new(range));
                }
                TimelineRenderAction::OpenCategories { application_id } => {
                    self.selected_category_applications.clear();
                    self.selected_category_applications.insert(application_id);
                    self.app
                        .controller_mut()
                        .reduce_local(UiAction::Navigate(openmanic_ui_egui::Route::Categories));
                }
            }
        }
        self.render_schedule_editor(ui);
        self.render_existing_schedule_controls(ui, &snapshot);
        ui.add_space(12.0);
        ui.heading("Application usage");
        render_usage_snapshot(ui, snapshot.usage());
        ui.add_space(12.0);
        ui.heading("Time distribution");
        render_distribution_snapshot(ui, snapshot.distribution());
    }

    fn render_categories_dashboard(&mut self, ui: &mut eframe::egui::Ui) {
        if self.app.controller().model().route() != openmanic_ui_egui::Route::Categories {
            return;
        }
        let data = self
            .app
            .controller()
            .model()
            .data()
            .visible_value()
            .cloned();
        let Some(snapshot) = data else {
            return;
        };
        ui.add_space(16.0);
        ui.heading("Categories");
        ui.horizontal(|ui| {
            ui.label("New category");
            ui.text_edit_singleline(&mut self.new_category_name);
            if ui
                .add_enabled(
                    !self.new_category_name.trim().is_empty(),
                    eframe::egui::Button::new("Create category"),
                )
                .clicked()
                && let Some(command) = self
                    .runtime
                    .identifiers
                    .catalog_create_category(&self.new_category_name)
                && let Some(command_id) = self.runtime.try_submit_catalog(command)
            {
                self.latest_catalog_command = Some(command_id);
                self.new_category_name.clear();
            }
        });
        if snapshot.categories().is_empty() {
            ui.label("No categories yet. Applications are currently Uncategorized.");
        } else {
            ui.label("Your categories");
            for category in snapshot.categories() {
                ui.horizontal(|ui| {
                    ui.label(category.name().as_str());
                    if ui.button("Rename").clicked() {
                        self.editing_category_id = Some(category.id());
                        self.category_rename_name = category.name().as_str().to_owned();
                        self.category_delete_confirmation = None;
                    }
                    if ui.button("Delete category").clicked() {
                        self.category_delete_confirmation = Some(category.id());
                        self.editing_category_id = None;
                    }
                });
            }
        }
        if let Some(category_id) = self.editing_category_id {
            if let Some(category) = snapshot
                .categories()
                .iter()
                .find(|category| category.id() == category_id)
            {
                let replacement = CategoryName::try_new(self.category_rename_name.trim()).ok();
                ui.horizontal(|ui| {
                    ui.label(format!("Rename {}", category.name().as_str()));
                    ui.text_edit_singleline(&mut self.category_rename_name);
                    if ui
                        .add_enabled(replacement.is_some(), eframe::egui::Button::new("Save name"))
                        .clicked()
                        && let Some(name) = replacement
                        && let Some(command_id) = self.runtime.try_submit_catalog(
                            self.runtime.identifiers.catalog_command(CatalogCommand::RenameCategory {
                                category_id,
                                name,
                                observed_at_utc: UtcMicros::new(utc_now_micros()),
                            }),
                        )
                    {
                        self.latest_catalog_command = Some(command_id);
                        self.editing_category_id = None;
                        self.category_rename_name.clear();
                    }
                    if ui.button("Cancel rename").clicked() {
                        self.editing_category_id = None;
                        self.category_rename_name.clear();
                    }
                });
            } else {
                self.editing_category_id = None;
                self.category_rename_name.clear();
            }
        }
        if let Some(category_id) = self.category_delete_confirmation {
            if let Some(category) = snapshot
                .categories()
                .iter()
                .find(|category| category.id() == category_id)
            {
                let assigned_count = snapshot
                    .applications()
                    .iter()
                    .filter(|(application, _)| application.category_id() == Some(category_id))
                    .count();
                ui.group(|ui| {
                    ui.label(format!(
                        "Delete {}? {assigned_count} assigned application(s) will become Uncategorized.",
                        category.name().as_str()
                    ));
                    ui.horizontal(|ui| {
                        if ui.button("Confirm category deletion").clicked()
                            && let Some(command_id) = self.runtime.try_submit_catalog(
                                self.runtime.identifiers.catalog_command(
                                    CatalogCommand::DeleteCategory { category_id },
                                ),
                            )
                        {
                            self.latest_catalog_command = Some(command_id);
                            if self.selected_category_id == Some(category_id) {
                                self.selected_category_id = None;
                            }
                            self.category_delete_confirmation = None;
                        }
                        if ui.button("Keep category").clicked() {
                            self.category_delete_confirmation = None;
                        }
                    });
                });
            } else {
                self.category_delete_confirmation = None;
            }
        }
        ui.add_space(8.0);
        ui.heading("Applications");
        ui.horizontal(|ui| {
            ui.label("Search");
            ui.text_edit_singleline(&mut self.application_search);
            ui.checkbox(
                &mut self.show_excluded_applications_only,
                "Excluded only",
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Show");
            if ui
                .selectable_label(self.application_filter_category.is_none(), "All")
                .clicked()
            {
                self.application_filter_category = None;
            }
            if ui
                .selectable_label(
                    self.application_filter_category == Some(None),
                    "Uncategorized",
                )
                .clicked()
            {
                self.application_filter_category = Some(None);
            }
            for category in snapshot.categories() {
                let filter = Some(Some(category.id()));
                if ui
                    .selectable_label(
                        self.application_filter_category == filter,
                        category.name().as_str(),
                    )
                    .clicked()
                {
                    self.application_filter_category = filter;
                }
            }
        });
        let selected_application_ids = self
            .selected_category_applications
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let has_selection = !selected_application_ids.is_empty();
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("{} selected", selected_application_ids.len()));
            if ui.button("Select all").clicked() {
                self.selected_category_applications = snapshot
                    .applications()
                    .iter()
                    .map(|(application, _)| application.id())
                    .collect();
            }
            if ui
                .add_enabled(has_selection, eframe::egui::Button::new("Clear selection"))
                .clicked()
            {
                self.selected_category_applications.clear();
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Assign selected to");
            if ui
                .selectable_label(self.selected_category_id.is_none(), "Uncategorized")
                .clicked()
            {
                self.selected_category_id = None;
            }
            for category in snapshot.categories() {
                if ui
                    .selectable_label(
                        self.selected_category_id == Some(category.id()),
                        category.name().as_str(),
                    )
                    .clicked()
                {
                    self.selected_category_id = Some(category.id());
                }
            }
        });
        let assignment_label = self
            .selected_category_id
            .and_then(|category_id| {
                snapshot
                    .categories()
                    .iter()
                    .find(|category| category.id() == category_id)
            })
            .map_or_else(
                || "Remove selected from category".to_owned(),
                |category| format!("Assign selected to {}", category.name().as_str()),
            );
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    has_selection,
                    eframe::egui::Button::new(assignment_label),
                )
                .clicked()
                && let Ok(payload) = CatalogCommand::try_assign_applications(
                    selected_application_ids.iter().copied(),
                    self.selected_category_id,
                )
                && let Some(command_id) = self
                    .runtime
                    .try_submit_catalog(self.runtime.identifiers.catalog_command(payload))
            {
                self.latest_catalog_command = Some(command_id);
            }
            if ui
                .add_enabled(
                    has_selection,
                    eframe::egui::Button::new("Exclude selected from future tracking"),
                )
                .clicked()
                && let Ok(payload) = CatalogCommand::try_set_applications_excluded(
                    selected_application_ids.iter().copied(),
                    true,
                )
                && let Some(command_id) = self
                    .runtime
                    .try_submit_catalog(self.runtime.identifiers.catalog_command(payload))
            {
                self.latest_catalog_command = Some(command_id);
            }
            if ui
                .add_enabled(
                    has_selection,
                    eframe::egui::Button::new("Include selected in future tracking"),
                )
                .clicked()
                && let Ok(payload) = CatalogCommand::try_set_applications_excluded(
                    selected_application_ids.iter().copied(),
                    false,
                )
                && let Some(command_id) = self
                    .runtime
                    .try_submit_catalog(self.runtime.identifiers.catalog_command(payload))
            {
                self.latest_catalog_command = Some(command_id);
            }
        });
        for (application, excluded) in snapshot.applications() {
            let search_matches = self.application_search.trim().is_empty()
                || application
                    .name()
                    .as_str()
                    .to_lowercase()
                    .contains(&self.application_search.trim().to_lowercase());
            let category_matches = self
                .application_filter_category
                .is_none_or(|category_id| application.category_id() == category_id);
            if !search_matches || !category_matches || (self.show_excluded_applications_only && !excluded)
            {
                continue;
            }
            let category_name = application
                .category_id()
                .and_then(|category_id| {
                    snapshot
                        .categories()
                        .iter()
                        .find(|category| category.id() == category_id)
                })
                .map_or("Uncategorized", |category| category.name().as_str());
            ui.horizontal(|ui| {
                let application_id = application.id();
                #[cfg(windows)]
                self.render_application_icon(ui, application_id);
                #[cfg(not(windows))]
                ui.label("□");
                let mut selected = self
                    .selected_category_applications
                    .contains(&application_id);
                if ui.checkbox(&mut selected, application.name().as_str()).changed() {
                    if selected {
                        self.selected_category_applications.insert(application_id);
                    } else {
                        self.selected_category_applications.remove(&application_id);
                    }
                }
                ui.label(format!("Category: {category_name}"));
                if *excluded {
                    ui.label("Excluded from future tracking");
                }
                let action_label = if *excluded {
                    "Include future tracking"
                } else {
                    "Exclude future tracking"
                };
                if ui.button(action_label).clicked()
                    && let Ok(payload) = CatalogCommand::try_set_applications_excluded(
                        [application.id()],
                        !*excluded,
                    )
                    && let Some(command_id) = self
                        .runtime
                        .try_submit_catalog(self.runtime.identifiers.catalog_command(payload))
                {
                    self.latest_catalog_command = Some(command_id);
                }
            });
        }
        if let Some(command_id) = self.latest_catalog_command
            && let Some(status) = self.app.controller().model().mutation_status(command_id)
        {
            ui.label(catalog_status_label(status));
        }
    }

    fn queue_tracking_control(&mut self, action: TrackingControlAction) {
        let request = self.runtime.identifiers.tracking_request(action);
        let _ = self
            .today
            .queue_tracking(self.app.controller_mut(), request);
    }

    fn render_focus_controls(&mut self, ui: &mut eframe::egui::Ui) {
        let snapshot = self
            .runtime
            .focus_snapshot
            .lock()
            .ok()
            .and_then(|latest| latest.clone());
        if let Some(snapshot) = snapshot.as_ref() {
            self.latest_focus_session = Some(snapshot.session_id());
        }

        ui.vertical(|ui| {
            match snapshot.as_ref().map(|snapshot| snapshot.session().state()) {
                Some(FocusSessionState::Ready | FocusSessionState::Planned { .. }) => {
                    ui.label("Focus is ready to start.");
                    if let Some(snapshot) = snapshot.as_ref()
                        && ui.button("Start focus").clicked()
                        && let Some(command_id) = self.runtime.try_submit_focus(
                            self.runtime.identifiers.focus_start(snapshot.session_id()),
                        )
                    {
                        self.latest_focus_command = Some(command_id);
                    }
                }
                Some(state @ FocusSessionState::Running { .. }) => {
                    if let Some(remaining_us) =
                        focus_remaining_us(state, UtcMicros::new(utc_now_micros()))
                    {
                        ui.label(format!("Focus: {}", focus_remaining_label(remaining_us)));
                        ui.ctx().request_repaint_after(Duration::from_secs(1));
                    }
                    if let Some(snapshot) = snapshot.as_ref() {
                        self.render_focus_lifecycle_controls(
                            ui,
                            snapshot.session_id(),
                            &[
                                ("Pause", FocusLifecycleAction::Pause),
                                ("Complete", FocusLifecycleAction::Complete),
                                ("Cancel", FocusLifecycleAction::Cancel),
                            ],
                        );
                    }
                }
                Some(state @ FocusSessionState::Paused { .. }) => {
                    if let Some(remaining_us) =
                        focus_remaining_us(state, UtcMicros::new(utc_now_micros()))
                    {
                        ui.label(format!(
                            "Focus paused: {}",
                            focus_remaining_label(remaining_us)
                        ));
                    }
                    if let Some(snapshot) = snapshot.as_ref() {
                        self.render_focus_lifecycle_controls(
                            ui,
                            snapshot.session_id(),
                            &[
                                ("Resume", FocusLifecycleAction::Resume),
                                ("Complete", FocusLifecycleAction::Complete),
                                ("Cancel", FocusLifecycleAction::Cancel),
                            ],
                        );
                    }
                }
                Some(FocusSessionState::Completed { .. }) => {
                    ui.label("Focus completed.");
                }
                Some(FocusSessionState::Cancelled { .. }) => {
                    ui.label("Focus cancelled.");
                }
                None => {
                    ui.label("No focus session is prepared.");
                }
            }
            let may_prepare = !matches!(
                snapshot.as_ref().map(|snapshot| snapshot.session().state()),
                Some(
                    FocusSessionState::Ready
                        | FocusSessionState::Planned { .. }
                        | FocusSessionState::Running { .. }
                        | FocusSessionState::Paused { .. }
                )
            );
            if may_prepare
                && ui.button("Prepare 25-minute focus").clicked()
                && let Some((session_id, command)) = self.runtime.identifiers.focus_draft()
                && let Some(command_id) = self.runtime.try_submit_focus(command)
            {
                self.latest_focus_session = Some(session_id);
                self.latest_focus_command = Some(command_id);
            }
            if let Some(command_id) = self.latest_focus_command
                && let Some(status) = self.app.controller().model().mutation_status(command_id)
            {
                ui.label(focus_status_label(status));
            }
            if self
                .runtime
                .focus_notification_error
                .lock()
                .ok()
                .and_then(|value| *value)
                .is_some()
            {
                ui.label("Focus completed, but the notification was unavailable.");
            }
        });
    }

    fn submit_focus_lifecycle(&mut self, session_id: FocusSessionId, action: FocusLifecycleAction) {
        let command = self.runtime.identifiers.focus_lifecycle(session_id, action);
        if let Some(command_id) = self.runtime.try_submit_focus(command) {
            self.latest_focus_command = Some(command_id);
        }
    }

    fn render_focus_lifecycle_controls(
        &mut self,
        ui: &mut eframe::egui::Ui,
        session_id: FocusSessionId,
        controls: &[(&str, FocusLifecycleAction)],
    ) {
        ui.horizontal(|ui| {
            for &(label, action) in controls {
                if ui.button(label).clicked() {
                    self.submit_focus_lifecycle(session_id, action);
                }
            }
        });
    }

    fn render_schedule_editor(&mut self, ui: &mut eframe::egui::Ui) {
        let schedule_draft_action = self.schedule_draft.as_mut().map(|draft| {
            let action = render_schedule_draft(ui, draft);
            (
                action,
                draft.range,
                draft.label.clone(),
                draft.repeats,
                draft.weekday_mask,
                draft.existing.clone(),
            )
        });
        let Some((action, range, label, repeats, weekday_mask, existing)) = schedule_draft_action else {
            self.render_schedule_status(ui);
            return;
        };
        let submission = match action {
            ScheduleDraftAction::Save => Some(if let Some(recurring) = self.schedule_draft.as_ref().and_then(|draft| draft.recurring.clone()) {
                self.queue_recurring_schedule_edit(recurring, range, &label, weekday_mask)
            } else if let Some(existing) = existing {
                self.queue_schedule_replacement(existing, range, &label, repeats, weekday_mask)
            } else {
                self.queue_schedule_draft(range, &label, repeats, weekday_mask)
            }),
            ScheduleDraftAction::Cancel | ScheduleDraftAction::None => None,
        };
        if let Some(Ok(command_id)) = submission {
            self.latest_schedule_command = Some(command_id);
        }
        if let Some(Err(error)) = submission
            && let Some(draft) = self.schedule_draft.as_mut()
        {
            draft.validation_error = Some(error);
        }
        if matches!(action, ScheduleDraftAction::Cancel)
            || matches!(action, ScheduleDraftAction::Save) && matches!(submission, Some(Ok(_)))
        {
            self.schedule_draft = None;
        }
        self.render_schedule_status(ui);
    }

    fn render_existing_schedule_controls(&mut self, ui: &mut eframe::egui::Ui, today: &TodaySnapshot) {
        let entries = today
            .timeline()
            .schedule_occurrences()
            .iter()
            .filter_map(|occurrence| {
                let (schedule_id, anchor_date) = match occurrence.id() {
                    ScheduleOccurrenceId::OneTime(schedule_id) => (schedule_id, None),
                    ScheduleOccurrenceId::Recurring {
                        schedule_id,
                        anchor_date,
                    } => (schedule_id, Some(anchor_date)),
                };
                today
                    .schedules()
                    .iter()
                    .find(|snapshot| snapshot.id() == schedule_id)
                    .cloned()
                    .map(|snapshot| (snapshot, anchor_date, occurrence.interval()))
            })
            .collect::<Vec<_>>();
        if !entries.is_empty() {
            ui.add_space(12.0);
            ui.heading("Schedules in this view");
            for (schedule, anchor_date, interval) in entries {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{} — {} to {} UTC",
                        schedule.rule().label(),
                        interval.start().get(),
                        interval.end().get()
                    ));
                    if ui.button("Delete…").clicked() {
                        self.schedule_delete_request = Some(ScheduleDeleteRequest {
                            snapshot: schedule.clone(),
                            anchor_date,
                            scope: ScheduleEditScope::OnlyThisDate,
                        });
                    }
                    if !schedule.rule().is_repeating()
                        && ui.button("Edit…").clicked()
                    {
                        self.schedule_draft = ScheduleDraft::from_existing(schedule.clone());
                    }
                    if schedule.rule().is_repeating()
                        && let (ScheduleId::Series(series_id), Some(anchor_date)) = (schedule.id(), anchor_date)
                        && ui.button("Edit…").clicked()
                    {
                        self.schedule_draft = Some(ScheduleDraft::from_recurring(
                            schedule, series_id, anchor_date, interval,
                        ));
                    }
                });
            }
        }
        self.render_schedule_delete_confirmation(ui);
    }

    fn render_schedule_delete_confirmation(&mut self, ui: &mut eframe::egui::Ui) {
        let Some(mut request) = self.schedule_delete_request.clone() else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        ui.group(|ui| {
            ui.strong("Delete schedule?");
            ui.label("This action changes only your personal schedule and never activity history.");
            if request.anchor_date.is_some() {
                ui.radio_value(
                    &mut request.scope,
                    ScheduleEditScope::OnlyThisDate,
                    "Only this occurrence",
                );
                ui.radio_value(
                    &mut request.scope,
                    ScheduleEditScope::ThisAndFuture,
                    "This and future occurrences",
                );
                ui.radio_value(
                    &mut request.scope,
                    ScheduleEditScope::EveryOccurrence,
                    "Every occurrence",
                );
            }
            confirm = ui.button("Confirm delete").clicked();
            cancel = ui.button("Keep schedule").clicked();
        });
        if cancel {
            self.schedule_delete_request = None;
            return;
        }
        if !confirm {
            self.schedule_delete_request = Some(request);
            return;
        }
        let command = match (request.snapshot.id(), request.anchor_date) {
            (ScheduleId::Series(series_id), Some(anchor_date)) => self
                .runtime
                .identifiers
                .schedule_delete_occurrence(
                    series_id,
                    anchor_date,
                    request.scope,
                    request.snapshot.entity_revision(),
                ),
            _ => self.runtime.identifiers.schedule_delete(&request.snapshot),
        };
        if let Some(command_id) = self.runtime.try_submit_schedule(command) {
            self.latest_schedule_command = Some(command_id);
            self.schedule_delete_request = None;
        } else {
            self.schedule_delete_request = Some(request);
        }
    }

    fn render_schedule_status(&self, ui: &mut eframe::egui::Ui) {
        if let Some(command_id) = self.latest_schedule_command
            && let Some(status) = self.app.controller().model().mutation_status(command_id)
        {
            ui.label(schedule_status_label(status));
        }
    }

    fn queue_schedule_draft(
        &self,
        range: HalfOpenInterval,
        label: &str,
        repeats: bool,
        weekday_mask: u8,
    ) -> Result<CommandId, ScheduleDraftValidationError> {
        let command = self
            .runtime
            .identifiers
            .schedule_create(range, label, repeats, weekday_mask)?;
        self.runtime
            .try_submit_schedule(command)
            .ok_or(ScheduleDraftValidationError::QueueUnavailable)
    }

    fn queue_schedule_replacement(
        &self,
        existing: ScheduleSnapshot,
        range: HalfOpenInterval,
        label: &str,
        repeats: bool,
        _weekday_mask: u8,
    ) -> Result<CommandId, ScheduleDraftValidationError> {
        if repeats || existing.rule().is_repeating() {
            return Err(ScheduleDraftValidationError::CannotRepresent);
        }
        let rule = ScheduleRule::one_time(label, None, range, "Etc/UTC")
            .map_err(|_| ScheduleDraftValidationError::CannotRepresent)?;
        let replacement = ScheduleSnapshot::try_new(
            existing.id(),
            rule,
            existing.entity_revision(),
            UtcMicros::new(utc_now_micros()),
        )
        .map_err(|_| ScheduleDraftValidationError::CannotRepresent)?;
        self.runtime
            .try_submit_schedule(self.runtime.identifiers.schedule_replace(replacement))
            .ok_or(ScheduleDraftValidationError::QueueUnavailable)
    }

    fn queue_recurring_schedule_edit(
        &self,
        recurring: RecurringScheduleEditRequest,
        range: HalfOpenInterval,
        label: &str,
        weekday_mask: u8,
    ) -> Result<CommandId, ScheduleDraftValidationError> {
        let edit = match recurring.scope {
            ScheduleEditScope::OnlyThisDate => RecurringScheduleEdit::OnlyThisDate(
                RecurringOccurrenceOverride { interval: range, start_after_gap: false, start_earlier_fold: false, end_after_gap: false, end_earlier_fold: false },
            ),
            ScheduleEditScope::ThisAndFuture | ScheduleEditScope::EveryOccurrence => {
                const DAY_US: i64 = 86_400_000_000;
                const SECOND_US: i64 = 1_000_000;
                let start_second = u32::try_from(range.start().get().rem_euclid(DAY_US) / SECOND_US)
                    .map_err(|_| ScheduleDraftValidationError::CannotRepresent)?;
                let end_second = u32::try_from(range.end().get().rem_euclid(DAY_US) / SECOND_US)
                    .map_err(|_| ScheduleDraftValidationError::CannotRepresent)?;
                RecurringScheduleEdit::Rule(RecurringScheduleRuleChange {
                    label: label.to_owned(), category_id: None, weekday_mask,
                    start_second_of_day: start_second, end_second_of_day: end_second,
                    time_zone_id: "Etc/UTC".to_owned(),
                })
            }
        };
        self.runtime.try_submit_schedule(self.runtime.identifiers.schedule_edit_occurrence(
            recurring.series_id, recurring.anchor_date, recurring.scope, recurring.expected_revision, edit,
        )).ok_or(ScheduleDraftValidationError::QueueUnavailable)
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScheduleDraftAction {
    None,
    Save,
    Cancel,
}

#[derive(Debug)]
struct ScheduleDraft {
    range: HalfOpenInterval,
    label: String,
    repeats: bool,
    weekday_mask: u8,
    validation_error: Option<ScheduleDraftValidationError>,
    existing: Option<ScheduleSnapshot>,
    recurring: Option<RecurringScheduleEditRequest>,
}

impl ScheduleDraft {
    fn new(range: HalfOpenInterval) -> Self {
        Self {
            range,
            label: "New schedule".to_owned(),
            repeats: false,
            weekday_mask: 0b0111_1111,
            validation_error: None,
            existing: None,
            recurring: None,
        }
    }

    fn from_existing(snapshot: ScheduleSnapshot) -> Option<Self> {
        let range = snapshot.rule().one_time_interval()?;
        Some(Self {
            range,
            label: snapshot.rule().label().to_owned(),
            repeats: false,
            weekday_mask: 0b0111_1111,
            validation_error: None,
            existing: Some(snapshot),
            recurring: None,
        })
    }

    fn from_recurring(snapshot: ScheduleSnapshot, series_id: ScheduleSeriesId, anchor_date: i32, range: HalfOpenInterval) -> Self {
        Self { range, label: snapshot.rule().label().to_owned(), repeats: true, weekday_mask: snapshot.rule().segments().first().map_or(0b0111_1111, |segment| segment.weekday_mask()), validation_error: None, existing: None, recurring: Some(RecurringScheduleEditRequest { series_id, anchor_date, scope: ScheduleEditScope::OnlyThisDate, expected_revision: snapshot.entity_revision() }) }
    }
}

#[derive(Clone, Debug)]
struct RecurringScheduleEditRequest { series_id: ScheduleSeriesId, anchor_date: i32, scope: ScheduleEditScope, expected_revision: EntityRevision }

fn render_schedule_draft(
    ui: &mut eframe::egui::Ui,
    draft: &mut ScheduleDraft,
) -> ScheduleDraftAction {
    ui.group(|ui| {
        ui.strong(if draft.existing.is_some() { "Edit schedule" } else { "New schedule" });
        ui.label(format!(
            "Provisional range: {} to {} UTC",
            draft.range.start().get(),
            draft.range.end().get()
        ));
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut draft.label);
        });
        ui.checkbox(&mut draft.repeats, "Repeat");
        if draft.repeats {
            render_weekday_selector(ui, &mut draft.weekday_mask);
            ui.label("Choose at least one day. Recurring times use UTC until local-zone editing is added.");
        }
        if let Some(recurring) = draft.recurring.as_mut() {
            ui.radio_value(&mut recurring.scope, ScheduleEditScope::OnlyThisDate, "Only this occurrence");
            ui.radio_value(&mut recurring.scope, ScheduleEditScope::ThisAndFuture, "This and future occurrences");
            ui.radio_value(&mut recurring.scope, ScheduleEditScope::EveryOccurrence, "Every occurrence");
        }
        if let Some(error) = draft.validation_error {
            ui.colored_label(eframe::egui::Color32::from_rgb(200, 70, 70), error.message());
        }
        if ui
            .add_enabled(!draft.label.trim().is_empty(), eframe::egui::Button::new("Save schedule"))
            .clicked()
        {
            ScheduleDraftAction::Save
        } else if ui.button("Cancel").clicked() {
            ScheduleDraftAction::Cancel
        } else {
            ScheduleDraftAction::None
        }
    })
    .inner
}

fn render_weekday_selector(ui: &mut eframe::egui::Ui, weekday_mask: &mut u8) {
    ui.horizontal(|ui| {
        for (index, label) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
            .iter()
            .enumerate()
        {
            let bit = 1_u8 << index;
            if ui.selectable_label(*weekday_mask & bit != 0, *label).clicked() {
                *weekday_mask ^= bit;
            }
        }
    });
}

impl eframe::App for VerticalSliceApp {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut eframe::Frame) {
        // The bootstrap data-root lock, instance owner, worker handles, and supervisor remain
        // process-owned while the viewport is hidden or stalled.
        let _ = (&self.bootstrap, &self.runtime.supervisor, &*frame);
        self.drain_worker_ingress();
        eframe::App::ui(&mut self.app, ui, frame);
        self.render_today_dashboard(ui);
        self.render_categories_dashboard(ui);
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
        CatalogCommand, EntityRevision, LaneCapacities, LaneReceive, ScheduleCommand,
        ScheduleId, ScheduleSnapshot, TrackingCommand, TrackingEvidence, WorkLane,
        bounded_runtime_lanes,
    };
    use openmanic_domain::{
        ApplicationId, FocusSessionState, HalfOpenInterval, ScheduleEditScope, ScheduleRule,
        ScheduleSeriesId, UtcMicros,
    };
    use openmanic_platform::WindowsPlatformAction;

    use super::{
        CommandIdentifiers, PlatformActionRouter, UiInbox, UiIngress, day_range,
        focus_remaining_label, focus_remaining_us, recurring_schedule_rule, store_identity,
        ScheduleDraftValidationError,
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
    fn focus_remaining_uses_the_authoritative_deadline_or_frozen_duration() {
        let running = FocusSessionState::Running {
            started_at: UtcMicros::new(100),
            deadline: UtcMicros::new(1_500_100),
        };
        assert_eq!(
            focus_remaining_us(running, UtcMicros::new(500_100)),
            Some(1_000_000)
        );
        assert_eq!(
            focus_remaining_us(running, UtcMicros::new(1_600_100)),
            Some(0)
        );
        assert_eq!(
            focus_remaining_us(
                FocusSessionState::Paused {
                    started_at: UtcMicros::new(100),
                    remaining_us: 61_000_001,
                },
                UtcMicros::new(999_999_999),
            ),
            Some(61_000_001)
        );
        assert_eq!(focus_remaining_label(61_000_001), "01:02 remaining");
        assert_eq!(
            focus_remaining_us(FocusSessionState::Ready, UtcMicros::new(0)),
            None
        );
    }

    #[test]
    fn schedule_delete_commands_retain_the_snapshot_revision_and_explicit_scope() {
        let identifiers = CommandIdentifiers::default();
        let series_id = ScheduleSeriesId::from_bytes([7; 16]);
        let snapshot = ScheduleSnapshot::try_new(
            ScheduleId::Series(series_id),
            ScheduleRule::repeating(
                "Review",
                None,
                1,
                9 * 3_600,
                10 * 3_600,
                0,
                None,
                "Etc/UTC",
            )
            .expect("valid recurring fixture"),
            EntityRevision::new(4),
            UtcMicros::new(10),
        )
        .expect("matching schedule identity");
        let command = identifiers.schedule_delete_occurrence(
            series_id,
            20,
            ScheduleEditScope::ThisAndFuture,
            snapshot.entity_revision(),
        );

        assert_eq!(command.expected_entity_revision(), Some(EntityRevision::new(4)));
        assert!(matches!(
            command.payload(),
            ScheduleCommand::DeleteOccurrence {
                series_id: actual_series_id,
                anchor_date: 20,
                scope: ScheduleEditScope::ThisAndFuture,
            } if *actual_series_id == series_id
        ));
    }

    #[test]
    fn category_creation_command_uses_a_validated_name_and_correlated_timestamp() {
        let identifiers = CommandIdentifiers::default();
        let command = identifiers
            .catalog_create_category("Deep work")
            .expect("valid category name produces a command");
        assert!(matches!(
            command.payload(),
            CatalogCommand::CreateCategory {
                category,
                observed_at_utc,
            } if category.name().as_str() == "Deep work" && *observed_at_utc == command.submitted_at_utc()
        ));
        assert!(identifiers.catalog_create_category("   ").is_none());
    }

    #[test]
    fn recurring_editor_conversion_retains_overnight_duration_and_rejects_empty_days() {
        let range = HalfOpenInterval::try_new(
            UtcMicros::new(23 * 3_600_000_000),
            UtcMicros::new(25 * 3_600_000_000),
        )
        .expect("positive overnight fixture");
        let rule = recurring_schedule_rule("Night review", range, 0b0100_0001)
            .expect("valid recurring conversion");
        let segment = rule.segments().pop().expect("one recurring segment");
        assert_eq!(segment.weekday_mask(), 0b0100_0001);
        assert_eq!(segment.start_second_of_day(), 23 * 3_600);
        assert_eq!(segment.end_second_of_day(), 3_600);
        assert_eq!(
            recurring_schedule_rule("No days", range, 0),
            Err(ScheduleDraftValidationError::NoWeekdays)
        );
        let fractional_range = HalfOpenInterval::try_new(
            UtcMicros::new(1),
            UtcMicros::new(3_600_000_001),
        )
        .expect("positive fractional fixture");
        assert_eq!(
            recurring_schedule_rule("Fractional", fractional_range, 0b0000_0001),
            Err(ScheduleDraftValidationError::WholeSecondTimes)
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
