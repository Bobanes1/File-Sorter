"""Match a single file to a course and move it into that course's folder."""

from __future__ import annotations

import shutil
from dataclasses import dataclass
from pathlib import Path

from .config import Config
from .matcher import match_file

# Partial downloads written by browsers before the file is complete.
TEMP_SUFFIXES = {".crdownload", ".part", ".partial", ".download", ".tmp", ".temp", ".opdownload"}


@dataclass
class SortResult:
    source: Path
    moved: bool
    destination: Path | None = None
    course: str | None = None
    reason: str = ""

    def describe(self) -> str:
        if self.moved:
            return f"Moved {self.source.name} → {self.course} ({self.reason})"
        return f"Left {self.source.name} in place – {self.reason}"


def is_candidate(path: Path, config: Config) -> bool:
    """True for finished files with an extension the user wants sorted."""
    name = path.name
    if name.startswith((".", "~$")):
        return False
    if path.suffix.lower() in TEMP_SUFFIXES:
        return False
    return path.suffix.lower() in set(config.extensions)


def unique_destination(folder: Path, filename: str) -> Path:
    """folder/filename, or folder/'name (1).ext' etc. if that already exists."""
    target = folder / filename
    if not target.exists():
        return target
    stem, suffix = Path(filename).stem, Path(filename).suffix
    n = 1
    while True:
        candidate = folder / f"{stem} ({n}){suffix}"
        if not candidate.exists():
            return candidate
        n += 1


def sort_file(path: Path, config: Config) -> SortResult:
    path = Path(path)
    if not path.is_file():
        return SortResult(path, False, reason="file no longer exists")
    if not is_candidate(path, config):
        return SortResult(path, False, reason="not a course file type")

    match = match_file(path, config.courses, read_contents=config.read_file_contents)
    if match is None:
        return SortResult(path, False, reason="no matching course")

    folder = config.course_folder(match.course)
    if folder.resolve() == path.parent.resolve():
        return SortResult(path, False, course=match.course.label, reason="already in course folder")
    folder.mkdir(parents=True, exist_ok=True)
    target = unique_destination(folder, path.name)
    shutil.move(str(path), str(target))
    return SortResult(path, True, destination=target, course=match.course.label, reason=match.reason)


def sort_existing(config: Config) -> list[SortResult]:
    """Sort every file currently sitting in the watch folder."""
    folder = config.watch_path()
    if not folder.is_dir():
        return []
    results = []
    for entry in sorted(folder.iterdir()):
        if entry.is_file() and is_candidate(entry, config):
            try:
                results.append(sort_file(entry, config))
            except OSError as exc:
                results.append(SortResult(entry, False, reason=f"error: {exc}"))
    return results
