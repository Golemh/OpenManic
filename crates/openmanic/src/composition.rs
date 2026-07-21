//! Primary process composition for the first Windows tracking vertical slice.
//!
//! The composition root owns process lifetime and wires accepted boundaries together. SQLite,
//! tracking reduction, projection work, and Windows callbacks never execute in an egui frame.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    error::Error,
    fmt, fs,
    io::Write,
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
    AppEvent, ApplicationError, ApplicationIconCache, ApplicationIconCacheLimits,
    ApplicationIconLookup, ApplicationIconResult, ApplicationPort, CalendarBlockId,
    CalendarDayContext, CalendarDayProjector, CalendarDaySnapshot, CalendarProjectionSource,
    CalendarSourceFocus, CancellationRequest, CancellationSource, CancellationToken,
    CatalogCommand, CatalogPersistence, CatalogPersistenceError, CatalogService, CommandEnvelope,
    CommandId, CommandReceipt, CsvExportRequest, CsvImportRequest, DataOperationDestination,
    DataOperationOutcome, DataRevision, EntityRevision, EventEnvelope, FocusCommand, FocusKind,
    FocusNotificationError, FocusNotificationPort, FocusPersistence, FocusPersistenceError,
    FocusService, FocusSnapshot, ImportBatchId, ImportDestinationScope, ImportScopeOutcome, JobId,
    JobState, LaneCapacities, LaneReceive, LaneSubmit, LatestMailbox, LatestMailboxReceiver,
    LayoutPersistence, LayoutSnapshot, MAX_FOREGROUND_SWITCH_DELAY_SECONDS,
    MIN_FOREGROUND_SWITCH_DELAY_SECONDS, MailboxReceive, MutationOutcome, OrderingKey,
    PortFailureReason, ProjectionContextKey, ProjectionRequest, ProjectionSlot,
    RecurringOccurrenceOverride, RecurringScheduleEdit, RecurringScheduleRuleChange,
    RuntimeLaneReceiver, RuntimeLanes, RuntimeSupervisor, RuntimeWorker, ScheduleCommand,
    ScheduleId, ScheduleOccurrenceId, SchedulePersistence, SchedulePersistenceError,
    ScheduleService, ScheduleSnapshot, SchemaRevision, SettingsPersistence, SettingsSnapshot,
    ShutdownCoordinator, ShutdownPhase, ShutdownStep, SnapshotEnvelope, ThreadRoot,
    TimelineApplication, TimelineContext, TimelineProjector, TimelineRawIntervalId,
    TimelineSnapshot, TimelineSourceActivity, TitleDisclosure, TitleObservationResult,
    TitleStabilizer, TrackingCheckpoint, TrackingCommand, TrackingEvidence,
    TrackingPersistenceIntent, TrackingPersistencePort, TrackingPersistenceSubmit, TrackingService,
    WorkLane, bounded_runtime_lanes, latest_mailbox,
};
use openmanic_domain::{
    ActivityCause, ActivityEvidence, ActivityInterval, ActivityState, Application, ApplicationId,
    ApplicationName, Category, CategoryId, CategoryName, FocusSessionId, FocusSessionState,
    HalfOpenInterval, LayoutDocument, LayoutHeight, OneTimeScheduleId, ScheduleEditScope,
    ScheduleRule, ScheduleSeriesId, TrackerRunId, UtcMicros,
};
#[cfg(windows)]
use openmanic_platform::{
    ActivationCommandDecode, InstanceAcquisition, LocalActivationCommand, WindowsActivationServer,
    WindowsApplicationMetadataRequest, WindowsControlAdapter, WindowsInstanceOwner,
    WindowsWindowTitleObservationRequest,
};
use openmanic_platform::{
    EvidencePublishResult, TrackingEvidenceSink, WindowsPlatformAction, WindowsTrayController,
};
use openmanic_storage_sqlite::{
    RecoveryOutcome, SqliteStore, StorageWriter, StoreOpenOptions, TrackerRunRegistration,
};
use openmanic_ui_egui::timeline::{TimelineRenderAction, TimelineRenderer};
use openmanic_ui_egui::{
    ApplicationUsage, ApplicationUsageSnapshot, CalendarAction, CalendarBlockKind,
    CalendarController, CalendarDataState, CalendarEffect, CommandDispatcher,
    DestructiveConfirmation, InboundMessage, JobDescriptor, JobPresentationState, JobsAction,
    JobsController, JobsEffect, LayoutEditAction, LayoutEditEffect, LayoutEditor, MutationStatus,
    OpenManicApp, PresentableData, SettingsAction, SettingsController, SettingsEffect,
    ShutdownController, ShutdownEffect, TodayAction, TodayController, TodayTrackingRequest,
    TodayWidgetKind, TodayWidgetResolution, TrackingControlAction, UiAction, UiController, UiModel,
    reflow_dashboard, render_distribution_snapshot, render_shutdown_failure, render_usage_snapshot,
};

use crate::bootstrap::{BootstrapDisposition, BootstrapError, BootstrapState, bootstrap};
use crate::cli::{CliError, parse_process_cli};
use crate::{
    data_root::{
        BootstrapLocator, DataRootValidator, LocalDataRootValidator, LocatorError,
        RejectKnownNetworkShares, load_locator, persist_locator,
    },
    diagnostics::export_diagnostics_bundle,
};

const UI_INBOUND_CAPACITY: usize = 64;
const UI_OUTBOUND_CAPACITY: usize = 32;
const UI_EVENT_CAPACITY: usize = 64;
const WRITER_CRITICAL_CAPACITY: usize = 64;
const WRITER_NORMAL_CAPACITY: usize = 64;
const WRITER_OPTIONAL_CAPACITY: usize = 16;
const DATA_OPERATION_REQUEST_CAPACITY: usize = 8;
const DATA_OPERATION_RESULT_CAPACITY: usize = 32;
const UI_POLL_INTERVAL: Duration = Duration::from_millis(250);
const WORKER_IDLE_INTERVAL: Duration = Duration::from_millis(10);
const PLATFORM_PUMP_INTERVAL: Duration = Duration::from_millis(50);
#[cfg(windows)]
const APPLICATION_METADATA_REQUEST_CAPACITY: usize = 32;
#[cfg(windows)]
const WINDOW_TITLE_OBSERVATION_CAPACITY: usize = 32;
#[cfg(windows)]
const APPLICATION_ICON_CACHE_ENTRIES: usize = 128;
#[cfg(windows)]
const APPLICATION_ICON_CACHE_BYTES: usize = 16 * 1024 * 1024;

const INITIAL_CATEGORY_NAMES: [&str; 8] = [
    "Development",
    "Communication",
    "Design",
    "Entertainment",
    "Web Browsing",
    "AI Assistants",
    "Productivity",
    "Security & Utilities",
];

/// Startup failures reported without exposing paths, titles, or raw platform errors.
#[derive(Debug)]
pub enum CompositionError {
    /// Bootstrap input was malformed.
    Cli(CliError),
    /// Data-root bootstrap did not finish successfully.
    Bootstrap(BootstrapError),
    /// The small per-user data-root locator could not be loaded safely.
    Locator(LocatorError),
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
            Self::Locator(error) => error.code(),
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
    CatalogMetadata {
        application: Application,
    },
    System(CommandEnvelope<TrackingCommand>),
    Ui(CommandEnvelope<TrackingCommand>),
    Catalog(CommandEnvelope<CatalogCommand>),
    Schedule(CommandEnvelope<ScheduleCommand>),
    Focus(CommandEnvelope<FocusCommand>),
    Layout {
        document: LayoutDocument,
        expected_revision: Option<EntityRevision>,
        observed_at_utc: UtcMicros,
    },
    Theme {
        theme_mode: u8,
        observed_at_utc: UtcMicros,
    },
    Settings {
        snapshot: SettingsSnapshot,
        expected_revision: Option<EntityRevision>,
    },
    WindowTitle(WindowTitleObservation),
}

impl WriterWork {
    fn into_parts(self) -> (CommandEnvelope<TrackingCommand>, bool) {
        match self {
            Self::System(command) | Self::CatalogForeground { command, .. } => (command, false),
            Self::Ui(command) => (command, true),
            Self::Catalog(_)
            | Self::CatalogMetadata { .. }
            | Self::Schedule(_)
            | Self::Focus(_)
            | Self::Layout { .. }
            | Self::Theme { .. }
            | Self::Settings { .. }
            | Self::WindowTitle(_) => {
                unreachable!("typed commands use their own application service")
            }
        }
    }
}

/// Private platform title data awaiting the serialized privacy gate. Its debug output redacts the
/// title so an accidental writer-lane diagnostic cannot disclose personal content.
struct WindowTitleObservation {
    application_id: ApplicationId,
    observed_at_utc: UtcMicros,
    title: String,
}

impl fmt::Debug for WindowTitleObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowTitleObservation")
            .field("application_id", &self.application_id)
            .field("observed_at_utc", &self.observed_at_utc)
            .field("title", &"<redacted>")
            .finish()
    }
}

/// The stateful application services which are exclusively owned by the writer worker.
struct WriterServices {
    tracking: TrackingService<WriterPersistence>,
    foreground_switch_stabilizer: ForegroundSwitchStabilizer,
    catalog: CatalogService<WriterPersistence>,
    schedules: ScheduleService<WriterPersistence>,
    focus: FocusService<WriterPersistence, InAppFocusNotifications>,
    title_stabilizer: TitleStabilizer,
    ui_event_sequence: u64,
}

#[derive(Debug)]
struct PendingForegroundSwitch {
    application: Application,
    command: CommandEnvelope<TrackingCommand>,
    observed_at_utc: UtcMicros,
}

#[derive(Debug)]
struct ForegroundSwitchStabilizer {
    accepted_application_id: Option<ApplicationId>,
    delay_seconds: u32,
    pending: Option<PendingForegroundSwitch>,
}

enum ForegroundSwitchDecision {
    Immediate {
        application: Application,
        command: CommandEnvelope<TrackingCommand>,
    },
    Pending,
}

impl ForegroundSwitchStabilizer {
    fn new(delay_seconds: u32) -> Self {
        Self {
            accepted_application_id: None,
            delay_seconds,
            pending: None,
        }
    }

    fn observe(
        &mut self,
        application: Application,
        command: CommandEnvelope<TrackingCommand>,
    ) -> ForegroundSwitchDecision {
        let application_id = application.id();
        let Some(observed_at_utc) = foreground_observed_at(&command) else {
            return ForegroundSwitchDecision::Immediate {
                application,
                command,
            };
        };
        if self.accepted_application_id.is_none()
            || self.accepted_application_id == Some(application_id)
        {
            self.accepted_application_id = Some(application_id);
            self.pending = None;
            return ForegroundSwitchDecision::Immediate {
                application,
                command,
            };
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.application.id() == application_id)
        {
            return ForegroundSwitchDecision::Pending;
        }
        self.pending = Some(PendingForegroundSwitch {
            application,
            command,
            observed_at_utc,
        });
        ForegroundSwitchDecision::Pending
    }

    fn take_mature(
        &mut self,
        now_utc: UtcMicros,
    ) -> Option<(Application, CommandEnvelope<TrackingCommand>)> {
        let pending = self.pending.as_ref()?;
        let required_us = i64::from(self.delay_seconds).saturating_mul(1_000_000);
        if now_utc.get().saturating_sub(pending.observed_at_utc.get()) < required_us {
            return None;
        }
        let pending = self.pending.take()?;
        self.accepted_application_id = Some(pending.application.id());
        Some((pending.application, pending.command))
    }

    fn set_delay_seconds(&mut self, delay_seconds: u32) {
        self.delay_seconds = delay_seconds;
    }

    fn clear_attribution(&mut self) {
        self.accepted_application_id = None;
        self.pending = None;
    }
}

fn foreground_observed_at(command: &CommandEnvelope<TrackingCommand>) -> Option<UtcMicros> {
    match command.payload() {
        TrackingCommand::Evidence(TrackingEvidence::Foreground {
            observed_at_utc, ..
        }) => Some(*observed_at_utc),
        _ => None,
    }
}

fn command_ends_foreground_attribution(command: &CommandEnvelope<TrackingCommand>) -> bool {
    !matches!(
        command.payload(),
        TrackingCommand::Checkpoint
            | TrackingCommand::Evidence(TrackingEvidence::Foreground { .. })
    )
}

/// Delivers a bounded in-app completion signal after durable focus completion.
struct InAppFocusNotifications {
    completion_pending: Arc<AtomicBool>,
    native_notice_sender: SyncSender<()>,
}

impl FocusNotificationPort for InAppFocusNotifications {
    fn notify_completed(&mut self, _: &FocusSnapshot) -> Result<(), FocusNotificationError> {
        self.completion_pending.store(true, Ordering::Release);
        let _ = self.native_notice_sender.try_send(());
        Ok(())
    }
}

/// Exclusive-worker control which is deliberately separate from ordinary ingress.
enum WriterControl {
    Checkpoint(SyncSender<bool>),
    Close(SyncSender<bool>),
    PrepareRestore(SyncSender<bool>),
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
    tracking_permitted: Arc<AtomicBool>,
    tracking_debug: Arc<Mutex<TrackingDebugState>>,
    tracking_debug_log: PathBuf,
}

