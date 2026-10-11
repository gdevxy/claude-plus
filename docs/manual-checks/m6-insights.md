# Morning check for the claude-plus Insights view and alerts

Run this by hand in a real terminal, after Tasks 1 to 11 of the M6 plan are on `main`.
Nothing here writes to `~/.claude/dev-mods`; `--plugin-dir` loads the mod for one session.

## Setup

Build the daemon from the working tree and make sure the running one is this build.
Stop the running daemon first, so the new one opens the store and moves it to schema 15, after the check of "An older daemon kept running" below if an M5 daemon runs.
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

- [ ] `$.ui.toast` shows the recap and the alerts at the top right of the transcript for about 8 s, and a click takes one off.
  While the pane holds the focus, note whether toasts still show (the pane is opened without `holdToasts`, so they should).
- [ ] `session.end` with the reason `clear` fires before the new session starts: right after `/clear` a toast reads `That session: $x.xx ...` for the session that just ended.
- [ ] `$.session.usage({ breakdown: 'summary' })` answers with `context.breakdown.autoCompactThreshold` without sending a request (no new entry in `/cost` or `stats`), and only once per session.
- [ ] `$.audio.play({ asset: 'sounds/chime.wav' })` plays the chime through `afplay` on macOS once the `chime` option is on, and plays nothing with it off.
- [ ] The hotkey `4` answers while the pane holds the focus, and never types into the composer.

## An older daemon kept running

Check this first when an M5 daemon is still running, before the `daemon stop` of Setup.
`--ensure` keeps a daemon of the same version, so after an update the M5 daemon keeps serving the new pane.

- [ ] Build as in Setup, skip `daemon stop`, start Claude Code with the mod as in Setup, then press `4`.
  Expected: the view reads `Insights`, then `This claude-plus daemon is older than the pane and has no Insights` and `Run claude-plus daemon stop to restart it`, both dim; it never stays on `Looking for ways to save…`, and no recap toast shows.
- [ ] Run `./target/release/claude-plus daemon stop` in another shell.
  Expected: `The daemon is not answering` shows for a moment, the pane starts the new daemon, and the Insights view draws its tips within a minute without a key press.

## Checklist

- [ ] The header reads `✻ claude-plus` and `1: Live 2: Context 3: History 4: Insights` at 60 and 100 columns.
- [ ] At 40 columns it reads `✻` and `1: Live 2: Ctx 3: Hist 4: Tips`, the active view keeping its full name (`4: Insights` once it is open), on one row with nothing cut.
- [ ] `4` opens the Insights view; `1`, `2` and `3` still switch to the other views.
- [ ] The first open shows `Looking for ways to save… ✻` for at most a moment, then `Insights` with `up to ~$x.xx/week` in the accent color and a `?`.
- [ ] The `?` beside the total says the savings are weekly estimates from the last 14 days and that tips can overlap.
- [ ] The dim line under the headline counts the tips, and at 60 columns and wider adds the share of a 5h window once the limit is calibrated.
- [ ] The wins read `✻ Cache hits saved you $x.xx this week` and, after two or more days above 90%, `✻ n days in a row above 90% cache hits`, in the success color.
- [ ] Tips are ranked by their weekly saving, highest first, and tips without a saving (costly tasks, retention) come last.
- [ ] Each priced tip starts with `Save ~$x.xx/week:` in the success color, then the tip in plain words; a tip without a saving starts with a capital letter.
- [ ] Evidence lines are dim, indented by two, and wrap between words without cutting a value.
- [ ] Token amounts in the evidence read like `~287k`, `1.2M` or `18.4B`, never `18408M`, and counts of a thousand or more are grouped like `1,284`.
- [ ] Each tip ends with a dim line naming its share of 5h (when calibrated) and its confidence, with a `?` that opens how the tip was worked out.
- [ ] The idle rebuild tip, if shown, agrees with the rebuilds of the History view over the last two weeks, and its typical break is plausible.
- [ ] The unused MCP server and skill tips name only things the Context view lists under `Not used in 14 days`.
- [ ] The memory tip, if shown, names files you load in every session; edit a CLAUDE.md to repeat a long line from another one, wait a minute, and the tip counts it.
- [ ] The large tool results tip names files and commands relative to the project, and a long command is cut with `…`.
- [ ] The retention tip appears while `cleanupPeriodDays` is under 90 in `~/.claude/settings.json` (or unset), and its `?` says what to add; do not edit the file for this check.
- [ ] In a project with no requests in the last 14 days the view says `Insights need a few sessions of history ✻`.
- [ ] Every right-aligned value ends in the same column; no row is cut or overlaps a `?`, at 40, 60 and 100 columns.
- [ ] Leaving the Insights view open for two minutes neither flickers nor slows the composer; the cache countdown keeps ticking every second.
- [ ] `./target/release/claude-plus daemon stop` makes the view show `The daemon is not answering` and how to start it again, and the view keeps its last tips.
- [ ] After `daemon --ensure` again, the notice goes away within a minute.
- [ ] While the Insights view builds its first report of this session, the Live view of another Claude Code session never shows `The daemon is not answering`.

## Recap and alerts

- [ ] Start the first session of a day: within a few seconds a toast reads `Yesterday: $x.xx · nn% cache hits · biggest win: ...`, or the date of your last working day instead of `Yesterday`.
  Its cost matches that day's row in `claude-plus export --format csv --days 2` summed by `cost_usd`.
- [ ] Start a second session the same day: no recap toast.
- [ ] Send a prompt, then wait without typing until the countdown in the status line reaches `0:59`: a toast reads `Cache goes cold in under a minute ✻ rewarming later costs ~$x.xx`, once.
- [ ] Turn the `chime` option on in the plugin's options, repeat the wait: the chime plays with the toast, softly and briefly.
- [ ] Turn the `alerts` option off, repeat the wait: no toast.
- [ ] In a long session, once context passes about 90% of the auto-compact threshold that the Context view shows, a toast reads `Context is close to auto-compact, ...k of ...k ✻ /compact now to choose what stays`, once until context drops under 80%.
- [ ] When the Live view shows `full in ... at this pace` for the 5h window, a toast says the same once for that window.
- [ ] `/clear` shows `That session: $x.xx · cache hits saved you $x.xx ✻` for the session that ended.
- [ ] A `claude -p "say hi"` run with the mod loaded shows no recap and no alert.
- [ ] The Live, Context and History views still behave as in the M3, M4 and M5 checks.

## Outcome

If every line holds, nothing is committed.
Otherwise fix each defect in its owning task's files with a test that pins it, commit it on its own as `fix(mod): ...` or `fix(<unit>): ...`, and repeat the check.
