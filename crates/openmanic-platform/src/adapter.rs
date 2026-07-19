//! Safe, bounded normalization from adapter observations to application tracking evidence.
//!
//! The normalizer accepts only resolved, platform-neutral observations. Concrete adapters retain
//! raw handles and operating-system error values in their private modules, then call this module
//! after callback work has completed. Publishing is one nonblocking attempt per evidence value;
//! a full or closed sink records explicit loss and requires fresh reconciliation.

use openmanic_application::TrackingEvidence;
use openmanic_domain::{ApplicationId, UtcMicros};

use crate::{AdapterAvailability, AvailabilityTransition, PlatformCapabilities};

/// The result of one nonblocking tracking-evidence publication attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidencePublishResult {
    /// The bounded application ingress accepted the evidence.
    Accepted,
    /// The bounded ingress is full and did not accept the evidence.
    Full,
    /// The ingress has stopped accepting evidence.
    Closed,
}

/// The nonblocking application ingress consumed by a platform adapter.
///
/// Implementations must return promptly from [`Self::try_publish`]. In particular, an operating
/// system callback must never wait for the tracking service, SQLite, or the UI to consume work.
pub trait TrackingEvidenceSink: Send + Sync {
    /// Attempts to publish one accepted application evidence value without waiting.
    #[must_use]
    fn try_publish(&self, evidence: TrackingEvidence) -> EvidencePublishResult;
}

/// A platform-neutral observation after raw platform data has been resolved privately.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdapterObservation {
    source_order: u64,
    observed_at_utc: UtcMicros,
    kind: AdapterObservationKind,
}

impl AdapterObservation {
    /// Creates one observation with the adapter's source-local ordering value.
    #[must_use]
    pub const fn new(
        source_order: u64,
        observed_at_utc: UtcMicros,
        kind: AdapterObservationKind,
    ) -> Self {
        Self {
            source_order,
            observed_at_utc,
            kind,
        }
    }

    /// Returns the source-local ordering value used to reject reordered observations.
    #[must_use]
    pub const fn source_order(self) -> u64 {
        self.source_order
    }

    /// Returns the UTC instant captured by the adapter for this observation.
    #[must_use]
    pub const fn observed_at_utc(self) -> UtcMicros {
        self.observed_at_utc
    }

    /// Returns the resolved platform-neutral observation kind.
    #[must_use]
    pub const fn kind(self) -> AdapterObservationKind {
        self.kind
    }
}

/// A resolved observation that can map directly to accepted tracking evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterObservationKind {
    /// A stable application identity was observed in the foreground.
    Foreground {
        /// The resolved stable application identity.
        application_id: ApplicationId,
    },
    /// The foreground application matched the user's exclusion policy.
    ExcludedForeground,
    /// The configured idle threshold was crossed.
    IdleThresholdCrossed,
    /// The user session locked.
    SessionLocked,
    /// The user session disconnected.
    SessionDisconnected,
    /// The system suspended or hibernated.
    SystemSuspended,
    /// The adapter observed a wall-clock discontinuity.
    ClockDiscontinuity,
    /// The adapter's safe availability state changed.
    Availability(AdapterAvailability),
}

/// Explains why a normalizer deliberately did not publish an observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationIgnoredReason {
    /// A source ordering value repeated or moved backwards.
    DuplicateOrReordered,
    /// The observation repeats the last accepted normal observation.
    DuplicateObservation,
    /// The availability state was already current.
    AvailabilityUnchanged,
}

/// The public result of processing one platform observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdapterPublishResult {
    status: AdapterPublishStatus,
    availability_transition: Option<AvailabilityTransition>,
    reconciliation_required: bool,
}

impl AdapterPublishResult {
    const fn new(
        status: AdapterPublishStatus,
        availability_transition: Option<AvailabilityTransition>,
        reconciliation_required: bool,
    ) -> Self {
        Self {
            status,
            availability_transition,
            reconciliation_required,
        }
    }

    /// Returns how publication or normalization completed.
    #[must_use]
    pub const fn status(self) -> AdapterPublishStatus {
        self.status
    }

