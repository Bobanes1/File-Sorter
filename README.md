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

> **"Eine Anwendungssteuerungsrichtlinie hat diese Datei blockiert" when starting from the taskbar:** Windows marks downloaded files as "from the web", and Smart App Control blocks shortcuts made from such files. File Sorter removes that mark from itself and its own shortcuts every time it starts, so start `FileSorter.exe` once directly (double-click) and the taskbar shortcut works again. If it still doesn't: unpin it, right-click `FileSorter.exe` → *Properties* → tick **Unblock** → *OK*, and pin it again.

> **Blank icon on the taskbar:** since v0.6 the icon is built into `FileSorter.exe`. If a shortcut you pinned with an older version still shows a blank icon, Windows is showing its cached copy: unpin it and pin it again once.

To start it automatically when you sign in to Windows, turn on **Settings → Start when I sign in**.

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

Double-click `FileSorter.exe` (or run `cargo run --release`). A window opens – in German if Windows runs in German, otherwise in English (change it under **Settings**). It has two tabs at the top right: **Overview**, where you set up folders and modules and watch the activity, and **Settings**.

At the top you see the status (a pulsing radar dot while watching) and four counters: files sorted today, downloads in progress, files that matched no module, and the number of modules.

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
5. Click **▶ Start watching**. Every download gets a card in *Activity*: it spins while the download is still running and turns green ("Moved to Informatik › Programmierung 1", with the matching keywords) once the file is sorted. At the same time a little file card flies into the module, which lights up and counts the file.

*Add / Edit* let you create a module by hand (pick the study program, type the module name; the folder is created on the first file). Modules without a study program go directly into the study folder.

*Sort existing files now* cleans up files that are already sitting in the folder.

Folders, modules and keywords are saved when you start watching, click *Save* or close the window.

Click a green activity card to open the module folder with that file selected.

## Right-click: sort any file right away

Like *Extract all…* for a zip: right-click a file and choose **Mit File Sorter sortieren** / **Sort with File Sorter**. A small window shows where it went (click a card to open the folder) and closes by itself after a few seconds (move the mouse over it to keep it open).

- On **Windows 11** the entry is in the classic menu: right-click → **Weitere Optionen anzeigen** / **Show more options** (or hold `Shift` while right-clicking). Only signed apps can add entries to the short new menu.
- Select **several files** (up to 15, a Windows limit for this kind of menu entry) and they are all sorted, with a single results window.
- On a **folder** it sorts the documents inside (not its subfolders) – handy for a folder of slides you just unpacked from a zip.
- Any file type works here, not only the document types the watcher looks at. Files that match no module stay where they are.
- **Dragging** files onto `FileSorter.exe` (or a shortcut to it) does the same.

The entry is added when you open File Sorter, and it follows the exe if you move it (open it once from the new place). Turn off **Settings → Right-click menu → Show «Sort with File Sorter»** to remove it again; *Show the results window* there switches the little window off if you only want the files sorted. Nothing is installed system-wide: it lives in your own user's registry (`HKEY_CURRENT_USER\Software\Classes`).

## How it decides where a file goes

1. Only document types are considered (`.pdf .pptx .ppt .docx .doc .key .odp .odt .txt .md .zip .ipynb .xlsx`). Unfinished downloads (`.crdownload`, `.part`, …) are ignored until the browser finishes them.
2. **File name first:** the module name and each keyword are looked for as whole words (`ai` doesn't match `maintain`; `algorithm` matches `algorithms`; case, accents, `_`, `-` and `.` don't matter, and `INF101`, `INF-101` and `INF 101` are all the same).
3. **Then the content:** if the file name doesn't point to exactly one module, the text of the first pages of PDFs, the first slides of PowerPoints, or the start of Word documents is read and scored the same way.
4. The module with the most hits wins. With no hits, or a tie between two modules, the file is left where it is.
5. Files are **moved** (unless you choose *Copy* in the settings): after sorting, a file exists only in its module folder. If another program still has it open (the browser, a PDF preview, the virus scanner), the sorter waits for it and tries again a little later instead of leaving it in two places.
6. If the **very same file** is already in the module folder (downloaded twice), the download is simply removed. If a *different* file with the same name is there (e.g. updated slides), the new one is saved as `name (1).pdf` – nothing is overwritten.
7. Only one File Sorter watches at a time; starting a second one (another window or `--headless`) shows a message instead of both sorting the same files.

