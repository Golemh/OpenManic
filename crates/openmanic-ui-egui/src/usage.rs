//! Private application-usage widget support.
//!
//! The application layer supplies an already aggregated immutable snapshot. This
//! module deliberately neither derives usage from timeline data nor asks a port
//! for more information while rendering.

#![allow(
    dead_code,
    reason = "OM-299 wires this private renderer into the composed Today screen"
)]

use std::sync::Arc;

use egui::Ui;
use openmanic_application::JobId;
use openmanic_domain::ApplicationId;

use crate::model::{DataLimitation, EmptyReason, PresentableData};

/// One immutable application total supplied for the selected range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ApplicationUsage {
    application_id: ApplicationId,
    display_name: String,
    duration_us: u64,
}

impl ApplicationUsage {
    /// Creates one application total. The caller owns name resolution and aggregation.
    #[must_use]
    pub(crate) fn new(
        application_id: ApplicationId,
        display_name: String,
        duration_us: u64,
    ) -> Self {
        Self {
            application_id,
            display_name,
            duration_us,
        }
    }

    /// Returns the stable application identity for a future action-only interaction.
    #[must_use]
    pub(crate) const fn application_id(&self) -> ApplicationId {
        self.application_id
    }

    /// Returns the supplied display name.
    #[must_use]
    pub(crate) fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Returns the exact aggregated duration in microseconds.
    #[must_use]
    pub(crate) const fn duration_us(&self) -> u64 {
        self.duration_us
    }
}

/// Immutable usage input for one exact selected range.
///
/// `range_label` is supplied by the application/composition boundary because
/// civil-time formatting cannot be inferred safely from UTC values in a widget.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ApplicationUsageSnapshot {
    range_label: String,
    applications: Arc<[ApplicationUsage]>,
}

impl ApplicationUsageSnapshot {
    /// Creates an immutable application-usage snapshot.
    #[must_use]
    pub(crate) fn new(range_label: String, applications: Vec<ApplicationUsage>) -> Self {
        Self {
            range_label,
            applications: Arc::from(applications),
        }
    }

    /// Returns the exact selected-range label supplied by the caller.
    #[must_use]
    pub(crate) fn range_label(&self) -> &str {
        &self.range_label
    }

    /// Returns the unmodified immutable application totals.
    #[must_use]
    pub(crate) fn applications(&self) -> &[ApplicationUsage] {
        &self.applications
    }
}

/// A precise percentage represented as a fraction of the selected-range total.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UsagePercentage {
    numerator_us: u64,
    denominator_us: u64,
}

impl UsagePercentage {
    /// Returns the exact numerator duration.
    #[must_use]
    pub(crate) const fn numerator_us(self) -> u64 {
        self.numerator_us
    }

    /// Returns the exact denominator duration.
    #[must_use]
    pub(crate) const fn denominator_us(self) -> u64 {
        self.denominator_us
    }

    /// Returns a rounded-to-nearest hundredth display value without floating point.
    #[must_use]
    pub(crate) fn hundredths(self) -> u32 {
        if self.denominator_us == 0 {
            return 0;
        }
        let scaled = u128::from(self.numerator_us) * 10_000;
        let rounded = scaled + (u128::from(self.denominator_us) / 2);
        u32::try_from(rounded / u128::from(self.denominator_us)).unwrap_or(u32::MAX)
    }
}

/// One deterministic row displayed by the widget.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UsageRow {
    application_id: Option<ApplicationId>,
    label: String,
    duration_us: u64,
    percentage: UsagePercentage,
    remaining_count: usize,
}

impl UsageRow {
    /// Returns the application identity, or `None` for the remaining-items aggregate.
    #[must_use]
    pub(crate) const fn application_id(&self) -> Option<ApplicationId> {
        self.application_id
    }

    /// Returns the ordinary-language row label.
    #[must_use]
    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    /// Returns the exact displayed duration.
    #[must_use]
    pub(crate) const fn duration_us(&self) -> u64 {
        self.duration_us
    }

