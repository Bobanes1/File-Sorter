from pathlib import Path

from file_sorter.config import Course
from file_sorter.matcher import match_file, normalise, score_text


def no_content(_path):
    raise AssertionError("content should not be read when the filename decides")


def test_normalise_strips_separators_and_accents():
    assert normalise("Analysis_Übung-03.Final") == "analysis ubung 03 final"


def test_filename_keyword_matches(courses):
    m = match_file(Path("Prog1_Lecture03.pdf"), courses, extract=no_content)
    assert m is not None and m.course.name == "Programming Course"


def test_multiword_keyword_and_plural(courses):
    assert match_file(Path("linear-algebra-sheet2.pdf"), courses).course.name == "Linear Algebra"
    assert match_file(Path("sorting algorithms.pdf"), courses, read_contents=False).course.name == "Programming Course"


def test_keyword_needs_word_boundary():
    course = Course("AI", ["ai"])
    assert score_text("maintain the system", course)[0] == 0
    assert score_text("intro to ai lecture", course)[0] == 1


def test_course_name_counts_as_keyword():
    course = Course("Statistics", [])
    assert match_file(Path("Statistics Lecture 1.pdf"), [course], read_contents=False).course is course


def test_no_match_returns_none(courses):
    assert match_file(Path("holiday photos.pdf"), courses, extract=lambda p: "beach and sun") is None


def test_tie_returns_none(courses):
    # One keyword for two courses in the filename, and nothing in the content.
    assert match_file(Path("python matrix.pdf"), courses, extract=lambda p: "") is None


def test_content_used_when_filename_is_generic(courses):
    text = "Lecture 5: Linear Algebra\nEigenvalues of a matrix"
    m = match_file(Path("lecture5.pdf"), courses, extract=lambda p: text)
    assert m.course.name == "Linear Algebra"


def test_content_breaks_filename_tie(courses):
    text = "Introduction to Programming with Python"
    m = match_file(Path("python matrix.pdf"), courses, extract=lambda p: text)
    assert m.course.name == "Programming Course"


def test_repeated_content_word_is_capped():
    a = Course("A", ["alpha"])
    b = Course("B", ["beta", "gamma", "delta"])
    text = "alpha " * 50 + "beta gamma delta beta gamma delta"
    m = match_file(Path("notes.pdf"), [a, b], extract=lambda p: text)
    assert m.course is b


def test_content_skipped_when_disabled(courses):
    assert match_file(Path("lecture5.pdf"), courses, read_contents=False, extract=no_content) is None
