"""End-to-end test: real Firefox downloads into a watched folder.

Starts the sorter in watch mode (the built exe, or `cargo run --release` by default), serves
PDFs slowly from a local web server so Firefox's placeholder phase is exercised,
lets headless Firefox download them, and checks they land in the right module
folder, complete and exactly once.

    python tests/e2e_firefox.py [--exe dist/FileSorter.exe]

Needs Firefox, geckodriver and `pip install selenium`. Not part of `cargo test`.
"""

from __future__ import annotations

import argparse
import http.server
import json
import os
import subprocess
import tempfile
import threading
import time
from pathlib import Path



def make_pdf(path: Path, text: str) -> Path:
    """Write a minimal one-page PDF containing `text`."""
    safe = text.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
    stream = f"BT /F1 24 Tf 72 720 Td ({safe}) Tj ET".encode()
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
        b"<< /Length %d >>\nstream\n" % len(stream) + stream + b"\nendstream",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ]
    out = bytearray(b"%PDF-1.4\n")
    offsets = []
    for i, obj in enumerate(objects, start=1):
        offsets.append(len(out))
        out += b"%d 0 obj\n" % i + obj + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
    for off in offsets:
        out += b"%010d 00000 n \n" % off
    out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objects) + 1, xref)
    path.write_bytes(bytes(out))
    return path

DOWNLOAD_SECONDS = 6  # how long each download takes; longer than the sorter's 2 s "stable" wait
TIMEOUT = 90


def padded_pdf(path: Path, text: str, size: int = 400_000) -> bytes:
    make_pdf(path, text)
    data = path.read_bytes()
    # Trailing PDF comments keep the file valid while making the download take a while.
    return data + b"\n%" + b"x" * (size - len(data)) + b"\n"


class SlowHandler(http.server.BaseHTTPRequestHandler):
    files: dict[str, bytes] = {}

    def do_GET(self):  # noqa: N802
        name = self.path.lstrip("/")
        if name in ("", "index.html"):
            links = "".join(f'<a id="f{i}" href="/{n}">{n}</a><br>' for i, n in enumerate(self.files))
            body = f"<html><body>{links}</body></html>".encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        data = self.files.get(name)
        if data is None:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", "application/pdf")
        self.send_header("Content-Disposition", f'attachment; filename="{name}"')
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        chunks = 30
        step = len(data) // chunks + 1
        for i in range(0, len(data), step):
            self.wfile.write(data[i:i + step])
            self.wfile.flush()
            time.sleep(DOWNLOAD_SECONDS / chunks)

    def log_message(self, *args):
        pass


def make_driver(download_dir: Path):
    from selenium import webdriver
    from selenium.webdriver.firefox.service import Service

    opts = webdriver.FirefoxOptions()
    opts.add_argument("-headless")
    prefs = {
        "browser.download.folderList": 2,
        "browser.download.dir": str(download_dir),
        "browser.download.useDownloadDir": True,
        "browser.download.manager.showWhenStarting": False,
        "browser.download.alwaysOpenPanel": False,
        "browser.download.always_ask_before_handling_new_types": False,
        "browser.download.open_pdf_attachments_inline": False,
        "browser.helperApps.neverAsk.saveToDisk": "application/pdf,application/octet-stream",
        "pdfjs.disabled": True,
    }
    for key, value in prefs.items():
        opts.set_preference(key, value)
    gecko_dir = os.environ.get("GECKOWEBDRIVER")
    service = None
    if gecko_dir:
        exe = Path(gecko_dir) / ("geckodriver.exe" if os.name == "nt" else "geckodriver")
        if exe.exists():
            service = Service(executable_path=str(exe))
    return webdriver.Firefox(options=opts, service=service) if service else webdriver.Firefox(options=opts)


