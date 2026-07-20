//! Overview presentation from immutable application snapshots and saved-view actions.
//!
//! Composition owns data delivery and command dispatch; this module keeps rendering and local
//! interaction state adapter-free.

use std::sync::Arc;

use openmanic_application::{
    EntityRevision, OverviewAllocation, OverviewAllocationIdentity, OverviewSnapshot,
    SavedViewCommand, SavedViewId, SavedViewSnapshot, SharedOverviewSelection,
};
use openmanic_domain::{HalfOpenInterval, SavedViewDefinition, UtcMicros};

use crate::{EmptyReason, PresentableData, UserFacingError};

/// One local Overview interaction.  All persistence and snapshot delivery remains outside this
/// module; values carried by an action are already validated application/domain contracts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OverviewAction {
    /// Shares a compatible exact interval with Timeline and requests a correlated projection.
    SetSharedSelection(Option<SharedOverviewSelection>),
    /// Restores a complete previously loaded saved view.
    LoadSavedView { id: SavedViewId },
    /// Creates a new saved view with composition-supplied identity and observation time.
    CreateSavedView {
        id: SavedViewId,
        definition: SavedViewDefinition,
        observed_at_utc: UtcMicros,
    },
    /// Renames one loaded saved view.
    RenameSavedView {
        id: SavedViewId,
        name: String,
        observed_at_utc: UtcMicros,
    },
    /// Duplicates one loaded saved view.
    DuplicateSavedView {
        source_id: SavedViewId,
        duplicate_id: SavedViewId,
        name: String,
        observed_at_utc: UtcMicros,
    },
    /// Replaces the complete saved-view order.
    ReorderSavedViews { ordered_ids: Vec<SavedViewId> },
    /// Opens the explicit destructive-action confirmation state.
    RequestDeleteSavedView { id: SavedViewId },
    /// Confirms deletion only for the currently requested view.
    ConfirmDeleteSavedView { id: SavedViewId },
    /// Dismisses an outstanding delete confirmation.
    CancelDeleteSavedView,
}

/// A shell-routed effect after an Overview action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OverviewEffect {
    /// Composition must acquire a projection for this shared selection.
    RequestSharedSelection(Option<SharedOverviewSelection>),
    /// Composition restores the complete saved-view contract, including its range and widgets.
    RestoreSavedView(SavedViewSnapshot),
    /// Composition dispatches this command through the sole saved-view service.
    SavedViewCommand {
        command: SavedViewCommand,
        expected_revision: Option<EntityRevision>,
    },
}

/// A render-ready allocation retaining exact immutable aggregation values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverviewPresentedAllocation {
    allocation: OverviewAllocation,
    selected: bool,
}

impl OverviewPresentedAllocation {
    /// Returns the application-owned stable allocation identity.
    #[must_use]
    pub const fn identity(self) -> OverviewAllocationIdentity {
        self.allocation.identity()
    }

    /// Returns the exact accumulated duration in microseconds.
    #[must_use]
    pub const fn duration_us(self) -> u64 {
        self.allocation.duration_us()
    }

    /// Returns the deterministic truncated share out of 10,000 basis points.
    #[must_use]
    pub const fn percentage_basis_points(self) -> u16 {
        self.allocation.percentage_basis_points()
    }

    /// Returns whether this allocation is locally highlighted.
    #[must_use]
    pub const fn selected(self) -> bool {
        self.selected
    }
}

/// One saved-view entry suitable for a conventional list or keyboard chooser.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverviewSavedViewItem {
    snapshot: SavedViewSnapshot,
    selected: bool,
}

impl OverviewSavedViewItem {
    /// Returns the immutable restoration snapshot.
    #[must_use]
    pub const fn snapshot(&self) -> &SavedViewSnapshot {
        &self.snapshot
    }

    /// Returns the stable row identity.
    #[must_use]
    pub const fn id(&self) -> SavedViewId {
        self.snapshot.id()
    }

    /// Returns the user-visible saved-view name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.snapshot.document().name()
    }

    /// Returns whether this is the restored selected saved view.
    #[must_use]
    pub const fn selected(&self) -> bool {
        self.selected
    }
}

/// Explicit presentation state without any port access from the renderer.
#[derive(Clone, Debug)]
pub enum OverviewDataState {
    /// No immutable correlated snapshot is available yet.
    Loading,
    /// The request completed but no allocation matches the range and filters.
    Empty(EmptyReason),
    /// Usable data, optionally with a non-blocking notice.
    Ready { notice: Option<String> },
    /// No usable snapshot is available; retain recovery text from composition.
    Error(UserFacingError),
}

/// Complete render input for an Overview frame.
#[derive(Clone, Debug)]
pub struct OverviewViewModel {
    state: OverviewDataState,
    exact_range: Option<HalfOpenInterval>,
    total_duration_us: Option<u64>,
    allocations: Vec<OverviewPresentedAllocation>,
    saved_views: Vec<OverviewSavedViewItem>,
    delete_confirmation: Option<SavedViewId>,
}

