//! Idempotent cancellation signals for safe background-work boundaries.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Owns the cancellation request for one unit of cancellable work.
#[derive(Debug)]
pub struct CancellationSource {
    requested: Arc<AtomicBool>,
}

/// Read-only cancellation view shared with the running work.
#[derive(Clone, Debug)]
pub struct CancellationToken {
    requested: Arc<AtomicBool>,
}

/// Reports whether a cancellation request changed the runtime state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancellationRequest {
    /// This call recorded the first cancellation request.
    Requested,
    /// Cancellation was already requested by an earlier call.
    AlreadyRequested,
}

impl CancellationSource {
    /// Creates a cancellation owner and a token for the corresponding work.
    #[must_use]
    pub fn new() -> (Self, CancellationToken) {
        let requested = Arc::new(AtomicBool::new(false));
        (
            Self {
                requested: Arc::clone(&requested),
            },
            CancellationToken { requested },
        )
    }

    /// Requests cancellation without blocking and reports whether it was new.
    #[must_use]
    pub fn cancel(&self) -> CancellationRequest {
        if self.requested.swap(true, Ordering::AcqRel) {
            CancellationRequest::AlreadyRequested
        } else {
            CancellationRequest::Requested
        }
    }
}

impl CancellationToken {
    /// Returns whether a cancellation request has been observed.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::{CancellationRequest, CancellationSource};

    #[test]
    fn cancellation_is_idempotent_and_visible_to_all_tokens() {
        let (source, token) = CancellationSource::new();
        let second_token = token.clone();

        assert!(!token.is_cancelled());
        assert_eq!(source.cancel(), CancellationRequest::Requested);
        assert!(token.is_cancelled());
        assert!(second_token.is_cancelled());
        assert_eq!(source.cancel(), CancellationRequest::AlreadyRequested);
    }
}
