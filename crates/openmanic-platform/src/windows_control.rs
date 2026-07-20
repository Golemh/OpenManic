//! Windows control-window and foreground-hook plumbing.
//!
//! The only work done by the `WinEvent` callback is copying the window value, event tick, and a
//! receive-time sample into a preallocated bounded ingress.  Normal-loop code validates and
//! reconciles raw window state before using the accepted platform normalizer.  Application
//! identity resolution is intentionally deferred to OM-250; until then a live raw foreground
//! window is reported as an explicit identity degradation, never as fabricated attribution.

use std::sync::Arc;

#[cfg(windows)]
use std::sync::mpsc::SyncSender;

#[cfg(windows)]
use std::{error::Error, fmt};

#[cfg(windows)]
use crate::windows_tray::WindowsTray;

#[cfg(windows)]
use crate::windows_identity::{
    ApplicationIdentityResolution, StableApplicationIdentity, WindowsIdentityResolver,
};
use crate::windows_raw::{
    RawEventIngress, RawForegroundEvent, RawIngressDrainError, RawWindowHandle, receive_time_utc,
};
use crate::{
    AdapterAvailability, AdapterObservation, AdapterObservationKind, AdapterPublishResult,
    AdapterPublishStatus, Capability, CapabilitySet, PlatformActivityAdapter, PlatformCapabilities,
    PlatformEventNormalizer, TrackingEvidenceSink,
};
#[cfg(windows)]
use crate::{DeliveryCapability, FieldSupport, FocusScope};

/// Number of raw foreground observations the callback can retain before declaring loss.
pub const WINDOWS_FOREGROUND_INGRESS_CAPACITY: usize = 128;

/// One bounded request for background Windows application metadata work.
#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowsApplicationMetadataRequest {
    application_id: openmanic_application::ApplicationId,
    observed_at_utc_us: i64,
    executable_path: String,
}

/// One bounded, unlogged sample of the current foreground window title.
///
/// The request holds no window handle or process identity. Consumers must pass its title only to
/// the privacy gate and title stabilizer; it is intentionally not `Debug` so ordinary diagnostics
/// cannot disclose it accidentally.
#[cfg(windows)]
pub struct WindowsWindowTitleObservationRequest {
    application_id: openmanic_application::ApplicationId,
    observed_at_utc_us: i64,
    title: String,
}

#[cfg(windows)]
impl WindowsWindowTitleObservationRequest {
    fn new(
        application_id: openmanic_application::ApplicationId,
        observed_at_utc_us: i64,
        title: String,
    ) -> Self {
        Self {
            application_id,
            observed_at_utc_us,
            title,
        }
    }

    /// Returns the resolved foreground application.
    #[must_use]
    pub const fn application_id(&self) -> openmanic_application::ApplicationId {
        self.application_id
    }

    /// Returns when the platform observed this title.
    #[must_use]
    pub const fn observed_at_utc_us(&self) -> i64 {
        self.observed_at_utc_us
    }

    /// Returns the raw title exclusively for private stabilization; callers must not log it.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }
}

#[cfg(windows)]
impl WindowsApplicationMetadataRequest {
    /// Creates a request using the already-resolved executable path; no raw handle escapes.
    #[must_use]
    pub fn new(
        application_id: openmanic_application::ApplicationId,
        observed_at_utc_us: i64,
        executable_path: String,
    ) -> Self {
        Self {
            application_id,
            observed_at_utc_us,
            executable_path,
        }
    }

    /// Returns the stable catalog application identifier.
    #[must_use]
    pub const fn application_id(&self) -> openmanic_application::ApplicationId {
        self.application_id
    }

    /// Returns when Windows observed the application identity.
    #[must_use]
    pub const fn observed_at_utc_us(&self) -> i64 {
        self.observed_at_utc_us
    }

    /// Returns the worker-only executable path.
    #[must_use]
    pub fn executable_path(&self) -> &str {
        &self.executable_path
    }
}

/// Summary of one nonblocking control-loop drain.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WindowsControlDrain {
    processed_raw_events: usize,
    published_evidence: u16,
    reconciliation_required: bool,
}

impl WindowsControlDrain {
    /// Returns how many retained raw notifications reached normal-loop processing.
    #[must_use]
    pub const fn processed_raw_events(self) -> usize {
        self.processed_raw_events
    }

    /// Returns the number of tracking-evidence values accepted by the bounded downstream sink.
    #[must_use]
    pub const fn published_evidence(self) -> u16 {
        self.published_evidence
    }

    /// Returns whether the platform adapter still needs a fresh foreground reconciliation.
    #[must_use]
    pub const fn reconciliation_required(self) -> bool {
        self.reconciliation_required
    }

