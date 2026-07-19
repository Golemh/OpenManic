//! The pure focus-session state machine.

use crate::{CategoryId, UtcMicros, UtcMicrosArithmeticError};

/// A single focus session and its scheduling metadata.
///
/// A session can be in exactly one [`FocusSessionState`]. Consequently, one
/// session can never be both running and paused. Coordination between multiple
/// sessions belongs to a higher-level collection or command handler.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusSession {
    category_id: Option<CategoryId>,
    intended_duration_us: i64,
    state: FocusSessionState,
}

impl FocusSession {
    /// Creates an unplanned session with a strictly positive intended duration.
    ///
    /// # Errors
    ///
    /// Returns [`FocusSessionError::NonPositiveIntendedDuration`] if the
    /// intended duration is zero or negative.
    pub fn try_new(
        intended_duration_us: i64,
        category_id: Option<CategoryId>,
    ) -> Result<Self, FocusSessionError> {
        if intended_duration_us <= 0 {
            return Err(FocusSessionError::NonPositiveIntendedDuration);
        }

        Ok(Self {
            category_id,
            intended_duration_us,
            state: FocusSessionState::Ready,
        })
    }

    /// Restores a session whose complete state was already validated by a
    /// durable boundary.
    ///
    /// This still validates the cross-field invariants so a malformed stored
    /// value cannot become an authoritative in-memory timer.
    ///
    /// # Errors
    ///
    /// Returns [`FocusSessionError::InvalidRestoredState`] when the duration
    /// or lifecycle payload is not representable by the domain state machine.
    pub fn try_restore(
        intended_duration_us: i64,
        category_id: Option<CategoryId>,
        state: FocusSessionState,
    ) -> Result<Self, FocusSessionError> {
        if intended_duration_us <= 0 || !state_is_valid(state) {
            return Err(FocusSessionError::InvalidRestoredState);
        }

        Ok(Self {
            category_id,
            intended_duration_us,
            state,
        })
    }

    /// Returns the session's optional category.
    #[must_use]
    pub const fn category_id(&self) -> Option<CategoryId> {
        self.category_id
    }

    /// Returns the configured duration in microseconds.
    #[must_use]
    pub const fn intended_duration_us(&self) -> i64 {
        self.intended_duration_us
    }

    /// Returns the session's current state.
    #[must_use]
    pub const fn state(&self) -> FocusSessionState {
        self.state
    }

    /// Returns whether the session currently occupies an active focus slot.
    ///
    /// This is true only for the mutually exclusive running and paused states.
    #[must_use]
    pub const fn is_active_or_paused(&self) -> bool {
        matches!(
            self.state,
            FocusSessionState::Running { .. } | FocusSessionState::Paused { .. }
        )
    }

    /// Records a positive planned wall-clock interval for a ready session.
    ///
    /// # Errors
    ///
    /// Returns [`FocusSessionError::PlannedEndNotAfterStart`] for a nonpositive
    /// interval, or [`FocusSessionError::InvalidTransition`] unless the session
    /// is ready.
    pub fn plan(
        &mut self,
        planned_start: UtcMicros,
        planned_end: UtcMicros,
    ) -> Result<(), FocusSessionError> {
        if planned_end <= planned_start {
            return Err(FocusSessionError::PlannedEndNotAfterStart);
        }
        if self.state != FocusSessionState::Ready {
            return Err(self.invalid_transition("plan"));
        }

        self.state = FocusSessionState::Planned {
            planned_start,
            planned_end,
        };
        Ok(())
    }

    /// Starts a ready or planned session at the supplied UTC timestamp.
    ///
    /// Starting is explicit: a planned time never starts a session by itself.
    ///
    /// # Errors
    ///
    /// Returns [`FocusSessionError::InvalidTransition`] unless the session is
    /// ready or planned, and [`FocusSessionError::TimeArithmetic`] if the
    /// deadline cannot be represented.
    pub fn start(&mut self, started_at: UtcMicros) -> Result<(), FocusSessionError> {
        if !matches!(
            self.state,
            FocusSessionState::Ready | FocusSessionState::Planned { .. }
        ) {
            return Err(self.invalid_transition("start"));
        }

        let deadline = started_at
            .checked_add(self.intended_duration_us)
            .map_err(FocusSessionError::TimeArithmetic)?;
        self.state = FocusSessionState::Running {
            started_at,
            deadline,
        };
        Ok(())
    }

