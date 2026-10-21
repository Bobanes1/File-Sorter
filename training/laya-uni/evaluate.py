"""Score checkpoints on row files the way the app uses them, and pick the app's thresholds.

    python3 evaluate.py --model base=base/multilingual --model laya-uni=out \
        --val data/val.jsonl --test data/test.jsonl --out report

The app sorts a file only if p_top >= MIN_PROBABILITY and p_top >= MIN_LEAD * p_second (src/laya.rs),
with the probabilities from the checkpoint's own temperatures as the app applies them. For every
(min_probability, min_lead) on a grid this reports, on each row file:
- coverage: share of the files that have a module which get sorted,
- precision: share of sorted files that went to the right module ("nothing fits" files sorted
  anywhere are wrong),
- rule accuracy: right sorts plus "nothing fits" files left alone, over all files.
The recommendation per model is the loosest pair (most coverage) with >= 95 % precision on --val.
Top-1 accuracy (files that have a module) and rule accuracy are also split by faculty and language.
Writes <out>/report.md and <out>/report.json.
"""
import argparse
import json
import math
from collections import defaultdict
from pathlib import Path

APP = (0.6, 2.0)   # MIN_PROBABILITY, MIN_LEAD in src/laya.rs today
TARGET_PRECISION = 0.95
MIN_PROBABILITIES = [round(0.05 * i, 2) for i in range(0, 20)]
MIN_LEADS = [1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0]


def app_probabilities(logits, k, cfg):
    """softmax(logits / T) with T chosen as src/laya.rs does: the option-count bucket, else the type-level one.

    Unlike laya's runtime, the app does not clamp T to [0.5, 5], so neither does this.
    """
    size = "2" if k <= 2 else "3-5" if k <= 5 else "6-10" if k <= 10 else "11+"
    t = (cfg.get("temperature_by_options") or {}).get("choice:" + size, (cfg.get("temperature") or [1.0])[0])
    z = [float(v) / t for v in logits[:k]]
    top = max(z)
    e = [math.exp(v - top) for v in z]
    return [v / sum(e) for v in e]


def score(model_dir, rows, device):
    """One result per row: {meta, p (sorted high to low), right (top is the answer), nothing}; None if skipped."""
    from laya.train import calibration_records, items_from_rows, load_checkpoint, resolve_device

    model, tok, cfg = load_checkpoint(model_dir)
    dev = resolve_device(device)
    model.to(dev)
    max_len, head_max_len = cfg.get("max_len", 512), cfg.get("head_max_len", 192)
    items, kept = [], []
    for i, row in enumerate(rows):
        made, _skipped = items_from_rows(tok, [row], max_len, head_max_len)
        if made:
            items.append(made[0])
            kept.append(i)
    results = [None] * len(rows)
    for i, (_qt, logits, target, k) in zip(kept, calibration_records(model, tok, items, dev, max_len, head_max_len)):
        p = app_probabilities(logits, k, cfg)
        nothing = "gold" in rows[i]
        top = max(range(k), key=lambda j: p[j])
        results[i] = {"meta": rows[i].get("meta", {}), "p": sorted(p, reverse=True), "nothing": nothing,
                      "right": not nothing and target[top] == max(target)}
    return results


def sorts(r, min_p, min_lead):
    p = r["p"]
    second = p[1] if len(p) > 1 else 0.0
    return p[0] >= min_p and p[0] >= min_lead * second


def rule(results, min_p, min_lead):
    rs = [r for r in results if r]
    labelled = [r for r in rs if not r["nothing"]]
    sorted_ = [r for r in rs if sorts(r, min_p, min_lead)]
    right = sum(r["right"] for r in sorted_)
    left_alone = sum(r["nothing"] and not sorts(r, min_p, min_lead) for r in rs)
    return {"min_probability": min_p, "min_lead": min_lead,
            "coverage": sum(not r["nothing"] for r in sorted_) / max(1, len(labelled)),
            "precision": right / len(sorted_) if sorted_ else None,
            "rule_accuracy": (right + left_alone) / max(1, len(rs)), "sorted": len(sorted_)}


def curve(results):
    return [rule(results, p, lead) for p in MIN_PROBABILITIES for lead in MIN_LEADS]


def recommend(points):
    ok = [c for c in points if c["precision"] is not None and c["precision"] >= TARGET_PRECISION]
    # Loosest = most coverage; ties go to the smaller thresholds.
    return max(ok, key=lambda c: (c["coverage"], -c["min_probability"], -c["min_lead"]), default=None)


def summary(results, thresholds):
    rs = [r for r in results if r]
    labelled = [r for r in rs if not r["nothing"]]
    return {"rows": len(results), "skipped": len(results) - len(rs), "with_module": len(labelled),
            "nothing_fits": len(rs) - len(labelled),
            "top1_accuracy": sum(r["right"] for r in labelled) / len(labelled) if labelled else None,
            "rule": rule(results, *thresholds)}


def groups(results, thresholds):
    out = {}
    for key in ("faculty", "lang"):
        by = defaultdict(list)
        for r in results:
            if r:
                by[r["meta"].get(key, "?")].append(r)
        out[key] = {g: summary(rs, thresholds) for g, rs in sorted(by.items())}
    return out


