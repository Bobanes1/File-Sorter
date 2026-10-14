import json
from pathlib import Path

from file_sorter.config import Config, Course, load_config, merge_courses, scan_study_folder


def make_tree(root: Path) -> Path:
    for sub in ["Informatik/Programmierung 1", "Informatik/Einführung in die Analysis I",
                "Mathematik/Analysis II", "Mathematik/Lineare Algebra", "Mathematik/.hidden"]:
        (root / "Studium" / sub).mkdir(parents=True)
    (root / "Studium" / "notes.txt").write_text("loose file")
    return root / "Studium"


def test_course_folder_with_and_without_subject(tmp_path):
    cfg = Config(destination_root=str(tmp_path))
    assert cfg.course_folder(Course("Analysis II", subject="Mathematik")) == tmp_path / "Mathematik" / "Analysis II"
    assert cfg.course_folder(Course("Statistik")) == tmp_path / "Statistik"
    assert cfg.course_folder(Course("X", subject="M", folder=str(tmp_path / "custom"))) == tmp_path / "custom"


def test_label():
    assert Course("Analysis II", subject="Mathematik").label == "Mathematik › Analysis II"
    assert Course("Statistik").label == "Statistik"


def test_scan_study_folder(tmp_path):
    found = {(c.subject, c.name): c.keywords for c in scan_study_folder(make_tree(tmp_path))}
    assert set(found) == {
        ("Informatik", "Programmierung 1"),
        ("Informatik", "Einführung in die Analysis I"),
        ("Mathematik", "Analysis II"),
        ("Mathematik", "Lineare Algebra"),
    }
    assert found[("Informatik", "Programmierung 1")] == ["programmierung"]
    # "analysis" alone is shared by two modules and "einführung" is a stopword,
    # but the core name "analysis i" identifies the module.
    assert found[("Informatik", "Einführung in die Analysis I")] == ["analysis i"]
    assert found[("Mathematik", "Lineare Algebra")] == ["lineare", "algebra"]


def test_scan_missing_folder(tmp_path):
    assert scan_study_folder(tmp_path / "nope") == []


def test_merge_keeps_existing(tmp_path):
    existing = [Course("Programmierung 1", ["prog1", "java"], subject="Informatik")]
    added = merge_courses(existing, scan_study_folder(make_tree(tmp_path)))
    assert added == 3
    assert len(existing) == 4
    assert existing[0].keywords == ["prog1", "java"]
    assert merge_courses(existing, scan_study_folder(tmp_path / "Studium")) == 0


def test_imported_modules_sort_into_subject_folders(tmp_path):
    from conftest import make_pdf

    from file_sorter.sorter import sort_file

    studium = make_tree(tmp_path)
    downloads = tmp_path / "Downloads"
    downloads.mkdir()
    cfg = Config(watch_folder=str(downloads), destination_root=str(studium), courses=scan_study_folder(studium))
    r1 = sort_file(make_pdf(downloads / "Analysis_II_Blatt3.pdf", "x"), cfg)
    r2 = sort_file(make_pdf(downloads / "skript.pdf", "Lineare Algebra, Kapitel 2"), cfg)
    r3 = sort_file(make_pdf(downloads / "Analysis_I_Blatt1.pdf", "x"), cfg)
    assert r1.destination == studium / "Mathematik" / "Analysis II" / "Analysis_II_Blatt3.pdf"
    assert r2.destination == studium / "Mathematik" / "Lineare Algebra" / "skript.pdf"
    assert r3.destination.parent == studium / "Informatik" / "Einführung in die Analysis I"


def test_old_config_without_subject_loads(tmp_path):
    path = tmp_path / "config.json"
    path.write_text(json.dumps({"courses": [{"name": "Statistik", "keywords": ["stats"]}]}), encoding="utf-8-sig")
    cfg = load_config(path)
    assert cfg.courses[0].subject == ""
    assert cfg.courses[0].keywords == ["stats"]
