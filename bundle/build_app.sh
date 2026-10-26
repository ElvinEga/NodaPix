#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "==> [1/5] Compiling make_icon generator..."
cargo run --manifest-path "$WORKSPACE_ROOT/Cargo.toml" --bin make_icon

echo "==> [2/5] Generating Retina iconset (.icns)..."
ICONSET_DIR="$WORKSPACE_ROOT/bundle/AppIcon.iconset"
rm -rf "$ICONSET_DIR"
mkdir -p "$ICONSET_DIR"

MASTER="$WORKSPACE_ROOT/bundle/icon_1024.png"

sips -z 16 16     "$MASTER" --out "$ICONSET_DIR/icon_16x16.png" > /dev/null
sips -z 32 32     "$MASTER" --out "$ICONSET_DIR/icon_16x16@2x.png" > /dev/null
sips -z 32 32     "$MASTER" --out "$ICONSET_DIR/icon_32x32.png" > /dev/null
sips -z 64 64     "$MASTER" --out "$ICONSET_DIR/icon_32x32@2x.png" > /dev/null
sips -z 128 128   "$MASTER" --out "$ICONSET_DIR/icon_128x128.png" > /dev/null
sips -z 256 256   "$MASTER" --out "$ICONSET_DIR/icon_128x128@2x.png" > /dev/null
sips -z 256 256   "$MASTER" --out "$ICONSET_DIR/icon_256x256.png" > /dev/null
sips -z 512 512   "$MASTER" --out "$ICONSET_DIR/icon_256x256@2x.png" > /dev/null
sips -z 512 512   "$MASTER" --out "$ICONSET_DIR/icon_512x512.png" > /dev/null
sips -z 1024 1024 "$MASTER" --out "$ICONSET_DIR/icon_512x512@2x.png" > /dev/null

iconutil -c icns "$ICONSET_DIR" -o "$WORKSPACE_ROOT/bundle/AppIcon.icns"
rm -rf "$ICONSET_DIR"
echo "    AppIcon.icns created successfully."

echo "==> [3/5] Building release binary..."
cargo build --manifest-path "$WORKSPACE_ROOT/Cargo.toml" --release -p nodapix-app

echo "==> [4/5] Assembling NodaPix.app bundle..."
APP_DIR="$WORKSPACE_ROOT/target/bundle/NodaPix.app"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

cp "$WORKSPACE_ROOT/bundle/Info.plist" "$APP_DIR/Contents/Info.plist"
cp "$WORKSPACE_ROOT/bundle/AppIcon.icns" "$APP_DIR/Contents/Resources/AppIcon.icns"
cp "$WORKSPACE_ROOT/target/release/nodapix-app" "$APP_DIR/Contents/MacOS/nodapix-app"
chmod +x "$APP_DIR/Contents/MacOS/nodapix-app"

echo "==> [5/5] Ad-hoc codesigning bundle..."
codesign --force --deep --sign - "$APP_DIR"

echo "==> NodaPix.app successfully built at:"
echo "    $APP_DIR"
