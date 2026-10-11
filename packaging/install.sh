#!/bin/sh
set -eu
trap 'printf "claude-plus install.sh: the script ended before its last line, so nothing was installed; download it again\n" >&2; exit 1' EXIT

github_owner="${CLAUDE_PLUS_GITHUB_OWNER:-@GITHUB_OWNER@}"
version="${CLAUDE_PLUS_VERSION:-latest}"
version="${version#v}"
install_dir="${CLAUDE_PLUS_INSTALL_DIR:-$HOME/.local/bin}"

say() {
  printf '%s\n' "$*"
}

fail() {
  printf 'claude-plus install.sh: %s\n' "$*" >&2
  exit 1
}

detect_target() {
  os=$(uname -s)
  arch=$(uname -m)
  case "$os" in
    Darwin)
      if [ "$arch" = x86_64 ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
        arch=arm64
      fi
      case "$arch" in
        arm64 | aarch64) echo aarch64-apple-darwin ;;
        x86_64) echo x86_64-apple-darwin ;;
        *) fail "claude-plus has no build for $arch Macs" ;;
      esac
      ;;
    Linux)
      case "$arch" in
        aarch64 | arm64) echo aarch64-unknown-linux-musl ;;
        x86_64 | amd64) echo x86_64-unknown-linux-musl ;;
        *) fail "claude-plus has no build for $arch Linux" ;;
      esac
      ;;
    *) fail "claude-plus runs on macOS and Linux, not $os" ;;
  esac
}

download_base() {
  if [ -n "${CLAUDE_PLUS_DOWNLOAD_URL:-}" ]; then
    echo "$CLAUDE_PLUS_DOWNLOAD_URL"
    return
  fi
  case "$github_owner" in
    @*@) fail "this copy of install.sh names no GitHub owner; use the install.sh of a claude-plus release, or set CLAUDE_PLUS_GITHUB_OWNER" ;;
  esac
  if [ "$version" = latest ]; then
    echo "https://github.com/$github_owner/claude-plus/releases/latest/download"
  else
    echo "https://github.com/$github_owner/claude-plus/releases/download/v$version"
  fi
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    fail "neither sha256sum nor shasum is installed, so the download cannot be checked"
  fi
}

main() {
  trap - EXIT
  command -v curl >/dev/null 2>&1 || fail "curl is needed to download claude-plus"
  target=$(detect_target)
  base=$(download_base)
  archive="claude-plus-$target.tar.gz"
  scratch=$(mktemp -d)
  trap 'rm -rf "$scratch"' EXIT
  trap 'rm -rf "$scratch"; exit 130' INT
  trap 'rm -rf "$scratch"; exit 143' TERM

  curl --fail --silent --show-error --location --retry 3 -o "$scratch/$archive" "$base/$archive" ||
    fail "could not download $base/$archive"
  curl --fail --silent --show-error --location --retry 3 -o "$scratch/SHA256SUMS" "$base/SHA256SUMS" ||
    fail "could not download $base/SHA256SUMS"
  expected=$(awk -v name="$archive" '$2 == name || $2 == "*" name { print $1 }' "$scratch/SHA256SUMS")
  [ -n "$expected" ] || fail "SHA256SUMS has no line for $archive"
  [ "$(sha256 "$scratch/$archive")" = "$expected" ] ||
    fail "$archive does not match its SHA256SUMS line, so nothing was installed"

  mkdir -p "$scratch/unpacked"
  tar -xzf "$scratch/$archive" -C "$scratch/unpacked"
  [ -f "$scratch/unpacked/claude-plus" ] || fail "$archive holds no claude-plus binary"
  installed=$("$scratch/unpacked/claude-plus" --version) ||
    fail "the claude-plus in $archive does not run on this machine, so nothing was installed"
  { mkdir -p "$install_dir" && [ -w "$install_dir" ]; } 2>/dev/null ||
    fail "cannot write to $install_dir; set CLAUDE_PLUS_INSTALL_DIR to a directory you can write to"
  cp "$scratch/unpacked/claude-plus" "$install_dir/.claude-plus.new"
  chmod 755 "$install_dir/.claude-plus.new"
  mv -f "$install_dir/.claude-plus.new" "$install_dir/claude-plus"

  say "✻ Installed $installed to $install_dir/claude-plus"
  case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) say "  $install_dir is not on PATH; add it, for example with: export PATH=\"$install_dir:\$PATH\"" ;;
  esac
  say "✻ Next, run claude-plus install to add the pane to Claude Code"
}

main "$@"
