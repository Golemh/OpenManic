//! Synthetic, deterministic datasets used by the performance fixture generator.
//!
//! This module owns only in-memory scenario records. Filesystem materialization
//! and command-line selection remain outside this library so benchmarks can use
//! the same records without depending on a storage format.

use crate::{ManualClock, MonotonicTicks, SeededPrng, UtcMicros};

const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
const MICROSECONDS_PER_SECOND_I64: i64 = 1_000_000;
const TITLE_RETAINED_LIMIT: usize = 16;
const DENSE_INTERVAL_COUNT: usize = 10_000;
const LARGE_LIST_COUNT: usize = 1_000;

/// Frozen performance-fixture scenarios named in `fixtures/performance/generator-config.toml`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scenario {
    /// Realistic work applications and category durations.
    NormalWorkday,
    /// More than the required ten thousand raw activity intervals.
    Dense10000IntervalRange,
    /// Independent category, state, and application segment boundaries.
    ThreeSegmentedBands,
    /// Rapid foreground and same-application window transitions.
    RapidABa,
    /// Adjacent, overnight, DST, and recurrence-exception schedules.
    ScheduleDstOvernight,
    /// Activity, focus, and schedule data sharing time ranges.
    SimultaneousOverlays,
    /// One thousand stable application and category entries.
    LargeApplicationCategoryLists,
    /// Browser title observations at the three required rates.
    TitleChurn1050100Hz,
    /// Tracking, import, and Overview jobs with separate identities.
    ConcurrentJobs,
    /// Bounded queue and coalescing evidence for a stalled UI.
    SlowedUi,
}

impl Scenario {
    /// Returns every supported scenario in frozen configuration order.
    #[must_use]
    pub const fn all() -> [Self; 10] {
        [
            Self::NormalWorkday,
            Self::Dense10000IntervalRange,
            Self::ThreeSegmentedBands,
            Self::RapidABa,
            Self::ScheduleDstOvernight,
            Self::SimultaneousOverlays,
            Self::LargeApplicationCategoryLists,
            Self::TitleChurn1050100Hz,
            Self::ConcurrentJobs,
            Self::SlowedUi,
        ]
    }

    /// Returns the matching frozen configuration name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::NormalWorkday => "normal-workday",
            Self::Dense10000IntervalRange => "dense-10000-interval-range",
            Self::ThreeSegmentedBands => "three-segmented-bands",
            Self::RapidABa => "rapid-a-b-a",
            Self::ScheduleDstOvernight => "schedule-dst-overnight",
            Self::SimultaneousOverlays => "simultaneous-overlays",
            Self::LargeApplicationCategoryLists => "large-application-category-lists",
            Self::TitleChurn1050100Hz => "title-churn-10-50-100-hz",
            Self::ConcurrentJobs => "concurrent-jobs",
            Self::SlowedUi => "slowed-ui",
        }
    }

    /// Builds synthetic data using only the supplied deterministic seed.
    #[must_use]
    pub fn generate(self, seed: u64) -> ScenarioFixture {
        let mut generator = SeededPrng::new(seed);
        let mut clock = ManualClock::new(
            UtcMicros::new(1_710_000_000_000_000),
            MonotonicTicks::new(0),
        );
        let mut fixture = ScenarioFixture::empty(self, seed, clock.utc_micros());
        match self {
            Self::NormalWorkday => normal_workday(&mut fixture, &mut generator, &mut clock),
            Self::Dense10000IntervalRange => dense_range(&mut fixture, &mut clock),
            Self::ThreeSegmentedBands => segmented_bands(&mut fixture, &mut clock),
            Self::RapidABa => rapid_switches(&mut fixture, &mut clock),
            Self::ScheduleDstOvernight => schedules(&mut fixture, &mut clock),
            Self::SimultaneousOverlays => overlays(&mut fixture, &mut clock),
            Self::LargeApplicationCategoryLists => large_lists(&mut fixture),
            Self::TitleChurn1050100Hz => title_churn(&mut fixture, &mut generator),
            Self::ConcurrentJobs => concurrent_jobs(&mut fixture),
            Self::SlowedUi => slowed_ui(&mut fixture),
        }
        fixture.metadata.raw_interval_count = fixture.activity_intervals.len();
        fixture.metadata.generated_at_utc_micros = clock.utc_micros();
        fixture.metadata.generated_at_monotonic_ticks = clock.monotonic_ticks();
        fixture
    }
}

