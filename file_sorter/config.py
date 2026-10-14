"""Loading and saving the user's settings (watch folder, courses, keywords)."""

from __future__ import annotations

import json
import os
import sys
from dataclasses import asdict, dataclass, field
from pathlib import Path

DEFAULT_EXTENSIONS = [
    ".pdf", ".pptx", ".ppt", ".docx", ".doc", ".key", ".odp", ".odt",
    ".txt", ".md", ".zip", ".ipynb", ".xlsx",
]


def default_config_path() -> Path:
    """Per-user location of config.json, following each OS's conventions."""
    if sys.platform.startswith("win"):
        base = Path(os.environ.get("APPDATA", Path.home() / "AppData" / "Roaming"))
        return base / "FileSorter" / "config.json"
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Application Support" / "FileSorter" / "config.json"
    base = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
    return base / "file-sorter" / "config.json"


def default_downloads() -> str:
    return str(Path.home() / "Downloads")


def default_destination() -> str:
    return str(Path.home() / "University")


@dataclass
class Course:
    name: str
    keywords: list[str] = field(default_factory=list)
    # Optional custom folder; when empty the course goes to destination_root/[subject/]name.
    folder: str = ""
    # Study program the module belongs to (a folder directly under destination_root).
    subject: str = ""

    @property
    def label(self) -> str:
        return f"{self.subject} › {self.name}" if self.subject else self.name


@dataclass
class Config:
    watch_folder: str = field(default_factory=default_downloads)
    destination_root: str = field(default_factory=default_destination)
    extensions: list[str] = field(default_factory=lambda: list(DEFAULT_EXTENSIONS))
    read_file_contents: bool = True
    sort_existing_on_start: bool = False
    courses: list[Course] = field(default_factory=list)

    def watch_path(self) -> Path:
        return Path(self.watch_folder).expanduser()

    def course_folder(self, course: Course) -> Path:
        if course.folder:
            return Path(course.folder).expanduser()
        root = Path(self.destination_root).expanduser()
        if course.subject:
            root = root / _safe_folder_name(course.subject)
        return root / _safe_folder_name(course.name)

    def to_dict(self) -> dict:
        return asdict(self)

    @classmethod
    def from_dict(cls, data: dict) -> "Config":
        defaults = cls()
        courses = [
            Course(
                name=str(c.get("name", "")).strip(),
                keywords=[str(k).strip() for k in c.get("keywords", []) if str(k).strip()],
                folder=str(c.get("folder", "") or ""),
                subject=str(c.get("subject", "") or "").strip(),
            )
            for c in data.get("courses", [])
            if str(c.get("name", "")).strip()
        ]
        extensions = data.get("extensions", defaults.extensions)
        return cls(
            watch_folder=data.get("watch_folder", defaults.watch_folder),
            destination_root=data.get("destination_root", defaults.destination_root),
            extensions=[_normalise_ext(e) for e in extensions],
            read_file_contents=bool(data.get("read_file_contents", True)),
            sort_existing_on_start=bool(data.get("sort_existing_on_start", False)),
            courses=courses,
        )


def _normalise_ext(ext: str) -> str:
    ext = ext.strip().lower()
    return ext if ext.startswith(".") else "." + ext


def _safe_folder_name(name: str) -> str:
    """Strip characters that are not allowed in folder names on Windows/macOS/Linux."""
    cleaned = "".join("_" if ch in '<>:"/\\|?*' else ch for ch in name).strip().rstrip(".")
    return cleaned or "Unnamed Course"


def load_config(path: Path | None = None) -> Config:
    path = path or default_config_path()
    if not path.exists():
        return Config()
    # utf-8-sig: Notepad and PowerShell may add a byte-order mark when editing by hand.
    with open(path, encoding="utf-8-sig") as fh:
        return Config.from_dict(json.load(fh))


def save_config(config: Config, path: Path | None = None) -> Path:
    path = path or default_config_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(config.to_dict(), fh, indent=2, ensure_ascii=False)
    os.replace(tmp, path)
    return path


# Words that say nothing about which module a file belongs to.
STOPWORDS = {
    "und", "oder", "der", "die", "das", "den", "dem", "des", "ein", "eine", "fur", "mit", "von",
    "zur", "zum", "einfuhrung", "grundlagen", "vorlesung", "ubung", "ubungen", "seminar", "modul",
    "kurs", "praktikum", "teil", "and", "the", "for", "with", "from", "introduction", "intro",
    "basics", "fundamentals", "lecture", "course", "module", "seminar", "part",
}


# Short filler words dropped when building a module's core name ("Einführung in die Analysis I" → "analysis i").
FILLER_WORDS = {"in", "im", "zu", "an", "am", "auf", "of", "to", "on", "a"}


def _core_name(name: str) -> str:
    from .matcher import normalise

    return " ".join(w for w in normalise(name).split() if w not in STOPWORDS and w not in FILLER_WORDS)


def _name_words(name: str) -> list[str]:
    from .matcher import normalise

    return [w for w in normalise(name).split() if len(w) > 3 and w not in STOPWORDS and not w.isdigit()]


def scan_study_folder(root: Path) -> list[Course]:
    """Find module folders laid out as root/<subject>/<module>.

    Each module gets the significant words of its name as starter keywords; words
    shared by several modules (e.g. "Analysis" in "Analysis I" and "Analysis II")
    are left out because they can't tell the modules apart. The full module name
    is matched anyway, which separates those, and so is its core name without
    filler words ("Einführung in die Analysis I" → "analysis i").
    """
    from .matcher import normalise

    root = Path(root).expanduser()
    if not root.is_dir():
        return []
    found: list[Course] = []
    for subject_dir in sorted(p for p in root.iterdir() if p.is_dir() and not p.name.startswith(".")):
        for module_dir in sorted(p for p in subject_dir.iterdir() if p.is_dir() and not p.name.startswith(".")):
            found.append(Course(name=module_dir.name, subject=subject_dir.name))

    counts: dict[str, int] = {}
    cores: dict[str, int] = {}
    for course in found:
        for word in set(_name_words(course.name)):
            counts[word] = counts.get(word, 0) + 1
        core = _core_name(course.name)
        cores[core] = cores.get(core, 0) + 1
    for course in found:
        keywords = [w for w in dict.fromkeys(_name_words(course.name)) if counts[w] == 1]
        # The name without filler words, if that differs from the full name and is unique.
        core = _core_name(course.name)
        if core and core not in keywords and cores[core] == 1 and core != normalise(course.name):
            keywords.insert(0, core)
        course.keywords = keywords
    return found


def merge_courses(existing: list[Course], found: list[Course]) -> int:
    """Add modules from `found` that aren't in `existing` yet. Returns how many were added."""
    known = {(c.subject.casefold(), c.name.casefold()) for c in existing}
    added = 0
    for course in found:
        key = (course.subject.casefold(), course.name.casefold())
        if key not in known:
            existing.append(course)
            known.add(key)
            added += 1
    return added
