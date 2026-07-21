//! Explicit Today-layout editing state isolated from normal dashboard interaction.

use openmanic_domain::{
    LayoutDefinition, LayoutDocument, LayoutFields, LayoutHeight, LayoutWidgetDefinition,
};

use crate::{TodayWidgetDefinition, TodayWidgetKind, TodayWidgetRegistry};

/// One explicit edit-mode operation. Normal widget gestures never produce these actions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayoutEditAction {
    /// Enters edit mode from the currently applied layout.
    Begin,
    /// Adds a registry-supported widget with a caller-assigned stable instance ID.
    Add {
        /// Caller-generated stable identity for the new instance.
        instance_id: String,
        /// Registered first-party kind to add.
        kind: TodayWidgetKind,
    },
    /// Removes an optional widget instance.
    Remove {
        /// Stable identity of the optional instance to remove.
        instance_id: String,
    },
    /// Moves one instance before the preceding persisted-order entry.
    MoveEarlier {
        /// Stable identity of the instance to move.
        instance_id: String,
    },
    /// Moves one instance after the following persisted-order entry.
    MoveLater {
        /// Stable identity of the instance to move.
        instance_id: String,
    },
    /// Sets one supported canonical width span.
    Resize {
        /// Stable identity of the instance to resize.
        instance_id: String,
        /// New supported canonical 12-column span.
        width_span: u8,
    },
    /// Sets one supported semantic height class.
    SetHeight {
        /// Stable identity of the instance to resize vertically.
        instance_id: String,
        /// New supported semantic height class.
        height: LayoutHeight,
    },
    /// Replaces the draft with the versioned built-in default.
    Reset,
    /// Restores the exact layout active when edit mode began.
    Cancel,
    /// Requests persistence of the complete validated draft.
    Save,
}

/// A typed output for a layout-edit action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayoutEditEffect {
    /// A complete validated layout requires authoritative persistence.
    Save(LayoutDocument),
    /// Edit mode ended without a persistence request.
    Cancelled,
}

/// UI-local layout draft and exact cancel baseline.
#[derive(Clone, Debug)]
pub struct LayoutEditor {
    active: LayoutDocument,
    original: Option<LayoutDocument>,
    draft: Option<LayoutDefinition>,
}

impl LayoutEditor {
    /// Creates an editor around the currently applied complete layout.
    #[must_use]
    pub const fn new(active: LayoutDocument) -> Self {
        Self {
            active,
            original: None,
            draft: None,
        }
    }

    /// Returns whether explicit edit mode is active.
    #[must_use]
    pub const fn is_editing(&self) -> bool {
        self.draft.is_some()
    }

    /// Returns the complete applied layout.
    #[must_use]
    pub const fn active(&self) -> &LayoutDocument {
        &self.active
    }

    /// Returns the current mutable draft only while edit mode is active.
    #[must_use]
    pub fn draft(&self) -> Option<&LayoutDefinition> {
        self.draft.as_ref()
    }

    /// Applies one edit-only action and emits a persistence effect only for Save.
    #[must_use]
    pub fn apply(
        &mut self,
        action: LayoutEditAction,
        registry: &TodayWidgetRegistry,
    ) -> Option<LayoutEditEffect> {
        match action {
            LayoutEditAction::Begin => {
                if self.draft.is_none() {
                    self.original = Some(self.active.clone());
                    self.draft = Some(self.active.definition());
                }
                None
            }
            LayoutEditAction::Cancel => {
                let original = self.original.take()?;
                self.active = original;
                self.draft = None;
                Some(LayoutEditEffect::Cancelled)
            }
            LayoutEditAction::Reset => {
                let draft = self.draft.as_mut()?;
                *draft = LayoutDocument::safe_default().definition();
                None
            }
            LayoutEditAction::Save => {
                let draft = self.draft.as_ref()?;
                let document =
                    LayoutDocument::try_new(draft.clone(), self.active.revision()).ok()?;
                Some(LayoutEditEffect::Save(document))
            }
            LayoutEditAction::Add { instance_id, kind } => {
                let draft = self.draft.as_mut()?;
                add_widget(draft, registry, instance_id, kind);
                None
            }
            LayoutEditAction::Remove { instance_id } => {
                let draft = self.draft.as_mut()?;
                remove_widget(draft, &instance_id);
                None
            }
            LayoutEditAction::MoveEarlier { instance_id } => {
                let draft = self.draft.as_mut()?;
                reorder_widget(draft, &instance_id, true);
                None
            }
            LayoutEditAction::MoveLater { instance_id } => {
                let draft = self.draft.as_mut()?;
                reorder_widget(draft, &instance_id, false);
                None
            }
            LayoutEditAction::Resize {
                instance_id,
                width_span,
            } => {
                let draft = self.draft.as_mut()?;
                if [3, 4, 6, 8, 9, 12].contains(&width_span)
                    && let Some(widget) = draft
                        .widgets
                        .iter_mut()
                        .find(|widget| widget.instance_id == instance_id)
                {
                    widget.width_span = width_span;
                }
                None
            }
            LayoutEditAction::SetHeight {
                instance_id,
                height,
            } => {
                let draft = self.draft.as_mut()?;
                if let Some(widget) = draft
                    .widgets
                    .iter_mut()
                    .find(|widget| widget.instance_id == instance_id)
                {
                    widget.height = height;
                }
                None
            }
        }
    }

