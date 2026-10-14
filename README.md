# File Sorter

Automatically moves the lecture slides, PDFs and other course files you download into the right module folder.

```
Studium\                      ← study folder
├── Informatik\               ← study program
│   ├── Programmierung 1\     ← module
│   └── Datenbanken\
└── Mathematik\
    ├── Analysis II\
    └── Lineare Algebra\
```

Download `Prog1_VL03.pdf` → it lands in `Studium\Informatik\Programmierung 1\`.
Download `vorlesung5.pdf` whose title page says "Lineare Algebra" → it lands in `Studium\Mathematik\Lineare Algebra\`.
Everything that doesn't belong to a module stays in Downloads, untouched.

Written in Rust with an [egui](https://github.com/emilk/egui) window drawn through wgpu. Works on Windows, macOS and Linux.

## Windows: just use the .exe

Nothing to install.

1. On GitHub open the **Actions** tab → **Build Windows exe** → the latest green run → download **FileSorter-windows** under *Artifacts* (it's a zip containing `FileSorter.exe`). Released versions are on the **Releases** page (easiest).
2. Put `FileSorter.exe` anywhere you like (e.g. `Documents\FileSorter\`) and double-click it.
3. Windows SmartScreen may warn because the exe isn't code-signed: click **More info → Run anyway**.

To start it automatically at login: press `Win+R`, type `shell:startup`, and put a shortcut to `FileSorter.exe` there. Add `--headless` to the shortcut's *Target* to run it silently in the background with your saved settings (set up your modules in the window first).

The window is drawn with the graphics card (DirectX 12 or Vulkan). On PCs without one, Windows' built-in software renderer takes over automatically.

## Build from source (any OS)

You need [Rust](https://rustup.rs) (stable).

```bash
git clone https://github.com/bobanes1/file-sorter.git
cd file-sorter
cargo build --release      # → target/release/file-sorter(.exe)
```

> Linux only: the window needs the usual desktop libraries, e.g. on Debian/Ubuntu `sudo apt install libxkbcommon-x11-0 libvulkan1 mesa-vulkan-drivers`.

## Use it

Double-click `FileSorter.exe` (or run `cargo run --release`). A window opens:

1. **Watch (Downloads)** – the folder to watch. Defaults to your Downloads folder; pick any folder with *Browse…*.
2. **Study folder** – your `Studium` folder, containing one folder per study program with the module folders inside.
3. Click **Import module folders** (happens automatically the first time you pick the study folder). Every `Study program \ Module` folder becomes an entry like `Informatik › Programmierung 1`, with starter keywords taken from the folder name. Run it again after creating new module folders; existing entries and keywords are kept.
4. Select a module and add **keywords**, one per line: anything that appears in the file names or on the first slides/pages, such as
   ```
   prog1
   java
   INF-101
   Prof. Müller
   ```
   The module name itself always counts as a keyword.

   Or let the program find them: select a module and click **Suggest keywords from files**. It reads the files already in that module's folder (names and the first pages) and adds terms that show up in several of them but hardly ever in your other modules, such as the module code (`inf 101`), the lecturer's name or recurring topics. Generic words like "Vorlesung" or "Übung" are skipped, and so are terms another module also uses, since those would make files ambiguous. Delete any line you don't want.
5. Click **▶ Start watching**. The *Activity* box shows `Detected … – waiting for the download to finish…` as soon as a download starts, and `Moved … → Informatik › Programmierung 1` when it's sorted.

*Add / Edit* let you create a module by hand (pick the study program, type the module name; the folder is created on the first file). Modules without a study program go directly into the study folder.

*Sort existing files now* cleans up files that are already sitting in the folder.

Settings are saved automatically when you start watching or close the window.

## How it decides where a file goes

1. Only document types are considered (`.pdf .pptx .ppt .docx .doc .key .odp .odt .txt .md .zip .ipynb .xlsx`). Unfinished downloads (`.crdownload`, `.part`, …) are ignored until the browser finishes them.
2. **File name first:** the module name and each keyword are looked for as whole words (`ai` doesn't match `maintain`; `algorithm` matches `algorithms`; case, accents, `_`, `-` and `.` don't matter, and `INF101`, `INF-101` and `INF 101` are all the same).
3. **Then the content:** if the file name doesn't point to exactly one module, the text of the first pages of PDFs, the first slides of PowerPoints, or the start of Word documents is read and scored the same way.
4. The module with the most hits wins. With no hits, or a tie between two modules, the file is left where it is.
5. If a file with the same name already exists in the module folder, the new one is saved as `name (1).pdf` – nothing is overwritten.

**Tips for good keywords:** module codes (`INF-101`, `MA1`), short names used in file names (`prog1`, `linalg`), and the exact module title as it appears on the slides work best. Avoid generic words like `lecture` or `exercise` that appear in every module.

You can turn off reading file contents in the window if you only want name-based sorting.

## Run it without a window / start automatically

```bash
FileSorter.exe --headless   # watch in the background using saved settings
FileSorter.exe --sort-now   # sort what's already in the folder once and exit
```

Set up your modules in the window once; the headless mode uses the same settings.

**Windows:** press `Win+R`, type `shell:startup`, and create a shortcut there with target
`"C:\path\to\FileSorter.exe" --headless`.

**macOS:** create `~/Library/LaunchAgents/com.filesorter.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>com.filesorter</string>
  <key>ProgramArguments</key><array>
    <string>/Users/YOU/file-sorter/target/release/file-sorter</string><string>--headless</string>
  </array>
  <key>RunAtLoad</key><true/>
