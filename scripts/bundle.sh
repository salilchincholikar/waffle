#!/bin/sh
# Build Waffle.app (Rust engine + Swift/AppKit app). No Xcode required.
#   scripts/bundle.sh            release build  → build/Waffle.app
#   scripts/bundle.sh debug      debug build with test hooks → build/debug/Waffle.app
set -eu
cd "$(dirname "$0")/.."
CONFIG="${1:-release}"
if [ "$CONFIG" = "debug" ]; then OUT="build/debug/Waffle.app"; else OUT="build/Waffle.app"; fi

echo "▸ Rust engine (release)"
cargo build --release --quiet -p waffle-ffi

echo "▸ Swift app ($CONFIG)"
(cd macos && swift build -c "$CONFIG" --quiet)
BIN="macos/.build/$CONFIG/Waffle"

echo "▸ Bundle"
rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS" "$OUT/Contents/Resources"
cp "$BIN" "$OUT/Contents/MacOS/Waffle"
[ "$CONFIG" = "release" ] && strip -x "$OUT/Contents/MacOS/Waffle" 2>/dev/null || true
cp macos/Resources/Info.plist "$OUT/Contents/Info.plist"
cp macos/Resources/AppIcon.icns "$OUT/Contents/Resources/AppIcon.icns"
printf 'APPL????' > "$OUT/Contents/PkgInfo"
codesign --force --sign - "$OUT" >/dev/null 2>&1 || true
if [ "$CONFIG" = "release" ]; then
  # Let Finder's "Open With" see the document types.
  /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$OUT" >/dev/null 2>&1 || true
fi
du -sh "$OUT" | awk '{print "✓ " $2 " (" $1 ")"}'
