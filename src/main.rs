//! File Sorter: automatically move downloaded course files into module folders.
//!
//! `file-sorter` opens the window, `--headless` only watches, `--sort-now` sorts once.

// No console window behind the app in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod archive;
mod categories;
mod cloud;
mod common_words;
mod config;
mod deadlines;
mod desktop;
mod extract;
mod fx;
mod gui;
mod history;
mod i18n;
mod index;
mod keywords;
mod ledger;
mod logger;
mod matcher;
mod origin;
mod quicksort;
mod resort;
mod sorter;
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
    register_menu: Option<bool>,
    list_clouds: bool,
    check_update: bool,
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
        register_menu: None,
        list_clouds: false,
        check_update: false,
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
            "--register-menu" => args.register_menu = Some(true),
            "--unregister-menu" => args.register_menu = Some(false),
            "--list-clouds" => args.list_clouds = true,
            "--check-update" => args.check_update = true,
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
