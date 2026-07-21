//! Initial egui shell rendering using only ordinary egui controls.

use eframe::egui::{self, Align, RichText};

use crate::{
    DataLimitation, EmptyReason, MutationStatus, PresentableData, Route, ThemeTokens, UiAction,
    UiModel,
};

/// Renders the application shell and returns whether ordinary input changed state.
pub(crate) fn render<T>(ui: &mut egui::Ui, model: &mut UiModel<T>, tokens: ThemeTokens) -> bool {
    let mut changed = false;
    egui::Frame::new()
        .fill(tokens.panel())
        .inner_margin(egui::Margin::symmetric(16, 10))
        .stroke(egui::Stroke::new(1.0, tokens.timeline_grid()))
        .show(ui, |ui| changed |= render_navigation(ui, model, tokens));

    egui::Frame::new()
        .fill(tokens.canvas())
        .inner_margin(egui::Margin::symmetric(16, 14))
        .show(ui, |ui| {
            changed |= render_presentation_state(ui, model.data(), tokens);
            changed |= render_mutation_state(ui, model, tokens);
            changed |= render_route(ui, model, tokens);
        });
    changed
}

fn render_navigation<T>(ui: &mut egui::Ui, model: &mut UiModel<T>, tokens: ThemeTokens) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("OM")
                .size(14.0)
                .strong()
                .color(tokens.interaction_primary()),
        );
        ui.strong(
            RichText::new("OpenManic")
                .size(17.0)
                .color(tokens.content_primary()),
        );
        ui.separator();
        for route in Route::all() {
            changed |= select_route(ui, model, route);
        }
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            ui.colored_label(tokens.success(), "Monitoring active");
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

fn render_presentation_state<T>(
    ui: &mut egui::Ui,
    data: &PresentableData<T>,
    tokens: ThemeTokens,
) -> bool {
    let (message, color) = match data {
        PresentableData::InitialLoading => ("Loading data...".to_owned(), tokens.warning()),
        PresentableData::Ready(_) => return false,
        PresentableData::Refreshing { .. } => (
            "Refreshing. Current data remains visible.".to_owned(),
            tokens.interaction_primary(),
        ),
        PresentableData::Empty(reason) => (
            empty_message(*reason).to_owned(),
            tokens.content_secondary(),
        ),
        PresentableData::Partial { limitations, .. } => {
            (partial_message(limitations), tokens.warning())
        }
        PresentableData::Failed { error, .. } => (error.message(), tokens.error()),
        PresentableData::Recovered { notice, .. } => (notice.clone(), tokens.success()),
    };
    egui::Frame::new().fill(tokens.panel()).show(ui, |ui| {
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

fn render_mutation_state<T>(ui: &mut egui::Ui, model: &UiModel<T>, tokens: ThemeTokens) -> bool {
    let Some((command_id, status)) = latest_mutation(model) else {
        return false;
    };
    let (message, color) = match status {
        MutationStatus::Pending => (
            "A change is waiting for confirmation.".to_owned(),
            tokens.warning(),
        ),
        MutationStatus::Confirmed { .. } => {
            ("The last change was saved.".to_owned(), tokens.success())
        }
        MutationStatus::Rejected { reason } => (
            format!("The last change was not saved: {reason}."),
            tokens.error(),
        ),
    };
    egui::Frame::new().fill(tokens.panel()).show(ui, |ui| {
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

fn render_route<T>(ui: &mut egui::Ui, model: &mut UiModel<T>, tokens: ThemeTokens) -> bool {
    let route = model.route();
    let mut changed = false;
    egui::Frame::new()
        .fill(tokens.panel())
        .stroke(egui::Stroke::new(1.0, tokens.timeline_grid()))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.heading(
                        RichText::new(route.label())
                            .size(18.0)
                            .color(tokens.content_primary()),
                    );
                    ui.colored_label(tokens.content_secondary(), route_description(route));
                });
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    changed |= render_date_controls(ui, model, route, tokens);
                });
            });
        });
    ui.add_space(12.0);

    if route != Route::Today {
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
    }

    changed
}

fn render_date_controls<T>(
    ui: &mut egui::Ui,
    model: &mut UiModel<T>,
    route: Route,
    tokens: ThemeTokens,
) -> bool {
    let mut changed = false;
    if ui.button("Next  ›").clicked() {
        crate::reducer::reduce(model, UiAction::MoveRouteDate { route, days: 1 });
        changed = true;
    }
    if ui
        .add(
            egui::Button::new(RichText::new("Today").strong())
                .fill(tokens.interaction_primary().gamma_multiply(0.72)),
        )
        .clicked()
    {
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
    if ui.button("‹  Previous").clicked() {
        crate::reducer::reduce(model, UiAction::MoveRouteDate { route, days: -1 });
        changed = true;
    }
    ui.colored_label(
        tokens.content_secondary(),
        day_context_label(model.route_state(route).date_offset_days()),
    );
    changed
}

fn day_context_label(offset: i32) -> String {
    match offset {
        0 => "Today".to_owned(),
        -1 => "Yesterday".to_owned(),
        value if value < 0 => format!("{} days ago", value.unsigned_abs()),
        1 => "Tomorrow".to_owned(),
        value => format!("In {value} days"),
    }
}

fn route_description(route: Route) -> &'static str {
    match route {
        Route::Today => "Your activity, categories, and focus for the selected day.",
        Route::Overview => "Review time across a selected range.",
        Route::Categories => "Organize applications with personal categories.",
        Route::Calendar => "Review one day of activity, focus, and schedules.",
        Route::Settings => "Manage privacy and appearance choices.",
    }
}
