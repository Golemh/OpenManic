//! Private Windows idle, session, power, and time-evidence normalization.
//!
//! This module deliberately consumes numeric message data and clock samples rather than owning a
//! window, callback, queue, or persistence handle.  OM-299 composes the actions below with the
//! control window and the accepted platform normalizer.  Keeping this state machine deterministic
//! lets it preserve uncertainty when Windows lifecycle delivery is incomplete.

#![expect(
    dead_code,
    reason = "OM-299 exclusively owns control-window composition; this private, unit-tested state machine is prepared for that wiring."
)]

use crate::{AdapterObservation, AdapterObservationKind};
use openmanic_application::UtcMicros;

const DEFAULT_IDLE_THRESHOLD_MS: u32 = 5 * 60 * 1_000;
const MAX_TRUSTED_IDLE_SAMPLE_GAP_MS: u32 = 5_000;
const TIME_BASE_TOLERANCE_US: u64 = 2_000_000;

const WM_TIMECHANGE_VALUE: u32 = 0x001E;
const WM_QUERYENDSESSION_VALUE: u32 = 0x0011;
const WM_ENDSESSION_VALUE: u32 = 0x0016;
const WM_POWERBROADCAST_VALUE: u32 = 0x0218;
const WM_WTSSESSION_CHANGE_VALUE: u32 = 0x02B1;

const PBT_APMSUSPEND_VALUE: u32 = 0x0004;
const PBT_APMRESUMESUSPEND_VALUE: u32 = 0x0007;
const PBT_APMRESUMEAUTOMATIC_VALUE: u32 = 0x0012;

const WTS_CONSOLE_CONNECT_VALUE: u32 = 0x0001;
const WTS_CONSOLE_DISCONNECT_VALUE: u32 = 0x0002;
const WTS_REMOTE_CONNECT_VALUE: u32 = 0x0003;
const WTS_REMOTE_DISCONNECT_VALUE: u32 = 0x0004;
const WTS_SESSION_LOGON_VALUE: u32 = 0x0005;
const WTS_SESSION_LOGOFF_VALUE: u32 = 0x0006;
const WTS_SESSION_LOCK_VALUE: u32 = 0x0007;
const WTS_SESSION_UNLOCK_VALUE: u32 = 0x0008;

/// A raw lifecycle input copied from a Windows message or a bounded health sample.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowsLifecycleRaw {
    /// Session-specific input timing sampled from `GetLastInputInfo` and `GetTickCount`.
    IdleSample {
        /// UTC timestamp captured with the sample.
        observed_at_utc: UtcMicros,
        /// `GetLastInputInfo`'s session tick value.
        last_input_tick_ms: u32,
        /// `GetTickCount` sampled immediately after the input tick.
        current_tick_ms: u32,
    },
    /// A `WM_WTSSESSION_CHANGE` notification decoded without a raw WTS handle.
    SessionChange {
        /// UTC timestamp captured while dispatching the message.
        observed_at_utc: UtcMicros,
        /// The session transition delivered by WTS.
        change: WindowsSessionChange,
    },
    /// A power-broadcast notification that does not distinguish sleep from hibernation.
    Power {
        /// UTC timestamp captured while dispatching the message.
        observed_at_utc: UtcMicros,
        /// The safe power transition classification.
        notification: WindowsPowerNotification,
    },
    /// `WM_TIMECHANGE`, which invalidates wall-clock attribution until rebaselined.
    TimeChange {
        /// UTC timestamp captured while dispatching the message.
        observed_at_utc: UtcMicros,
    },
    /// Simultaneously sampled wall and monotonic time bases.
    TimeBaseSample(WindowsTimeBaseSample),
    /// `WM_QUERYENDSESSION`: a shutdown or logoff proposal that may still be cancelled.
    QueryEndSession {
        /// UTC timestamp captured while dispatching the message.
        observed_at_utc: UtcMicros,
    },
    /// `WM_ENDSESSION`, including Windows' cancellation indication.
    EndSession {
        /// UTC timestamp captured while dispatching the message.
        observed_at_utc: UtcMicros,
        /// Whether Windows confirmed that the session is ending.
        ending: bool,
    },
    /// A later clean adapter startup boundary.
    StartupBoundary {
        /// UTC timestamp captured after a fresh adapter startup.
        observed_at_utc: UtcMicros,
    },
}

/// WTS session changes whose lifecycle semantics affect trustworthy tracking.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowsSessionChange {
    /// A local or remote session connection was established.
    Connected,
    /// A local or remote session connection was lost.
    Disconnected,
    /// WTS delivered a logon notification.
    LoggedOn,
    /// WTS delivered a logoff notification.
    LoggedOff,
    /// The session locked.
    Locked,
    /// The session unlocked.
    Unlocked,
}

/// The safe subset of power broadcasts needed for lifecycle attribution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowsPowerNotification {
    /// Windows is entering a low-power state; sleep and hibernate remain indistinguishable.
    Suspended,
    /// Windows resumed from a low-power state and needs fresh reconciliation.
    Resumed,
}

/// One wall/monotonic clock sample converted to microsecond units by the platform edge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WindowsTimeBaseSample {
    observed_at_utc: UtcMicros,
    monotonic_us: u64,
    unbiased_interrupt_us: u64,
}

