//! Bounded UI ingress and deferred command dispatching.
//!
//! The eframe update path drains only already-delivered messages. It never
//! waits for a worker. OM-200 owns the runtime lanes that will implement the
//! [`CommandDispatcher`] seam; this module deliberately owns no threads,
//! channels, storage, or platform adapters.

use std::collections::VecDeque;

use openmanic_application::{
    AppEvent, ApplicationError, CommandEnvelope, CommandPort, CommandReceipt, EventEnvelope,
    SnapshotEnvelope,
};

use crate::model::{EventReception, SnapshotReception, UiModel};

/// The normal maximum number of inbound messages handled by one eframe update.
pub(crate) const DEFAULT_INBOUND_DRAIN_LIMIT: usize = 32;

/// A message delivered to the UI from application-owned background work.
#[derive(Debug)]
pub enum InboundMessage<T> {
    /// A critical application event such as an authoritative mutation result.
    Event(EventEnvelope<AppEvent>),
    /// A fully prepared immutable presentation snapshot.
    Snapshot(SnapshotEnvelope<T>),
}

/// Rejects a controller configuration with an unusable bounded queue capacity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueCapacityError {
    /// A zero capacity cannot provide an explicit bounded delivery policy.
    ZeroCapacity,
}

/// Reports an attempted nonblocking queue operation that could not fit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueOverflow {
    /// The fixed-capacity inbound queue could not retain another message.
    InboundFull,
    /// The fixed-capacity outbound queue could not retain another command.
    OutboundFull,
}

/// Summarizes the bounded work performed during one eframe update.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InboundDrain {
    /// The number of messages removed from the bounded ingress queue.
    pub processed: usize,
    /// The number of application events that updated UI-owned state.
    pub applied_events: usize,
    /// The number of snapshots installed as current immutable values.
    pub accepted_snapshots: usize,
}

/// Summarizes a nonblocking attempt to hand queued commands to the runtime seam.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DispatchDrain {
    /// The number of commands the dispatcher accepted.
    pub dispatched: usize,
    /// The first typed application failure, if dispatch could not continue.
    pub stopped_by: Option<ApplicationError>,
}

/// A nonblocking bridge from UI-owned command envelopes to application handling.
///
/// OM-200 will connect this to a bounded runtime lane. The current blanket
/// implementation permits an existing application [`CommandPort`] to serve as
/// the bridge without exposing its adapter or queue details to the UI crate.
pub trait CommandDispatcher<P> {
    /// Attempts one immediate enqueue without waiting for background work.
    ///
    /// # Errors
    ///
    /// Returns the application port's typed backpressure or availability error.
    fn try_dispatch(
        &mut self,
        command: CommandEnvelope<P>,
    ) -> Result<CommandReceipt, ApplicationError>;
}

impl<P, T> CommandDispatcher<P> for T
where
    T: CommandPort<P>,
{
    fn try_dispatch(
        &mut self,
        command: CommandEnvelope<P>,
    ) -> Result<CommandReceipt, ApplicationError> {
        self.submit(command)
    }
}

/// Coordinates UI-owned state with bounded inbound and outbound work queues.
///
/// Commands become pending when accepted into the outbound UI queue. They are
/// only confirmed or rejected after a correlated application event arrives.
pub struct UiController<P, T> {
    model: UiModel<T>,
    inbound: VecDeque<InboundMessage<T>>,
    outbound: VecDeque<CommandEnvelope<P>>,
    inbound_capacity: usize,
    outbound_capacity: usize,
}

impl<P, T> UiController<P, T> {
    /// Creates a controller with explicit bounded ingress and egress capacities.
    ///
    /// # Errors
    ///
    /// Returns [`QueueCapacityError::ZeroCapacity`] when either queue capacity is zero.
    pub fn try_new(
        model: UiModel<T>,
        inbound_capacity: usize,
        outbound_capacity: usize,
    ) -> Result<Self, QueueCapacityError> {
        if inbound_capacity == 0 || outbound_capacity == 0 {
            return Err(QueueCapacityError::ZeroCapacity);
        }
        Ok(Self {
            model,
            inbound: VecDeque::with_capacity(inbound_capacity),
            outbound: VecDeque::with_capacity(outbound_capacity),
            inbound_capacity,
            outbound_capacity,
        })
    }

    /// Returns an immutable view of all UI-owned state.
    #[must_use]
    pub const fn model(&self) -> &UiModel<T> {
        &self.model
    }

    /// Returns mutable UI-owned state for shell rendering and pure reducers.
    pub(crate) fn model_mut(&mut self) -> &mut UiModel<T> {
        &mut self.model
    }

    /// Returns the number of messages waiting for a future bounded UI drain.
    #[must_use]
    pub fn inbound_len(&self) -> usize {
        self.inbound.len()
    }

    /// Offers an inbound message without waiting or allocating an unbounded backlog.
    ///
    /// # Errors
    ///
    /// Returns [`QueueOverflow::InboundFull`] when the explicit inbound capacity is reached.
    pub fn try_enqueue_inbound(&mut self, message: InboundMessage<T>) -> Result<(), QueueOverflow> {
        if self.inbound.len() == self.inbound_capacity {
            return Err(QueueOverflow::InboundFull);
        }
        self.inbound.push_back(message);
        Ok(())
    }

