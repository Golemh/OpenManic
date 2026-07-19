//! Pure personal-schedule rules, resolved exceptions, and validation policies.
//!
//! This module owns civil recurrence values and validates UTC ranges that an IANA-aware adapter
//! has already resolved. It deliberately does not load a time-zone database or expand rules.

use crate::{CategoryId, HalfOpenInterval, UtcMicros};
use core::fmt;
use std::collections::BTreeMap;

const WEEKDAY_MASK: u8 = 0b0111_1111;
const SECONDS_PER_DAY: u32 = 86_400;

/// The user-visible scope for changing a recurring schedule occurrence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleEditScope {
    /// Change or skip only the selected occurrence anchor date.
    OnlyThisDate,
    /// End the active rule segment before the anchor and start a replacement segment there.
    ThisAndFuture,
    /// Replace the recurring rule intent across all effective dates while retaining exceptions.
    EveryOccurrence,
}

/// One persistence-safe civil segment of a recurring personal schedule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleSegment {
    effective_start_date: i32,
    effective_end_date: Option<i32>,
    weekday_mask: u8,
    start_second_of_day: u32,
    end_second_of_day: u32,
    time_zone_id: String,
    label: String,
    category_id: Option<CategoryId>,
}

impl ScheduleSegment {
    /// Validates and creates a civil schedule segment.
    ///
    /// # Errors
    ///
    /// Returns [`ScheduleValidationError`] when one of the persisted civil fields is invalid.
    #[expect(
        clippy::too_many_arguments,
        reason = "The stored civil rule fields stay explicit at their validation boundary."
    )]
    pub fn try_new(
        effective_start_date: i32,
        effective_end_date: Option<i32>,
        weekday_mask: u8,
        start_second_of_day: u32,
        end_second_of_day: u32,
        time_zone_id: impl Into<String>,
        label: impl Into<String>,
        category_id: Option<CategoryId>,
    ) -> Result<Self, ScheduleValidationError> {
        let segment = RuleSegment::try_new(
            label.into(),
            category_id,
            weekday_mask,
            start_second_of_day,
            end_second_of_day,
            effective_start_date,
            effective_end_date,
            time_zone_id.into(),
        )?;
        Ok(Self::from_rule_segment(&segment))
    }

    /// Returns the first covered local civil date.
    #[must_use]
    pub const fn effective_start_date(&self) -> i32 {
        self.effective_start_date
    }
    /// Returns the inclusive final covered local civil date, when bounded.
    #[must_use]
    pub const fn effective_end_date(&self) -> Option<i32> {
        self.effective_end_date
    }
    /// Returns the Monday-first recurrence bit mask.
    #[must_use]
    pub const fn weekday_mask(&self) -> u8 {
        self.weekday_mask
    }
    /// Returns seconds after local midnight for the start boundary.
    #[must_use]
    pub const fn start_second_of_day(&self) -> u32 {
        self.start_second_of_day
    }
    /// Returns seconds after local midnight for the end boundary.
    #[must_use]
    pub const fn end_second_of_day(&self) -> u32 {
        self.end_second_of_day
    }
    /// Returns whether the end clock falls on the following civil day.
    #[must_use]
    pub const fn end_day_offset(&self) -> u8 {
        if self.end_second_of_day < self.start_second_of_day {
            1
        } else {
            0
        }
    }
    /// Returns the retained IANA time-zone identifier.
    #[must_use]
    pub fn time_zone_id(&self) -> &str {
        &self.time_zone_id
    }
    /// Returns the user-visible label.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
    /// Returns the optional category association.
    #[must_use]
    pub const fn category_id(&self) -> Option<CategoryId> {
        self.category_id
    }

    fn from_rule_segment(segment: &RuleSegment) -> Self {
        Self {
            effective_start_date: segment.effective_start_date,
            effective_end_date: segment.effective_end_date,
            weekday_mask: segment.weekday_mask,
            start_second_of_day: segment.start_second_of_day,
            end_second_of_day: segment.end_second_of_day,
            time_zone_id: segment.time_zone_id.clone(),
            label: segment.label.clone(),
            category_id: segment.category_id,
        }
    }

    fn into_rule_segment(self) -> Result<RuleSegment, ScheduleValidationError> {
        RuleSegment::try_new(
            self.label,
            self.category_id,
            self.weekday_mask,
            self.start_second_of_day,
            self.end_second_of_day,
            self.effective_start_date,
            self.effective_end_date,
            self.time_zone_id,
        )
    }
}

