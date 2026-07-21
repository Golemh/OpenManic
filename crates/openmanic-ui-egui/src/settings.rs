//! Presentation-only controller for the complete persisted settings snapshot.
//!
//! This module keeps basic and advanced controls as a local draft. It does not
//! perform I/O: composition persists an explicit [`SettingsEffect::Save`].

use openmanic_application::{EntityRevision, SettingsSnapshot, SettingsThemeMode};

/// Plain-language disclosure shown beside the title-collection control.
pub const TITLE_COLLECTION_DISCLOSURE: &str = "Window titles can contain sensitive text. Keep this off unless you explicitly want OpenManic to collect them on this device.";

/// The frequently used settings controls.
#[derive(Clone, Debug, Eq, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the editable projection deliberately groups independent checkbox preferences"
)]
pub struct SettingsBasicDraft {
    start_tracking_automatically: bool,
    start_at_login: bool,
    close_to_tray: bool,
    collect_window_titles: bool,
    theme_mode: SettingsThemeMode,
    notifications_enabled: bool,
    focus_sounds_enabled: bool,
}

impl SettingsBasicDraft {
    /// Replaces the automatic-tracking selection in this local draft.
    pub fn set_start_tracking_automatically(&mut self, value: bool) {
        self.start_tracking_automatically = value;
    }
    /// Replaces the Windows-login selection in this local draft.
    pub fn set_start_at_login(&mut self, value: bool) {
        self.start_at_login = value;
    }
    /// Replaces the close-to-tray selection in this local draft.
    pub fn set_close_to_tray(&mut self, value: bool) {
        self.close_to_tray = value;
    }
    /// Replaces the explicit title-collection consent in this local draft.
    pub fn set_collect_window_titles(&mut self, value: bool) {
        self.collect_window_titles = value;
    }
    /// Replaces the selected built-in appearance mode in this local draft.
    pub fn set_theme_mode(&mut self, value: SettingsThemeMode) {
        self.theme_mode = value;
    }
    /// Replaces the notification selection in this local draft.
    pub fn set_notifications_enabled(&mut self, value: bool) {
        self.notifications_enabled = value;
    }
    /// Replaces the focus-sound selection in this local draft.
    pub fn set_focus_sounds_enabled(&mut self, value: bool) {
        self.focus_sounds_enabled = value;
    }
    /// Returns whether tracking starts after consent.
    #[must_use]
    pub const fn start_tracking_automatically(&self) -> bool {
        self.start_tracking_automatically
    }
    /// Returns whether Windows login start is requested.
    #[must_use]
    pub const fn start_at_login(&self) -> bool {
        self.start_at_login
    }
    /// Returns whether closing the window keeps the app in the tray.
    #[must_use]
    pub const fn close_to_tray(&self) -> bool {
        self.close_to_tray
    }
    /// Returns whether privacy-sensitive window titles are collected.
    #[must_use]
    pub const fn collect_window_titles(&self) -> bool {
        self.collect_window_titles
    }
    /// Returns the selected built-in appearance mode.
    #[must_use]
    pub const fn theme_mode(&self) -> SettingsThemeMode {
        self.theme_mode
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
}

/// Less frequently changed operational and display settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsAdvancedDraft {
    consent_revision: u32,
    idle_threshold_seconds: u32,
    foreground_switch_delay_seconds: u32,
    idle_policy_code: u16,
    time_zone_mode: u8,
    manual_time_zone_id: Option<String>,
    density_code: u16,
    tray_explanation_acknowledged: bool,
}

