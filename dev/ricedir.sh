#!/bin/sh
# Run ricedir in the rig, with a home it is allowed to ruin.
#
#   eval "$(dev/rig.sh)"
#   dev/ricedir.sh tmp/tree &     # relative to the rig's home, never ~/
#   dev/wait-for-window.sh
#
# From M2 the rig can delete files for good. The `cfg(test)` guard in
# `jobs::carry_out` is compiled out of this binary, so nothing inside the
# program stops a wrong path in a pointer script. HOME therefore points at
# `tmp/rig-home`, so a delete that wanders off cannot reach anything real.
#
# The home itself is made once and kept, never rebuilt. What is thrown away is
# `~/tmp` inside it, which is where every test tree goes. Two reasons to keep
# the home: the config ricedir wrote on first run stays, so a run does not
# start with the first-run dialogue every time; and a script that rebuilds a
# tree has no reason to be able to remove the directory above it.
#
# HOME is set here rather than exported by `dev/rig.sh` because that one is
# read with `eval` and its variables follow the shell. `CARGO_HOME` is unset
# on this machine, so a HOME that followed the shell would send the next
# `cargo build` looking for its registry in the rig's home.
#
# The cost: Places shows an empty home, because `user-dirs.dirs` and the
# bookmarks file are not there. That is the right default for a test. A
# screenshot for `docs/` wants those directories made first.
set -eu

HERE=$(dirname "$0")
REPO=$(cd "$HERE/.." && pwd)
RIG_HOME=$REPO/tmp/rig-home

if [ -z "${WAYLAND_DISPLAY:-}" ]; then
    echo "ricedir.sh: no WAYLAND_DISPLAY; run  eval \"\$(dev/rig.sh)\"  first" >&2
    exit 1
fi

BINARY=$REPO/target/debug/ricedir
[ -x "$BINARY" ] || BINARY=$REPO/target/release/ricedir
if [ ! -x "$BINARY" ]; then
    echo "ricedir.sh: no binary; run  cargo build  first" >&2
    exit 1
fi

# Made, never removed. `tmp` inside it is the part a test is allowed to empty.
mkdir -p "$RIG_HOME/.config" "$RIG_HOME/tmp"

# Every path is put inside the rig's home, and one that points outside it is
# refused.
#
# **Never write `~/tmp/tree` here.** The calling shell expands `~` before this
# script sees it, and that shell's HOME is the real one. Doing it pointed
# ricedir at `/home/pgb/tmp/tree`, which exists -- so a delete driven by
# `dev/keys.sh` would have taken real files. Write `tmp/tree` instead.
count=$#
i=0
while [ "$i" -lt "$count" ]; do
    arg=$1
    shift
    case "$arg" in
        -*) ;;
        "$RIG_HOME" | "$RIG_HOME"/*) ;;
        /*)
            echo "ricedir.sh: $arg is outside the rig's home" >&2
            echo "ricedir.sh: write it relative to the home, such as tmp/tree." >&2
            echo "ricedir.sh: never ~/ -- your shell expands that against the real home." >&2
            exit 1
            ;;
        *) arg=$RIG_HOME/$arg ;;
    esac
    set -- "$@" "$arg"
    i=$((i + 1))
done

# `exec`, so a kill or a wait reaches ricedir and not a shell holding it.
exec env HOME="$RIG_HOME" XDG_CONFIG_HOME="$RIG_HOME/.config" "$BINARY" "$@"
