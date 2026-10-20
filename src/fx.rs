//! Look and motion in the style of Google's Material 3: colour roles for light and dark,
//! Roboto, switches, segmented buttons, ripples, easing curves and small drawings.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontFamily, FontId, Id, Pos2, Rect, RichText, Sense, Shadow, Stroke,
    StrokeKind, TextStyle, Theme, pos2, vec2,
};
use egui_phosphor::regular as icon;

use crate::config::ThemeChoice;

/// Material colour roles.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub primary: Color32,
    pub on_primary: Color32,
    pub primary_container: Color32,
    pub on_primary_container: Color32,
    pub secondary_container: Color32,
    pub on_secondary_container: Color32,
    /// The window background.
    pub surface: Color32,
    /// Cards.
    pub card: Color32,
    pub container: Color32,
    pub container_high: Color32,
    pub container_highest: Color32,
    pub on_surface: Color32,
    pub on_surface_variant: Color32,
    pub outline: Color32,
    pub outline_variant: Color32,
    pub inverse_surface: Color32,
    pub inverse_on_surface: Color32,
    pub inverse_primary: Color32,
    pub success: Color32,
    pub success_container: Color32,
    pub on_success_container: Color32,
    pub warning: Color32,
    pub warning_container: Color32,
    pub on_warning_container: Color32,
    pub error: Color32,
    pub error_container: Color32,
    pub on_error_container: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

const LIGHT: Palette = Palette {
    primary: hex(0x0B57D0),
    on_primary: hex(0xFFFFFF),
    primary_container: hex(0xD3E3FD),
    on_primary_container: hex(0x041E49),
    secondary_container: hex(0xC2E7FF),
    on_secondary_container: hex(0x001D35),
    surface: hex(0xF8FAFD),
    card: hex(0xFFFFFF),
    container: hex(0xF0F4F9),
    container_high: hex(0xE9EEF6),
    container_highest: hex(0xDDE3EA),
    on_surface: hex(0x1F1F1F),
    on_surface_variant: hex(0x444746),
    outline: hex(0x747775),
    outline_variant: hex(0xC4C7C5),
    inverse_surface: hex(0x303030),
    inverse_on_surface: hex(0xF2F2F2),
    inverse_primary: hex(0xA8C7FA),
    success: hex(0x146C2E),
    success_container: hex(0xC4EED0),
    on_success_container: hex(0x072711),
    warning: hex(0x8F4E00),
    warning_container: hex(0xFFDDB3),
    on_warning_container: hex(0x2C1600),
    error: hex(0xB3261E),
    error_container: hex(0xF9DEDC),
    on_error_container: hex(0x410E0B),
};

const DARK: Palette = Palette {
    primary: hex(0xA8C7FA),
    on_primary: hex(0x062E6F),
    primary_container: hex(0x0842A0),
    on_primary_container: hex(0xD3E3FD),
    secondary_container: hex(0x004A77),
    on_secondary_container: hex(0xC2E7FF),
    surface: hex(0x131314),
    card: hex(0x1E1F20),
    container: hex(0x1E1F20),
    container_high: hex(0x282A2C),
    container_highest: hex(0x333537),
    on_surface: hex(0xE3E3E3),
    on_surface_variant: hex(0xC4C7C5),
    outline: hex(0x8E918F),
    outline_variant: hex(0x444746),
    inverse_surface: hex(0xE3E3E3),
    inverse_on_surface: hex(0x303030),
    inverse_primary: hex(0x0B57D0),
    success: hex(0x6DD58C),
    success_container: hex(0x0F5223),
    on_success_container: hex(0xC4EED0),
    warning: hex(0xFFB95C),
    warning_container: hex(0x653E00),
    on_warning_container: hex(0xFFDDB3),
    error: hex(0xF2B8B5),
    error_container: hex(0x8C1D18),
    on_error_container: hex(0xF9DEDC),
};

