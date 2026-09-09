#!/bin/sh
# Start a headless sway and print the environment for talking to it.
#
#   eval "$(dev/rig.sh)"
#   ricedir /tmp/ricedir-test
#
# The display number is *not* wayland-2. It is whatever the new compositor
# took, and it goes up every time one is started while an old socket is still
# on disk. Hardcoding it sends the client at a dead socket, which looks exactly
# like a client that will not start.
set -eu

HERE=$(dirname "$0")

if ! pgrep -x sway >/dev/null 2>&1; then
    WLR_BACKENDS=headless WLR_HEADLESS_OUTPUTS=1 \
        setsid nohup sway --config "$HERE/rig.conf" >/tmp/ricedir-rig.log 2>&1 &
    sleep 4
fi

SOCK=$(ls -t /run/user/"$(id -u)"/sway-ipc."$(id -u)".*.sock | head -1)

# Ask the compositor which display it is on rather than guessing.
DISPLAY_NAME=$(
    for candidate in $(ls -t /run/user/"$(id -u)"/wayland-* | grep -v '\.lock$'); do
        name=$(basename "$candidate")
        if WAYLAND_DISPLAY="$name" SWAYSOCK="$SOCK" swaymsg -t get_version >/dev/null 2>&1; then
            echo "$name"
            break
        fi
    done
)

echo "export SWAYSOCK=$SOCK"
echo "export WAYLAND_DISPLAY=$DISPLAY_NAME"
echo "export XDG_CONFIG_HOME=/tmp/ricedir-cfg"