/// A validated personal one-time schedule or repeating schedule series.
///
/// One-time schedules contain a positive UTC interval. Repeating schedules retain their civil
/// local-time rule segments and occurrence-only exceptions; an IANA-aware adapter later expands
/// those values into UTC occurrences for a requested range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleRule {
    kind: ScheduleKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ScheduleKind {
    OneTime(OneTimeSchedule),
    Repeating(RepeatingSchedule),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OneTimeSchedule {
    label: String,
    category_id: Option<CategoryId>,
    interval: HalfOpenInterval,
    created_zone_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RepeatingSchedule {
    segments: Vec<RuleSegment>,
    exceptions: BTreeMap<i32, OccurrenceException>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuleSegment {
    effective_start_date: i32,
    effective_end_date: Option<i32>,
    weekday_mask: u8,
    start_second_of_day: u32,
    end_second_of_day: u32,
    end_day_offset: u8,
    time_zone_id: String,
    label: String,
    category_id: Option<CategoryId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum OccurrenceException {
    Skip,
    Override {
        interval: HalfOpenInterval,
        start_resolution: BoundaryResolution,
        end_resolution: BoundaryResolution,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoundaryResolution {
    Exact,
    FirstValidAfterGap,
    EarlierInstantInFold,
}

impl ScheduleRule {
    /// Creates a one-time personal schedule from an already validated UTC interval.
    ///
    /// `created_zone_id` is retained for explanation and export only. This pure domain module
    /// treats it as an opaque, nonempty IANA identifier because zone parsing belongs to the
    /// platform/application boundary.
    ///
    /// # Errors
    ///
    /// Returns [`ScheduleValidationError`] for an empty label or time-zone identifier.
    pub fn one_time(
        label: impl Into<String>,
        category_id: Option<CategoryId>,
        interval: HalfOpenInterval,
        created_zone_id: impl Into<String>,
    ) -> Result<Self, ScheduleValidationError> {
        let label = validate_label(label.into())?;
        let created_zone_id = validate_time_zone_id(created_zone_id.into())?;
        Ok(Self {
            kind: ScheduleKind::OneTime(OneTimeSchedule {
                label,
                category_id,
                interval,
                created_zone_id,
            }),
        })
    }

    /// Creates a recurring schedule with one initial civil rule segment.
    ///
    /// The interval becomes overnight when `end_second_of_day` is earlier than
    /// `start_second_of_day`; equal values are rejected rather than inferred as 24 hours.
    /// `weekday_mask` uses bits zero through six for Monday through Sunday respectively.
    ///
    /// # Errors
    ///
    /// Returns [`ScheduleValidationError`] when any civil rule invariant is invalid.
    #[expect(
        clippy::too_many_arguments,
        reason = "The persisted civil rule fields are kept explicit at their validation boundary."
    )]
    pub fn repeating(
        label: impl Into<String>,
        category_id: Option<CategoryId>,
        weekday_mask: u8,
        start_second_of_day: u32,
        end_second_of_day: u32,
        effective_start_date: i32,
        effective_end_date: Option<i32>,
        time_zone_id: impl Into<String>,
    ) -> Result<Self, ScheduleValidationError> {
        let segment = RuleSegment::try_new(
            label.into(),
            category_id,
            weekday_mask,
            start_second_of_day,
            end_second_of_day,
            effective_start_date,
            effective_end_date,
            time_zone_id.into(),
        )?;
        Ok(Self {
            kind: ScheduleKind::Repeating(RepeatingSchedule {
                segments: vec![segment],
                exceptions: BTreeMap::new(),
            }),
        })
    }

    /// Restores a recurring rule from validated persistence-safe civil segments.
    ///
    /// Exceptions are deliberately restored by the application/storage boundary after its
    /// resolved UTC values have been decoded; this constructor establishes the non-overlapping
    /// series lineage first.
    ///
    /// # Errors
    ///
    /// Returns [`ScheduleValidationError`] when the segments are empty, malformed, or overlap in
    /// effective civil-date coverage.
    pub fn try_restore_repeating(
        segments: impl IntoIterator<Item = ScheduleSegment>,
    ) -> Result<Self, ScheduleValidationError> {
        let segments = segments
            .into_iter()
            .map(ScheduleSegment::into_rule_segment)
            .collect::<Result<Vec<_>, _>>()?;
        if segments.is_empty() {
            return Err(ScheduleValidationError::MissingRuleSegment);
        }
        validate_segment_coverage(&segments)?;
        Ok(Self {
            kind: ScheduleKind::Repeating(RepeatingSchedule {
                segments,
                exceptions: BTreeMap::new(),
            }),
        })
    }

    /// Creates a positive resolved UTC range from adapter-provided instants.
    ///
    /// The caller applies IANA gap/fold rules, then passes the resolved instants and boundary
    /// metadata to [`Self::override_only_this_date`].
    ///
    /// # Errors
    ///
    /// Returns [`ScheduleValidationError::ResolvedOccurrenceNotPositive`] unless `end` is after
    /// `start`.
    pub fn resolved_utc_range(
        start: UtcMicros,
        end: UtcMicros,
    ) -> Result<HalfOpenInterval, ScheduleValidationError> {
        HalfOpenInterval::try_new(start, end)
            .map_err(|_| ScheduleValidationError::ResolvedOccurrenceNotPositive { start, end })
    }

    /// Returns true when this rule repeats through civil rule segments.
    #[must_use]
    pub const fn is_repeating(&self) -> bool {
        matches!(self.kind, ScheduleKind::Repeating(_))
    }

    /// Returns the label of a one-time schedule or its first repeating rule segment.
    #[must_use]
    pub fn label(&self) -> &str {
        match &self.kind {
            ScheduleKind::OneTime(schedule) => &schedule.label,
            ScheduleKind::Repeating(schedule) => schedule
                .segments
                .first()
                .map_or("", |segment| segment.label.as_str()),
        }
    }

    /// Returns the optional category association of a one-time schedule or first rule segment.
    #[must_use]
    pub fn category_id(&self) -> Option<CategoryId> {
        match &self.kind {
            ScheduleKind::OneTime(schedule) => schedule.category_id,
            ScheduleKind::Repeating(schedule) => schedule
                .segments
                .first()
                .and_then(|segment| segment.category_id),
        }
    }

    /// Returns the positive UTC interval for a one-time schedule.
    #[must_use]
    pub const fn one_time_interval(&self) -> Option<HalfOpenInterval> {
        match &self.kind {
            ScheduleKind::OneTime(schedule) => Some(schedule.interval),
            ScheduleKind::Repeating(_) => None,
        }
    }

    /// Returns the zone retained for a one-time schedule's creation context.
    #[must_use]
    pub fn created_zone_id(&self) -> Option<&str> {
        match &self.kind {
            ScheduleKind::OneTime(schedule) => Some(&schedule.created_zone_id),
            ScheduleKind::Repeating(_) => None,
        }
    }

    /// Returns the overnight day offset for the first recurring segment.
    ///
    /// A returned value of `1` means an earlier end clock is explicitly an overnight interval.
    #[must_use]
    pub fn end_day_offset(&self) -> Option<u8> {
        self.recurring()
            .and_then(|schedule| schedule.segments.first())
            .map(|segment| segment.end_day_offset)
    }

    /// Returns whether a recurring segment applies to the anchor date and weekday.
    ///
    /// `weekday_index` is Monday = 0 through Sunday = 6.
    #[must_use]
    pub fn applies_on_anchor_date(&self, anchor_date: i32, weekday_index: u8) -> bool {
        self.recurring().is_some_and(|schedule| {
            schedule
                .segments
                .iter()
                .any(|segment| segment.matches_anchor(anchor_date, weekday_index))
        })
    }

    /// Returns the rule-segment zone active on an anchor date.
    #[must_use]
    pub fn time_zone_for_anchor_date(&self, anchor_date: i32) -> Option<&str> {
        self.recurring()
            .and_then(|schedule| schedule.segment_for_date(anchor_date))
            .map(|segment| segment.time_zone_id.as_str())
    }

    /// Returns the number of repeating rule segments.
    #[must_use]
    pub fn segment_count(&self) -> usize {
        self.recurring()
            .map_or(0, |schedule| schedule.segments.len())
    }

    /// Returns persistence-safe copies of every recurring civil segment in effective-date order.
    #[must_use]
    pub fn segments(&self) -> Vec<ScheduleSegment> {
        self.recurring().map_or_else(Vec::new, |schedule| {
            schedule
                .segments
                .iter()
                .map(ScheduleSegment::from_rule_segment)
                .collect()
        })
    }

    /// Returns the number of occurrence-only exceptions on a repeating schedule.
    #[must_use]
    pub fn exception_count(&self) -> usize {
        self.recurring()
            .map_or(0, |schedule| schedule.exceptions.len())
    }

    /// Returns true when an occurrence-only skip exists for the anchor date.
    #[must_use]
    pub fn is_skipped_on(&self, anchor_date: i32) -> bool {
        self.recurring().is_some_and(|schedule| {
            matches!(
                schedule.exceptions.get(&anchor_date),
                Some(OccurrenceException::Skip)
            )
        })
    }

    /// Returns an occurrence-only fixed UTC override for the anchor date, if present.
    #[must_use]
    pub fn resolved_override_on(&self, anchor_date: i32) -> Option<HalfOpenInterval> {
        let exception = self.recurring()?.exceptions.get(&anchor_date)?;
        match exception {
            OccurrenceException::Skip => None,
            OccurrenceException::Override { interval, .. } => Some(*interval),
        }
    }

    /// Returns true when an override boundary used the DST gap policy.
    #[must_use]
    pub fn has_gap_adjustment_on(&self, anchor_date: i32) -> bool {
        self.exception_has_adjustment(anchor_date, BoundaryResolution::FirstValidAfterGap)
    }

    /// Returns true when an override boundary used the DST fold policy.
    #[must_use]
    pub fn has_fold_adjustment_on(&self, anchor_date: i32) -> bool {
        self.exception_has_adjustment(anchor_date, BoundaryResolution::EarlierInstantInFold)
    }

    /// Inserts or replaces an occurrence-only skip for a selected recurring anchor date.
    ///
    /// This is the domain operation for [`ScheduleEditScope::OnlyThisDate`] when no fixed UTC
    /// replacement is required.
    ///
    /// # Errors
    ///
    /// Returns [`ScheduleValidationError`] when this is not a repeating rule or the date has no
    /// matching rule segment.
    pub fn skip_only_this_date(&mut self, anchor_date: i32) -> Result<(), ScheduleValidationError> {
        let schedule = self.recurring_mut()?;
        ensure_anchor_is_covered(schedule, anchor_date)?;
        schedule
            .exceptions
            .insert(anchor_date, OccurrenceException::Skip);
        Ok(())
    }

    /// Inserts or replaces an occurrence-only resolved UTC override.
    ///
    /// The four adjustment inputs are supplied by the IANA-aware expansion boundary after it
    /// resolves each civil boundary. A boundary cannot be both a gap and a fold adjustment.
    /// This is the domain operation for [`ScheduleEditScope::OnlyThisDate`] with a fixed UTC
    /// replacement.
    ///
    /// # Errors
    ///
    /// Returns [`ScheduleValidationError`] for an uncovered anchor date or contradictory DST
    /// metadata.
    #[expect(
        clippy::fn_params_excessive_bools,
        reason = "Both resolved boundaries require independently auditable gap/fold inputs."
    )]
    pub fn override_only_this_date(
        &mut self,
        anchor_date: i32,
        interval: HalfOpenInterval,
        start_after_gap: bool,
        start_earlier_fold: bool,
        end_after_gap: bool,
        end_earlier_fold: bool,
    ) -> Result<(), ScheduleValidationError> {
        let start_resolution = BoundaryResolution::from_flags(start_after_gap, start_earlier_fold)?;
        let end_resolution = BoundaryResolution::from_flags(end_after_gap, end_earlier_fold)?;
        let schedule = self.recurring_mut()?;
        ensure_anchor_is_covered(schedule, anchor_date)?;
        schedule.exceptions.insert(
            anchor_date,
            OccurrenceException::Override {
                interval,
                start_resolution,
                end_resolution,
            },
        );
        Ok(())
    }

    /// Ends the active segment before `anchor_date` and inserts a replacement segment there.
    ///
    /// Existing occurrence-only exceptions remain attached to their stable anchor dates. This is
    /// the domain operation for [`ScheduleEditScope::ThisAndFuture`], including a user time-zone
    /// change after the caller has found the first future occurrence anchor.
    ///
    /// # Errors
    ///
    /// Returns [`ScheduleValidationError`] for an invalid replacement or uncovered anchor date.
    #[expect(
        clippy::too_many_arguments,
        reason = "A future rule segment validates each persisted civil field at the split boundary."
    )]
    pub fn change_this_and_future(
        &mut self,
        anchor_date: i32,
        label: impl Into<String>,
        category_id: Option<CategoryId>,
        weekday_mask: u8,
        start_second_of_day: u32,
        end_second_of_day: u32,
        time_zone_id: impl Into<String>,
    ) -> Result<(), ScheduleValidationError> {
        let schedule = self.recurring_mut()?;
        let index = schedule
            .segment_index_for_date(anchor_date)
            .ok_or(ScheduleValidationError::NoRuleSegmentForAnchor { anchor_date })?;
        let current = schedule.segments[index].clone();
        let replacement = RuleSegment::try_new(
            label.into(),
            category_id,
            weekday_mask,
            start_second_of_day,
            end_second_of_day,
            anchor_date,
            current.effective_end_date,
            time_zone_id.into(),
        )?;

        if current.effective_start_date == anchor_date {
            schedule.segments[index] = replacement;
        } else {
            let prior_end = anchor_date
                .checked_sub(1)
                .ok_or(ScheduleValidationError::AnchorDateUnderflow { anchor_date })?;
            schedule.segments[index].effective_end_date = Some(prior_end);
            schedule.segments.insert(index + 1, replacement);
        }
        validate_segment_coverage(&schedule.segments)
    }

    /// Replaces the recurring rule intent across all effective dates while preserving exceptions.
    ///
    /// This is the domain operation for [`ScheduleEditScope::EveryOccurrence`]. The caller later
    /// expands and revalidates preserved exceptions against the replacement rule.
    ///
    /// # Errors
    ///
    /// Returns [`ScheduleValidationError`] for an invalid replacement or a non-repeating rule.
    pub fn replace_every_occurrence(
        &mut self,
        label: impl Into<String>,
        category_id: Option<CategoryId>,
        weekday_mask: u8,
        start_second_of_day: u32,
        end_second_of_day: u32,
        time_zone_id: impl Into<String>,
    ) -> Result<(), ScheduleValidationError> {
        let schedule = self.recurring_mut()?;
        let first = schedule
            .segments
            .first()
            .ok_or(ScheduleValidationError::MissingRuleSegment)?;
        let last_end = schedule
            .segments
            .last()
            .and_then(|segment| segment.effective_end_date);
        let replacement = RuleSegment::try_new(
            label.into(),
            category_id,
            weekday_mask,
            start_second_of_day,
            end_second_of_day,
            first.effective_start_date,
            last_end,
            time_zone_id.into(),
        )?;
        schedule.segments = vec![replacement];
        Ok(())
    }

    /// Segments a recurring schedule at the caller-determined first future occurrence anchor.
    ///
    /// The current civil weekday/time fields are retained, while the new zone applies only from
    /// `first_future_anchor_date` onward. The caller determines that date from real occurrence
    /// expansion; this pure value object does not attempt IANA zone conversion.
    ///
    /// # Errors
    ///
    /// Returns [`ScheduleValidationError`] when no segment covers the requested anchor or the
    /// zone identifier is empty.
    pub fn change_time_zone_for_future(
        &mut self,
        first_future_anchor_date: i32,
        time_zone_id: impl Into<String>,
    ) -> Result<(), ScheduleValidationError> {
        let current = self
            .recurring()
            .ok_or(ScheduleValidationError::EditRequiresRepeatingRule)?
            .segment_for_date(first_future_anchor_date)
            .cloned()
            .ok_or(ScheduleValidationError::NoRuleSegmentForAnchor {
                anchor_date: first_future_anchor_date,
            })?;
        self.change_this_and_future(
            first_future_anchor_date,
            current.label,
            current.category_id,
            current.weekday_mask,
            current.start_second_of_day,
            current.end_second_of_day,
            time_zone_id,
        )
    }

    /// Rejects positive UTC overlap with existing resolved personal-schedule intervals.
    ///
    /// Adjacent half-open ranges are accepted. The persistence writer remains the final
    /// cross-entity authority; this deterministic helper is suitable for domain/UI preflight.
    ///
    /// # Errors
    ///
    /// Returns [`ScheduleValidationError::ResolvedScheduleConflict`] for the first overlapping
    /// existing range.
    pub fn validate_resolved_overlap(
        candidate: HalfOpenInterval,
        existing: &[HalfOpenInterval],
    ) -> Result<(), ScheduleValidationError> {
        if let Some((existing_index, interval)) = existing
            .iter()
            .copied()
            .enumerate()
            .find(|(_, interval)| candidate.overlaps(*interval))
        {
            return Err(ScheduleValidationError::ResolvedScheduleConflict {
                existing_index,
                interval,
            });
        }
        Ok(())
    }

    fn recurring(&self) -> Option<&RepeatingSchedule> {
        match &self.kind {
            ScheduleKind::OneTime(_) => None,
            ScheduleKind::Repeating(schedule) => Some(schedule),
        }
    }

    fn recurring_mut(&mut self) -> Result<&mut RepeatingSchedule, ScheduleValidationError> {
        match &mut self.kind {
            ScheduleKind::OneTime(_) => Err(ScheduleValidationError::EditRequiresRepeatingRule),
            ScheduleKind::Repeating(schedule) => Ok(schedule),
        }
    }

    fn exception_has_adjustment(&self, anchor_date: i32, adjustment: BoundaryResolution) -> bool {
        let Some(OccurrenceException::Override {
            start_resolution,
            end_resolution,
            ..
        }) = self
            .recurring()
            .and_then(|schedule| schedule.exceptions.get(&anchor_date))
        else {
            return false;
        };
        *start_resolution == adjustment || *end_resolution == adjustment
    }
}