/// A complete synthetic record set for one fixture scenario.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioFixture {
    /// Scenario identity.
    pub scenario: Scenario,
    /// Input seed used for deterministic variation.
    pub seed: u64,
    /// Counts and bounded-behavior evidence before aggregation.
    pub metadata: ScenarioMetadata,
    /// Raw activity intervals, sorted by stable ID and start time.
    pub activity_intervals: Vec<RawInterval>,
    /// Category-band segments independent of activity boundaries.
    pub category_band: Vec<BandSegment>,
    /// State-band segments independent of activity boundaries.
    pub state_band: Vec<BandSegment>,
    /// Application-band segments independent of activity boundaries.
    pub application_band: Vec<BandSegment>,
    /// Schedule occurrences and boundary markers.
    pub schedules: Vec<ScheduleOccurrence>,
    /// Focus and schedule overlay rows.
    pub overlays: Vec<Overlay>,
    /// Stable application names.
    pub applications: Vec<String>,
    /// Stable category names.
    pub categories: Vec<String>,
    /// Title-rate summaries retaining only a bounded tail of titles.
    pub title_rates: Vec<TitleRate>,
    /// Independent concurrent job identities.
    pub jobs: Vec<JobRecord>,
    /// Queue and replacement evidence for the stalled UI.
    pub slowed_ui: Option<SlowedUiMetadata>,
}

impl ScenarioFixture {
    fn empty(scenario: Scenario, seed: u64, utc: UtcMicros) -> Self {
        Self {
            scenario,
            seed,
            metadata: ScenarioMetadata::new(utc),
            activity_intervals: Vec::new(),
            category_band: Vec::new(),
            state_band: Vec::new(),
            application_band: Vec::new(),
            schedules: Vec::new(),
            overlays: Vec::new(),
            applications: Vec::new(),
            categories: Vec::new(),
            title_rates: Vec::new(),
            jobs: Vec::new(),
            slowed_ui: None,
        }
    }
}

/// Metadata exposed before any projection or paint-level aggregation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioMetadata {
    /// Raw interval count before aggregation.
    pub raw_interval_count: usize,
    /// Deterministic generation timestamp.
    pub generated_at_utc_micros: UtcMicros,
    /// Deterministic generation tick count.
    pub generated_at_monotonic_ticks: MonotonicTicks,
}

impl ScenarioMetadata {
    fn new(utc: UtcMicros) -> Self {
        Self {
            raw_interval_count: 0,
            generated_at_utc_micros: utc,
            generated_at_monotonic_ticks: MonotonicTicks::new(0),
        }
    }
}