    /// Returns the exact percentage fraction.
    #[must_use]
    pub(crate) const fn percentage(&self) -> UsagePercentage {
        self.percentage
    }

    /// Returns how many applications this row represents; one for a normal row.
    #[must_use]
    pub(crate) const fn represented_application_count(&self) -> usize {
        if self.remaining_count == 0 {
            1
        } else {
            self.remaining_count
        }
    }
}

/// A non-mutating presentation state for the widget.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum UsagePresentationState {
    /// No snapshot has arrived yet.
    InitialLoading,
    /// A complete value is visible.
    Ready,
    /// A complete prior value remains visible while a replacement is requested.
    Refreshing { job: JobId },
    /// The selected range has no usable values.
    Empty(EmptyReason),
    /// A usable value has limitations that must remain visible.
    Partial { limitations: Vec<DataLimitation> },
    /// A prior value may remain visible after a recoverable failure.
    Failed { message: String },
    /// A value replaced a prior recoverable failure.
    Recovered { notice: String },
}

/// The complete immutable render model for an application-usage widget.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UsagePresentation {
    state: UsagePresentationState,
    range_label: Option<String>,
    total_duration_us: u64,
    rows: Vec<UsageRow>,
}

impl UsagePresentation {
    /// Builds a presentation without mutating, requesting, or retaining the source snapshot.
    #[must_use]
    pub(crate) fn from_data(
        data: &PresentableData<ApplicationUsageSnapshot>,
        maximum_rows: usize,
    ) -> Self {
        match data {
            PresentableData::InitialLoading => {
                Self::without_value(UsagePresentationState::InitialLoading)
            }
            PresentableData::Empty(reason) => {
                Self::without_value(UsagePresentationState::Empty(*reason))
            }
            PresentableData::Ready(value) => {
                Self::from_snapshot(value, maximum_rows, UsagePresentationState::Ready)
            }
            PresentableData::Refreshing { prior, job } => Self::from_snapshot(
                prior,
                maximum_rows,
                UsagePresentationState::Refreshing { job: *job },
            ),
            PresentableData::Partial { value, limitations } => Self::from_snapshot(
                value,
                maximum_rows,
                UsagePresentationState::Partial {
                    limitations: limitations.clone(),
                },
            ),
            PresentableData::Failed { prior, error } => prior.as_ref().map_or_else(
                || {
                    Self::without_value(UsagePresentationState::Failed {
                        message: error.message(),
                    })
                },
                |value| {
                    Self::from_snapshot(
                        value,
                        maximum_rows,
                        UsagePresentationState::Failed {
                            message: error.message(),
                        },
                    )
                },
            ),
            PresentableData::Recovered { value, notice } => Self::from_snapshot(
                value,
                maximum_rows,
                UsagePresentationState::Recovered {
                    notice: notice.clone(),
                },
            ),
        }
    }

    fn without_value(state: UsagePresentationState) -> Self {
        Self {
            state,
            range_label: None,
            total_duration_us: 0,
            rows: Vec::new(),
        }
    }

