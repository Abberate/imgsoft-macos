#!/usr/bin/env bash
# Builds "dist/ImgSoft.app" (ad-hoc signed), with the `imgsoft` command line tool inside.
#   scripts/bundle.sh             native architecture
#   scripts/bundle.sh --universal Apple Silicon + Intel
#     (needs: rustup target add x86_64-apple-darwin aarch64-apple-darwin, and nasm: brew install nasm)
set -euo pipefail
cd "$(dirname "$0")/.."

APP="dist/ImgSoft.app"
BINS=(imgsoft-app imgsoft)
# Same minimum macOS for both architectures (matches LSMinimumSystemVersion).
export MACOSX_DEPLOYMENT_TARGET=11.0

build() { cargo build --release -p imgsoft -p imgsoft-cli "$@"; }

if [[ "${1:-}" == "--universal" ]]; then
  for t in aarch64-apple-darwin x86_64-apple-darwin; do
    build --target "$t"
  done
  OUT=target/universal
  mkdir -p "$OUT"
  for b in "${BINS[@]}"; do
    lipo -create -output "$OUT/$b" \
      "target/aarch64-apple-darwin/release/$b" "target/x86_64-apple-darwin/release/$b"
  done
else
  build
  OUT=target/release
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
for b in "${BINS[@]}"; do cp "$OUT/$b" "$APP/Contents/MacOS/"; done
cp packaging/Info.plist "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"

ICONSET="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z "$s" "$s" packaging/icon_1024.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s * 2)) $((s * 2)) packaging/icon_1024.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

codesign --force --deep --sign - "$APP"
echo "✓ $APP"
