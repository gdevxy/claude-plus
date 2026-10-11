# claude-plus Milestone 7 (Packaging) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One command after installing the binary, `claude-plus install`, gives a working pane in every new Claude Code session: it registers a plugin marketplace that the binary carries, installs the `plus` mod through the `claude plugin` CLI after the user agrees, and starts the daemon, which imports the history; `claude-plus uninstall [--purge]` undoes it; a tag push builds four release archives into a draft GitHub release with checksums, a Homebrew formula and a `curl | sh` installer; CI pins every action and tool it downloads.

**Architecture:** A build script embeds `.claude-plugin/marketplace.json` and the mod's files in the binary, and a new `setup` unit writes them to `~/.claude-plus/marketplace` (`bundle`), wraps the `claude plugin` CLI as JSON-reporting steps (`claude`), and decides which steps an install or an uninstall needs from what Claude Code lists (`install_steps`, `uninstall_steps`).
The CLI gains `install` and `uninstall` (`src/cli/setup.rs`), which ask before any step that makes Claude Code write its settings, then reuse the M2 `control::ensure` to start or replace the daemon; `daemon --ensure`, which the mod runs at every session start, also rewrites an older mod folder, so an upgraded binary updates the pane with no command.
Release packaging is POSIX shell under `packaging/` (archive, checksums, formula and installer rendering, the installer itself), driven by `.github/workflows/release.yml` and tested from Rust against scratch homes.

**Tech Stack:** Rust (edition 2024, MSRV 1.88), one new direct crate (`libc`, already in `Cargo.lock`); Claude Code 2.1.296 (`claude plugin marketplace add|list|remove`, `claude plugin install|update|enable|uninstall|list` with `--json`); POSIX `sh`, `curl`, `tar`, `shasum`/`sha256sum`; Ruby from the system for `ruby -c` and YAML parsing; GitHub Actions with first-party actions only.

**Spec:** `docs/superpowers/specs/2026-10-10-claude-plus-design.md` (section 1 "Success criteria" (one install command, never editing `~/.claude/settings.json`, uninstall with `--purge`) and "Audience and distribution" (Homebrew tap, `curl | sh`, a marketplace in the same repository), 3 "Install and use" and its subcommand table, 4 "Units" `cli` and "Daemon lifecycle" (`--ensure` and its version order), 7 "Error handling and safety" (0700 and 0600, no network in the daemon), 8 "Testing" (CI on macOS and Linux, release binaries for four targets), 9 milestone 7: "Packaging: install and uninstall, release CI, Homebrew tap, install script, plugin marketplace").

This plan starts from `main` at `b83f573`, the end state of the M6 plan with its first review fixes.
Every line below was run on a scratch worktree of that commit, task by task in this order, before the plan was written: each task's new tests failed as its step 2 says, and after its step 3 `cargo fmt --check`, clippy, `cargo test`, `cargo +1.88 check`, `claude plugin validate mod` and `claude plugin validate .` passed.
The Rust test counts went from 405 lib, 38 cli and 21 daemon tests at the start to 432 lib, 39 cli, 21 daemon, 15 setup and 9 packaging tests at the end, and five more runs of the CLI, setup and packaging suites passed the same; `claude plugin test mod` still runs its 248 tests, since the mod is not changed.
`packaging/e2e-install.sh` was also run with the real Claude Code 2.1.296 against a scratch `HOME` under `/tmp`, and passed; the scratch home was deleted by the script.
The x86_64 macOS release build was cross-compiled with `cargo build --release --target x86_64-apple-darwin` and its `--version` ran under Rosetta.
The workflows cannot run locally and no `actionlint` or `shellcheck` is installed here, so they were checked by a YAML parse (`ruby -ryaml`), by `dash -n` on every script, and by the Rust test of Task 9 that pins their rules; CI runs `actionlint` and `shellcheck` from Task 9 on.
If `main` has moved, apply the hunks by the code they quote, not by line number.

## Decisions

The spec is silent on these points, or leaves them to this milestone, so this plan decides them.

1. The binary carries the mod.
   `build.rs` embeds `.claude-plugin/marketplace.json` and every file under `mod/` except `mod/tests/` and the generated `mod/.claude-plugin/types/`, and `claude-plus install` writes them to `~/.claude-plus/marketplace`, which it registers as a directory marketplace.
   So the mod always matches the binary that serves it, install needs no network and no GitHub owner, and Claude Code reads a directory marketplace's plugin in place (`readFromFolder` in `claude plugin list --json`), so a rewrite of the folder reaches the next session with no reinstall; all three were checked against Claude Code 2.1.296 in a scratch home.
2. The repository root is a marketplace too, from the same `.claude-plugin/marketplace.json` (plugin `plus`, source `./mod`), so `/plugin install plus --marketplace <owner>/claude-plus` works as the plugin-authoring reference ("Sharing a mod") describes.
   Both are named `claude-plus`, so the plugin id is `plus@claude-plus` either way.
3. The spec says claude-plus never edits `~/.claude/settings.json`, and the claude-plus process never opens it.
   But `claude plugin marketplace add`, `install`, `enable`, `uninstall` and `marketplace remove` make Claude Code write `extraKnownMarketplaces` and `enabledPlugins` there (checked in a scratch home), so `install` and `uninstall` print the exact commands, say that Claude Code records them in that file, and ask `Continue? [y/N]` before running any of them.
   Only `y` or `yes` (any case) agrees; Enter, any other answer or a closed stdin is no; `--yes` (`-y`) agrees without asking.
   Declining exits 1 with `install cancelled, claude-plus changed nothing; claude-plus install --yes agrees without asking`, and claude-plus wrote nothing, `~/.claude-plus` included; the read-only `claude plugin` listings may still make Claude Code create its own `.claude.json` on a fresh config (fix wave).
   The question goes to stderr, so it shows when stdout is a file (fix wave).
   `claude plugin update` leaves the settings alone (checked by hash), so it runs without a question, and when no step would touch the settings there is no question.
4. `install` decides its steps from `claude plugin marketplace list --json` and `claude plugin list --json`: add the marketplace unless one named `claude-plus` already points at this folder (one that points elsewhere, such as the GitHub one, is pointed here, and the output says where it pointed before); install `plus@claude-plus` unless it is installed at the user scope; update it when its recorded version is older than the binary's; enable it when it is disabled.
   When the folder is newer than the binary, install keeps the folder, never updates down and prints `✻ Kept the plus mod v<newer>, newer than this claude-plus v<version>` (fix wave); only the folder counts, never a plugin record from another marketplace, and a missing folder is always written (residuals).
   When `plus@claude-plus` comes from another marketplace at a version newer than the folder would hold, install stops before it asks or changes anything, names that marketplace, and says to upgrade claude-plus or to uninstall that mod first (residuals); an older or equal one is switched to the folder after the usual question.
   Every step runs with `--json`, and its result is read from the last line of stdout; a failure answers with the command and Claude Code's message, and `not_installed`, `not_configured` or `already_in_goal_state` on an uninstall, a marketplace removal or an enable count as already done.
5. `install` needs Claude Code 2.1.296 or newer, the version the mod API was built against, compared with the semver order `daemon --ensure` already uses (`control::is_older`, made public); a missing `claude` stops it with what to do.
   `uninstall` without Claude Code still stops the daemon and says which command to run where Claude Code is.
6. After the Claude Code steps, `install` calls the M2 `control::ensure`, the version replacement of `daemon --ensure`, and prints its result with the same `ensured` lines: an install after an upgrade restarts an older daemon, and the daemon's own startup backfill is the history import the spec asks for.
7. Upgrades need no command.
   `daemon --ensure`, which the mod runs at every session start, rewrites `~/.claude-plus/marketplace` when the mod there is older than the binary, by the same version order, so it never downgrades, under an exclusive `flock` on `~/.claude-plus/marketplace.lock`, so sessions that start together rewrite it once.
   It never runs `claude`, and a failure only warns on stderr while the daemon is still ensured with exit 0, so the pane never reports a failed daemon for it.
   The folder is swapped in one `renameat2(RENAME_EXCHANGE)` or `renamex_np(RENAME_SWAP)` (through `rustix::fs::renameat_with`), so a reader never finds it missing; only a file system or sandbox that cannot swap (`EINVAL`, `ENOTSUP`, `ENOSYS`, `EPERM`) falls back to two renames, and a folder lost between them is rebuilt from `marketplace.old` (fix wave).
   The folder also holds `.bundle-hash`, a hash of the bundled files, and a folder of the binary's own version with another hash is refreshed too, so a rebuild from `main` reaches the pane without a version bump (fix wave).
   After `brew upgrade`, the next session replaces the daemon and refreshes the mod; `claude-plus install` re-records the version with `claude plugin update`.
8. The pane starts `claude-plus` from `PATH` (the `binaryPath` default).
   `install` does not set `binaryPath`, because that would override the user's own choice and Homebrew's Cellar path changes with every upgrade; it warns instead when `PATH` finds no `claude-plus`, or finds another one than the running binary, and says how to fix it.
9. `uninstall` uninstalls the mod, removes the marketplace, stops the daemon and deletes the marketplace folder and its lock; `--purge` also deletes the data directory, which the question lists.
   `--purge` refuses, before anything runs, a data directory that is a symlink, that holds the home directory or a Claude Code directory (`CLAUDE_CONFIG_DIR` and `~/.claude`), that sits inside one, or that has no `.claude-plus-data` marker (fix wave).
   `ensure_data_dir` writes the marker only into a directory it created or found empty, never into an existing, non-empty one (residuals, R1).
   A store carries the SQLite `application_id` `0x436c2b70` in its header from birth: `Store::open` stamps a new store in a private file, before WAL is on, and links it into place as `data.db` only if no `data.db` exists, so every existing `data.db` is either stamped or not claude-plus's (R1).
   `Store::open` and `open_read_only` accept only a missing `data.db`, which they create so, or a regular file whose header carries the id; they refuse any other file before SQLite opens it, without changing its content or its mode, and say it is not a claude-plus store and should be moved aside (R1).
   claude-plus has never been released, so no store without the id exists, and none is adopted (R1).
   Purge opens the directory with `O_NOFOLLOW`, checks again after the question that the path still names that directory, and deletes through it: `data.db` only when its header carries the `application_id`, its WAL files with it, and the daemon files only when born after the marker (residuals, R1).
   It then deletes only the files claude-plus writes there and removes the directory only when nothing else is left, naming what it kept.
   `HOME` and `CLAUDE_PLUS_HOME` must be absolute for every command, a relative `CLAUDE_CONFIG_DIR` is read from the working directory as Claude Code reads it, and trailing separators are dropped (residuals); the location guards apply to `--purge` alone, the only command that deletes the data directory, and the marketplace folder is never replaced or removed unless it is the claude-plus marketplace.
   A failed `claude plugin` listing only warns, and uninstall still stops the daemon and removes the folder (fix wave).
   Sessions that are open keep the loaded mod, which would start the daemon again within 30 s, so the last line asks to close them.
10. `doctor` gains `Claude Code:` (the version, or why it does not fit) and `Mod:` (what `claude plugin list --json` says loads, from which folder, or the command that fixes it); both only read.
11. Release targets are `aarch64-apple-darwin` and `x86_64-apple-darwin`, both built on `macos-15` (the second cross-compiled), and `aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl`, built natively on `ubuntu-24.04-arm` and `ubuntu-24.04` with `musl-tools`, static so they run on any Linux libc.
    An archive is `claude-plus-<target>.tar.gz` with `claude-plus`, `LICENSE` and `README.md` at its top, named without the version so `releases/latest/download/<archive>` always works.
    Archives are reproducible: files stamped with `SOURCE_DATE_EPOCH` or the commit time, owned by root, without extended attributes, and compressed with `gzip -n` (fix wave).
12. The release workflow runs on a `v*` tag, runs `cargo test`, checks that the tag is the version in `Cargo.toml` and in `mod/.claude-plugin/plugin.json` and that each binary says so (the Intel macOS one under Rosetta), and creates a draft release (fix wave); publishing it and creating the tap are the user's steps, in the check of Task 10.
13. The one placeholder is `@GITHUB_OWNER@`, the GitHub owner of the repository and of its tap `<owner>/homebrew-tap` (so `brew install <owner>/tap/claude-plus`).
    The release fills it in the formula and the installer it attaches, from `$GITHUB_REPOSITORY_OWNER`, together with the version and the checksums, so `README.md` is the only file the user edits; the repository's own `packaging/install.sh` refuses to run until it is given an owner.
14. `packaging/install.sh` is POSIX `sh`: it picks the archive for `uname -s` and `uname -m` (an x86_64 shell under Rosetta gets the arm64 build), downloads it and `SHA256SUMS` with `curl`, checks the sum with `sha256sum` or `shasum -a 256`, and moves the binary into `CLAUDE_PLUS_INSTALL_DIR` (default `~/.local/bin`) in one rename.
    Its steps run in `main`, called on the last line behind an `EXIT` guard, so a truncated download fails and changes nothing (fix wave).
    It does not run `claude-plus install` itself, because that asks a question and under `curl | sh` stdin is the script; it prints the next step and a `PATH` hint when needed.
    `CLAUDE_PLUS_DOWNLOAD_URL` and `CLAUDE_PLUS_VERSION` override where it downloads from, which is how the tests point it at a local release.
15. CI pins what it runs (the M3 carryover): every action by its full commit SHA with the version as a comment (`actions/checkout` v7.0.1, `actions/setup-node` v7.1.0, `actions/upload-artifact` v7.0.2, `actions/download-artifact` v8.0.2, read with `git ls-remote` on 2026-10-11); `dtolnay/rust-toolchain` is replaced by the runners' own `rustup` with the exact toolchains 1.99.0 (the local stable) and 1.88.0; runner images are pinned (`macos-15`, `ubuntu-24.04`); Node is 22.23.3.
    Claude Code 2.1.296 is downloaded straight from `downloads.claude.ai` and checked against the sha256 its release manifest lists, instead of piping `install.sh` into `bash`, which also fetched the latest build first; `actionlint` 1.7.12 is checked the same way.
    Checkouts keep no credentials, the workflows read contents only, and only the job that creates the draft may write.
    A Rust test pins these rules.