/// A raw activity span with stable synthetic identifiers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawInterval {
    /// Stable interval identity.
    pub id: u64,
    /// Inclusive UTC start time.
    pub start: UtcMicros,
    /// Exclusive UTC end time.
    pub end: UtcMicros,
    /// Synthetic foreground application identity.
    pub application_id: String,
    /// Synthetic category identity.
    pub category_id: String,
    /// Synthetic activity state label.
    pub state: String,
    /// Synthetic window identity.
    pub window_id: String,
}
/// A display-band segment whose boundaries need not align with activity data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BandSegment {
    /// Stable segment identity.
    pub id: u64,
    /// Inclusive UTC start time.
    pub start: UtcMicros,
    /// Exclusive UTC end time.
    pub end: UtcMicros,
    /// Synthetic segment label.
    pub label: String,
}
/// A synthetic schedule occurrence including special-calendar markers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleOccurrence {
    /// Stable occurrence identity.
    pub id: u64,
    /// Inclusive UTC start time.
    pub start: UtcMicros,
    /// Exclusive UTC end time.
    pub end: UtcMicros,
    /// Calendar edge case represented by this occurrence.
    pub marker: ScheduleMarker,
    /// Explicit synthetic named zone used to interpret the local evidence.
    pub zone_name: String,
    /// Synthetic local start date-time including its UTC offset.
    pub local_start: String,
    /// Synthetic local end date-time including its UTC offset.
    pub local_end: String,
}
/// Schedule characteristics deliberately represented in the fixture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleMarker {
    /// Back-to-back schedule brackets.
    Adjacent,
    /// A bracket crossing a local-day boundary.
    Overnight,
    /// A bracket on a daylight-saving transition date.
    DstBoundary,
    /// A recurring schedule occurrence intentionally omitted or changed.
    RecurrenceException,
}
/// An activity-adjacent overlay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Overlay {
    /// Stable overlay identity.
    pub id: u64,
    /// Inclusive UTC start time.
    pub start: UtcMicros,
    /// Exclusive UTC end time.
    pub end: UtcMicros,
    /// Layer represented by the overlay.
    pub kind: OverlayKind,
}
/// Overlay layer identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OverlayKind {
    /// Activity timeline layer.
    Activity,
    /// Focus-session layer.
    Focus,
    /// Schedule-occurrence layer.
    Schedule,
}
/// A rate summary retaining only a bounded recent title tail.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TitleRate {
    /// Requested synthetic title changes per second.
    pub changes_per_second: u16,
    /// Duration over which changes are counted.
    pub duration_seconds: u16,
    /// Exact number of generated title changes.
    pub change_count: usize,
    /// Bounded tail of generated titles retained for inspection.
    pub retained_titles: Vec<String>,
}
/// A concurrent synthetic workload identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobRecord {
    /// Stable concurrent job identity.
    pub job_id: u64,
    /// Workload category.
    pub kind: JobKind,
    /// Ordered event within its job stream.
    pub sequence: u32,
}
/// Job category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobKind {
    /// Foreground tracking write.
    TrackingWrite,
    /// Bulk import batch.
    ImportBatch,
    /// Overview projection work.
    OverviewProjection,
}
/// Bounded stalled-UI queue behavior.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlowedUiMetadata {
    /// Fixed snapshot queue capacity.
    pub queue_capacity: usize,
    /// Highest observed queue depth.
    pub high_water_mark: usize,
    /// Number of snapshots sent while the UI was slow.
    pub submitted_snapshots: usize,
    /// Identity of the snapshot ultimately delivered.
    pub delivered_snapshot_id: u64,
    /// Number of older snapshots replaced by a newer one.
    pub coalesced_snapshots: usize,
}

