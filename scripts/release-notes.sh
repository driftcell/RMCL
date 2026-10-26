#!/usr/bin/env bash
set -euo pipefail

version="${1:?usage: release-notes.sh <version> [changelog] [output]}"
changelog="${2:-CHANGELOG.md}"
output="${3:-release-notes.md}"
version="${version#v}"

awk -v version="$version" '
  $0 ~ "^## \\[" version "\\]" { found=1; next }
  found && $0 ~ "^## \\[" { exit }
  found { print }
' "$changelog" > "$output"

if [[ ! -s "$output" ]]; then
  echo "no release notes found for version ${version} in ${changelog}" >&2
  exit 1
fi
