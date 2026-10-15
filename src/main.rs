//! File Sorter: automatically move downloaded course files into module folders.
//!
//! `file-sorter` opens the window, `--headless` only watches, `--sort-now` sorts once.

// No console window behind the app in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod common_words;
mod config;
mod desktop;
mod extract;
mod fx;
mod gui;
mod history;
mod i18n;
mod keywords;
mod ledger;
mod logger;
mod matcher;
mod quicksort;
mod sorter;
mod undo;
mod watcher;
mod winfix;

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
  [--sort] FILE...  sort these files (or the documents in these folders) and show
                    the result; this is what the right-click menu runs
  --quiet           with --sort: no results window, only the log file
  --register-menu   add \"Sort with File Sorter\" to the right-click menu (Windows)
  --unregister-menu remove it again
  --config PATH     settings file (default: per-user config.json)";

struct Args {
    config: PathBuf,
    headless: bool,
    sort_now: bool,
    autostart: bool,
    /// Files to sort right away (right-click menu, or dropped onto the exe).
    files: Vec<PathBuf>,
    quiet: bool,
    register_menu: Option<bool>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        config: default_config_path(),
        headless: false,
        sort_now: false,
        autostart: false,
        files: Vec::new(),
        quiet: false,
        register_menu: None,
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
            "--config" => args.config = it.next().ok_or("--config needs a path")?.into(),
            "-h" | "--help" => return Err(String::new()),
            other if other.starts_with("--") => return Err(format!("unknown option: {other}")),
            // Anything else is a file or folder to sort (drag & drop onto the exe works too).
            path => args.files.push(PathBuf::from(path)),
        }
    }
    Ok(args)
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
    if !args.files.is_empty() {
        return quicksort::run(&args.config, args.files, args.quiet);
    }

    if !(args.headless || args.sort_now) {
        return match gui::run(args.config, args.autostart) {
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
        for result in sorter::sort_existing(&config) {
            log::info!("{}", result.describe());
        }
        return ExitCode::SUCCESS;
    }

    // Headless: results are logged by the watcher itself.
    let watch_path = config.watch_path();
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
