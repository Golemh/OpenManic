//! Bounded work lanes with explicit priority and overflow behavior.

use core::fmt;

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError, bounded};

/// Identifies the delivery policy assigned to a unit of runtime work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkLane {
    /// Authoritative work that must stay with the caller when capacity is full.
    Critical,
    /// Important work that is retained for the caller to retry when capacity is full.
    Normal,
    /// Replaceable work that may be discarded when its bounded lane is full.
    Optional,
}

/// Fixed capacities for the three independent runtime lanes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LaneCapacities {
    critical: usize,
    normal: usize,
    optional: usize,
}

impl LaneCapacities {
    /// Validates fixed, non-zero capacities for every runtime lane.
    ///
    /// # Errors
    ///
    /// Returns [`LaneConfigurationError`] when any lane has zero capacity.
    pub const fn try_new(
        critical: usize,
        normal: usize,
        optional: usize,
    ) -> Result<Self, LaneConfigurationError> {
        if critical == 0 {
            return Err(LaneConfigurationError::ZeroCapacity {
                lane: WorkLane::Critical,
            });
        }
        if normal == 0 {
            return Err(LaneConfigurationError::ZeroCapacity {
                lane: WorkLane::Normal,
            });
        }
        if optional == 0 {
            return Err(LaneConfigurationError::ZeroCapacity {
                lane: WorkLane::Optional,
            });
        }

        Ok(Self {
            critical,
            normal,
            optional,
        })
    }

    /// Returns the configured capacity for one lane.
    #[must_use]
    pub const fn capacity(self, lane: WorkLane) -> usize {
        match lane {
            WorkLane::Critical => self.critical,
            WorkLane::Normal => self.normal,
            WorkLane::Optional => self.optional,
        }
    }
}

/// Rejects an invalid bounded-lane configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaneConfigurationError {
    /// A lane would be unable to buffer any queued work.
    ZeroCapacity {
        /// The invalid lane.
        lane: WorkLane,
    },
}

impl fmt::Display for LaneConfigurationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroCapacity { lane } => {
                write!(formatter, "{lane:?} lane capacity must be non-zero")
            }
        }
    }
}

impl std::error::Error for LaneConfigurationError {}

/// Describes why a caller retained work instead of handing it to a lane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaneRetentionReason {
    /// The bounded lane already holds its configured maximum.
    Full,
    /// Every receiver for the lane has been dropped.
    Closed,
}

/// Reports the explicit result of a non-blocking lane submission.
#[derive(Debug, Eq, PartialEq)]
pub enum LaneSubmit<T> {
    /// The lane accepted the work.
    Enqueued,
    /// The caller still owns work that the runtime could not accept.
    Retained {
        /// The lane that rejected the work.
        lane: WorkLane,
        /// The reason this work remains with the caller.
        reason: LaneRetentionReason,
        /// The unchanged work item for retry, durable handling, or reporting.
        work: T,
    },
    /// Optional work was deliberately dropped because the bounded lane was full.
    Dropped {
        /// The optional lane that was at capacity.
        lane: WorkLane,
    },
}

/// Receives work from three bounded, independently configured lanes.
#[derive(Clone, Debug)]
pub struct RuntimeLanes<T> {
    critical: Sender<T>,
    normal: Sender<T>,
    optional: Sender<T>,
}

/// Receives work with critical, normal, then optional priority.
#[derive(Debug)]
pub struct RuntimeLaneReceiver<T> {
    critical: Receiver<T>,
    normal: Receiver<T>,
    optional: Receiver<T>,
}

/// Returns one sender facade and one receiver facade for bounded runtime work.
///
/// Critical and normal work are returned unchanged to the caller when their
/// lane is full. Optional work is dropped only when its own lane is full. A
/// full optional lane therefore cannot evict or overwrite critical work.
#[must_use]
pub fn bounded_runtime_lanes<T>(
    capacities: LaneCapacities,
) -> (RuntimeLanes<T>, RuntimeLaneReceiver<T>) {
    let (critical_sender, critical_receiver) = bounded(capacities.critical);
    let (normal_sender, normal_receiver) = bounded(capacities.normal);
    let (optional_sender, optional_receiver) = bounded(capacities.optional);

    (
        RuntimeLanes {
            critical: critical_sender,
            normal: normal_sender,
            optional: optional_sender,
        },
        RuntimeLaneReceiver {
            critical: critical_receiver,
            normal: normal_receiver,
            optional: optional_receiver,
        },
    )
}

impl<T> RuntimeLanes<T> {
    /// Attempts to enqueue work without blocking the caller.
    #[must_use]
    pub fn try_submit(&self, lane: WorkLane, work: T) -> LaneSubmit<T> {
        let sender = match lane {
            WorkLane::Critical => &self.critical,
            WorkLane::Normal => &self.normal,
            WorkLane::Optional => &self.optional,
        };

        match sender.try_send(work) {
            Ok(()) => LaneSubmit::Enqueued,
            Err(TrySendError::Full(_work)) if lane == WorkLane::Optional => {
                LaneSubmit::Dropped { lane }
            }
            Err(TrySendError::Full(work)) => LaneSubmit::Retained {
                lane,
                reason: LaneRetentionReason::Full,
                work,
            },
            Err(TrySendError::Disconnected(work)) => LaneSubmit::Retained {
                lane,
                reason: LaneRetentionReason::Closed,
                work,
            },
        }
    }
}

