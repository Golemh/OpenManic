//! OpenManic use cases, typed ports, versioned envelopes, and immutable snapshots.
//!
//! This crate owns application-boundary contracts and may depend only on domain policy. It
//! deliberately does not own concrete GUI, storage, platform, runtime, or serialization adapters.
//! Future concurrency belongs here, behind bounded protocols that use these contracts.

#![forbid(unsafe_code)]

mod application_metadata;
mod calendar_projection;
mod catalog;
mod commands;
mod data_operations;
mod errors;
mod events;
mod focus;
mod ids;
mod layout;
mod overview;
mod ports;
mod projection;
mod runtime;
mod saved_view;
mod schedule;
mod schedule_projection;
mod schedule_time;
mod timeline_projection;
mod title_stabilizer;
mod tracking;

// Platform adapters consume only this crate's facade. Re-export the domain values
// already present in normalized tracking evidence so that boundary remains intact.
pub use application_metadata::{
    ApplicationIcon, ApplicationIconCache, ApplicationIconCacheDiagnostics,
    ApplicationIconCacheInsert, ApplicationIconCacheLimitError, ApplicationIconCacheLimits,
    ApplicationIconDigest, ApplicationIconError, ApplicationIconKey, ApplicationIconLookup,
    ApplicationIconResult,
};
pub use calendar_projection::{
    ActivityTimelineNavigation, CalendarBlock, CalendarBlockId, CalendarBlockSource,
    CalendarContinuation, CalendarDayContext, CalendarDayProjector, CalendarDaySnapshot,
    CalendarProjectionError, CalendarProjectionSource, CalendarSourceFocus,
};
pub use catalog::{
    CatalogApplicationSnapshot, CatalogAssignmentFilter, CatalogCategorySnapshot, CatalogCommand,
    CatalogCommandError, CatalogFilter, CatalogPersistence, CatalogPersistenceError,
    CatalogService, CatalogSnapshot,
};
pub use commands::{CommandEnvelope, CommandReceipt, TrackingCommand, TrackingEvidence};
pub use data_operations::{
    CSV_INTERCHANGE_VERSION, CsvExportRequest, DataOperationDestination, DataOperationOutcome,
    DataOperationProgress, DataOperationProgressError, CsvImportRequest, ImportDestinationScope,
    ImportBatchId, ImportFailure, ImportFailureError, ImportScopeError, ImportScopeOutcome,
    TitleDisclosure,
};
pub use errors::{ApplicationError, ApplicationPort, PortFailureReason};
pub use events::{
    AppEvent, EventEnvelope, JobEvent, JobState, MutationConfirmation, MutationOutcome,
    MutationRejection, MutationRejectionReason, TrackingEvent, TrackingEvidenceIgnoredReason,
};
pub use focus::{
    FocusCommand, FocusKind, FocusMutation, FocusNotificationError, FocusNotificationPort,
    FocusPersistence, FocusPersistenceError, FocusService, FocusSnapshot,
};
pub use ids::{
    CommandId, DataRevision, EntityRevision, JobId, OrderingKey, ProjectionContextKey,
    ProjectionSlot, RequestId, SchemaRevision,
};
pub use layout::{
    LayoutMutation, LayoutPersistence, LayoutPersistenceError, LayoutService, LayoutSnapshot,
};
pub use openmanic_domain::{ApplicationId, FocusSessionId, PowerTransitionEvidence, UtcMicros};
pub use overview::{
    OVERVIEW_SNAPSHOT_SCHEMA_REVISION, OverviewAllocation, OverviewAllocationIdentity,
    OverviewCacheKey, OverviewContext, OverviewFilters, OverviewGrouping, OverviewProjectionError,
    OverviewProjectionResult, OverviewProjectionSlotState, OverviewProjectionSource,
    OverviewProjectionStatus, OverviewProjector, OverviewRange, OverviewSnapshot,
    OverviewSourceActivity, SharedOverviewSelection,
};
pub use ports::{
    CommandPort, ProjectionPort, TrackingPersistencePort, TrackingPersistenceRetentionReason,
    TrackingPersistenceSubmit,
};
pub use projection::{ProjectionRequest, ProjectionSlotState, SnapshotEnvelope, SnapshotRejection};
pub use runtime::{
    CancellationRequest, CancellationSource, CancellationToken, LaneCapacities,
    LaneConfigurationError, LaneReceive, LaneRetentionReason, LaneSubmit, LatestMailbox,
    LatestMailboxReceiver, MailboxPublish, MailboxReceive, RuntimeLaneReceiver, RuntimeLanes,
    RuntimeSupervisor, RuntimeWorker, ShutdownAdvance, ShutdownCoordinator, ShutdownError,
    ShutdownPhase, ShutdownStep, ThreadRoot, WorkLane, WorkerEscalation, WorkerFailure,
    WorkerHealth, WorkerHealthState, bounded_runtime_lanes, latest_mailbox,
};
pub use saved_view::{
    SavedViewCommand, SavedViewId, SavedViewLoad, SavedViewMutation, SavedViewPersistence,
    SavedViewPersistenceError, SavedViewRejection, SavedViewService, SavedViewSnapshot,
    SavedViewSnapshotError,
};
pub use schedule::{
    RecurringOccurrenceOverride, RecurringScheduleEdit, RecurringScheduleRuleChange,
    ScheduleCommand, ScheduleId, ScheduleMutation, SchedulePersistence, SchedulePersistenceError,
    ScheduleService, ScheduleSnapshot, ScheduleSnapshotError,
};
pub use schedule_projection::{
    ScheduleOccurrence, ScheduleOccurrenceId, project_schedule_occurrences,
};
pub use schedule_time::{
    ResolvedScheduleBoundary, ResolvedScheduleOccurrence, SCHEDULE_CIVIL_EPOCH,
    ScheduleBoundaryResolution, ScheduleTimeError, expand_repeating_schedule,
    expand_repeating_schedule_in_interval, repeating_schedule_rules_conflict,
    resolve_schedule_boundary, schedule_rule_conflicts_with_intervals,
};
pub use timeline_projection::{
    ActivityStateValue, ApplicationBandValue, CategoryBandValue, DataCompleteness, IntervalIndex,
    TIMELINE_SNAPSHOT_SCHEMA_REVISION, TimelineApplication, TimelineContext, TimelineInterval,
    TimelineProjectionError, TimelineProjectionSource, TimelineProjector, TimelineRawFragment,
    TimelineRawIntervalId, TimelineSnapshot, TimelineSourceActivity, TimelineTotals,
};
pub use title_stabilizer::{
    AcceptedWindowTitle, MAX_WINDOW_TITLE_BYTES, TITLE_STABILITY_US, TitleObservationResult,
    TitleStabilizer,
};
pub use tracking::{
    TRACKING_CHECKPOINT_INTERVAL_US, TrackingCheckpoint, TrackingCheckpointError,
    TrackingPersistenceIntent, TrackingPersistenceIntentError, TrackingService,
};

