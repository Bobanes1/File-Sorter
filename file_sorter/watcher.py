"""Watch the downloads folder and sort new files as soon as they finish downloading."""

from __future__ import annotations

import logging
import threading
import time
from pathlib import Path
from typing import Callable

from watchdog.events import FileSystemEvent, FileSystemEventHandler
from watchdog.observers import Observer

from .config import Config
from .sorter import TEMP_SUFFIXES, SortResult, is_candidate, sort_existing, sort_file

log = logging.getLogger(__name__)

STABLE_SECONDS = 2.0
POLL_INTERVAL = 0.5
MAX_WAIT_SECONDS = 600
RESCAN_SECONDS = 10.0


def has_temp_sibling(path: Path) -> bool:
    """True while a browser is still writing `path` under a temporary name.

    Firefox creates an empty placeholder `x.pdf` and downloads into `x.pdf.part`;
    Chrome and Edge download into `x.pdf.crdownload`.
    """
    return any((path.parent / (path.name + suffix)).exists() for suffix in TEMP_SUFFIXES)


def wait_until_complete(path: Path, stable_for: float = STABLE_SECONDS, timeout: float = MAX_WAIT_SECONDS) -> bool:
    """Wait until the download is finished and the file can be opened.

    Returns False if the file vanished or never finished within `timeout`.
    """
    deadline = time.monotonic() + timeout
    last_size = -1
    stable_since = time.monotonic()
    while time.monotonic() < deadline:
        try:
            size = path.stat().st_size
        except FileNotFoundError:
            if not has_temp_sibling(path):
                return False
            # The browser may delete its placeholder just before renaming the temp file over it.
            size = 0
        now = time.monotonic()
        if size != last_size or size == 0 or has_temp_sibling(path):
            # Still growing, an empty placeholder, or the browser is still writing a temp file.
            last_size = size
            stable_since = now
        elif now - stable_since >= stable_for:
            try:
                # On Windows this fails while the browser still holds the file open.
                with open(path, "rb"):
                    return True
            except OSError:
                stable_since = now
        time.sleep(POLL_INTERVAL)
    return False


def _signature(path: Path) -> tuple[int, int] | None:
    try:
        st = path.stat()
    except OSError:
        return None
    return st.st_size, st.st_mtime_ns


class _Handler(FileSystemEventHandler):
    def __init__(self, enqueue: Callable[[Path], None]):
        self._enqueue = enqueue

    def on_created(self, event: FileSystemEvent) -> None:
        if not event.is_directory:
            self._enqueue(Path(event.src_path))

    def on_moved(self, event: FileSystemEvent) -> None:
        # Browsers rename "x.pdf.part" / "x.pdf.crdownload" to "x.pdf" when done.
        if not event.is_directory:
            self._enqueue(Path(event.dest_path))


class FolderWatcher:
    """Watches a folder and sorts each new file once its download has finished.

    Every new file gets a short-lived thread that waits for the download to finish,
    so one slow download never blocks the others. Besides filesystem events, the
    folder is rescanned every few seconds in case an event got lost.
    """

    def __init__(self, config: Config,
                 on_result: Callable[[SortResult], None] | None = None,
                 on_message: Callable[[str], None] | None = None,
                 stable_for: float = STABLE_SECONDS,
                 rescan_interval: float = RESCAN_SECONDS):
        self.config = config
        self.on_result = on_result or (lambda r: None)
        self.on_message = on_message or (lambda m: None)
        self.stable_for = stable_for
        self.rescan_interval = rescan_interval
        self._pending: set[str] = set()
        # Files the rescan should skip while unchanged: present at start, or already left in place.
        self._ignore: dict[str, tuple[int, int] | None] = {}
        self._lock = threading.Lock()
        # Moves happen one at a time so two files can't race for the same name.
        self._move_lock = threading.Lock()
        self._stop = threading.Event()
        self._observer: Observer | None = None
        self._folder: Path | None = None

    @property
    def running(self) -> bool:
        return self._observer is not None

    def start(self) -> None:
        if self.running:
            return
        folder = self.config.watch_path()
        if not folder.is_dir():
            raise FileNotFoundError(f"Watch folder does not exist: {folder}")
        self._folder = folder.resolve()
        self._stop.clear()
        if not self.config.sort_existing_on_start:
            self._ignore = {p.name: _signature(p) for p in self._candidates()}
        self._observer = Observer()
        self._observer.schedule(_Handler(self._enqueue), str(folder), recursive=False)
        self._observer.start()
        threading.Thread(target=self._rescan_loop, name="file-sorter-rescan", daemon=True).start()
        if self.config.sort_existing_on_start:
            self.sort_existing_async()

    def stop(self) -> None:
        self._stop.set()
        if self._observer is not None:
            self._observer.stop()
            self._observer.join(timeout=5)
            self._observer = None

    def sort_existing_async(self) -> None:
        def run() -> None:
            with self._move_lock:
                results = sort_existing(self.config)
            for result in results:
                self._report(result)
        threading.Thread(target=run, name="file-sorter-existing", daemon=True).start()

    def rescan(self) -> None:
        """Queue new files that no filesystem event told us about."""
        for path in self._candidates():
            name = path.name
            if name in self._ignore and self._ignore[name] == _signature(path):
                continue
            self._enqueue(path)

    def _candidates(self) -> list[Path]:
        folder = self._folder or self.config.watch_path()
        try:
            return [p for p in folder.iterdir() if p.is_file() and is_candidate(p, self.config)]
        except OSError:
            return []

    def _rescan_loop(self) -> None:
        while not self._stop.wait(self.rescan_interval):
            try:
                self.rescan()
            except Exception:
                log.exception("Rescan failed")

    def _enqueue(self, path: Path) -> None:
        # Only files sitting directly in the watch folder; ignore temp files early.
        if self._folder is None or path.parent.resolve() != self._folder:
            return
        if not is_candidate(path, self.config):
            return
        name = path.name
        with self._lock:
            if name in self._pending:
                return
            self._pending.add(name)
        self._message(f"Detected {name} – waiting for the download to finish…")
        threading.Thread(target=self._handle, args=(self._folder / name,),
                         name="file-sorter-file", daemon=True).start()

    def _handle(self, path: Path) -> None:
        try:
            if wait_until_complete(path, stable_for=self.stable_for) and self.running:
                with self._move_lock:
                    result = sort_file(path, self.config)
                if result.moved:
                    self._ignore.pop(path.name, None)
                else:
                    self._ignore[path.name] = _signature(path)
                self._report(result)
        except Exception as exc:
            log.exception("Failed to sort %s", path)
            self._ignore[path.name] = _signature(path)
            self._report(SortResult(path, False, reason=f"error: {exc}"))
        finally:
            with self._lock:
                self._pending.discard(path.name)

    def _report(self, result: SortResult) -> None:
        log.info(result.describe())
        self.on_result(result)

    def _message(self, text: str) -> None:
        log.info(text)
        self.on_message(text)
