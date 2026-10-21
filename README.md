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

Or, with *Modules directly in the study folder*, without a folder per study program: `Studium\Statistik\`, `Studium\Programmierung 1\`, …

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

To start it automatically when you sign in to Windows, turn on **Settings → Start when I sign in**. With **Settings → Keep running in the background**, closing the window while it watches leaves File Sorter running next to the clock; click that icon to open it again, right-click it → *Quit* to end it. Starting `FileSorter.exe` again (taskbar, Start menu) shows the running window instead of opening a second one.

The window is drawn with the graphics card (DirectX 12 or Vulkan). On PCs without one, Windows' built-in software renderer takes over automatically.

## Linux: the ready-made download

On the **Releases** page download **FileSorter-x86_64.AppImage**, make it executable (file manager → *Properties* → *Allow executing*, or `chmod +x FileSorter-x86_64.AppImage`) and start it. The AppImage **updates itself** like the Windows version (the new AppImage is checked against GitHub's size and SHA-256 before it replaces the old one).

Or download **FileSorter-linux-x86_64.tar.gz**, unpack it and start `./file-sorter` in the unpacked `FileSorter` folder. For a menu entry, copy `file-sorter` to `~/.local/bin/`, `icon.png` to `~/.local/share/icons/file-sorter.png` and `file-sorter.desktop` to `~/.local/share/applications/`. Updates of this version are announced in the window with a link.

Text recognition for scans uses tesseract: `sudo apt install tesseract-ocr tesseract-ocr-deu poppler-utils` (Debian/Ubuntu). Reminders use `notify-send`, which most desktops have.

## Build from source (any OS)

You need [Rust](https://rustup.rs) (stable).

```bash
git clone https://github.com/bobanes1/file-sorter.git
cd file-sorter
cargo build --release      # → target/release/file-sorter(.exe)
```

> Linux only: the window needs the usual desktop libraries, e.g. on Debian/Ubuntu `sudo apt install libxkbcommon-x11-0 libvulkan1 mesa-vulkan-drivers`.

## Use it

Double-click `FileSorter.exe` (or run `cargo run --release`). A window opens – in German if Windows runs in German, otherwise in English (change it under **Settings**). The first time, a short **setup** asks for the downloads folder, the study folder (also directly in OneDrive, Dropbox or Google Drive if one is found) and reads in your module folders; you can skip it and run it again later under *Settings → About → Run setup*.

The window has tabs at the top right: **Overview**, where you set up folders and modules and see the activity, **This week** (deadlines and the files that came in), **Timetable**, **Grades** and **Settings**. Next to them is the **search field** (`Ctrl+F`).

1. **Watched folder (Downloads)** – the folder to watch, shown at the top as «Downloads → Studium». Defaults to your Downloads folder; the pencil picks another one, a click on the name opens it (hover for the whole path).
2. **Study folder** – your `Studium` folder, containing one folder per study program with the module folders inside – or the module folders directly (*Settings → Sorting → Modules directly in the study folder*; the setup guesses which from your folders and asks).
3. Click **Import module folders** (happens automatically the first time you pick the study folder). Every `Study program \ Module` folder becomes an entry like `Informatik › Programmierung 1`, with starter keywords taken from the folder name. Run it again after creating new module folders; existing entries and keywords are kept.
4. Select a module: on the right you see its name with a button to open its folder and **⋮** to edit or delete it, and two views, **Keywords** and **Subfolders** (see [Subfolders](#subfolders)). Add **keywords**, one per line: anything that appears in the file names or on the first slides/pages, such as
   ```
   prog1
   java
   INF-101
   Prof. Müller
   ```
   The module name itself always counts as a keyword.

   Or let the program find them: select a module and click **Suggest keywords from files**. It reads the files already in that module's folder (names and the first pages) and looks for terms that are typical for *this* module: the module code (`stat 101`), the lecturer (the name after «Prof.»/«Dr.»), and topics that recur in its files but hardly ever in your other modules (`varianz`, `regression`). Everyday words («heute», «Beispiel», «Ergebnis», …), words found in every course («Vorlesung», «Übung», …) and terms another module also uses are never suggested. The suggestions appear in a list, each with what kind of term it is and in how many files it appears; untick what doesn't fit and click *Add*.
5. Click **▶ Start watching**. Every download appears in *Activity*: it spins while the download is still running and turns green ("Moved to Informatik › Programmierung 1", with the matching keywords) once the file is sorted. At the same time a little file card flies into the module, which lights up and counts the file, and a bar at the bottom offers **Undo**.

*Add* (under the list) and the pencil (at the selected module) let you create or change a module by hand (pick the study program, type the module name; the folder is created on the first file). Modules without a study program go directly into the study folder.

**Modules directly in the study folder** (*Settings → Sorting*): no folder per study program (major, minor) – `Studium\Statistik` instead of `Studium\Informatik\Statistik`, and *Add* no longer asks for a program. Switched on, it offers to move the existing module folders up, with a preview: nothing is overwritten (a module whose new folder exists already stays where it is and is listed), empty program folders are removed, and **Undo** on the note puts everything back. Notes, lessons, deadlines and learned download places stay with their modules.

*Sort existing files* cleans up files that are already sitting in the folder. The menu next to it chooses between **Only the last 24 hours**, **Only the last 7 days** (files downloaded or changed in that time – handy when the Downloads folder holds years of other things) and **All files**; the choice is remembered.

Everything is saved automatically.

Each entry in *Activity* has two buttons: **open the file** (in its usual program – PDF viewer, PowerPoint, …) and **open its folder** (with the file selected). Clicking the entry itself opens the file too. The counters next to *Activity* show the files sorted today, downloads in progress and files that matched no module.

## Undo

Sorted a file by mistake? Click **Undo** next to it in *Activity* (or in the bar that appears right after sorting). The file goes back where it came from – a moved file is moved back, a copy is removed again, a duplicate that went to the Recycle Bin is restored – and **it stays there**: File Sorter won't sort that file again – also not after you renamed it (it is known by its content) – unless you sort it yourself with the right-click menu. Earlier sorts are still listed after a restart, so you can undo them later too. In *Copy* mode a copy you have changed since (notes added) is never removed by *Undo* or *Other module*.

## Put a file into a module yourself

A file that matched no module shows **Assign ▾** in *Activity* (and in the right-click results window); a sorted file shows **Other module ▾**. Pick the module and the file moves there – like any sort, it can be undone.

With **Learn from assigning** on (default), File Sorter then takes up to two terms from the file name that point to that module – a module code first (`INF 101`), otherwise a rare word (`rekursion`) – and adds them as keywords, so the next such file is sorted by itself. It never learns everyday words, words that are already a keyword, or words that appear in other modules' file names. The note «Learned keywords for …» has an *Undo* button that removes them again.

## Search, deadlines and «This week»

- **Search** (`Ctrl+F`): finds every file in the study folder whose name or text has all the words you type – PDFs page by page, PowerPoints slide by slide, Word and text files, scans (text recognition) and the notes of your modules – with the passage and page number (and the other pages with hits).
  - The start of a word is enough (`rekurs` finds «Rekursion»), `"exact phrase"` in quotes, `-word` leaves out; when nothing is found, one typo in a longer word is allowed.
  - The best files come first: rare words count more than common ones, the name and the title page more than the rest, newer files first when equal.
  - Filters below the field: module, kind (lecture, exercise, solution, exam), file type and period (last 7 days, month, semester).
  - `↑`/`↓` choose a result, `Enter` opens it, `Ctrl+Enter` shows it in its folder.
  - **Scans** are searched by the text recognised in them (up to 40 pages each). Recognition gets a letter wrong now and then, so in recognised text a word with one wrong letter is found too (after the exact hits), words split at the end of a line («Rekur-» / «sion») are joined again, and such hits say *recognised text*. Scans that were in the search index before text recognition came (0.13) are read again once.
  - The text is read in the background and kept in `search-index.json`. The study folder is watched, so new and changed files are in the search within seconds; files that are only in the cloud (OneDrive «online only») are found by name but not downloaded.
- **Deadlines and exam dates**: File Sorter reads the documents in the study folder (the first pages of those changed in the last five months – also files sorted with the right-click menu or copied in by hand, and also with *Search* switched off) and finds
  - hand-ins: «Abgabe: 24.10.2026, 23:59 Uhr», «Abgabe: 3.11 bis 10 Uhr», «bis zum 7.11. abzugeben», «müssen bis Freitag, den 7. November um 14 Uhr über Ilias abgegeben werden», «Bitte bis 12 Uhr am 28.10. in den Briefkasten einwerfen», «Projektabgabe», «Einreichungsfrist», «Fällig: Montag, 3. November 2026, 23:59» (Moodle), «Bearbeitungszeitraum: 27.10. – 10.11.», «due Oct 24th, 11:59 PM», «Due: 24/10/2026», «Deadline: 2026-11-05»;
  - a semester plan with «Blatt | Ausgabe | Abgabe» columns: one deadline per row, named after the row («Blatt 3»);
  - only a weekday («Abgabe: Montag, 10:15 Uhr») or a relative date («morgen», «übermorgen», «in zwei Wochen», «nächste Woche Freitag», «KW 45» – its Friday): counted from when the file came, marked **date estimated** («jeweils Montag» or «every Friday» is no deadline);
  - exams and other dates: «Klausur: 20.01.2027, 14:00», «Klausurtermin», «Zwischenprüfung am 25.11.», «Semesterendprüfung», «Sessionsprüfung», «Leistungskontrolle 1», «Zwischentest», «Testat», «Präsentation», «midterm».

  Dates are read in German and English, and with French, Italian and Spanish month names («7 novembre», «20 gennaio»). With the **AI model Laya** set up, every sentence with a date is checked by it too: deadlines, exams and appointments written in any language or wording are found (hover over the title: *found by the AI model – check the date*). It never takes away a date the rules found.

  Only a date in the same sentence (or table column) counts; numbers like «Aufgabe 3.2.» and dates like «Stand: 20.10.» or «Abgabebeginn: …» don't. Each entry is named after what is due when the text says it («Serie 5», «Übungsblatt 3», «Teil A», «Projektabgabe»), else after the file.

  **Categories**: every date gets one – built in are **Übung** (exercise), **Prüfung** (exam), **Projekt / Arbeit** (project or paper), **Anmeldung** (registration) and **Sonstiger Termin** (other appointment). In *Settings → Features → Deadlines & appointments* they can be renamed, given words (a date whose sentence contains one goes into that category: `exkursion, ausflug` for an *Exkursion* category), marked as a date to **be there** or one to hand in by, removed, and others added (*+ Category*). Your words count first, then what the built-in rules know («Anmeldung», «Klausur», «Seminararbeit», «Serie 5»); only otherwise the AI model Laya, when it is set up, picks the category from the sentence (also among your own categories, by their name and words) when it is clearly sure.

  *This week* shows the week as a strip (today marked, a dot per deadline) and the deadlines in groups – **Overdue**, **This week**, **Next week**, **Later** (closed). Entries found since your last visit have a small dot. Each shows what is due («Serie 5»), its **category** in its colour, when («übermorgen fällig», «in 7 Tagen»), the module, and the **document** it was found in – a click opens it. **Done** is next to it; the **⋮** menu has **Correct** (fix a date, title or category that was read wrong), **Add to calendar** (an `.ics` file that Outlook, Google and Apple calendars import, with a reminder the day before), **Show in folder** and **Delete**. Hover over the title for the passage it was found in. The same exam found in several documents of a module is listed once. **Done** entries of the last two weeks can be opened again. **+ Deadline** adds one by hand, **Check a file…** picks one document and shows right away what is found in it (nothing found: *Add by hand* with the file and module filled in), **Look again** reads all documents again; the line below says how many documents were read. Finding dates in free text can still go wrong; a deleted or corrected entry isn't found again in the same file.
- **Recurring deadlines**: in **+ Deadline** (and **Correct**), *Repeats* lets one come again – weekly, every two weeks or monthly, until a last date (e.g. the end of the semester) or for good. **Done** then moves it on to its next date (the done one stays in the list), and its reminders start again; after the last date it is done for good. *Add to calendar* writes it as a series.
- **Reminders**: a notification before each deadline and exam – by default a day before and three hours before (also «the morning of the day» and «an hour before»), also while File Sorter runs in the background. Windows shows them like other apps' notifications (a click opens File Sorter); on Linux `notify-send` is used. Each is sent once; after a time away only one comes.
- **New this week**: the files sorted in the last 7 days, per module.

## Timetable, grades, notes

- **Timetable** (tab *Timetable*): *Import calendar…* reads a calendar file (`.ics`) exported from the university portal, Outlook or Google; *Calendar link…* subscribes to the portal's timetable link (iCal/webcal), read again every day; *+ Lesson* adds one by hand. Weekly series (also every two weeks), single dates that repeat weekly, cancelled and moved dates are understood; the module of each lesson is guessed from its name and can be changed in the preview. The week grid shows the lessons in the module's colour, the running one marked and cancelled ones greyed; a click edits a lesson, cancels it for that one week or opens the module folder.
- **Grades & ECTS** (tab *Grades*): per module the semester, ECTS and grade (or *passed*/*failed* for modules without a grade) – on the Swiss scale 1–6 (4 and up passed) or the German 1.0–5.0 (up to 4.0 passed), switchable.
  - **Tests** (the arrow at a module): a module's midterm, final, papers … each with its weight – `1/6`, `30 %`, `0.25` or a relative weight like `2` – and grade. The module's grade is then their weighted average, *provisional* while some are still open (a provisional grade doesn't count as passed yet).
  - **+ Module** adds a module only to the grades list (one from before you used File Sorter, say); such modules can be deleted. Your sorting modules can be taken out of the list (the eye) and shown again; archived modules are listed with their grades too.
  - Below: the ECTS-weighted and the plain average, how many modules are graded, the ECTS earned towards your target (default 180), and the averages per semester.
- **Notes** per module: the third view next to *Keywords* and *Subfolders* at the selected module, saved as you type and found by the search.

## Subfolders

Select a module in the overview and switch to **Subfolders** (or turn on *Settings → Features → Subfolders*): every module gets the same subfolders – by default `Vorlesung`, `Übungen`, `Lösungen`, `Prüfungen`. The view shows the module's subfolders with how many files each holds (and which aren't created yet) and how many files still lie directly in the module folder:

- **Create** makes the missing folders in this module. Nothing else is touched.
- **Sort in** moves the files lying directly in this module folder into their subfolders: a preview first shows how many files go where. Nothing is overwritten (a name that is taken gets «(1)»), files open in another program stay, earlier entries (undo, deadlines) follow the files, and *Undo* on the note puts everything back.
- **Learn from files** looks at the files already in the subfolders of all modules and learns which words in a name mean which subfolder: a word that is in at least three files of one subfolder – in several modules – and hardly anywhere else («korrektur» for `Lösungen`, «folien» for `Vorlesung`). The learned words show under each folder; × takes one out for good. The note has *Undo*. With *Settings → Features → Subfolders → Learn from existing files by itself* this happens every 30 minutes and after sorting in.
- **The list is shared** (*Subfolders* at the top of the module card, or the list button): rename a folder, add your own words to it («zusammenfassung, summary»), add folders (*+ Subfolder*, e.g. `Zusammenfassungen`, `Literatur`) or delete them, see and remove learned words, *Create in all modules* and *Sort existing files into subfolders* for all modules. The four standard folders already know many words in German and English (VL, Folien, Skript; Übung, Blatt, Serie; Lösung, Musterlösung; Klausur, Prüfung, Exam …).
- **New files go in by themselves**: a file goes into the subfolder whose own words are in its name, else the one whose learned words are, else the one its kind says; if the name says nothing, the same on the title of the text – also after a university header («Universität Zürich · Institut … · Prof. … · **Serie 5**») – and then on signs in the first pages: numbered tasks and «Zeigen Sie …» (exercise sheet), «Lösung» (solutions), «Bearbeitungszeit», «Hilfsmittel» and points (exam), «Definition 3.1», «Satz», «Beweis», «Kapitel» (lecture material); only a clear lead decides. A folder you made yourself inside a module is used too: a module with a `Projekt` folder gets «Projekt_Anleitung.pdf» there. Files that fit nowhere stay in the module folder, and the activity list says «kein Unterordner passte». **With the AI model Laya on, Laya decides the subfolder instead** (see below).
- **Leftover files go in too** (*Settings → Features → Subfolders → Sort leftover files into subfolders*, on by default): files lying directly in a module folder – copied in by hand, synced from another computer, or sorted before – go into their subfolder by themselves a few minutes after they came (every five minutes File Sorter looks; a file is looked at again only when it changed). The first time, those already lying there are shown first (*Einsortieren* or *Nicht jetzt*: then they stay, and only new ones go by themselves). The activity list says «N Dateien in Unterordner verschoben» with **Undo**; a file put back that way stays where it is.

## Finish a semester

*Settings → Semester → Finish semester…*: tick the modules that are over, check the name (`HS2026` / `FS2027`, filled in for you) and click *Archive*. Their folders move to `Studium\Archiv\HS2026\…` (same rules as moving into the cloud: nothing is overwritten, files open in another program stay and are listed), and the modules leave the list. Each archived module is listed under its semester with **Restore**, which brings the module and its folder back.

## The AI model Laya

File Sorter can use **laya-uni**, a language model by [Nils Briggen](https://github.com/NilsBriggen): [Laya](https://huggingface.co/convaiinnovations/laya) (by ConvAI Innovations, multilingual, Apache 2.0) fine-tuned on course files of every faculty – from law and medicine to history, engineering and design – in German, English, French and Italian ([model card](https://github.com/NilsBriggen/laya-uni), training and tests in `training/laya-uni/`). With it, File Sorter

- **sorts every file on its own – module and subfolder** – instead of the keywords and rules (they only decide while Laya isn't there: not downloaded yet, or *Save resources in the background*). It reads the file's name and the start of its text, with your modules' keywords and your subfolders' words as the choices. It sorts when it is fairly sure (40 % and 1.5 times the next): between two close modules the activity list asks, when nothing stands out the file stays where it is, and a document that fits no subfolder stays in the module folder. On real course PDFs of 13 faculties it put 79 % of files first into the right module (the base Laya: 60 %); asked about subfolders the way it was trained (each folder with its words), it put all seven test documents – exercise sheets, also after a university header, solutions, scripts, slides, an exam – into the right one, and left course information in the module folder. Formula-heavy maths PDFs are its weak spot. It is used from the right-click menu too (that window then waits a few seconds for the model);
- **finds deadlines, exams and other appointments in any sentence and language** that the rules don't know (it never takes away a date the rules found), and picks their category when the rules can't.

It is **optional**: the setup offers it (ticked), File Sorter asks once after the update, and *Settings → Features → AI model Laya* has **Download** (with a progress bar), a switch and **Remove**.

It downloads about 680 MB once into `laya-uni` next to the settings; each file is checked against its size and SHA-256, which are fixed in File Sorter – a newer model only comes with a newer File Sorter, after it was tested with the real model. An interrupted download goes on where it stopped. (The Laya of File Sorter 0.15–0.16, with its own Python, is replaced by it and removed.)

It runs only on this computer, natively (no Python): while it is in use it needs about 1.4 GB of memory and about a second per question; it leaves memory after 15 minutes without questions, while *Save resources in the background* is on and the window is away, and with File Sorter. The right-click window doesn't wait for it.

## Updates

Once a day File Sorter asks GitHub whether there is a newer version (*Settings → Features → Updates*); on Linux the AppImage updates itself the same way. A bar then offers **Update now**: the new `FileSorter.exe` is downloaded, used only if its size and SHA-256 checksum are exactly those GitHub lists for the release, put in place of the old one (which is kept as `FileSorter.old.exe` and put back if anything fails) and started. With *Install automatically* it is installed in the background and runs from the next start. Installing works on Windows; elsewhere you get a link to the release.

## Your study folder in OneDrive, Dropbox or Google Drive

So your files are on your phone, tablet and other computers too: **Settings → Cloud → Study folder in the cloud**. File Sorter finds OneDrive (personal and work/school), Dropbox, Google Drive for desktop and iCloud Drive by itself; with **Other folder…** you can use any synced folder (Nextcloud, pCloud, a network drive, …). There are three choices:

| Choice | What happens |
|--------|--------------|
| **Off** | The study folder is only on this computer (as before). |
| **Move into the cloud** | The study folder is moved once into the cloud folder (e.g. `OneDrive\Studium`), and from then on files are sorted straight into it. The cloud app keeps it the same everywhere, **in both directions**: notes you write into a PDF on your tablet show up on your PC too. |
| **Copy in the cloud** | The study folder stays where it is. File Sorter keeps a copy in the cloud folder up to date – right after each sort and every minute. **One way only**: changes made on other devices stay in the cloud copy. |

Your files are safe while moving and copying:

- **Nothing is overwritten.** A different file with the same name that is already in the cloud folder stays; the moved one gets a number (`name (1).pdf`). The very same file isn't kept twice (the second one goes to the Recycle Bin).
- **Open files are left alone** – close your PDF viewer first; files that were still open stay at the old place and the card offers to move them later.
- **Copy in the cloud never destroys a change made in the cloud**: if a file was changed in the cloud copy *and* here, this computer's version is saved next to it with the computer's name (`Skript (DESKTOP-4FJ2K).pdf`). Files deleted here stay in the cloud copy; undoing a sort removes its copy again (unless it was changed there). While the cloud folder isn't there (drive not connected, app not signed in), File Sorter waits.
- **Undo keeps working** after the study folder has moved.
- Files that OneDrive keeps *online only* are not downloaded just to suggest keywords.

To bring the study folder back, choose **Off**: *Bring back* moves it to its old place (or `Documents`), *Leave it in OneDrive* keeps it there and File Sorter simply stops managing it.

**Several computers:** turn on **Share modules with my other computers**. Modules, keywords and file types are then kept in the study folder in the cloud (`.file-sorter.json`), so every computer sorts the same way; the newest change wins, and the first time two different sets of modules meet, File Sorter asks which to keep. Folders, language and startup settings stay per computer. On the second computer, choose the same cloud folder – if its study folder is still empty, File Sorter simply uses the one in the cloud.

On your phone or tablet, open the OneDrive, Dropbox or Google Drive app (or their website).

### Google Drive without the Google app

**Settings → Cloud → Google Drive (without the Google app)** keeps the study folder in sync with a folder in your Google Drive – **both ways** – without installing Google's app (also on Linux, where there is none):

1. Click **Connect Google Drive**. The first time, File Sorter fetches the open-source sync tool [rclone](https://rclone.org) (a fixed version from its GitHub releases, used only if its SHA-256 checksum is the published one; on Linux an installed `rclone` is used if there is one).
2. Your browser opens: sign in with Google and allow access. (If no browser opens, *Open sign-in page*.) The access is kept in File Sorter's settings folder (`google-drive\rclone.conf`), apart from any rclone setup of your own; *Disconnect* removes it.
3. The study folder is synced with the Drive folder of the same name (changeable under *Folder in Google Drive*): right after sorting, every 15 minutes (hourly while saving resources in the background) and at the start; *Sync now* any time. The first sync merges both sides.

Nothing is lost:

- A file **deleted or overwritten** on one side is first put into a backup on that side – on this computer in the settings folder (*Backups* opens it), in Drive in `File Sorter backup` – and kept for 30 days.
- A file **changed on both sides** is kept twice: the newer one under its name, the other next to it as `name.pdf.conflict1`.
- If **more than half of the files would be deleted** (say the study folder was emptied by mistake), nothing is synced and File Sorter says why.
- Google Docs, Sheets and Slides (which have no file) are left out.

Not offered while the study folder is in the Google Drive app's folder already (that app syncs it). `FileSorter.exe --list-clouds` writes the cloud folders File Sorter finds into the log file.

## The results window

Files sorted **by themselves** (watching, *Include files already there*) while File Sorter is minimised, in the notification area or behind other windows show up in a small window at the bottom right – the same one as after the right-click menu below: where each file went, **Undo**, **Other module** and **Assign** for files without a module. It closes after a few seconds (not while the mouse is over it; *Keep open* keeps it). When you look at File Sorter's window, the bar at its bottom shows them instead. *Settings → Sorting → Results window for automatic sorting* switches it off.

## Right-click: sort any file right away

Like *Extract all…* for a zip: right-click a file and choose **Mit File Sorter sortieren** / **Sort with File Sorter** (on Linux see below). A small window appears at the bottom right and shows where each file went (click one to open its folder, *Undo* to put it back); it closes by itself after a few seconds (it waits while the mouse is over it).

- On **Windows 11** the entry is in the classic menu: right-click → **Weitere Optionen anzeigen** / **Show more options** (or hold `Shift` while right-clicking). Only signed apps can add entries to the short new menu.
- Select **several files** (up to 15, a Windows limit for this kind of menu entry) and they are all sorted, with a single results window.
- On a **folder** it sorts the documents inside (not its subfolders) – handy for a folder of slides you just unpacked from a zip.
- Any file type works here, not only the document types the watcher looks at. Files that match no module stay where they are.
- **Dragging** files onto `FileSorter.exe` (or a shortcut to it) does the same.
- **Linux:** in Files (Nautilus) and Caja under **Scripts**, in Nemo, Dolphin and Thunar directly in the menu, and in every file manager under **Open with → Sort with File Sorter**. The entries live in your own folders (`~/.local/share/…`, `~/.config/…`), each only for the file managers that are installed, and go away with the setting.

The entry is added when you open File Sorter, and it follows the exe if you move it (open it once from the new place). Turn off **Settings → Right-click menu → Show «Sort with File Sorter»** to remove it again; *Show the results window* there switches the little window off if you only want the files sorted. Nothing is installed system-wide: it lives in your own user's registry (`HKEY_CURRENT_USER\Software\Classes`).

## How it decides where a file goes

1. Only document types are considered (`.pdf .pptx .ppt .docx .doc .key .odp .odt .txt .md .zip .ipynb .xlsx`). Unfinished downloads (`.crdownload`, `.part`, …) are ignored until the browser finishes them.
2. **File name first:** the module name and each keyword are looked for as whole words (`ai` doesn't match `maintain`; `algorithm` matches `algorithms`; case, accents, `_`, `-` and `.` don't matter, and `INF101`, `INF-101` and `INF 101` are all the same).
3. **Then the content:** if the file name doesn't point to exactly one module, the text of the first pages of PDFs, the first slides of PowerPoints, or the start of Word documents is read and scored the same way.
4. **How much a hit counts:** a keyword several modules share counts less (shared by two modules: half), and hits on the title page (the first ~60 words) count double. Keywords of six letters or more also match with **one typo** («Statisitk»), and **short forms of the module name** count when only one module has them («LinAlg» and «LA» for *Lineare Algebra*, «Prog1» for *Programmieren 1*, «Stat» for *Statistik*) – both at half weight.
   **With the AI model Laya on** (see [The AI model Laya](#the-ai-model-laya)), none of these steps is used: Laya alone picks the module (and the subfolder), and the activity list says «the AI model (Laya) read it – 82 % sure»; *Undo* works as always.
5. The module with the most points wins. **When two modules fit about equally well** (less than 25 % apart) and *Ask when unsure* is on, the file stays where it is and *Activity* asks «Unsicher: Statistik oder Programmieren?» with a button for each. Before asking, File Sorter tries three more things:
   - **Learned from the modules' files** (*Learn the modules from their files*): a small naive Bayes model knows each module by the names and first words of the files already in its folder, and answers only when it is clearly sure (80 % and twice as likely as the next).
   - **The timetable** (*Use the timetable for ties*): if one of the modules in question had its lesson when the file came (15 minutes before to 15 after), that one wins. It never decides against what the file says and never sorts a file nothing else fits – during a lesson you may well be working on something else.
   - **Download source**, below.
6. With no hits, or a tie between two modules, the file is left where it is – unless **Download source** knows better: on Windows, browsers note the page a file was downloaded from (Edge and Chrome always; Firefox too). For Moodle, ILIAS and OLAT that page names the course, so once files from one course went into a module, the next file from there goes there too, even if its name says nothing (`Blatt3.pdf`). The activity list then says «from moodle.uni.ch, like earlier files of this module».
   A **ZIP file** is matched by the names of the files in it (and the text of the first documents); with *Unpack ZIP files* on, it is unpacked next to itself into a new folder in the module (the ZIP is kept).
7. Files are **moved** (unless you choose *Copy* in the settings): after sorting, a file exists only in its module folder.
8. A file that is **still open** in another program (a PDF viewer you write notes in, Word, the browser) is left alone until that program closes it, so your notes are saved into the file before it is moved. The activity list says so while it waits.
9. **Nothing is ever overwritten.** If a *different* file with the same name is already in the module folder (e.g. updated slides, or your annotated copy), the new one is saved as `name (1).pdf`, and the activity list says so. If the **very same file** is there (downloaded twice), the download goes to the **Recycle Bin**, where you can still get it back.
10. **Scans** (PDFs whose pages are pictures, also with a stamp or page numbers as real text) and photos of pages are read with **text recognition** (at 300 dpi): on Windows with the one built into Windows (no download; it reads the languages installed in Windows), on Linux with tesseract if installed. Only pages without text are recognised, what was recognised is remembered by the file's content, and it is used for sorting, search and deadlines alike (*Recognise text in scans*).
11. Only one File Sorter watches at a time; starting a second one with `--headless` shows a message instead of both sorting the same files.

With the **AI model Laya** set up (see [below](#the-ai-model-laya)), step 5 asks it first: it reads the document and picks the module – only among the ones in question and only when it is clearly sure (*the AI model (Laya) read it – 87 % sure*).

**Tips for good keywords:** module codes (`INF-101`, `MA1`), short names used in file names (`prog1`, `linalg`), and the exact module title as it appears on the slides work best. Avoid generic words like `lecture` or `exercise` that appear in every module.

You can turn off *Read file contents* in the settings if you only want name-based sorting.

## Settings

Everything on the **Settings** tab is saved as soon as you change it (a green *Saved* appears for a moment).

| Setting | What it does |
|---------|--------------|
| **Language** | *Automatic* (like your computer), *Deutsch* or *English*. Also renames the right-click entry. |
| **Appearance** | *Automatic* (follows Windows' light/dark mode), *Light* or *Dark*. |
| **Animations** | Flying files, sliding entries, ripples and the pulsing dot on the stop button. Turn off for a calmer window. |
| **What happens to a file** | **Move** (default): the file ends up only in its module folder and Downloads stays tidy. **Copy**: a copy goes to the module folder and the original stays where it was. Each file is copied only once – File Sorter remembers what it copied (`copied.json` next to the settings), so it doesn't copy a file again when it sorts the folder later, even if you annotated or renamed the copy. A new version with the same name is copied as `name (1).pdf`. |
| **Read file contents** | When the file name isn't enough, read the text of PDFs, slides and Word files. |
| **Include files already there** | When watching starts, also sort the files already in the folder: *None*, *Last 24 hours*, *Last 7 days* or *All*. |
| **Modules directly in the study folder** | No folder per study program; offers to move the existing module folders up (with *Undo*) – see [Use it](#use-it). |
| **Results window for automatic sorting** | Show files sorted while File Sorter isn't in front in the small window at the bottom right – see [The results window](#the-results-window). |
| **File types** | Which types are sorted from the watched folder: tick or untick `.pdf`, `.pptx`, … or type others under *More* (e.g. `.svg, .ods`). *Defaults* restores the standard list. The right-click menu sorts any file you pick, whatever its type. |
| **Start when I sign in** | File Sorter opens minimised in the taskbar and starts watching right away (Windows and Linux). |
| **Start watching right away** | Begin watching as soon as you open the window. |
| **Keep running in the background** (Windows) | Closing the window while it watches keeps File Sorter running in the notification area (next to the clock); click the icon to open it again, right-click it → *Quit* to end it. With *Start when I sign in*, it then starts there too. |
| **Features** | **AI model Laya** (download, on/off, remove – see [above](#the-ai-model-laya)). Each of these can be switched on or off: **Learn from assigning**, **Learn the modules from their files**, **Ask when unsure**, **Use the timetable for ties**, **Download source**, **Subfolders** (off by default – see [Subfolders](#subfolders); with **Learn from existing files by itself**, off by default), **Unpack ZIP files** (off by default), **Keep older versions apart** (off by default: when «Skript_v2.pdf» arrives, «Skript.pdf» – with your notes in it – moves into `Ältere Versionen`; an older version arriving later goes straight there), **Recognise text in scans**, **Search in files**, **Deadlines & appointments** (with **Reminders** and their times, and the **Categories**), **New this week**, **Updates** (*Ask first* or *Install automatically*). Undo also reverses subfolders, older versions and unpacking (an unpacked folder is only removed if nothing in it was changed). |
| **Semester** | Finish a semester (archive modules) and restore archived ones – see [above](#finish-a-semester). |
| **Cloud** | Keep the study folder in OneDrive, Dropbox, Google Drive, … (moved there, or a copy), share modules with your other computers, or sync it with Google Drive without the Google app – see [above](#your-study-folder-in-onedrive-dropbox-or-google-drive). |
| **Right-click menu** (Windows) | Show or hide «Sort with File Sorter», and whether the results window appears afterwards. |
| **About** | Version, and buttons to open the log file and the settings folder. |

## Save resources in the background

*Settings → Save resources in the background* (off by default) makes File Sorter lighter while its window is closed (running next to the clock) or minimised: the search index leaves memory (it is on disk), files are read by name and first page only – without text recognition, the learned model, the AI model Laya or comparing versions by content – and learning, calendar links and the update check wait. Watching, sorting, undo and reminders go on. When the window is shown again, everything runs in full and what changed meanwhile is read. With 1600 documents of 22 pages in the study folder: about 460 MB with the window open, about 155 MB in the background with this setting on.

## Run it without a window / start automatically

```bash
FileSorter.exe --autostart  # open minimised and start watching (what "Start when I sign in" runs)
FileSorter.exe --headless   # watch in the background without any window, using saved settings
FileSorter.exe --sort-now   # sort what's already in the folder once and exit
FileSorter.exe --sort-now --days 7   # … only the files downloaded or changed in the last 7 days
FileSorter.exe --sort-now --hours 24 # … only the last 24 hours
FileSorter.exe --sort FILE_OR_FOLDER...  # what the right-click entry runs; add --quiet for no window
FileSorter.exe --register-menu           # add the right-click entry (Windows), --unregister-menu removes it
FileSorter.exe --list-clouds             # write the cloud folders found (OneDrive, Dropbox, …) to the log
FileSorter.exe --check-update            # write whether a newer version is out to the log
```

`--headless` keeps the copy in the cloud up to date too.

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

Next to it are File Sorter's own lists: `history.jsonl` (for undo), `kept.json`, `copied.json`, `cloud-mirror.json` (what the copy in the cloud holds), `origins.json` (which download page goes to which module), `search-index.json` (text for the search; can be deleted, it is rebuilt), `deadlines.json`, `timetable.json`, `notes.json`, `module-model.json` (learned from the modules' files; rebuilt), `ocr-cache.json` (recognised text of scans; can be deleted) and `recent-searches.json`.

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
| `src/gui/` | the egui window: `mod.rs` (state, background logic), `overview.rs`, `settings.rs`, `cloud.rs` (the cloud card), `dialogs.rs`, `cards.rs` (activity entries, snackbar) |
| `src/cloud/` | the study folder in the cloud: finding OneDrive/Dropbox/Google Drive (`detect.rs`), moving the folder (`relocate.rs`), the one-way copy (`mirror.rs`), modules shared between computers (`shared.rs`), the background thread (`worker.rs`) |
| `src/undo.rs`, `src/history.rs` | putting sorted files back; the history of sorted files (`history.jsonl`) |
| `src/winwin.rs`, `src/tray.rs` | minimising, the notification-area icon and showing the running window again (Windows) |
| `src/common_words.rs` | everyday words that are never suggested as keywords |
| `src/quicksort.rs` | right-click sorting: collects all selected files into one run and shows the results window |
| `src/desktop.rs` | the right-click menu entry, starting at sign-in (Windows registry / Linux autostart), "show in folder" |
| `src/winfix.rs` | removes Windows' "downloaded from the web" mark from the exe and its shortcuts |
| `build.rs`, `assets/` | puts the icon and version info into the Windows exe (`assets/make_icon.py` draws the icon); `assets/fonts/` holds Roboto (Apache-2.0, see the README there) |
| `src/fx.rs` | Material colours for light and dark, Roboto, switches, segmented buttons, chips, ripples, easing |
| `src/i18n.rs` | all window texts in English and German |
| `src/watcher.rs` | watches the folder, waits for downloads to finish, rescans every 10 s |
| `src/matcher.rs` | keyword scoring |
| `src/keywords.rs` | keyword suggestions from a module's files |
| `src/extract.rs` | text from PDF / PPTX / DOCX |
| `src/sorter.rs` | moving or copying files (also across drives), duplicate handling |
| `src/ledger.rs` | copy mode's list of copied files, and the files to leave alone after an undo |
| `src/config.rs` | settings, module-folder import |

`tests/e2e_firefox.py` downloads files with a real (headless) Firefox and checks they are sorted; it runs in CI on Windows (`pip install selenium`, then `python tests/e2e_firefox.py --exe dist/FileSorter.exe`).

### Making a release

Push a tag like `v0.4`, or run the **Build Windows exe** workflow manually (Actions tab → *Run workflow*) and fill in *release_tag*. Either way the exe is built, tested and attached to a GitHub release.
