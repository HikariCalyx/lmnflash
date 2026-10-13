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

# Build macOS universal binary. No cross-arch library prefix is needed: the
# `fastboot` crate enables rusb's `vendored` feature, so libusb is compiled from
# source for the target arch by the `cc` crate, and every other native
# dependency is a system framework.
cargo build --manifest-path Cargo.toml --bin lmnflash --target=aarch64-apple-darwin --release
cargo build --manifest-path Cargo.toml --bin lmnflash --target=x86_64-apple-darwin --release
lipo -create target/{aarch64-apple-darwin,x86_64-apple-darwin}/release/lmnflash -output LMNFlash.app/Contents/MacOS/lmnflash

# Ship Motorola's mfastboot with the bundle: the firmware-flash feature prefers it
# over the built-in fastboot. Only an x86_64 build is available, so it lives in a
# platform folder and is ignored when the bundle runs on Apple silicon (where it
# would need Rosetta).
#
# This is the Developer-ID signed build (hardened runtime, with the same
# entitlements as below): an x86_64 executable has to be signed to run on Apple
# silicon at all, and an unsigned one is killed before Rosetta 2 can translate
# it.
mkdir -p LMNFlash.app/Contents/Resources/mfastboot/darwin_amd64/29.0.6
cp prebuilt_binary/darwin_amd64/29.0.6/mfastboot \
   LMNFlash.app/Contents/Resources/mfastboot/darwin_amd64/29.0.6/mfastboot
chmod +x LMNFlash.app/Contents/MacOS/lmnflash \
         LMNFlash.app/Contents/Resources/mfastboot/darwin_amd64/29.0.6/mfastboot

# Sign the bundle with `macos/entitlements.plist` (the hardened runtime is what
# makes those entitlements mean anything). They are what lets the bundle start
# an x86_64 `mfastboot` on Apple silicon: without them macOS kills a child whose
# code Rosetta 2 has to translate, and the WebView needs the JIT exception as
# well.
#
# The signature is AD-HOC (`--sign -`): this workflow holds no Developer ID
# certificate, and the DMG is not notarized. Swap in `--sign "$IDENTITY"` once
# one is available.
#
# `--deep` is deliberately NOT used: it would replace the signature of the
# `mfastboot` inside the bundle with this ad-hoc one, and the Developer-ID
# signature it ships with is what lets macOS run it.
codesign --force --options runtime --timestamp=none \
         --entitlements "$(dirname "${BASH_SOURCE[0]}")/entitlements.plist" \
         --sign - LMNFlash.app

# Check what was signed, and that the bundled `mfastboot` kept its own
# signature (the DMG is built from this bundle, so this is the last moment the
# change could be noticed).
codesign --verify --verbose=2 LMNFlash.app
codesign --verify --verbose=2 \
         LMNFlash.app/Contents/Resources/mfastboot/darwin_amd64/29.0.6/mfastboot
codesign --display --verbose=4 LMNFlash.app

# Build DMG.
mkdir -p dist
python3 -m dmgbuild -s "$(dirname "${BASH_SOURCE[0]}")/dmgbuild.settings.py" LMNFlash dist/lmnflash-macos_unsigned.dmg
rm -rf lmnflash_macos_workdir
