"""Split the rows from `file-sorter --laya-rows` into train / val / test.

Whole programs go to test (one per faculty group), so the test measures degrees the model has never
seen, not module names it memorised. Validation is a random 10 % of the remaining rows, used for the
temperature fit and for choosing the app's confidence thresholds. Duplicate states are dropped.

    python3 split.py rows.jsonl out/        # writes out/train.jsonl, out/val.jsonl, out/test.jsonl
"""
import collections
import json
import random
import sys
from pathlib import Path

SEED = 20261020
VAL_SHARE = 0.10


def main(rows_path, out_dir):
    rows, seen = [], set()
    for line in open(rows_path, encoding="utf-8"):
        row = json.loads(line)
        key = (row["state"], json.dumps(row["questions"], sort_keys=True))
        if key not in seen:
            seen.add(key)
            rows.append(row)
    rng = random.Random(SEED)
    programs = collections.defaultdict(set)
    for r in rows:
        programs[r["meta"]["faculty"]].add(r["meta"]["program"])
    held_out = {rng.choice(sorted(p)) for p in programs.values() if len(p) > 1}
    split = {"train": [], "val": [], "test": []}
    for r in rows:
        if r["meta"]["program"] in held_out:
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
