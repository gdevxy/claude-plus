#!/bin/sh
set -eu

[ "$#" -eq 1 ] || {
  echo "usage: set-version.sh <version>" >&2
  exit 2
}
version=$1
printf '%s\n' "$version" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' || {
  echo "set-version.sh: $version is not a version like 1.2.3" >&2
  exit 1
}
manifest=Cargo.toml
lock=Cargo.lock
plugin=mod/.claude-plugin/plugin.json

rewrite() {
  file=$1
  shift
  "$@" "$file" > "$file.next"
  mv "$file.next" "$file"
}

rewrite "$manifest" awk -v version="$version" '
  !done && /^version = "/ { print "version = \"" version "\""; done = 1; next }
  { print }
'
rewrite "$lock" awk -v version="$version" '
  ours && /^version = "/ { print "version = \"" version "\""; ours = 0; next }
  { ours = /^name = "claude-plus"$/ }
  { print }
'
rewrite "$plugin" sed "s/^\(  \"version\": \"\)[^\"]*\"/\1$version\"/"

names() {
  grep -qxF "$2" "$1" || {
    echo "set-version.sh: $1 does not name $version" >&2
    exit 1
  }
}

names "$manifest" "version = \"$version\""
names "$plugin" "  \"version\": \"$version\","
grep -A 1 -xF 'name = "claude-plus"' "$lock" | grep -qxF "version = \"$version\"" || {
  echo "set-version.sh: $lock does not name $version" >&2
  exit 1
}