    fn record(&mut self, result: AdapterPublishResult) {
        if let AdapterPublishStatus::Emitted { evidence_count } = result.status() {
            self.published_evidence = self
                .published_evidence
                .saturating_add(u16::from(evidence_count));
        }
        self.reconciliation_required |= result.reconciliation_required();
    }
}

/// A platform-native Windows foreground adapter with no raw-handle escape hatch.
///
/// A single instance belongs to the Windows control/message-loop thread.  It exposes only the
/// accepted platform-neutral capability and evidence surfaces from OM-230.  OM-250 can extend
/// this module's private raw-resolution step without changing callers or exposing an `HWND`.
#[derive(Debug)]
pub struct WindowsControlAdapter {
    capabilities: PlatformCapabilities,
    normalizer: PlatformEventNormalizer,
    ingress: Arc<RawEventIngress>,
    startup_announced: bool,
    last_live_window: Option<RawWindowHandle>,
    #[cfg(windows)]
    identity_resolver: WindowsIdentityResolver,
    #[cfg(windows)]
    metadata_requests: Option<SyncSender<WindowsApplicationMetadataRequest>>,
    #[cfg(windows)]
    title_requests: Option<SyncSender<WindowsWindowTitleObservationRequest>>,
}

impl Default for WindowsControlAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsControlAdapter {
    /// Creates the Windows control adapter and preallocates its callback ingress.
    #[must_use]
    pub fn new() -> Self {
        Self::with_ingress_capacity(WINDOWS_FOREGROUND_INGRESS_CAPACITY)
    }

    /// Adds a bounded, nonblocking route for worker-only metadata requests.
    #[cfg(windows)]
    #[must_use]
    pub fn with_metadata_requests(
        mut self,
        sender: SyncSender<WindowsApplicationMetadataRequest>,
    ) -> Self {
        self.metadata_requests = Some(sender);
        self
    }

    /// Adds a bounded, nonblocking route for private title observations.
    #[cfg(windows)]
    #[must_use]
    pub fn with_title_requests(
        mut self,
        sender: SyncSender<WindowsWindowTitleObservationRequest>,
    ) -> Self {
        self.title_requests = Some(sender);
        self
    }

    /// Drains retained callback observations and publishes only honest normalized evidence.
    ///
    /// This method never waits for the application sink.  If the raw ingress was full, it first
    /// preserves and processes retained events, then emits explicit loss and samples the current
    /// foreground window before later observations can be trusted.
    #[must_use]
    pub fn drain(&mut self, sink: &dyn TrackingEvidenceSink) -> WindowsControlDrain {
        let mut drain = WindowsControlDrain::default();
        self.announce_startup(sink, &mut drain);

        let events = match self.ingress.try_drain() {
            Ok(events) => events,
            Err(RawIngressDrainError::Busy | RawIngressDrainError::Poisoned) => {
                self.ingress.mark_overflow();
                drain.reconciliation_required = true;
                return drain;
            }
        };
        let overflowed = self.ingress.take_overflow();
        let mut retry_current_foreground = false;

        for event in events {
            drain.processed_raw_events = drain.processed_raw_events.saturating_add(1);
            if Self::is_live_window(event.window()) {
                self.process_live_event(event, sink, &mut drain);
            } else {
                // A callback HWND can be null or destroyed before normal-loop resolution.  A
                // current-window retry happens below after every retained event keeps ordering.
                retry_current_foreground = true;
            }
        }

        if overflowed {
            self.announce_availability(
                self.next_control_observation(AdapterAvailability::EvidenceLost),
                sink,
                &mut drain,
            );
            retry_current_foreground = true;
        }

        if retry_current_foreground || self.normalizer.reconciliation_required() {
            self.reconcile_current_foreground(sink, &mut drain);
        }

        drain.reconciliation_required |= self.normalizer.reconciliation_required();
        drain
    }

    /// Installs the hidden Windows control window, foreground hook, and health-poll timer.
    ///
    /// The returned object is thread-affine and must be driven from the installing thread's
    /// message loop.  It retains all native handles privately and cleans them up on drop.
    ///
    /// # Errors
    ///
    /// Returns [`WindowsControlError`] when the hidden window, callback routing, foreground hook,
    /// or health timer cannot be installed.
    #[cfg(windows)]
    pub fn install_control_window(&self) -> Result<WindowsControlWindow, WindowsControlError> {
        WindowsControlWindow::install(Arc::clone(&self.ingress))
    }

