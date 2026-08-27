#!/bin/bash
set -euo pipefail

# Cleanup.
function cleanup {
    rm -rf LMNFlash.iconset LMNFlash.app lmnflash_macos_workdir
}
trap cleanup EXIT
cleanup

# Create directory structure.
mkdir LMNFlash.app{,/Contents{,/{MacOS,Resources}}}

# Generate an appropriate Info.plist.
python3 macos/mako_generate.py "$(dirname "${BASH_SOURCE[0]}")/Info.plist.mako" >LMNFlash.app/Contents/Info.plist

# Create icon.
mkdir LMNFlash.iconset
sips -z 16 16 src/icon.png --out LMNFlash.iconset/icon_16x16.png
sips -z 32 32 src/icon.png --out LMNFlash.iconset/icon_16x16@2x.png
sips -z 32 32 src/icon.png --out LMNFlash.iconset/icon_32x32.png
sips -z 64 64 src/icon.png --out LMNFlash.iconset/icon_32x32@2x.png
sips -z 128 128 src/icon.png --out LMNFlash.iconset/icon_128x128.png
sips -z 256 256 src/icon.png --out LMNFlash.iconset/icon_128x128@2x.png
sips -z 256 256 src/icon.png --out LMNFlash.iconset/icon_256x256.png
sips -z 512 512 src/icon.png --out LMNFlash.iconset/icon_256x256@2x.png
sips -z 512 512 src/icon.png --out LMNFlash.iconset/icon_512x512.png
sips -z 1024 1024 src/icon.png --out LMNFlash.iconset/icon_512x512@2x.png
iconutil -c icns LMNFlash.iconset --output LMNFlash.app/Contents/Resources/LMNFlash.icns
rm -rf LMNFlash.iconset

# Build macOS universal binary.
cargo build --manifest-path Cargo.toml --bin lmnflash --target=aarch64-apple-darwin --release
# X86_64_PKG_CONFIG_PATH / X86_64_LIBRARY_PATH are optional; set in CI to
# point at the Rosetta 2 Homebrew prefix so rusb finds the x86_64 libusb.
env PKG_CONFIG_PATH="${X86_64_PKG_CONFIG_PATH:-}" \
    LIBRARY_PATH="${X86_64_LIBRARY_PATH:-}" \
    cargo build --manifest-path Cargo.toml --bin lmnflash --target=x86_64-apple-darwin --release
lipo -create target/{aarch64-apple-darwin,x86_64-apple-darwin}/release/lmnflash -output LMNFlash.app/Contents/MacOS/lmnflash

# Build DMG.
mkdir -p dist
python3 -m dmgbuild -s "$(dirname "${BASH_SOURCE[0]}")/dmgbuild.settings.py" LMNFlash dist/lmnflash-macos_unsigned.dmg
rm -rf lmnflash_macos_workdir
