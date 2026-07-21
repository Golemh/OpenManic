//! Shared OpenManic design tokens and paint helpers for the Studio dark theme.
//!
//! Exact values follow the high-fidelity design reference (navy surfaces, purple
//! interaction accent, per-category and per-application brand colors, and the
//! top-dark-to-bottom-light cell gradient rule).

use eframe::egui::{
    self, Color32, CornerRadius, Painter, Rect, Stroke, StrokeKind,
    epaint::{Mesh, Vertex, WHITE_UV},
};

/// Deepest window background.
pub const BG_DEEP: Color32 = Color32::from_rgb(0x02, 0x06, 0x0F);
/// Standard canvas background.
pub const BG_CANVAS: Color32 = Color32::from_rgb(0x05, 0x0A, 0x18);
/// Card / surface background.
pub const SURFACE: Color32 = Color32::from_rgb(0x0A, 0x10, 0x1E);
/// Slightly raised surface (buttons, inputs).
pub const SURFACE_RAISED: Color32 = Color32::from_rgb(0x0E, 0x17, 0x30);
/// Inset panel background (timeline tracks, clocks).
pub const INSET: Color32 = Color32::from_rgb(0x06, 0x0B, 0x18);
/// Progress-track background.
pub const TRACK: Color32 = Color32::from_rgb(0x11, 0x1B, 0x30);
/// Standard card border.
pub const BORDER: Color32 = Color32::from_rgb(0x18, 0x23, 0x39);
/// Highlight border / active pill background.
pub const BORDER_STRONG: Color32 = Color32::from_rgb(0x1B, 0x27, 0x3F);
/// Hairline row separators.
pub const HAIRLINE: Color32 = Color32::from_rgb(0x10, 0x1A, 0x2E);
/// Title-bar background.
pub const TITLEBAR: Color32 = Color32::from_rgb(0x07, 0x0C, 0x1A);
/// Title-bar bottom border.
pub const TITLEBAR_BORDER: Color32 = Color32::from_rgb(0x13, 0x1E, 0x33);

/// Primary text.
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xF3, 0xF6, 0xFF);
/// Secondary text.
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0xDD, 0xE4, 0xF5);
/// Tertiary text.
pub const TEXT_TERTIARY: Color32 = Color32::from_rgb(0xB9, 0xC4, 0xE0);
/// Muted text.
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x7E, 0x8A, 0xA8);
/// Faint text.
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x5F, 0x6B, 0x87);
/// Faintest tick/label text.
pub const TEXT_TICK: Color32 = Color32::from_rgb(0x5A, 0x66, 0x80);
/// Section-header text.
pub const TEXT_SECTION: Color32 = Color32::from_rgb(0x8A, 0x96, 0xC0);

/// Brand interaction accent.
pub const ACCENT: Color32 = Color32::from_rgb(0x67, 0x54, 0xFF);
/// Light accent for gradients.
pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(0x8C, 0x7B, 0xFF);
/// Lightest accent for text on accent surfaces.
pub const ACCENT_TEXT: Color32 = Color32::from_rgb(0xA5, 0xB0, 0xFF);
/// Scheduled/cyan accent.
pub const SCHEDULED: Color32 = Color32::from_rgb(0x22, 0xD3, 0xEE);

/// Active state color.
pub const ACTIVE: Color32 = Color32::from_rgb(0x34, 0xD3, 0x99);
/// Away state color.
pub const AWAY: Color32 = Color32::from_rgb(0xF1, 0x6C, 0x7A);
/// Powered-off state color.
pub const POWERED_OFF: Color32 = Color32::from_rgb(0x47, 0x55, 0x69);
/// Unknown/idle category color.
pub const UNKNOWN: Color32 = Color32::from_rgb(0x3A, 0x47, 0x63);

/// Card corner radius (Studio theme).
pub const RADIUS_CARD: u8 = 11;
/// Button/pill corner radius.
pub const RADIUS_BUTTON: u8 = 6;
/// Chip/input corner radius.
pub const RADIUS_CHIP: u8 = 8;