impl RuleSegment {
    #[expect(
        clippy::too_many_arguments,
        reason = "The stored civil segment fields are validated together at their single construction boundary."
    )]
    fn try_new(
        label: String,
        category_id: Option<CategoryId>,
        weekday_mask: u8,
        start_second_of_day: u32,
        end_second_of_day: u32,
        effective_start_date: i32,
        effective_end_date: Option<i32>,
        time_zone_id: String,
    ) -> Result<Self, ScheduleValidationError> {
        let label = validate_label(label)?;
        let time_zone_id = validate_time_zone_id(time_zone_id)?;
        if weekday_mask == 0 || weekday_mask & !WEEKDAY_MASK != 0 {
            return Err(ScheduleValidationError::InvalidWeekdayMask { weekday_mask });
        }
        if start_second_of_day >= SECONDS_PER_DAY || end_second_of_day >= SECONDS_PER_DAY {
            return Err(ScheduleValidationError::InvalidSecondOfDay {
                start_second_of_day,
                end_second_of_day,
            });
        }
        if start_second_of_day == end_second_of_day {
            return Err(ScheduleValidationError::EqualCivilBoundaries {
                second_of_day: start_second_of_day,
            });
        }
        if effective_end_date.is_some_and(|end| end < effective_start_date) {
            return Err(ScheduleValidationError::InvertedEffectiveDateRange {
                start_date: effective_start_date,
                end_date: effective_end_date,
            });
        }
        Ok(Self {
            effective_start_date,
            effective_end_date,
            weekday_mask,
            start_second_of_day,
            end_second_of_day,
            end_day_offset: u8::from(end_second_of_day < start_second_of_day),
            time_zone_id,
            label,
            category_id,
        })
    }

    fn covers_date(&self, anchor_date: i32) -> bool {
        self.effective_start_date <= anchor_date
            && self
                .effective_end_date
                .is_none_or(|end_date| anchor_date <= end_date)
    }

    fn matches_anchor(&self, anchor_date: i32, weekday_index: u8) -> bool {
        self.covers_date(anchor_date)
            && weekday_index < 7
            && self.weekday_mask & (1_u8 << weekday_index) != 0
    }
}