fn normal_workday(fixture: &mut ScenarioFixture, random: &mut SeededPrng, clock: &mut ManualClock) {
    fixture.activity_intervals = Vec::with_capacity(12);
    for index in 0..10_u64 {
        let duration = 20_u64 + random.next_u64() % 41;
        push_interval(
            fixture,
            clock,
            index,
            duration * 60,
            ActivityLabels::new(
                format!("app-{}", index % 4),
                format!("category-{}", index % 3),
                "active",
                format!("window-{index}"),
            ),
        );
    }
}
fn dense_range(fixture: &mut ScenarioFixture, clock: &mut ManualClock) {
    fixture.activity_intervals = Vec::with_capacity(DENSE_INTERVAL_COUNT);
    for index in 0..DENSE_INTERVAL_COUNT as u64 {
        push_interval(
            fixture,
            clock,
            index,
            1,
            ActivityLabels::new(
                format!("app-{}", index % 7),
                format!("category-{}", index % 5),
                "active",
                format!("window-{}", index % 11),
            ),
        );
    }
}
fn segmented_bands(fixture: &mut ScenarioFixture, clock: &mut ManualClock) {
    let start = clock.utc_micros();
    let end = after_seconds(start, 120);
    fixture.category_band =
        band_with_boundaries(start, end, 100, ["planning", "coding", "review"], &[20, 70]);
    fixture.state_band = band_with_boundaries(
        start,
        end,
        200,
        ["active", "idle", "active", "away"],
        &[35, 90, 105],
    );
    fixture.application_band = band_with_boundaries(
        start,
        end,
        300,
        ["editor", "browser", "terminal", "editor", "chat"],
        &[10, 40, 80, 110],
    );
    clock.set_utc_micros(end);
    clock.advance_monotonic_ticks(120);
}
fn rapid_switches(fixture: &mut ScenarioFixture, clock: &mut ManualClock) {
    for (index, (app, window)) in [
        ("app-a", "window-1"),
        ("app-b", "window-2"),
        ("app-a", "window-3"),
        ("app-a", "window-4"),
    ]
    .into_iter()
    .enumerate()
    {
        push_interval(
            fixture,
            clock,
            index as u64,
            1,
            ActivityLabels::new(
                app.to_owned(),
                "category-rapid".to_owned(),
                "active",
                window.to_owned(),
            ),
        );
    }
}
fn schedules(fixture: &mut ScenarioFixture, clock: &mut ManualClock) {
    let first_start = clock.utc_micros();
    let adjacent_end = after_seconds(first_start, 30 * 60);
    let overnight_end = after_seconds(adjacent_end, 2 * 60 * 60);
    let dst_start = after_seconds(overnight_end, 60 * 60);
    let dst_end = after_seconds(dst_start, 60 * 60);
    let exception_end = after_seconds(dst_end, 45 * 60);
    fixture.schedules = vec![
        schedule(
            0,
            first_start,
            adjacent_end,
            ScheduleMarker::Adjacent,
            "Fixture/Standard",
            "2026-02-02T09:00:00+00:00",
            "2026-02-02T09:30:00+00:00",
        ),
        schedule(
            1,
            adjacent_end,
            overnight_end,
            ScheduleMarker::Adjacent,
            "Fixture/Standard",
            "2026-02-02T09:30:00+00:00",
            "2026-02-02T11:30:00+00:00",
        ),
        schedule(
            2,
            adjacent_end,
            overnight_end,
            ScheduleMarker::Overnight,
            "Fixture/Standard",
            "2026-02-02T23:30:00+00:00",
            "2026-02-03T01:30:00+00:00",
        ),
        schedule(
            3,
            dst_start,
            dst_end,
            ScheduleMarker::DstBoundary,
            "Fixture/Example-DST",
            "2026-03-08T01:30:00-05:00",
            "2026-03-08T03:30:00-04:00",
        ),
        schedule(
            4,
            dst_end,
            exception_end,
            ScheduleMarker::RecurrenceException,
            "Fixture/Standard",
            "2026-03-10T09:00:00+00:00",
            "2026-03-10T09:45:00+00:00",
        ),
    ];
    clock.set_utc_micros(exception_end);
    clock.advance_monotonic_ticks(5 * 60 * 60 + 15 * 60);
}
fn overlays(fixture: &mut ScenarioFixture, clock: &mut ManualClock) {
    let start = clock.utc_micros();
    clock.advance_utc_micros(30 * MICROSECONDS_PER_SECOND);
    let end = clock.utc_micros();
    fixture.overlays = [
        OverlayKind::Activity,
        OverlayKind::Focus,
        OverlayKind::Schedule,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, kind)| Overlay {
        id: index as u64,
        start,
        end,
        kind,
    })
    .collect();
}
fn large_lists(fixture: &mut ScenarioFixture) {
    fixture.applications = names("application", LARGE_LIST_COUNT);
    fixture.categories = names("category", LARGE_LIST_COUNT);
}
fn title_churn(fixture: &mut ScenarioFixture, random: &mut SeededPrng) {
    fixture.title_rates = [10_u16, 50, 100]
        .into_iter()
        .map(|rate| {
            let change_count = usize::from(rate) * 2;
            let retained_start = change_count.saturating_sub(TITLE_RETAINED_LIMIT);
            let retained_titles = (retained_start..change_count)
                .map(|index| format!("title-{rate}-{index}-{:x}", random.next_u64()))
                .collect();
            TitleRate {
                changes_per_second: rate,
                duration_seconds: 2,
                change_count,
                retained_titles,
            }
        })
        .collect();
}
fn concurrent_jobs(fixture: &mut ScenarioFixture) {
    fixture.jobs = Vec::with_capacity(9);
    for sequence in 1..=3 {
        fixture.jobs.extend([
            JobRecord {
                job_id: 10,
                kind: JobKind::TrackingWrite,
                sequence,
            },
            JobRecord {
                job_id: 20,
                kind: JobKind::ImportBatch,
                sequence,
            },
            JobRecord {
                job_id: 30,
                kind: JobKind::OverviewProjection,
                sequence,
            },
        ]);
    }
}
fn slowed_ui(fixture: &mut ScenarioFixture) {
    fixture.slowed_ui = Some(SlowedUiMetadata {
        queue_capacity: 8,
        high_water_mark: 8,
        submitted_snapshots: 24,
        delivered_snapshot_id: 24,
        coalesced_snapshots: 16,
    });
}

fn push_interval(
    fixture: &mut ScenarioFixture,
    clock: &mut ManualClock,
    id: u64,
    seconds: u64,
    labels: ActivityLabels,
) {
    let start = clock.utc_micros();
    clock.advance_utc_micros(seconds * MICROSECONDS_PER_SECOND);
    clock.advance_monotonic_ticks(seconds);
    fixture.activity_intervals.push(RawInterval {
        id,
        start,
        end: clock.utc_micros(),
        application_id: labels.application_id,
        category_id: labels.category_id,
        state: labels.state,
        window_id: labels.window_id,
    });
}
struct ActivityLabels {
    application_id: String,
    category_id: String,
    state: String,
    window_id: String,
}

