# Morning check for the claude-plus live pane

Run this by hand in a real terminal, after Tasks 1 to 10 are on `main`.
Nothing here writes to `~/.claude/dev-mods`; `--plugin-dir` loads the mod for one session.

## Setup

Build the working tree, stop any daemon an earlier build left running, and start this one.
Expected: `✻ Started the claude-plus daemon · pid N`.

```bash
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo build --release --locked
./target/release/claude-plus daemon stop
./target/release/claude-plus daemon --ensure
```

Start Claude Code with the mod.
Putting `target/release` first on `PATH` makes the default `binaryPath` (`claude-plus`) find this build.

```bash
PATH="$PWD/target/release:$PATH" claude --plugin-dir "$PWD/mod"
```

Check each line below in a dark theme, then again after `/theme` to a light theme.
Check at 40, 60 and 100 columns of pane width (resize the terminal; the pane docks from 144 columns, `/plus` places it at any width).

## Unverified API assumptions (look at these first)

- [ ] The window kinds are really `five_hour`, `seven_day` and `spend_limit`.
  The pane shows `5h`, `7d` and `Spend`; any other name appears raw, so note the raw kind if you see one.
- [ ] The pane never opens unasked on the main screen.
  With a fresh plugin store (the first session with the mod, or after moving the plus mod's store file aside; it lies under your Claude Code configuration directory), start a session in a main-screen terminal (tmux, or `CLAUDE_CODE_NO_FLICKER=0`) at least 144 columns wide: the pane stays closed.
  Run `/plus`: it answers `✻ claude-plus is open above the prompt`.
  Restart the session and confirm the pane stays closed until you run `/plus` again.
  Then run `/plus` in a fullscreen session, and start the next session on the main screen: the pane does not flash inline, not even for one frame.
- [ ] Plugin options reach a `--plugin-dir` mod.
  Try `echo '{"animations":"false"}' | claude plugin configure plus --values-stdin`, or Claude Code's plugin settings, then restart the session.
  If neither reaches the mod, note it, because the mod installed in M7 is the one users configure.

## Checklist

- [ ] The pane opens at start in a terminal of at least 144 columns, titled `claude-plus`, with `✻ claude-plus` in terracotta and `Live` on the right.
- [ ] `/plus` opens it at any width and answers `✻ claude-plus is open beside the conversation` in the fullscreen layout.
- [ ] The status entry reads like `$4.18 · cache 52:14 · 5h 54%` and its countdown moves every second.
- [ ] Before the first prompt the pane shows `Context shows after your first prompt` and `Cache starts with your first prompt ✻`, with no stray blank line.
- [ ] After a prompt, model and effort match the model picker, and the cost matches `/cost` within a cent.
- [ ] The context amount and percent match `/context`.
- [ ] The 5h and 7d percents and reset times match `/usage`.
- [ ] The cache countdown restarts after each request, reads `(1h)` on a subscription, and the `✻` breathes slowly (dim, normal, bold, normal) without flicker.
- [ ] After a long response (ask for a long essay with extended thinking, and time it), the countdown counts from when you sent the prompt, not from when the answer ended.
  Note the time you pressed Enter: the countdown must read the TTL minus the time since then, within a few seconds.
- [ ] With less than a minute left the countdown and `✻` turn amber and quicken.
- [ ] After expiry the line reads `Cache · cold (1h) ... expired` and the cold note names the rewarm cost, on one line when wide and as `Rewarming will cost ~$…` on its own row when narrow.
- [ ] While Claude is working, the `✻` holds still.
- [ ] Every right-aligned value ends in the same column; no value is cut, wrapped or overlapping the `?`, at 40, 60 and 100 columns.
- [ ] Bars fill smoothly with eighth blocks, the empty track is dim, and no color clashes with either theme.
- [ ] Pressing each `?` (Tab to it, then Enter) opens its dim explanation under its section and closes it again.
- [ ] The cost and the percents ease to new values over about 300 ms after each turn, with no flicker.
- [ ] With the `animations` option off (then a restart of the session), the heartbeat holds still and numbers jump; the countdown still ticks.
- [ ] `./target/release/claude-plus daemon stop` makes the pane show `The daemon is not answering · run claude-plus daemon --ensure` (two rows when narrow) within 3 s.
  The prompt stays responsive, and context, limits and cost keep updating from Claude Code alone: send a prompt and confirm the cost and the 5h percent move in the pane and in the status line.
  Within about 30 s the pane runs `--ensure` itself and the notice goes away.
- [ ] After `daemon --ensure` again, the notice goes away within 3 s and the cache and forecast come back.
- [ ] While the daemon rebuilds its index, the pane keeps its numbers and shows `Rebuilding the claude-plus index… numbers return in a moment`, never zeros.
- [ ] From a shell where `claude-plus` is not on `PATH` and no daemon runs, the pane shows `claude-plus is not installed yet. Install it, or set its path in the plugin options` and still shows context, limits and cost.
- [ ] `/clear` empties the session numbers and the pane follows the new session.
- [ ] A subagent run (for example a codebase search with the Explore agent) does not change the main countdown.
- [ ] A subagent that compacts its own context, or announces its own tools, neither hides a rebuild of the main thread nor explains one: `Rebuilds` counts the same rebuilds for this session as the History view's `Rebuilds` over the same time.
- [ ] When the `?` of `Rebuilds` names a changed context, it reads `because the MCP tools, CLAUDE.md, skills or agents changed`; a rebuild within the cache lifetime with no such change reads `because no cause could be told from the transcript`.
- [ ] Freeze the daemon, send a short prompt within 5 s, then let the daemon go, and count the stored events.
  The pane's posts time out while the daemon is frozen and are sent again, yet each event must be stored once: the two counts must be equal.

  ```bash
  pid=$(curl -s --unix-socket ~/.claude-plus/daemon.sock http://claude-plus/health | sed -E 's/.*"pid":([0-9]+).*/\1/')
  kill -STOP "$pid"
  # send the prompt in Claude Code now, then within 5 s:
  kill -CONT "$pid"
  sqlite3 ~/.claude-plus/data.db "SELECT count(*), count(DISTINCT session_id || client_id) FROM events WHERE client_id IS NOT NULL"
  ```
- [ ] Reload the mod (save a file under `mod/`, and accept Claude Code's reload prompt if it shows one).
  The pane keeps its numbers, does not open again if you had closed it, and `sqlite3 ~/.claude-plus/data.db "SELECT count(*) FROM events WHERE kind = 'session.start'"` does not grow.
- [ ] After a subagent run, the `agentId` the mod forwarded matches the subagent's transcript file name (`agent-<id>.jsonl`).

  ```bash
  sqlite3 ~/.claude-plus/data.db "SELECT DISTINCT json_extract(json, '$.data.agentId') FROM events WHERE kind = 'turn.step'"
  ls ~/.claude/projects/*/*/subagents/
  ```
- [ ] `claude-plus doctor` still reports a healthy daemon and schema 11.

## Outcome

If every line holds, nothing is committed.
Otherwise fix each defect in its owning task's files with a test that pins it, commit it on its own as `fix(mod): ...` or `fix(<unit>): ...`, and repeat the check.