impl RepeatingSchedule {
    fn segment_for_date(&self, anchor_date: i32) -> Option<&RuleSegment> {
        self.segments
            .iter()
            .find(|segment| segment.covers_date(anchor_date))
    }

    fn segment_index_for_date(&self, anchor_date: i32) -> Option<usize> {
        self.segments
            .iter()
            .position(|segment| segment.covers_date(anchor_date))
    }
}

impl BoundaryResolution {
    fn from_flags(after_gap: bool, earlier_fold: bool) -> Result<Self, ScheduleValidationError> {
        match (after_gap, earlier_fold) {
            (false, false) => Ok(Self::Exact),
            (true, false) => Ok(Self::FirstValidAfterGap),
            (false, true) => Ok(Self::EarlierInstantInFold),
            (true, true) => Err(ScheduleValidationError::ContradictoryDstAdjustment),
        }
    }
}

fn validate_label(label: String) -> Result<String, ScheduleValidationError> {
    if label.trim().is_empty() {
        return Err(ScheduleValidationError::EmptyLabel);
    }
    Ok(label)
}

fn validate_time_zone_id(time_zone_id: String) -> Result<String, ScheduleValidationError> {
    if time_zone_id.trim().is_empty() {
        return Err(ScheduleValidationError::EmptyTimeZoneId);
    }
    Ok(time_zone_id)
}

