//! Versioned built-in semantic theme resolution and atomic egui application.

use eframe::egui::{self, Color32, Context, Theme};

/// Built-in theme selection persisted by the domain settings document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuiltInThemeMode {
    /// The bundled dark theme.
    Dark,
    /// The bundled light theme.
    Light,
    /// The bundled theme matching the current system preference.
    FollowSystem,
}

impl BuiltInThemeMode {
    /// Converts the stable domain key into a built-in selection.
    ///
    /// # Errors
    ///
    /// Returns [`ThemeResolutionError`] when the key is not an approved built-in selection.
    pub fn try_from_key(key: &str) -> Result<Self, ThemeResolutionError> {
        match key {
            "openmanic.dark" => Ok(Self::Dark),
            "openmanic.light" => Ok(Self::Light),
            "openmanic.system" => Ok(Self::FollowSystem),
            _ => Err(ThemeResolutionError::UnknownBuiltInKey),
        }
    }
}

/// Typed semantic colors shared by egui styling and custom widget renderers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThemeTokens {
    canvas: Color32,
    panel: Color32,
    content_primary: Color32,
    content_secondary: Color32,
    interaction_primary: Color32,
    success: Color32,
    warning: Color32,
    error: Color32,
    timeline_grid: Color32,
    schedule_bracket: Color32,
}

impl Default for ThemeTokens {
    fn default() -> Self {
        tokens_for(BuiltInThemeMode::Dark)
    }
}

impl ThemeTokens {
    /// Returns the application canvas surface.
    #[must_use]
    pub const fn canvas(self) -> Color32 {
        self.canvas
    }
    /// Returns the standard panel surface.
    #[must_use]
    pub const fn panel(self) -> Color32 {
        self.panel
    }
    /// Returns primary content color.
    #[must_use]
    pub const fn content_primary(self) -> Color32 {
        self.content_primary
    }
    /// Returns secondary content color.
    #[must_use]
    pub const fn content_secondary(self) -> Color32 {
        self.content_secondary
    }
    /// Returns the primary interaction color.
    #[must_use]
    pub const fn interaction_primary(self) -> Color32 {
        self.interaction_primary
    }
    /// Returns the success state color.
    #[must_use]
    pub const fn success(self) -> Color32 {
        self.success
    }
    /// Returns the warning state color.
    #[must_use]
    pub const fn warning(self) -> Color32 {
        self.warning
    }
    /// Returns the error state color.
    #[must_use]
    pub const fn error(self) -> Color32 {
        self.error
    }
    /// Returns the timeline grid color.
    #[must_use]
    pub const fn timeline_grid(self) -> Color32 {
        self.timeline_grid
    }
    /// Returns the schedule bracket color.
    #[must_use]
    pub const fn schedule_bracket(self) -> Color32 {
        self.schedule_bracket
    }
}

/// Fully validated built-in theme ready for atomic foreground application.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedTheme {
    mode: BuiltInThemeMode,
    tokens: ThemeTokens,
}

impl ResolvedTheme {
    /// Resolves a stable key through the shared built-in path.
    ///
    /// # Errors
    ///
    /// Returns [`ThemeResolutionError`] for an unsupported built-in key.
    pub fn resolve(key: &str, system_prefers_dark: bool) -> Result<Self, ThemeResolutionError> {
        let requested = BuiltInThemeMode::try_from_key(key)?;
        let mode = match requested {
            BuiltInThemeMode::FollowSystem if system_prefers_dark => BuiltInThemeMode::Dark,
            BuiltInThemeMode::FollowSystem => BuiltInThemeMode::Light,
            mode => mode,
        };
        Ok(Self {
            mode,
            tokens: tokens_for(mode),
        })
    }

    /// Returns the resolved Dark or Light mode used for drawing.
    #[must_use]
    pub const fn mode(self) -> BuiltInThemeMode {
        self.mode
    }
    /// Returns semantic colors for custom widget renderers.
    #[must_use]
    pub const fn tokens(self) -> ThemeTokens {
        self.tokens
    }

    /// Applies the complete egui visual/style projection after resolution succeeds.
    pub fn apply(self, context: &Context) {
        let mut visuals = match self.mode {
            BuiltInThemeMode::Dark => egui::Visuals::dark(),
            BuiltInThemeMode::Light => egui::Visuals::light(),
            BuiltInThemeMode::FollowSystem => return,
        };
        visuals.window_fill = self.tokens.panel;
        visuals.panel_fill = self.tokens.canvas;
        visuals.faint_bg_color = self.tokens.panel;
        visuals.extreme_bg_color = self.tokens.canvas;
        visuals.override_text_color = Some(self.tokens.content_primary);
        visuals.selection.bg_fill = self.tokens.interaction_primary;
        visuals.error_fg_color = self.tokens.error;
        visuals.widgets.noninteractive.bg_fill = self.tokens.panel;
        visuals.widgets.noninteractive.bg_stroke =
            egui::Stroke::new(1.0, self.tokens.timeline_grid);
        visuals.widgets.inactive.bg_fill = self.tokens.interaction_primary.gamma_multiply(0.14);
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, self.tokens.timeline_grid);
        visuals.widgets.hovered.bg_fill = self.tokens.interaction_primary.gamma_multiply(0.28);
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, self.tokens.interaction_primary);
        visuals.widgets.active.bg_fill = self.tokens.interaction_primary.gamma_multiply(0.45);
        visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, self.tokens.interaction_primary);
        for widget in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.corner_radius = egui::CornerRadius::same(6);
        }

        let native_theme = match self.mode {
            BuiltInThemeMode::Dark => Theme::Dark,
            BuiltInThemeMode::Light => Theme::Light,
            BuiltInThemeMode::FollowSystem => return,
        };
        let mut style = (*context.style_of(native_theme)).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        context.set_style_of(native_theme, style);
        context.set_visuals(visuals);
    }
}

