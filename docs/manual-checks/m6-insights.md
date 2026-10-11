# Morning check for the claude-plus Insights view and alerts

Run this by hand in a real terminal, after Tasks 1 to 11 of the M6 plan are on `main`.
Nothing here writes to `~/.claude/dev-mods`; `--plugin-dir` loads the mod for one session.

## Setup

Build the daemon from the working tree and make sure the running one is this build.
Stop the running daemon first, so the new one opens the store and moves it to schema 16, after the check of "An older daemon kept running" below if an M5 daemon runs.
Expected: a line starting with `✻`, then `claude-plus doctor` reports a running daemon and no `run claude-plus import to upgrade` line.

```bash
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo build --release --locked
./target/release/claude-plus daemon stop
./target/release/claude-plus daemon --ensure
./target/release/claude-plus doctor
```

The migration asks for a rebuild of the derived tables, so the first minute may show `Rebuilding the claude-plus index…`.
Wait until `doctor` no longer prints `Reindex: running in the daemon`.

Start Claude Code with the mod in a project you have used on several days over the last two weeks.
Putting `target/release` first on `PATH` makes the default `binaryPath` (`claude-plus`) find this build.

```bash
PATH="$PWD/target/release:$PATH" claude --plugin-dir "$PWD/mod"
```

Check each line below in the `dark` theme, then again after `/theme` to `light`, and once in `dark-ansi`.
Check at 40, 60 and 100 columns of pane width (resize the terminal; the pane docks from 144 columns, `/plus` places it at any width).
Keep `claude-plus stats --days 14` at hand in another shell: it is the reference for the requests the tips are built from.

## Unverified API assumptions (look at these first)

- [x] `$.ui.toast` shows the recap and the alerts at the top right of the transcript for about 8 s, and a click takes one off.
  While the pane holds the focus, note whether toasts still show (the pane is opened without `holdToasts`, so they should).
- [x] `session.end` with the reason `clear` fires before the new session starts: right after `/clear` a toast reads `That session: $x.xx ...` for the session that just ended.
- [x] `$.session.usage({ breakdown: 'summary' })` answers with `context.breakdown.autoCompactThreshold` without sending a request (no new entry in `/cost` or `stats`), and only once per session.
- [x] `$.audio.play({ asset: 'sounds/chime.wav' })` plays the chime through `afplay` on macOS once the `chime` option is on, and plays nothing with it off.
- [x] The hotkey `4` answers while the pane holds the focus, and never types into the composer.
- [x] The host wraps evidence lines at spaces only, so the no-break space (U+00A0) the daemon puts between a value and its unit keeps `3 rebuilds`, `2h 48m` or `Opus 5.5` on one row, and it renders as a plain space.

## An older daemon kept running

Check this first when an M5 daemon is still running, before the `daemon stop` of Setup.
`--ensure` keeps a daemon of the same version, so after an update the M5 daemon keeps serving the new pane.

- [x] Build as in Setup, skip `daemon stop`, start Claude Code with the mod as in Setup, then press `4`.
  Expected: the view reads `Insights`, then `This claude-plus daemon is older than the pane and has no Insights` and `Run claude-plus daemon stop, then /plus`, both dim; it never stays on `Looking for ways to save…`, and no recap toast shows.
- [x] Run `./target/release/claude-plus daemon stop` in another shell, then `/plus`.
  Expected: the pane shows that the daemon is stopped until `/plus`, which starts the new daemon, and the Insights view draws its tips within a minute without a key press.

## Checklist

- [x] The header reads `✻ claude-plus` and `1: Live 2: Context 3: History 4: Insights` at 60 and 100 columns.
- [x] At 40 columns it reads `✻` and `1: Live 2: Ctx 3: Hist 4: Tips` on one row with nothing cut, and the labels stay in place when you switch views.
- [x] `4` opens the Insights view; `1`, `2` and `3` still switch to the other views.
- [x] The first open shows `Looking for ways to save… ✻` for at most a moment, then `Insights` with `up to ~$x.xx/week` in the accent color and a `?`.
- [x] The `?` beside the total says the savings are weekly estimates from the last 14 days and that tips can overlap.
- [x] The dim line under the headline counts the tips and, once the limit is calibrated, adds `~n% of a 5h window a week`, or `~n% of 5h/wk` where that does not fit.
- [x] The wins read `✻ Cache hits saved you $x.xx this week` and, after two or more days above 90%, `✻ n days in a row above 90% cache hits`, in the success color.
- [x] Tips are ranked by their weekly saving, highest first, and tips without a saving (costly tasks, retention) come last.
- [x] Each priced tip starts with `Save ~$x.xx/week:` in the success color, then the tip in plain words; a tip without a saving starts with a capital letter.
- [x] Evidence lines are dim, indented by two, and wrap between words without splitting a value from its unit.
- [x] Token amounts in the evidence read like `~287k`, `1.2M` or `18.4B`, never `18408M`, and counts of a thousand or more are grouped like `1,284`.
- [x] Each tip ends with a dim line naming its share of a 5h window a week (or of 5h/wk, when calibrated) and its confidence, with a `?` that opens how the tip was worked out.
- [x] The idle rebuild tip, if shown, agrees with the rebuilds of the History view over the last two weeks, and its typical break is plausible.
  Its weekly saving is less than half the cost in its first evidence line, since that cost covers two weeks and a fresh start still writes the smallest context of each session.
