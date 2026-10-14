"""Decide which course a file belongs to by scoring keyword hits."""

from __future__ import annotations

import re
import unicodedata
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

from .config import Course

FILENAME_WEIGHT = 10
# A keyword repeated 50 times in a PDF shouldn't outweigh several different keywords.
MAX_CONTENT_HITS_PER_KEYWORD = 5


@dataclass
class Match:
    course: Course
    score: int
    reason: str


def normalise(text: str) -> str:
    """Lowercase, strip accents and turn separators like _ - . into spaces."""
    text = unicodedata.normalize("NFKD", text)
    text = "".join(ch for ch in text if not unicodedata.combining(ch))
    text = text.lower()
    text = re.sub(r"[_\-.()\[\]{},;:/\\]", " ", text)
    return re.sub(r"\s+", " ", text).strip()


def _keyword_pattern(keyword: str) -> re.Pattern[str] | None:
    kw = normalise(keyword)
    if not kw:
        return None
    # Word boundaries so "ai" doesn't match "maintain". A plural "s" is allowed
    # ("algorithm" matches "algorithms") and digits may follow directly, so the
    # keyword "prog" still matches "prog1".
    return re.compile(r"(?<![a-z0-9])" + re.escape(kw) + r"s?(?![a-z])")


def _terms(course: Course) -> list[str]:
    terms = [course.name, *course.keywords]
    seen: set[str] = set()
    unique: list[str] = []
    for term in terms:
        key = normalise(term)
        if key and key not in seen:
            seen.add(key)
            unique.append(term)
    return unique


def score_text(text: str, course: Course, *, cap: int | None = None) -> tuple[int, list[str]]:
    """Count keyword hits for one course. Returns (hits, matched keywords)."""
    norm = normalise(text)
    total = 0
    matched: list[str] = []
    for term in _terms(course):
        pattern = _keyword_pattern(term)
        if pattern is None:
            continue
        hits = len(pattern.findall(norm))
        if hits:
            if cap is not None:
                hits = min(hits, cap)
            total += hits
            matched.append(term)
    return total, matched


def _best(scores: dict[int, tuple[int, list[str]]]) -> tuple[int, int] | None:
    """Index and score of the single best course, or None if no hits or a tie."""
    ranked = sorted(((s, i) for i, (s, _) in scores.items() if s > 0), reverse=True)
    if not ranked:
        return None
    if len(ranked) > 1 and ranked[0][0] == ranked[1][0]:
        return None
    return ranked[0][1], ranked[0][0]


def match_file(
    path: Path,
    courses: list[Course],
    *,
    read_contents: bool = True,
    extract: Callable[[Path], str] | None = None,
) -> Match | None:
    """Return the course the file belongs to, or None to leave it where it is.

    The filename is checked first. File contents are only read when the filename
    alone doesn't give one clear winner.
    """
    if not courses:
        return None

    name = path.stem
    name_scores = {i: score_text(name, c) for i, c in enumerate(courses)}
    best = _best(name_scores)
    if best is not None:
        idx, _ = best
        kw = ", ".join(name_scores[idx][1])
        return Match(courses[idx], name_scores[idx][0] * FILENAME_WEIGHT, f"filename contains {kw}")

    if not read_contents:
        return None

    if extract is None:
        from .extractor import extract_text as extract
    text = extract(path)
    if not text.strip():
        return None

    combined: dict[int, tuple[int, list[str]]] = {}
    for i, course in enumerate(courses):
        content_hits, content_kw = score_text(text, course, cap=MAX_CONTENT_HITS_PER_KEYWORD)
        name_hits, name_kw = name_scores[i]
        combined[i] = (name_hits * FILENAME_WEIGHT + content_hits, name_kw + content_kw)

    best = _best(combined)
    if best is None:
        return None
    idx, score = best
    kw = ", ".join(dict.fromkeys(combined[idx][1]))
    return Match(courses[idx], score, f"content mentions {kw}")
