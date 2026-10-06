#!/bin/sh
# End-to-end tests of install.sh and the self-updater: the built program, against
# releases made up here and served from file://. Needs curl.
#
#   tests/update.sh [path to the typeshelf binary]     default: target/release/typeshelf
#
# A build run from target/ never updates itself, so the tests work on a copy placed
# outside the checkout, the way an installed one is.
set -u

cd "$(dirname "$0")/.." || exit 1
ROOT=$PWD
BIN=${1:-target/release/typeshelf}
case $BIN in /*) ;; *) BIN=$ROOT/$BIN ;; esac
[ -x "$BIN" ] || { echo "update.sh: $BIN is not built (cargo build --release)" >&2; exit 1; }

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

fails=0
pass() { printf 'ok    %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1"; fails=$((fails + 1)); }
is() { # name expected actual
    if [ "$2" = "$3" ]; then pass "$1"; else fail "$1: expected '$2', got '$3'"; fi
}
has() { # name text-to-find text
    case $3 in *"$2"*) pass "$1" ;; *) fail "$1: no '$2' in '$3'" ;; esac
}

VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64 | Linux-amd64) ASSET=typeshelf-x86_64-unknown-linux-musl ;;
    Linux-aarch64 | Linux-arm64) ASSET=typeshelf-aarch64-unknown-linux-musl ;;
    Darwin-arm64) ASSET=typeshelf-aarch64-apple-darwin ;;
    Darwin-x86_64) ASSET=typeshelf-x86_64-apple-darwin ;;
    *) ASSET= ;;
esac
if ! command -v curl >/dev/null 2>&1 || [ -z "$ASSET" ]; then
    echo "skip  everything (no curl, or no release binary for this platform)"
    exit 0
fi

# make_release <dir> <version> <file to publish as this platform's binary>
make_release() {
    mkdir -p "$1"
    cp "$3" "$1/$ASSET"
    echo "$2" > "$1/VERSION"
    echo "$(sha256_of "$1/$ASSET")  $ASSET" > "$1/SHA256SUMS"
}

INST=$TMP/inst/bin/typeshelf
# Throwaway directories for everything the program reads or writes.
installed() {
    XDG_CONFIG_HOME="$TMP/c" XDG_DATA_HOME="$TMP/d" XDG_CACHE_HOME="$TMP/k" XDG_STATE_HOME="$TMP/s" "$@" 2>&1
}
# The copy a user would have: outside any checkout, in a directory they own.
fresh_copy() {
    rm -rf "$TMP/inst"
    mkdir -p "$TMP/inst/bin"
    cp "$BIN" "$INST"
}

# A "newer release" whose binary is a script, so that it is plain which one runs.
printf '#!/bin/sh\necho "typeshelf 99.0.0 (fake)"\n' > "$TMP/fake"
chmod 755 "$TMP/fake"
make_release "$TMP/rel-now" "$VERSION" "$BIN"
make_release "$TMP/rel-new" 99.0.0 "$TMP/fake"
make_release "$TMP/rel-old" 0.0.1 "$TMP/fake"
make_release "$TMP/rel-bad" 99.0.0 "$TMP/fake"
echo "0000000000000000000000000000000000000000000000000000000000000000  $ASSET" > "$TMP/rel-bad/SHA256SUMS"

has "a checkout never updates itself" "running from a source checkout" "$(installed "$BIN" update)"

out=$(TYPESHELF_RELEASE_URL="file://$TMP/rel-now" TYPESHELF_BIN_DIR="$TMP/inst/bin" sh install.sh 2>&1)
has "install.sh: installs the release" "typeshelf $VERSION (" "$(installed "$INST" --version)"
has "install.sh: says where" "Installed $INST" "$out"
out=$(TYPESHELF_RELEASE_URL="file://$TMP/rel-bad" TYPESHELF_BIN_DIR="$TMP/inst2/bin" sh install.sh 2>&1)
has "install.sh: refuses a bad checksum" "checksum mismatch" "$out"
if [ -e "$TMP/inst2/bin/typeshelf" ]; then fail "install.sh: installed despite a bad checksum"; else pass "install.sh: a refused download installs nothing"; fi
TYPESHELF_BIN_DIR="$TMP/inst/bin" sh install.sh --uninstall >/dev/null 2>&1
if [ -e "$INST" ]; then fail "install.sh: --uninstall left the binary"; else pass "install.sh: --uninstall removes it"; fi

fresh_copy
has "update: nothing newer" "typeshelf $VERSION is up to date (latest release is v$VERSION)." "$(TYPESHELF_RELEASE_URL="file://$TMP/rel-now" installed "$INST" update)"
has "update: never downgrades" "is up to date (latest release is v0.0.1)." "$(TYPESHELF_RELEASE_URL="file://$TMP/rel-old" installed "$INST" update)"
has "update: refuses a bad checksum" "checksum mismatch" "$(TYPESHELF_RELEASE_URL="file://$TMP/rel-bad" installed "$INST" update)"
has "update: a refused update changes nothing" "typeshelf $VERSION (" "$(installed "$INST" --version)"
has "update: reports an unreachable server" "could not check for updates" "$(TYPESHELF_RELEASE_URL="file://$TMP/nowhere" installed "$INST" update)"

# What a package does when it installs: a marker beside the binary's directory.
mkdir -p "$TMP/inst/share/typeshelf"
echo "Homebrew; use brew upgrade typeshelf" > "$TMP/inst/share/typeshelf/managed-by"
has "update: a package's copy refuses" "this copy can't update itself: installed with Homebrew; use brew upgrade typeshelf" "$(TYPESHELF_RELEASE_URL="file://$TMP/rel-new" installed "$INST" update)"
has "update: a package's copy is untouched" "typeshelf $VERSION (" "$(installed "$INST" --version)"

# Reached through a link, as Homebrew's bin directory does it: still refused.
mkdir -p "$TMP/link"
ln -s "$INST" "$TMP/link/typeshelf"
has "update: a package's copy refuses through a link too" "installed with Homebrew" "$(TYPESHELF_RELEASE_URL="file://$TMP/rel-new" installed "$TMP/link/typeshelf" update)"

fresh_copy
has "update: installs a newer release" "Updated to 99.0.0." "$(TYPESHELF_RELEASE_URL="file://$TMP/rel-new" installed "$INST" update)"
is "update: the new version is what runs" "typeshelf 99.0.0 (fake)" "$(installed "$INST" --version)"

# Through a link, the real file is replaced and the link is left alone.
fresh_copy
TYPESHELF_RELEASE_URL="file://$TMP/rel-new" installed "$TMP/link/typeshelf" update >/dev/null
if [ -L "$TMP/link/typeshelf" ] && [ "$("$INST" --version)" = "typeshelf 99.0.0 (fake)" ]; then
    pass "update: through a link, the real file is replaced and the link survives"
else
    fail "update: through a link, the link was replaced or the file was not"
fi

echo
if [ "$fails" -gt 0 ]; then
    echo "$fails failed"
    exit 1
fi
echo "all passed"
