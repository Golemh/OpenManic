//! Activity state, cause, and qualifying evidence invariants.

use crate::{ApplicationId, HalfOpenInterval, TrackerRunId, UtcMicros};
use core::fmt;

/// Canonical state of a positive activity interval.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ActivityState {
    /// A resolved application was in the foreground.
    Active,
    /// Tracking continued but the user met the idle threshold.
    Idle,
    /// The user explicitly paused tracking.
    PausedByUser,
    /// The foreground application was excluded by policy.
    Excluded,
    /// The platform could not provide usable tracking evidence.
    Unavailable,
    /// Affirmative shutdown and later startup evidence bound this interval.
    PoweredOff,
    /// Evidence is missing or contradictory and must not be guessed.
    UnknownMissing,
}

/// Explicit reason that explains an [`ActivityState`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ActivityCause {
    /// A resolved foreground application observation.
    ForegroundApplication,
    /// The configured idle threshold elapsed.
    IdleThreshold,
    /// A user requested pause.
    UserPause,
    /// An application matched an exclusion policy.
    ApplicationExcluded,
    /// The session became locked.
    SessionLocked,
    /// The session disconnected.
    SessionDisconnected,
    /// The system suspended or hibernated; the distinction is intentionally unavailable.
    SystemSuspended,
    /// The adapter is starting and has not yet produced trusted evidence.
    AdapterStarting,
    /// The adapter lost a required permission.
    AdapterPermissionLost,
    /// The adapter reported a non-permission failure.
    AdapterFailure,
    /// A bounded evidence queue overflowed.
    EvidenceQueueOverflow,
    /// The operating system supplied affirmative shutdown or end-session evidence.
    ConfirmedShutdown,
    /// Recovery found a prior process gap without qualifying power evidence.
    CrashRecoveryGap,
    /// Imported history did not contain a trustworthy source cause.
    ImportedUnknown,
    /// A wall-clock discontinuity makes normal attribution uncertain.
    ClockDiscontinuity,
}

/// Confirmed shutdown and later startup boundaries that qualify a Powered Off interval.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PowerTransitionEvidence {
    shutdown: UtcMicros,
    startup: UtcMicros,
}

impl PowerTransitionEvidence {
    /// Creates qualifying power-transition evidence.
    ///
    /// # Errors
    ///
    /// Returns [`PowerTransitionEvidenceError::StartupNotAfterShutdown`] unless startup is
    /// strictly later than shutdown.
    pub fn try_new(
        shutdown: UtcMicros,
        startup: UtcMicros,
    ) -> Result<Self, PowerTransitionEvidenceError> {
        if startup <= shutdown {
            return Err(PowerTransitionEvidenceError::StartupNotAfterShutdown {
                shutdown,
                startup,
            });
        }
        Ok(Self { shutdown, startup })
    }

    /// Returns the affirmative shutdown or end-session boundary.
    #[must_use]
    pub const fn shutdown(self) -> UtcMicros {
        self.shutdown
    }

    /// Returns the later startup boundary.
    #[must_use]
    pub const fn startup(self) -> UtcMicros {
        self.startup
    }
}

/// Failure while constructing qualifying power-transition evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerTransitionEvidenceError {
    /// The asserted startup boundary was not later than the shutdown boundary.
    StartupNotAfterShutdown {
        /// Confirmed shutdown or end-session instant.
        shutdown: UtcMicros,
        /// Requested startup instant.
        startup: UtcMicros,
    },
}

impl fmt::Display for PowerTransitionEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StartupNotAfterShutdown { shutdown, startup } => write!(
                formatter,
                "startup instant {} must be later than shutdown instant {}",
                startup.get(),
                shutdown.get()
            ),
        }
    }
}

impl std::error::Error for PowerTransitionEvidenceError {}

/// Private construction state for evidence paired with an activity interval.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum EvidenceKind {
    Observed,
    PowerTransition(PowerTransitionEvidence),
}

/// Evidence that supplies a cause and, when required, qualifying power boundaries.
///
/// The fields are private so callers cannot pair [`ActivityCause::ConfirmedShutdown`] with
/// missing startup evidence and then construct [`ActivityState::PoweredOff`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ActivityEvidence {
    cause: ActivityCause,
    kind: EvidenceKind,
}

impl ActivityEvidence {
    /// Creates ordinary evidence for a non-shutdown cause.
    ///
    /// # Errors
    ///
    /// Returns [`ActivityEvidenceError::ConfirmedShutdownRequiresPowerTransition`] when the
    /// cause needs affirmative shutdown and startup boundaries instead.
    pub fn try_from_cause(cause: ActivityCause) -> Result<Self, ActivityEvidenceError> {
        if cause == ActivityCause::ConfirmedShutdown {
            return Err(ActivityEvidenceError::ConfirmedShutdownRequiresPowerTransition);
        }
        Ok(Self {
            cause,
            kind: EvidenceKind::Observed,
        })
    }

