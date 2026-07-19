//! eframe lifecycle wrapper around the bounded UI controller.

use crate::{
    UiController,
    controller::DEFAULT_INBOUND_DRAIN_LIMIT,
    repaint::{RepaintReason, RepaintScheduler},
    shell,
};

/// The eframe application hosting the initial OpenManic shell.
///
/// The app drains only a bounded set of pre-delivered inbound messages and
/// renders immutable state. It deliberately does not invoke a runtime lane
/// itself: OM-200 will connect [`UiController`] to named bounded workers.
pub struct OpenManicApp<P, T> {
    controller: UiController<P, T>,
    repaint: RepaintScheduler,
    theme_applied: bool,
}

impl<P, T> OpenManicApp<P, T> {
    /// Creates the eframe application from a bounded controller.
    #[must_use]
    pub const fn new(controller: UiController<P, T>) -> Self {
        Self {
            controller,
            repaint: RepaintScheduler::new(),
            theme_applied: false,
        }
    }

    /// Returns the bounded controller for composition-root runtime wiring.
    #[must_use]
    pub const fn controller(&self) -> &UiController<P, T> {
        &self.controller
    }

    /// Returns mutable bounded controller state before or between native frames.
    pub fn controller_mut(&mut self) -> &mut UiController<P, T> {
        &mut self.controller
    }
}

impl<P, T> eframe::App for OpenManicApp<P, T>
where
    T: Send + Sync + 'static,
    P: 'static,
{
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        if !self.theme_applied {
            shell::apply_initial_dark_theme(&context);
            self.theme_applied = true;
        }

        let inbound = self.controller.drain_inbound(DEFAULT_INBOUND_DRAIN_LIMIT);
        if inbound.processed > 0 {
            self.repaint.request(RepaintReason::InboundWork);
        }
        if shell::render(ui, self.controller.model_mut()) {
            self.repaint.request(RepaintReason::UserInput);
        }
        self.repaint.request_if_needed(&context);
    }
}
