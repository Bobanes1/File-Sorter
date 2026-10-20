#!/usr/bin/env bash
# On a rented GPU box (Ubuntu + NVIDIA driver): install, train, evaluate, pack.
#   DATA=data ./run_remote.sh                 # needs $DATA/train.jsonl and val.jsonl (test.jsonl if there)
#   DATA=smoke ./run_remote.sh --epochs 1     # extra arguments go to train.py
# Leaves dist/ (the release files), report/ (report.md, report.json) and out/ (the full checkpoint).
# TORCH_INDEX picks the torch build: cu128 by default, https://download.pytorch.org/whl/cpu for a CPU try.
set -euo pipefail
cd "$(dirname "$0")"
DATA=${DATA:-data}
TORCH_INDEX=${TORCH_INDEX:-https://download.pytorch.org/whl/cu128}

if [ ! -x .venv/bin/python ]; then
    python3 -m venv .venv || { apt-get update && apt-get install -y python3-venv && python3 -m venv .venv; }
fi
. .venv/bin/activate
pip install -q --upgrade pip
pip install -q "torch==$(sed -n 's/^torch==//p' requirements.txt)" --index-url "$TORCH_INDEX"
pip install -q -r requirements.txt
python -c "import torch; print('torch', torch.__version__, 'cuda', torch.cuda.is_available())"

python train.py --train "$DATA/train.jsonl" --val "$DATA/val.jsonl" --out out "$@" 2>&1 | tee train.log
test_rows=()
[ -f "$DATA/test.jsonl" ] && test_rows=(--test "$DATA/test.jsonl")
python evaluate.py --model base=base/multilingual --model laya-uni=out --val "$DATA/val.jsonl" "${test_rows[@]}" --out report
cp train.log out/train_report.json report/
mkdir -p dist
python pack.py out dist | tee dist/FILES.rs
ls -l dist report
