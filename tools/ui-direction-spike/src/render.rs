//! Low-fidelity native egui rendering for the review-only mock model.

use crate::model::{
    BandKind, CommandState, DistributionPresentation, LayoutEditState, MockDataState, MockOverlay,
    MockSegment, MockSegmentRef, MockSelection, OverviewRange, PreviewWidth, Route, SpikeAction,
    SpikeState, TimeRange,
};
use eframe::egui::{self, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Stroke, Vec2};

/// The eframe application that hosts the direction spike.
pub(crate) struct UiDirectionApp {
    state: SpikeState,
    tokens: SpikeTokens,
}

impl Default for UiDirectionApp {
    fn default() -> Self {
        Self {
            state: SpikeState::default(),
            tokens: SpikeTokens::dark(),
        }
    }
}

impl eframe::App for UiDirectionApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Frame::new().fill(self.tokens.canvas).show(ui, |ui| {
            ui.add_space(self.tokens.space);
            render_top_controls(ui, &mut self.state, &self.tokens);
            ui.add_space(self.tokens.space);
            egui::ScrollArea::vertical().show(ui, |ui| {
                render_preview(ui, &mut self.state, &self.tokens);
                ui.add_space(self.tokens.space * 2.0);
            });
        });
    }
}

fn render_preview(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    let desired_width = f32::from(state.preview_width.logical_pixels());
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - desired_width) / 2.0).max(0.0));
        egui::Frame::new()
            .fill(tokens.panel)
            .inner_margin(egui::Margin::same(16))
            .stroke(Stroke::new(1.0, tokens.outline))
            .show(ui, |ui| {
                ui.set_width(desired_width.min(ui.available_width()));
                render_shell(ui, state, tokens);
            });
    });
}

/// A small local semantic token set. It is intentionally not the future `ThemeSpec` schema.
#[derive(Clone, Copy)]
struct SpikeTokens {
    canvas: Color32,
    panel: Color32,
    card: Color32,
    overlay: Color32,
    outline: Color32,
    content: Color32,
    content_secondary: Color32,
    accent: Color32,
    selected: Color32,
    success: Color32,
    warning: Color32,
    error: Color32,
    active: Color32,
    away: Color32,
    unavailable: Color32,
    category: Color32,
    application: Color32,
    schedule: Color32,
    focus: Color32,
    space: f32,
}

impl SpikeTokens {
    fn dark() -> Self {
        Self {
            canvas: Color32::from_rgb(18, 22, 31),
            panel: Color32::from_rgb(27, 33, 45),
            card: Color32::from_rgb(35, 43, 58),
            overlay: Color32::from_rgb(49, 59, 77),
            outline: Color32::from_rgb(83, 99, 125),
            content: Color32::from_rgb(238, 242, 250),
            content_secondary: Color32::from_rgb(176, 188, 208),
            accent: Color32::from_rgb(121, 151, 255),
            selected: Color32::from_rgb(92, 205, 184),
            success: Color32::from_rgb(107, 201, 139),
            warning: Color32::from_rgb(236, 190, 93),
            error: Color32::from_rgb(237, 113, 113),
            active: Color32::from_rgb(86, 180, 227),
            away: Color32::from_rgb(236, 190, 93),
            unavailable: Color32::from_rgb(237, 113, 113),
            category: Color32::from_rgb(96, 186, 139),
            application: Color32::from_rgb(121, 151, 255),
            schedule: Color32::from_rgb(244, 166, 85),
            focus: Color32::from_rgb(190, 126, 226),
            space: 8.0,
        }
    }

    fn segment_color(self, segment: &MockSegment) -> Color32 {
        match segment.band {
            BandKind::Category => self.category,
            BandKind::Application => self.application,
            BandKind::ActivityState => match segment.label.as_str() {
                "active" => self.active,
                "idle" | "away" => self.away,
                "Powered off" => self.panel,
                _ => self.unavailable,
            },
        }
    }
}

fn render_top_controls(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    ui.horizontal_wrapped(|ui| {
        ui.strong("OpenManic direction spike");
        ui.colored_label(
            tokens.content_secondary,
            "Review-only mock snapshots and local actions",
        );
        ui.separator();
        for width in PreviewWidth::all() {
            let label = format!("{} px", width.logical_pixels());
            if selectable_button(ui, label, state.preview_width == width).clicked() {
                state.reduce(SpikeAction::SelectPreviewWidth(width));
            }
        }
        ui.separator();
        render_data_state_picker(ui, state);
        ui.separator();
        ui.colored_label(
            tokens.content_secondary,
            "Width presets are logical-layout previews, not DPI evidence.",
        );
    });
}

