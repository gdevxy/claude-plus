#!/bin/sh
set -eu

[ "$#" -eq 3 ] || {
  echo "usage: package.sh <target> <claude-plus binary> <output dir>" >&2
  exit 2
}
target=$1
binary=$2
out=$3
root=$(cd "$(dirname "$0")/.." && pwd)
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT INT TERM

cp "$binary" "$staging/claude-plus"
chmod 755 "$staging/claude-plus"
cp "$root/LICENSE" "$root/README.md" "$staging/"
mkdir -p "$out"
tar -czf "$out/claude-plus-$target.tar.gz" -C "$staging" claude-plus LICENSE README.md
