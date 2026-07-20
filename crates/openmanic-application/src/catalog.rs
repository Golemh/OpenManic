//! Category and application catalog commands with an explicit persistence boundary.

use std::collections::BTreeSet;

use openmanic_domain::{Application, ApplicationId, Category, CategoryId, CategoryName, UtcMicros};

use crate::{
    CommandEnvelope, DataRevision, MutationConfirmation, MutationOutcome, MutationRejection,
    MutationRejectionReason,
};

/// A user-requested mutation of the application catalog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogCommand {
    /// Creates one category with a caller-assigned stable ID.
    CreateCategory {
        /// The validated category to create.
        category: Category,
        /// The authoritative command time used for the durable audit fields.
        observed_at_utc: UtcMicros,
    },
    /// Changes a category's display name without changing its assignments.
    RenameCategory {
        /// The category to rename.
        category_id: CategoryId,
        /// The already validated replacement name.
        name: CategoryName,
        /// The authoritative command time used for the durable audit fields.
        observed_at_utc: UtcMicros,
    },
    /// Deletes a category. Its assigned applications become Uncategorized.
    DeleteCategory {
        /// The category to delete.
        category_id: CategoryId,
    },
    /// Assigns the supplied applications to one category, or to Uncategorized.
    AssignApplications {
        /// The distinct applications selected by an explicit user action.
        application_ids: Vec<ApplicationId>,
        /// `None` explicitly means Uncategorized.
        category_id: Option<CategoryId>,
    },
    /// Enables or disables privacy-preserving exclusion for selected applications.
    SetApplicationsExcluded {
        /// The distinct applications selected by an explicit user action.
        application_ids: Vec<ApplicationId>,
        /// Whether future observations should persist only Excluded evidence.
        excluded: bool,
    },
}

impl CatalogCommand {
    /// Creates a bulk assignment after rejecting an empty or duplicate selection.
    ///
    /// A selection itself never creates this command; callers construct it only after the user
    /// confirms an assignment action.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogCommandError`] when no application was selected or an application appears
    /// more than once in the requested bulk assignment.
    pub fn try_assign_applications(
        application_ids: impl IntoIterator<Item = ApplicationId>,
        category_id: Option<CategoryId>,
    ) -> Result<Self, CatalogCommandError> {
        let application_ids = application_ids.into_iter().collect::<Vec<_>>();
        if application_ids.is_empty() {
            return Err(CatalogCommandError::EmptyApplicationSelection);
        }
        let distinct = application_ids.iter().copied().collect::<BTreeSet<_>>();
        if distinct.len() != application_ids.len() {
            return Err(CatalogCommandError::DuplicateApplicationSelection);
        }
        Ok(Self::AssignApplications {
            application_ids,
            category_id,
        })
    }

    /// Creates an explicit bulk exclusion-policy change after validating its selection.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogCommandError`] when the selection is empty or contains duplicates.
    pub fn try_set_applications_excluded(
        application_ids: impl IntoIterator<Item = ApplicationId>,
        excluded: bool,
    ) -> Result<Self, CatalogCommandError> {
        let application_ids = application_ids.into_iter().collect::<Vec<_>>();
        if application_ids.is_empty() {
            return Err(CatalogCommandError::EmptyApplicationSelection);
        }
        let distinct = application_ids.iter().copied().collect::<BTreeSet<_>>();
        if distinct.len() != application_ids.len() {
            return Err(CatalogCommandError::DuplicateApplicationSelection);
        }
        Ok(Self::SetApplicationsExcluded {
            application_ids,
            excluded,
        })
    }
}

/// Validation failure while constructing a catalog command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogCommandError {
    /// Bulk assignment requires at least one explicitly selected application.
    EmptyApplicationSelection,
    /// Bulk assignment must not write the same application more than once.
    DuplicateApplicationSelection,
}

/// The persistence operations required by [`CatalogService`].
pub trait CatalogPersistence {
    /// Creates one category and returns its committed store revision.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogPersistenceError`] when the category cannot be created.
    fn create_category(
        &mut self,
        category: &Category,
        observed_at_utc: UtcMicros,
    ) -> Result<DataRevision, CatalogPersistenceError>;

    /// Renames one existing category and returns its committed store revision.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogPersistenceError`] when the category no longer exists or cannot be
    /// renamed.
    fn rename_category(
        &mut self,
        category_id: CategoryId,
        name: &CategoryName,
        observed_at_utc: UtcMicros,
    ) -> Result<DataRevision, CatalogPersistenceError>;

