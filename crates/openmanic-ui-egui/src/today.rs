//! Today dashboard controller, shared state actions, and registered widget bindings.
//!
//! This module owns only UI-local selection, navigation, and command correlation.
//! It deliberately does not render widgets, query snapshots, access storage, or
//! call platform APIs. Widget renderers consume the immutable context supplied
//! by [`TodayController::widget_bindings`].

use std::sync::Arc;

use openmanic_application::{
    CommandEnvelope, CommandId, OrderingKey, SchemaRevision, TrackingCommand, UtcMicros,
};
use openmanic_domain::{ActivityState, ApplicationId, HalfOpenInterval, LayoutDefinition};

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

/// Stable persisted identity of one Today widget instance.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TodayWidgetInstanceId(String);

impl TodayWidgetInstanceId {
    /// Creates a stable instance identity from an already validated layout value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the storage-stable identity string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable reverse-domain identifier for a first-party widget kind.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TodayWidgetKind(&'static str);

impl TodayWidgetKind {
    /// The primary activity Timeline.
    pub const TIMELINE: Self = Self("openmanic.timeline.day");
    /// Application-duration summary.
    pub const APPLICATION_USAGE: Self = Self("openmanic.usage.application");
    /// Category/time distribution summary.
    pub const TIME_DISTRIBUTION: Self = Self("openmanic.distribution.time");
    /// Focus-session controls and state.
    pub const FOCUS: Self = Self("openmanic.focus.session");

    /// Returns the stable persisted kind ID.
    #[must_use]
    pub const fn id(self) -> &'static str {
        self.0
    }
}

/// Width constraints declared by a widget renderer for each responsive grid mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TodayWidgetSizePolicy {
    minimum_span_12: u8,
    minimum_span_8: u8,
    minimum_span_4: u8,
    preferred_span_12: u8,
}

impl TodayWidgetSizePolicy {
    const fn new(
        minimum_span_12: u8,
        minimum_span_8: u8,
        minimum_span_4: u8,
        preferred_span_12: u8,
    ) -> Self {
        Self {
            minimum_span_12,
            minimum_span_8,
            minimum_span_4,
            preferred_span_12,
        }
    }

    /// Returns the minimum span for a 12-column desktop grid.
    #[must_use]
    pub const fn minimum_span_12(self) -> u8 {
        self.minimum_span_12
    }

    /// Returns the minimum span for an 8-column responsive grid.
    #[must_use]
    pub const fn minimum_span_8(self) -> u8 {
        self.minimum_span_8
    }

    /// Returns the minimum span for a 4-column responsive grid.
    #[must_use]
    pub const fn minimum_span_4(self) -> u8 {
        self.minimum_span_4
    }

    /// Returns the preferred persisted desktop span.
    #[must_use]
    pub const fn preferred_span_12(self) -> u8 {
        self.preferred_span_12
    }
}

/// Static first-party widget contract registered before a dashboard layout is restored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TodayWidgetDefinition {
    kind: TodayWidgetKind,
    schema_version: u16,
    display_name: &'static str,
    description: &'static str,
    size_policy: TodayWidgetSizePolicy,
    supports_multiple_instances: bool,
}

impl TodayWidgetDefinition {
    /// Returns the stable widget kind.
    #[must_use]
    pub const fn kind(self) -> TodayWidgetKind {
        self.kind
    }
    /// Returns the latest compatible configuration schema version.
    #[must_use]
    pub const fn schema_version(self) -> u16 {
        self.schema_version
    }
    /// Returns picker-visible display text.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        self.display_name
    }
    /// Returns picker-visible explanatory text.
    #[must_use]
    pub const fn description(self) -> &'static str {
        self.description
    }
    /// Returns responsive sizing requirements.
    #[must_use]
    pub const fn size_policy(self) -> TodayWidgetSizePolicy {
        self.size_policy
    }
    /// Returns whether layouts may contain more than one instance of this kind.
    #[must_use]
    pub const fn supports_multiple_instances(self) -> bool {
        self.supports_multiple_instances
    }
    /// Returns whether a persisted configuration can be migrated by this renderer.
    #[must_use]
    pub const fn supports_schema(self, schema_version: u16) -> bool {
        schema_version > 0 && schema_version <= self.schema_version
    }
}

/// A persisted widget instance restored independently from renderer availability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TodayWidgetInstance {
    id: TodayWidgetInstanceId,
    kind_id: String,
    kind_schema_version: u16,
}

impl TodayWidgetInstance {
    fn new(id: &str, kind: TodayWidgetKind, kind_schema_version: u16) -> Self {
        Self {
            id: TodayWidgetInstanceId::new(id),
            kind_id: kind.id().to_owned(),
            kind_schema_version,
        }
    }

    /// Reconstructs an instance from a validated persisted layout placement.
    #[must_use]
    pub fn from_layout(
        instance_id: impl Into<String>,
        kind_id: impl Into<String>,
        kind_schema_version: u16,
    ) -> Self {
        Self {
            id: TodayWidgetInstanceId::new(instance_id),
            kind_id: kind_id.into(),
            kind_schema_version,
        }
    }

