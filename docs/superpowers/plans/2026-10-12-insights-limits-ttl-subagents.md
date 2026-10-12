# claude-plus Insights: Limit Hits, Cache Lifetime and Subagent Models Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Insights gains three rules: a 5-hour or weekly limit that ran out (shown first), the cache lifetime that costs less for the main thread and for subagents, and subagents that ran on a top-tier model, repriced on the next model down by agent type.

**Architecture:** Each rule is a new `Rule` in its own file of `src/insights` (`hits.rs`, `ttl.rs`, `subagents.rs`), fed by `InsightContext` like the ten rules of M6.
Two new inputs reach the context: the account's `limit_samples` of the last 14 days, already stored since M5, and each subagent's type, which the store did not keep.
Schema v19 adds an `agents` table that ingest fills from the `agent-<id>.meta.json` file Claude Code writes beside each subagent transcript.
The mod renders findings generically, so it only learns the three new rule ids.

**Tech Stack:** Rust (edition 2024, MSRV 1.88), no new crates; the mod in TypeScript, checked with `claude plugin validate mod`, `claude plugin test mod` and `npx --offline -p typescript@6.0.3 tsc -p mod-dev/tsconfig.json`.
Node and npx are at `/opt/homebrew/bin`, cargo at `/opt/homebrew/opt/rustup/bin` and `$HOME/.cargo/bin`.

**Spec:** `docs/superpowers/specs/2026-10-10-claude-plus-design.md`, section 6 "Insights" (the `Rule` trait, findings ranked by saving) and section 5 "Cache state" and "Subscription percent" (TTL, limit samples), extended by the Decisions below.

This plan starts from `main` at `5a552cb`.
Every line of Rust and TypeScript below was run on a scratch worktree of that commit before the plan was written, then replayed literally, task by task, on a second clean worktree, where each step failed and passed as it says here; and the final state passed `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo +1.88 check --all-targets`, `claude plugin validate mod`, `claude plugin test mod` (three runs) and `tsc`.
The lib tests go from 570 to 586 and the mod tests from 459 to 460; the cli, daemon and other suites keep 71, 29, 18 and 31.
The report was also run on a copy of the real store (36,000 requests over 14 days): it answers in about 0.36 s warm, as before.
There the subagent rule leads with `run general-purpose subagents on Sonnet 5.5`, about $458 a week at low confidence, with subagents drawing 81% of the spend.
The cache lifetime rule stays silent there, which is right: the main thread's 1-hour premium was $44.52 against $415 of rewrites the 5-minute cache would have cost, and subagents broke even ($300 of avoidable rebuilds against a $308 premium).
No limit ran out in the two days of samples that store holds, so the limit rule is pinned by its tests.
If `main` has moved, apply the hunks by the code they quote, not by line number.

## Facts this plan relies on

Checked on 2026-10-12 against https://code.claude.com/docs/en/prompt-caching and https://code.claude.com/docs/en/sub-agents, and on real transcripts of Claude Code 2.1.296.

- The main thread's cache lifetime is set by `promptCacheTtl` in settings or `CLAUDE_CODE_PROMPT_CACHE_TTL`, and that of subagents, compaction and titles by `subagentPromptCacheTtl` or `CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL`, each `5m` or `1h`, from Claude Code 2.1.242.
- On a Claude plan within its included usage the main thread defaults to 1 hour; with an API key, a cloud provider or usage credits it defaults to 5 minutes; subagents default to 5 minutes everywhere.
- A subagent's model comes from the call's `model`, then the `model:` of its agent file, then `CLAUDE_CODE_SUBAGENT_MODEL`, then the main model; a user agent named `Explore` replaces the built-in one.
- Each subagent transcript `agent-<id>.jsonl`, under `subagents/` or `subagents/workflows/<run>/`, has a sibling `agent-<id>.meta.json` holding `agentType` (all 200 of the newest on this machine do), along with a free-text `description` this plan does not store.
- `limit_samples.used_pct` is reported in whole percents, and a window that ran out reads 100.

## Decisions

1. A limit ran out when a sample of a `five_hour` or `seven_day` instance (as `limits::instances` splits them) reached 100% at or after the start of the 14-day window.
   The wait runs from that sample to the reset (`resets_at_ms` of the instance's last sample that has one), or to now when the reset is still ahead; an instance with no reset time is skipped.
   Other windows, such as a per-model weekly window, are skipped.
2. Limit hits read only the samples of the requesting session's account, since limits are per account; a session without an account gets none, as it gets no 5h share.
3. The limit finding has no dollar saving and medium confidence, and it ranks before every other finding, because a limit that ran out stopped work and a dollar total cannot say that.
   Its evidence gives, per window, how often it ran out and the median wait; the model that drew most of the spend inside those windows before they ran out; and the share subagents drew.
   Its tip points at subagents when they drew at least half of that spend on a top-tier model, at the cheaper model when the main thread did, and at pacing otherwise.
4. The cache lifetime rule replays the 14 days per thread (the main thread of each session, and each subagent apart), in two groups that Claude Code configures apart: the main thread and subagents.
   A group's current lifetime is the one most of its cache-write tokens used.
   From 1 hour to 5 minutes, the saving is the 1-hour premium on every 1-hour write less what each turn sent 5 to 60 minutes after the one before would pay to rewrite the prefix it read.
   From 5 minutes to 1 hour, the saving is what the rebuilds after 5 to 60-minute pauses cost above a warm read, less the 1-hour premium on every other 5-minute write.
   A pair of turns is replayed only with the same model, a prefix of at least 2,048 tokens (`MIN_REBUILD_PREFIX`) and, on the main thread, no compaction between them.
   The finding adds up the groups whose switch saves money, names the larger one in the tip and has medium confidence, since the replay assumes the same turn timing.
5. This rule can overlap idle rebuilds (rule 1): both count rebuilds after a 5 to 60-minute pause on a 5-minute cache, and the Insights total already says tips can overlap.
   The cache lifetime switch for subagents can also overlap subagent model fit (Decision 6), since both price the same subagent requests; their savings do not add up.
6. The subagent rule reprices every subagent request on Opus, Fable or Mythos at the same tokens on `forecast::alternative_for` (the model the forecast already offers), grouped by agent type.
   The tip names the type that would save most; it is medium confidence for Claude Code's read-only agents (`Explore`, `Plan`, `claude-code-guide`, `statusline-setup`) and low otherwise, since a cheaper model may do worse on code.
   A subagent whose meta file is missing or unreadable is grouped as `untyped`, and an untyped lead gives `run subagents on <model>`.
   Main-thread requests stay with model fit (rule 7), so the two never price the same request.
7. The store keeps only `agentType`, trimmed and cut to 64 characters, never the free-text `description`.
   The `agents` table is primary data, like `raw_records`: it is not a derived table, so a reindex keeps it.
   Ingest reads the meta file before the unchanged-file shortcut, so the first scan after the upgrade fills every known agent, and a meta file written after its transcript is read on the next pass.
   An agent's type is written once and never replaced.
8. The three rule ids are `subagent_model_fit`, `cache_ttl` and `limit_hits`, declared after `model_fit`, `cache_busts` and `cache_ttl` in `RuleId` so equal savings keep a stable order.
   `INSIGHTS_VERSION` stays 1: the report gains values, not fields.

## Global Constraints

- Never use the em dash; use `-` or rephrase. Long Markdown files keep one sentence per line.
- No code comments unless needed; match the surrounding style: no `unwrap` outside tests, `BTreeMap` for stable order, money through `words::money`, values joined to their unit with U+00A0 through `words::count`, `words::tokens`, `words::model_name`.
- Tips are lower case (the mod capitalizes them); evidence lines start with a capital.
- No new crates; Rust edition 2024, MSRV 1.88.
- The Rust gate after every task: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`.
- Lint warnings, test failures and flaky tests are defects, also when a task did not cause them.
- Commit messages carry no AI co-author line.
- Reviews of each task and of the branch are done by a front-tier model (Opus).

## Review Focus

- A store upgraded from v18 holds no agent types: the first ingest pass must fill them although no transcript changed (Task 1 test `a_subagent_type_is_read_from_its_meta_file_once_it_exists`, second run).
- A meta file that is malformed, missing, or names a 100-character type must never fail ingest or bloat the store (Task 1 tests).
- A subscription user whose main thread is on the 1-hour cache and who pauses often must not be told to switch (Task 2 test `pauses_that_cost_more_than_the_premium_keep_the_one_hour_cache`, and the real store above).
- A limit sample of another account, or of a session with no account, must never surface (Task 4 test `the_report_reads_the_limits_of_the_requesting_session_account_only`).
- A limit still out when the report runs must count its wait until now, not until a reset in the future (Task 4 test `a_limit_still_out_counts_its_wait_until_now`).

---

### Task 1: Keep each subagent's type in the store

**Files:**
- Modify: `src/store/schema.rs` (migration v19)
- Modify: `src/store/writer.rs` (`AgentRow`, `Writer::agent`)
- Modify: `src/store/insights.rs` (`agent_types_between`, `knows_agent`, tests)
- Modify: `src/store/mod.rs` (schema version tests 18 to 19)
- Modify: `src/api/tests.rs:176` (health schema 18 to 19)
- Modify: `src/ingest/mod.rs` (read `agent-<id>.meta.json`, tests)

**Interfaces:**
- Produces: `pub struct AgentRow { pub agent_id: String, pub session_id: String, pub agent_type: String }` in `crate::store`.
- Produces: `Writer::agent(&mut self, row: &AgentRow) -> Result<()>`.
- Produces: `Store::agent_types_between(&self, since_ts: i64, until_ts: i64) -> Result<BTreeMap<String, String>>` (agent id to type, for agents with a request in the span) and `Store::knows_agent(&self, agent_id: &str) -> Result<bool>`.
- Produces: `crate::ingest::AGENT_TYPE_CHARS: usize = 64`.

- [ ] **Step 1: Write the failing store test**

In `src/store/insights.rs`, replace the test imports:

```rust
    use crate::store::{AgentRow, AttachmentRow, PromptRow, RequestRow, ToolResultRow, ToolUseRow};
    use crate::transcript::TokenCounts;
```

and add at the end of `mod tests`:

```rust
    #[test]
    fn agent_types_are_read_for_the_subagents_that_spent_in_a_span() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        for (agent_id, agent_type) in [("a1", "Explore"), ("a2", "reviewer"), ("a3", "Plan")] {
            w.agent(&AgentRow {
                agent_id: agent_id.into(),
                session_id: "s1".into(),
                agent_type: agent_type.into(),
            })
            .unwrap();
        }
        for (key, ts, agent_id) in [
            ("r1", 100, Some("a1")),
            ("r2", 110, Some("a1")),
            ("r3", 120, Some("a2")),
            ("r4", 130, None),
            ("r5", 300, Some("a3")),
            ("r6", 140, Some("a4")),
        ] {
            w.request(&RequestRow {
                key: key.into(),
                session_id: "s1".into(),
                ts,
                sent_ts: None,
                model: "claude-opus-5-5".into(),
                tokens: TokenCounts::default(),
                cost_usd: Some(0.0),
                is_sidechain: agent_id.is_some(),
                agent_id: agent_id.map(String::from),
                skill: None,
                mcp_server: None,
                effort: None,
            })
            .unwrap();
        }
        w.commit().unwrap();
        let types = store.agent_types_between(100, 200).unwrap();
        assert_eq!(
            types.into_iter().collect::<Vec<_>>(),
            [
                ("a1".to_string(), "Explore".to_string()),
                ("a2".to_string(), "reviewer".to_string()),
            ]
        );
        assert!(store.knows_agent("a3").unwrap());
        assert!(!store.knows_agent("a4").unwrap());
    }
