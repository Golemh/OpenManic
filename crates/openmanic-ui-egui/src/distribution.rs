//! Private time-distribution widget support.
//!
//! The application summary snapshot for this widget has not been frozen yet.
//! These deliberately local types model the already approved presentation: a
//! labeled stacked bar with an exact-value list.  Keeping the input immutable
//! and local lets a future application snapshot adapter replace this boundary
//! without making the renderer a storage or aggregation owner.

#![allow(
    dead_code,
    reason = "OM-293 deliberately supplies a private replaceable model before OM-299 wires its application snapshot"
)]

use std::{collections::BTreeMap, sync::Arc};

use crate::model::{DataLimitation, PresentableData};
use egui::{Color32, Rect, Vec2};

/// The user-visible dimension used to group a distribution.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DistributionGrouping {
    /// Groups included time by category.
    Category,
    /// Groups included time by application.
    Application,
    /// Groups included time by tracking state.
    ActivityState,
}

impl DistributionGrouping {
    /// Returns the ordinary-language label for the active grouping.
    #[must_use]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Category => "Categories",
            Self::Application => "Applications",
            Self::ActivityState => "Activity states",
        }
    }
}

/// One immutable, already-filtered input contribution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistributionContribution {
    key: String,
    label: String,
    included_micros: u64,
}

impl DistributionContribution {
    /// Creates one contribution identified by an application-owned stable key.
    #[must_use]
    pub fn new(key: impl Into<String>, label: impl Into<String>, included_micros: u64) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            included_micros,
        }
    }
}

/// One grouped exact value displayed by the widget.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DistributionGroup {
    key: String,
    label: String,
    included_micros: u64,
}

impl DistributionGroup {
    /// Returns the stable presentation identity of this group.
    #[must_use]
    pub(crate) fn key(&self) -> &str {
        &self.key
    }

    /// Returns the text label; this is never encoded only by a paint color.
    #[must_use]
    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    /// Returns the exact included duration in microseconds.
    #[must_use]
    pub(crate) const fn included_micros(&self) -> u64 {
        self.included_micros
    }

    /// Formats this exact value without consulting a color or chart segment.
    #[must_use]
    pub(crate) fn exact_value_label(&self) -> String {
        format_duration(self.included_micros)
    }
}

/// A failure while constructing an immutable distribution snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DistributionBuildError {
    /// A key had no stable identity.
    EmptyKey,
    /// A group cannot be understood without an ordinary-language label.
    EmptyLabel,
    /// The same stable key supplied conflicting labels.
    ConflictingLabel,
    /// Summing the supplied durations would overflow.
    DurationOverflow,
}

/// Immutable presentation-ready distribution input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistributionSnapshot {
    grouping: DistributionGrouping,
    total_included_micros: u64,
    groups: Arc<[DistributionGroup]>,
}

impl DistributionSnapshot {
    /// Groups contributions deterministically, preserving exact total duration.
    ///
    /// # Errors
    ///
    /// Returns [`DistributionBuildError`] when a contribution lacks a stable key or label,
    /// conflicts with a prior label for the same key, or would overflow the exact total.
    pub fn try_from_contributions(
        grouping: DistributionGrouping,
        contributions: impl IntoIterator<Item = DistributionContribution>,
    ) -> Result<Self, DistributionBuildError> {
        let mut grouped = BTreeMap::<String, (String, u64)>::new();
        for contribution in contributions {
            if contribution.key.is_empty() {
                return Err(DistributionBuildError::EmptyKey);
            }
            if contribution.label.is_empty() {
                return Err(DistributionBuildError::EmptyLabel);
            }

            let Some((label, duration)) = grouped.get_mut(&contribution.key) else {
                grouped.insert(
                    contribution.key,
                    (contribution.label, contribution.included_micros),
                );
                continue;
            };
            if *label != contribution.label {
                return Err(DistributionBuildError::ConflictingLabel);
            }
            *duration = duration
                .checked_add(contribution.included_micros)
                .ok_or(DistributionBuildError::DurationOverflow)?;
        }

        let mut groups = grouped
            .into_iter()
            .map(|(key, (label, included_micros))| DistributionGroup {
                key,
                label,
                included_micros,
            })
            .collect::<Vec<_>>();
        groups.sort_by(|left, right| {
            right
                .included_micros
                .cmp(&left.included_micros)
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.key.cmp(&right.key))
        });
        let total_included_micros = groups.iter().try_fold(0_u64, |total, group| {
            total
                .checked_add(group.included_micros)
                .ok_or(DistributionBuildError::DurationOverflow)
        })?;

