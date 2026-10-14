from conftest import make_docx, make_pdf, make_pptx

from file_sorter.extractor import extract_text


def test_pdf_text(tmp_path):
    path = make_pdf(tmp_path / "a.pdf", "Introduction to Linear Algebra")
    assert "Linear Algebra" in extract_text(path)


def test_pptx_text(tmp_path):
    path = make_pptx(tmp_path / "a.pptx", "Programming 1 - Week 3")
    assert "Programming 1" in extract_text(path)


def test_docx_text(tmp_path):
    path = make_docx(tmp_path / "a.docx", "Statistics exercise sheet")
    assert "Statistics" in extract_text(path)


def test_broken_file_returns_empty(tmp_path):
    path = tmp_path / "broken.pdf"
    path.write_bytes(b"not really a pdf")
    assert extract_text(path) == ""


def test_unknown_type_returns_empty(tmp_path):
    path = tmp_path / "old.ppt"
    path.write_bytes(b"\x00\x01")
    assert extract_text(path) == ""
