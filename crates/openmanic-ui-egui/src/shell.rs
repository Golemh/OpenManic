//! Initial egui shell rendering using only ordinary egui controls.

use eframe::egui::{self, Align, Color32, Context, RichText, Theme};

use crate::{
    DataLimitation, EmptyReason, MutationStatus, PresentableData, Route, UiAction, UiModel,
};

const CANVAS: Color32 = Color32::from_rgb(18, 22, 31);
const PANEL: Color32 = Color32::from_rgb(27, 33, 45);
const CONTENT: Color32 = Color32::from_rgb(238, 242, 250);
const SECONDARY: Color32 = Color32::from_rgb(176, 188, 208);
const ACCENT: Color32 = Color32::from_rgb(121, 151, 255);
const SUCCESS: Color32 = Color32::from_rgb(107, 201, 139);
const WARNING: Color32 = Color32::from_rgb(236, 190, 93);
const ERROR: Color32 = Color32::from_rgb(237, 113, 113);

/// Applies the provisional dark visual direction resolved by the G0 spike.
///
/// This is intentionally a renderer-only bridge, not a persisted theme
/// schema. The versioned theme resolver remains owned by the later theme task.
pub(crate) fn apply_initial_dark_theme(context: &Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = PANEL;
    visuals.panel_fill = CANVAS;
    visuals.faint_bg_color = PANEL;
    visuals.extreme_bg_color = CANVAS;
    visuals.override_text_color = Some(CONTENT);
    visuals.selection.bg_fill = ACCENT;
    visuals.error_fg_color = ERROR;
    context.set_visuals(visuals);

    let mut style = (*context.style_of(Theme::Dark)).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    context.set_style_of(Theme::Dark, style);
}

/// Renders the application shell and returns whether ordinary input changed state.
pub(crate) fn render<T>(ui: &mut egui::Ui, model: &mut UiModel<T>) -> bool {
    let mut changed = false;
    egui::Frame::new()
        .fill(PANEL)
        .show(ui, |ui| changed |= render_navigation(ui, model));

    egui::Frame::new().fill(CANVAS).show(ui, |ui| {
        changed |= render_presentation_state(ui, model.data());
        changed |= render_mutation_state(ui, model);
        changed |= render_route(ui, model);
    });
    changed
}

fn render_navigation<T>(ui: &mut egui::Ui, model: &mut UiModel<T>) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.strong(RichText::new("OpenManic").color(CONTENT));
        ui.separator();
        for route in Route::all() {
            changed |= select_route(ui, model, route);
        }
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            ui.colored_label(SUCCESS, "Tracking status will appear here");
        });
    });
    changed
}

fn select_route<T>(ui: &mut egui::Ui, model: &mut UiModel<T>, route: Route) -> bool {
    if !ui
        .selectable_label(model.route() == route, route.label())
        .clicked()
    {
        return false;
    }
    crate::reducer::reduce(model, UiAction::Navigate(route));
    true
}

fn render_presentation_state<T>(ui: &mut egui::Ui, data: &PresentableData<T>) -> bool {
    let (message, color) = match data {
        PresentableData::InitialLoading => ("Loading data…".to_owned(), WARNING),
        PresentableData::Ready(_) => return false,
        PresentableData::Refreshing { .. } => (
            "Refreshing. Current data remains visible.".to_owned(),
            ACCENT,
        ),
        PresentableData::Empty(reason) => (empty_message(*reason).to_owned(), SECONDARY),
        PresentableData::Partial { limitations, .. } => (partial_message(limitations), WARNING),
        PresentableData::Failed { error, .. } => (error.message(), ERROR),
        PresentableData::Recovered { notice, .. } => (notice.clone(), SUCCESS),
    };
    egui::Frame::new().fill(PANEL).show(ui, |ui| {
        ui.colored_label(color, message);
        if matches!(data, PresentableData::Failed { .. }) {
            ui.small("Technical details can be added by the route controller.");
        }
    });
    ui.add_space(8.0);
    false
}

fn empty_message(reason: EmptyReason) -> &'static str {
    reason.message()
}

fn partial_message(limitations: &[DataLimitation]) -> String {
    match limitations.first() {
        Some(limitation) => format!("Partial data. {}", limitation.message()),
        None => "Partial data is available.".to_owned(),
    }
}

fn render_mutation_state<T>(ui: &mut egui::Ui, model: &UiModel<T>) -> bool {
    let Some((command_id, status)) = latest_mutation(model) else {
        return false;
    };
    let (message, color) = match status {
        MutationStatus::Pending => ("A change is waiting for confirmation.".to_owned(), WARNING),
        MutationStatus::Confirmed { .. } => ("The last change was saved.".to_owned(), SUCCESS),
        MutationStatus::Rejected { reason } => {
            (format!("The last change was not saved: {reason}."), ERROR)
        }
    };
    egui::Frame::new().fill(PANEL).show(ui, |ui| {
        ui.colored_label(color, message);
        ui.small(format!("Command {}", command_id.get()));
    });
    ui.add_space(8.0);
    false
}

fn latest_mutation<T>(
    model: &UiModel<T>,
) -> Option<(openmanic_application::CommandId, &MutationStatus)> {
    model.latest_mutation()
}

fn render_route<T>(ui: &mut egui::Ui, model: &mut UiModel<T>) -> bool {
    let route = model.route();
    let mut changed = false;
    ui.heading(RichText::new(route.label()).color(CONTENT));
    ui.colored_label(SECONDARY, route_description(route));
    ui.add_space(12.0);

    ui.horizontal(|ui| {
        if ui.button("Previous").clicked() {
            crate::reducer::reduce(model, UiAction::MoveRouteDate { route, days: -1 });
            changed = true;
        }
        if ui.button("Today").clicked() {
            let current_offset = model.route_state(route).date_offset_days();
            if current_offset != 0 {
                crate::reducer::reduce(
                    model,
                    UiAction::MoveRouteDate {
                        route,
                        days: current_offset.saturating_neg(),
                    },
                );
                changed = true;
            }
        }
        if ui.button("Next").clicked() {
            crate::reducer::reduce(model, UiAction::MoveRouteDate { route, days: 1 });
            changed = true;
        }
        ui.label(format!(
            "Day offset: {}",
            model.route_state(route).date_offset_days()
        ));
    });

    let mut filter = model.route_state(route).filter_text().to_owned();
    let filter_response = ui.add(
        egui::TextEdit::singleline(&mut filter)
            .hint_text("Filter this view")
            .desired_width(240.0),
    );
    if filter_response.changed() {
        crate::reducer::reduce(model, UiAction::SetRouteFilter { route, filter });
        changed = true;
    }

    egui::ScrollArea::vertical()
        .id_salt(route.label())
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(24.0);
            ui.colored_label(
                SECONDARY,
                "This initial shell retains navigation context while later tasks add route content.",
            );
            ui.add_space(320.0);
        });
    changed
}

fn route_description(route: Route) -> &'static str {
    match route {
        Route::Today => "A daily dashboard with Timeline as its central flow.",
        Route::Overview => "Review time across a selected range.",
        Route::Categories => "Organize applications with personal categories.",
        Route::Calendar => "Review one day of activity, focus, and schedules.",
        Route::Settings => "Manage privacy and appearance choices.",
    }
}