impl TrackingEvidenceSink for RuntimeEvidenceSink {
    fn try_publish(&self, evidence: TrackingEvidence) -> EvidencePublishResult {
        record_tracking_observation(&self.tracking_debug, &self.tracking_debug_log, &evidence);
        if !self.accepting.load(Ordering::Acquire)
            || !self.tracking_permitted.load(Ordering::Acquire)
        {
            set_tracking_delivery(
                &self.tracking_debug,
                &self.tracking_debug_log,
                "Blocked: tracking is not enabled.",
            );
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
        let result = match self.lanes.try_submit(WorkLane::Critical, work) {
            LaneSubmit::Enqueued => EvidencePublishResult::Accepted,
            LaneSubmit::Retained { reason, .. } => match reason {
                openmanic_application::LaneRetentionReason::Full => EvidencePublishResult::Full,
                openmanic_application::LaneRetentionReason::Closed => EvidencePublishResult::Closed,
            },
            LaneSubmit::Dropped { .. } => EvidencePublishResult::Full,
        };
        set_tracking_delivery(
            &self.tracking_debug,
            &self.tracking_debug_log,
            match result {
                EvidencePublishResult::Accepted => "Accepted by the writer lane.",
                EvidencePublishResult::Full => "Dropped: the writer lane is full.",
                EvidencePublishResult::Closed => "Rejected: the writer lane is closed.",
            },
        );
        result
    }
}

#[derive(Clone, Debug)]
struct TrackingDebugState {
    foreground_events: u64,
    latest_application_id: Option<ApplicationId>,
    latest_observed_at: Option<UtcMicros>,
    last_delivery: &'static str,
    latest_window_title: Option<String>,
    latest_executable: Option<String>,
    latest_product_name: Option<String>,
}

impl Default for TrackingDebugState {
    fn default() -> Self {
        Self {
            foreground_events: 0,
            latest_application_id: None,
            latest_observed_at: None,
            last_delivery: "Waiting for a Windows foreground event.",
            latest_window_title: None,
            latest_executable: None,
            latest_product_name: None,
        }
    }
}

fn record_tracking_observation(
    debug: &Mutex<TrackingDebugState>,
    log_path: &Path,
    evidence: &TrackingEvidence,
) {
    let TrackingEvidence::Foreground {
        application_id,
        observed_at_utc,
        ..
    } = *evidence
    else {
        return;
    };
    if let Ok(mut state) = debug.lock() {
        state.foreground_events = state.foreground_events.saturating_add(1);
        state.latest_application_id = Some(application_id);
        state.latest_observed_at = Some(observed_at_utc);
    }
    append_tracking_debug_log(
        log_path,
        &format!(
            "observed_at_utc_us={} event=foreground application_id={}",
            observed_at_utc.get(),
            id_label(&application_id.as_bytes())
        ),
    );
}

fn set_tracking_delivery(
    debug: &Mutex<TrackingDebugState>,
    log_path: &Path,
    delivery: &'static str,
) {
    if let Ok(mut state) = debug.lock() {
        state.last_delivery = delivery;
    }
    append_tracking_debug_log(log_path, &format!("writer_lane={delivery}"));
}

fn append_tracking_debug_log(log_path: &Path, line: &str) {
    let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    else {
        return;
    };
    let _ = writeln!(file, "{line}");
}

fn settings_with_development_defaults(settings: &SettingsSnapshot) -> SettingsSnapshot {
    SettingsSnapshot::new(
        1,
        settings.start_tracking_automatically(),
        settings.start_at_login(),
        settings.close_to_tray(),
        settings.idle_threshold_seconds(),
        settings.foreground_switch_delay_seconds(),
        settings.idle_policy_code(),
        true,
        settings.time_zone_mode(),
        settings.manual_time_zone_id().map(str::to_owned),
        settings.theme_mode(),
        settings.density_code(),
        settings.notifications_enabled(),
        settings.focus_sounds_enabled(),
        settings.tray_explanation_acknowledged(),
        settings.revision(),
    )
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
        let snapshot =
            ScheduleSnapshot::try_new(id, rule, EntityRevision::new(0), submitted_at_utc)
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

    fn schedule_delete(&self, snapshot: &ScheduleSnapshot) -> CommandEnvelope<ScheduleCommand> {
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

    fn schedule_replace(&self, snapshot: ScheduleSnapshot) -> CommandEnvelope<ScheduleCommand> {
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
            SchemaRevision::new(1),
            command_id,
            ordering_key,
            Some(expected_revision),
            submitted_at_utc,
            ScheduleCommand::EditOccurrence {
                series_id,
                anchor_date,
                scope,
                edit,
            },
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
        store
            .writer()
            .delete_schedule(schedule_id, expected_revision)
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

/// One bounded local file operation owned by the dedicated data-operation worker.
#[derive(Debug)]
enum DataOperationWork {
    Export(CsvExportRequest),
    Import {
        request: CsvImportRequest,
        cancellation: CancellationToken,
    },
    Backup {
        job_id: JobId,
        destination: PathBuf,
    },
    Diagnostics {
        job_id: JobId,
        destination: PathBuf,
    },
    Restore {
        job_id: JobId,
        source: PathBuf,
    },
    MoveDataRoot {
        job_id: JobId,
        source: PathBuf,
        destination: PathBuf,
        locator_path: PathBuf,
        store_identity: [u8; 16],
    },
}

/// Privacy-safe completion state made available to the host UI after a CSV operation finishes.
#[derive(Clone, Debug)]
enum DataOperationResult {
    Exported {
        request: CsvExportRequest,
        outcome: DataOperationOutcome,
    },
    Imported {
        request: CsvImportRequest,
        outcome: ImportScopeOutcome,
    },
    ImportCancelled {
        request: CsvImportRequest,
    },
    BackedUp {
        job_id: JobId,
    },
    DiagnosticsExported {
        job_id: JobId,
    },
    Restored {
        job_id: JobId,
    },
    RestoreFailed {
        job_id: JobId,
    },
    DataRootMoved {
        job_id: JobId,
        destination: PathBuf,
    },
    DataRootMoveFailed {
        job_id: JobId,
    },
    Failed {
        job_id: openmanic_application::JobId,
        operation: &'static str,
    },
}

fn retain_data_operation_result(
    results: &Arc<Mutex<VecDeque<DataOperationResult>>>,
    result: DataOperationResult,
) {
    let Ok(mut retained) = results.lock() else {
        return;
    };
    if retained.len() == DATA_OPERATION_RESULT_CAPACITY {
        let _ = retained.pop_front();
    }
    retained.push_back(result);
}

/// All process-owned workers and their bounded ingress boundaries.
struct RuntimeResources {
    // Retains the shared store until both workers have completed coordinated shutdown.
    _store: Arc<Mutex<SqliteStore>>,
    lanes: Arc<RuntimeLanes<WriterWork>>,
    projection_requests: LatestMailbox<ProjectionRequest<TimelineContext>>,
    calendar_projection_requests: LatestMailbox<ProjectionRequest<CalendarDayContext>>,
    ui_inbox: Arc<UiInbox>,
    writer_control: SyncSender<WriterControl>,
    writer_handle: Option<JoinHandle<()>>,
    writer_closed_for_restore: Arc<AtomicBool>,
    data_operation_requests: SyncSender<DataOperationWork>,
    data_operation_stop: Option<mpsc::Sender<()>>,
    data_operation_handle: Option<JoinHandle<()>>,
    data_operation_results: Arc<Mutex<VecDeque<DataOperationResult>>>,
    import_cancellations: Arc<Mutex<BTreeMap<JobId, CancellationSource>>>,
    accepting: Arc<AtomicBool>,
    identifiers: Arc<CommandIdentifiers>,
    focus_notification_error: Arc<Mutex<Option<FocusNotificationError>>>,
    focus_completion_pending: Arc<AtomicBool>,
    #[cfg(windows)]
    focus_notice_receiver: Option<Receiver<()>>,
    focus_snapshot: Arc<Mutex<Option<FocusSnapshot>>>,
    layout_snapshot: Arc<Mutex<LayoutSnapshot>>,
    theme_mode: Arc<Mutex<u8>>,
    settings_snapshot: Arc<Mutex<SettingsSnapshot>>,
    tracking_permitted: Arc<AtomicBool>,
    tracking_debug: Arc<Mutex<TrackingDebugState>>,
    tracking_debug_log: PathBuf,
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
    title_stop: Option<mpsc::Sender<()>>,
    #[cfg(windows)]
    title_handle: Option<JoinHandle<()>>,
    #[cfg(windows)]
    application_icons: Arc<Mutex<ApplicationIconCache>>,
    #[cfg(windows)]
    activation_stop: Arc<AtomicBool>,
    #[cfg(windows)]
    activation_handle: Option<JoinHandle<()>>,
}

type RuntimeStartResult = (
    RuntimeResources,
    LatestMailboxReceiver<SnapshotEnvelope<TodaySnapshot>>,
    LatestMailboxReceiver<SnapshotEnvelope<CalendarDaySnapshot>>,
);

impl RuntimeResources {
    #[expect(
        clippy::too_many_lines,
        reason = "construction names every owned runtime boundary so startup and shutdown ownership stay auditable."
    )]
    fn start(
        mut store: SqliteStore,
        data_root: PathBuf,
    ) -> Result<RuntimeStartResult, CompositionError> {
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
        let (calendar_projection_requests, calendar_projection_receiver) = latest_mailbox();
        let (calendar_snapshots, calendar_snapshot_receiver) = latest_mailbox();
        let (writer_control, control_receiver) = mpsc::sync_channel(2);
        let writer_closed_for_restore = Arc::new(AtomicBool::new(false));
        let ui_inbox = Arc::new(UiInbox::new(UI_EVENT_CAPACITY));
        let accepting = Arc::new(AtomicBool::new(true));
        let identifiers = Arc::new(CommandIdentifiers::default());
        let focus_notification_error = Arc::new(Mutex::new(None));
        let focus_completion_pending = Arc::new(AtomicBool::new(false));
        let (focus_notice_sender, focus_notice_receiver) = mpsc::sync_channel(1);
        let focus_snapshot = Arc::new(Mutex::new(None));
        // A corrupt or superseded optional layout must not prevent the core tracker
        // from starting. The editor exposes Reset so the safe default can be saved.
        let initial_layout = LayoutPersistence::load_layout(store.writer())
            .unwrap_or(None)
            .unwrap_or_else(|| {
                LayoutSnapshot::new(
                    LayoutDocument::safe_default(),
                    EntityRevision::new(0),
                    UtcMicros::new(utc_now_micros()),
                )
            });
        let layout_snapshot = Arc::new(Mutex::new(initial_layout));
        let mut initial_settings = store
            .writer()
            .settings_snapshot()
            // A corrupt optional settings document must not prevent recovery to the safe,
            // local-first defaults. Subsequent explicit settings saves replace the singleton.
            .unwrap_or(None)
            .unwrap_or_else(SettingsSnapshot::safe_default);
        if initial_settings.consent_revision() == 0 || !initial_settings.collect_window_titles() {
            let accepted = settings_with_development_defaults(&initial_settings);
            if SettingsPersistence::replace_settings(
                store.writer(),
                &accepted,
                Some(initial_settings.revision()),
            )
            .is_ok()
            {
                initial_settings = store
                    .writer()
                    .settings_snapshot()
                    .ok()
                    .flatten()
                    .unwrap_or(accepted);
            }
        }
        let initial_theme_mode = initial_settings.theme_mode().code();
        let theme_mode = Arc::new(Mutex::new(initial_theme_mode));
        let settings_snapshot = Arc::new(Mutex::new(initial_settings));
        let tracking_permitted = Arc::new(AtomicBool::new(settings_snapshot.lock().is_ok_and(
            |settings| settings.consent_revision() > 0 && settings.start_tracking_automatically(),
        )));
        let tracking_debug = Arc::new(Mutex::new(TrackingDebugState::default()));
        let tracking_debug_log = data_root.join("tracking-debug.log");
        let store = Arc::new(Mutex::new(store));
        let (data_operation_requests, data_operation_receiver) =
            mpsc::sync_channel(DATA_OPERATION_REQUEST_CAPACITY);
        let (data_operation_stop, data_operation_stop_receiver) = mpsc::channel();
        let data_operation_results = Arc::new(Mutex::new(VecDeque::new()));
        let import_cancellations = Arc::new(Mutex::new(BTreeMap::new()));
        let data_operation_handle = spawn_data_operation_worker(
            Arc::clone(&store),
            data_root,
            writer_control.clone(),
            Arc::clone(&writer_closed_for_restore),
            data_operation_receiver,
            data_operation_stop_receiver,
            Arc::clone(&data_operation_results),
        )?;
        let worker_ui_inbox = Arc::clone(&ui_inbox);
        let worker_focus_notification_error = Arc::clone(&focus_notification_error);
        let worker_focus_completion_pending = Arc::clone(&focus_completion_pending);
        let worker_focus_snapshot = Arc::clone(&focus_snapshot);
        let worker_layout_snapshot = Arc::clone(&layout_snapshot);
        let worker_theme_mode = Arc::clone(&theme_mode);
        let worker_settings_snapshot = Arc::clone(&settings_snapshot);
        let worker_tracking_permitted = Arc::clone(&tracking_permitted);
        let worker_store = Arc::clone(&store);
        let writer_handle = thread::Builder::new()
            .name(ThreadRoot::new(RuntimeWorker::Writer).name().to_owned())
            .spawn(move || {
                run_writer_worker(
                    worker_store,
                    lane_receiver,
                    control_receiver,
                    projection_receiver,
                    snapshots,
                    calendar_projection_receiver,
                    calendar_snapshots,
                    worker_ui_inbox,
                    worker_focus_notification_error,
                    worker_focus_completion_pending,
                    focus_notice_sender,
                    worker_focus_snapshot,
                    worker_layout_snapshot,
                    worker_theme_mode,
                    worker_settings_snapshot,
                    worker_tracking_permitted,
                );
            })
            .map_err(|_| CompositionError::Runtime)?;

        Ok((
            Self {
                _store: store,
                lanes,
                projection_requests,
                calendar_projection_requests,
                ui_inbox,
                writer_control,
                writer_handle: Some(writer_handle),
                writer_closed_for_restore,
                data_operation_requests,
                data_operation_stop: Some(data_operation_stop),
                data_operation_handle: Some(data_operation_handle),
                data_operation_results,
                import_cancellations,
                accepting,
                identifiers,
                focus_notification_error,
                focus_completion_pending,
                #[cfg(windows)]
                focus_notice_receiver: Some(focus_notice_receiver),
                focus_snapshot,
                layout_snapshot,
                theme_mode,
                settings_snapshot,
                tracking_permitted,
                tracking_debug,
                tracking_debug_log,
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
                title_stop: None,
                #[cfg(windows)]
                title_handle: None,
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
            calendar_snapshot_receiver,
        ))
    }

    fn evidence_sink(&self) -> RuntimeEvidenceSink {
        RuntimeEvidenceSink {
            lanes: self.lanes.clone(),
            identifiers: Arc::clone(&self.identifiers),
            accepting: Arc::clone(&self.accepting),
            tracking_permitted: Arc::clone(&self.tracking_permitted),
            tracking_debug: Arc::clone(&self.tracking_debug),
            tracking_debug_log: self.tracking_debug_log.clone(),
        }
    }

    fn tracking_debug_snapshot(&self) -> TrackingDebugState {
        self.tracking_debug.lock().map_or_else(
            |_| TrackingDebugState {
                last_delivery: "Unavailable: diagnostics state lock is poisoned.",
                ..TrackingDebugState::default()
            },
            |state| state.clone(),
        )
    }

    fn tracking_debug_log_path(&self) -> &Path {
        &self.tracking_debug_log
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
    fn application_icon(
        &self,
        application_id: ApplicationId,
    ) -> Option<openmanic_application::ApplicationIcon> {
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

    fn try_submit_layout(
        &self,
        document: LayoutDocument,
        expected_revision: Option<EntityRevision>,
        observed_at_utc: UtcMicros,
    ) -> bool {
        self.accepting.load(Ordering::Acquire)
            && matches!(
                self.lanes.try_submit(
                    WorkLane::Normal,
                    WriterWork::Layout {
                        document,
                        expected_revision,
                        observed_at_utc,
                    },
                ),
                LaneSubmit::Enqueued
            )
    }

    fn try_submit_theme(&self, theme_mode: u8, observed_at_utc: UtcMicros) -> bool {
        self.accepting.load(Ordering::Acquire)
            && matches!(
                self.lanes.try_submit(
                    WorkLane::Normal,
                    WriterWork::Theme {
                        theme_mode,
                        observed_at_utc,
                    },
                ),
                LaneSubmit::Enqueued
            )
    }

    fn try_submit_settings(
        &self,
        snapshot: SettingsSnapshot,
        expected_revision: Option<EntityRevision>,
    ) -> bool {
        self.accepting.load(Ordering::Acquire)
            && matches!(
                self.lanes.try_submit(
                    WorkLane::Normal,
                    WriterWork::Settings {
                        snapshot,
                        expected_revision,
                    },
                ),
                LaneSubmit::Enqueued
            )
    }

    /// Submits one explicitly confirmed CSV export without blocking an egui frame.
    fn try_submit_csv_export(&self, request: CsvExportRequest) -> bool {
        self.accepting.load(Ordering::Acquire)
            && self
                .data_operation_requests
                .try_send(DataOperationWork::Export(request))
                .is_ok()
    }

    /// Submits one explicitly initiated CSV import without blocking an egui frame.
    fn try_submit_csv_import(&self, request: CsvImportRequest) -> bool {
        if !self.accepting.load(Ordering::Acquire) {
            return false;
        }
        let job_id = request.job_id();
        let (source, cancellation) = CancellationSource::new();
        let Ok(mut cancellations) = self.import_cancellations.lock() else {
            return false;
        };
        if self
            .data_operation_requests
            .try_send(DataOperationWork::Import {
                request,
                cancellation,
            })
            .is_err()
        {
            return false;
        }
        cancellations.insert(job_id, source);
        true
    }

    /// Requests cancellation of a queued or running CSV import at its next durable checkpoint.
    fn try_cancel_csv_import(&self, job_id: JobId) -> bool {
        self.import_cancellations
            .lock()
            .ok()
            .and_then(|cancellations| cancellations.get(&job_id).map(CancellationSource::cancel))
            .is_some_and(|request| matches!(request, CancellationRequest::Requested))
    }

    /// Forgets the local cancellation handle after a terminal worker result is observed.
    fn finish_csv_import(&self, job_id: JobId) {
        if let Ok(mut cancellations) = self.import_cancellations.lock() {
            cancellations.remove(&job_id);
        }
    }

    /// Submits one new backup destination without blocking an egui frame.
    fn try_submit_backup(&self, job_id: JobId, destination: PathBuf) -> bool {
        self.accepting.load(Ordering::Acquire)
            && self
                .data_operation_requests
                .try_send(DataOperationWork::Backup {
                    job_id,
                    destination,
                })
                .is_ok()
    }

    /// Submits a privacy-safe diagnostics export without blocking an egui frame.
    fn try_submit_diagnostics(&self, job_id: JobId, destination: PathBuf) -> bool {
        self.accepting.load(Ordering::Acquire)
            && self
                .data_operation_requests
                .try_send(DataOperationWork::Diagnostics {
                    job_id,
                    destination,
                })
                .is_ok()
    }

    /// Starts an explicitly confirmed restore and stops ordinary submissions before the worker
    /// closes the current writer.
    fn try_submit_restore(&self, job_id: JobId, source: PathBuf) -> bool {
        if !self.accepting.load(Ordering::Acquire) {
            return false;
        }
        let submitted = self
            .data_operation_requests
            .try_send(DataOperationWork::Restore { job_id, source })
            .is_ok();
        if submitted {
            self.accepting.store(false, Ordering::Release);
        }
        submitted
    }

    /// Starts an explicitly confirmed data-root move and pauses normal submissions.
    fn try_submit_data_root_move(
        &self,
        job_id: JobId,
        source: PathBuf,
        destination: PathBuf,
        locator_path: PathBuf,
        store_identity: [u8; 16],
    ) -> bool {
        if !self.accepting.load(Ordering::Acquire) {
            return false;
        }
        let submitted = self
            .data_operation_requests
            .try_send(DataOperationWork::MoveDataRoot {
                job_id,
                source,
                destination,
                locator_path,
                store_identity,
            })
            .is_ok();
        if submitted {
            self.accepting.store(false, Ordering::Release);
        }
        submitted
    }

    /// Removes the next completed local data operation for presentation by the host UI.
    fn take_data_operation_result(&self) -> Option<DataOperationResult> {
        self.data_operation_results
            .lock()
            .ok()
            .and_then(|mut results| results.pop_front())
    }

    fn reject_nonessential_work(&self) {
        self.accepting.store(false, Ordering::Release);
    }

    fn checkpoint_writer(&self) -> bool {
        if self.writer_closed_for_restore.load(Ordering::Acquire) {
            return true;
        }
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
        if self.writer_closed_for_restore.load(Ordering::Acquire) {
            return self
                .writer_handle
                .take()
                .is_none_or(|handle| handle.join().is_ok());
        }
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

    fn close_data_operation_worker(&mut self) -> bool {
        if let Some(stop) = self.data_operation_stop.take() {
            let _ = stop.send(());
        }
        self.data_operation_handle
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
            Arc::clone(&self.lanes),
            Arc::clone(&self.application_icons),
            Arc::clone(&self.tracking_debug),
        )?;
        let (title_requests, title_stop, title_handle) =
            spawn_title_worker(Arc::clone(&self.lanes), Arc::clone(&self.tracking_debug))?;
        let Some(focus_notice_receiver) = self.focus_notice_receiver.take() else {
            return Err(CompositionError::Runtime);
        };
        let (stop_sender, platform_handle, ready_receiver) = spawn_platform_worker(
            self.action_router(),
            self.evidence_sink(),
            metadata_requests,
            title_requests,
            focus_notice_receiver,
        )?;
        if let Ok(Ok(())) = ready_receiver.recv_timeout(Duration::from_secs(5)) {
            self.activation_handle = Some(activation_handle);
            self.platform_stop = Some(stop_sender);
            self.platform_handle = Some(platform_handle);
            self.metadata_stop = Some(metadata_stop);
            self.metadata_handle = Some(metadata_handle);
            self.title_stop = Some(title_stop);
            self.title_handle = Some(title_handle);
            return Ok(());
        }

        self.activation_stop.store(true, Ordering::Release);
        let _ = stop_sender.send(());
        let _ = metadata_stop.send(());
        let _ = title_stop.send(());
        let _ = activation_handle.join();
        let _ = platform_handle.join();
        let _ = metadata_handle.join();
        let _ = title_handle.join();
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
        if let Some(stop) = self.title_stop.take() {
            let _ = stop.send(());
        }
        let title_joined = self
            .title_handle
            .take()
            .is_none_or(|handle| handle.join().is_ok());
        let activation_joined = self
            .activation_handle
            .take()
            .is_none_or(|handle| handle.join().is_ok());
        platform_joined && metadata_joined && title_joined && activation_joined
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
    title_requests: SyncSender<WindowsWindowTitleObservationRequest>,
    focus_notice_receiver: Receiver<()>,
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
                title_requests,
                focus_notice_receiver,
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
    title_requests: SyncSender<WindowsWindowTitleObservationRequest>,
    focus_notice_receiver: Receiver<()>,
    stop_receiver: Receiver<()>,
    ready_sender: &SyncSender<Result<(), ()>>,
) -> Result<(), openmanic_platform::WindowsControlError> {
    let mut adapter = WindowsControlAdapter::new()
        .with_metadata_requests(metadata_requests)
        .with_title_requests(title_requests);
    let mut control = adapter.install_control_window()?;
    let mut tray = control.install_tray()?;
    let _ = ready_sender.send(Ok(()));
    loop {
        control.pump_available_with_tray(&mut adapter, &evidence_sink, &mut tray)?;
        while focus_notice_receiver.try_recv().is_ok() {
            let _ = tray.show_focus_completion_notice();
        }
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
type MetadataWorkerStart = (
    SyncSender<WindowsApplicationMetadataRequest>,
    mpsc::Sender<()>,
    JoinHandle<()>,
);

#[cfg(windows)]
type TitleWorkerStart = (
    SyncSender<WindowsWindowTitleObservationRequest>,
    mpsc::Sender<()>,
    JoinHandle<()>,
);

#[cfg(windows)]
fn spawn_metadata_worker(
    lanes: Arc<RuntimeLanes<WriterWork>>,
    application_icons: Arc<Mutex<ApplicationIconCache>>,
    tracking_debug: Arc<Mutex<TrackingDebugState>>,
) -> Result<MetadataWorkerStart, CompositionError> {
    let (request_sender, request_receiver) =
        mpsc::sync_channel(APPLICATION_METADATA_REQUEST_CAPACITY);
    let (stop_sender, stop_receiver) = mpsc::channel();
    let handle = thread::Builder::new()
        .name(ThreadRoot::new(RuntimeWorker::BulkWorker).name().to_owned())
        .spawn(move || {
            run_metadata_worker(
                request_receiver,
                stop_receiver,
                lanes,
                application_icons,
                tracking_debug,
            );
        })
        .map_err(|_| CompositionError::Runtime)?;
    Ok((request_sender, stop_sender, handle))
}

#[cfg(windows)]
fn spawn_title_worker(
    lanes: Arc<RuntimeLanes<WriterWork>>,
    tracking_debug: Arc<Mutex<TrackingDebugState>>,
) -> Result<TitleWorkerStart, CompositionError> {
    let (request_sender, request_receiver) = mpsc::sync_channel(WINDOW_TITLE_OBSERVATION_CAPACITY);
    let (stop_sender, stop_receiver) = mpsc::channel();
    let handle = thread::Builder::new()
        .name("openmanic-window-titles".to_owned())
        .spawn(move || run_title_worker(request_receiver, stop_receiver, lanes, tracking_debug))
        .map_err(|_| CompositionError::Runtime)?;
    Ok((request_sender, stop_sender, handle))
}

#[cfg(windows)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the worker owns its receivers and lane publisher for its complete thread lifetime."
)]
fn run_title_worker(
    requests: Receiver<WindowsWindowTitleObservationRequest>,
    stop: Receiver<()>,
    lanes: Arc<RuntimeLanes<WriterWork>>,
    tracking_debug: Arc<Mutex<TrackingDebugState>>,
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
        let work = WriterWork::WindowTitle(WindowTitleObservation {
            application_id: request.application_id(),
            observed_at_utc: UtcMicros::new(request.observed_at_utc_us()),
            title: request.title().to_owned(),
        });
        if let Ok(mut debug) = tracking_debug.lock() {
            debug.latest_window_title = Some(request.title().to_owned());
        }
        let _ = lanes.try_submit(WorkLane::Optional, work);
    }
}

#[cfg(windows)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the worker owns its receivers and icon cache publisher for its complete thread lifetime."
)]
fn run_metadata_worker(
    requests: Receiver<WindowsApplicationMetadataRequest>,
    stop: Receiver<()>,
    lanes: Arc<RuntimeLanes<WriterWork>>,
    application_icons: Arc<Mutex<ApplicationIconCache>>,
    tracking_debug: Arc<Mutex<TrackingDebugState>>,
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
        let result: ApplicationIconResult = openmanic_platform::extract_application_icon(&request);
        let display_name = openmanic_platform::extract_application_display_name(&request);
        let observed_at_utc = UtcMicros::new(request.observed_at_utc_us());
        if let Ok(name) = ApplicationName::try_new(display_name.clone())
            && let Ok(application) = Application::try_new(
                request.application_id(),
                name,
                None,
                observed_at_utc,
                observed_at_utc,
            )
        {
            let _ = lanes.try_submit(
                WorkLane::Optional,
                WriterWork::CatalogMetadata { application },
            );
        }
        if let Ok(mut debug) = tracking_debug.lock() {
            debug.latest_executable = Some(request.executable_path().to_owned());
            debug.latest_product_name = Some(display_name);
        }
        if let Ok(mut cache) = application_icons.lock() {
            let _ = result.apply_to(&mut cache);
        }
    }
}

fn spawn_data_operation_worker(
    store: Arc<Mutex<SqliteStore>>,
    data_root: PathBuf,
    writer_control: SyncSender<WriterControl>,
    writer_closed_for_restore: Arc<AtomicBool>,
    requests: Receiver<DataOperationWork>,
    stop: Receiver<()>,
    results: Arc<Mutex<VecDeque<DataOperationResult>>>,
) -> Result<JoinHandle<()>, CompositionError> {
    thread::Builder::new()
        .name("openmanic-data-operations".to_owned())
        .spawn(move || {
            run_data_operation_worker(
                store,
                data_root,
                writer_control,
                writer_closed_for_restore,
                requests,
                stop,
                results,
            );
        })
        .map_err(|_| CompositionError::Runtime)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the operation worker owns its bounded request receiver and stop receiver."
)]
#[expect(
    clippy::too_many_lines,
    reason = "the operation worker keeps every local file operation and its worker boundary auditable."
)]
#[expect(
    clippy::excessive_nesting,
    reason = "restore must retain the explicit writer-close and restore-success boundaries."
)]
fn run_data_operation_worker(
    store: Arc<Mutex<SqliteStore>>,
    data_root: PathBuf,
    writer_control: SyncSender<WriterControl>,
    writer_closed_for_restore: Arc<AtomicBool>,
    requests: Receiver<DataOperationWork>,
    stop: Receiver<()>,
    results: Arc<Mutex<VecDeque<DataOperationResult>>>,
) {
    loop {
        if matches!(stop.try_recv(), Ok(()) | Err(TryRecvError::Disconnected)) {
            return;
        }
        let work = match requests.recv_timeout(WORKER_IDLE_INTERVAL) {
            Ok(work) => work,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        let completed_at_utc = UtcMicros::new(utc_now_micros());
        let result = match work {
            DataOperationWork::Export(request) => {
                let job_id = request.job_id();
                let export = store.lock().ok().and_then(|store| {
                    store
                        .open_read_session()
                        .ok()
                        .and_then(|mut reader| reader.export_csv(&request, completed_at_utc).ok())
                });
                export.map_or_else(
                    || DataOperationResult::Failed {
                        job_id,
                        operation: "CSV export",
                    },
                    |outcome| DataOperationResult::Exported { request, outcome },
                )
            }
            DataOperationWork::Import {
                request,
                cancellation,
            } => {
                let job_id = request.job_id();
                let import = store.lock().ok().and_then(|mut store| {
                    store
                        .writer()
                        .import_csv_cancellable(&request, completed_at_utc, &cancellation)
                        .ok()
                });
                import.map_or_else(
                    || DataOperationResult::Failed {
                        job_id,
                        operation: "CSV import",
                    },
                    |outcome| match outcome {
                        Some(outcome) => DataOperationResult::Imported { request, outcome },
                        None => DataOperationResult::ImportCancelled { request },
                    },
                )
            }
            DataOperationWork::Backup {
                job_id,
                destination,
            } => store
                .lock()
                .ok()
                .and_then(|mut store| store.create_backup(&destination).ok())
                .map_or_else(
                    || DataOperationResult::Failed {
                        job_id,
                        operation: "Backup",
                    },
                    |()| DataOperationResult::BackedUp { job_id },
                ),
            DataOperationWork::Diagnostics {
                job_id,
                destination,
            } => export_diagnostics_bundle(&data_root, &destination).map_or_else(
                |_| DataOperationResult::Failed {
                    job_id,
                    operation: "Diagnostics export",
                },
                |_| DataOperationResult::DiagnosticsExported { job_id },
            ),
            DataOperationWork::Restore { job_id, source } => {
                let (reply, receive) = mpsc::sync_channel(1);
                let writer_closed = writer_control
                    .try_send(WriterControl::PrepareRestore(reply))
                    .is_ok()
                    && receive
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap_or(false);
                if writer_closed {
                    writer_closed_for_restore.store(true, Ordering::Release);
                    if store
                        .lock()
                        .ok()
                        .and_then(|mut store| store.restore_backup(&source).ok())
                        .is_some()
                    {
                        DataOperationResult::Restored { job_id }
                    } else {
                        DataOperationResult::RestoreFailed { job_id }
                    }
                } else {
                    DataOperationResult::RestoreFailed { job_id }
                }
            }
            DataOperationWork::MoveDataRoot {
                job_id,
                source,
                destination,
                locator_path,
                store_identity,
            } => {
                let (reply, receive) = mpsc::sync_channel(1);
                let writer_closed = writer_control
                    .try_send(WriterControl::PrepareRestore(reply))
                    .is_ok()
                    && receive
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap_or(false);
                let moved = writer_closed
                    && move_data_root_with_online_backup(
                        &store,
                        &source,
                        &destination,
                        &locator_path,
                        store_identity,
                    );
                if writer_closed {
                    writer_closed_for_restore.store(true, Ordering::Release);
                }
                if moved {
                    DataOperationResult::DataRootMoved {
                        job_id,
                        destination,
                    }
                } else {
                    DataOperationResult::DataRootMoveFailed { job_id }
                }
            }
        };
        retain_data_operation_result(&results, result);
    }
}

fn move_data_root_with_online_backup(
    store: &Arc<Mutex<SqliteStore>>,
    source: &Path,
    destination: &Path,
    locator_path: &Path,
    store_identity: [u8; 16],
) -> bool {
    if source == destination
        || !source.is_dir()
        || (destination.exists()
            && fs::read_dir(destination)
                .ok()
                .and_then(|mut entries| entries.next())
                .is_some())
    {
        return false;
    }
    if fs::create_dir_all(destination).is_err()
        || LocalDataRootValidator::new(RejectKnownNetworkShares)
            .validate(destination)
            .is_err()
    {
        return false;
    }
    let database = destination.join("openmanic.sqlite3");
    if store
        .lock()
        .ok()
        .and_then(|mut store| store.create_backup(&database).ok())
        .is_none()
        || !copy_data_root_support_files(source, destination)
    {
        return false;
    }
    if SqliteStore::open(
        &database,
        &StoreOpenOptions::new(store_identity, utc_now_micros(), env!("CARGO_PKG_VERSION")),
    )
    .is_err()
    {
        return false;
    }
    persist_locator(
        locator_path,
        &BootstrapLocator::new(destination.to_path_buf(), None),
    )
    .is_ok()
}

fn copy_data_root_support_files(source: &Path, destination: &Path) -> bool {
    let Ok(entries) = fs::read_dir(source) else {
        return false;
    };
    for entry in entries.flatten() {
        let source_path = entry.path();
        let name = entry.file_name();
        if name == ".openmanic-data-root.lock"
            || name == "openmanic.sqlite3"
            || name == "openmanic.sqlite3-wal"
            || name == "openmanic.sqlite3-shm"
        {
            continue;
        }
        let destination_path = destination.join(name);
        let Ok(metadata) = entry.metadata() else {
            return false;
        };
        if metadata.is_dir() {
            if fs::create_dir(&destination_path).is_err()
                || !copy_data_root_support_files(&source_path, &destination_path)
            {
                return false;
            }
        } else if fs::copy(&source_path, &destination_path).is_err() {
            return false;
        }
    }
    true
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the named worker exclusively owns all receivers and publishers for its lifetime"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "the worker receives each independently owned runtime boundary explicitly."
)]
#[expect(
    clippy::too_many_lines,
    reason = "the worker loop keeps each receiver's ordering and snapshot publication behavior auditable"
)]
fn run_writer_worker(
    store: Arc<Mutex<SqliteStore>>,
    lanes: RuntimeLaneReceiver<WriterWork>,
    control: Receiver<WriterControl>,
    projection_requests: LatestMailboxReceiver<ProjectionRequest<TimelineContext>>,
    snapshots: LatestMailbox<SnapshotEnvelope<TodaySnapshot>>,
    calendar_projection_requests: LatestMailboxReceiver<ProjectionRequest<CalendarDayContext>>,
    calendar_snapshots: LatestMailbox<SnapshotEnvelope<CalendarDaySnapshot>>,
    ui_inbox: Arc<UiInbox>,
    focus_notification_error: Arc<Mutex<Option<FocusNotificationError>>>,
    focus_completion_pending: Arc<AtomicBool>,
    focus_notice_sender: SyncSender<()>,
    focus_snapshot: Arc<Mutex<Option<FocusSnapshot>>>,
    layout_snapshot: Arc<Mutex<LayoutSnapshot>>,
    theme_mode: Arc<Mutex<u8>>,
    settings_snapshot: Arc<Mutex<SettingsSnapshot>>,
    tracking_permitted: Arc<AtomicBool>,
) {
    let foreground_switch_delay_seconds = settings_snapshot.lock().map_or(
        openmanic_application::DEFAULT_FOREGROUND_SWITCH_DELAY_SECONDS,
        |settings| settings.foreground_switch_delay_seconds(),
    );
    let mut services = writer_services(
        &store,
        focus_notification_error,
        focus_completion_pending,
        focus_notice_sender,
        foreground_switch_delay_seconds,
    );
    reconcile_focus_snapshot(&mut services, &focus_snapshot);
    let mut current_projection = None;
    let mut current_calendar_projection = None;
    let mut last_focus_reconciliation = Instant::now();
    let mut running = true;

    while running {
        while let MailboxReceive::Latest(request) = projection_requests.try_receive() {
            current_projection = Some(request);
            if let Some(request) = current_projection.as_ref() {
                publish_today_snapshot(&store, request, &snapshots);
            }
        }
        while let MailboxReceive::Latest(request) = calendar_projection_requests.try_receive() {
            current_calendar_projection = Some(request);
            if let Some(request) = current_calendar_projection.as_ref() {
                publish_calendar_snapshot(&store, request, &calendar_snapshots);
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
            Ok(WriterControl::Close(reply) | WriterControl::PrepareRestore(reply)) => {
                drain_writer_lanes(
                    &lanes,
                    &mut services,
                    &store,
                    &mut current_projection,
                    &snapshots,
                    &ui_inbox,
                    &focus_snapshot,
                    &layout_snapshot,
                    &theme_mode,
                    &settings_snapshot,
                    &tracking_permitted,
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
            LaneReceive::Work { work, .. } => {
                process_writer_work(
                    work,
                    &mut services,
                    &store,
                    &mut current_projection,
                    &snapshots,
                    &ui_inbox,
                    &focus_snapshot,
                    &layout_snapshot,
                    &theme_mode,
                    &settings_snapshot,
                    &tracking_permitted,
                );
                if let Some(request) = current_calendar_projection.as_ref() {
                    publish_calendar_snapshot(&store, request, &calendar_snapshots);
                }
            }
            LaneReceive::Empty => thread::park_timeout(WORKER_IDLE_INTERVAL),
            LaneReceive::Closed => running = false,
        }
        if let Some((application, command)) = services
            .foreground_switch_stabilizer
            .take_mature(UtcMicros::new(utc_now_micros()))
        {
            process_catalog_foreground(
                application,
                command,
                &mut services,
                &store,
                &mut current_projection,
                &snapshots,
                &ui_inbox,
            );
        }
        if last_focus_reconciliation.elapsed() >= Duration::from_secs(1) {
            reconcile_focus_snapshot(&mut services, &focus_snapshot);
            last_focus_reconciliation = Instant::now();
        }
    }
}

fn writer_services(
    store: &Arc<Mutex<SqliteStore>>,
    _focus_notification_error: Arc<Mutex<Option<FocusNotificationError>>>,
    focus_completion_pending: Arc<AtomicBool>,
    focus_notice_sender: SyncSender<()>,
    foreground_switch_delay_seconds: u32,
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
        InAppFocusNotifications {
            completion_pending: focus_completion_pending,
            native_notice_sender: focus_notice_sender,
        },
    );
    WriterServices {
        tracking,
        foreground_switch_stabilizer: ForegroundSwitchStabilizer::new(
            foreground_switch_delay_seconds,
        ),
        catalog,
        schedules,
        focus,
        title_stabilizer: TitleStabilizer::default(),
        ui_event_sequence: 0,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "worker lanes retain explicit independently owned runtime boundaries"
)]
fn drain_writer_lanes(
    lanes: &RuntimeLaneReceiver<WriterWork>,
    services: &mut WriterServices,
    store: &Arc<Mutex<SqliteStore>>,
    current_projection: &mut Option<ProjectionRequest<TimelineContext>>,
    snapshots: &LatestMailbox<SnapshotEnvelope<TodaySnapshot>>,
    ui_inbox: &UiInbox,
    focus_snapshot: &Arc<Mutex<Option<FocusSnapshot>>>,
    layout_snapshot: &Arc<Mutex<LayoutSnapshot>>,
    theme_mode: &Arc<Mutex<u8>>,
    settings_snapshot: &Arc<Mutex<SettingsSnapshot>>,
    tracking_permitted: &Arc<AtomicBool>,
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
            layout_snapshot,
            theme_mode,
            settings_snapshot,
            tracking_permitted,
        );
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "the dispatch remains explicit so each typed work item reaches its sole authoritative service."
)]
#[expect(
    clippy::too_many_arguments,
    reason = "writer dispatch receives explicit independent runtime boundaries"
)]
fn process_writer_work(
    work: WriterWork,
    services: &mut WriterServices,
    store: &Arc<Mutex<SqliteStore>>,
    current_projection: &mut Option<ProjectionRequest<TimelineContext>>,
    snapshots: &LatestMailbox<SnapshotEnvelope<TodaySnapshot>>,
    ui_inbox: &UiInbox,
    focus_snapshot: &Arc<Mutex<Option<FocusSnapshot>>>,
    layout_snapshot: &Arc<Mutex<LayoutSnapshot>>,
    theme_mode: &Arc<Mutex<u8>>,
    settings_snapshot: &Arc<Mutex<SettingsSnapshot>>,
    tracking_permitted: &Arc<AtomicBool>,
) {
    if let WriterWork::Settings {
        snapshot,
        expected_revision,
    } = work
    {
        if let Ok(mut writer_store) = store.lock()
            && SettingsPersistence::replace_settings(
                writer_store.writer(),
                &snapshot,
                expected_revision,
            )
            .is_ok()
            && let Ok(mut current) = settings_snapshot.lock()
        {
            tracking_permitted.store(
                snapshot.consent_revision() > 0 && snapshot.start_tracking_automatically(),
                Ordering::Release,
            );
            services
                .foreground_switch_stabilizer
                .set_delay_seconds(snapshot.foreground_switch_delay_seconds());
            *current = snapshot;
        }
        return;
    }
    if let WriterWork::Layout {
        document,
        expected_revision,
        observed_at_utc,
    } = work
    {
        process_layout_replacement(
            document,
            expected_revision,
            observed_at_utc,
            store,
            layout_snapshot,
        );
        return;
    }
    if let WriterWork::Theme {
        theme_mode: selected_theme_mode,
        observed_at_utc,
    } = work
    {
        if let Ok(mut writer_store) = store.lock()
            && writer_store
                .writer()
                .set_theme_mode(selected_theme_mode, observed_at_utc)
                .is_ok()
            && let Ok(mut current) = theme_mode.lock()
        {
            *current = selected_theme_mode;
        }
        return;
    }
    if let WriterWork::WindowTitle(observation) = work {
        process_window_title_observation(&observation, services, store);
        return;
    }
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
    if let WriterWork::CatalogMetadata { application } = work {
        let updated = if let Ok(mut writer_store) = store.lock() {
            let application = preserve_or_classify_application(&mut writer_store, application);
            writer_store
                .writer()
                .upsert_application(&application)
                .is_ok()
        } else {
            false
        };
        if updated && let Some(request) = current_projection.as_ref() {
            publish_today_snapshot(store, request, snapshots);
        }
        return;
    }
    if let WriterWork::CatalogForeground {
        application,
        command,
    } = work
    {
        if let ForegroundSwitchDecision::Immediate {
            application,
            command,
        } = services
            .foreground_switch_stabilizer
            .observe(application, command)
        {
            process_catalog_foreground(
                application,
                command,
                services,
                store,
                current_projection,
                snapshots,
                ui_inbox,
            );
        }
        return;
    }
    let (command, from_ui) = work.into_parts();
    if command_ends_foreground_attribution(&command) {
        services.foreground_switch_stabilizer.clear_attribution();
    }
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

fn process_catalog_foreground(
    application: Application,
    command: CommandEnvelope<TrackingCommand>,
    services: &mut WriterServices,
    store: &Arc<Mutex<SqliteStore>>,
    current_projection: &mut Option<ProjectionRequest<TimelineContext>>,
    snapshots: &LatestMailbox<SnapshotEnvelope<TodaySnapshot>>,
    ui_inbox: &UiInbox,
) {
    let Ok(mut writer_store) = store.lock() else {
        return;
    };
    let application = preserve_or_classify_application(&mut writer_store, application);
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
}

fn process_layout_replacement(
    document: LayoutDocument,
    expected_revision: Option<EntityRevision>,
    observed_at_utc: UtcMicros,
    store: &Arc<Mutex<SqliteStore>>,
    layout_snapshot: &Arc<Mutex<LayoutSnapshot>>,
) {
    let Ok(mut writer_store) = store.lock() else {
        return;
    };
    let snapshot = LayoutSnapshot::new(
        document,
        expected_revision.unwrap_or(EntityRevision::new(0)),
        observed_at_utc,
    );
    if LayoutPersistence::replace_layout(writer_store.writer(), &snapshot, expected_revision)
        .is_err()
    {
        return;
    }
    let Ok(Some(updated)) = LayoutPersistence::load_layout(writer_store.writer()) else {
        return;
    };
    if let Ok(mut latest) = layout_snapshot.lock() {
        *latest = updated;
    }
}

fn process_window_title_observation(
    observation: &WindowTitleObservation,
    services: &mut WriterServices,
    store: &Arc<Mutex<SqliteStore>>,
) {
    let Ok(mut writer_store) = store.lock() else {
        return;
    };
    let Ok(collection_enabled) = writer_store.writer().window_title_collection_enabled() else {
        return;
    };
    let Ok(application_excluded) = writer_store
        .writer()
        .application_is_excluded(observation.application_id)
    else {
        return;
    };
    let TitleObservationResult::Accepted(title) = services.title_stabilizer.observe(
        observation.application_id,
        observation.observed_at_utc,
        &observation.title,
        collection_enabled,
        application_excluded,
    ) else {
        return;
    };
    let Some(tracker_run_id) = services
        .tracking
        .checkpoint()
        .map(openmanic_application::TrackingCheckpoint::tracker_run_id)
    else {
        return;
    };
    let _ = writer_store
        .writer()
        .persist_window_title(tracker_run_id, &title);
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
    if committed_revision.is_some()
        && let Some(application_ids) = newly_excluded
    {
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

/// Publishes only to Calendar's dedicated immutable mailbox.
///
/// This deliberately takes a Calendar-typed mailbox rather than the Today mailbox so the route
/// cannot accidentally receive a correlated-but-wrong Today response.
fn publish_calendar_snapshot(
    store: &Arc<Mutex<SqliteStore>>,
    request: &ProjectionRequest<CalendarDayContext>,
    snapshots: &LatestMailbox<SnapshotEnvelope<CalendarDaySnapshot>>,
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
    let Ok(snapshot) = build_calendar_snapshot(request, &read) else {
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
    let range = request.payload().visible_range();
    let (usage, distribution) = build_summaries(read, range)?;
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

/// Builds Calendar data from one correlated read snapshot without coupling it to Today data.
///
/// The caller owns a separate Calendar mailbox, so a Calendar request can never overwrite a
/// Timeline/Today response. Focus records contribute only when their persisted lifecycle state
/// supplies both canonical endpoints; paused sessions deliberately remain absent.
fn build_calendar_snapshot(
    request: &ProjectionRequest<CalendarDayContext>,
    read: &openmanic_storage_sqlite::ReadSnapshot,
) -> Result<SnapshotEnvelope<CalendarDaySnapshot>, ()> {
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
    let focus = read
        .focus_sessions()
        .iter()
        .filter_map(|record| {
            record
                .interval()
                .map(|interval| CalendarSourceFocus::new(record.snapshot().clone(), interval))
        })
        .collect::<Vec<_>>();
    let schedules = read
        .schedules()
        .iter()
        .map(|record| record.snapshot().clone())
        .collect::<Vec<_>>();
    let calendar = CalendarDayProjector::build(
        *request.payload(),
        CalendarProjectionSource::new(read.revision(), &activities, &focus, &schedules),
    )
    .map_err(|_| ())?;
    Ok(SnapshotEnvelope::new(
        request.request_id(),
        request.slot(),
        request.context_key(),
        read.revision(),
        openmanic_application::TIMELINE_SNAPSHOT_SCHEMA_REVISION,
        calendar,
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
    format_utc_range(range)
}

fn format_utc_range(range: HalfOpenInterval) -> String {
    format!(
        "{} - {} UTC",
        format_utc_clock(range.start()),
        format_utc_clock(range.end())
    )
}

fn format_utc_clock(instant: UtcMicros) -> String {
    const DAY_US: i64 = 86_400_000_000;
    const MINUTE_US: i64 = 60_000_000;
    let minute_of_day = instant.get().rem_euclid(DAY_US) / MINUTE_US;
    let hour = minute_of_day / 60;
    let minute = minute_of_day % 60;
    let suffix = if hour < 12 { "AM" } else { "PM" };
    let display_hour = match hour % 12 {
        0 => 12,
        value => value,
    };
    format!("{display_hour}:{minute:02} {suffix}")
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
            Self::CannotRepresent => {
                "This schedule cannot be saved with the current time settings."
            }
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

fn paint_today_widget_marker(
    ui: &mut eframe::egui::Ui,
    kind: TodayWidgetKind,
    tokens: openmanic_ui_egui::ThemeTokens,
) {
    let color = if kind == TodayWidgetKind::TIMELINE {
        eframe::egui::Color32::from_rgb(59, 130, 246)
    } else if kind == TodayWidgetKind::APPLICATION_USAGE {
        eframe::egui::Color32::from_rgb(6, 182, 212)
    } else if kind == TodayWidgetKind::TIME_DISTRIBUTION {
        eframe::egui::Color32::from_rgb(168, 85, 247)
    } else {
        tokens.success()
    };
    let (rect, _) =
        ui.allocate_exact_size(eframe::egui::vec2(8.0, 8.0), eframe::egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

const fn schedule_status_label(status: &MutationStatus) -> &'static str {
    match status {
        MutationStatus::Pending => "Saving schedule…",
        MutationStatus::Confirmed { .. } => "Schedule saved.",
        MutationStatus::Rejected {
            reason: openmanic_application::MutationRejectionReason::Conflict,
        } => "Schedule was not saved because it overlaps another schedule.",
        MutationStatus::Rejected {
            reason: openmanic_application::MutationRejectionReason::RevisionConflict,
        } => "Schedule changed elsewhere. Review it and try again.",
        MutationStatus::Rejected { .. } => {
            "Schedule was not saved. Review the schedule details and try again."
        }
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
static DATA_OPERATION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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

fn next_data_operation_id() -> JobId {
    let time = u64::try_from(utc_now_micros()).unwrap_or_default();
    let sequence = DATA_OPERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    JobId::new(time ^ sequence.rotate_left(48))
}

fn import_batch_id(job_id: JobId) -> ImportBatchId {
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&utc_now_micros().to_be_bytes());
    bytes[8..].copy_from_slice(&job_id.get().to_be_bytes());
    ImportBatchId::from_bytes(bytes)
}

/// Owns the composed vertical-slice resources until coordinated explicit quit completes.
pub struct VerticalSlice {
    bootstrap: BootstrapState,
    store_identity: [u8; 16],
    ui: UiController<TrackingCommand, TodaySnapshot>,
    snapshots: LatestMailboxReceiver<SnapshotEnvelope<TodaySnapshot>>,
    calendar_snapshots: LatestMailboxReceiver<SnapshotEnvelope<CalendarDaySnapshot>>,
    runtime: RuntimeResources,
    shutdown: ShutdownCoordinator,
    #[cfg(windows)]
    instance_owner: WindowsInstanceOwner,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "the native host retains independent settings and transient interaction toggles."
)]
struct VerticalSliceApp {
    bootstrap: BootstrapState,
    store_identity: [u8; 16],
    app: OpenManicApp<TrackingCommand, TodaySnapshot>,
    snapshots: LatestMailboxReceiver<SnapshotEnvelope<TodaySnapshot>>,
    calendar_snapshots: LatestMailboxReceiver<SnapshotEnvelope<CalendarDaySnapshot>>,
    runtime: RuntimeResources,
    shutdown: ShutdownCoordinator,
    today: TodayController,
    layout_editor: LayoutEditor,
    pending_layout_save: Option<LayoutDocument>,
    pending_layout_navigation: Option<openmanic_ui_egui::Route>,
    theme_key: String,
    calendar: CalendarController,
    calendar_data: PresentableData<CalendarDaySnapshot>,
    calendar_projection_sequence: u64,
    timeline: TimelineRenderer,
    close_to_tray: WindowsTrayController,
    settings_controller: SettingsController,
    onboarding_submission_pending: bool,
    export_destination: String,
    import_source: String,
    backup_destination: String,
    diagnostics_destination: String,
    restore_source: String,
    data_root_destination: String,
    export_includes_titles: bool,
    data_operation_message: Option<String>,
    jobs: JobsController,
    projection_sequence: u64,
    requested_range: Option<HalfOpenInterval>,
    overview_range_mode: OverviewRangeMode,
    create_schedule_mode: bool,
    schedule_draft: Option<ScheduleDraft>,
    schedule_delete_request: Option<ScheduleDeleteRequest>,
    latest_schedule_command: Option<CommandId>,
    latest_catalog_command: Option<CommandId>,
    new_category_name: String,
    selected_category_id: Option<CategoryId>,
    selected_category_applications: BTreeSet<ApplicationId>,
    application_search: String,
    #[expect(
        clippy::option_option,
        reason = "the UI distinguishes all categories, Uncategorized, and one explicit category."
    )]
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
        let store_identity = store_identity(bootstrap.data_root().path());
        let mut store = SqliteStore::open(
            &store_path,
            &StoreOpenOptions::new(store_identity, utc_now_micros(), env!("CARGO_PKG_VERSION")),
        )
        .map_err(|_| CompositionError::Storage)?;
        seed_initial_categories(&mut store)?;
        seed_demo_data(&mut store)?;
        let mut ui = UiController::try_new(
            UiModel::default(),
            UI_INBOUND_CAPACITY,
            UI_OUTBOUND_CAPACITY,
        )
        .map_err(|_| CompositionError::Runtime)?;
        let (mut runtime, snapshots, calendar_snapshots) =
            RuntimeResources::start(store, bootstrap.data_root().path().to_path_buf())?;
        #[cfg(windows)]
        runtime.start_windows(&instance_owner)?;
        let Some(initial_range) = day_range(0) else {
            return Err(CompositionError::Runtime);
        };
        let request = projection_request(1, initial_range);
        ui.begin_projection(&request);
        let _ = runtime.projection_requests.publish(request);
        let _ = runtime
            .calendar_projection_requests
            .publish(calendar_projection_request(1, initial_range));
        Ok(Self {
            bootstrap,
            store_identity,
            ui,
            snapshots,
            calendar_snapshots,
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
        let layout = self.runtime.layout_snapshot.lock().map_or_else(
            |_| LayoutDocument::safe_default(),
            |snapshot| snapshot.document().clone(),
        );
        let theme_key = self
            .runtime
            .theme_mode
            .lock()
            .map_or("openmanic.dark", |mode| match *mode {
                1 => "openmanic.light",
                2 => "openmanic.system",
                _ => "openmanic.dark",
            })
            .to_owned();
        let mut close_to_tray = WindowsTrayController::new();
        let settings_controller = self.runtime.settings_snapshot.lock().map_or_else(
            |_| SettingsController::new(SettingsSnapshot::safe_default()),
            |settings| SettingsController::new(settings.clone()),
        );
        if let Ok(settings) = self.runtime.settings_snapshot.lock() {
            close_to_tray.set_close_to_tray_enabled(settings.close_to_tray());
        }
        VerticalSliceApp {
            bootstrap: self.bootstrap,
            store_identity: self.store_identity,
            app: OpenManicApp::new_with_theme(self.ui, theme_key.clone()),
            snapshots: self.snapshots,
            calendar_snapshots: self.calendar_snapshots,
            runtime: self.runtime,
            shutdown: self.shutdown,
            today: TodayController::new(),
            layout_editor: LayoutEditor::new(layout),
            pending_layout_save: None,
            pending_layout_navigation: None,
            theme_key,
            calendar: CalendarController::default(),
            calendar_data: PresentableData::InitialLoading,
            calendar_projection_sequence: 0,
            timeline: TimelineRenderer::new(),
            close_to_tray,
            settings_controller,
            onboarding_submission_pending: false,
            export_destination: String::new(),
            import_source: String::new(),
            backup_destination: String::new(),
            diagnostics_destination: String::new(),
            restore_source: String::new(),
            data_root_destination: String::new(),
            export_includes_titles: false,
            data_operation_message: None,
            jobs: JobsController::default(),
            projection_sequence: 1,
            requested_range: None,
            overview_range_mode: OverviewRangeMode::SevenDays,
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
    fn render_application_icon(
        &mut self,
        ui: &mut eframe::egui::Ui,
        application_id: ApplicationId,
    ) {
        let Some(icon) = self.runtime.application_icon(application_id) else {
            self.application_icon_textures.remove(&application_id);
            ui.label("□");
            return;
        };
        let texture = self
            .application_icon_textures
            .entry(application_id)
            .or_insert_with(|| {
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

    #[expect(
        clippy::too_many_lines,
        reason = "the UI ingress drain owns authoritative completion reconciliation before snapshots update."
    )]
    fn drain_worker_ingress(&mut self) {
        self.runtime
            .ui_inbox
            .drain_into(self.app.controller_mut(), UI_INBOUND_CAPACITY / 2);
        let mut restore_finished = None;
        let mut data_root_move_finished = None;
        let mut data_root_move_failed = false;
        while let Some(result) = self.runtime.take_data_operation_result() {
            let (job_id, name, message, state) = match result {
                DataOperationResult::Exported { request, outcome } => (
                    request.job_id(),
                    "CSV export",
                    format!(
                        "Exported {} rows (window titles {}).",
                        outcome.row_count(),
                        if request.title_disclosure() == TitleDisclosure::IncludeAfterConfirmation {
                            "included"
                        } else {
                            "excluded"
                        }
                    ),
                    JobState::Succeeded,
                ),
                DataOperationResult::Imported { request, outcome } => {
                    let _ = request.destination_scope();
                    (
                        request.job_id(),
                        "CSV import",
                        format!(
                            "Imported {} of {} validated rows; {} rejected.",
                            outcome.committed(),
                            outcome.parsed(),
                            outcome.rejected()
                        ),
                        JobState::Succeeded,
                    )
                }
                DataOperationResult::ImportCancelled { request } => (
                    request.job_id(),
                    "CSV import",
                    "Import cancelled before any staged records were merged.".to_owned(),
                    JobState::Cancelled,
                ),
                DataOperationResult::BackedUp { job_id } => (
                    job_id,
                    "Backup",
                    "Backup finished and was verified before completion.".to_owned(),
                    JobState::Succeeded,
                ),
                DataOperationResult::DiagnosticsExported { job_id } => (
                    job_id,
                    "Diagnostics export",
                    "Privacy-safe diagnostics bundle created.".to_owned(),
                    JobState::Succeeded,
                ),
                DataOperationResult::Restored { job_id } => {
                    restore_finished = Some(true);
                    (
                        job_id,
                        "Restore",
                        "Backup restored. Restarting local services…".to_owned(),
                        JobState::Succeeded,
                    )
                }
                DataOperationResult::RestoreFailed { job_id } => {
                    restore_finished = Some(false);
                    (
                        job_id,
                        "Restore",
                        "Backup could not be restored. Restarting the existing local services…"
                            .to_owned(),
                        JobState::Failed {
                            error: ApplicationError::port_failure(
                                ApplicationPort::Command,
                                PortFailureReason::Failed,
                            ),
                        },
                    )
                }
                DataOperationResult::DataRootMoved {
                    job_id,
                    destination,
                } => {
                    data_root_move_finished = Some(destination);
                    (
                        job_id,
                        "Data location move",
                        "Data copied and verified. Restarting local services at the new location..."
                            .to_owned(),
                        JobState::Succeeded,
                    )
                }
                DataOperationResult::DataRootMoveFailed { job_id } => {
                    data_root_move_failed = true;
                    (
                        job_id,
                        "Data location move",
                        "Data location could not be moved. Restarting existing local services..."
                            .to_owned(),
                        JobState::Failed {
                            error: ApplicationError::port_failure(
                                ApplicationPort::Command,
                                PortFailureReason::Failed,
                            ),
                        },
                    )
                }
                DataOperationResult::Failed { job_id, operation } => (
                    job_id,
                    operation,
                    format!(
                        "{operation} job {} did not complete. Try again.",
                        job_id.get()
                    ),
                    JobState::Failed {
                        error: ApplicationError::port_failure(
                            ApplicationPort::Command,
                            PortFailureReason::Failed,
                        ),
                    },
                ),
            };
            if name == "CSV import" {
                self.runtime.finish_csv_import(job_id);
            }
            self.observe_data_job(job_id, name, &state);
            self.data_operation_message = Some(message);
        }
        if let Some(restored) = restore_finished {
            self.restart_runtime_after_restore(restored);
        }
        if let Some(destination) = data_root_move_finished {
            self.restart_runtime_after_data_root_move(destination);
        } else if data_root_move_failed {
            self.restart_runtime_after_restore(false);
        }
        if let MailboxReceive::Latest(snapshot) = self.snapshots.try_receive() {
            let _ = self
                .app
                .controller_mut()
                .try_enqueue_inbound(InboundMessage::Snapshot(snapshot));
        }
        if let MailboxReceive::Latest(snapshot) = self.calendar_snapshots.try_receive() {
            self.calendar_data = PresentableData::Ready(snapshot.shared_value());
        }
    }

    fn restart_runtime_after_restore(&mut self, restored: bool) {
        let data_root = self.bootstrap.data_root().path().to_path_buf();
        if !self.runtime.close_data_operation_worker()
            || !self.runtime.close_writer()
            || !self.stop_platform()
        {
            self.data_operation_message = Some(
                "Recovery services could not restart. Quit OpenManic and reopen it before tracking."
                    .to_owned(),
            );
            return;
        }
        let store_path = data_root.join("openmanic.sqlite3");
        let store = SqliteStore::open(
            &store_path,
            &StoreOpenOptions::new(
                self.store_identity,
                utc_now_micros(),
                env!("CARGO_PKG_VERSION"),
            ),
        );
        let Ok(store) = store else {
            self.data_operation_message = Some(
                "Recovery services could not reopen the local store. Quit OpenManic and reopen it before tracking."
                    .to_owned(),
            );
            return;
        };
        let Ok((mut runtime, snapshots, calendar_snapshots)) =
            RuntimeResources::start(store, data_root)
        else {
            self.data_operation_message = Some(
                "Recovery services could not restart. Quit OpenManic and reopen it before tracking."
                    .to_owned(),
            );
            return;
        };
        #[cfg(windows)]
        if runtime.start_windows(&self.instance_owner).is_err() {
            self.data_operation_message = Some(
                "The backup result is preserved, but Windows tracking services could not restart. Quit OpenManic and reopen it before tracking."
                    .to_owned(),
            );
            return;
        }
        self.runtime = runtime;
        self.snapshots = snapshots;
        self.calendar_snapshots = calendar_snapshots;
        self.requested_range = None;
        self.calendar_projection_sequence = 0;
        self.settings_controller = self.runtime.settings_snapshot.lock().map_or_else(
            |_| SettingsController::new(SettingsSnapshot::safe_default()),
            |settings| SettingsController::new(settings.clone()),
        );
        if let Ok(settings) = self.runtime.settings_snapshot.lock() {
            self.close_to_tray
                .set_close_to_tray_enabled(settings.close_to_tray());
        }
        self.layout_editor = self.runtime.layout_snapshot.lock().map_or_else(
            |_| LayoutEditor::new(LayoutDocument::safe_default()),
            |layout| LayoutEditor::new(layout.document().clone()),
        );
        self.calendar_data = PresentableData::InitialLoading;
        self.data_operation_message = Some(if restored {
            "Backup restored and local services restarted.".to_owned()
        } else {
            "Restore failed; the existing local services restarted.".to_owned()
        });
    }

    fn restart_runtime_after_data_root_move(&mut self, destination: PathBuf) {
        if self.bootstrap.switch_to_moved_root(destination).is_err() {
            self.data_operation_message = Some(
                "Data moved but the new location could not be activated. Quit OpenManic and reopen it before tracking."
                    .to_owned(),
            );
            return;
        }
        self.restart_runtime_after_restore(true);
        self.data_operation_message =
            Some("Data location moved and local services restarted.".to_owned());
    }

    fn observe_data_job(&mut self, job_id: JobId, name: &str, state: &JobState) {
        self.jobs.observe_job(
            JobDescriptor::new(job_id, name.to_owned(), "Local data operation".to_owned()),
            state,
            None,
        );
    }

    #[expect(
        clippy::excessive_nesting,
        clippy::too_many_lines,
        reason = "job rows, cancellation, and destructive confirmation must remain adjacent to their local interaction state."
    )]
    fn render_data_jobs(&mut self, ui: &mut eframe::egui::Ui) {
        let view = self.jobs.view_model();
        if !view.jobs().is_empty() {
            ui.add_space(8.0);
            ui.label("Recent data operations");
            let mut dismiss = None;
            for job in view.jobs() {
                let (dismiss_clicked, cancel_clicked) = ui
                    .horizontal(|ui| {
                        ui.label(job.name());
                        ui.small(match job.state() {
                            JobPresentationState::Queued => "Queued",
                            JobPresentationState::Running => "Running",
                            JobPresentationState::Cancelling => "Cancelling",
                            JobPresentationState::Succeeded => "Completed",
                            JobPresentationState::Cancelled => "Cancelled",
                            JobPresentationState::Failed { message } => message,
                            JobPresentationState::Interrupted => "Interrupted",
                        });
                        let cancel_clicked = ui
                            .add_enabled(
                                job.state().can_cancel(),
                                eframe::egui::Button::new("Cancel"),
                            )
                            .clicked();
                        let dismiss_clicked = ui
                            .add_enabled(
                                job.state().can_dismiss(),
                                eframe::egui::Button::new("Dismiss"),
                            )
                            .clicked();
                        (dismiss_clicked, cancel_clicked)
                    })
                    .inner;
                if dismiss_clicked {
                    dismiss = Some(job.job_id());
                }
                if cancel_clicked
                    && matches!(
                        self.jobs.apply(JobsAction::RequestCancel {
                            job_id: job.job_id(),
                        }),
                        Some(JobsEffect::CancelRequested { .. })
                    )
                    && self.runtime.try_cancel_csv_import(job.job_id())
                {
                    self.observe_data_job(job.job_id(), job.name(), &JobState::Cancelling);
                }
            }
            if let Some(job_id) = dismiss {
                let _ = self.jobs.apply(JobsAction::Dismiss { job_id });
            }
        }
        let confirmation = view.destructive_confirmation().cloned();
        if let Some(confirmation) = confirmation {
            ui.group(|ui| {
                ui.heading(confirmation.title());
                ui.label(confirmation.scope());
                ui.horizontal(|ui| {
                    if ui.button(confirmation.confirm_label()).clicked() {
                        match self.jobs.apply(JobsAction::ConfirmDestructive {
                            action_id: confirmation.action_id().to_owned(),
                        }) {
                            Some(JobsEffect::DestructiveConfirmed { action_id })
                                if action_id == "restore-backup" =>
                            {
                                self.submit_restore();
                            }
                            Some(JobsEffect::DestructiveConfirmed { action_id })
                                if action_id == "move-data-root" =>
                            {
                                self.submit_data_root_move();
                            }
                            _ => {}
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        let _ = self.jobs.apply(JobsAction::CancelDestructiveConfirmation);
                    }
                });
            });
        }
    }

    fn submit_restore(&mut self) {
        let job_id = next_data_operation_id();
        let submitted = self
            .runtime
            .try_submit_restore(job_id, PathBuf::from(self.restore_source.trim()));
        if submitted {
            self.observe_data_job(job_id, "Restore", &JobState::Running);
            self.data_operation_message =
                Some("Restore queued; pausing local services…".to_owned());
        } else {
            self.data_operation_message = Some(
                "Restore could not be queued. Try again after current work finishes.".to_owned(),
            );
        }
    }

    fn submit_data_root_move(&mut self) {
        let Ok(locator_path) = bootstrap_locator_path() else {
            self.data_operation_message = Some(
                "Data location could not be moved because the per-user locator is unavailable."
                    .to_owned(),
            );
            return;
        };
        let job_id = next_data_operation_id();
        let submitted = self.runtime.try_submit_data_root_move(
            job_id,
            self.bootstrap.data_root().path().to_path_buf(),
            PathBuf::from(self.data_root_destination.trim()),
            locator_path,
            self.store_identity,
        );
        if submitted {
            self.observe_data_job(job_id, "Data location move", &JobState::Running);
            self.data_operation_message =
                Some("Data move queued; pausing local services...".to_owned());
        } else {
            self.data_operation_message = Some(
                "Data location could not be queued. Try again after current work finishes."
                    .to_owned(),
            );
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

    fn publish_calendar_projection(&mut self, day_offset: i32) {
        let Some(day_range) = day_range(day_offset) else {
            return;
        };
        self.calendar_projection_sequence = self.calendar_projection_sequence.saturating_add(1);
        let sequence = self.calendar_projection_sequence;
        let request = calendar_projection_request(sequence, day_range);
        self.calendar_data = PresentableData::InitialLoading;
        let _ = self.runtime.calendar_projection_requests.publish(request);
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "the calendar's immediate-mode block rendering keeps selection and schedule controls adjacent"
    )]
    fn render_calendar_dashboard(&mut self, ui: &mut eframe::egui::Ui) {
        if self.app.controller().model().route() != openmanic_ui_egui::Route::Calendar {
            return;
        }
        ui.add_space(16.0);
        openmanic_ui_egui::design::card_frame()
            .inner_margin(eframe::egui::Margin::symmetric(20, 14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if openmanic_ui_egui::design::soft_button(ui, "‹") {
                        self.apply_calendar_action(CalendarAction::PreviousDay);
                    }
                    if openmanic_ui_egui::design::soft_button(ui, "○") {
                        self.apply_calendar_action(CalendarAction::ReturnToToday);
                    }
                    let next_enabled = self
                        .calendar
                        .view_model(&self.calendar_data)
                        .next_day_enabled();
                    if ui
                        .add_enabled(
                            next_enabled,
                            eframe::egui::Button::new(
                                eframe::egui::RichText::new("›")
                                    .size(14.0)
                                    .strong()
                                    .color(openmanic_ui_egui::design::TEXT_TERTIARY),
                            )
                            .fill(openmanic_ui_egui::design::SURFACE_RAISED)
                            .stroke(eframe::egui::Stroke::new(
                                1.0,
                                openmanic_ui_egui::design::BORDER,
                            )),
                        )
                        .clicked()
                    {
                        self.apply_calendar_action(CalendarAction::NextDay);
                    }
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(
                            eframe::egui::RichText::new("Calendar")
                                .size(20.0)
                                .strong()
                                .color(openmanic_ui_egui::design::TEXT_PRIMARY),
                        );
                        ui.label(
                            eframe::egui::RichText::new("Time entries & scheduled focus")
                                .size(12.5)
                                .color(openmanic_ui_egui::design::TEXT_MUTED),
                        );
                    });
                });
            });
        ui.add_space(11.0);
        let model = self.calendar.view_model(&self.calendar_data);
        match model.state() {
            CalendarDataState::Loading => {
                ui.label("Loading Calendar…");
            }
            CalendarDataState::Empty(_) => {
                ui.label("No Calendar blocks for this day.");
            }
            CalendarDataState::Error(error) => {
                ui.label(error.message());
            }
            CalendarDataState::Ready { notice } => {
                if let Some(notice) = notice {
                    ui.label(notice);
                }
            }
        }
        for presented in model.blocks() {
            let block = presented.block();
            let accent = if presented.kind() == CalendarBlockKind::Schedule {
                openmanic_ui_egui::design::SCHEDULED
            } else {
                openmanic_ui_egui::design::category_color("development")
            };
            let fill = if presented.selected() {
                accent.gamma_multiply(0.18)
            } else {
                openmanic_ui_egui::design::SURFACE
            };
            eframe::egui::Frame::new()
                .fill(fill)
                .stroke(eframe::egui::Stroke::new(
                    1.0,
                    if presented.selected() {
                        accent
                    } else {
                        openmanic_ui_egui::design::BORDER
                    },
                ))
                .corner_radius(10.0)
                .inner_margin(eframe::egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // Left color bar in the block's accent color.
                        let (bar, _) = ui.allocate_exact_size(
                            eframe::egui::vec2(3.0, 20.0),
                            eframe::egui::Sense::hover(),
                        );
                        ui.painter().rect_filled(bar, 2.0, accent);
                        let label = format!(
                            "{:?} · {}",
                            presented.kind(),
                            format_utc_range(block.visual_range())
                        );
                        if ui
                            .add(
                                eframe::egui::Label::new(
                                    eframe::egui::RichText::new(label)
                                        .size(13.0)
                                        .strong()
                                        .color(openmanic_ui_egui::design::TEXT_SECONDARY),
                                )
                                .sense(eframe::egui::Sense::click()),
                            )
                            .clicked()
                        {
                            self.apply_calendar_action(CalendarAction::SelectBlock {
                                id: block.id(),
                            });
                        }
                        if presented.kind() == CalendarBlockKind::Schedule {
                            self.render_calendar_schedule_controls(ui, block.id());
                        }
                    });
                });
            ui.add_space(6.0);
        }
        if let Some(details) = model.selected_details() {
            ui.label(format!(
                "Selected: {:?}, {}",
                details.kind(),
                format_utc_range(details.canonical_range())
            ));
        }
        self.render_schedule_editor(ui);
        self.render_schedule_delete_confirmation(ui);
        self.render_schedule_status(ui);
    }

    fn apply_calendar_action(&mut self, action: CalendarAction) {
        let snapshot = self.calendar_data.visible_value();
        if let Some(effect) = self.calendar.apply(action, snapshot.map(AsRef::as_ref)) {
            match effect {
                CalendarEffect::RequestDay { day_offset } => {
                    self.publish_calendar_projection(day_offset);
                }
                CalendarEffect::NavigateToTimeline(_) => self
                    .app
                    .controller_mut()
                    .reduce_local(UiAction::Navigate(openmanic_ui_egui::Route::Today)),
            }
        }
    }

    fn render_calendar_schedule_controls(
        &mut self,
        ui: &mut eframe::egui::Ui,
        block_id: CalendarBlockId,
    ) {
        let Some((schedule_id, anchor_date)) = calendar_schedule_target(block_id) else {
            return;
        };
        let Some(today) = self.app.controller().model().data().visible_value() else {
            return;
        };
        let Some(schedule) = today
            .schedules()
            .iter()
            .find(|snapshot| snapshot.id() == schedule_id)
            .cloned()
        else {
            return;
        };
        if ui.button("Edit…").clicked() {
            self.schedule_draft = match (schedule.id(), anchor_date) {
                (ScheduleId::Series(series_id), Some(anchor_date)) => self
                    .calendar_data
                    .visible_value()
                    .and_then(|calendar| {
                        calendar
                            .schedule_blocks()
                            .iter()
                            .find(|block| block.id() == block_id)
                    })
                    .and_then(|block| {
                        ScheduleDraft::from_recurring(
                            schedule.clone(),
                            series_id,
                            anchor_date,
                            block.canonical_range(),
                        )
                    }),
                _ => ScheduleDraft::from_existing(schedule.clone()),
            };
        }
        if ui.button("Delete…").clicked() {
            self.schedule_delete_request = Some(ScheduleDeleteRequest {
                snapshot: schedule,
                anchor_date,
                scope: ScheduleEditScope::OnlyThisDate,
            });
        }
    }

    #[expect(
        clippy::too_many_lines,
        clippy::excessive_nesting,
        reason = "the Today view keeps widget binding and explicit edit controls together"
    )]
    fn render_today_dashboard(&mut self, ui: &mut eframe::egui::Ui) {
        if self.app.controller().model().route() != openmanic_ui_egui::Route::Today {
            return;
        }
        let tokens = self.app.theme_tokens();
        let day_offset = self
            .app
            .controller()
            .model()
            .today_view_context()
            .selected_day_offset();
        openmanic_ui_egui::design::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                if openmanic_ui_egui::design::soft_button(ui, "‹") {
                    self.app
                        .controller_mut()
                        .reduce_local(UiAction::Today(TodayAction::PreviousDay));
                }
                if openmanic_ui_egui::design::soft_button(ui, "○") {
                    self.app
                        .controller_mut()
                        .reduce_local(UiAction::Today(TodayAction::ReturnToToday));
                }
                let can_navigate_next = self.today.can_navigate_next(self.app.controller().model());
                if ui
                    .add_enabled(
                        can_navigate_next,
                        eframe::egui::Button::new(eframe::egui::RichText::new("›").size(17.0)),
                    )
                    .clicked()
                {
                    self.app
                        .controller_mut()
                        .reduce_local(UiAction::Today(TodayAction::NextDay));
                }
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    let title = match day_offset {
                        0 => "Today".to_owned(),
                        -1 => "Yesterday".to_owned(),
                        value => format!("{} days ago", value.unsigned_abs()),
                    };
                    ui.label(
                        eframe::egui::RichText::new(title)
                            .size(22.0)
                            .strong()
                            .color(openmanic_ui_egui::design::TEXT_PRIMARY),
                    );
                    ui.label(
                        eframe::egui::RichText::new("Tracked activity & personal schedule")
                            .size(13.0)
                            .color(openmanic_ui_egui::design::TEXT_MUTED),
                    );
                });
            });
        });
        ui.add_space(11.0);
        self.render_layout_editor_controls(ui);
        let data = self
            .app
            .controller()
            .model()
            .data()
            .visible_value()
            .cloned();
        let context = self.app.controller().model().today_view_context().clone();
        let Some(snapshot) = data else {
            openmanic_ui_egui::design::card_frame().show(ui, |ui| {
                ui.heading("Preparing your dashboard");
                ui.colored_label(tokens.content_secondary(), "Waiting for activity data...");
            });
            return;
        };

        let layout = self
            .layout_editor
            .draft()
            .cloned()
            .unwrap_or_else(|| self.layout_editor.active().definition());
        let bindings = self
            .today
            .widget_bindings_for_layout(self.app.controller().model(), &layout);
        let reflow = reflow_dashboard(&layout, self.today.registry(), ui.available_width());
        let column_count = f32::from(reflow.columns().count());
        let grid_gap = 14.0;
        let grid_width = ui.available_width();
        let cell_width = ((grid_width - (grid_gap * (column_count - 1.0))) / column_count).max(1.0);
        let max_row = reflow
            .placements()
            .iter()
            .map(openmanic_ui_egui::DashboardPlacement::row)
            .max()
            .unwrap_or(0);
        let grid_rows = u16::try_from(max_row.saturating_add(1)).unwrap_or(u16::MAX);
        let widget_height_for = |height| match height {
            LayoutHeight::Compact => 150.0,
            LayoutHeight::Standard => 230.0,
            LayoutHeight::Tall => 390.0,
        };
        let mut row_heights = vec![0.0_f32; usize::from(grid_rows)];
        for placement in reflow.placements() {
            let row = usize::try_from(placement.row()).unwrap_or(usize::MAX);
            if let Some(height) = row_heights.get_mut(row) {
                *height = height.max(widget_height_for(placement.height()));
            }
        }
        let grid_height =
            row_heights.iter().sum::<f32>() + grid_gap * f32::from(grid_rows.saturating_sub(1));
        let (grid_rect, _) = ui.allocate_exact_size(
            eframe::egui::vec2(grid_width, grid_height),
            eframe::egui::Sense::hover(),
        );
        for placement in reflow.placements() {
            let Some(binding) = bindings
                .widgets()
                .iter()
                .find(|binding| binding.instance().id().as_str() == placement.instance_id())
            else {
                continue;
            };
            let widget_height = widget_height_for(placement.height());
            let x = grid_rect.min.x + f32::from(placement.column()) * (cell_width + grid_gap);
            let grid_row_u16 = u16::try_from(placement.row()).unwrap_or(u16::MAX);
            let grid_row = usize::from(grid_row_u16);
            let y = grid_rect.min.y
                + row_heights.iter().take(grid_row).sum::<f32>()
                + grid_gap * f32::from(grid_row_u16);
            let widget_rect = eframe::egui::Rect::from_min_size(
                eframe::egui::pos2(x, y),
                eframe::egui::vec2(
                    f32::from(placement.span()) * cell_width
                        + f32::from(placement.span().saturating_sub(1)) * grid_gap,
                    widget_height,
                ),
            );
            ui.scope_builder(
                eframe::egui::UiBuilder::new()
                    .max_rect(widget_rect)
                    .id_salt(placement.instance_id()),
                |ui| {
                    openmanic_ui_egui::design::card_frame()
                        .inner_margin(eframe::egui::Margin::same(14))
                        .show(ui, |ui| {
                            ui.set_min_size(eframe::egui::vec2(
                                (widget_rect.width() - 28.0).max(1.0),
                                (widget_rect.height() - 28.0).max(1.0),
                            ));
                            if self.layout_editor.is_editing() {
                                ui.label(format!(
                                    "Responsive placement: row {}, column {}, span {}/{}",
                                    placement.row(),
                                    placement.column(),
                                    placement.span(),
                                    reflow.columns().count(),
                                ));
                            }
                            if self.layout_editor.is_editing() {
                                ui.horizontal(|ui| {
                                    ui.label(binding.instance().id().as_str());
                                    if ui.button("Move earlier").clicked() {
                                        let _ = self.layout_editor.apply(
                                            LayoutEditAction::MoveEarlier {
                                                instance_id: binding
                                                    .instance()
                                                    .id()
                                                    .as_str()
                                                    .to_owned(),
                                            },
                                            self.today.registry(),
                                        );
                                    }
                                    if ui.button("Move later").clicked() {
                                        let _ = self.layout_editor.apply(
                                            LayoutEditAction::MoveLater {
                                                instance_id: binding
                                                    .instance()
                                                    .id()
                                                    .as_str()
                                                    .to_owned(),
                                            },
                                            self.today.registry(),
                                        );
                                    }
                                    if ui.button("Remove").clicked() {
                                        let _ = self.layout_editor.apply(
                                            LayoutEditAction::Remove {
                                                instance_id: binding
                                                    .instance()
                                                    .id()
                                                    .as_str()
                                                    .to_owned(),
                                            },
                                            self.today.registry(),
                                        );
                                    }
                                    ui.label("Width:");
                                    for width_span in [3, 4, 6, 8, 9, 12] {
                                        if ui.button(width_span.to_string()).clicked() {
                                            let _ = self.layout_editor.apply(
                                                LayoutEditAction::Resize {
                                                    instance_id: binding
                                                        .instance()
                                                        .id()
                                                        .as_str()
                                                        .to_owned(),
                                                    width_span,
                                                },
                                                self.today.registry(),
                                            );
                                        }
                                    }
                                    ui.label("Height:");
                                    for (label, height) in [
                                        ("Compact", LayoutHeight::Compact),
                                        ("Standard", LayoutHeight::Standard),
                                        ("Tall", LayoutHeight::Tall),
                                    ] {
                                        if ui.button(label).clicked() {
                                            let _ = self.layout_editor.apply(
                                                LayoutEditAction::SetHeight {
                                                    instance_id: binding
                                                        .instance()
                                                        .id()
                                                        .as_str()
                                                        .to_owned(),
                                                    height,
                                                },
                                                self.today.registry(),
                                            );
                                        }
                                    }
                                });
                            }
                            match binding.resolution() {
                                TodayWidgetResolution::MissingRenderer => {
                                    ui.group(|ui| {
                                        ui.strong("Unavailable dashboard widget");
                                        ui.label(binding.instance().kind_id());
                                        ui.label(
                            "This widget can be removed or the layout can be reset in Edit layout.",
                        );
                                    });
                                }
                                TodayWidgetResolution::Available(definition) => {
                                    ui.horizontal(|ui| {
                                        paint_today_widget_marker(ui, definition.kind(), tokens);
                                        openmanic_ui_egui::design::section_header(
                                            ui,
                                            definition.display_name(),
                                        );
                                    });
                                    ui.label(
                                        eframe::egui::RichText::new(definition.description())
                                            .size(12.5)
                                            .color(openmanic_ui_egui::design::TEXT_FAINT),
                                    );
                                    ui.add_space(8.0);
                                    if self.layout_editor.is_editing() {
                                        ui.colored_label(
                            tokens.content_secondary(),
                            "Widget interactions are disabled while editing this layout.",
                        );
                                    } else {
                                        self.render_today_widget(
                                            ui,
                                            definition.kind(),
                                            &snapshot,
                                            &context,
                                        );
                                    }
                                }
                            }
                        });
                },
            );
        }
    }

    fn render_overview_dashboard(&mut self, ui: &mut eframe::egui::Ui) {
        use openmanic_ui_egui::design;
        if self.app.controller().model().route() != openmanic_ui_egui::Route::Overview {
            return;
        }
        let data = self
            .app
            .controller()
            .model()
            .data()
            .visible_value()
            .cloned();

        design::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        eframe::egui::RichText::new("Overview")
                            .size(22.0)
                            .strong()
                            .color(design::TEXT_PRIMARY),
                    );
                    ui.label(
                        eframe::egui::RichText::new(self.overview_range_mode.title())
                            .size(13.0)
                            .color(design::TEXT_MUTED),
                    );
                });
                ui.with_layout(
                    eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                    |ui| {
                        for mode in OverviewRangeMode::ALL.iter().rev() {
                            if design::segment_option(
                                ui,
                                mode.label(),
                                self.overview_range_mode == *mode,
                            ) {
                                self.overview_range_mode = *mode;
                            }
                        }
                    },
                );
            });
        });
        ui.add_space(11.0);

        let Some(snapshot) = data else {
            design::card_frame().show(ui, |ui| {
                ui.colored_label(design::TEXT_MUTED, "Waiting for activity data...");
            });
            return;
        };
        let usage_total_us: u64 = snapshot
            .usage()
            .applications()
            .iter()
            .map(openmanic_ui_egui::ApplicationUsage::duration_us)
            .sum();
        let application_count = snapshot.usage().applications().len();
        let known_us = snapshot.timeline().totals().known_duration_us();

        ui.columns(3, |columns| {
            render_overview_tile(
                &mut columns[0],
                "Active total",
                &format_micros_duration(usage_total_us),
                "tracked this day",
                design::TEXT_PRIMARY,
            );
            render_overview_tile(
                &mut columns[1],
                "Known coverage",
                &format_micros_duration(known_us),
                "of the visible range",
                design::ACCENT_TEXT,
            );
            render_overview_tile(
                &mut columns[2],
                "Applications",
                &application_count.to_string(),
                "seen in this range",
                design::ACTIVE,
            );
        });
        ui.add_space(11.0);

        ui.columns(2, |columns| {
            design::card_frame().show(&mut columns[0], |ui| {
                design::section_header(ui, "Category breakdown");
                ui.add_space(8.0);
                render_distribution_snapshot(ui, snapshot.distribution());
            });
            design::card_frame().show(&mut columns[1], |ui| {
                design::section_header(ui, "Apps & websites");
                ui.add_space(8.0);
                render_usage_snapshot(ui, snapshot.usage());
            });
        });
        if self.overview_range_mode != OverviewRangeMode::SevenDays {
            ui.add_space(11.0);
            design::card_frame().show(ui, |ui| {
                ui.colored_label(
                    design::TEXT_FAINT,
                    "Multi-day aggregation for this range arrives with the range projection; the selected day is shown meanwhile.",
                );
            });
        }
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "the diagnostics accordion intentionally groups its bounded read-only table"
    )]
    fn render_live_backend_diagnostics(&mut self, ui: &mut eframe::egui::Ui) {
        if self.app.controller().model().route() != openmanic_ui_egui::Route::Today {
            return;
        }
        let debug = self.runtime.tracking_debug_snapshot();
        let tracking_enabled = self.runtime.tracking_permitted.load(Ordering::Acquire);
        eframe::egui::CollapsingHeader::new("Advanced tracking diagnostics")
            .default_open(false)
            .show(ui, |ui| {
                ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.strong("Tracking ingress");
                if ui
                    .button("Exit OpenManic")
                    .on_hover_text("Stop tracking, save local data, and completely close OpenManic.")
                    .clicked()
                {
                    self.runtime.quit_requested.store(true, Ordering::Release);
                }
            });
            ui.small("This table is populated directly by the Windows tracking ingress, even when dashboard snapshots are unavailable.");
            eframe::egui::Grid::new("live-backend-diagnostics")
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Field");
                    ui.strong("Observed value");
                    ui.end_row();
                    ui.label("Tracking enabled");
                    ui.label(if tracking_enabled { "Yes" } else { "No" });
                    ui.end_row();
                    ui.label("Foreground events received");
                    ui.label(debug.foreground_events.to_string());
                    ui.end_row();
                    ui.label("Latest application identity");
                    ui.label(debug.latest_application_id.map_or_else(
                        || "None yet".to_owned(),
                        |application_id| id_label(&application_id.as_bytes()),
                    ));
                    ui.end_row();
                    ui.label("Latest event time");
                    ui.label(debug.latest_observed_at.map_or_else(
                        || "None yet".to_owned(),
                        |observed_at| format!("{} UTC", format_utc_clock(observed_at)),
                    ));
                    ui.end_row();
                    ui.label("Latest actual window title");
                    ui.label(debug.latest_window_title.as_deref().unwrap_or("None yet"));
                    ui.end_row();
                    ui.label("Latest executable");
                    ui.label(debug.latest_executable.as_deref().unwrap_or("None yet"));
                    ui.end_row();
                    ui.label("Latest resolved product name");
                    ui.label(debug.latest_product_name.as_deref().unwrap_or("None yet"));
                    ui.end_row();
                    ui.label("Writer-lane delivery");
                    ui.label(debug.last_delivery);
                    ui.end_row();
                    ui.label("Tracking debug log");
                    ui.label(
                        self.runtime
                            .tracking_debug_log_path()
                            .to_string_lossy(),
                    );
                    ui.end_row();
                });
                });
            });
    }

    #[expect(
        clippy::excessive_nesting,
        clippy::too_many_lines,
        reason = "the explicit layout editor retains its actions next to their egui controls"
    )]
    fn render_layout_editor_controls(&mut self, ui: &mut eframe::egui::Ui) {
        if let Some(route) = self.pending_layout_navigation {
            ui.group(|ui| {
                ui.strong("Unsaved layout changes");
                ui.label(format!("Discard the draft and open {}?", route.label()));
                if ui.button("Discard changes").clicked() {
                    let _ = self
                        .layout_editor
                        .apply(LayoutEditAction::Cancel, self.today.registry());
                    self.pending_layout_navigation = None;
                    self.app
                        .controller_mut()
                        .reduce_local(UiAction::Navigate(route));
                }
                if ui.button("Keep editing").clicked() {
                    self.pending_layout_navigation = None;
                }
            });
        }
        if !self.layout_editor.is_editing() {
            let tokens = self.app.theme_tokens();
            eframe::egui::Frame::new()
                .fill(tokens.panel())
                .stroke(eframe::egui::Stroke::new(1.0, tokens.timeline_grid()))
                .corner_radius(9.0)
                .inner_margin(eframe::egui::Margin::symmetric(12, 7))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(tokens.success(), "● Monitoring this device");
                        if let Some(acknowledgement) = self
                            .today
                            .tracking_acknowledgement(self.app.controller().model())
                        {
                            ui.colored_label(
                                tokens.content_secondary(),
                                tracking_status_label(acknowledgement.status()),
                            );
                        }
                        ui.with_layout(
                            eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                            |ui| {
                                if ui.button("Customize dashboard").clicked() {
                                    let _ = self
                                        .layout_editor
                                        .apply(LayoutEditAction::Begin, self.today.registry());
                                }
                                if ui.button("Resume").clicked() {
                                    self.queue_tracking_control(TrackingControlAction::Resume);
                                }
                                if ui.button("Pause").clicked() {
                                    self.queue_tracking_control(TrackingControlAction::Pause);
                                }
                            },
                        );
                    });
                });
            ui.add_space(10.0);
            return;
        }
        ui.horizontal(|ui| {
            ui.strong("Editing layout");
            if ui.button("Reset").clicked() {
                let _ = self
                    .layout_editor
                    .apply(LayoutEditAction::Reset, self.today.registry());
            }
            if ui.button("Cancel").clicked() {
                let _ = self
                    .layout_editor
                    .apply(LayoutEditAction::Cancel, self.today.registry());
            }
            if ui.button("Save").clicked()
                && let Some(LayoutEditEffect::Save(document)) = self
                    .layout_editor
                    .apply(LayoutEditAction::Save, self.today.registry())
            {
                let expected_revision = self
                    .runtime
                    .layout_snapshot
                    .lock()
                    .ok()
                    .map(|snapshot| snapshot.entity_revision());
                if self.runtime.try_submit_layout(
                    document.clone(),
                    expected_revision,
                    UtcMicros::new(utc_now_micros()),
                ) {
                    self.pending_layout_save = Some(document);
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Add widget:");
            for definition in self.today.registry().definitions() {
                if ui.button(definition.display_name()).clicked() {
                    let instance_id = format!("layout-{}", utc_now_micros());
                    let _ = self.layout_editor.apply(
                        LayoutEditAction::Add {
                            instance_id,
                            kind: definition.kind(),
                        },
                        self.today.registry(),
                    );
                }
            }
        });
    }

    #[expect(
        clippy::excessive_nesting,
        clippy::too_many_lines,
        reason = "the small built-in appearance picker keeps persistence submission beside selection"
    )]
    fn render_settings_dashboard(&mut self, ui: &mut eframe::egui::Ui) {
        if self.app.controller().model().route() != openmanic_ui_egui::Route::Settings {
            return;
        }
        use openmanic_ui_egui::design;
        ui.add_space(16.0);
        design::card_frame()
            .inner_margin(eframe::egui::Margin::symmetric(20, 14))
            .show(ui, |ui| {
                ui.label(
                    eframe::egui::RichText::new("Settings")
                        .size(22.0)
                        .strong()
                        .color(design::TEXT_PRIMARY),
                );
                ui.label(
                    eframe::egui::RichText::new(
                        "Tracking, privacy, and wellness · all data stays on this device",
                    )
                    .size(13.0)
                    .color(design::TEXT_MUTED),
                );
            });
        ui.add_space(11.0);
        let mut basic = self.settings_controller.basic().clone();
        let mut automatic = basic.start_tracking_automatically();
        let mut login = basic.start_at_login();
        let mut close_to_tray = basic.close_to_tray();
        let mut titles = basic.collect_window_titles();
        let mut notifications = basic.notifications_enabled();
        let mut sounds = basic.focus_sounds_enabled();
        let mut advanced = self.settings_controller.advanced().clone();
        let mut foreground_switch_delay = advanced.foreground_switch_delay_seconds();
        design::card_frame().show(ui, |ui| {
            design::section_header(ui, "Tracking");
            ui.label(
                eframe::egui::RichText::new("How activity is captured")
                    .size(12.5)
                    .color(design::TEXT_FAINT),
            );
            ui.add_space(8.0);
            settings_toggle_row(ui, "Start tracking after consent", None, &mut automatic);
            settings_toggle_row(ui, "Start OpenManic when I sign in", None, &mut login);
            settings_toggle_row(
                ui,
                "Keep tracking in the tray when I close the window",
                None,
                &mut close_to_tray,
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Confirm a new foreground app after:");
                ui.add(
                    eframe::egui::Slider::new(
                        &mut foreground_switch_delay,
                        MIN_FOREGROUND_SWITCH_DELAY_SECONDS..=MAX_FOREGROUND_SWITCH_DELAY_SECONDS,
                    )
                    .suffix(" seconds"),
                );
            });
            ui.label(
                eframe::egui::RichText::new(
                    "Shorter window switches are folded into the previously active application.",
                )
                .size(12.0)
                .color(design::TEXT_FAINT),
            );
        });
        ui.add_space(11.0);
        design::card_frame().show(ui, |ui| {
            design::section_header(ui, "Privacy");
            ui.label(
                eframe::egui::RichText::new("You are always in control of your data")
                    .size(12.5)
                    .color(design::TEXT_FAINT),
            );
            ui.add_space(8.0);
            settings_toggle_row(
                ui,
                "Collect window titles",
                Some(self.settings_controller.title_collection_disclosure()),
                &mut titles,
            );
            settings_toggle_row(ui, "Show notifications", None, &mut notifications);
            settings_toggle_row(ui, "Play focus sounds", None, &mut sounds);
        });
        basic.set_start_tracking_automatically(automatic);
        basic.set_start_at_login(login);
        basic.set_close_to_tray(close_to_tray);
        basic.set_collect_window_titles(titles);
        basic.set_notifications_enabled(notifications);
        basic.set_focus_sounds_enabled(sounds);
        advanced.set_foreground_switch_delay_seconds(foreground_switch_delay);
        ui.add_space(11.0);
        ui.horizontal(|ui| {
            ui.label(
                eframe::egui::RichText::new("Appearance")
                    .size(13.0)
                    .strong()
                    .color(design::TEXT_SECONDARY),
            );
            for (mode, label) in [(0_u8, "Dark"), (1, "Light"), (2, "Follow system")] {
                if design::segment_option(ui, label, false) {
                    let _ = self
                        .runtime
                        .try_submit_theme(mode, UtcMicros::new(utc_now_micros()));
                }
            }
        });
        ui.add_space(12.0);
        ui.separator();
        design::section_header(ui, "Data & Categories");
        ui.label("Export and import operate only on files you select on this device.");
        ui.horizontal(|ui| {
            ui.label("Export CSV to:");
            ui.text_edit_singleline(&mut self.export_destination);
        });
        ui.checkbox(
            &mut self.export_includes_titles,
            "I understand this export includes collected window titles",
        );
        if ui
            .add_enabled(
                !self.export_destination.trim().is_empty(),
                eframe::egui::Button::new("Export current day"),
            )
            .clicked()
            && let Some(range) = day_range(0)
        {
            let disclosure = if self.export_includes_titles {
                TitleDisclosure::IncludeAfterConfirmation
            } else {
                TitleDisclosure::Exclude
            };
            let job_id = next_data_operation_id();
            let request = CsvExportRequest::new(
                job_id,
                range,
                DataOperationDestination::new(PathBuf::from(self.export_destination.trim())),
                disclosure,
            );
            let queued = self.runtime.try_submit_csv_export(request);
            if queued {
                self.observe_data_job(job_id, "CSV export", &JobState::Running);
            }
            self.data_operation_message = Some(if queued {
                "Export queued. You can continue using OpenManic while it runs.".to_owned()
            } else {
                "Export could not be queued. Try again after current work finishes.".to_owned()
            });
        }
        ui.horizontal(|ui| {
            ui.label("Import CSV from:");
            ui.text_edit_singleline(&mut self.import_source);
        });
        ui.label("Import merges validated records into this local store; it does not replace it.");
        if ui
            .add_enabled(
                !self.import_source.trim().is_empty(),
                eframe::egui::Button::new("Import CSV"),
            )
            .clicked()
        {
            let job_id = next_data_operation_id();
            let request = CsvImportRequest::new(
                job_id,
                import_batch_id(job_id),
                DataOperationDestination::new(PathBuf::from(self.import_source.trim())),
                ImportDestinationScope::CurrentStore,
            );
            let queued = self.runtime.try_submit_csv_import(request);
            if queued {
                self.observe_data_job(job_id, "CSV import", &JobState::Running);
            }
            self.data_operation_message = Some(if queued {
                "Import queued. You can continue using OpenManic while it runs.".to_owned()
            } else {
                "Import could not be queued. Try again after current work finishes.".to_owned()
            });
        }
        ui.horizontal(|ui| {
            ui.label("Create verified backup at:");
            ui.text_edit_singleline(&mut self.backup_destination);
        });
        if ui
            .add_enabled(
                !self.backup_destination.trim().is_empty(),
                eframe::egui::Button::new("Create backup"),
            )
            .clicked()
        {
            let job_id = next_data_operation_id();
            let queued = self
                .runtime
                .try_submit_backup(job_id, PathBuf::from(self.backup_destination.trim()));
            if queued {
                self.observe_data_job(job_id, "Backup", &JobState::Running);
            }
            self.data_operation_message = Some(if queued {
                "Backup queued. The existing local data remains unchanged.".to_owned()
            } else {
                "Backup could not be queued. Try again after current work finishes.".to_owned()
            });
        }
        ui.horizontal(|ui| {
            ui.label("Restore verified backup from:");
            ui.text_edit_singleline(&mut self.restore_source);
        });
        ui.label("Restore replaces all current local data. OpenManic pauses data operations while it restarts services.");
        if ui
            .add_enabled(
                !self.restore_source.trim().is_empty(),
                eframe::egui::Button::new("Restore backup…"),
            )
            .clicked()
        {
            let _ = self.jobs.apply(JobsAction::RequestDestructiveConfirmation(
                DestructiveConfirmation::new(
                    "restore-backup".to_owned(),
                    "Restore this backup?".to_owned(),
                    "All current local OpenManic data will be replaced by the selected backup."
                        .to_owned(),
                    "Restore backup".to_owned(),
                ),
            ));
        }
        ui.horizontal(|ui| {
            ui.label("Move local data to:");
            ui.text_edit_singleline(&mut self.data_root_destination);
        });
        ui.label("The new directory must be empty. The original data remains available after a verified move.");
        if ui
            .add_enabled(
                !self.data_root_destination.trim().is_empty(),
                eframe::egui::Button::new("Move data location..."),
            )
            .clicked()
        {
            let _ = self.jobs.apply(JobsAction::RequestDestructiveConfirmation(
                DestructiveConfirmation::new(
                    "move-data-root".to_owned(),
                    "Move local data?".to_owned(),
                    "OpenManic will pause local services, verify the new copy, and retain the original data directory."
                        .to_owned(),
                    "Move data location".to_owned(),
                ),
            ));
        }
        ui.add_space(8.0);
        ui.separator();
        ui.label("Advanced diagnostics");
        ui.label("The bundle excludes application names, file paths, and window titles.");
        ui.horizontal(|ui| {
            ui.label("Create diagnostics bundle at:");
            ui.text_edit_singleline(&mut self.diagnostics_destination);
        });
        if ui
            .add_enabled(
                !self.diagnostics_destination.trim().is_empty(),
                eframe::egui::Button::new("Create diagnostics bundle"),
            )
            .clicked()
        {
            let job_id = next_data_operation_id();
            let queued = self
                .runtime
                .try_submit_diagnostics(job_id, PathBuf::from(self.diagnostics_destination.trim()));
            if queued {
                self.observe_data_job(job_id, "Diagnostics export", &JobState::Running);
            }
            self.data_operation_message = Some(if queued {
                "Diagnostics export queued.".to_owned()
            } else {
                "Diagnostics export could not be queued. Try again after current work finishes."
                    .to_owned()
            });
        }
        self.render_data_jobs(ui);
        if let Some(message) = &self.data_operation_message {
            ui.small(message);
        }
        if basic != *self.settings_controller.basic() {
            let _ = self
                .settings_controller
                .apply(SettingsAction::SetBasic(basic));
        }
        if advanced != *self.settings_controller.advanced() {
            let _ = self
                .settings_controller
                .apply(SettingsAction::SetAdvanced(advanced));
        }
        ui.horizontal(|ui| {
            if ui.button("Save settings").clicked()
                && let Some(SettingsEffect::Save {
                    settings,
                    expected_revision,
                }) = self.settings_controller.apply(SettingsAction::Save)
                && self
                    .runtime
                    .try_submit_settings(settings, Some(expected_revision))
            {
                ui.label("Saving settings...");
            }
            if ui.button("Cancel changes").clicked() {
                let _ = self.settings_controller.apply(SettingsAction::Cancel);
            }
        });
    }

    fn render_onboarding(&mut self, ui: &mut eframe::egui::Ui) {
        let settings = self
            .runtime
            .settings_snapshot
            .lock()
            .ok()
            .map_or_else(SettingsSnapshot::safe_default, |settings| settings.clone());
        if settings.consent_revision() > 0 {
            return;
        }
        ui.add_space(16.0);
        ui.heading("Welcome to OpenManic");
        ui.label(
            "Your activity data stays on this device. No account or network setup is required.",
        );
        ui.label("OpenManic records the foreground application after you continue. Window titles remain off by default.");
        ui.label(
            "You can pause tracking or change privacy, startup, and data settings at any time.",
        );
        if self.onboarding_submission_pending {
            ui.label("Saving your local tracking choice...");
            return;
        }
        if ui.button("Accept defaults and start tracking").clicked() {
            let accepted = SettingsSnapshot::new(
                1,
                settings.start_tracking_automatically(),
                settings.start_at_login(),
                settings.close_to_tray(),
                settings.idle_threshold_seconds(),
                settings.foreground_switch_delay_seconds(),
                settings.idle_policy_code(),
                settings.collect_window_titles(),
                settings.time_zone_mode(),
                settings.manual_time_zone_id().map(str::to_owned),
                settings.theme_mode(),
                settings.density_code(),
                settings.notifications_enabled(),
                settings.focus_sounds_enabled(),
                settings.tray_explanation_acknowledged(),
                settings.revision(),
            );
            if self.runtime.try_submit_settings(accepted, None) {
                self.onboarding_submission_pending = true;
            }
        }
    }

    fn reconcile_persisted_appearance(&mut self, context: &eframe::egui::Context) {
        let Some(theme_key) = self.runtime.theme_mode.lock().ok().map(|mode| match *mode {
            1 => "openmanic.light",
            2 => "openmanic.system",
            _ => "openmanic.dark",
        }) else {
            return;
        };
        if theme_key != self.theme_key && self.app.apply_theme(context, theme_key, true).is_ok() {
            self.theme_key = theme_key.to_owned();
        }
    }

    fn reconcile_persisted_layout(&mut self) {
        let Some(pending) = self.pending_layout_save.as_ref() else {
            return;
        };
        let Ok(snapshot) = self.runtime.layout_snapshot.lock() else {
            return;
        };
        if snapshot.document() == pending {
            self.layout_editor.confirm_saved(pending.clone());
            self.pending_layout_save = None;
        }
    }

    fn request_layout_aware_navigation(&mut self, route: openmanic_ui_egui::Route) {
        if self.layout_editor.is_editing() {
            self.pending_layout_navigation = Some(route);
        } else {
            self.app
                .controller_mut()
                .reduce_local(UiAction::Navigate(route));
        }
    }

    fn render_today_widget(
        &mut self,
        ui: &mut eframe::egui::Ui,
        kind: TodayWidgetKind,
        snapshot: &TodaySnapshot,
        context: &openmanic_ui_egui::TodayViewContext,
    ) {
        if kind == TodayWidgetKind::TIMELINE {
            self.render_timeline_widget(ui, snapshot, context);
        } else if kind == TodayWidgetKind::APPLICATION_USAGE {
            render_usage_snapshot(ui, snapshot.usage());
        } else if kind == TodayWidgetKind::TIME_DISTRIBUTION {
            render_distribution_snapshot(ui, snapshot.distribution());
        } else if kind == TodayWidgetKind::FOCUS {
            self.render_focus_controls(ui);
        }
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "the compact toolbar keeps its two timeline controls together"
    )]
    fn render_timeline_widget(
        &mut self,
        ui: &mut eframe::egui::Ui,
        snapshot: &TodaySnapshot,
        context: &openmanic_ui_egui::TodayViewContext,
    ) {
        ui.horizontal(|ui| {
            ui.colored_label(
                self.app.theme_tokens().content_secondary(),
                "Wheel to zoom; drag the overview window to pan",
            );
            ui.with_layout(
                eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                |ui| {
                    if ui.button("Reset view").clicked() && self.timeline.reset_view() {
                        ui.ctx().request_repaint();
                    }
                    ui.toggle_value(&mut self.create_schedule_mode, "Add schedule");
                },
            );
        });
        if self.create_schedule_mode {
            ui.colored_label(
                self.app.theme_tokens().interaction_primary(),
                "Drag across the timeline to choose exact start and end times.",
            );
        }
        self.timeline.set_theme_tokens(self.app.theme_tokens());
        self.timeline.set_category_labels(
            snapshot
                .categories()
                .iter()
                .map(|category| (category.id(), category.name().as_str().to_owned())),
        );
        self.timeline.set_application_labels(
            snapshot
                .applications()
                .iter()
                .map(|(application, _)| (application.id(), application.name().as_str().to_owned())),
        );
        let output = self.timeline.show_snapshot(
            ui,
            snapshot.timeline(),
            context,
            self.create_schedule_mode,
        );
        for action in output.actions().iter().copied() {
            match action {
                TimelineRenderAction::Today(action) => {
                    self.app
                        .controller_mut()
                        .reduce_local(UiAction::Today(action));
                }
                TimelineRenderAction::ViewRangeChanged { .. } => ui.ctx().request_repaint(),
                TimelineRenderAction::ScheduleRequested { range } => {
                    self.schedule_draft = Some(ScheduleDraft::new(range));
                }
                TimelineRenderAction::OpenCategories { application_id } => {
                    self.selected_category_applications.clear();
                    self.selected_category_applications.insert(application_id);
                    self.request_layout_aware_navigation(openmanic_ui_egui::Route::Categories);
                }
            }
        }
        self.render_schedule_editor(ui);
        self.render_existing_schedule_controls(ui, snapshot);
    }

    #[expect(
        clippy::too_many_lines,
        clippy::excessive_nesting,
        reason = "the Categories screen keeps its interactive controls together to preserve one coherent selection state."
    )]
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
        openmanic_ui_egui::design::card_frame()
            .inner_margin(eframe::egui::Margin::symmetric(20, 14))
            .show(ui, |ui| {
                ui.label(
                    eframe::egui::RichText::new("Categories")
                        .size(22.0)
                        .strong()
                        .color(openmanic_ui_egui::design::TEXT_PRIMARY),
                );
                ui.label(
                    eframe::egui::RichText::new("Map applications to categories")
                        .size(13.0)
                        .color(openmanic_ui_egui::design::TEXT_MUTED),
                );
            });
        ui.add_space(11.0);
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
                        category
                            .name()
                            .as_str()
                            .clone_into(&mut self.category_rename_name);
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
                        .add_enabled(
                            replacement.is_some(),
                            eframe::egui::Button::new("Save name"),
                        )
                        .clicked()
                        && let Some(name) = replacement
                        && let Some(command_id) = self.runtime.try_submit_catalog(
                            self.runtime.identifiers.catalog_command(
                                CatalogCommand::RenameCategory {
                                    category_id,
                                    name,
                                    observed_at_utc: UtcMicros::new(utc_now_micros()),
                                },
                            ),
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
            ui.checkbox(&mut self.show_excluded_applications_only, "Excluded only");
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
                .add_enabled(has_selection, eframe::egui::Button::new(assignment_label))
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
            if !search_matches
                || !category_matches
                || (self.show_excluded_applications_only && !excluded)
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
                let accent =
                    openmanic_ui_egui::design::application_brand_color(application.name().as_str())
                        .unwrap_or(openmanic_ui_egui::design::UNKNOWN);
                openmanic_ui_egui::design::color_dot(ui, accent, 12.0);
                let mut selected = self
                    .selected_category_applications
                    .contains(&application_id);
                if ui
                    .checkbox(&mut selected, application.name().as_str())
                    .changed()
                {
                    if selected {
                        self.selected_category_applications.insert(application_id);
                    } else {
                        self.selected_category_applications.remove(&application_id);
                    }
                }
                if category_name == "Uncategorized" {
                    ui.label(
                        eframe::egui::RichText::new("Untracked")
                            .size(12.5)
                            .italics()
                            .color(openmanic_ui_egui::design::TEXT_FAINT),
                    );
                } else {
                    openmanic_ui_egui::design::percent_pill(
                        ui,
                        category_name,
                        openmanic_ui_egui::design::category_color(category_name),
                    );
                }
                if *excluded {
                    ui.label(
                        eframe::egui::RichText::new("Excluded from future tracking")
                            .size(12.0)
                            .color(openmanic_ui_egui::design::AWAY),
                    );
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

    #[expect(
        clippy::too_many_lines,
        clippy::excessive_nesting,
        reason = "focus controls intentionally render every state and recovery action from one immutable snapshot."
    )]
    fn render_focus_controls(&mut self, ui: &mut eframe::egui::Ui) {
        if self
            .runtime
            .focus_completion_pending
            .swap(false, Ordering::AcqRel)
        {
            eframe::egui::Frame::new()
                .fill(self.app.theme_tokens().success().gamma_multiply(0.12))
                .stroke(eframe::egui::Stroke::new(
                    1.0,
                    self.app.theme_tokens().success(),
                ))
                .corner_radius(8.0)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.strong("Focus session complete");
                    ui.label("Your completed focus session has been saved.");
                });
        }
        let snapshot = self
            .runtime
            .focus_snapshot
            .lock()
            .ok()
            .and_then(|latest| latest.clone());
        if let Some(snapshot) = snapshot.as_ref() {
            self.latest_focus_session = Some(snapshot.session_id());
        }

        let tokens = self.app.theme_tokens();
        eframe::egui::Frame::new()
            .fill(tokens.canvas())
            .stroke(eframe::egui::Stroke::new(1.0, tokens.timeline_grid()))
            .corner_radius(9.0)
            .inner_margin(eframe::egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                match snapshot.as_ref().map(|snapshot| snapshot.session().state()) {
                    Some(FocusSessionState::Ready | FocusSessionState::Planned { .. }) => {
                        ui.vertical_centered(|ui| {
                            ui.label(
                                eframe::egui::RichText::new("25:00")
                                    .monospace()
                                    .size(30.0)
                                    .strong(),
                            );
                            ui.colored_label(
                                self.app.theme_tokens().content_secondary(),
                                "Ready for a focus session",
                            );
                        });
                        if let Some(snapshot) = snapshot.as_ref()
                            && ui
                                .vertical_centered(|ui| {
                                    ui.add_sized(
                                        [ui.available_width().min(220.0), 28.0],
                                        eframe::egui::Button::new(
                                            eframe::egui::RichText::new("Start focus").strong(),
                                        )
                                        .fill(tokens.interaction_primary())
                                        .corner_radius(7.0),
                                    )
                                })
                                .inner
                                .clicked()
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
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    eframe::egui::RichText::new(focus_remaining_label(
                                        remaining_us,
                                    ))
                                    .monospace()
                                    .size(30.0)
                                    .strong(),
                                );
                                ui.colored_label(
                                    self.app.theme_tokens().success(),
                                    "Focus session active",
                                );
                            });
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
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    eframe::egui::RichText::new(focus_remaining_label(
                                        remaining_us,
                                    ))
                                    .monospace()
                                    .size(30.0)
                                    .strong(),
                                );
                                ui.colored_label(self.app.theme_tokens().warning(), "Focus paused");
                            });
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
                        ui.vertical_centered(|ui| {
                            ui.label(
                                eframe::egui::RichText::new("25:00")
                                    .monospace()
                                    .size(30.0)
                                    .strong(),
                            );
                            ui.colored_label(
                                self.app.theme_tokens().content_secondary(),
                                "No focus session prepared",
                            );
                        });
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
                    && ui
                        .vertical_centered(|ui| {
                            ui.add_sized(
                                [ui.available_width().min(240.0), 28.0],
                                eframe::egui::Button::new(
                                    eframe::egui::RichText::new("Set up 25-minute focus").strong(),
                                )
                                .fill(tokens.interaction_primary())
                                .corner_radius(7.0),
                            )
                        })
                        .inner
                        .clicked()
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

    #[expect(
        clippy::excessive_nesting,
        reason = "the compact control strip intentionally keeps each action beside its rendered button."
    )]
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
        let Some((action, range, label, repeats, weekday_mask, existing)) = schedule_draft_action
        else {
            self.render_schedule_status(ui);
            return;
        };
        let submission = match action {
            ScheduleDraftAction::Save => Some(
                if let Some(recurring) = self
                    .schedule_draft
                    .as_ref()
                    .and_then(|draft| draft.recurring.clone())
                {
                    self.queue_recurring_schedule_edit(recurring, range, &label, weekday_mask)
                } else if let Some(existing) = existing {
                    self.queue_schedule_replacement(existing, range, &label, repeats, weekday_mask)
                } else {
                    self.queue_schedule_draft(range, &label, repeats, weekday_mask)
                },
            ),
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

    #[expect(
        clippy::excessive_nesting,
        reason = "each occurrence retains its own edit and delete controls with the immutable occurrence identity."
    )]
    fn render_existing_schedule_controls(
        &mut self,
        ui: &mut eframe::egui::Ui,
        today: &TodaySnapshot,
    ) {
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
                        "{} - {}",
                        schedule.rule().label(),
                        format_utc_range(interval)
                    ));
                    if ui.button("Delete…").clicked() {
                        self.schedule_delete_request = Some(ScheduleDeleteRequest {
                            snapshot: schedule.clone(),
                            anchor_date,
                            scope: ScheduleEditScope::OnlyThisDate,
                        });
                    }
                    if !schedule.rule().is_repeating() && ui.button("Edit…").clicked() {
                        self.schedule_draft = ScheduleDraft::from_existing(schedule.clone());
                    }
                    if schedule.rule().is_repeating()
                        && let (ScheduleId::Series(series_id), Some(anchor_date)) =
                            (schedule.id(), anchor_date)
                        && ui.button("Edit…").clicked()
                        && let Some(draft) = ScheduleDraft::from_recurring(
                            schedule,
                            series_id,
                            anchor_date,
                            interval,
                        )
                    {
                        self.schedule_draft = Some(draft);
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
            (ScheduleId::Series(series_id), Some(anchor_date)) => {
                self.runtime.identifiers.schedule_delete_occurrence(
                    series_id,
                    anchor_date,
                    request.scope,
                    request.snapshot.entity_revision(),
                )
            }
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
        let command =
            self.runtime
                .identifiers
                .schedule_create(range, label, repeats, weekday_mask)?;
        self.runtime
            .try_submit_schedule(command)
            .ok_or(ScheduleDraftValidationError::QueueUnavailable)
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the queued replacement keeps an owned immutable snapshot until command construction completes."
    )]
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
            ScheduleEditScope::OnlyThisDate => {
                RecurringScheduleEdit::OnlyThisDate(RecurringOccurrenceOverride {
                    interval: range,
                    start_after_gap: false,
                    start_earlier_fold: false,
                    end_after_gap: false,
                    end_earlier_fold: false,
                })
            }
            ScheduleEditScope::ThisAndFuture | ScheduleEditScope::EveryOccurrence => {
                RecurringScheduleEdit::Rule(RecurringScheduleRuleChange {
                    label: label.to_owned(),
                    category_id: recurring.category_id,
                    weekday_mask,
                    start_second_of_day: recurring.start_second_of_day,
                    end_second_of_day: recurring.end_second_of_day,
                    time_zone_id: recurring.time_zone_id,
                })
            }
        };
        self.runtime
            .try_submit_schedule(self.runtime.identifiers.schedule_edit_occurrence(
                recurring.series_id,
                recurring.anchor_date,
                recurring.scope,
                recurring.expected_revision,
                edit,
            ))
            .ok_or(ScheduleDraftValidationError::QueueUnavailable)
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
        // A completed coordinated quit closes the viewport to release the process-owned
        // BootstrapState. Do not reinterpret that close request as a user preference to hide
        // the still-running application in the tray, or its data-root lock would remain held.
        if !matches!(self.shutdown.phase(), ShutdownPhase::Running) {
            return;
        }
        let tracking_enabled = self.runtime.settings_snapshot.lock().is_ok_and(|settings| {
            settings.consent_revision() > 0 && settings.start_tracking_automatically()
        });
        match self.close_to_tray.on_main_window_close(tracking_enabled) {
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
                ShutdownStep::CancelSafeReads | ShutdownStep::FlushSettings => true,
                ShutdownStep::JoinReadersAndWorkers => self.runtime.close_data_operation_worker(),
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

    fn render_shutdown_recovery(&mut self, context: &eframe::egui::Context) {
        let controller = ShutdownController;
        let phase = self.shutdown.phase();
        let Some(view_model) = controller.failure_view_model(phase) else {
            return;
        };
        let Some(action) = render_shutdown_failure(context, view_model) else {
            return;
        };
        match controller.apply(phase, action) {
            Some(ShutdownEffect::RetryCriticalFlush) => {
                let _ = self.shutdown.retry_critical_flush();
                self.begin_shutdown(context);
            }
            Some(ShutdownEffect::QuitAnyway) => {
                let _ = self.shutdown.quit_anyway();
                self.begin_shutdown(context);
            }
            None => {}
        }
    }
}

