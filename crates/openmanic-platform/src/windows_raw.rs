//! Private raw Windows foreground-event ingress.
//!
//! This module deliberately contains the native window representation and the bounded callback
//! queue.  It is not re-exported: later Windows identity work can resolve a raw observation inside
//! the platform crate without leaking an `HWND`, PID, or title through the application boundary.

use std::{
    collections::VecDeque,
    sync::{
        Mutex, TryLockError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use openmanic_application::UtcMicros;

/// Opaque native window value retained entirely inside the platform crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RawWindowHandle(isize);

impl RawWindowHandle {
    /// Creates a raw handle from the platform ABI value.
    #[must_use]
    pub(crate) const fn new(value: isize) -> Self {
        Self(value)
    }

    /// Returns whether this is the null-window sentinel supplied by Windows.
    #[must_use]
    pub(crate) const fn is_null(self) -> bool {
        self.0 == 0
    }

    #[cfg(windows)]
    #[must_use]
    pub(crate) const fn value(self) -> isize {
        self.0
    }
}

/// The callback-sized subset of a foreground notification.
///
/// The event tick is useful only for source ordering and latency diagnostics.  The receive UTC
/// sample is the observation timestamp; neither value is treated as an application identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RawForegroundEvent {
    source_order: u64,
    window: RawWindowHandle,
    source_event_tick: u32,
    received_at_utc: UtcMicros,
}

impl RawForegroundEvent {
    const fn new(
        source_order: u64,
        window: RawWindowHandle,
        source_event_tick: u32,
        received_at_utc: UtcMicros,
    ) -> Self {
        Self {
            source_order,
            window,
            source_event_tick,
            received_at_utc,
        }
    }

    #[must_use]
    pub(crate) const fn source_order(self) -> u64 {
        self.source_order
    }

    #[must_use]
    pub(crate) const fn window(self) -> RawWindowHandle {
        self.window
    }

    #[must_use]
    pub(crate) const fn source_event_tick(self) -> u32 {
        self.source_event_tick
    }

    #[must_use]
    pub(crate) const fn received_at_utc(self) -> UtcMicros {
        self.received_at_utc
    }
}

/// Why the control loop could not take the callback queue at this instant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RawIngressDrainError {
    /// A reentrant callback owns the ingress for its bounded critical section.
    Busy,
    /// A prior panic poisoned the ingress lock, so evidence must be treated as lost.
    Poisoned,
}

/// Preallocated, nonblocking callback ingress for raw foreground notifications.
///
/// The `WinEvent` callback uses only [`Self::try_enqueue_foreground`].  It never waits for the
/// control loop, the tracking service, or storage.  A full or contended queue records loss and
/// leaves already queued observations intact for normal-loop recovery.
#[derive(Debug)]
pub(crate) struct RawEventIngress {
    capacity: usize,
    events: Mutex<VecDeque<RawForegroundEvent>>,
    next_source_order: AtomicU64,
    overflowed: AtomicBool,
}