    /// Reconciles a confirmed authoritative replacement and leaves edit mode.
    pub fn confirm_saved(&mut self, document: LayoutDocument) {
        self.active = document;
        self.original = None;
        self.draft = None;
    }
}

fn add_widget(
    draft: &mut LayoutDefinition,
    registry: &TodayWidgetRegistry,
    instance_id: String,
    kind: TodayWidgetKind,
) {
    if draft
        .widgets
        .iter()
        .any(|widget| widget.instance_id == instance_id)
    {
        return;
    }
    let Some(definition) = registry
        .definitions()
        .iter()
        .copied()
        .find(|definition| definition.kind() == kind)
    else {
        return;
    };
    if !definition.supports_multiple_instances()
        && draft
            .widgets
            .iter()
            .any(|widget| widget.kind_id == kind.id())
    {
        return;
    }
    let Ok(order) = u32::try_from(draft.widgets.len()) else {
        return;
    };
    draft
        .widgets
        .push(new_widget(instance_id, definition, order));
}

fn new_widget(
    instance_id: String,
    definition: TodayWidgetDefinition,
    order: u32,
) -> LayoutWidgetDefinition {
    LayoutWidgetDefinition {
        instance_id,
        kind_id: definition.kind().id().to_owned(),
        kind_schema_version: definition.schema_version(),
        order,
        width_span: definition.size_policy().preferred_span_12(),
        height: LayoutHeight::Standard,
        configuration: LayoutFields::empty(),
        appearance_overrides: None,
    }
}

fn remove_widget(draft: &mut LayoutDefinition, instance_id: &str) {
    if draft.widgets.len() <= 1 {
        return;
    }
    let Some(index) = draft
        .widgets
        .iter()
        .position(|widget| widget.instance_id == instance_id)
    else {
        return;
    };
    if draft.widgets[index].kind_id == TodayWidgetKind::TIMELINE.id() {
        return;
    }
    draft.widgets.remove(index);
    normalize_order(draft);
}

fn reorder_widget(draft: &mut LayoutDefinition, instance_id: &str, earlier: bool) {
    draft
        .widgets
        .sort_by_key(|widget| (widget.order, widget.instance_id.clone()));
    let Some(index) = draft
        .widgets
        .iter()
        .position(|widget| widget.instance_id == instance_id)
    else {
        return;
    };
    let target = if earlier {
        index.checked_sub(1)
    } else {
        index
            .checked_add(1)
            .filter(|target| *target < draft.widgets.len())
    };
    if let Some(target) = target {
        draft.widgets.swap(index, target);
        normalize_order(draft);
    }
}

fn normalize_order(draft: &mut LayoutDefinition) {
    for (order, widget) in draft.widgets.iter_mut().enumerate() {
        let Ok(order) = u32::try_from(order) else {
            return;
        };
        widget.order = order;
    }
}

#[cfg(test)]
mod tests {
    use openmanic_domain::{LayoutDocument, LayoutHeight};

    use super::{LayoutEditAction, LayoutEditEffect, LayoutEditor};
    use crate::{TodayWidgetKind, TodayWidgetRegistry};

    #[test]
    fn cancel_restores_the_exact_entry_document_after_multiple_edits() {
        let active = LayoutDocument::safe_default();
        let mut editor = LayoutEditor::new(active.clone());
        let registry = TodayWidgetRegistry::default();
        let _ = editor.apply(LayoutEditAction::Begin, &registry);
        let _ = editor.apply(
            LayoutEditAction::MoveLater {
                instance_id: "today.timeline".to_owned(),
            },
            &registry,
        );
        let _ = editor.apply(
            LayoutEditAction::Resize {
                instance_id: "today.usage".to_owned(),
                width_span: 9,
            },
            &registry,
        );
        assert_eq!(
            editor.apply(LayoutEditAction::Cancel, &registry),
            Some(LayoutEditEffect::Cancelled)
        );
        assert_eq!(editor.active(), &active);
        assert!(!editor.is_editing());
    }

    #[test]
    fn add_reset_and_save_preserve_required_widget_identity_rules() {
        let mut editor = LayoutEditor::new(LayoutDocument::safe_default());
        let registry = TodayWidgetRegistry::default();
        let _ = editor.apply(LayoutEditAction::Begin, &registry);
        let _ = editor.apply(
            LayoutEditAction::Add {
                instance_id: "today.usage.second".to_owned(),
                kind: TodayWidgetKind::APPLICATION_USAGE,
            },
            &registry,
        );
        let _ = editor.apply(
            LayoutEditAction::Remove {
                instance_id: "today.timeline".to_owned(),
            },
            &registry,
        );
        let draft = editor.draft().expect("edit mode retains a draft");
        assert_eq!(draft.widgets.len(), 7);
        assert!(
            draft
                .widgets
                .iter()
                .any(|widget| widget.instance_id == "today.timeline")
        );

        let _ = editor.apply(LayoutEditAction::Reset, &registry);
        let _ = editor.apply(
            LayoutEditAction::SetHeight {
                instance_id: "today.usage".to_owned(),
                height: LayoutHeight::Tall,
            },
            &registry,
        );
        let effect = editor
            .apply(LayoutEditAction::Save, &registry)
            .expect("valid draft can request a complete save");
        let LayoutEditEffect::Save(document) = effect else {
            return;
        };
        let definition = document.definition();
        assert_eq!(
            definition
                .widgets
                .iter()
                .find(|widget| widget.instance_id == "today.usage")
                .map(|widget| widget.height),
            Some(LayoutHeight::Tall)
        );
    }
}