    /// Pauses a running session at `paused_at`.
    ///
    /// If the deadline has already passed, the session completes at its
    /// deadline instead of entering the paused state.
    ///
    /// # Errors
    ///
    /// Returns [`FocusSessionError::InvalidTransition`] unless the session is
    /// running, and [`FocusSessionError::TimeArithmetic`] if the remaining
    /// duration cannot be represented.
    pub fn pause(&mut self, paused_at: UtcMicros) -> Result<(), FocusSessionError> {
        let FocusSessionState::Running {
            started_at,
            deadline,
        } = self.state
        else {
            return Err(self.invalid_transition("pause"));
        };

        if paused_at >= deadline {
            self.state = FocusSessionState::Completed {
                started_at,
                completed_at: deadline,
            };
            return Ok(());
        }

        let remaining_us = deadline
            .checked_difference(paused_at)
            .map_err(FocusSessionError::TimeArithmetic)?;
        self.state = FocusSessionState::Paused {
            started_at,
            remaining_us,
        };
        Ok(())
    }

    /// Resumes a paused session at `resumed_at`.
    ///
    /// Paused time does not count toward the session's new deadline.
    ///
    /// # Errors
    ///
    /// Returns [`FocusSessionError::InvalidTransition`] unless the session is
    /// paused, and [`FocusSessionError::TimeArithmetic`] if the new deadline
    /// cannot be represented.
    pub fn resume(&mut self, resumed_at: UtcMicros) -> Result<(), FocusSessionError> {
        let FocusSessionState::Paused {
            started_at,
            remaining_us,
        } = self.state
        else {
            return Err(self.invalid_transition("resume"));
        };

        let deadline = resumed_at
            .checked_add(remaining_us)
            .map_err(FocusSessionError::TimeArithmetic)?;
        self.state = FocusSessionState::Running {
            started_at,
            deadline,
        };
        Ok(())
    }

    /// Completes a running or paused session at the supplied timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`FocusSessionError::InvalidTransition`] unless the session is
    /// running or paused.
    pub fn complete(&mut self, completed_at: UtcMicros) -> Result<(), FocusSessionError> {
        let (FocusSessionState::Running { started_at, .. }
        | FocusSessionState::Paused { started_at, .. }) = self.state
        else {
            return Err(self.invalid_transition("complete"));
        };

        self.state = FocusSessionState::Completed {
            started_at,
            completed_at,
        };
        Ok(())
    }

    /// Cancels a running or paused session at the supplied timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`FocusSessionError::InvalidTransition`] unless the session is
    /// running or paused.
    pub fn cancel(&mut self, cancelled_at: UtcMicros) -> Result<(), FocusSessionError> {
        let (FocusSessionState::Running { started_at, .. }
        | FocusSessionState::Paused { started_at, .. }) = self.state
        else {
            return Err(self.invalid_transition("cancel"));
        };

        self.state = FocusSessionState::Cancelled {
            started_at,
            cancelled_at,
        };
        Ok(())
    }

    /// Reconciles a persisted session against a supplied restart timestamp.
    ///
    /// A running session whose deadline has passed completes at that deadline;
    /// a running session before its deadline and every paused session are left
    /// unchanged. Supplying the timestamp keeps restart handling deterministic.
    pub fn reconcile_after_restart(&mut self, restarted_at: UtcMicros) -> FocusSessionState {
        if let FocusSessionState::Running {
            started_at,
            deadline,
        } = self.state
            && restarted_at >= deadline
        {
            self.state = FocusSessionState::Completed {
                started_at,
                completed_at: deadline,
            };
        }

        self.state
    }

    fn invalid_transition(&self, operation: &'static str) -> FocusSessionError {
        FocusSessionError::InvalidTransition {
            operation,
            state: self.state,
        }
    }
}

fn state_is_valid(state: FocusSessionState) -> bool {
    match state {
        FocusSessionState::Ready => true,
        FocusSessionState::Planned {
            planned_start,
            planned_end,
        } => planned_end > planned_start,
        FocusSessionState::Running {
            started_at,
            deadline,
        } => deadline > started_at,
        FocusSessionState::Paused { remaining_us, .. } => remaining_us > 0,
        FocusSessionState::Completed {
            started_at,
            completed_at,
        }
        | FocusSessionState::Cancelled {
            started_at,
            cancelled_at: completed_at,
        } => completed_at >= started_at,
    }
}

