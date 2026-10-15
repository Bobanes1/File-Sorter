//! Look and motion: theme, icon font, easing curves and small animated drawings.

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind, TextStyle, Theme, Vec2, pos2, vec2,
};

use crate::config::ThemeChoice;

pub const GREEN: Color32 = Color32::from_rgb(34, 197, 94);
pub const BLUE: Color32 = Color32::from_rgb(59, 130, 246);
pub const AMBER: Color32 = Color32::from_rgb(245, 158, 11);
pub const RED: Color32 = Color32::from_rgb(239, 68, 68);
pub const PURPLE: Color32 = Color32::from_rgb(168, 85, 247);
const ACCENT_DARK: Color32 = Color32::from_rgb(45, 212, 191);
const ACCENT_LIGHT: Color32 = Color32::from_rgb(13, 148, 136);

/// The accent colour for the current theme.
pub fn accent(ui: &egui::Ui) -> Color32 {
    accent_for(ui.visuals().dark_mode)
}

pub fn accent_for(dark_mode: bool) -> Color32 {
    if dark_mode { ACCENT_DARK } else { ACCENT_LIGHT }
}

/// Readable text colour on top of the accent colour.
pub fn on_accent(dark_mode: bool) -> Color32 {
    if dark_mode { Color32::from_rgb(4, 47, 46) } else { Color32::WHITE }
}

/// Icon font, larger text, rounded corners, roomier spacing and the accent colour.
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = vec2(8.0, 7.0);
        style.spacing.button_padding = vec2(10.0, 5.0);
        style.spacing.interact_size.y = 26.0;
        style.animation_time = 0.18;
        for (text_style, size) in [
            (TextStyle::Body, 14.0),
            (TextStyle::Button, 14.0),
            (TextStyle::Heading, 20.0),
            (TextStyle::Small, 11.5),
            (TextStyle::Monospace, 13.0),
        ] {
            let family = if text_style == TextStyle::Monospace {
                egui::FontFamily::Monospace
            } else {
                egui::FontFamily::Proportional
            };
            style.text_styles.insert(text_style, FontId::new(size, family));
        }
        let radius = CornerRadius::same(7);
        let w = &mut style.visuals.widgets;
        for widget in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            widget.corner_radius = radius;
        }
        style.visuals.window_corner_radius = CornerRadius::same(14);
        style.visuals.menu_corner_radius = CornerRadius::same(10);
    });
    for (theme, accent) in [(Theme::Dark, ACCENT_DARK), (Theme::Light, ACCENT_LIGHT)] {
        ctx.style_mut_of(theme, |style| {
            style.visuals.selection.bg_fill = accent;
            // Text on the accent: dark on the bright dark-mode teal, white on the deeper light-mode one.
            style.visuals.selection.stroke = Stroke::new(1.0, on_accent(theme == Theme::Dark));
            style.visuals.hyperlink_color = accent;
            style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, accent.gamma_multiply(0.7));
        });
    }
}

/// Light, dark, or follow the system.
pub fn apply_theme(ctx: &egui::Context, choice: ThemeChoice) {
    ctx.set_theme(match choice {
        ThemeChoice::System => egui::ThemePreference::System,
        ThemeChoice::Light => egui::ThemePreference::Light,
        ThemeChoice::Dark => egui::ThemePreference::Dark,
    });
}

fn motion_id() -> Id {
    Id::new("fx-motion")
}

/// Turn animations (flying files, sliding cards, pulsing dot) on or off.
pub fn set_motion(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(motion_id(), on));
    ctx.all_styles_mut(|style| style.animation_time = if on { 0.18 } else { 0.0 });
}

/// Whether animations are on (they are unless turned off).
pub fn motion(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp(motion_id())).unwrap_or(true)
}

/// `secs`, or no time at all when animations are off.
pub fn anim_time(ctx: &egui::Context, secs: f32) -> f32 {
    if motion(ctx) { secs } else { 0.0 }
}