**Tips for good keywords:** module codes (`INF-101`, `MA1`), short names used in file names (`prog1`, `linalg`), and the exact module title as it appears on the slides work best. Avoid generic words like `lecture` or `exercise` that appear in every module.

You can turn off *Read file contents* in the settings if you only want name-based sorting.

## Settings

Everything on the **Settings** tab is saved as soon as you change it (a green *Saved* appears for a moment).

| Setting | What it does |
|---------|--------------|
| **Language** | *Automatic* (like your computer), *Deutsch* or *English*. Also renames the right-click entry. |
| **Appearance** | *Automatic* (follows Windows' light/dark mode), *Light* or *Dark*. |
| **Animations** | Flying files, sliding cards and the pulsing status dot. Turn off for a calmer window. |
| **What happens to a file** | **Move** (default): the file ends up only in its module folder and Downloads stays tidy. **Copy**: a copy goes to the module folder and the original stays where it was. Each file is copied only once – File Sorter remembers what it copied (`copied.json` next to the settings), so it doesn't copy a file again when it sorts the folder later, even if you annotated or renamed the copy. A new version with the same name is copied as `name (1).pdf`. |
| **Read file contents** | When the file name isn't enough, read the text of PDFs, slides and Word files. |
| **Include files already there** | When watching starts, also sort the files already in the folder (otherwise only new downloads). |
| **File types** | Which types are sorted from the watched folder: tick or untick `.pdf`, `.pptx`, … or type others under *More* (e.g. `.svg, .ods`). *Defaults* restores the standard list. The right-click menu sorts any file you pick, whatever its type. |
| **Start when I sign in** | File Sorter opens minimised in the taskbar and starts watching right away (Windows and Linux). |
| **Start watching right away** | Begin watching as soon as you open the window. |
| **Right-click menu** (Windows) | Show or hide «Sort with File Sorter», and whether the results window appears afterwards. |
| **About** | Version, and buttons to open the log file and the settings folder. |

## Run it without a window / start automatically

```bash
FileSorter.exe --autostart  # open minimised and start watching (what "Start when I sign in" runs)
FileSorter.exe --headless   # watch in the background without any window, using saved settings
FileSorter.exe --sort-now   # sort what's already in the folder once and exit
FileSorter.exe --sort FILE_OR_FOLDER...  # what the right-click entry runs; add --quiet for no window
FileSorter.exe --register-menu           # add the right-click entry (Windows), --unregister-menu removes it
```

Set up your modules in the window once; the headless mode uses the same settings. The easiest way to start File Sorter at sign-in is **Settings → Start when I sign in**; the steps below run it completely without a window instead.

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
| `src/main.rs` | command line (`--headless`, `--sort-now`, `--sort`, `--config`, …), starts the window |
| `src/gui.rs` | the egui window: overview (activity cards, flying files, statistics) and settings tab |
| `src/quicksort.rs` | right-click sorting: collects all selected files into one run and shows the results window |
| `src/desktop.rs` | the right-click menu entry, starting at sign-in (Windows registry / Linux autostart), "show in folder" |
| `src/winfix.rs` | removes Windows' "downloaded from the web" mark from the exe and its shortcuts |
| `build.rs`, `assets/` | puts the icon and version info into the Windows exe (`assets/make_icon.py` draws the icon) |
| `src/fx.rs` | theme, icons, easing, switches and the animated drawings |
| `src/i18n.rs` | all window texts in English and German |
| `src/watcher.rs` | watches the folder, waits for downloads to finish, rescans every 10 s |
| `src/matcher.rs` | keyword scoring |
| `src/keywords.rs` | keyword suggestions from a module's files |
| `src/extract.rs` | text from PDF / PPTX / DOCX |
| `src/sorter.rs` | moving or copying files (also across drives), duplicate handling |
| `src/ledger.rs` | copy mode: remembers which files were already copied |
| `src/config.rs` | settings, module-folder import |

`tests/e2e_firefox.py` downloads files with a real (headless) Firefox and checks they are sorted; it runs in CI on Windows (`pip install selenium`, then `python tests/e2e_firefox.py --exe dist/FileSorter.exe`).

### Making a release

Push a tag like `v0.4`, or run the **Build Windows exe** workflow manually (Actions tab → *Run workflow*) and fill in *release_tag*. Either way the exe is built, tested and attached to a GitHub release.