fn ensure_anchor_is_covered(
    schedule: &RepeatingSchedule,
    anchor_date: i32,
) -> Result<(), ScheduleValidationError> {
    if schedule.segment_for_date(anchor_date).is_none() {
        return Err(ScheduleValidationError::NoRuleSegmentForAnchor { anchor_date });
    }
    Ok(())
}

fn validate_segment_coverage(segments: &[RuleSegment]) -> Result<(), ScheduleValidationError> {
    for pair in segments.windows(2) {
        let [left, right] = pair else {
            continue;
        };
        if left.effective_start_date >= right.effective_start_date
            || left
                .effective_end_date
                .is_none_or(|left_end| left_end >= right.effective_start_date)
        {
            return Err(ScheduleValidationError::OverlappingRuleSegments {
                left_start_date: left.effective_start_date,
                right_start_date: right.effective_start_date,
            });
        }
    }
    Ok(())
}

/// Failure while constructing, changing, or validating a personal schedule rule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleValidationError {
    /// A label had no non-whitespace content.
    EmptyLabel,
    /// An opaque IANA zone identifier had no non-whitespace content.
    EmptyTimeZoneId,
    /// A weekday mask was empty or contained bits outside Monday through Sunday.
    InvalidWeekdayMask {
        /// The rejected bit mask.
        weekday_mask: u8,
    },
    /// A local clock boundary was outside `0..86_400` seconds.
    InvalidSecondOfDay {
        /// Rejected local start boundary.
        start_second_of_day: u32,
        /// Rejected local end boundary.
        end_second_of_day: u32,
    },
    /// Equal local start/end boundaries would ambiguously imply a 24-hour schedule.
    EqualCivilBoundaries {
        /// The equal local second-of-day value.
        second_of_day: u32,
    },
    /// The inclusive effective end date was before the effective start date.
    InvertedEffectiveDateRange {
        /// Requested effective start date.
        start_date: i32,
        /// Requested effective end date.
        end_date: Option<i32>,
    },
    /// A resolved UTC occurrence did not have positive duration.
    ResolvedOccurrenceNotPositive {
        /// Requested UTC start instant.
        start: UtcMicros,
        /// Requested UTC end instant.
        end: UtcMicros,
    },
    /// A recurring-only operation targeted a one-time schedule.
    EditRequiresRepeatingRule,
    /// No rule segment covered the requested occurrence anchor date.
    NoRuleSegmentForAnchor {
        /// The uncovered local anchor date.
        anchor_date: i32,
    },
    /// An internal recurring schedule unexpectedly had no segments.
    MissingRuleSegment,
    /// Splitting before the minimum representable civil date would underflow.
    AnchorDateUnderflow {
        /// The requested split anchor.
        anchor_date: i32,
    },
    /// Adjacent segment ordering or effective ranges overlapped.
    OverlappingRuleSegments {
        /// Earlier segment effective start date.
        left_start_date: i32,
        /// Later segment effective start date.
        right_start_date: i32,
    },
    /// One civil boundary was incorrectly marked as both a gap and fold adjustment.
    ContradictoryDstAdjustment,
    /// A resolved personal-schedule range overlaps an existing range.
    ResolvedScheduleConflict {
        /// Position of the first conflicting existing range.
        existing_index: usize,
        /// The conflicting existing range.
        interval: HalfOpenInterval,
    },
}