pub fn palette(dark_mode: bool) -> Palette {
    if dark_mode { DARK } else { LIGHT }
}

/// The colours of the current theme.
pub fn pal(ui: &egui::Ui) -> Palette {
    palette(ui.visuals().dark_mode)
}

pub fn pal_ctx(ctx: &egui::Context) -> Palette {
    palette(ctx.theme() == Theme::Dark)
}

/// Container and content colours for a study program's avatar (cycling through a few hues).
pub fn avatar_colors(dark_mode: bool, index: usize) -> (Color32, Color32) {
    const LIGHT_SET: [(u32, u32); 6] = [
        (0xD3E3FD, 0x041E49),
        (0xC4EED0, 0x072711),
        (0xFFDF99, 0x251A00),
        (0xF9DEDC, 0x410E0B),
        (0xE8DEF8, 0x1D192B),
        (0xC2E7FF, 0x001D35),
    ];
    const DARK_SET: [(u32, u32); 6] = [
        (0x0842A0, 0xD3E3FD),
        (0x0F5223, 0xC4EED0),
        (0x5C4300, 0xFFDF99),
        (0x8C1D18, 0xF9DEDC),
        (0x4A4458, 0xE8DEF8),
        (0x004A77, 0xC2E7FF),
    ];
    let (bg, fg) = if dark_mode { DARK_SET } else { LIGHT_SET }[index % 6];
    (hex(bg), hex(fg))
}

/// The "medium" weight of Roboto, for titles and buttons.
pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

pub fn medium_font(size: f32) -> FontId {
    FontId::new(size, medium())
}

/// Text in the medium weight.
pub fn title(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).family(medium()).size(size)
}

