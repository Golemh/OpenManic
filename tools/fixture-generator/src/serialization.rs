//! Streaming, dependency-free JSON serialization for synthetic fixture records.

use crate::ScenarioFixture;
use std::io::{self, Write};

/// Streams one fixture's activity records as newline-delimited JSON.
///
/// # Errors
///
/// Returns an error from the supplied writer.
pub fn write_jsonl(writer: &mut impl Write, fixture: &ScenarioFixture) -> io::Result<u64> {
    let mut counted = CountedWriter::new(writer);
    for interval in &fixture.activity_intervals {
        writeln!(
            counted,
            "{{\"record\":\"activity\",\"id\":{},\"start\":{},\"end\":{},\"application\":\"{}\",\"category\":\"{}\",\"state\":\"{}\",\"window\":\"{}\"}}",
            interval.id,
            interval.start.get(),
            interval.end.get(),
            escape(&interval.application_id),
            escape(&interval.category_id),
            escape(&interval.state),
            escape(&interval.window_id),
        )?;
    }
    Ok(counted.checksum())
}

/// Produces deterministic metadata JSON for an ordered fixture slice.
#[must_use]
pub fn metadata_json(seed: u64, fixtures: &[ScenarioFixture]) -> String {
    let entries = fixtures
        .iter()
        .map(metadata_entry)
        .collect::<Vec<_>>()
        .join(",");
    format!("{{\"seed\":{seed},\"scenarios\":[{entries}]}}\n")
}

fn metadata_entry(fixture: &ScenarioFixture) -> String {
    let mut sink = Vec::new();
    let checksum = write_jsonl(&mut sink, fixture).unwrap_or(0);
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
    format!(
        "{{\"name\":\"{}\",\"raw_interval_count\":{},\"activity\":{},\"category_band\":{},\"state_band\":{},\"application_band\":{},\"schedules\":{},\"overlays\":{},\"title_changes\":{},\"titles_retained\":{},\"jobs\":{},\"checksum\":\"fnv1a64:{checksum:016x}\"}}",
        fixture.scenario.name(),
        fixture.metadata.raw_interval_count,
        fixture.activity_intervals.len(),
        fixture.category_band.len(),
        fixture.state_band.len(),
        fixture.application_band.len(),
        fixture.schedules.len(),
        fixture.overlays.len(),
        changes,
        retained,
        fixture.jobs.len()
    )
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
            other if other.is_control() => {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                let code = other as u32;
                let high = usize::try_from((code >> 4) & 15).unwrap_or(0);
                let low = usize::try_from(code & 15).unwrap_or(0);
                escaped.push_str("\\u00");
                escaped.push(char::from(HEX[high]));
                escaped.push(char::from(HEX[low]));
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
    fn identical_seed_has_identical_bytes_and_checksum() {
        let fixture = Scenario::NormalWorkday.generate(4);
        let mut first = Vec::new();
        let first_checksum = write_jsonl(&mut first, &fixture).expect("memory write");
        let mut second = Vec::new();
        let second_checksum = write_jsonl(&mut second, &fixture).expect("memory write");
        assert_eq!((first, first_checksum), (second, second_checksum));
    }
    #[test]
    fn all_names_and_dense_and_titles_are_present() {
        let fixtures = Scenario::all().map(|scenario| scenario.generate(2_026_030));
        assert!(fixtures[1].metadata.raw_interval_count >= 10_000);
        assert!(
            fixtures[7]
                .title_rates
                .iter()
                .all(|item| item.retained_titles.len() <= 16)
        );
        assert!(metadata_json(2_026_030, &fixtures).contains("normal-workday"));
    }
}