    /// Returns an availability transition carried by this observation, when one occurred.
    #[must_use]
    pub const fn availability_transition(self) -> Option<AvailabilityTransition> {
        self.availability_transition
    }

    /// Returns whether a concrete adapter must obtain a fresh platform observation.
    #[must_use]
    pub const fn reconciliation_required(self) -> bool {
        self.reconciliation_required
    }
}

/// The terminal status of one normalizer call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterPublishStatus {
    /// The sink accepted this many evidence values.
    Emitted {
        /// The number of values accepted by the sink.
        evidence_count: u8,
    },
    /// The observation changed adapter state but maps to no tracking evidence.
    StateChanged,
    /// The normalizer deliberately ignored an unpublishable observation.
    Ignored {
        /// The explicit reason this observation was not published.
        reason: ObservationIgnoredReason,
    },
    /// A bounded sink could not accept evidence without waiting.
    Backpressured,
    /// The sink stopped accepting evidence.
    SinkClosed,
    /// The monotonic adapter sequence has no remaining representable value.
    SequenceExhausted,
}

/// Safe common surface for platform adapters after their private raw-event resolution.
pub trait PlatformActivityAdapter: Send + 'static {
    /// Returns actual capability-probe results for the selected adapter.
    fn capabilities(&self) -> PlatformCapabilities;

    /// Returns the most recently announced safe availability state.
    fn availability(&self) -> AdapterAvailability;

    /// Normalizes and publishes a resolved observation with no blocking work.
    #[must_use]
    fn normalize_and_publish(
        &mut self,
        observation: AdapterObservation,
        sink: &dyn TrackingEvidenceSink,
    ) -> AdapterPublishResult;
}

/// Stateful normalizer that assigns sequences and preserves overflow/reconciliation evidence.
///
/// Sequence values are assigned before publication. A failed nonblocking attempt therefore leaves
/// a diagnostic gap, followed by an explicit `EvidenceQueueOverflow` value once delivery resumes.
#[derive(Debug)]
pub struct PlatformEventNormalizer {
    last_source_order: Option<u64>,
    last_published_kind: Option<AdapterObservationKind>,
    availability: Option<AdapterAvailability>,
    next_sequence: Option<u64>,
    evidence_loss_pending: bool,
    reconciliation_required: bool,
}

