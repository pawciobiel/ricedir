#!/bin/sh
# Start a headless sway and print the environment for talking to it.
#
#   eval "$(dev/rig.sh)"
#   ricedir /tmp/ricedir-test
#   dev/wait-for-window.sh          # before the first screenshot
#
# Two things this exists to stop.
#
# **The display number is not `wayland-2`.** Starting a compositor while an
# old socket is still on disk gives the new one `wayland-3`, and a client sent
# at the old number looks exactly like a client that will not start. So the
# compositor is asked which display it is on rather than told.
#
# **Waiting for a clock is not waiting.** This slept four seconds for a sway
# that is ready in 0.28, and five for a ricedir that maps a window in 0.5.
# Polling for the thing itself is both faster and correct: a slow machine
# still works, and a fast one does not sit there.
set -eu

HERE=$(dirname "$0")
RUNTIME=/run/user/$(id -u)

if ! pgrep -x sway >/dev/null 2>&1; then
    WLR_BACKENDS=headless WLR_HEADLESS_OUTPUTS=1 \
        setsid nohup sway --config "$HERE/rig.conf" >/tmp/ricedir-rig.log 2>&1 &
fi

# Up to ten seconds, in fiftieths. Long enough for a loaded machine, and it
# leaves the moment the compositor answers.
SOCK=""
for _ in $(seq 1 500); do
    SOCK=$(ls -t "$RUNTIME"/sway-ipc."$(id -u)".*.sock 2>/dev/null | head -1 || true)
    if [ -n "$SOCK" ] && SWAYSOCK="$SOCK" swaymsg -t get_version >/dev/null 2>&1; then
        break
    fi
    sleep 0.02
done

if [ -z "$SOCK" ]; then
    echo "rig.sh: sway never answered; see /tmp/ricedir-rig.log" >&2
    exit 1
fi

DISPLAY_NAME=""
for candidate in $(ls -t "$RUNTIME"/wayland-* | grep -v '\.lock$'); do
    name=$(basename "$candidate")
    if WAYLAND_DISPLAY="$name" SWAYSOCK="$SOCK" swaymsg -t get_version >/dev/null 2>&1; then
        DISPLAY_NAME="$name"
        break
    fi
done

if [ -z "$DISPLAY_NAME" ]; then
    echo "rig.sh: sway is running but on no display I can find" >&2
    exit 1
fi

# Never the real one. ricedir writes a config on first run and appends to it
# when somebody picks a handler or a bookmark, and a test run has no business
# in `~/.config/ricedir`.
mkdir -p /tmp/ricedir-cfg

echo "export SWAYSOCK=$SOCK"
echo "export WAYLAND_DISPLAY=$DISPLAY_NAME"
echo "export XDG_CONFIG_HOME=/tmp/ricedir-cfg"
