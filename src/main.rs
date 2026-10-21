//! File Sorter: automatically move downloaded course files into module folders.
//!
//! `file-sorter` opens the window, `--headless` only watches, `--sort-now` sorts once.

// No console window behind the app in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod archive;
mod categories;
mod classifier;
mod cloud;
mod common_words;
mod config;
mod deadlines;
mod desktop;
mod extract;
mod flatten;
mod fx;
mod gui;
mod history;
mod i18n;
mod index;
mod keywords;
mod laya;
mod laya_rows;
mod lean;
mod ledger;
#[cfg(target_os = "linux")]
mod linux_menu;
mod logger;
mod matcher;
mod notes;
mod ocr;
mod origin;
mod quicksort;
mod reminders;
mod resort;
mod sorter;
mod subfolder_learn;
mod timetable;
mod tray;
mod undo;
mod unzip;
mod update;
mod versions;
mod watcher;
mod winfix;
mod winwin;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use config::{default_config_path, load_config};

const USAGE: &str = "\
Usage: file-sorter [--config PATH] [--headless | --sort-now | [--sort] FILE... [--quiet]]

  (no options)      open the window
  --autostart       open the window minimised and start watching (used at sign-in)
  --headless        run without a window, only watching the folder
  --sort-now        sort files already in the watch folder once, then exit
  --days N          with --sort-now: only files downloaded or changed in the last N days
  --hours N         … or in the last N hours (e.g. --hours 24)
  [--sort] FILE...  sort these files (or the documents in these folders) and show
                    the result; this is what the right-click menu runs
  --quiet           with --sort: no results window, only the log file
  --register-menu   add \"Sort with File Sorter\" to the right-click menu (Windows)
  --unregister-menu remove it again
  --list-clouds     write the cloud folders found (OneDrive, Dropbox, …) to the log
  --check-update    write whether a newer File Sorter is available to the log
  --read-text FILE  write the text read from FILE (with text recognition for scans) to the log
  --laya-rows IN OUT  write training rows for the Laya model (see training/laya-uni)
  --config PATH     settings file (default: per-user config.json)";

struct Args {
    config: PathBuf,
    headless: bool,
    sort_now: bool,
    /// With `--sort-now`: only files from the last this many hours (`--days` × 24).
    hours: Option<u32>,
    autostart: bool,
    /// Files to sort right away (right-click menu, or dropped onto the exe).
    files: Vec<PathBuf>,
    quiet: bool,
    /// `--show-results`: the results window for what the main window sorted by itself.
    show_results: bool,
    /// `--laya-setup`: set the AI model up; `--laya-check`: ask it a few test questions.
    laya_setup: bool,
    laya_check: bool,
    register_menu: Option<bool>,
    list_clouds: bool,
    check_update: bool,
    /// Print the text read from this file (with text recognition for scans), and exit.
    read_text: Option<PathBuf>,
    /// `--laya-rows IN OUT`: write fine-tuning rows for the Laya model, and exit.
    laya_rows: Option<(PathBuf, PathBuf)>,
    /// Started again after an update: wait until the old process (this id) has ended.
    wait_for: Option<u32>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        config: default_config_path(),
        headless: false,
        sort_now: false,
        hours: None,
        autostart: false,
        files: Vec::new(),
        quiet: false,
        show_results: false,
        laya_setup: false,
        laya_check: false,
        register_menu: None,
        list_clouds: false,
        check_update: false,
        read_text: None,
        laya_rows: None,
        wait_for: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--headless" => args.headless = true,
            "--sort-now" => args.sort_now = true,
            "--autostart" => args.autostart = true,
            "--sort" => {}
            "--quiet" => args.quiet = true,
            "--show-results" => args.show_results = true,
            "--laya-setup" => args.laya_setup = true,
            "--laya-check" => args.laya_check = true,
            "--register-menu" => args.register_menu = Some(true),
            "--unregister-menu" => args.register_menu = Some(false),
            "--list-clouds" => args.list_clouds = true,
            "--check-update" => args.check_update = true,
            "--read-text" => args.read_text = Some(it.next().ok_or("--read-text needs a file")?.into()),
            "--laya-rows" => {
                let input = it.next().ok_or("--laya-rows needs an input folder and an output file")?;
                let output = it.next().ok_or("--laya-rows needs an output file")?;
                args.laya_rows = Some((input.into(), output.into()));
            }
            "--wait-for" => args.wait_for = it.next().and_then(|pid| pid.parse().ok()),
            "--config" => args.config = it.next().ok_or("--config needs a path")?.into(),
            "--days" | "--hours" => {
                let text = it.next().ok_or(format!("{arg} needs a number"))?;
                let n: u32 = text.parse().map_err(|_| format!("{arg} needs a number, not {text}"))?;
                args.hours = Some(if arg == "--days" { n.saturating_mul(24) } else { n });
            }
            "-h" | "--help" => return Err(String::new()),
            other if other.starts_with("--") => return Err(format!("unknown option: {other}")),
            // Anything else is a file or folder to sort (drag & drop onto the exe works too).
            path => args.files.push(PathBuf::from(path)),
        }
    }
    Ok(args)
}