fn calendar_schedule_target(block_id: CalendarBlockId) -> Option<(ScheduleId, Option<i32>)> {
    let CalendarBlockId::Schedule(occurrence_id) = block_id else {
        return None;
    };
    Some(match occurrence_id {
        ScheduleOccurrenceId::OneTime(id) => (id, None),
        ScheduleOccurrenceId::Recurring {
            schedule_id,
            anchor_date,
        } => (schedule_id, Some(anchor_date)),
    })
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

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the draft owns the snapshot-derived edit metadata and callers intentionally transfer its immutable input."
    )]
    fn from_recurring(
        snapshot: ScheduleSnapshot,
        series_id: ScheduleSeriesId,
        anchor_date: i32,
        range: HalfOpenInterval,
    ) -> Option<Self> {
        let segment = snapshot.rule().segments().into_iter().find(|segment| {
            segment.effective_start_date() <= anchor_date
                && segment
                    .effective_end_date()
                    .is_none_or(|end| anchor_date <= end)
        })?;
        Some(Self {
            range,
            label: segment.label().to_owned(),
            repeats: true,
            weekday_mask: segment.weekday_mask(),
            validation_error: None,
            existing: None,
            recurring: Some(RecurringScheduleEditRequest {
                series_id,
                anchor_date,
                scope: ScheduleEditScope::OnlyThisDate,
                expected_revision: snapshot.entity_revision(),
                category_id: segment.category_id(),
                start_second_of_day: segment.start_second_of_day(),
                end_second_of_day: segment.end_second_of_day(),
                time_zone_id: segment.time_zone_id().to_owned(),
            }),
        })
    }
}

