"""Split the rows from `file-sorter --laya-rows` into train / val / test.

Whole programs go to test (one per faculty group), so the test measures degrees the model has never
seen, not module names it memorised. Rows from collected real files (`oer/train/`) only go to
train and val. Validation is a random 10 % of the remaining rows, used for the
temperature fit and for choosing the app's confidence thresholds. Duplicate states are dropped.

    python3 split.py rows.jsonl out/        # writes out/train.jsonl, out/val.jsonl, out/test.jsonl
"""
import collections
import json
import random
import re
import sys
from pathlib import Path

SEED = 20261020
VAL_SHARE = 0.10
BOILERPLATE_ROWS = 100  # a line of 25+ characters in this many collected files is repository boilerplate
WIKI_SUFFIX = re.compile(r" - Wiki(books|versity|versité|versità)\b.*")
FOOTER = re.compile(r"^\s*(Abgerufen von|Retrieved from|Récupérée de|Estratto da)\b|For information about citing these materials")


def norm(line):
    return re.sub(r"\d+", "#", line.strip())


def clean(rows):
    """Drop boilerplate lines from the collected files' text (folder rows); the file name line stays."""
    folder = [r for r in rows if r["meta"].get("source") == "folder"]
    seen = collections.Counter(n for r in folder for n in {norm(l) for l in r["state"].split("\n")} if len(n) >= 25)
    common = {n for n, c in seen.items() if c >= BOILERPLATE_ROWS}
    for r in folder:
        head, _, text = r["state"].partition("\n\n")
        lines = [WIKI_SUFFIX.sub("", l) for l in text.split("\n") if norm(l) not in common and not FOOTER.search(l)]
        r["state"] = head + "\n\n" + "\n".join(lines)
    return len(common)


def main(rows_path, out_dir):
    rows, seen = [], set()
    for line in open(rows_path, encoding="utf-8"):
        row = json.loads(line)
        key = (row["state"], json.dumps(row["questions"], sort_keys=True))
        if key not in seen:
            seen.add(key)
            rows.append(row)
    print(f"{clean(rows)} boilerplate lines removed from collected files")
    rng = random.Random(SEED)
    programs = collections.defaultdict(set)
    for r in rows:
        if r["meta"].get("source") != "folder":
            programs[r["meta"]["faculty"]].add(r["meta"]["program"])
    held_out = {rng.choice(sorted(p)) for p in programs.values() if len(p) > 1}
    split = {"train": [], "val": [], "test": []}
    for r in rows:
        # Collected real files (folder rows) always train; the real test set is test_oer.
        if r["meta"]["program"] in held_out and r["meta"].get("source") != "folder":
            name = "test"
        else:
            name = "val" if rng.random() < VAL_SHARE else "train"
        r["meta"]["split"] = name
        split[name].append(r)
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    for name, part in split.items():
        with open(out / f"{name}.jsonl", "w", encoding="utf-8") as f:
            for r in part:
                f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"{len(rows)} rows ({len(seen)} unique), held-out programs: {', '.join(sorted(held_out))}")
    table = collections.Counter((r["meta"]["faculty"], r["meta"].get("lang", ""), r["meta"]["split"]) for r in rows)
    print(f"{'faculty':<22}{'lang':<6}{'train':>7}{'val':>6}{'test':>6}")
    for fac, lang in sorted({(f, l) for f, l, _ in table}):
        print(f"{fac:<22}{lang:<6}" + "".join(f"{table[(fac, lang, s)]:>{w}}" for s, w in [("train", 7), ("val", 6), ("test", 6)]))
    nothing = sum("gold" in r for r in rows)
    print(f"nothing-fits rows: {nothing} ({nothing / max(1, len(rows)):.0%})")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
