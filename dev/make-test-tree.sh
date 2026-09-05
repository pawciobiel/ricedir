#!/bin/sh
# Build a directory that a file manager has to survive.
#
#   dev/make-test-tree.sh [root] [count]
#
# The performance claim in TODO.md -- that a frame costs what the window is
# tall rather than what the directory holds -- means nothing without a
# directory big enough to disprove it. The nasty names are here because a file
# manager meets them: everything except `/` and NUL is legal in a Linux name,
# and code that assumes otherwise breaks on somebody's real files.
#
# Safe to run again: it removes only the root it was given, and refuses a root
# that does not look like one it made.
set -eu

ROOT=${1:-/tmp/ricedir-test}
COUNT=${2:-100000}

case "$ROOT" in
    /|/home|/home/*/|"$HOME")
        echo "make-test-tree: refusing to touch $ROOT" >&2
        exit 1
        ;;
esac

if [ -e "$ROOT" ] && [ ! -e "$ROOT/.ricedir-test-tree" ]; then
    echo "make-test-tree: $ROOT exists and was not made by this script" >&2
    exit 1
fi

rm -rf "$ROOT"
mkdir -p "$ROOT"
: > "$ROOT/.ricedir-test-tree"

# --- the big flat directory --------------------------------------------------
# The one the gate is measured against. `seq` and one `touch` per batch, since
# 100k separate processes would take longer than the test.
BIG="$ROOT/many"
mkdir -p "$BIG"
echo "make-test-tree: $COUNT entries in $BIG"
seq 1 "$COUNT" | sed "s|^|$BIG/file|; s|\$|.txt|" | xargs -n 500 touch

# --- names that are legal and awkward ----------------------------------------
NASTY="$ROOT/nasty"
mkdir -p "$NASTY"
touch "$NASTY/plain.txt"
touch "$NASTY/with a space.txt"
touch "$NASTY/with'a'quote.txt"
touch "$NASTY/with\"a\"double.txt"
touch "$NASTY/with\$a\$dollar.txt"
touch "$NASTY/with;a;semicolon.txt"
touch "$NASTY/--looks-like-a-flag"
touch "$NASTY/-"
touch "$NASTY/.hidden"
touch "$NASTY/no-extension"
touch "$NASTY/UPPER.TXT"
touch "$NASTY/$(printf 'with\na\nnewline')"
touch "$NASTY/$(printf 'with\ta\ttab')"
touch "$NASTY/zażółć-gęślą-jaźń.txt"
touch "$NASTY/日本語のファイル.txt"
touch "$NASTY/emoji-📁-name.txt"
touch "$NASTY/$(printf 'a%.0s' $(seq 1 250)).txt"

# Numbers, to prove the sort reads them as numbers.
for n in 1 2 3 9 10 11 20 100 101 1000; do
    touch "$NASTY/file$n.txt"
done
touch "$NASTY/file007.txt"

# --- links, including the ones that go nowhere -------------------------------
LINKS="$ROOT/links"
mkdir -p "$LINKS/target"
touch "$LINKS/target/inside.txt"
ln -s target "$LINKS/to-a-directory"
ln -s target/inside.txt "$LINKS/to-a-file"
ln -s nowhere "$LINKS/broken"
ln -s /absolutely/nowhere "$LINKS/broken-absolute"
ln -s loop-b "$LINKS/loop-a"
ln -s loop-a "$LINKS/loop-b"
ln -s . "$LINKS/self"

# --- sizes -------------------------------------------------------------------
SIZES="$ROOT/sizes"
mkdir -p "$SIZES"
: > "$SIZES/empty"
printf 'x' > "$SIZES/one-byte"
dd if=/dev/zero of="$SIZES/one-kilobyte" bs=1024 count=1 status=none
dd if=/dev/zero of="$SIZES/one-megabyte" bs=1024 count=1024 status=none
# Sparse, so a 40 GB file costs no disk. The size column should still say 40G.
truncate -s 40G "$SIZES/forty-gigabytes-sparse"

# --- an empty directory, and a deep one --------------------------------------
mkdir -p "$ROOT/empty"
DEEP="$ROOT/deep"
mkdir -p "$DEEP"
CURRENT="$DEEP"
for _ in $(seq 1 40); do
    CURRENT="$CURRENT/down"
    mkdir -p "$CURRENT"
done
touch "$CURRENT/bottom.txt"

# --- one nobody may read -----------------------------------------------------
mkdir -p "$ROOT/forbidden"
touch "$ROOT/forbidden/secret.txt"
chmod 000 "$ROOT/forbidden"

echo "make-test-tree: done"
echo
echo "  $BIG            $COUNT entries -- the gate"
echo "  $NASTY          names that are legal and awkward"
echo "  $LINKS          symlinks, broken ones and a loop"
echo "  $SIZES          empty to a sparse 40G"
echo "  $ROOT/deep      40 levels"
echo "  $ROOT/empty     nothing at all"
echo "  $ROOT/forbidden mode 000, must show an error not a blank window"
