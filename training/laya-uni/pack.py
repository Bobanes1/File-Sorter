"""Copy the four files the app downloads out of a checkpoint, and print them for `FILES` in src/laya.rs.

    python3 pack.py out dist

The names are the release assets of the `laya-uni-1` tag on bobanes1/file-sorter, so the first
entry of each tuple is the asset name (the app joins it to the release's download URL).
"""
import hashlib
import shutil
import sys
from pathlib import Path

# (file in the checkpoint, release asset), in the order of FILES in src/laya.rs
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
    print(f"const FILES: [(&str, &str, u64, &str); {len(FILES)}] = [")
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
        size = (dist / name).stat().st_size
        size = f"{size:_}" if size > 99_999 else str(size)   # as rustfmt/clippy write it in laya.rs
        print(f'    (\n        "{name}",\n        "{name}",\n        {size},\n        "{sha256(dist / name)}",\n    ),')
    print("];")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
