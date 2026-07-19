//! Exhaustive application of UI-local shell actions.

use crate::{UiAction, UiModel};

/// Applies one action without performing I/O, waiting, or consulting repaint state.
pub(crate) fn reduce<T>(model: &mut UiModel<T>, action: UiAction) {
    model.reduce(action);
}