/// Without a window, the copy in the cloud is still kept up to date (every minute).
fn headless_cloud_copy(config: &config::Config) -> Option<cloud::worker::Worker> {
    if config.cloud.mode != cloud::CloudMode::Mirror {
        return None;
    }
    let copy = cloud::cloud_study_root(config)?;
    let local = config::expand_user(config.destination_root.trim());
    let worker = cloud::worker::Worker::start(
        config.state_dir.clone(),
        Arc::new(|event| {
            if let cloud::worker::CloudEvent::Mirrored { report, .. } = event
                && report.copied + report.updated > 0
            {
                log::info!("Cloud copy: {} new, {} changed", report.copied, report.updated);
            }
        }),
    );
    log::info!("Keeping a copy of {} in {}", local.display(), copy.display());
    worker.send(cloud::worker::Job::SetMirror(Some((local, copy))));
    Some(worker)
}

/// Lock file next to the settings: only one File Sorter may watch at a time.
pub fn watch_lock_path(config_path: &std::path::Path) -> PathBuf {
    config_path.parent().map(|p| p.join("watch.lock")).unwrap_or_else(|| PathBuf::from("watch.lock"))
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("{msg}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let log_file = args.config.parent().map(|p| p.join("file-sorter.log")).unwrap_or("file-sorter.log".into());
    logger::init(&log_file);
    winfix::unblock_self();
    if let Some(pid) = args.wait_for {
        update::wait_for(pid);
    }
    update::clean_up();
    if let Some(file) = &args.read_text {
        ocr::set_cache_dir(&config::config_path_dir(&args.config));
        let text = extract::extract_text(file);
        log::info!("Text recognition: {:?}", ocr::available());
        log::info!("Text of {}:\n{text}", file.display());
        return if ocr::letters(&text) > 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    }
    if let Some((input, output)) = &args.laya_rows {
        return match laya_rows::run(input, output) {
            Ok(n) => {
                println!("{n} rows written to {}", output.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }
    if args.check_update {
        return match update::check() {
            Ok(Some(release)) => {
                let ready = if release.download.is_some() { "with a checked download" } else { "without a download" };
                log::info!("File Sorter {} is available ({ready}): {}", release.version, release.page);
                ExitCode::SUCCESS
            }
            Ok(None) => {
                log::info!("File Sorter {} is the newest version", env!("CARGO_PKG_VERSION"));
                ExitCode::SUCCESS
            }
            Err(e) => {
                log::error!("Could not look for updates: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if let Some(enable) = args.register_menu {
        let lang = load_config(&args.config)
            .map(|c| i18n::Lang::from_setting(c.language.as_deref()))
            .unwrap_or_else(|_| i18n::Lang::system());
        let result =
            if enable { desktop::register_menu(lang.context_menu_label()) } else { desktop::unregister_menu() };
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                log::error!("Could not change the right-click menu: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if args.list_clouds {
        let roots = cloud::detect::detect();
        for root in &roots {
            let account = root.account.as_deref().map(|a| format!(" ({a})")).unwrap_or_default();
            log::info!("Cloud folder: {:?}{account} at {}", root.provider, root.path.display());
        }
        log::info!("{} cloud folder(s) found", roots.len());
        return ExitCode::SUCCESS;
    }
    if !args.files.is_empty() {
        return quicksort::run(&args.config, args.files, args.quiet);
    }
    if args.show_results {
        return quicksort::run_results(&args.config);
    }

    if args.laya_setup || args.laya_check {
        return laya_command(&args);
    }

    if !(args.headless || args.sort_now) {
        // Already open (maybe in the notification area): show that window instead of a second one.
        let key = std::path::absolute(&args.config).unwrap_or(args.config.clone()).display().to_string();
        let Some(instance) = winwin::single_instance(&key, !args.autostart) else {
            log::info!("File Sorter is already open; showing that window");
            return ExitCode::SUCCESS;
        };
        return match gui::run(args.config, args.autostart, instance) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                log::error!("Could not open the window: {e}");
                ExitCode::FAILURE
            }
        };
    }

    let config = match load_config(&args.config) {
        Ok(config) => config,
        Err(e) => {
            log::error!("Could not read {}: {e}", args.config.display());
            return ExitCode::FAILURE;
        }
    };
    if config.courses.is_empty() {
        log::error!(
            "No modules configured yet. Run without --headless to set them up (settings file: {}).",
            args.config.display()
        );
        return ExitCode::FAILURE;
    }
    if args.sort_now {
        let scope = args.hours.map_or(config::ExistingScope::All, config::ExistingScope::LastHours);
        for result in sorter::sort_existing(&config, scope) {
            log::info!("{}", result.describe());
        }
        return ExitCode::SUCCESS;
    }

    // Headless: results are logged by the watcher itself.
    let watch_path = config.watch_path();
    let _cloud = headless_cloud_copy(&config);
    let mut watcher = watcher::FolderWatcher::new(config, Arc::new(|_| {})).with_lock(watch_lock_path(&args.config));
    if let Err(e) = watcher.start() {
        log::error!("{e}");
        return ExitCode::FAILURE;
    }
    log::info!("Watching {} (Ctrl+C to stop)", watch_path.display());
    loop {
        std::thread::park();
    }
}

/// `--laya-setup` (set the AI model up) and `--laya-check` (ask it a few test questions).
fn laya_command(args: &Args) -> ExitCode {
    let dir = config::config_path_dir(&args.config);
    let place = laya::Place::new(&dir);
    if args.laya_setup {
        let mut shown = 0;
        let progress = |done: u64, total: u64| {
            let tenth = (done * 10).checked_div(total).unwrap_or(0);
            if tenth > shown {
                shown = tenth;
                log::info!("Downloading Laya: {} of {} MB", done / 1_000_000, total / 1_000_000);
            }
        };
        if let Err(e) = laya::install(&place, progress) {
            log::error!("Laya could not be set up: {e}");
            return ExitCode::FAILURE;
        }
        log::info!("Laya is set up ({} MB)", place.size() / 1_000_000);
    }
    if args.laya_check {
        laya::configure(&dir, true);
        let mut config = config::Config::default();
        config.courses = vec![
            config::Course::new("Statistik", &["statistik"]),
            config::Course::new("Programmieren", &["programmieren"]),
            config::Course::new("Geschichte", &["geschichte"]),
        ];
        let text = "Lineare Regression und Varianzanalyse: Schätzung der Parameter, Konfidenzintervalle, \
                    t-Test und p-Werte für die Koeffizienten.";
        let module = laya::module_of(&config, "kapitel4.pdf", text);
        log::info!("Laya: kapitel4.pdf (regression, variance) → {module:?}");
        let rounded = |p: Vec<(String, f32)>| {
            p.into_iter().map(|(o, p)| format!("{o} {:.0} %", p * 100.0)).collect::<Vec<_>>().join(", ")
        };
        // Dates: (sentence, the category it gets; None: no entry; «must» be found).
        let dates = [
            ("Le devoir 3 est à rendre avant le vendredi 7 novembre 2026 à 18h.", Some("exercise"), true),
            ("L'examen final aura lieu le 20 janvier 2027 en salle B12.", Some("exam"), true),
            ("Version du 3 octobre 2026.", None, true),
            ("Die Anmeldung zur Klausur ist bis zum 15. November 2026 im Portal möglich.", Some("registration"), false),
            ("Abgabe der Seminararbeit: 30. November 2026.", Some("project"), true),
            ("Exkursion ins Naturhistorische Museum am 12. November 2026, Treffpunkt 9 Uhr.", Some("other"), false),
        ];
        let categories = deadlines::Category::defaults();
        let sentences: Vec<String> = dates.iter().map(|(s, _, _)| s.to_string()).collect();
        log::info!("Laya (raw): what the dates are → {:?}", laya::date_kinds(&sentences));
        let appointments: Vec<(String, String)> =
            categories.iter().filter(|c| c.appointment).map(|c| (c.id.clone(), c.about())).collect();
        let hand_ins: Vec<(String, String)> =
            categories.iter().filter(|c| !c.appointment).map(|c| (c.id.clone(), c.about())).collect();
        log::info!(
            "Laya (raw): which appointment → {:?}; which hand-in → {:?}",
            laya::date_categories(&sentences, &appointments),
            laya::date_categories(&sentences, &hand_ins)
        );
        let reference = chrono::NaiveDate::from_ymd_opt(2026, 10, 20).unwrap_or_default();
        let mut ok = matches!(&module, Some(laya::ModuleChoice::Sure(m, _)) if m == "Statistik");
        // Files of other faculties (Laya sorting alone): right, or not sure – never sure of a wrong module.
        for (modules, name, text, expected) in laya::REAL_CASES {
            let courses = modules.iter().map(|m| config::Course::new(*m, &[])).collect();
            let faculty = config::Config { courses, ..Default::default() };
            let raw = laya::module_probabilities(&faculty, name, text).map(rounded).unwrap_or_default();
            let got = laya::module_of(&faculty, name, text);
            let right = !matches!(&got, Some(laya::ModuleChoice::Sure(m, _)) if m != modules[expected]);
            let note = if right { "" } else { "  ✗" };
            log::info!("Laya: {name} (for «{}») → {got:?} [{raw}]{note}", modules[expected]);
            ok &= right;
        }
        for (sentence, expected, must) in dates {
            let found = deadlines::find_in(sentence, reference, &categories);
            let got: Vec<&str> = found.iter().map(|f| f.category.as_str()).collect();
            // A wrong category or a date that isn't one is an error; missing one only where it must be found.
            let right = match expected {
                Some(e) => got == [e] || got.is_empty() && !must,
                None => got.is_empty(),
            };
            log::info!("Laya: «{sentence}» → {got:?}{}", if right { "" } else { "  ✗" });
            ok &= right;
        }
        // Documents whose name says nothing: (text, the subfolder it goes in; None: the module folder).
        let documents = [
            (
                "Analysis I, Herbstsemester 2026. Blatt 4. Aufgabe 1: Zeigen Sie, dass die Folge konvergiert. \
                 Aufgabe 2: Bestimmen Sie den Grenzwert.",
                Some("Übungen"),
            ),
            (
                "Analysis I – Lösungen zu Blatt 4. Aufgabe 1: Da die Folge monoton wachsend und nach oben \
                 beschränkt ist, konvergiert sie. Aufgabe 2: Der Grenzwert ist 2, denn …",
                Some("Lösungen"),
            ),
            (
                "Kapitel 3: Folgen und Reihen. Definition 3.1: Eine Folge heisst konvergent, wenn es eine Zahl a \
                 gibt, so dass … Satz 3.2: Jede konvergente Folge ist beschränkt. Beweis: …",
                Some("Vorlesung"),
            ),
            (
                "Klausur Analysis I, 2. Februar 2026. Bearbeitungszeit: 120 Minuten. Erlaubte Hilfsmittel: keine. \
                 Aufgabe 1 (10 Punkte): …",
                Some("Prüfungen"),
            ),
            (
                "Universität Zürich, Institut für Mathematik, Analysis I, HS 2026, Prof. Dr. M. Muster. Serie 5. \
                 Abgabe bis Freitag. 1. Berechnen Sie die Ableitung von f(x) = x^2 sin x. 2. Zeigen Sie, dass f \
                 stetig ist.",
                Some("Übungen"),
            ),
            (
                "Lineare Algebra – Folien Woche 6. Eigenwerte und Eigenvektoren. Motivation, Beispiele, \
                 charakteristisches Polynom, Diagonalisierung. Zusammenfassung der Woche.",
                Some("Vorlesung"),
            ),
            (
                "Analysis I, Herbstsemester 2026. Organisatorisches: Sprechstunde nach Vereinbarung, Raum HG G 5, \
                 Kontakt per E-Mail an die Assistierenden. Bewertung: eine schriftliche Klausur am Ende des \
                 Semesters.",
                None,
            ),
        ];
        let subfolders = categories::subfolder_options(&config::Subfolder::defaults(), &[]);
        for (text, expected) in documents {
            // Laya sorting alone: each into its folder, and the course information into none.
            let raw =
                laya::subfolder_probabilities("scan_0042.pdf", text, &subfolders).map(rounded).unwrap_or_default();
            let got = laya::subfolder_of("scan_0042.pdf", text, &subfolders).map(|(f, _)| f);
            let right = got.as_deref() == expected;
            let note = if right { "" } else { "  ✗" };
            log::info!("Laya: a document for {expected:?} → {got:?} [{raw}]{note}");
            ok &= right;
        }
        if !ok {
            log::error!("Laya's answers were not as expected");
            return ExitCode::FAILURE;
        }
        log::info!("Laya answers as expected");
    }
    ExitCode::SUCCESS
}