/// Roboto, the icon font, sizes, spacing and the Material colours for light and dark.
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("roboto".into(), Arc::new(FontData::from_static(include_bytes!("../assets/fonts/Roboto-Regular.ttf"))));
    fonts.font_data.insert(
        "roboto-medium".into(),
        Arc::new(FontData::from_static(include_bytes!("../assets/fonts/Roboto-Medium.ttf"))),
    );
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "roboto".into());
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    let mut medium_fonts = vec!["roboto-medium".to_string()];
    medium_fonts.extend(fonts.families[&FontFamily::Proportional].iter().cloned());
    fonts.families.insert(medium(), medium_fonts);
    ctx.set_fonts(fonts);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = vec2(8.0, 8.0);
        style.spacing.button_padding = vec2(16.0, 7.0);
        style.spacing.interact_size.y = 32.0;
        style.spacing.window_margin = egui::Margin::same(24);
        style.animation_time = 0.18;
        for (text_style, size, family) in [
            (TextStyle::Body, 14.0, FontFamily::Proportional),
            (TextStyle::Button, 14.0, medium()),
            (TextStyle::Heading, 22.0, FontFamily::Proportional),
            (TextStyle::Small, 12.0, FontFamily::Proportional),
            (TextStyle::Monospace, 13.0, FontFamily::Monospace),
        ] {
            style.text_styles.insert(text_style, FontId::new(size, family));
        }
    });
    for (theme, p) in [(Theme::Dark, DARK), (Theme::Light, LIGHT)] {
        ctx.style_mut_of(theme, |style| {
            let v = &mut style.visuals;
            v.panel_fill = p.surface;
            v.window_fill = p.container_high;
            v.window_stroke = Stroke::NONE;
            v.window_corner_radius = CornerRadius::same(28);
            v.menu_corner_radius = CornerRadius::same(8);
            v.window_shadow = Shadow { offset: [0, 4], blur: 24, spread: 0, color: Color32::from_black_alpha(50) };
            v.popup_shadow = Shadow { offset: [0, 2], blur: 12, spread: 0, color: Color32::from_black_alpha(40) };
            v.extreme_bg_color = p.card;
            v.text_edit_bg_color = Some(p.card);
            v.faint_bg_color = p.container;
            v.override_text_color = Some(p.on_surface);
            v.hyperlink_color = p.primary;
            v.warn_fg_color = p.warning;
            v.error_fg_color = p.error;
            v.selection.bg_fill = p.primary_container;
            // The outline of a focused text field, and the text of selected items.
            v.selection.stroke = Stroke::new(2.0, p.primary);
            v.text_cursor.stroke = Stroke::new(2.0, p.primary);
            v.interact_cursor = Some(egui::CursorIcon::PointingHand);

            let w = &mut v.widgets;
            w.noninteractive.bg_fill = p.surface;
            w.noninteractive.weak_bg_fill = p.surface;
            w.noninteractive.bg_stroke = Stroke::new(1.0, p.outline_variant);
            w.noninteractive.fg_stroke = Stroke::new(1.0, p.on_surface);
            for (state, fill) in [
                (&mut w.inactive, p.container_high),
                (&mut w.hovered, p.container_highest),
                (&mut w.active, p.container_highest),
                (&mut w.open, p.container_highest),
            ] {
                state.bg_fill = fill;
                state.weak_bg_fill = fill;
                state.bg_stroke = Stroke::new(1.0, p.outline_variant);
                state.fg_stroke = Stroke::new(1.0, p.on_surface);
                state.expansion = 0.0;
            }
            w.hovered.bg_stroke = Stroke::new(1.0, p.outline);
            for state in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
                state.corner_radius = CornerRadius::same(8);
            }
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

/// Turn animations (flying files, sliding cards, ripples) on or off.
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

// ---------- interaction feedback ----------

const RIPPLE_SECS: f64 = 0.5;

/// A Material ripple: a circle spreading from where `response` was pressed, inside `rect`.
pub fn ripple(ui: &egui::Ui, response: &egui::Response, rect: Rect, color: Color32) {
    let ctx = ui.ctx();
    let id = response.id.with("ripple");
    let now = ctx.input(|i| i.time);
    if response.is_pointer_button_down_on() && ctx.input(|i| i.pointer.any_pressed()) && motion(ctx) {
        let at = ctx.input(|i| i.pointer.interact_pos()).unwrap_or(rect.center());
        ctx.data_mut(|d| d.insert_temp(id, (at, now)));
    }
    let Some((at, start)) = ctx.data(|d| d.get_temp::<(Pos2, f64)>(id)) else { return };
    let t = ((now - start) / RIPPLE_SECS) as f32;
    if t >= 1.0 {
        ctx.data_mut(|d| d.remove::<(Pos2, f64)>(id));
        return;
    }
    let reach = [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()]
        .iter()
        .map(|c| c.distance(at))
        .fold(0.0, f32::max);
    let radius = reach * ease_out_cubic(t.min(1.0) * 1.4);
    let alpha = 0.18 * (1.0 - t).powi(2);
    ui.painter_at(rect).circle_filled(at, radius, color.gamma_multiply(alpha));
    ctx.request_repaint();
}

/// A soft overlay for hovered and pressed items (Material "state layer").
pub fn state_layer(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: Rect,
    corner: impl Into<CornerRadius>,
    color: Color32,
) {
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.12);
    let alpha = 0.08 * hover + if response.is_pointer_button_down_on() { 0.04 } else { 0.0 };
    if alpha > 0.0 {
        ui.painter().rect_filled(rect, corner, color.gamma_multiply(alpha));
    }
}

// ---------- widgets ----------

/// The kinds of Material buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    /// Primary action: solid primary colour.
    Filled,
    /// Secondary action: tinted.
    Tonal,
    /// Low emphasis: text only.
    Text,
}

