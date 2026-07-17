use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Pos2,
    Rect, RichText, Sense, Stroke, StrokeKind, Vec2,
};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("OpenManic visual spike")
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([680.0, 520.0]),
        ..Default::default()
    };

    eframe::run_native(
        "OpenManic visual spike",
        options,
        Box::new(|cc| Ok(Box::new(OpenManicSpike::new(cc)))),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Route {
    Today,
    Overview,
    Categories,
    Calendar,
}

#[derive(Clone, Copy)]
struct Tokens {
    canvas: Color32,
    card: Color32,
    card_alt: Color32,
    border: Color32,
    text: Color32,
    muted: Color32,
    primary: Color32,
    primary_soft: Color32,
    active: Color32,
    away: Color32,
    offline: Color32,
}

impl Tokens {
    fn dark() -> Self {
        Self {
            canvas: Color32::from_rgb(15, 15, 20),
            card: Color32::from_rgb(23, 22, 30),
            card_alt: Color32::from_rgb(30, 29, 39),
            border: Color32::from_rgb(48, 46, 61),
            text: Color32::from_rgb(239, 237, 246),
            muted: Color32::from_rgb(160, 156, 174),
            primary: Color32::from_rgb(126, 92, 218),
            primary_soft: Color32::from_rgb(74, 57, 122),
            active: Color32::from_rgb(91, 196, 132),
            away: Color32::from_rgb(234, 171, 72),
            offline: Color32::from_rgb(103, 197, 210),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WidgetKind {
    Timeline,
    Pomodoro,
    Distribution,
    TopApps,
}

struct WidgetInstance {
    id: &'static str,
    kind: WidgetKind,
    span: usize,
}

struct OpenManicSpike {
    route: Route,
    tokens: Tokens,
    edit_layout: bool,
    selected_segment: Option<usize>,
    widgets: Vec<WidgetInstance>,
    pomodoro_running: bool,
    pomodoro_remaining: Duration,
    last_tick: Instant,
    category_query: String,
}

impl OpenManicSpike {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let tokens = Tokens::dark();
        apply_theme(&cc.egui_ctx, tokens);
        Self {
            route: Route::Today,
            tokens,
            edit_layout: false,
            selected_segment: None,
            widgets: vec![
                WidgetInstance {
                    id: "timeline",
                    kind: WidgetKind::Timeline,
                    span: 8,
                },
                WidgetInstance {
                    id: "pomodoro",
                    kind: WidgetKind::Pomodoro,
                    span: 4,
                },
                WidgetInstance {
                    id: "distribution",
                    kind: WidgetKind::Distribution,
                    span: 4,
                },
                WidgetInstance {
                    id: "top-apps",
                    kind: WidgetKind::TopApps,
                    span: 8,
                },
            ],
            pomodoro_running: false,
            pomodoro_remaining: Duration::from_secs(25 * 60),
            last_tick: Instant::now(),
            category_query: String::new(),
        }
    }

    fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if self.pomodoro_running {
            let elapsed = now.saturating_duration_since(self.last_tick);
            self.pomodoro_remaining = self.pomodoro_remaining.saturating_sub(elapsed);
            if self.pomodoro_remaining.is_zero() {
                self.pomodoro_running = false;
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        self.last_tick = now;
    }

    fn shell(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("OpenManic").color(self.tokens.text));
            ui.add_space(18.0);
            nav_button(ui, "Today", &mut self.route, Route::Today);
            nav_button(ui, "Overview", &mut self.route, Route::Overview);
            nav_button(ui, "Categories", &mut self.route, Route::Categories);
            nav_button(ui, "Calendar", &mut self.route, Route::Calendar);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.toggle_value(&mut self.edit_layout, "Edit layout");
            });
        });
        ui.add_space(12.0);

        match self.route {
            Route::Today => self.today(ui),
            Route::Overview => self.overview(ui),
            Route::Categories => self.categories(ui),
            Route::Calendar => self.calendar(ui),
        }
    }

    fn today(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Today");
            ui.label(RichText::new("Saturday, July 18, 2026").color(self.tokens.muted));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let _ = ui.button("Next ›");
                let _ = ui.button("‹ Previous");
            });
        });
        ui.add_space(8.0);

        let columns = if ui.available_width() < 860.0 { 1 } else { 12 };
        let mut index = 0;
        while index < self.widgets.len() {
            if columns == 1 {
                let kind = self.widgets[index].kind;
                self.widget_card(ui, index, kind, ui.available_width());
                index += 1;
                ui.add_space(10.0);
                continue;
            }

            let mut used = 0usize;
            let row_start = index;
            while index < self.widgets.len() {
                let span = self.widgets[index].span.clamp(4, 12);
                if used > 0 && used + span > 12 {
                    break;
                }
                used += span;
                index += 1;
                if used == 12 {
                    break;
                }
            }

            let gap = 10.0;
            let count = index - row_start;
            let available = ui.available_width() - gap * count.saturating_sub(1) as f32;
            ui.horizontal(|ui| {
                for widget_index in row_start..index {
                    let span = self.widgets[widget_index].span;
                    let width = available * span as f32 / used as f32;
                    let kind = self.widgets[widget_index].kind;
                    self.widget_card(ui, widget_index, kind, width);
                }
            });
            ui.add_space(gap);
        }
    }

    fn widget_card(&mut self, ui: &mut egui::Ui, index: usize, kind: WidgetKind, width: f32) {
        let height = match kind {
            WidgetKind::Timeline | WidgetKind::Pomodoro => 286.0,
            WidgetKind::Distribution | WidgetKind::TopApps => 238.0,
        };
        let frame = Frame::new()
            .fill(self.tokens.card)
            .stroke(Stroke::new(1.0, self.tokens.border))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::same(14));

        ui.allocate_ui_with_layout(
            Vec2::new(width, height),
            Layout::top_down(Align::Min),
            |ui| {
                frame.show(ui, |ui| {
                    ui.set_min_size(Vec2::new((width - 2.0).max(120.0), height - 2.0));
                    ui.horizontal(|ui| {
                        ui.strong(widget_title(kind));
                        ui.label(
                            RichText::new(self.widgets[index].id)
                                .small()
                                .color(self.tokens.muted),
                        );
                        if self.edit_layout {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.small_button("Wider").clicked() {
                                    self.widgets[index].span =
                                        (self.widgets[index].span + 4).min(12);
                                }
                                if ui.small_button("Narrower").clicked() {
                                    self.widgets[index].span =
                                        self.widgets[index].span.saturating_sub(4).max(4);
                                }
                            });
                        }
                    });
                    ui.separator();
                    match kind {
                        WidgetKind::Timeline => self.timeline(ui),
                        WidgetKind::Pomodoro => self.pomodoro(ui),
                        WidgetKind::Distribution => self.distribution(ui),
                        WidgetKind::TopApps => self.top_apps(ui),
                    }
                });
            },
        );
    }

    fn timeline(&mut self, ui: &mut egui::Ui) {
        let desired = Vec2::new(ui.available_width(), 205.0);
        let (response, painter) = ui.allocate_painter(desired, Sense::click());
        let rect = response.rect.shrink2(Vec2::new(8.0, 10.0));
        let plot = Rect::from_min_max(
            Pos2::new(rect.left() + 34.0, rect.top() + 14.0),
            Pos2::new(rect.right() - 4.0, rect.bottom() - 28.0),
        );

        for hour in 0..=8 {
            let x = egui::lerp(plot.x_range(), hour as f32 / 8.0);
            painter.line_segment(
                [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
                Stroke::new(1.0, self.tokens.border),
            );
            painter.text(
                Pos2::new(x, plot.bottom() + 8.0),
                Align2::CENTER_TOP,
                format!("{}:00", hour + 9),
                FontId::proportional(11.0),
                self.tokens.muted,
            );
        }

        let segments = [
            (0.02, 0.17, 0),
            (0.18, 0.29, 1),
            (0.30, 0.43, 0),
            (0.46, 0.54, 2),
            (0.55, 0.72, 0),
            (0.75, 0.84, 1),
            (0.85, 0.98, 0),
        ];
        for (i, (start, end, app)) in segments.iter().copied().enumerate() {
            let x1 = egui::lerp(plot.x_range(), start);
            let x2 = egui::lerp(plot.x_range(), end);
            let segment = Rect::from_min_max(
                Pos2::new(x1, plot.top() + 18.0),
                Pos2::new(x2, plot.bottom() - 22.0),
            );
            let base = match app {
                0 => self.tokens.primary,
                1 => Color32::from_rgb(85, 156, 222),
                _ => Color32::from_rgb(205, 112, 178),
            };
            let color = if self.selected_segment == Some(i) {
                base
            } else {
                base.gamma_multiply(0.72)
            };
            painter.rect_filled(segment, CornerRadius::same(3), color);
            painter.rect_stroke(
                segment,
                CornerRadius::same(3),
                Stroke::new(1.0, base),
                StrokeKind::Inside,
            );
        }

        let y = plot.bottom() - 12.0;
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(2.0, self.tokens.active),
        );

        if response.clicked() && let Some(pointer) = response.interact_pointer_pos() {
            let fraction = ((pointer.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
            self.selected_segment = segments
                .iter()
                .position(|(start, end, _)| fraction >= *start && fraction <= *end);
        }

        if let Some(index) = self.selected_segment {
            response.on_hover_text(format!(
                "Selected activity segment {} — click empty space to clear",
                index + 1
            ));
        }
    }

    fn pomodoro(&mut self, ui: &mut egui::Ui) {
        let seconds = self.pomodoro_remaining.as_secs();
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            ui.label(RichText::new("FOCUS").color(self.tokens.muted));
            ui.heading(format!("{:02}:{:02}", seconds / 60, seconds % 60));
            ui.label("Planned · 2:30 PM–2:55 PM");
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui
                    .button(if self.pomodoro_running {
                        "Pause"
                    } else {
                        "Start"
                    })
                    .clicked()
                {
                    self.pomodoro_running = !self.pomodoro_running;
                    self.last_tick = Instant::now();
                }
                if ui.button("Reset").clicked() {
                    self.pomodoro_running = false;
                    self.pomodoro_remaining = Duration::from_secs(25 * 60);
                }
            });
            ui.add_space(10.0);
            ui.label(RichText::new("Linked category: Productive").color(self.tokens.muted));
        });
    }

    fn distribution(&self, ui: &mut egui::Ui) {
        let (response, painter) =
            ui.allocate_painter(Vec2::new(ui.available_width(), 150.0), Sense::hover());
        let center = response.rect.center();
        painter.circle_stroke(center, 55.0, Stroke::new(15.0, self.tokens.card_alt));
        painter.circle_stroke(center, 55.0, Stroke::new(12.0, self.tokens.primary));
        painter.text(
            center,
            Align2::CENTER_CENTER,
            "4h 33m",
            FontId::proportional(18.0),
            self.tokens.text,
        );
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(self.tokens.primary, "● Productive 61%");
            ui.colored_label(self.tokens.away, "● Entertainment 27%");
            ui.colored_label(self.tokens.offline, "● Other 12%");
        });
    }

    fn top_apps(&self, ui: &mut egui::Ui) {
        for (name, duration, ratio, color) in [
            ("Discord", "1h 45m", 0.39, self.tokens.primary),
            (
                "Slay the Spire 2",
                "1h 25m",
                0.31,
                Color32::from_rgb(85, 156, 222),
            ),
            (
                "TopHatch Concepts",
                "23m",
                0.09,
                Color32::from_rgb(205, 112, 178),
            ),
            ("Zen Browser", "20m", 0.08, self.tokens.active),
        ] {
            ui.horizontal(|ui| {
                ui.label(name);
                let bar_width = (ui.available_width() - 96.0).max(30.0);
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(bar_width, 8.0), Sense::hover());
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(4), self.tokens.card_alt);
                let filled = Rect::from_min_size(
                    rect.min,
                    Vec2::new(rect.width() * ratio, rect.height()),
                );
                ui.painter()
                    .rect_filled(filled, CornerRadius::same(4), color);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(duration);
                });
            });
            ui.add_space(7.0);
        }
    }

    fn overview(&self, ui: &mut egui::Ui) {
        screen_header(ui, "Overview", "Week 29 · July 13–19, 2026");
        ui.horizontal(|ui| {
            ui.selectable_label(false, "Day");
            ui.selectable_label(true, "Week");
            ui.selectable_label(false, "Month");
            let _ = ui.button("Custom…");
        });
        card(ui, self.tokens, |ui| {
            ui.strong("Tracked hours");
            ui.add_space(12.0);
            let (response, painter) =
                ui.allocate_painter(Vec2::new(ui.available_width(), 260.0), Sense::hover());
            let plot = response.rect.shrink2(Vec2::new(26.0, 20.0));
            let values = [2.4, 5.1, 3.7, 6.2, 4.55, 1.8, 0.0];
            let labels = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
            for (i, value) in values.into_iter().enumerate() {
                let cell = plot.width() / 7.0;
                let bar = Rect::from_min_max(
                    Pos2::new(
                        plot.left() + cell * i as f32 + cell * 0.2,
                        plot.bottom() - plot.height() * value / 7.0,
                    ),
                    Pos2::new(
                        plot.left() + cell * (i + 1) as f32 - cell * 0.2,
                        plot.bottom(),
                    ),
                );
                painter.rect_filled(bar, CornerRadius::same(3), self.tokens.primary);
                painter.text(
                    Pos2::new(bar.center().x, plot.bottom() + 8.0),
                    Align2::CENTER_TOP,
                    labels[i],
                    FontId::proportional(11.0),
                    self.tokens.muted,
                );
            }
        });
    }

    fn categories(&mut self, ui: &mut egui::Ui) {
        screen_header(ui, "Categories", "Assign apps individually or in bulk");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.category_query)
                    .hint_text("Search applications"),
            );
            egui::ComboBox::from_id_salt("category-filter")
                .selected_text("All categories")
                .show_ui(ui, |ui| {
                    ui.selectable_label(true, "All categories");
                    ui.selectable_label(false, "Uncategorized");
                    ui.selectable_label(false, "Productive");
                });
            let _ = ui.button("Assign category…");
        });
        ui.add_space(12.0);
        card(ui, self.tokens, |ui| {
            for (name, category, color) in [
                ("Discord", "Communication", self.tokens.primary),
                ("Slay the Spire 2", "Entertainment", self.tokens.away),
                ("TopHatch Concepts", "Creative", self.tokens.offline),
                ("Zen Browser", "Uncategorized", self.tokens.muted),
            ] {
                if !self.category_query.is_empty()
                    && !name
                        .to_lowercase()
                        .contains(&self.category_query.to_lowercase())
                {
                    continue;
                }
                ui.horizontal(|ui| {
                    let mut selected = false;
                    ui.checkbox(&mut selected, "");
                    ui.strong(name);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let _ = ui.button("Change…");
                        ui.colored_label(color, format!("● {category}"));
                    });
                });
                ui.separator();
            }
        });
    }

    fn calendar(&self, ui: &mut egui::Ui) {
        screen_header(ui, "Calendar", "Saturday, July 18, 2026");
        card(ui, self.tokens, |ui| {
            let (response, painter) =
                ui.allocate_painter(Vec2::new(ui.available_width(), 520.0), Sense::hover());
            let plot = response.rect.shrink2(Vec2::new(50.0, 12.0));
            for hour in 0..=8 {
                let y = egui::lerp(plot.y_range(), hour as f32 / 8.0);
                painter.line_segment(
                    [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                    Stroke::new(1.0, self.tokens.border),
                );
                painter.text(
                    Pos2::new(plot.left() - 8.0, y),
                    Align2::RIGHT_CENTER,
                    format!("{}:00", hour + 9),
                    FontId::proportional(11.0),
                    self.tokens.muted,
                );
            }
            for (start, end, label, color) in [
                (
                    0.18,
                    0.40,
                    "Project planning · Concepts",
                    self.tokens.primary,
                ),
                (
                    0.41,
                    0.50,
                    "Team communication · Discord",
                    Color32::from_rgb(85, 156, 222),
                ),
                (
                    0.57,
                    0.72,
                    "Focus session · Productive",
                    self.tokens.active,
                ),
                (
                    0.74,
                    0.93,
                    "Creative work · Concepts",
                    self.tokens.primary,
                ),
            ] {
                let block = Rect::from_min_max(
                    Pos2::new(plot.left() + 8.0, egui::lerp(plot.y_range(), start)),
                    Pos2::new(plot.right() - 8.0, egui::lerp(plot.y_range(), end)),
                );
                painter.rect_filled(block, CornerRadius::same(5), color.gamma_multiply(0.78));
                painter.text(
                    block.left_top() + Vec2::new(9.0, 8.0),
                    Align2::LEFT_TOP,
                    label,
                    FontId::proportional(13.0),
                    self.tokens.text,
                );
            }
        });
    }
}