16. CI gains an `install` job, which runs `packaging/e2e-install.sh` with the real Claude Code against a scratch `HOME` on macOS and Linux, and a `lint` job with `shellcheck` and `actionlint`; the `mod` job also validates the repository's marketplace.
17. Every CLI test puts a fake `claude` (`tests/fake-claude/claude`, a shell script that keeps its state and a log of its calls in the test's temporary directory) first on `PATH`, so no Rust test runs the real Claude Code or depends on whether it is installed.
18. `claude-plus doctor | head -n 1` panicked with `failed printing to stdout: Broken pipe` (reproduced on `main`); every read-only command, `daemon --ensure` and `daemon stop` included, now restores the default `SIGPIPE` action, so it ends quietly, while the daemon keeps ignoring `SIGPIPE`, which its sockets need (fix wave).
    `install` and `uninstall` keep ignoring it and drop output the reader no longer takes, so a closed pipe never stops them between two Claude Code commands.
    `libc` becomes a direct dependency for it.
19. The mod is not changed: the pane's notice for a missing binary still names no `brew` command (M3 Decision 21), because the owner is still a placeholder.
    `mod/.claude-plugin/plugin.json` must carry the crate's version (a test pins it), so a release bumps both.
20. `README.md` and `LICENSE` (MIT, as `Cargo.toml` says) are added, because the archives and the formula ship them and the plugin-authoring reference puts the install line in the README.

## Global Constraints

- Rust edition 2024; `rust-version = "1.88"` stays; the only new crate is `libc = "0.2.190"`, already in `Cargo.lock`.
- Toolchain prefix for every cargo command: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" &&`.
- Claude Code prefix for every `claude` command: `export PATH="/opt/homebrew/bin:$HOME/.local/bin:$PATH" &&`.
- Nothing pushes, tags, publishes, creates a GitHub release or creates the tap repository; the release workflow only drafts, and only when the user pushes a tag.
- The real `~/.claude` and `~/.claude-plus` are never touched while implementing: tests use temporary directories under `/tmp`, the fake `claude` and `CLAUDE_CONFIG_DIR` and `CLAUDE_PLUS_HOME`; the only real Claude Code run is `packaging/e2e-install.sh`, which exports a scratch `HOME`, `CLAUDE_CONFIG_DIR` and `CLAUDE_PLUS_HOME` under `/tmp` and deletes them on exit.
- No interactive `claude` session, no `--plugin-dir` and no write into `~/.claude/dev-mods` while implementing; the session by hand is Task 10, for the user.
- claude-plus never opens `~/.claude/settings.json` for writing; the Claude Code commands that write it run only after the user agrees at run time or passes `--yes`.
- No network access in the daemon; only `install.sh` and CI download, always checked against a pinned or published sha256.
- The data directory and everything claude-plus writes in it are private: directories 0700, files 0600.
- Never use the em dash character anywhere (code, scripts, docs, output); use `-`.
- No code comments unless absolutely necessary (the one `SAFETY:` line of Task 7 is).
- Every Markdown sentence on its own line.
- Lint warnings, test failures and flaky tests are defects: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo +1.88 check --all-targets`, `claude plugin validate mod` and `claude plugin validate .` pass at the end of every task.
- Commit messages use Conventional Commits and never include a co-author line.
- Work happens directly on `main`.

## Review Focus

1. A `claude-plus` marketplace the user already added from GitHub (`/plugin install plus --marketplace <owner>/claude-plus`) must be pointed at the bundled folder only after asking, and the output must say where it pointed before (Task 3 test `a_marketplace_of_that_name_elsewhere_is_pointed_here_after_asking`).
2. Sessions that start together after an upgrade each run `daemon --ensure`; the folder must be rewritten once and never be left half written (Task 5 test `sessions_starting_together_refresh_the_folder_once`).
3. `--purge` with a `CLAUDE_PLUS_HOME` that holds the Claude Code directory or the home directory must refuse before any Claude Code command runs (Task 4 test `purge_refuses_a_data_dir_that_holds_the_claude_dir_before_changing_anything`).
4. A home directory with a space must install through `install.sh` and get a `PATH` hint that still works when pasted (Task 8 test `the_installer_of_a_release_installs_its_binary_into_a_scratch_home_with_a_space`).
5. Output piped into `head` must end quietly instead of panicking (Task 7 test `a_reader_that_goes_away_ends_the_output_without_a_panic`).

---

## File Structure

| File | Responsibility |
|---|---|
| `.claude-plugin/marketplace.json` (new) | The `claude-plus` marketplace listing the `plus` mod at `./mod`, for the repository and for the bundle. |
| `build.rs` (new) | Embeds the marketplace file and the mod's files (tests and generated types left out) as `FILES`. |
| `src/setup/mod.rs` (new) | `Found`, and which steps an install or an uninstall needs. |
| `src/setup/bundle.rs` (new) | Writes, refreshes and removes `~/.claude-plus/marketplace` atomically under a lock; reads a folder's mod version. |
| `src/setup/claude.rs` (new) | The `claude plugin` CLI: `Step`, its command line and JSON outcome, the lists, the version. |
| `src/cli/setup.rs` (new) | `claude-plus install` and `uninstall`: the question, the steps, the daemon, the `PATH` check, the purge guard. |
| `src/cli/mod.rs`, `src/cli/daemon.rs`, `src/cli/doctor.rs` | The two subcommands and the default `SIGPIPE`; `ensured` and `stopped` shared, the mod refresh in `--ensure`; the `Claude Code:` and `Mod:` lines. |
| `src/daemon/control.rs`, `src/paths.rs`, `src/lib.rs`, `Cargo.toml` | `is_older` made public; `marketplace_dir`; `pub mod setup`; `libc`. |
| `tests/fake-claude/claude` (new), `tests/common/mod.rs` | The fake Claude Code CLI, put first on `PATH` by every CLI test. |
| `tests/setup.rs` (new), `tests/cli.rs` | Install, uninstall, refresh and doctor through the binary; help, doctor lines, the closed pipe. |
| `packaging/install.sh`, `packaging/package.sh`, `packaging/release-assets.sh`, `packaging/claude-plus.rb.in` (new) | The `curl \| sh` installer; one archive; checksums, formula and installer for a release; the formula template. |
| `packaging/e2e-install.sh` (new) | Install, reinstall, doctor and uninstall with the real Claude Code in a scratch home. |
| `tests/packaging.rs` (new) | The installer and the release assets against scratch homes; the workflow rules. |
| `.github/workflows/ci.yml`, `.github/workflows/release.yml` (new), `.github/scripts/install-claude-code.sh` (new), `.github/scripts/actionlint.sh` (new) | Pinned CI with the `install` and `lint` jobs; the draft release; Claude Code and `actionlint` by checksum. |
| `README.md`, `LICENSE` (new) | Install, upgrade, uninstall and commands; the MIT license. |
| `docs/manual-checks/m7-packaging.md` (new) | The check by hand and the release steps. |

## What the user fills in

Nothing here blocks the tasks; these are the steps only the user can take, listed again in the check of Task 10.

- `@GITHUB_OWNER@` is the single placeholder: the GitHub user or organization that will host `claude-plus` and `homebrew-tap`.
  Replace it in `README.md` (three places); `git grep -n @GITHUB_OWNER@` then lists only `packaging/install.sh` and `packaging/claude-plus.rb.in`, which keep it because the release fills them in.
- Push `main` to `github.com/<owner>/claude-plus`, then push the tag `v0.1.0`, and publish the draft release the workflow creates.
- Create `github.com/<owner>/homebrew-tap` and commit the draft's `claude-plus.rb` as `Formula/claude-plus.rb`, for each release.
- If the repository is private, `ubuntu-24.04-arm` runners are not free; build `aarch64-unknown-linux-musl` elsewhere or make the repository public before the first tag.
- Optionally, once the owner is known, name `brew install <owner>/tap/claude-plus` in the pane's missing-binary notice (`mod/hooks/state.ts`).

## Unverified assumptions (look at these first when CI runs)

- `ubuntu-24.04` has `shellcheck` and `gh` preinstalled, and `ubuntu-24.04-arm` exists for this repository.
- `cc` builds the bundled SQLite for the musl targets with `musl-gcc`, which the release workflow names in `CC_x86_64_unknown_linux_musl` and `CC_aarch64_unknown_linux_musl`; only the macOS targets were built here.
- `claude plugin` commands run without a login on a CI runner, as they did in a scratch home here.
- `downloads.claude.ai` keeps the 2.1.296 builds whose checksums `.github/scripts/install-claude-code.sh` pins.

### Task 1: The mod and its marketplace, carried by the binary

**Files:**
- Create: `.claude-plugin/marketplace.json`, `build.rs`, `src/setup/mod.rs`, `src/setup/bundle.rs`
- Modify: `src/lib.rs`, `src/paths.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `setup::bundle::{FILES: &[(&str, &[u8])], VERSION, MARKETPLACE = "claude-plus", PLUGIN_ID = "plus@claude-plus", write(dir: &Path) -> Result<()>, remove(dir: &Path) -> Result<()>, version_in(dir: &Path) -> Option<String>}`; `Paths::marketplace_dir() -> PathBuf` (`<data dir>/marketplace`).

- [ ] **Step 1: Write the failing tests**

The tests come first; `src/setup/bundle.rs` holds only its test module until Step 3.

Create `src/setup/mod.rs`:

```rust
pub mod bundle;
```

In `src/lib.rs`, replace:

```rust
pub mod pricing;
```

with:

```rust
pub mod pricing;
pub mod setup;
```

Create `src/setup/bundle.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn names() -> Vec<&'static str> {
        FILES.iter().map(|(name, _)| *name).collect()
    }

    #[test]
    fn the_bundle_holds_the_marketplace_and_the_mod_without_its_tests() {
        let names = names();
        for expected in [
            ".claude-plugin/marketplace.json",
            "mod/.claude-plugin/plugin.json",
            "mod/hooks/hooks.json",
            "mod/hooks/register.tsx",
            "mod/types/index.d.ts",
        ] {
            assert!(names.contains(&expected), "{expected} is missing");
        }
        assert!(
            names.iter().all(|name| !name.starts_with("mod/tests/")
                && !name.starts_with("mod/.claude-plugin/types/")),
            "{names:?}"
        );
    }

    #[test]
    fn every_bundled_file_is_the_one_in_the_working_tree() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        for (name, bytes) in FILES {
            assert_eq!(std::fs::read(root.join(name)).unwrap(), *bytes, "{name}");
        }
    }

    #[test]
    fn the_mod_and_the_marketplace_name_this_release() {
        let manifest = |name: &str| -> serde_json::Value {
            let (_, bytes) = FILES.iter().find(|(file, _)| *file == name).unwrap();
            serde_json::from_slice(bytes).unwrap()
        };
        assert_eq!(manifest(MANIFEST)["version"], VERSION);
        let marketplace = manifest(".claude-plugin/marketplace.json");
        assert_eq!(marketplace["name"], MARKETPLACE);
        assert_eq!(
            marketplace["plugins"][0]["name"],
            manifest(MANIFEST)["name"]
        );
        assert_eq!(marketplace["plugins"][0]["source"], "./mod");
        assert_eq!(
            PLUGIN_ID,
            format!(
                "{}@{MARKETPLACE}",
                manifest(MANIFEST)["name"].as_str().unwrap()
            )
        );
    }

    #[test]
    fn writing_the_bundle_replaces_the_folder_and_keeps_it_private() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("marketplace");
        std::fs::create_dir_all(dir.join("mod/hooks")).unwrap();
        std::fs::write(dir.join("mod/hooks/stale.ts"), "old").unwrap();
        std::fs::create_dir_all(temp.path().join("marketplace.new")).unwrap();
        write(&dir).unwrap();
        assert!(!dir.join("mod/hooks/stale.ts").exists());
        assert!(!temp.path().join("marketplace.new").exists());
        assert!(!temp.path().join("marketplace.old").exists());
        assert_eq!(version_in(&dir).as_deref(), Some(VERSION));
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.join("mod/hooks")), 0o700);
        assert_eq!(mode(&dir.join("mod/hooks/register.tsx")), 0o600);
        write(&dir).unwrap();
        assert_eq!(version_in(&dir).as_deref(), Some(VERSION));
    }

    #[test]
    fn removing_the_folder_takes_its_lock_too() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("marketplace");
        write(&dir).unwrap();
        remove(&dir).unwrap();
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
        remove(&dir).unwrap();
    }

    #[test]
    fn a_missing_or_unreadable_manifest_has_no_version() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(version_in(temp.path()), None);
        std::fs::create_dir_all(temp.path().join("mod/.claude-plugin")).unwrap();
        std::fs::write(temp.path().join(MANIFEST), "{ not json").unwrap();
        assert_eq!(version_in(temp.path()), None);
    }
}
```

In `src/paths.rs`, insert above these lines:

```rust
    #[test]
    fn overrides_win() {
```

this:

```rust
    #[test]
    fn the_mod_marketplace_lives_in_the_data_dir() {
        let paths = Paths::resolve(Path::new("/home/ada"), None, Some(PathBuf::from("/plus")));
        assert_eq!(paths.marketplace_dir(), PathBuf::from("/plus/marketplace"));
    }

```

- [ ] **Step 2: Run them to verify they fail**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --lib -- setup::bundle paths::tests::the_mod_marketplace`
Expected: FAIL, the crate does not compile: `cannot find value FILES in this scope`, `cannot find function write`, `no method named marketplace_dir`.

- [ ] **Step 3: Implement**

The build script lists the files in a sorted, fixed order and leaves out the mod's tests and the types Claude Code generates; a symbolic link is read as the file it names.
`write` builds the new folder beside the old one (`marketplace.new`) and swaps it in with two renames, so a session never reads a half-written mod.

Create `.claude-plugin/marketplace.json`:

```json
{
  "name": "claude-plus",
  "owner": { "name": "Gabriel Dinant" },
  "metadata": { "description": "Token, cache and cost visibility for Claude Code" },
  "plugins": [
    {
      "name": "plus",
      "source": "./mod",
      "description": "Token usage, context, cache and cost in a pane beside the conversation"
    }
  ]
}
```

Create `build.rs`:

```rust
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const MARKETPLACE: &str = ".claude-plugin/marketplace.json";
const MOD_DIR: &str = "mod";
const LEFT_OUT: &[&str] = &["mod/tests", "mod/.claude-plugin/types"];

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-changed={MARKETPLACE}");
    println!("cargo:rerun-if-changed={MOD_DIR}");
    let mut files = vec![MARKETPLACE.to_string()];
    collect(&root, &root.join(MOD_DIR), &mut files);
    files.sort();
    let mut source = String::from("pub const FILES: &[(&str, &[u8])] = &[\n");
    for file in &files {
        let absolute = root.join(file);
        writeln!(
            source,
            "    ({file:?}, include_bytes!({:?})),",
            absolute.display().to_string()
        )
        .unwrap();
    }
    source.push_str("];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("bundle.rs");
    std::fs::write(out, source).unwrap();
}

fn collect(root: &Path, dir: &Path, files: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let name = entry.file_name();
        let hidden = name
            .to_str()
            .is_some_and(|n| n.starts_with('.') && n != ".claude-plugin");
        if hidden || LEFT_OUT.contains(&relative.as_str()) {
            continue;
        }
        if entry.file_type().unwrap().is_dir() {
            collect(root, &path, files);
        } else {
            files.push(relative);
        }
    }
}
```

In `src/setup/bundle.rs`, insert above the line `#[cfg(test)]`:

```rust
use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rustix::fs::{FlockOperation, flock};

include!(concat!(env!("OUT_DIR"), "/bundle.rs"));

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MARKETPLACE: &str = "claude-plus";
pub const PLUGIN_ID: &str = "plus@claude-plus";
const MANIFEST: &str = "mod/.claude-plugin/plugin.json";

pub fn write(dir: &Path) -> Result<()> {
    let _lock = lock(dir)?;
    replace(dir)
}

fn lock(dir: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = sibling(dir, "lock");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    flock(&file, FlockOperation::LockExclusive)
        .with_context(|| format!("cannot lock {}", path.display()))?;
    Ok(file)
}

pub fn remove(dir: &Path) -> Result<()> {
    remove_if_present(dir)?;
    match std::fs::remove_file(sibling(dir, "lock")) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

fn replace(dir: &Path) -> Result<()> {
    let staging = sibling(dir, "new");
    let previous = sibling(dir, "old");
    for leftover in [&staging, &previous] {
        remove_if_present(leftover)?;
    }
    for (name, bytes) in FILES {
        let path = staging.join(name);
        create_private_dirs(path.parent().expect("a bundled file has a parent"))?;
        write_private(&path, bytes)?;
    }
    if dir.exists() {
        std::fs::rename(dir, &previous)
            .with_context(|| format!("cannot move {} aside", dir.display()))?;
    }
    std::fs::rename(&staging, dir)
        .with_context(|| format!("cannot put the mod in {}", dir.display()))?;
    remove_if_present(&previous)
}

pub fn version_in(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(MANIFEST)).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(manifest.get("version")?.as_str()?.to_string())
}

fn sibling(dir: &Path, suffix: &str) -> PathBuf {
    let mut name = dir.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{suffix}"));
    dir.with_file_name(name)
}

fn remove_if_present(dir: &Path) -> Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("cannot remove {}", dir.display()))
        }
        _ => Ok(()),
    }
}

fn create_private_dirs(dir: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("cannot create {}", dir.display()))
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut file| file.write_all(bytes))
        .with_context(|| format!("cannot write {}", path.display()))
}

```

In `src/paths.rs`, replace:

```rust
    pub fn daemon_log(&self) -> PathBuf {
        self.data_dir.join("daemon.log")
    }
```

with:

```rust
    pub fn daemon_log(&self) -> PathBuf {
        self.data_dir.join("daemon.log")
    }

    pub fn marketplace_dir(&self) -> PathBuf {
        self.data_dir.join("marketplace")
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --lib -- setup::bundle paths::tests::the_mod_marketplace`
Expected: PASS, 7 tests.

- [ ] **Step 5: Run every gate**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Then: `export PATH="/opt/homebrew/bin:$HOME/.local/bin:$PATH" && claude plugin validate mod && claude plugin validate .`
Expected: every command exits 0; `claude plugin validate .` reads the new marketplace file and passes with no warning.

- [ ] **Step 6: Commit**

```bash
git add .claude-plugin/marketplace.json build.rs src/setup/mod.rs src/setup/bundle.rs src/lib.rs src/paths.rs
git commit -m "feat(setup): carry the mod and its marketplace in the binary"
```

### Task 2: The claude plugin CLI as steps

**Files:**
- Create: `src/setup/claude.rs`
- Modify: `src/setup/mod.rs`

**Interfaces:**
- Consumes: `bundle::{MARKETPLACE, PLUGIN_ID}` (Task 1).
- Produces: `claude::{PROGRAM = "claude", MIN_VERSION = "2.1.296", Marketplace { name, source, path, repo, url } with location(), Plugin { id, version, scope, enabled, folder, folder_version } with loaded_version(), Step { AddMarketplace(PathBuf), Install, Update { from }, Enable, Uninstall, RemoveMarketplace } with args(), command_line(), edits_settings(), Outcome { Done(String), AlreadyDone(String) }, outcome(&Step, &str) -> Result<Outcome>, version_of(&str) -> Option<&str>, ClaudeCode { new, default, version() -> Result<Option<String>>, marketplaces(), plugins(), run(&Step) -> Result<Outcome> }, same_path(&Path, &Path) -> bool}`; `setup::{Found { marketplaces, plugins } with marketplace(), plugin(), install_steps(&Found, &Path, &str) -> Vec<Step>, uninstall_steps(&Found) -> Vec<Step>}`.

- [ ] **Step 1: Write the failing tests**

The JSON in these tests is what Claude Code 2.1.296 printed in a scratch home for each command, trimmed to the fields claude-plus reads.

