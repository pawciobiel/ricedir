#!/bin/sh
# Send keys to whatever has focus in the headless rig.
#
#   dev/keys.sh <<'KEYS'
#   wait 300
#   key Down
#   wait 100
#   ctrl a
#   type notes.txt
#   KEYS
#
# **Start every script with a wait.** The compositor gives the new keyboard a
# seat and a keymap on a round trip of its own, and a key sent before that
# lands nowhere. The key is not refused and nothing is logged -- the
# application simply never sees it, which reads exactly like a binding that
# does not work. It cost a session's worth of doubt about Ctrl+\ and Ctrl+-,
# both of which turned out to be fine.
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