impl eframe::App for OpenManicSpike {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.tick(ui.ctx());
        egui::CentralPanel::default()
            .frame(
                Frame::central_panel(&ui.style())
                    .fill(self.tokens.canvas)
                    .inner_margin(Margin::same(18)),
            )
            .show_inside(ui, |ui| self.shell(ui));
    }
}

fn apply_theme(ctx: &egui::Context, tokens: Tokens) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.visuals.panel_fill = tokens.canvas;
    style.visuals.window_fill = tokens.card;
    style.visuals.override_text_color = Some(tokens.text);
    style.visuals.widgets.inactive.bg_fill = tokens.card_alt;
    style.visuals.widgets.hovered.bg_fill = tokens.primary_soft;
    style.visuals.widgets.active.bg_fill = tokens.primary;
    style.visuals.selection.bg_fill = tokens.primary;
    ctx.set_style(style);
}

fn nav_button(ui: &mut egui::Ui, label: &str, route: &mut Route, target: Route) {
    if ui.selectable_label(*route == target, label).clicked() {
        *route = target;
    }
}

fn widget_title(kind: WidgetKind) -> &'static str {
    match kind {
        WidgetKind::Timeline => "Activity timeline",
        WidgetKind::Pomodoro => "Pomodoro",
        WidgetKind::Distribution => "Time distribution",
        WidgetKind::TopApps => "Top applications",
    }
}

fn screen_header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        ui.heading(title);
        ui.label(subtitle);
    });
    ui.add_space(10.0);
}

fn card(ui: &mut egui::Ui, tokens: Tokens, content: impl FnOnce(&mut egui::Ui)) {
    Frame::new()
        .fill(tokens.card)
        .stroke(Stroke::new(1.0, tokens.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(14))
        .show(ui, content);
}
