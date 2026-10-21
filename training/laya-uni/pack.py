"""Copy the four files the app downloads out of a checkpoint into dist/, and print their sizes and SHA-256.

    python3 pack.py out dist

Then write the release's manifest with `manifest.py` once the thresholds are chosen from the report.
"""
import hashlib
import shutil
import sys
from pathlib import Path

# (file in the checkpoint, release asset)
FILES = [("rl_agent_config.json", "rl_agent_config.json"), ("encoder/config.json", "encoder-config.json"),
         ("tokenizer/tokenizer.json", "tokenizer.json"), ("model.safetensors", "model.safetensors")]


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def main(checkpoint, dist):
    dist = Path(dist)
    dist.mkdir(parents=True, exist_ok=True)
    for src, name in FILES:
        if name == "tokenizer.json":
            # laya cuts option texts to 48 tokens and the saved tokenizer keeps that truncation;
            # the app's `encode` would then cut every file's text to 48 tokens too.
            from tokenizers import Tokenizer

            tok = Tokenizer.from_file(str(Path(checkpoint) / src))
            tok.no_truncation()
            tok.no_padding()
            tok.save(str(dist / name))
        else:
            shutil.copyfile(Path(checkpoint) / src, dist / name)
        print(f"{name}  {(dist / name).stat().st_size}  {sha256(dist / name)}")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