    /// Deletes one existing category and returns its committed store revision.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogPersistenceError`] when the category no longer exists or cannot be
    /// deleted.
    fn delete_category(
        &mut self,
        category_id: CategoryId,
    ) -> Result<DataRevision, CatalogPersistenceError>;

    /// Reassigns every supplied existing application in one revision.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogPersistenceError`] when a selected application or category no longer
    /// exists, or the assignment cannot be committed.
    fn assign_applications(
        &mut self,
        application_ids: &[ApplicationId],
        category_id: Option<CategoryId>,
    ) -> Result<DataRevision, CatalogPersistenceError>;

    /// Sets the privacy exclusion policy for every supplied existing application.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogPersistenceError`] when an application no longer exists or the policy
    /// change cannot be committed.
    fn set_applications_excluded(
        &mut self,
        application_ids: &[ApplicationId],
        excluded: bool,
    ) -> Result<DataRevision, CatalogPersistenceError>;
}

/// Stable failure category returned by the catalog persistence boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogPersistenceError {
    /// A category or application named by the command no longer exists.
    NotFound,
    /// The persistence adapter could not commit the requested mutation.
    Failed,
}

/// Limits a catalog query to all applications, one category, or Uncategorized applications.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogAssignmentFilter {
    /// Retains every application regardless of its current assignment.
    All,
    /// Retains applications assigned to the stated category.
    Category(CategoryId),
    /// Retains applications with no category assignment.
    Uncategorized,
}

/// Immutable catalog query supplied by a screen controller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogFilter {
    search_folded: String,
    assignment: CatalogAssignmentFilter,
}

impl CatalogFilter {
    /// Creates a query whose text is trimmed and compared case-insensitively.
    #[must_use]
    pub fn new(search: impl AsRef<str>, assignment: CatalogAssignmentFilter) -> Self {
        Self {
            search_folded: search.as_ref().trim().to_lowercase(),
            assignment,
        }
    }

    /// Returns the selected assignment scope.
    #[must_use]
    pub const fn assignment(&self) -> CatalogAssignmentFilter {
        self.assignment
    }

    fn matches(&self, application: &CatalogApplicationSnapshot) -> bool {
        let assignment_matches = match self.assignment {
            CatalogAssignmentFilter::All => true,
            CatalogAssignmentFilter::Category(category_id) => {
                application.category_id == Some(category_id)
            }
            CatalogAssignmentFilter::Uncategorized => application.category_id.is_none(),
        };
        assignment_matches
            && (self.search_folded.is_empty()
                || application
                    .display_name
                    .to_lowercase()
                    .contains(&self.search_folded))
    }
}

/// One immutable application row for category search, selection, and assignment actions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogApplicationSnapshot {
    application_id: ApplicationId,
    display_name: String,
    category_id: Option<CategoryId>,
}

impl CatalogApplicationSnapshot {
    /// Returns the stable application identity used by selection and mutation commands.
    #[must_use]
    pub const fn application_id(&self) -> ApplicationId {
        self.application_id
    }

    /// Returns the current application label.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Returns the current category, or `None` for Uncategorized.
    #[must_use]
    pub const fn category_id(&self) -> Option<CategoryId> {
        self.category_id
    }
}

/// One immutable category row for category selection and editing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogCategorySnapshot {
    category_id: CategoryId,
    display_name: String,
}

impl CatalogCategorySnapshot {
    /// Returns the stable category identity.
    #[must_use]
    pub const fn category_id(&self) -> CategoryId {
        self.category_id
    }

    /// Returns the current category label.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }
}

/// A correlated immutable catalog read at one committed data revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogSnapshot {
    revision: DataRevision,
    applications: Vec<CatalogApplicationSnapshot>,
    categories: Vec<CatalogCategorySnapshot>,
}

impl CatalogSnapshot {
    /// Builds a stable presentation snapshot from correlated domain catalog facts.
    #[must_use]
    pub fn from_domain(
        revision: DataRevision,
        applications: impl IntoIterator<Item = Application>,
        categories: impl IntoIterator<Item = Category>,
    ) -> Self {
        let mut applications = applications
            .into_iter()
            .map(|application| CatalogApplicationSnapshot {
                application_id: application.id(),
                display_name: application.name().as_str().to_owned(),
                category_id: application.category_id(),
            })
            .collect::<Vec<_>>();
        applications.sort_by(|left, right| {
            left.display_name
                .cmp(&right.display_name)
                .then(left.application_id.cmp(&right.application_id))
        });
        let mut categories = categories
            .into_iter()
            .map(|category| CatalogCategorySnapshot {
                category_id: category.id(),
                display_name: category.name().as_str().to_owned(),
            })
            .collect::<Vec<_>>();
        categories.sort_by(|left, right| {
            left.display_name
                .cmp(&right.display_name)
                .then(left.category_id.cmp(&right.category_id))
        });
        Self {
            revision,
            applications,
            categories,
        }
    }

