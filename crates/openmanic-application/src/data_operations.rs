//! Typed contracts for local CSV interchange and full-fidelity data operations.
//!
//! Concrete workers own streaming file I/O. This application boundary captures the explicit
//! scope, destination, privacy disclosure, cancellation, and authoritative outcome needed by
//! presentation and persistence adapters.

use std::path::PathBuf;

use openmanic_domain::{HalfOpenInterval, UtcMicros};

use crate::{DataRevision, JobId};

/// The fixed first MVP CSV interchange version.
pub const CSV_INTERCHANGE_VERSION: u16 = 1;

/// A user-visible local data-operation destination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataOperationDestination {
    path: PathBuf,
}

impl DataOperationDestination {
    /// Creates a destination selected explicitly by the user.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Returns the selected local path without opening or creating it.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

/// Whether exported CSV includes privacy-sensitive window-title text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TitleDisclosure {
    /// Exclude title text from every exported row.
    Exclude,
    /// Include title text after an explicit user confirmation.
    IncludeAfterConfirmation,
}

/// One explicitly confirmed CSV export request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CsvExportRequest {
    job_id: JobId,
    range: HalfOpenInterval,
    destination: DataOperationDestination,
    title_disclosure: TitleDisclosure,
}

impl CsvExportRequest {
    /// Creates a bounded export request after destination and title disclosure are confirmed.
    #[must_use]
    pub const fn new(
        job_id: JobId,
        range: HalfOpenInterval,
        destination: DataOperationDestination,
        title_disclosure: TitleDisclosure,
    ) -> Self {
        Self {
            job_id,
            range,
            destination,
            title_disclosure,
        }
    }

    /// Returns the stable background job identity.
    #[must_use]
    pub const fn job_id(&self) -> JobId {
        self.job_id
    }

    /// Returns the half-open UTC scope to export.
    #[must_use]
    pub const fn range(&self) -> HalfOpenInterval {
        self.range
    }

    /// Returns the explicitly selected destination.
    #[must_use]
    pub const fn destination(&self) -> &DataOperationDestination {
        &self.destination
    }

    /// Returns the title privacy disclosure selected for this operation.
    #[must_use]
    pub const fn title_disclosure(&self) -> TitleDisclosure {
        self.title_disclosure
    }
}

/// A stable row count and exact data revision produced by a completed operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DataOperationOutcome {
    row_count: u64,
    source_revision: DataRevision,
    completed_at_utc: UtcMicros,
}

impl DataOperationOutcome {
    /// Creates one authoritative successful operation outcome.
    #[must_use]
    pub const fn new(
        row_count: u64,
        source_revision: DataRevision,
        completed_at_utc: UtcMicros,
    ) -> Self {
        Self {
            row_count,
            source_revision,
            completed_at_utc,
        }
    }

    /// Returns the exact streamed row count.
    #[must_use]
    pub const fn row_count(self) -> u64 {
        self.row_count
    }

    /// Returns the correlated source revision.
    #[must_use]
    pub const fn source_revision(self) -> DataRevision {
        self.source_revision
    }

    /// Returns the authoritative completion instant.
    #[must_use]
    pub const fn completed_at_utc(self) -> UtcMicros {
        self.completed_at_utc
    }
}

/// Progress suitable for a retained named job without claiming a false final result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DataOperationProgress {
    completed: u64,
    total: Option<u64>,
}

impl DataOperationProgress {
    /// Creates progress after validating that a known total is not below completed work.
    ///
    /// # Errors
    ///
    /// Returns [`DataOperationProgressError`] when a known total is less than completed work.
    pub fn try_new(completed: u64, total: Option<u64>) -> Result<Self, DataOperationProgressError> {
        if total.is_some_and(|total| total < completed) {
            return Err(DataOperationProgressError::TotalBeforeCompleted);
        }
        Ok(Self { completed, total })
    }

    /// Returns completed stream records.
    #[must_use]
    pub const fn completed(self) -> u64 {
        self.completed
    }

    /// Returns the known total when discovery has completed.
    #[must_use]
    pub const fn total(self) -> Option<u64> {
        self.total
    }
}

/// Progress validation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataOperationProgressError {
    /// The claimed total is below completed work.
    TotalBeforeCompleted,
}

/// The explicit destination policy for one CSV import.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportDestinationScope {
    /// Merge validated records into the current local store.
    CurrentStore,
}

/// A stable 128-bit public identity for one persisted import batch.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ImportBatchId([u8; 16]);

impl ImportBatchId {
    /// Creates a caller-generated batch identity before persistence begins.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the exact SQLite BLOB/interchange representation.
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 16] {
        self.0
    }
}

/// One explicitly initiated CSV import request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CsvImportRequest {
    job_id: JobId,
    batch_id: ImportBatchId,
    source: DataOperationDestination,
    destination_scope: ImportDestinationScope,
}

impl CsvImportRequest {
    /// Creates an import request with an explicit local source and destination scope.
    #[must_use]
    pub const fn new(
        job_id: JobId,
        batch_id: ImportBatchId,
        source: DataOperationDestination,
        destination_scope: ImportDestinationScope,
    ) -> Self {
        Self {
            job_id,
            batch_id,
            source,
            destination_scope,
        }
    }