#[derive(Clone, Debug)]
struct RecurringScheduleEditRequest {
    series_id: ScheduleSeriesId,
    anchor_date: i32,
    scope: ScheduleEditScope,
    expected_revision: EntityRevision,
    category_id: Option<CategoryId>,
    start_second_of_day: u32,
    end_second_of_day: u32,
    time_zone_id: String,
}

/// Range selection for the Overview analytics header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OverviewRangeMode {
    /// The last seven days.
    SevenDays,
    /// The last twelve weeks.
    Weeks,
    /// The last twelve months.
    Months,
}

impl OverviewRangeMode {
    const ALL: [Self; 3] = [Self::SevenDays, Self::Weeks, Self::Months];

    const fn label(self) -> &'static str {
        match self {
            Self::SevenDays => "7 Days",
            Self::Weeks => "Weeks",
            Self::Months => "Months",
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::SevenDays => "Last 7 days",
            Self::Weeks => "Last 12 weeks",
            Self::Months => "Last 12 months",
        }
    }
}

fn render_overview_tile(
    ui: &mut eframe::egui::Ui,
    label: &str,
    value: &str,
    sub: &str,
    value_color: eframe::egui::Color32,
) {
    use openmanic_ui_egui::design;
    design::card_frame()
        .inner_margin(eframe::egui::Margin::symmetric(18, 15))
        .show(ui, |ui| {
            ui.label(
                eframe::egui::RichText::new(label.to_uppercase())
                    .size(10.5)
                    .strong()
                    .color(design::TEXT_MUTED),
            );
            ui.add_space(6.0);
            ui.label(
                eframe::egui::RichText::new(value)
                    .size(26.0)
                    .monospace()
                    .color(value_color),
            );
            ui.add_space(6.0);
            ui.label(
                eframe::egui::RichText::new(sub)
                    .size(11.5)
                    .strong()
                    .color(design::TEXT_FAINT),
            );
        });
}

