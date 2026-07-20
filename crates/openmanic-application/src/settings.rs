//! Authoritative singleton settings contracts shared by onboarding, runtime, and storage.

use crate::{DataRevision, EntityRevision};

/// One approved built-in appearance mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsThemeMode {
    /// Bundled dark appearance.
    Dark,
    /// Bundled light appearance.
    Light,
    /// Resolve against the operating-system preference.
    FollowSystem,
}

impl SettingsThemeMode {
    /// Returns the durable integer representation owned by the initial schema.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Dark => 0,
            Self::Light => 1,
            Self::FollowSystem => 2,
        }
    }

    /// Decodes the immutable persisted representation.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError`] when the mode code is unsupported.
    pub const fn try_from_code(code: u8) -> Result<Self, SettingsError> {
        match code {
            0 => Ok(Self::Dark),
            1 => Ok(Self::Light),
            2 => Ok(Self::FollowSystem),
            _ => Err(SettingsError::InvalidThemeMode),
        }
    }
}

/// Immutable complete persisted settings snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the persisted singleton deliberately mirrors independent user preferences"
)]
pub struct SettingsSnapshot {
    consent_revision: u32,
    start_tracking_automatically: bool,
    start_at_login: bool,
    close_to_tray: bool,
    idle_threshold_seconds: u32,
    idle_policy_code: u16,
    collect_window_titles: bool,
    time_zone_mode: u8,
    manual_time_zone_id: Option<String>,
    theme_mode: SettingsThemeMode,
    density_code: u16,
    notifications_enabled: bool,
    focus_sounds_enabled: bool,
    tray_explanation_acknowledged: bool,
    revision: EntityRevision,
}

impl SettingsSnapshot {
    /// Returns safe local-first defaults before a settings record has been persisted.
    #[must_use]
    pub const fn safe_default() -> Self {
        Self {
            consent_revision: 0,
            start_tracking_automatically: true,
            start_at_login: false,
            close_to_tray: true,
            idle_threshold_seconds: 300,
            idle_policy_code: 1,
            collect_window_titles: false,
            time_zone_mode: 0,
            manual_time_zone_id: None,
            theme_mode: SettingsThemeMode::Dark,
            density_code: 1,
            notifications_enabled: true,
            focus_sounds_enabled: true,
            tray_explanation_acknowledged: false,
            revision: EntityRevision::new(0),
        }
    }

    /// Creates a complete valid replacement snapshot.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        clippy::fn_params_excessive_bools,
        reason = "the atomic replacement contract deliberately names every persisted preference"
    )]
    pub const fn new(
        consent_revision: u32,
        start_tracking_automatically: bool,
        start_at_login: bool,
        close_to_tray: bool,
        idle_threshold_seconds: u32,
        idle_policy_code: u16,
        collect_window_titles: bool,
        time_zone_mode: u8,
        manual_time_zone_id: Option<String>,
        theme_mode: SettingsThemeMode,
        density_code: u16,
        notifications_enabled: bool,
        focus_sounds_enabled: bool,
        tray_explanation_acknowledged: bool,
        revision: EntityRevision,
    ) -> Self {
        Self {
            consent_revision,
            start_tracking_automatically,
            start_at_login,
            close_to_tray,
            idle_threshold_seconds,
            idle_policy_code,
            collect_window_titles,
            time_zone_mode,
            manual_time_zone_id,
            theme_mode,
            density_code,
            notifications_enabled,
            focus_sounds_enabled,
            tray_explanation_acknowledged,
            revision,
        }
    }

    /// Returns the persisted local tracking consent revision; zero means not accepted.
    #[must_use]
    pub const fn consent_revision(&self) -> u32 {
        self.consent_revision
    }
    /// Returns whether tracking starts automatically after consent.
    #[must_use]
    pub const fn start_tracking_automatically(&self) -> bool {
        self.start_tracking_automatically
    }
    /// Returns whether Windows login start is requested.
    #[must_use]
    pub const fn start_at_login(&self) -> bool {
        self.start_at_login
    }
    /// Returns the persisted close-to-tray preference.
    #[must_use]
    pub const fn close_to_tray(&self) -> bool {
        self.close_to_tray
    }
    /// Returns the persisted idle threshold in seconds.
    #[must_use]
    pub const fn idle_threshold_seconds(&self) -> u32 {
        self.idle_threshold_seconds
    }
    /// Returns the durable idle-policy code.
    #[must_use]
    pub const fn idle_policy_code(&self) -> u16 {
        self.idle_policy_code
    }
    /// Returns whether title collection has explicit consent.
    #[must_use]
    pub const fn collect_window_titles(&self) -> bool {
        self.collect_window_titles
    }
    /// Returns the durable time-zone mode.
    #[must_use]
    pub const fn time_zone_mode(&self) -> u8 {
        self.time_zone_mode
    }
    /// Returns the manual IANA time-zone selection when manual mode is active.
    #[must_use]
    pub fn manual_time_zone_id(&self) -> Option<&str> {
        self.manual_time_zone_id.as_deref()
    }
    /// Returns the selected theme mode.
    #[must_use]
    pub const fn theme_mode(&self) -> SettingsThemeMode {
        self.theme_mode
    }
    /// Returns the durable density selection code.
    #[must_use]
    pub const fn density_code(&self) -> u16 {
        self.density_code
    }
    /// Returns whether notifications are enabled.
    #[must_use]
    pub const fn notifications_enabled(&self) -> bool {
        self.notifications_enabled
    }
    /// Returns whether focus sounds are enabled.
    #[must_use]
    pub const fn focus_sounds_enabled(&self) -> bool {
        self.focus_sounds_enabled
    }
    /// Returns whether the first close-to-tray explanation has been acknowledged.
    #[must_use]
    pub const fn tray_explanation_acknowledged(&self) -> bool {
        self.tray_explanation_acknowledged
    }
    /// Returns the optimistic settings revision.
    #[must_use]
    pub const fn revision(&self) -> EntityRevision {
        self.revision
    }
}

/// Typed persistence boundary for atomic complete settings replacement.
pub trait SettingsPersistence {
    /// Reads the persisted settings singleton, if it exists.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsPersistenceError`] when storage cannot read a validated snapshot.
    fn load_settings(&mut self) -> Result<Option<SettingsSnapshot>, SettingsPersistenceError>;

    /// Atomically replaces every persisted settings field after optimistic revision validation.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsPersistenceError`] for conflicts or persistence failure.
    fn replace_settings(
        &mut self,
        settings: &SettingsSnapshot,
        expected_revision: Option<EntityRevision>,
    ) -> Result<DataRevision, SettingsPersistenceError>;
}

/// Settings persistence failure without a concrete storage dependency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsPersistenceError {
    /// The current singleton revision does not match the requested revision.
    RevisionConflict,
    /// The persisted representation is invalid.
    InvalidDocument,
    /// The backing store could not complete the operation.
    Failed,
}

/// Settings validation error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsError {
    /// The durable theme code is unsupported.
    InvalidThemeMode,
}

#[cfg(test)]
mod tests {
    use super::{SettingsSnapshot, SettingsThemeMode};
    use crate::EntityRevision;

    #[test]
    fn defaults_keep_titles_disabled_and_close_to_tray_enabled() {
        let settings = SettingsSnapshot::safe_default();
        assert!(!settings.collect_window_titles());
        assert!(settings.close_to_tray());
        assert_eq!(settings.revision(), EntityRevision::new(0));
        assert_eq!(
            SettingsThemeMode::try_from_code(2),
            Ok(SettingsThemeMode::FollowSystem)
        );
    }
}