def pct(v):
    return "–" if v is None else f"{100 * v:.1f}"


def markdown(report):
    names = list(report["models"])
    lines = ["# laya-uni evaluation", "",
             f"Acceptance rule: sort if p_top >= MIN_PROBABILITY and p_top >= MIN_LEAD * p_second. "
             f"Recommended = loosest pair with >= {TARGET_PRECISION:.0%} precision on validation "
             f"(the app uses {APP[0]} / {APP[1]} today). Numbers in %.", "",
             "## Recommended thresholds", "", "| model | MIN_PROBABILITY | MIN_LEAD | val coverage | val precision |",
             "|---|---|---|---|---|"]
    for name in names:
        rec = report["models"][name]["recommended"]
        lines.append(f"| {name} | " + (f"{rec['min_probability']} | {rec['min_lead']} | {pct(rec['coverage'])} | "
                                       f"{pct(rec['precision'])} |" if rec else "none reaches the target | | | |"))
    for split in report["splits"]:
        lines += ["", f"## {split}", "", "| | " + " | ".join(names) + " |", "|---" * (len(names) + 1) + "|"]
        s = {n: report["models"][n]["splits"][split] for n in names}
        first = s[names[0]]
        lines.append(f"| rows (with a module / nothing fits / skipped) | " + " | ".join(
            f"{v['with_module']} / {v['nothing_fits']} / {v['skipped']}" for v in s.values()) + " |")
        lines.append("| top-1 accuracy | " + " | ".join(pct(v["top1_accuracy"]) for v in s.values()) + " |")
        for label, key in [("app thresholds", "app"), ("recommended thresholds", "recommended")]:
            for m in ("coverage", "precision", "rule_accuracy"):
                lines.append(f"| {m.replace('_', ' ')} at {label} | " + " | ".join(
                    pct(v[key][m]) if v.get(key) else "–" for v in s.values()) + " |")
        for key in ("faculty", "lang"):
            lines += ["", f"### {split} by {key}: top-1 accuracy / rule accuracy at recommended thresholds", "",
                      f"| {key} | n | " + " | ".join(names) + " |", "|---" * (len(names) + 2) + "|"]
            for g, v in first["groups"][key].items():
                cells = []
                for n in names:
                    gv = s[n]["groups"][key][g]
                    cells.append(f"{pct(gv['top1_accuracy'])} / {pct(gv['rule']['rule_accuracy'])}")
                lines.append(f"| {g} | {v['rows'] - v['skipped']} | " + " | ".join(cells) + " |")
    shown = [0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]
    for name in names:
        lines += ["", f"## Coverage / precision on {report['curve_split']}: {name}", "",
                  "MIN_PROBABILITY down, MIN_LEAD across; each cell is coverage / precision.", "",
                  "| | " + " | ".join(str(l) for l in MIN_LEADS) + " |", "|---" * (len(MIN_LEADS) + 1) + "|"]
        points = {(c["min_probability"], c["min_lead"]): c for c in report["models"][name]["curve"]}
        for p in shown:
            lines.append(f"| {p} | " + " | ".join(
                f"{pct(points[(p, l)]['coverage'])} / {pct(points[(p, l)]['precision'])}" for l in MIN_LEADS) + " |")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--model", action="append", required=True, metavar="NAME=DIR")
    parser.add_argument("--val", required=True, help="rows the thresholds are chosen on")
    parser.add_argument("--test", help="held-out rows, scored with the thresholds chosen on --val")
    parser.add_argument("--out", default="report")
    parser.add_argument("--device", default="auto")
    args = parser.parse_args()

    from laya.train import read_jsonl

    splits = {"val": read_jsonl(args.val)}
    if args.test:
        splits["test"] = read_jsonl(args.test)
    report = {"app_thresholds": APP, "target_precision": TARGET_PRECISION, "splits": list(splits),
              "curve_split": "val", "models": {}}
    for spec in args.model:
        name, _, model_dir = spec.partition("=")
        results = {s: score(model_dir, rows, args.device) for s, rows in splits.items()}
        points = curve(results["val"])
        rec = recommend(points)
        entry = {"dir": model_dir, "recommended": rec, "curve": points, "splits": {}}
        for s, rs in results.items():
            v = summary(rs, APP)
            v["app"] = v.pop("rule")
            v["recommended"] = rule(rs, rec["min_probability"], rec["min_lead"]) if rec else None
            v["groups"] = groups(rs, (rec["min_probability"], rec["min_lead"]) if rec else APP)
            entry["splits"][s] = v
        report["models"][name] = entry
        print(f"{name}: val top-1 {pct(entry['splits']['val']['top1_accuracy'])} %, recommended "
              + (f"MIN_PROBABILITY {rec['min_probability']}, MIN_LEAD {rec['min_lead']}" if rec else "none"), flush=True)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    (out / "report.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    (out / "report.md").write_text(markdown(report), encoding="utf-8")
    print(f"wrote {out / 'report.md'} and {out / 'report.json'}")


if __name__ == "__main__":
    main()