        Ok(Self {
            grouping,
            total_included_micros,
            groups: Arc::from(groups),
        })
    }

    /// Returns the active grouping label.
    #[must_use]
    pub(crate) const fn grouping(&self) -> DistributionGrouping {
        self.grouping
    }

    /// Returns the exact total represented by all groups.
    #[must_use]
    pub(crate) const fn total_included_micros(&self) -> u64 {
        self.total_included_micros
    }

    /// Returns the deterministic grouped values.
    #[must_use]
    pub(crate) fn groups(&self) -> &[DistributionGroup] {
        &self.groups
    }
}

/// The rectangle-dependent presentation selected by the caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DistributionLayout {
    /// A full labeled list and stacked bar for wider widgets.
    Expanded,
    /// The compact labeled stacked-bar treatment for narrow widget spans.
    Compact { max_named_groups: usize },
}

/// One text-bearing segment in a distribution presentation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DistributionSegment {
    label: String,
    included_micros: u64,
    grouped_count: usize,
}

impl DistributionSegment {
    /// Returns the ordinary-language label rendered alongside the segment.
    #[must_use]
    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    /// Returns the exact value represented by this segment.
    #[must_use]
    pub(crate) const fn included_micros(&self) -> u64 {
        self.included_micros
    }

    /// Returns the number of source groups folded into this segment.
    #[must_use]
    pub(crate) const fn grouped_count(&self) -> usize {
        self.grouped_count
    }

    /// Returns exact value text that remains meaningful without color.
    #[must_use]
    pub(crate) fn exact_value_label(&self) -> String {
        format_duration(self.included_micros)
    }
}

/// A ready-to-paint stack and textual summary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DistributionPresentation {
    grouping_label: &'static str,
    total_included_micros: u64,
    segments: Arc<[DistributionSegment]>,
}

impl DistributionPresentation {
    /// Builds a compact or expanded presentation without changing source data.
    #[must_use]
    pub(crate) fn from_snapshot(
        snapshot: &DistributionSnapshot,
        layout: DistributionLayout,
    ) -> Self {
        let named_count = match layout {
            DistributionLayout::Expanded => snapshot.groups.len(),
            DistributionLayout::Compact { max_named_groups } => {
                max_named_groups.min(snapshot.groups.len())
            }
        };
        let mut segments = snapshot.groups[..named_count]
            .iter()
            .map(|group| DistributionSegment {
                label: group.label.clone(),
                included_micros: group.included_micros,
                grouped_count: 1,
            })
            .collect::<Vec<_>>();
        if named_count < snapshot.groups.len() {
            let remaining = snapshot.groups[named_count..]
                .iter()
                .fold(0_u64, |total, group| {
                    // The validated snapshot guarantees this sum cannot overflow.
                    total + group.included_micros
                });
            segments.push(DistributionSegment {
                label: format!("Remaining ({})", snapshot.groups.len() - named_count),
                included_micros: remaining,
                grouped_count: snapshot.groups.len() - named_count,
            });
        }

        Self {
            grouping_label: snapshot.grouping.label(),
            total_included_micros: snapshot.total_included_micros,
            segments: Arc::from(segments),
        }
    }

    /// Returns the visible grouping label.
    #[must_use]
    pub(crate) const fn grouping_label(&self) -> &'static str {
        self.grouping_label
    }

    /// Returns the visible exact total.
    #[must_use]
    pub(crate) const fn total_included_micros(&self) -> u64 {
        self.total_included_micros
    }

    /// Returns the exact total as ordinary text.
    #[must_use]
    pub(crate) fn total_label(&self) -> String {
        format_duration(self.total_included_micros)
    }

    /// Returns text-bearing stacked-bar segments.
    #[must_use]
    pub(crate) fn segments(&self) -> &[DistributionSegment] {
        &self.segments
    }
}

/// A complete, non-mutating visual state for the distribution renderer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DistributionRenderModel {
    /// No immutable snapshot has arrived yet.
    InitialLoading,
    /// A current or preserved distribution stays visible.
    Content {
        /// The stacked bar and exact text values to draw.
        presentation: DistributionPresentation,
        /// Indicates a replacement snapshot is in progress.
        refreshing: bool,
        /// States that constrain interpretation without hiding usable values.
        limitations: Vec<DataLimitation>,
        /// A nontechnical recovery notice, when a replacement succeeded.
        recovery_notice: Option<String>,
    },
    /// The requested range has no values for the stated reason.
    Empty { message: String },
    /// A failure with any still-valid prior presentation preserved.
    Failed {
        /// The presentation remains visible when prior data existed.
        prior: Option<DistributionPresentation>,
        /// A safe, ordinary-language error explanation.
        message: String,
    },
}

