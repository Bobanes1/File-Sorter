"""Entry point: `python -m file_sorter` opens the window, `--headless` just watches."""

from __future__ import annotations

import argparse
import logging
import logging.handlers
import signal
import sys
import threading
from pathlib import Path

from .config import default_config_path, load_config


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="file_sorter", description=__doc__)
    parser.add_argument("--config", type=Path, default=None,
                        help=f"settings file (default: {default_config_path()})")
    parser.add_argument("--headless", action="store_true",
                        help="run without a window, only watching the folder")
    parser.add_argument("--sort-now", action="store_true",
                        help="sort files already in the watch folder once, then exit")
    args = parser.parse_args(argv)

    config_path = args.config or default_config_path()
    _setup_logging(config_path.parent / "file-sorter.log")

    if args.sort_now or args.headless:
        config = load_config(config_path)
        if not config.courses:
            print(f"No courses configured yet. Run without --headless to set them up "
                  f"(settings file: {config_path}).")
            return 1
        if args.sort_now:
            from .sorter import sort_existing
            for result in sort_existing(config):
                logging.info(result.describe())
            return 0
        return _run_headless(config)

    from .gui import run_gui
    run_gui(config_path)
    return 0


def _setup_logging(log_file: Path) -> None:
    """Log to the console when there is one, and always to a file next to the settings.

    The Windows exe has no console, so the file is the only trace of what happened.
    """
    fmt = logging.Formatter("%(asctime)s %(message)s", datefmt="%Y-%m-%d %H:%M:%S")
    root = logging.getLogger()
    root.setLevel(logging.INFO)
    if sys.stderr is not None:
        console = logging.StreamHandler()
        console.setFormatter(logging.Formatter("%(asctime)s %(message)s", datefmt="%H:%M:%S"))
        root.addHandler(console)
    try:
        log_file.parent.mkdir(parents=True, exist_ok=True)
        handler = logging.handlers.RotatingFileHandler(log_file, maxBytes=1_000_000, backupCount=1,
                                                       encoding="utf-8")
        handler.setFormatter(fmt)
        root.addHandler(handler)
    except OSError:
        pass


def _run_headless(config) -> int:
    from .watcher import FolderWatcher

    watcher = FolderWatcher(config)  # results are logged by the watcher itself
    try:
        watcher.start()
    except FileNotFoundError as exc:
        print(exc)
        return 1
    logging.info("Watching %s (Ctrl+C to stop)", config.watch_path())
    stop = threading.Event()
    signal.signal(signal.SIGINT, lambda *_: stop.set())
    signal.signal(signal.SIGTERM, lambda *_: stop.set())
    while not stop.wait(0.5):
        pass
    watcher.stop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
