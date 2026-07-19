//! Exact, non-color timeline details and action routing.
//!
//! Details are a small UI-owned copy of the clicked or hovered presentation interval. They retain
//! the raw identity and both clipped and canonical times when the pointer identifies a source
//! fragment, but never own or mutate the immutable projection. Schedule details deliberately
//! require a caller-supplied identity because the current timeline snapshot has no schedule
//! occurrence contract.

use openmanic_application::{
    ActivityStateValue, ApplicationBandValue, CategoryBandValue, TimelineRawFragment,
    TimelineRawIntervalId,
};
use openmanic_domain::{ActivityState, HalfOpenInterval, TrackerRunId, UtcMicros};

use crate::{TodayAction, TodayCategoryFilter};

/// The independently segmented band that supplied a timeline detail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimelineBand {
    /// Category assignment or an explicit non-application state.
    Category,
    /// Canonical activity state.
    Activity,
    /// Application identity or an explicit no-application state.
    Application,
}

impl TimelineBand {
    /// Returns the visible, non-color label for this band.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Category => "Category",
            Self::Activity => "Activity state",
            Self::Application => "Application",
        }
    }
}

/// The exact, typed value shown in a persistent or transient detail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimelineDetailValue {
    /// A category-band value.
    Category(CategoryBandValue),
    /// An activity-band value.
    Activity(ActivityStateValue),
    /// An application-band value.
    Application(ApplicationBandValue),
}

impl TimelineDetailValue {
    /// Returns an ordinary-language, non-color description of the selected value.
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Self::Category(CategoryBandValue::Category(category_id)) => {
                format!("Category ({})", category_id.to_lowercase_hex())
            }
            Self::Category(CategoryBandValue::Uncategorized) => "Uncategorized".to_owned(),
            Self::Category(CategoryBandValue::NonApplicationState(state)) => {
                format!("No category: {}", activity_state_label(state))
            }
            Self::Activity(value) => activity_state_label(value.state()).to_owned(),
            Self::Application(ApplicationBandValue::Application(application_id)) => {
                format!("Application ({})", application_id.to_lowercase_hex())
            }
            Self::Application(ApplicationBandValue::NoApplication(state)) => {
                format!("No application: {}", activity_state_label(state))
            }
            Self::Application(ApplicationBandValue::UnresolvedApplication) => {
                "Unresolved application".to_owned()
            }
        }
    }

    /// Returns the visible action appropriate to this exact presentation value.
    ///
    /// The returned value is an action for the caller to reduce later. Selecting a detail never
    /// invokes this method or mutates an application/category assignment.
    #[must_use]
    pub const fn action(self) -> Option<TodayAction> {
        match self {
            Self::Category(CategoryBandValue::Category(category_id)) => {
                Some(TodayAction::AddCategoryFilter {
                    filter: TodayCategoryFilter::Category(category_id),
                })
            }
            Self::Category(CategoryBandValue::Uncategorized) => {
                Some(TodayAction::AddCategoryFilter {
                    filter: TodayCategoryFilter::Uncategorized,
                })
            }
            Self::Category(CategoryBandValue::NonApplicationState(state))
            | Self::Application(ApplicationBandValue::NoApplication(state)) => {
                Some(TodayAction::AddActivityStateFilter { state })
            }
            Self::Activity(value) => Some(TodayAction::AddActivityStateFilter {
                state: value.state(),
            }),
            Self::Application(ApplicationBandValue::Application(application_id)) => {
                Some(TodayAction::AddApplicationFilter { application_id })
            }
            Self::Application(ApplicationBandValue::UnresolvedApplication) => None,
        }
    }

    /// Returns the action-control label when this value can narrow the timeline.
    #[must_use]
    pub const fn action_label(self) -> Option<&'static str> {
        match self {
            Self::Category(CategoryBandValue::Category(_) | CategoryBandValue::Uncategorized) => {
                Some("Filter this category")
            }
            Self::Application(ApplicationBandValue::Application(_)) => {
                Some("Filter this application")
            }
            Self::Category(CategoryBandValue::NonApplicationState(_))
            | Self::Activity(_)
            | Self::Application(ApplicationBandValue::NoApplication(_)) => {
                Some("Filter this activity state")
            }
            Self::Application(ApplicationBandValue::UnresolvedApplication) => None,
        }
    }
}

/// A stable source fragment captured by an exact pointer hit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimelineRawDetail {
    raw_id: TimelineRawIntervalId,
    tracker_run_id: TrackerRunId,
    raw_range: HalfOpenInterval,
    visible_range: HalfOpenInterval,
}

impl TimelineRawDetail {
    pub(crate) const fn from_fragment(fragment: TimelineRawFragment) -> Self {
        Self {
            raw_id: fragment.raw_id(),
            tracker_run_id: fragment.tracker_run_id(),
            raw_range: fragment.raw_range(),
            visible_range: fragment.visible_range(),
        }
    }

    /// Returns the stable raw interval identity selected at the pointer instant.
    #[must_use]
    pub const fn raw_id(self) -> TimelineRawIntervalId {
        self.raw_id
    }

    /// Returns the tracker run that emitted the raw interval.
    #[must_use]
    pub const fn tracker_run_id(self) -> TrackerRunId {
        self.tracker_run_id
    }

    /// Returns the original, un-clipped raw start and end times.
    #[must_use]
    pub const fn raw_range(self) -> HalfOpenInterval {
        self.raw_range
    }

