# claude-plus - Design Spec

Date: 2026-10-10
Status: Approved in brainstorming, pending written-spec review

## 1. Purpose

Claude Code users lack visibility into what their sessions cost and why.
claude-plus is a local terminal companion that runs Claude Code in a tmux split with a live side panel.
The panel shows token usage, granular context composition, cache state and expiry, what-if costs, and subscription limit consumption, in dollars and in percent of the subscription windows.
A history view and an insights engine help the user change habits that waste tokens (idle cache expiry, unused MCP servers and skills, oversized memory, large tool results, late compaction).

### Success criteria

- One install command and one launch command (`claude-plus`) give a working Claude pane plus panel, with no manual config editing.
- Per-request cost matches a hand computation from the transcript usage fields to the cent.
- Cache expiry countdown is correct within a few seconds of the true TTL.
- The panel never slows down, blocks or breaks Claude Code, including when claude-plus itself is crashed or not running.
- The user's existing status line keeps rendering exactly as before.
- `claude-plus uninstall` restores `~/.claude/settings.json` byte-for-byte to its pre-install state when it was not modified in between, and otherwise removes only claude-plus entries.

### Non-goals (v1)

- Desktop GUI app or embedded terminal emulator.
- Browser dashboard (the daemon API makes this possible later).
- Windows support.
- Multi-user or team aggregation.
- Any network access, telemetry or remote token counting.

### Audience and distribution

Personal use first, shareable later.
Targets macOS first and Linux second, on x86_64 and aarch64.
Distributed as a single static-ish binary via a Homebrew tap and a `curl | sh` installer.

## 2. Technology

- Language: Rust (stable), single binary, multiple subcommands.
- TUI: Ratatui with the crossterm backend.
- Async runtime: Tokio.
- Storage: SQLite via rusqlite (bundled feature), WAL mode, single writer.
- File watching: notify.
- IPC: Unix domain socket at `~/.claude-plus/daemon.sock`, newline-delimited JSON messages.
- Config: TOML at `~/.claude-plus/config.toml`.
- Tests: built-in test harness, insta for snapshot tests, a tmux-driven end-to-end suite.

## 3. User experience

### Install and launch

```
brew install <owner>/tap/claude-plus      # or: curl -fsSL <url>/install.sh | sh
claude-plus                               # launch
```

Subcommands:

| Command | Behavior |
|---|---|
| `claude-plus [-- <claude args>]` | Ensure setup, ensure daemon, open Claude plus panel in tmux. Arguments after `--` (and unknown arguments) pass through to `claude`. |
| `claude-plus setup` | Idempotent install of status line wrapper and hooks into `~/.claude/settings.json`, with backup. |
| `claude-plus uninstall` | Remove all claude-plus entries, restore the original status line command. |
| `claude-plus panel [--pane <id>]` | Run the panel TUI only (used inside the tmux split, or standalone). |
| `claude-plus daemon` | Run the background daemon in the foreground (normally auto-spawned). |
| `claude-plus statusline` | Status line wrapper invoked by Claude Code. |
| `claude-plus hook <event>` | Hook entrypoint invoked by Claude Code. |
| `claude-plus reindex` | Rebuild derived tables from stored raw records. |
| `claude-plus export --format json\|csv` | Export history. |
| `claude-plus doctor` | Diagnose setup, daemon, tmux, parse error counts. |

Launch behavior:

- Outside tmux: create a new tmux session `claude-plus-<n>` with the Claude pane on the left and the panel on the right, then attach.
- Inside tmux: split the current window horizontally, Claude stays in the current pane, panel opens to the right.
- Default panel width is 60 columns, configurable, resizable with normal tmux bindings.
- If tmux is missing, print the install command for the platform (`brew install tmux`, `apt install tmux`, ...) and offer to run it.
- `setup` optionally adds a tmux binding (default `prefix + C`) to toggle a panel next to any pane running Claude.

### Panel tabs

Switch with `1`-`4` or `Tab`.
Layout adapts to 40, 60 and 100+ column widths.
Values that are estimates are prefixed with `~`, and `?` on a row shows its derivation.

