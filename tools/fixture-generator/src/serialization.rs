//! Streaming, dependency-free JSON serialization for synthetic fixture records.

use crate::ScenarioFixture;
use crate::scenarios::{JobKind, OverlayKind, ScheduleMarker};
use std::io::{self, Write};

/// Streams every fixture surface in stable order as newline-delimited JSON.
///
/// # Errors
///
/// Returns an error from the supplied writer.
pub fn write_jsonl(writer: &mut impl Write, fixture: &ScenarioFixture) -> io::Result<u64> {
    let mut counted = CountedWriter::new(writer);
    write_activity(&mut counted, fixture)?;
    write_bands(&mut counted, fixture)?;
    write_schedules(&mut counted, fixture)?;
    write_overlays(&mut counted, fixture)?;
    write_names(&mut counted, fixture)?;
    write_titles(&mut counted, fixture)?;
    write_jobs(&mut counted, fixture)?;
    write_slowed_ui(&mut counted, fixture)?;
    Ok(counted.checksum())
}

fn write_activity<W: Write>(
    counted: &mut CountedWriter<'_, W>,
    fixture: &ScenarioFixture,
) -> io::Result<()> {
    for item in &fixture.activity_intervals {
        writeln!(
            counted,
            "{{\"record\":\"activity\",\"id\":{},\"start\":{},\"end\":{},\"application\":\"{}\",\"category\":\"{}\",\"state\":\"{}\",\"window\":\"{}\"}}",
            item.id,
            item.start.get(),
            item.end.get(),
            escape(&item.application_id),
            escape(&item.category_id),
            escape(&item.state),
            escape(&item.window_id)
        )?;
    }
    Ok(())
}

fn write_bands<W: Write>(
    counted: &mut CountedWriter<'_, W>,
    fixture: &ScenarioFixture,
) -> io::Result<()> {
    for (record, items) in [
        ("category_band", &fixture.category_band),
        ("state_band", &fixture.state_band),
        ("application_band", &fixture.application_band),
    ] {
        for item in items {
            writeln!(
                counted,
                "{{\"record\":\"{record}\",\"id\":{},\"start\":{},\"end\":{},\"label\":\"{}\"}}",
                item.id,
                item.start.get(),
                item.end.get(),
                escape(&item.label)
            )?;
        }
    }
    Ok(())
}

fn write_schedules<W: Write>(
    counted: &mut CountedWriter<'_, W>,
    fixture: &ScenarioFixture,
) -> io::Result<()> {
    for item in &fixture.schedules {
        writeln!(
            counted,
            "{{\"record\":\"schedule\",\"id\":{},\"start\":{},\"end\":{},\"marker\":\"{}\",\"zone\":\"{}\",\"local_start\":\"{}\",\"local_end\":\"{}\"}}",
            item.id,
            item.start.get(),
            item.end.get(),
            schedule_marker(&item.marker),
            escape(&item.zone_name),
            escape(&item.local_start),
            escape(&item.local_end)
        )?;
    }
    Ok(())
}

fn write_overlays<W: Write>(
    counted: &mut CountedWriter<'_, W>,
    fixture: &ScenarioFixture,
) -> io::Result<()> {
    for item in &fixture.overlays {
        writeln!(
            counted,
            "{{\"record\":\"overlay\",\"id\":{},\"start\":{},\"end\":{},\"kind\":\"{}\"}}",
            item.id,
            item.start.get(),
            item.end.get(),
            overlay_kind(&item.kind)
        )?;
    }
    Ok(())
}

fn write_names<W: Write>(
    counted: &mut CountedWriter<'_, W>,
    fixture: &ScenarioFixture,
) -> io::Result<()> {
    for (record, items) in [
        ("application", &fixture.applications),
        ("category", &fixture.categories),
    ] {
        for (id, item) in items.iter().enumerate() {
            writeln!(
                counted,
                "{{\"record\":\"{record}\",\"id\":{id},\"name\":\"{}\"}}",
                escape(item)
            )?;
        }
    }
    Ok(())
}