impl SettingsAdvancedDraft {
    /// Replaces the accepted consent-document revision in this local draft.
    pub fn set_consent_revision(&mut self, value: u32) {
        self.consent_revision = value;
    }
    /// Replaces the inactivity threshold in this local draft.
    pub fn set_idle_threshold_seconds(&mut self, value: u32) {
        self.idle_threshold_seconds = value;
    }
    /// Replaces the foreground-switch confirmation delay in this local draft.
    pub fn set_foreground_switch_delay_seconds(&mut self, value: u32) {
        self.foreground_switch_delay_seconds = value;
    }
    /// Replaces the durable idle-policy selection in this local draft.
    pub fn set_idle_policy_code(&mut self, value: u16) {
        self.idle_policy_code = value;
    }
    /// Replaces the durable time-zone mode in this local draft.
    pub fn set_time_zone_mode(&mut self, value: u8) {
        self.time_zone_mode = value;
    }
    /// Replaces the optional manual IANA time-zone selection in this local draft.
    pub fn set_manual_time_zone_id(&mut self, value: Option<String>) {
        self.manual_time_zone_id = value;
    }
    /// Replaces the durable density selection in this local draft.
    pub fn set_density_code(&mut self, value: u16) {
        self.density_code = value;
    }
    /// Replaces the tray-explanation acknowledgement in this local draft.
    pub fn set_tray_explanation_acknowledged(&mut self, value: bool) {
        self.tray_explanation_acknowledged = value;
    }
    /// Returns the accepted consent-document revision; zero means not accepted.
    #[must_use]
    pub const fn consent_revision(&self) -> u32 {
        self.consent_revision
    }
    /// Returns the inactivity threshold in seconds.
    #[must_use]
    pub const fn idle_threshold_seconds(&self) -> u32 {
        self.idle_threshold_seconds
    }
    /// Returns the foreground-switch confirmation delay in seconds.
    #[must_use]
    pub const fn foreground_switch_delay_seconds(&self) -> u32 {
        self.foreground_switch_delay_seconds
    }
    /// Returns the durable idle-policy selection.
    #[must_use]
    pub const fn idle_policy_code(&self) -> u16 {
        self.idle_policy_code
    }
    /// Returns the durable time-zone mode.
    #[must_use]
    pub const fn time_zone_mode(&self) -> u8 {
        self.time_zone_mode
    }
    /// Returns the optional manual IANA time-zone selection.
    #[must_use]
    pub fn manual_time_zone_id(&self) -> Option<&str> {
        self.manual_time_zone_id.as_deref()
    }
    /// Returns the durable density selection.
    #[must_use]
    pub const fn density_code(&self) -> u16 {
        self.density_code
    }
    /// Returns whether the tray explanation was acknowledged.
    #[must_use]
    pub const fn tray_explanation_acknowledged(&self) -> bool {
        self.tray_explanation_acknowledged
    }
}

/// Typed local changes accepted by the settings controller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettingsAction {
    /// Discards all local changes and reloads an authoritative persisted snapshot.
    Load(SettingsSnapshot),
    /// Changes one basic setting.
    SetBasic(SettingsBasicDraft),
    /// Changes one advanced setting.
    SetAdvanced(SettingsAdvancedDraft),
    /// Reverts the draft to its last authoritative snapshot.
    Cancel,
    /// Requests atomic persistence of the complete draft.
    Save,
}

/// Typed output from a presentation-only settings action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettingsEffect {
    /// Composition must atomically replace the complete snapshot using this revision.
    Save {
        /// Complete replacement document, including unedited fields.
        settings: SettingsSnapshot,
        /// Revision the persistence adapter must compare before replacing.
        expected_revision: EntityRevision,
    },
    /// The local draft was discarded without an I/O request.
    Cancelled,
}

/// Complete settings draft separated into basic and advanced presentation sections.
#[derive(Clone, Debug)]
pub struct SettingsController {
    authoritative: SettingsSnapshot,
    basic: SettingsBasicDraft,
    advanced: SettingsAdvancedDraft,
}

impl SettingsController {
    /// Creates an editable presentation projection of the authoritative snapshot.
    #[must_use]
    pub fn new(settings: SettingsSnapshot) -> Self {
        let basic = basic_from(&settings);
        let advanced = advanced_from(&settings);
        Self {
            authoritative: settings,
            basic,
            advanced,
        }
    }

    /// Returns the last authoritative complete snapshot.
    #[must_use]
    pub const fn authoritative(&self) -> &SettingsSnapshot {
        &self.authoritative
    }
    /// Returns the current basic section draft.
    #[must_use]
    pub const fn basic(&self) -> &SettingsBasicDraft {
        &self.basic
    }
    /// Returns the current advanced section draft.
    #[must_use]
    pub const fn advanced(&self) -> &SettingsAdvancedDraft {
        &self.advanced
    }
    /// Returns the exact title-collection privacy disclosure.
    #[must_use]
    pub const fn title_collection_disclosure(&self) -> &'static str {
        TITLE_COLLECTION_DISCLOSURE
    }

    /// Applies a local edit and emits persistence only for an explicit save action.
    #[must_use]
    pub fn apply(&mut self, action: SettingsAction) -> Option<SettingsEffect> {
        match action {
            SettingsAction::Load(settings) => {
                self.authoritative = settings;
                self.reset_draft();
                None
            }
            SettingsAction::SetBasic(basic) => {
                self.basic = basic;
                None
            }
            SettingsAction::SetAdvanced(advanced) => {
                self.advanced = advanced;
                None
            }
            SettingsAction::Cancel => {
                self.reset_draft();
                Some(SettingsEffect::Cancelled)
            }
            SettingsAction::Save => Some(SettingsEffect::Save {
                settings: self.snapshot(),
                expected_revision: self.authoritative.revision(),
            }),
        }
    }

    /// Reconciles an authoritative successful replacement and resets the draft.
    pub fn confirm_saved(&mut self, settings: SettingsSnapshot) {
        self.authoritative = settings;
        self.reset_draft();
    }

    fn snapshot(&self) -> SettingsSnapshot {
        SettingsSnapshot::new(
            self.advanced.consent_revision,
            self.basic.start_tracking_automatically,
            self.basic.start_at_login,
            self.basic.close_to_tray,
            self.advanced.idle_threshold_seconds,
            self.advanced.foreground_switch_delay_seconds,
            self.advanced.idle_policy_code,
            self.basic.collect_window_titles,
            self.advanced.time_zone_mode,
            self.advanced.manual_time_zone_id.clone(),
            self.basic.theme_mode,
            self.advanced.density_code,
            self.basic.notifications_enabled,
            self.basic.focus_sounds_enabled,
            self.advanced.tray_explanation_acknowledged,
            self.authoritative.revision(),
        )
    }

    fn reset_draft(&mut self) {
        self.basic = basic_from(&self.authoritative);
        self.advanced = advanced_from(&self.authoritative);
    }
}

