//! Deterministic platform adapter and bounded sink fakes for application and adapter tests.

use std::{
    collections::VecDeque,
    sync::{Mutex, TryLockError},
};

use openmanic_application::TrackingEvidence;

use crate::{
    AdapterAvailability, AdapterObservation, AdapterPublishResult, EvidencePublishResult,
    PlatformActivityAdapter, PlatformCapabilities, PlatformEventNormalizer, TrackingEvidenceSink,
};

/// A deterministic queue-backed platform adapter for tests outside this crate.
#[derive(Debug)]
pub struct FakePlatformAdapter {
    capabilities: PlatformCapabilities,
    normalizer: PlatformEventNormalizer,
    observations: VecDeque<AdapterObservation>,
}

impl FakePlatformAdapter {
    /// Creates an empty fake adapter with explicit probe results.
    #[must_use]
    pub fn new(capabilities: PlatformCapabilities) -> Self {
        Self {
            capabilities,
            normalizer: PlatformEventNormalizer::new(),
            observations: VecDeque::new(),
        }
    }

    /// Queues one resolved observation for deterministic later publication.
    pub fn queue(&mut self, observation: AdapterObservation) {
        self.observations.push_back(observation);
    }

    /// Publishes one queued observation, if any, without waiting for the sink.
    #[must_use]
    pub fn publish_next(
        &mut self,
        sink: &dyn TrackingEvidenceSink,
    ) -> Option<AdapterPublishResult> {
        self.observations
            .pop_front()
            .map(|observation| self.normalize_and_publish(observation, sink))
    }

    /// Returns the number of observations still queued in this fake.
    #[must_use]
    pub fn queued_observation_count(&self) -> usize {
        self.observations.len()
    }

    /// Clears a prior overflow requirement after the test simulates fresh reconciliation.
    pub fn acknowledge_reconciliation(&mut self) {
        self.normalizer.acknowledge_reconciliation();
    }
}

impl PlatformActivityAdapter for FakePlatformAdapter {
    fn capabilities(&self) -> PlatformCapabilities {
        self.capabilities
    }

    fn availability(&self) -> AdapterAvailability {
        self.normalizer.availability()
    }

    fn normalize_and_publish(
        &mut self,
        observation: AdapterObservation,
        sink: &dyn TrackingEvidenceSink,
    ) -> AdapterPublishResult {
        self.normalizer.normalize_and_publish(observation, sink)
    }
}

/// A deterministic fake sink that models capacity and shutdown without blocking callers.
#[derive(Debug)]
pub struct FakeEvidenceSink {
    state: Mutex<FakeEvidenceSinkState>,
}

#[derive(Debug)]
struct FakeEvidenceSinkState {
    capacity: usize,
    accepting: bool,
    evidence: Vec<TrackingEvidence>,
}

/// A nonblocking fake-sink inspection or control failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FakeEvidenceSinkError {
    /// Another test operation currently owns the fake sink state.
    Busy,
    /// A prior panic poisoned the fake sink state.
    Poisoned,
}

