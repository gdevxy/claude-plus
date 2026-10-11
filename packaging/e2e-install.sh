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
trap cleanup EXIT INT TERM
mkdir -p "$CLAUDE_CONFIG_DIR"

check() {
  printf '✻ %s\n' "$1"
}

if printf 'n\n' | "$binary" install >/dev/null 2>&1; then
  echo "install went ahead without consent" >&2
  exit 1
fi
[ ! -e "$CLAUDE_CONFIG_DIR/settings.json" ] || {
  echo "a declined install changed $CLAUDE_CONFIG_DIR/settings.json" >&2
  exit 1
}
check "a declined install changes nothing"

"$binary" install --yes
claude plugin list --json | grep -q '"id": "plus@claude-plus"'
claude plugin validate "$CLAUDE_PLUS_HOME/marketplace"
claude plugin validate "$CLAUDE_PLUS_HOME/marketplace/mod"
check "install added the marketplace and the mod"

before=$(cat "$CLAUDE_CONFIG_DIR/settings.json")
"$binary" install </dev/null
[ "$(cat "$CLAUDE_CONFIG_DIR/settings.json")" = "$before" ] || {
  echo "a second install changed the settings" >&2
  exit 1
}
check "a second install asks nothing and changes nothing"

"$binary" doctor > "$scratch/doctor"
grep -q "^  Mod:         plus v" "$scratch/doctor"
check "doctor names the installed mod"

"$binary" uninstall --purge --yes
if claude plugin list --json | grep -q 'plus@claude-plus'; then
  echo "uninstall left the mod installed" >&2
  exit 1
fi
if claude plugin marketplace list --json | grep -q '"claude-plus"'; then
  echo "uninstall left the marketplace" >&2
  exit 1
fi
[ ! -e "$CLAUDE_PLUS_HOME" ] || {
  echo "uninstall --purge left $CLAUDE_PLUS_HOME" >&2
  exit 1
}
check "uninstall --purge removed the mod, the marketplace and the data"
