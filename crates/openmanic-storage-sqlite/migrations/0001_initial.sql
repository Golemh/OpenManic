-- OM-150: immutable first OpenManic SQLite schema.
-- All durable user/tracking tables are STRICT. Cross-row interval and schedule
-- overlap policies remain writer-owned because row CHECK constraints cannot
-- express them safely.

CREATE TABLE schema_migration (
    version INTEGER PRIMARY KEY CHECK(version > 0),
    checksum BLOB NOT NULL CHECK(length(checksum) = 8),
    applied_utc_us INTEGER NOT NULL,
    app_version TEXT NOT NULL CHECK(length(trim(app_version)) > 0)
) STRICT;

CREATE TABLE store_metadata (
    singleton_id INTEGER PRIMARY KEY CHECK(singleton_id = 1),
    store_id BLOB NOT NULL UNIQUE CHECK(length(store_id) = 16),
    data_revision INTEGER NOT NULL CHECK(data_revision >= 0),
    schema_version INTEGER NOT NULL CHECK(schema_version >= 0),
    created_utc_us INTEGER NOT NULL,
    last_opened_app_version TEXT NOT NULL CHECK(length(trim(last_opened_app_version)) > 0),
    last_clean_shutdown_utc_us INTEGER
) STRICT;

CREATE TABLE category (
    id INTEGER PRIMARY KEY,
    public_id BLOB NOT NULL UNIQUE CHECK(length(public_id) = 16),
    display_name TEXT NOT NULL CHECK(length(trim(display_name)) > 0),
    color_spec TEXT,
    icon_spec TEXT,
    description TEXT,
    productivity_class INTEGER CHECK(productivity_class IS NULL OR productivity_class >= 0),
    created_utc_us INTEGER NOT NULL,
    updated_utc_us INTEGER NOT NULL
) STRICT;

CREATE TABLE application (
    id INTEGER PRIMARY KEY,
    public_id BLOB NOT NULL UNIQUE CHECK(length(public_id) = 16),
    display_name TEXT NOT NULL CHECK(length(trim(display_name)) > 0),
    display_name_override TEXT,
    category_id INTEGER REFERENCES category(id) ON DELETE SET NULL,
    exclusion_policy INTEGER NOT NULL CHECK(exclusion_policy >= 0),
    first_seen_utc_us INTEGER NOT NULL,
    last_seen_utc_us INTEGER NOT NULL CHECK(last_seen_utc_us >= first_seen_utc_us),
    icon_digest BLOB
) STRICT;

CREATE TABLE application_identity (
    id INTEGER PRIMARY KEY,
    application_id INTEGER NOT NULL REFERENCES application(id) ON DELETE RESTRICT,
    platform INTEGER NOT NULL CHECK(platform >= 0),
    identity_kind INTEGER NOT NULL CHECK(identity_kind >= 0),
    normalized_value BLOB NOT NULL CHECK(length(normalized_value) > 0),
    original_value TEXT,
    confidence INTEGER NOT NULL CHECK(confidence >= 0),
    first_seen_utc_us INTEGER NOT NULL,
    last_seen_utc_us INTEGER NOT NULL CHECK(last_seen_utc_us >= first_seen_utc_us),
    UNIQUE(platform, identity_kind, normalized_value)
) STRICT;

CREATE TABLE window_title_text (
    id INTEGER PRIMARY KEY,
    text_hash BLOB NOT NULL CHECK(length(text_hash) > 0),
    title TEXT NOT NULL CHECK(
        length(CAST(title AS BLOB)) > 0
        AND length(CAST(title AS BLOB)) <= 2048
    ),
    UNIQUE(text_hash, title)
) STRICT;

