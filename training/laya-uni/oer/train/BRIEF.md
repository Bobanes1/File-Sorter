# Collecting real course files for laya-uni training

laya-uni picks the university module a student's downloaded file belongs to (from the file name and
the start of its text). It was trained on synthetic files. We now want **real, openly licensed course
files** as extra TRAINING data. You **collect** existing files (find and list them). You do not write
or generate any content.

## Output
One JSON array in `training/laya-uni/oer/train/<your-group>.json`, same schema as `oer/sources.json`:

```json
{"url": "<direct download URL that returns the file itself>",
 "file": "<original file name, e.g. VL03_Thermodynamik.pdf>",
 "licence": "CC BY 4.0", "faculty": "<faculty key>", "program": "<degree program>",
 "module": "<module, as a curriculum names it>", "lang": "de|en|fr|it",
 "kind": "slides|exercise|solution|script|reading|exam|lab|case",
 "source": "<landing page where the licence is stated>"}
```

Faculty keys: medicine_health, law, humanities_languages, social_sciences, economics_business,
natural_sciences, mathematics, engineering, computer_science, architecture_arts, education, theology,
agriculture_environment.

## What makes it useful
- **Target about 250 files** for your group, spread evenly over your faculties.
- **Structure like a real student's folder:** per program at least **4 modules**, each with **at least
  3 files** (5–10 is better). The model chooses between sibling modules of one program, so a program with
  look-alike modules (Analysis I / Analysis II, Mikro / Makro, Strafrecht AT / BT) is ideal. Group
  files from one course under one module; name programs and modules the way a curriculum would
  ("Organische Chemie", "Microeconomics"), not after the course page's marketing title.
- **Variety of kinds:** slides, exercise sheets, solutions, script chapters, readings, exams.
- **PDF only** (the fetcher checks that the response starts with `%PDF`). Prefer files under 30 MB.
- **Licence:** openly licensed only, read from the item's own page: CC0, CC BY, CC BY-SA, public domain;
  CC BY-NC / BY-NC-SA is allowed but write it exactly in `licence`. Skip anything without a stated licence.
- **Good sources:** twillo.de, zoerr.de, ORCA.nrw, OpenRewi / Wikibooks, Zenodo (lesson/lecture records),
  MIT OpenCourseWare, OpenStax / LibreTexts chapter PDFs, Open Yale Courses, TU Delft OCW, university
  OER portals (e.g. Swiss SWITCH edu-sharing, Austrian OER portals), FUN-MOOC, Italian/French open
  university repositories, HAL (cours), arXiv lecture notes (check the licence). Use WebSearch/WebFetch.

## Hard rules
- **Never overlap the test set.** `oer/sources.json` is held-out test data. Skip every `url` and every
  `source` listed there, and skip any other file from the same course (same course page, same MIT
  course number, same twillo/ZOERR collection). Check with a script before you finish.
- **Verify every URL** returns a PDF: `curl -sL --max-time 60 -r 0-7 "<url>" | head -c 5` must print
  `%PDF-`. Drop it otherwise. Do not download whole files and do not store any file in the repo.
- No duplicates (by url) within your file, and none with the other files in `oer/train/` (other agents
  work in parallel: check again right before you finish). Real people's names in the PDFs are fine (they are public
  course material); do not collect anything personal (student submissions, grade lists).
- Save your JSON every ~40 entries with the Write tool so finished work survives an interruption. It
  must stay a valid JSON array: check with `python3 -c "import json,sys; json.load(open(sys.argv[1]))" <file>`.
- Touch nothing outside your own output file. Don't commit.

## When done
Report: total files, counts per faculty / program / module count, languages, licences, sources used,
and how many URLs you dropped (not a PDF, no licence, test overlap).
