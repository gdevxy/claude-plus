#!/bin/sh
set -eu

[ "$#" -eq 3 ] || {
  echo "usage: release-assets.sh <version> <github owner> <dist dir>" >&2
  exit 2
}
version=$1
owner=$2
dist=$3
here=$(cd "$(dirname "$0")" && pwd)
targets="aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-musl x86_64-unknown-linux-musl"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

: > "$dist/SHA256SUMS"
substitutions="s|@VERSION@|$version|g; s|@GITHUB_OWNER@|$owner|g"
for target in $targets; do
  archive="claude-plus-$target.tar.gz"
  [ -f "$dist/$archive" ] || {
    echo "release-assets.sh: $dist/$archive is missing" >&2
    exit 1
  }
  sum=$(sha256 "$dist/$archive")
  printf '%s  %s\n' "$sum" "$archive" >> "$dist/SHA256SUMS"
  key=$(printf '%s' "$target" | tr 'a-z-' 'A-Z_')
  substitutions="$substitutions; s|@SHA256_${key}@|$sum|g"
done
sed "$substitutions" "$here/claude-plus.rb.in" > "$dist/claude-plus.rb"
sed "s|@GITHUB_OWNER@|$owner|g" "$here/install.sh" > "$dist/install.sh"