1. Live: model and effort, context bar (used / window, percent), cache state (warm or cold, TTL type, countdown, hit ratio), next-turn estimates (warm, cold, alternative model), session totals (cost, tokens in and out, turns, subagents), subscription bars for 5h and 7d windows with reset time and pace forecast.
2. Context: drill-down tree of context composition: system prompt and built-in tools, MCP tools per server, skills listing, memory and CLAUDE.md files, conversation split into tool results, user messages and assistant replies. Rows carry warnings (unused server, never-invoked skills, large tool results).
3. History: time range 24h, 7d, 30d, 90d (left and right arrows), metric toggle `g` between dollars, tokens and percent of limit. Charts: daily cost stacked by model, cache hit ratio over time, 5h limit usage over time, top sessions and projects by cost, context growth curve of a selected session with compaction markers.
4. Insights: ranked list of findings, each with evidence and estimated weekly saving in dollars and percent of limit.

### Alerts

Shown as a transient banner in the panel, and optionally as an OS notification (`osascript` on macOS, `notify-send` on Linux):

- Cache expires in 60 seconds.
- Pace forecast reaches 100% of the 5h window before reset.
- Context approaches the auto-compaction threshold.

## 4. Architecture

```
 Claude Code ──statusline JSON──▶ claude-plus statusline ──▶ (prints user's original status line)
     │                                   │ fire-and-forget
     ├──hook JSON──▶ claude-plus hook ───┤
     │                                   ▼
     └──writes──▶ ~/.claude/projects/**/*.jsonl ──watch──▶ daemon ──▶ SQLite (~/.claude-plus/data.db)
                                                            ▲  │
                         ~/.claude config files ──scan──────┘  │ socket: subscribe / query
                                                               ▼
                                                     claude-plus panel (one per Claude pane)
```

### Units

Each unit is a Rust module with a narrow public interface and is testable in isolation.

| Unit | Responsibility | Depends on |
|---|---|---|
| `transcript` | Parse transcript JSONL lines into typed records, tolerant of unknown fields, keeping raw JSON. | none |
| `pricing` | Model price table (bundled plus config overrides), cost of a usage record. | none |
| `store` | SQLite schema, migrations, writes and queries, ingestion checkpoints. | transcript |
| `ingest` | Backfill and live tail of transcript files, resumable per file offset. | transcript, store |
| `config_scan` | Locate and measure context sources: CLAUDE.md chain, MEMORY.md and memory files, skills listings, MCP server configs and plugins. | none |
| `context_model` | Estimate context composition and scale it to the measured real total. | config_scan, store |
| `cache_model` | Cache state, TTL, expiry, rebuild detection and cause attribution. | store |
| `forecast` | Next-turn warm, cold and alternative-model cost estimates. | pricing, cache_model, store |
| `limits` | Record rate-limit samples, detect resets, fit percent-per-dollar per window and model, pace forecast. | store |
| `insights` | Rule trait plus rule set, each rule returns findings with evidence and savings. | store, pricing, limits, context_model |
| `daemon` | Socket server, event intake, orchestration of ingest and scans, push of live snapshots. | all core units |
| `ipc` | Message types and client used by statusline, hook and panel. | none |
| `statusline` | Wrapper: forward input to daemon, run the user's original command with identical stdin, print its output. | ipc |
| `hooks` | Hook entrypoint: forward event plus `$TMUX_PANE` to daemon, always exit 0 quickly. | ipc |
| `setup` | Edit `settings.json` safely, back up, install and uninstall, tmux binding. | none |
| `launcher` | tmux session and split orchestration, daemon spawn. | setup, ipc |
| `tui` | Ratatui app with the four tabs, rendering from snapshots. | ipc |

### Data sources

1. Status line input JSON (per turn): model id and display name, context window usage and size, session cost, effort level, rate limits (`five_hour` and `seven_day` used percentage and reset time), session id, transcript path, workspace.
2. Hook events: SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, PreCompact, Stop, SubagentStop, SessionEnd. Each carries session id and transcript path. The hook process adds `$TMUX_PANE`, which binds a session to a tmux pane.
3. Transcripts: `~/.claude/projects/<project>/<session>.jsonl` and subagent sidechain files. Assistant records carry `message.model`, `message.usage` (input, output, cache read, cache creation split into `ephemeral_5m_input_tokens` and `ephemeral_1h_input_tokens`, thinking tokens), timestamp, request id. User records carry tool results.
4. Config on disk: `~/.claude/CLAUDE.md`, project CLAUDE.md chain, memory directories, `~/.claude/settings.json`, plugin and skill directories, MCP server definitions.