    /// Returns the stable job identity used for progress and final reporting.
    #[must_use]
    pub const fn job_id(&self) -> JobId {
        self.job_id
    }

    /// Returns the durable batch identity distinct from the transient job ID.
    #[must_use]
    pub const fn batch_id(&self) -> ImportBatchId {
        self.batch_id
    }

    /// Returns the explicitly selected local CSV source.
    #[must_use]
    pub const fn source(&self) -> &DataOperationDestination {
        &self.source
    }

    /// Returns the scope that must be stated before merge work begins.
    #[must_use]
    pub const fn destination_scope(&self) -> ImportDestinationScope {
        self.destination_scope
    }
}

/// A privacy-safe row-level validation failure retained for an import batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportFailure {
    line: u64,
    field: Option<String>,
    code: &'static str,
}

impl ImportFailure {
    /// Creates a bounded, row-numbered import validation failure.
    ///
    /// # Errors
    ///
    /// Returns [`ImportFailureError`] when the source line is zero or the stable code is empty.
    pub fn try_new(
        line: u64,
        field: Option<String>,
        code: &'static str,
    ) -> Result<Self, ImportFailureError> {
        if line == 0 {
            return Err(ImportFailureError::ZeroLine);
        }
        if code.trim().is_empty() {
            return Err(ImportFailureError::EmptyCode);
        }
        Ok(Self { line, field, code })
    }

    /// Returns the one-based source CSV line.
    #[must_use]
    pub const fn line(&self) -> u64 {
        self.line
    }

    /// Returns the named malformed field where one is known.
    #[must_use]
    pub fn field(&self) -> Option<&str> {
        self.field.as_deref()
    }

    /// Returns the fixed safe error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

/// Import failure construction error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportFailureError {
    /// CSV source lines are one-based.
    ZeroLine,
    /// A failure must have a fixed code suitable for persistence and presentation.
    EmptyCode,
}

/// Exact accepted/rejected/committed counts reported by a finished or cancelled import.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImportScopeOutcome {
    parsed: u64,
    accepted: u64,
    rejected: u64,
    committed: u64,
}

impl ImportScopeOutcome {
    /// Creates validated import counts that cannot claim hidden committed success.
    ///
    /// # Errors
    ///
    /// Returns [`ImportScopeError`] when the counts are internally inconsistent.
    pub fn try_new(
        parsed: u64,
        accepted: u64,
        rejected: u64,
        committed: u64,
    ) -> Result<Self, ImportScopeError> {
        if accepted > parsed || rejected > parsed || accepted.saturating_add(rejected) != parsed {
            return Err(ImportScopeError::ParsedCountsInconsistent);
        }
        if committed > accepted {
            return Err(ImportScopeError::CommittedBeyondAccepted);
        }
        Ok(Self {
            parsed,
            accepted,
            rejected,
            committed,
        })
    }

    /// Returns parsed records.
    #[must_use]
    pub const fn parsed(self) -> u64 {
        self.parsed
    }

    /// Returns accepted records.
    #[must_use]
    pub const fn accepted(self) -> u64 {
        self.accepted
    }

    /// Returns rejected records.
    #[must_use]
    pub const fn rejected(self) -> u64 {
        self.rejected
    }

    /// Returns exactly committed records, including zero for cancellation before merge.
    #[must_use]
    pub const fn committed(self) -> u64 {
        self.committed
    }
}

/// Import count validation error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportScopeError {
    /// Accepted and rejected counts do not partition parsed records.
    ParsedCountsInconsistent,
    /// A committed count cannot exceed validated records.
    CommittedBeyondAccepted,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use openmanic_domain::{HalfOpenInterval, UtcMicros};

    use super::{
        CsvExportRequest, DataOperationDestination, DataOperationProgress, ImportFailure,
        ImportScopeOutcome, TitleDisclosure,
    };
    use crate::JobId;

    #[test]
    fn export_request_retains_explicit_scope_destination_and_title_disclosure() {
        let range = HalfOpenInterval::try_new(UtcMicros::new(10), UtcMicros::new(20))
            .expect("positive fixture range");
        let request = CsvExportRequest::new(
            JobId::new(7),
            range,
            DataOperationDestination::new(PathBuf::from("export.csv")),
            TitleDisclosure::Exclude,
        );

        assert_eq!(request.job_id(), JobId::new(7));
        assert_eq!(request.range(), range);
        assert_eq!(request.destination().path(), PathBuf::from("export.csv"));
        assert_eq!(request.title_disclosure(), TitleDisclosure::Exclude);
    }

    #[test]
    fn progress_rejects_a_total_below_completed_work() {
        assert!(DataOperationProgress::try_new(10, Some(9)).is_err());
        assert_eq!(DataOperationProgress::try_new(10, Some(10)).map(DataOperationProgress::completed), Ok(10));
    }

    #[test]
    fn import_scope_reports_exact_committed_work_without_hidden_success() {
        assert_eq!(
            ImportScopeOutcome::try_new(10, 7, 3, 5).map(ImportScopeOutcome::committed),
            Ok(5)
        );
        assert!(ImportScopeOutcome::try_new(10, 7, 3, 8).is_err());
        assert!(ImportFailure::try_new(0, None, "invalid.row").is_err());
    }
}