fn render_data_state_picker(ui: &mut egui::Ui, state: &mut SpikeState) {
    egui::ComboBox::from_id_salt("mock-data-state")
        .selected_text(state.data_state.label())
        .show_ui(ui, |ui| {
            for data_state in MockDataState::all() {
                if ui
                    .selectable_label(state.data_state == data_state, data_state.label())
                    .clicked()
                {
                    state.reduce(SpikeAction::SelectDataState(data_state));
                }
            }
        });
}

fn render_shell(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    ui.horizontal_wrapped(|ui| {
        for route in Route::all() {
            if selectable_button(ui, route.label(), state.route == route).clicked() {
                state.reduce(SpikeAction::Navigate(route));
            }
        }
        ui.separator();
        ui.colored_label(tokens.active, "Tracking active");
        if ui.button("Pause tracking").clicked() {
            state.reduce(SpikeAction::BeginCommand("Pausing tracking"));
        }
        if ui.button("Settings").clicked() {
            state.reduce(SpikeAction::Navigate(Route::Settings));
        }
    });
    ui.add_space(tokens.space);
    render_command_status(ui, state, tokens);
    render_data_state_banner(ui, state, tokens);
    ui.add_space(tokens.space);
    match state.route {
        Route::Today => render_today(ui, state, tokens),
        Route::Overview => render_overview(ui, state, tokens),
        Route::Categories => render_categories(ui, state, tokens),
        Route::Calendar => render_calendar(ui, state, tokens),
        Route::Settings => render_settings(ui, state, tokens),
    }
}

fn render_command_status(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    let (message, color) = match &state.command {
        CommandState::Idle => return,
        CommandState::Pending(message) => (*message, tokens.warning),
        CommandState::Confirmed(message) => (*message, tokens.success),
        CommandState::Rejected(message) => (*message, tokens.error),
    };
    egui::Frame::new().fill(tokens.overlay).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(color, message);
            if matches!(state.command, CommandState::Pending(_)) {
                render_pending_command_controls(ui, state);
            }
        });
    });
    ui.add_space(tokens.space);
}

fn render_pending_command_controls(ui: &mut egui::Ui, state: &mut SpikeState) {
    if ui.button("Simulate accepted").clicked() {
        state.reduce(SpikeAction::ConfirmCommand(
            "Action accepted in the local spike",
        ));
    }
    if ui.button("Simulate rejected").clicked() {
        state.reduce(SpikeAction::RejectCommand(
            "Action rejected; review the inline explanation",
        ));
    }
}

fn render_data_state_banner(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    let (message, color) = match state.data_state {
        MockDataState::Ready => return,
        MockDataState::InitialLoading => {
            ("Loading an initial local mock snapshot…", tokens.warning)
        }
        MockDataState::Refreshing => ("Refreshing — prior values remain visible.", tokens.accent),
        MockDataState::Empty => (
            "No activity matches this date and narrowing criteria.",
            tokens.content_secondary,
        ),
        MockDataState::Partial => (
            "Partial data: tracking was paused or unavailable for part of this range.",
            tokens.warning,
        ),
        MockDataState::Failed => (
            "This mock request failed. Tracking data remains safe; retry or inspect details.",
            tokens.error,
        ),
        MockDataState::Recovered => (
            "A recoverable issue was repaired; the returned snapshot is now visible.",
            tokens.success,
        ),
    };
    egui::Frame::new().fill(tokens.overlay).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(color, message);
            if state.data_state == MockDataState::Failed {
                render_failed_data_state_controls(ui, state);
            }
        });
    });
}

fn render_failed_data_state_controls(ui: &mut egui::Ui, state: &mut SpikeState) {
    if ui.button("Retry locally").clicked() {
        state.reduce(SpikeAction::SelectDataState(MockDataState::Refreshing));
    }
    egui::CollapsingHeader::new("Technical details").show(ui, |ui| {
        ui.monospace("Mock query: deterministic review fixture unavailable");
    });
}

fn render_today(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    heading(
        ui,
        "Today",
        "A daily dashboard with the Timeline as its central flow.",
        tokens,
    );
    render_today_navigation(ui, state, tokens);
    render_narrowing(ui, state, tokens);
    render_layout_controls(ui, state, tokens);
    if state.data_state == MockDataState::Empty {
        empty_card(
            ui,
            "No recorded activity on this date",
            "Choose another day or clear narrowing criteria.",
            tokens,
        );
        return;
    }
    render_timeline(ui, state, tokens);
    ui.add_space(tokens.space);
    if state.layout.swapped_supporting_widgets {
        render_supporting_widgets(ui, state, tokens, true);
    } else {
        render_supporting_widgets(ui, state, tokens, false);
    }
}