/// A Material pill button with a ripple. `glyph` may be empty.
pub fn button(ui: &mut egui::Ui, style: ButtonStyle, glyph: &str, text: &str) -> egui::Response {
    let p = pal(ui);
    let (fill, fg, stroke) = match style {
        ButtonStyle::Filled => (p.primary, p.on_primary, Stroke::NONE),
        ButtonStyle::Tonal => (p.secondary_container, p.on_secondary_container, Stroke::NONE),
        ButtonStyle::Text => (Color32::TRANSPARENT, p.primary, Stroke::NONE),
    };
    let enabled = ui.is_enabled();
    let (fill, fg) = if enabled {
        (fill, fg)
    } else {
        let dim = p.on_surface.gamma_multiply(0.38);
        (if fill == Color32::TRANSPARENT { fill } else { p.on_surface.gamma_multiply(0.12) }, dim)
    };
    let label = if glyph.is_empty() { text.to_string() } else { format!("{glyph}  {text}") };
    let galley = ui.painter().layout_no_wrap(label, medium_font(14.0), fg);
    let pad = if style == ButtonStyle::Text { 12.0 } else { 20.0 };
    let size = vec2(galley.size().x + 2.0 * pad, 36.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(18);
        ui.painter().rect(rect, radius, fill, stroke, StrokeKind::Inside);
        if enabled {
            state_layer(ui, &response, rect, radius, fg);
            ripple(ui, &response, rect, fg);
        }
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    }
    if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response }
}

