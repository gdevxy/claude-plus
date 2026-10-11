#!/bin/sh
set -eu

[ "$#" -eq 3 ] || {
  echo "usage: update-tap.sh <tag> <claude-plus repository> <tap git url>" >&2
  exit 2
}
tag=$1
repository=$2
tap=$3
version=${tag#v}
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT INT TERM

gh release download "$tag" --repo "$repository" --pattern claude-plus.rb --dir "$scratch"
grep -q "/releases/download/$tag/" "$scratch/claude-plus.rb" || {
  echo "update-tap.sh: the claude-plus.rb attached to $tag does not download $tag" >&2
  exit 1
}

git clone --quiet --depth=1 "$tap" "$scratch/tap"
mkdir -p "$scratch/tap/Formula"
cp "$scratch/claude-plus.rb" "$scratch/tap/Formula/claude-plus.rb"
cd "$scratch/tap"
git add Formula/claude-plus.rb
if git diff --cached --quiet; then
  echo "update-tap.sh: the tap already has claude-plus $version"
  exit 0
fi
git -c user.name="github-actions[bot]" \
  -c user.email="41898282+github-actions[bot]@users.noreply.github.com" \
  commit --quiet --message "claude-plus $version"
git push --quiet origin HEAD
echo "update-tap.sh: the tap now has claude-plus $version"
