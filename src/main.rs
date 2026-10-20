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

    // The window, --headless and --sort-now may use the AI model (not the right-click window,
    // which shouldn't wait for it to load).
    laya::allow();
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
        if let Err(e) =
            laya::install(&place, |step| log::info!("Setting up Laya: step {} of 4 ({step:?})", step.number()))
        {
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
        let module = laya::module_for(&config, "kapitel4.pdf", text, &[]);
        log::info!("Laya: kapitel4.pdf (regression, variance) → {module:?}");
        // Dates: (sentence, the category it is; None: not a date to note).
        let dates = [
            ("Le devoir 3 est à rendre avant le vendredi 7 novembre 2026 à 18h.", Some("exercise")),
            ("L'examen final aura lieu le 20 janvier 2027 en salle B12.", Some("exam")),
            ("Version du 3 octobre 2026.", None),
            ("Die Anmeldung zur Klausur ist bis zum 15. November 2026 im Portal möglich.", Some("registration")),
            ("Abgabe der Seminararbeit: 30. November 2026.", Some("project")),
            ("Exkursion ins Naturhistorische Museum am 12. November 2026, Treffpunkt 9 Uhr.", Some("other")),
        ];
        let categories = deadlines::Category::defaults();
        let options: Vec<(String, String)> = categories.iter().map(|c| (c.id.clone(), c.about())).collect();
        let sentences: Vec<String> = dates.iter().map(|(s, _)| s.to_string()).collect();
        let kinds = laya::date_categories(&sentences, &options).unwrap_or_default();
        // Documents whose name says nothing: (text, the subfolder it goes in).
        let documents = [
            (
                "Analysis I, Herbstsemester 2026. Blatt 4. Aufgabe 1: Zeigen Sie, dass die Folge konvergiert. \
                 Aufgabe 2: Bestimmen Sie den Grenzwert.",
                "Übungen",
            ),
            (
                "Analysis I – Lösungen zu Blatt 4. Aufgabe 1: Da die Folge monoton wachsend und nach oben \
                 beschränkt ist, konvergiert sie. Aufgabe 2: Der Grenzwert ist 2, denn …",
                "Lösungen",
            ),
            (
                "Kapitel 3: Folgen und Reihen. Definition 3.1: Eine Folge heisst konvergent, wenn es eine Zahl a \
                 gibt, so dass … Satz 3.2: Jede konvergente Folge ist beschränkt. Beweis: …",
                "Vorlesung",
            ),
            (
                "Klausur Analysis I, 2. Februar 2026. Bearbeitungszeit: 120 Minuten. Erlaubte Hilfsmittel: keine. \
                 Aufgabe 1 (10 Punkte): …",
                "Prüfungen",
            ),
        ];
        let subfolders = categories::subfolder_options(&config::Subfolder::defaults(), &[]);
        let mut ok = module.is_some_and(|(m, _)| m == "Statistik");
        for ((sentence, expected), (got, p)) in dates.iter().zip(&kinds) {
            // Wrong and sure enough to be used: that would put a date in the wrong place.
            let right = got.as_deref() == *expected;
            let used = got.is_some() && *p >= 0.75 || got.is_none() && *p >= 0.9;
            log::info!("Laya: «{sentence}» → {got:?} {:.0} %{}", p * 100.0, if right { "" } else { "  ✗" });
            ok &= right || !used;
        }
        // The French hand-in and exam must be found.
        ok &= kinds.len() == dates.len()
            && kinds[0].0.as_deref() == Some("exercise")
            && kinds[1].0.as_deref() == Some("exam");
        for (text, expected) in documents {
            let got = laya::subfolder_for("scan_0042.pdf", text, &subfolders);
            let right = got.as_ref().is_some_and(|(f, _)| f == expected);
            log::info!("Laya: a document for «{expected}» → {got:?}{}", if right { "" } else { "  ✗" });
            // Not sure is fine (the rules decide then); a wrong folder is not.
            ok &= right || got.is_none();
        }
        ok &= laya::subfolder_for("scan_0042.pdf", documents[0].0, &subfolders).is_some_and(|(f, _)| f == "Übungen");
        if !ok {
            log::error!("Laya's answers were not as expected");
            return ExitCode::FAILURE;
        }
        log::info!("Laya answers as expected");
    }
    ExitCode::SUCCESS
}
