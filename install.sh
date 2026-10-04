#!/bin/sh
# Builds typeshelf and installs it so `typeshelf` runs from anywhere.
# Works on macOS and Linux. Run it from a checkout, or straight from GitHub:
#
#   curl -fsSL https://raw.githubusercontent.com/anwarahmed/typeshelf/main/install.sh | sh
#
set -eu

REPO_URL="https://github.com/anwarahmed/typeshelf.git"
BIN_DIR="${TYPESHELF_BIN_DIR:-$HOME/.local/bin}"
TARGET="$BIN_DIR/typeshelf"

usage() {
    cat <<EOF
Usage: install.sh [--link | --uninstall]

  (no option)   build and copy typeshelf to $BIN_DIR
  --link        symlink to this checkout's build instead of copying, so later
                rebuilds are picked up without reinstalling (checkout only)
  --uninstall   remove typeshelf from $BIN_DIR (settings and progress are kept)

Set TYPESHELF_BIN_DIR to install somewhere other than ~/.local/bin.
EOF
}

die() {
    echo "install.sh: $*" >&2
    exit 1
}

mode=copy
case "${1:-}" in
    "") ;;
    --link) mode=link ;;
    --uninstall) mode=uninstall ;;
    -h | --help) usage; exit 0 ;;
    *) usage >&2; exit 1 ;;
esac

if [ "$mode" = uninstall ]; then
    if [ -e "$TARGET" ] || [ -L "$TARGET" ]; then
        rm -f "$TARGET"
        echo "Removed $TARGET"
    else
        echo "Nothing to remove: $TARGET does not exist"
    fi
    exit 0
fi

command -v cargo >/dev/null 2>&1 || die "cargo not found. Install Rust first: https://rustup.rs"

# Use the checkout this script sits in; when piped from curl there is none, so clone one.
src=""
case "$0" in
    */*) dir=$(cd "$(dirname "$0")" && pwd) && [ -f "$dir/Cargo.toml" ] && src="$dir" ;;
esac
if [ -z "$src" ] && [ -f ./Cargo.toml ] && grep -q '^name = "typeshelf"' ./Cargo.toml; then
    src=$(pwd)
fi

tmp=""
cleanup() {
    [ -z "$tmp" ] || rm -rf "$tmp"
}
trap cleanup EXIT

if [ -z "$src" ]; then
    [ "$mode" = copy ] || die "--link needs a checkout; clone the repo and run ./install.sh --link from it"
    command -v git >/dev/null 2>&1 || die "git not found; it is needed to fetch the source"
    tmp=$(mktemp -d)
    echo "Fetching $REPO_URL"
    git clone --quiet --depth 1 "$REPO_URL" "$tmp/typeshelf"
    src="$tmp/typeshelf"
fi

echo "Building typeshelf (the first build takes a minute or two)"
(cd "$src" && cargo build --release --locked)
built="$src/target/release/typeshelf"
[ -x "$built" ] || die "build finished but $built is missing"

mkdir -p "$BIN_DIR"
# Remove first: replaces a symlink rather than writing through it, and is safe while
# an older copy is still running.
rm -f "$TARGET"
if [ "$mode" = link ]; then
    ln -s "$built" "$TARGET"
    echo "Linked $TARGET -> $built"
else
    cp "$built" "$TARGET"
    chmod 755 "$TARGET"
    echo "Installed $TARGET"
fi
"$TARGET" --version

case ":$PATH:" in
    *":$BIN_DIR:"*) echo "Run it with: typeshelf" ;;
    *)
        echo
        echo "$BIN_DIR is not on your PATH. Add this line to your shell profile"
        echo "(~/.zshrc on macOS, ~/.bashrc on Linux), then open a new terminal:"
        echo
        echo "  export PATH=\"$BIN_DIR:\$PATH\""
        ;;
esac
