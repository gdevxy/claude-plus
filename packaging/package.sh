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
epoch=${SOURCE_DATE_EPOCH:-$(git -C "$root" log -1 --format=%ct 2>/dev/null || echo 0)}
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
trap 'rm -rf "$staging"; exit 130' INT
trap 'rm -rf "$staging"; exit 143' TERM

stamp() {
  TZ=UTC date -r "$1" +%Y%m%d%H%M.%S 2>/dev/null || TZ=UTC date -d "@$1" +%Y%m%d%H%M.%S
}

cat "$binary" > "$staging/claude-plus"
cat "$root/LICENSE" > "$staging/LICENSE"
cat "$root/README.md" > "$staging/README.md"
chmod 755 "$staging/claude-plus"
chmod 644 "$staging/LICENSE" "$staging/README.md"
TZ=UTC touch -t "$(stamp "$epoch")" "$staging/claude-plus" "$staging/LICENSE" "$staging/README.md"
mkdir -p "$out"
if tar --version 2>/dev/null | grep -q 'GNU tar'; then
  tar --format=ustar --owner=0 --group=0 --numeric-owner --mtime="@$epoch" \
    -cf "$staging/archive.tar" -C "$staging" claude-plus LICENSE README.md
else
  COPYFILE_DISABLE=1 tar --format ustar --uid 0 --gid 0 --uname '' --gname '' \
    --no-xattrs --no-acls --no-fflags --no-mac-metadata \
    -cf "$staging/archive.tar" -C "$staging" claude-plus LICENSE README.md
fi
gzip -n -9 < "$staging/archive.tar" > "$out/claude-plus-$target.tar.gz"
