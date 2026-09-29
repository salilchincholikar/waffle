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
if [ "$CONFIG" = "debug" ]; then
  # A separate app for macOS: its own name and settings, and it never offers itself for
  # files (LSHandlerRank None), so it stays out of Finder's "Open With" next to the
  # installed Waffle. It keeps the document types: an NSDocument app needs them to open files.
  PB=/usr/libexec/PlistBuddy
  PL="$OUT/Contents/Info.plist"
  $PB -c "Set :CFBundleIdentifier com.salilchincholikar.waffle.debug" "$PL"
  $PB -c "Set :CFBundleName Waffle Debug" -c "Set :CFBundleDisplayName Waffle Debug" "$PL"
  i=0
  while $PB -c "Print :CFBundleDocumentTypes:$i" "$PL" >/dev/null 2>&1; do
    $PB -c "Delete :CFBundleDocumentTypes:$i:LSHandlerRank" "$PL" 2>/dev/null || true
    $PB -c "Add :CFBundleDocumentTypes:$i:LSHandlerRank string None" "$PL"
    i=$((i + 1))
  done
fi
cp macos/Resources/AppIcon.icns "$OUT/Contents/Resources/AppIcon.icns"
printf 'APPL????' > "$OUT/Contents/PkgInfo"
codesign --force --sign - "$OUT" >/dev/null 2>&1 || true
# Builds here are for development: keep them out of Finder's "Open With" (the installed
# /Applications/Waffle.app is the one to offer). macOS may still register a build you open.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -u "$OUT" >/dev/null 2>&1 || true
du -sh "$OUT" | awk '{print "✓ " $2 " (" $1 ")"}'
