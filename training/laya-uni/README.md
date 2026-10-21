# laya-uni: training and evaluation

Fine-tunes Laya's multilingual checkpoint (`convaiinnovations/laya`, `multilingual/`, the revision
pinned in `src/laya.rs`) on rows made by `file-sorter --laya-rows` and split by `split.py`, so it
picks the right module for files from every faculty, not just computer science.

| file | does |
|---|---|
| `train.py` | downloads the base checkpoint, runs `laya.train.finetune` (3 epochs, effective batch 64, options shuffled, 1024 / 256 tokens, fp16) |
| `evaluate.py` | scores base and fine-tuned side by side, by faculty and language; coverage / precision for the app's `MIN_PROBABILITY` / `MIN_LEAD` rule and the loosest pair with ≥ 95 % precision on validation |
| `pack.py` | copies the four release files into `dist/` and lists their sizes and SHA-256 |
| `manifest.py` | writes the `manifest.json` the app reads (release name, version, thresholds, files) |
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

The app pins its model: the release's file URLs, sizes and SHA-256 are in `MODEL` and `MODEL_URL` in
`src/laya.rs`, and a newer model reaches users only with a newer File Sorter, after the CI step *Test the
AI model Laya* passed with it. So:

1. **Every model gets its own release.** The tag is the model's name (`laya-uni-2`, `laya-uni-3`, …). Never
   replace or delete the files of a published release: every File Sorter that pins it downloads them from
   there, and a changed file fails its checksum.
2. **Choose the thresholds** from `report/report.md`: the validation table, cross-checked on the test rows.
3. **Write the manifest:** `python3 manifest.py dist laya-uni-2 2 <MIN_PROBABILITY> <MIN_LEAD>`, and create
   the release with the files in `dist/`.
4. **Open a pull request** to File Sorter that puts the new manifest's files, sizes, checksums and
   thresholds into `MODEL` and the release into `MODEL_URL` (and update the numbers in the laya-uni README).

The network tests download into one folder, so run them one at a time:
`FILE_SORTER_LAYA_DIR=/some/dir cargo test laya -- --ignored --test-threads=1`.

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

## laya-uni-1 (October 2026)

Trained on an RTX 4090 in about 20 minutes on 3,789 rows (`results/report*`, not in git). The model is
published as [laya-uni-1](https://github.com/NilsBriggen/laya-uni/releases/tag/laya-uni-1).

| | base | laya-uni |
|---|---|---|
| unseen programs: right module first | 57.5 % | 81.8 % |
| unseen programs: precision at 0.6 / 2.0 | 77.6 % | 93.2 % |
| real OER PDFs: right module first | 59.6 % | 78.8 % |
| real OER PDFs: precision at 0.6 / 2.0 | 75.9 % | 94.7 % |

The app uses 0.7 / 2.0 (on validation: 76 % sorted at 96.5 % precision). Weak spots are formula-heavy
maths PDFs (no gain on 15 real files) and French (only 16 test files).

## laya-uni-2 (October 2026)

Trained on an RTX 5090 on 8,218 rows: the same synthetic rows plus about 4,400 real, openly licensed
course PDFs from 13 faculties (Wikibooks, Wikiversité, HAL, Zenodo, MIT OpenCourseWare, OER
repositories), in German, English, French and Italian. None comes from a course in the OER test set,
and repository boilerplate (HAL cover pages, wiki headers) was removed from their text. The synthetic
test is the same as for laya-uni-1. Published as
[laya-uni-2](https://github.com/NilsBriggen/laya-uni/releases/tag/laya-uni-2).

| | base | laya-uni-1 | laya-uni-2 |
|---|---|---|---|
| unseen programs: right module first | 57.5 % | 81.8 % | 84.1 % |
| unseen programs: precision at 0.6 / 2.0 | 77.6 % | 93.2 % | 93.0 % |
| real OER PDFs: right module first | 59.6 % | 78.8 % | 85.0 % |
| real OER PDFs: sorted / precision at 0.6 / 2.0 | 60.4 / 75.9 % | 62.5 / 94.7 % | 81.7 / 93.9 % |
| real OER maths PDFs (15): right module first | 46.7 % | 46.7 % | 66.7 % |

Its calibrated pair is 0.75 / 2.0 (on validation: 79 % sorted at 95 % precision; on the real PDFs
76 % at 94.5 %). File Sorter sorts with less (`ALONE`, 0.4 / 1.5): on validation that sorts 87 % at
92 % precision. French is still the least tested language (16 test files).