    fn from_snapshot(
        snapshot: &ApplicationUsageSnapshot,
        maximum_rows: usize,
        state: UsagePresentationState,
    ) -> Self {
        let mut applications = snapshot.applications().to_vec();
        applications.sort_by(|left, right| {
            right
                .duration_us()
                .cmp(&left.duration_us())
                .then_with(|| left.display_name().cmp(right.display_name()))
                .then_with(|| left.application_id().cmp(&right.application_id()))
        });
        let total_duration_us = applications.iter().fold(0_u64, |total, item| {
            total.saturating_add(item.duration_us())
        });
        let shown_count = maximum_rows.min(applications.len());
        let mut rows: Vec<_> = applications[..shown_count]
            .iter()
            .map(|item| UsageRow {
                application_id: Some(item.application_id()),
                label: item.display_name().to_owned(),
                duration_us: item.duration_us(),
                percentage: UsagePercentage {
                    numerator_us: item.duration_us(),
                    denominator_us: total_duration_us,
                },
                remaining_count: 0,
            })
            .collect();
        if shown_count < applications.len() {
            let remaining = &applications[shown_count..];
            let duration_us = remaining.iter().fold(0_u64, |total, item| {
                total.saturating_add(item.duration_us())
            });
            rows.push(UsageRow {
                application_id: None,
                label: format!("Remaining applications ({})", remaining.len()),
                duration_us,
                percentage: UsagePercentage {
                    numerator_us: duration_us,
                    denominator_us: total_duration_us,
                },
                remaining_count: remaining.len(),
            });
        }
        Self {
            state,
            range_label: Some(snapshot.range_label().to_owned()),
            total_duration_us,
            rows,
        }
    }

    /// Returns the current non-mutating state.
    #[must_use]
    pub(crate) const fn state(&self) -> &UsagePresentationState {
        &self.state
    }

    /// Returns the exact label for the visible selected range, when data exists.
    #[must_use]
    pub(crate) fn range_label(&self) -> Option<&str> {
        self.range_label.as_deref()
    }

    /// Returns the exact sum of source durations.
    #[must_use]
    pub(crate) const fn total_duration_us(&self) -> u64 {
        self.total_duration_us
    }

    /// Returns deterministic visible rows.
    #[must_use]
    pub(crate) fn rows(&self) -> &[UsageRow] {
        &self.rows
    }
}

