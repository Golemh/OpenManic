//! Presentation-only recovery controls for a graceful-shutdown flush failure.
//!
//! The application layer owns the ordered shutdown state machine. This module only maps its
//! immutable state to ordinary-language UI content and returns typed intentions for the host to
//! apply to that state machine. It never performs a flush, closes the process, or accesses I/O.

use eframe::egui;
use openmanic_application::{ShutdownPhase, ShutdownStep};

/// A deliberate choice exposed when a critical shutdown write cannot finish.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShutdownAction {
    /// Ask the host to retry the failed critical operation.
    Retry,
    /// Ask the host to continue shutdown without claiming the failed operation succeeded.
    QuitAnyway,
}

/// Typed host work requested by a visible shutdown recovery control.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShutdownEffect {
    /// The host must call `ShutdownCoordinator::retry_critical_flush`.
    RetryCriticalFlush,
    /// The host must call `ShutdownCoordinator::quit_anyway`.
    QuitAnyway,
}

/// Immutable, privacy-safe content for the shutdown recovery dialog.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShutdownFailureViewModel {
    failed_step: ShutdownStep,
    heading: &'static str,
    detail: &'static str,
    warning: &'static str,
}

impl ShutdownFailureViewModel {
    /// Returns the lifecycle step that the host should retry or explicitly skip.
    #[must_use]
    pub const fn failed_step(self) -> ShutdownStep {
        self.failed_step
    }

    /// Returns the ordinary-language recovery heading.
    #[must_use]
    pub const fn heading(self) -> &'static str {
        self.heading
    }

    /// Returns the ordinary-language description of what could not be saved.
    #[must_use]
    pub const fn detail(self) -> &'static str {
        self.detail
    }

    /// Returns the explicit consequence of proceeding without a successful retry.
    #[must_use]
    pub const fn warning(self) -> &'static str {
        self.warning
    }
}

/// Stateless controller for shutdown recovery presentation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ShutdownController;

impl ShutdownController {
    /// Builds a dialog model only while an authoritative critical flush failure is pending.
    #[must_use]
    pub const fn failure_view_model(self, phase: ShutdownPhase) -> Option<ShutdownFailureViewModel> {
        let ShutdownPhase::FlushFailed { step } = phase else {
            return None;
        };
        Some(failure_view_model(step))
    }

    /// Converts a user choice into host work only while recovery is still valid.
    ///
    /// The host must apply the returned effect to the same authoritative shutdown coordinator,
    /// then provide its new phase on the next frame. Stale clicks are ignored rather than being
    /// interpreted as a successful shutdown transition.
    #[must_use]
    pub const fn apply(self, phase: ShutdownPhase, action: ShutdownAction) -> Option<ShutdownEffect> {
        if !matches!(phase, ShutdownPhase::FlushFailed { .. }) {
            return None;
        }
        Some(match action {
            ShutdownAction::Retry => ShutdownEffect::RetryCriticalFlush,
            ShutdownAction::QuitAnyway => ShutdownEffect::QuitAnyway,
        })
    }
}

/// Renders one modal recovery dialog and returns the selected typed action, if any.
///
/// The composition host owns visibility and lifecycle mutation: obtain the view model from
/// [`ShutdownController::failure_view_model`], render it while present, and pass the resulting
/// action through [`ShutdownController::apply`] before calling the application coordinator.
pub fn render_shutdown_failure(
    context: &egui::Context,
    view_model: ShutdownFailureViewModel,
) -> Option<ShutdownAction> {
    let mut selected = None;
    egui::Window::new(view_model.heading())
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(context, |ui| {
            ui.label(view_model.detail());
            ui.add_space(8.0);
            ui.label(view_model.warning());
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Retry").clicked() {
                    selected = Some(ShutdownAction::Retry);
                }
                if ui.button("Quit Anyway").clicked() {
                    selected = Some(ShutdownAction::QuitAnyway);
                }
            });
        });
    selected
}

const fn failure_view_model(step: ShutdownStep) -> ShutdownFailureViewModel {
    let (detail, warning) = match step {
        ShutdownStep::CheckpointCriticalActivity => (
            "OpenManic could not save your current tracking state before closing.",
            "Retry to keep your latest activity. Quit Anyway may leave recent activity unsaved.",
        ),
        ShutdownStep::FlushSettings => (
            "OpenManic could not save your recent settings before closing.",
            "Retry to keep your latest settings. Quit Anyway may leave recent changes unsaved.",
        ),
        ShutdownStep::CloseWriter => (
            "OpenManic could not safely finish closing its local data before exit.",
            "Retry to finish safely. Quit Anyway closes now and may leave recent changes unsaved.",
        ),
        ShutdownStep::RejectNonessentialWork
        | ShutdownStep::CancelSafeReads
        | ShutdownStep::JoinReadersAndWorkers
        | ShutdownStep::StopPlatform
        | ShutdownStep::JoinSupervisor => (
            "OpenManic could not finish a required shutdown step.",
            "Retry to finish safely. Quit Anyway closes now and may leave recent changes unsaved.",
        ),
    };
    ShutdownFailureViewModel {
        failed_step: step,
        heading: "Couldn’t finish saving before quit",
        detail,
        warning,
    }
}

#[cfg(test)]
mod tests {
    use openmanic_application::{ShutdownPhase, ShutdownStep};

    use super::{ShutdownAction, ShutdownController, ShutdownEffect};

    #[test]
    fn each_critical_failure_has_plain_language_recovery_content() {
        let controller = ShutdownController;
        for step in [
            ShutdownStep::CheckpointCriticalActivity,
            ShutdownStep::FlushSettings,
            ShutdownStep::CloseWriter,
        ] {
            let model = controller
                .failure_view_model(ShutdownPhase::FlushFailed { step })
                .expect("flush failure is visible");
            assert_eq!(model.failed_step(), step);
            assert_eq!(model.heading(), "Couldn’t finish saving before quit");
            assert!(!model.detail().contains("SQLite"));
            assert!(model.warning().contains("Quit Anyway"));
        }
    }

    #[test]
    fn retry_and_quit_anyway_are_typed_only_while_failure_is_pending() {
        let controller = ShutdownController;
        let failure = ShutdownPhase::FlushFailed {
            step: ShutdownStep::FlushSettings,
        };
        assert_eq!(
            controller.apply(failure, ShutdownAction::Retry),
            Some(ShutdownEffect::RetryCriticalFlush)
        );
        assert_eq!(
            controller.apply(failure, ShutdownAction::QuitAnyway),
            Some(ShutdownEffect::QuitAnyway)
        );
        assert_eq!(
            controller.apply(ShutdownPhase::Executing(ShutdownStep::CloseWriter), ShutdownAction::Retry),
            None
        );
        assert_eq!(
            controller.failure_view_model(ShutdownPhase::Complete),
            None
        );
    }
}
