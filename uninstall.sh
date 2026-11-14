#!/bin/sh
# Removes ByteBroom from this Mac: the app, its Desktop shortcut, everything it stored,
# its privacy permissions, and this folder's build output. The source code stays.
# Usage: ./uninstall.sh
cd "$(dirname "$0")"

pkill -x bytebroom && sleep 1 # quit it if it is running

remove() {
  if [ -e "$1" ] || [ -L "$1" ]; then
    rm -rf "$1" && echo "Removed $1"
  fi
}

remove /Applications/ByteBroom.app
# Only the shortcut install.sh made, never a real file someone put there.
[ -L "$HOME/Desktop/ByteBroom.app" ] && remove "$HOME/Desktop/ByteBroom.app"

# What the app stores: its icon cache (also under the old name), and what macOS keeps per app.
remove "$HOME/Library/Caches/bytebroom"
remove "$HOME/Library/Caches/space_eater"
remove "$HOME/Library/Saved Application State/com.p32929.bytebroom.savedState"
remove "$HOME/Library/Preferences/com.p32929.bytebroom.plist"
remove "$HOME/Library/HTTPStorages/com.p32929.bytebroom"

# Forget the permissions it was given, such as Full Disk Access.
tccutil reset All com.p32929.bytebroom >/dev/null 2>&1 && echo "Reset its privacy permissions"

# Build output from ./run.sh, ./install.sh and ./release.sh.
remove target
remove dist

echo "ByteBroom is uninstalled."