impl RawEventIngress {
    /// Creates an ingress with all callback storage allocated before hook installation.
    #[must_use]
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            events: Mutex::new(VecDeque::with_capacity(capacity)),
            // Source order one belongs to the adapter's startup availability observation.
            next_source_order: AtomicU64::new(2),
            overflowed: AtomicBool::new(false),
        }
    }

    /// Records one callback observation without waiting or allocating.
    ///
    /// Returns `false` after recording explicit overflow when the queue cannot retain the event.
    pub(crate) fn try_enqueue_foreground(
        &self,
        window: RawWindowHandle,
        source_event_tick: u32,
        received_at_utc: UtcMicros,
    ) -> bool {
        let Some(event) = self.make_foreground_event(window, source_event_tick, received_at_utc)
        else {
            self.mark_overflow();
            return false;
        };

        match self.events.try_lock() {
            Ok(mut events) if events.len() < self.capacity => {
                events.push_back(event);
                true
            }
            Ok(_) | Err(TryLockError::WouldBlock | TryLockError::Poisoned(_)) => {
                self.mark_overflow();
                false
            }
        }
    }

    /// Allocates an ordering value for a normal-loop reconciliation observation.
    #[must_use]
    pub(crate) fn make_foreground_event(
        &self,
        window: RawWindowHandle,
        source_event_tick: u32,
        received_at_utc: UtcMicros,
    ) -> Option<RawForegroundEvent> {
        self.allocate_source_order().map(|source_order| {
            RawForegroundEvent::new(source_order, window, source_event_tick, received_at_utc)
        })
    }

    /// Takes all currently retained raw events without blocking a callback.
    pub(crate) fn try_drain(&self) -> Result<VecDeque<RawForegroundEvent>, RawIngressDrainError> {
        match self.events.try_lock() {
            Ok(mut events) => Ok(std::mem::take(&mut *events)),
            Err(TryLockError::WouldBlock) => Err(RawIngressDrainError::Busy),
            Err(TryLockError::Poisoned(_)) => Err(RawIngressDrainError::Poisoned),
        }
    }

    /// Returns and clears the loss indication accumulated by callbacks.
    #[must_use]
    pub(crate) fn take_overflow(&self) -> bool {
        self.overflowed.swap(false, Ordering::AcqRel)
    }

    /// Marks evidence loss without trying to acquire the callback queue.
    pub(crate) fn mark_overflow(&self) {
        self.overflowed.store(true, Ordering::Release);
    }

    fn allocate_source_order(&self) -> Option<u64> {
        self.next_source_order
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                if current == 0 {
                    None
                } else if current == u64::MAX {
                    Some(0)
                } else {
                    Some(current + 1)
                }
            })
            .ok()
    }
}

/// Samples a UTC receive time without assigning meaning to the `WinEvent` tick value.
#[must_use]
pub(crate) fn receive_time_utc() -> UtcMicros {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => UtcMicros::new(duration_to_i64_micros(duration)),
        Err(error) => UtcMicros::new(duration_to_i64_micros(error.duration()).saturating_neg()),
    }
}

fn duration_to_i64_micros(duration: std::time::Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{RawEventIngress, RawWindowHandle};
    use openmanic_application::UtcMicros;

    #[test]
    fn ingress_preserves_queued_callback_order_without_allocation() {
        let ingress = RawEventIngress::new(2);

        assert!(ingress.try_enqueue_foreground(
            RawWindowHandle::new(10),
            100,
            UtcMicros::new(1_000),
        ));
        assert!(ingress.try_enqueue_foreground(
            RawWindowHandle::new(11),
            101,
            UtcMicros::new(1_001),
        ));

        let drained = ingress.try_drain();
        assert!(drained.is_ok());
        let Some(events) = drained.ok() else {
            return;
        };
        let collected: Vec<_> = events.into_iter().collect();

        assert_eq!(collected[0].source_order(), 2);
        assert_eq!(collected[0].window(), RawWindowHandle::new(10));
        assert_eq!(collected[0].source_event_tick(), 100);
        assert_eq!(collected[0].received_at_utc(), UtcMicros::new(1_000));
        assert_eq!(collected[1].source_order(), 3);
        assert_eq!(collected[1].window(), RawWindowHandle::new(11));
    }

    #[test]
    fn full_ingress_preserves_retained_event_and_marks_explicit_loss() {
        let ingress = RawEventIngress::new(1);

        assert!(ingress.try_enqueue_foreground(
            RawWindowHandle::new(10),
            100,
            UtcMicros::new(1_000),
        ));
        assert!(!ingress.try_enqueue_foreground(
            RawWindowHandle::new(11),
            101,
            UtcMicros::new(1_001),
        ));

        let drained = ingress.try_drain();
        assert!(drained.is_ok());
        let Some(events) = drained.ok() else {
            return;
        };
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].window(), RawWindowHandle::new(10));
        assert!(ingress.take_overflow());
        assert!(!ingress.take_overflow());
    }
}
