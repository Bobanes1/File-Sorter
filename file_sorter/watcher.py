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
from .sorter import SortResult, is_candidate, sort_existing, sort_file

log = logging.getLogger(__name__)

STABLE_SECONDS = 2.0
POLL_INTERVAL = 0.5
MAX_WAIT_SECONDS = 600


def wait_until_complete(path: Path, stable_for: float = STABLE_SECONDS, timeout: float = MAX_WAIT_SECONDS) -> bool:
    """Wait until the file stops growing and can be opened. False if it vanished or timed out."""
    deadline = time.monotonic() + timeout
    last_size = -1
    stable_since = time.monotonic()
    while time.monotonic() < deadline:
        try:
            size = path.stat().st_size
        except FileNotFoundError:
            return False
        now = time.monotonic()
        if size != last_size:
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


class _Handler(FileSystemEventHandler):
    def __init__(self, enqueue: Callable[[Path], None]):
        self._enqueue = enqueue

    def on_created(self, event: FileSystemEvent) -> None:
        if not event.is_directory:
            self._enqueue(Path(event.src_path))

    def on_moved(self, event: FileSystemEvent) -> None:
        # Browsers download to "x.pdf.crdownload" and rename to "x.pdf" when done.
        if not event.is_directory:
            self._enqueue(Path(event.dest_path))


class FolderWatcher:
    """Runs a watchdog observer; each new file gets a short-lived thread that waits
    for the download to finish, so one slow download never blocks the others."""

    def __init__(self, config: Config, on_result: Callable[[SortResult], None] | None = None,
                 stable_for: float = STABLE_SECONDS):
        self.config = config
        self.on_result = on_result or (lambda r: log.info(r.describe()))
        self.stable_for = stable_for
        self._pending: set[Path] = set()
        self._lock = threading.Lock()
        # Moves happen one at a time so two files can't race for the same name.
        self._move_lock = threading.Lock()
        self._observer: Observer | None = None

    @property
    def running(self) -> bool:
        return self._observer is not None

    def start(self) -> None:
        if self.running:
            return
        folder = self.config.watch_path()
        if not folder.is_dir():
            raise FileNotFoundError(f"Watch folder does not exist: {folder}")
        self._observer = Observer()
        self._observer.schedule(_Handler(self._enqueue), str(folder), recursive=False)
        self._observer.start()
        if self.config.sort_existing_on_start:
            self.sort_existing_async()

    def stop(self) -> None:
        if self._observer is not None:
            self._observer.stop()
            self._observer.join(timeout=5)
            self._observer = None

    def sort_existing_async(self) -> None:
        def run() -> None:
            with self._move_lock:
                results = sort_existing(self.config)
            for result in results:
                self.on_result(result)
        threading.Thread(target=run, name="file-sorter-existing", daemon=True).start()

    def _enqueue(self, path: Path) -> None:
        # Only files sitting directly in the watch folder; ignore temp files early.
        if path.parent.resolve() != self.config.watch_path().resolve():
            return
        if not is_candidate(path, self.config):
            return
        with self._lock:
            if path in self._pending:
                return
            self._pending.add(path)
        threading.Thread(target=self._handle, args=(path,), name="file-sorter-file", daemon=True).start()

    def _handle(self, path: Path) -> None:
        try:
            if wait_until_complete(path, stable_for=self.stable_for) and self.running:
                with self._move_lock:
                    result = sort_file(path, self.config)
                self.on_result(result)
        except Exception as exc:
            log.exception("Failed to sort %s", path)
            self.on_result(SortResult(path, False, reason=f"error: {exc}"))
        finally:
            with self._lock:
                self._pending.discard(path)