impl WindowsTimeBaseSample {
    /// Creates a time-base sample after native counter values have been scaled to microseconds.
    #[must_use]
    pub(crate) const fn new(
        observed_at_utc: UtcMicros,
        monotonic_us: u64,
        unbiased_interrupt_us: u64,
    ) -> Self {
        Self {
            observed_at_utc,
            monotonic_us,
            unbiased_interrupt_us,
        }
    }

    /// Returns the wall-clock component of the sample.
    #[must_use]
    pub(crate) const fn observed_at_utc(self) -> UtcMicros {
        self.observed_at_utc
    }
}

/// The precision of a derived idle boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IdleBoundaryConfidence {
    /// Stable input timing crossed the configured threshold inside the observation window.
    DerivedFromThreshold,
    /// The source timing was unusual, so the boundary is clamped to the observation instant.
    ObservationClamped,
}

/// An idle threshold boundary together with the source confidence that produced it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IdleBoundary {
    occurred_at_utc: UtcMicros,
    confidence: IdleBoundaryConfidence,
}

impl IdleBoundary {
    /// Returns the safe UTC boundary to give the tracking reducer.
    #[must_use]
    pub(crate) const fn occurred_at_utc(self) -> UtcMicros {
        self.occurred_at_utc
    }

    /// Returns whether the boundary was derived or conservatively clamped.
    #[must_use]
    pub(crate) const fn confidence(self) -> IdleBoundaryConfidence {
        self.confidence
    }
}

/// A private lifecycle observation awaiting OM-299's bounded platform publication path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LifecycleObservation {
    /// An observation already supported by the common platform normalizer.
    Adapter {
        /// Timestamp to preserve when constructing `AdapterObservation`.
        observed_at_utc: UtcMicros,
        /// The platform-neutral event kind.
        kind: AdapterObservationKind,
    },
    /// An affirmative end-session boundary followed by a later startup boundary.
    ///
    /// This is the only lifecycle output eligible to become `ConfirmedPowerTransition`.
    ConfirmedPowerTransition {
        /// The `WM_ENDSESSION(TRUE)` instant.
        shutdown_at_utc: UtcMicros,
        /// The later clean startup instant.
        startup_at_utc: UtcMicros,
    },
}

impl LifecycleObservation {
    /// Converts common lifecycle evidence to the existing bounded normalizer input.
    ///
    /// A confirmed power transition deliberately returns `None`: the current common normalizer
    /// does not allocate that multi-boundary evidence, and OM-299 must compose it explicitly.
    #[must_use]
    pub(crate) const fn adapter_observation(self, source_order: u64) -> Option<AdapterObservation> {
        match self {
            Self::Adapter {
                observed_at_utc,
                kind,
            } => Some(AdapterObservation::new(source_order, observed_at_utc, kind)),
            Self::ConfirmedPowerTransition { .. } => None,
        }
    }
}

/// Nonblocking work requested after a lifecycle raw input is normalized.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct LifecycleNormalization {
    observation: Option<LifecycleObservation>,
    idle_boundary: Option<IdleBoundary>,
    follow_up: LifecycleFollowUp,
}

/// The one bounded follow-up action selected by a lifecycle transition.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum LifecycleFollowUp {
    /// The transition needs no asynchronous follow-up.
    #[default]
    None,
    /// Queue a high-priority checkpoint without delaying `WM_QUERYENDSESSION`.
    HighPriorityCheckpoint,
    /// Permit only the bounded final flush after `WM_ENDSESSION(TRUE)`.
    BoundedFinalFlush,
    /// Obtain fresh foreground evidence and sample time bases again before active attribution.
    ReconcileForegroundAndRebaselineTime,
}

impl LifecycleNormalization {
    /// Returns the one normalized tracking observation produced by the raw input, if any.
    #[must_use]
    pub(crate) const fn observation(self) -> Option<LifecycleObservation> {
        self.observation
    }

    /// Returns detailed idle-boundary confidence when the observation is an idle transition.
    #[must_use]
    pub(crate) const fn idle_boundary(self) -> Option<IdleBoundary> {
        self.idle_boundary
    }

    /// Returns whether a high-priority checkpoint should be queued without delaying the message.
    #[must_use]
    pub(crate) fn checkpoint_requested(self) -> bool {
        self.follow_up == LifecycleFollowUp::HighPriorityCheckpoint
    }

    /// Returns whether the caller may attempt only its bounded final flush path.
    #[must_use]
    pub(crate) fn final_flush_permitted(self) -> bool {
        self.follow_up == LifecycleFollowUp::BoundedFinalFlush
    }

    /// Returns whether a fresh foreground observation is required before active attribution.
    #[must_use]
    pub(crate) fn foreground_reconciliation_required(self) -> bool {
        self.follow_up == LifecycleFollowUp::ReconcileForegroundAndRebaselineTime
    }

    /// Returns whether wall/monotonic bases must be sampled again before trusting them.
    #[must_use]
    pub(crate) fn time_base_rebaseline_required(self) -> bool {
        self.follow_up == LifecycleFollowUp::ReconcileForegroundAndRebaselineTime
    }