    fn with_ingress_capacity(capacity: usize) -> Self {
        Self {
            capabilities: detected_capabilities(),
            normalizer: PlatformEventNormalizer::new(),
            ingress: Arc::new(RawEventIngress::new(capacity)),
            startup_announced: false,
            last_live_window: None,
            #[cfg(windows)]
            identity_resolver: WindowsIdentityResolver::default(),
            #[cfg(windows)]
            metadata_requests: None,
            #[cfg(windows)]
            title_requests: None,
        }
    }

    fn announce_startup(
        &mut self,
        sink: &dyn TrackingEvidenceSink,
        drain: &mut WindowsControlDrain,
    ) {
        if self.startup_announced {
            return;
        }
        self.startup_announced = true;
        self.announce_availability(
            AdapterObservation::new(
                1,
                receive_time_utc(),
                AdapterObservationKind::Availability(AdapterAvailability::Starting),
            ),
            sink,
            drain,
        );
    }

    fn announce_identity_degraded(
        &mut self,
        event: RawForegroundEvent,
        sink: &dyn TrackingEvidenceSink,
        drain: &mut WindowsControlDrain,
    ) {
        if self.normalizer.reconciliation_required() {
            // A newly sampled, live foreground window is enough to close the platform's raw
            // evidence-loss loop.  It remains identity-degraded until OM-250 resolves it.
            self.normalizer.acknowledge_reconciliation();
        }
        let _event_tick = event.source_event_tick();
        self.announce_availability(
            AdapterObservation::new(
                event.source_order(),
                event.received_at_utc(),
                AdapterObservationKind::Availability(AdapterAvailability::Degraded {
                    missing: CapabilitySet::only(Capability::ApplicationIdentity),
                }),
            ),
            sink,
            drain,
        );
    }

    fn reconcile_current_foreground(
        &mut self,
        sink: &dyn TrackingEvidenceSink,
        drain: &mut WindowsControlDrain,
    ) {
        let Some(event) = self.current_foreground_event() else {
            self.announce_temporary_unavailability(sink, drain);
            return;
        };
        drain.processed_raw_events = drain.processed_raw_events.saturating_add(1);
        if Self::is_live_window(event.window()) {
            self.process_live_event(event, sink, drain);
        } else {
            self.announce_temporary_unavailability(sink, drain);
        }
    }

    fn process_live_event(
        &mut self,
        event: RawForegroundEvent,
        sink: &dyn TrackingEvidenceSink,
        drain: &mut WindowsControlDrain,
    ) {
        let changed_window = self.last_live_window != Some(event.window());
        self.last_live_window = Some(event.window());
        let publish_foreground = changed_window || self.normalizer.reconciliation_required();
        #[cfg(windows)]
        match self
            .identity_resolver
            .resolve_window(event.window().value())
        {
            ApplicationIdentityResolution::Resolved(application) => {
                if self.normalizer.reconciliation_required() {
                    self.normalizer.acknowledge_reconciliation();
                }
                let application_id = stable_application_id(application.identity());
                if let (Some(sender), Some(path)) =
                    (&self.metadata_requests, application.display_path())
                {
                    let _ = sender.try_send(WindowsApplicationMetadataRequest::new(
                        application_id,
                        event.received_at_utc().get(),
                        path.to_owned(),
                    ));
                }
                if let (Some(sender), Some(title)) =
                    (&self.title_requests, current_window_title(event.window()))
                {
                    let _ = sender.try_send(WindowsWindowTitleObservationRequest::new(
                        application_id,
                        event.received_at_utc().get(),
                        title,
                    ));
                }
                if publish_foreground {
                    self.announce_foreground(event, application_id, sink, drain);
                }
            }
            ApplicationIdentityResolution::Unresolved { .. } => {
                self.announce_identity_degraded(event, sink, drain);
            }
        }
        #[cfg(not(windows))]
        self.announce_identity_degraded(event, sink, drain);
    }

    #[cfg(windows)]
    fn announce_foreground(
        &mut self,
        event: RawForegroundEvent,
        application_id: openmanic_application::ApplicationId,
        sink: &dyn TrackingEvidenceSink,
        drain: &mut WindowsControlDrain,
    ) {
        drain.record(self.normalizer.normalize_and_publish(
            AdapterObservation::new(
                event.source_order(),
                event.received_at_utc(),
                AdapterObservationKind::Foreground { application_id },
            ),
            sink,
        ));
    }

    fn announce_temporary_unavailability(
        &mut self,
        sink: &dyn TrackingEvidenceSink,
        drain: &mut WindowsControlDrain,
    ) {
        self.last_live_window = None;
        self.announce_availability(
            self.next_control_observation(AdapterAvailability::TemporarilyUnavailable),
            sink,
            drain,
        );
    }