Exact field names are verified against real payloads during implementation, and the parser treats every field as optional.

### Transcript facts verified on real data (Claude Code 2.1.296)

- One API response is written as several `assistant` records, one per content block, each repeating the identical `message.usage`. Requests must be deduplicated on `requestId` (fallback `message.id`) before any cost aggregation.
- `assistant` records carry `attributionSkill`, `attributionMcpServer`, `attributionMcpTool`, `attributionPlugin`, `effort`, `perTurnEffort` and, when limited, `quotaLimits` (`status`, `rateLimitType`, `resetsAt`).
- `attachment` records carry the actual injected context: `skill_listing` (content, names, skillCount), `instructions` (CLAUDE.md files with path, type and content), `nested_memory`, `mcp_instructions_delta`, `deferred_tools_record` and `deferred_tools_delta` (tool schemas), `agent_listing_delta`, `environment`, `invoked_skills`.
- `system` records with `subtype: compact_boundary` carry `compactMetadata` (`trigger`, `preTokens`, `postTokens`).
- `cost-state` records carry Claude Code's own per-model cost totals, used as a cross-check of the pricing module.
- `<synthetic>` model records carry no billable usage and are skipped.
- Subagent transcripts live in `<project>/<session-id>/subagents/agent-<id>.jsonl` and are attributed to the parent session.

Consequently `context_model` prefers the transcript attachments as the source of truth for what was injected, and `config_scan` is only a fallback for sessions whose transcripts predate those attachments, and for the dead-weight analysis of configured but never-loaded items.

### Pane to session binding

The hook process reads `$TMUX_PANE` and sends it with the session id.
The panel is launched with `--pane <claude pane id>` and subscribes to snapshots of whichever session is currently bound to that pane.
A new session started in the same pane (for example `/clear`) rebinds automatically.
A panel launched without `--pane` follows the most recently active session and offers a picker (`s`).

### Storage schema (initial)

- `raw_records(id, file_id, line_no, json)` keeps every parsed line for reindexing.
- `files(id, path, offset, mtime, parse_errors)` holds ingestion checkpoints.
- `sessions(id, project, cwd, started_at, ended_at, parent_session_id, pane_id)`.
- `requests(id, session_id, ts, model, input, output, thinking, cache_read, cache_write_5m, cache_write_1h, cost_usd, is_sidechain)`, deduplicated on request id.
- `tool_results(id, session_id, ts, tool_name, input_summary, chars, est_tokens)`.
- `limit_samples(ts, session_id, model, window, used_pct, resets_at)`.
- `events(ts, session_id, kind, payload)` for hook events.
- `context_snapshots(session_id, ts, component, est_tokens)`.
- `meta(key, value)` holds schema version and calibration parameters.

## 5. Computation

### Request cost (exact)

`cost = input * p_in + cache_write_5m * p_cw5 + cache_write_1h * p_cw1h + cache_read * p_cr + output * p_out`

Prices come from the bundled table, keyed by model id, overridable in `config.toml`.
An unknown model shows "price unknown" and is excluded from dollar totals with a visible count, never guessed.
Session, project and day aggregates are sums of deduplicated request costs, with subagent requests attributed to the parent session.
The pricing module is cross-checked against `cost-state` records in tests.

### Cache state

- TTL type for a session is taken from the most recent cache write (`1h` when `ephemeral_1h_input_tokens > 0`, else `5m`).
- Expiry is `last_request_ts + ttl`, since every cache hit refreshes the TTL.
- Hit ratio is `cache_read / (cache_read + cache_write + input)` over the session.
- A rebuild is detected when a request has a large cache write and near-zero cache read relative to the previous request's prefix.
- Cause attribution: idle gap above TTL gives "idle", a model change gives "model switch", a change of a context source file between the two requests gives "prefix changed", otherwise "unknown".

### Next-turn forecast (estimate)

- Prefix size `P` is the previous request's cache read plus cache write.
- New input `N` and output `O` are the medians of the last 10 turns of this session.
- `p_cw` is the cache write price matching the session's TTL type (`p_cw5` or `p_cw1h`).
- Warm: `P * p_cr + N * p_cw + O * p_out`.
- Cold: `(P + N) * p_cw + O * p_out`.
- Alternative model: first turn is priced cold at that model's prices (cache is per model), following turns warm, both displayed.

