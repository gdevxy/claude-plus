# Morning check for claude-plus packaging

Run this by hand in a real terminal, after Tasks 1 to 9 of the M7 plan are on `main`.
This is the first check that changes the real `~/.claude/settings.json`, through `claude plugin`, and only after you answer `y`.
Close every Claude Code session that runs the mod with `--plugin-dir` first, so the installed mod is the only one loaded.

## Install from the working tree

Build the release binary and put it first on `PATH`, as Homebrew or the install script would.
Expected: `claude-plus 0.1.0`.

```bash
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo build --release --locked
export PATH="$PWD/target/release:$PATH"
claude-plus --version
```

- [x] Run `cp ~/.claude/settings.json /tmp/settings.before.json`, then `claude-plus install`: it lists `claude plugin marketplace add` with the full path of `~/.claude-plus/marketplace` and `claude plugin install plus@claude-plus`, says Claude Code records them in `~/.claude/settings.json`, and asks `Continue? [y/N]`.
- [x] Answering `n` prints `claude-plus: install cancelled, claude-plus changed nothing; ...`, `diff /tmp/settings.before.json ~/.claude/settings.json` prints nothing, and `~/.claude-plus/marketplace` does not exist.
- [x] Running it again and answering `y` prints `✻ Added the claude-plus marketplace · ...`, `✻ Installed the plus mod v0.1.0`, a daemon line, no PATH warning, and the line about starting a new session.
- [x] `~/.claude/settings.json` now holds `extraKnownMarketplaces.claude-plus` (a `directory` source at `~/.claude-plus/marketplace`) and `enabledPlugins["plus@claude-plus"]: true`, and nothing else changed.
- [x] `claude plugin list` shows `plus@claude-plus` 0.1.0, read from `~/.claude-plus/marketplace/mod`.
- [x] `claude-plus doctor` shows `Claude Code: 2.1.296` (or newer) and `Mod:         plus v0.1.0 · ~/.claude-plus/marketplace/mod` (the full path).
- [x] A new `claude` session (no `--plugin-dir`) at 144 columns or more docks the pane at once; `/plus` opens it in a narrower terminal; the status entry shows.
- [x] `claude-plus install` a second time asks nothing and says `✻ The plus mod v0.1.0 is installed`.
- [x] `claude-plus stats | head -n 3` prints three lines and no panic.

## Upgrade

- [x] Edit `~/.claude-plus/marketplace/mod/.claude-plugin/plugin.json` to `"version": "0.0.1"`, then run `claude-plus daemon --ensure`: it prints `✻ Updated the plus mod from v0.0.1 to v0.1.0 · new Claude Code sessions load it`, and the file says `0.1.0` again.
- [x] Repeat the edit and start a new Claude Code session instead: the session's own `--ensure` puts the file back to `0.1.0`, and the pane works from the session after it.
- [x] Run `echo other > ~/.claude-plus/marketplace/.bundle-hash`, then `claude-plus daemon --ensure`: it prints `✻ Updated the plus mod v0.1.0 to this build · new Claude Code sessions load it`, as a rebuild from `main` without a version bump would.
- [x] Edit `plugin.json` to `"version": "999.0.0"`, then run `claude-plus install`: it prints `✻ Kept the plus mod v999.0.0, newer than this claude-plus v0.1.0`, runs no `claude plugin update`, and the file still says `999.0.0`; then edit it back to `0.1.0`.
- [x] In a new shell with a scratch home (`export HOME=$(mktemp -d); unset CLAUDE_CONFIG_DIR CLAUDE_PLUS_HOME`), run `claude plugin marketplace add <owner>/claude-plus` and `claude plugin install plus@claude-plus` once a release newer than the binary exists, then `claude-plus install`: it prints `claude-plus: the plus mod v<newer> comes from the claude-plus marketplace at <owner>/claude-plus, and this claude-plus carries the older v0.1.0; ...` and runs no `claude plugin` command that changes anything.

## Uninstall