    fn adapter(observed_at_utc: UtcMicros, kind: AdapterObservationKind) -> Self {
        Self {
            observation: Some(LifecycleObservation::Adapter {
                observed_at_utc,
                kind,
            }),
            ..Self::default()
        }
    }

    fn clock_discontinuity(observed_at_utc: UtcMicros) -> Self {
        Self {
            follow_up: LifecycleFollowUp::ReconcileForegroundAndRebaselineTime,
            ..Self::adapter(observed_at_utc, AdapterObservationKind::ClockDiscontinuity)
        }
    }

    fn requires_reconciliation_and_rebaseline() -> Self {
        Self {
            follow_up: LifecycleFollowUp::ReconcileForegroundAndRebaselineTime,
            ..Self::default()
        }
    }
}

/// Deterministic normalizer for Windows lifecycle data retained privately by the platform crate.
#[derive(Debug)]
pub(crate) struct WindowsLifecycleNormalizer {
    idle_threshold_ms: u32,
    idle: IdleTrackingState,
    session: SessionTrackingState,
    end_session: EndSessionState,
    time_base: Option<WindowsTimeBaseSample>,
}

impl Default for WindowsLifecycleNormalizer {
    fn default() -> Self {
        Self::new(DEFAULT_IDLE_THRESHOLD_MS)
    }
}

impl WindowsLifecycleNormalizer {
    /// Creates lifecycle normalization with the configured idle threshold in milliseconds.
    #[must_use]
    pub(crate) const fn new(idle_threshold_ms: u32) -> Self {
        Self {
            idle_threshold_ms,
            idle: IdleTrackingState::new(),
            session: SessionTrackingState::new(),
            end_session: EndSessionState::None,
            time_base: None,
        }
    }

    /// Converts one raw lifecycle input into at most one safe tracking observation and actions.
    #[must_use]
    pub(crate) fn normalize(&mut self, raw: WindowsLifecycleRaw) -> LifecycleNormalization {
        match raw {
            WindowsLifecycleRaw::IdleSample {
                observed_at_utc,
                last_input_tick_ms,
                current_tick_ms,
            } => self.normalize_idle(observed_at_utc, last_input_tick_ms, current_tick_ms),
            WindowsLifecycleRaw::SessionChange {
                observed_at_utc,
                change,
            } => self.normalize_session_change(observed_at_utc, change),
            WindowsLifecycleRaw::Power {
                observed_at_utc,
                notification,
            } => self.normalize_power(observed_at_utc, notification),
            WindowsLifecycleRaw::TimeChange { observed_at_utc } => {
                self.time_base = None;
                self.idle.reset();
                LifecycleNormalization::clock_discontinuity(observed_at_utc)
            }
            WindowsLifecycleRaw::TimeBaseSample(sample) => self.normalize_time_base(sample),
            WindowsLifecycleRaw::QueryEndSession { observed_at_utc } => {
                self.normalize_query_end_session(observed_at_utc)
            }
            WindowsLifecycleRaw::EndSession {
                observed_at_utc,
                ending,
            } => self.normalize_end_session(observed_at_utc, ending),
            WindowsLifecycleRaw::StartupBoundary { observed_at_utc } => {
                self.normalize_startup_boundary(observed_at_utc)
            }
        }
    }

    fn normalize_idle(
        &mut self,
        observed_at_utc: UtcMicros,
        last_input_tick_ms: u32,
        current_tick_ms: u32,
    ) -> LifecycleNormalization {
        let current = IdleSample::new(observed_at_utc, last_input_tick_ms, current_tick_ms);
        let previous = self.idle.previous;
        self.idle.previous = Some(current);

        if current.idle_elapsed_ms() < self.idle_threshold_ms {
            self.idle.crossed_threshold = false;
            return LifecycleNormalization::default();
        }
        if self.idle.crossed_threshold || self.session.is_attribution_blocked() {
            return LifecycleNormalization::default();
        }

        self.idle.crossed_threshold = true;
        let boundary = previous
            .filter(|sample| sample.is_trustworthy_predecessor(current, self.idle_threshold_ms))
            .and_then(|_| current.threshold_boundary(self.idle_threshold_ms))
            .map_or(
                IdleBoundary {
                    occurred_at_utc: observed_at_utc,
                    confidence: IdleBoundaryConfidence::ObservationClamped,
                },
                |occurred_at_utc| IdleBoundary {
                    occurred_at_utc,
                    confidence: IdleBoundaryConfidence::DerivedFromThreshold,
                },
            );
        LifecycleNormalization {
            idle_boundary: Some(boundary),
            ..LifecycleNormalization::adapter(
                boundary.occurred_at_utc(),
                AdapterObservationKind::IdleThresholdCrossed,
            )
        }
    }

    fn normalize_session_change(
        &mut self,
        observed_at_utc: UtcMicros,
        change: WindowsSessionChange,
    ) -> LifecycleNormalization {
        match change {
            WindowsSessionChange::Locked => self.enter_lock(observed_at_utc),
            WindowsSessionChange::Disconnected | WindowsSessionChange::LoggedOff => {
                self.enter_disconnect(observed_at_utc)
            }
            WindowsSessionChange::Unlocked
            | WindowsSessionChange::Connected
            | WindowsSessionChange::LoggedOn => self.leave_session_blocker(),
        }
    }