fn write_titles<W: Write>(
    counted: &mut CountedWriter<'_, W>,
    fixture: &ScenarioFixture,
) -> io::Result<()> {
    for item in &fixture.title_rates {
        writeln!(
            counted,
            "{{\"record\":\"title_rate\",\"changes_per_second\":{},\"duration_seconds\":{},\"change_count\":{}}}",
            item.changes_per_second, item.duration_seconds, item.change_count
        )?;
        for (index, title) in item.retained_titles.iter().enumerate() {
            writeln!(
                counted,
                "{{\"record\":\"title\",\"changes_per_second\":{},\"index\":{index},\"value\":\"{}\"}}",
                item.changes_per_second,
                escape(title)
            )?;
        }
    }
    Ok(())
}

fn write_jobs<W: Write>(
    counted: &mut CountedWriter<'_, W>,
    fixture: &ScenarioFixture,
) -> io::Result<()> {
    for item in &fixture.jobs {
        writeln!(
            counted,
            "{{\"record\":\"job\",\"job_id\":{},\"kind\":\"{}\",\"sequence\":{}}}",
            item.job_id,
            job_kind(item.kind),
            item.sequence
        )?;
    }
    Ok(())
}

fn write_slowed_ui<W: Write>(
    counted: &mut CountedWriter<'_, W>,
    fixture: &ScenarioFixture,
) -> io::Result<()> {
    if let Some(item) = &fixture.slowed_ui {
        writeln!(
            counted,
            "{{\"record\":\"slowed_ui\",\"queue_capacity\":{},\"high_water_mark\":{},\"submitted_snapshots\":{},\"delivered_snapshot_id\":{},\"coalesced_snapshots\":{}}}",
            item.queue_capacity,
            item.high_water_mark,
            item.submitted_snapshots,
            item.delivered_snapshot_id,
            item.coalesced_snapshots
        )?;
    }
    Ok(())
}

/// Produces deterministic metadata JSON for an ordered fixture slice.
///
/// # Errors
///
/// Returns an error encountered while streaming fixture bytes to the checksum sink.
pub fn metadata_json(seed: u64, fixtures: &[ScenarioFixture]) -> io::Result<String> {
    let mut entries = String::new();
    for (index, fixture) in fixtures.iter().enumerate() {
        if index > 0 {
            entries.push(',');
        }
        entries.push_str(&metadata_entry(fixture)?);
    }
    Ok(format!("{{\"seed\":{seed},\"scenarios\":[{entries}]}}\n"))
}

fn metadata_entry(fixture: &ScenarioFixture) -> io::Result<String> {
    let mut sink = io::sink();
    let checksum = write_jsonl(&mut sink, fixture)?;
    let retained = fixture
        .title_rates
        .iter()
        .map(|item| item.retained_titles.len())
        .sum::<usize>();
    let changes = fixture
        .title_rates
        .iter()
        .map(|item| item.change_count)
        .sum::<usize>();
    Ok(format!(
        "{{\"name\":\"{}\",\"raw_interval_count\":{},\"activity\":{},\"category_band\":{},\"state_band\":{},\"application_band\":{},\"schedules\":{},\"overlays\":{},\"applications\":{},\"categories\":{},\"title_rates\":{},\"title_changes\":{},\"titles_retained\":{},\"jobs\":{},\"slowed_ui\":{},\"checksum\":\"fnv1a64:{checksum:016x}\"}}",
        escape(fixture.scenario.name()),
        fixture.metadata.raw_interval_count,
        fixture.activity_intervals.len(),
        fixture.category_band.len(),
        fixture.state_band.len(),
        fixture.application_band.len(),
        fixture.schedules.len(),
        fixture.overlays.len(),
        fixture.applications.len(),
        fixture.categories.len(),
        fixture.title_rates.len(),
        changes,
        retained,
        fixture.jobs.len(),
        usize::from(fixture.slowed_ui.is_some())
    ))
}

fn schedule_marker(marker: &ScheduleMarker) -> &'static str {
    match marker {
        ScheduleMarker::Adjacent => "adjacent",
        ScheduleMarker::Overnight => "overnight",
        ScheduleMarker::DstBoundary => "dst_boundary",
        ScheduleMarker::RecurrenceException => "recurrence_exception",
    }
}
fn overlay_kind(kind: &OverlayKind) -> &'static str {
    match kind {
        OverlayKind::Activity => "activity",
        OverlayKind::Focus => "focus",
        OverlayKind::Schedule => "schedule",
    }
}
fn job_kind(kind: JobKind) -> &'static str {
    match kind {
        JobKind::TrackingWrite => "tracking_write",
        JobKind::ImportBatch => "import_batch",
        JobKind::OverviewProjection => "overview_projection",
    }
}