- [x] `claude-plus uninstall` lists the two `claude plugin` commands, asks, and after `y` prints `✻ Uninstalled the plus mod`, `✻ Removed the claude-plus marketplace`, `✻ Stopped the claude-plus daemon · pid ...` and the line about open sessions.
- [x] `~/.claude/settings.json` no longer lists the marketplace or the plugin, `~/.claude-plus/marketplace` is gone, and `~/.claude-plus/data.db` is still there.
- [x] A new `claude` session has no pane and no `/plus`.
- [x] `CLAUDE_PLUS_HOME=. claude-plus uninstall --purge --yes` prints `claude-plus: CLAUDE_PLUS_HOME must be an absolute path, not .`, and `CLAUDE_PLUS_HOME=$HOME claude-plus uninstall --purge --yes` prints `claude-plus: refusing to purge ...: it holds your home directory ...`; neither runs a `claude plugin` command or deletes anything.
- [x] `~/.claude-plus/.claude-plus-data` exists: `--purge` deletes only a directory that carries it, and only the files claude-plus writes there.
- [x] `xxd -s 68 -l 4 ~/.claude-plus/data.db` prints `436c 2b70`, the claude-plus stamp.
- [x] In a new shell with a scratch home (`export HOME=$(mktemp -d) CLAUDE_PLUS_HOME=$HOME/foreign; unset CLAUDE_CONFIG_DIR`), run `mkdir $CLAUDE_PLUS_HOME && echo mine > $CLAUDE_PLUS_HOME/data.db && echo mine > $CLAUDE_PLUS_HOME/daemon.log`: `claude-plus reindex` then fails with `claude-plus: .../data.db is not a claude-plus store, so claude-plus leaves it as it is; move it aside, or set CLAUDE_PLUS_HOME to a directory of its own` and writes no `.claude-plus-data`, and `claude-plus uninstall --purge --yes` refuses and deletes nothing.
- [x] In the same shell, run `rm $CLAUDE_PLUS_HOME/data.db && sqlite3 $CLAUDE_PLUS_HOME/data.db "CREATE TABLE sessions (id TEXT); INSERT INTO sessions VALUES ('mine'); CREATE TABLE meta (key TEXT, value TEXT); PRAGMA user_version = 16;"` and note `shasum $CLAUDE_PLUS_HOME/data.db`: `claude-plus daemon --ensure`, `claude-plus import` and `claude-plus reindex` each refuse it with the same message, the sum is unchanged, there is still no `.claude-plus-data`, and `claude-plus uninstall --purge --yes` refuses and deletes nothing.
- [x] Reinstall with `claude-plus install --yes` to keep using the pane; do not run `--purge` unless you mean to delete the history.

## Releasing (after the GitHub owner is filled in)

Nothing in the plan pushed, tagged or published; these steps are yours.

- [x] Replace `@GITHUB_OWNER@` in `README.md` with the GitHub owner (`git grep -n @GITHUB_OWNER@` lists every place; `packaging/install.sh` and `packaging/claude-plus.rb.in` keep it, because the release fills them in).
- [x] Push `main` to `github.com/<owner>/claude-plus`; the `ci` workflow passes on macOS and Linux, its `install` job included.
- [ ] Push the tag `v0.1.0`; the `release` workflow builds four archives and creates a draft release with them, `SHA256SUMS`, `install.sh` and `claude-plus.rb`.
- [ ] The `release` workflow ran `cargo test` before it built, and its Intel macOS job ran the binary under Rosetta.
- [ ] Download the draft's assets and check `shasum -a 256 -c SHA256SUMS`; publish the draft.
- [ ] Create `github.com/<owner>/homebrew-tap` and commit the draft's `claude-plus.rb` as `Formula/claude-plus.rb`.
- [ ] `brew install <owner>/tap/claude-plus` then `brew test claude-plus` and `brew audit --strict <owner>/tap/claude-plus` pass, and the caveats ask to run `claude-plus uninstall` before `brew uninstall`.
- [ ] `curl -fsSL https://github.com/<owner>/claude-plus/releases/latest/download/install.sh | sh` installs into `~/.local/bin` with the PATH hint when needed.
- [ ] `/plugin install plus --marketplace <owner>/claude-plus` at a Claude Code prompt finds the `plus` mod.

## Outcome

If every line holds, nothing is committed.
Otherwise fix each defect in its owning task's files with a test that pins it, commit it on its own as `fix(<unit>): ...`, and repeat the check.