    /// Queues one command for a later nonblocking runtime-lane attempt.
    ///
    /// # Errors
    ///
    /// Returns [`QueueOverflow::OutboundFull`] when the explicit outbound capacity is reached.
    pub fn try_queue_command(&mut self, command: CommandEnvelope<P>) -> Result<(), QueueOverflow> {
        if self.outbound.len() == self.outbound_capacity {
            return Err(QueueOverflow::OutboundFull);
        }
        self.model.record_pending_mutation(command.command_id());
        self.outbound.push_back(command);
        Ok(())
    }

    /// Processes at most `limit` messages that were already available to the UI.
    pub fn drain_inbound(&mut self, limit: usize) -> InboundDrain {
        let mut drain = InboundDrain::default();
        while drain.processed < limit {
            let Some(message) = self.inbound.pop_front() else {
                break;
            };
            drain.processed += 1;
            drain.record(self.apply_inbound(message));
        }
        drain
    }

    /// Attempts to dispatch at most `limit` queued commands without waiting.
    ///
    /// A typed failure stops this drain immediately. OM-200's eventual lane
    /// adapter owns retry/backpressure policy; this shell never blocks to retry.
    pub fn drain_dispatcher<D>(&mut self, dispatcher: &mut D, limit: usize) -> DispatchDrain
    where
        D: CommandDispatcher<P>,
    {
        let mut drain = DispatchDrain::default();
        while drain.dispatched < limit {
            let Some(command) = self.outbound.pop_front() else {
                break;
            };
            match dispatcher.try_dispatch(command) {
                Ok(_) => drain.dispatched += 1,
                Err(error) => {
                    drain.stopped_by = Some(error);
                    break;
                }
            }
        }
        drain
    }

    fn apply_inbound(&mut self, message: InboundMessage<T>) -> InboundEffect {
        match message {
            InboundMessage::Event(event) => {
                InboundEffect::from_event(self.model.receive_event(event))
            }
            InboundMessage::Snapshot(snapshot) => {
                InboundEffect::from_snapshot(self.model.accept_snapshot(&snapshot))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct InboundEffect {
    applied_event: bool,
    accepted_snapshot: bool,
}

impl InboundEffect {
    const fn from_event(reception: EventReception) -> Self {
        Self {
            applied_event: matches!(reception, EventReception::Applied),
            accepted_snapshot: false,
        }
    }

    const fn from_snapshot(reception: SnapshotReception) -> Self {
        Self {
            applied_event: false,
            accepted_snapshot: matches!(reception, SnapshotReception::Accepted),
        }
    }
}

impl InboundDrain {
    fn record(&mut self, effect: InboundEffect) {
        self.applied_events += usize::from(effect.applied_event);
        self.accepted_snapshots += usize::from(effect.accepted_snapshot);
    }
}

#[cfg(test)]
mod tests {
    use openmanic_application::{
        AppEvent, CommandEnvelope, CommandId, DataRevision, EventEnvelope, MutationConfirmation,
        MutationOutcome, OrderingKey, SchemaRevision,
    };
    use openmanic_domain::UtcMicros;

    use super::{InboundMessage, QueueCapacityError, UiController};
    use crate::UiModel;

    #[test]
    fn inbound_drain_is_bounded_and_leaves_later_messages_for_the_next_frame() {
        let mut controller = UiController::<(), String>::try_new(UiModel::default(), 4, 2)
            .expect("fixture capacities are nonzero");
        for sequence in 1..=3 {
            controller
                .try_enqueue_inbound(InboundMessage::Event(confirmation_event(sequence)))
                .expect("fixture fits the bounded inbound queue");
        }

        let first = controller.drain_inbound(2);
        assert_eq!(first.processed, 2);
        assert_eq!(controller.inbound_len(), 1);
        let second = controller.drain_inbound(2);
        assert_eq!(second.processed, 1);
        assert_eq!(controller.inbound_len(), 0);
    }

    #[test]
    fn zero_capacity_is_rejected_before_constructing_queues() {
        assert!(matches!(
            UiController::<(), String>::try_new(UiModel::default(), 0, 1),
            Err(QueueCapacityError::ZeroCapacity)
        ));
    }

    #[test]
    fn accepted_outbound_command_is_marked_pending_before_runtime_dispatch() {
        let mut controller = UiController::<(), String>::try_new(UiModel::default(), 1, 1)
            .expect("fixture capacities are nonzero");
        let command_id = CommandId::new(77);
        controller
            .try_queue_command(CommandEnvelope::new(
                SchemaRevision::new(1),
                command_id,
                OrderingKey::new(1),
                None,
                UtcMicros::new(0),
                (),
            ))
            .expect("fixture fits the bounded outbound queue");
        assert_eq!(
            controller.model().mutation_status(command_id),
            Some(&crate::MutationStatus::Pending)
        );
    }

    fn confirmation_event(sequence: u64) -> EventEnvelope<AppEvent> {
        let command_id = CommandId::new(sequence);
        EventEnvelope::new(
            SchemaRevision::new(1),
            sequence,
            Some(command_id),
            Some(DataRevision::new(sequence)),
            UtcMicros::new(0),
            AppEvent::Mutation(MutationOutcome::Confirmed(MutationConfirmation::new(
                command_id,
                DataRevision::new(sequence),
            ))),
        )
    }
}