    fn enter_lock(&mut self, observed_at_utc: UtcMicros) -> LifecycleNormalization {
        if self.session.locked {
            return LifecycleNormalization::default();
        }
        self.session.locked = true;
        self.idle.reset();
        LifecycleNormalization::adapter(observed_at_utc, AdapterObservationKind::SessionLocked)
    }

    fn enter_disconnect(&mut self, observed_at_utc: UtcMicros) -> LifecycleNormalization {
        if self.session.disconnected {
            return LifecycleNormalization::default();
        }
        self.session.disconnected = true;
        self.idle.reset();
        LifecycleNormalization::adapter(
            observed_at_utc,
            AdapterObservationKind::SessionDisconnected,
        )
    }

    fn leave_session_blocker(&mut self) -> LifecycleNormalization {
        let was_blocked = self.session.locked || self.session.disconnected;
        self.session.locked = false;
        self.session.disconnected = false;
        self.idle.reset();
        if was_blocked {
            LifecycleNormalization::requires_reconciliation_and_rebaseline()
        } else {
            LifecycleNormalization::default()
        }
    }

    fn normalize_power(
        &mut self,
        observed_at_utc: UtcMicros,
        notification: WindowsPowerNotification,
    ) -> LifecycleNormalization {
        match notification {
            WindowsPowerNotification::Suspended => {
                if self.session.suspended {
                    return LifecycleNormalization::default();
                }
                self.session.suspended = true;
                self.end_session = EndSessionState::None;
                self.idle.reset();
                self.time_base = None;
                LifecycleNormalization {
                    follow_up: LifecycleFollowUp::ReconcileForegroundAndRebaselineTime,
                    ..LifecycleNormalization::adapter(
                        observed_at_utc,
                        AdapterObservationKind::SystemSuspended,
                    )
                }
            }
            WindowsPowerNotification::Resumed => {
                self.session.suspended = false;
                self.idle.reset();
                self.time_base = None;
                LifecycleNormalization::requires_reconciliation_and_rebaseline()
            }
        }
    }

    fn normalize_time_base(&mut self, current: WindowsTimeBaseSample) -> LifecycleNormalization {
        let previous = self.time_base.replace(current);
        let Some(previous) = previous else {
            return LifecycleNormalization::default();
        };
        let comparison = TimeBaseComparison::between(previous, current);
        if comparison.indicates_missed_low_power() {
            self.session.suspended = true;
            self.end_session = EndSessionState::None;
            self.idle.reset();
            return LifecycleNormalization {
                follow_up: LifecycleFollowUp::ReconcileForegroundAndRebaselineTime,
                ..LifecycleNormalization::adapter(
                    current.observed_at_utc(),
                    AdapterObservationKind::SystemSuspended,
                )
            };
        }
        if comparison.indicates_clock_discontinuity() {
            self.idle.reset();
            return LifecycleNormalization::clock_discontinuity(current.observed_at_utc());
        }
        LifecycleNormalization::default()
    }

    fn normalize_query_end_session(
        &mut self,
        observed_at_utc: UtcMicros,
    ) -> LifecycleNormalization {
        if matches!(self.end_session, EndSessionState::None) {
            self.end_session = EndSessionState::Proposed { observed_at_utc };
            return LifecycleNormalization {
                follow_up: LifecycleFollowUp::HighPriorityCheckpoint,
                ..LifecycleNormalization::default()
            };
        }
        LifecycleNormalization::default()
    }

    fn normalize_end_session(
        &mut self,
        observed_at_utc: UtcMicros,
        ending: bool,
    ) -> LifecycleNormalization {
        if !ending {
            self.end_session = EndSessionState::None;
            return LifecycleNormalization::default();
        }
        self.end_session = EndSessionState::Confirmed { observed_at_utc };
        self.idle.reset();
        LifecycleNormalization {
            follow_up: LifecycleFollowUp::BoundedFinalFlush,
            ..LifecycleNormalization::default()
        }
    }

