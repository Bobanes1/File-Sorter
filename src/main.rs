//! File Sorter: automatically move downloaded course files into module folders.
//!
//! `file-sorter` opens the window, `--headless` only watches, `--sort-now` sorts once.

// No console window behind the app in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod config;
mod extract;
mod fx;
mod gui;
mod i18n;
mod keywords;
mod logger;
mod matcher;
mod sorter;
mod watcher;
mod winfix;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use config::{default_config_path, load_config};

const USAGE: &str = "\
Usage: file-sorter [--config PATH] [--headless | --sort-now]

  (no options)   open the window
  --headless     run without a window, only watching the folder
  --sort-now     sort files already in the watch folder once, then exit
  --config PATH  settings file (default: per-user config.json)";

struct Args {
    config: PathBuf,
    headless: bool,
    sort_now: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args { config: default_config_path(), headless: false, sort_now: false };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--headless" => args.headless = true,
            "--sort-now" => args.sort_now = true,
            "--config" => args.config = it.next().ok_or("--config needs a path")?.into(),
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown option: {other}")),
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

    if !(args.headless || args.sort_now) {
        return match gui::run(args.config) {
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
