# claude-plus - Design Spec

Date: 2026-10-10
Revision: 2 (panel moved from a tmux split to a native Claude Code mod pane)
Status: Approved in brainstorming, pending written-spec review

## 1. Purpose

Claude Code users lack visibility into what their sessions cost and why.
claude-plus is a local companion made of a Rust daemon and a native Claude Code mod that draws a live side pane inside Claude Code.
The pane shows token usage, granular context composition, cache state and expiry, what-if costs, and subscription limit consumption, in dollars and in percent of the subscription windows.
A history view and an insights engine help the user change habits that waste tokens (idle cache expiry, unused MCP servers and skills, oversized memory, large tool results, late compaction).

### Success criteria

- One install command (`claude-plus install`, after installing the binary) gives a working pane in every new Claude Code session, with no manual config editing.
- Per-request cost matches a hand computation from the transcript usage fields to the cent.
- Cache expiry countdown is correct within a few seconds of the true TTL.
- The pane never slows down, blocks or breaks Claude Code, including when the daemon is crashed, missing or not yet installed.
- claude-plus never edits `~/.claude/settings.json`, the status line or hooks.
- `claude-plus uninstall` removes the mod and stops the daemon, and optionally deletes `~/.claude-plus`.
- The pane looks and feels native to Claude Code and is delightful to keep open (section "Look and feel"), with no misalignment, truncation or flicker at any supported width or theme.

### Non-goals (v1)

- Desktop GUI app, tmux panel or embedded terminal emulator.
- Browser dashboard (the daemon API makes this possible later).
- Windows support.
- Multi-user or team aggregation.
- Any network access beyond the local Unix socket, telemetry or remote token counting.

### Audience and distribution

Personal use first, shareable later.
Targets macOS first and Linux second, on x86_64 and aarch64.
The binary ships via a Homebrew tap and a `curl | sh` installer, the mod ships via a plugin marketplace in the same repository.

## 2. Technology

- Daemon and CLI: Rust (stable), single binary, multiple subcommands.
- Async runtime: Tokio.
- HTTP over Unix socket: hyper (server side), JSON bodies with serde.
- Storage: SQLite via rusqlite (bundled feature), WAL mode, single writer.
- File watching: notify.
- Config: TOML at `~/.claude-plus/config.toml`.
- Mod: TypeScript/TSX hooks module against the Claude Code mod API (`claude-code` types, early access, verified against Claude Code 2.1.296).
- Tests: Rust built-in harness and insta snapshots for the daemon, `claude plugin test` for the mod, `claude plugin validate` in CI.

## 3. User experience

### Install and use

```
brew install <owner>/tap/claude-plus      # or: curl -fsSL <url>/install.sh | sh
claude-plus install                       # adds the marketplace, installs the mod, imports history
claude                                    # the pane appears beside the conversation
```

Subcommands:

| Command | Behavior |
|---|---|
| `claude-plus install` | Register the plugin marketplace and install the mod via the `claude plugin` CLI, start the daemon, kick off the history import. Idempotent. |
| `claude-plus uninstall [--purge]` | Uninstall the mod, stop the daemon, with `--purge` delete `~/.claude-plus`. |
| `claude-plus daemon [--ensure]` | Run the daemon in the foreground, or with `--ensure` start it detached if no live daemon answers on the socket, then return. |
| `claude-plus import` | Backfill all transcripts with progress output (also run by the daemon in the background). |
| `claude-plus stats [--days N]` | Print daily cost per model as a table, useful without the pane. |
| `claude-plus reindex` | Rebuild derived tables from stored raw records. |
| `claude-plus export --format json\|csv` | Export history. |
| `claude-plus doctor` | Diagnose binary, daemon, mod install, Claude Code version, transcript retention setting, parse error counts. |

### The pane

The mod opens a pane titled `claude-plus` at session start.
Claude Code docks it as a sidebar beside the transcript when the terminal is at least 144 columns wide (110 once the user has opened it before), otherwise it waits until the user opens it with the `/plus` command, which places it at any width.
The mod also sets a one-line status entry (`$.ui.status`) such as `$4.18 · cache 52:14 · 5h 54%`, visible even when the pane is not placed.

