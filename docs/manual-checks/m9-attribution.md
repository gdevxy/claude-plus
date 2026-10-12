# Manual check for session attribution

Run this by hand in a real terminal on a machine that has months of Claude Code history, after Tasks 1 to 5 of the session attribution backfill plan are on `main`.
It needs the Claude desktop app signed in to a company account and the CLI signed in to a private one.

## Run this build

Build the release binary and put it first on `PATH`, so the pane and the CLI both use this build and not the Homebrew one.

```bash
cargo build --release --locked
export PATH="$PWD/target/release:$PATH"
claude-plus daemon stop
claude-plus --version
```

- [ ] Before anything else, copy the database so the check can be repeated: `cp ~/.claude-plus/data.db /tmp/data-before.db`.
- [ ] Note the Unknown total: `claude-plus stats --days 31 --account all` and `claude-plus accounts`.

## Checks

- [ ] Run `claude-plus doctor`: the `Desktop:` line names `~/Library/Application Support/Claude/claude-code-sessions` with a number of session files, and any skipped ones are worth a look in the files themselves.
- [ ] Run `claude-plus import` with the daemon stopped: it prints the usual report and at most one `Skipped the desktop session file ...` line per broken file.
- [ ] Run `claude-plus accounts`: the desktop app's account appears with its CLI label if the CLI ever signed in to it, else as `Desktop account <8 characters>`, and its Sessions count matches the desktop app's session list.
- [ ] Pick a session you ran in the desktop app while the CLI was signed in to the private account; `sqlite3 ~/.claude-plus/data.db "SELECT account_id, source FROM session_accounts WHERE session_id = '<cliSessionId>'"` shows the desktop account and `desktop`.
- [ ] Run `claude-plus accounts assign "<private label>" --project '~/<private folder>/*' --dry-run`: it says `Would assign <n> sessions to <private label>` with a date range and spend that look right, and `claude-plus accounts` is unchanged.
- [ ] Run the same command without `--dry-run`: it says `Assigned <n> sessions ...`, and running it again says `No Unknown sessions match`.
- [ ] Assign the company period: `claude-plus accounts assign "<company label>" --since <first day> --until <last day>`.
- [ ] Run `claude-plus stats --days 31` and `claude-plus stats --days 31 --account "<company label>"`: the private and company totals now add up to most of what was Unknown.
- [ ] Open the History view in the pane and press `a`: each account shows its own history and the Unknown note shrinks.
- [ ] Run `claude-plus accounts unassign --project '~/<private folder>/*' --dry-run`, then without `--dry-run`: the sessions are Unknown again, and assigning them once more restores the totals.
- [ ] Start a new session in the desktop app and send one prompt: within a minute the pane shows the company account for it, not the CLI's private one.
- [ ] Confirm nothing was written to the desktop app's folder: `ls -lT ~/Library/Application\ Support/Claude/claude-code-sessions/*/*/ | head` shows no modification time from this check other than the desktop app's own.

## Outcome

If every line holds, nothing is committed.
Otherwise fix each defect in its owning task's files with a test that pins it, commit it on its own as `fix(<unit>): ...`, and repeat the check.
