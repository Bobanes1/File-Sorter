//! The overview tab: statistics, folders, modules with keywords, start/stop and the
//! activity list.

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Id, LayerId, Layout, Margin, Order, Pos2, Rect, RichText,
    Sense, Stroke, StrokeKind, vec2,
};
use egui_phosphor::regular as icon;

use super::cards::{self, RowAction};
use super::{App, CourseDialog, FLIGHT_SECS, GLOW_SECS};
use crate::fx::{self, ButtonStyle};

impl App {
    pub(super) fn overview_ui(&mut self, ui: &mut egui::Ui, fade: f32) {
        let p = fx::pal(ui);
        egui::Panel::bottom("activity")
            .resizable(true)
            .default_size(230.0)
            .min_size(130.0)
            .frame(egui::Frame::new().fill(p.surface).inner_margin(Margin { left: 20, right: 20, top: 4, bottom: 8 }))
            .show(ui, |ui| {
                ui.set_opacity(fade);
                self.activity_ui(ui);
            });
        // Directly above the activity list, so the buttons are always visible.
        egui::Panel::bottom("controls")
            .frame(egui::Frame::new().fill(p.surface).inner_margin(Margin { left: 20, right: 20, top: 10, bottom: 10 }))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.set_opacity(fade);
                self.controls_ui(ui);
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.surface).inner_margin(Margin { left: 20, right: 20, top: 4, bottom: 0 }))
            .show(ui, |ui| {
                ui.set_opacity(fade);
                egui::ScrollArea::vertical().id_salt("main").auto_shrink([false, false]).show(ui, |ui| {
                    ui.add_space(8.0);
                    fx::card(ui, |ui| self.folders_ui(ui));
                    ui.add_space(12.0);
                    fx::card(ui, |ui| self.modules_ui(ui));
                    ui.add_space(8.0);
                });
            });
    }

    /// Three quiet numbers that count up.
    fn stats_ui(&mut self, ui: &mut egui::Ui) {
        self.roll_day();
        let p = fx::pal(ui);
        let lang = self.lang;
        let now = self.now();
        let values = [self.stats.sorted, self.waiting_count(), self.stats.unmatched];
        for (i, value) in values.iter().enumerate() {
            if *value > self.stat_last[i] {
                self.stat_bumped[i] = now;
            }
            self.stat_last[i] = *value;
        }
        let items = [
            (p.success, lang.stat_sorted_today()),
            (p.primary, lang.stat_waiting()),
            (p.warning, lang.stat_unmatched()),
        ];
        // Right to left: the last one first.
        for (i, ((color, label), value)) in items.into_iter().zip(values).enumerate().rev() {
            let shown = self.ctx.animate_value_with_time(Id::new(("stat", i)), value as f32, 0.8);
            let pop = (1.0 - (now - self.stat_bumped[i]) / 0.6).clamp(0.0, 1.0) as f32;
            ui.label(RichText::new(label).color(p.on_surface_variant));
            let size = 18.0 + 5.0 * fx::ease_out_cubic(pop);
            ui.label(fx::title(format!("{}", shown.round() as i64), size).color(color));
            ui.add_space(14.0);
        }
    }

    fn folders_ui(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        let lang = self.lang;
        ui.horizontal(|ui| {
            ui.label(fx::title(lang.folders(), 16.0).color(p.on_surface));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let import = fx::button(ui, ButtonStyle::Text, icon::FOLDERS, lang.import_modules());
                if import.on_hover_text(lang.layout_hint()).clicked() {
                    self.import_modules(false);
                }
            });
        });

        /// Label, field and a "Change" button on one line. Returns the field and the folder picked, if any.
        fn folder_row(ui: &mut egui::Ui, label: &str, change: &str, value: &mut String) -> (Rect, Option<String>) {
            const LABEL_WIDTH: f32 = 236.0;
            let p = fx::pal(ui);
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(vec2(LABEL_WIDTH, 36.0), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_min_width(LABEL_WIDTH);
                    ui.label(RichText::new(label).color(p.on_surface_variant));
                });
                let button_width = fx::text_size(ui, change, fx::medium_font(14.0)).x + 40.0 + 28.0;
                let width = ui.available_width() - button_width - ui.spacing().item_spacing.x;
                let field = ui
                    .add(
                        egui::TextEdit::singleline(value)
                            .desired_width(width)
                            .margin(Margin::symmetric(12, 8))
                            .font(egui::TextStyle::Body),
                    )
                    .rect;
                let picked = fx::button(ui, ButtonStyle::Tonal, icon::FOLDER_OPEN, change)
                    .clicked()
                    .then(|| App::pick_folder(value))
                    .flatten();
                (field, picked)
            })
            .inner
        }

        let (field, picked) = folder_row(ui, lang.field_watch(), lang.change(), &mut self.config.watch_folder);
        self.watch_field_rect = Some(field);
        if let Some(folder) = picked {
            self.config.watch_folder = folder;
        }
        let (study, picked) = folder_row(ui, lang.field_study(), lang.change(), &mut self.config.destination_root);
        // Where the study folder is connected to (opens the settings).
        if let Some(chip) = self.cloud_chip() {
            let glyph = if self.cloud.moving() { icon::CLOUD_ARROW_UP } else { icon::CLOUD_CHECK };
            let text = RichText::new(format!("{glyph}  {chip}")).small().color(p.primary);
            let rect = Rect::from_min_size(egui::pos2(study.left() + 12.0, study.bottom() + 2.0), vec2(study.width(), 16.0));
            let link = ui.put(rect, egui::Label::new(text).sense(Sense::click()).halign(Align::Min));
            if link.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.switch_tab(super::Tab::Settings);
            }
            ui.add_space(14.0);
        }
        if let Some(folder) = picked {
            self.config.destination_root = folder;
            if self.config.courses.is_empty() {
                self.import_modules(true);
            }
        }
    }

    fn modules_ui(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        let lang = self.lang;
        ui.label(fx::title(lang.modules(), 16.0).color(p.on_surface));
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(330.0);
                egui::ScrollArea::vertical()
                    .id_salt("modules")
                    .max_height(230.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.module_rows(ui));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if fx::button(ui, ButtonStyle::Tonal, icon::PLUS, lang.add()).clicked() {
                        let subject = self.subjects().into_iter().next().unwrap_or_default();
                        self.dialog =
                            Some(CourseDialog { editing: None, subject, name: String::new(), focused: false });
                    }
                    let has_selection = self.selected.is_some();
                    ui.add_enabled_ui(has_selection, |ui| {
                        if fx::button(ui, ButtonStyle::Text, icon::PENCIL_SIMPLE, lang.edit()).clicked()
                            && let Some(i) = self.selected
                        {
                            self.open_edit(i);
                        }
                        if fx::button(ui, ButtonStyle::Text, icon::TRASH, lang.delete()).clicked() {
                            self.confirm_delete = self.selected;
                        }
                    });
                });
            });
            ui.add_space(12.0);
            ui.vertical(|ui| self.keywords_ui(ui));
        });
    }

    /// The module list: avatar per study program, name, study program, counter and glow.
    fn module_rows(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        let dark = ui.visuals().dark_mode;
        let lang = self.lang;
        self.list_clip = Some(ui.clip_rect());
        self.row_rects.clear();
        if self.config.courses.is_empty() {
            ui.add_space(16.0);
            ui.label(RichText::new(lang.no_modules_yet()).color(p.on_surface_variant));
            return;
        }
        let now = self.now();
        let subjects: Vec<String> = {
            let mut s: Vec<String> = self.config.courses.iter().map(|c| c.subject.clone()).collect();
            s.dedup();
            s
        };
        ui.spacing_mut().item_spacing.y = 2.0;
        let mut clicked = None;
        let mut double_clicked = None;
        for (i, course) in self.config.courses.iter().enumerate() {
            let label = course.label();
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
            self.row_rects.insert(label.clone(), rect);
            if response.clicked() {
                clicked = Some(i);
            }
            if response.double_clicked() {
                double_clicked = Some(i);
            }
            let selected = self.selected == Some(i);
            let row = rect.shrink2(vec2(0.0, 1.0));
            let radius = CornerRadius::same(26);
            let painter = ui.painter();
            if selected {
                painter.rect_filled(row, radius, p.secondary_container);
            }
            fx::state_layer(ui, &response, row, radius, p.on_surface);
            fx::ripple(ui, &response, row, p.primary);

            // A file just arrived here: glow, and a "+1" floating up.
            if let Some(&t0) = self.glows.get(&label) {
                let t = ((now - t0) / GLOW_SECS).clamp(0.0, 1.0) as f32;
                let glow = 1.0 - fx::ease_out_cubic(t);
                painter.rect_filled(row, radius, p.primary_container.gamma_multiply(0.9 * glow));
                painter.rect_stroke(row, radius, Stroke::new(2.0, p.primary.gamma_multiply(glow)), StrokeKind::Inside);
                painter.text(
                    rect.right_center() - vec2(64.0, 2.0 + 16.0 * t),
                    Align2::CENTER_CENTER,
                    "+1",
                    fx::medium_font(16.0),
                    p.primary.gamma_multiply(glow.sqrt()),
                );
            }

            let subject_index = subjects.iter().position(|s| *s == course.subject).unwrap_or(0);
            let (fill, fg) = fx::avatar_colors(dark, subject_index);
            let letter: String =
                course.name.chars().find(|c| c.is_alphanumeric()).unwrap_or('?').to_uppercase().collect();
            fx::avatar(painter, rect.left_center() + vec2(26.0, 0.0), 18.0, &letter, fill, fg);
            let text_x = rect.left() + 56.0;
            let name_color = if selected { p.on_secondary_container } else { p.on_surface };
            let sub_color = if selected { p.on_secondary_container } else { p.on_surface_variant };
            let max_width = rect.width() - 56.0 - 52.0;
            let name = painter.layout(course.name.clone(), FontId::new(14.0, fx::medium()), name_color, max_width);
            let has_subject = !course.subject.is_empty();
            let name_y = if has_subject { rect.center().y - 17.0 } else { rect.center().y - name.size().y / 2.0 };
            painter.galley(Pos2::new(text_x, name_y), name, name_color);
            if has_subject {
                painter.text(
                    Pos2::new(text_x, rect.center().y + 9.0),
                    Align2::LEFT_CENTER,
                    &course.subject,
                    FontId::proportional(12.0),
                    sub_color,
                );
            }
            if let Some(count) = self.module_counts.get(&label).filter(|c| **c > 0) {
                cards::badge(
                    painter,
                    rect.right_center() - vec2(24.0, 0.0),
                    &count.to_string(),
                    p.primary,
                    p.on_primary,
                );
            }
            response.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        if let Some(i) = clicked {
            self.select(Some(i));
        }
        if let Some(i) = double_clicked {
            self.open_edit(i);
        }
    }

    fn keywords_ui(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        let lang = self.lang;
        let enabled = self.selected.is_some();
        ui.label(RichText::new(lang.keywords_title()).family(fx::medium()).color(p.on_surface));
        ui.label(RichText::new(lang.keywords_hint()).small().color(p.on_surface_variant));
        const VISIBLE_ROWS: usize = 7;
        // Fixed height: long keyword lists scroll instead of pushing everything below out of view.
        let row_height = ui.text_style_height(&egui::TextStyle::Body);
        let changed = egui::ScrollArea::vertical()
            .id_salt("keywords")
            .max_height(row_height * VISIBLE_ROWS as f32 + 20.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let hint = if enabled { "prog1\nINF-101\nProf. Müller" } else { lang.select_module() };
                let edit = egui::TextEdit::multiline(&mut self.keywords_text)
                    .desired_rows(VISIBLE_ROWS)
                    .desired_width(f32::INFINITY)
                    .margin(Margin::symmetric(12, 8))
                    .hint_text(hint);
                ui.add_enabled(enabled, edit).changed()
            })
            .inner;
        if changed && let Some(i) = self.selected {
            self.config.courses[i].keywords = self
                .keywords_text
                .lines()
                .flat_map(|line| line.split(','))
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(str::to_string)
                .collect();
        }
        ui.add_space(2.0);
        if self.keyword_scan.is_some() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().color(p.primary));
                ui.label(RichText::new(lang.scanning()).color(p.primary));
            });
        } else {
            let response = ui
                .add_enabled_ui(enabled, |ui| {
                    fx::button(ui, ButtonStyle::Tonal, icon::SPARKLE, lang.suggest_keywords())
                })
                .inner;
            if response.on_hover_text(lang.suggest_tooltip()).clicked() {
                self.start_keyword_scan();
            }
        }
        if let Some(i) = self.selected {
            let folder = self.config.course_folder(&self.config.courses[i]);
            let text = format!("{} {}", lang.files_go_to(), folder.display());
            ui.add(egui::Label::new(RichText::new(text).small().color(p.on_surface_variant)).wrap());
        }
    }

    // ---------- controls and activity ----------

    fn controls_ui(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        let lang = self.lang;
        ui.horizontal(|ui| {
            if fx::button(ui, ButtonStyle::Tonal, icon::BROOM, lang.sort_now()).clicked() {
                self.sort_now();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Extended floating action button: start, or stop with a pulsing dot.
                let running = self.running();
                let t = self.ctx.animate_bool_with_time(Id::new("fab"), running, fx::anim_time(&self.ctx, 0.35));
                let fill = fx::lerp_color(p.primary_container, p.error_container, t);
                let fg = fx::lerp_color(p.on_primary_container, p.on_error_container, t);
                let (glyph, text) = if running { (icon::STOP, lang.stop()) } else { (icon::PLAY, lang.start()) };
                let galley = ui.painter().layout_no_wrap(format!("{glyph}   {text}"), fx::medium_font(15.0), fg);
                let width = self.ctx.animate_value_with_time(
                    Id::new("fab-width"),
                    galley.size().x + 48.0 + if running { 16.0 } else { 0.0 },
                    fx::anim_time(&self.ctx, 0.3),
                );
                let (rect, response) = ui.allocate_exact_size(vec2(width, 56.0), Sense::click());
                let radius = CornerRadius::same(16);
                let painter = ui.painter();
                painter.rect_filled(rect.translate(vec2(0.0, 2.0)), radius, Color32::from_black_alpha(28));
                painter.rect_filled(rect, radius, fill);
                fx::state_layer(ui, &response, rect, radius, fg);
                fx::ripple(ui, &response, rect, fg);
                let text_left = rect.left() + 24.0 + if running { 16.0 } else { 0.0 };
                if running {
                    fx::pulse_dot(
                        painter,
                        Pos2::new(rect.left() + 22.0, rect.center().y),
                        fg,
                        fx::motion(&self.ctx),
                        self.now(),
                    );
                }
                painter.galley(Pos2::new(text_left, rect.center().y - galley.size().y / 2.0), galley, fg);
                if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.toggle_watching();
                }
            });
        });
    }

    fn activity_ui(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        let lang = self.lang;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(fx::title(lang.activity(), 16.0).color(p.on_surface));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| self.stats_ui(ui));
        });
        ui.add_space(2.0);
        let mut actions = Vec::new();
        egui::ScrollArea::vertical().id_salt("activity").auto_shrink([false, false]).show(ui, |ui| {
            if self.cards.is_empty() {
                ui.add_space(16.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(icon::TRAY_ARROW_DOWN).size(34.0).color(p.outline));
                    ui.label(RichText::new(lang.activity_empty()).color(p.on_surface_variant));
                });
                return;
            }
            let now = self.now();
            ui.spacing_mut().item_spacing.y = 2.0;
            for card in self.cards.iter().rev() {
                match cards::activity_row(ui, lang, card, now) {
                    RowAction::None => {}
                    action => actions.push((card.key, action)),
                }
            }
        });
        for (key, action) in actions {
            match action {
                RowAction::Open(path) => crate::desktop::reveal(&path),
                RowAction::Undo => self.start_undo(key),
                RowAction::None => {}
            }
        }
    }

    /// Draw the file cards flying from the Downloads field to their module rows.
    pub(super) fn flights_ui(&mut self, ctx: &egui::Context) {
        if self.flights.is_empty() {
            return;
        }
        let now = self.now();
        let p = fx::pal_ctx(ctx);
        let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("flights")));
        let screen = ctx.content_rect();
        let source = self
            .watch_field_rect
            .map(|r| r.left_center() + vec2(90.0, 0.0))
            .unwrap_or(screen.left_top() + vec2(120.0, 60.0));
        let mut arrived = Vec::new();
        let fly = fx::motion(ctx) && self.tab == super::Tab::Overview && !self.in_tray();
        for (i, flight) in self.flights.iter().enumerate() {
            let t = ((now - flight.start) / FLIGHT_SECS) as f32;
            if t >= 1.0 || !fly {
                arrived.push(i);
                continue;
            }
            // Fly to the row, or to the edge of the list if the row is scrolled out of view.
            let target = match (self.row_rects.get(&flight.target), self.list_clip) {
                (Some(row), Some(clip)) => {
                    let at = row.left_center() + vec2(26.0, 0.0);
                    Pos2::new(at.x, at.y.clamp(clip.top() + 10.0, clip.bottom() - 10.0))
                }
                (Some(row), None) => row.left_center() + vec2(26.0, 0.0),
                _ => self.list_clip.map(|c| c.center_top()).unwrap_or(screen.center()),
            };
            let control = Pos2::new((source.x + target.x) / 2.0 + 160.0, source.y.min(target.y) - 80.0);
            let e = fx::ease_in_out_cubic(t);
            let pos = fx::bezier(source, control, target, e);
            let pop = fx::ease_out_back((t / 0.25).min(1.0));
            let shrink = if t > 0.7 { 1.0 - (t - 0.7) / 0.3 * 0.5 } else { 1.0 };
            let alpha = if t > 0.85 { 1.0 - (t - 0.85) / 0.15 } else { 1.0 };
            // A soft trail of dots.
            for k in 1..=6 {
                let tk = (e - k as f32 * 0.035).max(0.0);
                let at = fx::bezier(source, control, target, tk);
                painter.circle_filled(
                    at,
                    4.0 - k as f32 * 0.5,
                    p.primary.gamma_multiply(alpha * (0.45 - k as f32 * 0.06)),
                );
            }
            let mut name = flight.name.clone();
            if name.chars().count() > 26 {
                name = name.chars().take(25).collect::<String>() + "…";
            }
            let font = FontId::new(13.0 * pop.max(0.2) * shrink, fx::medium());
            let text = format!("{}  {name}", icon::FILE_TEXT);
            let galley = painter.layout_no_wrap(text, font, p.on_primary_container.gamma_multiply(alpha));
            let rect = Rect::from_center_size(pos, galley.size() + vec2(20.0, 12.0));
            painter.rect_filled(
                rect.translate(vec2(0.0, 3.0)),
                CornerRadius::same(8),
                Color32::from_black_alpha((50.0 * alpha) as u8),
            );
            painter.rect_filled(rect, CornerRadius::same(8), p.primary_container.gamma_multiply(alpha));
            painter.galley(rect.min + vec2(10.0, 6.0), galley, p.on_primary_container);
        }
        for i in arrived.into_iter().rev() {
            let flight = self.flights.remove(i);
            *self.module_counts.entry(flight.target.clone()).or_default() += 1;
            self.glows.insert(flight.target, now);
        }
    }
}