/// The lifecycle state of a [`FocusSession`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusSessionState {
    /// An unplanned draft that has not started.
    Ready,
    /// A draft with a planned positive UTC interval.
    Planned {
        /// The planned start timestamp.
        planned_start: UtcMicros,
        /// The planned end timestamp.
        planned_end: UtcMicros,
    },
    /// A session currently counting down to its UTC deadline.
    Running {
        /// The explicit timestamp at which the session began.
        started_at: UtcMicros,
        /// The UTC timestamp at which the session completes automatically.
        deadline: UtcMicros,
    },
    /// A session with its remaining duration frozen.
    Paused {
        /// The explicit timestamp at which the session began.
        started_at: UtcMicros,
        /// The positive duration that remains when the session resumes.
        remaining_us: i64,
    },
    /// A session completed either explicitly or when its deadline elapsed.
    Completed {
        /// The explicit timestamp at which the session began.
        started_at: UtcMicros,
        /// The timestamp recorded for completion.
        completed_at: UtcMicros,
    },
    /// A session cancelled explicitly before completion.
    Cancelled {
        /// The explicit timestamp at which the session began.
        started_at: UtcMicros,
        /// The timestamp recorded for cancellation.
        cancelled_at: UtcMicros,
    },
}

/// A validation, arithmetic, or lifecycle-transition error for a focus session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FocusSessionError {
    /// The requested intended duration was zero or negative.
    NonPositiveIntendedDuration,
    /// The planned interval did not have a positive duration.
    PlannedEndNotAfterStart,
    /// The requested operation is not valid from the current lifecycle state.
    InvalidTransition {
        /// The operation that was attempted.
        operation: &'static str,
        /// The state from which the operation was attempted.
        state: FocusSessionState,
    },
    /// A timestamp calculation could not be represented as UTC microseconds.
    TimeArithmetic(UtcMicrosArithmeticError),
    /// A persisted state did not satisfy the domain's cross-field invariants.
    InvalidRestoredState,
}

#[cfg(test)]
mod tests {
    use super::{FocusSession, FocusSessionError, FocusSessionState};
    use crate::UtcMicros;

    fn micros(value: i64) -> UtcMicros {
        UtcMicros::new(value)
    }

    fn session() -> FocusSession {
        FocusSession::try_new(50, None).expect("the fixture duration is positive")
    }

    #[test]
    fn rejects_non_positive_intended_duration() {
        assert_eq!(
            FocusSession::try_new(0, None),
            Err(FocusSessionError::NonPositiveIntendedDuration)
        );
    }

    #[test]
    fn plans_then_starts_at_an_explicit_time() {
        let mut session = session();

        session
            .plan(micros(100), micros(200))
            .expect("the planned interval is positive");
        assert_eq!(
            session.state(),
            FocusSessionState::Planned {
                planned_start: micros(100),
                planned_end: micros(200),
            }
        );

        session
            .start(micros(120))
            .expect("a planned session can start");
        assert_eq!(
            session.state(),
            FocusSessionState::Running {
                started_at: micros(120),
                deadline: micros(170),
            }
        );
    }

    #[test]
    fn starts_ready_session_without_a_plan() {
        let mut session = session();

        session
            .start(micros(100))
            .expect("a ready session can start");

        assert!(session.is_active_or_paused());
        assert_eq!(
            session.state(),
            FocusSessionState::Running {
                started_at: micros(100),
                deadline: micros(150),
            }
        );
    }

    #[test]
    fn rejects_invalid_planning_and_lifecycle_transitions() {
        let mut session = session();

        assert_eq!(
            session.plan(micros(20), micros(20)),
            Err(FocusSessionError::PlannedEndNotAfterStart)
        );
        assert_eq!(
            session.pause(micros(10)),
            Err(FocusSessionError::InvalidTransition {
                operation: "pause",
                state: FocusSessionState::Ready,
            })
        );

        session
            .start(micros(10))
            .expect("a ready session can start");
        assert_eq!(
            session.start(micros(11)),
            Err(FocusSessionError::InvalidTransition {
                operation: "start",
                state: FocusSessionState::Running {
                    started_at: micros(10),
                    deadline: micros(60),
                },
            })
        );
    }

