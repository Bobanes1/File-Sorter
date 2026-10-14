"""Small Tkinter window for managing modules and starting/stopping the sorter."""

from __future__ import annotations

import queue
import tkinter as tk
from pathlib import Path
from tkinter import filedialog, messagebox, ttk

from .config import Config, Course, load_config, merge_courses, save_config, scan_study_folder
from .sorter import SortResult


class CourseDialog(tk.Toplevel):
    """Modal dialog asking for a module's subject (study program) and name."""

    def __init__(self, parent: tk.Misc, title: str, subjects: list[str], course: Course | None = None):
        super().__init__(parent)
        self.title(title)
        self.transient(parent)
        self.resizable(False, False)
        self.result: tuple[str, str] | None = None

        frame = ttk.Frame(self, padding=12)
        frame.pack(fill="both", expand=True)
        ttk.Label(frame, text="Study program:").grid(row=0, column=0, sticky="w", pady=2)
        self.subject = ttk.Combobox(frame, values=subjects, width=32)
        self.subject.grid(row=0, column=1, pady=2)
        ttk.Label(frame, text="Module:").grid(row=1, column=0, sticky="w", pady=2)
        self.name = ttk.Entry(frame, width=35)
        self.name.grid(row=1, column=1, pady=2)
        if course:
            self.subject.set(course.subject)
            self.name.insert(0, course.name)
        elif subjects:
            self.subject.set(subjects[0])

        buttons = ttk.Frame(frame)
        buttons.grid(row=2, column=0, columnspan=2, sticky="e", pady=(8, 0))
        ttk.Button(buttons, text="OK", command=self._ok).pack(side="left", padx=4)
        ttk.Button(buttons, text="Cancel", command=self.destroy).pack(side="left")
        self.bind("<Return>", lambda _e: self._ok())
        self.bind("<Escape>", lambda _e: self.destroy())
        self.name.focus_set()
        self.grab_set()

    def _ok(self) -> None:
        name = self.name.get().strip()
        if not name:
            self.name.focus_set()
            return
        self.result = (self.subject.get().strip(), name)
        self.destroy()


