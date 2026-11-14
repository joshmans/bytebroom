#!/bin/sh
# Builds the files attached to a GitHub release: a .dmg (drag to Applications) and a .zip.
# Usage: ./release.sh   (needs cargo-bundle: cargo install cargo-bundle)
# Output goes to dist/. Upload with: gh release create v<version> dist/*
set -e
cd "$(dirname "$0")"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
arch=$(uname -m) # ponytail: builds for this Mac's chip only; a universal build needs both rustup targets and lipo
name="ByteBroom-$version-macos-$arch"

cargo bundle --release
app=target/release/bundle/osx/ByteBroom.app
# Ad-hoc signature: no Apple account needed. Without one, a downloaded copy is reported
# as "damaged"; with one, macOS only asks the user to allow it once.
codesign --force --deep --sign - "$app"

rm -rf dist && mkdir -p dist/stage
cp -R "$app" dist/stage/
ln -s /Applications dist/stage/Applications
hdiutil create -quiet -volname ByteBroom -srcfolder dist/stage -format UDZO "dist/$name.dmg"
ditto -c -k --keepParent "$app" "dist/$name.zip"
rm -rf dist/stage
ls -lh dist
