# laya-uni: training and evaluation

Fine-tunes Laya's multilingual checkpoint (`convaiinnovations/laya`, `multilingual/`, the revision
pinned in `src/laya.rs`) on rows made by `file-sorter --laya-rows` and split by `split.py`, so it
picks the right module for files from every faculty, not just computer science.

| file | does |
|---|---|
| `train.py` | downloads the base checkpoint, runs `laya.train.finetune` (3 epochs, effective batch 64, options shuffled, 1024 / 256 tokens, fp16) |
| `evaluate.py` | scores base and fine-tuned side by side, by faculty and language; coverage / precision for the app's `MIN_PROBABILITY` / `MIN_LEAD` rule and the loosest pair with ≥ 95 % precision on validation |
| `pack.py` | copies the four release files into `dist/` and prints the `FILES` table for `src/laya.rs` |
| `run_remote.sh` | all of it on a fresh GPU box |
| `smoke_rows.py`, `test_evaluate.py` | a 40-row sample to try the pipeline, and a check of the threshold maths |

## 1. Make the data (here)

```bash
cargo run --release -- --laya-rows training/laya-uni/synthetic rows.jsonl
```

```bash
python3 training/laya-uni/split.py rows.jsonl training/laya-uni/data
```

This leaves `data/train.jsonl`, `data/val.jsonl` and `data/test.jsonl`. The test rows are whole
programs the model never sees in training.

## 2. Rent a GPU

Any NVIDIA card with 24 GB or more (RTX 4090, RTX 3090, L4, A10G, L40S, A100), an Ubuntu image with
the NVIDIA driver (RunPod, vast.ai, Lambda and the like all have one). 40 GB of disk is enough.

## 3. Copy, run, fetch

```bash
rsync -av --exclude .venv --exclude base --exclude out --exclude dist training/laya-uni/ gpu:laya-uni/
```

```bash
ssh gpu 'cd laya-uni && ./run_remote.sh'
```

`run_remote.sh` makes a venv, installs `requirements.txt` (torch from the CUDA 12.8 index), then runs
train → evaluate → pack. Arguments go to `train.py` (`./run_remote.sh --epochs 4`); `DATA=other`
reads the rows from another folder. Run it in `tmux` so a dropped connection does not stop it.
Try it first on the smoke sample (a minute on a GPU):

```bash
ssh gpu 'cd laya-uni && python3 smoke_rows.py smoke && DATA=smoke ./run_remote.sh --epochs 1'
```

When it is done, bring back the release files and the report, then stop the box:

```bash
rsync -av gpu:laya-uni/dist gpu:laya-uni/report training/laya-uni/
```

`out/` (the full checkpoint, also with `checkpoint_latest/`) stays on the box; fetch it too if you
want to train further from it later.

## 4. Read the report

`report/report.md` (and `report.json` with every number):

- **top-1 accuracy**: the best option is the right module (files that have one).
- **coverage**: share of files that have a module which the rule sorts.
- **precision**: share of sorted files that went to the right module. A "nothing fits" file sorted
  anywhere is a wrong sort.
- **rule accuracy**: right sorts plus "nothing fits" files left alone, over all files.
- **recommended thresholds**: the loosest (`MIN_PROBABILITY`, `MIN_LEAD`) with ≥ 95 % precision on
  validation; the test rows show whether that holds on unseen programs.

Probabilities are computed as the app does: the checkpoint's temperature for the option-count
bucket (else its choice temperature), without the [0.5, 5] clamp laya's own runtime adds.

## 5. Publish

1. Upload the four files in `dist/` as assets of a release tagged `laya-uni-1` on bobanes1/file-sorter.
2. Paste `dist/FILES.rs` over `FILES` in `src/laya.rs` and point the download at the release.
3. Set `MIN_PROBABILITY` / `MIN_LEAD` to the recommended ones.

`pack.py` writes `tokenizer.json` without the 48-token truncation the saved tokenizer carries from
training (laya cuts option texts that way); the app's tokenizer would otherwise cut every file's
text to 48 tokens as well.

## Time and cost

Measured on this laptop's RTX 5070 (8 GB): about 30 rows per second in training with the default
settings, for rows like the synthetic ones (a file text of 60–200 words and 12 options, about 350
tokens in all; micro-batch 8 fits in 8 GB). A 24 GB card such as an RTX 4090 or L40S is two to three
times as fast. Rows with long texts (real files, up to the 1024-token limit) take about three times
as long each.

| rows in train.jsonl | 3 epochs on an RTX 4090 (estimate) | on the RTX 5070 here |
|---|---|---|
| 5 000 | ~5 min | ~8 min |
| 20 000 | ~15 min | ~35 min |

On top of that: the install (about 5 GB, 3–5 minutes), the base download (650 MB) and the
evaluation (a few minutes). A 24 GB card rents for about 0.3–0.8 USD an hour, so a run costs well
under 1 USD; leave the box running only as long as you need it. Because 8 GB is enough, the training
also runs on this laptop: `./run_remote.sh` works here as it is.

## Smoke test here (CPU)

```bash
python3 smoke_rows.py smoke && DATA=smoke TORCH_INDEX=https://download.pytorch.org/whl/cpu ./run_remote.sh --epochs 1 --micro-batch 4 --grad-accum 2 --no-amp
```

About two minutes after the install. It only shows that everything runs: the 40 rows are made up
and the validation rows are also in training. laya warns that the calibration rests on 4 items and
that the question differs between rows; both are expected (every file has its own options).