    fn next_control_observation(&self, availability: AdapterAvailability) -> AdapterObservation {
        match self
            .ingress
            .make_foreground_event(RawWindowHandle::new(0), 0, receive_time_utc())
        {
            Some(event) => AdapterObservation::new(
                event.source_order(),
                event.received_at_utc(),
                AdapterObservationKind::Availability(availability),
            ),
            None => AdapterObservation::new(
                u64::MAX,
                receive_time_utc(),
                AdapterObservationKind::Availability(AdapterAvailability::Fatal),
            ),
        }
    }

    fn announce_availability(
        &mut self,
        observation: AdapterObservation,
        sink: &dyn TrackingEvidenceSink,
        drain: &mut WindowsControlDrain,
    ) {
        drain.record(self.normalizer.normalize_and_publish(observation, sink));
    }

    fn current_foreground_event(&self) -> Option<RawForegroundEvent> {
        self.ingress
            .make_foreground_event(current_foreground_window(), 0, receive_time_utc())
    }

    fn is_live_window(window: RawWindowHandle) -> bool {
        is_live_window(window)
    }

    #[cfg(windows)]
    fn note_callback_registration_loss(&self) {
        if CALLBACK_REGISTRATION_LOSS.swap(false, std::sync::atomic::Ordering::AcqRel) {
            self.ingress.mark_overflow();
        }
    }
}

/// Maximum UTF-16 code units retained from one Win32 title query before private normalization.
#[cfg(windows)]
const WINDOW_TITLE_CAPTURE_MAX_CODE_UNITS: usize = 4_096;

#[cfg(windows)]
fn current_window_title(window: RawWindowHandle) -> Option<String> {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW},
    };

    let hwnd = HWND(window.value() as *mut core::ffi::c_void);
    // SAFETY: The HWND came from the current foreground sample and is used only synchronously.
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length <= 0 {
        return None;
    }
    let length = usize::try_from(length)
        .ok()?
        .min(WINDOW_TITLE_CAPTURE_MAX_CODE_UNITS);
    let capacity = length.checked_add(1)?;
    let mut buffer = vec![0_u16; capacity];
    // SAFETY: `buffer` is valid writable UTF-16 storage with a trailing NUL slot and
    // Windows retains neither the HWND-derived buffer pointer nor the title after return.
    let written = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    if written <= 0 {
        return None;
    }
    let written = usize::try_from(written).ok()?;
    Some(String::from_utf16_lossy(&buffer[..written]))
}

impl PlatformActivityAdapter for WindowsControlAdapter {
    fn capabilities(&self) -> PlatformCapabilities {
        self.capabilities
    }

    fn availability(&self) -> AdapterAvailability {
        self.normalizer.availability()
    }

    fn normalize_and_publish(
        &mut self,
        observation: AdapterObservation,
        sink: &dyn TrackingEvidenceSink,
    ) -> AdapterPublishResult {
        self.normalizer.normalize_and_publish(observation, sink)
    }
}

fn detected_capabilities() -> PlatformCapabilities {
    #[cfg(windows)]
    {
        PlatformCapabilities::unavailable()
            .with_delivery(DeliveryCapability::EventDriven)
            .with_focus_scope(FocusScope::DesktopGlobal)
            .with_field_support(Capability::ForegroundTracking, FieldSupport::Available)
            .with_field_support(Capability::WindowInstance, FieldSupport::Available)
            .with_field_support(Capability::ApplicationIdentity, FieldSupport::Available)
    }

    #[cfg(not(windows))]
    {
        PlatformCapabilities::unavailable()
    }
}