Four views, switched with single-key buttons `1` to `4` (the mod API allows one digit or lowercase letter per button hotkey), arrows scroll, Tab walks the elements, Esc returns to the prompt.
Values that are estimates are prefixed with `~`, and a `?` button on a row shows its derivation.

1. Live: model and effort, context bar (used / window, percent), cache state (warm or cold, TTL type, countdown, hit ratio), next-turn estimates (warm, cold, alternative model), session totals (cost, tokens in and out, turns, subagents), subscription bars for 5h, 7d and spend windows with reset time and pace forecast.
2. Context: drill-down of context composition from Claude Code's own breakdown (categories, memory files, MCP tools, auto-compact threshold), annotated with warnings from history (unused MCP server, never-invoked skills, large tool results). Rows expand with Enter.
3. History: time range 24h, 7d, 30d, 90d (`r` cycles), metric toggle `g` between dollars, tokens and percent of limit. Charts drawn as Raster cells (braille and block characters): daily cost stacked by model, cache hit ratio over time, 5h limit usage over time, top sessions and projects by cost, context growth curve of the current session with compaction markers.
4. Insights: ranked list of findings, each with evidence and estimated weekly saving in dollars and percent of limit.

When the daemon is unreachable or the binary is missing, the pane still shows everything the mod receives directly (context, limits, cost, per-request usage) and a one-line notice with the fix (`brew install ...` or `claude-plus daemon --ensure`).

### Look and feel

claude-plus must feel like a natural part of Claude Code and be pleasant to keep open all day, not only useful.
Delight comes from craft and calm motion, never from noise.

Visual language:

- Colors come from Claude Code's theme keys (`ThemeKey`), so the pane follows the user's light, dark or custom theme; Claude's terracotta accent is used for the brand mark, focused elements and the single most important number per view.
- Semantic colors are consistent everywhere: success for savings and warm cache, warning for approaching limits, error only for real problems, dim text for secondary detail.
- A small fixed glyph set echoing Claude Code's own: `✻` brand mark and cache heartbeat, `·` separators, `▸ ▾` disclosure, eighth-block characters (`▏▎▍▌▋▊▉█`) for smooth bars, braille cells for charts.
- Rounded box borders, generous but consistent padding, numbers right-aligned in columns, units always present, no line ever wraps mid-value.