Create `src/setup/claude.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_step_runs_the_claude_plugin_command_it_shows() {
        let add = Step::AddMarketplace(PathBuf::from("/home/ada/.claude-plus/marketplace"));
        assert_eq!(
            add.command_line(),
            "claude plugin marketplace add /home/ada/.claude-plus/marketplace"
        );
        assert_eq!(add.args().last().unwrap(), "--json");
        let lines: Vec<String> = [
            Step::Install,
            Step::Update {
                from: "0.0.9".into(),
            },
            Step::Enable,
            Step::Uninstall,
            Step::RemoveMarketplace,
        ]
        .iter()
        .map(Step::command_line)
        .collect();
        assert_eq!(
            lines,
            [
                "claude plugin install plus@claude-plus",
                "claude plugin update plus@claude-plus",
                "claude plugin enable plus@claude-plus",
                "claude plugin uninstall plus@claude-plus",
                "claude plugin marketplace remove claude-plus",
            ]
        );
    }

    #[test]
    fn only_an_update_leaves_the_settings_alone() {
        assert!(
            !Step::Update {
                from: "0.0.9".into()
            }
            .edits_settings()
        );
        for step in [
            Step::AddMarketplace(PathBuf::from("/m")),
            Step::Install,
            Step::Enable,
            Step::Uninstall,
            Step::RemoveMarketplace,
        ] {
            assert!(step.edits_settings(), "{step:?}");
        }
    }

    #[test]
    fn a_json_reply_is_read_from_the_last_line() {
        let stdout = "Adding marketplace…\n{\"command\":\"marketplace-add\",\"outcome\":\"ok\",\"message\":\"Successfully added marketplace: claude-plus\"}\n";
        assert_eq!(
            outcome(&Step::AddMarketplace(PathBuf::from("/m")), stdout).unwrap(),
            Outcome::Done("Successfully added marketplace: claude-plus".into())
        );
    }

    #[test]
    fn removing_what_is_already_gone_is_no_failure() {
        let gone = r#"{"command":"uninstall","outcome":"failed","message":"Plugin \"plus@claude-plus\" not found in installed plugins","failureCode":"not_installed"}"#;
        assert!(matches!(
            outcome(&Step::Uninstall, gone).unwrap(),
            Outcome::AlreadyDone(_)
        ));
        let missing = r#"{"command":"marketplace-remove","outcome":"failed","message":"Marketplace 'claude-plus' not found","failureCode":"not_configured"}"#;
        assert!(matches!(
            outcome(&Step::RemoveMarketplace, missing).unwrap(),
            Outcome::AlreadyDone(_)
        ));
        let enabled = r#"{"command":"enable","outcome":"failed","message":"Plugin \"plus@claude-plus\" is already enabled","failureCode":"already_in_goal_state"}"#;
        assert!(matches!(
            outcome(&Step::Enable, enabled).unwrap(),
            Outcome::AlreadyDone(_)
        ));
    }

    #[test]
    fn a_failed_step_names_the_command_and_the_reason() {
        let failed = r#"{"command":"install","outcome":"failed","message":"Plugin \"plus\" not found in marketplace \"claude-plus\"","failureCode":"not_found"}"#;
        assert_eq!(
            outcome(&Step::Install, failed).unwrap_err().to_string(),
            "claude plugin install plus@claude-plus failed: Plugin \"plus\" not found in marketplace \"claude-plus\""
        );
        let not_json = outcome(&Step::Install, "Segmentation fault\n").unwrap_err();
        assert_eq!(
            not_json.to_string(),
            "claude plugin install plus@claude-plus answered \"Segmentation fault\""
        );
    }

    #[test]
    fn the_version_is_the_first_word_of_claude_version() {
        assert_eq!(version_of("2.1.296 (Claude Code)\n"), Some("2.1.296"));
        assert_eq!(
            version_of("2.2.0-beta.1 (Claude Code)"),
            Some("2.2.0-beta.1")
        );
        assert_eq!(version_of(""), None);
        assert_eq!(version_of("Claude Code"), None);
    }

    #[test]
    fn lists_read_what_claude_plugin_list_prints() {
        let plugins: Vec<Plugin> = serde_json::from_str(
            r#"[{"id":"plus@claude-plus","version":"0.1.0","scope":"user","enabled":true,"installPath":"/c","readFromFolder":"/m/mod","folderVersion":"0.1.0"}]"#,
        )
        .unwrap();
        assert_eq!(
            plugins,
            [Plugin {
                id: "plus@claude-plus".into(),
                version: "0.1.0".into(),
                scope: "user".into(),
                enabled: true,
                folder: Some("/m/mod".into()),
                folder_version: Some("0.1.0".into()),
            }]
        );
        let marketplaces: Vec<Marketplace> = serde_json::from_str(
            r#"[{"name":"claude-plus","source":"directory","path":"/m","installLocation":"/m"},{"name":"other","source":"github","repo":"ada/other"}]"#,
        )
        .unwrap();
        assert_eq!(marketplaces[0].location(), "/m");
        assert_eq!(marketplaces[1].location(), "ada/other");
    }
}
```

Replace the whole of `src/setup/mod.rs`:

```rust
pub mod bundle;
pub mod claude;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn ours(dir: &Path) -> Marketplace {
        Marketplace {
            name: MARKETPLACE.into(),
            source: "directory".into(),
            path: Some(dir.display().to_string()),
            repo: None,
            url: None,
        }
    }

    fn plus(version: &str, enabled: bool) -> Plugin {
        Plugin {
            id: PLUGIN_ID.into(),
            version: version.into(),
            scope: "user".into(),
            enabled,
            folder: None,
            folder_version: None,
        }
    }

    #[test]
    fn a_fresh_install_adds_the_marketplace_then_the_mod() {
        let dir = PathBuf::from("/home/ada/.claude-plus/marketplace");
        assert_eq!(
            install_steps(&Found::default(), &dir, "0.1.0"),
            [Step::AddMarketplace(dir.clone()), Step::Install]
        );
    }

    #[test]
    fn an_installed_mod_of_this_version_needs_nothing() {
        let dir = PathBuf::from("/home/ada/.claude-plus/marketplace");
        let found = Found {
            marketplaces: vec![ours(&dir)],
            plugins: vec![plus("0.1.0", true)],
        };
        assert_eq!(install_steps(&found, &dir, "0.1.0"), []);
    }

    #[test]
    fn another_version_is_updated_and_a_disabled_mod_enabled() {
        let dir = PathBuf::from("/home/ada/.claude-plus/marketplace");
        let found = Found {
            marketplaces: vec![ours(&dir)],
            plugins: vec![plus("0.0.9", false)],
        };
        assert_eq!(
            install_steps(&found, &dir, "0.1.0"),
            [
                Step::Update {
                    from: "0.0.9".into()
                },
                Step::Enable
            ]
        );
    }

    #[test]
    fn a_marketplace_of_that_name_elsewhere_is_pointed_at_this_folder() {
        let dir = PathBuf::from("/home/ada/.claude-plus/marketplace");
        let github = Marketplace {
            source: "github".into(),
            path: None,
            repo: Some("ada/claude-plus".into()),
            ..ours(&dir)
        };
        let found = Found {
            marketplaces: vec![github],
            plugins: vec![plus("0.1.0", true)],
        };
        assert_eq!(
            install_steps(&found, &dir, "0.1.0"),
            [Step::AddMarketplace(dir.clone())]
        );
    }

    #[test]
    fn a_project_scoped_copy_or_another_plugin_is_not_ours() {
        let dir = PathBuf::from("/m");
        let found = Found {
            marketplaces: vec![ours(&dir)],
            plugins: vec![
                Plugin {
                    scope: "project".into(),
                    ..plus("0.1.0", true)
                },
                Plugin {
                    id: "plus@elsewhere".into(),
                    ..plus("0.1.0", true)
                },
            ],
        };
        assert_eq!(install_steps(&found, &dir, "0.1.0"), [Step::Install]);
        assert_eq!(uninstall_steps(&found), [Step::RemoveMarketplace]);
    }

    #[test]
    fn uninstalling_removes_the_mod_before_its_marketplace() {
        let dir = PathBuf::from("/m");
        let found = Found {
            marketplaces: vec![ours(&dir)],
            plugins: vec![plus("0.1.0", true)],
        };
        assert_eq!(
            uninstall_steps(&found),
            [Step::Uninstall, Step::RemoveMarketplace]
        );
        assert_eq!(uninstall_steps(&Found::default()), []);
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --lib setup::`
Expected: FAIL, the crate does not compile: `cannot find type Step`, `cannot find function outcome`, `cannot find type Marketplace`.

- [ ] **Step 3: Implement**

A missing `claude` is `Ok(None)` from `version`, so callers can say what to do; every other spawn error is an error.
`same_path` compares canonical paths when both exist, because `/tmp` is `/private/tmp` on macOS and Claude Code lists the path as it was given.

In `src/setup/claude.rs`, insert above the line `#[cfg(test)]`:

```rust
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::bundle::{MARKETPLACE, PLUGIN_ID};

pub const PROGRAM: &str = "claude";
pub const MIN_VERSION: &str = "2.1.296";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Marketplace {
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

impl Marketplace {
    pub fn location(&self) -> String {
        self.path
            .clone()
            .or_else(|| self.repo.clone())
            .or_else(|| self.url.clone())
            .unwrap_or_else(|| self.source.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Plugin {
    pub id: String,
    pub version: String,
    pub scope: String,
    pub enabled: bool,
    #[serde(default, rename = "readFromFolder")]
    pub folder: Option<String>,
    #[serde(default, rename = "folderVersion")]
    pub folder_version: Option<String>,
}

impl Plugin {
    pub fn loaded_version(&self) -> &str {
        self.folder_version.as_deref().unwrap_or(&self.version)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    AddMarketplace(PathBuf),
    Install,
    Update { from: String },
    Enable,
    Uninstall,
    RemoveMarketplace,
}

impl Step {
    pub fn args(&self) -> Vec<OsString> {
        let words: &[&str] = match self {
            Step::AddMarketplace(_) => &["plugin", "marketplace", "add"],
            Step::Install => &["plugin", "install", PLUGIN_ID],
            Step::Update { .. } => &["plugin", "update", PLUGIN_ID],
            Step::Enable => &["plugin", "enable", PLUGIN_ID],
            Step::Uninstall => &["plugin", "uninstall", PLUGIN_ID],
            Step::RemoveMarketplace => &["plugin", "marketplace", "remove", MARKETPLACE],
        };
        let mut args: Vec<OsString> = words.iter().map(OsString::from).collect();
        if let Step::AddMarketplace(path) = self {
            args.push(path.into());
        }
        args.push("--json".into());
        args
    }

    pub fn command_line(&self) -> String {
        let args = self.args();
        let shown = args[..args.len() - 1]
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned());
        std::iter::once(PROGRAM.to_string())
            .chain(shown)
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn edits_settings(&self) -> bool {
        !matches!(self, Step::Update { .. })
    }

    fn may_find_it_done(&self) -> bool {
        matches!(
            self,
            Step::Uninstall | Step::RemoveMarketplace | Step::Enable
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done(String),
    AlreadyDone(String),
}

#[derive(Deserialize)]
struct Reply {
    outcome: String,
    #[serde(default)]
    message: String,
    #[serde(default, rename = "failureCode")]
    failure_code: Option<String>,
}

const ALREADY_DONE: &[&str] = &["not_installed", "not_configured", "already_in_goal_state"];

pub fn outcome(step: &Step, stdout: &str) -> Result<Outcome> {
    let line = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    let reply: Reply = serde_json::from_str(line)
        .with_context(|| format!("{} answered {line:?}", step.command_line()))?;
    let already = reply
        .failure_code
        .as_deref()
        .is_some_and(|code| ALREADY_DONE.contains(&code));
    match reply.outcome.as_str() {
        "ok" => Ok(Outcome::Done(reply.message)),
        "failed" if already && step.may_find_it_done() => Ok(Outcome::AlreadyDone(reply.message)),
        _ => bail!("{} failed: {}", step.command_line(), reply.message),
    }
}

pub fn version_of(text: &str) -> Option<&str> {
    let version = text.split_whitespace().next()?;
    version
        .split('.')
        .all(|part| !part.is_empty())
        .then_some(version)
        .filter(|version| version.chars().next().is_some_and(|c| c.is_ascii_digit()))
}

#[derive(Debug, Clone)]
pub struct ClaudeCode {
    program: PathBuf,
}

impl Default for ClaudeCode {
    fn default() -> Self {
        Self::new(PROGRAM)
    }
}

impl ClaudeCode {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    pub fn version(&self) -> Result<Option<String>> {
        let output = match self.output(["--version"]) {
            Err(error) if is_not_found(&error) => return Ok(None),
            result => result?,
        };
        let text = String::from_utf8_lossy(&output.stdout);
        version_of(&text)
            .map(|version| Some(version.to_string()))
            .with_context(|| format!("claude --version answered {:?}", text.trim()))
    }

    pub fn marketplaces(&self) -> Result<Vec<Marketplace>> {
        self.list(&["plugin", "marketplace", "list", "--json"])
    }

    pub fn plugins(&self) -> Result<Vec<Plugin>> {
        self.list(&["plugin", "list", "--json"])
    }

    pub fn run(&self, step: &Step) -> Result<Outcome> {
        let output = self.output(step.args())?;
        outcome(step, &String::from_utf8_lossy(&output.stdout))
    }

    fn list<T: for<'de> Deserialize<'de>>(&self, args: &[&str]) -> Result<Vec<T>> {
        let output = self.output(args)?;
        let command = format!("{PROGRAM} {}", args[..args.len() - 1].join(" "));
        if !output.status.success() {
            bail!(
                "{command} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        serde_json::from_slice(&output.stdout)
            .with_context(|| format!("{command} answered something other than a list"))
    }

    fn output<I, S>(&self, args: I) -> std::io::Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        Command::new(&self.program)
            .args(args)
            .stdin(Stdio::null())
            .output()
    }
}

fn is_not_found(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

```

In `src/setup/mod.rs`, insert above the line `#[cfg(test)]`:

```rust
use std::path::Path;

use bundle::{MARKETPLACE, PLUGIN_ID};
use claude::{Marketplace, Plugin, Step, same_path};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    pub marketplaces: Vec<Marketplace>,
    pub plugins: Vec<Plugin>,
}

impl Found {
    pub fn marketplace(&self) -> Option<&Marketplace> {
        self.marketplaces.iter().find(|m| m.name == MARKETPLACE)
    }

    pub fn plugin(&self) -> Option<&Plugin> {
        self.plugins
            .iter()
            .find(|p| p.id == PLUGIN_ID && p.scope == "user")
    }
}

pub fn install_steps(found: &Found, dir: &Path, version: &str) -> Vec<Step> {
    let ours = found
        .marketplace()
        .and_then(|m| m.path.as_deref())
        .is_some_and(|path| same_path(Path::new(path), dir));
    let mut steps = Vec::new();
    if !ours {
        steps.push(Step::AddMarketplace(dir.to_path_buf()));
    }
    match found.plugin() {
        None => steps.push(Step::Install),
        Some(plugin) => {
            if plugin.version != version {
                steps.push(Step::Update {
                    from: plugin.version.clone(),
                });
            }
            if !plugin.enabled {
                steps.push(Step::Enable);
            }
        }
    }
    steps
}

pub fn uninstall_steps(found: &Found) -> Vec<Step> {
    let mut steps = Vec::new();
    if found.plugin().is_some() {
        steps.push(Step::Uninstall);
    }
    if found.marketplace().is_some() {
        steps.push(Step::RemoveMarketplace);
    }
    steps
}

```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --lib setup::`
Expected: PASS, every test under `setup::` passes (the 6 of Task 1, 7 in `setup::claude`, 6 in `setup::tests`, and the M6 `insights::setup` tests the filter also matches).

- [ ] **Step 5: Run every gate**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Then: `export PATH="/opt/homebrew/bin:$HOME/.local/bin:$PATH" && claude plugin validate mod && claude plugin validate .`
Expected: every command exits 0.

- [ ] **Step 6: Commit**

```bash
git add src/setup/claude.rs src/setup/mod.rs
git commit -m "feat(setup): drive the claude plugin CLI as JSON-reporting steps"
```

### Task 3: claude-plus install

**Files:**
- Create: `src/cli/setup.rs`, `tests/fake-claude/claude`, `tests/setup.rs`
- Modify: `src/cli/mod.rs`, `src/cli/daemon.rs`, `src/daemon/control.rs`, `tests/common/mod.rs`, `tests/cli.rs`

**Interfaces:**
- Consumes: `bundle::{write, VERSION, PLUGIN_ID}` and `Paths::marketplace_dir` (Task 1); `ClaudeCode`, `Step`, `Outcome`, `Found`, `install_steps`, `MIN_VERSION` (Task 2); `control::ensure` (M2).
- Produces: `cli::setup::{install(&Paths, yes: bool) -> Result<()>, confirm(&[String], &mut impl BufRead, &mut impl Write, typed: bool) -> Result<bool>, path_warning(exe: &Path, path_var: Option<OsString>) -> Option<String>}`; `cli::daemon::ensured(&Ensured) -> Vec<String>`; `control::is_older` public; `Command::Install { yes }`; in tests, `Env::claude_state()`, `Env::claude_calls()` and `fake_claude_path()`.

- [ ] **Step 1: Write the failing tests**

`src/cli/setup.rs` holds only its test module until Step 3, and `mod setup;` is declared now so that module compiles once the implementation is in.
The fake answers exactly the command lines claude-plus sends, in the JSON shape Claude Code 2.1.296 prints, keeps its marketplace path, the installed version and whether the mod is enabled in files under `FAKE_CLAUDE_STATE`, logs every call to `calls`, and fails the one command written in `fail`.
`tests/common/mod.rs` puts it first on `PATH` for every CLI test, ahead of `/usr/bin:/bin` only, so the real Claude Code is never reached.
The help test of `tests/cli.rs` gains the new command; its column does not move, because `install` is as long as `reindex`.

Create `tests/fake-claude/claude`, and make it executable (`chmod +x tests/fake-claude/claude`):

```sh
#!/bin/sh
set -eu
state="${FAKE_CLAUDE_STATE:?FAKE_CLAUDE_STATE is not set}"
mkdir -p "$state"
printf '%s\n' "$*" >> "$state/calls"

