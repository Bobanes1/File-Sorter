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
    # Optional custom folder; when empty the course goes to destination_root/name.
    folder: str = ""


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
        return Path(self.destination_root).expanduser() / _safe_folder_name(course.name)

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
    with open(path, encoding="utf-8") as fh:
        return Config.from_dict(json.load(fh))


def save_config(config: Config, path: Path | None = None) -> Path:
    path = path or default_config_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(config.to_dict(), fh, indent=2, ensure_ascii=False)
    os.replace(tmp, path)
    return path