class App:
    def __init__(self, root: tk.Tk, config_path: Path):
        self.root = root
        self.config_path = config_path
        self.config = load_config(config_path)
        self.watcher = None
        self.results: queue.Queue[SortResult | str] = queue.Queue()

        root.title("File Sorter")
        root.minsize(680, 540)
        root.protocol("WM_DELETE_WINDOW", self.on_close)

        self.watch_var = tk.StringVar(value=self.config.watch_folder)
        self.dest_var = tk.StringVar(value=self.config.destination_root)
        self.contents_var = tk.BooleanVar(value=self.config.read_file_contents)
        self.existing_var = tk.BooleanVar(value=self.config.sort_existing_on_start)
        self.status_var = tk.StringVar(value="Stopped")

        self._current: int | None = None
        self._build()
        self._sort_courses()
        self._refresh_courses()
        self.root.after(200, self._drain_results)

    # ---------- layout ----------
    def _build(self) -> None:
        pad = {"padx": 8, "pady": 4}
        outer = ttk.Frame(self.root, padding=8)
        outer.pack(fill="both", expand=True)

        folders = ttk.LabelFrame(outer, text="Folders", padding=8)
        folders.pack(fill="x")
        folders.columnconfigure(1, weight=1)
        self._folder_row(folders, 0, "Watch (Downloads):", self.watch_var)
        self._folder_row(folders, 1, "Study folder:", self.dest_var, on_chosen=self._study_folder_chosen)
        ttk.Label(folders, text="Layout: Study folder \\ Study program \\ Module",
                  foreground="gray").grid(row=2, column=1, sticky="w", padx=4)
        ttk.Button(folders, text="Import module folders",
                   command=self.import_modules).grid(row=2, column=2, sticky="e")
        ttk.Checkbutton(folders, text="Read text inside PDFs / slides / Word files",
                        variable=self.contents_var).grid(row=3, column=0, columnspan=3, sticky="w")
        ttk.Checkbutton(folders, text="Also sort files already in the folder when starting",
                        variable=self.existing_var).grid(row=4, column=0, columnspan=3, sticky="w")

        courses = ttk.LabelFrame(outer, text="Modules", padding=8)
        courses.pack(fill="both", expand=True, pady=(8, 0))
        courses.columnconfigure(1, weight=1)
        courses.rowconfigure(0, weight=1)

        left = ttk.Frame(courses)
        left.grid(row=0, column=0, sticky="ns")
        self.course_list = tk.Listbox(left, height=9, exportselection=False, width=36)
        self.course_list.pack(fill="y", expand=True)
        self.course_list.bind("<<ListboxSelect>>", lambda _e: self._show_keywords())
        self.course_list.bind("<Double-Button-1>", lambda _e: self.edit_course())
        buttons = ttk.Frame(left)
        buttons.pack(fill="x", pady=(4, 0))
        ttk.Button(buttons, text="Add", command=self.add_course).pack(side="left")
        ttk.Button(buttons, text="Edit", command=self.edit_course).pack(side="left")
        ttk.Button(buttons, text="Delete", command=self.delete_course).pack(side="left")

        right = ttk.Frame(courses)
        right.grid(row=0, column=1, sticky="nsew", padx=(8, 0))
        ttk.Label(right, text="Keywords (one per line, e.g. module code, short name, professor):").pack(anchor="w")
        self.keywords_text = tk.Text(right, height=8, width=30, wrap="word")
        self.keywords_text.pack(fill="both", expand=True)
        self.keywords_text.bind("<FocusOut>", lambda _e: self._store_keywords())
        self.folder_label = ttk.Label(right, text="", foreground="gray", wraplength=380)
        self.folder_label.pack(anchor="w")

        controls = ttk.Frame(outer)
        controls.pack(fill="x", pady=8)
        self.start_btn = ttk.Button(controls, text="▶ Start watching", command=self.toggle_watching)
        self.start_btn.pack(side="left")
        ttk.Button(controls, text="Sort existing files now", command=self.sort_now).pack(side="left", **pad)
        ttk.Button(controls, text="Save settings", command=self.save).pack(side="left")
        ttk.Label(controls, textvariable=self.status_var).pack(side="right")

        log_frame = ttk.LabelFrame(outer, text="Activity", padding=4)
        log_frame.pack(fill="both", expand=True)
        self.log = tk.Text(log_frame, height=8, state="disabled", wrap="none")
        scroll = ttk.Scrollbar(log_frame, command=self.log.yview)
        self.log.configure(yscrollcommand=scroll.set)
        self.log.pack(side="left", fill="both", expand=True)
        scroll.pack(side="right", fill="y")

    def _folder_row(self, parent, row: int, label: str, var: tk.StringVar, on_chosen=None) -> None:
        ttk.Label(parent, text=label).grid(row=row, column=0, sticky="w")
        ttk.Entry(parent, textvariable=var).grid(row=row, column=1, sticky="ew", padx=4)

        def browse() -> None:
            chosen = filedialog.askdirectory(initialdir=str(Path(var.get()).expanduser()))
            if chosen:
                var.set(chosen)
                if on_chosen:
                    on_chosen()

        ttk.Button(parent, text="Browse…", command=browse).grid(row=row, column=2)

    # ---------- modules ----------
    def _selected_index(self) -> int | None:
        sel = self.course_list.curselection()
        return sel[0] if sel else None

    def _sort_courses(self) -> None:
        self.config.courses.sort(key=lambda c: (c.subject.casefold(), c.name.casefold()))

    def _refresh_courses(self, select: Course | None = None) -> None:
        self.course_list.delete(0, "end")
        for course in self.config.courses:
            self.course_list.insert("end", course.label)
        if select is not None and select in self.config.courses:
            idx = self.config.courses.index(select)
            self.course_list.selection_set(idx)
            self.course_list.see(idx)
        # Callers already stored pending edits; don't write the old text into the new selection.
        self._current = None
        self._show_keywords()

    def _show_keywords(self) -> None:
        # Save edits for the previously selected module before switching.
        self._store_keywords()
        idx = self._selected_index()
        self._current = idx
        self.keywords_text.delete("1.0", "end")
        self.folder_label.configure(text="")
        if idx is not None:
            course = self.config.courses[idx]
            self.keywords_text.insert("1.0", "\n".join(course.keywords))
            self._collect_folders()
            self.folder_label.configure(text=f"Files go to: {self.config.course_folder(course)}")

    def _store_keywords(self) -> None:
        idx = self._current
        if idx is None or idx >= len(self.config.courses):
            return
        raw = self.keywords_text.get("1.0", "end")
        words = [w.strip() for line in raw.splitlines() for w in line.split(",") if w.strip()]
        self.config.courses[idx].keywords = words

    def _subjects(self) -> list[str]:
        subjects = {c.subject for c in self.config.courses if c.subject}
        root = Path(self.dest_var.get().strip()).expanduser()
        if root.is_dir():
            subjects.update(p.name for p in root.iterdir() if p.is_dir() and not p.name.startswith("."))
        return sorted(subjects, key=str.casefold)

    def add_course(self) -> None:
        self._store_keywords()
        dialog = CourseDialog(self.root, "Add module", self._subjects())
        self.root.wait_window(dialog)
        if not dialog.result:
            return
        subject, name = dialog.result
        course = Course(name=name, subject=subject, keywords=[])
        found = scan_study_folder(Path(self.dest_var.get().strip()))
        # Reuse the starter keywords an import would give this module, if its folder exists.
        for f in found:
            if f.subject.casefold() == subject.casefold() and f.name.casefold() == name.casefold():
                course.keywords = f.keywords
        self.config.courses.append(course)
        self._sort_courses()
        self._refresh_courses(select=course)
        self.keywords_text.focus_set()

    def edit_course(self) -> None:
        idx = self._selected_index()
        if idx is None:
            return
        self._store_keywords()
        course = self.config.courses[idx]
        dialog = CourseDialog(self.root, "Edit module", self._subjects(), course)
        self.root.wait_window(dialog)
        if dialog.result:
            course.subject, course.name = dialog.result
            self._sort_courses()
            self._refresh_courses(select=course)

    def delete_course(self) -> None:
        idx = self._selected_index()
        if idx is None:
            return
        label = self.config.courses[idx].label
        if messagebox.askyesno("Delete module", f"Remove '{label}'? (Its folder and files stay where they are.)"):
            del self.config.courses[idx]
            self._current = None
            self._refresh_courses()

    def import_modules(self, quiet: bool = False) -> None:
        self._store_keywords()
        root = Path(self.dest_var.get().strip()).expanduser()
        if not root.is_dir():
            if not quiet:
                messagebox.showerror("Folder not found", f"Study folder does not exist:\n{root}")
            return
        found = scan_study_folder(root)
        if not found:
            if not quiet:
                messagebox.showinfo("No modules found",
                                    "No module folders found.\nExpected: Study folder \\ Study program \\ Module")
            return
        added = merge_courses(self.config.courses, found)
        self._sort_courses()
        self._refresh_courses()
        self._log(f"Imported {added} module(s) from {root} ({len(found) - added} already known)")

    def _study_folder_chosen(self) -> None:
        if not self.config.courses:
            self.import_modules(quiet=True)

    # ---------- actions ----------
    def _collect_folders(self) -> None:
        self.config.watch_folder = self.watch_var.get().strip()
        self.config.destination_root = self.dest_var.get().strip()

    def _collect(self) -> Config:
        self._store_keywords()
        self._collect_folders()
        self.config.read_file_contents = self.contents_var.get()
        self.config.sort_existing_on_start = self.existing_var.get()
        return self.config

    def save(self) -> None:
        path = save_config(self._collect(), self.config_path)
        self._log(f"Settings saved to {path}")

    def _check_ready(self) -> bool:
        config = self._collect()
        if not config.courses:
            messagebox.showinfo("No modules", "Add or import at least one module first.")
            return False
        if not config.watch_path().is_dir():
            messagebox.showerror("Folder not found", f"Watch folder does not exist:\n{config.watch_path()}")
            return False
        return True

    def toggle_watching(self) -> None:
        if self.watcher and self.watcher.running:
            self.watcher.stop()
            self.watcher = None
            self.start_btn.configure(text="▶ Start watching")
            self.status_var.set("Stopped")
            self._log("Stopped watching.")
            return
        if not self._check_ready():
            return
        self.save()
        from .watcher import FolderWatcher

        self.watcher = FolderWatcher(self.config, on_result=self.results.put, on_message=self.results.put)
        try:
            self.watcher.start()
        except Exception as exc:
            messagebox.showerror("Could not start", str(exc))
            self.watcher = None
            return
        self.start_btn.configure(text="■ Stop")
        self.status_var.set("Watching…")
        self._log(f"Watching {self.config.watch_path()} – new downloads will be sorted automatically.")

    def sort_now(self) -> None:
        if not self._check_ready():
            return
        from .watcher import FolderWatcher

        if self.watcher and self.watcher.running:
            self.watcher.sort_existing_async()
        else:
            FolderWatcher(self.config, on_result=self.results.put).sort_existing_async()
        self._log("Sorting files already in the folder…")

    def _drain_results(self) -> None:
        # Results arrive from background threads; Tk must only be touched here.
        while True:
            try:
                item = self.results.get_nowait()
            except queue.Empty:
                break
            self._log(item if isinstance(item, str) else item.describe())
        self.root.after(300, self._drain_results)

    def _log(self, message: str) -> None:
        self.log.configure(state="normal")
        self.log.insert("end", message + "\n")
        self.log.see("end")
        self.log.configure(state="disabled")

    def on_close(self) -> None:
        if self.watcher:
            self.watcher.stop()
        self._collect()
        save_config(self.config, self.config_path)
        self.root.destroy()


def run_gui(config_path: Path) -> None:
    root = tk.Tk()
    App(root, config_path)
    root.mainloop()