    /// Creates the only evidence form that can qualify [`ActivityState::PoweredOff`].
    #[must_use]
    pub const fn confirmed_shutdown(boundaries: PowerTransitionEvidence) -> Self {
        Self {
            cause: ActivityCause::ConfirmedShutdown,
            kind: EvidenceKind::PowerTransition(boundaries),
        }
    }

    /// Returns the explicit source cause.
    #[must_use]
    pub const fn cause(self) -> ActivityCause {
        self.cause
    }

    /// Returns qualifying shutdown/startup evidence only when it exists.
    #[must_use]
    pub const fn power_transition(self) -> Option<PowerTransitionEvidence> {
        match self.kind {
            EvidenceKind::Observed => None,
            EvidenceKind::PowerTransition(boundaries) => Some(boundaries),
        }
    }
}

/// Failure while constructing state evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivityEvidenceError {
    /// Confirmed shutdown requires both the affirmative shutdown and later startup boundaries.
    ConfirmedShutdownRequiresPowerTransition,
}

impl fmt::Display for ActivityEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("confirmed shutdown requires qualifying power-transition evidence")
    }
}

impl std::error::Error for ActivityEvidenceError {}

/// A canonical activity interval with state/cause/application invariants enforced at creation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ActivityInterval {
    tracker_run_id: TrackerRunId,
    range: HalfOpenInterval,
    state: ActivityState,
    evidence: ActivityEvidence,
    application_id: Option<ApplicationId>,
}

impl ActivityInterval {
    /// Creates a canonical activity interval from positive time, state, and qualifying evidence.
    ///
    /// # Errors
    ///
    /// Returns [`ActivityInvariantError`] when application association or Powered Off evidence
    /// does not satisfy the activity-state invariants.
    pub fn try_new(
        tracker_run_id: TrackerRunId,
        range: HalfOpenInterval,
        state: ActivityState,
        evidence: ActivityEvidence,
        application_id: Option<ApplicationId>,
    ) -> Result<Self, ActivityInvariantError> {
        if state == ActivityState::Active && application_id.is_none() {
            return Err(ActivityInvariantError::ApplicationRequiredForActive);
        }
        if state != ActivityState::Active && application_id.is_some() {
            return Err(ActivityInvariantError::ApplicationForbiddenForState { state });
        }
        if state == ActivityState::PoweredOff && evidence.power_transition().is_none() {
            return Err(ActivityInvariantError::PoweredOffRequiresQualifyingEvidence);
        }
        if state != ActivityState::PoweredOff && evidence.power_transition().is_some() {
            return Err(ActivityInvariantError::PowerTransitionEvidenceRequiresPoweredOffState);
        }
        Ok(Self {
            tracker_run_id,
            range,
            state,
            evidence,
            application_id,
        })
    }

    /// Returns the tracker run that recorded this interval.
    #[must_use]
    pub const fn tracker_run_id(self) -> TrackerRunId {
        self.tracker_run_id
    }

    /// Returns the positive half-open temporal range.
    #[must_use]
    pub const fn range(self) -> HalfOpenInterval {
        self.range
    }

    /// Returns the canonical state.
    #[must_use]
    pub const fn state(self) -> ActivityState {
        self.state
    }

    /// Returns the explicit cause paired with the state.
    #[must_use]
    pub const fn cause(self) -> ActivityCause {
        self.evidence.cause()
    }

    /// Returns the source evidence, including qualifying power boundaries when present.
    #[must_use]
    pub const fn evidence(self) -> ActivityEvidence {
        self.evidence
    }

    /// Returns the resolved application only for [`ActivityState::Active`].
    #[must_use]
    pub const fn application_id(self) -> Option<ApplicationId> {
        self.application_id
    }
}

/// Failure while constructing a canonical activity interval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivityInvariantError {
    /// Active attribution is meaningless without a resolved application ID.
    ApplicationRequiredForActive,
    /// The supplied state must not retain an application association.
    ApplicationForbiddenForState {
        /// State that rejects application details.
        state: ActivityState,
    },
    /// Powered Off requires affirmative shutdown and a later startup boundary.
    PoweredOffRequiresQualifyingEvidence,
    /// Power-transition evidence is meaningful only for the Powered Off state.
    PowerTransitionEvidenceRequiresPoweredOffState,
}