fn render_today_navigation(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("Previous day").clicked() {
            state.reduce(SpikeAction::TodayPrevious);
        }
        let date_label = if state.today_offset == 0 {
            "Today"
        } else {
            "Yesterday"
        };
        ui.colored_label(tokens.content, format!("Selected date: {date_label}"));
        if ui
            .add_enabled(state.today_offset < 0, egui::Button::new("Next day"))
            .clicked()
        {
            state.reduce(SpikeAction::TodayNext);
        }
        if ui.button("Date picker (mock)").clicked() {
            state.reduce(SpikeAction::TodayPrevious);
        }
        if ui
            .add_enabled(state.today_offset < 0, egui::Button::new("Today"))
            .clicked()
        {
            state.reduce(SpikeAction::TodayGoCurrent);
        }
    });
    ui.add_space(tokens.space);
}

fn render_narrowing(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    egui::Frame::new().fill(tokens.card).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label("Narrowing:");
            if selectable_button(ui, "Application 1", state.application_filter).clicked() {
                state.reduce(SpikeAction::ToggleApplicationFilter);
            }
            if selectable_button(ui, "Productive", state.category_filter).clicked() {
                state.reduce(SpikeAction::ToggleCategoryFilter);
            }
            ui.colored_label(tokens.content_secondary, state.narrowing_summary());
            if ui.button("Clear all").clicked() {
                state.reduce(SpikeAction::ClearNarrowing);
            }
        });
    });
    ui.add_space(tokens.space);
}

fn render_layout_controls(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    ui.horizontal_wrapped(|ui| match state.layout_edit {
        LayoutEditState::Viewing => {
            if ui.button("Edit layout").clicked() {
                state.reduce(SpikeAction::BeginLayoutEdit);
            }
        }
        LayoutEditState::Editing { .. } => {
            ui.colored_label(
                tokens.warning,
                "Layout editing — widget content interaction is isolated.",
            );
            if ui.button("Reorder supporting widgets").clicked() {
                state.reduce(SpikeAction::SwapSupportingWidgets);
            }
            if ui.button("Resize Timeline").clicked() {
                state.reduce(SpikeAction::ResizeTimeline);
            }
            if ui.button("Reset draft").clicked() {
                state.reduce(SpikeAction::ResetLayout);
            }
            if ui.button("Save").clicked() {
                state.reduce(SpikeAction::SaveLayout);
            }
            if ui.button("Cancel/Revert").clicked() {
                state.reduce(SpikeAction::CancelLayout);
            }
        }
    });
    ui.add_space(tokens.space);
}

fn render_timeline(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    card(ui, tokens, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Activity Timeline");
            ui.colored_label(
                tokens.content_secondary,
                "Three continuous bands · one pointer response",
            );
            if ui.button("← Pan").clicked() {
                state.reduce(SpikeAction::PanTimeline(-15.0));
            }
            if ui.button("Pan →").clicked() {
                state.reduce(SpikeAction::PanTimeline(15.0));
            }
            if ui.button("Zoom in").clicked() {
                state.reduce(SpikeAction::ZoomTimeline(0.75));
            }
            if ui.button("Zoom out").clicked() {
                state.reduce(SpikeAction::ZoomTimeline(1.25));
            }
            if ui.button("Reset view").clicked() {
                state.reduce(SpikeAction::ResetTimeline);
            }
            if selectable_button(ui, "Create schedule", state.timeline.create_schedule_mode)
                .clicked()
            {
                state.reduce(SpikeAction::ToggleCreateScheduleMode);
            }
        });
        ui.add_space(4.0);
        let desired_size = Vec2::new(ui.available_width().max(380.0), 226.0);
        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());
        paint_timeline(ui, rect, state, tokens);
        handle_timeline_interaction(ui, &response, rect, state);
        render_timeline_details(ui, state, tokens);
    });
}