    /// Returns the stable instance identity.
    #[must_use]
    pub fn id(&self) -> &TodayWidgetInstanceId {
        &self.id
    }
    /// Returns the stable persisted widget-kind identifier.
    #[must_use]
    pub fn kind_id(&self) -> &str {
        &self.kind_id
    }
    /// Returns the persisted widget configuration schema version.
    #[must_use]
    pub const fn kind_schema_version(&self) -> u16 {
        self.kind_schema_version
    }
}

/// Recoverable outcome of resolving a restored layout instance through the registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TodayWidgetResolution {
    /// A compatible first-party renderer is available.
    Available(TodayWidgetDefinition),
    /// The kind is absent or its persisted schema is newer than the renderer supports.
    MissingRenderer,
}

/// Registry construction failure used to reject duplicate first-party widget kinds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TodayWidgetRegistryError {
    /// More than one definition claims the same stable kind ID.
    DuplicateKind,
}

const FIRST_PARTY_WIDGETS: [TodayWidgetDefinition; 4] = [
    TodayWidgetDefinition {
        kind: TodayWidgetKind::TIMELINE,
        schema_version: 1,
        display_name: "Timeline",
        description: "Tracked activity and personal schedule",
        size_policy: TodayWidgetSizePolicy::new(6, 4, 4, 12),
        supports_multiple_instances: false,
    },
    TodayWidgetDefinition {
        kind: TodayWidgetKind::APPLICATION_USAGE,
        schema_version: 1,
        display_name: "Application usage",
        description: "Exact duration by application",
        size_policy: TodayWidgetSizePolicy::new(3, 2, 2, 4),
        supports_multiple_instances: true,
    },
    TodayWidgetDefinition {
        kind: TodayWidgetKind::TIME_DISTRIBUTION,
        schema_version: 1,
        display_name: "Time distribution",
        description: "Exact duration by category",
        size_policy: TodayWidgetSizePolicy::new(3, 2, 2, 4),
        supports_multiple_instances: true,
    },
    TodayWidgetDefinition {
        kind: TodayWidgetKind::FOCUS,
        schema_version: 1,
        display_name: "Focus session",
        description: "Focus timer controls and status",
        size_policy: TodayWidgetSizePolicy::new(3, 2, 2, 4),
        supports_multiple_instances: false,
    },
];

/// First-party widget definitions and their recoverable layout resolution contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TodayWidgetRegistry {
    definitions: Vec<TodayWidgetDefinition>,
}

impl Default for TodayWidgetRegistry {
    fn default() -> Self {
        Self {
            definitions: FIRST_PARTY_WIDGETS.to_vec(),
        }
    }
}

impl TodayWidgetRegistry {
    /// Builds a registry, rejecting duplicate stable kind IDs before presentation starts.
    ///
    /// # Errors
    ///
    /// Returns [`TodayWidgetRegistryError::DuplicateKind`] for duplicate stable widget kind IDs.
    pub fn try_new(
        definitions: Vec<TodayWidgetDefinition>,
    ) -> Result<Self, TodayWidgetRegistryError> {
        for (index, definition) in definitions.iter().enumerate() {
            if definitions[..index]
                .iter()
                .any(|existing| existing.kind == definition.kind)
            {
                return Err(TodayWidgetRegistryError::DuplicateKind);
            }
        }
        Ok(Self { definitions })
    }

    /// Returns all registered first-party widget definitions in picker order.
    #[must_use]
    pub fn definitions(&self) -> &[TodayWidgetDefinition] {
        &self.definitions
    }

    /// Resolves a restored widget without allowing an unavailable renderer to block startup.
    #[must_use]
    pub fn resolve(&self, instance: &TodayWidgetInstance) -> TodayWidgetResolution {
        self.definitions
            .iter()
            .copied()
            .find(|definition| {
                definition.kind.id() == instance.kind_id
                    && definition.supports_schema(instance.kind_schema_version)
            })
            .map_or(
                TodayWidgetResolution::MissingRenderer,
                TodayWidgetResolution::Available,
            )
    }

    fn default_instances() -> Vec<TodayWidgetInstance> {
        vec![
            TodayWidgetInstance::new("today.timeline", TodayWidgetKind::TIMELINE, 1),
            TodayWidgetInstance::new("today.usage", TodayWidgetKind::APPLICATION_USAGE, 1),
            TodayWidgetInstance::new("today.distribution", TodayWidgetKind::TIME_DISTRIBUTION, 1),
            TodayWidgetInstance::new("today.focus", TodayWidgetKind::FOCUS, 1),
        ]
    }
}

/// One renderer input pairing a restored widget with the shared immutable context.
#[derive(Clone, Debug)]
pub struct TodayWidgetBinding {
    instance: TodayWidgetInstance,
    resolution: TodayWidgetResolution,
    context: Arc<TodayViewContext>,
}