/// Returns the default color for a category display label.
#[must_use]
pub fn category_color(label: &str) -> Color32 {
    match label.to_ascii_lowercase().as_str() {
        "communication" => Color32::from_rgb(0x3F, 0xC5, 0xC0),
        "development" => Color32::from_rgb(0x5B, 0x8D, 0xEF),
        "web browsing" | "web" => Color32::from_rgb(0x9B, 0x7E, 0xF0),
        "productivity" => Color32::from_rgb(0x4F, 0xB0, 0xF5),
        "gaming" | "entertainment" => Color32::from_rgb(0xEC, 0x6A, 0x9C),
        "design" => Color32::from_rgb(0xC7, 0x7D, 0xEE),
        "ai assistants" | "ai" => Color32::from_rgb(0x7B, 0x8C, 0xF4),
        "utilities" | "utility" | "security & utilities" => Color32::from_rgb(0xE0, 0xA9, 0x6D),
        _ => UNKNOWN,
    }
}

/// Returns the brand color for a known application display name, if any.
#[must_use]
pub fn application_brand_color(name: &str) -> Option<Color32> {
    let lower = name.to_ascii_lowercase();
    let color = if lower.contains("discord") {
        Color32::from_rgb(0x58, 0x65, 0xF2)
    } else if lower.contains("slack") {
        Color32::from_rgb(0xE0, 0x1E, 0x5A)
    } else if lower.contains("codex") {
        Color32::from_rgb(0x10, 0xA3, 0x7F)
    } else if lower.contains("vs code")
        || lower.contains("visual studio code")
        || lower.contains("code")
    {
        Color32::from_rgb(0x3E, 0xA0, 0xE8)
    } else if lower.contains("chrome") {
        Color32::from_rgb(0xF4, 0xB4, 0x00)
    } else if lower.contains("zen") {
        Color32::from_rgb(0xF7, 0x6F, 0x53)
    } else if lower.contains("rize") {
        Color32::from_rgb(0x6C, 0x5C, 0xE7)
    } else if lower.contains("openmanic") {
        Color32::from_rgb(0x8C, 0x7B, 0xFF)
    } else if lower.contains("slay the spire") {
        Color32::from_rgb(0xC0, 0x39, 0x2B)
    } else if lower.contains("steam") {
        Color32::from_rgb(0x66, 0xC0, 0xF4)
    } else if lower.contains("figma") {
        Color32::from_rgb(0xF2, 0x4E, 0x1E)
    } else if lower.contains("claude") {
        Color32::from_rgb(0xD9, 0x77, 0x57)
    } else if lower.contains("explorer") {
        Color32::from_rgb(0xFF, 0xC8, 0x3D)
    } else if lower.contains("terminal") {
        Color32::from_rgb(0x4E, 0xC9, 0xB0)
    } else {
        return None;
    };
    Some(color)
}

/// Shades a color toward black (`amount < 0`) or white (`amount > 0`).
#[must_use]
pub fn shade(color: Color32, amount: f32) -> Color32 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the channel value is clamped into 0..=255 before conversion"
    )]
    fn channel(value: u8, amount: f32) -> u8 {
        let value = f32::from(value);
        let shifted = if amount < 0.0 {
            value * (1.0 + amount)
        } else {
            value + (255.0 - value) * amount
        };
        shifted.clamp(0.0, 255.0).round() as u8
    }
    Color32::from_rgb(
        channel(color.r(), amount),
        channel(color.g(), amount),
        channel(color.b(), amount),
    )
}

/// Paints the standard cell gradient (dark top, light bottom) for a base color.
///
/// The gradient follows the design rule `-26% / -4% @62% / +9%` and is painted
/// as two vertically stacked quads inside `rect`.
pub fn paint_cell_gradient(painter: &Painter, rect: Rect, base: Color32) {
    let top = shade(base, -0.26);
    let mid = shade(base, -0.04);
    let bottom = shade(base, 0.09);
    let mid_y = rect.top() + rect.height() * 0.62;
    let mut mesh = Mesh::default();
    push_gradient_quad(
        &mut mesh,
        Rect::from_min_max(rect.min, egui::pos2(rect.right(), mid_y)),
        top,
        mid,
    );
    push_gradient_quad(
        &mut mesh,
        Rect::from_min_max(egui::pos2(rect.left(), mid_y), rect.max),
        mid,
        bottom,
    );
    painter.add(mesh);
}

