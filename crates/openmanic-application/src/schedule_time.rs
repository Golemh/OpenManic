//! IANA-aware conversion between stored schedule civil values and UTC instants.

use core::fmt;

use jiff::{Span, civil::Date, tz};
use openmanic_domain::{CategoryId, HalfOpenInterval, ScheduleOccurrenceException, ScheduleRule, UtcMicros};

/// The stable civil-date encoding used by schedule segments and occurrence anchors.
///
/// Day zero is 1970-01-01 in the proleptic Gregorian calendar. The value is deliberately a
/// civil day, not a UTC duration, so it remains stable across time-zone changes.
pub const SCHEDULE_CIVIL_EPOCH: (i16, i8, i8) = (1970, 1, 1);

/// Provenance for a resolved local schedule boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleBoundaryResolution {
    /// The local civil time mapped to exactly one UTC instant.
    Exact,
    /// The local civil time was in a forward DST gap and moved to its first valid instant.
    FirstValidAfterGap,
    /// The local civil time was in a backward DST fold and chose the earlier instant.
    EarlierInstantInFold,
}

/// One UTC boundary derived from a persisted civil schedule value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedScheduleBoundary {
    instant: UtcMicros,
    resolution: ScheduleBoundaryResolution,
}

impl ResolvedScheduleBoundary {
    /// Returns the resolved UTC microsecond instant.
    #[must_use]
    pub const fn instant(self) -> UtcMicros {
        self.instant
    }

    /// Returns whether daylight-saving disambiguation adjusted this boundary.
    #[must_use]
    pub const fn resolution(self) -> ScheduleBoundaryResolution {
        self.resolution
    }
}

/// One expanded recurring occurrence retained as a canonical UTC interval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedScheduleOccurrence {
    anchor_local_date: i32,
    interval: HalfOpenInterval,
    label: String,
    category_id: Option<CategoryId>,
    start_resolution: ScheduleBoundaryResolution,
    end_resolution: ScheduleBoundaryResolution,
}

impl ResolvedScheduleOccurrence {
    /// Returns the stable local anchor date of this occurrence.
    #[must_use]
    pub const fn anchor_local_date(&self) -> i32 {
        self.anchor_local_date
    }

    /// Returns the canonical positive UTC interval without presentation clipping.
    #[must_use]
    pub const fn interval(&self) -> HalfOpenInterval {
        self.interval
    }

    /// Returns the label active for the occurrence's rule segment.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the optional category active for the occurrence's rule segment.
    #[must_use]
    pub const fn category_id(&self) -> Option<CategoryId> {
        self.category_id
    }

    /// Returns the start-boundary provenance.
    #[must_use]
    pub const fn start_resolution(&self) -> ScheduleBoundaryResolution {
        self.start_resolution
    }

    /// Returns the end-boundary provenance.
    #[must_use]
    pub const fn end_resolution(&self) -> ScheduleBoundaryResolution {
        self.end_resolution
    }
}

/// Expands a repeating schedule over an inclusive local-anchor-date range.
///
/// The caller supplies the exact bounded range needed by its projection or overlap check. Skips
/// suppress matching anchors; fixed overrides replace their resolved base interval.
///
/// # Errors
///
/// Returns [`ScheduleTimeError`] for a non-repeating rule, invalid date range, or an unresolvable
/// civil boundary.
pub fn expand_repeating_schedule(
    rule: &ScheduleRule,
    first_anchor_date: i32,
    last_anchor_date: i32,
) -> Result<Vec<ResolvedScheduleOccurrence>, ScheduleTimeError> {
    if !rule.is_repeating() {
        return Err(ScheduleTimeError::RuleDoesNotRepeat);
    }
    if last_anchor_date < first_anchor_date {
        return Err(ScheduleTimeError::InvalidAnchorRange);
    }
    let segments = rule.segments();
    let exceptions = rule.exceptions();
    let mut occurrences = Vec::new();
    for anchor_date in first_anchor_date..=last_anchor_date {
        let weekday = weekday_index(anchor_date);
        let Some(segment) = segments.iter().find(|segment| {
            segment.effective_start_date() <= anchor_date
                && segment
                    .effective_end_date()
                    .is_none_or(|end_date| anchor_date <= end_date)
                && (segment.weekday_mask() & (1_u8 << weekday)) != 0
        }) else {
            continue;
        };
        let exception = exceptions
            .iter()
            .find(|exception| exception_anchor_date(**exception) == anchor_date);
        if matches!(exception, Some(ScheduleOccurrenceException::Skip { .. })) {
            continue;
        }
        let (interval, start_resolution, end_resolution) = match exception {
            Some(ScheduleOccurrenceException::Override {
                interval,
                start_after_gap,
                start_earlier_fold,
                end_after_gap,
                end_earlier_fold,
                ..
            }) => (
                *interval,
                boundary_resolution_from_flags(*start_after_gap, *start_earlier_fold)?,
                boundary_resolution_from_flags(*end_after_gap, *end_earlier_fold)?,
            ),
            None => {
                let start = resolve_schedule_boundary(
                    anchor_date,
                    segment.start_second_of_day(),
                    segment.time_zone_id(),
                )?;
                let end_anchor_date = anchor_date
                    .checked_add(i32::from(segment.end_day_offset()))
                    .ok_or(ScheduleTimeError::CivilDateOutOfRange)?;
                let end = resolve_schedule_boundary(
                    end_anchor_date,
                    segment.end_second_of_day(),
                    segment.time_zone_id(),
                )?;
                let interval = ScheduleRule::resolved_utc_range(start.instant(), end.instant())
                    .map_err(|_| ScheduleTimeError::NonPositiveOccurrence)?;
                (interval, start.resolution(), end.resolution())
            }
            Some(ScheduleOccurrenceException::Skip { .. }) => continue,
        };
        occurrences.push(ResolvedScheduleOccurrence {
            anchor_local_date: anchor_date,
            interval,
            label: segment.label().to_owned(),
            category_id: segment.category_id(),
            start_resolution,
            end_resolution,
        });
    }
    Ok(occurrences)
}