fn settings_toggle_row(
    ui: &mut eframe::egui::Ui,
    label: &str,
    description: Option<&str>,
    value: &mut bool,
) {
    use openmanic_ui_egui::design;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(
                eframe::egui::RichText::new(label)
                    .size(14.0)
                    .strong()
                    .color(design::TEXT_SECONDARY),
            );
            if let Some(description) = description {
                ui.label(
                    eframe::egui::RichText::new(description)
                        .size(12.0)
                        .color(design::TEXT_FAINT),
                );
            }
        });
        ui.with_layout(
            eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
            |ui| {
                let _ = design::toggle_switch(ui, value);
            },
        );
    });
    // Hairline separator between rows.
    let (line, _) = ui.allocate_exact_size(
        eframe::egui::vec2(ui.available_width(), 1.0),
        eframe::egui::Sense::hover(),
    );
    ui.painter().rect_filled(line, 0.0, design::HAIRLINE);
    ui.add_space(6.0);
}

fn format_micros_duration(duration_us: u64) -> String {
    let total_seconds = duration_us / 1_000_000;
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

fn render_schedule_draft(
    ui: &mut eframe::egui::Ui,
    draft: &mut ScheduleDraft,
) -> ScheduleDraftAction {
    ui.group(|ui| {
        ui.strong(if draft.existing.is_some() { "Edit schedule" } else { "New schedule" });
        ui.label(format!(
            "Provisional range: {}",
            format_utc_range(draft.range)
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
            if ui
                .selectable_label(*weekday_mask & bit != 0, *label)
                .clicked()
            {
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
        self.reconcile_persisted_appearance(ui.ctx());
        self.reconcile_persisted_layout();
        let route_before_shell = self.app.controller().model().route();
        eframe::App::ui(&mut self.app, ui, frame);
        let route_after_shell = self.app.controller().model().route();
        if self.layout_editor.is_editing() && route_after_shell != route_before_shell {
            self.pending_layout_navigation = Some(route_after_shell);
            self.app
                .controller_mut()
                .reduce_local(UiAction::Navigate(route_before_shell));
        }
        eframe::egui::ScrollArea::vertical()
            .id_salt("openmanic-dashboard-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| match route_after_shell {
                openmanic_ui_egui::Route::Today => {
                    self.render_today_dashboard(ui);
                    ui.add_space(8.0);
                    self.render_live_backend_diagnostics(ui);
                }
                openmanic_ui_egui::Route::Overview => self.render_overview_dashboard(ui),
                openmanic_ui_egui::Route::Categories => self.render_categories_dashboard(ui),
                openmanic_ui_egui::Route::Calendar => self.render_calendar_dashboard(ui),
                openmanic_ui_egui::Route::Settings => self.render_settings_dashboard(ui),
            });
        self.render_onboarding(ui);
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
        self.render_shutdown_recovery(&context);
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
    let locator_path = bootstrap_locator_path()?;
    let locator = load_locator(&locator_path).map_err(CompositionError::Locator)?;
    let validator = LocalDataRootValidator::new(RejectKnownNetworkShares);
    let environment_root = std::env::var_os("OPENMANIC_DATA_DIR").map(PathBuf::from);
    let disposition = bootstrap(
        &cli,
        environment_root,
        locator,
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

fn bootstrap_locator_path() -> Result<PathBuf, CompositionError> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|directory| directory.join("OpenManic").join("bootstrap.locator"))
        .ok_or(CompositionError::Storage)
}

fn utc_now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
        })
}

fn initial_category_id(index: u8) -> CategoryId {
    let mut bytes = [0_u8; 16];
    bytes[..5].copy_from_slice(b"OMCAT");
    bytes[5] = index;
    bytes[15] = 0xa1;
    CategoryId::from_bytes(bytes)
}

fn initial_category_for_application(name: &str) -> Option<CategoryId> {
    let name = name.trim().to_ascii_lowercase();
    let category_index = if contains_any(
        &name,
        &[
            "terminal",
            "powershell",
            "command prompt",
            "visual studio",
            "vs code",
            "jetbrains",
            "github desktop",
        ],
    ) {
        0
    } else if contains_any(&name, &["discord", "slack", "teams", "zoom"]) {
        1
    } else if contains_any(
        &name,
        &[
            "figma",
            "photoshop",
            "illustrator",
            "blender",
            "davinci resolve",
        ],
    ) {
        2
    } else if contains_any(&name, &["spotify", "mpv", "vlc", "steam", "media player"]) {
        3
    } else if contains_any(
        &name,
        &[
            "google chrome",
            "chrome",
            "firefox",
            "microsoft edge",
            "brave",
        ],
    ) {
        4
    } else if contains_any(&name, &["chatgpt", "claude", "gemini"]) {
        5
    } else if contains_any(
        &name,
        &[
            "microsoft word",
            "winword",
            "excel",
            "powerpoint",
            "notion",
            "obsidian",
        ],
    ) {
        6
    } else if contains_any(
        &name,
        &["keepass", "1password", "file explorer", "task manager"],
    ) {
        7
    } else {
        return None;
    };
    Some(initial_category_id(category_index))
}

fn contains_any(value: &str, candidates: &[&str]) -> bool {
    candidates.iter().any(|candidate| value.contains(candidate))
}

fn seed_initial_categories(store: &mut SqliteStore) -> Result<(), CompositionError> {
    let applications = {
        let mut reader = store
            .open_read_session()
            .map_err(|_| CompositionError::Storage)?;
        let snapshot = reader.snapshot().map_err(|_| CompositionError::Storage)?;
        if !snapshot.categories().is_empty() {
            return Ok(());
        }
        snapshot
            .applications()
            .iter()
            .map(|record| record.application().clone())
            .collect::<Vec<_>>()
    };
    let observed_at = UtcMicros::new(utc_now_micros());
    for (index, name) in INITIAL_CATEGORY_NAMES.iter().enumerate() {
        let index = u8::try_from(index).map_err(|_| CompositionError::Storage)?;
        let name = CategoryName::try_new(name).map_err(|_| CompositionError::Storage)?;
        store
            .writer()
            .create_category(
                &Category::new(initial_category_id(index), name),
                observed_at,
            )
            .map_err(|_| CompositionError::Storage)?;
    }
    let mut assignments = BTreeMap::<CategoryId, Vec<ApplicationId>>::new();
    for application in applications {
        if let Some(category_id) = initial_category_for_application(application.name().as_str()) {
            assignments
                .entry(category_id)
                .or_default()
                .push(application.id());
        }
    }
    for (category_id, application_ids) in assignments {
        <StorageWriter as CatalogPersistence>::assign_applications(
            store.writer(),
            &application_ids,
            Some(category_id),
        )
        .map_err(|_| CompositionError::Storage)?;
    }
    Ok(())
}

/// Fills a brand-new local store with a small, deterministic activity story.
///
/// The seed is deliberately idempotent: as soon as the store has either an
/// application or recorded activity, OpenManic leaves the user's data alone.
/// Keeping it at the composition boundary also means every dashboard is fed by
/// ordinary persisted projections rather than a UI-only fixture.
fn seed_demo_data(store: &mut SqliteStore) -> Result<(), CompositionError> {
    let is_empty = {
        let mut reader = store
            .open_read_session()
            .map_err(|_| CompositionError::Storage)?;
        let snapshot = reader.snapshot().map_err(|_| CompositionError::Storage)?;
        snapshot.applications().is_empty() && snapshot.activities().is_empty()
    };
    if !is_empty {
        return Ok(());
    }

    const DEMO_APPLICATIONS: [(&str, u8, u8); 7] = [
        ("Visual Studio Code", 0, 1),
        ("Slack", 1, 2),
        ("Google Chrome", 4, 3),
        ("Figma", 2, 4),
        ("ChatGPT", 5, 5),
        ("Notion", 6, 6),
        ("Spotify", 3, 7),
    ];
    let now = utc_now_micros();
    let observed_at = UtcMicros::new(now);
    let mut application_ids = BTreeMap::new();
    for (name, category_index, id_byte) in DEMO_APPLICATIONS {
        let application_id = demo_application_id(id_byte);
        let application = Application::try_new(
            application_id,
            ApplicationName::try_new(name).map_err(|_| CompositionError::Storage)?,
            Some(initial_category_id(category_index)),
            UtcMicros::new(now.saturating_sub(6 * 60 * 60 * 1_000_000)),
            observed_at,
        )
        .map_err(|_| CompositionError::Storage)?;
        store
            .writer()
            .upsert_application(&application)
            .map_err(|_| CompositionError::Storage)?;
        application_ids.insert(id_byte, application_id);
    }

    let run_id = demo_tracker_run_id();
    let run_start = UtcMicros::new(now.saturating_sub(6 * 60 * 60 * 1_000_000));
    let registration = TrackerRunRegistration::try_new(run_id, run_start, "openmanic-demo-v1")
        .map_err(|_| CompositionError::Storage)?;
    store
        .writer()
        .register_tracker_run(&registration)
        .map_err(|_| CompositionError::Storage)?;

    let active_evidence = ActivityEvidence::try_from_cause(ActivityCause::ForegroundApplication)
        .map_err(|_| CompositionError::Storage)?;
    let mut cursor = run_start.get();
    let mut intervals = Vec::new();
    for (application_byte, minutes) in [
        (1_u8, 72_i64),
        (2, 18),
        (3, 46),
        (4, 51),
        (5, 31),
        (6, 42),
        (7, 19),
        (1, 34),
    ] {
        let end = cursor.saturating_add(minutes.saturating_mul(60 * 1_000_000));
        let application_id = application_ids
            .get(&application_byte)
            .copied()
            .ok_or(CompositionError::Storage)?;
        let range = HalfOpenInterval::try_new(UtcMicros::new(cursor), UtcMicros::new(end))
            .map_err(|_| CompositionError::Storage)?;
        let interval = ActivityInterval::try_new(
            run_id,
            range,
            ActivityState::Active,
            active_evidence,
            Some(application_id),
        )
        .map_err(|_| CompositionError::Storage)?;
        intervals.push(interval);
        // Short intentional gaps make the timeline's unknown/coverage states visible.
        cursor = end.saturating_add(6 * 60 * 1_000_000);
    }
    let checkpoint_start = cursor.max(now.saturating_sub(30 * 1_000_000));
    let checkpoint = TrackingCheckpoint::try_new(
        run_id,
        UtcMicros::new(checkpoint_start),
        UtcMicros::new(checkpoint_start),
        ActivityState::Unavailable,
        ActivityEvidence::try_from_cause(ActivityCause::AdapterStarting)
            .map_err(|_| CompositionError::Storage)?,
        None,
        1,
    )
    .map_err(|_| CompositionError::Storage)?;
    let intent = TrackingPersistenceIntent::try_new(intervals, checkpoint)
        .map_err(|_| CompositionError::Storage)?;
    store
        .writer()
        .persist_tracking(&intent)
        .map_err(|_| CompositionError::Storage)?;
    Ok(())
}

fn demo_application_id(value: u8) -> ApplicationId {
    let mut bytes = [0_u8; 16];
    bytes[..5].copy_from_slice(b"OMAPP");
    bytes[5] = value;
    bytes[15] = 0xd1;
    ApplicationId::from_bytes(bytes)
}

fn demo_tracker_run_id() -> TrackerRunId {
    let mut bytes = [0_u8; 16];
    bytes[..5].copy_from_slice(b"OMRUN");
    bytes[15] = 0xd1;
    TrackerRunId::from_bytes(bytes)
}

fn preserve_or_classify_application(
    store: &mut SqliteStore,
    application: Application,
) -> Application {
    let resolved_category = store
        .open_read_session()
        .ok()
        .and_then(|mut reader| reader.snapshot().ok())
        .map_or(application.category_id(), |snapshot| {
            let existing = snapshot
                .applications()
                .iter()
                .find(|record| record.application().id() == application.id());
            if let Some(existing) = existing {
                return existing.application().category_id();
            }
            let suggested = initial_category_for_application(application.name().as_str());
            suggested.filter(|category_id| {
                snapshot
                    .categories()
                    .iter()
                    .any(|record| record.category().id() == *category_id)
            })
        });
    if resolved_category == application.category_id() {
        return application;
    }
    Application::try_new(
        application.id(),
        application.name().clone(),
        resolved_category,
        application.first_seen(),
        application.last_seen(),
    )
    .unwrap_or(application)
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

fn calendar_projection_request(
    sequence: u64,
    range: HalfOpenInterval,
) -> ProjectionRequest<CalendarDayContext> {
    let context_key = ProjectionContextKey::new(sequence);
    ProjectionRequest::new(
        openmanic_application::RequestId::new(sequence),
        ProjectionSlot::new(2),
        context_key,
        DataRevision::new(0),
        CalendarDayContext::new(context_key, range),
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
    use std::{
        fs,
        sync::{Arc, Mutex},
    };

    use openmanic_application::{
        CalendarBlockId, CatalogCommand, EntityRevision, LaneCapacities, LaneReceive,
        ScheduleCommand, ScheduleId, ScheduleOccurrenceId, ScheduleSnapshot, TrackingCommand,
        TrackingEvidence, WorkLane, bounded_runtime_lanes,
    };
    use openmanic_domain::{
        Application, ApplicationId, ApplicationName, FocusSessionState, HalfOpenInterval,
        ScheduleEditScope, ScheduleRule, ScheduleSeriesId, UtcMicros,
    };
    use openmanic_platform::WindowsPlatformAction;

    use super::{
        CommandIdentifiers, ForegroundSwitchDecision, ForegroundSwitchStabilizer,
        PlatformActionRouter, ScheduleDraft, ScheduleDraftValidationError, TrackingDebugState,
        UiInbox, UiIngress, calendar_schedule_target, day_range, focus_remaining_label,
        focus_remaining_us, foreground_observed_at, format_utc_clock, format_utc_range,
        initial_category_for_application, initial_category_id, record_tracking_observation,
        recurring_schedule_rule, set_tracking_delivery, store_identity,
    };

    fn foreground_application(value: u8, observed_at_utc: UtcMicros) -> Application {
        Application::try_new(
            ApplicationId::from_bytes([value; 16]),
            ApplicationName::try_new(format!("Application {value}"))
                .expect("fixture application name is valid"),
            None,
            observed_at_utc,
            observed_at_utc,
        )
        .expect("fixture application bounds are valid")
    }

    fn foreground_command(
        identifiers: &CommandIdentifiers,
        sequence: u64,
        observed_at_utc: UtcMicros,
        application_id: ApplicationId,
    ) -> openmanic_application::CommandEnvelope<TrackingCommand> {
        identifiers.command(TrackingCommand::Evidence(TrackingEvidence::Foreground {
            sequence,
            observed_at_utc,
            application_id,
        }))
    }

    #[test]
    fn foreground_switch_is_confirmed_after_delay_with_original_boundary() {
        let identifiers = CommandIdentifiers::default();
        let mut stabilizer = ForegroundSwitchStabilizer::new(10);
        let first_at = UtcMicros::new(1_000_000);
        let first = foreground_application(1, first_at);
        let first_id = first.id();
        assert!(matches!(
            stabilizer.observe(
                first,
                foreground_command(&identifiers, 1, first_at, first_id)
            ),
            ForegroundSwitchDecision::Immediate { .. }
        ));

        let candidate_at = UtcMicros::new(2_000_000);
        let candidate = foreground_application(2, candidate_at);
        let candidate_id = candidate.id();
        assert!(matches!(
            stabilizer.observe(
                candidate,
                foreground_command(&identifiers, 2, candidate_at, candidate_id)
            ),
            ForegroundSwitchDecision::Pending
        ));
        assert!(stabilizer.take_mature(UtcMicros::new(11_999_999)).is_none());

        let (accepted, command) = stabilizer
            .take_mature(UtcMicros::new(12_000_000))
            .expect("candidate matures at the configured threshold");
        assert_eq!(accepted.id(), candidate_id);
        assert_eq!(foreground_observed_at(&command), Some(candidate_at));
    }

    #[test]
    fn brief_foreground_switch_is_discarded_when_previous_app_returns() {
        let identifiers = CommandIdentifiers::default();
        let mut stabilizer = ForegroundSwitchStabilizer::new(10);
        let first_at = UtcMicros::new(1_000_000);
        let first = foreground_application(1, first_at);
        let first_id = first.id();
        let _ = stabilizer.observe(
            first,
            foreground_command(&identifiers, 1, first_at, first_id),
        );
        let candidate_at = UtcMicros::new(2_000_000);
        let candidate = foreground_application(2, candidate_at);
        let candidate_id = candidate.id();
        let _ = stabilizer.observe(
            candidate,
            foreground_command(&identifiers, 2, candidate_at, candidate_id),
        );

        let returned_at = UtcMicros::new(5_000_000);
        let returned = foreground_application(1, returned_at);
        assert!(matches!(
            stabilizer.observe(
                returned,
                foreground_command(&identifiers, 3, returned_at, first_id)
            ),
            ForegroundSwitchDecision::Immediate { .. }
        ));
        assert!(stabilizer.take_mature(UtcMicros::new(30_000_000)).is_none());
    }

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
    fn tracking_debug_records_foreground_evidence_and_writer_delivery() {
        let log_path = std::env::temp_dir().join(format!(
            "openmanic-tracking-debug-{}.log",
            std::process::id()
        ));
        let _ = fs::remove_file(&log_path);
        let debug = Mutex::new(TrackingDebugState::default());
        let application_id = ApplicationId::from_bytes([7; 16]);
        record_tracking_observation(
            &debug,
            &log_path,
            &TrackingEvidence::Foreground {
                sequence: 11,
                observed_at_utc: UtcMicros::new(123_456),
                application_id,
            },
        );
        set_tracking_delivery(&debug, &log_path, "Accepted by the writer lane.");

        let state = debug.lock().expect("fixture diagnostics state");
        assert_eq!(state.foreground_events, 1);
        assert_eq!(state.latest_application_id, Some(application_id));
        assert_eq!(state.latest_observed_at, Some(UtcMicros::new(123_456)));
        assert_eq!(state.last_delivery, "Accepted by the writer lane.");
        drop(state);
        let log = fs::read_to_string(&log_path).expect("tracking debug log is written");
        assert!(log.contains("observed_at_utc_us=123456 event=foreground"));
        assert!(log.contains("writer_lane=Accepted by the writer lane."));
        fs::remove_file(log_path).expect("fixture tracking debug log is removed");
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
    fn utc_values_are_presented_as_readable_twelve_hour_times() {
        const HOUR_US: i64 = 3_600_000_000;
        let range = HalfOpenInterval::try_new(
            UtcMicros::new(9 * HOUR_US),
            UtcMicros::new(13 * HOUR_US + 30 * 60_000_000),
        )
        .expect("fixture range is positive");

        assert_eq!(format_utc_clock(range.start()), "9:00 AM");
        assert_eq!(format_utc_range(range), "9:00 AM - 1:30 PM UTC");
    }

    #[test]
    fn common_desktop_applications_receive_the_initial_taxonomy() {
        for (application, category_index) in [
            ("Discord", 1),
            ("Google Chrome", 4),
            ("Mozilla Firefox", 4),
            ("ChatGPT", 5),
            ("Claude", 5),
            ("Gemini", 5),
            ("Windows Terminal", 0),
            ("Spotify", 3),
            ("mpv", 3),
            ("KeePassXC", 7),
            ("1Password", 7),
        ] {
            assert_eq!(
                initial_category_for_application(application),
                Some(initial_category_id(category_index))
            );
        }
        assert_eq!(initial_category_for_application("Unknown tool"), None);
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
            ScheduleRule::repeating("Review", None, 1, 9 * 3_600, 10 * 3_600, 0, None, "Etc/UTC")
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

        assert_eq!(
            command.expected_entity_revision(),
            Some(EntityRevision::new(4))
        );
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
    fn calendar_occurrence_target_uses_the_timeline_delete_contract() {
        let identifiers = CommandIdentifiers::default();
        let series_id = ScheduleSeriesId::from_bytes([12; 16]);
        let block_id = CalendarBlockId::Schedule(ScheduleOccurrenceId::Recurring {
            schedule_id: ScheduleId::Series(series_id),
            anchor_date: 25,
        });
        let (schedule_id, anchor_date) = calendar_schedule_target(block_id)
            .expect("schedule blocks expose an occurrence target");

        assert_eq!(schedule_id, ScheduleId::Series(series_id));
        assert_eq!(anchor_date, Some(25));
        let command = identifiers.schedule_delete_occurrence(
            series_id,
            anchor_date.expect("recurring occurrences retain an anchor date"),
            ScheduleEditScope::OnlyThisDate,
            EntityRevision::new(2),
        );
        assert!(matches!(
            command.payload(),
            ScheduleCommand::DeleteOccurrence {
                series_id: actual_series_id,
                anchor_date: 25,
                scope: ScheduleEditScope::OnlyThisDate,
            } if *actual_series_id == series_id
        ));
    }

    #[test]
    fn recurring_editor_uses_the_segment_active_on_the_selected_date() {
        let series_id = ScheduleSeriesId::from_bytes([8; 16]);
        let mut rule = ScheduleRule::repeating(
            "Original",
            None,
            0b0001_1111,
            9 * 3_600,
            10 * 3_600,
            0,
            None,
            "Asia/Karachi",
        )
        .expect("valid initial segment");
        rule.change_this_and_future(
            120,
            "Later",
            None,
            0b0110_0000,
            14 * 3_600,
            15 * 3_600,
            "Europe/London",
        )
        .expect("valid later segment");
        let snapshot = ScheduleSnapshot::try_new(
            ScheduleId::Series(series_id),
            rule,
            EntityRevision::new(3),
            UtcMicros::new(0),
        )
        .expect("matching recurrence identity");
        let range = HalfOpenInterval::try_new(
            UtcMicros::new(120 * 86_400_000_000),
            UtcMicros::new(120 * 86_400_000_000 + 3_600_000_000),
        )
        .expect("positive selected occurrence");

        let draft = ScheduleDraft::from_recurring(snapshot, series_id, 120, range)
            .expect("selected date belongs to a rule segment");
        let recurring = draft.recurring.expect("recurring edit metadata");
        assert_eq!(draft.label, "Later");
        assert_eq!(draft.weekday_mask, 0b0110_0000);
        assert_eq!(recurring.start_second_of_day, 14 * 3_600);
        assert_eq!(recurring.end_second_of_day, 15 * 3_600);
        assert_eq!(recurring.time_zone_id, "Europe/London");
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
        let fractional_range =
            HalfOpenInterval::try_new(UtcMicros::new(1), UtcMicros::new(3_600_000_001))
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
