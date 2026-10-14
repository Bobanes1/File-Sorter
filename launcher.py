"""Entry script for PyInstaller (it needs a plain script, not `python -m`)."""

from file_sorter.__main__ import main

if __name__ == "__main__":
    raise SystemExit(main())