fn paint_timeline(ui: &egui::Ui, rect: Rect, state: &SpikeState, tokens: &SpikeTokens) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, tokens.overlay);
    let title_width = 92.0;
    let plot = Rect::from_min_max(
        Pos2::new(rect.left() + title_width, rect.top() + 20.0),
        Pos2::new(rect.right() - 8.0, rect.bottom() - 26.0),
    );
    let category_rect = Rect::from_min_max(plot.min, Pos2::new(plot.right(), plot.top() + 78.0));
    let state_rect = Rect::from_min_max(
        Pos2::new(plot.left(), category_rect.bottom()),
        Pos2::new(plot.right(), category_rect.bottom() + 34.0),
    );
    let application_rect = Rect::from_min_max(
        Pos2::new(plot.left(), state_rect.bottom()),
        Pos2::new(plot.right(), plot.bottom()),
    );
    paint_band(
        &painter,
        category_rect,
        &state.snapshot.timeline.category.segments,
        state,
        tokens,
    );
    paint_band(
        &painter,
        state_rect,
        &state.snapshot.timeline.state.segments,
        state,
        tokens,
    );
    paint_band(
        &painter,
        application_rect,
        &state.snapshot.timeline.application.segments,
        state,
        tokens,
    );
    for (label, band_rect) in [
        ("Category", category_rect),
        ("Tracking state", state_rect),
        ("Application", application_rect),
    ] {
        painter.text(
            Pos2::new(rect.left() + 6.0, band_rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(12.0),
            tokens.content_secondary,
        );
    }
    for overlay in &state.snapshot.timeline.overlays {
        paint_overlay(&painter, plot, overlay, state, tokens);
    }
    if let Some(provisional) = state.timeline.provisional_schedule {
        paint_bracket(&painter, plot, provisional, tokens.accent, 3.0, state);
    }
    paint_time_axis(&painter, plot, state, tokens);
    if let Some(MockSelection::Range(range)) = state.timeline.selection {
        let left = time_to_x(plot, state, range.start);
        let right = time_to_x(plot, state, range.end);
        painter.rect_filled(
            Rect::from_min_max(Pos2::new(left, plot.top()), Pos2::new(right, plot.bottom())),
            0.0,
            tokens.selected.linear_multiply(0.20),
        );
    }
}

fn paint_band(
    painter: &Painter,
    rect: Rect,
    segments: &[MockSegment],
    state: &SpikeState,
    tokens: &SpikeTokens,
) {
    painter.rect_filled(rect, 0.0, tokens.panel);
    for segment in segments {
        let left = time_to_x(rect, state, segment.start);
        let right = time_to_x(rect, state, segment.end);
        let segment_rect =
            Rect::from_min_max(Pos2::new(left, rect.top()), Pos2::new(right, rect.bottom()));
        if segment.unfilled {
            painter.line_segment(
                [segment_rect.left_top(), segment_rect.left_bottom()],
                Stroke::new(2.0, tokens.content_secondary),
            );
            painter.line_segment(
                [segment_rect.right_top(), segment_rect.right_bottom()],
                Stroke::new(2.0, tokens.content_secondary),
            );
        } else {
            painter.rect_filled(segment_rect, 0.0, tokens.segment_color(segment));
        }
        if selection_matches(state.timeline.selection.as_ref(), segment) {
            painter.line_segment(
                [segment_rect.left_top(), segment_rect.right_top()],
                Stroke::new(3.0, tokens.selected),
            );
            painter.line_segment(
                [segment_rect.left_bottom(), segment_rect.right_bottom()],
                Stroke::new(3.0, tokens.selected),
            );
        }
    }
}

fn paint_overlay(
    painter: &Painter,
    plot: Rect,
    overlay: &MockOverlay,
    state: &SpikeState,
    tokens: &SpikeTokens,
) {
    if overlay.schedule {
        paint_bracket(painter, plot, overlay.range, tokens.schedule, 1.0, state);
    } else {
        let left = time_to_x(plot, state, overlay.range.start);
        let right = time_to_x(plot, state, overlay.range.end);
        let focus_rect = Rect::from_min_max(
            Pos2::new(left, plot.top() + 4.0),
            Pos2::new(right, plot.bottom() - 4.0),
        );
        painter.rect_filled(focus_rect, 0.0, tokens.focus.linear_multiply(0.28));
    }
}

fn paint_bracket(
    painter: &Painter,
    plot: Rect,
    range: TimeRange,
    color: Color32,
    offset: f32,
    state: &SpikeState,
) {
    let left = time_to_x(plot, state, range.start) + offset;
    let right = time_to_x(plot, state, range.end) - offset;
    let top = plot.top() - 4.0 - offset;
    let bottom = plot.bottom() + 4.0 + offset;
    let stroke = Stroke::new(2.0, color);
    painter.line_segment([Pos2::new(left, top), Pos2::new(left, bottom)], stroke);
    painter.line_segment([Pos2::new(right, top), Pos2::new(right, bottom)], stroke);
    painter.line_segment([Pos2::new(left, top), Pos2::new(left + 8.0, top)], stroke);
    painter.line_segment(
        [Pos2::new(left, bottom), Pos2::new(left + 8.0, bottom)],
        stroke,
    );
    painter.line_segment([Pos2::new(right - 8.0, top), Pos2::new(right, top)], stroke);
    painter.line_segment(
        [Pos2::new(right - 8.0, bottom), Pos2::new(right, bottom)],
        stroke,
    );
}

