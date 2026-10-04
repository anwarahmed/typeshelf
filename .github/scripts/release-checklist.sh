#!/bin/sh
# Fails a pull request that raises the version unless its description has every line of
# the checklist in RELEASING.md, ticked.
#
#   release-checklist.sh <version on main> <version in the pull request> <file holding the description>
set -eu

base=$1
head=$2
body=$3
checklist=$(dirname "$0")/../../RELEASING.md

if [ "$base" = "$head" ]; then
    echo "The version is unchanged ($head): not a release, nothing to check."
    exit 0
fi

if [ "$(printf '%s\n%s\n' "$base" "$head" | sort -V | tail -n 1)" != "$head" ]; then
    echo "::error title=Version goes backwards::Cargo.toml has $head but main has $base."
    exit 1
fi

items=$(mktemp)
text=$(mktemp)
trap 'rm -f "$items" "$text"' EXIT
grep '^- \[ \] ' "$checklist" | sed 's/^- \[ \] //' > "$items"
# Descriptions edited on github.com have CRLF line ends.
tr -d '\r' < "$body" > "$text"

if [ ! -s "$items" ]; then
    echo "::error::No checklist lines found in RELEASING.md."
    exit 1
fi

missing=0
while IFS= read -r item; do
    if grep -qiF -- "- [x] $item" "$text"; then
        echo "ticked:  $item"
    else
        echo "::error title=Release checklist::Not ticked in the pull request description: $item"
        missing=$((missing + 1))
    fi
done < "$items"

if [ "$missing" -gt 0 ]; then
    echo "This pull request releases $head ($base on main). Copy the checklist from RELEASING.md into its description and tick every line."
    exit 1
fi
echo "Release $head: all $(wc -l < "$items" | tr -d ' ') checklist lines are ticked."