/// Produces a deterministic local catalog key from a resolved Windows identity.
///
/// The source value is already a stable AUMID or normalized full executable path; raw window,
/// PID, title, and executable filename values never participate. The root composition persists
/// the resulting ID before it submits foreground evidence to the tracking reducer.
#[cfg(windows)]
fn stable_application_id(
    identity: &StableApplicationIdentity,
) -> openmanic_application::ApplicationId {
    let (kind, value) = match identity {
        StableApplicationIdentity::Packaged { aumid, .. } => {
            (b"aumid:".as_slice(), aumid.as_bytes())
        }
        StableApplicationIdentity::ExecutablePath { executable_path } => (
            b"path:".as_slice(),
            executable_path.identity_path().as_bytes(),
        ),
    };
    let mut first = 0xcbf2_9ce4_8422_2325_u64;
    let mut second = 0x9e37_79b9_7f4a_7c15_u64;
    for byte in kind.iter().chain(value) {
        first = (first ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        second = second.rotate_left(5) ^ u64::from(*byte);
        second = second.wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&first.to_be_bytes());
    bytes[8..].copy_from_slice(&second.to_be_bytes());
    openmanic_application::ApplicationId::from_bytes(bytes)
}

#[cfg(windows)]
fn current_foreground_window() -> RawWindowHandle {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    // SAFETY: GetForegroundWindow has no pointer arguments and merely returns the current HWND.
    let window = unsafe { GetForegroundWindow() };
    RawWindowHandle::new(window.0 as isize)
}

#[cfg(not(windows))]
fn current_foreground_window() -> RawWindowHandle {
    RawWindowHandle::new(0)
}

#[cfg(windows)]
fn is_live_window(window: RawWindowHandle) -> bool {
    use windows::{
        Win32::{Foundation::HWND, UI::WindowsAndMessaging::IsWindow},
        core::BOOL,
    };

    if window.is_null() {
        return false;
    }
    // SAFETY: The opaque ABI value originated from Windows. IsWindow only validates it and does
    // not dereference application-owned memory.
    let result: BOOL = unsafe { IsWindow(Some(HWND(window.value() as *mut _))) };
    result.as_bool()
}

#[cfg(not(windows))]
fn is_live_window(_window: RawWindowHandle) -> bool {
    false
}

/// A Windows control-window installation or message-loop failure without raw OS error leakage.
#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowsControlError {
    /// The hidden built-in control window could not be created.
    ControlWindowCreation,
    /// Another active control loop already owns the process-global `WinEvent` callback routing.
    CallbackAlreadyInstalled,
    /// Callback routing could not safely acquire its registration state.
    CallbackRegistrationUnavailable,
    /// Windows failed to install the foreground `WinEvent` hook.
    ForegroundHookInstall,
    /// Windows failed to install the one-second foreground health-poll timer.
    HealthTimerInstall,
    /// Windows could not install or recover the notification-area tray icon.
    Tray,
    /// The message loop reported an unrecoverable failure.
    MessageLoop,
}

#[cfg(windows)]
impl fmt::Display for WindowsControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::ControlWindowCreation => "Windows could not create the tracking control window",
            Self::CallbackAlreadyInstalled => {
                "another Windows tracking control loop is already active"
            }
            Self::CallbackRegistrationUnavailable => "Windows callback routing is not available",
            Self::ForegroundHookInstall => "Windows could not install the foreground hook",
            Self::HealthTimerInstall => "Windows could not install the foreground health timer",
            Self::Tray => "Windows could not install or recover the OpenManic tray icon",
            Self::MessageLoop => "Windows reported a control message-loop failure",
        };
        formatter.write_str(message)
    }
}

#[cfg(windows)]
impl Error for WindowsControlError {}

/// Hidden thread-affine Win32 control window and foreground hook.
#[cfg(windows)]
#[derive(Debug)]
pub struct WindowsControlWindow {
    window: windows::Win32::Foundation::HWND,
    hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    timer_id: usize,
    ingress: Arc<RawEventIngress>,
    _thread_affinity: std::marker::PhantomData<std::rc::Rc<()>>,
}

#[cfg(windows)]
impl WindowsControlWindow {
    fn install(ingress: Arc<RawEventIngress>) -> Result<Self, WindowsControlError> {
        use windows::{
            Win32::UI::{
                Accessibility::SetWinEventHook,
                WindowsAndMessaging::{
                    CreateWindowExW, EVENT_SYSTEM_FOREGROUND, SetTimer, WINDOW_EX_STYLE,
                    WINDOW_STYLE, WINEVENT_OUTOFCONTEXT,
                },
            },
            core::w,
        };

        // SAFETY: STATIC is a system-owned built-in class. The control window is hidden because
        // its style has no visibility bit, and no caller-provided pointer is retained.
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("OpenManic tracking control"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                None,
                None,
                None,
                None,
            )
        }
        .map_err(|_| WindowsControlError::ControlWindowCreation)?;

        if let Err(error) = register_callback_ingress(Arc::clone(&ingress)) {
            destroy_window(window);
            return Err(error);
        }

        // SAFETY: The callback has the exact Windows ABI and accesses only the registered
        // bounded ingress through nonblocking synchronization.
        let hook = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(foreground_win_event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };
        if hook.0.is_null() {
            unregister_callback_ingress(&ingress);
            destroy_window(window);
            return Err(WindowsControlError::ForegroundHookInstall);
        }

