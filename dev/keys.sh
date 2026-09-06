#!/bin/sh
# Send keys to whatever has focus in the headless rig.
#
#   dev/keys.sh <<'KEYS'
#   key Down
#   wait 100
#   ctrl a
#   type notes.txt
#   KEYS
#
# Builds vkeyboard on demand. See dev/vkeyboard/src/main.rs for why this
# exists: nothing else installed here can put a key into a compositor, and
# that gap hid a missing double click for a whole milestone.
set -eu

HERE=$(dirname "$0")
BINARY="$HERE/vkeyboard/target/release/vkeyboard"

if [ ! -x "$BINARY" ]; then
    ( cd "$HERE/vkeyboard" && cargo build --release )
fi

exec "$BINARY"
