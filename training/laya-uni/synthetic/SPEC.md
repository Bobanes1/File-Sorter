# Synthetic training data for laya-uni

File Sorter sorts files that students download (from Moodle, ILIAS, OLAT and the like) into one folder
per university module. A small language model gets a file's **name** and the **start of its text**,
plus the student's list of modules, and has to choose the right module. This data teaches it to do that
for **every kind of degree**, not just computer science.

Each generator writes one folder `synthetic/<group>/` with these files:

## `programs.json`
A JSON array of degree programs:

```json
[{"program": "Humanmedizin", "faculty": "medicine", "lang": "de", "country": "DE",
  "modules": [{"name": "Anatomie I", "code": "MED-1101", "keywords": ["Anatomie", "Bewegungsapparat", "Osteologie"]},
              ...]}]
```

- **Modules:** 10–14 per program, as a real curriculum names them. Include **look-alike modules** that are
  hard to tell apart (Anatomie I / Anatomie II, Strafrecht AT / Strafrecht BT, Organische Chemie /
  Anorganische Chemie, Mikroökonomie / Makroökonomie).
- **Codes:** `code` is optional. Codes look like a real university's, but no real university is named.
- **Keywords:** 2–5 words a student might type as keywords for that module.
- **Language and country:** `lang` is "de" or "en". `country` is one of DE, CH, AT, UK, US, NL. Swiss
  programs use Swiss spelling (ss for ß) and Swiss terms (Vorlesung, Übung, Testat, ECTS). Austrian ones
  use Austrian terms (LV, VO, UE, PS).

## `records-<program-slug>.jsonl`
One file per program, one JSON object per line:

```json
{"program": "Humanmedizin", "module": "Anatomie I", "lang": "de", "kind": "slides",
 "file_name": "VL07_Obere_Extremitaet.pdf",
 "text": "Anatomie I – Vorlesung 7\nObere Extremität: Schultergürtel und Oberarm\nDr. med. K. Brandner · HS 2025\n..."}
```

- **`module`:** exactly one of the program's module names, or `null` for a **"nothing fits" file**.
- **`kind`:** one of `slides`, `exercise`, `solution`, `script`, `reading`, `lab`, `case`, `exam_prep`,
  `paper`, `admin`.

**How many:** about **40 records per program**, at least 2 per module. Of these, about **4 per program**
are "nothing fits" files (`module: null`): timetables, enrolment confirmations, the library
regulations, a canteen menu, an invoice, a job ad, a general "Studienordnung" for the whole faculty,
a campus map. These are documents a student downloads that belong to none of the modules.

**File names**, a realistic mix:
- About 40 % descriptive: `Strafrecht_AT_Fall3_Irrtum.pdf`, `Lecture 5 - Cell Signalling.pdf`.
- About 30 % generic and numbered: `Blatt3.pdf`, `VL_05.pdf`, `Uebung_2_Loesung.pdf`,
  `Folien_Woche9.pptx`, `Tutorium-04.pdf`.
- About 15 % cryptic: `scan_0012.pdf`, `document(3).pdf`, `2025-10-14.pdf`, `Handout.pdf`,
  `download.pdf`.
- About 15 % with the module code: `MED1101_Skript_Kap2.pdf`.
- Mix the extensions too: `.pdf` (most), `.pptx`, `.docx`, `.xlsx`, `.ipynb`, `.zip`, as fits the kind.

**`text`** is what the first page or slide would say: **60–200 words**, with line breaks.
- **Look:** a title, often a header line (a fictional university or institute, a fictional lecturer,
  the semester such as "HS 2025", "WiSe 2025/26" or "Fall 2025", "Blatt 3", "Abgabe bis …"), then the
  start of the actual content: headings, the first sentences, task statements, formulas written as
  text, references, or case facts.
- **Make it hard:** only **about half** of the texts name the module or its code. The other half are
  recognisable **only from the topic**. Sometimes the content touches a neighbouring module's topic too.
- **Length:** vary it. Some texts are very short (a cover slide with 15 words), some are dense.
- **Language:** write in the program's language. English programs are in English. German programs may
  quote English papers.
- **No real people:** use only fictional people. No real university names in headers (use "Universität
  Musterstadt", "Institut für …", or nothing).

**Rules**
- Every line must be valid JSON. After each file, check it with
  `python3 -c "import json,sys; [json.loads(l) for l in open(sys.argv[1])]" <file>`
- Every `module` value must match a name in `programs.json` exactly (or be `null`).
- Don't copy the same text between records. Each file should read differently.
- Write the files with the Write tool, one program at a time, so finished work is saved even if
  something stops.
- Don't touch anything outside your own `synthetic/<group>/` folder, and don't commit.