fn paint_time_axis(painter: &Painter, plot: Rect, state: &SpikeState, tokens: &SpikeTokens) {
    for seconds in [0.0, 30.0, 60.0, 90.0, 120.0] {
        let x = time_to_x(plot, state, seconds);
        painter.line_segment(
            [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
            Stroke::new(1.0, tokens.outline.linear_multiply(0.35)),
        );
        painter.text(
            Pos2::new(x, plot.bottom() + 9.0),
            Align2::CENTER_TOP,
            format_time(seconds),
            FontId::proportional(11.0),
            tokens.content_secondary,
        );
    }
}

fn handle_timeline_interaction(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: Rect,
    state: &mut SpikeState,
) {
    let plot = timeline_plot_rect(rect);
    let hover = response
        .hover_pos()
        .and_then(|position| hit_segment(state, plot, position))
        .map(segment_reference);
    state.reduce(SpikeAction::SetHover(hover));
    if response.drag_started()
        && let Some(position) = response.interact_pointer_pos()
    {
        state.reduce(SpikeAction::BeginTimelineDrag(x_to_time(
            plot, state, position.x,
        )));
    }
    if response.drag_stopped()
        && let Some(position) = response.interact_pointer_pos()
    {
        state.reduce(SpikeAction::EndTimelineDrag(x_to_time(
            plot, state, position.x,
        )));
    }
    if response.clicked() {
        if let Some(segment) = response
            .interact_pointer_pos()
            .and_then(|position| hit_segment(state, plot, position))
        {
            state.reduce(SpikeAction::SelectSegment(segment_reference(segment)));
        } else {
            state.reduce(SpikeAction::ClearTimelineSelection);
        }
    }
    if response.hovered() {
        let wheel_delta = ui.ctx().input(|input| input.smooth_scroll_delta.y);
        if wheel_delta.abs() > 0.0 {
            let multiplier = if wheel_delta > 0.0 { 0.85 } else { 1.18 };
            state.reduce(SpikeAction::ZoomTimeline(multiplier));
        }
    }
}

#[expect(
    clippy::excessive_nesting,
    reason = "The review-only timeline keeps its related immediate-mode controls in one scoped panel."
)]
fn render_timeline_details(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    ui.horizontal_wrapped(|ui| {
        match &state.timeline.hover {
            Some(hover) => ui.colored_label(
                tokens.content_secondary,
                format!(
                    "Hover: {} · {} · 09:00–09:30",
                    hover.band.label(),
                    hover.label
                ),
            ),
            None => ui.colored_label(
                tokens.content_secondary,
                "Hover a segment for exact local details.",
            ),
        };
        match &state.timeline.selection {
            Some(MockSelection::Segment(segment)) => {
                ui.colored_label(tokens.selected, format!("Selected: {}", segment.label));
                match segment.band {
                    BandKind::Category => {
                        if ui.button("Edit category").clicked() {
                            state.reduce(SpikeAction::Navigate(Route::Categories));
                            state.reduce(SpikeAction::BeginCommand("Opening category editor"));
                        }
                    }
                    BandKind::Application => {
                        if ui.button("Assign category").clicked() {
                            state.reduce(SpikeAction::Navigate(Route::Categories));
                            state.reduce(SpikeAction::BeginCommand(
                                "Opening application assignment",
                            ));
                        }
                    }
                    BandKind::ActivityState => {
                        ui.label(
                            "State details are inspectable; unsupported state edits are absent.",
                        );
                    }
                }
            }
            Some(MockSelection::Range(range)) => {
                ui.colored_label(
                    tokens.selected,
                    format!("Selected range: {}", format_range(*range)),
                );
                if ui.button("Clear selection").clicked() {
                    state.reduce(SpikeAction::ClearTimelineSelection);
                }
            }
            None => {}
        }
    });
    if let Some(range) = state.timeline.provisional_schedule {
        egui::Frame::new().fill(tokens.overlay).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    tokens.schedule,
                    format!("Schedule draft: {}", format_range(range)),
                );
                ui.label("Exact times and recurrence would open in the shared editor.");
                if ui.button("Save mock schedule").clicked() {
                    state.reduce(SpikeAction::ConfirmCommand(
                        "Schedule accepted in the local spike",
                    ));
                }
                if ui.button("Cancel draft").clicked() {
                    state.timeline.provisional_schedule = None;
                }
            });
        });
    }
}

type WidgetRenderer = fn(&mut egui::Ui, &mut SpikeState, &SpikeTokens);

fn render_supporting_widgets(
    ui: &mut egui::Ui,
    state: &mut SpikeState,
    tokens: &SpikeTokens,
    reverse: bool,
) {
    if state.preview_width == PreviewWidth::Compact {
        render_usage_widget(ui, state, tokens);
        render_distribution_widget(ui, state, tokens);
        render_focus_widget(ui, state, tokens);
        return;
    }
    let mut renderers: [WidgetRenderer; 3] = [
        render_usage_widget,
        render_distribution_widget,
        render_focus_widget,
    ];
    if reverse {
        renderers.swap(0, 1);
    }
    ui.columns(3, |columns| {
        for (index, render) in renderers.into_iter().enumerate() {
            render(&mut columns[index], state, tokens);
        }
    });
}