/// Paints a presentation already prepared from immutable data.
///
/// This intentionally does not submit actions or touch any application port.
pub(crate) fn render_usage(ui: &mut Ui, presentation: &UsagePresentation) {
    if let Some(range_label) = presentation.range_label() {
        ui.label(range_label);
        ui.label(format!("Total: {} us", presentation.total_duration_us()));
        for row in presentation.rows() {
            let percentage = row.percentage().hundredths();
            ui.label(format!(
                "{}: {} us ({}.{:02}%)",
                row.label(),
                row.duration_us(),
                percentage / 100,
                percentage % 100
            ));
        }
    }
    match presentation.state() {
        UsagePresentationState::InitialLoading => {
            ui.label("Loading application usage…");
        }
        UsagePresentationState::Refreshing { .. } => {
            ui.label("Refreshing application usage…");
        }
        UsagePresentationState::Empty(reason) => {
            ui.label(reason.message());
        }
        UsagePresentationState::Partial { limitations } => {
            for limitation in limitations {
                ui.label(limitation.message());
            }
        }
        UsagePresentationState::Failed { message } => {
            ui.label(message);
        }
        UsagePresentationState::Recovered { notice } => {
            ui.label(notice);
        }
        UsagePresentationState::Ready => {}
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use openmanic_application::{ApplicationError, ApplicationPort, JobId, PortFailureReason};
    use openmanic_domain::ApplicationId;

    use super::{
        ApplicationUsage, ApplicationUsageSnapshot, UsagePresentation, UsagePresentationState,
        UsageRow,
    };
    use crate::{DataLimitation, EmptyReason, PresentableData, UserFacingError};

    fn application(byte: u8, name: &str, duration_us: u64) -> ApplicationUsage {
        ApplicationUsage::new(
            ApplicationId::from_bytes([byte; 16]),
            name.to_owned(),
            duration_us,
        )
    }

    fn snapshot() -> ApplicationUsageSnapshot {
        ApplicationUsageSnapshot::new(
            "Friday, 17 May 2024, 09:00–17:00".to_owned(),
            vec![
                application(2, "Beta", 20),
                application(3, "Alpha", 20),
                application(1, "Gamma", 60),
            ],
        )
    }

    #[test]
    fn rows_sort_by_duration_then_name_then_identity_and_retain_exact_totals() {
        let presentation =
            UsagePresentation::from_data(&PresentableData::Ready(Arc::new(snapshot())), 3);

        assert_eq!(presentation.total_duration_us(), 100);
        assert_eq!(
            presentation.range_label(),
            Some("Friday, 17 May 2024, 09:00–17:00")
        );
        assert_eq!(
            presentation
                .rows()
                .iter()
                .map(UsageRow::label)
                .collect::<Vec<_>>(),
            ["Gamma", "Alpha", "Beta"]
        );
        assert_eq!(presentation.rows()[0].percentage().numerator_us(), 60);
        assert_eq!(presentation.rows()[0].percentage().denominator_us(), 100);
        assert_eq!(presentation.rows()[0].percentage().hundredths(), 6_000);
    }

    #[test]
    fn remaining_items_are_aggregated_without_losing_their_exact_total() {
        let presentation =
            UsagePresentation::from_data(&PresentableData::Ready(Arc::new(snapshot())), 1);

        assert_eq!(presentation.rows().len(), 2);
        assert_eq!(presentation.rows()[1].label(), "Remaining applications (2)");
        assert_eq!(presentation.rows()[1].application_id(), None);
        assert_eq!(presentation.rows()[1].represented_application_count(), 2);
        assert_eq!(presentation.rows()[1].duration_us(), 40);
        assert_eq!(presentation.rows()[1].percentage().hundredths(), 4_000);
    }

    #[test]
    fn zero_row_limit_aggregates_every_application() {
        let presentation =
            UsagePresentation::from_data(&PresentableData::Ready(Arc::new(snapshot())), 0);

        assert_eq!(presentation.rows().len(), 1);
        assert_eq!(presentation.rows()[0].duration_us(), 100);
        assert_eq!(presentation.rows()[0].represented_application_count(), 3);
    }

    #[test]
    fn every_presentable_data_state_has_an_explicit_non_mutating_presentation() {
        let value = Arc::new(snapshot());
        let error = UserFacingError::Application(ApplicationError::port_failure(
            ApplicationPort::Projection,
            PortFailureReason::Unavailable,
        ));
        let cases = [
            (
                PresentableData::InitialLoading,
                UsagePresentationState::InitialLoading,
                false,
            ),
            (
                PresentableData::Ready(Arc::clone(&value)),
                UsagePresentationState::Ready,
                true,
            ),
            (
                PresentableData::Refreshing {
                    prior: Arc::clone(&value),
                    job: JobId::new(7),
                },
                UsagePresentationState::Refreshing { job: JobId::new(7) },
                true,
            ),
            (
                PresentableData::Empty(EmptyReason::NoMatchingResults),
                UsagePresentationState::Empty(EmptyReason::NoMatchingResults),
                false,
            ),
            (
                PresentableData::Partial {
                    value: Arc::clone(&value),
                    limitations: vec![DataLimitation::TrackingPaused],
                },
                UsagePresentationState::Partial {
                    limitations: vec![DataLimitation::TrackingPaused],
                },
                true,
            ),
            (
                PresentableData::Failed {
                    prior: Some(Arc::clone(&value)),
                    error: error.clone(),
                },
                UsagePresentationState::Failed {
                    message: error.message(),
                },
                true,
            ),
            (
                PresentableData::Recovered {
                    value,
                    notice: "Usage data recovered.".to_owned(),
                },
                UsagePresentationState::Recovered {
                    notice: "Usage data recovered.".to_owned(),
                },
                true,
            ),
        ];

        for (data, expected_state, expects_value) in cases {
            let presentation = UsagePresentation::from_data(&data, 2);
            assert_eq!(presentation.state(), &expected_state);
            assert_eq!(presentation.range_label().is_some(), expects_value);
        }

        let failed =
            UsagePresentation::from_data(&PresentableData::Failed { prior: None, error }, 2);
        assert!(matches!(
            failed.state(),
            UsagePresentationState::Failed { .. }
        ));
        assert!(failed.rows().is_empty());
    }
}
