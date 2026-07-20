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
    theme: crate::ThemeController,
    theme_key: String,
    theme_applied: bool,
}

impl<P, T> OpenManicApp<P, T> {
    /// Creates the eframe application from a bounded controller.
    #[must_use]
    pub fn new(controller: UiController<P, T>) -> Self {
        Self::new_with_theme(controller, "openmanic.dark")
    }

    /// Creates the eframe application with a persisted built-in theme key.
    #[must_use]
    pub fn new_with_theme(controller: UiController<P, T>, theme_key: impl Into<String>) -> Self {
        Self {
            controller,
            repaint: RepaintScheduler::new(),
            theme: crate::ThemeController::default(),
            theme_key: theme_key.into(),
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

    /// Applies one validated built-in theme at the next foreground-safe call site.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ThemeResolutionError`] without changing the active theme when the key is invalid.
    pub fn apply_theme(
        &mut self,
        context: &eframe::egui::Context,
        key: &str,
        system_prefers_dark: bool,
    ) -> Result<(), crate::ThemeResolutionError> {
        self.theme.apply_key(context, key, system_prefers_dark)?;
        key.clone_into(&mut self.theme_key);
        Ok(())
    }

    /// Returns the semantic tokens consumed by custom OpenManic rendering.
    #[must_use]
    pub const fn theme_tokens(&self) -> crate::ThemeTokens {
        self.theme.current().tokens()
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
            if self
                .theme
                .apply_key(&context, &self.theme_key, true)
                .is_err()
            {
                self.theme.apply_current(&context);
            }
            self.theme_applied = true;
        }

        let inbound = self.controller.drain_inbound(DEFAULT_INBOUND_DRAIN_LIMIT);
        if inbound.processed > 0 {
            self.repaint.request(RepaintReason::InboundWork);
        }
        let theme_tokens = self.theme_tokens();
        if shell::render(ui, self.controller.model_mut(), theme_tokens) {
            self.repaint.request(RepaintReason::UserInput);
        }
        self.repaint.request_if_needed(&context);
    }
}