fn weekday_index(anchor_local_date: i32) -> i32 {
    // 1970-01-01 was Thursday (Monday-zero index 3).
    (anchor_local_date.rem_euclid(7) + 3).rem_euclid(7)
}

fn exception_anchor_date(exception: ScheduleOccurrenceException) -> i32 {
    match exception {
        ScheduleOccurrenceException::Skip { anchor_date }
        | ScheduleOccurrenceException::Override { anchor_date, .. } => anchor_date,
    }
}

fn boundary_resolution_from_flags(
    after_gap: bool,
    earlier_fold: bool,
) -> Result<ScheduleBoundaryResolution, ScheduleTimeError> {
    match (after_gap, earlier_fold) {
        (false, false) => Ok(ScheduleBoundaryResolution::Exact),
        (true, false) => Ok(ScheduleBoundaryResolution::FirstValidAfterGap),
        (false, true) => Ok(ScheduleBoundaryResolution::EarlierInstantInFold),
        (true, true) => Err(ScheduleTimeError::ContradictoryBoundaryResolution),
    }
}

/// Resolves a stored civil day and second-of-day using the MVP's DST policy.
///
/// Gaps resolve to the first valid instant after the gap; folds select the earlier instant.
/// `anchor_local_date` is counted from [`SCHEDULE_CIVIL_EPOCH`].
///
/// # Errors
///
/// Returns [`ScheduleTimeError`] when the stored civil date, second, time-zone ID, or resulting
/// UTC instant cannot be represented.
pub fn resolve_schedule_boundary(
    anchor_local_date: i32,
    second_of_day: u32,
    time_zone_id: &str,
) -> Result<ResolvedScheduleBoundary, ScheduleTimeError> {
    let local = civil_datetime(anchor_local_date, second_of_day)?;
    let zone = tz::TimeZone::get(time_zone_id).map_err(|_| ScheduleTimeError::UnknownTimeZone)?;
    let ambiguous = zone.to_ambiguous_zoned(local);
    let (zoned, resolution) = if let Ok(zoned) = ambiguous.clone().unambiguous() {
        (zoned, ScheduleBoundaryResolution::Exact)
    } else {
        let earlier = ambiguous
            .clone()
            .earlier()
            .map_err(|_| ScheduleTimeError::UnresolvableCivilTime)?;
        let compatible = ambiguous
            .compatible()
            .map_err(|_| ScheduleTimeError::UnresolvableCivilTime)?;
        let resolution = if earlier.datetime() == local {
            ScheduleBoundaryResolution::EarlierInstantInFold
        } else {
            ScheduleBoundaryResolution::FirstValidAfterGap
        };
        (compatible, resolution)
    };
    Ok(ResolvedScheduleBoundary {
        instant: UtcMicros::new(zoned.timestamp().as_microsecond()),
        resolution,
    })
}

fn civil_datetime(
    anchor_local_date: i32,
    second_of_day: u32,
) -> Result<jiff::civil::DateTime, ScheduleTimeError> {
    if second_of_day >= 86_400 {
        return Err(ScheduleTimeError::SecondOfDayOutOfRange);
    }
    let epoch = Date::new(
        SCHEDULE_CIVIL_EPOCH.0,
        SCHEDULE_CIVIL_EPOCH.1,
        SCHEDULE_CIVIL_EPOCH.2,
    )
    .map_err(|_| ScheduleTimeError::CivilDateOutOfRange)?;
    let date = epoch
        .checked_add(Span::new().days(i64::from(anchor_local_date)))
        .map_err(|_| ScheduleTimeError::CivilDateOutOfRange)?;
    let hour = i8::try_from(second_of_day / 3_600)
        .map_err(|_| ScheduleTimeError::SecondOfDayOutOfRange)?;
    let minute = i8::try_from((second_of_day % 3_600) / 60)
        .map_err(|_| ScheduleTimeError::SecondOfDayOutOfRange)?;
    let second = i8::try_from(second_of_day % 60)
        .map_err(|_| ScheduleTimeError::SecondOfDayOutOfRange)?;
    Ok(date.at(hour, minute, second, 0))
}