```

In `src/store/mod.rs`, change the five schema asserts from `18` to `19`: the four `assert_eq!(store.schema_version().unwrap(), 18);` and the one `assert_eq!(Store::latest_schema_version(), 18);`.
In `src/api/tests.rs`, change `(1, env!("CARGO_PKG_VERSION"), 18, std::process::id())` to `(1, env!("CARGO_PKG_VERSION"), 19, std::process::id())`.

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test --lib store::`
Expected: does not compile, `unresolved import crate::store::AgentRow`, `no method named agent`, `no method named agent_types_between` and `no method named knows_agent`.

- [ ] **Step 3: Add the table, the row, and the reads**

In `src/store/schema.rs`, append to `MIGRATIONS`, after `"CREATE INDEX limit_samples_by_time ON limit_samples(ts_ms);",`:

```rust
    r#"
CREATE TABLE agents (
    agent_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    agent_type TEXT NOT NULL
);
"#,
```

Do not add `agents` to `DERIVED_TABLES`: a reindex replays `raw_records` and never reads the meta files again.

In `src/store/writer.rs`, after `PromptRow`:

```rust
#[derive(Debug, Clone)]
pub struct AgentRow {
    pub agent_id: String,
    pub session_id: String,
    pub agent_type: String,
}
```

and before `pub fn tool_result`:

```rust
    pub fn agent(&mut self, row: &AgentRow) -> Result<()> {
        self.exec(
            "INSERT OR REPLACE INTO agents (agent_id, session_id, agent_type) VALUES (?1, ?2, ?3)",
            params![row.agent_id, row.session_id, row.agent_type],
        )?;
        Ok(())
    }
```

In `src/store/insights.rs`, add `use std::collections::BTreeMap;` above `use anyhow::Result;`, then after `SESSION_PROJECT`:

```rust
const AGENT_TYPES_BETWEEN: &str = "SELECT DISTINCT a.agent_id, a.agent_type
     FROM requests r JOIN agents a ON a.agent_id = r.agent_id
     WHERE r.ts >= ?1 AND r.ts < ?2";

const KNOWS_AGENT: &str = "SELECT 1 FROM agents WHERE agent_id = ?1";
```

add both to `insights_statements()` so the existing no-`SCAN` test covers them:

```rust
        SESSION_PROJECT.to_string(),
        AGENT_TYPES_BETWEEN.to_string(),
        KNOWS_AGENT.to_string(),
    ]
```

and before `pub fn prefix_kinds_between`:

```rust
    pub fn agent_types_between(
        &self,
        since_ts: i64,
        until_ts: i64,
    ) -> Result<BTreeMap<String, String>> {
        let mut statement = self.conn.prepare_cached(AGENT_TYPES_BETWEEN)?;
        let rows = statement.query_map([since_ts, until_ts], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn knows_agent(&self, agent_id: &str) -> Result<bool> {
        Ok(self.conn.prepare_cached(KNOWS_AGENT)?.exists([agent_id])?)
    }
```

- [ ] **Step 4: Run the store tests**

Run: `cargo test --lib store::`
Expected: PASS, including `the_insights_reads_search_indexes_instead_of_scanning_tables`.

- [ ] **Step 5: Write the failing ingest tests**

In `src/ingest/mod.rs` `mod tests`, before `fn import_reconciles_with_claude_counters`:

```rust
    #[test]
    fn a_subagent_type_is_read_from_its_meta_file_once_it_exists() {
        let fx = Fixture::new();
        let line = assistant(
            "s1",
            "req_1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        fx.write("-w-proj/s1/subagents/agent-a1.jsonl", &format!("{line}\n"));
        fx.write("-w-proj/s1/subagents/agent-a2.jsonl", &format!("{line}\n"));
        fx.write("-w-proj/s1/subagents/agent-a2.meta.json", "{not json");
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        assert!(!store.knows_agent("a1").unwrap());
        assert!(!store.knows_agent("a2").unwrap());
        fx.write(
            "-w-proj/s1/subagents/agent-a1.meta.json",
            r#"{"agentType":"Explore","description":"Find the parser","toolUseId":"toolu_1"}"#,
        );
        fx.run(&mut store);
        let since = chrono::DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
            .unwrap()
            .timestamp();
        let types = store.agent_types_between(since, since + 86_400).unwrap();
        assert_eq!(
            types.into_iter().collect::<Vec<_>>(),
            [("a1".to_string(), "Explore".to_string())]
        );
        fx.write(
            "-w-proj/s1/subagents/agent-a1.meta.json",
            r#"{"agentType":"reviewer"}"#,
        );
        fx.run(&mut store);
        let kept = store.agent_types_between(since, since + 86_400).unwrap();
        assert_eq!(kept.get("a1").map(String::as_str), Some("Explore"));
    }

    #[test]
    fn workflow_agents_keep_their_type_and_a_long_type_is_cut() {
        let fx = Fixture::new();
        let line = assistant(
            "s1",
            "req_1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        let run = "-w-proj/s1/subagents/workflows/wf_1";
        fx.write(&format!("{run}/agent-w1.jsonl"), &format!("{line}\n"));
        let long = "x".repeat(100);
        fx.write(
            &format!("{run}/agent-w1.meta.json"),
            &format!(r#"{{"agentType":"{long}"}}"#),
        );
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        let since = chrono::DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
            .unwrap()
            .timestamp();
        let types = store.agent_types_between(since, since + 86_400).unwrap();
        assert_eq!(types.get("w1").map(String::len), Some(AGENT_TYPE_CHARS));
    }
```

The second `fx.run` of the first test is the upgrade case: the transcript did not change, yet the type must be read.

- [ ] **Step 6: Run them to see them fail**

Run: `cargo test --lib ingest::`
Expected: does not compile, `cannot find value AGENT_TYPE_CHARS`.

- [ ] **Step 7: Read the meta file during ingest**