struct CountedWriter<'a, W> {
    writer: &'a mut W,
    hash: u64,
}
impl<'a, W: Write> CountedWriter<'a, W> {
    fn new(writer: &'a mut W) -> Self {
        Self {
            writer,
            hash: 0xcbf2_9ce4_8422_2325,
        }
    }
    fn checksum(&self) -> u64 {
        self.hash
    }
}
impl<W: Write> Write for CountedWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.writer.write(bytes)?;
        for byte in &bytes[..written] {
            self.hash = (self.hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3);
        }
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

fn escape(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other if other <= '\u{001f}' => {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                let code = other as u32;
                escaped.push_str("\\u00");
                escaped.push(char::from(HEX[((code >> 4) & 15) as usize]));
                escaped.push(char::from(HEX[(code & 15) as usize]));
            }
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scenario;

    #[test]
    fn escapes_json_controls_without_changing_unicode() {
        assert_eq!(
            escape("\"\\\n\r\t\0\u{001f}é"),
            "\\\"\\\\\\n\\r\\t\\u0000\\u001fé"
        );
    }
    #[test]
    fn every_scenario_has_its_fixture_record_kind() {
        for scenario in Scenario::all() {
            let fixture = scenario.generate(7);
            let mut bytes = Vec::new();
            write_jsonl(&mut bytes, &fixture).expect("memory write");
            let jsonl = String::from_utf8(bytes).expect("utf8");
            let expected = match scenario {
                Scenario::NormalWorkday
                | Scenario::Dense10000IntervalRange
                | Scenario::RapidABa => "activity",
                Scenario::ThreeSegmentedBands => "category_band",
                Scenario::ScheduleDstOvernight => "schedule",
                Scenario::SimultaneousOverlays => "overlay",
                Scenario::LargeApplicationCategoryLists => "application",
                Scenario::TitleChurn1050100Hz => "title_rate",
                Scenario::ConcurrentJobs => "job",
                Scenario::SlowedUi => "slowed_ui",
            };
            assert!(!jsonl.is_empty());
            assert!(jsonl.contains(&format!("\"record\":\"{expected}\"")));
        }
    }
    #[test]
    fn same_seed_is_identical_and_different_seed_varies() {
        let a = Scenario::NormalWorkday.generate(4);
        let b = Scenario::NormalWorkday.generate(4);
        let c = Scenario::NormalWorkday.generate(5);
        let mut first = Vec::new();
        let mut second = Vec::new();
        let mut third = Vec::new();
        assert_eq!(
            write_jsonl(&mut first, &a).expect("write"),
            write_jsonl(&mut second, &b).expect("write")
        );
        write_jsonl(&mut third, &c).expect("write");
        assert_eq!(first, second);
        assert_ne!(first, third);
    }
    #[test]
    fn metadata_order_is_frozen_and_checksum_matches_direct_stream() {
        let fixtures = Scenario::all().map(|scenario| scenario.generate(2_026_030));
        let metadata = metadata_json(2_026_030, &fixtures).expect("metadata");
        let mut previous = 0;
        for scenario in Scenario::all() {
            let position = metadata[previous..]
                .find(scenario.name())
                .expect("scenario name")
                + previous;
            assert!(position >= previous);
            previous = position + scenario.name().len();
        }
        let mut bytes = Vec::new();
        let checksum = write_jsonl(&mut bytes, &fixtures[0]).expect("write");
        assert!(metadata.contains(&format!("fnv1a64:{checksum:016x}")));
    }
    #[test]
    fn dense_and_titles_respect_required_bounds() {
        let dense = Scenario::Dense10000IntervalRange.generate(1);
        assert!(dense.metadata.raw_interval_count >= 10_000);
        let titles = Scenario::TitleChurn1050100Hz.generate(1);
        assert!(
            titles
                .title_rates
                .iter()
                .all(|item| item.retained_titles.len() <= 16)
        );
    }
}