ok() {
  printf '{"command":"%s","outcome":"ok","message":"%s"}\n' "$1" "$2"
}

failed() {
  printf '{"command":"%s","outcome":"failed","message":"%s","failureCode":"%s"}\n' "$1" "$2" "$3"
  exit 1
}

manifest_version() {
  sed -n 's/^ *"version": *"\([^"]*\)".*/\1/p' "$(cat "$state/marketplace")/mod/.claude-plugin/plugin.json"
}

if [ -f "$state/fail" ] && [ "$(cat "$state/fail")" = "$*" ]; then
  failed fake "the fake was told to fail" fake_failure
fi

case "$*" in
  --version)
    printf '%s (Claude Code)\n' "${FAKE_CLAUDE_VERSION:-2.1.296}"
    ;;
  "plugin marketplace list --json")
    if [ -f "$state/marketplace" ]; then
      printf '[{"name":"claude-plus","source":"directory","path":"%s"}]\n' "$(cat "$state/marketplace")"
    else
      echo '[]'
    fi
    ;;
  "plugin marketplace add "*" --json")
    path="$4"
    [ -d "$path" ] || failed marketplace-add "Path does not exist: $path" invalid_source
    printf '%s' "$path" > "$state/marketplace"
    ok marketplace-add "Successfully added marketplace: claude-plus"
    ;;
  "plugin marketplace remove claude-plus --json")
    [ -f "$state/marketplace" ] || failed marketplace-remove "Marketplace 'claude-plus' not found" not_configured
    rm "$state/marketplace"
    ok marketplace-remove "Successfully removed marketplace: claude-plus"
    ;;
  "plugin list --json")
    if [ -f "$state/plugin" ]; then
      printf '[{"id":"plus@claude-plus","version":"%s","scope":"user","enabled":%s,"readFromFolder":"%s/mod","folderVersion":"%s"}]\n' \
        "$(cat "$state/plugin")" "$(cat "$state/enabled" 2>/dev/null || echo true)" \
        "$(cat "$state/marketplace")" "$(manifest_version)"
    else
      echo '[]'
    fi
    ;;
  "plugin install plus@claude-plus --json")
    [ -f "$state/marketplace" ] || failed install "Plugin \\\"plus\\\" not found" not_found
    manifest_version > "$state/plugin"
    ok install "Successfully installed plugin: plus@claude-plus (scope: user)"
    ;;
  "plugin update plus@claude-plus --json")
    old="$(cat "$state/plugin")"
    manifest_version > "$state/plugin"
    ok update "Plugin \\\"plus\\\" updated from $old to $(cat "$state/plugin") for scope user."
    ;;
  "plugin enable plus@claude-plus --json")
    echo true > "$state/enabled"
    ok enable "Enabled plus@claude-plus"
    ;;
  "plugin uninstall plus@claude-plus --json")
    [ -f "$state/plugin" ] || failed uninstall "Plugin \\\"plus@claude-plus\\\" not found in installed plugins" not_installed
    rm -f "$state/plugin" "$state/enabled"
    ok uninstall "Successfully uninstalled plugin: plus (scope: user)"
    ;;
  *)
    echo "fake claude does not know: $*" >&2
    exit 2
    ;;
esac
```

In `tests/common/mod.rs`, insert above the line `pub const VARS: &[(&str, &str)] = &[`:

```rust
impl Env {
    pub fn claude_state(&self) -> PathBuf {
        self._temp.path().join("fake-claude")
    }

    pub fn claude_calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.claude_state().join("calls"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }
}

pub fn fake_claude_path() -> std::ffi::OsString {
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake-claude");
    std::env::join_paths([fake.as_path(), Path::new("/usr/bin"), Path::new("/bin")]).unwrap()
}

```

In `tests/common/mod.rs`, replace:

```rust
    let mut command = Command::cargo_bin("claude-plus").unwrap();
    command
        .args(args)
        .env("CLAUDE_CONFIG_DIR", &env.claude)
        .env("CLAUDE_PLUS_HOME", &env.plus)
```

with:

```rust
    let mut command = Command::cargo_bin("claude-plus").unwrap();
    command
        .args(args)
        .env("CLAUDE_CONFIG_DIR", &env.claude)
        .env("CLAUDE_PLUS_HOME", &env.plus)
        .env("PATH", fake_claude_path())
        .env("FAKE_CLAUDE_STATE", env.claude_state())
```

In `tests/common/mod.rs`, replace:

```rust
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_claude-plus"));
    command
        .args(args)
        .env("CLAUDE_CONFIG_DIR", &env.claude)
        .env("CLAUDE_PLUS_HOME", &env.plus)
```

with:

```rust
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_claude-plus"));
    command
        .args(args)
        .env("CLAUDE_CONFIG_DIR", &env.claude)
        .env("CLAUDE_PLUS_HOME", &env.plus)
        .env("PATH", fake_claude_path())
        .env("FAKE_CLAUDE_STATE", env.claude_state())
```

Create `tests/setup.rs`:

```rust
mod common;

use claude_plus::api::client::Client;
use common::{Env, cmd, env};
use predicates::prelude::*;

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Daemon<'a>(&'a Env);

impl Drop for Daemon<'_> {
    fn drop(&mut self) {
        let _ = cmd(self.0, &["daemon", "stop"]).output();
    }
}

fn marketplace(env: &Env) -> std::path::PathBuf {
    env.plus.join("marketplace")
}

fn listing_calls() -> Vec<String> {
    [
        "--version",
        "plugin marketplace list --json",
        "plugin list --json",
    ]
    .map(String::from)
    .to_vec()
}

fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

#[test]
fn install_adds_the_marketplace_installs_the_mod_and_starts_the_daemon() {
    let env = env();
    let _daemon = Daemon(&env);
    let assert = cmd(&env, &["install", "--yes"]).assert().success();
    let out = stdout(&assert);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[..2],
        [
            format!(
                "✻ Added the claude-plus marketplace · {}",
                marketplace(&env).display()
            )
            .as_str(),
            format!("✻ Installed the plus mod v{VERSION}").as_str(),
        ]
    );
    assert!(
        lines[2].starts_with("✻ Started the claude-plus daemon · pid "),
        "{out}"
    );
    assert!(
        lines[3].starts_with("claude-plus is not on PATH, where the pane looks for it"),
        "{out}"
    );
    assert_eq!(
        lines[4],
        "✻ Start a new Claude Code session to see the pane: it docks beside the conversation from 144 columns, and /plus opens it at any width"
    );
    let mut expected = listing_calls();
    expected.push(format!(
        "plugin marketplace add {} --json",
        marketplace(&env).display()
    ));
    expected.push("plugin install plus@claude-plus --json".into());
    assert_eq!(env.claude_calls(), expected);
    assert!(marketplace(&env).join("mod/hooks/register.tsx").is_file());
    assert!(!marketplace(&env).join("mod/tests").exists());
    assert!(Client::new(&env.plus.join("daemon.sock")).health().is_ok());
}

#[test]
fn install_asks_before_claude_code_records_anything_in_its_settings() {
    let env = env();
    let _daemon = Daemon(&env);
    let settings = env.claude.join("settings.json");
    cmd(&env, &["install"])
        .write_stdin("n\n")
        .assert()
        .failure()
        .stdout(predicate::str::contains(format!(
            "claude-plus install will:\n  run claude plugin marketplace add {}\n  run claude plugin install plus@claude-plus\n  which Claude Code records in {}\nContinue? [y/N] ",
            marketplace(&env).display(),
            settings.display()
        ) + "n\n"))
        .stderr("claude-plus: install cancelled, nothing was changed; claude-plus install --yes agrees without asking\n");
    assert_eq!(env.claude_calls(), listing_calls());
    assert!(!env.plus.exists());
    cmd(&env, &["install"])
        .write_stdin("y\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "✻ Installed the plus mod v{VERSION}"
        )));
}

#[test]
fn a_marketplace_of_that_name_elsewhere_is_pointed_here_after_asking() {
    let env = env();
    let _daemon = Daemon(&env);
    std::fs::create_dir_all(env.claude_state()).unwrap();
    std::fs::write(env.claude_state().join("marketplace"), "/elsewhere").unwrap();
    std::fs::write(env.claude_state().join("plugin"), VERSION).unwrap();
    cmd(&env, &["install"])
        .write_stdin("y\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "  run claude plugin marketplace add {0}\n  which Claude Code records in",
            marketplace(&env).display()
        )))
        .stdout(predicate::str::contains(format!(
            "✻ Pointed the claude-plus marketplace at {} (was /elsewhere)\n✻ The plus mod v{VERSION} is installed\n",
            marketplace(&env).display()
        )));
}

#[test]
fn installing_again_asks_nothing_and_changes_nothing() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["install", "--yes"]).assert().success();
    let before = env.claude_calls().len();
    cmd(&env, &["install"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(format!(
            "✻ The plus mod v{VERSION} is installed\n✻ claude-plus daemon is running · pid "
        )));
    assert_eq!(env.claude_calls()[before..], listing_calls());
}

#[test]
fn an_older_mod_is_updated_without_asking() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["install", "--yes"]).assert().success();
    std::fs::write(env.claude_state().join("plugin"), "0.0.9").unwrap();
    std::fs::write(env.claude_state().join("enabled"), "true").unwrap();
    cmd(&env, &["install"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(format!(
            "✻ Updated the plus mod from v0.0.9 to v{VERSION}\n"
        )));
    assert_eq!(
        env.claude_calls().last().unwrap(),
        "plugin update plus@claude-plus --json"
    );
}

#[test]
fn install_needs_claude_code_of_the_mod_api_version() {
    let env = env();
    cmd(&env, &["install", "--yes"])
        .env("PATH", "/usr/bin:/bin")
        .assert()
        .failure()
        .stderr("claude-plus: Claude Code was not found on PATH; install it first, then run claude-plus install again\n");
    cmd(&env, &["install", "--yes"])
        .env("FAKE_CLAUDE_VERSION", "2.1.200")
        .assert()
        .failure()
        .stderr("claude-plus: Claude Code 2.1.200 is older than 2.1.296, which the plus mod needs; run claude update, then claude-plus install again\n");
    assert!(!env.plus.exists());
}

#[test]
fn a_failing_claude_plugin_command_stops_the_install_with_its_reason() {
    let env = env();
    std::fs::create_dir_all(env.claude_state()).unwrap();
    std::fs::write(
        env.claude_state().join("fail"),
        "plugin install plus@claude-plus --json",
    )
    .unwrap();
    cmd(&env, &["install", "--yes"]).assert().failure().stderr(
        "claude-plus: claude plugin install plus@claude-plus failed: the fake was told to fail\n",
    );
    assert!(!env.plus.join("daemon.sock").exists());
}
```

In `tests/cli.rs`, replace:

```rust
        "  import   Read new Claude Code transcripts into the local store\n",
        "  stats    Show daily cost per model, importing first\n",
        "  export   Print stored usage per day, project and model, importing first\n",
        "  doctor   Check the setup and compare totals with Claude Code, read-only\n",
        "  reindex  Rebuild every derived table from the stored records\n",
        "  daemon   Run the daemon that keeps the store live, in the foreground\n",
```

with:

```rust
        "  install  Add the claude-plus pane to Claude Code and start the daemon\n",
        "  import   Read new Claude Code transcripts into the local store\n",
        "  stats    Show daily cost per model, importing first\n",
        "  export   Print stored usage per day, project and model, importing first\n",
        "  doctor   Check the setup and compare totals with Claude Code, read-only\n",
        "  reindex  Rebuild every derived table from the stored records\n",
        "  daemon   Run the daemon that keeps the store live, in the foreground\n",
