"""Fine-tune Laya's multilingual checkpoint into laya-uni.

Downloads the base checkpoint the app pins (convaiinnovations/laya, `multilingual/`) and trains it on
rows from `split.py` with `laya.train.finetune`. Rows with `expected` train on that one module; the
"nothing fits" rows carry `gold` probabilities (uniform), which laya trains on as a soft target.

    python3 train.py --train data/train.jsonl --val data/val.jsonl --out out
    python3 train.py ... --epochs 1 --micro-batch 2 --no-amp --device cpu     # smoke test

Every TrainConfig default below has a flag (`--micro-batch` for `micro_batch`, and so on).
"""
import argparse
import json
from pathlib import Path

REPO = "convaiinnovations/laya"
# The same revision as `REVISION` in src/laya.rs.
REVISION = "7b928d828b7b0e022f929d9bd2e44165aa270148"

DEFAULTS = dict(epochs=3, micro_batch=8, grad_accum=8, shuffle_options=("choice",), max_len=1024,
                head_max_len=256, amp=True)


def base_checkpoint(base_dir):
    """The base checkpoint's folder (downloaded on first use)."""
    from huggingface_hub import snapshot_download

    snapshot_download(REPO, revision=REVISION, allow_patterns=["multilingual/*"], local_dir=base_dir)
    return str(Path(base_dir) / "multilingual")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--train", required=True, help="train.jsonl")
    parser.add_argument("--val", help="val.jsonl (laya scores the base and the result on it)")
    parser.add_argument("--out", default="out", help="where the fine-tuned checkpoint goes")
    parser.add_argument("--base-dir", default="base", help="where the base checkpoint is downloaded to")
    parser.add_argument("--device", default="auto")
    for name, value in DEFAULTS.items():
        flag = "--" + name.replace("_", "-")
        if isinstance(value, bool):
            parser.add_argument(flag, action=argparse.BooleanOptionalAction, default=value)
        elif isinstance(value, tuple):
            parser.add_argument(flag, nargs="*", default=value, help="question types, or none")
        else:
            parser.add_argument(flag, type=type(value), default=value)
    args = parser.parse_args()

    from laya.train import TrainConfig, finetune

    config = TrainConfig(**{name: getattr(args, name) for name in DEFAULTS}, eval_data=args.val)
    config.shuffle_options = tuple(config.shuffle_options)
    summary = finetune(args.train, base_checkpoint(args.base_dir), args.out, config=config, device=args.device)
    keep = ("train_items", "calibration_items", "eval_items", "skipped", "epoch_loss", "temperature",
            "temperature_by_options")
    print(json.dumps({k: summary[k] for k in keep}, indent=2))
    report = summary["train_report"]
    if report["before"] and report["after"]:
        print("val accuracy %.3f -> %.3f" % (report["before"]["accuracy"], report["after"]["accuracy"]))


if __name__ == "__main__":
    main()