#[cfg(test)]
mod tests {
    use openmanic_domain::UtcMicros;

    use crate::{
        AppEvent, ApplicationError, ApplicationPort, CommandEnvelope, CommandId, CommandPort,
        CommandReceipt, DataRevision, EntityRevision, EventEnvelope, JobEvent, JobId, JobState,
        MutationConfirmation, MutationOutcome, MutationRejection, MutationRejectionReason,
        OrderingKey, PortFailureReason, ProjectionContextKey, ProjectionPort, ProjectionRequest,
        ProjectionSlot, ProjectionSlotState, RequestId, SchemaRevision, SnapshotEnvelope,
        SnapshotRejection,
    };

    #[test]
    fn identifier_value_shapes_remain_distinct_and_exact_without_serialization() {
        assert_eq!(CommandId::new(u64::MAX).get(), u64::MAX);
        assert_eq!(JobId::new(9).get(), 9);
        assert_eq!(RequestId::new(8).get(), 8);
        assert_eq!(DataRevision::new(7).get(), 7);
        assert_eq!(EntityRevision::new(6).get(), 6);
        assert_eq!(SchemaRevision::new(u16::MAX).get(), u16::MAX);
    }

    #[test]
    fn command_and_event_envelopes_preserve_mutation_correlation() {
        let command_id = CommandId::new(41);
        let command = CommandEnvelope::new(
            SchemaRevision::new(2),
            command_id,
            OrderingKey::new(9),
            Some(EntityRevision::new(3)),
            UtcMicros::new(1_000),
            TestCommand::Pause,
        );
        assert_eq!(command.command_id(), command_id);
        assert_eq!(command.schema_revision(), SchemaRevision::new(2));
        assert_eq!(command.ordering_key(), OrderingKey::new(9));
        assert_eq!(
            command.expected_entity_revision(),
            Some(EntityRevision::new(3))
        );
        assert_eq!(command.payload(), &TestCommand::Pause);

        let confirmation = MutationConfirmation::new(command_id, DataRevision::new(12));
        let event = EventEnvelope::new(
            SchemaRevision::new(2),
            77,
            Some(command_id),
            Some(DataRevision::new(12)),
            UtcMicros::new(1_001),
            AppEvent::Mutation(MutationOutcome::Confirmed(confirmation)),
        );

        assert_eq!(event.causation_command_id(), Some(command.command_id()));
        assert_eq!(event.schema_revision(), SchemaRevision::new(2));
        assert_eq!(event.sequence(), 77);
        assert_eq!(event.committed_data_revision(), Some(DataRevision::new(12)));
        assert_eq!(
            event.payload(),
            &AppEvent::Mutation(MutationOutcome::Confirmed(confirmation))
        );
    }