```

In `src/cli/mod.rs`, replace:

```rust
mod reindex;
```

with:

```rust
mod reindex;
mod setup;
```

Create `src/cli/setup.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_yes_agrees() {
        for (answer, agreed) in [
            ("y\n", true),
            ("YES\n", true),
            ("n\n", false),
            ("\n", false),
            ("", false),
            ("maybe\n", false),
        ] {
            let mut output = Vec::new();
            let lines = ["claude-plus install will:".to_string()];
            assert_eq!(
                confirm(&lines, &mut answer.as_bytes(), &mut output, false).unwrap(),
                agreed,
                "{answer:?}"
            );
            assert_eq!(
                String::from_utf8(output).unwrap(),
                format!(
                    "claude-plus install will:\nContinue? [y/N] {}\n",
                    answer.trim()
                )
            );
        }
    }

    #[test]
    fn the_pane_finds_this_binary_on_path_or_says_how() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        let other = temp.path().join("other");
        for dir in [&bin, &other] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(BINARY), "").unwrap();
            std::fs::set_permissions(dir.join(BINARY), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        let exe = bin.join(BINARY);
        let path = |dirs: &[&Path]| Some(std::env::join_paths(dirs).unwrap());
        assert_eq!(path_warning(&exe, path(&[&bin])), None);
        let shadowed = path_warning(&exe, path(&[&other, &bin])).unwrap();
        assert!(
            shadowed.starts_with("The pane starts claude-plus from PATH, which finds"),
            "{shadowed}"
        );
        let missing = path_warning(&exe, path(&[temp.path()])).unwrap();
        assert!(
            missing.starts_with("claude-plus is not on PATH, where the pane looks for it: add"),
            "{missing}"
        );
        assert!(
            missing.ends_with("with /plugin configure plus@claude-plus"),
            "{missing}"
        );
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --test setup`
Expected: FAIL, `cargo test --test setup` reports 7 failures, each with `error: unrecognized subcommand 'install'`.

- [ ] **Step 3: Implement**

`install` asks before it writes anything, then writes the bundle, runs the steps in order and stops at the first failure, then ensures the daemon.
`ensure` of `src/cli/daemon.rs` now prints the lines of a new `ensured`, which `install` prints too.

In `src/daemon/control.rs`, replace:

```rust
fn is_older(version: &str, than: &str) -> bool {
```

with:

```rust
pub fn is_older(version: &str, than: &str) -> bool {
```

In `src/cli/daemon.rs`, replace:

```rust
pub fn ensure(paths: &Paths) -> Result<()> {
    match control::ensure(paths)? {
        Ensured::Running(health) => println!("{}", running(&health)),
        Ensured::KeptNewer(health) => {
            println!("{}", running(&health));
            println!(
                "  It runs v{}, newer than this claude-plus v{}",
                health.version,
                control::VERSION
            );
        }
        Ensured::Started(health) => {
            println!("✻ Started the claude-plus daemon · pid {}", health.pid)
        }
        Ensured::Restarted { previous, health } => {
            println!("{}", restarted(&previous, &health))
        }
    }
    Ok(())
}
```

with:

```rust
pub fn ensure(paths: &Paths) -> Result<()> {
    for line in ensured(&control::ensure(paths)?) {
        println!("{line}");
    }
    Ok(())
}

pub fn ensured(ensured: &Ensured) -> Vec<String> {
    match ensured {
        Ensured::Running(health) => vec![running(health)],
        Ensured::KeptNewer(health) => vec![
            running(health),
            format!(
                "  It runs v{}, newer than this claude-plus v{}",
                health.version,
                control::VERSION
            ),
        ],
        Ensured::Started(health) => {
            vec![format!(
                "✻ Started the claude-plus daemon · pid {}",
                health.pid
            )]
        }
        Ensured::Restarted { previous, health } => vec![restarted(previous, health)],
    }
}
```

In `src/cli/setup.rs`, insert above the line `#[cfg(test)]`:

```rust
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::daemon::ensured;
use crate::daemon::control::{self, is_older};
use crate::paths::Paths;
use crate::setup::bundle::{self, PLUGIN_ID};
use crate::setup::claude::{self, ClaudeCode, Outcome, Step};
use crate::setup::{Found, install_steps};

const BINARY: &str = "claude-plus";

pub fn install(paths: &Paths, yes: bool) -> Result<()> {
    let claude = ClaudeCode::default();
    let Some(version) = claude.version()? else {
        bail!(
            "Claude Code was not found on PATH; install it first, then run claude-plus install again"
        );
    };
    if is_older(&version, claude::MIN_VERSION) {
        bail!(
            "Claude Code {version} is older than {}, which the plus mod needs; run claude update, then claude-plus install again",
            claude::MIN_VERSION
        );
    }
    let dir = paths.marketplace_dir();
    let found = found(&claude)?;
    let steps = install_steps(&found, &dir, bundle::VERSION);
    confirm_settings(paths, "install", &steps, &[], yes)?;
    paths.ensure_data_dir()?;
    bundle::write(&dir)?;
    for step in &steps {
        let outcome = claude.run(step)?;
        println!("{}", installed(step, &found, &outcome));
    }
    if steps
        .iter()
        .all(|step| matches!(step, Step::AddMarketplace(_)))
    {
        println!("✻ The plus mod v{} is installed", bundle::VERSION);
    }
    for line in ensured(&control::ensure(paths)?) {
        println!("{line}");
    }
    if let Some(warning) = path_warning(&std::env::current_exe()?, std::env::var_os("PATH")) {
        println!("{warning}");
    }
    println!(
        "✻ Start a new Claude Code session to see the pane: it docks beside the conversation from 144 columns, and /plus opens it at any width"
    );
    Ok(())
}

fn found(claude: &ClaudeCode) -> Result<Found> {
    Ok(Found {
        marketplaces: claude.marketplaces()?,
        plugins: claude.plugins()?,
    })
}

fn confirm_settings(
    paths: &Paths,
    command: &str,
    steps: &[Step],
    also: &[String],
    yes: bool,
) -> Result<()> {
    let edits = steps.iter().any(Step::edits_settings);
    if yes || (!edits && also.is_empty()) {
        return Ok(());
    }
    let mut lines = vec![format!("claude-plus {command} will:")];
    for step in steps {
        lines.push(format!("  run {}", step.command_line()));
    }
    if edits {
        lines.push(format!(
            "  which Claude Code records in {}",
            paths.settings_file().display()
        ));
    }
    lines.extend(also.iter().map(|line| format!("  {line}")));
    let stdin = std::io::stdin();
    let typed = stdin.is_terminal();
    if !confirm(&lines, &mut stdin.lock(), &mut std::io::stdout(), typed)? {
        bail!(
            "{command} cancelled, nothing was changed; claude-plus {command} --yes agrees without asking"
        );
    }
    Ok(())
}

pub fn confirm(
    lines: &[String],
    input: &mut impl BufRead,
    output: &mut impl Write,
    typed: bool,
) -> Result<bool> {
    for line in lines {
        writeln!(output, "{line}")?;
    }
    write!(output, "Continue? [y/N] ")?;
    output.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    if !typed {
        writeln!(output, "{}", answer.trim())?;
    }
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

fn installed(step: &Step, found: &Found, outcome: &Outcome) -> String {
    match step {
        Step::AddMarketplace(dir) => match found.marketplace() {
            Some(previous) => format!(
                "✻ Pointed the claude-plus marketplace at {} (was {})",
                dir.display(),
                previous.location()
            ),
            None => format!("✻ Added the claude-plus marketplace · {}", dir.display()),
        },
        Step::Install => format!("✻ Installed the plus mod v{}", bundle::VERSION),
        Step::Update { from } => {
            format!(
                "✻ Updated the plus mod from v{from} to v{}",
                bundle::VERSION
            )
        }
        Step::Enable => "✻ Enabled the plus mod".into(),
        Step::Uninstall | Step::RemoveMarketplace => said(outcome),
    }
}

fn said(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Done(message) | Outcome::AlreadyDone(message) => format!("  {message}"),
    }
}

pub fn path_warning(exe: &Path, path_var: Option<std::ffi::OsString>) -> Option<String> {
    let exe = exe.canonicalize().unwrap_or(exe.to_path_buf());
    let found = path_var
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(BINARY))
        .find(|candidate| is_executable(candidate));
    match found {
        Some(found) if found.canonicalize().ok().as_ref() == Some(&exe) => None,
        Some(found) => Some(format!(
            "The pane starts {BINARY} from PATH, which finds {} and not this binary ({}); set binaryPath with /plugin configure {PLUGIN_ID} if that is not the one you want",
            found.display(),
            exe.display()
        )),
        None => Some(format!(
            "{BINARY} is not on PATH, where the pane looks for it: add {} to PATH, or set binaryPath to {} with /plugin configure {PLUGIN_ID}",
            exe.parent()
                .map(PathBuf::from)
                .unwrap_or_default()
                .display(),
            exe.display()
        )),
    }
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

```

In `src/cli/mod.rs`, replace:

```rust
enum Command {
```

with:

```rust
enum Command {
    #[command(about = "Add the claude-plus pane to Claude Code and start the daemon")]
    Install {
        #[arg(short, long, help = "Agree to the Claude Code changes without asking")]
        yes: bool,
    },
```

In `src/cli/mod.rs`, replace:

```rust
        Command::Reindex => reindex::run(&paths, &prices),
```

with:

```rust
        Command::Reindex => reindex::run(&paths, &prices),
        Command::Install { yes } => setup::install(&paths, yes),
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --test setup && cargo test --lib cli::setup && cargo test --test cli help_describes_every_command`
Expected: PASS, 7 tests in `tests/setup.rs`, 2 in `cli::setup` and the help test.

- [ ] **Step 5: Check that the fake keeps out of the real Claude Code**

Run: `ls -la tests/fake-claude/claude && grep -c FAKE_CLAUDE_STATE tests/common/mod.rs`
Expected: the fake is executable (`-rwxr-xr-x`), and the count is `2` (`cmd` and `process`).

- [ ] **Step 6: Run every gate**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Then: `export PATH="/opt/homebrew/bin:$HOME/.local/bin:$PATH" && claude plugin validate mod && claude plugin validate .`
Expected: every command exits 0.

- [ ] **Step 7: Commit**

```bash
git add src/cli/setup.rs src/cli/mod.rs src/cli/daemon.rs src/daemon/control.rs tests/fake-claude/claude tests/common/mod.rs tests/setup.rs tests/cli.rs
git commit -m "feat(cli): add claude-plus install"
```

### Task 4: claude-plus uninstall [--purge]

**Files:**
- Modify: `src/cli/setup.rs`, `src/cli/mod.rs`, `src/cli/daemon.rs`, `tests/setup.rs`, `tests/cli.rs`

**Interfaces:**
- Consumes: `uninstall_steps` (Task 2); `bundle::remove` (Task 1); `control::stop` (M2); `confirm_settings`, `found`, `said` (Task 3).
- Produces: `cli::setup::uninstall(&Paths, purge: bool, yes: bool) -> Result<()>`; `cli::daemon::stopped` public; `Command::Uninstall { purge, yes }`.

- [ ] **Step 1: Write the failing tests**

`uninstall` is two characters longer than `install` and `reindex`, the longest names so far, so every line of the help test moves by two columns.

Append to `tests/setup.rs`:

```rust

#[test]
fn uninstall_removes_the_mod_and_its_marketplace_and_stops_the_daemon() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["install", "--yes"]).assert().success();
    let before = env.claude_calls().len();
    cmd(&env, &["uninstall", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::is_match(
            "^✻ Uninstalled the plus mod\n✻ Removed the claude-plus marketplace\n✻ Stopped the claude-plus daemon · pid [0-9]+\n✻ claude-plus is uninstalled · close open Claude Code sessions, or their pane starts the daemon again\n$",
        ).unwrap());
    assert_eq!(
        env.claude_calls()[before..],
        [
            "--version",
            "plugin marketplace list --json",
            "plugin list --json",
            "plugin uninstall plus@claude-plus --json",
            "plugin marketplace remove claude-plus --json",
        ]
    );
    assert!(!marketplace(&env).exists());
    assert!(env.plus.join("data.db").exists());
}

#[test]
fn uninstall_asks_first_and_purge_deletes_the_data_dir() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["install", "--yes"]).assert().success();
    cmd(&env, &["uninstall", "--purge"])
        .write_stdin("")
        .assert()
        .failure()
        .stdout(predicate::str::contains(format!(
            "  delete {} with the history claude-plus keeps\nContinue? [y/N] ",
            env.plus.display()
        )));
    assert!(env.plus.join("data.db").exists());
    cmd(&env, &["uninstall", "--purge"])
        .write_stdin("y\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "✻ Deleted {}\n",
            env.plus.display()
        )));
    assert!(!env.plus.exists());
}

#[test]
fn purge_refuses_a_data_dir_that_holds_the_claude_dir_before_changing_anything() {
    let env = env();
    let parent = env.claude.parent().unwrap();
    cmd(&env, &["uninstall", "--purge", "--yes"])
        .env("CLAUDE_PLUS_HOME", parent)
        .assert()
        .failure()
        .stderr(format!(
            "claude-plus: refusing to purge {}: it holds {} or your home directory\n",
            parent.display(),
            env.claude.display()
        ));
    assert_eq!(env.claude_calls(), Vec::<String>::new());
    assert!(env.claude.join("projects").exists());
}

#[test]
fn uninstall_without_claude_code_still_stops_the_daemon() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["daemon", "--ensure"]).assert().success();
    cmd(&env, &["uninstall"])
        .env("PATH", "/usr/bin:/bin")
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "Claude Code was not found on PATH, so the plus mod may still be installed: run claude plugin uninstall plus@claude-plus where it is\n✻ Stopped the claude-plus daemon · pid ",
        ));
}

#[test]
fn uninstalling_twice_finds_nothing_left() {
    let env = env();
    cmd(&env, &["uninstall"])
        .assert()
        .success()
        .stdout("No claude-plus daemon is running\n✻ claude-plus is uninstalled · close open Claude Code sessions, or their pane starts the daemon again\n");
}
```

In `tests/cli.rs`, replace:

```rust
        "  install  Add the claude-plus pane to Claude Code and start the daemon\n",
        "  import   Read new Claude Code transcripts into the local store\n",
        "  stats    Show daily cost per model, importing first\n",
        "  export   Print stored usage per day, project and model, importing first\n",
        "  doctor   Check the setup and compare totals with Claude Code, read-only\n",
        "  reindex  Rebuild every derived table from the stored records\n",
        "  daemon   Run the daemon that keeps the store live, in the foreground\n",
```

with:

```rust
        "  install    Add the claude-plus pane to Claude Code and start the daemon\n",
        "  uninstall  Remove the claude-plus pane from Claude Code and stop the daemon\n",
        "  import     Read new Claude Code transcripts into the local store\n",
        "  stats      Show daily cost per model, importing first\n",
        "  export     Print stored usage per day, project and model, importing first\n",
        "  doctor     Check the setup and compare totals with Claude Code, read-only\n",
        "  reindex    Rebuild every derived table from the stored records\n",
        "  daemon     Run the daemon that keeps the store live, in the foreground\n",
```

In `src/cli/setup.rs`, insert above these lines:

```rust
    #[test]
    fn the_pane_finds
```

this:

```rust
    #[test]
    fn purging_never_reaches_home_or_the_claude_dir() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let refused = |plus: PathBuf| purge_refusal(&Paths::resolve(home, None, Some(plus)));
        assert_eq!(refused(home.join(".claude-plus")), None);
        assert!(refused(home.to_path_buf()).is_some());
        assert!(refused(PathBuf::from("/")).is_some());
        assert!(refused(home.join(".claude")).is_some());
        assert!(
            purge_refusal(&Paths::resolve(
                home,
                Some(home.join("a/claude")),
                Some(home.join("a"))
            ))
            .is_some()
        );
    }

```

- [ ] **Step 2: Run them to verify they fail**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --test setup`
Expected: FAIL, 5 of the 12 tests fail, each because the binary answers `error: unrecognized subcommand 'uninstall'`; the 7 install tests still pass.

- [ ] **Step 3: Implement**

The purge guard runs before Claude Code is even asked for its lists.

In `src/cli/setup.rs`, replace:

```rust
use anyhow::{Result, bail};

use super::daemon::ensured;
```

with:

```rust
use anyhow::{Context, Result, bail};

use super::daemon::{ensured, stopped};
```

In `src/cli/setup.rs`, replace:

```rust
use crate::setup::{Found, install_steps};
```

with:

```rust
use crate::setup::{Found, install_steps, uninstall_steps};
```

In `src/cli/setup.rs`, insert above the line `fn found(claude: &ClaudeCode)`:

```rust
pub fn uninstall(paths: &Paths, purge: bool, yes: bool) -> Result<()> {
    if purge && let Some(refusal) = purge_refusal(paths) {
        bail!(refusal);
    }
    let claude = ClaudeCode::default();
    let found = match claude.version()? {
        Some(_) => Some(found(&claude)?),
        None => None,
    };
    let steps = found.as_ref().map(uninstall_steps).unwrap_or_default();
    let deleted = if purge {
        vec![format!(
            "delete {} with the history claude-plus keeps",
            paths.data_dir.display()
        )]
    } else {
        Vec::new()
    };
    confirm_settings(paths, "uninstall", &steps, &deleted, yes)?;
    for step in &steps {
        let outcome = claude.run(step)?;
        println!("{}", uninstalled(step, &outcome));
    }
    if found.is_none() {
        println!(
            "Claude Code was not found on PATH, so the plus mod may still be installed: run claude plugin uninstall {PLUGIN_ID} where it is"
        );
    }
    println!("{}", stopped(control::stop(paths)?));
    bundle::remove(&paths.marketplace_dir())?;
    if purge {
        remove_dir(&paths.data_dir)?;
        println!("✻ Deleted {}", paths.data_dir.display());
    }
    println!(
        "✻ claude-plus is uninstalled · close open Claude Code sessions, or their pane starts the daemon again"
    );
    Ok(())
}

```

In `src/cli/setup.rs`, insert above the line `fn said(outcome: &Outcome)`:

```rust
fn uninstalled(step: &Step, outcome: &Outcome) -> String {
    match (step, outcome) {
        (Step::Uninstall, Outcome::Done(_)) => "✻ Uninstalled the plus mod".into(),
        (Step::RemoveMarketplace, Outcome::Done(_)) => {
            "✻ Removed the claude-plus marketplace".into()
        }
        _ => said(outcome),
    }
}

```

In `src/cli/setup.rs`, insert above the line `pub fn path_warning(`:

```rust
fn remove_dir(dir: &Path) -> Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("cannot delete {}", dir.display()))
        }
        _ => Ok(()),
    }
}

fn purge_refusal(paths: &Paths) -> Option<String> {
    let data = paths
        .data_dir
        .canonicalize()
        .unwrap_or(paths.data_dir.clone());
    let guarded = [
        paths.home.as_path(),
        paths.claude_dir.as_path(),
        Path::new("/"),
    ];
    guarded
        .iter()
        .map(|dir| dir.canonicalize().unwrap_or(dir.to_path_buf()))
        .any(|dir| dir.starts_with(&data))
        .then(|| {
            format!(
                "refusing to purge {}: it holds {} or your home directory",
                paths.data_dir.display(),
                paths.claude_dir.display()
            )
        })
}

```

In `src/cli/daemon.rs`, replace:

```rust
fn stopped(stopped: Stopped) -> String {
```

with:

```rust
pub fn stopped(stopped: Stopped) -> String {
```

In `src/cli/mod.rs`, replace:

```rust
    #[command(about = "Add the claude-plus pane to Claude Code and start the daemon")]
    Install {
        #[arg(short, long, help = "Agree to the Claude Code changes without asking")]
        yes: bool,
    },
```

with:

```rust
    #[command(about = "Add the claude-plus pane to Claude Code and start the daemon")]
    Install {
        #[arg(short, long, help = "Agree to the Claude Code changes without asking")]
        yes: bool,
    },
    #[command(about = "Remove the claude-plus pane from Claude Code and stop the daemon")]
    Uninstall {
        #[arg(
            long,
            help = "Also delete the claude-plus data directory and the history it keeps"
        )]
        purge: bool,
        #[arg(short, long, help = "Agree to the changes without asking")]
        yes: bool,
    },
```

In `src/cli/mod.rs`, replace:

```rust
        Command::Install { yes } => setup::install(&paths, yes),
```

with:

```rust
        Command::Install { yes } => setup::install(&paths, yes),
        Command::Uninstall { purge, yes } => setup::uninstall(&paths, purge, yes),
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --test setup && cargo test --lib cli::setup && cargo test --test cli help_describes_every_command`
Expected: PASS, 12 tests in `tests/setup.rs`, 3 in `cli::setup` and the help test.

- [ ] **Step 5: Run every gate**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Then: `export PATH="/opt/homebrew/bin:$HOME/.local/bin:$PATH" && claude plugin validate mod && claude plugin validate .`
Expected: every command exits 0.

- [ ] **Step 6: Commit**

```bash
git add src/cli/setup.rs src/cli/mod.rs src/cli/daemon.rs tests/setup.rs tests/cli.rs
git commit -m "feat(cli): add claude-plus uninstall with --purge"
```

### Task 5: daemon --ensure brings an older mod folder up to the binary

**Files:**
- Modify: `src/setup/bundle.rs`, `src/cli/daemon.rs`, `tests/setup.rs`

**Interfaces:**
- Consumes: `control::is_older` (Task 3); `bundle::{version_in, lock, replace}` (Task 1).
- Produces: `bundle::refresh(dir: &Path) -> Result<Option<String>>`, the version it replaced, or `None` when the folder is missing, unreadable, current or newer.

- [ ] **Step 1: Write the failing tests**

The thread test runs eight refreshes at once on one folder, as eight sessions starting together would.

In `src/setup/bundle.rs`, insert above these lines:

```rust
    #[test]
    fn removing_the_folder
