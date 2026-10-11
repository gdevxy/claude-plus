# Morning check for the claude-plus History view

Run this by hand in a real terminal, after Tasks 1 to 9 of the M5 plan are on `main`.
Nothing here writes to `~/.claude/dev-mods`; `--plugin-dir` loads the mod for one session.

## Setup

Build the daemon from the working tree and make sure the running one is this build.
The schema does not change in M5, so `--ensure` would keep an older daemon running; stop it first, after the check of "An older daemon kept running" below if an M4 daemon runs.
Expected: a line starting with `✻`, then `claude-plus doctor` reports a healthy daemon.

```bash
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo build --release --locked
./target/release/claude-plus daemon stop
./target/release/claude-plus daemon --ensure
./target/release/claude-plus doctor
```

Start Claude Code with the mod in a project you have used on several days this week.
Putting `target/release` first on `PATH` makes the default `binaryPath` (`claude-plus`) find this build.

```bash
PATH="$PWD/target/release:$PATH" claude --plugin-dir "$PWD/mod"
```

Check each line below in the `dark` theme, then again after `/theme` to `light`, and once in `dark-ansi`.
The lines marked daltonized are checked again in `dark-daltonized` and `light-daltonized`.
Check at 40, 60 and 100 columns of pane width (resize the terminal; the pane docks from 144 columns, `/plus` places it at any width).
Keep `claude-plus stats --days 7` at hand in another shell: it is the reference for every cost in the view.

## Unverified API assumptions (look at these first)

- [ ] A `Raster` draws its cells as the colors the plan's palette gives: in `dark` the Opus bars are the same orange as `✻ claude-plus`, and the hit ratio bars the same green as the Live view's cache bar.
  Note any chart whose colors differ from the text drawn in the same theme key next to it.
- [ ] `/theme` goes through `config.set` with the key `theme`: right after switching from `dark` to `light`, the charts change color without waiting 30 s.
  If they change only after up to 30 s, the theme is read on the next poll only; note it.
- [ ] `$.config.list()` reports the theme by the names `dark`, `light`, `dark-daltonized`, `light-daltonized`, `dark-ansi` and `light-ansi`.
  In `dark-ansi` and `auto` the charts are drawn as colored text and look the same as the cells, without gaps between rows.
- [ ] The hotkeys `r` and `g` answer while the pane holds the focus (ctrl+x tab or a click), and never type into the composer.

## An older daemon kept running

Check this first when an M4 daemon is still running, before the `daemon stop` of Setup.
`--ensure` keeps a daemon of the same version, so after an update the M4 daemon keeps serving the new pane.

- [ ] Build as in Setup, skip `daemon stop`, start Claude Code with the mod as in Setup, then press `3`.
  Expected: the view reads `Last 7 days`, then `This claude-plus daemon is older than the pane and has no History` and `Run claude-plus daemon stop to restart it`, both dim; it never stays on `Loading`.
- [ ] Run `./target/release/claude-plus daemon stop` in another shell.
  Expected: `The daemon is not answering` shows for at most 30 s, the pane starts the new daemon, and the History view draws its charts without a key press.

## Checklist

- [ ] The header reads `✻ claude-plus` and `1: Live 2: Context 3: History` at 60 and 100 columns, and `✻` alone with the tabs at 40 columns, the active view bright and the others dim.
- [ ] `3` opens the History view; `r: 7 days  g: Cost` sits under the header; `1` and `2` still switch to Live and Context.
- [ ] The first open shows `Loading the last 7 days… ✻` for at most a moment, then `Last 7 days` with the total cost, which matches the Total of `claude-plus stats --days 7`.
- [ ] The dim line under the headline counts sessions and requests plausibly, and requests on an unknown model add a dim `not priced` line.
- [ ] "Cost by day" has seven bars, today on the right above `today`, the first day's label under the first bar, and the bar heights follow the daily totals of `stats --days 7`.
- [ ] Each bar is stacked by model in the colors of the legend under it, the largest model at the bottom, and a day with any spend at all shows at least a sliver.
- [ ] (daltonized) In all four color themes, every model swatch of the legend and `Other` can be told from its neighbours, and from the bars' pieces next to it; `Other` is a dim grey.
  Real data with five or more models shows all six series; use the 90 days range if the week has fewer.
- [ ] Token amounts of a billion or more read like `18.4B` with the Tokens metric, never `18408M`, and session and request counts are grouped like `14,577`.
- [ ] `peak $x.xx` on the right of "Cost by day" is the tallest bar's value.
- [ ] `r` steps through `24 hours`, `30 days`, `90 days` and back to `7 days`; each shows its own chart at once or after `Loading the last ...`, and `24 hours` reads "Cost by hour" with `now` under the last bar.
- [ ] At 40 columns `90 days` reads "Cost per 3 days" and its bars fill the width with no row cut or wrapped.
- [ ] `g` steps through `Tokens`, `% of 5h` (only once the 5h limit is calibrated) and back to `Cost`; the headline, the bars, the legend and the top lists change units, and the order of the top lists stays.
- [ ] With `% of 5h`, every value has a `~`, the headline has a `?`, and its explanation names how many limit readings it learned from.
- [ ] "Cache hit ratio" matches what the Live view reported for today's sessions, and its `?` says where the bars start.
- [ ] "Saved by cache hits" and "Rebuilds" are present and plausible.
- [ ] "5h limit, highest" appears only when the daemon has 5h readings in the range, and bars at 80% or more are amber.
- [ ] "Top projects" names each project by its folder (with its parent folder when two share a name), and "Top sessions" shows titles, with `Untitled` and the id start for a session without one.
- [ ] "Context this session" draws a curve that rises with the session and drops at a `┊` marker after a compaction, with `<tokens> / <window>` matching `/context`.
- [ ] Every right-aligned value ends in the same column; no value is cut, wrapped or overlapping the `?`, at 40, 60 and 100 columns.
- [ ] Charts look the same in `dark-ansi` (text) as in `dark` (cells), save for the colors of the terminal's palette.
- [ ] Leaving the History view open for a minute neither flickers nor slows the composer; the cache countdown in the status line keeps ticking every second.
- [ ] `./target/release/claude-plus daemon stop` makes the view show `The daemon is not answering` and how to start it again, and the view keeps its last charts.
- [ ] While History builds a `90 days` report in this session, the Live view of another Claude Code session never shows `The daemon is not answering`.
- [ ] After `daemon --ensure` again, the notice goes away within 30 s.
- [ ] `./target/release/claude-plus export --format csv --days 7` prints one row per day, project and model, and the sum of `cost_usd` matches the Total of `stats --days 7`.
- [ ] `./target/release/claude-plus export --format json` prints `{"v":1,"rows":[...]}` that `python3 -m json.tool` accepts.
- [ ] Every `cost_usd` in the JSON has at most 6 decimals and equals the CSV's value for the same row.
- [ ] `CLAUDE_CONFIG_DIR=/tmp/nowhere ./target/release/claude-plus export --format csv > /tmp/empty.csv` prints the `No Claude Code transcripts found` notice on the terminal, and `/tmp/empty.csv` holds the header row alone.
- [ ] The Live and Context views still behave as in the M3 and M4 checks.

## Outcome

If every line holds, nothing is committed.
Otherwise fix each defect in its owning task's files with a test that pins it, commit it on its own as `fix(mod): ...` or `fix(<unit>): ...`, and repeat the check.