- [x] A cache bust tip blames MCP only when a session's MCP tools changed between two requests; a bust with no cause in the transcripts reads `n rewrite(s) mid-session with no cause in the transcripts` with no saving and low confidence.
- [x] The late compaction tip, if shown, reads `run /compact once a session passes ...k tokens`, its evidence names sessions that really ran above that size, and its `?` says the saving is net of the `/compact` request and the rewrite after it.
- [x] The unused MCP server and skill tips name only things the Context view lists under `Not used in 14 days`.
- [x] The memory tip, if shown, names files you load in every session; edit a CLAUDE.md to repeat a long line from another one, wait a minute, and the tip counts it.
- [x] The large tool results tip lists files relative to the project and commands by program and first argument (`Bash git diff`, never a whole command line), a long path cut at its head with `…`, and its tip names the costliest file or program.
- [x] The retention tip appears while `cleanupPeriodDays` is under 90 in `~/.claude/settings.json` (or unset), and its `?` says what to add and that only that file was read; do not edit the file for this check.
  Its first evidence line says what that file holds, `cleanupPeriodDays is n in ~/.claude/settings.json` or `~/.claude/settings.json leaves cleanupPeriodDays at its default of 30`.
- [x] In a project with no requests in the last 14 days the view says `Insights need a few sessions of history ✻`.
- [x] Every right-aligned value ends in the same column; no row is cut or overlaps a `?`, at 40, 60 and 100 columns.
- [x] Leaving the Insights view open for two minutes neither flickers nor slows the composer; the cache countdown keeps ticking every second.
- [x] `./target/release/claude-plus daemon stop` makes the view show `The claude-plus daemon is stopped · run /plus to start it` (two rows when narrow), the daemon stays stopped, and the view keeps its last tips.
- [x] After `daemon --ensure` again, the notice goes away within a minute.
- [x] While the Insights view builds its first report of this session, the Live view of another Claude Code session never shows `The daemon is not answering`.

## Recap and alerts

- [x] Start the first session of a day: within a few seconds a toast reads `Yesterday: $x.xx · nn% cache hits · biggest win: ...`, or the date of your last working day instead of `Yesterday`.
  Its cost matches that day's row in `claude-plus export --format csv --days 2` summed by `cost_usd`, reconciled rows included, and the day's cost in `claude-plus stats`.
- [x] Right after the schema 16 upgrade of Setup, the first session shows the recap once the rebuild finishes, without a restart.
- [x] Start a second session the same day: no recap toast.
- [x] Send a prompt, then wait without typing until the countdown in the status line reaches `0:59`: a toast reads `Cache goes cold in under a minute ✻ rewarming later costs ~$x.xx`, once.
- [x] Turn the `chime` option on in the plugin's options, repeat the wait: the chime plays with the toast, softly and briefly.
- [x] Turn the `alerts` option off, repeat the wait: no toast.
- [x] In a long session, once context passes about 90% of the auto-compact threshold that the Context view shows, a toast reads `Context is close to auto-compact, ...k of ...k ✻ /compact now to choose what stays`, once until context drops under 80%.
- [x] When the Live view shows `full in ... at this pace` for the 5h window, a toast says the same once for that window.
- [x] An alert never replaces the recap or another alert: it waits until the toast on screen is gone, and two alerts due together show as one toast joined by ` · `.
- [x] After `/model` to a model with another context window, the context alert compares against the new auto-compact threshold.
- [x] After the context alert fired, `/model` to a model with a larger window and then context passing 90% of its threshold shows the alert again.
- [x] `/clear` shows `That session: $x.xx · cache hits saved you $x.xx ✻` for the session that ended.
- [x] A `claude -p "say hi"` run with the mod loaded shows no recap and no alert.
- [x] The Live, Context and History views still behave as in the M3, M4 and M5 checks.

## Outcome

If every line holds, nothing is committed.
Otherwise fix each defect in its owning task's files with a test that pins it, commit it on its own as `fix(mod): ...` or `fix(<unit>): ...`, and repeat the check.