/// A round icon-only button (Material "standard icon button").
pub fn icon_button(ui: &mut egui::Ui, glyph: &str) -> egui::Response {
    let p = pal(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(16);
        state_layer(ui, &response, rect, radius, p.on_surface_variant);
        ripple(ui, &response, rect, p.primary);
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, glyph, FontId::proportional(17.0), p.primary);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A «⋮» button with a menu of the less used actions (glyph, text, whether it destroys
/// something): returns the one picked.
pub fn more_menu(ui: &mut egui::Ui, hint: &str, items: &[(&str, &str, bool)]) -> Option<usize> {
    let p = pal(ui);
    let button = icon_button(ui, icon::DOTS_THREE_VERTICAL).on_hover_text(hint);
    let mut picked = None;
    egui::Popup::menu(&button).show(|ui| {
        ui.set_min_width(200.0);
        for (i, (glyph, text, destroys)) in items.iter().enumerate() {
            let colour = if *destroys { p.error } else { p.on_surface };
            let text = egui::RichText::new(format!("{glyph}   {text}")).color(colour);
            if ui.add(egui::Button::selectable(false, text)).clicked() {
                picked = Some(i);
                ui.close();
            }
        }
    });
    picked
}

/// A Material 3 switch: the thumb grows and shows a check when it is on.
pub fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(52.0, 32.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    let state = *on;
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, state, ""));
    if ui.is_rect_visible(rect) {
        let p = pal(ui);
        let t = ui.ctx().animate_bool_with_time(response.id, *on, anim_time(ui.ctx(), 0.18));
        let painter = ui.painter();
        let track = lerp_color(p.container_highest, p.primary, t);
        let border = lerp_color(p.outline, p.primary, t);
        painter.rect(rect, 16.0, track, Stroke::new(2.0, border), StrokeKind::Inside);
        let pressed = response.is_pointer_button_down_on();
        let radius = if pressed { 14.0 } else { egui::lerp(8.0..=12.0, t) };
        let x = egui::lerp((rect.left() + 16.0)..=(rect.right() - 16.0), t);
        let center = pos2(x, rect.center().y);
        let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.1);
        if hover > 0.0 {
            let halo = lerp_color(p.on_surface, p.primary, t).gamma_multiply(0.08 * hover);
            painter.circle_filled(center, 20.0, halo);
        }
        painter.circle_filled(center, radius, lerp_color(p.outline, p.on_primary, t));
        if t > 0.5 {
            let check = p.primary.gamma_multiply((t - 0.5) * 2.0);
            painter.text(center, egui::Align2::CENTER_CENTER, icon::CHECK, FontId::proportional(14.0), check);
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A Material radio button.
pub fn radio(ui: &mut egui::Ui, selected: bool, id: Id) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
    let p = pal(ui);
    let t = ui.ctx().animate_bool_with_time(id, selected, anim_time(ui.ctx(), 0.15));
    let color = lerp_color(p.on_surface_variant, p.primary, t);
    ui.painter().circle_stroke(rect.center(), 9.0, Stroke::new(2.0, color));
    if t > 0.0 {
        ui.painter().circle_filled(rect.center(), 5.0 * t, color);
    }
    rect
}

/// Material segmented buttons; the chosen segment is tinted and shows a check, and the
/// tint slides to a newly chosen one. Returns true when it changed.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    id_salt: &str,
    value: &mut T,
    options: &[(T, String)],
) -> bool {
    let id = ui.make_persistent_id(id_salt);
    let p = pal(ui);
    let font = medium_font(14.0);
    let check_width = 22.0;
    let galleys: Vec<_> = options
        .iter()
        .map(|(_, label)| ui.painter().layout_no_wrap(label.clone(), font.clone(), Color32::PLACEHOLDER))
        .collect();
    let height = 36.0;
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 28.0 + check_width).collect();
    let (rect, _) = ui.allocate_exact_size(vec2(widths.iter().sum::<f32>(), height), Sense::hover());
    let mut changed = false;
    let mut cells = Vec::new();
    let mut x = rect.left();
    for (i, width) in widths.iter().enumerate() {
        let cell = Rect::from_min_size(pos2(x, rect.top()), vec2(*width, height));
        let response = ui.interact(cell, id.with(i), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() && options[i].0 != *value {
            *value = options[i].0;
            changed = true;
        }
        cells.push((cell, response));
        x += width;
    }
    let selected = options.iter().position(|(v, _)| v == value).unwrap_or(0);
    let target = cells[selected].0;
    let secs = anim_time(ui.ctx(), 0.22);
    let left = ui.ctx().animate_value_with_time(id.with("left"), target.left(), secs);
    let width = ui.ctx().animate_value_with_time(id.with("width"), target.width(), secs);
    let painter = ui.painter();
    let radius = CornerRadius::same((height / 2.0) as u8);
    // The tint, clipped to the pill shape at the ends.
    let tint = Rect::from_min_size(pos2(left, rect.top()), vec2(width, height));
    let first = (tint.left() - rect.left()).abs() < 1.0;
    let last = (tint.right() - rect.right()).abs() < 1.0;
    let r = (height / 2.0) as u8;
    let tint_radius = CornerRadius {
        nw: if first { r } else { 0 },
        sw: if first { r } else { 0 },
        ne: if last { r } else { 0 },
        se: if last { r } else { 0 },
    };
    painter.rect_filled(tint, tint_radius, p.secondary_container);
    for (i, (cell, response)) in cells.iter().enumerate() {
        if i > 0 {
            painter.vline(cell.left(), rect.y_range(), Stroke::new(1.0, p.outline));
        }
        if i != selected {
            state_layer(ui, response, *cell, 0, p.on_surface);
        }
    }
    painter.rect_stroke(rect, radius, Stroke::new(1.0, p.outline), StrokeKind::Inside);
    for (i, ((cell, _), galley)) in cells.iter().zip(galleys).enumerate() {
        let chosen = i == selected;
        let color = if chosen { p.on_secondary_container } else { p.on_surface };
        let total = galley.size().x + if chosen { check_width } else { 0.0 };
        let start = cell.center().x - total / 2.0;
        if chosen {
            painter.text(
                pos2(start + 8.0, cell.center().y),
                egui::Align2::CENTER_CENTER,
                icon::CHECK,
                FontId::proportional(16.0),
                color,
            );
        }
        let at = pos2(start + if chosen { check_width } else { 0.0 }, cell.center().y - galley.size().y / 2.0);
        painter.galley(at, galley, color);
    }
    changed
}