impl TodayWidgetBinding {
    /// Returns the restored widget instance to render or recover.
    #[must_use]
    pub fn instance(&self) -> &TodayWidgetInstance {
        &self.instance
    }
    /// Returns whether a compatible renderer is available for this instance.
    #[must_use]
    pub const fn resolution(&self) -> TodayWidgetResolution {
        self.resolution
    }

    /// Returns the same immutable context object supplied to every Today widget.
    #[must_use]
    pub const fn context(&self) -> &Arc<TodayViewContext> {
        &self.context
    }
}

/// The complete registered Today widget input set for one normal UI frame.
#[derive(Clone, Debug)]
pub struct TodayWidgetBindings {
    widgets: Vec<TodayWidgetBinding>,
}

impl TodayWidgetBindings {
    /// Returns widgets in deterministic restored-layout order.
    #[must_use]
    pub fn widgets(&self) -> &[TodayWidgetBinding] {
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
    /// Creates a controller with the registered first-party widget definitions.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the first-party widget registry used by this controller.
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

    /// Builds renderer inputs whose widgets all share one immutable context object.
    #[must_use]
    pub fn widget_bindings<T>(&self, model: &UiModel<T>) -> TodayWidgetBindings {
        self.widget_bindings_for_instances(model, TodayWidgetRegistry::default_instances())
    }

    /// Builds bindings for a validated persisted layout without requiring a renderer for every row.
    #[must_use]
    pub fn widget_bindings_for_layout<T>(
        &self,
        model: &UiModel<T>,
        layout: &LayoutDefinition,
    ) -> TodayWidgetBindings {
        let mut widgets = layout.widgets.clone();
        widgets.sort_by_key(|widget| (widget.order, widget.instance_id.clone()));
        self.widget_bindings_for_instances(
            model,
            widgets
                .into_iter()
                .map(|widget| {
                    TodayWidgetInstance::from_layout(
                        widget.instance_id,
                        widget.kind_id,
                        widget.kind_schema_version,
                    )
                })
                .collect(),
        )
    }

    fn widget_bindings_for_instances<T>(
        &self,
        model: &UiModel<T>,
        instances: Vec<TodayWidgetInstance>,
    ) -> TodayWidgetBindings {
        let context = Arc::new(model.today_view_context().clone());
        TodayWidgetBindings {
            widgets: instances
                .into_iter()
                .map(|instance| TodayWidgetBinding {
                    resolution: self.registry.resolve(&instance),
                    instance,
                    context: Arc::clone(&context),
                })
                .collect(),
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
    use openmanic_domain::{
        ActivityState, ApplicationId, HalfOpenInterval, LayoutDocument, UtcMicros,
    };

    use super::{
        TodayAction, TodayCategoryFilter, TodayController, TodayNarrowingCriterion,
        TodayTrackingRequest, TodayWidgetKind, TodayWidgetRegistry, TodayWidgetRegistryError,
        TodayWidgetResolution, TrackingControlAction,
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
    fn registered_widgets_share_one_context_and_include_the_focus_widget() {
        let controller = TodayController::new();
        let model = UiModel::<()>::default();
        let widgets = controller.widget_bindings(&model);

        assert_eq!(controller.registry().definitions().len(), 4);
        assert_eq!(
            widgets.widgets()[0].instance().id().as_str(),
            "today.timeline"
        );
        assert_eq!(
            widgets.widgets()[1].instance().kind_id(),
            TodayWidgetKind::APPLICATION_USAGE.id()
        );
        assert_eq!(
            widgets.widgets()[3].instance().kind_id(),
            TodayWidgetKind::FOCUS.id()
        );
        assert!(matches!(
            widgets.widgets()[0].resolution(),
            TodayWidgetResolution::Available(_)
        ));
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
    fn registry_rejects_duplicate_stable_kind_ids() {
        let definitions = TodayWidgetRegistry::default().definitions().to_vec();
        let duplicate = definitions[0];
        assert_eq!(
            TodayWidgetRegistry::try_new(vec![duplicate, duplicate]),
            Err(TodayWidgetRegistryError::DuplicateKind)
        );
    }

    #[test]
    fn restored_unknown_layout_widget_stays_recoverable_without_hiding_known_widgets() {
        let controller = TodayController::new();
        let model = UiModel::<()>::default();
        let mut layout = LayoutDocument::safe_default().definition();
        layout.widgets[1].kind_id = "openmanic.future.unavailable".to_owned();
        let bindings = controller.widget_bindings_for_layout(&model, &layout);

        assert_eq!(bindings.widgets().len(), 4);
        assert_eq!(
            bindings.widgets()[1].resolution(),
            TodayWidgetResolution::MissingRenderer
        );
        assert!(matches!(
            bindings.widgets()[0].resolution(),
            TodayWidgetResolution::Available(_)
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
