"""Small Tkinter window for managing courses and starting/stopping the sorter."""

from __future__ import annotations

import queue
import tkinter as tk
from pathlib import Path
from tkinter import filedialog, messagebox, simpledialog, ttk

from .config import Config, Course, load_config, save_config
from .sorter import SortResult


class App:
    def __init__(self, root: tk.Tk, config_path: Path):
        self.root = root
        self.config_path = config_path
        self.config = load_config(config_path)
        self.watcher = None
        self.results: queue.Queue[SortResult] = queue.Queue()

        root.title("File Sorter")
        root.minsize(640, 520)
        root.protocol("WM_DELETE_WINDOW", self.on_close)

        self.watch_var = tk.StringVar(value=self.config.watch_folder)
        self.dest_var = tk.StringVar(value=self.config.destination_root)
        self.contents_var = tk.BooleanVar(value=self.config.read_file_contents)
        self.existing_var = tk.BooleanVar(value=self.config.sort_existing_on_start)
        self.status_var = tk.StringVar(value="Stopped")

        self._current: int | None = None
        self._build()
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
        self._folder_row(folders, 1, "Course folders in:", self.dest_var)
        ttk.Checkbutton(folders, text="Read text inside PDFs / slides / Word files",
                        variable=self.contents_var).grid(row=2, column=0, columnspan=3, sticky="w")
        ttk.Checkbutton(folders, text="Also sort files already in the folder when starting",
                        variable=self.existing_var).grid(row=3, column=0, columnspan=3, sticky="w")

        courses = ttk.LabelFrame(outer, text="Courses", padding=8)
        courses.pack(fill="both", expand=True, pady=(8, 0))
        courses.columnconfigure(1, weight=1)
        courses.rowconfigure(0, weight=1)

        left = ttk.Frame(courses)
        left.grid(row=0, column=0, sticky="ns")
        self.course_list = tk.Listbox(left, height=8, exportselection=False, width=28)
        self.course_list.pack(fill="y", expand=True)
        self.course_list.bind("<<ListboxSelect>>", lambda _e: self._show_keywords())
        buttons = ttk.Frame(left)
        buttons.pack(fill="x", pady=(4, 0))
        ttk.Button(buttons, text="Add", command=self.add_course).pack(side="left")
        ttk.Button(buttons, text="Rename", command=self.rename_course).pack(side="left")
        ttk.Button(buttons, text="Delete", command=self.delete_course).pack(side="left")

        right = ttk.Frame(courses)
        right.grid(row=0, column=1, sticky="nsew", padx=(8, 0))
        ttk.Label(right, text="Keywords (one per line, e.g. course name, code, professor):").pack(anchor="w")
        self.keywords_text = tk.Text(right, height=8, width=30, wrap="word")
        self.keywords_text.pack(fill="both", expand=True)
        self.keywords_text.bind("<FocusOut>", lambda _e: self._store_keywords())

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

    def _folder_row(self, parent, row: int, label: str, var: tk.StringVar) -> None:
        ttk.Label(parent, text=label).grid(row=row, column=0, sticky="w")
        ttk.Entry(parent, textvariable=var).grid(row=row, column=1, sticky="ew", padx=4)

        def browse() -> None:
            chosen = filedialog.askdirectory(initialdir=str(Path(var.get()).expanduser()))
            if chosen:
                var.set(chosen)

        ttk.Button(parent, text="Browse…", command=browse).grid(row=row, column=2)

    # ---------- courses ----------
    def _selected_index(self) -> int | None:
        sel = self.course_list.curselection()
        return sel[0] if sel else None

    def _refresh_courses(self, select: int | None = None) -> None:
        self.course_list.delete(0, "end")
        for course in self.config.courses:
            self.course_list.insert("end", course.name)
        if select is not None and 0 <= select < len(self.config.courses):
            self.course_list.selection_set(select)
        # Callers already stored pending edits; don't write the old text into the new selection.
        self._current = None
        self._show_keywords()

    def _show_keywords(self) -> None:
        # Save edits for the previously selected course before switching.
        self._store_keywords()
        idx = self._selected_index()
        self._current = idx
        self.keywords_text.delete("1.0", "end")
        if idx is not None:
            self.keywords_text.insert("1.0", "\n".join(self.config.courses[idx].keywords))

    def _store_keywords(self) -> None:
        idx = self._current
        if idx is None or idx >= len(self.config.courses):
            return
        raw = self.keywords_text.get("1.0", "end")
        words = [w.strip() for line in raw.splitlines() for w in line.split(",") if w.strip()]
        self.config.courses[idx].keywords = words

    def add_course(self) -> None:
        name = simpledialog.askstring("Add course", "Course folder name (e.g. Programming Course):",
                                      parent=self.root)
        if not name or not name.strip():
            return
        self._store_keywords()
        name = name.strip()
        # Start with the words of the course name as keywords; user can refine them.
        starter = [w for w in name.replace("-", " ").split() if len(w) > 3 and w.lower() != "course"]
        self.config.courses.append(Course(name=name, keywords=starter))
        self._current = None
        self._refresh_courses(select=len(self.config.courses) - 1)
        self.keywords_text.focus_set()

    def rename_course(self) -> None:
        idx = self._selected_index()
        if idx is None:
            return
        course = self.config.courses[idx]
        name = simpledialog.askstring("Rename course", "New name:", initialvalue=course.name,
                                      parent=self.root)
        if name and name.strip():
            self._store_keywords()
            course.name = name.strip()
            self._current = None
            self._refresh_courses(select=idx)

    def delete_course(self) -> None:
        idx = self._selected_index()
        if idx is None:
            return
        name = self.config.courses[idx].name
        if messagebox.askyesno("Delete course", f"Remove '{name}'? (Files already sorted stay where they are.)"):
            del self.config.courses[idx]
            self._current = None
            self._refresh_courses()

    # ---------- actions ----------
    def _collect(self) -> Config:
        self._store_keywords()
        self.config.watch_folder = self.watch_var.get().strip()
        self.config.destination_root = self.dest_var.get().strip()
        self.config.read_file_contents = self.contents_var.get()
        self.config.sort_existing_on_start = self.existing_var.get()
        return self.config

    def save(self) -> None:
        path = save_config(self._collect(), self.config_path)
        self._log(f"Settings saved to {path}")

    def _check_ready(self) -> bool:
        config = self._collect()
        if not config.courses:
            messagebox.showinfo("No courses", "Add at least one course first.")
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

        self.watcher = FolderWatcher(self.config, on_result=self.results.put)
        try:
            self.watcher.start()
        except Exception as exc:
            messagebox.showerror("Could not start", str(exc))
            self.watcher = None
            return
        self.start_btn.configure(text="■ Stop")
        self.status_var.set("Watching…")
        self._log(f"Watching {self.config.watch_path()}")

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
                result = self.results.get_nowait()
            except queue.Empty:
                break
            self._log(result.describe())
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