In `src/ingest/mod.rs`, add `AgentRow` to the `crate::store` import (the gate's `cargo fmt` rewraps it), and after `pub const BATCH_LINES: u64 = 5000;`:

```rust
pub const AGENT_TYPE_CHARS: usize = 64;
```

In `ingest_file_limited`, right after the `let mut report = IngestReport { ... };` and before `let metadata = std::fs::metadata(&file.path)?;`:

```rust
        if let Some(agent_id) = &file.location.agent_id {
            self.record_agent_type(file, agent_id)?;
        }
```

Before `fn reconcile_touched`:

```rust
impl Ingestor<'_> {
    fn record_agent_type(&mut self, file: &TranscriptFile, agent_id: &str) -> Result<()> {
        if self.store.knows_agent(agent_id)? {
            return Ok(());
        }
        let Some(agent_type) = agent_type(&file.path.with_extension("meta.json")) else {
            return Ok(());
        };
        let mut writer = self.store.writer()?;
        writer.agent(&AgentRow {
            agent_id: agent_id.to_string(),
            session_id: file.location.session_hint.clone(),
            agent_type,
        })?;
        writer.commit()
    }
}

fn agent_type(meta: &Path) -> Option<String> {
    let text = std::fs::read_to_string(meta).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let name = value.get("agentType")?.as_str()?.trim();
    (!name.is_empty()).then(|| name.chars().take(AGENT_TYPE_CHARS).collect())
}
```

`with_extension` turns `agent-a1.jsonl` into `agent-a1.meta.json`, in `subagents/` and in a workflow run directory alike.

- [ ] **Step 8: Run the Rust gate**

Run: `cargo fmt && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Expected: PASS, 573 lib tests.

- [ ] **Step 9: Commit**

```bash
git add src/store src/ingest/mod.rs src/api/tests.rs
git commit -m "feat(store): keep each subagent's type from its meta file (schema 19)"
```

---

### Task 2: Cache lifetime rule

**Files:**
- Create: `src/insights/ttl.rs`
- Modify: `src/cache_model.rs:323` (`read_instead` becomes `pub`)
- Modify: `src/insights/mod.rs` (module, `RuleId::CacheTtl`, `rules()`)

**Interfaces:**
- Consumes: `cache_model::{FIVE_MINUTES_MS, ONE_HOUR_MS, MIN_REBUILD_PREFIX, Ttl}`, `cache_model::read_instead(&TokenCounts, u64) -> TokenCounts`, `PriceTable::{cost, cost_within}`, `InsightContext::priced`.
- Produces: `pub struct CacheTtl;` implementing `Rule`, and `RuleId::CacheTtl` (serialized `cache_ttl`).

- [ ] **Step 1: Write the failing tests**

Create `src/insights/ttl.rs` with only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::fixture::*;

    fn written_1h(cache_read: u64, cache_write_1h: u64) -> TokenCounts {
        TokenCounts {
            input: 10,
            output: 10,
            cache_read,
            cache_write_1h,
            ..Default::default()
        }
    }

    fn close(actual: Option<f64>, expected: f64) {
        let actual = actual.unwrap();
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    fn short_main_turns(fixture: &mut Fixture) {
        fixture.request("s1", SINCE, OPUS, written_1h(0, 100_000));
        for turn in 1..=20 {
            fixture.request("s1", SINCE + turn * 60, OPUS, written_1h(100_000, 10_000));
        }
        fixture.request(
            "s1",
            SINCE + 20 * 60 + 1_200,
            OPUS,
            written_1h(100_000, 10_000),
        );
    }

    fn paused_subagent(fixture: &mut Fixture) {
        fixture.subagent("s1", SINCE, OPUS, counts(10, 0, 100_000, 10));
        fixture.subagent("s1", SINCE + 600, OPUS, counts(10, 0, 100_000, 10));
        fixture.subagent("s1", SINCE + 2_400, OPUS, counts(10, 0, 100_000, 10));
        fixture.subagent("s1", SINCE + 2_460, OPUS, counts(10, 100_000, 1_000, 10));
    }

    #[test]
    fn a_main_thread_that_rarely_pauses_is_told_to_use_the_five_minute_cache() {
        let mut fixture = Fixture::new();
        short_main_turns(&mut fixture);
        let findings = CacheTtl.evaluate(&fixture.ctx());
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(finding.rule, RuleId::CacheTtl);
        assert_eq!(
            finding.tip,
            "use the 5-minute cache, your turns rarely pause long enough to need an hour"
        );
        let premium = 310_000.0 * (8.0 - 5.0) / 1e6;
        let rewrite = 100_000.0 * (5.0 - 0.2) / 1e6;
        close(finding.saving_usd, (premium - rewrite) / 2.0);
        assert_eq!(finding.confidence, Confidence::Medium);
        assert_eq!(
            finding.evidence,
            [
                "Main thread: 310k\u{a0}tokens written at the 1-hour rate, $0.93 above the 5-minute rate",
                "1 of 21\u{a0}turns came 5 to 60 minutes after the one before, rewriting it would cost $0.48",
            ]
        );
    }

    #[test]
    fn subagents_that_pause_past_five_minutes_are_told_to_use_the_one_hour_cache() {
        let mut fixture = Fixture::new();
        paused_subagent(&mut fixture);
        let finding = &CacheTtl.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "use the 1-hour cache for subagents, they often pause past 5 minutes"
        );
        let rebuilds = 2.0 * 100_000.0 * (5.0 - 0.2) / 1e6;
        let premium = 101_000.0 * (8.0 - 5.0) / 1e6;
        close(finding.saving_usd, (rebuilds - premium) / 2.0);
        assert_eq!(
            finding.evidence,
            [
                "Subagents: 2\u{a0}rebuilds after 5 to 60-minute pauses, $0.96",
                "The 1-hour rate on the other cache writes would add $0.30",
            ]
        );
    }

    #[test]
    fn both_switches_add_up_and_the_larger_one_names_the_tip() {
        let mut fixture = Fixture::new();
        short_main_turns(&mut fixture);
        paused_subagent(&mut fixture);
        let finding = &CacheTtl.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "use the 1-hour cache for subagents, they often pause past 5 minutes"
        );
        assert_eq!(finding.evidence.len(), 4);
        assert!(finding.evidence[2].starts_with("Main thread: "));
        let main = (310_000.0 * 3.0 - 100_000.0 * 4.8) / 1e6;
        let subagents = (2.0 * 100_000.0 * 4.8 - 101_000.0 * 3.0) / 1e6;
        close(finding.saving_usd, (main + subagents) / 2.0);
    }

    #[test]
    fn pauses_that_cost_more_than_the_premium_keep_the_one_hour_cache() {
        let paused = |gap: i64, model: &str, compacted: bool| {
            let mut fixture = Fixture::new();
            fixture.request("s1", SINCE, OPUS, written_1h(0, 100_000));
            fixture.request("s1", SINCE + gap, model, written_1h(100_000, 0));
            if compacted {
                fixture.compaction("s1", SINCE + 10);
            }
            CacheTtl.evaluate(&fixture.ctx())
        };
        assert!(paused(1_200, OPUS, false).is_empty());
        for kept in [
            paused(3_600, OPUS, false),
            paused(1_200, SONNET, false),
            paused(1_200, OPUS, true),
        ] {
            close(kept[0].saving_usd, 100_000.0 * 3.0 / 1e6 / 2.0);
        }
        assert!(CacheTtl.evaluate(&Fixture::new().ctx()).is_empty());
    }
}
```

The bundled prices of Opus 5.5 are $4 input, $0.20 cache read, so $5 for a 5-minute write and $8 for a 1-hour write per million tokens; savings are per week, so the 14-day figure is halved.
The last test pins the case of the real store: one 20-minute pause costs more to rewrite ($0.48) than the 1-hour premium saves ($0.30), so no tip; a pause of an hour or more, a model switch, or a compaction between the turns takes the pause out of the replay.

In `src/insights/mod.rs`, add `mod ttl;` after `pub mod sources;`, `pub use ttl::CacheTtl;` after the `pub use setup::...` line, `CacheTtl,` after `CacheBusts,` in `RuleId`, and `Box::new(CacheTtl),` after `Box::new(CacheBusts),` in `rules()`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --lib insights::ttl`
Expected: does not compile, among others `unresolved import ttl::CacheTtl`, `cannot find value CacheTtl` and `cannot find type TokenCounts`.

- [ ] **Step 3: Write the rule**

In `src/cache_model.rs`, make `read_instead` public: `pub fn read_instead(tokens: &TokenCounts, moved: u64) -> TokenCounts {`.

Put this above the test module of `src/insights/ttl.rs`:

```rust
use std::collections::BTreeMap;

use super::words::{count, money, tokens};
use super::{Confidence, Draft, Finding, InsightContext, Rule, RuleId, is_main};
use crate::cache_model::{FIVE_MINUTES_MS, MIN_REBUILD_PREFIX, ONE_HOUR_MS, Ttl, read_instead};
use crate::pricing::normalize_model;
use crate::store::SpentRequest;
use crate::transcript::TokenCounts;

pub struct CacheTtl;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Bucket {
    Main,
    Subagents,
}

#[derive(Default)]
struct Replay {
    turns: u64,
    written_1h: u64,
    written_5m: u64,
    premium_1h_usd: f64,
    premium_5m_usd: f64,
    paused_warm: u64,
    paused_warm_usd: f64,
    paused_rebuilds: u64,
    paused_rebuild_usd: f64,
}

struct Switch {
    bucket: Bucket,
    to_5m: bool,
    net_usd: f64,
    evidence: Vec<String>,
}

impl Rule for CacheTtl {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        let mut threads: BTreeMap<(Bucket, &str, Option<&str>), Vec<&SpentRequest>> =
            BTreeMap::new();
        for request in ctx.requests {
            let bucket = if is_main(request) {
                Bucket::Main
            } else {
                Bucket::Subagents
            };
            threads
                .entry((
                    bucket,
                    request.session_id.as_str(),
                    request.point.agent_id.as_deref(),
                ))
                .or_default()
                .push(request);
        }
        let mut replays: BTreeMap<Bucket, Replay> = BTreeMap::new();
        for ((bucket, session, _), mut thread) in threads {
            thread.sort_by_key(|r| r.point.ts);
            let replay = replays.entry(bucket).or_default();
            replay_thread(ctx, bucket, session, &thread, replay);
        }
        let mut switches: Vec<Switch> = replays
            .into_iter()
            .filter_map(|(bucket, replay)| switch(bucket, &replay))
            .filter(|s| s.net_usd > 0.0)
            .collect();
        switches.sort_by(|a, b| b.net_usd.total_cmp(&a.net_usd));
        let Some(top) = switches.first() else {
            return Vec::new();
        };
        let draft = Draft {
            rule: RuleId::CacheTtl,
            tip: tip(top.bucket, top.to_5m).into(),
            evidence: switches.iter().flat_map(|s| s.evidence.clone()).collect(),
            confidence: Confidence::Medium,
            how: "Replays the last 14 days with the other cache lifetime, turn by turn, \
                  for the main thread and for subagents apart. \
                  A turn sent 5 to 60 minutes after the one before reads a 1-hour cache but rewrites a 5-minute one, \
                  and the 1-hour cache costs more to write. \
                  Set \"promptCacheTtl\" (main thread) or \"subagentPromptCacheTtl\" (subagents) to \"5m\" or \"1h\" \
                  in ~/.claude/settings.json, from Claude Code 2.1.242."
                .into(),
        };
        let net = switches.iter().map(|s| s.net_usd).sum();
        ctx.priced(draft, net).into_iter().collect()
    }
}

fn replay_thread(
    ctx: &InsightContext,
    bucket: Bucket,
    session: &str,
    thread: &[&SpentRequest],
    replay: &mut Replay,
) {
    let mut moved_by_index = vec![0u64; thread.len()];
    for (index, pair) in thread.windows(2).enumerate() {
        let (previous, request) = (pair[0], pair[1]);
        replay.turns += 1;
        let prefix = previous.point.tokens.cache_read + previous.point.tokens.cache_write();
        let gap_ms = (sent(request) - previous.point.ts) * 1000;
        let paused = (FIVE_MINUTES_MS..ONE_HOUR_MS).contains(&gap_ms);
        let same_model =
            normalize_model(&previous.point.model) == normalize_model(&request.point.model);
        if !paused
            || !same_model
            || prefix < MIN_REBUILD_PREFIX
            || compacted_between(ctx, bucket, session, previous.point.ts, request.point.ts)
        {
            continue;
        }
        let tokens = &request.point.tokens;
        let total = tokens.prompt_total();
        let model = request.point.model.as_str();
        if tokens.cache_read * 10 < prefix && tokens.cache_write() * 2 >= prefix {
            let moved = tokens.cache_write().min(prefix);
            moved_by_index[index + 1] = moved.min(tokens.cache_write_5m);
            let paid = ctx.prices.cost(model, tokens).unwrap_or(0.0);
            let warm = ctx
                .prices
                .cost(model, &read_instead(tokens, moved))
                .unwrap_or(0.0);
            replay.paused_rebuilds += 1;
            replay.paused_rebuild_usd += paid - warm;
        } else {
            let moved = tokens.cache_read.min(prefix);
            let rewrite = cost_of(ctx, model, total, Ttl::FiveMinutes.written(moved));
            let read = cost_of(
                ctx,
                model,
                total,
                TokenCounts {
                    cache_read: moved,
                    ..Default::default()
                },
            );
            replay.paused_warm += 1;
            replay.paused_warm_usd += rewrite - read;
        }
    }
    for (request, moved) in thread.iter().zip(moved_by_index) {
        let tokens = &request.point.tokens;
        let total = tokens.prompt_total();
        let model = request.point.model.as_str();
        replay.written_1h += tokens.cache_write_1h;
        replay.written_5m += tokens.cache_write_5m;
        replay.premium_1h_usd += premium(ctx, model, total, tokens.cache_write_1h);
        replay.premium_5m_usd += premium(ctx, model, total, tokens.cache_write_5m - moved);
    }
}

fn switch(bucket: Bucket, replay: &Replay) -> Option<Switch> {
    let who = match bucket {
        Bucket::Main => "Main thread",
        Bucket::Subagents => "Subagents",
    };
    if replay.written_1h > 0 && replay.written_1h >= replay.written_5m {
        return Some(Switch {
            bucket,
            to_5m: true,
            net_usd: replay.premium_1h_usd - replay.paused_warm_usd,
            evidence: vec![
                format!(
                    "{who}: {}\u{a0}tokens written at the 1-hour rate, {} above the 5-minute rate",
                    tokens(replay.written_1h),
                    money(replay.premium_1h_usd)
                ),
                format!(
                    "{} of {} came 5 to 60 minutes after the one before, rewriting {} would cost {}",
                    replay.paused_warm,
                    count(replay.turns, "turn", "turns"),
                    if replay.paused_warm == 1 {
                        "it"
                    } else {
                        "them"
                    },
                    money(replay.paused_warm_usd)
                ),
            ],
        });
    }
    (replay.written_5m > 0).then(|| Switch {
        bucket,
        to_5m: false,
        net_usd: replay.paused_rebuild_usd - replay.premium_5m_usd,
        evidence: vec![
            format!(
                "{who}: {} after 5 to 60-minute pauses, {}",
                count(replay.paused_rebuilds, "rebuild", "rebuilds"),
                money(replay.paused_rebuild_usd)
            ),
            format!(
                "The 1-hour rate on the other cache writes would add {}",
                money(replay.premium_5m_usd)
            ),
        ],
    })
}

fn tip(bucket: Bucket, to_5m: bool) -> &'static str {
    match (bucket, to_5m) {
        (Bucket::Main, true) => {
            "use the 5-minute cache, your turns rarely pause long enough to need an hour"
        }
        (Bucket::Main, false) => "use the 1-hour cache, your pauses often outlast 5 minutes",
        (Bucket::Subagents, true) => {
            "use the 5-minute cache for subagents, they rarely pause long enough to need an hour"
        }
        (Bucket::Subagents, false) => {
            "use the 1-hour cache for subagents, they often pause past 5 minutes"
        }
    }
}

