# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

File Sorter: a single Rust binary (edition 2024, no library crate) with an egui/eframe window on wgpu
that watches the Downloads folder and moves course files into `Study folder / Study program / Module`
folders. Targets Windows first (CI builds and releases `FileSorter.exe`), also Linux and macOS.
User-facing docs live in README.md and are long; keep them in sync when behaviour changes.

## Commands

    cargo build --release                 # → target/release/file-sorter(.exe)
    cargo run --release                   # open the window
    cargo run --release -- --headless     # watch without window, saved settings
    cargo run --release -- --sort-now --days 7
    cargo run --release -- --config /tmp/x/config.json   # use a throwaway config, not the real one
    cargo test                            # all unit tests (inline #[cfg(test)] modules)
    cargo test matcher                    # tests whose path contains "matcher"
    cargo test sorter::tests::recent_means_downloaded_or_changed_in_the_last_days -- --exact
    cargo test gdrive -- --ignored        # network: downloads rclone and syncs two folders (CI runs it)
    cargo clippy --all-targets
    cargo fmt                             # rustfmt.toml: max_width 120, use_small_heuristics = "Max"

CI (`.github/workflows/build-exe.yml`, Windows + ubuntu-22.04) runs `cargo test --release`, the ignored
gdrive tests, builds, checks the exe's embedded icon/version, then `tests/e2e_firefox.py --exe
dist/FileSorter.exe` (real headless Firefox via selenium, Windows only). Pushing a tag `vX.Y` builds a
GitHub release. Linux build needs `libxkbcommon-x11-0 libvulkan1 mesa-vulkan-drivers`.

Test fixtures in `tests/fixtures/` are binary (`.gitattributes`); don't let them be re-encoded.

## Architecture

**Sort pipeline.** `watcher.rs` (notify + 10 s rescan, waits for `.crdownload`/`.part`/Firefox
placeholders to finish) → `sorter::sort_file*` → `choose_course` decides the module, in order:
0. `laya.rs` (opt-in `features.laya`): the downloaded laya-uni model (candle port of Laya's ModernBERT +
   decision head; release assets of NilsBriggen/laya-uni pinned by SHA-256, kept in `state_dir/laya-uni/`);
   only when clearly sure, else on to 1. Training rows come from `--laya-rows` (`laya_rows.rs`), and the
   data, scripts and evaluation live in `training/laya-uni/`.
1. `matcher::decide`: keyword/module-name scoring on the file name, then on extracted text
   (`extract.rs`, `ocr.rs` for scans). Returns Clear / Unsure (within `UNSURE_MARGIN`) / Nothing.
2. `classifier.rs` (naive Bayes over module folders, `module-model.json`), only when sure.
3. `timetable.rs`: the module whose lesson was running when the file arrived, ties only.
4. `origin.rs`: download source URL (Moodle/ILIAS course page) remembered per module.
5. Otherwise left in place as Unsure or NoMatch.
Then `place_file` moves/copies (never overwrites: `name (1).pdf`; identical content → recycle bin),
handles subfolders, older versions (`versions.rs`), unzip, and `history::record` appends to
`history.jsonl`, which `undo.rs` reverses. `ledger.rs` tracks copy-mode copies and files to leave alone
after an undo. Any new step that moves files must stay undoable and must not overwrite.

**Naming.** In code a module is a `Course` (`config.courses`, `course: usize` index); "module" is the UI word.

**Config and state.** `config.rs` holds the serde `Config` (JSON at `%APPDATA%\FileSorter\`,
`~/.config/file-sorter/`, `~/Library/Application Support/FileSorter/`). Old Python-era (v0.1/v0.2)
configs must keep loading, so add fields with serde defaults. `config.state_dir` is where the side
files live (`history.jsonl`, `copied.json`, `origins.json`, `search-index.json`, `deadlines.json`,
`timetable.json`, `notes.json`, `ocr-cache.json`, …); several are caches that are safe to rebuild.

**GUI.** `src/gui/mod.rs` owns `App`. Background work that must continue while minimised or in the
tray (watch results, undo, tray, closing) runs in `App::logic`; `App::ui` only draws. Tabs/cards are
split into `gui/*.rs`. Styling (Material colours, Roboto, widgets, animations) is in `fx.rs`.
Every UI string goes through `i18n.rs` with both English and German.

**Cloud.** `src/cloud/`: detect OneDrive/Dropbox/Drive/iCloud (`detect.rs`), move the study folder
(`relocate.rs`), one-way mirror (`mirror.rs`, `cloud-mirror.json`), modules shared across machines via
`.file-sorter.json` in the study folder (`shared.rs`), rclone-based Google Drive two-way sync
(`gdrive.rs`), all driven by a background thread (`worker.rs`).

**Platform code.** Windows-only pieces are behind `cfg(windows)`: `tray.rs`, `winwin.rs` (single
instance / show running window), `winfix.rs` (strip Mark-of-the-Web), `desktop.rs` (HKCU right-click
menu, autostart; Linux autostart too), Windows OCR and notifications. `build.rs` embeds icon + version
into the exe on Windows only. Keep Linux building when touching these.

**Other features** (each a module in `src/`): search index (`index.rs`), deadlines/exams parsing
(`deadlines.rs`) and reminders, keyword suggestions (`keywords.rs`, `common_words.rs`), subfolder
learning (`subfolder_learn.rs`, `categories.rs`), semester archive (`archive.rs`), flattening modules
into the study folder (`flatten.rs`), self-update with SHA-256 check (`update.rs`), resource-saving
background mode (`lean.rs`), right-click batch sorting with results window (`quicksort.rs`).

## Conventions seen in the code

- Doc comments explain behaviour in plain English from the user's point of view; match that tone.
- A release bumps the `Cargo.toml` version and updates the README in the same change
  (e.g. "README for v0.14, version 0.14.0").