impl fmt::Display for ScheduleValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyLabel => formatter.write_str("schedule label must not be empty"),
            Self::EmptyTimeZoneId => {
                formatter.write_str("schedule time-zone identifier must not be empty")
            }
            Self::InvalidWeekdayMask { weekday_mask } => {
                write!(
                    formatter,
                    "invalid schedule weekday mask {weekday_mask:#09b}"
                )
            }
            Self::InvalidSecondOfDay {
                start_second_of_day,
                end_second_of_day,
            } => write!(
                formatter,
                "schedule seconds of day must be below {SECONDS_PER_DAY}, got {start_second_of_day} and {end_second_of_day}"
            ),
            Self::EqualCivilBoundaries { second_of_day } => write!(
                formatter,
                "schedule start and end are both at local second {second_of_day}; 24-hour inference is not allowed"
            ),
            Self::InvertedEffectiveDateRange {
                start_date,
                end_date,
            } => write!(
                formatter,
                "schedule effective end date {end_date:?} is before start date {start_date}"
            ),
            Self::ResolvedOccurrenceNotPositive { start, end } => write!(
                formatter,
                "resolved schedule occurrence must be positive, got [{}, {})",
                start.get(),
                end.get()
            ),
            Self::EditRequiresRepeatingRule => {
                formatter.write_str("occurrence edit scopes require a repeating schedule")
            }
            Self::NoRuleSegmentForAnchor { anchor_date } => {
                write!(
                    formatter,
                    "no schedule rule segment covers anchor date {anchor_date}"
                )
            }
            Self::MissingRuleSegment => {
                formatter.write_str("repeating schedule has no rule segment")
            }
            Self::AnchorDateUnderflow { anchor_date } => write!(
                formatter,
                "cannot end a rule segment before minimum anchor date {anchor_date}"
            ),
            Self::OverlappingRuleSegments {
                left_start_date,
                right_start_date,
            } => write!(
                formatter,
                "schedule rule segments starting at {left_start_date} and {right_start_date} overlap"
            ),
            Self::ContradictoryDstAdjustment => {
                formatter.write_str("a resolved civil boundary cannot be both a DST gap and fold")
            }
            Self::ResolvedScheduleConflict {
                existing_index,
                interval,
            } => write!(
                formatter,
                "resolved schedule conflicts with existing interval {existing_index} [{}, {})",
                interval.start().get(),
                interval.end().get()
            ),
        }
    }
}