fn basic_from(settings: &SettingsSnapshot) -> SettingsBasicDraft {
    SettingsBasicDraft {
        start_tracking_automatically: settings.start_tracking_automatically(),
        start_at_login: settings.start_at_login(),
        close_to_tray: settings.close_to_tray(),
        collect_window_titles: settings.collect_window_titles(),
        theme_mode: settings.theme_mode(),
        notifications_enabled: settings.notifications_enabled(),
        focus_sounds_enabled: settings.focus_sounds_enabled(),
    }
}

fn advanced_from(settings: &SettingsSnapshot) -> SettingsAdvancedDraft {
    SettingsAdvancedDraft {
        consent_revision: settings.consent_revision(),
        idle_threshold_seconds: settings.idle_threshold_seconds(),
        foreground_switch_delay_seconds: settings.foreground_switch_delay_seconds(),
        idle_policy_code: settings.idle_policy_code(),
        time_zone_mode: settings.time_zone_mode(),
        manual_time_zone_id: settings.manual_time_zone_id().map(str::to_owned),
        density_code: settings.density_code(),
        tray_explanation_acknowledged: settings.tray_explanation_acknowledged(),
    }
}

#[cfg(test)]
mod tests {
    use super::{SettingsAction, SettingsController, SettingsEffect, TITLE_COLLECTION_DISCLOSURE};
    use openmanic_application::{EntityRevision, SettingsSnapshot, SettingsThemeMode};

    fn persisted() -> SettingsSnapshot {
        SettingsSnapshot::new(
            4,
            false,
            true,
            false,
            120,
            15,
            7,
            true,
            2,
            Some("Asia/Karachi".to_owned()),
            SettingsThemeMode::FollowSystem,
            3,
            false,
            false,
            true,
            EntityRevision::new(9),
        )
    }

    #[test]
    fn save_intent_contains_every_persisted_field_and_expected_revision() {
        let mut controller = SettingsController::new(persisted());
        let mut basic = controller.basic().clone();
        basic.set_collect_window_titles(false);
        let _ = controller.apply(SettingsAction::SetBasic(basic));

        let effect = controller.apply(SettingsAction::Save);
        assert!(matches!(effect, Some(SettingsEffect::Save { .. })));
        let Some(SettingsEffect::Save {
            settings,
            expected_revision,
        }) = effect
        else {
            return;
        };
        assert_eq!(expected_revision, EntityRevision::new(9));
        assert!(!settings.collect_window_titles());
        assert_eq!(settings.manual_time_zone_id(), Some("Asia/Karachi"));
        assert_eq!(settings.consent_revision(), 4);
        assert_eq!(settings.theme_mode(), SettingsThemeMode::FollowSystem);
        assert_eq!(settings.foreground_switch_delay_seconds(), 15);
    }

    #[test]
    fn cancel_restores_authoritative_basic_and_advanced_drafts() {
        let mut controller = SettingsController::new(persisted());
        let mut advanced = controller.advanced().clone();
        advanced.set_idle_threshold_seconds(42);
        advanced.set_foreground_switch_delay_seconds(20);
        let _ = controller.apply(SettingsAction::SetAdvanced(advanced));
        assert_eq!(controller.advanced().idle_threshold_seconds(), 42);

        assert_eq!(
            controller.apply(SettingsAction::Cancel),
            Some(SettingsEffect::Cancelled)
        );
        assert_eq!(controller.advanced().idle_threshold_seconds(), 120);
        assert_eq!(controller.advanced().foreground_switch_delay_seconds(), 15);
        assert!(controller.basic().collect_window_titles());
    }

    #[test]
    fn title_collection_is_explicitly_described_as_sensitive() {
        let controller = SettingsController::new(SettingsSnapshot::safe_default());
        assert_eq!(
            controller.title_collection_disclosure(),
            TITLE_COLLECTION_DISCLOSURE
        );
        assert!(TITLE_COLLECTION_DISCLOSURE.contains("sensitive"));
    }
}
