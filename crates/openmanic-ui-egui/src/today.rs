//! Today dashboard controller, shared state actions, and fixed bootstrap widgets.
//!
//! This module owns only UI-local selection, navigation, and command correlation.
//! It deliberately does not render widgets, query snapshots, access storage, or
//! call platform APIs. Widget renderers consume the immutable context supplied
//! by [`TodayController::widget_bindings`].

use std::sync::Arc;

use openmanic_application::{
    CommandEnvelope, CommandId, OrderingKey, SchemaRevision, TrackingCommand, UtcMicros,
};
use openmanic_domain::{ActivityState, ApplicationId, HalfOpenInterval};

use crate::{
    MutationStatus, QueueOverflow, TodayCategoryFilter, TodayNarrowingCriterion, TodayViewContext,
    UiController, UiModel,
};

/// A UI-local interaction on the Today dashboard.
///
/// Each action changes only selected context or UI-owned filters. Selection does
/// not change recorded activity, applications, or categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TodayAction {
    /// Selects the previous local day.
    PreviousDay,
    /// Selects the next local day when the selected day is historical.
    NextDay,
    /// Selects a date directly from the date picker as an offset from today.
    ///
    /// Positive offsets normalize to today because the Today dashboard does not
    /// navigate into future days.
    SelectDateOffset {
        /// The selected day relative to the composition root's current local day.
        day_offset: i32,
    },
    /// Returns to the current local day in one action.
    ReturnToToday,
    /// Replaces the shared explicit range supplied by another route or control.
    SetSharedRange {
        /// The valid positive range, if one is active.
        range: Option<HalfOpenInterval>,
    },
    /// Replaces the shared range selected from the timeline.
    SetTimelineSelection {
        /// The valid positive selected range, if one is active.
        selection: Option<HalfOpenInterval>,
    },
    /// Adds one application identity to the shared filter.
    AddApplicationFilter {
        /// The stable application identity to retain.
        application_id: ApplicationId,
    },
    /// Removes one application identity from the shared filter.
    RemoveApplicationFilter {
        /// The stable application identity to stop retaining.
        application_id: ApplicationId,
    },
    /// Adds one category-oriented criterion to the shared filter.
    AddCategoryFilter {
        /// The category or uncategorized criterion to retain.
        filter: TodayCategoryFilter,
    },
    /// Removes one category-oriented criterion from the shared filter.
    RemoveCategoryFilter {
        /// The category or uncategorized criterion to stop retaining.
        filter: TodayCategoryFilter,
    },
    /// Adds one activity state to the shared filter.
    AddActivityStateFilter {
        /// The canonical activity state to retain.
        state: ActivityState,
    },
    /// Removes one activity state from the shared filter.
    RemoveActivityStateFilter {
        /// The canonical activity state to stop retaining.
        state: ActivityState,
    },
    /// Clears one visible narrowing criterion.
    ClearNarrowing {
        /// The independently removable criterion.
        criterion: TodayNarrowingCriterion,
    },
    /// Clears the timeline selection and every explicit shared filter.
    ClearAllNarrowing,
}

/// One typed tracking action exposed by the Today screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackingControlAction {
    /// Requests that authoritative tracking pause.
    Pause,
    /// Requests that authoritative tracking resume.
    Resume,
}

impl TrackingControlAction {
    const fn into_command(self) -> TrackingCommand {
        match self {
            Self::Pause => TrackingCommand::Pause,
            Self::Resume => TrackingCommand::Resume,
        }
    }
}

/// Correlation metadata supplied by the composition root for one tracking control.
///
/// The controller receives time and ordering from the application boundary; it
/// never reads a clock or fabricates command identifiers on the egui thread.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TodayTrackingRequest {
    action: TrackingControlAction,
    command_id: CommandId,
    ordering_key: OrderingKey,
    submitted_at_utc: UtcMicros,
}

impl TodayTrackingRequest {
    /// Creates one typed, correlated pause or resume request.
    #[must_use]
    pub const fn new(
        action: TrackingControlAction,
        command_id: CommandId,
        ordering_key: OrderingKey,
        submitted_at_utc: UtcMicros,
    ) -> Self {
        Self {
            action,
            command_id,
            ordering_key,
            submitted_at_utc,
        }
    }

    /// Returns the requested tracking operation.
    #[must_use]
    pub const fn action(self) -> TrackingControlAction {
        self.action
    }

    /// Returns the command correlation identifier.
    #[must_use]
    pub const fn command_id(self) -> CommandId {
        self.command_id
    }
}

/// Visible acknowledgement for one submitted Today tracking control.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackingControlAcknowledgement {
    command_id: CommandId,
    status: MutationStatus,
}

impl TrackingControlAcknowledgement {
    /// Returns the command whose state is being displayed.
    #[must_use]
    pub const fn command_id(&self) -> CommandId {
        self.command_id
    }