        // SAFETY: The timer is bound to the hidden control HWND and sends only WM_TIMER to its
        // installing thread's normal message loop; it retains no callback pointer.
        let timer_id = unsafe { SetTimer(Some(window), 1, 1_000, None) };
        if timer_id == 0 {
            unhook_win_event(hook);
            unregister_callback_ingress(&ingress);
            destroy_window(window);
            return Err(WindowsControlError::HealthTimerInstall);
        }

        Ok(Self {
            window,
            hook,
            timer_id,
            ingress,
            _thread_affinity: std::marker::PhantomData,
        })
    }

    /// Installs the notification-area icon on this hidden control window's message loop.
    ///
    /// The returned tray keeps native callbacks on the control thread and exposes only queued
    /// local actions for the primary composition task to route.
    ///
    /// # Errors
    ///
    /// Returns [`WindowsControlError::Tray`] when Windows rejects the notification-area icon.
    pub fn install_tray(&self) -> Result<WindowsTray, WindowsControlError> {
        WindowsTray::install(self.window).map_err(|_| WindowsControlError::Tray)
    }

    /// Runs the installing thread's message loop until Windows posts `WM_QUIT`.
    ///
    /// Every delivered message is dispatched normally. The one-second timer causes a fresh
    /// `GetForegroundWindow` sample, while callback events remain bounded and are drained only
    /// after message dispatch has returned to ordinary control-loop work.
    ///
    /// # Errors
    ///
    /// Returns [`WindowsControlError::MessageLoop`] when Windows reports a message retrieval
    /// failure. A normal `WM_QUIT` message returns `Ok(())`.
    pub fn run(
        mut self,
        adapter: &mut WindowsControlAdapter,
        sink: &dyn TrackingEvidenceSink,
    ) -> Result<(), WindowsControlError> {
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, MSG, TranslateMessage,
        };

        adapter.note_callback_registration_loss();
        let _ = adapter.drain(sink);
        loop {
            let mut message = MSG::default();
            // SAFETY: MSG is initialized storage owned by this thread. Passing no HWND and zero
            // filters reads the installing thread's queue, which this object owns.
            let message_result = unsafe { GetMessageW(&raw mut message, None, 0, 0) };
            if message_result.0 == -1 {
                return Err(WindowsControlError::MessageLoop);
            }
            if message_result.0 == 0 {
                return Ok(());
            }
            self.dispatch_message(message, adapter);
            // SAFETY: The message was obtained from this thread's queue and remains valid for
            // the duration of this dispatch. Windows owns any message-associated memory.
            unsafe {
                let _ = TranslateMessage(&raw const message);
                DispatchMessageW(&raw const message);
            }
            adapter.note_callback_registration_loss();
            let _ = adapter.drain(sink);
        }
    }

    /// Runs the control loop while also dispatching notification-area callbacks.
    ///
    /// The tray only queues local actions; it never invokes the application service from a
    /// callback. Callers drain those actions through [`WindowsTray::take_next_action`] and route
    /// them through the accepted composition path.
    ///
    /// # Errors
    ///
    /// Returns [`WindowsControlError::Tray`] if Explorer recreation cannot restore the tray icon,
    /// or [`WindowsControlError::MessageLoop`] when Windows reports message retrieval failure.
    pub fn run_with_tray(
        self,
        adapter: &mut WindowsControlAdapter,
        sink: &dyn TrackingEvidenceSink,
        tray: &mut WindowsTray,
    ) -> Result<(), WindowsControlError> {
        self.run_with_tray_actions(adapter, sink, tray, |_| {})
    }

    /// Runs the control loop and forwards each retained tray action after its Win32 callback
    /// returns. The action handler executes on the control-loop thread and must remain bounded.
    ///
    /// # Errors
    ///
    /// Returns [`WindowsControlError::Tray`] if Explorer recreation cannot restore the tray icon,
    /// or [`WindowsControlError::MessageLoop`] when Windows reports message retrieval failure.
    pub fn run_with_tray_actions<F>(
        mut self,
        adapter: &mut WindowsControlAdapter,
        sink: &dyn TrackingEvidenceSink,
        tray: &mut WindowsTray,
        mut on_action: F,
    ) -> Result<(), WindowsControlError>
    where
        F: FnMut(crate::WindowsPlatformAction),
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, MSG, TranslateMessage,
        };

        adapter.note_callback_registration_loss();
        let _ = adapter.drain(sink);
        let result = loop {
            let mut message = MSG::default();
            // SAFETY: MSG is initialized storage owned by this thread. Passing no HWND and zero
            // filters reads the installing thread's queue, which this object owns.
            let message_result = unsafe { GetMessageW(&raw mut message, None, 0, 0) };
            if message_result.0 == -1 {
                break Err(WindowsControlError::MessageLoop);
            }
            if message_result.0 == 0 {
                break Ok(());
            }
            if let Err(error) = self.dispatch_message_with_tray(message, adapter, tray) {
                break Err(error);
            }
            // SAFETY: The message was obtained from this thread's queue and remains valid for
            // the duration of this dispatch. Windows owns any message-associated memory.
            unsafe {
                let _ = TranslateMessage(&raw const message);
                DispatchMessageW(&raw const message);
            }
            adapter.note_callback_registration_loss();
            let _ = adapter.drain(sink);
            while let Some(action) = tray.take_next_action() {
                on_action(action);
            }
        };
        tray.remove_icon();
        result
    }

    /// Drains all currently queued messages without blocking, primarily for controlled fixtures.
    #[must_use]
    pub fn pump_available(
        &mut self,
        adapter: &mut WindowsControlAdapter,
        sink: &dyn TrackingEvidenceSink,
    ) -> WindowsControlDrain {
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
        };

        loop {
            let mut message = MSG::default();
            // SAFETY: MSG is initialized stack storage and this method runs on the installing
            // thread. PM_REMOVE prevents a message from being dispatched twice.
            let available = unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_REMOVE) };
            if !available.as_bool() {
                break;
            }
            self.dispatch_message(message, adapter);
            // SAFETY: The message came from this thread's queue and is valid for dispatch.
            unsafe {
                let _ = TranslateMessage(&raw const message);
                DispatchMessageW(&raw const message);
            }
        }
        adapter.note_callback_registration_loss();
        adapter.drain(sink)
    }

    /// Drains available control and notification-area messages without blocking.
    ///
    /// This is the controlled-fixture counterpart of [`Self::run_with_tray`].
    ///
    /// # Errors
    ///
    /// Returns [`WindowsControlError::Tray`] when a taskbar recreation cannot restore the icon.
    pub fn pump_available_with_tray(
        &mut self,
        adapter: &mut WindowsControlAdapter,
        sink: &dyn TrackingEvidenceSink,
        tray: &mut WindowsTray,
    ) -> Result<WindowsControlDrain, WindowsControlError> {
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
        };

        loop {
            let mut message = MSG::default();
            // SAFETY: MSG is initialized stack storage and this method runs on the installing
            // thread. PM_REMOVE prevents a message from being dispatched twice.
            let available = unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_REMOVE) };
            if !available.as_bool() {
                break;
            }
            self.dispatch_message_with_tray(message, adapter, tray)?;
            // SAFETY: The message came from this thread's queue and is valid for dispatch.
            unsafe {
                let _ = TranslateMessage(&raw const message);
                DispatchMessageW(&raw const message);
            }
        }
        adapter.note_callback_registration_loss();
        Ok(adapter.drain(sink))
    }

    fn dispatch_message(
        &mut self,
        message: windows::Win32::UI::WindowsAndMessaging::MSG,
        adapter: &WindowsControlAdapter,
    ) {
        use windows::Win32::UI::WindowsAndMessaging::WM_TIMER;

        if message.hwnd == self.window
            && message.message == WM_TIMER
            && message.wParam.0 == self.timer_id
        {
            adapter.ingress.try_enqueue_foreground(
                current_foreground_window(),
                message.time,
                receive_time_utc(),
            );
        }
    }

    fn dispatch_message_with_tray(
        &mut self,
        message: windows::Win32::UI::WindowsAndMessaging::MSG,
        adapter: &WindowsControlAdapter,
        tray: &mut WindowsTray,
    ) -> Result<(), WindowsControlError> {
        tray.handle_control_message(message.message, message.wParam.0, message.lParam.0)
            .map_err(|_| WindowsControlError::Tray)?;
        self.dispatch_message(message, adapter);
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for WindowsControlWindow {
    fn drop(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::KillTimer;

        // SAFETY: Each handle was created by this object on the same control thread. Cleanup is
        // best-effort because Drop cannot surface a shutdown failure and retained uncertainty is
        // preferable to attempting a second teardown.
        let _ = unsafe { KillTimer(Some(self.window), self.timer_id) };
        unhook_win_event(self.hook);
        unregister_callback_ingress(&self.ingress);
        destroy_window(self.window);
    }
}

#[cfg(windows)]
static CALLBACK_INGRESS: std::sync::Mutex<Option<Arc<RawEventIngress>>> =
    std::sync::Mutex::new(None);

#[cfg(windows)]
static CALLBACK_REGISTRATION_LOSS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(windows)]
fn register_callback_ingress(ingress: Arc<RawEventIngress>) -> Result<(), WindowsControlError> {
    match CALLBACK_INGRESS.lock() {
        Ok(mut registered) if registered.is_none() => {
            *registered = Some(ingress);
            CALLBACK_REGISTRATION_LOSS.store(false, std::sync::atomic::Ordering::Release);
            Ok(())
        }
        Ok(_) => Err(WindowsControlError::CallbackAlreadyInstalled),
        Err(_) => Err(WindowsControlError::CallbackRegistrationUnavailable),
    }
}

