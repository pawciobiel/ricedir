#!/bin/sh
# Wait until ricedir has a window on screen, then return.
#
#   dev/wait-for-window.sh && grim -o HEADLESS-1 shot.png
#
# A screenshot taken before the first frame is a screenshot of the wallpaper,
# and it looks exactly like a rendering bug. Polling the tree beats sleeping:
# it is faster when the machine is idle and correct when it is not.
set -eu

for _ in $(seq 1 500); do
    if swaymsg -t get_tree 2>/dev/null | grep -q ricedir; then
        exit 0
    fi
    sleep 0.02
done

echo "wait-for-window.sh: no ricedir window after ten seconds" >&2
exit 1