fn sent(request: &SpentRequest) -> i64 {
    request
        .point
        .sent
        .filter(|sent| *sent <= request.point.ts)
        .unwrap_or(request.point.ts)
}

fn compacted_between(
    ctx: &InsightContext,
    bucket: Bucket,
    session: &str,
    after: i64,
    until: i64,
) -> bool {
    bucket == Bucket::Main
        && ctx
            .compactions
            .iter()
            .any(|c| c.main && c.session_id == session && after < c.ts && c.ts <= until)
}

fn cost_of(ctx: &InsightContext, model: &str, total: u64, part: TokenCounts) -> f64 {
    ctx.prices.cost_within(model, &part, total).unwrap_or(0.0)
}

fn premium(ctx: &InsightContext, model: &str, total: u64, written: u64) -> f64 {
    if written == 0 {
        return 0.0;
    }
    cost_of(ctx, model, total, Ttl::OneHour.written(written))
        - cost_of(ctx, model, total, Ttl::FiveMinutes.written(written))
}
```

A rebuild here has the shape `cache_model::rebuild` checks (a read under a tenth of the previous prefix and a write of at least half of it); its moved 5-minute tokens become reads under the 1-hour cache, so they leave the premium sum.
`sent` mirrors `timeline::from_transcript`: the send time when it is known and not after the answer.

- [ ] **Step 4: Run the Rust gate**

Run: `cargo fmt && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Expected: PASS, 577 lib tests; the report tests keep their rule lists, since none of their fixtures pays for a lifetime it does not need.

