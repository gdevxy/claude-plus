#!/bin/sh
set -eu

[ "$#" -le 1 ] || {
  echo "usage: next-version.sh [version]" >&2
  exit 2
}
current=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
requested=${1:-}

is_version() {
  printf '%s\n' "$1" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'
}

is_released() {
  git rev-parse -q --verify "refs/tags/v$1" >/dev/null
}

is_older() {
  [ "$(printf '%s\n%s\n' "$1" "$2" | sort -t . -k 1,1n -k 2,2n -k 3,3n | head -n 1)" = "$1" ] && [ "$1" != "$2" ]
}

is_version "$current" || {
  echo "next-version.sh: Cargo.toml names $current, which is not a version like 1.2.3" >&2
  exit 1
}
if [ -n "$requested" ]; then
  version=${requested#v}
elif is_released "$current"; then
  version=$(printf '%s' "$current" | awk -F . '{ print $1 "." $2 "." $3 + 1 }')
else
  version=$current
fi
is_version "$version" || {
  echo "next-version.sh: the requested version is not a version like 1.2.3" >&2
  exit 1
}
if is_released "$version"; then
  echo "next-version.sh: v$version is already released" >&2
  exit 1
fi
if is_older "$version" "$current"; then
  echo "next-version.sh: $version is older than $current in Cargo.toml" >&2
  exit 1
fi
echo "$version"
