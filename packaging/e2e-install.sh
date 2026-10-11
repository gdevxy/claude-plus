#!/bin/sh
set -eu

[ "$#" -eq 1 ] || {
  echo "usage: e2e-install.sh <claude-plus binary>" >&2
  exit 2
}
binary=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
scratch=$(mktemp -d /tmp/claude-plus-e2e.XXXXXX)
export HOME="$scratch/home"
export CLAUDE_CONFIG_DIR="$HOME/.claude"
export CLAUDE_PLUS_HOME="$HOME/.claude-plus"
export CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1
export DISABLE_AUTOUPDATER=1
cleanup() {
  "$binary" daemon stop >/dev/null 2>&1 || true
  rm -rf "$scratch"
}
trap cleanup EXIT
trap "cleanup; exit 130" INT
trap "cleanup; exit 143" TERM
mkdir -p "$CLAUDE_CONFIG_DIR"

check() {
  printf '✻ %s\n' "$1"
}

fail() {
  echo "$1" >&2
  exit 1
}

version=$("$binary" --version | cut -d ' ' -f 2)

status=0
printf 'n\n' | "$binary" install >"$scratch/declined" 2>&1 || status=$?
[ "$status" -eq 1 ] || fail "a declined install exited $status"
grep -q "install cancelled, claude-plus changed nothing" "$scratch/declined" ||
  fail "a declined install did not say it changed nothing: $(cat "$scratch/declined")"
[ ! -e "$CLAUDE_CONFIG_DIR/settings.json" ] || fail "a declined install changed $CLAUDE_CONFIG_DIR/settings.json"
[ ! -e "$CLAUDE_PLUS_HOME" ] || fail "a declined install created $CLAUDE_PLUS_HOME"
check "a declined install changes nothing"

"$binary" install --yes
claude plugin list --json | grep -q '"id": "plus@claude-plus"'
claude plugin validate "$CLAUDE_PLUS_HOME/marketplace"
claude plugin validate "$CLAUDE_PLUS_HOME/marketplace/mod"
check "install added the marketplace and the mod"

before=$(cat "$CLAUDE_CONFIG_DIR/settings.json")
"$binary" install </dev/null
[ "$(cat "$CLAUDE_CONFIG_DIR/settings.json")" = "$before" ] || fail "a second install changed the settings"
check "a second install asks nothing and changes nothing"

manifest="$CLAUDE_PLUS_HOME/marketplace/mod/.claude-plugin/plugin.json"
sed "s/\"version\": \"$version\"/\"version\": \"0.0.1\"/" "$manifest" > "$scratch/plugin.json"
cat "$scratch/plugin.json" > "$manifest"
"$binary" daemon --ensure > "$scratch/ensure"
grep -q "Updated the plus mod from v0.0.1 to v$version" "$scratch/ensure" ||
  fail "daemon --ensure did not refresh an older mod: $(cat "$scratch/ensure")"
claude plugin list --json | grep -q "\"folderVersion\": \"$version\"" ||
  fail "Claude Code does not read v$version from the refreshed folder"
check "daemon --ensure refreshes an older mod in place"

cp -R "$CLAUDE_PLUS_HOME/marketplace" "$scratch/elsewhere"
claude plugin marketplace remove claude-plus >/dev/null
claude plugin marketplace add "$scratch/elsewhere" >/dev/null
"$binary" install --yes > "$scratch/repointed"
grep -q "Pointed the claude-plus marketplace at $CLAUDE_PLUS_HOME/marketplace (was $scratch/elsewhere)" "$scratch/repointed" ||
  fail "install did not point the marketplace back: $(cat "$scratch/repointed")"
claude plugin marketplace list --json | grep -q "\"path\": \"$CLAUDE_PLUS_HOME/marketplace\"" ||
  fail "the claude-plus marketplace does not point at $CLAUDE_PLUS_HOME/marketplace"
check "install points a claude-plus marketplace elsewhere back at its folder"

"$binary" doctor > "$scratch/doctor"
grep -q "^  Mod:         plus v" "$scratch/doctor"
check "doctor names the installed mod"

"$binary" uninstall --purge --yes
if claude plugin list --json | grep -q 'plus@claude-plus'; then
  fail "uninstall left the mod installed"
fi
if claude plugin marketplace list --json | grep -q '"claude-plus"'; then
  fail "uninstall left the marketplace"
fi
[ ! -e "$CLAUDE_PLUS_HOME" ] || fail "uninstall --purge left $CLAUDE_PLUS_HOME"
check "uninstall --purge removed the mod, the marketplace and the data"