CREATE TABLE tracker_run (
    id INTEGER PRIMARY KEY,
    public_id BLOB NOT NULL UNIQUE CHECK(length(public_id) = 16),
    started_utc_us INTEGER NOT NULL,
    ended_utc_us INTEGER CHECK(ended_utc_us IS NULL OR ended_utc_us >= started_utc_us),
    clean_end INTEGER NOT NULL CHECK(clean_end IN (0, 1)),
    platform_session_marker TEXT,
    adapter_version TEXT NOT NULL CHECK(length(trim(adapter_version)) > 0),
    end_evidence INTEGER CHECK(end_evidence IS NULL OR end_evidence >= 0)
) STRICT;

CREATE TABLE activity_interval (
    id INTEGER PRIMARY KEY,
    tracker_run_id INTEGER NOT NULL REFERENCES tracker_run(id) ON DELETE RESTRICT,
    start_utc_us INTEGER NOT NULL,
    end_utc_us INTEGER NOT NULL CHECK(end_utc_us > start_utc_us),
    state INTEGER NOT NULL CHECK(state BETWEEN 0 AND 6),
    cause INTEGER NOT NULL CHECK(cause BETWEEN 0 AND 14),
    application_id INTEGER REFERENCES application(id) ON DELETE RESTRICT,
    origin INTEGER NOT NULL CHECK(origin BETWEEN 0 AND 2),
    uncertainty_us INTEGER NOT NULL CHECK(uncertainty_us >= 0),
    source_revision INTEGER NOT NULL CHECK(source_revision >= 0),
    CHECK(
        (state = 0 AND application_id IS NOT NULL)
        OR (state <> 0 AND application_id IS NULL)
    ),
    CHECK(state <> 5 OR cause = 11)
) STRICT;

CREATE TABLE open_activity_checkpoint (
    singleton_id INTEGER PRIMARY KEY CHECK(singleton_id = 1),
    tracker_run_id INTEGER NOT NULL REFERENCES tracker_run(id) ON DELETE RESTRICT,
    open_start_utc_us INTEGER NOT NULL,
    last_confirmed_utc_us INTEGER NOT NULL CHECK(last_confirmed_utc_us >= open_start_utc_us),
    state INTEGER NOT NULL CHECK(state BETWEEN 0 AND 6),
    cause INTEGER NOT NULL CHECK(cause BETWEEN 0 AND 14),
    application_id INTEGER REFERENCES application(id) ON DELETE RESTRICT,
    platform_sequence INTEGER NOT NULL CHECK(platform_sequence >= 0),
    checkpoint_revision INTEGER NOT NULL CHECK(checkpoint_revision >= 0),
    CHECK(
        (state = 0 AND application_id IS NOT NULL)
        OR (state <> 0 AND application_id IS NULL)
    ),
    CHECK(state <> 5 OR cause = 11)
) STRICT;

CREATE TABLE window_title_span (
    id INTEGER PRIMARY KEY,
    application_id INTEGER NOT NULL REFERENCES application(id) ON DELETE RESTRICT,
    tracker_run_id INTEGER NOT NULL REFERENCES tracker_run(id) ON DELETE RESTRICT,
    title_text_id INTEGER NOT NULL REFERENCES window_title_text(id) ON DELETE RESTRICT,
    start_utc_us INTEGER NOT NULL,
    end_utc_us INTEGER NOT NULL CHECK(end_utc_us > start_utc_us),
    source_revision INTEGER NOT NULL CHECK(source_revision >= 0)
) STRICT;