#[cfg(windows)]
fn unregister_callback_ingress(ingress: &Arc<RawEventIngress>) {
    if let Ok(mut registered) = CALLBACK_INGRESS.lock()
        && registered
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, ingress))
    {
        *registered = None;
    }
}

#[cfg(windows)]
unsafe extern "system" fn foreground_win_event(
    _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    _event: u32,
    window: windows::Win32::Foundation::HWND,
    _object_id: i32,
    _child_id: i32,
    _event_thread: u32,
    event_tick: u32,
) {
    let ingress = if let Ok(registered) = CALLBACK_INGRESS.try_lock() {
        registered.clone()
    } else {
        CALLBACK_REGISTRATION_LOSS.store(true, std::sync::atomic::Ordering::Release);
        return;
    };
    let Some(ingress) = ingress else {
        CALLBACK_REGISTRATION_LOSS.store(true, std::sync::atomic::Ordering::Release);
        return;
    };

    // The callback has deliberately reached no process, title, storage, logging, or application
    // service API. `try_enqueue_foreground` takes only a try-lock and writes preallocated storage.
    ingress.try_enqueue_foreground(
        RawWindowHandle::new(window.0 as isize),
        event_tick,
        receive_time_utc(),
    );
}

#[cfg(windows)]
fn unhook_win_event(hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK) {
    use windows::Win32::UI::Accessibility::UnhookWinEvent;

    // SAFETY: The hook was returned by SetWinEventHook for this process and is not reused after
    // this teardown attempt.
    let _ = unsafe { UnhookWinEvent(hook) };
}

