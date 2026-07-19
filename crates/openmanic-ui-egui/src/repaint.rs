//! Repaint requests that wake rendering without advancing product state.

use eframe::egui::Context;

/// A reason to request another egui frame after state has already changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RepaintReason {
    /// A normal shell control changed UI-owned state.
    UserInput,
    /// A bounded ingress drain accepted already-delivered work.
    InboundWork,
}

/// Coalesces wake-up requests without using frame count or elapsed repaint time as state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RepaintScheduler {
    requested: bool,
}

impl RepaintScheduler {
    /// Creates a scheduler with no pending repaint request.
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self { requested: false }
    }

    /// Records that the renderer should be woken after the current frame.
    pub(crate) fn request(&mut self, _reason: RepaintReason) {
        self.requested = true;
    }

    /// Requests one egui repaint when work changed; it never mutates the UI model.
    pub(crate) fn request_if_needed(&mut self, context: &Context) {
        if self.requested {
            self.requested = false;
            context.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RepaintReason, RepaintScheduler};

    #[test]
    fn repaint_requests_are_coalesced_without_a_frame_counter() {
        let mut scheduler = RepaintScheduler::default();
        scheduler.request(RepaintReason::UserInput);
        scheduler.request(RepaintReason::InboundWork);
        assert!(scheduler.requested);
    }
}
