# Manual check for accounts and budgets

Run this by hand in a real terminal, after Tasks 1 to 12 of the accounts and budgets plan are on `main`.
It needs a private Max subscription login and a company Enterprise login, and a Claude Code session for each.

## Run this build

Build the release binary and put it first on `PATH`, so the pane and the CLI both use this build and not the Homebrew one.
Expected: `claude-plus 0.1.0`.

```bash
cargo build --release --locked
export PATH="$PWD/target/release:$PATH"
claude-plus --version
```

- [ ] Run `claude-plus daemon stop` before you switch binaries, so the next session starts a daemon from this build.
- [ ] Run `./target/release/claude-plus install` from the repo: it installs the mod bundled in this build, and the next Claude Code session starts the new daemon.
- [ ] As an alternative to `PATH`, set `binaryPath` to the absolute path of `target/release/claude-plus` with `/plugin configure plus@claude-plus`.
- [ ] `claude-plus doctor` shows a `Login:` line and `Prices: Anthropic's ...`, and adds `, with your prices.toml` once you create that file.

## Checks

- [ ] Signed in with the private subscription, start a session and send one prompt: the pane names `<org> · Max` under the model and shows the 5h and 7d bars under Limits.
- [ ] Run `/logout`, then `/login` with the company account, start a new session and send one prompt: the pane names `<org> · Enterprise`, shows only `Spend` with the percent and the hint `claude-plus budget set <usd> shows dollars`, and no 7d bar from the private account.
- [ ] Run `claude-plus budget set 700`: within a few seconds the Spend detail reads `$x of $700 · resets in ...` with `Claude Code here: $y` under it, and the dollar figure matches the claude.ai usage page to within one percent of $700.
- [ ] Run `claude-plus budget show`: it prints `Account:`, `Budget:`, `Reported:`, `Local:` and `Pace:` lines.
- [ ] Open the History view and press `a`: the title cycles through the company account, the private one and All, and the totals change accordingly.
- [ ] Run `claude-plus accounts`: it lists both accounts, with this month's spend and the budget on the company row.
- [ ] Run `claude-plus stats --days 1`: it shows the company account; `claude-plus stats --days 1 --account all` shows both sessions.
- [ ] Run `claude-plus export --format csv | head -n 2`: the header has an `account` column after `project`.
- [ ] Switch back to the private login and start a session: its 5h bar continues from the value it had, with no reset and no "Learning" restart.
- [ ] Look at every new line at 26 (the narrowest supported pane), 60 and 144 columns in the dark and light themes: nothing is cut mid-word or misaligned.

## Outcome

If every line holds, nothing is committed.
Otherwise fix each defect in its owning task's files with a test that pins it, commit it on its own as `fix(<unit>): ...`, and repeat the check.
