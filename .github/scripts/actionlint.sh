#!/bin/sh
set -eu

version=1.7.12
expected=8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8
archive="actionlint_${version}_linux_amd64.tar.gz"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT INT TERM
curl --fail --silent --show-error --location --retry 3 -o "$scratch/$archive" \
  "https://github.com/rhysd/actionlint/releases/download/v$version/$archive"
actual=$(sha256sum "$scratch/$archive" | awk '{print $1}')
if [ "$actual" != "$expected" ]; then
  echo "actionlint.sh: $archive does not match its pinned checksum" >&2
  exit 1
fi
tar -xzf "$scratch/$archive" -C "$scratch" actionlint
"$scratch/actionlint" -color