```

this:

```rust
    fn with_version(dir: &Path, version: &str) {
        let manifest = dir.join(MANIFEST);
        let text = std::fs::read_to_string(&manifest).unwrap();
        let old = text.replace(
            &format!("\"version\": \"{VERSION}\""),
            &format!("\"version\": \"{version}\""),
        );
        std::fs::write(&manifest, old).unwrap();
    }

    #[test]
    fn an_older_mod_folder_is_refreshed_and_a_newer_or_missing_one_left_alone() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("marketplace");
        assert_eq!(refresh(&dir).unwrap(), None);
        assert!(!dir.exists());
        write(&dir).unwrap();
        with_version(&dir, "0.0.1");
        std::fs::write(dir.join("mod/hooks/stale.ts"), "old").unwrap();
        assert_eq!(refresh(&dir).unwrap().as_deref(), Some("0.0.1"));
        assert_eq!(version_in(&dir).as_deref(), Some(VERSION));
        assert!(!dir.join("mod/hooks/stale.ts").exists());
        assert_eq!(refresh(&dir).unwrap(), None);
        with_version(&dir, "999.0.0");
        assert_eq!(refresh(&dir).unwrap(), None);
        assert_eq!(version_in(&dir).as_deref(), Some("999.0.0"));
    }

    #[test]
    fn sessions_starting_together_refresh_the_folder_once() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("marketplace");
        write(&dir).unwrap();
        with_version(&dir, "0.0.1");
        let refreshed: Vec<Option<String>> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| refresh(&dir).unwrap()))
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect()
        });
        assert_eq!(
            refreshed.iter().filter(|r| r.is_some()).count(),
            1,
            "{refreshed:?}"
        );
        assert_eq!(version_in(&dir).as_deref(), Some(VERSION));
        assert!(dir.join("mod/hooks/register.tsx").is_file());
    }

```

Append to `tests/setup.rs`:

```rust

#[test]
fn daemon_ensure_brings_an_older_installed_mod_up_to_this_binary() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["install", "--yes"]).assert().success();
    let manifest = marketplace(&env).join("mod/.claude-plugin/plugin.json");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace(
            &format!("\"version\": \"{VERSION}\""),
            "\"version\": \"0.0.1\"",
        ),
    )
    .unwrap();
    let before = env.claude_calls();
    cmd(&env, &["daemon", "--ensure"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(format!(
            "✻ Updated the plus mod from v0.0.1 to v{VERSION} · new Claude Code sessions load it\n✻ claude-plus daemon is running · pid "
        )));
    assert!(
        std::fs::read_to_string(&manifest)
            .unwrap()
            .contains(&format!("\"version\": \"{VERSION}\""))
    );
    assert_eq!(env.claude_calls(), before);
    cmd(&env, &["daemon", "--ensure"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "✻ claude-plus daemon is running · pid ",
        ));
}

#[test]
fn daemon_ensure_without_an_installed_mod_writes_no_folder() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["daemon", "--ensure"]).assert().success();
    assert!(!marketplace(&env).exists());
    assert!(!env.plus.join("marketplace.lock").exists());
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --lib setup::bundle`
Expected: FAIL, `cargo test --lib setup::bundle` does not compile: `cannot find function refresh in this scope`.

- [ ] **Step 3: Implement**

`refresh` looks without the lock first, so the common case (nothing to do) takes no lock and creates no file, then looks again under the lock before it rewrites.

In `src/setup/bundle.rs`, replace:

```rust
use rustix::fs::{FlockOperation, flock};
```

with:

```rust
use rustix::fs::{FlockOperation, flock};

use crate::daemon::control::is_older;
```

In `src/setup/bundle.rs`, insert above the line `fn lock(dir: &Path)`:

```rust
pub fn refresh(dir: &Path) -> Result<Option<String>> {
    let outdated = |dir: &Path| version_in(dir).filter(|installed| is_older(installed, VERSION));
    if outdated(dir).is_none() {
        return Ok(None);
    }
    let _lock = lock(dir)?;
    let Some(installed) = outdated(dir) else {
        return Ok(None);
    };
    replace(dir)?;
    Ok(Some(installed))
}

```

In `src/cli/daemon.rs`, replace:

```rust
use crate::pricing::PriceTable;
```

with:

```rust
use crate::pricing::PriceTable;
use crate::setup::bundle;
```

In `src/cli/daemon.rs`, replace:

```rust
pub fn ensure(paths: &Paths) -> Result<()> {
    for line in ensured(&control::ensure(paths)?) {
        println!("{line}");
    }
    Ok(())
}
```

with:

```rust
pub fn ensure(paths: &Paths) -> Result<()> {
    match bundle::refresh(&paths.marketplace_dir()) {
        Ok(Some(from)) => println!("{}", refreshed(&from)),
        Ok(None) => {}
        Err(error) => eprintln!("claude-plus: could not update the plus mod: {error:#}"),
    }
    for line in ensured(&control::ensure(paths)?) {
        println!("{line}");
    }
    Ok(())
}

fn refreshed(from: &str) -> String {
    format!(
        "✻ Updated the plus mod from v{from} to v{} · new Claude Code sessions load it",
        bundle::VERSION
    )
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --lib setup::bundle && cargo test --test setup daemon_ensure`
Expected: PASS, 8 tests in `setup::bundle` and the 2 `daemon_ensure` tests.

- [ ] **Step 5: Run every gate**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Then: `export PATH="/opt/homebrew/bin:$HOME/.local/bin:$PATH" && claude plugin validate mod && claude plugin validate .`
Expected: every command exits 0.

- [ ] **Step 6: Commit**

```bash
git add src/setup/bundle.rs src/cli/daemon.rs tests/setup.rs
git commit -m "feat(daemon): refresh an older installed mod on --ensure"
```

### Task 6: doctor reports Claude Code and the mod

**Files:**
- Modify: `src/cli/doctor.rs`, `tests/cli.rs`, `tests/setup.rs`

**Interfaces:**
- Consumes: `ClaudeCode::{version, plugins}`, `Plugin::loaded_version`, `MIN_VERSION` (Task 2); `bundle::{VERSION, PLUGIN_ID}` (Task 1).
- Produces: two `doctor` lines, `Claude Code:` and `Mod:`, after `Daemon:`.

- [ ] **Step 1: Write the failing tests**

Three exact `doctor` outputs in `tests/cli.rs` gain the two lines the fake produces when nothing is installed.

In `tests/cli.rs`, replace:

```rust
  Daemon:      not running, start it with claude-plus daemon --ensure
  Imported:    5 requests in 4 files · 1 unparsable line
```

with:

```rust
  Daemon:      not running, start it with claude-plus daemon --ensure
  Claude Code: 2.1.296
  Mod:         not installed, run claude-plus install
  Imported:    5 requests in 4 files · 1 unparsable line
```

In `tests/cli.rs`, replace:

```rust
  Daemon:      not running, start it with claude-plus daemon --ensure
  Imported:    {imported}
```

with:

```rust
  Daemon:      not running, start it with claude-plus daemon --ensure
  Claude Code: 2.1.296
  Mod:         not installed, run claude-plus install
  Imported:    {imported}
```

In `tests/cli.rs`, replace:

```rust
  Daemon:      not running, start it with claude-plus daemon --ensure
  Database:    not created yet, run claude-plus import
```

with:

```rust
  Daemon:      not running, start it with claude-plus daemon --ensure
  Claude Code: 2.1.296
  Mod:         not installed, run claude-plus install
  Database:    not created yet, run claude-plus import
```

Append to `tests/setup.rs`:

```rust

#[test]
fn doctor_names_the_installed_mod_and_the_folder_it_loads_from() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["install", "--yes"]).assert().success();
    cmd(&env, &["doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "  Claude Code: 2.1.296\n  Mod:         plus v{VERSION} · {}/mod\n",
            marketplace(&env).display()
        )));
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --test cli doctor`
Expected: FAIL, `cargo test --test cli doctor` reports 3 failures (`doctor_output_is_exact`, `doctor_lists_counter_only_unpriced_models`, `doctor_on_a_fresh_home_creates_nothing`) whose expected output has `Claude Code: 2.1.296` and `Mod:         not installed, run claude-plus install` after the `Daemon:` line.

- [ ] **Step 3: Implement**

`doctor` asks only `claude --version` and `claude plugin list --json`, both read only.

In `src/cli/doctor.rs`, replace:

```rust
use crate::pricing::PriceTable;
use crate::store::Store;
```

with:

```rust
use crate::pricing::PriceTable;
use crate::setup::bundle::{self, PLUGIN_ID};
use crate::setup::claude::{self, ClaudeCode, Plugin};
use crate::store::Store;
```

In `src/cli/doctor.rs`, replace:

```rust
    println!("{}", labeled("Daemon:", &line));
```

with:

```rust
    println!("{}", labeled("Daemon:", &line));
    let (claude_code, plus) = setup(&ClaudeCode::default());
    println!("{}", labeled("Claude Code:", &claude_code));
    println!("{}", labeled("Mod:", &plus));