#[cfg(windows)]
fn destroy_window(window: windows::Win32::Foundation::HWND) {
    use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;

    // SAFETY: The hidden HWND was created by this adapter and is not exposed to callers.
    let _ = unsafe { DestroyWindow(window) };
}

#[cfg(test)]
mod tests {
    use openmanic_application::TrackingEvidence;

    use super::WindowsControlAdapter;
    use crate::FakeEvidenceSink;
    use crate::windows_raw::{RawWindowHandle, receive_time_utc};

    #[test]
    fn overflowing_raw_callback_ingress_announces_loss_before_a_fresh_foreground_sample() {
        let mut adapter = WindowsControlAdapter::with_ingress_capacity(1);
        let sink = FakeEvidenceSink::new(8);

        assert!(adapter.ingress.try_enqueue_foreground(
            RawWindowHandle::new(0),
            10,
            receive_time_utc(),
        ));
        assert!(!adapter.ingress.try_enqueue_foreground(
            RawWindowHandle::new(0),
            11,
            receive_time_utc(),
        ));

        let drain = adapter.drain(&sink);
        let evidence = sink.snapshot();

        assert_eq!(drain.processed_raw_events(), 2);
        assert!(evidence.is_ok());
        let Some(evidence) = evidence.ok() else {
            return;
        };
        assert!(matches!(
            evidence.first(),
            Some(TrackingEvidence::AdapterStarting { .. })
        ));
        assert!(
            evidence
                .iter()
                .any(|value| matches!(value, TrackingEvidence::EvidenceQueueOverflow { .. }))
        );
        let overflow = evidence
            .iter()
            .position(|value| matches!(value, TrackingEvidence::EvidenceQueueOverflow { .. }));
        let foreground = evidence
            .iter()
            .position(|value| matches!(value, TrackingEvidence::Foreground { .. }));
        assert!(
            foreground
                .is_none_or(|foreground| overflow.is_some_and(|overflow| overflow < foreground))
        );
    }

    #[cfg(windows)]
    #[test]
    fn hidden_control_window_installs_and_drains_a_fixture_message_loop() {
        let mut adapter = WindowsControlAdapter::new();
        let sink = FakeEvidenceSink::new(8);
        let installed = adapter.install_control_window();
        assert!(installed.is_ok());
        let Some(mut window) = installed.ok() else {
            return;
        };

        let drain = window.pump_available(&mut adapter, &sink);

        assert!(drain.published_evidence() >= 1);
        assert!(matches!(
            sink.snapshot().as_deref(),
            Ok([TrackingEvidence::AdapterStarting { .. }, ..])
        ));
    }
}