impl OverviewViewModel {
    /// Returns loading, empty, ready, or error state.
    #[must_use]
    pub const fn state(&self) -> &OverviewDataState {
        &self.state
    }

    /// Returns the exact aggregation interval supplied by the current snapshot.
    #[must_use]
    pub const fn exact_range(&self) -> Option<HalfOpenInterval> {
        self.exact_range
    }

    /// Returns the exact filtered total in microseconds.
    #[must_use]
    pub const fn total_duration_us(&self) -> Option<u64> {
        self.total_duration_us
    }

    /// Returns deterministic duration-descending allocation rows.
    #[must_use]
    pub fn allocations(&self) -> &[OverviewPresentedAllocation] {
        &self.allocations
    }

    /// Returns valid saved views in their application-defined display order.
    #[must_use]
    pub fn saved_views(&self) -> &[OverviewSavedViewItem] {
        &self.saved_views
    }

    /// Returns the view that needs an explicit delete confirmation, if any.
    #[must_use]
    pub const fn delete_confirmation(&self) -> Option<SavedViewId> {
        self.delete_confirmation
    }
}

/// Local Overview interaction state. This reducer never queries storage, a clock, or a port.
#[derive(Clone, Debug, Default)]
pub struct OverviewController {
    saved_views: Vec<SavedViewSnapshot>,
    selected_saved_view: Option<SavedViewId>,
    selected_allocation: Option<OverviewAllocationIdentity>,
    delete_confirmation: Option<SavedViewId>,
}

impl OverviewController {
    /// Replaces restoration data from one immutable application load in deterministic order.
    pub fn set_saved_views(&mut self, mut snapshots: Vec<SavedViewSnapshot>) {
        snapshots.sort_by_key(|snapshot| (snapshot.document().display_order(), snapshot.id()));
        if self
            .selected_saved_view
            .is_some_and(|id| !snapshots.iter().any(|view| view.id() == id))
        {
            self.selected_saved_view = None;
        }
        if self
            .delete_confirmation
            .is_some_and(|id| !snapshots.iter().any(|view| view.id() == id))
        {
            self.delete_confirmation = None;
        }
        self.saved_views = snapshots;
    }

    /// Reduces an action and returns a typed shell effect only when it is valid locally.
    #[must_use]
    pub fn apply(&mut self, action: OverviewAction) -> Option<OverviewEffect> {
        match action {
            OverviewAction::SetSharedSelection(selection) => {
                self.selected_allocation = None;
                Some(OverviewEffect::RequestSharedSelection(selection))
            }
            OverviewAction::LoadSavedView { id } => {
                let snapshot = self
                    .saved_views
                    .iter()
                    .find(|view| view.id() == id)?
                    .clone();
                self.selected_saved_view = Some(id);
                self.delete_confirmation = None;
                Some(OverviewEffect::RestoreSavedView(snapshot))
            }
            OverviewAction::CreateSavedView {
                id,
                definition,
                observed_at_utc,
            } => Some(saved_view_effect(
                SavedViewCommand::Create {
                    id,
                    definition,
                    observed_at_utc,
                },
                None,
            )),
            OverviewAction::RenameSavedView {
                id,
                name,
                observed_at_utc,
            } => {
                let revision = self.saved_view_revision(id)?;
                Some(saved_view_effect(
                    SavedViewCommand::Rename {
                        id,
                        name,
                        observed_at_utc,
                    },
                    Some(revision),
                ))
            }
            OverviewAction::DuplicateSavedView {
                source_id,
                duplicate_id,
                name,
                observed_at_utc,
            } => {
                let revision = self.saved_view_revision(source_id)?;
                Some(saved_view_effect(
                    SavedViewCommand::Duplicate {
                        source_id,
                        duplicate_id,
                        name,
                        observed_at_utc,
                    },
                    Some(revision),
                ))
            }
            OverviewAction::ReorderSavedViews { ordered_ids } => {
                valid_reorder(&self.saved_views, &ordered_ids)
                    .then(|| saved_view_effect(SavedViewCommand::Reorder { ordered_ids }, None))
            }
            OverviewAction::RequestDeleteSavedView { id } => {
                if self.saved_view_revision(id).is_some() {
                    self.delete_confirmation = Some(id);
                }
                None
            }
            OverviewAction::ConfirmDeleteSavedView { id } => (self.delete_confirmation == Some(id))
                .then(|| self.saved_view_revision(id))
                .flatten()
                .map(|revision| {
                    self.delete_confirmation = None;
                    saved_view_effect(SavedViewCommand::DeleteConfirmed { id }, Some(revision))
                }),
            OverviewAction::CancelDeleteSavedView => {
                self.delete_confirmation = None;
                None
            }
        }
    }

    /// Locally highlights one allocation without mutating source data or shared selection.
    pub fn select_allocation(&mut self, identity: Option<OverviewAllocationIdentity>) {
        self.selected_allocation = identity;
    }