```

In `src/cli/doctor.rs`, insert above the line `fn reindex_state(`:

```rust
fn setup(claude: &ClaudeCode) -> (String, String) {
    let version = match claude.version() {
        Ok(Some(version)) => version,
        Ok(None) => {
            return (
                format!(
                    "not found on PATH, the pane needs {} or newer",
                    claude::MIN_VERSION
                ),
                "unknown without Claude Code".into(),
            );
        }
        Err(error) => return (format!("{error:#}"), "unknown".into()),
    };
    let plugin = claude.plugins().map(|plugins| {
        plugins
            .into_iter()
            .find(|plugin| plugin.id == PLUGIN_ID && plugin.scope == "user")
    });
    let plus = match plugin {
        Ok(plugin) => mod_state(plugin.as_ref()),
        Err(error) => format!("{error:#}"),
    };
    (claude_code_state(&version), plus)
}

fn claude_code_state(version: &str) -> String {
    if control::is_older(version, claude::MIN_VERSION) {
        format!(
            "{version}, older than the {} the pane needs, run claude update",
            claude::MIN_VERSION
        )
    } else {
        version.to_string()
    }
}

fn mod_state(plugin: Option<&Plugin>) -> String {
    let Some(plugin) = plugin else {
        return "not installed, run claude-plus install".into();
    };
    let version = plugin.loaded_version();
    let from = plugin
        .folder
        .as_deref()
        .map(|folder| format!(" · {folder}"))
        .unwrap_or_default();
    if !plugin.enabled {
        format!("plus v{version} is disabled, run claude-plus install to enable it")
    } else if version != bundle::VERSION {
        format!(
            "plus v{version}, run claude-plus install to match this v{}",
            bundle::VERSION
        )
    } else {
        format!("plus v{version}{from}")
    }
}

```

In `src/cli/doctor.rs`, insert above these lines:

```rust
    #[test]
    fn a_pending_reindex
```

this:

```rust
    fn plus(version: &str, enabled: bool) -> Plugin {
        Plugin {
            id: PLUGIN_ID.into(),
            version: "0.0.1".into(),
            scope: "user".into(),
            enabled,
            folder: Some("/home/ada/.claude-plus/marketplace/mod".into()),
            folder_version: Some(version.into()),
        }
    }

    #[test]
    fn the_mod_line_says_what_loads_and_how_to_fix_it() {
        let this = bundle::VERSION;
        assert_eq!(
            mod_state(Some(&plus(this, true))),
            format!("plus v{this} · /home/ada/.claude-plus/marketplace/mod")
        );
        assert_eq!(mod_state(None), "not installed, run claude-plus install");
        assert_eq!(
            mod_state(Some(&plus(this, false))),
            format!("plus v{this} is disabled, run claude-plus install to enable it")
        );
        assert_eq!(
            mod_state(Some(&plus("0.0.9", true))),
            format!("plus v0.0.9, run claude-plus install to match this v{this}")
        );
    }

    #[test]
    fn an_old_claude_code_is_named_with_the_fix() {
        assert_eq!(claude_code_state("2.1.296"), "2.1.296");
        assert_eq!(claude_code_state("2.2.0"), "2.2.0");
        assert_eq!(
            claude_code_state("2.1.200"),
            "2.1.200, older than the 2.1.296 the pane needs, run claude update"
        );
    }

```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --test cli doctor && cargo test --test setup doctor && cargo test --lib cli::doctor`
Expected: PASS, the 10 doctor tests of `tests/cli.rs`, the doctor test of `tests/setup.rs` and 3 tests in `cli::doctor`.

- [ ] **Step 5: Run every gate**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Then: `export PATH="/opt/homebrew/bin:$HOME/.local/bin:$PATH" && claude plugin validate mod && claude plugin validate .`
Expected: every command exits 0.

- [ ] **Step 6: Commit**

```bash
git add src/cli/doctor.rs tests/cli.rs tests/setup.rs
git commit -m "feat(doctor): report the Claude Code version and the installed mod"
```

### Task 7: Output into a closed pipe ends quietly

**Files:**
- Modify: `Cargo.toml`, `Cargo.lock`, `src/cli/mod.rs`, `tests/cli.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: every subcommand but `daemon` dies of `SIGPIPE` like other command line tools when its reader goes away.

This defect predates M7: on `main`, `claude-plus doctor | head -n 1` prints its first line, then `thread 'main' panicked ... failed printing to stdout: Broken pipe (os error 32)` and exits 101.
The test closes the read end of a pipe before the command starts, so the very first line hits it.

- [ ] **Step 1: Write the failing tests**

The test names the signal through `libc`, which Step 3 uses too, so the dependency comes first; `cargo` adds its line to `Cargo.lock` from the crates already there.

In `Cargo.toml`, replace:

```toml
indicatif = "0.18.6"
```

with:

```toml
indicatif = "0.18.6"
libc = "0.2.190"
```

Append to `tests/cli.rs`:

```rust

#[test]
fn a_reader_that_goes_away_ends_the_output_without_a_panic() {
    use std::os::unix::process::ExitStatusExt;
    let env = env();
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let output = common::process(&env, &["doctor"])
        .stdout(writer)
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(output.status.signal(), Some(libc::SIGPIPE));
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "");
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --test cli a_reader_that_goes_away`
Expected: FAIL, `assertion left == right failed`, `left: None`, `right: Some(13)`: the command exits 101 with the panic instead of dying of the signal.

- [ ] **Step 3: Implement**

The daemon keeps Rust's default of ignoring `SIGPIPE`: it writes to sockets whose clients may leave.

In `src/cli/mod.rs`, replace:

```rust
    let cli = Cli::parse();
```

with:

```rust
    let cli = Cli::parse();
    if !matches!(cli.command, Command::Daemon { .. }) {
        end_quietly_when_the_reader_goes_away();
    }
```

In `src/cli/mod.rs`, insert above the line `fn progress_bar(`:

```rust
fn end_quietly_when_the_reader_goes_away() {
    // SAFETY: restoring the default action of SIGPIPE has no preconditions.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --test cli a_reader_that_goes_away`
Expected: PASS, 1 test.

- [ ] **Step 5: Run every gate**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Then: `export PATH="/opt/homebrew/bin:$HOME/.local/bin:$PATH" && claude plugin validate mod && claude plugin validate .`
Expected: every command exits 0; `git diff Cargo.lock` adds only `"libc",` to the `claude-plus` package.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/cli/mod.rs tests/cli.rs
git commit -m "fix(cli): end quietly when the reader of the output goes away"
```

### Task 8: The install script and the release assets

**Files:**
- Create: `packaging/install.sh`, `packaging/package.sh`, `packaging/release-assets.sh`, `packaging/claude-plus.rb.in`, `LICENSE`, `README.md`, `tests/packaging.rs`

**Interfaces:**
- Consumes: the `claude-plus` binary (`--version` prints `claude-plus <version>`).
- Produces: `package.sh <target> <binary> <out dir>` writes `claude-plus-<target>.tar.gz`; `release-assets.sh <version> <owner> <dist dir>` writes `SHA256SUMS`, `claude-plus.rb` and `install.sh` there; `install.sh` reads `CLAUDE_PLUS_GITHUB_OWNER`, `CLAUDE_PLUS_VERSION`, `CLAUDE_PLUS_INSTALL_DIR` and `CLAUDE_PLUS_DOWNLOAD_URL`.

- [ ] **Step 1: Write the failing tests**

The tests build a whole release in a scratch directory under `/tmp`: four archives of the test binary (one per target, so the installer finds its own whichever machine runs it), then the release assets, then they run the rendered installer with `HOME` in that directory, a clean environment and `PATH=/usr/bin:/bin:/usr/sbin:/sbin`, against `file://` URLs.
A fake `uname` stands in for an unsupported system.

Create `tests/packaging.rs`:

```rust
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
];
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn script(name: &str) -> PathBuf {
    root().join("packaging").join(name)
}

fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("cp-pkg-")
        .tempdir_in("/tmp")
        .unwrap()
}

fn run(command: &mut Command) -> Output {
    command.output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn release(dir: &Path, owner: &str) -> PathBuf {
    let dist = dir.join("dist");
    for target in TARGETS {
        let packaged = run(Command::new("sh")
            .arg(script("package.sh"))
            .args([target, env!("CARGO_BIN_EXE_claude-plus")])
            .arg(&dist));
        assert!(packaged.status.success(), "{}", text(&packaged.stderr));
    }
    let assets = run(Command::new("sh")
        .arg(script("release-assets.sh"))
        .args([VERSION, owner])
        .arg(&dist));
    assert!(assets.status.success(), "{}", text(&assets.stderr));
    dist
}

fn install(script: &Path, home: &Path, vars: &[(&str, &str)]) -> Output {
    run(Command::new("sh")
        .arg(script)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .envs(vars.iter().copied()))
}

#[test]
fn the_installer_of_a_release_installs_its_binary_into_a_scratch_home_with_a_space() {
    let temp = scratch();
    let dist = release(temp.path(), "ada");
    let home = temp.path().join("my home");
    let url = format!("file://{}", dist.display());
    let output = install(
        &dist.join("install.sh"),
        &home,
        &[("CLAUDE_PLUS_DOWNLOAD_URL", &url)],
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    let bin = home.join(".local/bin");
    assert_eq!(
        text(&output.stdout),
        format!(
            "✻ Installed claude-plus {VERSION} to {0}/claude-plus\n  {0} is not on PATH; add it, for example with: export PATH=\"{0}:$PATH\"\n✻ Next, run claude-plus install to add the pane to Claude Code\n",
            bin.display()
        )
    );
    let version = run(Command::new(bin.join("claude-plus")).arg("--version"));
    assert_eq!(text(&version.stdout), format!("claude-plus {VERSION}\n"));
    assert_eq!(std::fs::read_dir(&bin).unwrap().count(), 1);
}

#[test]
fn the_installer_installs_where_it_is_told_and_says_nothing_about_a_path_that_has_it() {
    let temp = scratch();
    let dist = release(temp.path(), "ada");
    let dir = temp.path().join("tools");
    let url = format!("file://{}", dist.display());
    let output = run(Command::new("sh")
        .arg(dist.join("install.sh"))
        .env_clear()
        .env("HOME", temp.path())
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", dir.display()),
        )
        .env("CLAUDE_PLUS_DOWNLOAD_URL", &url)
        .env("CLAUDE_PLUS_INSTALL_DIR", &dir));
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        format!(
            "✻ Installed claude-plus {VERSION} to {}/claude-plus\n✻ Next, run claude-plus install to add the pane to Claude Code\n",
            dir.display()
        )
    );
}

#[test]
fn the_installer_refuses_a_download_that_does_not_match_its_checksum() {
    let temp = scratch();
    let dist = release(temp.path(), "ada");
    let sums = std::fs::read_to_string(dist.join("SHA256SUMS")).unwrap();
    let tampered: String = sums
        .lines()
        .map(|line| format!("{}{}\n", "0".repeat(64), &line[64..]))
        .collect();
    std::fs::write(dist.join("SHA256SUMS"), tampered).unwrap();
    let home = temp.path().join("home");
    let url = format!("file://{}", dist.display());
    let output = install(
        &dist.join("install.sh"),
        &home,
        &[("CLAUDE_PLUS_DOWNLOAD_URL", &url)],
    );
    assert!(!output.status.success());
    assert!(
        text(&output.stderr)
            .ends_with("does not match its SHA256SUMS line, so nothing was installed\n"),
        "{}",
        text(&output.stderr)
    );
    assert!(!home.join(".local/bin/claude-plus").exists());
}

#[test]
fn the_installer_in_the_repository_asks_for_a_github_owner() {
    let temp = scratch();
    let output = install(&script("install.sh"), temp.path(), &[]);
    assert!(!output.status.success());
    assert_eq!(
        text(&output.stderr),
        "claude-plus install.sh: this copy of install.sh names no GitHub owner; use the install.sh of a claude-plus release, or set CLAUDE_PLUS_GITHUB_OWNER\n"
    );
}

#[test]
fn the_installer_names_a_platform_without_a_build() {
    use std::os::unix::fs::PermissionsExt;
    let temp = scratch();
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        bin.join("uname"),
        "#!/bin/sh\ncase \"$1\" in -s) echo FreeBSD ;; -m) echo amd64 ;; esac\n",
    )
    .unwrap();
    std::fs::set_permissions(bin.join("uname"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = run(Command::new("sh")
        .arg(script("install.sh"))
        .env_clear()
        .env("HOME", temp.path())
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("CLAUDE_PLUS_GITHUB_OWNER", "ada"));
    assert!(!output.status.success());
    assert_eq!(
        text(&output.stderr),
        "claude-plus install.sh: claude-plus runs on macOS and Linux, not FreeBSD\n"
    );
}

#[test]
fn release_assets_hold_checksums_a_formula_and_an_installer_for_the_owner() {
    let temp = scratch();
    let dist = release(temp.path(), "ada");
    let sums = std::fs::read_to_string(dist.join("SHA256SUMS")).unwrap();
    let names: Vec<&str> = sums.lines().map(|line| &line[66..]).collect();
    assert_eq!(
        names,
        TARGETS.map(|target| format!("claude-plus-{target}.tar.gz"))
    );
    let formula = std::fs::read_to_string(dist.join("claude-plus.rb")).unwrap();
    for line in sums.lines() {
        let (sum, archive) = line.split_once("  ").unwrap();
        assert!(formula.contains(&format!(
            "      url \"https://github.com/ada/claude-plus/releases/download/v{VERSION}/{archive}\"\n      sha256 \"{sum}\"\n"
        )), "{formula}");
    }
    assert!(formula.contains(&format!("  version \"{VERSION}\"\n")));
    assert!(!formula.contains('@'), "{formula}");
    let ruby = run(Command::new("ruby")
        .arg("-c")
        .arg(dist.join("claude-plus.rb")));
    assert!(ruby.status.success(), "{}", text(&ruby.stderr));
    let installer = std::fs::read_to_string(dist.join("install.sh")).unwrap();
    assert!(installer.contains("github_owner=\"${CLAUDE_PLUS_GITHUB_OWNER:-ada}\"\n"));
    assert!(!installer.contains("@GITHUB_OWNER@"));
    let parsed = run(Command::new("sh").arg("-n").arg(dist.join("install.sh")));
    assert!(parsed.status.success(), "{}", text(&parsed.stderr));
    let listing = run(Command::new("tar")
        .arg("-tzf")
        .arg(dist.join("claude-plus-x86_64-apple-darwin.tar.gz")));
    assert_eq!(text(&listing.stdout), "claude-plus\nLICENSE\nREADME.md\n");
}

#[test]
fn release_assets_need_all_four_builds() {
    let temp = scratch();
    let dist = temp.path().join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    let packaged = run(Command::new("sh")
        .arg(script("package.sh"))
        .args(["aarch64-apple-darwin", env!("CARGO_BIN_EXE_claude-plus")])
        .arg(&dist));
    assert!(packaged.status.success());
    let assets = run(Command::new("sh")
        .arg(script("release-assets.sh"))
        .args([VERSION, "ada"])
        .arg(&dist));
    assert!(!assets.status.success());
    assert_eq!(
        text(&assets.stderr),
        format!(
            "release-assets.sh: {}/claude-plus-x86_64-apple-darwin.tar.gz is missing\n",
            dist.display()
        )
    );
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --test packaging`
Expected: FAIL, all 7 fail: the scripts do not exist (`sh: .../packaging/package.sh: No such file or directory`).

- [ ] **Step 3: Implement**

The scripts take their arguments and environment only, quote every path, and clean up their temporary directories on exit.

Create `LICENSE`:

```text
MIT License

Copyright (c) 2026 Gabriel Dinant

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

Create `README.md`:

````markdown
# claude-plus

Token, cache and cost visibility for Claude Code.
claude-plus draws a live pane beside the conversation with what each session costs, what fills its context, when its cache goes cold, how fast it uses your subscription windows, and tips that save tokens.
A local daemon reads your Claude Code transcripts and keeps their history; nothing leaves your machine.

## Install

claude-plus runs on macOS and Linux and needs Claude Code 2.1.296 or newer.

Install the binary with Homebrew:

```bash
brew install @GITHUB_OWNER@/tap/claude-plus
```

Or with the install script, which puts it in `~/.local/bin`:

```bash
curl -fsSL https://github.com/@GITHUB_OWNER@/claude-plus/releases/latest/download/install.sh | sh
```

Then add the pane to Claude Code:

```bash
claude-plus install
```

`claude-plus install` shows the `claude plugin` commands it runs and asks before it runs them, because Claude Code records the marketplace and the plugin in `~/.claude/settings.json`; `--yes` agrees without asking.
It then starts the daemon, which imports your history in the background.
Start a new Claude Code session: the pane docks beside the conversation when the terminal is at least 144 columns wide, and `/plus` opens it at any width.

The mod alone can also be installed from Claude Code's prompt; it still needs the `claude-plus` binary on `PATH`:

```
/plugin install plus --marketplace @GITHUB_OWNER@/claude-plus
```

## Upgrade

Upgrade the binary with `brew upgrade claude-plus` or by running the install script again.
The next Claude Code session restarts the daemon on the new version and loads the new pane.

## Uninstall

```bash
claude-plus uninstall
```

This removes the mod and its marketplace from Claude Code and stops the daemon; `--purge` also deletes `~/.claude-plus` with the history claude-plus keeps.
Then remove the binary with `brew uninstall claude-plus`, or delete `~/.local/bin/claude-plus`.

## Commands

| Command | What it does |
|---|---|
| `claude-plus install` | Add the pane to Claude Code and start the daemon. |
| `claude-plus uninstall [--purge]` | Remove the pane and stop the daemon, and with `--purge` delete the data. |
| `claude-plus stats [--days N]` | Print daily cost per model. |
| `claude-plus export --format json\|csv` | Print stored usage per day, project and model. |
| `claude-plus doctor` | Check the binary, daemon, mod, Claude Code version and transcript retention. |
| `claude-plus import` | Read new transcripts into the store, with progress. |
| `claude-plus reindex` | Rebuild derived tables from stored records. |
| `claude-plus daemon [--ensure]` | Run the daemon, or start it detached unless one answers. |

## Privacy

claude-plus makes no network requests.
The daemon listens only on a Unix socket in `~/.claude-plus`, readable by you alone, and never edits Claude Code's files.

## License

MIT
````

Create `packaging/install.sh`, and make it executable (`chmod +x packaging/install.sh`):

```sh
#!/bin/sh
set -eu

github_owner="${CLAUDE_PLUS_GITHUB_OWNER:-@GITHUB_OWNER@}"
version="${CLAUDE_PLUS_VERSION:-latest}"
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

command -v curl >/dev/null 2>&1 || fail "curl is needed to download claude-plus"
target=$(detect_target)
base=$(download_base)
archive="claude-plus-$target.tar.gz"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT INT TERM

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
mkdir -p "$install_dir"
cp "$scratch/unpacked/claude-plus" "$install_dir/.claude-plus.new"
chmod 755 "$install_dir/.claude-plus.new"
mv -f "$install_dir/.claude-plus.new" "$install_dir/claude-plus"

say "✻ Installed $("$install_dir/claude-plus" --version) to $install_dir/claude-plus"
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *) say "  $install_dir is not on PATH; add it, for example with: export PATH=\"$install_dir:\$PATH\"" ;;
esac
say "✻ Next, run claude-plus install to add the pane to Claude Code"
```

Create `packaging/package.sh`, and make it executable (`chmod +x packaging/package.sh`):

```sh
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
```

Create `packaging/release-assets.sh`, and make it executable (`chmod +x packaging/release-assets.sh`):

```sh
#!/bin/sh
set -eu

[ "$#" -eq 3 ] || {
  echo "usage: release-assets.sh <version> <github owner> <dist dir>" >&2
  exit 2
}
version=$1
owner=$2
dist=$3
here=$(cd "$(dirname "$0")" && pwd)
targets="aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-musl x86_64-unknown-linux-musl"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

: > "$dist/SHA256SUMS"
substitutions="s|@VERSION@|$version|g; s|@GITHUB_OWNER@|$owner|g"
for target in $targets; do
  archive="claude-plus-$target.tar.gz"
  [ -f "$dist/$archive" ] || {
    echo "release-assets.sh: $dist/$archive is missing" >&2
    exit 1
  }
  sum=$(sha256 "$dist/$archive")
  printf '%s  %s\n' "$sum" "$archive" >> "$dist/SHA256SUMS"
  key=$(printf '%s' "$target" | tr 'a-z-' 'A-Z_')
  substitutions="$substitutions; s|@SHA256_${key}@|$sum|g"
done
sed "$substitutions" "$here/claude-plus.rb.in" > "$dist/claude-plus.rb"
sed "s|@GITHUB_OWNER@|$owner|g" "$here/install.sh" > "$dist/install.sh"
```

Create `packaging/claude-plus.rb.in`:

```ruby
class ClaudePlus < Formula
  desc "Token, cache and cost visibility for Claude Code"
  homepage "https://github.com/@GITHUB_OWNER@/claude-plus"
  version "@VERSION@"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/@GITHUB_OWNER@/claude-plus/releases/download/v@VERSION@/claude-plus-aarch64-apple-darwin.tar.gz"
      sha256 "@SHA256_AARCH64_APPLE_DARWIN@"
    end
    on_intel do
      url "https://github.com/@GITHUB_OWNER@/claude-plus/releases/download/v@VERSION@/claude-plus-x86_64-apple-darwin.tar.gz"
      sha256 "@SHA256_X86_64_APPLE_DARWIN@"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/@GITHUB_OWNER@/claude-plus/releases/download/v@VERSION@/claude-plus-aarch64-unknown-linux-musl.tar.gz"
      sha256 "@SHA256_AARCH64_UNKNOWN_LINUX_MUSL@"
    end
    on_intel do
      url "https://github.com/@GITHUB_OWNER@/claude-plus/releases/download/v@VERSION@/claude-plus-x86_64-unknown-linux-musl.tar.gz"
      sha256 "@SHA256_X86_64_UNKNOWN_LINUX_MUSL@"
    end
  end

  def install
    bin.install "claude-plus"
  end

  def caveats
    <<~EOS
      Run claude-plus install to add the pane to Claude Code.
      After an upgrade, new Claude Code sessions update the pane by themselves.
    EOS
  end

  test do
    assert_match "claude-plus #{version}", shell_output("#{bin}/claude-plus --version")
  end
end
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --test packaging`
Expected: PASS, 7 tests.

- [ ] **Step 5: Check the scripts with a POSIX shell**

Run: `for f in packaging/*.sh; do dash -n "$f" || echo "$f"; done; grep -c "$(printf '\342\200\224')" README.md packaging/*`
Expected: no file name printed by the first loop (macOS and Ubuntu both ship `/bin/dash`), and every count `0` (no em dash).

- [ ] **Step 6: Run every gate**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Then: `export PATH="/opt/homebrew/bin:$HOME/.local/bin:$PATH" && claude plugin validate mod && claude plugin validate .`
Expected: every command exits 0.

- [ ] **Step 7: Commit**

```bash
git add packaging/install.sh packaging/package.sh packaging/release-assets.sh packaging/claude-plus.rb.in LICENSE README.md tests/packaging.rs
git commit -m "feat(packaging): add the install script, release archives and Homebrew formula"
```

### Task 9: Pinned CI, the install job and the draft release

**Files:**
- Modify: `.github/workflows/ci.yml`, `tests/packaging.rs`
- Create: `.github/workflows/release.yml`, `.github/scripts/install-claude-code.sh`, `.github/scripts/actionlint.sh`, `packaging/e2e-install.sh`

**Interfaces:**
- Consumes: `packaging/package.sh` and `packaging/release-assets.sh` (Task 8); `claude-plus install --yes`, `install` without a terminal, `doctor`, `uninstall --purge --yes` (Tasks 3, 4 and 6).
- Produces: the `ci` jobs `rust`, `msrv`, `mod`, `install` and `lint`; the `release` workflow with `build` (four targets) and `draft`.

- [ ] **Step 1: Write the failing tests**

The test reads every workflow line by line: each `uses:` must be `<owner>/<repo>@<40 lowercase hex> # v<major>.<minor>.<patch>`, no `os:` may end in `-latest`, and no line may pipe into `sh` or `bash` or name a moving `@stable` or `@master`.

Append to `tests/packaging.rs`:

```rust

fn workflows() -> Vec<(String, String)> {
    let dir = root().join(".github/workflows");
    let mut found: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read_to_string(&path).unwrap())
        })
        .collect();
    found.sort();
    found
}

fn is_pinned(action: &str) -> bool {
    let Some((name, rest)) = action.split_once('@') else {
        return false;
    };
    let Some((sha, tag)) = rest.split_once(" # ") else {
        return false;
    };
    name.split('/').count() == 2
        && sha.len() == 40
        && sha
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        && tag.strip_prefix('v').is_some_and(|version| {
            version.split('.').count() == 3 && version.split('.').all(|n| n.parse::<u32>().is_ok())
        })
}

#[test]
fn workflows_pin_every_action_to_a_commit_and_every_runner_to_an_image() {
    let found = workflows();
    assert_eq!(
        found
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["ci.yml", "release.yml"]
    );
    for (name, text) in &found {
        for line in text.lines().map(str::trim) {
            if let Some(action) = line.trim_start_matches("- ").strip_prefix("uses: ") {
                assert!(is_pinned(action), "{name}: {line}");
            }
            if let Some(runner) = line.trim_start_matches("- ").strip_prefix("os: ") {
                for image in runner.trim_matches(['[', ']']).split(", ") {
                    assert!(!image.ends_with("-latest"), "{name}: {line}");
                }
            }
            assert!(
                !line.contains("| bash") && !line.contains("| sh"),
                "{name}: {line}"
            );
            assert!(
                !line.contains("@stable") && !line.contains("@master"),
                "{name}: {line}"
            );
        }
    }
    assert!(!is_pinned("actions/checkout@v4"));
    assert!(!is_pinned(
        "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
    ));
    assert!(is_pinned(
        "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1"
    ));
}

#[test]
fn the_release_only_drafts_and_the_tag_must_match_the_version() {
    let (_, release) = workflows()
        .into_iter()
        .find(|(name, _)| name == "release.yml")
        .unwrap();
    assert!(release.contains("on:\n  push:\n    tags: [\"v*\"]\n"));
    assert!(
        release.contains(
            "gh release create \"$GITHUB_REF_NAME\" --repo \"$GITHUB_REPOSITORY\" --draft "
        )
    );
    assert_eq!(release.matches("contents: write").count(), 1);
    for target in TARGETS {
        assert!(
            release.contains(&format!("          - target: {target}\n")),
            "{target}"
        );
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --test packaging -- workflows the_release`
Expected: FAIL, both tests fail: `ci.yml: os: [macos-latest, ubuntu-latest]`, and the workflow list is `["ci.yml"]` instead of `["ci.yml", "release.yml"]`.

- [ ] **Step 3: Implement**

The SHAs are those the tags pointed at on 2026-10-11 (`git ls-remote https://github.com/actions/checkout refs/tags/v7.0.1` and so on; all four are lightweight tags, so the SHA is the commit).
The Claude Code checksums are the `darwin-arm64` and `linux-x64` entries of `https://downloads.claude.ai/claude-code-releases/2.1.296/manifest.json`; the `darwin-arm64` one was checked against a download.
The `actionlint` checksum is the `linux_amd64` line of its v1.7.12 `checksums.txt`.

Replace the whole of `.github/workflows/ci.yml`:

```yaml
name: ci
on:
  push:
  pull_request:
permissions:
  contents: read
env:
  RUST_TOOLCHAIN: 1.99.0
  MSRV_TOOLCHAIN: 1.88.0
  NODE_VERSION: 22.23.3
  DISABLE_AUTOUPDATER: 1
jobs:
  rust:
    strategy:
      matrix:
        os: [macos-15, ubuntu-24.04]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - run: rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal --component rustfmt,clippy && rustup default "$RUST_TOOLCHAIN"
      - run: cargo fmt --check
      - run: cargo clippy --locked --all-targets -- -D warnings
      - run: cargo test --locked
  msrv:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - run: rustup toolchain install "$MSRV_TOOLCHAIN" --profile minimal
      - run: cargo "+$MSRV_TOOLCHAIN" check --locked --all-targets
  mod:
    strategy:
      matrix:
        os: [macos-15, ubuntu-24.04]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - uses: actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15c1 # v7.1.0
        with:
          node-version: ${{ env.NODE_VERSION }}
      - run: sh .github/scripts/install-claude-code.sh
      - run: echo "$HOME/.local/bin" >> "$GITHUB_PATH"
      - run: claude plugin validate mod
      - run: claude plugin validate .
      - run: claude plugin test mod
      - run: npx --yes -p typescript@6.0.3 tsc -p mod-dev/tsconfig.json
  install:
    strategy:
      matrix:
        os: [macos-15, ubuntu-24.04]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - run: rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal && rustup default "$RUST_TOOLCHAIN"
      - run: sh .github/scripts/install-claude-code.sh
      - run: echo "$HOME/.local/bin" >> "$GITHUB_PATH"
      - run: cargo build --locked
      - run: sh packaging/e2e-install.sh target/debug/claude-plus
  lint:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - run: shellcheck --shell=sh packaging/*.sh .github/scripts/*.sh tests/fake-claude/claude
      - run: sh .github/scripts/actionlint.sh
```

Create `.github/workflows/release.yml`:

```yaml
name: release
on:
  push:
    tags: ["v*"]
permissions:
  contents: read
env:
  RUST_TOOLCHAIN: 1.99.0
  CC_x86_64_unknown_linux_musl: musl-gcc
  CC_aarch64_unknown_linux_musl: musl-gcc
jobs:
  build:
    strategy:
      matrix:
        include:
          - target: aarch64-apple-darwin
            os: macos-15
          - target: x86_64-apple-darwin
            os: macos-15
          - target: aarch64-unknown-linux-musl
            os: ubuntu-24.04-arm
          - target: x86_64-unknown-linux-musl
            os: ubuntu-24.04
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - name: Check that the tag names this version
        run: test "claude-plus ${GITHUB_REF_NAME#v}" = "claude-plus $(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
      - run: rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal --target "${{ matrix.target }}" && rustup default "$RUST_TOOLCHAIN"
      - if: runner.os == 'Linux'
        run: sudo apt-get update && sudo apt-get install --yes musl-tools
      - run: cargo build --release --locked --target "${{ matrix.target }}"
      - name: Check that the binary runs and names its version
        if: matrix.target != 'x86_64-apple-darwin'
        run: test "$("target/${{ matrix.target }}/release/claude-plus" --version)" = "claude-plus ${GITHUB_REF_NAME#v}"
      - run: sh packaging/package.sh "${{ matrix.target }}" "target/${{ matrix.target }}/release/claude-plus" dist
      - uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2
        with:
          name: claude-plus-${{ matrix.target }}
          path: dist/claude-plus-${{ matrix.target }}.tar.gz
          if-no-files-found: error
  draft:
    needs: build
    runs-on: ubuntu-24.04
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - uses: actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333 # v8.0.2
        with:
          pattern: claude-plus-*
          path: dist
          merge-multiple: true
      - run: sh packaging/release-assets.sh "${GITHUB_REF_NAME#v}" "$GITHUB_REPOSITORY_OWNER" dist
      - name: Create a draft release for a person to publish
        env:
          GH_TOKEN: ${{ github.token }}
        run: gh release create "$GITHUB_REF_NAME" --repo "$GITHUB_REPOSITORY" --draft --verify-tag --title "claude-plus ${GITHUB_REF_NAME#v}" --generate-notes dist/*
```

Create `.github/scripts/install-claude-code.sh`, and make it executable (`chmod +x .github/scripts/install-claude-code.sh`):

```sh
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
```

Create `.github/scripts/actionlint.sh`, and make it executable (`chmod +x .github/scripts/actionlint.sh`):

```sh
#!/bin/sh
set -eu

version=1.7.12
expected=8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8
archive="actionlint_${version}_linux_amd64.tar.gz"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT INT TERM
curl --fail --silent --show-error --location --retry 3 -o "$scratch/$archive" \
  "https://github.com/rhysd/actionlint/releases/download/v$version/$archive"
actual=$(sha256sum "$scratch/$archive" | awk '{print $1}')
if [ "$actual" != "$expected" ]; then
  echo "actionlint.sh: $archive does not match its pinned checksum" >&2
  exit 1
fi
tar -xzf "$scratch/$archive" -C "$scratch" actionlint
"$scratch/actionlint" -color
```

Create `packaging/e2e-install.sh`, and make it executable (`chmod +x packaging/e2e-install.sh`):

```sh
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo test --test packaging`
Expected: PASS, 9 tests.

- [ ] **Step 5: Check the workflows offline and run the install end to end**

No `actionlint` or `shellcheck` is installed here, so parse the workflows and the scripts instead, then run the install with the real Claude Code in a scratch home (the script makes it under `/tmp`, stops its daemon and deletes it on exit).

Run: `for f in .github/workflows/*.yml; do ruby -ryaml -e 'YAML.load_file(ARGV[0]); puts "parsed #{ARGV[0]}"' "$f"; done; for f in packaging/*.sh .github/scripts/*.sh tests/fake-claude/claude; do dash -n "$f" || echo "$f"; done`
Expected: `parsed .github/workflows/ci.yml` and `parsed .github/workflows/release.yml`, and no script name.

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo build --locked && export PATH="/opt/homebrew/bin:$HOME/.local/bin:$PATH" && sh packaging/e2e-install.sh target/debug/claude-plus`
Expected: exit 0, with these lines among the command output:

```text
✻ a declined install changes nothing
✻ install added the marketplace and the mod
✻ a second install asks nothing and changes nothing
✻ doctor names the installed mod
✻ uninstall --purge removed the mod, the marketplace and the data
```

Then: `ls -d /tmp/claude-plus-e2e.* 2>/dev/null | wc -l`
Expected: `0`.

- [ ] **Step 6: Run every gate**

Run: `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Then: `export PATH="/opt/homebrew/bin:$HOME/.local/bin:$PATH" && claude plugin validate mod && claude plugin validate .`
Expected: every command exits 0.

- [ ] **Step 7: Commit**

```bash
git add .github/workflows/ci.yml .github/workflows/release.yml .github/scripts/install-claude-code.sh .github/scripts/actionlint.sh packaging/e2e-install.sh tests/packaging.rs
git commit -m "ci: pin actions and tools by checksum, test the install, draft releases"
```

### Task 10: The check by hand

This task writes the check, then the user runs it in a real terminal after Tasks 1 to 9 are on `main`.
Writing the file needs no session; running it is the only place in M7 where the real `~/.claude/settings.json` changes (after the user answers `y`) and where an interactive `claude` session is started, and it is the user who does it.

**Files:**
- Create: `docs/manual-checks/m7-packaging.md`

- [ ] **Step 1: Write the check**

Create `docs/manual-checks/m7-packaging.md`:

````markdown
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

- [ ] Run `cp ~/.claude/settings.json /tmp/settings.before.json`, then `claude-plus install`: it lists `claude plugin marketplace add` with the full path of `~/.claude-plus/marketplace` and `claude plugin install plus@claude-plus`, says Claude Code records them in `~/.claude/settings.json`, and asks `Continue? [y/N]`.
- [ ] Answering `n` prints `claude-plus: install cancelled, nothing was changed; ...`, `diff /tmp/settings.before.json ~/.claude/settings.json` prints nothing, and `~/.claude-plus/marketplace` does not exist.
- [ ] Running it again and answering `y` prints `✻ Added the claude-plus marketplace · ...`, `✻ Installed the plus mod v0.1.0`, a daemon line, no PATH warning, and the line about starting a new session.
- [ ] `~/.claude/settings.json` now holds `extraKnownMarketplaces.claude-plus` (a `directory` source at `~/.claude-plus/marketplace`) and `enabledPlugins["plus@claude-plus"]: true`, and nothing else changed.
- [ ] `claude plugin list` shows `plus@claude-plus` 0.1.0, read from `~/.claude-plus/marketplace/mod`.
- [ ] `claude-plus doctor` shows `Claude Code: 2.1.296` (or newer) and `Mod:         plus v0.1.0 · ~/.claude-plus/marketplace/mod` (the full path).
- [ ] A new `claude` session (no `--plugin-dir`) at 144 columns or more docks the pane at once; `/plus` opens it in a narrower terminal; the status entry shows.
- [ ] `claude-plus install` a second time asks nothing and says `✻ The plus mod v0.1.0 is installed`.
- [ ] `claude-plus stats | head -n 3` prints three lines and no panic.

## Upgrade

- [ ] Edit `~/.claude-plus/marketplace/mod/.claude-plugin/plugin.json` to `"version": "0.0.1"`, then run `claude-plus daemon --ensure`: it prints `✻ Updated the plus mod from v0.0.1 to v0.1.0 · new Claude Code sessions load it`, and the file says `0.1.0` again.
- [ ] Repeat the edit and start a new Claude Code session instead: the session's own `--ensure` puts the file back to `0.1.0`, and the pane works.

## Uninstall

- [ ] `claude-plus uninstall` lists the two `claude plugin` commands, asks, and after `y` prints `✻ Uninstalled the plus mod`, `✻ Removed the claude-plus marketplace`, `✻ Stopped the claude-plus daemon · pid ...` and the line about open sessions.
- [ ] `~/.claude/settings.json` no longer lists the marketplace or the plugin, `~/.claude-plus/marketplace` is gone, and `~/.claude-plus/data.db` is still there.
- [ ] A new `claude` session has no pane and no `/plus`.
- [ ] Reinstall with `claude-plus install --yes` to keep using the pane; do not run `--purge` unless you mean to delete the history.

## Releasing (after the GitHub owner is filled in)

Nothing in the plan pushed, tagged or published; these steps are yours.

- [ ] Replace `@GITHUB_OWNER@` in `README.md` with the GitHub owner (`git grep -n @GITHUB_OWNER@` lists every place; `packaging/install.sh` and `packaging/claude-plus.rb.in` keep it, because the release fills them in).
- [ ] Push `main` to `github.com/<owner>/claude-plus`; the `ci` workflow passes on macOS and Linux, its `install` job included.
- [ ] Push the tag `v0.1.0`; the `release` workflow builds four archives and creates a draft release with them, `SHA256SUMS`, `install.sh` and `claude-plus.rb`.
- [ ] Download the draft's assets and check `shasum -a 256 -c SHA256SUMS`; publish the draft.
- [ ] Create `github.com/<owner>/homebrew-tap` and commit the draft's `claude-plus.rb` as `Formula/claude-plus.rb`.
- [ ] `brew install <owner>/tap/claude-plus` then `brew test claude-plus` pass.
- [ ] `curl -fsSL https://github.com/<owner>/claude-plus/releases/latest/download/install.sh | sh` installs into `~/.local/bin` with the PATH hint when needed.
- [ ] `/plugin install plus --marketplace <owner>/claude-plus` at a Claude Code prompt finds the `plus` mod.

## Outcome

If every line holds, nothing is committed.
Otherwise fix each defect in its owning task's files with a test that pins it, commit it on its own as `fix(<unit>): ...`, and repeat the check.
````

- [ ] **Step 2: Check the prose rules**

Run: `grep -nE "[.!?] +[A-Z]" docs/manual-checks/m7-packaging.md README.md; grep -c "$(printf '\342\200\224')" docs/manual-checks/m7-packaging.md`
Expected: the first command prints nothing (one sentence per line) and the second prints `0` (no em dash).

- [ ] **Step 3: Commit**

```bash
git add docs/manual-checks/m7-packaging.md
git commit -m "docs: add M7 manual check"
```

- [ ] **Step 4: The user runs the check**

The user walks through `docs/manual-checks/m7-packaging.md` in a real terminal, then the release steps when the owner is known.
If every line holds, nothing more is committed.
Otherwise each defect is fixed in its owning task's files with a test that pins it, committed on its own as `fix(<unit>): ...`, and the check is repeated.

---

## Self-Review

Spec coverage, line by line of milestone 7 and the sections it touches:

- "`claude-plus install`: Register the plugin marketplace and install the mod via the `claude plugin` CLI, start the daemon, kick off the history import. Idempotent.": Task 1 (the marketplace the binary carries), Task 2 (the CLI), Task 3 (the steps, the daemon through `control::ensure`, whose backfill is the import; the second run asks nothing and changes nothing).
- "`claude-plus uninstall [--purge]`: Uninstall the mod, stop the daemon, with `--purge` delete `~/.claude-plus`.": Task 4.
- "One install command (`claude-plus install`, after installing the binary) gives a working pane in every new Claude Code session, with no manual config editing.": Tasks 1 and 3, checked with the real Claude Code by Task 9's `e2e-install.sh` and by hand in Task 10.
- "claude-plus never edits `~/.claude/settings.json`": Decision 3; claude-plus never opens it, and the Claude Code commands that write it run only after consent (Task 3 test `install_asks_before_claude_code_records_anything_in_its_settings`, Task 4 test `uninstall_asks_first_and_purge_deletes_the_data_dir`, the declined install of `e2e-install.sh`).
- "The binary ships via a Homebrew tap and a `curl | sh` installer": Task 8 (`install.sh`, the formula), Task 9 (the release that fills them in); the tap repository is the user's step.
- "the mod ships via a plugin marketplace in the same repository": Task 1 (`.claude-plugin/marketplace.json` at the root, validated in CI by Task 9).
- "macOS first and Linux second, on x86_64 and aarch64" and "builds release binaries for four targets": Decision 11, Task 9 (`release.yml`).
- "CI on GitHub Actions for macOS and Linux runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `claude plugin validate`, `claude plugin test`": kept in Task 9's `ci.yml`, now pinned, with the `install` and `lint` jobs added.
- "`--ensure` orders two builds ... replaces a running daemon that is older": reused by `install` (Task 3) and extended to the mod folder (Task 5).
- "`doctor`: Diagnose binary, daemon, mod install, Claude Code version": the mod and Claude Code lines are Task 6.
- "0700 directory, 0600 files": Task 1 test `writing_the_bundle_replaces_the_folder_and_keeps_it_private`.
- The M3 carryover, pinning CI actions and checksums: Decision 15, Task 9 test `workflows_pin_every_action_to_a_commit_and_every_runner_to_an_image`.

Placeholder scan: every step holds the full file or the exact hunk it applies, every run names its command and its expected result; `@GITHUB_OWNER@` is the one deliberate placeholder, which the release fills in and the user replaces in `README.md`.

Type consistency: the names in each task's Interfaces block are the ones its code defines and later tasks call (`bundle::write`, `bundle::refresh`, `bundle::remove`, `ClaudeCode::run`, `Step::edits_settings`, `install_steps`, `uninstall_steps`, `ensured`, `stopped`, `confirm`, `path_warning`), and the fake `claude` answers exactly the command lines `Step::args` builds, which Task 3's call-log assertions pin.

What only the check by hand and the first tag can tell, and why the plan does not guess it:

- whether the pane docks in a real session from the installed mod, as it does from `--plugin-dir`;
- whether the Linux musl builds and the `ubuntu-24.04-arm` runner work as assumed;
- whether Homebrew accepts the formula in a real tap (`brew audit` needs one);
- whether open sessions really start the daemon again after an uninstall, as Decision 9 expects.
