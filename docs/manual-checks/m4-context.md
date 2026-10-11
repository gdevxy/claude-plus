# Morning check for the claude-plus Context view

Run this by hand in a real terminal, after Tasks 1 to 10 of the M4 plan are on `main`.
Nothing here writes to `~/.claude/dev-mods`; `--plugin-dir` loads the mod for one session.

## Setup

Build the daemon from the working tree and make sure the running one is this build.
The store moves to schema 14, so `--ensure` replaces a daemon that still runs an older build.
Schema 14 rebuilds the derived tables once, so the first minute may show `Rebuilding the claude-plus index…`.
Expected: a line starting with `✻`, then `claude-plus doctor` reports schema 14.

```bash
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo build --release --locked
./target/release/claude-plus daemon --ensure
./target/release/claude-plus doctor
```

Start Claude Code with the mod in a project that has a `CLAUDE.md`, at least one MCP server and a few skills.
Putting `target/release` first on `PATH` makes the default `binaryPath` (`claude-plus`) find this build.

```bash
PATH="$PWD/target/release:$PATH" claude --plugin-dir "$PWD/mod"
```

Check each line below in a dark theme, then again after `/theme` to a light theme.
Check at 40, 60 and 100 columns of pane width (resize the terminal; the pane docks from 144 columns, `/plus` places it at any width).
Keep `/context` at hand: it is the reference for every number in the view.

## Unverified API assumptions (look at these first)

- [x] Claude Code names its categories `Memory files`, `Skills`, `MCP tools` and `Custom agents`.
  Those rows show `▸` when they have items; note the exact name of any category that lists items in `/context` but shows no `▸`.
- [x] The paths in the breakdown's memory files are the paths the scan finds.
  Under "Largest items" a CLAUDE.md file reads `CLAUDE.md · Project` or `CLAUDE.md · User`, not a full path.
- [x] Skill names in the breakdown match the `attributionSkill` of transcript records (for example `superpowers:brainstorming`).
  Invoke one listed skill, wait a minute, and confirm it leaves "Not used in 14 days".
- [x] A skill whose folder name differs from its frontmatter `name` (for example the folder `spring-boot-4-migration-skill` with `name: migrating-to-spring-boot-4`) is listed under its folder name, as Claude Code's own skill listing names it.
  Use it once and confirm it leaves "Not used in 14 days"; note whether the breakdown names it by its folder or its frontmatter name.
- [x] A full breakdown after a turn does not slow the prompt down; the composer stays responsive right after each answer.

## Checklist

- [x] The header reads `✻ claude-plus` on the left and `1: Live 2: Context` on the right, the active view bright and the other dim.
- [x] With the pane focused (ctrl+x tab or a click), `2` opens the Context view and `1` returns to Live; the tabs answer a click too.
- [x] Before the first prompt of a new session the view shows `Context` and `The breakdown appears after your first prompt ✻`, with no stray blank line.
- [x] After a prompt the headline reads `Context · <model>` with used and window matching `/context`, and the bar's percent matches it.
- [x] `Auto-compact at <threshold> · <left> to go` matches `/context`'s threshold, and turns amber as `due now` once the context passes it.
- [x] Every category under "By category" matches `/context` to the shown precision; `Free space` and `Autocompact buffer` are dim and have no bar.
- [x] When tools load on demand, `+ <n> of tool schemas load on demand` sits under the categories, dim.
- [x] Each bar's color matches the color of its category in `/context`'s grid, or is plain text where `/context` uses a color outside the theme.
- [x] Tab reaches `▸ Memory files`; Enter turns it into `▾` and lists the files `/context` lists, with the same tokens; Enter closes it again.
- [x] `▸ MCP tools` opens to one line per server with its tool count; `▸ Skills` opens to the listed skills, at most 8 and then `+ N more`.
- [x] "Largest items" lists the six largest items, largest first, with the kind in front (`MCP`, `Skill`, `Agent`, `Style`) except for CLAUDE.md files.
- [x] "Not used in 14 days" lists only skills and MCP servers you have not used in 14 days, with `<tokens> a turn` on the right, and never a built-in skill such as `init`.
  With more than four, a dim `+ N more` row ends the list with what the rest cost.
- [x] After Claude reads a large file, "Largest tool results" names `Read` with a `~` value; an MCP tool reads as `<server> <tool>`, a plugin's MCP tool without its `plugin_<name>_` prefix.
- [x] Results a subagent read never show under "Largest tool results", and after `/compact` the list starts over with what came after it.
- [x] "Configured" counts match what you have: CLAUDE.md files (with `on demand` for nested ones), skills, MCP servers (`/mcp`) and plugins (`off` for disabled ones).
- [x] Each `?` (Tab to it, then Enter) opens its dim explanation under its section and closes it again, and the times in it ("counted 2m ago", "Read from disk <1m ago") are plausible.
- [x] Every right-aligned value ends in the same column; no value is cut, wrapped or overlapping the `?` or a `▸`, at 40, 60 and 100 columns.
- [x] Resume a session that ran before this build (`claude --resume` with the same `--plugin-dir`) and open the Context view before prompting.
  It shows "Estimated from the transcript" with `~` values and `~<n> injected since your first request` when attachments came later; after the first prompt it switches to "By category".
  If the session's last breakdown is older than five minutes, the heading reads `By category · <age> ago` until the next one.
  An open row in the estimate starts with a dim `Configured now` line, the sum of what is on disk today.
- [x] `./target/release/claude-plus daemon stop` makes the view show `The claude-plus daemon is stopped · run /plus to start it` (two rows when narrow) within 3 s, and the view keeps its last numbers; after the next prompt the headline and "By category" still follow Claude Code.
- [x] After `daemon --ensure` again, the notice goes away within 3 s.
- [x] From a shell where `claude-plus` is not on `PATH` and no daemon runs, the Context view shows `claude-plus is not installed yet. Install it, or set its path in the plugin options` and, after a prompt, still shows the headline and "By category".
- [x] `/clear` empties the view, which fills again after the next prompt, for the new session only.
- [x] Switching views while Claude works does not flicker, and the Live view still behaves as in the M3 check.
- [x] Closing the pane (ctrl+x x) stops the breakdowns: with the pane closed, answer two prompts a minute apart and confirm the latest breakdown time does not move.
  Reopen it with `/plus` and confirm a new breakdown arrives within a minute of the next prompt.

  ```bash
  sqlite3 "file:$HOME/.claude-plus/data.db?mode=ro" "SELECT session_id, datetime(ts_ms / 1000, 'unixepoch', 'localtime') FROM live_breakdowns ORDER BY ts_ms DESC LIMIT 1"
  ```

- [x] `/context` answers quickly on a busy store: while Claude works through a long task, time it a few times with this session's id (`/status` shows it).
  Every call takes under 0.2 s.

  ```bash
  for i in 1 2 3 4 5; do curl -s -o /dev/null -w '%{time_total}\n' --unix-socket ~/.claude-plus/daemon.sock "http://claude-plus/context?session=<id>"; sleep 2; done
  ```

- [x] `claude-plus doctor` still reports a healthy daemon and schema 14.

## Outcome

If every line holds, nothing is committed.
Otherwise fix each defect in its owning task's files with a test that pins it, commit it on its own as `fix(mod): ...` or `fix(<unit>): ...`, and repeat the check.
