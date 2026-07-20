//! Pure responsive Today-dashboard grid reflow.
//!
//! Persisted layouts retain canonical 12-column order and spans. This module derives placement
//! for the current logical width without mutating that durable document.

use openmanic_domain::{LayoutDefinition, LayoutHeight};

use crate::{TodayWidgetInstance, TodayWidgetRegistry, TodayWidgetResolution};

/// Responsive grid column count selected from an allocated logical dashboard width.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DashboardColumnCount {
    /// Full desktop grid.
    Twelve,
    /// Medium responsive grid.
    Eight,
    /// Compact or scroll-safe grid.
    Four,
}

impl DashboardColumnCount {
    /// Selects the active grid from an already DPI-normalized logical width.
    #[must_use]
    pub fn for_logical_width(logical_width: f32) -> Self {
        if logical_width >= 1200.0 {
            Self::Twelve
        } else if logical_width >= 900.0 {
            Self::Eight
        } else {
            Self::Four
        }
    }

    /// Returns the integer active column count.
    #[must_use]
    pub const fn count(self) -> u8 {
        match self {
            Self::Twelve => 12,
            Self::Eight => 8,
            Self::Four => 4,
        }
    }

    fn allowed_spans(self) -> &'static [u8] {
        match self {
            Self::Twelve => &[3, 4, 6, 8, 9, 12],
            Self::Eight => &[2, 3, 4, 5, 6, 8],
            Self::Four => &[2, 4],
        }
    }
}

/// One derived in-memory placement for the active responsive grid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DashboardPlacement {
    instance_id: String,
    row: u32,
    column: u8,
    span: u8,
    height: LayoutHeight,
    missing_renderer: bool,
}

impl DashboardPlacement {
    /// Returns the stable persisted widget-instance identity.
    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Returns the zero-based derived grid row.
    #[must_use]
    pub const fn row(&self) -> u32 {
        self.row
    }

    /// Returns the zero-based derived grid column.
    #[must_use]
    pub const fn column(&self) -> u8 {
        self.column
    }

    /// Returns the derived responsive width span.
    #[must_use]
    pub const fn span(&self) -> u8 {
        self.span
    }

    /// Returns the persisted semantic height class.
    #[must_use]
    pub const fn height(&self) -> LayoutHeight {
        self.height
    }

    /// Returns whether this placement needs the recoverable missing-renderer presentation.
    #[must_use]
    pub const fn missing_renderer(&self) -> bool {
        self.missing_renderer
    }
}

/// Complete immutable responsive layout derived from a canonical document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DashboardReflow {
    columns: DashboardColumnCount,
    placements: Vec<DashboardPlacement>,
}

impl DashboardReflow {
    /// Returns the active responsive column count.
    #[must_use]
    pub const fn columns(&self) -> DashboardColumnCount {
        self.columns
    }

    /// Returns placements in canonical `(order, instance_id)` traversal order.
    #[must_use]
    pub fn placements(&self) -> &[DashboardPlacement] {
        &self.placements
    }
}

/// Derives deterministic first-fit responsive placements without altering the saved document.
#[must_use]
pub fn reflow_dashboard(
    layout: &LayoutDefinition,
    registry: &TodayWidgetRegistry,
    logical_width: f32,
) -> DashboardReflow {
    let columns = DashboardColumnCount::for_logical_width(logical_width);
    let mut widgets = layout.widgets.clone();
    widgets.sort_by_key(|widget| (widget.order, widget.instance_id.clone()));
    let mut occupied_rows: Vec<Vec<bool>> = Vec::new();
    let placements = widgets
        .into_iter()
        .map(|widget| {
            let instance = TodayWidgetInstance::from_layout(
                widget.instance_id.clone(),
                widget.kind_id,
                widget.kind_schema_version,
            );
            let (minimum_span, missing_renderer) = match registry.resolve(&instance) {
                TodayWidgetResolution::Available(definition) => (
                    match columns {
                        DashboardColumnCount::Twelve => definition.size_policy().minimum_span_12(),
                        DashboardColumnCount::Eight => definition.size_policy().minimum_span_8(),
                        DashboardColumnCount::Four => definition.size_policy().minimum_span_4(),
                    },
                    false,
                ),
                TodayWidgetResolution::MissingRenderer => (2, true),
            };
            let span = responsive_span(widget.width_span, columns, minimum_span);
            let (row, column) = first_fit(&mut occupied_rows, columns.count(), span);
            DashboardPlacement {
                instance_id: widget.instance_id,
                row,
                column,
                span,
                height: widget.height,
                missing_renderer,
            }
        })
        .collect();
    DashboardReflow {
        columns,
        placements,
    }
}