    /// Returns the current pending, confirmed, or rejected reconciliation state.
    #[must_use]
    pub const fn status(&self) -> &MutationStatus {
        &self.status
    }
}

/// Stable identity for a fixed Today bootstrap widget instance.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TodayWidgetInstanceId {
    /// The primary three-band activity timeline instance.
    Timeline,
    /// The default application-usage summary instance.
    ApplicationUsage,
    /// The default time-distribution summary instance.
    TimeDistribution,
}

/// The compiled-in kind of one fixed Today bootstrap widget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TodayWidgetKind {
    /// The primary activity timeline widget.
    Timeline,
    /// The application-duration summary widget.
    ApplicationUsage,
    /// The category/time distribution summary widget.
    TimeDistribution,
}

/// One fixed, compiled-in default dashboard widget instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TodayWidgetInstance {
    id: TodayWidgetInstanceId,
    kind: TodayWidgetKind,
}

impl TodayWidgetInstance {
    const fn new(id: TodayWidgetInstanceId, kind: TodayWidgetKind) -> Self {
        Self { id, kind }
    }

    /// Returns the stable bootstrap instance identity.
    #[must_use]
    pub const fn id(self) -> TodayWidgetInstanceId {
        self.id
    }

    /// Returns the compiled-in widget kind.
    #[must_use]
    pub const fn kind(self) -> TodayWidgetKind {
        self.kind
    }
}

const BOOTSTRAP_WIDGETS: [TodayWidgetInstance; 3] = [
    TodayWidgetInstance::new(TodayWidgetInstanceId::Timeline, TodayWidgetKind::Timeline),
    TodayWidgetInstance::new(
        TodayWidgetInstanceId::ApplicationUsage,
        TodayWidgetKind::ApplicationUsage,
    ),
    TodayWidgetInstance::new(
        TodayWidgetInstanceId::TimeDistribution,
        TodayWidgetKind::TimeDistribution,
    ),
];

/// Fixed bootstrap registry for the three default Today widget instances.
///
/// This intentionally has no mutation API. The versioned configurable registry
/// and saved layouts are owned by OM-500 and later work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TodayWidgetRegistry {
    instances: [TodayWidgetInstance; 3],
}

impl Default for TodayWidgetRegistry {
    fn default() -> Self {
        Self {
            instances: BOOTSTRAP_WIDGETS,
        }
    }
}

impl TodayWidgetRegistry {
    /// Returns the exactly three compiled-in bootstrap widget instances.
    #[must_use]
    pub const fn instances(&self) -> &[TodayWidgetInstance; 3] {
        &self.instances
    }
}

/// One renderer input pairing a fixed widget with the shared immutable context.
#[derive(Clone, Debug)]
pub struct TodayWidgetBinding {
    instance: TodayWidgetInstance,
    context: Arc<TodayViewContext>,
}

impl TodayWidgetBinding {
    /// Returns the fixed widget instance to render.
    #[must_use]
    pub const fn instance(&self) -> TodayWidgetInstance {
        self.instance
    }

    /// Returns the same immutable context object supplied to every Today widget.
    #[must_use]
    pub const fn context(&self) -> &Arc<TodayViewContext> {
        &self.context
    }
}

/// The complete fixed Today widget input set for one normal UI frame.
#[derive(Clone, Debug)]
pub struct TodayWidgetBindings {
    widgets: [TodayWidgetBinding; 3],
}

impl TodayWidgetBindings {
    /// Returns the fixed widgets in their deterministic bootstrap order.
    #[must_use]
    pub const fn widgets(&self) -> &[TodayWidgetBinding; 3] {
        &self.widgets
    }
}

/// Controller for the Today dashboard's navigation, context, and tracking controls.
#[derive(Debug, Default)]
pub struct TodayController {
    registry: TodayWidgetRegistry,
    latest_tracking_command: Option<CommandId>,
}

impl TodayController {
    /// Creates a controller with the fixed three-widget bootstrap registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the fixed bootstrap registry used by this controller.
    #[must_use]
    pub const fn registry(&self) -> &TodayWidgetRegistry {
        &self.registry
    }

    /// Returns whether the Next control is enabled for the selected date.
    ///
    /// Today is the newest navigable day, so a Next action is disabled there.
    #[must_use]
    pub fn can_navigate_next<T>(&self, model: &UiModel<T>) -> bool {
        model.today_view_context().selected_day_offset() < 0
    }

    /// Applies one UI-local Today action without performing I/O or data mutation.
    pub fn apply<T>(&mut self, model: &mut UiModel<T>, action: TodayAction) {
        crate::reducer::reduce(model, crate::UiAction::Today(action));
    }