impl std::error::Error for ScheduleValidationError {}

#[cfg(test)]
mod tests {
    use super::{ScheduleRule, ScheduleValidationError};
    use crate::{HalfOpenInterval, UtcMicros};

    const MONDAY: u8 = 0b000_0001;
    const WEEKDAYS: u8 = 0b001_1111;

    fn interval(start: i64, end: i64) -> HalfOpenInterval {
        HalfOpenInterval::try_new(UtcMicros::new(start), UtcMicros::new(end))
            .expect("test interval is positive")
    }

    fn repeating_rule() -> ScheduleRule {
        ScheduleRule::repeating(
            "Productive time",
            None,
            WEEKDAYS,
            9 * 3_600,
            11 * 3_600,
            100,
            None,
            "Asia/Karachi",
        )
        .expect("valid recurring fixture")
    }

    #[test]
    fn adjacent_resolved_ranges_are_valid_but_positive_intersection_is_rejected() {
        let existing = [interval(100, 200)];
        assert_eq!(
            ScheduleRule::validate_resolved_overlap(interval(200, 300), &existing),
            Ok(())
        );
        assert_eq!(
            ScheduleRule::validate_resolved_overlap(interval(199, 300), &existing),
            Err(ScheduleValidationError::ResolvedScheduleConflict {
                existing_index: 0,
                interval: existing[0],
            })
        );
    }

    #[test]
    fn earlier_end_clock_becomes_explicitly_overnight_and_equal_clock_is_rejected() {
        let overnight = ScheduleRule::repeating(
            "Reading",
            None,
            MONDAY,
            23 * 3_600,
            3_600,
            100,
            None,
            "Asia/Karachi",
        )
        .expect("earlier end clock is overnight");
        assert_eq!(overnight.end_day_offset(), Some(1));
        assert_eq!(
            ScheduleRule::repeating(
                "Ambiguous",
                None,
                MONDAY,
                12 * 3_600,
                12 * 3_600,
                100,
                None,
                "Asia/Karachi",
            ),
            Err(ScheduleValidationError::EqualCivilBoundaries {
                second_of_day: 12 * 3_600,
            })
        );
    }