impl FakeEvidenceSink {
    /// Creates an accepting bounded fake sink with the given evidence capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(FakeEvidenceSinkState {
                capacity,
                accepting: true,
                evidence: Vec::new(),
            }),
        }
    }

    /// Returns the evidence currently accepted by this fake without consuming it.
    ///
    /// # Errors
    ///
    /// Returns [`FakeEvidenceSinkError`] if a concurrent fake operation owns or poisoned state.
    pub fn snapshot(&self) -> Result<Vec<TrackingEvidence>, FakeEvidenceSinkError> {
        self.with_state(|state| state.evidence.clone())
    }

    /// Removes and returns all accepted evidence.
    ///
    /// # Errors
    ///
    /// Returns [`FakeEvidenceSinkError`] if a concurrent fake operation owns or poisoned state.
    pub fn drain(&self) -> Result<Vec<TrackingEvidence>, FakeEvidenceSinkError> {
        self.with_state(|state| std::mem::take(&mut state.evidence))
    }

    /// Changes the bounded capacity used by later publication attempts.
    ///
    /// # Errors
    ///
    /// Returns [`FakeEvidenceSinkError`] if a concurrent fake operation owns or poisoned state.
    pub fn set_capacity(&self, capacity: usize) -> Result<(), FakeEvidenceSinkError> {
        self.with_state(|state| state.capacity = capacity)
    }

    /// Stops the fake from accepting later evidence.
    ///
    /// # Errors
    ///
    /// Returns [`FakeEvidenceSinkError`] if a concurrent fake operation owns or poisoned state.
    pub fn close(&self) -> Result<(), FakeEvidenceSinkError> {
        self.with_state(|state| state.accepting = false)
    }

    fn with_state<T>(
        &self,
        operation: impl FnOnce(&mut FakeEvidenceSinkState) -> T,
    ) -> Result<T, FakeEvidenceSinkError> {
        match self.state.try_lock() {
            Ok(mut state) => Ok(operation(&mut state)),
            Err(TryLockError::WouldBlock) => Err(FakeEvidenceSinkError::Busy),
            Err(TryLockError::Poisoned(_)) => Err(FakeEvidenceSinkError::Poisoned),
        }
    }
}

impl TrackingEvidenceSink for FakeEvidenceSink {
    fn try_publish(&self, evidence: TrackingEvidence) -> EvidencePublishResult {
        match self.with_state(|state| {
            if !state.accepting {
                EvidencePublishResult::Closed
            } else if state.evidence.len() == state.capacity {
                EvidencePublishResult::Full
            } else {
                state.evidence.push(evidence);
                EvidencePublishResult::Accepted
            }
        }) {
            Ok(result) => result,
            Err(FakeEvidenceSinkError::Busy | FakeEvidenceSinkError::Poisoned) => {
                EvidencePublishResult::Closed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use openmanic_application::TrackingEvidence;
    use openmanic_domain::{ApplicationId, UtcMicros};

    use crate::{
        AdapterObservation, AdapterObservationKind, AdapterPublishStatus, FakeEvidenceSink,
        FakePlatformAdapter, PlatformActivityAdapter, PlatformCapabilities,
    };

    #[test]
    fn fake_adapter_and_sink_make_application_evidence_deterministic() {
        let mut adapter = FakePlatformAdapter::new(PlatformCapabilities::unavailable());
        let sink = FakeEvidenceSink::new(2);
        adapter.queue(foreground(1, 10, 3));

        let published = adapter
            .publish_next(&sink)
            .expect("a queued fake observation produces one result");
        assert_eq!(
            published.status(),
            AdapterPublishStatus::Emitted { evidence_count: 1 }
        );
        assert_eq!(adapter.queued_observation_count(), 0);
        assert_eq!(
            sink.snapshot(),
            Ok(vec![TrackingEvidence::Foreground {
                sequence: 1,
                observed_at_utc: UtcMicros::new(10),
                application_id: ApplicationId::new(3),
            }])
        );
    }

    #[test]
    fn fake_sink_reports_full_and_closed_without_waiting() {
        let sink = FakeEvidenceSink::new(1);
        let evidence = TrackingEvidence::IdleThresholdCrossed {
            sequence: 1,
            observed_at_utc: UtcMicros::new(1),
        };

        assert_eq!(
            crate::TrackingEvidenceSink::try_publish(&sink, evidence),
            crate::EvidencePublishResult::Accepted
        );
        assert_eq!(
            crate::TrackingEvidenceSink::try_publish(&sink, evidence),
            crate::EvidencePublishResult::Full
        );
        assert_eq!(sink.close(), Ok(()));
        assert_eq!(
            crate::TrackingEvidenceSink::try_publish(&sink, evidence),
            crate::EvidencePublishResult::Closed
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
}