/// Paints a left-dark to right-light horizontal accent bar for progress tracks.
pub fn paint_bar_gradient(painter: &Painter, rect: Rect, base: Color32) {
    let left = shade(base, -0.10);
    let right = shade(base, 0.22);
    let mut mesh = Mesh::default();
    let index = u32::try_from(mesh.vertices.len()).unwrap_or(0);
    for (pos, color) in [
        (rect.left_top(), left),
        (rect.right_top(), right),
        (rect.right_bottom(), right),
        (rect.left_bottom(), left),
    ] {
        mesh.vertices.push(Vertex {
            pos,
            uv: WHITE_UV,
            color,
        });
    }
    mesh.indices
        .extend_from_slice(&[index, index + 1, index + 2, index, index + 2, index + 3]);
    painter.add(mesh);
}

fn push_gradient_quad(mesh: &mut Mesh, rect: Rect, top: Color32, bottom: Color32) {
    let index = u32::try_from(mesh.vertices.len()).unwrap_or(0);
    for (pos, color) in [
        (rect.left_top(), top),
        (rect.right_top(), top),
        (rect.right_bottom(), bottom),
        (rect.left_bottom(), bottom),
    ] {
        mesh.vertices.push(Vertex {
            pos,
            uv: WHITE_UV,
            color,
        });
    }
    mesh.indices
        .extend_from_slice(&[index, index + 1, index + 2, index, index + 2, index + 3]);
}

/// Returns the standard card frame (surface fill, border, radius, padding).
#[must_use]
pub fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(RADIUS_CARD))
        .inner_margin(egui::Margin::same(16))
}

/// Returns the inset panel frame used behind timelines and clocks.
#[must_use]
pub fn inset_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(INSET)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(RADIUS_BUTTON))
        .inner_margin(egui::Margin::same(0))
}

/// Renders an uppercase, letter-spaced section header label.
pub fn section_header(ui: &mut egui::Ui, text: &str) {
    let spaced: String = text
        .to_ascii_uppercase()
        .chars()
        .flat_map(|character| [character, '\u{200a}'])
        .collect();
    ui.label(
        egui::RichText::new(spaced)
            .size(11.5)
            .strong()
            .color(TEXT_SECTION),
    );
}

/// Renders a small color swatch dot with a subtle glow.
pub fn color_dot(ui: &mut egui::Ui, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(
        rect.expand(1.5),
        CornerRadius::same(5),
        color.gamma_multiply(0.25),
    );
    painter.rect_filled(rect, CornerRadius::same(4), color);
}

/// Renders a soft (ghost) button and reports whether it was clicked.
#[must_use]
pub fn soft_button(ui: &mut egui::Ui, text: &str) -> bool {
    ui.add(
        egui::Button::new(
            egui::RichText::new(text)
                .size(12.5)
                .strong()
                .color(TEXT_TERTIARY),
        )
        .fill(SURFACE_RAISED)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(RADIUS_BUTTON)),
    )
    .clicked()
}

/// Renders one navigation pill; returns whether it was clicked.
#[must_use]
pub fn nav_pill(ui: &mut egui::Ui, text: &str, selected: bool) -> bool {
    let (fill, text_color) = if selected {
        (BORDER_STRONG, Color32::WHITE)
    } else {
        (Color32::TRANSPARENT, TEXT_MUTED)
    };
    ui.add(
        egui::Button::new(
            egui::RichText::new(text)
                .size(13.5)
                .strong()
                .color(text_color),
        )
        .fill(fill)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(9)),
    )
    .clicked()
}

