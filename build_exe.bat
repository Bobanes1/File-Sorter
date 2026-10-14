@echo off
rem Builds dist\FileSorter.exe on Windows. Requires Python 3.10+ on PATH.
cd /d "%~dp0"
python -m pip install -r requirements.txt pyinstaller || goto :error
python -m PyInstaller --noconfirm --clean FileSorter.spec || goto :error
echo.
echo Done: %cd%\dist\FileSorter.exe
pause
exit /b 0
:error
echo Build failed.
pause
exit /b 1