    /// Returns the exact source contribution visible in the current snapshot.
    #[must_use]
    pub const fn visible_range(self) -> HalfOpenInterval {
        self.visible_range
    }
}

/// One hover or persistent selection detail from an immutable timeline snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimelineDetail {
    band: TimelineBand,
    value: TimelineDetailValue,
    instant: UtcMicros,
    interval_range: HalfOpenInterval,
    raw: Option<TimelineRawDetail>,
}

impl TimelineDetail {
    pub(crate) fn new(
        band: TimelineBand,
        value: TimelineDetailValue,
        instant: UtcMicros,
        interval_range: HalfOpenInterval,
        fragments: &[TimelineRawFragment],
    ) -> Self {
        let raw = fragments
            .iter()
            .copied()
            .find(|fragment| contains(fragment.visible_range(), instant))
            .map(TimelineRawDetail::from_fragment);
        Self {
            band,
            value,
            instant,
            interval_range,
            raw,
        }
    }

    /// Returns the band which supplied the detail.
    #[must_use]
    pub const fn band(self) -> TimelineBand {
        self.band
    }

    /// Returns the selected presentation value without relying on its color.
    #[must_use]
    pub const fn value(self) -> TimelineDetailValue {
        self.value
    }

    /// Returns the exact UTC instant selected by the pointer conversion.
    #[must_use]
    pub const fn instant(self) -> UtcMicros {
        self.instant
    }

    /// Returns the immutable presentation interval shown by this detail.
    #[must_use]
    pub const fn interval_range(self) -> HalfOpenInterval {
        self.interval_range
    }

    /// Returns the source identity and raw times when the interval represents raw evidence.
    ///
    /// Synthesized `UnknownMissing` intervals correctly return `None`; the renderer never
    /// manufactures an identity for an uncovered interval.
    #[must_use]
    pub const fn raw(self) -> Option<TimelineRawDetail> {
        self.raw
    }

    /// Returns the deferred UI action appropriate to this selection, if any.
    #[must_use]
    pub const fn action(self) -> Option<TodayAction> {
        self.value.action()
    }
}

/// A schedule detail requires a caller-owned identity.
///
/// `TimelineSnapshot` currently contains no schedule occurrence values, so OM-291 exposes this
/// typed holder without inventing a placeholder ID. A later schedule projection can pass its own
/// stable occurrence identity directly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimelineScheduleDetail<I> {
    identity: I,
    range: HalfOpenInterval,
}

impl<I> TimelineScheduleDetail<I> {
    /// Creates a schedule detail from an authoritative caller-supplied occurrence identity.
    #[must_use]
    pub const fn new(identity: I, range: HalfOpenInterval) -> Self {
        Self { identity, range }
    }

    /// Returns the supplied occurrence identity unchanged.
    #[must_use]
    pub const fn identity(&self) -> &I {
        &self.identity
    }

    /// Returns the exact occurrence range.
    #[must_use]
    pub const fn range(&self) -> HalfOpenInterval {
        self.range
    }
}

fn contains(range: HalfOpenInterval, instant: UtcMicros) -> bool {
    range.start() <= instant && instant < range.end()
}

const fn activity_state_label(state: ActivityState) -> &'static str {
    match state {
        ActivityState::Active => "Active",
        ActivityState::Idle => "Idle",
        ActivityState::PausedByUser => "Paused by you",
        ActivityState::Excluded => "Excluded",
        ActivityState::Unavailable => "Tracking unavailable",
        ActivityState::PoweredOff => "Powered Off",
        ActivityState::UnknownMissing => "Unknown activity",
    }
}

#[cfg(test)]
mod tests {
    use openmanic_application::{ActivityStateValue, ApplicationBandValue, CategoryBandValue};
    use openmanic_domain::{ActivityState, ApplicationId, CategoryId};

    use super::{TimelineDetailValue, TimelineScheduleDetail};
    use crate::{TodayAction, TodayCategoryFilter};

    #[test]
    fn category_and_application_actions_are_typed_filter_intents() {
        let category_id = CategoryId::from_bytes([3; 16]);
        let application_id = ApplicationId::from_bytes([4; 16]);
        assert_eq!(
            TimelineDetailValue::Category(CategoryBandValue::Category(category_id)).action(),
            Some(TodayAction::AddCategoryFilter {
                filter: TodayCategoryFilter::Category(category_id),
            })
        );
        assert_eq!(
            TimelineDetailValue::Application(ApplicationBandValue::Application(application_id))
                .action(),
            Some(TodayAction::AddApplicationFilter { application_id })
        );
        assert_eq!(
            TimelineDetailValue::Activity(ActivityStateValue::PoweredOff).action(),
            Some(TodayAction::AddActivityStateFilter {
                state: ActivityState::PoweredOff,
            })
        );
    }

    #[test]
    fn schedule_detail_requires_the_caller_owned_identity() {
        let range = openmanic_domain::HalfOpenInterval::try_new(
            openmanic_domain::UtcMicros::new(10),
            openmanic_domain::UtcMicros::new(20),
        )
        .expect("fixture range is positive");
        let detail = TimelineScheduleDetail::new("occurrence-7", range);
        assert_eq!(detail.identity(), &"occurrence-7");
        assert_eq!(detail.range(), range);
    }
}