</dict></plist>
```

then run `launchctl load ~/Library/LaunchAgents/com.filesorter.plist`. (macOS may ask you to allow file-sorter access to the Downloads folder under *System Settings → Privacy & Security → Files and Folders*.)

**Linux:** create `~/.config/autostart/file-sorter.desktop`:

```ini
[Desktop Entry]
Type=Application
Name=File Sorter
Exec=sh -c "~/file-sorter/target/release/file-sorter --headless"
X-GNOME-Autostart-enabled=true
```

## Where settings are stored

| OS      | Path |
|---------|------|
| Windows | `%APPDATA%\FileSorter\config.json` |
| macOS   | `~/Library/Application Support/FileSorter/config.json` |
| Linux   | `~/.config/file-sorter/config.json` |

It's plain JSON, so you can also edit it by hand. Settings from the earlier Python versions (v0.1/v0.2) keep working. Use `--config path/to/config.json` to use a different file. A module can have an optional `"folder"` entry to send its files somewhere else entirely.

A log of everything the sorter did is written to `file-sorter.log` in the same folder. If something doesn't get sorted, that's the place to look.

## Firefox, Chrome and Edge

All three are supported: the sorter waits until the browser has finished the download (Firefox's empty placeholder file and `.part` file, Chrome/Edge's `.crdownload` file) before moving anything.

If PDFs open in Firefox's built-in viewer instead of being downloaded, they never reach your Downloads folder. To change that: Firefox *Settings → General → Applications → Portable Document Format (PDF) → Save File*.

## Development

```bash
cargo test                       # unit tests, incl. simulated Firefox/Chrome downloads
cargo clippy --all-targets
```

| File | What it does |
|------|--------------|
| `src/main.rs` | command line (`--headless`, `--sort-now`, `--config`), starts the window |
| `src/gui.rs` | the egui window |
| `src/watcher.rs` | watches the folder, waits for downloads to finish, rescans every 10 s |
| `src/matcher.rs` | keyword scoring |
| `src/keywords.rs` | keyword suggestions from a module's files |
| `src/extract.rs` | text from PDF / PPTX / DOCX |
| `src/sorter.rs` | moving files (also across drives) |
| `src/config.rs` | settings, module-folder import |

`tests/e2e_firefox.py` downloads files with a real (headless) Firefox and checks they are sorted; it runs in CI on Windows (`pip install selenium`, then `python tests/e2e_firefox.py --exe dist/FileSorter.exe`).

### Making a release

Push a tag like `v0.4`, or run the **Build Windows exe** workflow manually (Actions tab → *Run workflow*) and fill in *release_tag*. Either way the exe is built, tested and attached to a GitHub release.
