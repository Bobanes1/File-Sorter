#!/bin/sh
# Builds FileSorter-x86_64.AppImage from a release build of file-sorter.
#   scripts/build-appimage.sh <file-sorter binary> <output folder>
set -eu
BINARY=$1
OUT=$2
# appimagetool, a fixed release checked against its SHA-256.
TOOL_URL=https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage
TOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
APPDIR=$WORK/AppDir
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/256x256/apps"
cp "$BINARY" "$APPDIR/usr/bin/file-sorter"
cp assets/file-sorter.desktop "$APPDIR/file-sorter.desktop"
cp assets/file-sorter.desktop "$APPDIR/usr/share/applications/file-sorter.desktop"
cp assets/icon.png "$APPDIR/file-sorter.png"
cp assets/icon.png "$APPDIR/usr/share/icons/hicolor/256x256/apps/file-sorter.png"
cat > "$APPDIR/AppRun" <<'SH'
#!/bin/sh
HERE=$(dirname "$(readlink -f "$0")")
exec "$HERE/usr/bin/file-sorter" "$@"
SH
chmod +x "$APPDIR/AppRun" "$APPDIR/usr/bin/file-sorter"

curl -sSL -o "$WORK/appimagetool" "$TOOL_URL"
echo "$TOOL_SHA256  $WORK/appimagetool" | sha256sum -c -
chmod +x "$WORK/appimagetool"
mkdir -p "$OUT"
# Without FUSE (CI, containers) the tool runs from its extracted self.
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$WORK/appimagetool" --no-appstream "$APPDIR" "$OUT/FileSorter-x86_64.AppImage"
ls -la "$OUT/FileSorter-x86_64.AppImage"
