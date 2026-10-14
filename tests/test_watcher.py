import os
import threading
import time
from pathlib import Path

import pytest
from conftest import make_pdf, make_pptx

from file_sorter.watcher import FolderWatcher, has_temp_sibling, wait_until_complete


def _wait_for(predicate, timeout=15.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if predicate():
            return True
        time.sleep(0.1)
    return False


@pytest.fixture
def watcher(config):
    results = []
    messages = []
    w = FolderWatcher(config, on_result=results.append, on_message=messages.append,
                      stable_for=0.3, rescan_interval=0.5)
    w.results, w.messages = results, messages
    yield w
    w.stop()


def firefox_download(target: Path, payload: bytes, seconds: float = 2.0) -> None:
    """Mimic Firefox: empty placeholder + growing .part file, then rename over the placeholder."""
    target.write_bytes(b"")
    part = target.with_name(target.name + ".part")
    with open(part, "wb") as fh:
        steps = 10
        step = len(payload) // steps + 1
        for i in range(0, len(payload), step):
            fh.write(payload[i:i + step])
            fh.flush()
            time.sleep(seconds / steps)
    os.replace(part, target)


def test_temp_sibling_detection(tmp_path):
    f = tmp_path / "a.pdf"
    f.write_bytes(b"x")
    assert not has_temp_sibling(f)
    (tmp_path / "a.pdf.part").write_bytes(b"")
    assert has_temp_sibling(f)


def test_wait_ignores_empty_placeholder(tmp_path):
    f = tmp_path / "a.pdf"
    f.write_bytes(b"")
    assert not wait_until_complete(f, stable_for=0.2, timeout=1.5)


def test_firefox_download_sorted_once_and_complete(config, watcher, tmp_path):
    payload = make_pdf(tmp_path / "src.pdf", "Week 3").read_bytes() + b"%" + b"x" * 50_000
    watcher.start()
    downloads = config.watch_path()
    thread = threading.Thread(target=firefox_download, args=(downloads / "prog1_vl3.pdf", payload))
    thread.start()
    thread.join()

    folder = Path(config.destination_root) / "Programming Course"
    assert _wait_for(lambda: (folder / "prog1_vl3.pdf").exists())
    time.sleep(1.5)  # give a wrong duplicate the chance to appear
    assert [p.name for p in folder.iterdir()] == ["prog1_vl3.pdf"]
    assert (folder / "prog1_vl3.pdf").read_bytes() == payload
    assert list(downloads.iterdir()) == []
    assert any("Detected prog1_vl3.pdf" in m for m in watcher.messages)


def test_firefox_download_matched_by_content(config, watcher, tmp_path):
    payload = make_pptx(tmp_path / "src.pptx", "Statistics: Probability").read_bytes()
    watcher.start()
    firefox_download(config.watch_path() / "lecture7.pptx", payload, seconds=1.0)
    assert _wait_for(lambda: (Path(config.destination_root) / "Statistics" / "lecture7.pptx").exists())


def test_short_lived_file_not_announced(config, watcher):
    watcher.start()
    tmp = config.watch_path() / "Qd_ZJYyR.pdf"
    tmp.write_bytes(b"")
    time.sleep(0.05)
    tmp.unlink()
    time.sleep(1.0)
    assert not any("Qd_ZJYyR" in m for m in watcher.messages)


def test_chrome_style_rename_and_unmatched_file(config, watcher):
    watcher.start()
    w = config.watch_path()
    make_pdf(w / "python_intro.pdf", "x")
    tmp = make_pptx(w / "lecture7.pptx.crdownload", "Statistics: Probability")
    time.sleep(0.8)
    assert tmp.exists(), "partial download must not be touched"
    tmp.rename(w / "lecture7.pptx")
    make_pdf(w / "shopping.pdf", "milk")

    uni = Path(config.destination_root)
    assert _wait_for(lambda: (uni / "Programming Course" / "python_intro.pdf").exists())
    assert _wait_for(lambda: (uni / "Statistics" / "lecture7.pptx").exists())
    assert _wait_for(lambda: any(r.source.name == "shopping.pdf" for r in watcher.results))
    assert (w / "shopping.pdf").exists()


def test_rescan_finds_files_without_events(config):
    """Files present at start are left alone; new ones are found even without an event."""
    w = config.watch_path()
    make_pdf(w / "old_python.pdf", "x")
    results = []
    watcher = FolderWatcher(config, on_result=results.append, stable_for=0.2, rescan_interval=3600)
    watcher.start()
    watcher._observer.stop()  # simulate lost events: only the rescan is left
    try:
        make_pdf(w / "new_python.pdf", "x")
        make_pdf(w / "unrelated.pdf", "x")
        watcher.rescan()
        uni = Path(config.destination_root) / "Programming Course"
        assert _wait_for(lambda: (uni / "new_python.pdf").exists())
        assert (w / "old_python.pdf").exists()

        def reports():
            return sum(r.source.name == "unrelated.pdf" for r in results)

        assert _wait_for(lambda: reports() == 1)
        watcher.rescan()
        time.sleep(0.8)
        assert reports() == 1, "unmatched file re-reported"
    finally:
        watcher.stop()


def test_sort_existing_on_start(config):
    config.sort_existing_on_start = True
    make_pdf(config.watch_path() / "stats_old.pdf", "x")
    watcher = FolderWatcher(config, stable_for=0.2)
    watcher.start()
    try:
        assert _wait_for(lambda: (Path(config.destination_root) / "Statistics" / "stats_old.pdf").exists())
    finally:
        watcher.stop()
