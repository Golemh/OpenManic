//! Versioned document values for user-customizable OpenManic state.

//!
//! This module validates in-memory values and provides deterministic migration and safe fallback.
//! Parsing, serialization, persistence, and renderer conversion deliberately belong elsewhere.

#![expect(
    dead_code,
    reason = "OM-130 defines unconnected value internals until OM-140 owns application contract wiring"
)]

use core::fmt;
use std::collections::BTreeSet;

const LAYOUT_SCHEMA: u16 = 1;
const SAVED_VIEW_SCHEMA: u16 = 1;
const SETTINGS_SCHEMA: u16 = 1;
const THEME_SCHEMA: u16 = 1;
const MAX_TEXT_BYTES: usize = 512;

/// A validated, versioned Today-dashboard layout value.
///
/// It contains stable widget identity, constrained placement, scalar configuration values, and
/// scalar appearance overrides. It never owns executable behavior.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayoutDocument {
    envelope: Envelope<LayoutPayload>,
}

impl LayoutDocument {
    /// Returns the safe built-in layout used after an invalid value is preserved for diagnostics.
    #[must_use]
    pub fn safe_default() -> Self {
        Self {
            envelope: Envelope::new(
                LAYOUT_SCHEMA,
                0,
                LayoutPayload {
                    widgets: vec![
                        layout_widget(
                            "today.timeline",
                            "openmanic.timeline.day",
                            0,
                            12,
                            Height::Tall,
                        ),
                        layout_widget(
                            "today.usage",
                            "openmanic.usage.application",
                            1,
                            4,
                            Height::Standard,
                        ),
                        layout_widget(
                            "today.distribution",
                            "openmanic.distribution.time",
                            2,
                            4,
                            Height::Standard,
                        ),
                        layout_widget(
                            "today.focus",
                            "openmanic.focus.session",
                            3,
                            4,
                            Height::Standard,
                        ),
                    ],
                },
            ),
        }
    }

    /// Returns the normalized document schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.envelope.schema_version
    }

    /// Returns the optimistic-concurrency revision retained by this value.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.envelope.revision
    }

    /// Returns the number of stable widget instances.
    #[must_use]
    pub fn widget_count(&self) -> usize {
        self.envelope.payload.widgets.len()
    }
}

/// A normalized saved Overview-view value without dashboard-layout ownership.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SavedViewDocument {
    envelope: Envelope<SavedViewPayload>,
}

impl SavedViewDocument {
    /// Returns a safe default Overview view with a relative current-week range.
    #[must_use]
    pub fn safe_default() -> Self {
        Self {
            envelope: Envelope::new(
                SAVED_VIEW_SCHEMA,
                0,
                SavedViewPayload {
                    public_id: "default-overview-view".to_owned(),
                    name: "Overview".to_owned(),
                    display_order: 0,
                    range: Range::Relative(RelativeRange::Week),
                    grouping: "category".to_owned(),
                    filters: VersionedFields::empty(),
                    sort: "duration-descending".to_owned(),
                    widget_configuration: VersionedFields::empty(),
                },
            ),
        }
    }

    /// Returns the normalized document schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.envelope.schema_version
    }

    /// Returns the validated user-visible saved-view name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.envelope.payload.name
    }
}

/// A validated singleton settings value without excluded-application duplication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsDocument {
    envelope: Envelope<SettingsPayload>,
}

impl SettingsDocument {
    /// Returns safe local-first defaults applied after malformed settings are quarantined.
    #[must_use]
    pub fn safe_default() -> Self {
        Self {
            envelope: Envelope::new(
                SETTINGS_SCHEMA,
                0,
                SettingsPayload {
                    first_launch_consent_revision: 0,
                    start_tracking_automatically: true,
                    start_at_login: false,
                    close_to_tray: true,
                    idle_threshold_seconds: 300,
                    idle_policy_code: 1,
                    collect_window_titles: false,
                    time_zone_mode: TimeZoneMode::Automatic,
                    theme_selection: ThemeSelection::dark(),
                    density_code: 1,
                    notifications_enabled: true,
                    focus_sounds_enabled: true,
                    tray_explanation_acknowledged: false,
                },
            ),
        }
    }