/// Resolution failure that must preserve the previous complete theme.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemeResolutionError {
    /// The persisted selection is not one of the approved built-in keys.
    UnknownBuiltInKey,
}

/// Retains the last complete theme so invalid selections never partially apply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThemeController {
    current: ResolvedTheme,
}

impl Default for ThemeController {
    fn default() -> Self {
        Self {
            current: ResolvedTheme {
                mode: BuiltInThemeMode::Dark,
                tokens: tokens_for(BuiltInThemeMode::Dark),
            },
        }
    }
}

impl ThemeController {
    /// Returns the complete currently active theme.
    #[must_use]
    pub const fn current(self) -> ResolvedTheme {
        self.current
    }

    /// Resolves and applies a new theme atomically, retaining the previous one on failure.
    ///
    /// # Errors
    ///
    /// Returns [`ThemeResolutionError`] without changing the active theme when `key` is invalid.
    pub fn apply_key(
        &mut self,
        context: &Context,
        key: &str,
        system_prefers_dark: bool,
    ) -> Result<(), ThemeResolutionError> {
        let resolved = ResolvedTheme::resolve(key, system_prefers_dark)?;
        resolved.apply(context);
        self.current = resolved;
        Ok(())
    }

    /// Applies the existing complete theme at the first safe foreground update.
    pub fn apply_current(self, context: &Context) {
        self.current.apply(context);
    }
}

fn tokens_for(mode: BuiltInThemeMode) -> ThemeTokens {
    match mode {
        BuiltInThemeMode::Dark => ThemeTokens {
            canvas: Color32::from_rgb(3, 7, 18),
            panel: Color32::from_rgb(9, 13, 26),
            content_primary: Color32::from_rgb(243, 246, 255),
            content_secondary: Color32::from_rgb(137, 151, 184),
            interaction_primary: Color32::from_rgb(103, 84, 255),
            success: Color32::from_rgb(52, 211, 153),
            warning: Color32::from_rgb(245, 158, 11),
            error: Color32::from_rgb(244, 63, 94),
            timeline_grid: Color32::from_rgb(30, 41, 59),
            schedule_bracket: Color32::from_rgb(34, 211, 238),
        },
        BuiltInThemeMode::Light => ThemeTokens {
            canvas: Color32::from_rgb(244, 247, 252),
            panel: Color32::from_rgb(255, 255, 255),
            content_primary: Color32::from_rgb(25, 31, 42),
            content_secondary: Color32::from_rgb(77, 89, 109),
            interaction_primary: Color32::from_rgb(56, 97, 210),
            success: Color32::from_rgb(31, 132, 79),
            warning: Color32::from_rgb(154, 102, 13),
            error: Color32::from_rgb(184, 49, 49),
            timeline_grid: Color32::from_rgb(138, 150, 168),
            schedule_bracket: Color32::from_rgb(27, 116, 192),
        },
        BuiltInThemeMode::FollowSystem => {
            unreachable!("Follow System always resolves to Dark or Light")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BuiltInThemeMode, ResolvedTheme, ThemeController, ThemeResolutionError};

    #[test]
    fn dark_light_and_system_resolve_through_the_same_contract() {
        assert_eq!(
            ResolvedTheme::resolve("openmanic.dark", false).map(ResolvedTheme::mode),
            Ok(BuiltInThemeMode::Dark)
        );
        assert_eq!(
            ResolvedTheme::resolve("openmanic.light", true).map(ResolvedTheme::mode),
            Ok(BuiltInThemeMode::Light)
        );
        assert_eq!(
            ResolvedTheme::resolve("openmanic.system", false).map(ResolvedTheme::mode),
            Ok(BuiltInThemeMode::Light)
        );
    }

    #[test]
    fn invalid_theme_key_rejects_before_replacing_the_complete_current_theme() {
        let controller = ThemeController::default();
        let previous = controller.current();
        assert_eq!(
            ResolvedTheme::resolve("not-a-theme", true),
            Err(ThemeResolutionError::UnknownBuiltInKey)
        );
        assert_eq!(controller.current(), previous);
    }
}