/// Converts the generic immutable-data state into renderer-ready text and segments.
#[must_use]
pub(crate) fn render_model(
    data: &PresentableData<DistributionSnapshot>,
    layout: DistributionLayout,
) -> DistributionRenderModel {
    match data {
        PresentableData::InitialLoading => DistributionRenderModel::InitialLoading,
        PresentableData::Ready(value) => content(value, layout, false, Vec::new(), None),
        PresentableData::Refreshing { prior, .. } => content(prior, layout, true, Vec::new(), None),
        PresentableData::Empty(reason) => DistributionRenderModel::Empty {
            message: reason.message().to_owned(),
        },
        PresentableData::Partial { value, limitations } => {
            content(value, layout, false, limitations.clone(), None)
        }
        PresentableData::Failed { prior, error } => DistributionRenderModel::Failed {
            prior: prior
                .as_ref()
                .map(|snapshot| DistributionPresentation::from_snapshot(snapshot, layout)),
            message: error.message(),
        },
        PresentableData::Recovered { value, notice } => {
            content(value, layout, false, Vec::new(), Some(notice.clone()))
        }
    }
}

fn content(
    snapshot: &Arc<DistributionSnapshot>,
    layout: DistributionLayout,
    refreshing: bool,
    limitations: Vec<DataLimitation>,
    recovery_notice: Option<String>,
) -> DistributionRenderModel {
    DistributionRenderModel::Content {
        presentation: DistributionPresentation::from_snapshot(snapshot, layout),
        refreshing,
        limitations,
        recovery_notice,
    }
}

/// Paints the approved compact time-distribution treatment from an immutable snapshot.
///
/// The widget keeps exact labels and values visible beneath the stacked bar, so no meaning
/// depends on color alone. It performs no aggregation or application-port work during a frame.
pub fn render_distribution_snapshot(ui: &mut egui::Ui, snapshot: &DistributionSnapshot) {
    let data = PresentableData::Ready(Arc::new(snapshot.clone()));
    let DistributionRenderModel::Content { presentation, .. } = render_model(
        &data,
        DistributionLayout::Compact {
            max_named_groups: 4,
        },
    ) else {
        return;
    };

    ui.label(format!(
        "{}: {}",
        presentation.grouping_label(),
        presentation.total_label()
    ));
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 14.0), egui::Sense::hover());
    let palette = [
        Color32::from_rgb(121, 151, 255),
        Color32::from_rgb(107, 201, 139),
        Color32::from_rgb(236, 190, 93),
        Color32::from_rgb(197, 128, 255),
        Color32::from_rgb(176, 188, 208),
    ];
    let visual_total = presentation
        .segments()
        .iter()
        .map(|segment| visual_minutes(segment.included_micros()))
        .map(f32::from)
        .sum::<f32>()
        .max(1.0);
    let mut left = rect.left();
    for (index, segment) in presentation.segments().iter().enumerate() {
        let width =
            rect.width() * (f32::from(visual_minutes(segment.included_micros())) / visual_total);
        let right = if index + 1 == presentation.segments().len() {
            rect.right()
        } else {
            (left + width).min(rect.right())
        };
        let segment_rect = Rect::from_min_max(
            egui::pos2(left, rect.top()),
            egui::pos2(right, rect.bottom()),
        );
        ui.painter()
            .rect_filled(segment_rect, 2.0, palette[index % palette.len()]);
        left = right;
    }
    for segment in presentation.segments() {
        ui.label(format!(
            "{}: {}",
            segment.label(),
            segment.exact_value_label()
        ));
    }
}

fn visual_minutes(micros: u64) -> u16 {
    const MICROS_PER_MINUTE: u64 = 60_000_000;
    if micros == 0 {
        return 0;
    }
    let rounded_up = micros.saturating_add(MICROS_PER_MINUTE.saturating_sub(1)) / MICROS_PER_MINUTE;
    u16::try_from(rounded_up).unwrap_or(u16::MAX)
}

