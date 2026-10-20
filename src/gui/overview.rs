//! The overview tab: statistics, folders, modules with keywords, start/stop and the
//! activity list.

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Id, LayerId, Layout, Margin, Order, Pos2, Rect, RichText,
    Sense, Stroke, StrokeKind, vec2,
};
use egui_phosphor::regular as icon;

use super::cards::{self, RowAction};
use super::{App, CourseDialog, FLIGHT_SECS, GLOW_SECS};
use crate::config::SortRange;
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

    /// The two folders on one line: Downloads → study folder, each by its name (the whole
    /// path on hover), opened with a click and changed with the pencil.
    fn folders_ui(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        let lang = self.lang;
        let mut picked_watch = None;
        let mut picked_study = None;
        ui.allocate_ui_with_layout(vec2(ui.available_width(), 38.0), Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (pill, change) =
                folder_pill(ui, lang, icon::DOWNLOAD_SIMPLE, lang.field_watch(), &self.config.watch_folder);
            self.watch_field_rect = Some(pill);
            if change {
                picked_watch = App::pick_folder(&self.config.watch_folder);
            }
            ui.label(RichText::new(icon::ARROW_RIGHT).size(16.0).color(p.outline));
            let (_, change) =
                folder_pill(ui, lang, icon::GRADUATION_CAP, lang.field_study(), &self.config.destination_root);
            if change {
                picked_study = App::pick_folder(&self.config.destination_root);
            }
            // Where the study folder is connected to (opens the settings).
            if let Some(chip) = self.cloud_chip() {
                let glyph = if self.cloud.moving() { icon::CLOUD_ARROW_UP } else { icon::CLOUD_CHECK };
                let text = RichText::new(format!("{glyph}  {chip}")).small().color(p.primary);
                let link = ui.add(egui::Label::new(text).sense(Sense::click()));
                if link.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.switch_tab(super::Tab::Settings);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let import = fx::button(ui, ButtonStyle::Text, icon::FOLDERS, lang.import_modules());
                if import.on_hover_text(lang.layout_hint()).clicked() {
                    self.import_modules(false);
                }
            });
        });
        if let Some(folder) = picked_watch {
            self.config.watch_folder = folder;
        }
        if let Some(folder) = picked_study {
            self.config.destination_root = folder;
            if self.config.courses.is_empty() {
                self.import_modules(true);
            }
        }
    }

    fn modules_ui(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        let lang = self.lang;
        ui.horizontal(|ui| {
            ui.label(fx::title(lang.modules(), 16.0).color(p.on_surface));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if fx::button(ui, ButtonStyle::Text, icon::TREE_STRUCTURE, lang.feature_subfolders())
                    .on_hover_text(lang.subfolder_list_title())
                    .clicked()
                {
                    self.subfolder_action(super::subfolders::SubfolderAction::OpenList);
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(300.0);
                egui::ScrollArea::vertical()
                    .id_salt("modules")
                    .max_height(250.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.module_rows(ui));
                ui.add_space(4.0);
                if fx::button(ui, ButtonStyle::Tonal, icon::PLUS, lang.add()).clicked() {
                    let subject = if self.config.flat_modules {
                        String::new()
                    } else {
                        self.subjects().into_iter().next().unwrap_or_default()
                    };
                    self.dialog = Some(CourseDialog { editing: None, subject, name: String::new(), focused: false });
                }
            });
            ui.add_space(12.0);
            // The selected module: what it is, its keywords or its subfolders.
            let detail = egui::Frame::new()
                .fill(p.container_high)
                .corner_radius(CornerRadius::same(14))
                .inner_margin(Margin::same(14));
            let width = ui.available_width() - 28.0;
            detail.show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.set_min_height(268.0);
                    match self.selected {
                        Some(i) if i < self.config.courses.len() => self.module_detail_ui(ui, i),
                        _ => {
                            ui.add_space(70.0);
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new(icon::HAND_POINTING).size(30.0).color(p.outline));
                                ui.label(RichText::new(lang.select_module()).color(p.on_surface_variant));
                            });
                        }
                    }
                });
            });
        });
    }

    fn module_detail_ui(&mut self, ui: &mut egui::Ui, i: usize) {
        let p = fx::pal(ui);
        let lang = self.lang;
        let course = self.config.courses[i].clone();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(fx::title(&course.name, 17.0).color(p.on_surface));
                if !course.subject.is_empty() {
                    ui.label(RichText::new(&course.subject).small().color(p.on_surface_variant));
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if fx::icon_button(ui, icon::TRASH).on_hover_text(lang.delete()).clicked() {
                    self.confirm_delete = Some(i);
                }
                if fx::icon_button(ui, icon::PENCIL_SIMPLE).on_hover_text(lang.edit()).clicked() {
                    self.open_edit(i);
                }
                let folder = self.config.course_folder(&course);
                if fx::icon_button(ui, icon::FOLDER_OPEN)
                    .on_hover_text(format!("{}\n{}", lang.open_module_folder(), folder.display()))
                    .clicked()
                {
                    crate::desktop::open(&folder);
                }
            });
        });
        ui.add_space(8.0);
        let panes = [
            (ModulePane::Keywords, lang.keywords_title().to_string()),
            (ModulePane::Subfolders, lang.feature_subfolders().to_string()),
            (ModulePane::Notes, lang.notes_title().to_string()),
        ];
        fx::segmented(ui, "module-pane", &mut self.module_pane, &panes);
        ui.add_space(8.0);
        match self.module_pane {
            ModulePane::Keywords => self.keywords_ui(ui, i),
            ModulePane::Subfolders => self.module_subfolders_ui(ui, i),
            ModulePane::Notes => self.notes_ui(ui, i),
        }
    }

    /// Notes about the module, saved as they are typed.
    fn notes_ui(&mut self, ui: &mut egui::Ui, i: usize) {
        let p = fx::pal(ui);
        let lang = self.lang;
        let label = self.config.courses[i].label();
        ui.label(RichText::new(lang.notes_hint()).small().color(p.on_surface_variant));
        let mut text = self.notes.by_module.get(&label).cloned().unwrap_or_default();
        let field = egui::TextEdit::multiline(&mut text)
            .desired_rows(9)
            .desired_width(f32::INFINITY)
            .margin(Margin::symmetric(12, 8))
            .hint_text(lang.notes_title());
        if ui.add(field).changed() {
            if text.trim().is_empty() {
                self.notes.by_module.remove(&label);
            } else {
                self.notes.by_module.insert(label, text);
            }
            if let Some(file) = self.config.state_file(crate::notes::NOTES_FILE)
                && let Err(e) = self.notes.save(&file)
            {
                log::info!("Could not save the notes: {e}");
            }
        }
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

    fn keywords_ui(&mut self, ui: &mut egui::Ui, i: usize) {
        let p = fx::pal(ui);
        let lang = self.lang;
        ui.label(RichText::new(lang.keywords_hint()).small().color(p.on_surface_variant));
        const VISIBLE_ROWS: usize = 6;
        // Fixed height: long keyword lists scroll instead of pushing everything below out of view.
        let row_height = ui.text_style_height(&egui::TextStyle::Body);
        let changed = egui::ScrollArea::vertical()
            .id_salt("keywords")
            .max_height(row_height * VISIBLE_ROWS as f32 + 20.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let edit = egui::TextEdit::multiline(&mut self.keywords_text)
                    .desired_rows(VISIBLE_ROWS)
                    .desired_width(f32::INFINITY)
                    .margin(Margin::symmetric(12, 8))
                    .hint_text("prog1\nINF-101\nProf. Müller");
                ui.add(edit).changed()
            })
            .inner;
        if changed {
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
            let response = fx::button(ui, ButtonStyle::Tonal, icon::SPARKLE, lang.suggest_keywords());
            if response.on_hover_text(lang.suggest_tooltip()).clicked() {
                self.start_keyword_scan();
            }
        }
        // Where the files go, from the study folder on ("Studium › Informatik › Programmieren").
        let folder = self.config.course_folder(&self.config.courses[i]);
        let root = crate::config::expand_user(self.config.destination_root.trim());
        let start = root.parent().unwrap_or(&root);
        let relative = folder.strip_prefix(start).unwrap_or(&folder);
        let place = relative.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join(" › ");
        let text = format!("{} {place}", lang.files_go_to());
        ui.add(egui::Label::new(RichText::new(text).small().color(p.on_surface_variant)).wrap());
    }

    // ---------- controls and activity ----------

    fn controls_ui(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        let lang = self.lang;
        ui.horizontal(|ui| {
            let range = self.config.sort_existing_range;
            let main = fx::button(ui, ButtonStyle::Tonal, icon::BROOM, lang.sort_now());
            if main.on_hover_text(lang.sort_now_tooltip(range)).clicked() {
                self.sort_now();
            }
            // The range (last 24 hours / last 7 days / all), with a menu to sort with another one.
            let menu_button = fx::button(ui, ButtonStyle::Text, icon::CARET_DOWN, lang.range_short(range));
            let mut chosen = None;
            egui::Popup::menu(&menu_button).width(330.0).show(|ui| {
                ui.set_min_width(330.0);
                for value in [SortRange::Day, SortRange::Week, SortRange::All] {
                    if scope_menu_row(ui, value == range, lang.range_title(value), lang.range_hint(value)) {
                        chosen = Some(value);
                        ui.close();
                    }
                }
            });
            if let Some(value) = chosen {
                if value != self.config.sort_existing_range {
                    self.config.sort_existing_range = value;
                    self.settings_changed();
                }
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
            let modules: Vec<String> = self.config.courses.iter().map(|c| c.label()).collect();
            let running = self.semester.running_now();
            ui.spacing_mut().item_spacing.y = 2.0;
            for card in self.cards.iter().rev() {
                match cards::activity_row(ui, lang, card, now, &modules, running.as_deref()) {
                    RowAction::None => {}
                    action => actions.push((card.key, action)),
                }
            }
        });
        for (key, action) in actions {
            match action {
                RowAction::OpenFile(path) => crate::desktop::open(&path),
                RowAction::ShowFolder(path) => crate::desktop::reveal(&path),
                RowAction::Undo => self.undo_entry(key),
                RowAction::Assign(course) => self.start_assign(key, course),
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

/// What the selected module shows.
#[derive(Clone, Copy, PartialEq, Default)]
pub(super) enum ModulePane {
    #[default]
    Keywords,
    Subfolders,
    Notes,
}

/// A folder as a rounded pill with its name (the whole path on hover; a click opens it) and a
/// pencil to choose another one. Returns the pill's place and whether the pencil was clicked.
fn folder_pill(ui: &mut egui::Ui, lang: crate::i18n::Lang, glyph: &str, what: &str, path: &str) -> (Rect, bool) {
    let p = fx::pal(ui);
    let full = crate::config::expand_user(path.trim());
    let name = full.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|_| !path.trim().is_empty());
    let shown = name.clone().unwrap_or_else(|| lang.not_chosen().to_string());
    let frame = egui::Frame::new().fill(p.container_high).corner_radius(CornerRadius::same(18)).inner_margin(Margin {
        left: 12,
        right: 4,
        top: 4,
        bottom: 4,
    });
    let mut change = false;
    let response = frame
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(glyph).size(16.0).color(p.primary));
            let short = match shown.chars().count() {
                n if n > 28 => shown.chars().take(27).collect::<String>() + "…",
                _ => shown.clone(),
            };
            let label =
                egui::Label::new(RichText::new(short).family(fx::medium()).color(p.on_surface)).sense(Sense::click());
            let link = ui.add(label);
            let hover = format!("{what}\n{}", if name.is_some() { full.display().to_string() } else { shown.clone() });
            if link.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(hover).clicked() {
                if full.is_dir() {
                    crate::desktop::open(&full);
                } else {
                    change = true;
                }
            }
            if fx::icon_button(ui, icon::PENCIL_SIMPLE).on_hover_text(lang.change()).clicked() {
                change = true;
            }
        })
        .response;
    (response.rect, change)
}

/// One choice in the "sort existing files" menu: a check for the remembered one, title and hint.
fn scope_menu_row(ui: &mut egui::Ui, selected: bool, title: &str, hint: &str) -> bool {
    let p = fx::pal(ui);
    let response = ui
        .horizontal(|ui| {
            ui.set_min_height(52.0);
            ui.add_space(8.0);
            let (check, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
            if selected {
                ui.painter().text(
                    check.center(),
                    Align2::CENTER_CENTER,
                    icon::CHECK,
                    FontId::proportional(16.0),
                    p.primary,
                );
            }
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add_space(6.0);
                ui.label(RichText::new(title).size(14.5).color(p.on_surface));
                ui.add(egui::Label::new(RichText::new(hint).small().color(p.on_surface_variant)).wrap());
            });
        })
        .response;
    let rect = response.rect;
    let click = ui.interact(rect, ui.make_persistent_id(("scope-row", title)), Sense::click());
    fx::state_layer(ui, &click, rect, 8, p.on_surface);
    click.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}