/// Reports one non-blocking receive attempt across all runtime lanes.
#[derive(Debug, Eq, PartialEq)]
pub enum LaneReceive<T> {
    /// One queued item, paired with the lane that determined its priority.
    Work {
        /// The lane from which the item was removed.
        lane: WorkLane,
        /// The queued work item.
        work: T,
    },
    /// At least one lane could still receive future work, but none is ready.
    Empty,
    /// Every lane is disconnected and all queued work has been drained.
    Closed,
}

impl<T> RuntimeLaneReceiver<T> {
    /// Removes one item without blocking, favoring critical then normal work.
    #[must_use]
    pub fn try_receive(&self) -> LaneReceive<T> {
        let critical_closed = match self.critical.try_recv() {
            Ok(work) => {
                return LaneReceive::Work {
                    lane: WorkLane::Critical,
                    work,
                };
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => true,
        };
        let normal_closed = match self.normal.try_recv() {
            Ok(work) => {
                return LaneReceive::Work {
                    lane: WorkLane::Normal,
                    work,
                };
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => true,
        };
        let optional_closed = match self.optional.try_recv() {
            Ok(work) => {
                return LaneReceive::Work {
                    lane: WorkLane::Optional,
                    work,
                };
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => true,
        };

        if critical_closed && normal_closed && optional_closed {
            LaneReceive::Closed
        } else {
            LaneReceive::Empty
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LaneCapacities, LaneConfigurationError, LaneReceive, LaneRetentionReason, LaneSubmit,
        WorkLane, bounded_runtime_lanes,
    };

    #[test]
    fn lane_capacities_reject_zero_for_each_lane() {
        assert_eq!(
            LaneCapacities::try_new(0, 1, 1),
            Err(LaneConfigurationError::ZeroCapacity {
                lane: WorkLane::Critical
            })
        );
        assert_eq!(
            LaneCapacities::try_new(1, 0, 1),
            Err(LaneConfigurationError::ZeroCapacity {
                lane: WorkLane::Normal
            })
        );
        assert_eq!(
            LaneCapacities::try_new(1, 1, 0),
            Err(LaneConfigurationError::ZeroCapacity {
                lane: WorkLane::Optional
            })
        );
    }

    #[test]
    fn critical_full_retains_authoritative_work_for_the_caller() {
        let capacities = LaneCapacities::try_new(1, 1, 1)
            .expect("non-zero capacities construct a bounded runtime");
        let (lanes, receiver) = bounded_runtime_lanes(capacities);

        assert_eq!(
            lanes.try_submit(WorkLane::Critical, 10),
            LaneSubmit::Enqueued
        );
        assert_eq!(
            lanes.try_submit(WorkLane::Critical, 11),
            LaneSubmit::Retained {
                lane: WorkLane::Critical,
                reason: LaneRetentionReason::Full,
                work: 11,
            }
        );
        assert_eq!(
            receiver.try_receive(),
            LaneReceive::Work {
                lane: WorkLane::Critical,
                work: 10,
            }
        );
    }

    #[test]
    fn optional_full_is_explicitly_dropped_without_displacing_critical_work() {
        let capacities = LaneCapacities::try_new(1, 1, 1)
            .expect("non-zero capacities construct a bounded runtime");
        let (lanes, receiver) = bounded_runtime_lanes(capacities);

        assert_eq!(
            lanes.try_submit(WorkLane::Optional, 1),
            LaneSubmit::Enqueued
        );
        assert_eq!(
            lanes.try_submit(WorkLane::Optional, 2),
            LaneSubmit::Dropped {
                lane: WorkLane::Optional
            }
        );
        assert_eq!(
            lanes.try_submit(WorkLane::Critical, 3),
            LaneSubmit::Enqueued
        );
        assert_eq!(
            receiver.try_receive(),
            LaneReceive::Work {
                lane: WorkLane::Critical,
                work: 3,
            }
        );
        assert_eq!(
            receiver.try_receive(),
            LaneReceive::Work {
                lane: WorkLane::Optional,
                work: 1,
            }
        );
    }

    #[test]
    fn receiver_prioritizes_critical_then_normal_then_optional_work() {
        let capacities = LaneCapacities::try_new(2, 2, 2)
            .expect("non-zero capacities construct a bounded runtime");
        let (lanes, receiver) = bounded_runtime_lanes(capacities);

        assert_eq!(
            lanes.try_submit(WorkLane::Optional, 1),
            LaneSubmit::Enqueued
        );
        assert_eq!(lanes.try_submit(WorkLane::Normal, 2), LaneSubmit::Enqueued);
        assert_eq!(
            lanes.try_submit(WorkLane::Critical, 3),
            LaneSubmit::Enqueued
        );

        assert_eq!(
            receiver.try_receive(),
            LaneReceive::Work {
                lane: WorkLane::Critical,
                work: 3,
            }
        );
        assert_eq!(
            receiver.try_receive(),
            LaneReceive::Work {
                lane: WorkLane::Normal,
                work: 2,
            }
        );
        assert_eq!(
            receiver.try_receive(),
            LaneReceive::Work {
                lane: WorkLane::Optional,
                work: 1,
            }
        );
    }

    #[test]
    fn disconnected_and_drained_lanes_report_closed() {
        let capacities = LaneCapacities::try_new(1, 1, 1)
            .expect("non-zero capacities construct a bounded runtime");
        let (lanes, receiver) = bounded_runtime_lanes::<u8>(capacities);

        drop(lanes);

        assert_eq!(receiver.try_receive(), LaneReceive::Closed);
    }
}
