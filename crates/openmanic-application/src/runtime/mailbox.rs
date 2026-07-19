//! One-slot latest-value mailbox for replaceable runtime updates.

use std::sync::{Arc, Mutex, MutexGuard};

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError, bounded};

/// Publishes replaceable values into a one-slot latest-value mailbox.
#[derive(Clone, Debug)]
pub struct LatestMailbox<T> {
    state: Arc<Mutex<MailboxState<T>>>,
    notification: Sender<()>,
}

/// Receives coalesced values from a one-slot latest-value mailbox.
#[derive(Debug)]
pub struct LatestMailboxReceiver<T> {
    state: Arc<Mutex<MailboxState<T>>>,
    notification: Receiver<()>,
}

#[derive(Debug)]
struct MailboxState<T> {
    value: Option<T>,
    notified: bool,
}

/// Reports the explicit result of publishing a replaceable mailbox value.
#[derive(Debug, Eq, PartialEq)]
pub enum MailboxPublish<T> {
    /// The mailbox accepted a value that was not replacing a pending value.
    Published,
    /// The mailbox replaced an older pending value with the newer value.
    Coalesced,
    /// The receiver is closed, so the caller retains the unpublished value.
    Closed(T),
}

/// Reports a non-blocking mailbox receive attempt.
#[derive(Debug, Eq, PartialEq)]
pub enum MailboxReceive<T> {
    /// The newest value available when the mailbox was observed.
    Latest(T),
    /// The mailbox remains open but has no ready update.
    Empty,
    /// Every publisher has been dropped and no value remains.
    Closed,
}

/// Creates a fixed single-slot mailbox that always favors the latest value.
///
/// A published notification is also bounded to one item. Publishers replace a
/// pending value rather than queuing additional notifications, which prevents
/// progress or refresh updates from growing without bound.
#[must_use]
pub fn latest_mailbox<T>() -> (LatestMailbox<T>, LatestMailboxReceiver<T>) {
    let state = Arc::new(Mutex::new(MailboxState {
        value: None,
        notified: false,
    }));
    let (notification_sender, notification_receiver) = bounded(1);

    (
        LatestMailbox {
            state: Arc::clone(&state),
            notification: notification_sender,
        },
        LatestMailboxReceiver {
            state,
            notification: notification_receiver,
        },
    )
}

impl<T> LatestMailbox<T> {
    /// Publishes a value without blocking, replacing a pending older value.
    #[must_use]
    pub fn publish(&self, value: T) -> MailboxPublish<T> {
        let mut state = recover_lock(&self.state);
        let replaced_pending_value = state.value.replace(value).is_some();

        if state.notified {
            return MailboxPublish::Coalesced;
        }

        state.notified = true;
        match self.notification.try_send(()) {
            Ok(()) => {
                if replaced_pending_value {
                    MailboxPublish::Coalesced
                } else {
                    MailboxPublish::Published
                }
            }
            Err(TrySendError::Full(())) => MailboxPublish::Coalesced,
            Err(TrySendError::Disconnected(())) => {
                state.notified = false;
                match state.value.take() {
                    Some(value) => MailboxPublish::Closed(value),
                    None => MailboxPublish::Coalesced,
                }
            }
        }
    }
}

impl<T> LatestMailboxReceiver<T> {
    /// Removes the latest available value without blocking.
    #[must_use]
    pub fn try_receive(&self) -> MailboxReceive<T> {
        match self.notification.try_recv() {
            Ok(()) => {
                let mut state = recover_lock(&self.state);
                state.notified = false;
                match state.value.take() {
                    Some(value) => MailboxReceive::Latest(value),
                    None => MailboxReceive::Empty,
                }
            }
            Err(TryRecvError::Empty) => MailboxReceive::Empty,
            Err(TryRecvError::Disconnected) => MailboxReceive::Closed,
        }
    }
}

fn recover_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::{MailboxPublish, MailboxReceive, latest_mailbox};

    #[test]
    fn mailbox_coalesces_pending_values_to_the_latest_one() {
        let (mailbox, receiver) = latest_mailbox();

        assert_eq!(mailbox.publish("first"), MailboxPublish::Published);
        assert_eq!(mailbox.publish("latest"), MailboxPublish::Coalesced);
        assert_eq!(receiver.try_receive(), MailboxReceive::Latest("latest"));
        assert_eq!(receiver.try_receive(), MailboxReceive::Empty);
    }

    #[test]
    fn closed_mailbox_returns_the_value_to_the_publisher() {
        let (mailbox, receiver) = latest_mailbox();
        drop(receiver);

        assert_eq!(mailbox.publish(42), MailboxPublish::Closed(42));
    }

    #[test]
    fn mailbox_reports_closed_after_publishers_are_dropped() {
        let (mailbox, receiver) = latest_mailbox::<u8>();
        drop(mailbox);

        assert_eq!(receiver.try_receive(), MailboxReceive::Closed);
    }
}