/// Renders a primary accent button with the brand purple gradient.
#[must_use]
pub fn accent_button(ui: &mut egui::Ui, text: &str) -> bool {
    let response = ui.add(
        egui::Button::new(
            egui::RichText::new(text)
                .size(13.0)
                .strong()
                .color(Color32::WHITE),
        )
        .fill(ACCENT)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(RADIUS_BUTTON)),
    );
    let rect = response.rect;
    let painter = ui.painter();
    painter.rect_stroke(
        rect,
        CornerRadius::same(RADIUS_BUTTON),
        Stroke::new(1.0, ACCENT_LIGHT.gamma_multiply(0.5)),
        StrokeKind::Inside,
    );
    response.clicked()
}

/// Renders one segmented-control option; returns whether it was clicked.
#[must_use]
pub fn segment_option(ui: &mut egui::Ui, text: &str, selected: bool) -> bool {
    let (fill, text_color) = if selected {
        (ACCENT, Color32::WHITE)
    } else {
        (Color32::TRANSPARENT, TEXT_MUTED)
    };
    ui.add(
        egui::Button::new(
            egui::RichText::new(text)
                .size(12.5)
                .strong()
                .color(text_color),
        )
        .fill(fill)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(RADIUS_BUTTON)),
    )
    .clicked()
}

/// Renders a labeled stat chip (small uppercase key over a mono-style value).
pub fn stat_chip(ui: &mut egui::Ui, key: &str, value: &str, accent: bool) {
    let (border, value_color, fill) = if accent {
        (ACCENT, ACCENT_TEXT, ACCENT.gamma_multiply(0.10))
    } else {
        (BORDER, TEXT_SECONDARY, Color32::from_rgb(0x0A, 0x12, 0x24))
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(RADIUS_CHIP))
        .inner_margin(egui::Margin::symmetric(15, 8))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_min_width(66.0);
                ui.label(
                    egui::RichText::new(key.to_ascii_uppercase())
                        .size(10.5)
                        .strong()
                        .color(Color32::from_rgb(0x6B, 0x77, 0x94)),
                );
                ui.label(
                    egui::RichText::new(value)
                        .size(16.0)
                        .monospace()
                        .color(value_color),
                );
            });
        });
}

/// Renders a percentage pill tinted with the row color.
pub fn percent_pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .corner_radius(CornerRadius::same(20))
        .inner_margin(egui::Margin::symmetric(9, 2))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(11.5).strong().color(color));
        });
}

/// Renders a toggle switch bound to `value`; returns whether it changed.
#[must_use]
pub fn toggle_switch(ui: &mut egui::Ui, value: &mut bool) -> bool {
    let size = egui::vec2(46.0, 26.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        *value = !*value;
    }
    let on = *value;
    let painter = ui.painter();
    let fill = if on { ACCENT } else { BORDER_STRONG };
    painter.rect_filled(rect, CornerRadius::same(13), fill);
    let knob_x = if on {
        rect.right() - 13.0
    } else {
        rect.left() + 13.0
    };
    painter.circle_filled(egui::pos2(knob_x, rect.center().y), 10.0, Color32::WHITE);
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::{application_brand_color, category_color, shade};
    use eframe::egui::Color32;

    #[test]
    fn category_labels_resolve_to_design_palette_colors() {
        assert_eq!(
            category_color("Development"),
            Color32::from_rgb(0x5B, 0x8D, 0xEF)
        );
        assert_eq!(
            category_color("security & utilities"),
            Color32::from_rgb(0xE0, 0xA9, 0x6D)
        );
        assert_eq!(category_color("unheard of"), super::UNKNOWN);
    }

    #[test]
    fn shade_darkens_and_lightens_channels_within_range() {
        let base = Color32::from_rgb(100, 150, 200);
        assert_eq!(shade(base, -1.0), Color32::from_rgb(0, 0, 0));
        assert_eq!(shade(base, 1.0), Color32::from_rgb(255, 255, 255));
        assert_eq!(shade(base, 0.0), base);
    }

    #[test]
    fn known_application_names_resolve_brand_colors() {
        assert_eq!(
            application_brand_color("Windows Terminal"),
            Some(Color32::from_rgb(0x4E, 0xC9, 0xB0))
        );
        assert_eq!(application_brand_color("Mystery App"), None);
    }
}
