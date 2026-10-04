#!/bin/sh
# Installs typeshelf so `typeshelf` runs from anywhere. Works on macOS and Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/anwarahmed/typeshelf/main/install.sh | sh
#
# Downloads the prebuilt binary of the latest release; builds from source only where
# there is no prebuilt binary, or when asked to with --source.
set -eu

REPO="anwarahmed/typeshelf"
BIN_DIR="${TYPESHELF_BIN_DIR:-$HOME/.local/bin}"
TARGET="$BIN_DIR/typeshelf"

usage() {
    cat <<EOF
Usage: install.sh [--source | --link | --uninstall]

  (no option)   download the latest release and install it to $BIN_DIR
  --source      build from source instead (needs Rust 1.88+, git and a C compiler)
  --link        build this checkout and symlink to it, so later rebuilds are
                picked up without reinstalling (for development)
  --uninstall   remove typeshelf from $BIN_DIR (settings and progress are kept)

Set TYPESHELF_BIN_DIR to install somewhere other than ~/.local/bin.
EOF
}

die() {
    echo "install.sh: $*" >&2
    exit 1
}

mode=binary
case "${1:-}" in
    "") ;;
    --source) mode=source ;;
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

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# The release asset built for this machine; empty if there is none.
asset_name() {
    case "$(uname -s)-$(uname -m)" in
        Linux-x86_64 | Linux-amd64) echo typeshelf-x86_64-unknown-linux-musl ;;
        Linux-aarch64 | Linux-arm64) echo typeshelf-aarch64-unknown-linux-musl ;;
        Darwin-arm64) echo typeshelf-aarch64-apple-darwin ;;
        Darwin-x86_64) echo typeshelf-x86_64-apple-darwin ;;
    esac
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

# Fetches the latest release's binary into $tmp/typeshelf. Fails (without exiting)
# when there is nothing to download, so the caller can fall back to building.
download_release() {
    asset=$(asset_name)
    [ -n "$asset" ] || { echo "No prebuilt binary for $(uname -s) $(uname -m)."; return 1; }
    command -v curl >/dev/null 2>&1 || { echo "curl not found."; return 1; }
    base="${TYPESHELF_RELEASE_URL:-https://github.com/$REPO/releases/latest/download}"
    echo "Downloading $asset"
    curl -fsSL "$base/$asset" -o "$tmp/typeshelf" || { echo "Could not download a release binary."; return 1; }
    curl -fsSL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS" || die "could not download the release checksums"
    want=$(awk -v f="$asset" '$2 == f || $2 == "*" f { print $1 }' "$tmp/SHA256SUMS")
    [ -n "$want" ] || die "the release has no checksum for $asset"
    got=$(sha256_of "$tmp/typeshelf")
    [ "$want" = "$got" ] || die "checksum mismatch for $asset (expected $want, got $got)"
}

# Prints the directory of a source tree to build: the checkout this script is in or
# was run from, else a fresh clone.
source_dir() {
    case "$0" in
        */*)
            dir=$(cd "$(dirname "$0")" && pwd)
            if [ -f "$dir/Cargo.toml" ]; then
                echo "$dir"
                return
            fi
            ;;
    esac
    if [ -f ./Cargo.toml ] && grep -q '^name = "typeshelf"' ./Cargo.toml; then
        pwd
        return
    fi
    [ "$mode" != link ] || die "--link needs a checkout; clone the repo and run ./install.sh --link from it"
    command -v git >/dev/null 2>&1 || die "git not found; it is needed to fetch the source"
    echo "Fetching https://github.com/$REPO" >&2
    git clone --quiet --depth 1 "https://github.com/$REPO.git" "$tmp/src"
    echo "$tmp/src"
}

build() {
    command -v cargo >/dev/null 2>&1 || die "cargo not found. Install Rust first: https://rustup.rs"
    src=$(source_dir)
    echo "Building typeshelf (the first build takes a minute or two)"
    (cd "$src" && cargo build --release --locked)
    built="$src/target/release/typeshelf"
    [ -x "$built" ] || die "build finished but $built is missing"
}

if [ "$mode" = binary ]; then
    if download_release; then
        built="$tmp/typeshelf"
    else
        echo "Building from source instead."
        mode=source
    fi
fi
[ "$mode" = binary ] || build

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