    /// Returns the committed revision shared by every catalog row.
    #[must_use]
    pub const fn revision(&self) -> DataRevision {
        self.revision
    }

    /// Returns every current application in deterministic display order.
    #[must_use]
    pub fn applications(&self) -> &[CatalogApplicationSnapshot] {
        &self.applications
    }

    /// Returns every current category in deterministic display order.
    #[must_use]
    pub fn categories(&self) -> &[CatalogCategorySnapshot] {
        &self.categories
    }

    /// Returns filtered application rows without changing this immutable snapshot.
    #[must_use]
    pub fn filtered_applications(&self, filter: &CatalogFilter) -> Vec<CatalogApplicationSnapshot> {
        self.applications
            .iter()
            .filter(|application| filter.matches(application))
            .cloned()
            .collect()
    }
}

/// Handles explicit catalog mutations and correlates their authoritative outcomes.
pub struct CatalogService<P> {
    persistence: P,
}

impl<P> CatalogService<P>
where
    P: CatalogPersistence,
{
    /// Creates a catalog service around its exclusive persistence port.
    #[must_use]
    pub const fn new(persistence: P) -> Self {
        Self { persistence }
    }

    /// Handles one catalog command without mutating state for an invalid selection.
    #[must_use]
    pub fn handle(&mut self, command: &CommandEnvelope<CatalogCommand>) -> MutationOutcome {
        let command_id = command.command_id();
        let result = match command.payload() {
            CatalogCommand::CreateCategory {
                category,
                observed_at_utc,
            } => self.persistence.create_category(category, *observed_at_utc),
            CatalogCommand::RenameCategory {
                category_id,
                name,
                observed_at_utc,
            } => self
                .persistence
                .rename_category(*category_id, name, *observed_at_utc),
            CatalogCommand::DeleteCategory { category_id } => {
                self.persistence.delete_category(*category_id)
            }
            CatalogCommand::AssignApplications {
                application_ids,
                category_id,
            } => self
                .persistence
                .assign_applications(application_ids, *category_id),
            CatalogCommand::SetApplicationsExcluded {
                application_ids,
                excluded,
            } => self
                .persistence
                .set_applications_excluded(application_ids, *excluded),
        };
        match result {
            Ok(revision) => {
                MutationOutcome::Confirmed(MutationConfirmation::new(command_id, revision))
            }
            Err(CatalogPersistenceError::NotFound) => MutationOutcome::Rejected(
                MutationRejection::new(command_id, MutationRejectionReason::RevisionConflict),
            ),
            Err(CatalogPersistenceError::Failed) => MutationOutcome::Rejected(
                MutationRejection::new(command_id, MutationRejectionReason::PersistenceFailure),
            ),
        }
    }

    /// Returns the exclusive persistence port after service shutdown.
    #[must_use]
    pub fn into_persistence(self) -> P {
        self.persistence
    }
}

#[cfg(test)]
mod tests {
    use openmanic_domain::{
        Application, ApplicationId, ApplicationName, Category, CategoryId, CategoryName, UtcMicros,
    };

    use super::{
        CatalogAssignmentFilter, CatalogCommand, CatalogCommandError, CatalogFilter,
        CatalogPersistence, CatalogPersistenceError, CatalogService, CatalogSnapshot,
    };

    #[test]
    fn bulk_assignment_requires_a_nonduplicated_explicit_selection() {
        let application = ApplicationId::from_bytes([1; 16]);
        assert_eq!(
            CatalogCommand::try_assign_applications([], None),
            Err(CatalogCommandError::EmptyApplicationSelection)
        );
        assert_eq!(
            CatalogCommand::try_assign_applications([application, application], None),
            Err(CatalogCommandError::DuplicateApplicationSelection)
        );
        assert!(matches!(
            CatalogCommand::try_assign_applications(
                [application],
                Some(CategoryId::from_bytes([2; 16]))
            ),
            Ok(CatalogCommand::AssignApplications { .. })
        ));
        assert!(matches!(
            CatalogCommand::try_set_applications_excluded([application], true),
            Ok(CatalogCommand::SetApplicationsExcluded {
                excluded: true,
                ..
            })
        ));
    }