fn responsive_span(saved_span: u8, columns: DashboardColumnCount, minimum_span: u8) -> u8 {
    let active_columns = columns.count();
    let scaled = u8::try_from(
        u16::from(saved_span)
            .saturating_mul(u16::from(active_columns))
            .div_ceil(12),
    )
    .unwrap_or(active_columns);
    let required = scaled.max(minimum_span).min(active_columns);
    columns
        .allowed_spans()
        .iter()
        .copied()
        .find(|span| *span >= required)
        .unwrap_or(active_columns)
}

fn first_fit(occupied_rows: &mut Vec<Vec<bool>>, columns: u8, span: u8) -> (u32, u8) {
    let columns = usize::from(columns);
    let span = usize::from(span);
    for (row_index, row) in occupied_rows.iter_mut().enumerate() {
        if let Some(column) = first_free_run(row, span) {
            row[column..column + span].fill(true);
            return (
                u32::try_from(row_index).unwrap_or(u32::MAX),
                u8::try_from(column).unwrap_or(u8::MAX),
            );
        }
    }
    let mut row = vec![false; columns];
    row[..span].fill(true);
    let row_index = u32::try_from(occupied_rows.len()).unwrap_or(u32::MAX);
    occupied_rows.push(row);
    (row_index, 0)
}

fn first_free_run(row: &[bool], span: usize) -> Option<usize> {
    row.windows(span)
        .position(|candidate| candidate.iter().all(|occupied| !occupied))
}

#[cfg(test)]
mod tests {
    use openmanic_domain::{LayoutDefinition, LayoutDocument};

    use super::{DashboardColumnCount, reflow_dashboard};
    use crate::TodayWidgetRegistry;

    #[test]
    fn reflow_uses_required_column_breakpoints_without_mutating_the_document() {
        let layout = LayoutDocument::safe_default().definition();
        let registry = TodayWidgetRegistry::default();
        let original = layout.clone();

        let wide = reflow_dashboard(&layout, &registry, 1440.0);
        let medium = reflow_dashboard(&layout, &registry, 1024.0);
        let narrow = reflow_dashboard(&layout, &registry, 720.0);

        assert_eq!(wide.columns(), DashboardColumnCount::Twelve);
        assert_eq!(medium.columns(), DashboardColumnCount::Eight);
        assert_eq!(narrow.columns(), DashboardColumnCount::Four);
        assert_eq!(wide.placements()[0].span(), 12);
        assert_eq!(medium.placements()[0].span(), 8);
        assert_eq!(narrow.placements()[0].span(), 4);
        assert_eq!(layout, original);
    }

    #[test]
    fn reflow_is_deterministic_for_each_required_scale_matrix_case() {
        let layout: LayoutDefinition = LayoutDocument::safe_default().definition();
        let registry = TodayWidgetRegistry::default();
        for logical_width in [720.0, 1024.0, 1440.0] {
            for scale in [1.25, 1.5, 1.75, 2.0] {
                let physical_width = logical_width * scale;
                let first = reflow_dashboard(&layout, &registry, physical_width / scale);
                let second = reflow_dashboard(&layout, &registry, physical_width / scale);
                assert_eq!(first, second);
            }
        }
    }
}