    /// Returns the normalized document schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.envelope.schema_version
    }

    /// Returns the selected built-in theme value.
    #[must_use]
    pub fn theme_selection(&self) -> &ThemeSelection {
        &self.envelope.payload.theme_selection
    }
}

/// A versioned choice among built-in declarative themes.
///
/// This type is deliberately only a built-in key selector. A UI layer later resolves that key into
/// a validated semantic theme and framework-specific presentation values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThemeSelection {
    envelope: Envelope<ThemePayload>,
}

impl ThemeSelection {
    /// Returns the bundled dark theme selection.
    #[must_use]
    pub fn dark() -> Self {
        Self::from_mode(ThemeMode::Dark)
    }

    /// Returns the bundled light theme selection.
    #[must_use]
    pub fn light() -> Self {
        Self::from_mode(ThemeMode::Light)
    }

    /// Returns the built-in Follow System theme selection.
    #[must_use]
    pub fn follow_system() -> Self {
        Self::from_mode(ThemeMode::FollowSystem)
    }

    /// Returns the normalized document schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.envelope.schema_version
    }

    /// Returns the stable built-in key selected by this value.
    #[must_use]
    pub fn built_in_key(&self) -> &'static str {
        self.envelope.payload.mode.key()
    }

    fn from_mode(mode: ThemeMode) -> Self {
        Self {
            envelope: Envelope::new(THEME_SCHEMA, 0, ThemePayload { mode }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Envelope<T> {
    schema_version: u16,
    revision: u64,
    payload: T,
}

impl<T> Envelope<T> {
    const fn new(schema_version: u16, revision: u64, payload: T) -> Self {
        Self {
            schema_version,
            revision,
            payload,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Loaded<T> {
    Valid(T),
    Fallback {
        document: T,
        invalid: InvalidDocument,
    },
}

#[cfg(test)]
impl<T> Loaded<T> {
    fn into_valid(self) -> Result<T, InvalidDocument> {
        match self {
            Self::Valid(document) => Ok(document),
            Self::Fallback { invalid, .. } => Err(invalid),
        }
    }

    fn into_fallback(self) -> Result<(T, InvalidDocument), T> {
        match self {
            Self::Valid(document) => Err(document),
            Self::Fallback { document, invalid } => Ok((document, invalid)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct InvalidDocument {
    raw_source: String,
    error: DocumentError,
    revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DocumentError {
    UnsupportedSchema { schema_version: u16 },
    EmptyCollection { field: &'static str },
    DuplicateValue { field: &'static str, value: String },
    InvalidIdentifier { field: &'static str, value: String },
    InvalidValue { field: &'static str, value: String },
    InvalidWidth { width_span: u8 },
    InvertedDateRange { start: String, end: String },
    InvalidTimeZoneId { value: String },
}

impl fmt::Display for DocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema { schema_version } => {
                write!(formatter, "unsupported document schema {schema_version}")
            }
            Self::EmptyCollection { field } => write!(formatter, "{field} must not be empty"),
            Self::DuplicateValue { field, value } => {
                write!(formatter, "duplicate {field}: {value}")
            }
            Self::InvalidIdentifier { field, value } => {
                write!(formatter, "invalid {field}: {value}")
            }
            Self::InvalidValue { field, value } => write!(formatter, "invalid {field}: {value}"),
            Self::InvalidWidth { width_span } => {
                write!(formatter, "invalid width span {width_span}")
            }
            Self::InvertedDateRange { start, end } => {
                write!(formatter, "range ends {end} before {start}")
            }
            Self::InvalidTimeZoneId { value } => write!(formatter, "invalid time zone ID: {value}"),
        }
    }
}

impl std::error::Error for DocumentError {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LayoutPayload {
    widgets: Vec<LayoutWidget>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LayoutWidget {
    instance_id: String,
    kind_id: String,
    kind_schema_version: u16,
    order: u32,
    width_span: u8,
    height: Height,
    configuration: VersionedFields,
    appearance_overrides: Option<VersionedFields>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Height {
    Compact,
    Standard,
    Tall,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VersionedFields {
    schema_version: u16,
    fields: Vec<DocumentField>,
}

impl VersionedFields {
    const fn empty() -> Self {
        Self {
            schema_version: 1,
            fields: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DocumentField {
    name: String,
    value: Scalar,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Scalar {
    Boolean(bool),
    Integer(i64),
    Text(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LegacyLayoutV0 {
    widget_kind_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum LayoutIncoming {
    Current(Envelope<LayoutPayload>),
    LegacyV0(Envelope<LegacyLayoutV0>),
    Unsupported { schema_version: u16, revision: u64 },
}

fn layout_widget(
    instance_id: &str,
    kind_id: &str,
    order: u32,
    width_span: u8,
    height: Height,
) -> LayoutWidget {
    LayoutWidget {
        instance_id: instance_id.to_owned(),
        kind_id: kind_id.to_owned(),
        kind_schema_version: 1,
        order,
        width_span,
        height,
        configuration: VersionedFields::empty(),
        appearance_overrides: None,
    }
}

fn load_layout(source: String, incoming: LayoutIncoming) -> Loaded<LayoutDocument> {
    let migrated = match incoming {
        LayoutIncoming::Current(envelope) => envelope,
        LayoutIncoming::LegacyV0(envelope) => {
            let revision = envelope.revision;
            match migrate_layout_v0(envelope) {
                Ok(migrated) => migrated,
                Err(error) => return layout_fallback(source, error, revision),
            }
        }
        LayoutIncoming::Unsupported {
            schema_version,
            revision,
        } => {
            return layout_fallback(
                source,
                DocumentError::UnsupportedSchema { schema_version },
                revision,
            );
        }
    };
    match validate_layout(&migrated.payload) {
        Ok(()) => Loaded::Valid(LayoutDocument { envelope: migrated }),
        Err(error) => layout_fallback(source, error, migrated.revision),
    }
}

fn layout_fallback(source: String, error: DocumentError, revision: u64) -> Loaded<LayoutDocument> {
    Loaded::Fallback {
        document: LayoutDocument::safe_default(),
        invalid: InvalidDocument {
            raw_source: source,
            error,
            revision,
        },
    }
}

fn migrate_layout_v0(
    envelope: Envelope<LegacyLayoutV0>,
) -> Result<Envelope<LayoutPayload>, DocumentError> {
    let mut widgets = Vec::with_capacity(envelope.payload.widget_kind_ids.len());
    for (index, kind_id) in envelope.payload.widget_kind_ids.into_iter().enumerate() {
        let order = u32::try_from(index).map_err(|_| DocumentError::InvalidValue {
            field: "legacy widget order",
            value: index.to_string(),
        })?;
        let timeline = kind_id == "openmanic.timeline.day";
        widgets.push(layout_widget(
            &format!("legacy-widget-{index}"),
            &kind_id,
            order,
            if timeline { 12 } else { 4 },
            if timeline {
                Height::Tall
            } else {
                Height::Standard
            },
        ));
    }
    Ok(Envelope::new(
        LAYOUT_SCHEMA,
        envelope.revision,
        LayoutPayload { widgets },
    ))
}

fn validate_layout(layout: &LayoutPayload) -> Result<(), DocumentError> {
    if layout.widgets.is_empty() {
        return Err(DocumentError::EmptyCollection { field: "widgets" });
    }
    let mut instances = BTreeSet::new();
    let mut orders = BTreeSet::new();
    for widget in &layout.widgets {
        validate_identifier("widget instance ID", &widget.instance_id)?;
        validate_identifier("widget kind ID", &widget.kind_id)?;
        if widget.kind_schema_version == 0 {
            return Err(invalid_number("widget kind schema version", 0));
        }
        if ![3, 4, 6, 8, 9, 12].contains(&widget.width_span) {
            return Err(DocumentError::InvalidWidth {
                width_span: widget.width_span,
            });
        }
        validate_fields("widget configuration", &widget.configuration)?;
        if let Some(appearance) = &widget.appearance_overrides {
            validate_fields("widget appearance overrides", appearance)?;
        }
        if !instances.insert(&widget.instance_id) {
            return Err(DocumentError::DuplicateValue {
                field: "widget instance ID",
                value: widget.instance_id.clone(),
            });
        }
        if !orders.insert(widget.order) {
            return Err(DocumentError::DuplicateValue {
                field: "widget order",
                value: widget.order.to_string(),
            });
        }
        let _ = widget.height;
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SavedViewPayload {
    public_id: String,
    name: String,
    display_order: u32,
    range: Range,
    grouping: String,
    filters: VersionedFields,
    sort: String,
    widget_configuration: VersionedFields,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Range {
    Relative(RelativeRange),
    Fixed {
        start_local_date: String,
        end_local_date: String,
        time_zone_behavior: TimeZoneMode,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RelativeRange {
    Day,
    Week,
    Month,
    Year,
    Custom,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LegacySavedViewV0 {
    public_id: String,
    name: String,
    display_order: u32,
    range: RelativeRange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SavedViewIncoming {
    Current(Envelope<SavedViewPayload>),
    LegacyV0(Envelope<LegacySavedViewV0>),
    Unsupported { schema_version: u16, revision: u64 },
}

fn load_saved_view(source: String, incoming: SavedViewIncoming) -> Loaded<SavedViewDocument> {
    let migrated = match incoming {
        SavedViewIncoming::Current(envelope) => envelope,
        SavedViewIncoming::LegacyV0(envelope) => migrate_saved_view_v0(envelope),
        SavedViewIncoming::Unsupported {
            schema_version,
            revision,
        } => {
            return saved_view_fallback(
                source,
                DocumentError::UnsupportedSchema { schema_version },
                revision,
            );
        }
    };
    match validate_saved_view(&migrated.payload) {
        Ok(()) => Loaded::Valid(SavedViewDocument { envelope: migrated }),
        Err(error) => saved_view_fallback(source, error, migrated.revision),
    }
}

fn saved_view_fallback(
    source: String,
    error: DocumentError,
    revision: u64,
) -> Loaded<SavedViewDocument> {
    Loaded::Fallback {
        document: SavedViewDocument::safe_default(),
        invalid: InvalidDocument {
            raw_source: source,
            error,
            revision,
        },
    }
}

fn migrate_saved_view_v0(envelope: Envelope<LegacySavedViewV0>) -> Envelope<SavedViewPayload> {
    Envelope::new(
        SAVED_VIEW_SCHEMA,
        envelope.revision,
        SavedViewPayload {
            public_id: envelope.payload.public_id,
            name: envelope.payload.name,
            display_order: envelope.payload.display_order,
            range: Range::Relative(envelope.payload.range),
            grouping: "category".to_owned(),
            filters: VersionedFields::empty(),
            sort: "duration-descending".to_owned(),
            widget_configuration: VersionedFields::empty(),
        },
    )
}

fn validate_saved_view(view: &SavedViewPayload) -> Result<(), DocumentError> {
    validate_identifier("saved view public ID", &view.public_id)?;
    validate_display_text("saved view name", &view.name)?;
    validate_identifier("saved view grouping", &view.grouping)?;
    validate_identifier("saved view sort", &view.sort)?;
    validate_fields("saved view filters", &view.filters)?;
    validate_fields(
        "saved view widget configuration",
        &view.widget_configuration,
    )?;
    match &view.range {
        Range::Relative(_) => Ok(()),
        Range::Fixed {
            start_local_date,
            end_local_date,
            time_zone_behavior,
        } => {
            validate_local_date(start_local_date)?;
            validate_local_date(end_local_date)?;
            if end_local_date < start_local_date {
                return Err(DocumentError::InvertedDateRange {
                    start: start_local_date.clone(),
                    end: end_local_date.clone(),
                });
            }
            validate_time_zone(time_zone_behavior)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "fields mirror the product-defined persisted settings singleton"
)]
struct SettingsPayload {
    first_launch_consent_revision: u32,
    start_tracking_automatically: bool,
    start_at_login: bool,
    close_to_tray: bool,
    idle_threshold_seconds: u32,
    idle_policy_code: u16,
    collect_window_titles: bool,
    time_zone_mode: TimeZoneMode,
    theme_selection: ThemeSelection,
    density_code: u16,
    notifications_enabled: bool,
    focus_sounds_enabled: bool,
    tray_explanation_acknowledged: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TimeZoneMode {
    Automatic,
    Manual(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LegacySettingsV0 {
    first_launch_consent_revision: u32,
    idle_threshold_seconds: u32,
    theme_key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SettingsIncoming {
    Current(Envelope<SettingsPayload>),
    LegacyV0(Envelope<LegacySettingsV0>),
    Unsupported { schema_version: u16, revision: u64 },
}

fn load_settings(source: String, incoming: SettingsIncoming) -> Loaded<SettingsDocument> {
    let migrated = match incoming {
        SettingsIncoming::Current(envelope) => envelope,
        SettingsIncoming::LegacyV0(envelope) => migrate_settings_v0(&envelope),
        SettingsIncoming::Unsupported {
            schema_version,
            revision,
        } => {
            return settings_fallback(
                source,
                DocumentError::UnsupportedSchema { schema_version },
                revision,
            );
        }
    };
    match validate_settings(&migrated.payload) {
        Ok(()) => Loaded::Valid(SettingsDocument { envelope: migrated }),
        Err(error) => settings_fallback(source, error, migrated.revision),
    }
}

fn settings_fallback(
    source: String,
    error: DocumentError,
    revision: u64,
) -> Loaded<SettingsDocument> {
    Loaded::Fallback {
        document: SettingsDocument::safe_default(),
        invalid: InvalidDocument {
            raw_source: source,
            error,
            revision,
        },
    }
}

fn migrate_settings_v0(envelope: &Envelope<LegacySettingsV0>) -> Envelope<SettingsPayload> {
    let theme_selection = ThemeSelection::try_from_key(&envelope.payload.theme_key)
        .unwrap_or_else(|_| ThemeSelection::dark());
    Envelope::new(
        SETTINGS_SCHEMA,
        envelope.revision,
        SettingsPayload {
            first_launch_consent_revision: envelope.payload.first_launch_consent_revision,
            start_tracking_automatically: true,
            start_at_login: false,
            close_to_tray: true,
            idle_threshold_seconds: envelope.payload.idle_threshold_seconds,
            idle_policy_code: 1,
            collect_window_titles: false,
            time_zone_mode: TimeZoneMode::Automatic,
            theme_selection,
            density_code: 1,
            notifications_enabled: true,
            focus_sounds_enabled: true,
            tray_explanation_acknowledged: false,
        },
    )
}

fn validate_settings(settings: &SettingsPayload) -> Result<(), DocumentError> {
    if settings.idle_threshold_seconds == 0 {
        return Err(invalid_number("idle threshold seconds", 0));
    }
    if settings.idle_policy_code == 0 {
        return Err(invalid_number("idle policy code", 0));
    }
    if settings.density_code == 0 {
        return Err(invalid_number("density code", 0));
    }
    validate_time_zone(&settings.time_zone_mode)?;
    validate_theme(&settings.theme_selection.envelope.payload)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ThemePayload {
    mode: ThemeMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThemeMode {
    Dark,
    Light,
    FollowSystem,
}

impl ThemeMode {
    const fn key(self) -> &'static str {
        match self {
            Self::Dark => "openmanic.dark",
            Self::Light => "openmanic.light",
            Self::FollowSystem => "openmanic.system",
        }
    }

    fn try_from_key(key: &str) -> Result<Self, DocumentError> {
        match key {
            "openmanic.dark" | "dark" => Ok(Self::Dark),
            "openmanic.light" | "light" => Ok(Self::Light),
            "openmanic.system" | "system" => Ok(Self::FollowSystem),
            _ => Err(DocumentError::InvalidValue {
                field: "built-in theme key",
                value: key.to_owned(),
            }),
        }
    }
}

impl ThemeSelection {
    fn try_from_key(key: &str) -> Result<Self, DocumentError> {
        ThemeMode::try_from_key(key).map(Self::from_mode)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LegacyThemeV0 {
    key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ThemeIncoming {
    Current(Envelope<ThemePayload>),
    LegacyV0(Envelope<LegacyThemeV0>),
    Unsupported { schema_version: u16, revision: u64 },
}

fn load_theme(source: String, incoming: ThemeIncoming) -> Loaded<ThemeSelection> {
    let migrated = match incoming {
        ThemeIncoming::Current(envelope) => envelope,
        ThemeIncoming::LegacyV0(envelope) => migrate_theme_v0(&envelope),
        ThemeIncoming::Unsupported {
            schema_version,
            revision,
        } => {
            return theme_fallback(
                source,
                DocumentError::UnsupportedSchema { schema_version },
                revision,
            );
        }
    };
    match validate_theme(&migrated.payload) {
        Ok(()) => Loaded::Valid(ThemeSelection { envelope: migrated }),
        Err(error) => theme_fallback(source, error, migrated.revision),
    }
}

fn theme_fallback(source: String, error: DocumentError, revision: u64) -> Loaded<ThemeSelection> {
    Loaded::Fallback {
        document: ThemeSelection::dark(),
        invalid: InvalidDocument {
            raw_source: source,
            error,
            revision,
        },
    }
}

fn migrate_theme_v0(envelope: &Envelope<LegacyThemeV0>) -> Envelope<ThemePayload> {
    let mode = ThemeMode::try_from_key(&envelope.payload.key).unwrap_or(ThemeMode::Dark);
    Envelope::new(THEME_SCHEMA, envelope.revision, ThemePayload { mode })
}

fn validate_theme(payload: &ThemePayload) -> Result<(), DocumentError> {
    let _ = ThemeMode::try_from_key(payload.mode.key())?;
    Ok(())
}

fn validate_fields(field: &'static str, fields: &VersionedFields) -> Result<(), DocumentError> {
    if fields.schema_version == 0 {
        return Err(DocumentError::InvalidValue {
            field,
            value: "schema version 0".to_owned(),
        });
    }
    let mut names = BTreeSet::new();
    for document_field in &fields.fields {
        validate_identifier(field, &document_field.name)?;
        if !names.insert(&document_field.name) {
            return Err(DocumentError::DuplicateValue {
                field,
                value: document_field.name.clone(),
            });
        }
        match &document_field.value {
            Scalar::Boolean(_) | Scalar::Integer(_) => {}
            Scalar::Text(value) => validate_text(field, value)?,
        }
    }
    Ok(())
}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), DocumentError> {
    let valid = !value.is_empty()
        && value.len() <= MAX_TEXT_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        });
    if valid {
        Ok(())
    } else {
        Err(DocumentError::InvalidIdentifier {
            field,
            value: value.to_owned(),
        })
    }
}

fn validate_display_text(field: &'static str, value: &str) -> Result<(), DocumentError> {
    if value.trim().is_empty() || value.len() > MAX_TEXT_BYTES {
        return Err(DocumentError::InvalidValue {
            field,
            value: value.to_owned(),
        });
    }
    Ok(())
}

fn validate_text(field: &'static str, value: &str) -> Result<(), DocumentError> {
    if value.len() > MAX_TEXT_BYTES || value.contains(['\n', '\r', '\0']) {
        return Err(DocumentError::InvalidValue {
            field,
            value: value.to_owned(),
        });
    }
    Ok(())
}

fn validate_local_date(value: &str) -> Result<(), DocumentError> {
    let bytes = value.as_bytes();
    let valid = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit());
    if valid {
        Ok(())
    } else {
        Err(DocumentError::InvalidValue {
            field: "fixed local date",
            value: value.to_owned(),
        })
    }
}

fn validate_time_zone(mode: &TimeZoneMode) -> Result<(), DocumentError> {
    let TimeZoneMode::Manual(value) = mode else {
        return Ok(());
    };
    let valid = !value.is_empty()
        && value.len() <= MAX_TEXT_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-' | b'+' | b'.')
        });
    if valid {
        Ok(())
    } else {
        Err(DocumentError::InvalidTimeZoneId {
            value: value.clone(),
        })
    }
}

fn invalid_number(field: &'static str, value: u16) -> DocumentError {
    DocumentError::InvalidValue {
        field,
        value: value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_defaults_validate_the_complete_document_values() {
        let layout = LayoutDocument::safe_default();
        assert_eq!(layout.schema_version(), LAYOUT_SCHEMA);
        assert_eq!(layout.widget_count(), 4);
        assert!(validate_layout(&layout.envelope.payload).is_ok());

        let saved_view = SavedViewDocument::safe_default();
        assert_eq!(saved_view.schema_version(), SAVED_VIEW_SCHEMA);
        assert_eq!(saved_view.name(), "Overview");
        assert!(validate_saved_view(&saved_view.envelope.payload).is_ok());

        let settings = SettingsDocument::safe_default();
        assert_eq!(settings.schema_version(), SETTINGS_SCHEMA);
        assert!(validate_settings(&settings.envelope.payload).is_ok());
        assert_eq!(settings.theme_selection().built_in_key(), "openmanic.dark");
    }

    #[test]
    fn layout_v0_migration_is_deterministic_and_retains_the_revision() {
        let incoming = || {
            LayoutIncoming::LegacyV0(Envelope::new(
                0,
                9,
                LegacyLayoutV0 {
                    widget_kind_ids: vec![
                        "openmanic.timeline.day".to_owned(),
                        "openmanic.focus.session".to_owned(),
                    ],
                },
            ))
        };
        let first = load_layout("legacy".to_owned(), incoming());
        let second = load_layout("legacy".to_owned(), incoming());
        assert_eq!(first, second);
        let layout = first.into_valid().expect("valid v0 layout must migrate");
        assert_eq!(layout.schema_version(), LAYOUT_SCHEMA);
        assert_eq!(layout.revision(), 9);
        assert_eq!(layout.widget_count(), 2);
    }

    #[test]
    fn invalid_layout_preserves_its_original_source_before_safe_fallback() {
        let source = "the original malformed layout document".to_owned();
        let widget = layout_widget(
            "same-instance",
            "openmanic.timeline.day",
            0,
            12,
            Height::Tall,
        );
        let result = load_layout(
            source.clone(),
            LayoutIncoming::Current(Envelope::new(
                LAYOUT_SCHEMA,
                42,
                LayoutPayload {
                    widgets: vec![widget.clone(), widget],
                },
            )),
        );
        let (document, invalid) = result
            .into_fallback()
            .expect("duplicate widget IDs must fall back");
        assert_eq!(document, LayoutDocument::safe_default());
        assert_eq!(invalid.raw_source, source);
        assert_eq!(invalid.revision, 42);
        assert!(matches!(
            invalid.error,
            DocumentError::DuplicateValue { .. }
        ));
    }

    #[test]
    fn saved_view_and_settings_invalid_values_take_deterministic_fallbacks() {
        let saved_view = load_saved_view(
            "inverted saved view".to_owned(),
            SavedViewIncoming::Current(Envelope::new(
                SAVED_VIEW_SCHEMA,
                3,
                SavedViewPayload {
                    public_id: "review".to_owned(),
                    name: "Review".to_owned(),
                    display_order: 0,
                    range: Range::Fixed {
                        start_local_date: "2026-07-20".to_owned(),
                        end_local_date: "2026-07-19".to_owned(),
                        time_zone_behavior: TimeZoneMode::Automatic,
                    },
                    grouping: "category".to_owned(),
                    filters: VersionedFields::empty(),
                    sort: "duration-descending".to_owned(),
                    widget_configuration: VersionedFields::empty(),
                },
            )),
        );
        assert!(matches!(
            saved_view,
            Loaded::Fallback {
                invalid: InvalidDocument {
                    error: DocumentError::InvertedDateRange { .. },
                    ..
                },
                ..
            }
        ));

        let settings = || {
            SettingsIncoming::LegacyV0(Envelope::new(
                0,
                7,
                LegacySettingsV0 {
                    first_launch_consent_revision: 2,
                    idle_threshold_seconds: 600,
                    theme_key: "system".to_owned(),
                },
            ))
        };
        let first = load_settings("settings".to_owned(), settings());
        let second = load_settings("settings".to_owned(), settings());
        assert_eq!(first, second);
        let settings = first
            .into_valid()
            .expect("valid legacy settings must migrate");
        assert_eq!(
            settings.theme_selection().built_in_key(),
            "openmanic.system"
        );
    }

    #[test]
    fn theme_selection_migration_and_scalar_documents_have_no_renderer_values() {
        let theme = || {
            ThemeIncoming::LegacyV0(Envelope::new(
                0,
                5,
                LegacyThemeV0 {
                    key: "light".to_owned(),
                },
            ))
        };
        assert_eq!(
            load_theme("theme".to_owned(), theme()),
            load_theme("theme".to_owned(), theme())
        );
        let scalar_fields = VersionedFields {
            schema_version: 1,
            fields: vec![DocumentField {
                name: "compact".to_owned(),
                value: Scalar::Boolean(true),
            }],
        };
        assert!(validate_fields("configuration", &scalar_fields).is_ok());

        let source = include_str!("documents.rs");
        let (production_source, _) = source
            .split_once("\n#[cfg(test)]\nmod tests")
            .expect("test module marker must remain present");
        for forbidden in ["style", "visuals", "fonts", concat!("e", "gui")] {
            assert!(
                !production_source.contains(forbidden),
                "forbidden renderer value: {forbidden}"
            );
        }
    }
}
