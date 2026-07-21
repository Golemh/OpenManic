//! Application shell chrome styled after the OpenManic Studio design.
//!
//! Renders the fixed title bar (logo chip, wordmark, navigation pills, live
//! monitoring indicator), plus shared presentation and mutation notices.

use eframe::egui::{self, Align, Color32, CornerRadius, RichText, Stroke, StrokeKind};

use crate::{
    DataLimitation, EmptyReason, MutationStatus, PresentableData, Route, ThemeTokens, UiAction,
    UiModel, design,
};

/// Renders the application shell and returns whether ordinary input changed state.
pub(crate) fn render<T>(ui: &mut egui::Ui, model: &mut UiModel<T>, tokens: ThemeTokens) -> bool {
    let mut changed = false;
    egui::Frame::new()
        .fill(design::TITLEBAR)
        .inner_margin(egui::Margin::symmetric(22, 12))
        .show(ui, |ui| changed |= render_navigation(ui, model));
    // 1px title-bar bottom border.
    let separator_rect = egui::Rect::from_min_size(
        ui.cursor().min,
        egui::vec2(ui.available_width(), 1.0),
    );
    ui.painter()
        .rect_filled(separator_rect, 0.0, design::TITLEBAR_BORDER);

    egui::Frame::new()
        .fill(tokens.canvas())
        .inner_margin(egui::Margin::symmetric(26, 22))
        .show(ui, |ui| {
            changed |= render_presentation_state(ui, model.data(), tokens);
            changed |= render_mutation_state(ui, model, tokens);
        });
    changed
}

fn render_navigation<T>(ui: &mut egui::Ui, model: &mut UiModel<T>) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        render_logo_chip(ui);
        ui.label(
            RichText::new("OpenManic")
                .size(16.0)
                .strong()
                .color(design::TEXT_PRIMARY),
        );
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for route in Route::all() {
                if design::nav_pill(ui, route.label(), model.route() == route) {
                    crate::reducer::reduce(model, UiAction::Navigate(route));
                    changed = true;
                }
            }
        });
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 9.0;
            // Faux window dots.
            for color in [design::AWAY, design::ACTIVE, Color32::from_rgb(0xF5, 0xB5, 0x4A)] {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(13.0, 13.0), egui::Sense::hover());
                ui.painter()
                    .circle_filled(rect.center(), 6.5, color);
            }
            ui.add_space(5.0);
            render_monitoring_indicator(ui);
        });
    });
    changed
}

fn render_logo_chip(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
    let painter = ui.painter();
    // Glow, then gradient chip, then wordmark initials.
    painter.rect_filled(
        rect.expand(4.0),
        CornerRadius::same(12),
        design::ACCENT.gamma_multiply(0.18),
    );
    design::paint_cell_gradient(painter, rect, design::ACCENT_LIGHT);
    painter.rect_stroke(
        rect,
        CornerRadius::same(9),
        Stroke::new(1.0, design::ACCENT_LIGHT.gamma_multiply(0.6)),
        StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "OM",
        egui::FontId::proportional(13.0),
        Color32::WHITE,
    );
}

fn render_monitoring_indicator(ui: &mut egui::Ui) {
    ui.label(
        RichText::new("Monitoring")
            .size(13.0)
            .strong()
            .color(design::ACTIVE),
    );
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    let time = ui.input(|input| input.time);
    // Pulse between 0.35 and 1.0 opacity on a two-second cycle.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the bounded pulse phase fits f32 precision requirements"
    )]
    let phase = ((time * std::f64::consts::PI).sin().abs()) as f32;
    let alpha = 0.35 + 0.65 * phase;
    ui.painter().circle_filled(
        rect.center(),
        4.0,
        design::ACTIVE.gamma_multiply(alpha),
    );
    ui.painter().circle_filled(
        rect.center(),
        7.0,
        design::ACTIVE.gamma_multiply(alpha * 0.25),
    );
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(90));
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
    design::card_frame().show(ui, |ui| {
        ui.colored_label(color, message);
        if matches!(data, PresentableData::Failed { .. }) {
            ui.small("Technical details can be added by the route controller.");
        }
    });
    ui.add_space(11.0);
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
    design::card_frame().show(ui, |ui| {
        ui.colored_label(color, message);
        ui.small(format!("Command {}", command_id.get()));
    });
    ui.add_space(11.0);
    false
}

fn latest_mutation<T>(
    model: &UiModel<T>,
) -> Option<(openmanic_application::CommandId, &MutationStatus)> {
    model.latest_mutation()
}