fn render_usage_widget(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    card(ui, tokens, |ui| {
        ui.strong("Application usage");
        let applications = state.snapshot.applications.clone();
        for (index, application) in applications.iter().take(3).enumerate() {
            if ui
                .selectable_label(
                    state.application_filter && index == 0,
                    format!(
                        "{} · {} · {}%",
                        application.name, application.duration, application.percent
                    ),
                )
                .clicked()
            {
                state.reduce(SpikeAction::ToggleApplicationFilter);
            }
        }
        ui.colored_label(
            tokens.content_secondary,
            "Exact duration remains visible in compact presentation.",
        );
    });
}

#[expect(
    clippy::excessive_nesting,
    reason = "The review-only distribution alternatives share a compact immediate-mode card."
)]
fn render_distribution_widget(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    card(ui, tokens, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Time distribution");
            for presentation in [
                DistributionPresentation::StackedBar,
                DistributionPresentation::Ring,
            ] {
                if selectable_button(ui, presentation.label(), state.distribution == presentation)
                    .clicked()
                {
                    state.reduce(SpikeAction::SelectDistribution(presentation));
                }
            }
        });
        match state.distribution {
            DistributionPresentation::StackedBar => paint_distribution_bar(ui, tokens),
            DistributionPresentation::Ring => paint_distribution_ring(ui, tokens),
        }
        ui.label("Grouping: Category · Total included time: 3 h 12 min");
        ui.colored_label(
            tokens.content_secondary,
            "Provisional default: labeled stacked bar.",
        );
    });
}

fn paint_distribution_bar(ui: &mut egui::Ui, tokens: &SpikeTokens) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
    let painter = ui.painter_at(rect);
    let colors = [tokens.category, tokens.application, tokens.away];
    let weights = [0.48, 0.32, 0.20];
    let mut left = rect.left();
    for (weight, color) in weights.into_iter().zip(colors) {
        let right = left + rect.width() * weight;
        painter.rect_filled(
            Rect::from_min_max(Pos2::new(left, rect.top()), Pos2::new(right, rect.bottom())),
            2.0,
            color,
        );
        left = right;
    }
    ui.label("Productive 48% · Communication 32% · Other 20%");
}

fn paint_distribution_ring(ui: &mut egui::Ui, tokens: &SpikeTokens) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(58.0), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.circle_filled(rect.center(), 25.0, tokens.category);
        painter.circle_filled(rect.center(), 15.0, tokens.card);
        ui.vertical(|ui| {
            ui.label("Productive 48%");
            ui.label("Communication 32%");
            ui.label("Other 20%");
        });
    });
}

fn render_focus_widget(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    card(ui, tokens, |ui| {
        ui.strong("Focus session");
        ui.label("Ready · 25 min focus · planned end 10:25");
        if ui.button("Start focus").clicked() {
            state.reduce(SpikeAction::BeginCommand("Starting focus session"));
        }
        ui.colored_label(
            tokens.content_secondary,
            "Running appears only after simulated acceptance.",
        );
    });
}

fn render_overview(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    heading(
        ui,
        "Overview",
        "Review allocation across an exact selected range.",
        tokens,
    );
    ui.horizontal_wrapped(|ui| {
        for range in OverviewRange::all() {
            if selectable_button(ui, range.label(), state.overview_range == range).clicked() {
                state.reduce(SpikeAction::SetOverviewRange(range));
            }
        }
        ui.label(format!(
            "Exact range: {} review period",
            state.overview_range.label()
        ));
        if ui.button("Save view locally").clicked() {
            state.reduce(SpikeAction::SaveOverviewView);
        }
    });
    if let Some(view) = &state.saved_view_name {
        ui.colored_label(
            tokens.success,
            format!("Saved-view-like local flow: {view}"),
        );
    }
    ui.add_space(tokens.space);
    render_distribution_widget(ui, state, tokens);
    ui.add_space(tokens.space);
    render_usage_widget(ui, state, tokens);
    ui.colored_label(
        tokens.content_secondary,
        "Selecting an allocation would narrow compatible support widgets in production.",
    );
}