- [ ] **Step 5: Commit**

```bash
git add src/insights/ttl.rs src/insights/mod.rs src/cache_model.rs
git commit -m "feat(insights): suggest the cache lifetime that costs less, for the main thread and for subagents"
```

---

### Task 3: Subagent model fit rule

**Files:**
- Create: `src/insights/subagents.rs`
- Modify: `src/insights/mod.rs` (module, `RuleId::SubagentModelFit`, `rules()`, `InsightContext::agent_types`)
- Modify: `src/insights/fixture.rs` (`agent_types`, `push_agent`, `typed`)
- Modify: `src/insights/report.rs` (read the types, report tests)

**Interfaces:**
- Consumes: `Store::agent_types_between` (Task 1), `forecast::{alternative_for, family}`, `usage::TOP_TIER`, `insights::listed`.
- Produces: `pub struct SubagentModelFit;`, `RuleId::SubagentModelFit` (serialized `subagent_model_fit`), and `pub agent_types: &'a BTreeMap<String, String>` on `InsightContext`.
- Produces for Task 4: the report test helpers `account_store(subagent_type: Option<&str>, limits: &[(&str, &str, f64)]) -> Store` and `report_for(store: &Store, session: &str) -> InsightsReport`.

- [ ] **Step 1: Write the failing rule tests**

Create `src/insights/subagents.rs` with only its test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::fixture::*;

    fn close(actual: Option<f64>, expected: f64) {
        let actual = actual.unwrap();
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    #[test]
    fn top_tier_subagents_are_repriced_by_agent_type_and_the_costliest_type_names_the_tip() {
        let mut fixture = Fixture::new();
        fixture.typed(
            "a1",
            "Explore",
            SINCE + 10,
            OPUS,
            counts(100_000, 0, 0, 1_000),
        );
        fixture.typed(
            "a1",
            "Explore",
            SINCE + 20,
            OPUS,
            counts(100_000, 0, 0, 1_000),
        );
        fixture.typed(
            "a2",
            "Explore",
            SINCE + 30,
            OPUS,
            counts(100_000, 0, 0, 1_000),
        );
        fixture.typed(
            "a3",
            "reviewer",
            SINCE + 40,
            OPUS,
            counts(50_000, 0, 0, 1_000),
        );
        fixture.typed(
            "a4",
            "Explore",
            SINCE + 50,
            SONNET,
            counts(100_000, 0, 0, 1_000),
        );
        fixture.push_agent("a5", SINCE + 60, OPUS, counts(10_000, 0, 0, 0));
        fixture.request("s1", SINCE + 70, OPUS, counts(100_000, 0, 0, 1_000));
        let findings = SubagentModelFit.evaluate(&fixture.ctx());
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(finding.rule, RuleId::SubagentModelFit);
        assert_eq!(finding.tip, "run Explore subagents on Sonnet\u{a0}5.5");
        assert_eq!(finding.confidence, Confidence::Medium);
        close(finding.saving_usd, (0.63 + 0.11 + 0.02) / 2.0);
        assert_eq!(
            finding.evidence,
            [
                "Explore: 2\u{a0}runs on Opus\u{a0}5.5 cost $1.26, $0.63 on Sonnet\u{a0}5.5",
                "reviewer: 1\u{a0}run on Opus\u{a0}5.5 cost $0.22, $0.11 on Sonnet\u{a0}5.5",
                "untyped: 1\u{a0}run on Opus\u{a0}5.5 cost $0.04, $0.02 on Sonnet\u{a0}5.5",
                "Subagents drew 80% of your spend",
            ]
        );
    }

    #[test]
    fn agents_that_write_code_get_a_low_confidence_tip_and_untyped_ones_a_generic_one() {
        let mut fixture = Fixture::new();
        fixture.typed(
            "a1",
            "general-purpose",
            SINCE,
            OPUS,
            counts(100_000, 0, 0, 1_000),
        );
        let finding = &SubagentModelFit.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "run general-purpose subagents on Sonnet\u{a0}5.5"
        );
        assert_eq!(finding.confidence, Confidence::Low);
        let mut untyped = Fixture::new();
        untyped.push_agent("a9", SINCE, OPUS, counts(100_000, 0, 0, 1_000));
        let finding = &SubagentModelFit.evaluate(&untyped.ctx())[0];
        assert_eq!(finding.tip, "run subagents on Sonnet\u{a0}5.5");
        assert_eq!(finding.confidence, Confidence::Low);
    }

    #[test]
    fn subagents_already_on_a_cheaper_model_or_spending_pennies_give_no_tip() {
        let mut fixture = Fixture::new();
        fixture.typed(
            "a1",
            "Explore",
            SINCE,
            SONNET,
            counts(1_000_000, 0, 0, 1_000),
        );
        fixture.request("s1", SINCE + 10, OPUS, counts(1_000_000, 0, 0, 1_000));
        assert!(SubagentModelFit.evaluate(&fixture.ctx()).is_empty());
        fixture.typed("a2", "Explore", SINCE + 20, OPUS, counts(90_000, 0, 0, 0));
        assert!(SubagentModelFit.evaluate(&fixture.ctx()).is_empty());
        assert!(SubagentModelFit.evaluate(&Fixture::new().ctx()).is_empty());
    }
}
```

Sonnet 5.5 costs half of Opus 5.5 in the bundled prices, so each Opus request saves half its cost; the main-thread request counts in the spend share but is never repriced, and the Sonnet subagent counts in the share but has nothing to save.

In `src/insights/mod.rs`, add `mod subagents;` after `pub mod sources;`, `pub use subagents::SubagentModelFit;` before `pub use ttl::CacheTtl;`, `SubagentModelFit,` after `ModelFit,` in `RuleId`, `Box::new(SubagentModelFit),` after `Box::new(ModelFit),` in `rules()`, and the field on `InsightContext`, after `requests`:

```rust
    pub agent_types: &'a BTreeMap<String, String>,
```

In `src/insights/fixture.rs`, add `use std::collections::BTreeMap;` at the top, then a field `pub agent_types: BTreeMap<String, String>,` after `requests` (with `agent_types: BTreeMap::new(),` in `new()` and `agent_types: &self.agent_types,` in `ctx()`), and after the closing brace of `pub fn subagent`:

```rust
    pub fn push_agent(&mut self, agent: &str, ts: i64, model: &str, tokens: TokenCounts) {
        self.push("s1", ts, model, tokens, Some(agent));
    }

    pub fn typed(
        &mut self,
        agent: &str,
        agent_type: &str,
        ts: i64,
        model: &str,
        tokens: TokenCounts,
    ) {
        self.push("s1", ts, model, tokens, Some(agent));
        self.agent_types.insert(agent.into(), agent_type.into());
    }
```

- [ ] **Step 2: Write the failing report test**

In `src/insights/report.rs`, widen the test imports:

```rust
    use crate::store::{
        AgentRow, AttachmentRow, AttributionSource, LimitSample, PromptRow, RequestPoint,
        RequestRow, SessionRow,
    };
```

and add at the end of `mod tests`:

```rust
    fn account_store(subagent_type: Option<&str>, limits: &[(&str, &str, f64)]) -> Store {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.session(&SessionRow {
            id: "s1".into(),
            project: "-work".into(),
            cwd: None,
            ts: Some(utc("2026-10-10T09:00:00Z")),
        })
        .unwrap();
        w.attribute("s1", "oauth:me", AttributionSource::Start)
            .unwrap();
        if let Some(agent_type) = subagent_type {
            w.agent(&AgentRow {
                agent_id: "a1".into(),
                session_id: "s1".into(),
                agent_type: agent_type.into(),
            })
            .unwrap();
        }
        let tokens = TokenCounts {
            input: 1_000_000,
            ..Default::default()
        };
        w.request(&RequestRow {
            key: "r1".into(),
            session_id: "s1".into(),
            ts: utc("2026-10-10T09:30:00Z"),
            sent_ts: None,
            model: OPUS.into(),
            cost_usd: PriceTable::bundled().cost(OPUS, &tokens),
            tokens,
            is_sidechain: true,
            agent_id: Some("a1".into()),
            skill: None,
            mcp_server: None,
            effort: None,
        })
        .unwrap();
        for (account, at, used_pct) in limits {
            w.limit_sample(&LimitSample {
                ts_ms: utc(at) * 1000,
                session_id: "s1".into(),
                account_id: Some(account.to_string()),
                model: Some(OPUS.into()),
                window: "five_hour".into(),
                used_pct: *used_pct,
                resets_at_ms: Some(utc("2026-10-10T11:00:00Z") * 1000),
            })
            .unwrap();
        }
        w.commit().unwrap();
        store
    }

    fn report_for(store: &Store, session: &str) -> InsightsReport {
        let temp = tempfile::tempdir().unwrap();
        let scan = config_scan::scan(&Roots {
            home: temp.path().join("home"),
            claude_dir: temp.path().join("claude"),
            claude_json: temp.path().join("claude.json"),
            project: None,
        });
        read(
            store,
            &PriceTable::bundled(),
            &Request {
                session,
                scan: &scan,
                retention_days: Some(90),
                now_ms: utc("2026-10-11T12:00:00Z") * 1000,
            },
            &Utc,
        )
        .unwrap()
    }

    #[test]
    fn the_report_names_subagents_by_the_type_the_store_keeps() {
        let typed = report_for(&account_store(Some("Explore"), &[]), "s1");
        let rules: Vec<RuleId> = typed.findings.iter().map(|f| f.rule).collect();
        assert_eq!(rules, [RuleId::SubagentModelFit]);
        assert_eq!(
            typed.findings[0].tip,
            "run Explore subagents on Sonnet\u{a0}5.5"
        );
        let untyped = report_for(&account_store(None, &[]), "s1");
        assert_eq!(untyped.findings[0].tip, "run subagents on Sonnet\u{a0}5.5");
    }
