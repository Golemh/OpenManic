//! Immutable personal-schedule occurrences shared by Timeline and Calendar projections.

use openmanic_domain::{CategoryId, HalfOpenInterval};

use crate::{
    ScheduleBoundaryResolution, ScheduleId, ScheduleSnapshot, ScheduleTimeError,
    expand_repeating_schedule_in_interval,
};

/// Stable identity of one schedule occurrence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleOccurrenceId {
    /// The sole occurrence of a standalone one-time schedule.
    OneTime(ScheduleId),
    /// One anchored occurrence of a recurring series.
    Recurring {
        /// The recurring series stable identity.
        schedule_id: ScheduleId,
        /// The civil date on which the unmodified occurrence begins.
        anchor_date: i32,
    },
}

/// One canonical schedule occurrence ready for shared time-to-pixel geometry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleOccurrence {
    id: ScheduleOccurrenceId,
    interval: HalfOpenInterval,
    label: String,
    category_id: Option<CategoryId>,
    adjusted: bool,
}

impl ScheduleOccurrence {
    /// Returns the stable identity used for selection and scope-specific edit routing.
    #[must_use]
    pub const fn id(&self) -> ScheduleOccurrenceId {
        self.id
    }

    /// Returns the complete canonical interval before presentation clipping.
    #[must_use]
    pub const fn interval(&self) -> HalfOpenInterval {
        self.interval
    }

    /// Returns the label active for this occurrence.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the category active for this occurrence.
    #[must_use]
    pub const fn category_id(&self) -> Option<CategoryId> {
        self.category_id
    }

    /// Returns whether the schedule's DST policy adjusted either boundary.
    #[must_use]
    pub const fn adjusted(&self) -> bool {
        self.adjusted
    }
}

/// Projects every schedule occurrence with a positive intersection with `visible_range`.
///
/// The returned values retain un-clipped intervals and stable identities. Renderers can therefore
/// use the common timeline transform without mutating schedule truth or recorded activity.
///
/// # Errors
///
/// Returns [`ScheduleTimeError`] if a persisted recurring boundary cannot be resolved.
pub fn project_schedule_occurrences(
    visible_range: HalfOpenInterval,
    schedules: &[ScheduleSnapshot],
) -> Result<Vec<ScheduleOccurrence>, ScheduleTimeError> {
    let mut occurrences = Vec::new();
    for schedule in schedules {
        if let Some(interval) = schedule.rule().one_time_interval() {
            if interval.overlaps(visible_range) {
                occurrences.push(ScheduleOccurrence {
                    id: ScheduleOccurrenceId::OneTime(schedule.id()),
                    interval,
                    label: schedule.rule().label().to_owned(),
                    category_id: schedule.rule().category_id(),
                    adjusted: false,
                });
            }
            continue;
        }
        for occurrence in expand_repeating_schedule_in_interval(schedule.rule(), visible_range)? {
            occurrences.push(ScheduleOccurrence {
                id: ScheduleOccurrenceId::Recurring {
                    schedule_id: schedule.id(),
                    anchor_date: occurrence.anchor_local_date(),
                },
                interval: occurrence.interval(),
                label: occurrence.label().to_owned(),
                category_id: occurrence.category_id(),
                adjusted: occurrence.start_resolution() != ScheduleBoundaryResolution::Exact
                    || occurrence.end_resolution() != ScheduleBoundaryResolution::Exact,
            });
        }
    }
    occurrences.sort_by_key(|occurrence| (occurrence.interval.start(), occurrence.interval.end()));
    Ok(occurrences)
}

#[cfg(test)]
mod tests {
    use openmanic_domain::{
        HalfOpenInterval, OneTimeScheduleId, ScheduleRule, ScheduleSeriesId, UtcMicros,
    };

    use super::{ScheduleOccurrenceId, project_schedule_occurrences};
    use crate::{EntityRevision, ScheduleId, ScheduleSnapshot};

    #[test]
    fn projection_keeps_exact_occurrence_identity_and_unclipped_interval() {
        let one_time = ScheduleSnapshot::try_new(
            ScheduleId::OneTime(OneTimeScheduleId::from_bytes([1; 16])),
            ScheduleRule::one_time(
                "Appointment",
                None,
                HalfOpenInterval::try_new(UtcMicros::new(90), UtcMicros::new(210))
                    .expect("positive appointment"),
                "Etc/UTC",
            )
            .expect("valid appointment"),
            EntityRevision::new(0),
            UtcMicros::new(1),
        )
        .expect("matching one-time identity");
        let recurring = ScheduleSnapshot::try_new(
            ScheduleId::Series(ScheduleSeriesId::from_bytes([2; 16])),
            ScheduleRule::repeating(
                "Daily planning", None, 0b0111_1111, 9 * 3_600, 10 * 3_600, 0, None, "Etc/UTC",
            )
            .expect("valid recurring rule"),
            EntityRevision::new(0),
            UtcMicros::new(1),
        )
        .expect("matching recurring identity");
        let visible = HalfOpenInterval::try_new(
            UtcMicros::new(100),
            UtcMicros::new(9 * 3_600_000_000 + 1),
        )
        .expect("positive visible range");

        let occurrences = project_schedule_occurrences(visible, &[one_time, recurring])
            .expect("projection should resolve");
        assert_eq!(occurrences.len(), 2);
        assert_eq!(occurrences[0].interval().start().get(), 90);
        assert!(matches!(occurrences[0].id(), ScheduleOccurrenceId::OneTime(_)));
        assert!(matches!(occurrences[1].id(), ScheduleOccurrenceId::Recurring { anchor_date: 0, .. }));
        assert_eq!(occurrences[1].interval().start().get(), 9 * 3_600_000_000);
    }
}
