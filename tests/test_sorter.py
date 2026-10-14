import json
import time
from pathlib import Path

from conftest import make_pdf, make_pptx

from file_sorter.config import Config, load_config, save_config
from file_sorter.sorter import is_candidate, sort_existing, sort_file, unique_destination
from file_sorter.watcher import FolderWatcher


def test_moves_matching_file(config):
    src = make_pdf(config.watch_path() / "prog1_slides.pdf", "Week 1")
    result = sort_file(src, config)
    assert result.moved
    assert result.destination == Path(config.destination_root) / "Programming Course" / "prog1_slides.pdf"
    assert result.destination.exists() and not src.exists()


def test_sorts_by_content(config):
    src = make_pptx(config.watch_path() / "lecture5.pptx", "Linear Algebra - Lecture 5")
    result = sort_file(src, config)
    assert result.course == "Linear Algebra"


def test_unmatched_file_stays(config):
    src = make_pdf(config.watch_path() / "holiday.pdf", "Beach trip")
    result = sort_file(src, config)
    assert not result.moved and src.exists()


def test_name_collision_gets_number(tmp_path):
    (tmp_path / "a.pdf").write_text("x")
    (tmp_path / "a (1).pdf").write_text("x")
    assert unique_destination(tmp_path, "a.pdf").name == "a (2).pdf"


def test_duplicate_download_not_overwritten(config):
    folder = Path(config.destination_root) / "Programming Course"
    folder.mkdir(parents=True)
    (folder / "prog1.pdf").write_text("old")
    src = make_pdf(config.watch_path() / "prog1.pdf", "new")
    result = sort_file(src, config)
    assert result.destination.name == "prog1 (1).pdf"
    assert (folder / "prog1.pdf").read_text() == "old"


def test_temp_and_unwanted_files_ignored(config):
    w = config.watch_path()
    assert not is_candidate(w / "prog1.pdf.crdownload", config)
    assert not is_candidate(w / "prog1.pdf.part", config)
    assert not is_candidate(w / "~$prog1.docx", config)
    assert not is_candidate(w / ".prog1.pdf", config)
    assert not is_candidate(w / "prog1.exe", config)
    assert is_candidate(w / "prog1.PDF", config)


def test_sort_existing(config):
    make_pdf(config.watch_path() / "stats_hw.pdf", "x")
    make_pdf(config.watch_path() / "random.pdf", "nothing")
    results = {r.source.name: r for r in sort_existing(config)}
    assert results["stats_hw.pdf"].course == "Statistics"
    assert not results["random.pdf"].moved


def test_config_roundtrip(tmp_path, config):
    path = save_config(config, tmp_path / "cfg" / "config.json")
    loaded = load_config(path)
    assert loaded == config
    assert json.loads(path.read_text())["courses"][0]["name"] == "Programming Course"


def test_missing_config_gives_defaults(tmp_path):
    assert load_config(tmp_path / "nope.json") == Config()


def test_custom_course_folder(config, tmp_path):
    config.courses[2].folder = str(tmp_path / "Elsewhere" / "Stats")
    src = make_pdf(config.watch_path() / "stats.pdf", "x")
    assert sort_file(src, config).destination.parent == tmp_path / "Elsewhere" / "Stats"


def _wait_for(predicate, timeout=10.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if predicate():
            return True
        time.sleep(0.1)
    return False


def test_watcher_sorts_new_and_renamed_downloads(config):
    results = []
    watcher = FolderWatcher(config, on_result=results.append, stable_for=0.3)
    watcher.start()
    try:
        w = config.watch_path()
        make_pdf(w / "python_intro.pdf", "x")
        # Simulate a browser download: temp name first, renamed when complete.
        tmp = make_pptx(w / "lecture7.pptx.crdownload", "Statistics: Probability")
        time.sleep(0.5)
        assert tmp.exists(), "partial download must not be touched"
        tmp.rename(w / "lecture7.pptx")
        make_pdf(w / "shopping.pdf", "milk")

        uni = Path(config.destination_root)
        assert _wait_for(lambda: (uni / "Programming Course" / "python_intro.pdf").exists())
        assert _wait_for(lambda: (uni / "Statistics" / "lecture7.pptx").exists())
        assert _wait_for(lambda: any(r.source.name == "shopping.pdf" for r in results))
        assert (w / "shopping.pdf").exists()
    finally:
        watcher.stop()