    /// Builds fixed renderer inputs whose widgets all share one immutable context object.
    #[must_use]
    pub fn widget_bindings<T>(&self, model: &UiModel<T>) -> TodayWidgetBindings {
        let context = Arc::new(model.today_view_context().clone());
        TodayWidgetBindings {
            widgets: self.registry.instances.map(|instance| TodayWidgetBinding {
                instance,
                context: Arc::clone(&context),
            }),
        }
    }

    /// Queues a typed tracking pause or resume request without waiting for the runtime.
    ///
    /// A successful enqueue marks the command [`MutationStatus::Pending`] before
    /// the next normal frame. Authoritative confirmation or rejection arrives via
    /// the bounded inbound event path and is exposed by
    /// [`Self::tracking_acknowledgement`].
    ///
    /// # Errors
    ///
    /// Returns [`QueueOverflow::OutboundFull`] when the bounded UI command queue
    /// cannot accept another request.
    pub fn queue_tracking<T>(
        &mut self,
        controller: &mut UiController<TrackingCommand, T>,
        request: TodayTrackingRequest,
    ) -> Result<TrackingControlAcknowledgement, QueueOverflow> {
        let command = CommandEnvelope::new(
            SchemaRevision::new(1),
            request.command_id,
            request.ordering_key,
            None,
            request.submitted_at_utc,
            request.action.into_command(),
        );
        controller.try_queue_command(command)?;
        self.latest_tracking_command = Some(request.command_id);
        Ok(TrackingControlAcknowledgement {
            command_id: request.command_id,
            status: MutationStatus::Pending,
        })
    }

