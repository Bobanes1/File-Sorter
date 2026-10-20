# OER test set (and collected training files) for laya-uni

Real, openly licensed course PDFs used as the **held-out test set** for the laya-uni
module classifier. These files are never used for training or for tuning prompts or
criteria. Training data is synthetic and lives elsewhere.

`sources.json` lists 240 PDFs (lecture slides, exercise sheets, script chapters,
readings, exams) across 13 faculties, about 13 to 22 files each, in German (158) and
English (82):

| faculty | programs |
|---|---|
| medicine_health | Humanmedizin, Pflegewissenschaft, Medicine |
| law | Rechtswissenschaft, Law |
| humanities_languages | Philosophie, Germanistik, Geschichtswissenschaft, Linguistics and Philosophy |
| social_sciences | Soziale Arbeit, Psychology, Political Science |
| economics_business | Betriebswirtschaftslehre, Economics and Management |
| natural_sciences | Physik, Geophysik, Biologie, Chemistry and Biology |
| mathematics | Mathematik, Mathematics |
| engineering | Elektrotechnik, Maschinenbau, Photonics |
| computer_science | Informatik, Computer Science |
| architecture_arts | Museum Studies, Raumplanung, Musik, Architecture, Music |
| education | Lehramt Physik, Lehramt Politik, Bildungswissenschaften, Teacher Education |
| theology | Islamische Theologie, Evangelische Theologie, Religious Studies |
| agriculture_environment | Agrarwissenschaften, Umweltwissenschaften, Environmental Sciences |

Each entry has `url`, `file` (the original file name), `licence`, `faculty`,
`program`, `module`, `lang`, `kind` (slides/exercise/script/reading/exam) and
`source` (the landing page). Every URL was checked to return a PDF on 2026-10-20.

## Sources and licences

- twillo (Lower Saxony) and ZOERR (Baden-Württemberg) OER repositories: CC BY,
  CC BY-SA or CC0, taken from each item's licence metadata.
- Zenodo lesson records: CC BY 4.0.
- OpenRewi law textbooks on German Wikibooks: CC BY-SA 4.0. The PDFs are rendered
  on request, so their sha256 can change between downloads.
- MIT OpenCourseWare: **CC BY-NC-SA 4.0** (61 entries). That is fine for a
  non-commercial test set, but filter on `licence` if you need commercial-safe data.

Licence counts: CC BY-SA 4.0 97, CC BY 4.0 67, CC BY-NC-SA 4.0 61, CC BY 3.0 7,
CC0 6, CC BY-SA 3.0 2. Credit the authors via the `source` page when you show or
share a file.

## Running it

```bash
python3 fetch_oer.py                 # into ~/.cache/laya-uni/oer
python3 fetch_oer.py /some/other/dir
```

It needs only the Python 3.10+ standard library. It writes
`<out>/<faculty>/<program>/<module>/<file>`, skips files that already exist, writes
`<out>/manifest.json` (sha256, bytes, licence, source per file) and prints the count
per faculty. The full set is about 450 MB. Keep the output outside the repo: the
PDFs must never be committed.

Then build the test rows with the app's own text extractor:

```bash
file-sorter --laya-rows ~/.cache/laya-uni/oer test_oer.jsonl
```

## Training lists (`train/`)

`train/*.json` lists about 4,900 more openly licensed course PDFs in the same schema, collected for
**training** (laya-uni-2 on): every faculty, German, English, French and Italian, with no url, landing
page or course shared with `sources.json`. `train/BRIEF.md` is the brief they were collected to.
Most are CC BY / BY-SA (Wikibooks, Wikiversité, HAL, Zenodo, OER repositories); MIT OpenCourseWare,
Exo7 and some others are CC BY-NC-SA. Lists named `*.local.json` hold files used with their author's
permission whose links must not be published; they are gitignored. Wikimedia renders its PDFs on
request, so their bytes change between downloads, and it answers 429 when asked too fast (the fetcher
retries).