### Subscription percent (learned estimate)

- Each status line update stores a `limit_samples` row per window.
- A drop in used percent marks a reset.
- Between consecutive samples within a window, pair the percent delta with the API-equivalent dollars spent in that interval.
- Fit `pct_per_usd` per window and model family with a robust regression (median of ratios, outliers trimmed), stored in `meta`.
- Until at least 20 valid pairs exist the UI shows "calibrating".
- Pace forecast extrapolates the current window's burn rate over the last 30 minutes to the reset time.

### Context composition (estimate scaled to measured total)

- For each source, take its injected text from the transcript attachments (fallback: files on disk) and estimate tokens from text length with a per-kind chars-per-token ratio (prose, code, JSON schema).
- Measured fixed total is the first request of the session's `input + cache_read + cache_write`, minus the estimated first user message.
- Scale all fixed-component estimates by `measured / estimated_sum`.
- The conversation component is tracked from per-turn real deltas and split by tool result sizes and message sizes.
- Drift between estimate and measurement is shown in the Context tab.

## 6. Insights

Each rule implements a common trait: `fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding>`.
A finding has a title, evidence lines, an estimated weekly saving in dollars and percent of 5h limit, and a confidence.
Findings are ranked by saving.

Initial rules:

1. Idle cache rebuilds: cost of rebuilds caused by idle gaps, with the typical gap length.
2. Unused MCP servers and tools: tokens per request versus invocation count over 14 and 30 days.
3. Unused skills: listing tokens versus invocation count.
4. Oversized memory and CLAUDE.md: token size, duplicated lines across files.
5. Large tool results: files and commands that most often inject the most tokens.
6. Late compaction: sessions that stayed above a context threshold for many turns.
7. Model and effort fit: short low-output tasks run on a top-tier model, repriced on a cheaper model.
8. Mid-session cache busts: model switches, context source edits, MCP reconnects.
9. Cost-per-task outliers: prompt-to-stop units far above the user's median.

## 7. Error handling and safety

- Statusline and hook processes connect to the daemon with a 50 ms budget and never fail: on any error they continue silently, and the statusline still prints the original command's output.
- The statusline wrapper passes the original stdin bytes unchanged to the user's original status line command and propagates its stdout.
- If the daemon is not running, hooks do nothing and the launcher spawns it on next `claude-plus` run.
- The panel process is isolated in its own pane, its crash does not affect Claude.
- The transcript parser never panics on input, unknown record types are stored raw, unparsable lines are counted per file and reported by `doctor`.
- Ingestion is resumable from per-file byte offsets, handles truncated last lines and file rotation.
- `setup` writes `settings.json` atomically (write temp, fsync, rename), keeps a timestamped backup, preserves unrelated keys and formatting as much as practical, and records the original status line command in `~/.claude-plus/state.json`.
- All data lives under `~/.claude-plus`, with no network access anywhere in the binary.

## 8. Testing

- Unit tests per unit: pricing math, cache expiry and rebuild detection, forecast, limits regression and reset detection, context scaling, each insight rule.
- Fixture tests: a corpus of anonymized transcripts covering subagents, compaction, model switches, 5m and 1h caches, malformed lines, with hand-computed expected costs.
- Snapshot tests: each tab rendered with Ratatui's test backend at 40, 60 and 100 columns, compared with insta snapshots.
- End-to-end: a scripted tmux session runs `claude-plus` against a fake `claude` binary that writes fixture transcript lines and invokes the statusline and hooks like Claude Code does, then asserts panel content via `tmux capture-pane`, countdown progression, and that setup followed by uninstall leaves `settings.json` identical.
- CI on GitHub Actions for macOS and Linux runs fmt, clippy with warnings denied, tests, and builds release binaries for four targets.

## 9. Delivery milestones

1. Core: transcript parser, pricing, store, ingest with backfill, `doctor`.
2. Live path: daemon, IPC, statusline wrapper, hooks, setup and uninstall, launcher with tmux.
3. Live tab: cache model, forecast, limits sampling.
4. Context tab: config scan and context model.
5. History tab with charts.
6. Insights tab and alerts.
7. Packaging: release CI, Homebrew tap, install script.