    #[test]
    fn pauses_and_resumes_without_counting_paused_time() {
        let mut session = session();
        session
            .start(micros(100))
            .expect("a ready session can start");

        session
            .pause(micros(110))
            .expect("a running session pauses");
        assert_eq!(
            session.state(),
            FocusSessionState::Paused {
                started_at: micros(100),
                remaining_us: 40,
            }
        );

        session
            .resume(micros(200))
            .expect("a paused session resumes");
        assert_eq!(
            session.state(),
            FocusSessionState::Running {
                started_at: micros(100),
                deadline: micros(240),
            }
        );
    }

    #[test]
    fn completes_running_and_paused_sessions_early() {
        let mut running = session();
        running
            .start(micros(100))
            .expect("a ready session can start");
        running
            .complete(micros(110))
            .expect("a running session completes");
        assert_eq!(
            running.state(),
            FocusSessionState::Completed {
                started_at: micros(100),
                completed_at: micros(110),
            }
        );

        let mut paused = session();
        paused
            .start(micros(100))
            .expect("a ready session can start");
        paused.pause(micros(120)).expect("a running session pauses");
        paused
            .complete(micros(130))
            .expect("a paused session completes");
        assert_eq!(
            paused.state(),
            FocusSessionState::Completed {
                started_at: micros(100),
                completed_at: micros(130),
            }
        );
    }

    #[test]
    fn cancels_running_and_paused_sessions() {
        let mut running = session();
        running
            .start(micros(100))
            .expect("a ready session can start");
        running
            .cancel(micros(110))
            .expect("a running session cancels");
        assert_eq!(
            running.state(),
            FocusSessionState::Cancelled {
                started_at: micros(100),
                cancelled_at: micros(110),
            }
        );

        let mut paused = session();
        paused
            .start(micros(100))
            .expect("a ready session can start");
        paused.pause(micros(120)).expect("a running session pauses");
        paused
            .cancel(micros(130))
            .expect("a paused session cancels");
        assert_eq!(
            paused.state(),
            FocusSessionState::Cancelled {
                started_at: micros(100),
                cancelled_at: micros(130),
            }
        );
    }

    #[test]
    fn pausing_after_the_deadline_completes_at_the_deadline() {
        let mut session = session();
        session
            .start(micros(100))
            .expect("a ready session can start");

        session
            .pause(micros(150))
            .expect("a running session can reach its deadline");

        assert_eq!(
            session.state(),
            FocusSessionState::Completed {
                started_at: micros(100),
                completed_at: micros(150),
            }
        );
        assert!(!session.is_active_or_paused());
    }

    #[test]
    fn restart_before_deadline_keeps_a_running_session() {
        let mut session = session();
        session
            .start(micros(100))
            .expect("a ready session can start");

        let state = session.reconcile_after_restart(micros(149));

        assert_eq!(
            state,
            FocusSessionState::Running {
                started_at: micros(100),
                deadline: micros(150),
            }
        );
    }

    #[test]
    fn restart_after_deadline_completes_at_the_deadline() {
        let mut session = session();
        session
            .start(micros(100))
            .expect("a ready session can start");

        let state = session.reconcile_after_restart(micros(151));

        assert_eq!(
            state,
            FocusSessionState::Completed {
                started_at: micros(100),
                completed_at: micros(150),
            }
        );
    }

    #[test]
    fn restart_leaves_a_paused_session_and_its_remaining_time_unchanged() {
        let mut session = session();
        session
            .start(micros(100))
            .expect("a ready session can start");
        session
            .pause(micros(120))
            .expect("a running session pauses");

        let state = session.reconcile_after_restart(micros(1_000));

        assert_eq!(
            state,
            FocusSessionState::Paused {
                started_at: micros(100),
                remaining_us: 30,
            }
        );
    }

    #[test]
    fn restored_state_requires_the_same_invariants_as_live_state() {
        assert_eq!(
            FocusSession::try_restore(
                50,
                None,
                FocusSessionState::Paused {
                    started_at: micros(10),
                    remaining_us: 0,
                }
            ),
            Err(FocusSessionError::InvalidRestoredState)
        );
        assert_eq!(
            FocusSession::try_restore(
                50,
                None,
                FocusSessionState::Running {
                    started_at: micros(10),
                    deadline: micros(9),
                }
            ),
            Err(FocusSessionError::InvalidRestoredState)
        );
    }
}
