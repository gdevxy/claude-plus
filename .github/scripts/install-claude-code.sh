#!/bin/sh
set -eu

version=2.1.296
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)
    platform=darwin-arm64
    expected=c9b5341637becbd423ddffc5b254afb645682a3868cb708bbc6cc0e7bb419937
    ;;
  Linux-x86_64)
    platform=linux-x64
    expected=24972e3bc859fab2b46ed4c1e51f7d6130f06d3bd550811a114640de3370d0de
    ;;
  *)
    echo "install-claude-code.sh: no pinned Claude Code $version build for $(uname -s) $(uname -m)" >&2
    exit 1
    ;;
esac
dir="$HOME/.local/bin"
mkdir -p "$dir"
curl --fail --silent --show-error --location --retry 3 -o "$dir/claude.download" \
  "https://downloads.claude.ai/claude-code-releases/$version/$platform/claude"
if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$dir/claude.download" | awk '{print $1}')
else
  actual=$(shasum -a 256 "$dir/claude.download" | awk '{print $1}')
fi
if [ "$actual" != "$expected" ]; then
  rm -f "$dir/claude.download"
  echo "install-claude-code.sh: Claude Code $version for $platform does not match its pinned checksum" >&2
  exit 1
fi
chmod 755 "$dir/claude.download"
mv -f "$dir/claude.download" "$dir/claude"
"$dir/claude" --version
