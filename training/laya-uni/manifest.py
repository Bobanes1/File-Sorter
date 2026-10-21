"""Write the manifest.json of a release of NilsBriggen/laya-uni (its files' sizes and SHA-256).

    python3 manifest.py dist laya-uni-2 2 0.7 2.0     # dist/, release tag, version, MIN_PROBABILITY, MIN_LEAD

File Sorter pins its model: the manifest's files, sizes, checksums and thresholds go into `MODEL` in
src/laya.rs with a pull request. So every model gets its own release (tag = name), and published
files are never replaced. Pick the thresholds from report/report.md (validation rows; check that the
test rows hold up too).
"""
import hashlib
import json
import sys
from pathlib import Path

REPO = "https://github.com/NilsBriggen/laya-uni/releases/download"
FILES = ["rl_agent_config.json", "encoder-config.json", "tokenizer.json", "model.safetensors"]


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def main(dist, name, version, min_probability, min_lead):
    dist = Path(dist)
    manifest = {
        "name": name,
        "version": int(version),
        "min_probability": float(min_probability),
        "min_lead": float(min_lead),
        "files": [{"name": f, "url": f"{REPO}/{name}/{f}", "size": (dist / f).stat().st_size,
                   "sha256": sha256(dist / f)} for f in FILES],
    }
    (dist / "manifest.json").write_text(json.dumps(manifest, indent=1) + "\n", encoding="utf-8")
    print(json.dumps(manifest, indent=1))


if __name__ == "__main__":
    if len(sys.argv) != 6:
        sys.exit(__doc__)
    main(*sys.argv[1:])