    #[test]
    fn mutation_outcomes_express_confirmed_and_rejected_results() {
        let command_id = CommandId::new(12);
        let confirmed =
            MutationOutcome::Confirmed(MutationConfirmation::new(command_id, DataRevision::new(4)));
        let rejected = MutationOutcome::Rejected(MutationRejection::new(
            command_id,
            MutationRejectionReason::RevisionConflict,
        ));

        assert_eq!(
            confirmed,
            MutationOutcome::Confirmed(MutationConfirmation::new(command_id, DataRevision::new(4)))
        );
        assert_eq!(
            rejected,
            MutationOutcome::Rejected(MutationRejection::new(
                command_id,
                MutationRejectionReason::RevisionConflict
            ))
        );
    }

    #[test]
    fn deterministic_fake_adapters_compile_against_application_ports() {
        let command_id = CommandId::new(21);
        let mut command_port = FakeCommandPort::default();
        let receipt = command_port
            .submit(CommandEnvelope::new(
                SchemaRevision::new(1),
                command_id,
                OrderingKey::new(1),
                None,
                UtcMicros::new(100),
                TestCommand::Pause,
            ))
            .expect("the deterministic fake accepts the fixture command");
        assert_eq!(receipt, CommandReceipt::accepted(command_id));
        assert_eq!(command_port.last_command_id, Some(command_id));

        let request = ProjectionRequest::new(
            RequestId::new(22),
            ProjectionSlot::new(2),
            ProjectionContextKey::new(3),
            DataRevision::new(4),
            "fixture projection",
        );
        let mut projection_port = FakeProjectionPort;
        let snapshot = projection_port
            .project(request)
            .expect("the deterministic fake returns a presentation snapshot");
        assert_eq!(snapshot.value(), &"fixture projection");
        assert_eq!(snapshot.source_data_revision(), DataRevision::new(4));
        assert_eq!(snapshot.snapshot_schema_revision(), SchemaRevision::new(1));
        assert_eq!(*snapshot.shared_value(), "fixture projection");
    }