Calm motion (all drawn with `$.ui.blit` or atom updates, never faster than the engine's limits):

- Cache heartbeat: the `✻` beside the countdown breathes slowly while the cache is warm, quickens in the last minute, and settles into a dim `·` when cold.
- The countdown bar drains continuously using eighth blocks instead of jumping.
- Changed numbers ease to their new value over about 300 ms instead of snapping.
- Motion pauses while Claude is working on long turns if it would distract, and `animations = false` in config turns all of it off.

Warm, encouraging microcopy:

- Status reads like a teammate, not a meter: "Cache is warm ✻ 52:14 left", "Cache went cold while you were away, rewarming will cost ~$0.62".
- Wins are celebrated: "Cache hits saved you $3.20 this session", "5 days in a row above 90% cache hit ratio".
- Insights are phrased as friendly tips with the payoff first ("Save ~$9/week: …"), never scolding.
- Empty and loading states have charm and say what is happening: "Learning how your plan counts tokens… a few more turns ✻", "Importing 1,284 sessions from your history…".
- A short recap toast greets the first session of the day: "Yesterday: $12.40 · 94% cache hits · biggest win: …".

Sound is off by default; an opt-in soft chime can mark the 60 s cache warning via `$.audio`.

Quality bar: every view is reviewed at 40, 60 and 100 columns in light and dark themes, and any misalignment, truncation, color clash or flicker is a bug.

### Alerts

Shown as a toast (`$.ui.toast`):

- Cache expires in 60 seconds.
- Pace forecast reaches 100% of the 5h window before reset.
- Context approaches the auto-compaction threshold.

## 4. Architecture

```
 Claude Code process
 ┌──────────────────────────────────────────────┐
 │  claude-plus mod (TS)                        │
 │   session.measure ─┐   turn.step / complete ─┤── POST /events ─────┐
 │   session.usage ───┤                         │                     │
 │   $.clock (1s) ────┴─▶ atoms ─▶ ui.render    │◀── GET /snapshot ───┤  HTTP over
 │                         (Pane, status, toast)│                     │  Unix socket
 └──────────────────────────────────────────────┘                     ▼
 ~/.claude/projects/**/*.jsonl ──watch/tail──▶ claude-plus daemon (Rust) ──▶ SQLite
 ~/.claude config files ───────────scan──────▶   (~/.claude-plus/data.db)
```

The mod is a thin display and forwarding layer, because the mod API is early access and may change between releases.
All computation (pricing, cache model, forecasts, limit calibration, insights, history) lives in the daemon, where it is tested in Rust.

### Units

Daemon (Rust modules, each with a narrow public interface, testable in isolation):

| Unit | Responsibility | Depends on |
|---|---|---|
| `paths` | Resolve `~/.claude` (honoring `CLAUDE_CONFIG_DIR`) and `~/.claude-plus` (honoring `CLAUDE_PLUS_HOME`). | none |
| `transcript` | Parse transcript JSONL lines into typed records, tolerant of unknown fields, keeping raw JSON. | none |
| `pricing` | Model price table (bundled plus config overrides), cost of a usage record. | none |
| `store` | SQLite schema, migrations, writes and queries, ingestion checkpoints. | transcript |
| `ingest` | Backfill and live tail of transcript files, resumable per file offset. | transcript, store |
| `config_scan` | Locate configured context sources on disk (CLAUDE.md chain, memory, skills, MCP servers, plugins) for dead-weight analysis. | paths |
| `context_model` | Context composition for history sessions from transcript attachments, and merge of the live breakdown pushed by the mod. | store |
| `cache_model` | Cache state, TTL, expiry, rebuild detection and cause attribution. | store |
| `forecast` | Next-turn warm, cold and alternative-model cost estimates. | pricing, cache_model, store |
| `limits` | Record rate-limit samples, detect resets, fit percent-per-dollar per window and model, pace forecast. | store |
| `insights` | Rule trait plus rule set, each rule returns findings with evidence and savings. | store, pricing, limits, context_model |
| `api` | HTTP routes over the Unix socket: `POST /events`, `GET /snapshot?session=`, `GET /history`, `GET /insights`, `GET /health`. | all core units |
| `daemon` | Lifecycle: single instance via socket probe and lock file, detach for `--ensure`, orchestration of ingest and scans. | api, ingest |
| `cli` | Subcommands from section 3, including `install` and `uninstall` driving the `claude plugin` CLI. | all |

Mod (TypeScript, files under `mod/`):

| File | Responsibility |
|---|---|
| `register.tsx` | Wire events: `session.start` opens the pane, registers `/plus`, starts the 1 s clock and ensures the daemon; `session.measure`, `turn.step`, `turn.complete`, `session.compact` forward to the daemon and update atoms. |
| `client.ts` | Daemon client over `$.http.fetch` with `socketPath`, short timeouts, never throws to callers. |
| `state.ts` | Atoms: latest snapshot, live measure, active view, daemon health. |
| `views/*.tsx` | One file per view (live, context, history, insights) plus shared widgets (bar, sparkline raster). |
| `format.ts` | Pure formatting of money, tokens, durations, percent. |

### Data sources

1. Mod events, pushed to the daemon:
   - `session.measure`: context usage (tokens, window, percent), rate limits (`five_hour`, `seven_day`, `spend_limit` with `percentUsed` and `resetsAt`), session cost. Fires after each main-thread turn and whenever a rate-limit window moves a whole point.
   - `$.session.usage({ breakdown: 'full' })`: Claude Code's own context breakdown (categories, memory files, MCP tools, auto-compact threshold, last API usage), requested after each turn.
   - `turn.step` result: per-request usage (`input_tokens`, `output_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens`, model) and effort, as soon as the request completes.
   - `turn.complete`, `session.compact`, `session.start`, `session.end`: lifecycle markers.
2. Transcripts, tailed by the daemon: `~/.claude/projects/<project>/<session>.jsonl` and `<project>/<session>/subagents/agent-<id>.jsonl`. They supply what the mod cannot see (the 5m and 1h split of cache writes, subagent usage, tool results, injected context attachments) and all history.
3. Config on disk, scanned by the daemon, for configured but never-loaded items.

The session id from `$.session.id()` is the transcript file name, which joins mod events to transcript records.
Mod events are the low-latency source for the Live view, and transcript records are the source of truth for money and history: when both describe the same request, the transcript record wins.

### Transcript facts verified on real data (Claude Code 2.1.296)

Claude Code documents the transcript format as internal and subject to change, so the parser treats every field as optional and every unknown record type as opaque.

- One API response is written as several `assistant` records, one per content block, each repeating `message.usage`. Input and cache fields are identical across them, but about 20% of responses show `output_tokens` growing from record to record as the response streams. Requests must be deduplicated on `requestId` (fallback `message.id`, then `uuid`) before any cost aggregation, keeping the record with the highest `output_tokens`.
- `message.usage.iterations` lists the API calls behind one response; it has more than one entry only for refusal fallbacks, where the top-level counts describe the final call. v1 uses the top-level counts.
- `user` records that are not human prompts carry `isMeta`, `isCompactSummary` or `isVisibleInTranscriptOnly`, or start with tooling markers such as `<task-notification`, `<command-`, `<local-command`, `<system-reminder`, `[Request interrupted`.
- `tool_result` content can contain base64 `image` blocks, which must be counted as images, never as text characters.
- `message.usage.cache_creation` splits cache writes into `ephemeral_5m_input_tokens` and `ephemeral_1h_input_tokens`, and `output_tokens_details.thinking_tokens` gives thinking tokens.
- `assistant` records carry `attributionSkill`, `attributionMcpServer`, `attributionMcpTool`, `attributionPlugin`, `effort`, `perTurnEffort` and, when limited, `quotaLimits` (`status`, `rateLimitType`, `resetsAt`).
- `attachment` records carry the actual injected context: `skill_listing` (content, names, skillCount), `instructions` (CLAUDE.md files with path, type and content), `nested_memory`, `mcp_instructions_delta`, `deferred_tools_record` and `deferred_tools_delta` (tool schemas), `agent_listing_delta`, `environment`, `invoked_skills`.
- `user` records carry `tool_result` blocks (`tool_use_id`, `content`, `is_error`), matched to `tool_use` blocks of assistant records for the tool name.
- `system` records with `subtype: compact_boundary` carry `compactMetadata` (`trigger`, `preTokens`, `postTokens`).
- `cost-state` records carry Claude Code's own cumulative per-session counters (`totalCostUSD`, and `modelUsage` per model with `inputTokens`, `outputTokens`, `cacheReadInputTokens`, `cacheCreationInputTokens`, `costUSD`); they price consistently with the bundled table within about 1%, and only some sessions have one.
- Subagent transcripts often keep only the preliminary streaming `output_tokens` (2 to 16) of a response: across sessions with subagents, transcripts hold about 47% of the output and 78% of the cache reads that `modelUsage` reports, while sessions without subagents match within 2%.
- The parent's `Agent` tool result carries `toolUseResult.usage`, which describes only the subagent's last call, so it cannot repair earlier calls.
- Workflow agents write to `<session>/subagents/workflows/<run>/agent-<id>.jsonl` beside a `journal.jsonl` that holds no usage.
- `<synthetic>` model records carry no billable usage and are skipped.
- Subagent records keep the parent `sessionId` and set `isSidechain: true`.
- Claude Code deletes transcripts after `cleanupPeriodDays` (default 30), so the daemon's own store is the long-term history, and `doctor` reports the setting.

### Daemon lifecycle

- The mod runs `claude-plus daemon --ensure` with `$.process.run` at session start (binary resolved from `userConfig.binaryPath`, default `claude-plus` on `PATH`), which returns at once after starting a detached daemon if none answers on the socket.
- A daemon started by the mod's `$.process.spawn` would die with the module on reload, which is why the daemon detaches itself instead.
- Single instance is enforced by an exclusive lock on `~/.claude-plus/daemon.lock` and a socket probe, so many concurrent sessions share one daemon.
- The daemon stays resident and idles cheaply; `claude-plus uninstall` stops it.

### Storage schema (initial)

- `raw_records(id, file_id, line_no, kind, json)` keeps every relevant parsed line for reindexing, with `tool_result` content replaced by its length to bound size.
- `files(id, path, offset, size, mtime, parse_errors)` holds ingestion checkpoints.
- `sessions(id, project, cwd, started_at, last_seen_at, title)`.
- `requests(key, session_id, ts, model, input, output, thinking, cache_read, cache_write_5m, cache_write_1h, cost_usd, is_sidechain, agent_id, skill, mcp_server, effort)`, `key` being the deduplication key.
- `tool_uses(id, session_id, ts, name, input_chars)` and `tool_results(tool_use_id, session_id, ts, chars, is_error)`.
- `attachments(session_id, ts, kind, chars, names)`.
- `compactions(session_id, ts, trigger, pre_tokens, post_tokens)`.
- `cost_states(session_id, ts, total_cost_usd, json)`.
- `adjustments(session_id, model, ts, input, output, cache_read, cache_write, cost_usd)` holds reconciliation rows.
- `limit_samples(ts, session_id, model, window, used_pct, resets_at)`.
- `live_breakdowns(session_id, ts, json)` holds the latest context breakdown pushed by the mod.
- `meta(key, value)` holds schema version, price table version and calibration parameters.

## 5. Computation

### Request cost (exact)

`cost = input * p_in + cache_write_5m * p_cw5 + cache_write_1h * p_cw1h + cache_read * p_cr + output * p_out`

- Prices come from the bundled table, keyed by model id (date suffixes such as `-20251001` stripped), overridable in `config.toml`.
- Cache writes are 1.25x base input for 5m and 2x for 1h; cache reads are a per-model rate (for example $0.20/MTok on Claude Opus 5.5, $0.10/MTok on Claude Sonnet 5.5, $0.25/MTok on Claude Fable 5.1, 0.1x base input on most older models), all taken from the published pricing page when the table is built and recorded with its source date.
- Tiered models (Claude Haiku 5.5 above 100K prompt tokens) use the tier selected by the request's total input.
- Thinking tokens are billed as output and already included in `output_tokens`.
- An unknown model shows "price unknown" and is excluded from dollar totals with a visible count, never guessed.
- Session, project and day aggregates are sums of deduplicated request costs, with subagent requests attributed to the parent session.
- Reconciliation: for each session with a `cost-state`, the per-model shortfall of transcript usage against `modelUsage` becomes adjustment rows, spread over that model's requests by hour, priced like requests (missing cache writes at the 5-minute rate), never negative.
- Adjustments count in session, day and total costs, are shown as "reconciled", and never alter request rows; sessions with subagents but no counter are reported as possibly understated, never extrapolated.
- The pricing module is cross-checked against `cost-state` records in tests.

### Cache state

- TTL type for a session is taken from the most recent cache write (`1h` when `ephemeral_1h_input_tokens > 0`, else `5m`).
- Expiry is `last_request_ts + ttl`, since every cache hit refreshes the TTL; the mod counts down locally from the snapshot's `expires_at` with a 1 s clock.
- Hit ratio is `cache_read / (cache_read + cache_write + input)` over the session.
- A rebuild is detected when a request has a large cache write and near-zero cache read relative to the previous request's prefix.
- Cause attribution: idle gap above TTL gives "idle", a model change gives "model switch", a change of an injected context attachment between the two requests gives "prefix changed", otherwise "unknown".

### Next-turn forecast (estimate)

- Prefix size `P` is the previous request's cache read plus cache write.
- New input `N` and output `O` are the medians of the last 10 turns of this session.
- `p_cw` is the cache write price matching the session's TTL type (`p_cw5` or `p_cw1h`).
- Warm: `P * p_cr + N * p_cw + O * p_out`.
- Cold: `(P + N) * p_cw + O * p_out`.
- Alternative model: first turn is priced cold at that model's prices (cache is per model), following turns warm, both displayed.

### Subscription percent (learned estimate)

- Each `session.measure` stores a `limit_samples` row per window.
- A drop in used percent, or a passed `resetsAt`, marks a reset.
- Between consecutive samples within a window, pair the percent delta with the API-equivalent dollars spent in that interval across all sessions.
- Fit `pct_per_usd` per window and model family with a robust estimator (median of ratios, outliers trimmed), stored in `meta`.
- Until at least 20 valid pairs exist the UI shows "calibrating".
- Pace forecast extrapolates the current window's burn rate over the last 30 minutes to the reset time.

### Context composition

- Live session: Claude Code's own breakdown pushed by the mod is shown as is (exact).
- History sessions: injected text comes from transcript attachments, tokens are estimated from text length with a per-kind chars-per-token ratio (prose, code, JSON schema), then all fixed components are scaled so they sum to the first request's measured `input + cache_read + cache_write` minus the estimated first user message.
- The per-kind ratios are calibrated against live breakdowns whenever both exist for the same session.

## 6. Insights

Each rule implements a common trait: `fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding>`.
A finding has a title, evidence lines, an estimated weekly saving in dollars and percent of 5h limit, and a confidence.
Findings are ranked by saving.

Initial rules:

1. Idle cache rebuilds: cost of rebuilds caused by idle gaps, with the typical gap length.
2. Unused MCP servers and tools: tokens per request versus invocation count (`attributionMcpServer`, tool names) over 14 and 30 days.
3. Unused skills: listing tokens versus invocation count (`attributionSkill`, `invoked_skills`).
4. Oversized memory and CLAUDE.md: token size, duplicated lines across files.
5. Large tool results: files and commands that most often inject the most tokens.
6. Late compaction: sessions that stayed above a context threshold for many turns.
7. Model and effort fit: short low-output tasks run on a top-tier model, repriced on a cheaper model.
8. Mid-session cache busts: model switches, context attachment changes, MCP reconnects.
9. Cost-per-task outliers: prompt-to-stop units far above the user's median.
10. Transcript retention: `cleanupPeriodDays` shorter than the history the user wants, suggesting a higher value.

## 7. Error handling and safety

- The mod never throws out of a hook: every daemon call has a 500 ms timeout and failures only set a "daemon unavailable" state.
- Mod hooks stay far below the 10 s hook budget; long-lived work runs on `$.clock` timers started in `session.start`.
- Forwarding to the daemon is fire-and-forget from the mod's point of view, and the pane renders from the last good snapshot.
- The daemon parser never panics on input, unknown record types are stored raw, unparsable lines are counted per file and reported by `doctor`.
- Ingestion is resumable from per-file byte offsets, handles a truncated last line still being written (not counted as an error, re-read later), file truncation or replacement (re-read from 0, deduplication prevents double counting) and non-UTF-8 bytes.
- Backfill streams files and commits in batches, so memory stays bounded on gigabytes of transcripts.
- The socket and data directory are created with user-only permissions (0700 directory, 0600 files).
- No network access anywhere in the daemon; the mod only calls the local socket.

## 8. Testing

- Daemon unit tests per unit: pricing math, deduplication, cache expiry and rebuild detection, forecast, limits estimator and reset detection, context scaling, each insight rule.
- Daemon fixture tests: a corpus of anonymized transcripts covering subagents, compaction, model switches, 5m and 1h caches, malformed and truncated lines, with hand-computed expected costs, plus a cross-check against `cost-state` totals.
- API tests: the daemon started on a temporary socket, exercised with HTTP requests matching what the mod sends.
- Mod tests with `claude plugin test`: views mounted with `$.ui.mount` against mocked snapshots at 40, 60 and 100 columns, hotkeys switching views, countdown and heartbeat progression with `mock.clock`, eased number transitions, `animations = false`, degraded rendering when the daemon client fails.
- Visual review: each view is checked by eye in a real session in light and dark themes at each milestone, and visual defects are fixed before the milestone closes.
- End-to-end: a real `claude` session with the mod loaded via `--plugin-dir` against a running daemon, checked by hand against `/context` and `/usage`, once per milestone.
- CI on GitHub Actions for macOS and Linux runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `claude plugin validate`, `claude plugin test`, and builds release binaries for four targets.

## 9. Delivery milestones

Each milestone gets its own implementation plan and produces working, tested software.

1. Core: paths, transcript parser, pricing, store, ingest with backfill, `import`, `stats`, `doctor`.
2. Daemon and API: lifecycle with `--ensure`, live tail, HTTP over Unix socket, event intake from the mod.
3. Mod skeleton and Live view: pane, status entry, `/plus`, forwarding, cache model, forecast, limits sampling.
4. Context view: live breakdown, history composition, config scan.
5. History view with raster charts.
6. Insights view and alerts.
7. Packaging: `install` and `uninstall`, release CI, Homebrew tap, install script, plugin marketplace.