impl ActivityLabels {
    fn new(application_id: String, category_id: String, state: &str, window_id: String) -> Self {
        Self {
            application_id,
            category_id,
            state: state.to_owned(),
            window_id,
        }
    }
}
fn band_with_boundaries(
    start: UtcMicros,
    end: UtcMicros,
    first_id: u64,
    labels: impl IntoIterator<Item = &'static str>,
    boundary_seconds: &[u64],
) -> Vec<BandSegment> {
    let labels = labels.into_iter().collect::<Vec<_>>();
    let mut boundaries = Vec::with_capacity(labels.len() + 1);
    boundaries.push(start);
    boundaries.extend(
        boundary_seconds
            .iter()
            .map(|seconds| after_seconds(start, *seconds)),
    );
    boundaries.push(end);
    labels
        .into_iter()
        .enumerate()
        .map(|(index, label)| BandSegment {
            id: first_id + index as u64,
            start: boundaries[index],
            end: boundaries[index + 1],
            label: label.to_owned(),
        })
        .collect()
}
fn after_seconds(start: UtcMicros, seconds: u64) -> UtcMicros {
    let seconds = i64::try_from(seconds).unwrap_or(i64::MAX);
    UtcMicros::new(
        start
            .get()
            .saturating_add(seconds.saturating_mul(MICROSECONDS_PER_SECOND_I64)),
    )
}
fn schedule(
    id: u64,
    start: UtcMicros,
    end: UtcMicros,
    marker: ScheduleMarker,
    zone_name: &str,
    local_start: &str,
    local_end: &str,
) -> ScheduleOccurrence {
    ScheduleOccurrence {
        id,
        start,
        end,
        marker,
        zone_name: zone_name.to_owned(),
        local_start: local_start.to_owned(),
        local_end: local_end.to_owned(),
    }
}
fn names(prefix: &str, count: usize) -> Vec<String> {
    (0..count)
        .map(|index| format!("{prefix}-{index:04}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_match_frozen_configuration() {
        assert_eq!(
            Scenario::all().map(Scenario::name),
            [
                "normal-workday",
                "dense-10000-interval-range",
                "three-segmented-bands",
                "rapid-a-b-a",
                "schedule-dst-overnight",
                "simultaneous-overlays",
                "large-application-category-lists",
                "title-churn-10-50-100-hz",
                "concurrent-jobs",
                "slowed-ui"
            ]
        );
    }
    #[test]
    fn activity_band_and_switch_scenarios_have_concrete_records() {
        let seed = 7;
        assert_eq!(
            Scenario::NormalWorkday
                .generate(seed)
                .activity_intervals
                .len(),
            10
        );
        assert!(
            Scenario::Dense10000IntervalRange
                .generate(seed)
                .metadata
                .raw_interval_count
                >= DENSE_INTERVAL_COUNT
        );
        let bands = Scenario::ThreeSegmentedBands.generate(seed);
        let category_boundaries = boundaries(&bands.category_band);
        let state_boundaries = boundaries(&bands.state_band);
        let application_boundaries = boundaries(&bands.application_band);
        assert_eq!(category_boundaries.first(), state_boundaries.first());
        assert_eq!(category_boundaries.last(), state_boundaries.last());
        assert_eq!(category_boundaries.first(), application_boundaries.first());
        assert_eq!(category_boundaries.last(), application_boundaries.last());
        assert_ne!(category_boundaries, state_boundaries);
        assert_ne!(state_boundaries, application_boundaries);
        let rapid = Scenario::RapidABa.generate(seed);
        assert_eq!(
            rapid.activity_intervals[0].application_id,
            rapid.activity_intervals[2].application_id
        );
        assert_ne!(
            rapid.activity_intervals[2].window_id,
            rapid.activity_intervals[3].window_id
        );
    }
    #[test]
    fn schedule_scenario_has_concrete_calendar_evidence() {
        let schedule = Scenario::ScheduleDstOvernight.generate(7);
        let adjacent = schedule
            .schedules
            .iter()
            .filter(|item| item.marker == ScheduleMarker::Adjacent)
            .collect::<Vec<_>>();
        assert_eq!(adjacent.len(), 2);
        assert_eq!(adjacent[0].end, adjacent[1].start);
        let overnight = schedule
            .schedules
            .iter()
            .find(|item| item.marker == ScheduleMarker::Overnight);
        assert_eq!(
            overnight.map(|item| item.local_start.starts_with("2026-02-02")),
            Some(true)
        );
        assert_eq!(
            overnight.map(|item| item.local_end.starts_with("2026-02-03")),
            Some(true)
        );
        let dst = schedule
            .schedules
            .iter()
            .find(|item| item.marker == ScheduleMarker::DstBoundary);
        assert_eq!(
            dst.map(|item| item.zone_name.as_str()),
            Some("Fixture/Example-DST")
        );
        assert_eq!(
            dst.map(|item| item.end.get() - item.start.get()),
            Some(60 * 60 * MICROSECONDS_PER_SECOND_I64)
        );
        assert_eq!(
            dst.map(|item| item.local_start.as_str()),
            Some("2026-03-08T01:30:00-05:00")
        );
        assert_eq!(
            dst.map(|item| item.local_end.as_str()),
            Some("2026-03-08T03:30:00-04:00")
        );
        assert!(
            schedule
                .schedules
                .iter()
                .any(|item| item.marker == ScheduleMarker::RecurrenceException)
        );
    }
    #[test]
    fn overlay_and_large_list_scenarios_have_required_scale() {
        let seed = 7;
        assert_eq!(
            Scenario::SimultaneousOverlays.generate(seed).overlays.len(),
            3
        );
        let lists = Scenario::LargeApplicationCategoryLists.generate(seed);
        assert_eq!(
            (lists.applications.len(), lists.categories.len()),
            (LARGE_LIST_COUNT, LARGE_LIST_COUNT)
        );
    }
    #[test]
    fn title_rates_are_exact_and_retention_is_bounded() {
        let titles = Scenario::TitleChurn1050100Hz.generate(9).title_rates;
        assert_eq!(
            titles
                .iter()
                .map(|item| (item.changes_per_second, item.change_count))
                .collect::<Vec<_>>(),
            vec![(10, 20), (50, 100), (100, 200)]
        );
        assert!(
            titles
                .iter()
                .all(|item| item.retained_titles.len() <= TITLE_RETAINED_LIMIT)
        );
    }
    #[test]
    fn concurrency_and_slowed_ui_metadata_are_identifiable() {
        let jobs = Scenario::ConcurrentJobs.generate(1).jobs;
        assert_eq!(
            jobs.iter().map(|job| job.job_id).collect::<Vec<_>>(),
            vec![10, 20, 30, 10, 20, 30, 10, 20, 30]
        );
        assert_eq!(job_sequences(&jobs, JobKind::TrackingWrite), vec![1, 2, 3]);
        assert_eq!(job_sequences(&jobs, JobKind::ImportBatch), vec![1, 2, 3]);
        assert_eq!(
            job_sequences(&jobs, JobKind::OverviewProjection),
            vec![1, 2, 3]
        );
        let slowed_ui = Scenario::SlowedUi.generate(1).slowed_ui;
        assert!(slowed_ui.is_some());
        let Some(ui) = slowed_ui else {
            return;
        };
        assert_eq!(ui.high_water_mark, ui.queue_capacity);
        assert_eq!(ui.delivered_snapshot_id, ui.submitted_snapshots as u64);
        assert_eq!(
            ui.submitted_snapshots,
            ui.high_water_mark + ui.coalesced_snapshots
        );
    }
    #[test]
    fn identical_seeds_repeat_and_distinct_seeds_vary() {
        assert_eq!(
            Scenario::NormalWorkday.generate(4),
            Scenario::NormalWorkday.generate(4)
        );
        assert_ne!(
            Scenario::NormalWorkday.generate(4),
            Scenario::NormalWorkday.generate(5)
        );
    }
    fn boundaries(segments: &[BandSegment]) -> Vec<UtcMicros> {
        let Some(first) = segments.first() else {
            return Vec::new();
        };
        let mut values = Vec::with_capacity(segments.len() + 1);
        values.push(first.start);
        values.extend(segments.iter().map(|segment| segment.end));
        values
    }
    fn job_sequences(jobs: &[JobRecord], kind: JobKind) -> Vec<u32> {
        jobs.iter()
            .filter(|job| job.kind == kind)
            .map(|job| job.sequence)
            .collect()
    }
}