/// An on/off switch whose knob slides over.
pub fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(44.0, 24.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    let state = *on;
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, state, ""));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(response.id, *on, anim_time(ui.ctx(), 0.16));
        let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.1);
        let radius = rect.height() / 2.0;
        let off = ui.visuals().widgets.inactive.bg_fill;
        let fill = lerp_color(off, accent(ui), t);
        let painter = ui.painter();
        painter.rect(
            rect,
            radius,
            fill,
            Stroke::new(1.0, accent(ui).gamma_multiply(0.25 + 0.5 * hover)),
            StrokeKind::Inside,
        );
        let center = pos2(egui::lerp((rect.left() + radius)..=(rect.right() - radius), t), rect.center().y);
        painter.circle_filled(center + vec2(0.0, 1.0), radius - 3.0, Color32::from_black_alpha(40));
        painter.circle_filled(center, radius - 3.5 + hover, Color32::WHITE);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A row of choices with a highlight that slides to the chosen one. Returns true when it changed.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    id_salt: &str,
    value: &mut T,
    options: &[(T, String)],
) -> bool {
    let id = ui.make_persistent_id(id_salt);
    let font = TextStyle::Button.resolve(ui.style());
    let galleys: Vec<_> = options
        .iter()
        .map(|(_, label)| ui.painter().layout_no_wrap(label.clone(), font.clone(), Color32::PLACEHOLDER))
        .collect();
    let height = 30.0;
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 28.0).collect();
    let (rect, _) = ui.allocate_exact_size(vec2(widths.iter().sum::<f32>() + 6.0, height), Sense::hover());
    let mut changed = false;
    let mut cells = Vec::new();
    let mut x = rect.left() + 3.0;
    for (i, width) in widths.iter().enumerate() {
        let cell = Rect::from_min_size(pos2(x, rect.top() + 3.0), vec2(*width, height - 6.0));
        let response = ui.interact(cell, id.with(i), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() && options[i].0 != *value {
            *value = options[i].0;
            changed = true;
        }
        cells.push((cell, response.hovered()));
        x += width;
    }
    let selected = options.iter().position(|(v, _)| v == value).unwrap_or(0);
    let target = cells[selected].0;
    let secs = anim_time(ui.ctx(), 0.22);
    let left = ui.ctx().animate_value_with_time(id.with("left"), target.left(), secs);
    let width = ui.ctx().animate_value_with_time(id.with("width"), target.width(), secs);
    let painter = ui.painter();
    painter.rect_filled(rect, height / 2.0, ui.visuals().extreme_bg_color);
    painter.rect_stroke(rect, height / 2.0, ui.visuals().widgets.noninteractive.bg_stroke, StrokeKind::Inside);
    let pill = Rect::from_min_size(pos2(left, target.top()), vec2(width, target.height()));
    painter.rect_filled(pill, pill.height() / 2.0, accent(ui));
    for (i, ((cell, hovered), galley)) in cells.iter().zip(galleys).enumerate() {
        let color = if i == selected {
            ui.visuals().selection.stroke.color
        } else if *hovered {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color()
        };
        painter.galley(cell.center() - galley.size() / 2.0, galley, color);
    }
    changed
}

pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

/// A slight overshoot at the end, for things that "pop" into place.
pub fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

/// Point on a quadratic Bézier curve.
pub fn bezier(a: Pos2, control: Pos2, b: Pos2, t: f32) -> Pos2 {
    let u = 1.0 - t;
    Pos2::new(u * u * a.x + 2.0 * u * t * control.x + t * t * b.x, u * u * a.y + 2.0 * u * t * control.y + t * t * b.y)
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()), mix(a.a(), b.a()))
}

/// A status dot. When `active`, it breathes and sends out radar rings.
pub fn pulse_dot(painter: &egui::Painter, center: Pos2, color: Color32, active: bool, time: f64) {
    if active {
        for offset in [0.0, 0.5] {
            let phase = ((time / 1.6 + offset) % 1.0) as f32;
            let radius = 4.5 + phase * 10.0;
            painter.circle_stroke(center, radius, Stroke::new(1.6, color.gamma_multiply(1.0 - phase)));
        }
        let breathe = 4.5 + 0.7 * ((time * 3.0).sin() as f32);
        painter.circle_filled(center, breathe, color);
    } else {
        painter.circle_filled(center, 4.5, color);
    }
}

/// Size of a text in the given font, for laying out painted things.
pub fn text_size(ui: &egui::Ui, text: &str, font: FontId) -> Vec2 {
    ui.painter().layout_no_wrap(text.to_owned(), font, Color32::WHITE).size()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_hits_the_ends() {
        for f in [ease_out_cubic, ease_in_out_cubic, ease_out_back] {
            assert!((f(0.0)).abs() < 1e-4);
            assert!((f(1.0) - 1.0).abs() < 1e-4);
        }
        assert!(ease_out_back(0.8) > 1.0, "overshoots before settling");
    }

    #[test]
    fn bezier_and_colours() {
        let (a, c, b) = (Pos2::new(0.0, 0.0), Pos2::new(50.0, -100.0), Pos2::new(100.0, 0.0));
        assert_eq!(bezier(a, c, b, 0.0), a);
        assert_eq!(bezier(a, c, b, 1.0), b);
        assert!(bezier(a, c, b, 0.5).y < 0.0, "the curve arcs upwards");
        assert_eq!(lerp_color(Color32::BLACK, Color32::WHITE, 0.0), Color32::BLACK);
        assert_eq!(lerp_color(Color32::BLACK, Color32::WHITE, 1.0), Color32::WHITE);
    }
}
