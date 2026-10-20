"""A tiny made-up sample in the rows format, to check the pipeline runs end to end (not to learn from).

    python3 smoke_rows.py smoke/        # writes smoke/train.jsonl (40 rows), smoke/val.jsonl, smoke/test.jsonl
"""
import json
import random
import sys
from pathlib import Path

INSTRUCTIONS = "Which university module does this file belong to?"   # laya::INSTRUCTIONS
PROGRAMS = [
    ("medicine", "de", ["Anatomie I", "Anatomie II", "Physiologie", "Biochemie"],
     ["Obere Extremität und Schultergürtel", "Untere Extremität, Hüfte und Knie", "Herzzyklus und Blutdruck",
      "Citratzyklus und Atmungskette"]),
    ("law", "de", ["Strafrecht AT", "Strafrecht BT", "Privatrecht I", "Staatsrecht"],
     ["Vorsatz und Irrtum über Tatumstände", "Diebstahl und Betrug", "Vertragsschluss und Willenserklärung",
      "Grundrechte und Gewaltenteilung"]),
    ("economics", "en", ["Microeconomics", "Macroeconomics", "Statistics I", "Accounting"],
     ["Consumer choice and indifference curves", "Inflation, GDP and monetary policy",
      "Probability distributions and sampling", "Balance sheet and double-entry bookkeeping"]),
    ("chemistry", "en", ["Organic Chemistry", "Inorganic Chemistry", "Physical Chemistry", "Lab Course"],
     ["Nucleophilic substitution of alkyl halides", "Transition metal complexes and ligands",
      "Thermodynamics and the Gibbs free energy", "Safety rules and titration protocol"]),
]
NOTHING = ["Mensa: Speiseplan KW 43", "Immatrikulationsbestätigung Wintersemester", "Campus map and opening hours"]


def row(faculty, lang, modules, answer, text, name, rng):
    options = sorted(modules, key=lambda _: rng.random())
    q = {"module": {"type": "choice", "instructions": INSTRUCTIONS, "criteria": {m: None for m in options}}}
    r = {"state": f"File name: {name}\n\n{text}", "questions": q,
         "meta": {"faculty": faculty, "program": faculty, "lang": lang, "source": "smoke"}}
    if answer is None:
        r["gold"] = {"module": {"probabilities": {m: 1 / len(options) for m in options}}}
    else:
        r["expected"] = {"module": answer}
    return r


def main(out_dir):
    rng = random.Random(1)
    rows = []
    for faculty, lang, modules, topics in PROGRAMS:
        for i, (module, topic) in enumerate(zip(modules, topics)):
            rows.append(row(faculty, lang, modules, module, f"{module}\nVorlesung {i + 1}\n{topic}", f"VL_{i + 1:02}.pdf", rng))
            rows.append(row(faculty, lang, modules, module, topic, f"scan_{rng.randrange(1000):04}.pdf", rng))
        rows.append(row(faculty, lang, modules, None, NOTHING[rng.randrange(3)], "download.pdf", rng))
        rows.append(row(faculty, lang, modules, None, NOTHING[rng.randrange(3)], "document(3).pdf", rng))
    rng.shuffle(rows)
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    for name, part in [("train", rows), ("val", rows[:20]), ("test", rows[20:])]:
        with open(out / f"{name}.jsonl", "w", encoding="utf-8") as f:
            f.writelines(json.dumps(r, ensure_ascii=False) + "\n" for r in part)
    print(f"{len(rows)} rows in {out}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