CREATE TABLE focus_session (
    id INTEGER PRIMARY KEY,
    public_id BLOB NOT NULL UNIQUE CHECK(length(public_id) = 16),
    kind INTEGER NOT NULL CHECK(kind IN (0, 1)),
    state INTEGER NOT NULL CHECK(state BETWEEN 0 AND 4),
    label TEXT CHECK(label IS NULL OR length(trim(label)) > 0),
    category_id INTEGER REFERENCES category(id) ON DELETE SET NULL,
    planned_start_utc_us INTEGER,
    planned_end_utc_us INTEGER,
    intended_duration_us INTEGER NOT NULL CHECK(intended_duration_us > 0),
    actual_start_utc_us INTEGER,
    deadline_utc_us INTEGER,
    paused_remaining_us INTEGER,
    completed_utc_us INTEGER,
    cancelled_utc_us INTEGER,
    revision INTEGER NOT NULL CHECK(revision >= 0),
    CHECK(
        planned_start_utc_us IS NULL
        OR planned_end_utc_us IS NULL
        OR planned_end_utc_us > planned_start_utc_us
    ),
    CHECK(
        (state = 0
            AND actual_start_utc_us IS NULL
            AND deadline_utc_us IS NULL
            AND paused_remaining_us IS NULL
            AND completed_utc_us IS NULL
            AND cancelled_utc_us IS NULL)
        OR (state = 1
            AND actual_start_utc_us IS NOT NULL
            AND deadline_utc_us IS NOT NULL
            AND paused_remaining_us IS NULL
            AND completed_utc_us IS NULL
            AND cancelled_utc_us IS NULL)
        OR (state = 2
            AND actual_start_utc_us IS NOT NULL
            AND deadline_utc_us IS NULL
            AND paused_remaining_us > 0
            AND completed_utc_us IS NULL
            AND cancelled_utc_us IS NULL)
        OR (state = 3
            AND actual_start_utc_us IS NOT NULL
            AND deadline_utc_us IS NULL
            AND paused_remaining_us IS NULL
            AND completed_utc_us IS NOT NULL
            AND cancelled_utc_us IS NULL)
        OR (state = 4
            AND actual_start_utc_us IS NOT NULL
            AND deadline_utc_us IS NULL
            AND paused_remaining_us IS NULL
            AND completed_utc_us IS NULL
            AND cancelled_utc_us IS NOT NULL)
    )
) STRICT;

CREATE TABLE one_time_schedule (
    id INTEGER PRIMARY KEY,
    public_id BLOB NOT NULL UNIQUE CHECK(length(public_id) = 16),
    label TEXT NOT NULL CHECK(length(trim(label)) > 0),
    category_id INTEGER REFERENCES category(id) ON DELETE SET NULL,
    start_utc_us INTEGER NOT NULL,
    end_utc_us INTEGER NOT NULL CHECK(end_utc_us > start_utc_us),
    created_zone_id TEXT NOT NULL CHECK(length(trim(created_zone_id)) > 0),
    created_utc_us INTEGER NOT NULL,
    updated_utc_us INTEGER NOT NULL CHECK(updated_utc_us >= created_utc_us),
    revision INTEGER NOT NULL CHECK(revision >= 0)
) STRICT;

CREATE TABLE schedule_series (
    id INTEGER PRIMARY KEY,
    public_id BLOB NOT NULL UNIQUE CHECK(length(public_id) = 16),
    created_utc_us INTEGER NOT NULL,
    deleted_utc_us INTEGER,
    revision INTEGER NOT NULL CHECK(revision >= 0)
) STRICT;

CREATE TABLE schedule_rule_segment (
    id INTEGER PRIMARY KEY,
    series_id INTEGER NOT NULL REFERENCES schedule_series(id) ON DELETE RESTRICT,
    effective_start_date INTEGER NOT NULL,
    effective_end_date INTEGER CHECK(
        effective_end_date IS NULL OR effective_end_date >= effective_start_date
    ),
    weekday_mask INTEGER NOT NULL CHECK(weekday_mask BETWEEN 1 AND 127),
    start_second_of_day INTEGER NOT NULL CHECK(start_second_of_day BETWEEN 0 AND 86399),
    end_second_of_day INTEGER NOT NULL CHECK(end_second_of_day BETWEEN 0 AND 86399),
    end_day_offset INTEGER NOT NULL CHECK(end_day_offset IN (0, 1)),
    time_zone_id TEXT NOT NULL CHECK(length(trim(time_zone_id)) > 0),
    label TEXT NOT NULL CHECK(length(trim(label)) > 0),
    category_id INTEGER REFERENCES category(id) ON DELETE SET NULL,
    created_utc_us INTEGER NOT NULL,
    revision INTEGER NOT NULL CHECK(revision >= 0),
    CHECK(
        (end_day_offset = 0 AND end_second_of_day > start_second_of_day)
        OR (end_day_offset = 1 AND end_second_of_day < start_second_of_day)
    )
) STRICT;

