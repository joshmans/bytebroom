#!/bin/sh

# Builds ByteBroom, bundles it as a macOS .app, and installs it to /Applications.
# Usage: ./install.sh

# Build the app in release mode
echo "Building ByteBroom..."
cargo build --release

# Bundle the app using cargo-bundle
echo "Bundling ByteBroom..."
cargo bundle --release

# Install the .app bundle to /Applications
echo "Installing ByteBroom to /Applications..."
sudo cp -r target/release/bundle/osx/ByteBroom.app /Applications/

# Set permissions
echo "Setting permissions..."
sudo chmod -R 755 /Applications/ByteBroom.app

# Create a desktop shortcut (optional)
echo "Creating desktop shortcut..."
ln -sf /Applications/ByteBroom.app /Users/$USER/Desktop/ByteBroom.app

echo "Installation complete! Open ByteBroom from /Applications."