#[expect(
    clippy::excessive_nesting,
    reason = "Each compact category row keeps its selection and secondary-detail controls together."
)]
fn render_categories(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    heading(
        ui,
        "Categories",
        "Assign several known applications without losing selection.",
        tokens,
    );
    ui.horizontal_wrapped(|ui| {
        let mut search = state.categories.search.clone();
        if ui
            .add(egui::TextEdit::singleline(&mut search).hint_text("Search applications"))
            .changed()
        {
            state.reduce(SpikeAction::SetCategorySearch(search));
        }
        if selectable_button(ui, "Uncategorized", state.categories.uncategorized_only).clicked() {
            state.reduce(SpikeAction::ToggleUncategorizedOnly);
        }
        ui.label(format!("{} selected", state.categories.selected_rows.len()));
    });
    ui.add_space(tokens.space);
    let applications = state.snapshot.applications.clone();
    for (index, application) in applications.iter().enumerate() {
        if state.categories.uncategorized_only && index != 3 {
            continue;
        }
        let selected = state.categories.selected_rows.contains(&index);
        card(ui, tokens, |ui| {
            ui.horizontal_wrapped(|ui| {
                if selectable_button(ui, "Select", selected).clicked() {
                    state.reduce(SpikeAction::ToggleCategoryRow(index));
                }
                ui.label(&application.name);
                ui.colored_label(
                    tokens.content_secondary,
                    format!("{} · {}", application.category, application.duration),
                );
                egui::CollapsingHeader::new("Details").show(ui, |ui| {
                    ui.label("Executable identity/path appears only as a secondary detail.");
                });
            });
        });
    }
    ui.horizontal_wrapped(|ui| {
        for category in ["Productive", "Communication", "Create ‘Study’"] {
            if ui.button(category).clicked() {
                state.reduce(SpikeAction::AssignSelectedCategory(category.to_owned()));
            }
        }
    });
    if let Some(category) = &state.categories.assigned_category {
        ui.colored_label(
            tokens.warning,
            format!("Pending local bulk assignment: {category}"),
        );
    }
}

fn render_calendar(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    heading(
        ui,
        "Calendar",
        "A vertical day review with activity, focus, and schedule layers.",
        tokens,
    );
    ui.horizontal_wrapped(|ui| {
        if ui.button("Previous day").clicked() {
            state.reduce(SpikeAction::ShiftCalendar(-1));
        }
        ui.label(if state.calendar_offset == 0 {
            "Today"
        } else {
            "Earlier day"
        });
        if ui
            .add_enabled(state.calendar_offset < 0, egui::Button::new("Next day"))
            .clicked()
        {
            state.reduce(SpikeAction::ShiftCalendar(1));
        }
        if ui.button("Today").clicked() {
            state.reduce(SpikeAction::CalendarGoCurrent);
        }
        if ui.button("Create schedule mode").clicked() {
            state.reduce(SpikeAction::ToggleCreateScheduleMode);
        }
        if ui.button("Open matching Timeline").clicked() {
            state.reduce(SpikeAction::Navigate(Route::Today));
        }
    });
    ui.add_space(tokens.space);
    card(ui, tokens, |ui| render_calendar_surface(ui, state, tokens));
    ui.colored_label(
        tokens.content_secondary,
        "Overnight continuation and exact schedule times are review labels, not a schedule contract.",
    );
}

fn render_calendar_surface(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 360.0), Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, tokens.overlay);
    let axis = rect.left() + 58.0;
    painter.line_segment(
        [
            Pos2::new(axis, rect.top() + 12.0),
            Pos2::new(axis, rect.bottom() - 12.0),
        ],
        Stroke::new(1.0, tokens.outline),
    );
    for hour in [9_i16, 10, 11, 12, 13, 14] {
        let y = rect.top() + 20.0 + f32::from(hour - 9) * 52.0;
        painter.text(
            Pos2::new(rect.left() + 8.0, y),
            Align2::LEFT_CENTER,
            format!("{hour}:00"),
            FontId::proportional(12.0),
            tokens.content_secondary,
        );
        painter.line_segment(
            [Pos2::new(axis, y), Pos2::new(rect.right() - 8.0, y)],
            Stroke::new(1.0, tokens.outline.linear_multiply(0.35)),
        );
    }
    let activity = Rect::from_min_max(
        Pos2::new(axis + 16.0, rect.top() + 44.0),
        Pos2::new(rect.right() - 26.0, rect.top() + 192.0),
    );
    painter.rect_filled(activity, 2.0, tokens.application.linear_multiply(0.72));
    let focus = Rect::from_min_max(
        Pos2::new(axis + 30.0, rect.top() + 104.0),
        Pos2::new(rect.right() - 42.0, rect.top() + 252.0),
    );
    painter.rect_filled(focus, 2.0, tokens.focus.linear_multiply(0.42));
    let left = axis + 24.0;
    let right = rect.right() - 32.0;
    let top = rect.top() + 72.0;
    let bottom = rect.top() + 220.0;
    for x in [left, right] {
        painter.line_segment(
            [Pos2::new(x, top), Pos2::new(x, bottom)],
            Stroke::new(2.0, tokens.schedule),
        );
    }
    if response.clicked() {
        state.reduce(SpikeAction::BeginCommand(
            "Calendar block selected: 09:30–12:20",
        ));
    }
}