CREATE TABLE schedule_exception (
    id INTEGER PRIMARY KEY,
    series_id INTEGER NOT NULL REFERENCES schedule_series(id) ON DELETE RESTRICT,
    anchor_local_date INTEGER NOT NULL,
    kind INTEGER NOT NULL CHECK(kind IN (0, 1)),
    override_start_utc_us INTEGER,
    override_end_utc_us INTEGER,
    label_override TEXT CHECK(label_override IS NULL OR length(trim(label_override)) > 0),
    category_id_override INTEGER REFERENCES category(id) ON DELETE SET NULL,
    resolved_zone_id TEXT,
    revision INTEGER NOT NULL CHECK(revision >= 0),
    UNIQUE(series_id, anchor_local_date),
    CHECK(
        (kind = 0
            AND override_start_utc_us IS NULL
            AND override_end_utc_us IS NULL)
        OR (kind = 1
            AND override_start_utc_us IS NOT NULL
            AND override_end_utc_us IS NOT NULL
            AND override_end_utc_us > override_start_utc_us)
    )
) STRICT;

CREATE TABLE dashboard_layout (
    id INTEGER PRIMARY KEY CHECK(id = 1),
    schema_version INTEGER NOT NULL CHECK(schema_version > 0),
    revision INTEGER NOT NULL CHECK(revision >= 0),
    document_json TEXT NOT NULL CHECK(length(trim(document_json)) > 0),
    updated_utc_us INTEGER NOT NULL
) STRICT;

CREATE TABLE saved_overview_view (
    id INTEGER PRIMARY KEY,
    public_id BLOB NOT NULL UNIQUE CHECK(length(public_id) = 16),
    name TEXT NOT NULL CHECK(length(trim(name)) > 0),
    display_order INTEGER NOT NULL,
    schema_version INTEGER NOT NULL CHECK(schema_version > 0),
    revision INTEGER NOT NULL CHECK(revision >= 0),
    definition_json TEXT NOT NULL CHECK(length(trim(definition_json)) > 0),
    created_utc_us INTEGER NOT NULL,
    updated_utc_us INTEGER NOT NULL CHECK(updated_utc_us >= created_utc_us)
) STRICT;

CREATE TABLE user_settings (
    singleton_id INTEGER PRIMARY KEY CHECK(singleton_id = 1),
    schema_version INTEGER NOT NULL CHECK(schema_version > 0),
    first_launch_consent_revision INTEGER NOT NULL CHECK(first_launch_consent_revision >= 0),
    start_tracking_automatically INTEGER NOT NULL CHECK(start_tracking_automatically IN (0, 1)),
    start_at_login INTEGER NOT NULL CHECK(start_at_login IN (0, 1)),
    close_to_tray INTEGER NOT NULL CHECK(close_to_tray IN (0, 1)),
    idle_threshold_seconds INTEGER NOT NULL CHECK(idle_threshold_seconds > 0),
    idle_policy INTEGER NOT NULL CHECK(idle_policy > 0),
    collect_window_titles INTEGER NOT NULL CHECK(collect_window_titles IN (0, 1)),
    time_zone_mode INTEGER NOT NULL CHECK(time_zone_mode IN (0, 1)),
    manual_time_zone_id TEXT,
    theme_mode INTEGER NOT NULL CHECK(theme_mode BETWEEN 0 AND 2),
    density INTEGER NOT NULL CHECK(density > 0),
    notifications_enabled INTEGER NOT NULL CHECK(notifications_enabled IN (0, 1)),
    focus_sounds_enabled INTEGER NOT NULL CHECK(focus_sounds_enabled IN (0, 1)),
    tray_explanation_acknowledged INTEGER NOT NULL CHECK(tray_explanation_acknowledged IN (0, 1)),
    revision INTEGER NOT NULL CHECK(revision >= 0),
    updated_utc_us INTEGER NOT NULL,
    CHECK(
        (time_zone_mode = 0 AND manual_time_zone_id IS NULL)
        OR (time_zone_mode = 1 AND length(trim(manual_time_zone_id)) > 0)
    )
) STRICT;