fn format_duration(micros: u64) -> String {
    let seconds = micros / 1_000_000;
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let remaining_seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {remaining_seconds}s")
    } else {
        format!("{remaining_seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use openmanic_application::{ApplicationError, ApplicationPort, PortFailureReason};

    use crate::model::{DataLimitation, EmptyReason, PresentableData, UserFacingError};

    use super::{
        DistributionBuildError, DistributionContribution, DistributionGrouping, DistributionLayout,
        DistributionRenderModel, DistributionSnapshot, render_model,
    };

    fn snapshot() -> DistributionSnapshot {
        DistributionSnapshot::try_from_contributions(
            DistributionGrouping::Category,
            [
                DistributionContribution::new("work", "Work", 3_600_000_000),
                DistributionContribution::new("play", "Play", 1_800_000_000),
                DistributionContribution::new("work", "Work", 600_000_000),
                DistributionContribution::new("admin", "Admin", 300_000_000),
            ],
        )
        .expect("valid deterministic snapshot")
    }

    #[test]
    fn grouping_merges_duplicate_keys_and_preserves_exact_total() {
        let snapshot = snapshot();

        assert_eq!(snapshot.grouping().label(), "Categories");
        assert_eq!(snapshot.total_included_micros(), 6_300_000_000);
        assert_eq!(snapshot.groups().len(), 3);
        assert_eq!(snapshot.groups()[0].label(), "Work");
        assert_eq!(snapshot.groups()[0].included_micros(), 4_200_000_000);
    }

    #[test]
    fn labels_and_exact_values_are_available_without_color() {
        let snapshot = snapshot();
        let presentation =
            super::DistributionPresentation::from_snapshot(&snapshot, DistributionLayout::Expanded);

        assert_eq!(presentation.grouping_label(), "Categories");
        assert_eq!(presentation.total_label(), "1h 45m");
        assert_eq!(presentation.segments()[0].label(), "Work");
        assert_eq!(presentation.segments()[0].exact_value_label(), "1h 10m");
    }

    #[test]
    fn compact_presentation_folds_remaining_groups_without_losing_total() {
        let snapshot = snapshot();
        let presentation = super::DistributionPresentation::from_snapshot(
            &snapshot,
            DistributionLayout::Compact {
                max_named_groups: 1,
            },
        );

        assert_eq!(presentation.segments().len(), 2);
        assert_eq!(presentation.segments()[1].label(), "Remaining (2)");
        assert_eq!(presentation.segments()[1].included_micros(), 2_100_000_000);
        assert_eq!(
            presentation
                .segments()
                .iter()
                .map(super::DistributionSegment::included_micros)
                .sum::<u64>(),
            presentation.total_included_micros()
        );
    }

    #[test]
    fn invalid_or_conflicting_group_identity_is_rejected() {
        assert_eq!(
            DistributionSnapshot::try_from_contributions(
                DistributionGrouping::Category,
                [
                    DistributionContribution::new("same", "One", 1),
                    DistributionContribution::new("same", "Two", 1),
                ],
            ),
            Err(DistributionBuildError::ConflictingLabel)
        );
    }

    #[test]
    fn every_presentable_data_state_has_an_explicit_render_model() {
        let value = Arc::new(snapshot());
        let layout = DistributionLayout::Compact {
            max_named_groups: 2,
        };
        assert!(matches!(
            render_model(&PresentableData::InitialLoading, layout),
            DistributionRenderModel::InitialLoading
        ));
        assert!(matches!(
            render_model(&PresentableData::Ready(Arc::clone(&value)), layout),
            DistributionRenderModel::Content { refreshing: false, limitations, recovery_notice: None, .. } if limitations.is_empty()
        ));
        assert!(matches!(
            render_model(
                &PresentableData::Refreshing {
                    prior: Arc::clone(&value),
                    job: openmanic_application::JobId::new(4)
                },
                layout,
            ),
            DistributionRenderModel::Content {
                refreshing: true,
                ..
            }
        ));
        assert!(matches!(
            render_model(
                &PresentableData::Empty(EmptyReason::NoMatchingResults),
                layout
            ),
            DistributionRenderModel::Empty { .. }
        ));
        assert!(matches!(
            render_model(
                &PresentableData::Partial { value: Arc::clone(&value), limitations: vec![DataLimitation::TrackingPaused] },
                layout,
            ),
            DistributionRenderModel::Content { limitations, .. } if limitations == vec![DataLimitation::TrackingPaused]
        ));
        assert!(matches!(
            render_model(
                &PresentableData::Failed {
                    prior: Some(Arc::clone(&value)),
                    error: UserFacingError::Application(ApplicationError::port_failure(
                        ApplicationPort::Projection,
                        PortFailureReason::Unavailable,
                    )),
                },
                layout,
            ),
            DistributionRenderModel::Failed { prior: Some(_), .. }
        ));
        assert!(matches!(
            render_model(
                &PresentableData::Recovered { value, notice: "Updated".to_owned() },
                layout,
            ),
            DistributionRenderModel::Content { recovery_notice: Some(notice), .. } if notice == "Updated"
        ));
    }
}