fn render_settings(ui: &mut egui::Ui, state: &mut SpikeState, tokens: &SpikeTokens) {
    heading(
        ui,
        "Settings",
        "Plain-language controls with secondary technical disclosure.",
        tokens,
    );
    card(ui, tokens, |ui| {
        ui.strong("Tracking and privacy");
        let mut start_tracking = state.settings.start_tracking_automatically;
        if ui
            .checkbox(&mut start_tracking, "Start tracking automatically")
            .changed()
        {
            state.reduce(SpikeAction::SetStartTrackingAutomatically(start_tracking));
        }
        let mut collect_titles = state.settings.collect_window_titles;
        if ui
            .checkbox(&mut collect_titles, "Collect window titles")
            .changed()
        {
            state.reduce(SpikeAction::SetCollectWindowTitles(collect_titles));
        }
        ui.label(
            "Window-title collection affects future activity only; existing records are unchanged.",
        );
        if ui.button("Pause tracking").clicked() {
            state.reduce(SpikeAction::BeginCommand("Pausing tracking"));
        }
    });
    ui.add_space(tokens.space);
    card(ui, tokens, |ui| {
        ui.strong("Appearance and focus");
        ui.label("Theme: Dark · Density: Comfortable · Focus sounds: On");
        ui.label("Platform permission status: tracking is active.");
    });
    ui.add_space(tokens.space);
    if selectable_button(ui, "Advanced settings", state.settings.advanced_visible).clicked() {
        state.reduce(SpikeAction::ToggleAdvancedSettings);
    }
    if state.settings.advanced_visible {
        card(ui, tokens, |ui| {
            ui.strong("Advanced details");
            ui.label(
                "Data location, import/export, exact identities, and diagnostics are grouped here.",
            );
            ui.monospace(
                "Mock diagnostic: deterministic review fixture · no platform or storage I/O",
            );
        });
    }
}

fn heading(ui: &mut egui::Ui, title: &str, subtitle: &str, tokens: &SpikeTokens) {
    ui.heading(title);
    ui.colored_label(tokens.content_secondary, subtitle);
    ui.add_space(tokens.space);
}

fn card(ui: &mut egui::Ui, tokens: &SpikeTokens, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(tokens.card)
        .inner_margin(egui::Margin::same(10))
        .stroke(Stroke::new(1.0, tokens.outline.linear_multiply(0.7)))
        .show(ui, add_contents);
}

fn empty_card(ui: &mut egui::Ui, title: &str, detail: &str, tokens: &SpikeTokens) {
    card(ui, tokens, |ui| {
        ui.strong(title);
        ui.colored_label(tokens.content_secondary, detail);
    });
}

fn selectable_button(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    selected: bool,
) -> egui::Response {
    ui.add(egui::Button::new(label).selected(selected))
}

fn timeline_plot_rect(rect: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(rect.left() + 92.0, rect.top() + 20.0),
        Pos2::new(rect.right() - 8.0, rect.bottom() - 26.0),
    )
}

fn time_to_x(plot: Rect, state: &SpikeState, time: f32) -> f32 {
    let normalized = (time - state.timeline.view_start) / state.timeline.view_span;
    plot.left() + plot.width() * normalized
}

fn x_to_time(plot: Rect, state: &SpikeState, x: f32) -> f32 {
    let normalized = ((x - plot.left()) / plot.width()).clamp(0.0, 1.0);
    state.timeline.view_start + normalized * state.timeline.view_span
}

fn hit_segment(state: &SpikeState, plot: Rect, position: Pos2) -> Option<&MockSegment> {
    let category_bottom = plot.top() + 78.0;
    let state_bottom = category_bottom + 34.0;
    let band = if position.y < category_bottom {
        &state.snapshot.timeline.category.segments
    } else if position.y < state_bottom {
        &state.snapshot.timeline.state.segments
    } else if position.y <= plot.bottom() {
        &state.snapshot.timeline.application.segments
    } else {
        return None;
    };
    let time = x_to_time(plot, state, position.x);
    band.iter()
        .find(|segment| segment.start <= time && time < segment.end)
}

fn segment_reference(segment: &MockSegment) -> MockSegmentRef {
    MockSegmentRef {
        band: segment.band,
        id: segment.id,
        label: segment.label.clone(),
    }
}

fn selection_matches(selection: Option<&MockSelection>, segment: &MockSegment) -> bool {
    matches!(selection, Some(MockSelection::Segment(selected)) if selected.id == segment.id && selected.band == segment.band)
}

fn format_time(seconds: f32) -> String {
    format!("09:{:02.0}", (seconds / 2.0).round())
}

fn format_range(range: TimeRange) -> String {
    format!("{}–{}", format_time(range.start), format_time(range.end))
}