/// Schedule civil-time resolution failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleTimeError {
    /// The supplied rule is one-time and has no civil recurrence to expand.
    RuleDoesNotRepeat,
    /// The requested local anchor range ends before it starts.
    InvalidAnchorRange,
    /// The persisted day lies outside the resolver's supported civil range.
    CivilDateOutOfRange,
    /// The persisted second is not in `0..86400`.
    SecondOfDayOutOfRange,
    /// The retained IANA zone identifier is not available in the resolver database.
    UnknownTimeZone,
    /// The resolver could not derive the policy-selected instant.
    UnresolvableCivilTime,
    /// An occurrence's resolved end did not follow its resolved start.
    NonPositiveOccurrence,
    /// A stored boundary cannot be both a gap and fold adjustment.
    ContradictoryBoundaryResolution,
}

impl fmt::Display for ScheduleTimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::RuleDoesNotRepeat => "schedule rule does not repeat",
            Self::InvalidAnchorRange => "schedule anchor range ends before it starts",
            Self::CivilDateOutOfRange => "schedule civil date is outside the supported range",
            Self::SecondOfDayOutOfRange => "schedule second of day is outside the supported range",
            Self::UnknownTimeZone => "schedule time zone is unavailable",
            Self::UnresolvableCivilTime => "schedule civil time cannot be resolved",
            Self::NonPositiveOccurrence => "schedule occurrence is not positive",
            Self::ContradictoryBoundaryResolution => "schedule boundary resolution is contradictory",
        })
    }
}

impl std::error::Error for ScheduleTimeError {}

#[cfg(test)]
mod tests {
    use openmanic_domain::{HalfOpenInterval, ScheduleRule, UtcMicros};

    use super::{
        SCHEDULE_CIVIL_EPOCH, ScheduleBoundaryResolution, ScheduleTimeError,
        expand_repeating_schedule, resolve_schedule_boundary,
    };

    #[test]
    fn resolves_epoch_day_without_adjustment() {
        let resolved = resolve_schedule_boundary(0, 0, "Etc/UTC").expect("valid UTC boundary");
        assert_eq!(resolved.instant().get(), 0);
        assert_eq!(resolved.resolution(), ScheduleBoundaryResolution::Exact);
        assert_eq!(SCHEDULE_CIVIL_EPOCH, (1970, 1, 1));
    }

    #[test]
    fn resolves_gap_after_and_fold_earlier() {
        let gap = resolve_schedule_boundary(19_792, 2 * 3_600 + 30 * 60, "America/New_York")
            .expect("gap resolves after transition");
        let fold = resolve_schedule_boundary(20_030, 3_600 + 30 * 60, "America/New_York")
            .expect("fold resolves to earlier instant");
        assert_eq!(
            gap.resolution(),
            ScheduleBoundaryResolution::FirstValidAfterGap
        );
        assert_eq!(
            fold.resolution(),
            ScheduleBoundaryResolution::EarlierInstantInFold
        );
        assert_eq!(gap.instant().get(), 1_710_055_800_000_000);
        assert_eq!(fold.instant().get(), 1_730_611_800_000_000);
    }

    #[test]
    fn rejects_invalid_civil_second_and_unknown_zone() {
        assert_eq!(
            resolve_schedule_boundary(0, 86_400, "Etc/UTC"),
            Err(ScheduleTimeError::SecondOfDayOutOfRange)
        );
        assert_eq!(
            resolve_schedule_boundary(0, 0, "Invalid/Zone"),
            Err(ScheduleTimeError::UnknownTimeZone)
        );
    }

    #[test]
    fn expands_rules_and_applies_skips_and_fixed_overrides() {
        let mut rule = ScheduleRule::repeating(
            "Daily planning",
            None,
            0b0111_1111,
            9 * 3_600,
            10 * 3_600,
            0,
            None,
            "Etc/UTC",
        )
        .expect("valid recurring rule");
        rule.skip_only_this_date(1).expect("covered date");
        let override_interval = HalfOpenInterval::try_new(UtcMicros::new(200), UtcMicros::new(300))
            .expect("positive override");
        rule.override_only_this_date(2, override_interval, false, false, false, false)
            .expect("covered date");

        let occurrences = expand_repeating_schedule(&rule, 0, 2).expect("expand recurring rule");
        assert_eq!(occurrences.len(), 2);
        assert_eq!(occurrences[0].anchor_local_date(), 0);
        assert_eq!(occurrences[0].interval().start().get(), 32_400_000_000);
        assert_eq!(occurrences[0].interval().end().get(), 36_000_000_000);
        assert_eq!(occurrences[1].anchor_local_date(), 2);
        assert_eq!(occurrences[1].interval(), override_interval);
    }
}