```

`account_store` already takes the limit samples Task 4 needs; this task passes none.

- [ ] **Step 3: Run them to see them fail**

Run: `cargo test --lib insights::`
Expected: does not compile, among others `cannot find value SubagentModelFit` and `missing field agent_types` in `report.rs`.

- [ ] **Step 4: Write the rule and read the types in the report**

Put this above the test module of `src/insights/subagents.rs`:

```rust
use std::collections::{BTreeMap, BTreeSet};

use super::usage::TOP_TIER;
use super::words::{count, model_name, money};
use super::{Confidence, Draft, Finding, InsightContext, Rule, RuleId, is_main, listed};
use crate::forecast::{alternative_for, family};

pub const READ_ONLY_AGENTS: &[&str] = &["Explore", "Plan", "claude-code-guide", "statusline-setup"];
pub const UNKNOWN_AGENT: &str = "untyped";

pub struct SubagentModelFit;

#[derive(Default)]
struct Spent<'a> {
    runs: BTreeSet<&'a str>,
    paid: f64,
    cheaper: f64,
    by_model: BTreeMap<&'a str, f64>,
}

impl Spent<'_> {
    fn saving(&self) -> f64 {
        self.paid - self.cheaper
    }

    fn model(&self) -> &str {
        self.by_model
            .iter()
            .max_by(|a, b| a.1.total_cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map_or("", |(model, _)| model)
    }
}

impl Rule for SubagentModelFit {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        let mut by_type: BTreeMap<&str, Spent> = BTreeMap::new();
        let (mut all, mut subagents) = (0.0, 0.0);
        for request in ctx.requests {
            let paid = request.cost_usd.unwrap_or(0.0);
            all += paid;
            let Some(agent) = request
                .point
                .agent_id
                .as_deref()
                .filter(|_| !is_main(request))
            else {
                continue;
            };
            subagents += paid;
            let model = request.point.model.as_str();
            if !TOP_TIER.contains(&family(model)) {
                continue;
            }
            let Some(cheaper) = alternative_for(model)
                .and_then(|alternative| ctx.prices.cost(alternative, &request.point.tokens))
            else {
                continue;
            };
            let kind = ctx
                .agent_types
                .get(agent)
                .map_or(UNKNOWN_AGENT, String::as_str);
            let spent = by_type.entry(kind).or_default();
            spent.runs.insert(agent);
            spent.paid += paid;
            spent.cheaper += cheaper;
            *spent.by_model.entry(model).or_default() += paid;
        }
        let mut ordered: Vec<(&str, Spent)> = by_type.into_iter().collect();
        ordered.sort_by(|a, b| b.1.saving().total_cmp(&a.1.saving()).then(a.0.cmp(b.0)));
        let Some((top, spent)) = ordered.first() else {
            return Vec::new();
        };
        let alternative = alternative_for(spent.model()).unwrap_or_default();
        let mut evidence = listed(
            &ordered,
            |(kind, spent)| {
                let model = spent.model();
                format!(
                    "{kind}: {} on {} cost {}, {} on {}",
                    count(spent.runs.len() as u64, "run", "runs"),
                    model_name(model),
                    money(spent.paid),
                    money(spent.cheaper),
                    model_name(alternative_for(model).unwrap_or_default())
                )
            },
            "agent types",
        );
        if all > 0.0 {
            evidence.push(format!(
                "Subagents drew {:.0}% of your spend",
                subagents / all * 100.0
            ));
        }
        let draft = Draft {
            rule: RuleId::SubagentModelFit,
            tip: match *top {
                UNKNOWN_AGENT => format!("run subagents on {}", model_name(alternative)),
                kind => format!("run {kind} subagents on {}", model_name(alternative)),
            },
            evidence,
            confidence: match READ_ONLY_AGENTS.contains(top) {
                true => Confidence::Medium,
                false => Confidence::Low,
            },
            how: "Reprices the subagent requests of the last 14 days that ran on Opus, Fable or Mythos \
                  on the next model down at the same token counts, by the agent type each subagent's .meta.json names. \
                  A cheaper model may take more turns or do worse on hard work, so check its results first. \
                  Set model: in the agent's file under .claude/agents, add your own agent named Explore to replace the built-in one, \
                  or set CLAUDE_CODE_SUBAGENT_MODEL for the agents that name no model."
                .into(),
        };
        let saving = ordered.iter().map(|(_, spent)| spent.saving()).sum();
        ctx.priced(draft, saving).into_iter().collect()
    }
}
```

In `src/insights/report.rs` `read`, right after the `let (rebuilds, compactions) = ...;` statement:

```rust
    let agent_types = store.agent_types_between(since, spans.until)?;
```

and in the `InsightContext { ... }` literal, after `requests: &requests,`:

```rust
        agent_types: &agent_types,
```

- [ ] **Step 5: Run the Rust gate**

Run: `cargo fmt && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Expected: PASS, 581 lib tests.

- [ ] **Step 6: Commit**

```bash
git add src/insights
git commit -m "feat(insights): reprice top-tier subagents on the next model down, by agent type"
```

---

### Task 4: Limit hits rule, ranked first