    fn normalize_startup_boundary(&mut self, startup_at_utc: UtcMicros) -> LifecycleNormalization {
        self.idle.reset();
        self.time_base = None;
        let end_session = std::mem::replace(&mut self.end_session, EndSessionState::None);
        let Some(shutdown_at_utc) = end_session.confirmed_at() else {
            return LifecycleNormalization::requires_reconciliation_and_rebaseline();
        };
        if startup_at_utc <= shutdown_at_utc {
            return LifecycleNormalization::clock_discontinuity(startup_at_utc);
        }
        LifecycleNormalization {
            observation: Some(LifecycleObservation::ConfirmedPowerTransition {
                shutdown_at_utc,
                startup_at_utc,
            }),
            follow_up: LifecycleFollowUp::ReconcileForegroundAndRebaselineTime,
            ..LifecycleNormalization::default()
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct IdleSample {
    observed_at_utc: UtcMicros,
    last_input_tick_ms: u32,
    current_tick_ms: u32,
}

impl IdleSample {
    const fn new(
        observed_at_utc: UtcMicros,
        last_input_tick_ms: u32,
        current_tick_ms: u32,
    ) -> Self {
        Self {
            observed_at_utc,
            last_input_tick_ms,
            current_tick_ms,
        }
    }

    const fn idle_elapsed_ms(self) -> u32 {
        self.current_tick_ms.wrapping_sub(self.last_input_tick_ms)
    }

    fn is_trustworthy_predecessor(self, current: Self, threshold_ms: u32) -> bool {
        self.last_input_tick_ms == current.last_input_tick_ms
            && current.current_tick_ms.wrapping_sub(self.current_tick_ms)
                <= MAX_TRUSTED_IDLE_SAMPLE_GAP_MS
            && self.idle_elapsed_ms() < threshold_ms
            && current.idle_elapsed_ms() >= threshold_ms
    }

    fn threshold_boundary(self, threshold_ms: u32) -> Option<UtcMicros> {
        let elapsed_after_threshold_us = i64::from(self.idle_elapsed_ms() - threshold_ms) * 1_000;
        self.observed_at_utc
            .checked_sub(elapsed_after_threshold_us)
            .ok()
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct IdleTrackingState {
    previous: Option<IdleSample>,
    crossed_threshold: bool,
}

impl IdleTrackingState {
    const fn new() -> Self {
        Self {
            previous: None,
            crossed_threshold: false,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct SessionTrackingState {
    locked: bool,
    disconnected: bool,
    suspended: bool,
}

impl SessionTrackingState {
    const fn new() -> Self {
        Self {
            locked: false,
            disconnected: false,
            suspended: false,
        }
    }

    const fn is_attribution_blocked(self) -> bool {
        self.locked || self.disconnected || self.suspended
    }
}

#[derive(Clone, Copy, Debug)]
enum EndSessionState {
    None,
    Proposed { observed_at_utc: UtcMicros },
    Confirmed { observed_at_utc: UtcMicros },
}

impl EndSessionState {
    const fn confirmed_at(self) -> Option<UtcMicros> {
        match self {
            Self::Confirmed { observed_at_utc } => Some(observed_at_utc),
            Self::None | Self::Proposed { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TimeBaseComparison {
    wall: Option<i64>,
    monotonic: Option<u64>,
    unbiased: Option<u64>,
}

impl TimeBaseComparison {
    fn between(previous: WindowsTimeBaseSample, current: WindowsTimeBaseSample) -> Self {
        Self {
            wall: current
                .observed_at_utc
                .checked_difference(previous.observed_at_utc)
                .ok(),
            monotonic: current.monotonic_us.checked_sub(previous.monotonic_us),
            unbiased: current
                .unbiased_interrupt_us
                .checked_sub(previous.unbiased_interrupt_us),
        }
    }

    fn indicates_missed_low_power(self) -> bool {
        let (Some(monotonic), Some(unbiased)) = (self.monotonic, self.unbiased) else {
            return false;
        };
        monotonic.saturating_sub(unbiased) > TIME_BASE_TOLERANCE_US
    }

    fn indicates_clock_discontinuity(self) -> bool {
        let (Some(wall), Some(monotonic)) = (self.wall, self.monotonic) else {
            return true;
        };
        wall < 0 || abs_difference_us(wall, monotonic) > TIME_BASE_TOLERANCE_US
    }
}

fn abs_difference_us(signed: i64, unsigned: u64) -> u64 {
    let signed = i128::from(signed);
    let unsigned = i128::from(unsigned);
    u64::try_from((signed - unsigned).unsigned_abs()).unwrap_or(u64::MAX)
}

/// Decodes a Windows lifecycle message without retaining its HWND, session ID, or raw handles.
#[must_use]
pub(crate) fn decode_windows_lifecycle_message(
    message: u32,
    w_param: usize,
    observed_at_utc: UtcMicros,
) -> Option<WindowsLifecycleRaw> {
    match message {
        WM_TIMECHANGE_VALUE => Some(WindowsLifecycleRaw::TimeChange { observed_at_utc }),
        WM_QUERYENDSESSION_VALUE => Some(WindowsLifecycleRaw::QueryEndSession { observed_at_utc }),
        WM_ENDSESSION_VALUE => Some(WindowsLifecycleRaw::EndSession {
            observed_at_utc,
            ending: w_param != 0,
        }),
        WM_POWERBROADCAST_VALUE => decode_power_notification(w_param, observed_at_utc),
        WM_WTSSESSION_CHANGE_VALUE => decode_session_change(w_param, observed_at_utc),
        _ => None,
    }
}

fn decode_power_notification(
    w_param: usize,
    observed_at_utc: UtcMicros,
) -> Option<WindowsLifecycleRaw> {
    let code = u32::try_from(w_param).ok()?;
    let notification = match code {
        PBT_APMSUSPEND_VALUE => WindowsPowerNotification::Suspended,
        PBT_APMRESUMESUSPEND_VALUE | PBT_APMRESUMEAUTOMATIC_VALUE => {
            WindowsPowerNotification::Resumed
        }
        _ => return None,
    };
    Some(WindowsLifecycleRaw::Power {
        observed_at_utc,
        notification,
    })
}

fn decode_session_change(
    w_param: usize,
    observed_at_utc: UtcMicros,
) -> Option<WindowsLifecycleRaw> {
    let code = u32::try_from(w_param).ok()?;
    let change = match code {
        WTS_CONSOLE_CONNECT_VALUE | WTS_REMOTE_CONNECT_VALUE => WindowsSessionChange::Connected,
        WTS_CONSOLE_DISCONNECT_VALUE | WTS_REMOTE_DISCONNECT_VALUE => {
            WindowsSessionChange::Disconnected
        }
        WTS_SESSION_LOGON_VALUE => WindowsSessionChange::LoggedOn,
        WTS_SESSION_LOGOFF_VALUE => WindowsSessionChange::LoggedOff,
        WTS_SESSION_LOCK_VALUE => WindowsSessionChange::Locked,
        WTS_SESSION_UNLOCK_VALUE => WindowsSessionChange::Unlocked,
        _ => return None,
    };
    Some(WindowsLifecycleRaw::SessionChange {
        observed_at_utc,
        change,
    })
}

/// Failure from a native idle or time-base sampler without exposing a Windows error value.
#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowsLifecycleSampleError {
    /// `GetLastInputInfo` did not provide a session input tick.
    LastInputUnavailable,
    /// A native performance counter could not be read.
    PerformanceCounterUnavailable,
    /// The performance-counter frequency was not positive.
    InvalidPerformanceFrequency,
    /// A native timestamp could not be represented as UTC microseconds.
    TimestampOutOfRange,
}

/// Samples Windows' session-specific input time without making an attribution decision.
///
/// # Errors
///
/// Returns [`WindowsLifecycleSampleError::LastInputUnavailable`] when Windows cannot provide the
/// per-session input tick. The caller must publish explicit adapter unavailability in that case.
#[cfg(windows)]
pub(crate) fn sample_windows_idle(
    observed_at_utc: UtcMicros,
) -> Result<WindowsLifecycleRaw, WindowsLifecycleSampleError> {
    use std::mem::size_of;
    use windows::Win32::{
        System::SystemInformation::GetTickCount,
        UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO},
    };

    let cb_size = u32::try_from(size_of::<LASTINPUTINFO>())
        .map_err(|_| WindowsLifecycleSampleError::TimestampOutOfRange)?;
    let mut last_input = LASTINPUTINFO {
        cbSize: cb_size,
        ..Default::default()
    };
    // SAFETY: `last_input` is initialized, uniquely borrowed stack storage of the exact Win32
    // structure size. The call reads only session-local input metadata and retains no pointer.
    let received = unsafe { GetLastInputInfo(&raw mut last_input) };
    if !received.as_bool() {
        return Err(WindowsLifecycleSampleError::LastInputUnavailable);
    }
    // SAFETY: `GetTickCount` has no pointer or ownership precondition and returns a plain value.
    let current_tick_ms = unsafe { GetTickCount() };
    Ok(WindowsLifecycleRaw::IdleSample {
        observed_at_utc,
        last_input_tick_ms: last_input.dwTime,
        current_tick_ms,
    })
}

/// Samples the precise Windows wall, QPC, and unbiased-interrupt time bases together.
///
/// # Errors
///
/// Returns a typed private error when a counter cannot be read or represented. The caller must
/// retain uncertainty and retry later rather than reuse an older time-base sample.
#[cfg(windows)]
pub(crate) fn sample_windows_time_bases()
-> Result<WindowsTimeBaseSample, WindowsLifecycleSampleError> {
    use windows::Win32::System::{
        Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
        SystemInformation::GetSystemTimePreciseAsFileTime,
        WindowsProgramming::QueryUnbiasedInterruptTimePrecise,
    };

    let mut frequency = 0_i64;
    let mut counter = 0_i64;
    // SAFETY: Both pointers refer to initialized, uniquely borrowed stack integers. The Win32
    // functions write exactly one counter value and retain neither pointer.
    unsafe {
        QueryPerformanceFrequency(&raw mut frequency)
            .map_err(|_| WindowsLifecycleSampleError::PerformanceCounterUnavailable)?;
        QueryPerformanceCounter(&raw mut counter)
            .map_err(|_| WindowsLifecycleSampleError::PerformanceCounterUnavailable)?;
    }
    if frequency <= 0 {
        return Err(WindowsLifecycleSampleError::InvalidPerformanceFrequency);
    }
    // SAFETY: These Win32 calls take no caller-owned pointers. They return value snapshots that
    // the adapter immediately converts; no handle, pointer, or native error crosses the module.
    let (file_time, unbiased_interrupt_100ns) = unsafe {
        (
            GetSystemTimePreciseAsFileTime(),
            QueryUnbiasedInterruptTimePrecise(),
        )
    };
    let observed_at_utc =
        utc_micros_from_file_time(file_time.dwLowDateTime, file_time.dwHighDateTime)?;
    let monotonic_us = qpc_to_micros(counter, frequency)?;
    let unbiased_interrupt_us = unbiased_interrupt_100ns / 10;
    Ok(WindowsTimeBaseSample::new(
        observed_at_utc,
        monotonic_us,
        unbiased_interrupt_us,
    ))
}

#[cfg(windows)]
fn utc_micros_from_file_time(
    low_date_time: u32,
    high_date_time: u32,
) -> Result<UtcMicros, WindowsLifecycleSampleError> {
    const WINDOWS_TO_UNIX_EPOCH_US: i64 = 11_644_473_600_000_000;

    let file_time_100ns = (u64::from(high_date_time) << 32) | u64::from(low_date_time);
    let micros_since_windows_epoch = i64::try_from(file_time_100ns / 10)
        .map_err(|_| WindowsLifecycleSampleError::TimestampOutOfRange)?;
    let micros_since_unix_epoch = micros_since_windows_epoch
        .checked_sub(WINDOWS_TO_UNIX_EPOCH_US)
        .ok_or(WindowsLifecycleSampleError::TimestampOutOfRange)?;
    Ok(UtcMicros::new(micros_since_unix_epoch))
}

#[cfg(windows)]
fn qpc_to_micros(counter: i64, frequency: i64) -> Result<u64, WindowsLifecycleSampleError> {
    let counter = u64::try_from(counter)
        .map_err(|_| WindowsLifecycleSampleError::PerformanceCounterUnavailable)?;
    let frequency = u64::try_from(frequency)
        .map_err(|_| WindowsLifecycleSampleError::InvalidPerformanceFrequency)?;
    let seconds = counter / frequency;
    let remainder = counter % frequency;
    let micros = u128::from(seconds)
        .checked_mul(1_000_000)
        .and_then(|whole| {
            whole.checked_add(u128::from(remainder).checked_mul(1_000_000)? / u128::from(frequency))
        })
        .ok_or(WindowsLifecycleSampleError::TimestampOutOfRange)?;
    u64::try_from(micros).map_err(|_| WindowsLifecycleSampleError::TimestampOutOfRange)
}

#[cfg(test)]
mod tests {
    use super::{
        IdleBoundaryConfidence, LifecycleObservation, WindowsLifecycleNormalizer,
        WindowsLifecycleRaw, WindowsPowerNotification, WindowsSessionChange, WindowsTimeBaseSample,
        decode_windows_lifecycle_message,
    };
    use crate::AdapterObservationKind;
    use openmanic_application::UtcMicros;

    #[test]
    fn stable_idle_samples_derive_the_configured_threshold_boundary() {
        let mut normalizer = WindowsLifecycleNormalizer::new(5_000);

        assert!(normalizer.normalize(idle(0, 0, 0)).observation().is_none());
        let crossing = normalizer.normalize(idle(5_000_000, 0, 5_000));

        assert_eq!(
            crossing
                .idle_boundary()
                .map(super::IdleBoundary::occurred_at_utc),
            Some(time(5_000_000))
        );
        assert_eq!(
            crossing
                .idle_boundary()
                .map(super::IdleBoundary::confidence),
            Some(IdleBoundaryConfidence::DerivedFromThreshold)
        );
        assert_adapter_kind(
            crossing.observation(),
            AdapterObservationKind::IdleThresholdCrossed,
        );
    }

    #[test]
    fn unusual_idle_ticks_clamp_at_the_observation_without_negative_time() {
        let mut normalizer = WindowsLifecycleNormalizer::new(5_000);

        assert!(
            normalizer
                .normalize(idle(1_000_000, 900, 1_000))
                .observation()
                .is_none()
        );
        let crossing = normalizer.normalize(idle(2_000_000, 9_000, 1_000));

        assert_eq!(
            crossing
                .idle_boundary()
                .map(super::IdleBoundary::occurred_at_utc),
            Some(time(2_000_000))
        );
        assert_eq!(
            crossing
                .idle_boundary()
                .map(super::IdleBoundary::confidence),
            Some(IdleBoundaryConfidence::ObservationClamped)
        );
    }

    #[test]
    fn lock_suspend_and_disconnect_suppress_idle_until_fresh_reconciliation() {
        let mut normalizer = WindowsLifecycleNormalizer::new(1_000);

        let locked = normalizer.normalize(session(10, WindowsSessionChange::Locked));
        assert_adapter_kind(locked.observation(), AdapterObservationKind::SessionLocked);
        assert!(
            normalizer
                .normalize(idle(20, 0, 2_000))
                .observation()
                .is_none()
        );

        let unlocked = normalizer.normalize(session(30, WindowsSessionChange::Unlocked));
        assert!(unlocked.foreground_reconciliation_required());
        assert!(unlocked.time_base_rebaseline_required());

        let suspended = normalizer.normalize(power(40, WindowsPowerNotification::Suspended));
        assert_adapter_kind(
            suspended.observation(),
            AdapterObservationKind::SystemSuspended,
        );
        assert!(
            normalizer
                .normalize(idle(50, 0, 3_000))
                .observation()
                .is_none()
        );

        let resumed = normalizer.normalize(power(60, WindowsPowerNotification::Resumed));
        assert!(resumed.foreground_reconciliation_required());
        let disconnected = normalizer.normalize(session(70, WindowsSessionChange::Disconnected));
        assert_adapter_kind(
            disconnected.observation(),
            AdapterObservationKind::SessionDisconnected,
        );
    }

    #[test]
    fn time_base_missed_low_power_precedes_clock_uncertainty_and_never_powers_off() {
        let mut normalizer = WindowsLifecycleNormalizer::default();

        assert!(
            normalizer
                .normalize(time_base(0, 0, 0))
                .observation()
                .is_none()
        );
        let suspended = normalizer.normalize(time_base(10_000_000, 10_000_000, 1_000_000));

        assert_adapter_kind(
            suspended.observation(),
            AdapterObservationKind::SystemSuspended,
        );
        assert!(suspended.foreground_reconciliation_required());
        assert!(suspended.time_base_rebaseline_required());
    }

    #[test]
    fn wall_clock_discontinuity_rebaselines_without_powered_off_inference() {
        let mut normalizer = WindowsLifecycleNormalizer::default();

        assert!(
            normalizer
                .normalize(time_base(10_000_000, 10_000_000, 10_000_000))
                .observation()
                .is_none()
        );
        let discontinuity = normalizer.normalize(time_base(5_000_000, 20_000_000, 20_000_000));

        assert_adapter_kind(
            discontinuity.observation(),
            AdapterObservationKind::ClockDiscontinuity,
        );
        assert!(discontinuity.foreground_reconciliation_required());
        assert!(discontinuity.time_base_rebaseline_required());
    }

    #[test]
    fn cancelled_shutdown_never_creates_a_power_transition() {
        let mut normalizer = WindowsLifecycleNormalizer::default();

        let proposal = normalizer.normalize(WindowsLifecycleRaw::QueryEndSession {
            observed_at_utc: time(10),
        });
        assert!(proposal.checkpoint_requested());
        let cancellation = normalizer.normalize(WindowsLifecycleRaw::EndSession {
            observed_at_utc: time(20),
            ending: false,
        });
        assert!(!cancellation.final_flush_permitted());
        let startup = normalizer.normalize(WindowsLifecycleRaw::StartupBoundary {
            observed_at_utc: time(30),
        });

        assert!(startup.observation().is_none());
        assert!(startup.foreground_reconciliation_required());
    }

    #[test]
    fn affirmative_end_session_and_later_startup_are_required_for_power_transition() {
        let mut normalizer = WindowsLifecycleNormalizer::default();

        assert!(
            normalizer
                .normalize(WindowsLifecycleRaw::QueryEndSession {
                    observed_at_utc: time(10),
                })
                .checkpoint_requested()
        );
        let ending = normalizer.normalize(WindowsLifecycleRaw::EndSession {
            observed_at_utc: time(20),
            ending: true,
        });
        assert!(ending.final_flush_permitted());
        let startup = normalizer.normalize(WindowsLifecycleRaw::StartupBoundary {
            observed_at_utc: time(30),
        });

        assert_eq!(
            startup.observation(),
            Some(LifecycleObservation::ConfirmedPowerTransition {
                shutdown_at_utc: time(20),
                startup_at_utc: time(30),
            })
        );
        assert!(startup.foreground_reconciliation_required());
    }

    #[test]
    fn decoded_raw_messages_preserve_wts_power_and_cancel_semantics() {
        let lock = decode_windows_lifecycle_message(0x02B1, 7, time(1));
        let suspend = decode_windows_lifecycle_message(0x0218, 4, time(2));
        let cancelled_end = decode_windows_lifecycle_message(0x0016, 0, time(3));

        assert_eq!(
            lock,
            Some(WindowsLifecycleRaw::SessionChange {
                observed_at_utc: time(1),
                change: WindowsSessionChange::Locked,
            })
        );
        assert_eq!(
            suspend,
            Some(WindowsLifecycleRaw::Power {
                observed_at_utc: time(2),
                notification: WindowsPowerNotification::Suspended,
            })
        );
        assert_eq!(
            cancelled_end,
            Some(WindowsLifecycleRaw::EndSession {
                observed_at_utc: time(3),
                ending: false,
            })
        );
    }

    fn idle(at_us: i64, last_input_tick_ms: u32, current_tick_ms: u32) -> WindowsLifecycleRaw {
        WindowsLifecycleRaw::IdleSample {
            observed_at_utc: time(at_us),
            last_input_tick_ms,
            current_tick_ms,
        }
    }

    fn session(at_us: i64, change: WindowsSessionChange) -> WindowsLifecycleRaw {
        WindowsLifecycleRaw::SessionChange {
            observed_at_utc: time(at_us),
            change,
        }
    }

    fn power(at_us: i64, notification: WindowsPowerNotification) -> WindowsLifecycleRaw {
        WindowsLifecycleRaw::Power {
            observed_at_utc: time(at_us),
            notification,
        }
    }

    fn time_base(at_us: i64, monotonic_us: u64, unbiased_interrupt_us: u64) -> WindowsLifecycleRaw {
        WindowsLifecycleRaw::TimeBaseSample(WindowsTimeBaseSample::new(
            time(at_us),
            monotonic_us,
            unbiased_interrupt_us,
        ))
    }

    fn time(value: i64) -> UtcMicros {
        UtcMicros::new(value)
    }

    fn assert_adapter_kind(
        observation: Option<LifecycleObservation>,
        expected_kind: AdapterObservationKind,
    ) {
        assert!(matches!(
            observation,
            Some(LifecycleObservation::Adapter { kind, .. }) if kind == expected_kind
        ));
    }
}