    /// Returns the latest tracking-control acknowledgement after normal-frame reconciliation.
    #[must_use]
    pub fn tracking_acknowledgement<T>(
        &self,
        model: &UiModel<T>,
    ) -> Option<TrackingControlAcknowledgement> {
        let command_id = self.latest_tracking_command?;
        let status = model.mutation_status(command_id)?.clone();
        Some(TrackingControlAcknowledgement { command_id, status })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use openmanic_application::{
        AppEvent, CommandId, DataRevision, EventEnvelope, MutationConfirmation, MutationOutcome,
        MutationRejection, MutationRejectionReason, OrderingKey, SchemaRevision, TrackingEvent,
    };
    use openmanic_domain::{ActivityState, ApplicationId, HalfOpenInterval, UtcMicros};

    use super::{
        TodayAction, TodayCategoryFilter, TodayController, TodayNarrowingCriterion,
        TodayTrackingRequest, TodayWidgetInstanceId, TodayWidgetKind, TrackingControlAction,
    };
    use crate::{InboundMessage, MutationStatus, UiController, UiModel};

    #[test]
    fn date_navigation_disables_next_on_today_and_direct_future_selection_normalizes_to_today() {
        let mut controller = TodayController::new();
        let mut model = UiModel::<()>::default();

        assert!(!controller.can_navigate_next(&model));
        controller.apply(&mut model, TodayAction::NextDay);
        assert_eq!(model.today_view_context().selected_day_offset(), 0);

        controller.apply(&mut model, TodayAction::PreviousDay);
        assert_eq!(model.today_view_context().selected_day_offset(), -1);
        assert!(controller.can_navigate_next(&model));
        controller.apply(&mut model, TodayAction::NextDay);
        assert_eq!(model.today_view_context().selected_day_offset(), 0);

        controller.apply(&mut model, TodayAction::SelectDateOffset { day_offset: 4 });
        assert_eq!(model.today_view_context().selected_day_offset(), 0);
        controller.apply(&mut model, TodayAction::SelectDateOffset { day_offset: -9 });
        controller.apply(&mut model, TodayAction::ReturnToToday);
        assert_eq!(model.today_view_context().selected_day_offset(), 0);
    }

    #[test]
    fn date_navigation_clears_timeline_selection_but_preserves_explicit_filters() {
        let mut controller = TodayController::new();
        let mut model = UiModel::<()>::default();
        let application_id = ApplicationId::from_bytes([7; 16]);
        let selection = HalfOpenInterval::try_new(UtcMicros::new(10), UtcMicros::new(20))
            .expect("fixture selection is positive");

        controller.apply(
            &mut model,
            TodayAction::AddApplicationFilter { application_id },
        );
        controller.apply(
            &mut model,
            TodayAction::SetTimelineSelection {
                selection: Some(selection),
            },
        );
        controller.apply(&mut model, TodayAction::PreviousDay);

        let context = model.today_view_context();
        assert_eq!(context.timeline_selection(), None);
        assert!(context.application_filter().contains(&application_id));
    }

    #[test]
    fn each_narrowing_criterion_is_independently_clearable_and_all_can_be_cleared() {
        let mut controller = TodayController::new();
        let mut model = UiModel::<()>::default();
        let application_id = ApplicationId::from_bytes([9; 16]);
        let selection = HalfOpenInterval::try_new(UtcMicros::new(30), UtcMicros::new(40))
            .expect("fixture selection is positive");

        controller.apply(
            &mut model,
            TodayAction::SetTimelineSelection {
                selection: Some(selection),
            },
        );
        controller.apply(
            &mut model,
            TodayAction::AddApplicationFilter { application_id },
        );
        controller.apply(
            &mut model,
            TodayAction::AddCategoryFilter {
                filter: TodayCategoryFilter::Uncategorized,
            },
        );
        controller.apply(
            &mut model,
            TodayAction::AddActivityStateFilter {
                state: ActivityState::Idle,
            },
        );
        assert_eq!(
            model.today_view_context().active_narrowing_criteria().len(),
            4
        );

        controller.apply(
            &mut model,
            TodayAction::ClearNarrowing {
                criterion: TodayNarrowingCriterion::Application(application_id),
            },
        );
        assert!(
            !model
                .today_view_context()
                .application_filter()
                .contains(&application_id)
        );
        assert_eq!(
            model.today_view_context().active_narrowing_criteria().len(),
            3
        );

        controller.apply(&mut model, TodayAction::ClearAllNarrowing);
        assert!(model.today_view_context().has_no_narrowing());
    }

    #[test]
    fn fixed_bootstrap_registry_has_exactly_three_instances_with_one_shared_context() {
        let controller = TodayController::new();
        let model = UiModel::<()>::default();
        let widgets = controller.widget_bindings(&model);

        assert_eq!(controller.registry().instances().len(), 3);
        assert_eq!(
            controller.registry().instances()[0].id(),
            TodayWidgetInstanceId::Timeline
        );
        assert_eq!(
            controller.registry().instances()[1].kind(),
            TodayWidgetKind::ApplicationUsage
        );
        assert_eq!(
            controller.registry().instances()[2].id(),
            TodayWidgetInstanceId::TimeDistribution
        );
        assert!(Arc::ptr_eq(
            widgets.widgets()[0].context(),
            widgets.widgets()[1].context()
        ));
        assert!(Arc::ptr_eq(
            widgets.widgets()[1].context(),
            widgets.widgets()[2].context()
        ));
    }

    #[test]
    fn pause_and_resume_are_pending_then_reconcile_on_the_next_normal_inbound_drain() {
        let mut today = TodayController::new();
        let mut controller = UiController::try_new(UiModel::<()>::default(), 4, 4)
            .expect("fixture capacities are nonzero");

        let pause = today
            .queue_tracking(
                &mut controller,
                request(TrackingControlAction::Pause, CommandId::new(41)),
            )
            .expect("fixture queue has capacity");
        assert_eq!(pause.status(), &MutationStatus::Pending);
        controller
            .try_enqueue_inbound(InboundMessage::Event(tracking_event(
                1,
                CommandId::new(41),
                MutationOutcome::Confirmed(MutationConfirmation::new(
                    CommandId::new(41),
                    DataRevision::new(3),
                )),
            )))
            .expect("fixture inbound queue has capacity");
        controller.drain_inbound(1);
        assert_eq!(
            today
                .tracking_acknowledgement(controller.model())
                .expect("latest command remains known")
                .status(),
            &MutationStatus::Confirmed {
                data_revision: DataRevision::new(3),
            }
        );

        let resume = today
            .queue_tracking(
                &mut controller,
                request(TrackingControlAction::Resume, CommandId::new(42)),
            )
            .expect("fixture queue has capacity");
        assert_eq!(resume.status(), &MutationStatus::Pending);
        controller
            .try_enqueue_inbound(InboundMessage::Event(tracking_event(
                2,
                CommandId::new(42),
                MutationOutcome::Rejected(MutationRejection::new(
                    CommandId::new(42),
                    MutationRejectionReason::ServiceUnavailable,
                )),
            )))
            .expect("fixture inbound queue has capacity");
        controller.drain_inbound(1);
        assert_eq!(
            today
                .tracking_acknowledgement(controller.model())
                .expect("latest command remains known")
                .status(),
            &MutationStatus::Rejected {
                reason: MutationRejectionReason::ServiceUnavailable,
            }
        );
    }

    fn request(action: TrackingControlAction, command_id: CommandId) -> TodayTrackingRequest {
        TodayTrackingRequest::new(action, command_id, OrderingKey::new(1), UtcMicros::new(99))
    }

    fn tracking_event(
        sequence: u64,
        command_id: CommandId,
        outcome: MutationOutcome,
    ) -> EventEnvelope<AppEvent> {
        EventEnvelope::new(
            SchemaRevision::new(1),
            sequence,
            Some(command_id),
            None,
            UtcMicros::new(100),
            AppEvent::Tracking(TrackingEvent::Mutation {
                outcome,
                checkpoint: None,
            }),
        )
    }
}