**Files:**
- Create: `src/insights/hits.rs`
- Modify: `src/insights/mod.rs` (module, `RuleId::LimitHits`, `rules()`, `InsightContext::limit_samples`, `rank`)
- Modify: `src/insights/fixture.rs` (`limit_samples`, `limit`)
- Modify: `src/insights/report.rs` (read the account's samples, report test)
- Modify: `src/insights/tests.rs` (ranking test)

**Interfaces:**
- Consumes: `limits::instances(&[LimitSample]) -> Vec<&[LimitSample]>`, `history::FIVE_HOUR`, `Store::limit_samples_since(since_ms)`, `Scope::account()`, Task 3's `account_store` and `report_for`.
- Produces: `pub struct LimitHits;`, `RuleId::LimitHits` (serialized `limit_hits`), and `pub limit_samples: &'a [LimitSample]` on `InsightContext`.

- [ ] **Step 1: Write the failing tests**

Create `src/insights/hits.rs` with only its test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::fixture::*;

    const HOUR: i64 = 3_600;
    const DAY: i64 = 24 * HOUR;

    #[test]
    fn five_hour_limits_that_ran_out_name_the_wait_the_model_and_the_subagents_behind_them() {
        let mut fixture = Fixture::new();
        fixture.limit(FIVE_HOUR, SINCE + 5 * HOUR + 60, 40.0, SINCE + 10 * HOUR);
        fixture.limit(FIVE_HOUR, SINCE + 7 * HOUR, 100.0, SINCE + 10 * HOUR);
        fixture.limit(
            FIVE_HOUR,
            SINCE + 2 * DAY + HOUR,
            30.0,
            SINCE + 2 * DAY + 5 * HOUR,
        );
        fixture.limit(
            FIVE_HOUR,
            SINCE + 2 * DAY + 4 * HOUR,
            100.0,
            SINCE + 2 * DAY + 5 * HOUR,
        );
        fixture.subagent("s1", SINCE + 6 * HOUR, OPUS, counts(100_000, 0, 0, 0));
        fixture.request("s1", SINCE + 6 * HOUR + 60, OPUS, counts(50_000, 0, 0, 0));
        fixture.subagent(
            "s1",
            SINCE + 2 * DAY + 2 * HOUR,
            OPUS,
            counts(100_000, 0, 0, 0),
        );
        fixture.request("s1", SINCE + DAY, SONNET, counts(1_000_000, 0, 0, 0));
        let findings = LimitHits.evaluate(&fixture.ctx());
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(finding.rule, RuleId::LimitHits);
        assert_eq!(
            finding.tip,
            "run subagents on Sonnet\u{a0}5.5 so your limits last"
        );
        assert_eq!(finding.saving_usd, None);
        assert_eq!(finding.confidence, Confidence::Medium);
        assert_eq!(
            finding.evidence,
            [
                "The 5-hour limit ran out twice, a typical 2h before it reset",
                "Opus\u{a0}5.5 drew 100% of the spend before it ran out",
                "Subagents drew 80% of it",
            ]
        );
    }

    #[test]
    fn a_weekly_limit_spent_on_a_cheaper_model_suggests_pacing_the_week() {
        let mut fixture = Fixture::new();
        fixture.limit(SEVEN_DAY, SINCE + 8 * DAY, 90.0, SINCE + 10 * DAY);
        fixture.limit(SEVEN_DAY, SINCE + 9 * DAY, 100.0, SINCE + 10 * DAY);
        fixture.request("s1", SINCE + 8 * DAY + 10, SONNET, counts(100_000, 0, 0, 0));
        let finding = &LimitHits.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "pace heavy work across the week so the weekly limit lasts"
        );
        assert_eq!(
            finding.evidence,
            [
                "The weekly limit ran out once, 1d before it reset",
                "Sonnet\u{a0}5.5 drew 100% of the spend before it ran out",
            ]
        );
    }

    #[test]
    fn a_limit_still_out_counts_its_wait_until_now() {
        let mut fixture = Fixture::new();
        fixture.limit(FIVE_HOUR, NOW_S - 2 * HOUR, 80.0, NOW_S + 2 * HOUR);
        fixture.limit(FIVE_HOUR, NOW_S - HOUR, 100.0, NOW_S + 2 * HOUR);
        fixture.request("s1", NOW_S - 90 * 60, OPUS, counts(100_000, 0, 0, 0));
        let finding = &LimitHits.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "move routine work to Sonnet\u{a0}5.5 so your limits last"
        );
        assert_eq!(
            finding.evidence[0],
            "The 5-hour limit ran out once, 1h before it reset"
        );
    }

    #[test]
    fn limits_that_never_ran_out_in_the_window_give_no_tip() {
        let mut fixture = Fixture::new();
        fixture.limit(FIVE_HOUR, SINCE - HOUR, 100.0, SINCE + HOUR);
        fixture.limit(FIVE_HOUR, SINCE + DAY, 99.0, SINCE + DAY + HOUR);
        fixture.limit("seven_day_opus", SINCE + 2 * DAY, 100.0, SINCE + 3 * DAY);
        fixture.limit(FIVE_HOUR, SINCE + 3 * DAY, 100.0, SINCE + 3 * DAY);
        fixture.limit_samples.last_mut().unwrap().resets_at_ms = None;
        assert!(LimitHits.evaluate(&fixture.ctx()).is_empty());
        assert!(LimitHits.evaluate(&Fixture::new().ctx()).is_empty());
    }
}
```

The first test's waits are 3h and 1h, so the median is 2h; the Sonnet request falls in no window that ran out, and subagents drew $0.80 of the $1.00 spent inside them.
The last test covers a cap before the 14 days, 99%, a window kind this rule does not read, and a cap with no reset time.

In `src/insights/tests.rs`, rename `findings_rank_by_weekly_saving_then_rule_with_unpriced_ones_last` to `a_limit_that_ran_out_comes_first_then_findings_rank_by_weekly_saving_with_unpriced_ones_last`, add `finding(RuleId::LimitHits, None),` as the last input, and `RuleId::LimitHits,` as the first expected rule.

In `src/insights/report.rs`, add:

```rust
    #[test]
    fn the_report_reads_the_limits_of_the_requesting_session_account_only() {
        let store = account_store(
            Some("Explore"),
            &[
                ("oauth:me", "2026-10-10T08:00:00Z", 50.0),
                ("oauth:me", "2026-10-10T10:00:00Z", 100.0),
                ("oauth:work", "2026-10-10T08:00:00Z", 100.0),
            ],
        );
        let mine = report_for(&store, "s1");
        let rules: Vec<RuleId> = mine.findings.iter().map(|f| f.rule).collect();
        assert_eq!(rules, [RuleId::LimitHits, RuleId::SubagentModelFit]);
        assert_eq!(
            mine.findings[0].evidence[0],
            "The 5-hour limit ran out once, 1h before it reset"
        );
        let unattributed = report_for(&store, "s2");
        let rules: Vec<RuleId> = unattributed.findings.iter().map(|f| f.rule).collect();
        assert_eq!(rules, [RuleId::SubagentModelFit]);
    }
```

In `src/insights/mod.rs`, add `mod hits;` after `mod cache;`, `pub use hits::LimitHits;` after `pub use cache::...`, `LimitHits,` after `CacheTtl,` in `RuleId`, `Box::new(LimitHits),` after `Box::new(CacheTtl),` in `rules()`, `LimitSample` to the `crate::store` import (run `cargo fmt` to rewrap it), and the field as the last one of `InsightContext`:

```rust
    pub limit_samples: &'a [LimitSample],
```

In `src/insights/fixture.rs`, add `LimitSample` to the `crate::store` import, a field `pub limit_samples: Vec<LimitSample>,` as the last field (with `limit_samples: Vec::new(),` last in `new()` and `limit_samples: &self.limit_samples,` last in `ctx()`), and before `pub fn mcp_changed`:

```rust
    pub fn limit(&mut self, window: &str, ts: i64, used_pct: f64, resets: i64) {
        self.limit_samples.push(LimitSample {
            ts_ms: ts * 1000,
            session_id: "s1".into(),
            account_id: Some("oauth:me".into()),
            model: Some(OPUS.into()),
            window: window.into(),
            used_pct,
            resets_at_ms: Some(resets * 1000),
        });
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --lib insights::`
Expected: does not compile, among others `cannot find value LimitHits`, `cannot find value FIVE_HOUR` and `missing field limit_samples` in `report.rs`.

- [ ] **Step 3: Write the rule, rank it first, and read the account's samples**

Put this above the test module of `src/insights/hits.rs`:

```rust
use std::collections::BTreeMap;

use super::usage::TOP_TIER;
use super::words::{duration, model_name, times};
use super::{Confidence, Draft, Finding, InsightContext, Rule, RuleId, is_main, median};
use crate::forecast::{alternative_for, family};
use crate::history::FIVE_HOUR;
use crate::limits::instances;
use crate::store::LimitSample;

pub const SEVEN_DAY: &str = "seven_day";
pub const CAPPED_PCT: f64 = 100.0;
pub const SUBAGENT_LED: f64 = 0.5;
const HOUR_MS: i64 = 3_600_000;

pub struct LimitHits;

struct Hit {
    window: &'static str,
    start_ms: i64,
    capped_ms: i64,
    waited_ms: i64,
}

impl Rule for LimitHits {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        let hits: Vec<Hit> = instances(ctx.limit_samples)
            .into_iter()
            .filter_map(|instance| hit(ctx, instance))
            .collect();
        if hits.is_empty() {
            return Vec::new();
        }
        let mut evidence = Vec::new();
        for (window, name) in [(FIVE_HOUR, "5-hour"), (SEVEN_DAY, "weekly")] {
            let waits: Vec<f64> = hits
                .iter()
                .filter(|h| h.window == window)
                .map(|h| h.waited_ms as f64)
                .collect();
            let Some(wait) = median(waits.clone()) else {
                continue;
            };
            evidence.push(format!(
                "The {name} limit ran out {}, {}{} before it reset",
                times(waits.len() as u64),
                if waits.len() > 1 { "a typical " } else { "" },
                duration(wait as i64)
            ));
        }
        let mut by_model: BTreeMap<&str, f64> = BTreeMap::new();
        let (mut spent, mut subagents) = (0.0, 0.0);
        for request in ctx.requests {
            let at_ms = request.point.ts * 1000;
            if !hits
                .iter()
                .any(|h| h.start_ms <= at_ms && at_ms <= h.capped_ms)
            {
                continue;
            }
            let paid = request.cost_usd.unwrap_or(0.0);
            spent += paid;
            *by_model.entry(request.point.model.as_str()).or_default() += paid;
            if !is_main(request) {
                subagents += paid;
            }
        }
        let model = by_model
            .iter()
            .max_by(|a, b| a.1.total_cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(model, usd)| (*model, *usd));
        if let Some((model, usd)) = model.filter(|_| spent > 0.0) {
            evidence.push(format!(
                "{} drew {:.0}% of the spend before it ran out",
                model_name(model),
                usd / spent * 100.0
            ));
        }
        let subagent_share = if spent > 0.0 { subagents / spent } else { 0.0 };
        if subagent_share > 0.0 {
            evidence.push(format!(
                "Subagents drew {:.0}% of it",
                subagent_share * 100.0
            ));
        }
        let cheaper = model
            .map(|(model, _)| model)
            .filter(|model| TOP_TIER.contains(&family(model)))
            .and_then(alternative_for);
        let tip = match cheaper {
            Some(alternative) if subagent_share >= SUBAGENT_LED => format!(
                "run subagents on {} so your limits last",
                model_name(alternative)
            ),
            Some(alternative) => format!(
                "move routine work to {} so your limits last",
                model_name(alternative)
            ),
            None if hits.iter().all(|h| h.window == SEVEN_DAY) => {
                "pace heavy work across the week so the weekly limit lasts".into()
            }
            None => "spread heavy work across 5-hour windows so the limit lasts".into(),
        };
        vec![
            Draft {
                rule: RuleId::LimitHits,
                tip,
                evidence,
                confidence: Confidence::Medium,
                how: "Reads the 5-hour and weekly limits Claude Code reported for this account in the last 14 days. \
                      A limit ran out when it reached 100%, and the wait runs from then to its reset. \
                      The spend is what this account's requests cost at API prices in each window before its limit ran out. \
                      A limit that ran out stops your work, so this tip comes first."
                    .into(),
            }
            .into_finding(),
        ]
    }
}

