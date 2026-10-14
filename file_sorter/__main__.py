"""Entry point: `python -m file_sorter` opens the window, `--headless` just watches."""

from __future__ import annotations

import argparse
import logging
import signal
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

    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(message)s", datefmt="%H:%M:%S")
    config_path = args.config or default_config_path()

    if args.sort_now or args.headless:
        config = load_config(config_path)
        if not config.courses:
            print(f"No courses configured yet. Run without --headless to set them up "
                  f"(settings file: {config_path}).")
            return 1
        if args.sort_now:
            from .sorter import sort_existing
            for result in sort_existing(config):
                print(result.describe())
            return 0
        return _run_headless(config)

    from .gui import run_gui
    run_gui(config_path)
    return 0


def _run_headless(config) -> int:
    from .watcher import FolderWatcher

    watcher = FolderWatcher(config, on_result=lambda r: logging.info(r.describe()))
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
