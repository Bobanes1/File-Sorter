//! The window: folders, modules with keywords, start/stop and an activity log (egui on wgpu).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui;

use crate::config::{Config, Course, expand_user, load_config, merge_courses, save_config, scan_study_folder};
use crate::watcher::{FolderWatcher, Sink, WatchEvent};

pub fn run(config_path: PathBuf) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("File Sorter")
            .with_inner_size([780.0, 660.0])
            .with_min_inner_size([640.0, 540.0]),
        ..Default::default()
    };
    eframe::run_native("File Sorter", options, Box::new(move |cc| Ok(Box::new(App::new(cc, config_path)))))
}

/// The Add/Edit module dialog.
struct CourseDialog {
    /// Index of the module being edited, or None when adding.
    editing: Option<usize>,
    subject: String,
    name: String,
    /// The module field gets keyboard focus once, when the dialog opens.
    focused: bool,
}

struct App {
    config_path: PathBuf,
    config: Config,
    /// Settings the running watcher currently uses.
    pushed: Option<Config>,
    watcher: Option<FolderWatcher>,
    events_tx: Sender<WatchEvent>,
    events_rx: Receiver<WatchEvent>,
    ctx: egui::Context,
    log: Vec<String>,
    selected: Option<usize>,
    keywords_text: String,
    dialog: Option<CourseDialog>,
    confirm_delete: Option<usize>,
    error: Option<(String, String)>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, config_path: PathBuf) -> Self {
        let (events_tx, events_rx) = channel();
        let mut log = Vec::new();
        let config = load_config(&config_path).unwrap_or_else(|e| {
            log.push(format!("Could not read {} ({e}); starting with default settings.", config_path.display()));
            Config::default()
        });
        let mut app = App {
            config_path,
            config,
            pushed: None,
            watcher: None,
            events_tx,
            events_rx,
            ctx: cc.egui_ctx.clone(),
            log,
            selected: None,
            keywords_text: String::new(),
            dialog: None,
            confirm_delete: None,
            error: None,
        };
        app.sort_courses();
        app
    }

    // ---------- helpers ----------

    fn sink(&self) -> Sink {
        let tx = self.events_tx.clone();
        let ctx = self.ctx.clone();
        Arc::new(move |event| {
            let _ = tx.send(event);
            ctx.request_repaint(); // wake the UI: results arrive from background threads
        })
    }

    fn push_log(&mut self, line: impl Into<String>) {
        let line = line.into();
        log::info!("{line}");
        self.log.push(line);
    }

    fn running(&self) -> bool {
        self.watcher.as_ref().is_some_and(FolderWatcher::running)
    }

    fn sort_courses(&mut self) {
        let selected = self.selected.map(|i| self.config.courses[i].clone());
        self.config.courses.sort_by_key(|c| (c.subject.to_lowercase(), c.name.to_lowercase()));
        self.select(selected.and_then(|c| self.config.courses.iter().position(|x| *x == c)));
    }

    fn select(&mut self, index: Option<usize>) {
        self.selected = index.filter(|&i| i < self.config.courses.len());
        self.keywords_text = self.selected.map(|i| self.config.courses[i].keywords.join("\n")).unwrap_or_default();
    }

    fn study_root(&self) -> PathBuf {
        expand_user(self.config.destination_root.trim())
    }

    fn subjects(&self) -> Vec<String> {
        let mut subjects: Vec<String> =
            self.config.courses.iter().map(|c| c.subject.clone()).filter(|s| !s.is_empty()).collect();
        if let Ok(entries) = std::fs::read_dir(self.study_root()) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.path().is_dir() && !name.starts_with('.') {
                    subjects.push(name);
                }
            }
        }
        subjects.sort_by_key(|s| s.to_lowercase());
        subjects.dedup();
        subjects
    }

    fn save(&mut self) {
        match save_config(&self.config, &self.config_path) {
            Ok(()) => self.push_log(format!("Settings saved to {}", self.config_path.display())),
            Err(e) => self.error = Some(("Could not save settings".into(), e.to_string())),
        }
    }

    fn import_modules(&mut self, quiet: bool) {
        let root = self.study_root();
        if !root.is_dir() {
            if !quiet {
                self.error =
                    Some(("Folder not found".into(), format!("Study folder does not exist:\n{}", root.display())));
            }
            return;
        }
        let found = scan_study_folder(&root);
        if found.is_empty() {
            if !quiet {
                self.error = Some((
                    "No modules found".into(),
                    "No module folders found.\nExpected: Study folder \\ Study program \\ Module".into(),
                ));
            }
            return;
        }
        let total = found.len();
        let added = merge_courses(&mut self.config.courses, found);
        self.sort_courses();
        self.push_log(format!("Imported {added} module(s) from {} ({} already known)", root.display(), total - added));
    }

    fn check_ready(&mut self) -> bool {
        if self.config.courses.is_empty() {
            self.error = Some(("No modules".into(), "Add or import at least one module first.".into()));
            return false;
        }
        let watch = self.config.watch_path();
        if !watch.is_dir() {
            self.error =
                Some(("Folder not found".into(), format!("Watch folder does not exist:\n{}", watch.display())));
            return false;
        }
        true
    }

    fn toggle_watching(&mut self) {
        if self.running() {
            if let Some(mut w) = self.watcher.take() {
                w.stop();
            }
            self.pushed = None;
            self.push_log("Stopped watching.");
            return;
        }
        if !self.check_ready() {
            return;
        }
        self.save();
        let mut watcher = FolderWatcher::new(self.config.clone(), self.sink());
        match watcher.start() {
            Ok(()) => {
                self.pushed = Some(self.config.clone());
                self.watcher = Some(watcher);
                self.push_log(format!(
                    "Watching {} – new downloads will be sorted automatically.",
                    self.config.watch_path().display()
                ));
            }
            Err(e) => self.error = Some(("Could not start".into(), e)),
        }
    }

    fn sort_now(&mut self) {
        if !self.check_ready() {
            return;
        }
        match &self.watcher {
            Some(w) if w.running() => w.sort_existing_async(),
            _ => FolderWatcher::new(self.config.clone(), self.sink()).sort_existing_async(),
        }
        self.push_log("Sorting files already in the folder…");
    }

    fn pick_folder(current: &str) -> Option<String> {
        let start = expand_user(current.trim());
        let mut dialog = rfd::FileDialog::new();
        if start.is_dir() {
            dialog = dialog.set_directory(&start);
        }
        dialog.pick_folder().map(|p| p.to_string_lossy().into_owned())
    }

    // ---------- UI sections ----------

    fn folders_ui(&mut self, ui: &mut egui::Ui) {
        const LABEL_WIDTH: f32 = 130.0;
        const BUTTON_WIDTH: f32 = 80.0;
        ui.set_width(ui.available_width());

        /// Label, stretching text field and Browse… button. Returns the folder picked, if any.
        fn folder_row(ui: &mut egui::Ui, label: &str, value: &mut String) -> Option<String> {
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(LABEL_WIDTH, 20.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_width(LABEL_WIDTH);
                        ui.label(label)
                    },
                );
                let width = ui.available_width() - BUTTON_WIDTH - ui.spacing().item_spacing.x;
                ui.add(egui::TextEdit::singleline(value).desired_width(width));
                ui.add_sized([BUTTON_WIDTH, 20.0], egui::Button::new("Browse…"))
                    .clicked()
                    .then(|| App::pick_folder(value))
                    .flatten()
            })
            .inner
        }

        if let Some(folder) = folder_row(ui, "Watch (Downloads):", &mut self.config.watch_folder) {
            self.config.watch_folder = folder;
        }
        if let Some(folder) = folder_row(ui, "Study folder:", &mut self.config.destination_root) {
            self.config.destination_root = folder;
            if self.config.courses.is_empty() {
                self.import_modules(true);
            }
        }
        ui.horizontal(|ui| {
            ui.add_space(LABEL_WIDTH + ui.spacing().item_spacing.x);
            ui.label(egui::RichText::new("Layout: Study folder \\ Study program \\ Module").weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Import module folders").clicked() {
                    self.import_modules(false);
                }
            });
        });
        ui.checkbox(&mut self.config.read_file_contents, "Read text inside PDFs / slides / Word files");
        ui.checkbox(&mut self.config.sort_existing_on_start, "Also sort files already in the folder when starting");
    }

    fn modules_ui(&mut self, ui: &mut egui::Ui) {
        ui.set_width(ui.available_width());
        let list_height = 190.0;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(280.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("modules")
                        .max_height(list_height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if self.config.courses.is_empty() {
                                ui.label(egui::RichText::new("No modules yet – import or add some.").weak());
                            }
                            let mut clicked = None;
                            let mut double_clicked = None;
                            for (i, course) in self.config.courses.iter().enumerate() {
                                let response = ui.selectable_label(self.selected == Some(i), course.label());
                                if response.clicked() {
                                    clicked = Some(i);
                                }
                                if response.double_clicked() {
                                    double_clicked = Some(i);
                                }
                            }
                            if let Some(i) = clicked {
                                self.select(Some(i));
                            }
                            if let Some(i) = double_clicked {
                                self.open_edit(i);
                            }
                        });
                });
                ui.horizontal(|ui| {
                    if ui.button("Add").clicked() {
                        let subject = self.subjects().into_iter().next().unwrap_or_default();
                        self.dialog =
                            Some(CourseDialog { editing: None, subject, name: String::new(), focused: false });
                    }
                    if ui.add_enabled(self.selected.is_some(), egui::Button::new("Edit")).clicked()
                        && let Some(i) = self.selected
                    {
                        self.open_edit(i);
                    }
                    if ui.add_enabled(self.selected.is_some(), egui::Button::new("Delete")).clicked() {
                        self.confirm_delete = self.selected;
                    }
                });
            });

            ui.vertical(|ui| {
                ui.label("Keywords (one per line, e.g. module code, short name, professor):");
                let enabled = self.selected.is_some();
                let edit = egui::TextEdit::multiline(&mut self.keywords_text)
                    .desired_rows(9)
                    .desired_width(f32::INFINITY)
                    .hint_text(if enabled { "prog1\nINF-101\nProf. Müller" } else { "Select a module" });
                if ui.add_enabled(enabled, edit).changed()
                    && let Some(i) = self.selected
                {
                    self.config.courses[i].keywords = self
                        .keywords_text
                        .lines()
                        .flat_map(|line| line.split(','))
                        .map(str::trim)
                        .filter(|k| !k.is_empty())
                        .map(str::to_string)
                        .collect();
                }
                if let Some(i) = self.selected {
                    let folder = self.config.course_folder(&self.config.courses[i]);
                    ui.add(
                        egui::Label::new(egui::RichText::new(format!("Files go to: {}", folder.display())).weak())
                            .wrap(),
                    );
                }
            });
        });
    }

    fn open_edit(&mut self, i: usize) {
        let c = &self.config.courses[i];
        self.dialog =
            Some(CourseDialog { editing: Some(i), subject: c.subject.clone(), name: c.name.clone(), focused: false });
    }

    fn controls_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let label = if self.running() { "■ Stop" } else { "▶ Start watching" };
            if ui.add(egui::Button::new(label).min_size(egui::vec2(140.0, 0.0))).clicked() {
                self.toggle_watching();
            }
            if ui.button("Sort existing files now").clicked() {
                self.sort_now();
            }
            if ui.button("Save settings").clicked() {
                self.save();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.running() {
                    ui.colored_label(egui::Color32::from_rgb(90, 200, 120), "Watching…");
                } else {
                    ui.label("Stopped");
                }
            });
        });
    }

    fn activity_ui(&mut self, ui: &mut egui::Ui) {
        ui.strong("Activity");
        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::ScrollArea::both().id_salt("activity").stick_to_bottom(true).auto_shrink([false, false]).show(
                ui,
                |ui| {
                    for line in &self.log {
                        ui.add(egui::Label::new(egui::RichText::new(line).monospace()).extend());
                    }
                },
            );
        });
    }

    fn dialogs_ui(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() {
            let subjects = self.subjects();
            let mut close = false;
            let mut ok = false;
            let title = if self.dialog.as_ref().unwrap().editing.is_some() { "Edit module" } else { "Add module" };
            let modal = egui::Modal::new(egui::Id::new("course-dialog")).show(ctx, |ui| {
                let d = self.dialog.as_mut().unwrap();
                ui.heading(title);
                egui::Grid::new("course-dialog-grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("Study program:");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut d.subject).desired_width(220.0));
                        egui::ComboBox::from_id_salt("subject-choices").selected_text("").width(24.0).show_ui(
                            ui,
                            |ui| {
                                for s in &subjects {
                                    if ui.selectable_label(d.subject == *s, s).clicked() {
                                        d.subject = s.clone();
                                    }
                                }
                            },
                        );
                    });
                    ui.end_row();
                    ui.label("Module:");
                    let name = ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(250.0));
                    if !d.focused {
                        name.request_focus();
                        d.focused = true;
                    }
                    if name.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        ok = true;
                    }
                    ui.end_row();
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(!d.name.trim().is_empty(), egui::Button::new("OK")).clicked() {
                        ok = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
            if modal.should_close() {
                close = true;
            }
            if ok && !self.dialog.as_ref().unwrap().name.trim().is_empty() {
                self.apply_dialog();
            } else if close {
                self.dialog = None;
            }
        }

        if let Some(i) = self.confirm_delete {
            let label = self.config.courses.get(i).map(Course::label).unwrap_or_default();
            let modal = egui::Modal::new(egui::Id::new("confirm-delete")).show(ctx, |ui| {
                ui.heading("Delete module");
                ui.label(format!("Remove '{label}'? (Its folder and files stay where they are.)"));
                ui.add_space(8.0);
                ui.horizontal(|ui| (ui.button("Delete").clicked(), ui.button("Cancel").clicked())).inner
            });
            let (delete, cancel) = modal.inner;
            if delete && i < self.config.courses.len() {
                self.config.courses.remove(i);
                self.select(None);
            }
            if delete || cancel || modal.should_close() {
                self.confirm_delete = None;
            }
        }

        if let Some((title, text)) = self.error.clone() {
            let modal = egui::Modal::new(egui::Id::new("error")).show(ctx, |ui| {
                ui.heading(title);
                ui.label(text);
                ui.add_space(8.0);
                ui.button("OK").clicked()
            });
            if modal.inner || modal.should_close() {
                self.error = None;
            }
        }
    }

    fn apply_dialog(&mut self) {
        let Some(d) = self.dialog.take() else { return };
        let subject = d.subject.trim().to_string();
        let name = d.name.trim().to_string();
        let course = match d.editing {
            Some(i) => {
                let c = &mut self.config.courses[i];
                c.subject = subject;
                c.name = name;
                c.clone()
            }
            None => {
                let mut course = Course::new(name, &[]).with_subject(subject);
                // Reuse the starter keywords an import would give this module, if its folder exists.
                if let Some(found) = scan_study_folder(&self.study_root()).into_iter().find(|f| {
                    f.subject.to_lowercase() == course.subject.to_lowercase()
                        && f.name.to_lowercase() == course.name.to_lowercase()
                }) {
                    course.keywords = found.keywords;
                }
                self.config.courses.push(course.clone());
                course
            }
        };
        self.selected = None;
        self.sort_courses();
        let index = self.config.courses.iter().position(|c| *c == course);
        self.select(index);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.events_rx.try_recv() {
            self.log.push(event.text());
        }

        egui::Panel::bottom("activity").resizable(true).default_size(200.0).min_size(110.0).show(ui, |ui| {
            ui.add_space(6.0);
            self.activity_ui(ui);
        });
        egui::CentralPanel::default_margins().show(ui, |ui| {
            ui.strong("Folders");
            egui::Frame::group(ui.style()).show(ui, |ui| self.folders_ui(ui));
            ui.add_space(8.0);
            ui.strong("Modules");
            egui::Frame::group(ui.style()).show(ui, |ui| self.modules_ui(ui));
            ui.add_space(8.0);
            self.controls_ui(ui);
        });
        let ctx = ui.ctx().clone();
        self.dialogs_ui(&ctx);

        // Keyword or folder edits take effect immediately while watching.
        if let Some(w) = &self.watcher
            && self.pushed.as_ref() != Some(&self.config)
        {
            w.update_config(self.config.clone());
            self.pushed = Some(self.config.clone());
        }
    }

    fn on_exit(&mut self) {
        if let Some(mut w) = self.watcher.take() {
            w.stop();
        }
        if let Err(e) = save_config(&self.config, &self.config_path) {
            log::error!("Could not save settings: {e}");
        }
    }
}
