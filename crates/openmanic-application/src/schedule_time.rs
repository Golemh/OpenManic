//! IANA-aware conversion between stored schedule civil values and UTC instants.

use core::fmt;

use jiff::{Span, civil::Date, tz};
use openmanic_domain::UtcMicros;

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
    /// The persisted day lies outside the resolver's supported civil range.
    CivilDateOutOfRange,
    /// The persisted second is not in `0..86400`.
    SecondOfDayOutOfRange,
    /// The retained IANA zone identifier is not available in the resolver database.
    UnknownTimeZone,
    /// The resolver could not derive the policy-selected instant.
    UnresolvableCivilTime,
}

impl fmt::Display for ScheduleTimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CivilDateOutOfRange => "schedule civil date is outside the supported range",
            Self::SecondOfDayOutOfRange => "schedule second of day is outside the supported range",
            Self::UnknownTimeZone => "schedule time zone is unavailable",
            Self::UnresolvableCivilTime => "schedule civil time cannot be resolved",
        })
    }
}

impl std::error::Error for ScheduleTimeError {}

#[cfg(test)]
mod tests {
    use super::{
        SCHEDULE_CIVIL_EPOCH, ScheduleBoundaryResolution, ScheduleTimeError,
        resolve_schedule_boundary,
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
}
