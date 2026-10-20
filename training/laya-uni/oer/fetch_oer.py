#!/usr/bin/env python3
"""Download the openly licensed OER test PDFs listed in sources.json.

Layout: <out>/<faculty>/<program>/<module>/<file>, plus <out>/manifest.json with
sha256, bytes, licence and source of every file. Files already present are kept.
sources.json is held-out TEST data: never train on it. The training lists in train/ are
fetched the same way with --sources.

    python3 fetch_oer.py [--sources sources.json] [--workers 4] [out]
"""
import argparse
import collections
import concurrent.futures
import hashlib
import json
import pathlib
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
DEFAULT_OUT = pathlib.Path.home() / ".cache" / "laya-uni" / "oer"
UA = {"User-Agent": "file-sorter-oer-testset/0.1 (+https://github.com/Bobanes1/file-sorter)"}


def part(name):
    # The folders become module labels and live outside the repo: refuse anything that could escape <out>.
    if not name or name in (".", "..") or "/" in name or "\\" in name:
        raise ValueError(f"bad path component: {name!r}")
    return name


def download(url, tries=6):
    url = urllib.parse.quote(url, safe=":/?#[]@!$&'()*+,;=%~")  # urllib needs ASCII: "Génie-logiciel.pdf"
    # Wikibooks renders PDFs on request and answers 429 when asked too fast: back off and retry.
    for i in range(tries):
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=300) as r:
                return r.read()
        except urllib.error.HTTPError as e:
            if e.code not in (429, 502, 503, 504) or i == tries - 1:
                raise
            time.sleep(int(e.headers.get("Retry-After") or 0) or 5 * 2**i)


def fetch(src, out):
    path = out / part(src["faculty"]) / part(src["program"]) / part(src["module"]) / part(src["file"])
    fresh = not path.exists()
    if fresh:
        data = download(src["url"])
        if not data.startswith(b"%PDF"):
            raise ValueError("response is not a PDF")
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_name(path.name + ".part")
        tmp.write_bytes(data)
        tmp.replace(path)  # no half-written PDFs if interrupted
    data = path.read_bytes()
    row = {k: src[k] for k in ("url", "licence", "source", "lang", "kind")}
    row.update(sha256=hashlib.sha256(data).hexdigest(), bytes=len(data))
    return path.relative_to(out).as_posix(), row, fresh


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out", nargs="?", type=pathlib.Path, default=DEFAULT_OUT)
    ap.add_argument("--sources", type=pathlib.Path, default=HERE / "sources.json")
    ap.add_argument("--workers", type=int, default=4, help="parallel downloads; 1 for small private servers")
    args = ap.parse_args()
    sources = json.loads(args.sources.read_text(encoding="utf-8"))
    out = args.out.expanduser().resolve()
    out.mkdir(parents=True, exist_ok=True)

    manifest_path = out / "manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8")) if manifest_path.exists() else {}
    counts, new, failed = collections.Counter(), 0, []
    with concurrent.futures.ThreadPoolExecutor(args.workers) as pool:
        jobs = {pool.submit(fetch, s, out): s for s in sources}
        for job in concurrent.futures.as_completed(jobs):
            src = jobs[job]
            try:
                rel, row, fresh = job.result()
            except Exception as e:
                failed.append(f"{src['url']}: {e}")
                continue
            manifest[rel] = row
            counts[src["faculty"]] += 1
            new += fresh
    manifest_path.write_text(json.dumps(dict(sorted(manifest.items())), ensure_ascii=False, indent=1), encoding="utf-8")

    for faculty, n in sorted(counts.items()):
        print(f"{faculty:26} {n:4}")
    print(f"{'total':26} {sum(counts.values()):4}  ({new} downloaded, {len(failed)} failed) -> {out}")
    for f in failed:
        print("FAILED", f, file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