/// A filter chip: tinted with a check when on.
pub fn chip(ui: &mut egui::Ui, on: bool, text: &str) -> egui::Response {
    let p = pal(ui);
    let label = if on { format!("{}  {text}", icon::CHECK) } else { text.to_string() };
    let fg = if on { p.on_secondary_container } else { p.on_surface_variant };
    let galley = ui.painter().layout_no_wrap(label, medium_font(13.0), fg);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 24.0, 32.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(8);
        let fill = if on { p.secondary_container } else { Color32::TRANSPARENT };
        let stroke = if on { Stroke::NONE } else { Stroke::new(1.0, p.outline) };
        ui.painter().rect(rect, radius, fill, stroke, StrokeKind::Inside);
        state_layer(ui, &response, rect, radius, fg);
        ripple(ui, &response, rect, fg);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A filled circle with a letter or icon in it.
pub fn avatar(painter: &egui::Painter, center: Pos2, radius: f32, text: &str, fill: Color32, fg: Color32) {
    painter.circle_filled(center, radius, fill);
    let font = if text.chars().count() == 1 && text.chars().all(char::is_alphanumeric) {
        medium_font(radius)
    } else {
        FontId::proportional(radius * 1.1)
    };
    painter.text(center, egui::Align2::CENTER_CENTER, text, font, fg);
}

/// A Material card: a rounded surface without a border.
pub fn card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    let p = pal(ui);
    egui::Frame::new()
        .fill(p.card)
        .corner_radius(CornerRadius::same(16))
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui);
        })
        .response
}

// ---------- easing and drawing helpers ----------

/// A Material linear progress indicator across the available width: `fraction` of the way
/// (animated), or a segment sliding along while the amount is unknown.
pub fn progress_bar(ui: &mut egui::Ui, id: Id, fraction: Option<f32>) {
    let p = pal(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(2), p.secondary_container);
    let (from, to) = match fraction {
        Some(f) => {
            let shown = ui.ctx().animate_value_with_time(id, f.clamp(0.0, 1.0), anim_time(ui.ctx(), 0.3));
            (0.0, shown)
        }
        None => {
            let t = (ui.input(|i| i.time) % 1.6 / 1.6) as f32;
            ui.ctx().request_repaint();
            let head = ease_in_out_cubic(t) * 1.4 - 0.2;
            ((head - 0.35).max(0.0), head.min(1.0))
        }
    };
    if to > from {
        let bar = Rect::from_min_max(
            pos2(rect.left() + rect.width() * from, rect.top()),
            pos2(rect.left() + rect.width() * to, rect.bottom()),
        );
        painter.rect_filled(bar, CornerRadius::same(2), p.primary);
    }
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

/// A status dot. When `active`, it breathes and sends out rings.
pub fn pulse_dot(painter: &egui::Painter, center: Pos2, color: Color32, active: bool, time: f64) {
    if active {
        let phase = ((time / 1.6) % 1.0) as f32;
        painter.circle_stroke(center, 3.5 + phase * 7.0, Stroke::new(1.5, color.gamma_multiply(1.0 - phase)));
    }
    painter.circle_filled(center, 3.5, color);
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

    #[test]
    fn palettes_have_readable_text() {
        // Contrast between text and its background, roughly (relative luminance ratio).
        fn lum(c: Color32) -> f32 {
            let f = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
        }
        fn contrast(a: Color32, b: Color32) -> f32 {
            let (x, y) = (lum(a), lum(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        }
        for p in [LIGHT, DARK] {
            assert!(contrast(p.on_surface, p.surface) > 7.0);
            assert!(contrast(p.on_primary, p.primary) > 4.5);
            assert!(contrast(p.on_secondary_container, p.secondary_container) > 4.5);
            assert!(contrast(p.inverse_on_surface, p.inverse_surface) > 4.5);
            assert!(contrast(p.on_surface_variant, p.card) > 4.5);
        }
    }
}