def listing(root: Path) -> str:
    lines = []
    for p in sorted(root.rglob("*")):
        if p.is_file():
            lines.append(f"  {p.relative_to(root)}  ({p.stat().st_size} bytes)")
    return "\n".join(lines) or "  (empty)"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", help="path to FileSorter.exe; default runs cargo run --release")
    args = parser.parse_args()

    work = Path(tempfile.mkdtemp(prefix="fs-e2e-"))
    downloads = work / "Downloads"
    prog = work / "Studium" / "Informatik" / "Programmierung 1"
    ana = work / "Studium" / "Mathematik" / "Analysis I"
    for d in (downloads, prog, ana):
        d.mkdir(parents=True)

    config = {
        "watch_folder": str(downloads),
        "destination_root": str(work / "Studium"),
        "courses": [
            {"name": "Programmierung 1", "subject": "Informatik", "keywords": ["prog1", "programmierung"]},
            {"name": "Analysis I", "subject": "Mathematik", "keywords": ["analysis"]},
        ],
    }
    cfg = work / "config.json"
    cfg.write_text(json.dumps(config), encoding="utf-8")

    src = work / "src"
    src.mkdir()
    SlowHandler.files = {
        "Prog1_VL3.pdf": padded_pdf(src / "a.pdf", "Week 3"),  # matched by filename
        "vorlesung5.pdf": padded_pdf(src / "b.pdf", "Programmierung 1 - Vorlesung 5"),  # matched by content
    }
    expected = {name: len(data) for name, data in SlowHandler.files.items()}

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), SlowHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{server.server_address[1]}"

    cmd = [args.exe] if args.exe else ["cargo", "run", "--release", "--quiet", "--"]
    log_path = work / "sorter-output.txt"
    log_fh = open(log_path, "w")
    repo = Path(__file__).resolve().parent.parent
    sorter = subprocess.Popen(cmd + ["--headless", "--config", str(cfg)], cwd=repo,
                              stdout=log_fh, stderr=subprocess.STDOUT)
    time.sleep(3)

    driver = make_driver(downloads)
    problems: list[str] = ["test did not finish"]
    try:
        driver.get(base + "/index.html")
        for i in range(len(SlowHandler.files)):
            driver.find_element("id", f"f{i}").click()
            time.sleep(0.5)

        def done() -> bool:
            return all((prog / n).is_file() and (prog / n).stat().st_size == size for n, size in expected.items())

        deadline = time.monotonic() + TIMEOUT
        while time.monotonic() < deadline and not done():
            time.sleep(1)
        time.sleep(4)  # let any late duplicate show up before checking

        problems = []
        for name, size in expected.items():
            target = prog / name
            if not target.is_file():
                problems.append(f"{name} missing from {prog.relative_to(work)}")
            elif target.stat().st_size != size:
                problems.append(f"{name} has {target.stat().st_size} bytes, expected {size}")
        leftovers = [p.name for p in downloads.iterdir()]
        if leftovers:
            problems.append(f"left in Downloads: {leftovers}")
        extra = [p.name for p in prog.iterdir() if p.name not in expected]
        if extra:
            problems.append(f"unexpected files in module folder: {extra}")

        # Where Firefox says the files came from (Windows keeps it in the Zone.Identifier
        # stream), and whether a file whose name and text say nothing follows the others
        # from the same page. Reported only: it depends on the browser.
        if os.name == "nt":
            for name in expected:
                try:
                    with open(str(prog / name) + ":Zone.Identifier", errors="replace") as fh:
                        mark = fh.read()
                except OSError as e:
                    mark = f"(none: {e})"
                print(f"Zone.Identifier of {name}:\n  " + mark.strip().replace("\n", "\n  "))
        SlowHandler.files["Blatt3.pdf"] = padded_pdf(src / "c.pdf", "Aufgabe 1", size=50_000)
        driver.get(base + "/index.html")
        driver.find_element("id", f"f{len(SlowHandler.files) - 1}").click()
        for _ in range(20):
            if (prog / "Blatt3.pdf").is_file():
                break
            time.sleep(1)
        where = "Programmierung 1" if (prog / "Blatt3.pdf").is_file() else "not sorted"
        print(f"Blatt3.pdf (same page, nothing in name or text): {where}")
        origins = work / "origins.json"
        print("origins.json:\n" + (origins.read_text(errors="replace") if origins.exists() else "  (none)"))
    finally:
        driver.quit()
        sorter.terminate()
        try:
            sorter.wait(timeout=10)
        except subprocess.TimeoutExpired:
            sorter.kill()
        log_fh.close()
        server.shutdown()

    print("Files after the run:\n" + listing(work))
    print("Sorter output:\n" + (log_path.read_text(errors="replace") or "  (none)"))
    sorter_log = work / "file-sorter.log"
    if sorter_log.exists():
        print("Sorter log file:\n" + sorter_log.read_text(errors="replace"))
    if not problems:
        print("PASS: Firefox downloads were sorted correctly")
        return 0
    print("FAIL:\n  " + "\n  ".join(problems))
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