    #[test]
    fn category_values_remain_domain_validated_before_commands_are_built() {
        let category = Category::new(
            CategoryId::from_bytes([3; 16]),
            CategoryName::try_new("Work").expect("fixture name is valid"),
        );
        assert_eq!(category.name().as_str(), "Work");
    }

    #[test]
    fn immutable_catalog_snapshot_filters_without_mutating_selection_or_assignments() {
        let work = category(2, "Work");
        let snapshot = CatalogSnapshot::from_domain(
            crate::DataRevision::new(8),
            [
                application(1, "Browser", Some(work.id())),
                application(3, "Editor", None),
            ],
            [work],
        );

        let work_matches = snapshot.filtered_applications(&CatalogFilter::new(
            "BROW",
            CatalogAssignmentFilter::Category(category_id(2)),
        ));
        assert_eq!(work_matches.len(), 1);
        assert_eq!(work_matches[0].display_name(), "Browser");
        assert_eq!(
            snapshot.filtered_applications(&CatalogFilter::new(
                "",
                CatalogAssignmentFilter::Uncategorized,
            ))[0]
                .display_name(),
            "Editor"
        );
        assert_eq!(snapshot.revision(), crate::DataRevision::new(8));
        assert_eq!(snapshot.applications().len(), 2);
        assert_eq!(
            snapshot.applications()[0].category_id(),
            Some(category_id(2))
        );
    }

    #[test]
    fn catalog_service_correlates_persistence_outcomes_without_local_optimism() {
        let command = crate::CommandEnvelope::new(
            crate::SchemaRevision::new(1),
            crate::CommandId::new(4),
            crate::OrderingKey::new(4),
            None,
            UtcMicros::new(10),
            CatalogCommand::DeleteCategory {
                category_id: category_id(2),
            },
        );
        let mut accepted = CatalogService::new(FakeCatalogPersistence::accepted(9));
        assert_eq!(
            accepted.handle(&command),
            crate::MutationOutcome::Confirmed(crate::MutationConfirmation::new(
                crate::CommandId::new(4),
                crate::DataRevision::new(9),
            ))
        );

        let mut stale = CatalogService::new(FakeCatalogPersistence::not_found());
        assert_eq!(
            stale.handle(&command),
            crate::MutationOutcome::Rejected(crate::MutationRejection::new(
                crate::CommandId::new(4),
                crate::MutationRejectionReason::RevisionConflict,
            ))
        );
    }

    struct FakeCatalogPersistence {
        result: Result<crate::DataRevision, CatalogPersistenceError>,
    }

    impl FakeCatalogPersistence {
        fn accepted(revision: u64) -> Self {
            Self {
                result: Ok(crate::DataRevision::new(revision)),
            }
        }

        fn not_found() -> Self {
            Self {
                result: Err(CatalogPersistenceError::NotFound),
            }
        }
    }

    impl CatalogPersistence for FakeCatalogPersistence {
        fn create_category(
            &mut self,
            _: &Category,
            _: UtcMicros,
        ) -> Result<crate::DataRevision, CatalogPersistenceError> {
            self.result
        }

        fn rename_category(
            &mut self,
            _: CategoryId,
            _: &CategoryName,
            _: UtcMicros,
        ) -> Result<crate::DataRevision, CatalogPersistenceError> {
            self.result
        }

        fn delete_category(
            &mut self,
            _: CategoryId,
        ) -> Result<crate::DataRevision, CatalogPersistenceError> {
            self.result
        }

        fn assign_applications(
            &mut self,
            _: &[ApplicationId],
            _: Option<CategoryId>,
        ) -> Result<crate::DataRevision, CatalogPersistenceError> {
            self.result
        }

        fn set_applications_excluded(
            &mut self,
            _: &[ApplicationId],
            _: bool,
        ) -> Result<crate::DataRevision, CatalogPersistenceError> {
            self.result
        }
    }

    fn application(byte: u8, name: &str, category_id: Option<CategoryId>) -> Application {
        Application::try_new(
            ApplicationId::from_bytes([byte; 16]),
            ApplicationName::try_new(name).expect("fixture application name is valid"),
            category_id,
            UtcMicros::new(0),
            UtcMicros::new(0),
        )
        .expect("fixture application observation bounds are valid")
    }

    fn category(byte: u8, name: &str) -> Category {
        Category::new(
            category_id(byte),
            CategoryName::try_new(name).expect("fixture category name is valid"),
        )
    }

    fn category_id(byte: u8) -> CategoryId {
        CategoryId::from_bytes([byte; 16])
    }
}
