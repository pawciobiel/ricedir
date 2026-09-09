#!/bin/sh
# Move and click a pointer in the headless rig.
#
#   dev/pointer.sh <<'MOVES'
#   wait 300
#   move 1155 16
#   click
#   MOVES
#
# The size the coordinates are in defaults to the rig's 1280x800; pass another
# with `dev/pointer.sh <width> <height>`.
#
# **Start every script with a wait**, for the same reason `dev/keys.sh` does:
# the compositor sets the new pointer up on a round trip of its own, and
# anything sent before that reaches nobody, silently.
#
# **A move is not a hover.** iced learns where the pointer is from the events
# it receives, so a click with no `move` before it lands wherever the pointer
# was left -- often on another tile. Move, then click.
#
# Builds vpointer on demand. See dev/vpointer/src/main.rs for why a
# long-running pointer is needed rather than `wlrctl pointer`.
set -eu

HERE=$(dirname "$0")
BINARY="$HERE/vpointer/target/release/vpointer"

if [ ! -x "$BINARY" ]; then
    ( cd "$HERE/vpointer" && cargo build --release )
fi

exec "$BINARY" "${1:-1280}" "${2:-800}"