impl fmt::Display for ActivityInvariantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApplicationRequiredForActive => {
                formatter.write_str("active activity requires a resolved application ID")
            }
            Self::ApplicationForbiddenForState { state } => {
                write!(
                    formatter,
                    "{state:?} activity must not retain an application ID"
                )
            }
            Self::PoweredOffRequiresQualifyingEvidence => formatter.write_str(
                "powered-off activity requires confirmed shutdown and later startup evidence",
            ),
            Self::PowerTransitionEvidenceRequiresPoweredOffState => formatter.write_str(
                "power-transition evidence may only be paired with powered-off activity",
            ),
        }
    }
}

impl std::error::Error for ActivityInvariantError {}

#[cfg(test)]
mod tests {
    use super::{
        ActivityCause, ActivityEvidence, ActivityInvariantError, ActivityState,
        PowerTransitionEvidence, PowerTransitionEvidenceError,
    };
    use crate::{ApplicationId, HalfOpenInterval, TrackerRunId, UtcMicros};

    fn interval() -> HalfOpenInterval {
        HalfOpenInterval::try_new(UtcMicros::new(10), UtcMicros::new(20)).expect("positive")
    }

    fn tracker_run_id() -> TrackerRunId {
        TrackerRunId::from_bytes([3; 16])
    }

    fn application_id() -> ApplicationId {
        ApplicationId::from_bytes([4; 16])
    }

    #[test]
    fn active_requires_application_and_other_states_reject_one() {
        let active_evidence =
            ActivityEvidence::try_from_cause(ActivityCause::ForegroundApplication)
                .expect("ordinary evidence");
        assert_eq!(
            super::ActivityInterval::try_new(
                tracker_run_id(),
                interval(),
                ActivityState::Active,
                active_evidence,
                None,
            ),
            Err(ActivityInvariantError::ApplicationRequiredForActive)
        );
        assert!(
            super::ActivityInterval::try_new(
                tracker_run_id(),
                interval(),
                ActivityState::Active,
                active_evidence,
                Some(application_id()),
            )
            .is_ok()
        );
        let idle_evidence = ActivityEvidence::try_from_cause(ActivityCause::IdleThreshold)
            .expect("ordinary evidence");
        assert!(matches!(
            super::ActivityInterval::try_new(
                tracker_run_id(),
                interval(),
                ActivityState::Idle,
                idle_evidence,
                Some(application_id()),
            ),
            Err(ActivityInvariantError::ApplicationForbiddenForState {
                state: ActivityState::Idle
            })
        ));
    }

    #[test]
    fn powered_off_needs_confirmed_shutdown_and_later_startup() {
        assert_eq!(
            PowerTransitionEvidence::try_new(UtcMicros::new(20), UtcMicros::new(20)),
            Err(PowerTransitionEvidenceError::StartupNotAfterShutdown {
                shutdown: UtcMicros::new(20),
                startup: UtcMicros::new(20),
            })
        );
        let boundaries = PowerTransitionEvidence::try_new(UtcMicros::new(12), UtcMicros::new(18))
            .expect("later startup");
        let evidence = ActivityEvidence::confirmed_shutdown(boundaries);
        let activity = super::ActivityInterval::try_new(
            tracker_run_id(),
            interval(),
            ActivityState::PoweredOff,
            evidence,
            None,
        );
        assert_eq!(
            activity.map(super::ActivityInterval::cause),
            Ok(ActivityCause::ConfirmedShutdown)
        );
        assert_eq!(
            activity.map(|value| value.evidence().power_transition()),
            Ok(Some(boundaries))
        );
        assert_eq!(
            super::ActivityInterval::try_new(
                tracker_run_id(),
                interval(),
                ActivityState::Active,
                evidence,
                Some(application_id()),
            ),
            Err(ActivityInvariantError::PowerTransitionEvidenceRequiresPoweredOffState)
        );
    }

    #[test]
    fn gaps_sleep_and_adapter_loss_cannot_construct_powered_off() {
        for cause in [
            ActivityCause::CrashRecoveryGap,
            ActivityCause::SystemSuspended,
            ActivityCause::AdapterPermissionLost,
            ActivityCause::AdapterFailure,
            ActivityCause::EvidenceQueueOverflow,
        ] {
            let evidence = ActivityEvidence::try_from_cause(cause).expect("ordinary evidence");
            assert_eq!(
                super::ActivityInterval::try_new(
                    tracker_run_id(),
                    interval(),
                    ActivityState::PoweredOff,
                    evidence,
                    None,
                ),
                Err(ActivityInvariantError::PoweredOffRequiresQualifyingEvidence),
                "{cause:?} must remain non-powered-off evidence"
            );
        }
    }
}
