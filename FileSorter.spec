# PyInstaller build recipe: `pyinstaller FileSorter.spec` → dist/FileSorter(.exe)
from PyInstaller.utils.hooks import collect_data_files, collect_submodules

hiddenimports = (
    collect_submodules("watchdog.observers")  # platform-specific backends are picked at runtime
    + ["pypdf", "pptx", "docx"]  # imported lazily inside extractor.py
)
datas = collect_data_files("pptx") + collect_data_files("docx")

a = Analysis(
    ["launcher.py"],
    pathex=[],
    datas=datas,
    hiddenimports=hiddenimports,
    excludes=["pytest"],
)
pyz = PYZ(a.pure)
exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    name="FileSorter",
    console=False,  # no black terminal window behind the app
    upx=False,
)