    /// Derives an entire frame from immutable snapshot data and retained local interactions.
    #[must_use]
    pub fn view_model(&self, data: &PresentableData<OverviewSnapshot>) -> OverviewViewModel {
        let (state, snapshot) = overview_state(data);
        let (exact_range, total_duration_us, allocations) = snapshot.map_or_else(
            || (None, None, Vec::new()),
            |snapshot| {
                (
                    Some(snapshot.context().effective_range()),
                    Some(snapshot.total_duration_us()),
                    snapshot
                        .allocations()
                        .iter()
                        .copied()
                        .map(|allocation| OverviewPresentedAllocation {
                            selected: Some(allocation.identity()) == self.selected_allocation,
                            allocation,
                        })
                        .collect(),
                )
            },
        );
        let saved_views = self
            .saved_views
            .iter()
            .cloned()
            .map(|snapshot| OverviewSavedViewItem {
                selected: Some(snapshot.id()) == self.selected_saved_view,
                snapshot,
            })
            .collect();
        OverviewViewModel {
            state,
            exact_range,
            total_duration_us,
            allocations,
            saved_views,
            delete_confirmation: self.delete_confirmation,
        }
    }

    fn saved_view_revision(&self, id: SavedViewId) -> Option<EntityRevision> {
        self.saved_views
            .iter()
            .find(|view| view.id() == id)
            .map(SavedViewSnapshot::entity_revision)
    }
}

fn saved_view_effect(
    command: SavedViewCommand,
    expected_revision: Option<EntityRevision>,
) -> OverviewEffect {
    OverviewEffect::SavedViewCommand {
        command,
        expected_revision,
    }
}

fn valid_reorder(saved_views: &[SavedViewSnapshot], ordered_ids: &[SavedViewId]) -> bool {
    saved_views.len() == ordered_ids.len()
        && saved_views
            .iter()
            .map(SavedViewSnapshot::id)
            .all(|id| ordered_ids.contains(&id))
        && ordered_ids
            .iter()
            .enumerate()
            .all(|(index, id)| !ordered_ids[..index].contains(id))
}

fn overview_state(
    data: &PresentableData<OverviewSnapshot>,
) -> (OverviewDataState, Option<&OverviewSnapshot>) {
    match data {
        PresentableData::InitialLoading => (OverviewDataState::Loading, None),
        PresentableData::Empty(reason) => (OverviewDataState::Empty(*reason), None),
        PresentableData::Failed { prior: None, error } => {
            (OverviewDataState::Error(error.clone()), None)
        }
        PresentableData::Ready(snapshot) => ready_state(snapshot, None),
        PresentableData::Partial { value, .. } => {
            ready_state(value, Some("Overview data is partial."))
        }
        PresentableData::Refreshing { prior, .. } => {
            ready_state(prior, Some("Refreshing overview data."))
        }
        PresentableData::Recovered { value, notice } => ready_state(value, Some(notice)),
        PresentableData::Failed {
            prior: Some(value), ..
        } => ready_state(
            value,
            Some("Overview data could not be refreshed. Showing the last available data."),
        ),
    }
}

fn ready_state<'a>(
    snapshot: &'a Arc<OverviewSnapshot>,
    notice: Option<&str>,
) -> (OverviewDataState, Option<&'a OverviewSnapshot>) {
    let state = if snapshot.allocations().is_empty() {
        OverviewDataState::Empty(EmptyReason::NoMatchingResults)
    } else {
        OverviewDataState::Ready {
            notice: notice.map(str::to_owned),
        }
    };
    (state, Some(Arc::as_ref(snapshot)))
}

#[cfg(test)]
mod tests {
    use openmanic_application::SavedViewId;

    use super::{OverviewAction, OverviewController, OverviewDataState, OverviewEffect};
    use crate::PresentableData;

    #[test]
    fn saved_view_effects_require_loaded_revisions_and_explicit_delete_confirmation() {
        let mut controller = OverviewController::default();
        let id = SavedViewId::from_bytes([7; 16]);
        assert_eq!(
            controller.apply(OverviewAction::RequestDeleteSavedView { id }),
            None
        );
        assert_eq!(
            controller.apply(OverviewAction::ConfirmDeleteSavedView { id }),
            None
        );
        assert_eq!(
            controller.apply(OverviewAction::SetSharedSelection(None)),
            Some(OverviewEffect::RequestSharedSelection(None))
        );
    }

    #[test]
    fn loading_partial_and_empty_never_query_a_port() {
        let controller = OverviewController::default();
        assert!(matches!(
            controller
                .view_model(&PresentableData::InitialLoading)
                .state(),
            OverviewDataState::Loading
        ));
        assert!(matches!(
            controller
                .view_model(&PresentableData::Empty(
                    crate::EmptyReason::NoMatchingResults
                ))
                .state(),
            OverviewDataState::Empty(_)
        ));
    }
}