impl Default for PlatformEventNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformEventNormalizer {
    /// Creates a normalizer before the adapter has announced its availability.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_source_order: None,
            last_published_kind: None,
            availability: None,
            next_sequence: Some(1),
            evidence_loss_pending: false,
            reconciliation_required: false,
        }
    }

    /// Returns the most recently announced availability, defaulting to startup.
    #[must_use]
    pub const fn availability(&self) -> AdapterAvailability {
        match self.availability {
            Some(availability) => availability,
            None => AdapterAvailability::Starting,
        }
    }

    /// Returns whether a concrete adapter must query fresh platform state before trusting output.
    #[must_use]
    pub const fn reconciliation_required(&self) -> bool {
        self.reconciliation_required
    }

    /// Clears the reconciliation requirement after a concrete adapter obtains fresh platform data.
    ///
    /// Callers must invoke this only after a successful current-state query, never merely after a
    /// sink becomes writable. The next observation still receives a fresh adapter sequence.
    pub fn acknowledge_reconciliation(&mut self) {
        self.reconciliation_required = false;
    }

    /// Normalizes one resolved observation and makes bounded nonblocking publication attempts.
    #[must_use]
    pub fn normalize_and_publish(
        &mut self,
        observation: AdapterObservation,
        sink: &dyn TrackingEvidenceSink,
    ) -> AdapterPublishResult {
        if self.is_reordered(observation.source_order()) {
            return self.result(
                AdapterPublishStatus::Ignored {
                    reason: ObservationIgnoredReason::DuplicateOrReordered,
                },
                None,
            );
        }
        self.last_source_order = Some(observation.source_order());

        let availability_transition = self.apply_availability(observation.kind());
        if self.is_unchanged_availability(observation.kind(), availability_transition) {
            return self.result(
                AdapterPublishStatus::Ignored {
                    reason: ObservationIgnoredReason::AvailabilityUnchanged,
                },
                None,
            );
        }
        if self.is_duplicate_normal_observation(observation.kind()) {
            return self.result(
                AdapterPublishStatus::Ignored {
                    reason: ObservationIgnoredReason::DuplicateObservation,
                },
                availability_transition,
            );
        }

        let evidence_count = match self.flush_evidence_loss(observation.observed_at_utc(), sink) {
            LossFlush::NotPending => 0,
            LossFlush::Published => 1,
            LossFlush::Backpressured => {
                return self.result(AdapterPublishStatus::Backpressured, availability_transition);
            }
            LossFlush::SinkClosed => {
                return self.result(AdapterPublishStatus::SinkClosed, availability_transition);
            }
            LossFlush::SequenceExhausted => {
                return self.result(
                    AdapterPublishStatus::SequenceExhausted,
                    availability_transition,
                );
            }
        };

        let evidence = match self.evidence_for(observation) {
            EvidenceCreation::Evidence(evidence) => evidence,
            EvidenceCreation::NotRequired => {
                return self.result(
                    Self::status_after_state_change(evidence_count),
                    availability_transition,
                );
            }
            EvidenceCreation::SequenceExhausted => {
                return self.result(
                    AdapterPublishStatus::SequenceExhausted,
                    availability_transition,
                );
            }
        };
        match self.publish(evidence, sink) {
            PublicationAttempt::Accepted => {
                self.last_published_kind = Some(observation.kind());
                evidence_count += 1;
                self.result(
                    AdapterPublishStatus::Emitted { evidence_count },
                    availability_transition,
                )
            }
            PublicationAttempt::Backpressured => {
                self.mark_evidence_loss();
                self.result(AdapterPublishStatus::Backpressured, availability_transition)
            }
            PublicationAttempt::SinkClosed => {
                self.mark_evidence_loss();
                self.result(AdapterPublishStatus::SinkClosed, availability_transition)
            }
            PublicationAttempt::SequenceExhausted => self.result(
                AdapterPublishStatus::SequenceExhausted,
                availability_transition,
            ),
        }
    }

    fn is_reordered(&self, source_order: u64) -> bool {
        self.last_source_order
            .is_some_and(|last| source_order <= last)
    }

    fn apply_availability(
        &mut self,
        kind: AdapterObservationKind,
    ) -> Option<AvailabilityTransition> {
        let AdapterObservationKind::Availability(current) = kind else {
            return None;
        };
        if self.availability == Some(current) {
            return None;
        }
        let transition = AvailabilityTransition::new(self.availability, current);
        self.availability = Some(current);
        if current == AdapterAvailability::EvidenceLost {
            self.reconciliation_required = true;
            self.last_published_kind = None;
        }
        Some(transition)
    }

    fn is_unchanged_availability(
        &self,
        kind: AdapterObservationKind,
        transition: Option<AvailabilityTransition>,
    ) -> bool {
        matches!(kind, AdapterObservationKind::Availability(_)) && transition.is_none()
    }

    fn is_duplicate_normal_observation(&self, kind: AdapterObservationKind) -> bool {
        !matches!(kind, AdapterObservationKind::Availability(_))
            && !self.evidence_loss_pending
            && self.last_published_kind == Some(kind)
    }

    fn flush_evidence_loss(
        &mut self,
        observed_at_utc: UtcMicros,
        sink: &dyn TrackingEvidenceSink,
    ) -> LossFlush {
        if !self.evidence_loss_pending {
            return LossFlush::NotPending;
        }
        let Ok(evidence) = self.next_evidence(|sequence| TrackingEvidence::EvidenceQueueOverflow {
            sequence,
            observed_at_utc,
        }) else {
            return LossFlush::SequenceExhausted;
        };
        match sink.try_publish(evidence) {
            EvidencePublishResult::Accepted => {
                self.evidence_loss_pending = false;
                self.last_published_kind = None;
                LossFlush::Published
            }
            EvidencePublishResult::Full => LossFlush::Backpressured,
            EvidencePublishResult::Closed => LossFlush::SinkClosed,
        }
    }

    fn evidence_for(&mut self, observation: AdapterObservation) -> EvidenceCreation {
        let occurred_at_utc = observation.observed_at_utc();
        match observation.kind() {
            AdapterObservationKind::Foreground { application_id } => {
                Self::from_next_evidence(self.next_evidence(|sequence| {
                    TrackingEvidence::Foreground {
                        sequence,
                        observed_at_utc: occurred_at_utc,
                        application_id,
                    }
                }))
            }
            AdapterObservationKind::ExcludedForeground => {
                Self::from_next_evidence(self.next_evidence(|sequence| {
                    TrackingEvidence::ExcludedForeground {
                        sequence,
                        observed_at_utc: occurred_at_utc,
                    }
                }))
            }
            AdapterObservationKind::IdleThresholdCrossed => {
                Self::from_next_evidence(self.next_evidence(|sequence| {
                    TrackingEvidence::IdleThresholdCrossed {
                        sequence,
                        observed_at_utc: occurred_at_utc,
                    }
                }))
            }
            AdapterObservationKind::SessionLocked => {
                Self::from_next_evidence(self.next_evidence(|sequence| {
                    TrackingEvidence::SessionLocked {
                        sequence,
                        observed_at_utc: occurred_at_utc,
                    }
                }))
            }
            AdapterObservationKind::SessionDisconnected => {
                Self::from_next_evidence(self.next_evidence(|sequence| {
                    TrackingEvidence::SessionDisconnected {
                        sequence,
                        observed_at_utc: occurred_at_utc,
                    }
                }))
            }
            AdapterObservationKind::SystemSuspended => {
                Self::from_next_evidence(self.next_evidence(|sequence| {
                    TrackingEvidence::SystemSuspended {
                        sequence,
                        observed_at_utc: occurred_at_utc,
                    }
                }))
            }
            AdapterObservationKind::ClockDiscontinuity => {
                Self::from_next_evidence(self.next_evidence(|sequence| {
                    TrackingEvidence::ClockDiscontinuity {
                        sequence,
                        observed_at_utc: occurred_at_utc,
                    }
                }))
            }
            AdapterObservationKind::Availability(availability) => {
                self.evidence_for_availability(availability, occurred_at_utc)
            }
        }
    }

    fn evidence_for_availability(
        &mut self,
        availability: AdapterAvailability,
        observed_at_utc: UtcMicros,
    ) -> EvidenceCreation {
        match availability {
            AdapterAvailability::Starting => {
                Self::from_next_evidence(self.next_evidence(|sequence| {
                    TrackingEvidence::AdapterStarting {
                        sequence,
                        observed_at_utc,
                    }
                }))
            }
            AdapterAvailability::PermissionRequired => {
                Self::from_next_evidence(self.next_evidence(|sequence| {
                    TrackingEvidence::AdapterPermissionLost {
                        sequence,
                        observed_at_utc,
                    }
                }))
            }
            AdapterAvailability::TemporarilyUnavailable
            | AdapterAvailability::Fatal
            | AdapterAvailability::Unsupported => {
                Self::from_next_evidence(self.next_evidence(|sequence| {
                    TrackingEvidence::AdapterFailure {
                        sequence,
                        observed_at_utc,
                    }
                }))
            }
            AdapterAvailability::EvidenceLost => {
                Self::from_next_evidence(self.next_evidence(|sequence| {
                    TrackingEvidence::EvidenceQueueOverflow {
                        sequence,
                        observed_at_utc,
                    }
                }))
            }
            AdapterAvailability::Ready
            | AdapterAvailability::Degraded { .. }
            | AdapterAvailability::HelperRequired
            | AdapterAvailability::Stopping => EvidenceCreation::NotRequired,
        }
    }

    fn next_evidence(
        &mut self,
        create: impl FnOnce(u64) -> TrackingEvidence,
    ) -> Result<TrackingEvidence, ()> {
        let Some(sequence) = self.next_sequence else {
            return Err(());
        };
        self.next_sequence = sequence.checked_add(1);
        Ok(create(sequence))
    }

    fn from_next_evidence(result: Result<TrackingEvidence, ()>) -> EvidenceCreation {
        match result {
            Ok(evidence) => EvidenceCreation::Evidence(evidence),
            Err(()) => EvidenceCreation::SequenceExhausted,
        }
    }

    fn publish(
        &mut self,
        evidence: TrackingEvidence,
        sink: &dyn TrackingEvidenceSink,
    ) -> PublicationAttempt {
        match sink.try_publish(evidence) {
            EvidencePublishResult::Accepted => PublicationAttempt::Accepted,
            EvidencePublishResult::Full => PublicationAttempt::Backpressured,
            EvidencePublishResult::Closed => PublicationAttempt::SinkClosed,
        }
    }

    fn mark_evidence_loss(&mut self) {
        self.evidence_loss_pending = true;
        self.reconciliation_required = true;
        self.last_published_kind = None;
        self.availability = Some(AdapterAvailability::EvidenceLost);
    }

    fn result(
        &self,
        status: AdapterPublishStatus,
        availability_transition: Option<AvailabilityTransition>,
    ) -> AdapterPublishResult {
        AdapterPublishResult::new(
            status,
            availability_transition,
            self.reconciliation_required,
        )
    }

    const fn status_after_state_change(evidence_count: u8) -> AdapterPublishStatus {
        if evidence_count == 0 {
            AdapterPublishStatus::StateChanged
        } else {
            AdapterPublishStatus::Emitted { evidence_count }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationAttempt {
    Accepted,
    Backpressured,
    SinkClosed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LossFlush {
    NotPending,
    Published,
    Backpressured,
    SinkClosed,
    SequenceExhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EvidenceCreation {
    Evidence(TrackingEvidence),
    NotRequired,
    SequenceExhausted,
}

#[cfg(test)]
mod tests {
    use openmanic_application::TrackingEvidence;
    use openmanic_domain::{ApplicationId, UtcMicros};

    use crate::{
        AdapterAvailability, AdapterObservation, AdapterObservationKind, AdapterPublishStatus,
        EvidencePublishResult, PlatformEventNormalizer, TrackingEvidenceSink,
    };

    #[derive(Default)]
    struct RecordingSink {
        evidence: std::sync::Mutex<Vec<TrackingEvidence>>,
        capacity: std::sync::atomic::AtomicUsize,
    }

    impl RecordingSink {
        fn with_capacity(capacity: usize) -> Self {
            Self {
                evidence: std::sync::Mutex::new(Vec::new()),
                capacity: std::sync::atomic::AtomicUsize::new(capacity),
            }
        }

        fn set_capacity(&self, capacity: usize) {
            self.capacity
                .store(capacity, std::sync::atomic::Ordering::Relaxed);
        }

        fn evidence(&self) -> Vec<TrackingEvidence> {
            match self.evidence.lock() {
                Ok(evidence) => evidence.clone(),
                Err(poisoned) => poisoned.into_inner().clone(),
            }
        }

        fn clear(&self) {
            match self.evidence.lock() {
                Ok(mut evidence) => evidence.clear(),
                Err(poisoned) => poisoned.into_inner().clear(),
            }
        }
    }

    impl TrackingEvidenceSink for RecordingSink {
        fn try_publish(&self, evidence: TrackingEvidence) -> EvidencePublishResult {
            match self.evidence.try_lock() {
                Ok(mut stored) => {
                    if stored.len() == self.capacity.load(std::sync::atomic::Ordering::Relaxed) {
                        EvidencePublishResult::Full
                    } else {
                        stored.push(evidence);
                        EvidencePublishResult::Accepted
                    }
                }
                Err(std::sync::TryLockError::WouldBlock) => EvidencePublishResult::Full,
                Err(std::sync::TryLockError::Poisoned(_)) => EvidencePublishResult::Closed,
            }
        }
    }

    #[test]
    fn full_sink_marks_explicit_loss_before_a_fresh_observation() {
        let mut normalizer = PlatformEventNormalizer::new();
        let sink = RecordingSink::with_capacity(1);

        assert_eq!(
            normalizer
                .normalize_and_publish(foreground(1, 10, 7), &sink)
                .status(),
            AdapterPublishStatus::Emitted { evidence_count: 1 }
        );
        let overflow = normalizer.normalize_and_publish(foreground(2, 20, 8), &sink);
        assert_eq!(overflow.status(), AdapterPublishStatus::Backpressured);
        assert!(overflow.reconciliation_required());

        sink.clear();
        sink.set_capacity(2);
        let recovered = normalizer.normalize_and_publish(foreground(3, 30, 8), &sink);
        assert_eq!(
            recovered.status(),
            AdapterPublishStatus::Emitted { evidence_count: 2 },
            "loss is explicit before a fresh current foreground observation"
        );
        assert_eq!(sink.evidence()[0].sequence(), 3);
        assert!(matches!(
            sink.evidence()[0],
            TrackingEvidence::EvidenceQueueOverflow { .. }
        ));

        assert!(matches!(
            sink.evidence()[1],
            TrackingEvidence::Foreground {
                sequence: 4,
                application_id,
                ..
            } if application_id == ApplicationId::new(8)
        ));
    }

    #[test]
    fn availability_transitions_are_modeled_without_raw_platform_errors() {
        let mut normalizer = PlatformEventNormalizer::new();
        let sink = RecordingSink::with_capacity(4);

        let starting = normalizer
            .normalize_and_publish(availability(1, 10, AdapterAvailability::Starting), &sink);
        assert_eq!(
            starting
                .availability_transition()
                .map(|transition| transition.previous()),
            Some(None)
        );
        assert_eq!(
            normalizer
                .normalize_and_publish(availability(2, 20, AdapterAvailability::Ready), &sink)
                .status(),
            AdapterPublishStatus::StateChanged
        );
        let unavailable = normalizer.normalize_and_publish(
            availability(3, 30, AdapterAvailability::TemporarilyUnavailable),
            &sink,
        );

        assert_eq!(
            normalizer.availability(),
            AdapterAvailability::TemporarilyUnavailable
        );
        assert_eq!(
            unavailable
                .availability_transition()
                .map(|transition| transition.current()),
            Some(AdapterAvailability::TemporarilyUnavailable)
        );
        assert!(matches!(
            sink.evidence()[1],
            TrackingEvidence::AdapterFailure { sequence: 2, .. }
        ));
    }

    #[test]
    fn duplicate_reordered_and_rapid_switch_observations_have_honest_ordering() {
        let mut normalizer = PlatformEventNormalizer::new();
        let sink = RecordingSink::with_capacity(4);

        normalizer.normalize_and_publish(foreground(10, 10, 1), &sink);
        let duplicate = normalizer.normalize_and_publish(foreground(11, 20, 1), &sink);
        let reordered = normalizer.normalize_and_publish(foreground(9, 30, 2), &sink);
        normalizer.normalize_and_publish(foreground(12, 40, 2), &sink);
        normalizer.normalize_and_publish(foreground(13, 50, 1), &sink);

        assert!(matches!(
            duplicate.status(),
            AdapterPublishStatus::Ignored { .. }
        ));
        assert!(matches!(
            reordered.status(),
            AdapterPublishStatus::Ignored { .. }
        ));
        assert_eq!(
            sink.evidence(),
            vec![
                TrackingEvidence::Foreground {
                    sequence: 1,
                    observed_at_utc: UtcMicros::new(10),
                    application_id: ApplicationId::new(1),
                },
                TrackingEvidence::Foreground {
                    sequence: 2,
                    observed_at_utc: UtcMicros::new(40),
                    application_id: ApplicationId::new(2),
                },
                TrackingEvidence::Foreground {
                    sequence: 3,
                    observed_at_utc: UtcMicros::new(50),
                    application_id: ApplicationId::new(1),
                },
            ]
        );
    }

    fn foreground(
        source_order: u64,
        observed_at_utc: i64,
        application_id: u64,
    ) -> AdapterObservation {
        AdapterObservation::new(
            source_order,
            UtcMicros::new(observed_at_utc),
            AdapterObservationKind::Foreground {
                application_id: ApplicationId::new(application_id),
            },
        )
    }

    fn availability(
        source_order: u64,
        observed_at_utc: i64,
        availability: AdapterAvailability,
    ) -> AdapterObservation {
        AdapterObservation::new(
            source_order,
            UtcMicros::new(observed_at_utc),
            AdapterObservationKind::Availability(availability),
        )
    }
}