    #[test]
    fn only_this_date_creates_skip_and_resolved_override_without_changing_other_rules() {
        let mut rule = repeating_rule();
        rule.skip_only_this_date(105).expect("covered anchor");
        rule.override_only_this_date(110, interval(1_000, 2_000), false, false, false, false)
            .expect("covered anchor");

        assert!(rule.is_skipped_on(105));
        assert_eq!(rule.resolved_override_on(110), Some(interval(1_000, 2_000)));
        assert_eq!(rule.exception_count(), 2);
        assert!(rule.applies_on_anchor_date(111, 0));
    }

    #[test]
    fn this_and_future_splits_at_anchor_and_preserves_existing_exceptions() {
        let mut rule = repeating_rule();
        rule.skip_only_this_date(105).expect("covered anchor");
        rule.change_this_and_future(
            120,
            "London hours",
            None,
            MONDAY,
            8 * 3_600,
            10 * 3_600,
            "Europe/London",
        )
        .expect("valid future split");

        assert_eq!(rule.segment_count(), 2);
        assert_eq!(rule.time_zone_for_anchor_date(119), Some("Asia/Karachi"));
        assert_eq!(rule.time_zone_for_anchor_date(120), Some("Europe/London"));
        assert!(rule.is_skipped_on(105));
    }

    #[test]
    fn persistence_segments_round_trip_a_future_only_rule_split() {
        let mut rule = repeating_rule();
        rule.change_this_and_future(
            120,
            "London hours",
            None,
            MONDAY,
            8 * 3_600,
            10 * 3_600,
            "Europe/London",
        )
        .expect("valid future split");

        let restored = ScheduleRule::try_restore_repeating(rule.segments())
            .expect("persisted civil segments should restore");
        assert_eq!(restored.segment_count(), 2);
        assert_eq!(
            restored.time_zone_for_anchor_date(119),
            Some("Asia/Karachi")
        );
        assert_eq!(
            restored.time_zone_for_anchor_date(120),
            Some("Europe/London")
        );
    }

    #[test]
    fn every_occurrence_replaces_segments_and_retains_occurrence_exceptions() {
        let mut rule = repeating_rule();
        rule.skip_only_this_date(105).expect("covered anchor");
        rule.change_this_and_future(
            120,
            "London hours",
            None,
            MONDAY,
            8 * 3_600,
            10 * 3_600,
            "Europe/London",
        )
        .expect("valid future split");
        rule.replace_every_occurrence(
            "Every day review",
            None,
            WEEKDAYS,
            7 * 3_600,
            9 * 3_600,
            "Etc/UTC",
        )
        .expect("valid replacement");

        assert_eq!(rule.segment_count(), 1);
        assert_eq!(rule.label(), "Every day review");
        assert_eq!(rule.time_zone_for_anchor_date(120), Some("Etc/UTC"));
        assert!(rule.is_skipped_on(105));
    }

    #[test]
    fn gap_and_fold_resolution_inputs_are_retained_as_occurrence_adjustments() {
        let mut rule = repeating_rule();
        rule.override_only_this_date(110, interval(2_000, 3_000), true, false, false, false)
            .expect("gap-adjusted range is positive");
        rule.override_only_this_date(115, interval(3_000, 4_000), false, true, false, false)
            .expect("fold-adjusted range is positive");

        assert!(rule.has_gap_adjustment_on(110));
        assert!(rule.has_fold_adjustment_on(115));
        assert_eq!(
            ScheduleRule::resolved_utc_range(UtcMicros::new(5), UtcMicros::new(5)),
            Err(ScheduleValidationError::ResolvedOccurrenceNotPositive {
                start: UtcMicros::new(5),
                end: UtcMicros::new(5),
            })
        );
    }

    #[test]
    fn zone_change_creates_a_future_only_rule_segment_at_the_supplied_anchor_boundary() {
        let mut rule = repeating_rule();
        rule.change_time_zone_for_future(120, "Europe/London")
            .expect("covered first future anchor");

        assert_eq!(rule.segment_count(), 2);
        assert_eq!(rule.time_zone_for_anchor_date(119), Some("Asia/Karachi"));
        assert_eq!(rule.time_zone_for_anchor_date(120), Some("Europe/London"));
        assert!(rule.applies_on_anchor_date(120, 0));
    }

    #[test]
    fn contradictory_gap_and_fold_metadata_is_rejected() {
        let mut rule = repeating_rule();
        assert_eq!(
            rule.override_only_this_date(110, interval(2_000, 3_000), true, true, false, false),
            Err(ScheduleValidationError::ContradictoryDstAdjustment)
        );
    }
}
