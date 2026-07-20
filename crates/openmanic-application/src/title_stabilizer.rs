//! Privacy-preserving stabilization for optional foreground window-title observations.

use openmanic_domain::{ApplicationId, UtcMicros};

/// The required uninterrupted observation period before a title can be persisted.
pub const TITLE_STABILITY_US: i64 = 2_000_000;
/// Maximum UTF-8 bytes retained for one accepted title.
pub const MAX_WINDOW_TITLE_BYTES: usize = 2_048;

/// A title that is stable enough to pass to the durable title-span writer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedWindowTitle {
    application_id: ApplicationId,
    stable_since_utc: UtcMicros,
    accepted_at_utc: UtcMicros,
    text: String,
    text_hash: u64,
}

impl AcceptedWindowTitle {
    /// Returns the foreground application that owned the stable observation.
    #[must_use]
    pub const fn application_id(&self) -> ApplicationId {
        self.application_id
    }

    /// Returns the first instant at which this exact normalized title was observed.
    #[must_use]
    pub const fn stable_since_utc(&self) -> UtcMicros {
        self.stable_since_utc
    }

    /// Returns the observation instant that passed the stability threshold.
    #[must_use]
    pub const fn accepted_at_utc(&self) -> UtcMicros {
        self.accepted_at_utc
    }

    /// Returns bounded normalized title text. Callers must not log it.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the stable local comparison digest for deduplication before persistence.
    #[must_use]
    pub const fn text_hash(&self) -> u64 {
        self.text_hash
    }
}

/// Result of one title observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TitleObservationResult {
    /// The observation is disabled, excluded, empty, unstable, or already accepted.
    Ignored,
    /// The candidate has been stable long enough to persist exactly once.
    Accepted(AcceptedWindowTitle),
}

/// One bounded candidate title. It holds no window handle, PID, path, or title history.
#[derive(Debug, Default)]
pub struct TitleStabilizer {
    candidate: Option<TitleCandidate>,
}

#[derive(Debug)]
struct TitleCandidate {
    application_id: ApplicationId,
    stable_since_utc: UtcMicros,
    text: String,
    text_hash: u64,
    accepted: bool,
}

impl TitleStabilizer {
    /// Observes a current foreground title without affecting activity attribution.
    ///
    /// Disabled collection and excluded foreground immediately discard the candidate so neither
    /// can later produce retained title text.
    #[must_use]
    pub fn observe(
        &mut self,
        application_id: ApplicationId,
        observed_at_utc: UtcMicros,
        raw_title: &str,
        collection_enabled: bool,
        application_excluded: bool,
    ) -> TitleObservationResult {
        if !collection_enabled || application_excluded {
            self.candidate = None;
            return TitleObservationResult::Ignored;
        }
        let Some(text) = normalize_title(raw_title) else {
            self.candidate = None;
            return TitleObservationResult::Ignored;
        };
        let text_hash = title_hash(&text);
        let same_candidate = self.candidate.as_ref().is_some_and(|candidate| {
            candidate.application_id == application_id
                && candidate.text_hash == text_hash
                && candidate.text == text
        });
        if !same_candidate {
            self.candidate = Some(TitleCandidate {
                application_id,
                stable_since_utc: observed_at_utc,
                text,
                text_hash,
                accepted: false,
            });
            return TitleObservationResult::Ignored;
        }
        let Some(candidate) = self.candidate.as_mut() else {
            return TitleObservationResult::Ignored;
        };
        if candidate.accepted
            || observed_at_utc
                .get()
                .saturating_sub(candidate.stable_since_utc.get())
                < TITLE_STABILITY_US
        {
            return TitleObservationResult::Ignored;
        }
        candidate.accepted = true;
        TitleObservationResult::Accepted(AcceptedWindowTitle {
            application_id: candidate.application_id,
            stable_since_utc: candidate.stable_since_utc,
            accepted_at_utc: observed_at_utc,
            text: candidate.text.clone(),
            text_hash: candidate.text_hash,
        })
    }
}

fn normalize_title(raw_title: &str) -> Option<String> {
    let mut normalized = String::new();
    let mut previous_was_space = true;
    for character in raw_title.chars() {
        let replacement = if character.is_control() {
            ' '
        } else {
            character
        };
        if replacement.is_whitespace() {
            if !previous_was_space {
                normalized.push(' ');
            }
            previous_was_space = true;
        } else {
            normalized.push(replacement);
            previous_was_space = false;
        }
    }
    let normalized = normalized.trim();
    if normalized.is_empty() {
        return None;
    }
    let mut bounded = String::new();
    for character in normalized.chars() {
        if bounded.len().saturating_add(character.len_utf8()) > MAX_WINDOW_TITLE_BYTES {
            break;
        }
        bounded.push(character);
    }
    (!bounded.is_empty()).then_some(bounded)
}

fn title_hash(text: &str) -> u64 {
    text.as_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            hash.wrapping_mul(0x0000_0100_0000_01b3) ^ u64::from(*byte)
        })
}

#[cfg(test)]
mod tests {
    use openmanic_domain::{ApplicationId, UtcMicros};

    use super::{
        MAX_WINDOW_TITLE_BYTES, TITLE_STABILITY_US, TitleObservationResult, TitleStabilizer,
    };

    #[test]
    fn accepts_one_normalized_stable_title_without_activity_side_effects() {
        let mut stabilizer = TitleStabilizer::default();
        let application = ApplicationId::from_bytes([1; 16]);
        assert!(matches!(
            stabilizer.observe(
                application,
                UtcMicros::new(10),
                "  Plan\tfor today  ",
                true,
                false
            ),
            TitleObservationResult::Ignored
        ));
        let accepted = stabilizer.observe(
            application,
            UtcMicros::new(10 + TITLE_STABILITY_US),
            "Plan for today",
            true,
            false,
        );
        assert!(
            matches!(accepted, TitleObservationResult::Accepted(title) if title.text() == "Plan for today")
        );
        assert!(matches!(
            stabilizer.observe(
                application,
                UtcMicros::new(20 + TITLE_STABILITY_US),
                "Plan for today",
                true,
                false
            ),
            TitleObservationResult::Ignored
        ));
    }

    #[test]
    fn rapid_title_churn_at_all_required_rates_and_exclusion_never_accept_a_title() {
        let application = ApplicationId::from_bytes([2; 16]);
        for interval_us in [100_000, 20_000, 10_000] {
            let mut stabilizer = TitleStabilizer::default();
            for change in 0..100 {
                assert!(matches!(
                    stabilizer.observe(
                        application,
                        UtcMicros::new(change * interval_us),
                        &format!("Page {change}"),
                        true,
                        false,
                    ),
                    TitleObservationResult::Ignored
                ));
            }
        }
        let mut stabilizer = TitleStabilizer::default();
        assert!(matches!(
            stabilizer.observe(
                application,
                UtcMicros::new(3_000_000),
                "Private",
                true,
                true
            ),
            TitleObservationResult::Ignored
        ));
    }

    #[test]
    fn bounds_utf8_without_splitting_a_character() {
        let mut stabilizer = TitleStabilizer::default();
        let application = ApplicationId::from_bytes([3; 16]);
        let raw = "é".repeat(MAX_WINDOW_TITLE_BYTES);
        let _ = stabilizer.observe(application, UtcMicros::new(0), &raw, true, false);
        let result = stabilizer.observe(
            application,
            UtcMicros::new(TITLE_STABILITY_US),
            &raw,
            true,
            false,
        );
        assert!(
            matches!(result, TitleObservationResult::Accepted(title) if title.text().len() <= MAX_WINDOW_TITLE_BYTES)
        );
    }
}