fn hit(ctx: &InsightContext, instance: &[LimitSample]) -> Option<Hit> {
    let first = instance.first()?;
    let window = [FIVE_HOUR, SEVEN_DAY]
        .into_iter()
        .find(|window| *window == first.window)?;
    let capped = instance.iter().find(|s| s.used_pct >= CAPPED_PCT)?;
    let resets = instance.iter().rev().find_map(|s| s.resets_at_ms)?;
    (capped.ts_ms >= ctx.since_ts * 1000).then(|| Hit {
        window,
        start_ms: resets - length(window),
        capped_ms: capped.ts_ms,
        waited_ms: (resets.min(ctx.now_ms) - capped.ts_ms).max(0),
    })
}

fn length(window: &str) -> i64 {
    match window {
        SEVEN_DAY => 7 * 24 * HOUR_MS,
        _ => 5 * HOUR_MS,
    }
}
```

In `src/insights/mod.rs`, rank the limit first:

```rust
pub fn rank(mut findings: Vec<Finding>) -> Vec<Finding> {
    findings.sort_by(|a, b| {
        (b.rule == RuleId::LimitHits)
            .cmp(&(a.rule == RuleId::LimitHits))
            .then_with(|| {
                b.saving_usd
                    .unwrap_or(-1.0)
                    .total_cmp(&a.saving_usd.unwrap_or(-1.0))
            })
            .then_with(|| a.rule.cmp(&b.rule))
    });
    findings
}
```

In `src/insights/report.rs` `read`, after the `agent_types` line of Task 3:

```rust
    let limit_samples = match scope.account() {
        Some(account) => store
            .limit_samples_since(since * 1000)?
            .into_iter()
            .filter(|s| s.account_id.as_deref() == Some(account))
            .collect(),
        None => Vec::new(),
    };
```

and at the end of the `InsightContext { ... }` literal:

```rust
        limit_samples: &limit_samples,
```

`limit_samples_since` returns the samples ordered by account and window, then time, which is the order `instances` needs.

- [ ] **Step 4: Run the Rust gate**

Run: `cargo fmt && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Expected: PASS, 586 lib tests.

- [ ] **Step 5: Commit**

```bash
git add src/insights
git commit -m "feat(insights): lead with a 5-hour or weekly limit that ran out and what drew it down"
```

---

### Task 5: The mod learns the new rules, and the docs say so

**Files:**
- Modify: `mod/types/index.d.ts` (`RuleId`)
- Modify: `mod/tests/insights.test.ts`
- Modify: `README.md` (section "4. Insights")
- Create: `docs/manual-checks/m10-insights-limits-ttl-subagents.md`

**Interfaces:**
- Consumes: the report of Tasks 2 to 4, whose findings serialize the rule ids `subagent_model_fit`, `cache_ttl` and `limit_hits`.

- [ ] **Step 1: Write the failing mod test**

In `mod/tests/insights.test.ts`, before the test `the recap reads like a greeting and leaves out what it does not know`:

```ts
  test('a limit that ran out leads with its tip and evidence even without a saving', () => {
    const limit = finding({
      rule: 'limit_hits',
      tip: 'run subagents on Sonnet 5.5 so your limits last',
      evidence: ['The 5-hour limit ran out twice, a typical 2h before it reset'],
      saving_usd: null,
      saving_pct_5h: null,
      confidence: 'medium',
    })
    const lines = insightsScreen(input({ report: insightsReport({ findings: [limit, ...insightsReport().findings] }) }))
    const keys = lines.map(found => found.key).filter(key => /^ins-[a-z_]+$/.test(key) && !['ins-head', 'ins-sub'].includes(key))
    expect(keys[0]).toBe('ins-limit_hits')
    expect(text(line(lines, 'ins-limit_hits'))).toBe('Run subagents on Sonnet 5.5 so your limits last')
    expect(text(line(lines, 'ins-limit_hits-0'))).toBe('The 5-hour limit ran out twice, a typical 2h before it reset')
    expect(text(line(lines, 'ins-limit_hits-meta'))).toBe('  medium confidence')
  })
```

- [ ] **Step 2: Run the type check to see it fail**

Run: `export PATH="/opt/homebrew/bin:$HOME/.local/bin:$PATH" && npx --offline -p typescript@6.0.3 tsc -p mod-dev/tsconfig.json`
Expected: FAIL, `Type '"limit_hits"' is not assignable to type 'RuleId | undefined'`.

- [ ] **Step 3: Add the rule ids**

In `mod/types/index.d.ts`, make `RuleId`:

```ts
export type RuleId =
  | 'idle_rebuilds'
  | 'unused_mcp_servers'
  | 'unused_skills'
  | 'oversized_memory'
  | 'large_tool_results'
  | 'late_compaction'
  | 'model_fit'
  | 'subagent_model_fit'
  | 'cache_busts'
  | 'cache_ttl'
  | 'limit_hits'
  | 'costly_tasks'
  | 'retention'
```

In `README.md`, under `### 4. Insights`, after `Each tip names the sessions and files behind it, so you can see where the tokens went.`:

```markdown
A 5-hour or weekly limit that ran out in the last two weeks comes first, with what drew it down.
```

- [ ] **Step 4: Write the manual check**

Create `docs/manual-checks/m10-insights-limits-ttl-subagents.md`:

````markdown
# Morning check for the limit, cache lifetime and subagent model tips

Run this by hand in a real terminal, after Tasks 1 to 4 of the plan are on `main`.

## Setup

Build the daemon and restart it, so it moves the store to schema 19 and reads every subagent's meta file.

```bash
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && cargo build --release --locked
./target/release/claude-plus daemon stop
./target/release/claude-plus daemon --ensure
./target/release/claude-plus doctor
PATH="$PWD/target/release:$PATH" claude --plugin-dir "$PWD/mod"
```

## Checklist

- [ ] `sqlite3 ~/.claude-plus/data.db "select agent_type, count(*) from agents group by 1 order by 2 desc"` lists your agent types (for example `general-purpose`, `Explore`), and no row holds a description.
- [ ] Press `4`: when your subagents ran on Opus, a tip reads `Save ~$x/week: run <type> subagents on Sonnet 5.5`, its evidence lists up to three agent types with `runs on Opus 5.5 cost ...`, and `Subagents drew n% of your spend`.
- [ ] The `?` of that tip says how it was priced and how to set `model:`, at `low confidence` for a type that writes code.
- [ ] No cache lifetime tip shows while your main thread is on the 1-hour cache and often waits on subagents; when one shows, its evidence names `Main thread` or `Subagents` and its `?` names `promptCacheTtl` or `subagentPromptCacheTtl`.
- [ ] After a day where a 5-hour or weekly limit ran out, the first tip is `Run subagents on ...`, `Move routine work to ...` or a pacing tip, without a saving, above every priced tip, and its first evidence line says how often the limit ran out and how long before it reset.
- [ ] A session of another account never shows that account's limit tip.
- [ ] At 40, 60 and 100 columns, in `dark`, `light` and `dark-ansi`, no new tip is cut and no value splits from its unit across rows.
````

- [ ] **Step 5: Run the mod gate and the Rust gate**

Run: `export PATH="/opt/homebrew/bin:$HOME/.local/bin:$PATH" && claude plugin validate mod && claude plugin test mod && claude plugin test mod && claude plugin test mod && npx --offline -p typescript@6.0.3 tsc -p mod-dev/tsconfig.json`
Expected: validation passed, 460 pass and 0 fail on each of the three runs, tsc silent.

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Expected: PASS, 586 lib tests.

- [ ] **Step 6: Commit**

```bash
git add mod/types/index.d.ts mod/tests/insights.test.ts README.md docs/manual-checks/m10-insights-limits-ttl-subagents.md
git commit -m "feat(mod): show the limit, cache lifetime and subagent model tips"
```

---

### Task 6: Morning check (the user)

- [ ] Run `docs/manual-checks/m10-insights-limits-ttl-subagents.md` by hand, then tick its boxes.