    #[test]
    fn stale_request_context_revision_and_missing_target_results_are_rejected() {
        let request = ProjectionRequest::new(
            RequestId::new(10),
            ProjectionSlot::new(20),
            ProjectionContextKey::new(30),
            DataRevision::new(5),
            (),
        );
        let mut state = ProjectionSlotState::new(&request);

        assert_eq!(
            state.accept_if_current(&snapshot(
                RequestId::new(10),
                ProjectionSlot::new(20),
                ProjectionContextKey::new(30),
                DataRevision::new(4),
            )),
            Err(SnapshotRejection::SourceRevisionOlderThanRequired {
                source: DataRevision::new(4),
                required: DataRevision::new(5),
            })
        );
        state
            .accept_if_current(&snapshot(
                RequestId::new(10),
                ProjectionSlot::new(20),
                ProjectionContextKey::new(30),
                DataRevision::new(6),
            ))
            .expect("a current snapshot at a newer revision is accepted");
        assert_eq!(
            state.last_accepted_data_revision(),
            Some(DataRevision::new(6))
        );

        assert_eq!(
            state.accept_if_current(&snapshot(
                RequestId::new(9),
                ProjectionSlot::new(20),
                ProjectionContextKey::new(30),
                DataRevision::new(7),
            )),
            Err(SnapshotRejection::RequestNotCurrent {
                expected: RequestId::new(10),
                received: RequestId::new(9),
            })
        );
        assert_eq!(
            state.accept_if_current(&snapshot(
                RequestId::new(10),
                ProjectionSlot::new(20),
                ProjectionContextKey::new(31),
                DataRevision::new(7),
            )),
            Err(SnapshotRejection::ContextMismatch {
                expected: ProjectionContextKey::new(30),
                received: ProjectionContextKey::new(31),
            })
        );
        assert_eq!(
            state.accept_if_current(&snapshot(
                RequestId::new(10),
                ProjectionSlot::new(20),
                ProjectionContextKey::new(30),
                DataRevision::new(5),
            )),
            Err(SnapshotRejection::SourceRevisionOlderThanAccepted {
                source: DataRevision::new(5),
                last_accepted: DataRevision::new(6),
            })
        );

        state.remove_target();
        assert_eq!(
            state.accept_if_current(&snapshot(
                RequestId::new(10),
                ProjectionSlot::new(20),
                ProjectionContextKey::new(30),
                DataRevision::new(7),
            )),
            Err(SnapshotRejection::TargetMissing {
                slot: ProjectionSlot::new(20),
            })
        );
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TestCommand {
        Pause,
    }

    #[derive(Default)]
    struct FakeCommandPort {
        last_command_id: Option<CommandId>,
    }

    impl CommandPort<TestCommand> for FakeCommandPort {
        fn submit(
            &mut self,
            command: CommandEnvelope<TestCommand>,
        ) -> Result<CommandReceipt, crate::ApplicationError> {
            self.last_command_id = Some(command.command_id());
            Ok(CommandReceipt::accepted(command.command_id()))
        }
    }

    struct FakeProjectionPort;

    impl ProjectionPort<&'static str, &'static str> for FakeProjectionPort {
        fn project(
            &mut self,
            request: ProjectionRequest<&'static str>,
        ) -> Result<SnapshotEnvelope<&'static str>, crate::ApplicationError> {
            Ok(SnapshotEnvelope::new(
                request.request_id(),
                request.slot(),
                request.context_key(),
                request.required_data_revision(),
                SchemaRevision::new(1),
                request.into_payload(),
            ))
        }
    }

    fn snapshot(
        request_id: RequestId,
        slot: ProjectionSlot,
        context_key: ProjectionContextKey,
        source_data_revision: DataRevision,
    ) -> SnapshotEnvelope<()> {
        SnapshotEnvelope::new(
            request_id,
            slot,
            context_key,
            source_data_revision,
            SchemaRevision::new(1),
            (),
        )
    }

    #[test]
    fn job_events_keep_their_stable_identifier() {
        let event = JobEvent::new(
            JobId::new(80),
            JobState::Failed {
                error: ApplicationError::port_failure(
                    ApplicationPort::Projection,
                    PortFailureReason::Unavailable,
                ),
            },
        );

        assert_eq!(event.job_id(), JobId::new(80));
        assert_eq!(
            event.state(),
            &JobState::Failed {
                error: ApplicationError::port_failure(
                    ApplicationPort::Projection,
                    PortFailureReason::Unavailable,
                ),
            }
        );
    }
}