CREATE TABLE job_record (
    id INTEGER PRIMARY KEY,
    public_id BLOB NOT NULL UNIQUE CHECK(length(public_id) = 16),
    kind INTEGER NOT NULL CHECK(kind >= 0),
    state INTEGER NOT NULL CHECK(state >= 0),
    progress_current INTEGER NOT NULL CHECK(progress_current >= 0),
    progress_total INTEGER CHECK(progress_total IS NULL OR progress_total >= progress_current),
    source_reference TEXT,
    destination_reference TEXT,
    safe_checkpoint TEXT,
    error_summary TEXT,
    created_utc_us INTEGER NOT NULL,
    started_utc_us INTEGER,
    completed_utc_us INTEGER,
    CHECK(started_utc_us IS NULL OR started_utc_us >= created_utc_us),
    CHECK(completed_utc_us IS NULL OR started_utc_us IS NULL OR completed_utc_us >= started_utc_us)
) STRICT;

CREATE TABLE import_batch (
    id INTEGER PRIMARY KEY,
    public_id BLOB NOT NULL UNIQUE CHECK(length(public_id) = 16),
    file_fingerprint BLOB NOT NULL CHECK(length(file_fingerprint) > 0),
    format_schema_version INTEGER NOT NULL CHECK(format_schema_version > 0),
    state INTEGER NOT NULL CHECK(state >= 0),
    parsed_count INTEGER NOT NULL CHECK(parsed_count >= 0),
    accepted_count INTEGER NOT NULL CHECK(accepted_count BETWEEN 0 AND parsed_count),
    rejected_count INTEGER NOT NULL CHECK(rejected_count BETWEEN 0 AND parsed_count),
    committed_count INTEGER NOT NULL CHECK(committed_count BETWEEN 0 AND accepted_count),
    created_utc_us INTEGER NOT NULL,
    completed_utc_us INTEGER,
    error_report_reference TEXT
) STRICT;

CREATE TABLE import_error (
    id INTEGER PRIMARY KEY,
    import_batch_id INTEGER NOT NULL REFERENCES import_batch(id) ON DELETE RESTRICT,
    source_line INTEGER NOT NULL CHECK(source_line > 0),
    field_name TEXT,
    error_code TEXT NOT NULL CHECK(length(trim(error_code)) > 0),
    summary TEXT NOT NULL CHECK(length(trim(summary)) > 0)
) STRICT;

CREATE INDEX idx_activity_interval_start
    ON activity_interval(start_utc_us);
CREATE INDEX idx_activity_interval_application_start
    ON activity_interval(application_id, start_utc_us);
CREATE INDEX idx_window_title_text_hash
    ON window_title_text(text_hash);
CREATE INDEX idx_window_title_span_application_start
    ON window_title_span(application_id, start_utc_us);
CREATE INDEX idx_application_category_display_name
    ON application(category_id, display_name);
CREATE INDEX idx_schedule_rule_segment_series_effective_dates
    ON schedule_rule_segment(series_id, effective_start_date, effective_end_date);
CREATE INDEX idx_focus_session_actual_start
    ON focus_session(actual_start_utc_us);
CREATE INDEX idx_saved_overview_view_display_order
    ON saved_overview_view(display_order);
CREATE UNIQUE INDEX idx_focus_session_at_most_one_active_or_paused
    ON focus_session((1))
    WHERE state IN (1, 2);
