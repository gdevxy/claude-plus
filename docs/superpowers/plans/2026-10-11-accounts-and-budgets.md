# claude-plus Accounts and Budgets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** claude-plus attributes every new session to the account it bills to (claude.ai login, API key, cloud provider or gateway), keeps limits, calibration, History, Insights, `stats` and `export` per account, shows an enterprise monthly spend limit in dollars against a budget set with `claude-plus budget set`, and prices gateway and cloud model ids, with an optional user price file over the bundled prices.

**Architecture:** A new unit `src/accounts.rs` reads the `oauthAccount` block of Claude Code's `.claude.json` read-only and keeps an account timeline (`account_periods`), and resolves a session's account from the `auth` object the mod adds to `session.start`, falling back to the timeline.
Attribution lives in a non-derived table `session_accounts`, because `sessions` is cleared by `reindex`; `limit_samples` gains `account_id`.
`limits` splits instances, fits and windows by account and adds a monthly pace for `spend_limit` and budgets; the snapshot gains `live.account` and `live.budget`; History, Insights, `stats` and `export` read through a `Scope`.
The mod sends `auth`, shows the account, renders the spend and budget bars, raises budget alerts, and cycles History accounts with `a`.

**Tech Stack:** Rust (edition 2024, MSRV 1.88), no new crates (rusqlite, chrono with `std` and `clock`, serde, clap); Claude Code 2.1.296 function hooks (TypeScript, `$.env.get`), `claude plugin validate mod`, `claude plugin test mod`, TypeScript 6.0.3 via `npx --offline -p typescript@6.0.3 tsc` for type-checking only.

**Spec:** `docs/superpowers/specs/2026-10-11-accounts-and-budgets-design.md`

This plan starts from `main` at `7aa9ccc` plus the spec amendment that follows it.
If `main` has moved, apply the hunks by the code they quote, not by line number.

## Global Constraints

- claude-plus makes no network requests and never writes Claude Code's files; `.claude.json` is only read.
- The email address from `oauthAccount` is never read into memory structures or stored; only `accountUuid`, `organizationName`, `organizationType`, `billingType` are kept.
- The values of `ANTHROPIC_API_KEY` and `ANTHROPIC_AUTH_TOKEN` are never sent; only whether they are set.
- The bundled Anthropic prices stay the default; `$CLAUDE_PLUS_HOME/prices.toml` is optional and never created.
- Budget period is the calendar month in UTC: it starts on the 1st at 00:00 UTC.
- Budget amounts must be finite and above zero.
- No em dash anywhere in code, strings or docs; use `-`.
- Every Markdown file edited puts each sentence on its own line.
- No comments in code unless absolutely necessary.
- The Rust gate after every task: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check`.
- The mod gate after every mod task: `claude plugin validate mod && claude plugin test mod && (cd mod && npx --offline -p typescript@6.0.3 tsc --noEmit -p .)`.
- Commit messages follow the repository's `type(scope): summary` style and end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Decisions

The spec is silent on these points, so this plan decides them.

1. Account ids: `oauth:<accountUuid>`, `api`, `cloud:<provider>`, `gateway:<scheme>://<host>:<port>` with host lower-cased and the default port filled in (443 for https, 80 for http).
2. The History route takes `account=all` or `account=<percent-encoded id>`; absent means `all`, so older panes keep today's behaviour.
   The mod's default choice is `current`, which it resolves to `snapshot.live.account.id`, or `all` when the session has no account.
3. "Current account" in the CLI is the account of the most recently started attributed session, else the account of the open OAuth period, else none (then `--account` is required for `budget set`).
4. With scope `all`, History shows no `% of 5h` metric and no 5h peak, because percentages of different accounts' windows cannot be added.
5. The account label is shown on its own dim line under the headline (`account` key), not appended to the model, so the headline keeps its width budget at 24 columns.
6. Meter labels pad to the longest shown window name, at least 5, so `Budget` lines up and existing layouts without it do not move.
7. A budget window without a reported spend limit is synthesized by the daemon as `window: "budget"`, so the mod draws it with the same meter code.

## Review Focus

1. A user who switches accounts mid-window and back: the private 5h series resumes as the same instance when its `resetsAt` is unchanged (Task 4 test `switching_accounts_and_back_resumes_the_same_instance`).
2. A `.claude.json` being rewritten by Claude Code at the moment it is read (truncated JSON): no period is closed (Task 2 test `a_malformed_or_missing_file_changes_nothing`).
3. A `session.start` arriving after the transcript was already ingested and timeline-attributed: the start attribution replaces it (Task 1 test `a_start_attribution_replaces_a_timeline_one_but_not_the_reverse`).
4. Spend limit percent above 100: the bar is full, in warning colour, with the real percent and the dollar figure computed from it (Task 10 test `an_exceeded_spend_limit_draws_full_with_its_real_percent`, and Task 5 must not cap `used_pct`).
5. The first hours of a month: no pace is shown before one full day, and the period boundary is UTC midnight, not local (Task 4 test `monthly_pace_waits_a_full_day_and_uses_utc_months`).

---

### Task 1: Schema and store for accounts, periods, attribution and budgets

**Files:**
- Modify: `src/store/schema.rs` (append one migration to `MIGRATIONS`)
- Create: `src/store/accounts.rs`
- Modify: `src/store/mod.rs` (declare `mod accounts;` and re-export)
- Modify: `src/store/limits.rs` (`LimitSample.account_id`, dedupe and ordering per account, `SpendPoint.session_id`)
- Modify every `LimitSample { .. }` literal (`src/limits.rs` tests, `src/store/limits.rs` tests, `src/insights/fixture.rs`, `src/history/tests.rs`) to add `account_id: None`

**Interfaces:**
- Produces:
  - `pub struct AccountRow { pub id: String, pub kind: AccountKind, pub label: String, pub organization_type: Option<String>, pub billing_type: Option<String> }`
  - `pub enum AccountKind { Oauth, Api, Cloud, Gateway }` with `as_str()` and `parse(&str) -> Option<Self>`
  - `pub struct AccountInfo { pub id: String, pub kind: AccountKind, pub label: String, pub organization_type: Option<String>, pub first_seen_ms: i64 }`
  - `pub enum AttributionSource { Start, Timeline }`
  - `pub struct Budget { pub account_id: String, pub amount_usd: f64, pub updated_ms: i64 }`
  - Writer: `upsert_account(&AccountRow, now_ms) -> Result<()>`, `open_period(&str, from_ms) -> Result<()>`, `close_open_period(at_ms) -> Result<Option<String>>`, `attribute(session, account, AttributionSource) -> Result<bool>`, `rename_account(id, label) -> Result<bool>`, `set_budget(id, usd, now_ms) -> Result<()>`, `clear_budget(id) -> Result<bool>`, `open_account() -> Result<Option<String>>`, `session_account(session) -> Result<Option<String>>`
  - Store: `accounts() -> Result<Vec<AccountInfo>>`, `account(id) -> Result<Option<AccountInfo>>`, `open_account()`, `session_account(session)`, `account_at(ms) -> Result<Option<String>>`, `unattributed_sessions() -> Result<Vec<(String, i64)>>`, `session_accounts() -> Result<HashMap<String, String>>`, `sessions_of(account) -> Result<HashSet<String>>`, `latest_session_account() -> Result<Option<String>>`, `budget(id) -> Result<Option<Budget>>`, `account_spend_since(account, since_ts) -> Result<f64>`, `account_sessions_count(account) -> Result<u64>`
  - `LimitSample.account_id: Option<String>`, `SpendPoint.session_id: String`

- [ ] **Step 1: Write the failing store tests**

Create `src/store/accounts.rs` with only the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{SessionRow, Store};

    fn oauth(id: &str, label: &str) -> AccountRow {
        AccountRow {
            id: format!("oauth:{id}"),
            kind: AccountKind::Oauth,
            label: label.into(),
            organization_type: Some("claude_enterprise".into()),
            billing_type: None,
        }
    }

    fn session(store: &mut Store, id: &str, started_at: i64) {
        let mut w = store.writer().unwrap();
        w.session(&SessionRow {
            id: id.into(),
            project: "p".into(),
            cwd: None,
            started_at: Some(started_at),
        })
        .unwrap();
        w.commit().unwrap();
    }

    #[test]
    fn periods_open_and_close_and_answer_which_account_was_active() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("a", "Acme · Enterprise"), 1_000).unwrap();
        w.upsert_account(&oauth("b", "Me · Max"), 1_000).unwrap();
        w.open_period("oauth:a", 1_000).unwrap();
        assert_eq!(w.close_open_period(5_000).unwrap().as_deref(), Some("oauth:a"));
        w.open_period("oauth:b", 5_000).unwrap();
        w.commit().unwrap();
        assert_eq!(store.account_at(999).unwrap(), None);
        assert_eq!(store.account_at(1_000).unwrap().as_deref(), Some("oauth:a"));
        assert_eq!(store.account_at(4_999).unwrap().as_deref(), Some("oauth:a"));
        assert_eq!(store.account_at(5_000).unwrap().as_deref(), Some("oauth:b"));
        assert_eq!(store.account_at(i64::MAX).unwrap().as_deref(), Some("oauth:b"));
        assert_eq!(store.open_account().unwrap().as_deref(), Some("oauth:b"));
    }

    #[test]
    fn a_start_attribution_replaces_a_timeline_one_but_not_the_reverse() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("a", "A"), 0).unwrap();
        w.upsert_account(&oauth("b", "B"), 0).unwrap();
        assert!(w.attribute("s1", "oauth:a", AttributionSource::Timeline).unwrap());
        assert!(w.attribute("s1", "oauth:b", AttributionSource::Start).unwrap());
        assert!(!w.attribute("s1", "oauth:a", AttributionSource::Timeline).unwrap());
        assert!(!w.attribute("s1", "oauth:a", AttributionSource::Start).unwrap());
        w.commit().unwrap();
        assert_eq!(store.session_account("s1").unwrap().as_deref(), Some("oauth:b"));
    }

    #[test]
    fn unattributed_sessions_are_those_with_a_start_and_no_account() {
        let mut store = Store::open_in_memory().unwrap();
        session(&mut store, "s1", 10);
        session(&mut store, "s2", 20);
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("a", "A"), 0).unwrap();
        w.attribute("s1", "oauth:a", AttributionSource::Timeline).unwrap();
        w.commit().unwrap();
        assert_eq!(store.unattributed_sessions().unwrap(), vec![("s2".to_string(), 20)]);
        assert_eq!(store.latest_session_account().unwrap().as_deref(), Some("oauth:a"));
    }

    #[test]
    fn budgets_are_set_replaced_and_cleared() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("a", "A"), 0).unwrap();
        w.set_budget("oauth:a", 500.0, 1).unwrap();
        w.set_budget("oauth:a", 700.0, 2).unwrap();
        w.commit().unwrap();
        let budget = store.budget("oauth:a").unwrap().unwrap();
        assert_eq!((budget.amount_usd, budget.updated_ms), (700.0, 2));
        let mut w = store.writer().unwrap();
        assert!(w.clear_budget("oauth:a").unwrap());
        assert!(!w.clear_budget("oauth:a").unwrap());
        w.commit().unwrap();
        assert_eq!(store.budget("oauth:a").unwrap(), None);
    }

    #[test]
    fn upserting_keeps_a_custom_label_and_refreshes_the_plan() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("a", "Acme · Enterprise"), 1).unwrap();
        assert!(w.rename_account("oauth:a", "Work").unwrap());
        let mut changed = oauth("a", "Acme · Team");
        changed.organization_type = Some("claude_team".into());
        w.upsert_account(&changed, 2).unwrap();
        w.commit().unwrap();
        let info = store.account("oauth:a").unwrap().unwrap();
        assert_eq!(info.label, "Work");
        assert_eq!(info.organization_type.as_deref(), Some("claude_team"));
        assert_eq!(info.first_seen_ms, 1);
    }

    #[test]
    fn an_automatic_label_follows_the_account() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("a", "Acme · Enterprise"), 1).unwrap();
        w.upsert_account(&oauth("a", "Acme Corp · Enterprise"), 2).unwrap();
        w.commit().unwrap();
        assert_eq!(store.account("oauth:a").unwrap().unwrap().label, "Acme Corp · Enterprise");
    }
}
```

Check the exact fields of `SessionRow` in `src/store/writer.rs:24` and adapt the literal if they differ (for example a `last_seen_at` field); keep `started_at` in seconds as the table stores it.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test --lib store::accounts`
Expected: compile errors for the missing types and methods.

- [ ] **Step 3: Add the migration**

Append to `MIGRATIONS` in `src/store/schema.rs`:

```rust
    r#"
CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    label TEXT NOT NULL,
    label_is_custom INTEGER NOT NULL DEFAULT 0,
    organization_type TEXT,
    billing_type TEXT,
    first_seen_ms INTEGER NOT NULL
);
CREATE TABLE account_periods (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL,
    from_ms INTEGER NOT NULL,
    to_ms INTEGER
);
CREATE INDEX account_periods_by_time ON account_periods(from_ms);
CREATE TABLE session_accounts (
    session_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    source TEXT NOT NULL
);
CREATE INDEX session_accounts_by_account ON session_accounts(account_id);
CREATE TABLE budgets (
    account_id TEXT PRIMARY KEY,
    amount_usd REAL NOT NULL,
    period TEXT NOT NULL,
    updated_ms INTEGER NOT NULL
);
ALTER TABLE limit_samples ADD COLUMN account_id TEXT;
CREATE INDEX limit_samples_by_account ON limit_samples(account_id, window, ts_ms);
"#,
```

None of these tables join `DERIVED_TABLES`: they are not rebuilt from transcripts.

- [ ] **Step 4: Implement `src/store/accounts.rs`**

Above the test module:

```rust
use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

use super::{Store, Writer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    Oauth,
    Api,
    Cloud,
    Gateway,
}

impl AccountKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AccountKind::Oauth => "oauth",
            AccountKind::Api => "api",
            AccountKind::Cloud => "cloud",
            AccountKind::Gateway => "gateway",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Self::Oauth, Self::Api, Self::Cloud, Self::Gateway]
            .into_iter()
            .find(|kind| kind.as_str() == text)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountRow {
    pub id: String,
    pub kind: AccountKind,
    pub label: String,
    pub organization_type: Option<String>,
    pub billing_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountInfo {
    pub id: String,
    pub kind: AccountKind,
    pub label: String,
    pub organization_type: Option<String>,
    pub first_seen_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributionSource {
    Start,
    Timeline,
}

impl AttributionSource {
    fn as_str(self) -> &'static str {
        match self {
            AttributionSource::Start => "start",
            AttributionSource::Timeline => "timeline",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Budget {
    pub account_id: String,
    pub amount_usd: f64,
    pub updated_ms: i64,
}

const ACCOUNT_COLUMNS: &str = "id, kind, label, organization_type, first_seen_ms";

impl Writer<'_> {
    pub fn upsert_account(&mut self, row: &AccountRow, now_ms: i64) -> Result<()> {
        self.tx
            .prepare_cached(
                "INSERT INTO accounts (id, kind, label, organization_type, billing_type, first_seen_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (id) DO UPDATE SET
                     label = CASE WHEN label_is_custom = 1 THEN label ELSE excluded.label END,
                     organization_type = excluded.organization_type,
                     billing_type = excluded.billing_type",
            )?
            .execute(params![
                row.id,
                row.kind.as_str(),
                row.label,
                row.organization_type,
                row.billing_type,
                now_ms
            ])?;
        Ok(())
    }

    pub fn open_period(&mut self, account_id: &str, from_ms: i64) -> Result<()> {
        self.tx
            .prepare_cached("INSERT INTO account_periods (account_id, from_ms) VALUES (?1, ?2)")?
            .execute(params![account_id, from_ms])?;
        Ok(())
    }

    pub fn close_open_period(&mut self, at_ms: i64) -> Result<Option<String>> {
        let open = open_account(&self.tx)?;
        self.tx
            .prepare_cached("UPDATE account_periods SET to_ms = ?1 WHERE to_ms IS NULL")?
            .execute([at_ms])?;
        Ok(open)
    }

    pub fn attribute(
        &mut self,
        session_id: &str,
        account_id: &str,
        source: AttributionSource,
    ) -> Result<bool> {
        let changed = self
            .tx
            .prepare_cached(
                "INSERT INTO session_accounts (session_id, account_id, source) VALUES (?1, ?2, ?3)
                 ON CONFLICT (session_id) DO UPDATE SET account_id = excluded.account_id, source = excluded.source
                 WHERE session_accounts.source = 'timeline' AND excluded.source = 'start'",
            )?
            .execute(params![session_id, account_id, source.as_str()])?;
        Ok(changed > 0)
    }

    pub fn rename_account(&mut self, account_id: &str, label: &str) -> Result<bool> {
        let changed = self
            .tx
            .prepare_cached("UPDATE accounts SET label = ?2, label_is_custom = 1 WHERE id = ?1")?
            .execute(params![account_id, label])?;
        Ok(changed > 0)
    }

    pub fn set_budget(&mut self, account_id: &str, amount_usd: f64, now_ms: i64) -> Result<()> {
        self.tx
            .prepare_cached(
                "INSERT OR REPLACE INTO budgets (account_id, amount_usd, period, updated_ms)
                 VALUES (?1, ?2, 'monthly', ?3)",
            )?
            .execute(params![account_id, amount_usd, now_ms])?;
        Ok(())
    }

    pub fn clear_budget(&mut self, account_id: &str) -> Result<bool> {
        let removed = self
            .tx
            .prepare_cached("DELETE FROM budgets WHERE account_id = ?1")?
            .execute([account_id])?;
        Ok(removed > 0)
    }

    pub fn open_account(&self) -> Result<Option<String>> {
        open_account(&self.tx)
    }

    pub fn session_account(&self, session_id: &str) -> Result<Option<String>> {
        session_account(&self.tx, session_id)
    }

    pub fn account_at(&self, at_ms: i64) -> Result<Option<String>> {
        account_at(&self.tx, at_ms)
    }
}

impl Store {
    pub fn accounts(&self) -> Result<Vec<AccountInfo>> {
        let mut statement = self.conn.prepare_cached(&format!(
            "SELECT {ACCOUNT_COLUMNS} FROM accounts ORDER BY first_seen_ms, id"
        ))?;
        let rows = statement.query_map([], read_account)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn account(&self, account_id: &str) -> Result<Option<AccountInfo>> {
        Ok(self
            .conn
            .prepare_cached(&format!("SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = ?1"))?
            .query_row([account_id], read_account)
            .optional()?)
    }

    pub fn open_account(&self) -> Result<Option<String>> {
        open_account(&self.conn)
    }

    pub fn session_account(&self, session_id: &str) -> Result<Option<String>> {
        session_account(&self.conn, session_id)
    }

    pub fn account_at(&self, at_ms: i64) -> Result<Option<String>> {
        account_at(&self.conn, at_ms)
    }

    pub fn unattributed_sessions(&self) -> Result<Vec<(String, i64)>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT s.id, s.started_at FROM sessions s
             LEFT JOIN session_accounts a ON a.session_id = s.id
             WHERE a.session_id IS NULL AND s.started_at IS NOT NULL ORDER BY s.started_at, s.id",
        )?;
        let rows = statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn session_accounts(&self) -> Result<HashMap<String, String>> {
        let mut statement = self
            .conn
            .prepare_cached("SELECT session_id, account_id FROM session_accounts")?;
        let rows = statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn sessions_of(&self, account_id: &str) -> Result<HashSet<String>> {
        let mut statement = self
            .conn
            .prepare_cached("SELECT session_id FROM session_accounts WHERE account_id = ?1")?;
        let rows = statement.query_map([account_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn latest_session_account(&self) -> Result<Option<String>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT a.account_id FROM session_accounts a JOIN sessions s ON s.id = a.session_id
                 ORDER BY s.started_at DESC, s.id DESC LIMIT 1",
            )?
            .query_row([], |r| r.get(0))
            .optional()?)
    }

    pub fn budget(&self, account_id: &str) -> Result<Option<Budget>> {
        Ok(self
            .conn
            .prepare_cached("SELECT account_id, amount_usd, updated_ms FROM budgets WHERE account_id = ?1")?
            .query_row([account_id], |r| {
                Ok(Budget {
                    account_id: r.get(0)?,
                    amount_usd: r.get(1)?,
                    updated_ms: r.get(2)?,
                })
            })
            .optional()?)
    }

    pub fn account_spend_since(&self, account_id: &str, since_ts: i64) -> Result<f64> {
        let mut total = 0.0;
        for table in ["requests", "adjustments"] {
            total += self
                .conn
                .prepare_cached(&format!(
                    "SELECT COALESCE(SUM(t.cost_usd), 0) FROM {table} t
                     JOIN session_accounts a ON a.session_id = t.session_id
                     WHERE a.account_id = ?1 AND t.ts >= ?2"
                ))?
                .query_row(params![account_id, since_ts], |r| r.get::<_, f64>(0))?;
        }
        Ok(total)
    }

    pub fn account_sessions_count(&self, account_id: &str) -> Result<u64> {
        Ok(self
            .conn
            .prepare_cached("SELECT COUNT(*) FROM session_accounts WHERE account_id = ?1")?
            .query_row([account_id], |r| r.get::<_, i64>(0))? as u64)
    }
}

fn open_account(conn: &Connection) -> Result<Option<String>> {
    Ok(conn
        .prepare_cached(
            "SELECT account_id FROM account_periods WHERE to_ms IS NULL ORDER BY from_ms DESC, id DESC LIMIT 1",
        )?
        .query_row([], |r| r.get(0))
        .optional()?)
}

fn session_account(conn: &Connection, session_id: &str) -> Result<Option<String>> {
    Ok(conn
        .prepare_cached("SELECT account_id FROM session_accounts WHERE session_id = ?1")?
        .query_row([session_id], |r| r.get(0))
        .optional()?)
}

fn account_at(conn: &Connection, at_ms: i64) -> Result<Option<String>> {
    Ok(conn
        .prepare_cached(
            "SELECT account_id FROM account_periods
             WHERE from_ms <= ?1 AND (to_ms IS NULL OR to_ms > ?1)
             ORDER BY from_ms DESC, id DESC LIMIT 1",
        )?
        .query_row([at_ms], |r| r.get(0))
        .optional()?)
}

fn read_account(r: &rusqlite::Row) -> rusqlite::Result<AccountInfo> {
    let kind: String = r.get(1)?;
    Ok(AccountInfo {
        id: r.get(0)?,
        kind: AccountKind::parse(&kind).unwrap_or(AccountKind::Oauth),
        label: r.get(2)?,
        organization_type: r.get(3)?,
        first_seen_ms: r.get(4)?,
    })
}
```

`Writer.tx` is a `rusqlite::Transaction`, which derefs to `Connection`, so the free functions serve both.
If `adjustments` has no `session_id` column, check `src/store/schema.rs` (it does: `PRIMARY KEY (session_id, model, ts)`).

In `src/store/mod.rs` add `mod accounts;` and `pub use accounts::{AccountInfo, AccountKind, AccountRow, AttributionSource, Budget};`.

- [ ] **Step 5: Carry the account on limit samples and session on spend points**

In `src/store/limits.rs`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct LimitSample {
    pub ts_ms: i64,
    pub session_id: String,
    pub account_id: Option<String>,
    pub model: Option<String>,
    pub window: String,
    pub used_pct: f64,
    pub resets_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpendPoint {
    pub ts_ms: i64,
    pub session_id: String,
    pub model: String,
    pub cost_usd: f64,
}
```

`repeats` also requires `self.account_id == other.account_id`.
The dedupe lookup in `Writer::limit_sample` becomes `WHERE window = ?1 AND account_id IS ?3 AND ts_ms <= ?2` with `params![sample.window, sample.ts_ms, sample.account_id]`, and the insert writes `account_id`.
Every `SELECT` of samples reads `ts_ms, session_id, account_id, model, window, used_pct, resets_at_ms` and `read_sample` maps the seven columns in that order.
`limit_samples_since` orders by `account_id, window, ts_ms, id`, so each account's series of a window is contiguous.
`limit_samples_between` in `src/store/history.rs` keeps its order and also selects `account_id`.
`SPEND_SINCE` becomes `SELECT ts, session_id, model, cost_usd FROM requests WHERE ts >= (?1 + 999) / 1000 AND cost_usd IS NOT NULL ORDER BY ts`, mapped in that order.

Add `account_id: None` to every existing `LimitSample` literal and `session_id: "s1".into()` to every `SpendPoint` literal (`rg -n "LimitSample \{|SpendPoint \{" src`).

Add to the `src/store/limits.rs` tests:

```rust
    #[test]
    fn the_same_reading_from_another_account_is_kept() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let mine = sample(1_000, "five_hour", 10.0, Some(9_000_000));
        let theirs = LimitSample { account_id: Some("oauth:b".into()), ts_ms: 2_000, ..mine.clone() };
        assert!(w.limit_sample(&mine).unwrap());
        assert!(w.limit_sample(&theirs).unwrap());
        w.commit().unwrap();
        assert_eq!(store.limit_samples_since(0).unwrap().len(), 2);
    }
```

- [ ] **Step 6: Run the store tests**

Run: `cargo test --lib store::`
Expected: PASS, including the six new account tests and the sample test.

- [ ] **Step 7: Run the Rust gate and commit**

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check
git add src/store
git commit -m "feat(store): keep accounts, their periods, session attribution and budgets

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Account unit: read the login, keep the timeline, resolve sessions

**Files:**
- Create: `src/accounts.rs`
- Modify: `src/lib.rs` (`pub mod accounts;`)

**Interfaces:**
- Consumes: Task 1 store API.
- Produces:
  - `pub enum Login { SignedIn(AccountRow), SignedOut }`
  - `pub fn read_login(claude_json: &Path) -> Result<Login>` (Err when missing, unreadable or malformed)
  - `pub fn plan_name(organization_type: &str) -> String`
  - `pub fn observe(store: &mut Store, claude_json: &Path, now_ms: i64) -> Result<Option<String>>` returns the open OAuth account after observing; an Err leaves the store unchanged
  - `#[derive(Deserialize)] pub struct Auth { pub provider: String, pub base_url: Option<String>, pub api_key: bool }`
  - `pub fn auth_account(auth: &Auth) -> Option<AccountRow>`
  - `pub fn attribute_start(writer: &mut Writer, session: &str, auth: Option<&Auth>, open: Option<&str>, now_ms: i64) -> Result<()>`
  - `pub fn attribute_pending(store: &mut Store) -> Result<usize>`
  - `pub fn account_for_sample(writer: &mut Writer, session: &str, at_ms: i64) -> Result<Option<String>>`

- [ ] **Step 1: Write the failing tests**

At the bottom of `src/accounts.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SessionRow;

    fn login_file(dir: &Path, body: &str) -> std::path::PathBuf {
        let path = dir.join(".claude.json");
        std::fs::write(&path, body).unwrap();
        path
    }

    const ACME: &str = r#"{"oauthAccount":{"accountUuid":"u-1","emailAddress":"x@y","organizationName":"Acme","organizationType":"claude_enterprise","billingType":"invoice"}}"#;
    const MINE: &str = r#"{"oauthAccount":{"accountUuid":"u-2","organizationName":"Me","organizationType":"claude_max","billingType":"stripe_subscription"}}"#;

    #[test]
    fn a_login_is_read_without_the_email() {
        let dir = tempfile::tempdir().unwrap();
        let Login::SignedIn(row) = read_login(&login_file(dir.path(), ACME)).unwrap() else {
            panic!("signed in expected");
        };
        assert_eq!(
            row,
            AccountRow {
                id: "oauth:u-1".into(),
                kind: AccountKind::Oauth,
                label: "Acme · Enterprise".into(),
                organization_type: Some("claude_enterprise".into()),
                billing_type: Some("invoice".into()),
            }
        );
        assert_eq!(read_login(&login_file(dir.path(), "{}")).unwrap(), Login::SignedOut);
    }

    #[test]
    fn plans_have_friendly_names_and_unknown_ones_stay_raw() {
        assert_eq!(plan_name("claude_max"), "Max");
        assert_eq!(plan_name("claude_pro"), "Pro");
        assert_eq!(plan_name("claude_team"), "Team");
        assert_eq!(plan_name("claude_enterprise"), "Enterprise");
        assert_eq!(plan_name("claude_edu"), "claude_edu");
    }

    #[test]
    fn switching_logins_closes_one_period_and_opens_the_next() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_in_memory().unwrap();
        let path = login_file(dir.path(), ACME);
        assert_eq!(observe(&mut store, &path, 1_000).unwrap().as_deref(), Some("oauth:u-1"));
        assert_eq!(observe(&mut store, &path, 2_000).unwrap().as_deref(), Some("oauth:u-1"));
        login_file(dir.path(), MINE);
        assert_eq!(observe(&mut store, &path, 3_000).unwrap().as_deref(), Some("oauth:u-2"));
        login_file(dir.path(), "{}");
        assert_eq!(observe(&mut store, &path, 4_000).unwrap(), None);
        assert_eq!(store.account_at(2_500).unwrap().as_deref(), Some("oauth:u-1"));
        assert_eq!(store.account_at(3_500).unwrap().as_deref(), Some("oauth:u-2"));
        assert_eq!(store.account_at(4_500).unwrap(), None);
    }

    #[test]
    fn a_malformed_or_missing_file_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_in_memory().unwrap();
        let path = login_file(dir.path(), ACME);
        observe(&mut store, &path, 1_000).unwrap();
        login_file(dir.path(), r#"{"oauthAccount":{"accountUu"#);
        assert!(observe(&mut store, &path, 2_000).is_err());
        std::fs::remove_file(&path).unwrap();
        assert!(observe(&mut store, &path, 3_000).is_err());
        assert_eq!(store.open_account().unwrap().as_deref(), Some("oauth:u-1"));
    }

    #[test]
    fn auth_resolves_cloud_then_gateway_then_api_then_nothing() {
        let auth = |provider: &str, base_url: Option<&str>, api_key: bool| Auth {
            provider: provider.into(),
            base_url: base_url.map(String::from),
            api_key,
        };
        let id = |auth: Auth| auth_account(&auth).map(|row| (row.id, row.label));
        assert_eq!(id(auth("bedrock", Some("https://gw"), true)), Some(("cloud:bedrock".into(), "Bedrock".into())));
        assert_eq!(
            id(auth("anthropic", Some("HTTPS://LLM.Corp.example/v1"), true)),
            Some(("gateway:https://llm.corp.example:443".into(), "llm.corp.example".into()))
        );
        assert_eq!(
            id(auth("anthropic", Some("http://localhost:4000"), false)),
            Some(("gateway:http://localhost:4000".into(), "localhost".into()))
        );
        assert_eq!(id(auth("anthropic", None, true)), Some(("api".into(), "API key".into())));
        assert_eq!(id(auth("anthropic", Some(""), false)), None);
    }

    #[test]
    fn pending_sessions_take_the_account_of_their_start_time() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_in_memory().unwrap();
        let path = login_file(dir.path(), ACME);
        observe(&mut store, &path, 10_000).unwrap();
        login_file(dir.path(), MINE);
        observe(&mut store, &path, 20_000).unwrap();
        let mut w = store.writer().unwrap();
        for (id, started_at) in [("before", 5), ("acme", 15), ("mine", 25)] {
            w.session(&SessionRow { id: id.into(), project: "p".into(), cwd: None, started_at: Some(started_at) })
                .unwrap();
        }
        w.commit().unwrap();
        assert_eq!(attribute_pending(&mut store).unwrap(), 2);
        assert_eq!(store.session_account("before").unwrap(), None);
        assert_eq!(store.session_account("acme").unwrap().as_deref(), Some("oauth:u-1"));
        assert_eq!(store.session_account("mine").unwrap().as_deref(), Some("oauth:u-2"));
    }

    #[test]
    fn a_start_without_auth_takes_the_open_login_and_with_auth_its_own_account() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.upsert_account(
            &AccountRow { id: "oauth:u-1".into(), kind: AccountKind::Oauth, label: "A".into(), organization_type: None, billing_type: None },
            0,
        )
        .unwrap();
        attribute_start(&mut w, "s1", None, Some("oauth:u-1"), 0).unwrap();
        let gateway = Auth { provider: "anthropic".into(), base_url: Some("https://gw".into()), api_key: false };
        attribute_start(&mut w, "s2", Some(&gateway), Some("oauth:u-1"), 0).unwrap();
        attribute_start(&mut w, "s3", None, None, 0).unwrap();
        w.commit().unwrap();
        assert_eq!(store.session_account("s1").unwrap().as_deref(), Some("oauth:u-1"));
        assert_eq!(store.session_account("s2").unwrap().as_deref(), Some("gateway:https://gw:443"));
        assert_eq!(store.session_account("s3").unwrap(), None);
        assert_eq!(store.account("gateway:https://gw:443").unwrap().unwrap().label, "gw");
    }
}
```

Use the `SessionRow` fields as found in Task 1.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --lib accounts::tests`
Expected: compile errors.

- [ ] **Step 3: Implement `src/accounts.rs`**

```rust
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::store::{AccountKind, AccountRow, AttributionSource, Store, Writer};

#[derive(Debug, Clone, PartialEq)]
pub enum Login {
    SignedIn(AccountRow),
    SignedOut,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Auth {
    pub provider: String,
    pub base_url: Option<String>,
    pub api_key: bool,
}

pub fn read_login(claude_json: &Path) -> Result<Login> {
    let text = std::fs::read_to_string(claude_json)
        .with_context(|| format!("cannot read {}", claude_json.display()))?;
    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("{} is not valid JSON", claude_json.display()))?;
    let Some(account) = value.get("oauthAccount").filter(|a| a.is_object()) else {
        return Ok(Login::SignedOut);
    };
    let text_of = |key: &str| account.get(key).and_then(Value::as_str).map(String::from);
    let uuid = text_of("accountUuid").context("oauthAccount has no accountUuid")?;
    let organization_type = text_of("organizationType");
    let label = match (text_of("organizationName"), organization_type.as_deref()) {
        (Some(name), Some(kind)) => format!("{name} · {}", plan_name(kind)),
        (Some(name), None) => name,
        (None, Some(kind)) => plan_name(kind),
        (None, None) => "Claude account".to_string(),
    };
    Ok(Login::SignedIn(AccountRow {
        id: format!("oauth:{uuid}"),
        kind: AccountKind::Oauth,
        label,
        organization_type,
        billing_type: text_of("billingType"),
    }))
}

pub fn plan_name(organization_type: &str) -> String {
    match organization_type {
        "claude_max" => "Max",
        "claude_pro" => "Pro",
        "claude_team" => "Team",
        "claude_enterprise" => "Enterprise",
        other => other,
    }
    .to_string()
}

pub fn observe(store: &mut Store, claude_json: &Path, now_ms: i64) -> Result<Option<String>> {
    let login = read_login(claude_json)?;
    let mut writer = store.writer()?;
    let open = writer.open_account()?;
    let now_open = match login {
        Login::SignedIn(row) => {
            writer.upsert_account(&row, now_ms)?;
            if open.as_deref() != Some(row.id.as_str()) {
                writer.close_open_period(now_ms)?;
                writer.open_period(&row.id, now_ms)?;
            }
            Some(row.id)
        }
        Login::SignedOut => {
            writer.close_open_period(now_ms)?;
            None
        }
    };
    writer.commit()?;
    Ok(now_open)
}

pub fn auth_account(auth: &Auth) -> Option<AccountRow> {
    let row = |id: String, kind, label: String| AccountRow {
        id,
        kind,
        label,
        organization_type: None,
        billing_type: None,
    };
    if auth.provider != "anthropic" {
        let label = capitalized(&auth.provider);
        return Some(row(format!("cloud:{}", auth.provider), AccountKind::Cloud, label));
    }
    if let Some((origin, host)) = auth.base_url.as_deref().and_then(origin_of) {
        return Some(row(format!("gateway:{origin}"), AccountKind::Gateway, host));
    }
    auth.api_key
        .then(|| row("api".into(), AccountKind::Api, "API key".into()))
}

fn origin_of(base_url: &str) -> Option<(String, String)> {
    let (scheme, rest) = base_url.trim().split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let authority = rest.split(['/', '?', '#']).next()?.to_ascii_lowercase();
    let authority = authority.rsplit_once('@').map_or(authority.as_str(), |(_, host)| host);
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|b| b.is_ascii_digit()) && !port.is_empty() => {
            (host.to_string(), port.to_string())
        }
        _ => {
            let port = match scheme.as_str() {
                "https" => "443",
                "http" => "80",
                _ => return None,
            };
            (authority.to_string(), port.to_string())
        }
    };
    (!host.is_empty()).then(|| (format!("{scheme}://{host}:{port}"), host))
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

pub fn attribute_start(
    writer: &mut Writer,
    session: &str,
    auth: Option<&Auth>,
    open: Option<&str>,
    now_ms: i64,
) -> Result<()> {
    let resolved = auth.and_then(auth_account);
    let account = match &resolved {
        Some(row) => {
            writer.upsert_account(row, now_ms)?;
            Some(row.id.as_str())
        }
        None => open,
    };
    if let Some(account) = account {
        writer.attribute(session, account, AttributionSource::Start)?;
    }
    Ok(())
}

pub fn attribute_pending(store: &mut Store) -> Result<usize> {
    let pending = store.unattributed_sessions()?;
    let mut writer = store.writer()?;
    let mut attributed = 0;
    for (session, started_at) in pending {
        if let Some(account) = writer.account_at(started_at.saturating_mul(1000))? {
            attributed += usize::from(writer.attribute(&session, &account, AttributionSource::Timeline)?);
        }
    }
    writer.commit()?;
    Ok(attributed)
}

pub fn account_for_sample(writer: &mut Writer, session: &str, at_ms: i64) -> Result<Option<String>> {
    if let Some(account) = writer.session_account(session)? {
        return Ok(Some(account));
    }
    let Some(account) = writer.account_at(at_ms)? else {
        return Ok(None);
    };
    writer.attribute(session, &account, AttributionSource::Timeline)?;
    Ok(Some(account))
}
```

`now_ms` is the `first_seen_ms` of an account first met at this session start.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib accounts::tests`
Expected: PASS (7 tests).

- [ ] **Step 5: Run the Rust gate and commit**

```bash
git add src/accounts.rs src/lib.rs
git commit -m "feat(accounts): read the claude.ai login and resolve each session's account

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Daemon wiring: observe the login, attribute sessions, tag limit samples

**Files:**
- Modify: `src/daemon/writer.rs` (`spawn` takes `claude_json: PathBuf`; `store_events` handles `session.start`; `finish_ingest` attributes pending sessions)
- Modify: `src/daemon/mod.rs` (pass `paths.claude_json.clone()`, observe once at startup through the writer)
- Modify: `src/cli/import.rs` (`incremental` observes and attributes after its own import)
- Modify: `src/api/wire.rs` (`pub const START_KIND: &str = "session.start";`)

**Interfaces:**
- Consumes: `accounts::{observe, attribute_start, attribute_pending, account_for_sample, Auth}`.
- Produces: every session that starts after the upgrade has a `session_accounts` row; every new limit sample has `account_id` when its session has one.

- [ ] **Step 1: Write the failing writer test**

In `src/daemon/writer.rs` tests, extend `Fixture::new` to write a `.claude.json` into the temp dir and pass its path to `spawn`, then add:

```rust
    #[tokio::test]
    async fn a_session_start_takes_the_open_login_and_its_samples_carry_it() {
        let fx = Fixture::new();
        std::fs::write(
            fx.temp.path().join(".claude.json"),
            r#"{"oauthAccount":{"accountUuid":"u-1","organizationName":"Acme","organizationType":"claude_enterprise"}}"#,
        )
        .unwrap();
        let event = |kind: &str, ts_ms: i64, json: &str| EventRow {
            client_id: None,
            session_id: "s1".into(),
            kind: kind.into(),
            ts_ms,
            received_ms: ts_ms,
            json: json.into(),
        };
        fx.handle
            .store_events(vec![
                event("session.start", 1_000, r#"{"cwd":"/w","auth":{"provider":"anthropic","base_url":null,"api_key":false}}"#),
                event("session.measure", 2_000, r#"{"rateLimits":[{"kind":"spend_limit","percentUsed":4,"resetsAt":"2026-11-01T00:00:00Z"}]}"#),
            ])
            .await
            .unwrap();
        let reader = fx.reader();
        assert_eq!(reader.session_account("s1").unwrap().as_deref(), Some("oauth:u-1"));
        let samples = reader.limit_samples_since(0).unwrap();
        assert_eq!(samples[0].account_id.as_deref(), Some("oauth:u-1"));
    }

    #[tokio::test]
    async fn a_gateway_session_is_its_own_account_even_while_signed_in() {
        let fx = Fixture::new();
        std::fs::write(fx.temp.path().join(".claude.json"), r#"{"oauthAccount":{"accountUuid":"u-1"}}"#).unwrap();
        fx.handle
            .store_events(vec![EventRow {
                client_id: None,
                session_id: "s2".into(),
                kind: "session.start".into(),
                ts_ms: 1_000,
                received_ms: 1_000,
                json: r#"{"auth":{"provider":"anthropic","base_url":"https://llm.corp","api_key":true}}"#.into(),
            }])
            .await
            .unwrap();
        assert_eq!(
            fx.reader().session_account("s2").unwrap().as_deref(),
            Some("gateway:https://llm.corp:443")
        );
    }
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test --lib daemon::writer::tests::a_session_start`
Expected: FAIL (no attribution, `account_id` is None).

- [ ] **Step 3: Implement**

`spawn(store, prices, projects_dir, claude_json: PathBuf, log)` stores `claude_json` on `Worker`.
`store_events` takes the login path and the log; the `drain` closure that calls it captures clones of both.
It observes the login before its writer transaction opens, because `observe` commits its own transaction:

```rust
fn store_events(store: &mut Store, claude_json: &Path, log: &Log, rows: &[EventRow]) -> Result<()> {
    let starts = rows.iter().any(|row| row.kind == START_KIND);
    let open = if starts {
        match accounts::observe(store, claude_json, epoch_ms(SystemTime::now())) {
            Ok(open) => open,
            Err(error) => {
                log.warn_once("login", format!("cannot read the Claude Code login: {error:#}"));
                store.open_account()?
            }
        }
    } else {
        None
    };
    let mut writer = store.writer()?;
    for row in rows {
        if row.kind == USAGE_KIND {
            writer.live_breakdown(row)?;
            continue;
        }
        if !writer.event(row)? {
            continue;
        }
        let Ok(data) = serde_json::from_str::<Value>(&row.json) else {
            continue;
        };
        if row.kind == START_KIND {
            let auth: Option<Auth> = data
                .get("auth")
                .and_then(|auth| serde_json::from_value(auth.clone()).ok());
            accounts::attribute_start(&mut writer, &row.session_id, auth.as_ref(), open.as_deref(), row.received_ms)?;
            continue;
        }
        if row.kind != MEASURE_KIND {
            continue;
        }
        let account = accounts::account_for_sample(&mut writer, &row.session_id, row.ts_ms)?;
        for mut sample in limits::samples_from_measure(&row.session_id, row.ts_ms, &data) {
            sample.account_id = account.clone();
            writer.limit_sample(&sample)?;
        }
    }
    writer.commit()
}
```

`Log` has no `warn_once`; add one in `src/daemon/log.rs` that keeps a `Mutex<HashSet<&'static str>>` of keys already warned and logs only the first warning per key per run:

```rust
    pub fn warn_once(&self, key: &'static str, message: impl AsRef<str>) {
        if self.warned.lock().unwrap_or_else(|p| p.into_inner()).insert(key) {
            self.warn(message);
        }
    }
```

with `warned: Mutex<HashSet<&'static str>>` initialised empty in `Log::open`.

In `finish_ingest`, after `refresh_fit` on success, call:

```rust
            if let Err(error) = accounts::attribute_pending(&mut self.store) {
                self.log.warn(format!("cannot attribute sessions to accounts: {error:#}"));
            }
```

and on every full scan (`Job::Scan`) observe first, so the periodic rescan keeps the timeline fresh:

```rust
                Job::Scan(reply) => {
                    if let Err(error) = accounts::observe(&mut self.store, &self.claude_json, epoch_ms(SystemTime::now())) {
                        self.log.warn_once("login", format!("cannot read the Claude Code login: {error:#}"));
                    }
                    let outcome = self.scan(&mut drain);
                    ...
```

`attribute_pending` must run before `refresh_fit` so fits see fresh attributions; order the two calls that way.

`limits::samples_from_measure` leaves `account_id: None`; the writer fills it.

In `src/daemon/mod.rs` pass `paths.claude_json.clone()` to `writer::spawn`; the initial `writer.scan()` already observes.

In `src/cli/import.rs` `incremental`, after the ingest and before returning the store, run:

```rust
    let _ = accounts::observe(&mut store, &paths.claude_json, epoch_ms(SystemTime::now()));
    accounts::attribute_pending(&mut store)?;
```

The CLI ignores a login read error silently because `doctor` reports it (Task 9).

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib daemon::writer`
Expected: PASS, the two new tests included.

- [ ] **Step 5: Run the Rust gate and commit**

```bash
git add src/daemon src/cli/import.rs src/api/wire.rs
git commit -m "feat(daemon): attribute sessions to accounts and tag limit samples with them

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Limits per account and the monthly pace

**Files:**
- Modify: `src/limits.rs`

**Interfaces:**
- Consumes: `LimitSample.account_id`, `SpendPoint.session_id`, `Store::session_accounts`.
- Produces:
  - `Fit.account: Option<String>` (`#[serde(default)]`)
  - `pub fn fit(samples, spend, accounts: &HashMap<String, String>, settled_before_ms) -> Vec<Fit>`
  - `pub fn windows(samples, fits, model, now_ms, reported, account: Option<&str>) -> Vec<LimitWindow>`
  - `pub const SPEND_LIMIT: &str = "spend_limit";`, `pub const BUDGET: &str = "budget";`
  - `pub fn month_start_utc(ms: i64) -> i64`, `pub fn next_month_start_utc(ms: i64) -> i64`
  - `pub fn monthly_pace(used_pct: f64, period_start_ms: i64, resets_at_ms: i64, now_ms: i64) -> Option<Pace>`
  - `pub fn fits_of<'a>(fits: &'a [Fit], account: Option<&str>) -> Vec<Fit>`

- [ ] **Step 1: Write the failing tests**

In `src/limits.rs` tests, add a helper and tests:

```rust
    fn of(account: &str, minute: i64, window: &str, used_pct: f64, resets_minute: i64) -> LimitSample {
        LimitSample { account_id: Some(account.into()), ..sample(minute, window, used_pct, resets_minute) }
    }

    #[test]
    fn switching_accounts_and_back_resumes_the_same_instance() {
        let mut samples = vec![
            of("oauth:me", 0, "five_hour", 60.0, 300),
            of("oauth:me", 40, "five_hour", 62.0, 300),
            of("oauth:work", 10, "five_hour", 5.0, 200),
            of("oauth:work", 20, "five_hour", 6.0, 200),
        ];
        samples.sort_by(|a, b| (&a.account_id, &a.window, a.ts_ms).cmp(&(&b.account_id, &b.window, b.ts_ms)));
        let split: Vec<Vec<f64>> = instances(&samples)
            .iter()
            .map(|instance| instance.iter().map(|s| s.used_pct).collect())
            .collect();
        assert_eq!(split, vec![vec![60.0, 62.0], vec![5.0, 6.0]]);
    }

    #[test]
    fn a_fit_only_counts_spend_of_its_own_account() {
        let samples = vec![of("oauth:me", 0, "five_hour", 10.0, 300), of("oauth:me", 10, "five_hour", 12.0, 300)];
        let spend = vec![
            SpendPoint { ts_ms: 5 * 60_000, session_id: "mine".into(), model: "claude-opus-5-5".into(), cost_usd: 1.0 },
            SpendPoint { ts_ms: 6 * 60_000, session_id: "work".into(), model: "claude-opus-5-5".into(), cost_usd: 9.0 },
        ];
        let accounts = HashMap::from([
            ("mine".to_string(), "oauth:me".to_string()),
            ("work".to_string(), "oauth:work".to_string()),
        ]);
        let fits = fit(&samples, &spend, &accounts, i64::MAX);
        assert_eq!(fits.len(), 1);
        assert_eq!(fits[0].account.as_deref(), Some("oauth:me"));
        assert!((fits[0].pct_per_usd - 2.0).abs() < 1e-9, "{}", fits[0].pct_per_usd);
    }

    #[test]
    fn windows_show_only_the_asked_account() {
        let samples = vec![of("oauth:me", 0, "seven_day", 40.0, 9000), of("oauth:work", 1, "spend_limit", 4.0, 50_000)];
        let shown = |account| -> Vec<String> {
            windows(&samples, &[], None, 2 * 60_000, &[], account).into_iter().map(|w| w.window).collect()
        };
        assert_eq!(shown(Some("oauth:work")), vec!["spend_limit"]);
        assert_eq!(shown(Some("oauth:me")), vec!["seven_day"]);
        assert_eq!(shown(None), Vec::<String>::new());
    }

    #[test]
    fn monthly_pace_waits_a_full_day_and_uses_utc_months() {
        let oct_1 = DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z").unwrap().timestamp_millis();
        let nov_1 = DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z").unwrap().timestamp_millis();
        let oct_11 = DateTime::parse_from_rfc3339("2026-10-11T12:00:00Z").unwrap().timestamp_millis();
        assert_eq!(month_start_utc(oct_11), oct_1);
        assert_eq!(next_month_start_utc(oct_11), nov_1);
        assert_eq!(month_start_utc(nov_1 - 1), oct_1);
        assert_eq!(monthly_pace(4.0, oct_1, nov_1, oct_1 + DAY_MS - 1), None);
        let pace = monthly_pace(4.0, oct_1, nov_1, oct_11).unwrap();
        let elapsed_days = 10.5;
        let projected = 4.0 / elapsed_days * 31.0;
        assert!((pace.projected_pct.unwrap() - projected).abs() < 1e-6);
        assert_eq!(pace.full_at_ms, None);
        let hot = monthly_pace(50.0, oct_1, nov_1, oct_11).unwrap();
        assert!(hot.full_at_ms.unwrap() < nov_1);
    }

    #[test]
    fn a_spend_limit_window_paces_over_its_month() {
        let resets = DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z").unwrap().timestamp_millis();
        let now = DateTime::parse_from_rfc3339("2026-10-11T12:00:00Z").unwrap().timestamp_millis();
        let samples = vec![LimitSample {
            ts_ms: now - 60_000,
            session_id: "s1".into(),
            account_id: Some("oauth:work".into()),
            model: None,
            window: SPEND_LIMIT.into(),
            used_pct: 4.0,
            resets_at_ms: Some(resets),
        }];
        let shown = windows(&samples, &[], None, now, &[], Some("oauth:work"));
        let projected = shown[0].pace.as_ref().unwrap().projected_pct.unwrap();
        assert!((projected - 4.0 / 10.5 * 31.0).abs() < 1e-6, "{projected}");
    }
```

Add `use std::collections::HashMap;` to the test module if needed, and update the existing `fit(...)` and `windows(...)` calls in the tests to pass `&HashMap::new()` and `None` respectively, and the existing samples to carry `account_id: None` (they already do since Task 1).

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --lib limits::tests`
Expected: compile errors for the new signatures.

- [ ] **Step 3: Implement**

`instances` also splits when `sample.account_id != previous.account_id`.

`Fit` gains:

```rust
    #[serde(default)]
    pub account: Option<String>,
```

`fit`:

```rust
pub fn fit(
    samples: &[LimitSample],
    spend: &[SpendPoint],
    accounts: &HashMap<String, String>,
    settled_before_ms: i64,
) -> Vec<Fit> {
    let mut ratios: BTreeMap<(Option<String>, String, String), Vec<f64>> = BTreeMap::new();
    for instance in instances(samples) {
        let account = instance[0].account_id.as_deref();
        let own: Vec<&SpendPoint> = spend
            .iter()
            .filter(|point| accounts.get(&point.session_id).map(String::as_str) == account)
            .collect();
        for pair in instance.windows(2) {
            let (from, to) = (&pair[0], &pair[1]);
            let gained = to.used_pct - from.used_pct;
            if to.ts_ms > settled_before_ms || gained <= 0.0 {
                continue;
            }
            let Some((family, usd)) = dominant_spend(&own, from.ts_ms, to.ts_ms) else {
                continue;
            };
            ratios
                .entry((account.map(String::from), to.window.clone(), family.to_string()))
                .or_default()
                .push(gained / usd);
        }
    }
    ratios
        .into_iter()
        .map(|((account, window, family), ratios)| Fit {
            account,
            window,
            family,
            pairs: ratios.len() as u32,
            pct_per_usd: trimmed_median(ratios),
        })
        .collect()
}
```

`dominant_spend` takes `&[&SpendPoint]`; keep its body, dereferencing points.

`fits_of` filters by account:

```rust
pub fn fits_of(fits: &[Fit], account: Option<&str>) -> Vec<Fit> {
    fits.iter().filter(|fit| fit.account.as_deref() == account).cloned().collect()
}
```

`windows` gains `account: Option<&str>` as its last parameter; it keeps only instances whose `account_id.as_deref() == account` and looks up fits with `fit.account.as_deref() == account`; when `account` is `None` it returns an empty list, because Unknown has no own windows.
For an instance whose window is `SPEND_LIMIT` and whose last sample has `resets_at_ms: Some(resets)`, `pace` is `monthly_pace(sample.used_pct, month_start_utc(resets - 1), resets, now_ms)` instead of `pace(instance, now_ms)`.

Month helpers with chrono:

```rust
use chrono::{Datelike, TimeZone, Utc};

pub fn month_start_utc(ms: i64) -> i64 {
    let at = Utc.timestamp_millis_opt(ms).single().expect("a representable time");
    Utc.with_ymd_and_hms(at.year(), at.month(), 1, 0, 0, 0)
        .single()
        .expect("the first of a month exists")
        .timestamp_millis()
}

pub fn next_month_start_utc(ms: i64) -> i64 {
    let at = Utc.timestamp_millis_opt(ms).single().expect("a representable time");
    let (year, month) = if at.month() == 12 { (at.year() + 1, 1) } else { (at.year(), at.month() + 1) };
    Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0)
        .single()
        .expect("the first of a month exists")
        .timestamp_millis()
}

pub fn monthly_pace(used_pct: f64, period_start_ms: i64, resets_at_ms: i64, now_ms: i64) -> Option<Pace> {
    let elapsed = now_ms - period_start_ms;
    if elapsed < DAY_MS {
        return None;
    }
    let per_ms = used_pct / elapsed as f64;
    let projected = used_pct + per_ms * (resets_at_ms - now_ms).max(0) as f64;
    let full_at_ms = (projected >= 100.0 && per_ms > 0.0)
        .then(|| now_ms + ((100.0 - used_pct).max(0.0) / per_ms).round() as i64);
    Some(Pace {
        pct_per_hour: per_ms * HOUR_MS,
        projected_pct: Some(projected),
        full_at_ms,
    })
}
```

`refresh_fit` passes `&store.session_accounts()?`.
`WINDOW_ORDER` becomes `&["five_hour", "seven_day", "spend_limit", "budget"]`.

Update callers:
- `src/api/live.rs`: `windows(..., store.session_account(session)?.as_deref())` (Task 5 finishes this file).
- `src/history/mod.rs` and `src/insights/report.rs` keep reading every stored fit in this task; Task 6 narrows them to the scope's account with `fits_of`.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib limits`
Expected: PASS.

- [ ] **Step 5: Run the Rust gate and commit**

```bash
git add src/limits.rs src/api/live.rs src/daemon/writer.rs
git commit -m "feat(limits): keep windows and calibration per account and pace spend limits over the month

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Snapshot account and budget

**Files:**
- Modify: `src/api/live.rs`
- Modify: `src/api/wire.rs` (shape test)
- Test: `src/api/tests.rs`

**Interfaces:**
- Consumes: Tasks 1 and 4.
- Produces in `Live`:
  - `#[serde(default)] pub account: Option<AccountRef>` with `pub struct AccountRef { pub id: String, pub label: String, pub kind: AccountKind }`
  - `#[serde(default)] pub budget: Option<BudgetView>` with `pub struct BudgetView { pub amount_usd: f64, pub local_usd: f64, pub period_start_ms: i64, pub resets_at_ms: i64 }`
  - a synthesized `LimitWindow { window: "budget", .. }` when a budget is set and no `spend_limit` window is shown

- [ ] **Step 1: Write the failing API tests**

In `src/api/tests.rs`, following the style of `only_this_session_s_own_limits_say_it_runs_on_a_subscription`:

```rust
#[test]
fn a_snapshot_shows_only_its_own_account_s_windows() {
    let server = Server::start();
    let now_ms = epoch_ms(SystemTime::now());
    server.login(r#"{"oauthAccount":{"accountUuid":"me","organizationName":"Me","organizationType":"claude_max"}}"#);
    server.start_session(SESSION, now_ms - 9000);
    server.measure(SESSION, now_ms - 8000, json!([{"kind": "seven_day", "percentUsed": 40, "resetsAt": "2099-01-01T00:00:00Z"}]));
    server.login(r#"{"oauthAccount":{"accountUuid":"work","organizationName":"Acme","organizationType":"claude_enterprise"}}"#);
    let other = "22222222-2222-2222-2222-222222222222";
    server.start_session(other, now_ms - 2000);
    server.measure(other, now_ms - 1000, json!([{"kind": "spend_limit", "percentUsed": 4, "resetsAt": "2099-01-01T00:00:00Z"}]));
    let live = server.client().snapshot(other).unwrap().live;
    assert_eq!(live.limits.iter().map(|w| w.window.as_str()).collect::<Vec<_>>(), vec!["spend_limit"]);
    let account = live.account.unwrap();
    assert_eq!((account.id.as_str(), account.label.as_str()), ("oauth:work", "Acme · Enterprise"));
}

#[test]
fn a_budget_without_a_reported_limit_is_its_own_window() {
    let server = Server::start();
    let now_ms = epoch_ms(SystemTime::now());
    server.start_session_with(SESSION, now_ms - 2000, json!({"provider": "anthropic", "base_url": "https://llm.corp", "api_key": true}));
    server.set_budget("gateway:https://llm.corp:443", 700.0);
    let live = server.client().snapshot(SESSION).unwrap().live;
    let budget = live.budget.unwrap();
    assert_eq!(budget.amount_usd, 700.0);
    assert_eq!(budget.resets_at_ms, crate::limits::next_month_start_utc(now_ms));
    assert_eq!(live.limits.iter().map(|w| w.window.as_str()).collect::<Vec<_>>(), vec!["budget"]);
}
```

Add the helpers to the `Server` test harness in `src/api/tests.rs`: `login(json)` writes `.claude.json` in the server's temp home (the harness already builds `Paths`; write to `paths.claude_json`), `start_session(id, ts_ms)` and `start_session_with(id, ts_ms, auth)` send a `session.start` event with `{"cwd": "/w", "auth": auth}` (default auth `{"provider":"anthropic","base_url":null,"api_key":false}`), `measure(id, ts_ms, rate_limits)` sends a `session.measure` with that `rateLimits`, and `set_budget(id, usd)` opens the store at the harness database and calls `Writer::set_budget`.

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --lib api::tests::a_snapshot_shows_only api::tests::a_budget_without`
Expected: FAIL.

- [ ] **Step 3: Implement in `Live::read`**

```rust
        let account_id = store.session_account(session)?;
        let account = match account_id.as_deref() {
            Some(id) => store.account(id)?.map(|info| AccountRef {
                id: info.id,
                label: info.label,
                kind: info.kind,
            }),
            None => None,
        };
        let mut limits = limits::windows(
            &limits::recent_samples(store, now_ms)?,
            &limits::stored_fit(store)?,
            model.as_deref(),
            now_ms,
            &reported,
            account_id.as_deref(),
        );
        let budget = match account_id.as_deref() {
            Some(id) => store.budget(id)?.map(|budget| {
                let period_start_ms = limits::month_start_utc(now_ms);
                Ok::<_, anyhow::Error>(BudgetView {
                    amount_usd: budget.amount_usd,
                    local_usd: store.account_spend_since(id, period_start_ms.div_euclid(1000))?,
                    period_start_ms,
                    resets_at_ms: limits::next_month_start_utc(now_ms),
                })
            }).transpose()?,
            None => None,
        };
        if let Some(budget) = &budget
            && !limits.iter().any(|window| window.window == limits::SPEND_LIMIT)
        {
            limits.push(limits::budget_window(budget, model.as_deref(), now_ms));
        }
```

Add to `src/limits.rs`:

```rust
pub fn budget_window(budget: &BudgetView, model: Option<&str>, now_ms: i64) -> LimitWindow {
    let used_pct = budget.local_usd / budget.amount_usd * 100.0;
    LimitWindow {
        window: BUDGET.into(),
        used_pct,
        resets_at_ms: Some(budget.resets_at_ms),
        sampled_at_ms: now_ms,
        reset_since_sample: false,
        pace: monthly_pace(used_pct, budget.period_start_ms, budget.resets_at_ms, now_ms),
        calibration: Calibration {
            family: model.map_or("other", family).to_string(),
            pairs: 0,
            needed: PAIRS_NEEDED,
            pct_per_usd: None,
        },
    }
}
```

Define `AccountRef` and `BudgetView` in `src/api/live.rs` with `Serialize, Deserialize, Debug, Clone, PartialEq` and import `BudgetView` into `limits.rs` from there, or define `BudgetView` in `limits.rs` and re-export it from `live.rs`; choose the latter to avoid a cycle (`limits` must not depend on `api`).

Update the snapshot shape test in `src/api/wire.rs` (`snapshot_v1_has_the_documented_shape`) to include `"account": null, "budget": null` in the expected JSON for the fixtures that have none, and one fixture with both set.
`SNAPSHOT_VERSION` stays 1: the fields are additions that older panes ignore.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib api`
Expected: PASS.

- [ ] **Step 5: Run the Rust gate and commit**

```bash
git add src/api src/limits.rs
git commit -m "feat(api): show the session's account and its monthly budget in the snapshot

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Scoped History, Insights, stats and export

**Files:**
- Create: `src/scope.rs`, `pub mod scope;` in `src/lib.rs`
- Modify: `src/history/mod.rs` (`read` takes `&Scope`; report gains `account` and `accounts`)
- Modify: `src/api/history.rs` (`account=` query parameter)
- Modify: `src/insights/report.rs` (scope of the request's session)
- Modify: `src/store/mod.rs` (`daily_costs` takes `Option<&HashSet<String>>`)
- Modify: `src/history/export.rs` (`ExportRow.account`, CSV column)
- Test: `src/history/tests.rs`, `src/api/history.rs` tests

**Interfaces:**
- Consumes: Task 1 store API, Task 4 `fits_of`.
- Produces:
  - `pub enum Scope { All, Account(String) }` with `pub fn keeps(&self, sessions: &Option<HashSet<String>>, session: &str) -> bool` and `pub fn sessions(&self, store: &Store) -> Result<Option<HashSet<String>>>` and `pub fn account(&self) -> Option<&str>`
  - `pub fn percent_decode(text: &str) -> Option<String>`
  - `HistoryReport.account: Option<String>`, `HistoryReport.accounts: Vec<AccountRef>`
  - `ExportRow.account: String` (label, or `Unknown`), CSV header gains `account` after `project`

- [ ] **Step 1: Write the failing tests**

`src/scope.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decoding_reads_gateway_ids_and_rejects_broken_escapes() {
        assert_eq!(percent_decode("gateway%3Ahttps%3A%2F%2Fllm.corp%3A443").as_deref(), Some("gateway:https://llm.corp:443"));
        assert_eq!(percent_decode("oauth:u-1").as_deref(), Some("oauth:u-1"));
        assert_eq!(percent_decode("bad%2"), None);
        assert_eq!(percent_decode("bad%zz"), None);
    }

    #[test]
    fn all_keeps_everything_and_an_account_only_its_sessions() {
        let only = Some(HashSet::from(["s1".to_string()]));
        assert!(Scope::All.keeps(&None, "s9"));
        assert!(Scope::Account("a".into()).keeps(&only, "s1"));
        assert!(!Scope::Account("a".into()).keeps(&only, "s9"));
    }
}
```

`src/history/tests.rs`: build a store with two sessions on two accounts (use the existing fixture helpers that insert requests; add `session_accounts` rows with `Writer::attribute`), then:

```rust
#[test]
fn history_of_one_account_counts_only_its_sessions_and_lists_both() {
    let (store, prices) = two_account_store();
    let report = read(&store, &prices, Range::Week, NOW_MS, &chrono::Utc, &Scope::Account("oauth:work".into())).unwrap();
    assert_eq!(report.totals.spend.requests, 3);
    assert_eq!(report.account.as_deref(), Some("oauth:work"));
    assert_eq!(report.accounts.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), vec!["oauth:me", "oauth:work"]);
    let all = read(&store, &prices, Range::Week, NOW_MS, &chrono::Utc, &Scope::All).unwrap();
    assert_eq!(all.totals.spend.requests, 5);
    assert_eq!(all.calibration.calibrated, false);
    assert_eq!(all.totals.peak_5h_pct, None);
}
```

`two_account_store` inserts 2 requests in session `mine` (attributed `oauth:me`) and 3 in `work` (attributed `oauth:work`) inside the week, using the request-row helper the file already uses; name the constants as the file does (`NOW_MS` or its equivalent).

`src/api/history.rs` tests:

```rust
    #[test]
    fn the_account_defaults_to_all_and_decodes_ids() {
        assert_eq!(scope_of(None), Ok(Scope::All));
        assert_eq!(scope_of(Some("range=7d&account=all")), Ok(Scope::All));
        assert_eq!(
            scope_of(Some("account=gateway%3Ahttps%3A%2F%2Fgw%3A443")),
            Ok(Scope::Account("gateway:https://gw:443".into()))
        );
        assert_eq!(scope_of(Some("account=")), Err(ACCOUNT_RULE));
        assert_eq!(scope_of(Some("account=%zz")), Err(ACCOUNT_RULE));
    }
```

`src/history/export.rs` tests: update the CSV header expectation to `day,project,account,model,...` and add a row whose session is unattributed, expecting `Unknown`.

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --lib scope history api::history`
Expected: compile errors.

- [ ] **Step 3: Implement `src/scope.rs`**

```rust
use std::collections::HashSet;

use anyhow::Result;

use crate::store::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    All,
    Account(String),
}

impl Scope {
    pub fn account(&self) -> Option<&str> {
        match self {
            Scope::All => None,
            Scope::Account(id) => Some(id),
        }
    }

    pub fn sessions(&self, store: &Store) -> Result<Option<HashSet<String>>> {
        match self {
            Scope::All => Ok(None),
            Scope::Account(id) => store.sessions_of(id).map(Some),
        }
    }

    pub fn keeps(&self, sessions: &Option<HashSet<String>>, session: &str) -> bool {
        sessions.as_ref().is_none_or(|kept| kept.contains(session))
    }
}

pub fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}
```

`is_none_or` is stable since Rust 1.82, within the 1.88 MSRV.

- [ ] **Step 4: Scope History**

`history::read` gains `scope: &Scope` as its last parameter.
After fetching, keep only the scope's rows:

```rust
    let kept = scope.sessions(store)?;
    let requests: Vec<SpentRequest> = store
        .requests_between(since, until)?
        .into_iter()
        .filter(|r| scope.keeps(&kept, &r.session_id))
        .collect();
```

Apply the same filter to `adjustments_between` and `sessions_spending_between` (`SessionInfo.id`).
Samples: `scope.account().map_or(Ok(Vec::new()), |account| store.limit_samples_between(FIVE_HOUR, ..).map(|samples| samples.into_iter().filter(|s| s.account_id.as_deref() == Some(account)).collect()))`.
Fits: `limits::fits_of(&limits::stored_fit(store)?, scope.account())`, empty for `All` because `fits_of(.., None)` keeps only unattributed fits; pass `&[]` for `All` explicitly to keep the intent readable.

`HistoryReport` gains:

```rust
    pub account: Option<String>,
    pub accounts: Vec<AccountRef>,
```

`accounts` lists every account from `store.accounts()` that has at least one session spending in the range (`store.session_accounts()` intersected with `sessions_spending_between` ids), plus the scope's own account; ordered by `first_seen_ms`.
`build` stays pure: `read` sets both fields on the built report.

`src/api/history.rs`:

```rust
pub const ACCOUNT_RULE: &str = "account must be all or an account id";

pub fn scope_of(query: Option<&str>) -> Result<Scope, &'static str> {
    match query.unwrap_or("").split('&').find_map(|pair| pair.strip_prefix("account=")) {
        None | Some("all") => Ok(Scope::All),
        Some("") => Err(ACCOUNT_RULE),
        Some(text) => percent_decode(text).filter(|id| !id.is_empty()).map(Scope::Account).ok_or(ACCOUNT_RULE),
    }
}
```

and `history()` parses both range and scope, answering 400 with the rule on error.

- [ ] **Step 5: Scope Insights, stats and export**

`insights::report::read`: `let scope = match store.session_account(request.session)? { Some(id) => Scope::Account(id), None => Scope::All };` then filter `quarter`, `reconciled`, `sessions` and the `large_tool_results_between` rows by `scope.keeps`, and use `Rates::of(&limits::fits_of(&limits::stored_fit(store)?, scope.account()))`.
`prompts_of` and `rebuilds` already derive from the filtered `requests`.

`Store::daily_costs(since, until, tz, sessions: Option<&HashSet<String>>)`: `costs_between` also selects `session_id` and skips rows whose session is not kept.

`export::rows` gains `accounts: &HashMap<String, String>` (session to label) and fills `ExportRow.account` with the label or `"Unknown"`; `CSV_HEADER` becomes `day,project,account,model,requests,unpriced,input,output,cache_read,cache_write_5m,cache_write_1h,cost_usd,reconciled_usd`; `EXPORT_VERSION` becomes 2 because the JSON rows change shape.

- [ ] **Step 6: Run the tests**

Run: `cargo test`
Expected: PASS; fix every call site of `history::read`, `daily_costs` and `export::rows` (`rg -n "history::read\(|daily_costs\(|export::rows\(" src`) by passing `&Scope::All`, `None` and the label map until Task 9 wires the CLI options.

- [ ] **Step 7: Run the Rust gate and commit**

```bash
git add src
git commit -m "feat(history): read History, Insights, stats and export per account

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Gateway and cloud model ids, optional user price file

**Files:**
- Modify: `src/pricing/mod.rs`
- Modify: `src/paths.rs` (`pub fn user_prices(&self) -> PathBuf { self.data_dir.join("prices.toml") }`)
- Modify: `src/cli/mod.rs` (load prices through `PriceTable::with_user_file`)

**Interfaces:**
- Produces:
  - `normalize_model` handles Bedrock, Vertex and gateway ids
  - `pub enum UserPrices { Absent, Loaded, Invalid(String) }`
  - `pub fn with_user_file(path: &Path) -> (PriceTable, UserPrices)`

- [ ] **Step 1: Write the failing tests**

In `src/pricing/mod.rs` tests:

```rust
    #[test]
    fn cloud_and_gateway_model_ids_find_their_price() {
        for id in [
            "us.anthropic.claude-opus-5-5-20260901-v1:0",
            "global.anthropic.claude-opus-5-5-v1:0",
            "anthropic.claude-opus-5-5-20260901-v2:0",
            "claude-opus-5-5@20260901",
            "anthropic/claude-opus-5-5",
            "bedrock/us.anthropic.claude-opus-5-5-20260901-v1:0",
            "claude-opus-5-5-20260901[1m]",
        ] {
            assert_eq!(normalize_model(id), "claude-opus-5-5", "{id}");
        }
        assert_eq!(normalize_model("gpt-5"), "gpt-5");
    }

    #[test]
    fn the_user_file_is_optional_merged_and_reported_when_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prices.toml");
        let (table, state) = PriceTable::with_user_file(&path);
        assert_eq!(state, UserPrices::Absent);
        assert_eq!(table.version(), PriceTable::bundled().version());
        std::fs::write(&path, "[models.claude-opus-5-5]\ninput = 3\noutput = 15\ncache_read = 0.1\n").unwrap();
        let (table, state) = PriceTable::with_user_file(&path);
        assert_eq!(state, UserPrices::Loaded);
        let cost = table.cost("claude-opus-5-5", &tokens(1_000_000, 0, 0, 0, 0)).unwrap();
        assert!(close(cost, 3.0), "{cost}");
        assert!(table.knows("claude-haiku-5-5"));
        std::fs::write(&path, "[models.x]\ninput = -1\noutput = 1\ncache_read = 1\n").unwrap();
        let (table, state) = PriceTable::with_user_file(&path);
        assert!(matches!(state, UserPrices::Invalid(_)));
        assert_eq!(table.version(), PriceTable::bundled().version());
    }
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --lib pricing`
Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
pub fn normalize_model(model: &str) -> &str {
    let model = model.split('[').next().unwrap_or(model);
    let model = model.rsplit_once('/').map_or(model, |(_, name)| name);
    let model = model.split_once('@').map_or(model, |(name, _)| name);
    let model = model
        .find("anthropic.")
        .map_or(model, |at| &model[at + "anthropic.".len()..]);
    let model = strip_suffix_where(model, |suffix| {
        suffix.strip_prefix('v').and_then(|rest| rest.split_once(':')).is_some_and(|(major, minor)| {
            is_digits(major) && is_digits(minor)
        })
    });
    strip_suffix_where(model, |suffix| suffix.len() == 8 && is_digits(suffix))
}

fn strip_suffix_where(model: &str, matches: impl Fn(&str) -> bool) -> &str {
    match model.rsplit_once('-') {
        Some((base, suffix)) if matches(suffix) => base,
        _ => model,
    }
}

fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}
```

`version` becomes `#[serde(default = "local_version")]` with `fn local_version() -> String { "local".into() }`, so a user file needs no `version` line.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserPrices {
    Absent,
    Loaded,
    Invalid(String),
}

impl PriceTable {
    pub fn with_user_file(path: &Path) -> (Self, UserPrices) {
        let bundled = Self::bundled();
        match std::fs::read_to_string(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (bundled, UserPrices::Absent),
            Err(error) => (bundled, UserPrices::Invalid(format!("cannot read {}: {error}", path.display()))),
            Ok(text) => match Self::from_toml(&text) {
                Ok(overrides) => (bundled.merged_with(overrides), UserPrices::Loaded),
                Err(error) => (bundled, UserPrices::Invalid(format!("{}: {error:#}", path.display()))),
            },
        }
    }
}
```

In `src/cli/mod.rs` replace `let prices = PriceTable::bundled();` with `let (prices, user_prices) = PriceTable::with_user_file(&paths.user_prices());`, print `UserPrices::Invalid(message)` to stderr as `warning: {message}, using Anthropic's prices` for every command except `install` and `uninstall`, and pass `&user_prices` to `doctor::run`.
The daemon receives the merged table through `daemon::run(&paths, prices, ...)` as today.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib pricing`
Expected: PASS.

- [ ] **Step 5: Run the Rust gate and commit**

```bash
git add src/pricing src/paths.rs src/cli/mod.rs
git commit -m "feat(pricing): price cloud and gateway model ids and merge an optional user price file

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: CLI `accounts` and `budget`

**Files:**
- Create: `src/cli/accounts.rs`
- Modify: `src/cli/mod.rs` (subcommands)
- Test: `tests/cli.rs` (or the existing CLI integration test file; find it with `ls tests`)

**Interfaces:**
- Consumes: Task 1 store API, Task 5 `next_month_start_utc` and `month_start_utc`.
- Produces:
  - `claude-plus accounts`, `claude-plus accounts rename <account> <label>`
  - `claude-plus budget set <usd> [--account <account>]`, `budget show [--account]`, `budget clear [--account]`
  - `pub fn resolve(store: &Store, given: Option<&str>) -> Result<String>` (exact id, else case-insensitive label, else error; `None` means the current account per Decision 3)

- [ ] **Step 1: Write the failing integration tests**

Follow the existing pattern in `tests/` (an `assert_cmd` command with `HOME`, `CLAUDE_CONFIG_DIR` and `CLAUDE_PLUS_HOME` pointing into a temp dir):

```rust
#[test]
fn a_budget_is_set_shown_and_cleared_for_the_signed_in_account() {
    let env = Env::new();
    env.write_claude_json(r#"{"oauthAccount":{"accountUuid":"work","organizationName":"Acme","organizationType":"claude_enterprise"}}"#);
    env.write_transcript_today("s1", "claude-opus-5-5");
    env.cmd(&["import"]).assert().success();
    env.cmd(&["budget", "set", "700"]).assert().success().stdout(predicates::str::contains("Budget for Acme · Enterprise set to $700.00 a month"));
    env.cmd(&["budget", "show"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Acme · Enterprise"))
        .stdout(predicates::str::contains("$700.00 a month"))
        .stdout(predicates::str::contains("Claude Code here:"));
    env.cmd(&["budget", "clear"]).assert().success().stdout(predicates::str::contains("Budget for Acme · Enterprise cleared"));
}

#[test]
fn a_budget_must_be_a_positive_amount() {
    let env = Env::new();
    for amount in ["0", "-5", "nan", "inf", "abc"] {
        env.cmd(&["budget", "set", amount, "--account", "api"])
            .assert()
            .failure()
            .stderr(predicates::str::contains("a budget is a number of dollars above zero"));
    }
}

#[test]
fn an_ambiguous_label_lists_the_matches() {
    let env = Env::new();
    env.seed_accounts(&[("oauth:a", "Work"), ("oauth:b", "work")]);
    env.cmd(&["budget", "set", "10", "--account", "WORK"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("\"WORK\" matches oauth:a (Work), oauth:b (work)"));
}

#[test]
fn accounts_lists_and_renames() {
    let env = Env::new();
    env.seed_accounts(&[("oauth:a", "Acme · Enterprise")]);
    env.cmd(&["accounts", "rename", "oauth:a", "Work"]).assert().success();
    env.cmd(&["accounts"]).assert().success().stdout(predicates::str::contains("Work")).stdout(predicates::str::contains("oauth:a"));
}
```

Add `Env` helpers where missing: `write_claude_json`, `write_transcript_today` (one assistant line with usage, as in `src/daemon/writer.rs` tests' `line`), `seed_accounts` (opens the store at `CLAUDE_PLUS_HOME/data.db` with `claude_plus::store::Store::open` and upserts the rows).

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --test cli budget accounts`
Expected: FAIL (unknown subcommands).

- [ ] **Step 3: Implement**

`src/cli/mod.rs`:

```rust
    #[command(about = "List the accounts claude-plus has seen, or rename one")]
    Accounts {
        #[command(subcommand)]
        action: Option<AccountsAction>,
    },
    #[command(about = "Set, show or clear a monthly budget for an account")]
    Budget {
        #[command(subcommand)]
        action: BudgetAction,
    },
```

```rust
#[derive(Subcommand)]
enum AccountsAction {
    #[command(about = "Give an account your own label")]
    Rename { account: String, label: String },
}

#[derive(Subcommand)]
enum BudgetAction {
    #[command(about = "Set the monthly budget in USD, reset on the 1st at 00:00 UTC")]
    Set {
        usd: String,
        #[arg(long, help = "Account id or label [default: the current account]")]
        account: Option<String>,
    },
    #[command(about = "Show the budget, this month's spend and the pace")]
    Show {
        #[arg(long)]
        account: Option<String>,
    },
    #[command(about = "Remove the budget")]
    Clear {
        #[arg(long)]
        account: Option<String>,
    },
}
```

`usd` is a `String` so the error message is ours: parse with `str::parse::<f64>()`, reject non-finite or `<= 0.0` with `bail!("a budget is a number of dollars above zero, not {usd}")`.

`src/cli/accounts.rs`:

```rust
pub fn resolve(store: &Store, given: Option<&str>) -> Result<String> {
    let Some(given) = given else {
        return match store.latest_session_account()?.or(store.open_account()?) {
            Some(id) => Ok(id),
            None => bail!("no account known yet; pass --account, see claude-plus accounts"),
        };
    };
    let accounts = store.accounts()?;
    if let Some(exact) = accounts.iter().find(|a| a.id == given) {
        return Ok(exact.id.clone());
    }
    let matches: Vec<&AccountInfo> = accounts.iter().filter(|a| a.label.eq_ignore_ascii_case(given)).collect();
    match matches.as_slice() {
        [one] => Ok(one.id.clone()),
        [] if given == "api" || given.starts_with("gateway:") || given.starts_with("cloud:") => Ok(given.to_string()),
        [] => bail!("no account called \"{given}\"; see claude-plus accounts"),
        many => bail!(
            "\"{given}\" matches {}; pass the id",
            many.iter().map(|a| format!("{} ({})", a.id, a.label)).collect::<Vec<_>>().join(", ")
        ),
    }
}
```

`budget set` on an id that has no `accounts` row (only possible for `api`, `gateway:` and `cloud:` ids typed by hand) upserts it with the label `auth_account` would give, so a budget can be set before first use.

Output wording (use `super::format::usd` for amounts, and the `labeled` helper for `show`):
- set: `Budget for {label} set to $700.00 a month, reset on the 1st at 00:00 UTC`
- clear: `Budget for {label} cleared` or `{label} has no budget`
- show:
  ```
    Account:     Acme · Enterprise (oauth:work)
    Budget:      $700.00 a month, resets in 20d
    Reported:    4% used, about $28.00
    Claude Code here: $23.10
    Pace:        on pace for 31% by the reset
  ```
  `Reported` reads the latest `spend_limit` sample of the account from `limit_samples_since(month_start_utc(now))`; it is left out when there is none.
  `Pace` uses `monthly_pace` on the reported percent when present, else on local spend; it says `needs a full day of this month` before one day.
- accounts: a `comfy-table` with columns `Account`, `Id`, `Kind`, `Sessions`, `This month`, `Budget`; `This month` is `account_spend_since(id, month_start_utc(now)/1000)`; `Budget` is `$700.00` or `-`.

The CLI imports first (`super::import::incremental`) for `accounts` and `budget show`, so attribution is fresh.

- [ ] **Step 4: Run the tests**

Run: `cargo test --test cli`
Expected: PASS.

- [ ] **Step 5: Run the Rust gate and commit**

```bash
git add src/cli tests
git commit -m "feat(cli): list and rename accounts and set a monthly budget per account

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: `--account` on stats and export, and doctor lines

**Files:**
- Modify: `src/cli/mod.rs`, `src/cli/stats.rs`, `src/cli/export.rs`, `src/cli/doctor.rs`
- Test: `tests/cli.rs`, `src/cli/doctor.rs` tests

**Interfaces:**
- Consumes: `cli::accounts::resolve`, `Scope`, `UserPrices`, `accounts::read_login`.
- Produces: `--account <label|id|all>` on `stats` and `export`, defaulting to the current account; doctor lines `Account:`, `Login:`, `Prices:`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn stats_count_only_the_current_account_unless_all_is_asked() {
    let env = Env::new();
    env.write_claude_json(r#"{"oauthAccount":{"accountUuid":"me","organizationName":"Me","organizationType":"claude_max"}}"#);
    env.write_transcript_today("mine", "claude-opus-5-5");
    env.cmd(&["import"]).assert().success();
    env.write_claude_json(r#"{"oauthAccount":{"accountUuid":"work","organizationName":"Acme","organizationType":"claude_enterprise"}}"#);
    env.write_transcript_today("work", "claude-opus-5-5");
    env.cmd(&["import"]).assert().success();
    let requests = |args: &[&str]| -> String { String::from_utf8(env.cmd(args).output().unwrap().stdout).unwrap() };
    assert!(requests(&["stats", "--days", "1"]).contains("Acme · Enterprise"));
    assert!(requests(&["stats", "--days", "1", "--account", "all"]).contains("All accounts"));
}
```

`write_transcript_today` must give each session a `started_at` after the login it belongs to: write the transcript after the `write_claude_json` that precedes it and stamp its timestamp with the current time.

Doctor unit test: with a `.claude.json` holding an enterprise login and a valid user price file, the output contains `Account:     Acme · Enterprise (oauth:work)` and `Prices:      Anthropic's, with your prices.toml`; with a malformed `.claude.json` it contains `Login:       cannot read`.

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --test cli stats_count_only`
Expected: FAIL.

- [ ] **Step 3: Implement**

`Stats` and `Export` gain:

```rust
        #[arg(long, help = "Account id or label, or all [default: the current account]")]
        account: Option<String>,
```

Both resolve the scope after their import: `Some("all")` is `Scope::All`; anything else goes through `accounts::resolve`, and when no account is known at all they fall back to `Scope::All` without an error, so a fresh install still prints.
`stats` prints a first line `Account: {label}` or `Account: All accounts` above the table.
`export` passes the label map from `store.accounts()` and `store.session_accounts()` and filters rows by scope.

`doctor` prints, after `Mod:`:
- `Account:` the current account label and id, or `none seen yet`.
- `Login:` `signed in` / `signed out` / `cannot read {path}: {error}` from `accounts::read_login(&paths.claude_json)`.
- `Prices:` `Anthropic's` (Absent), `Anthropic's, with your prices.toml` (Loaded), or `Anthropic's; {message}` (Invalid).
The existing `Unpriced:` lines already list unpriced model ids.

- [ ] **Step 4: Run the tests**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 5: Run the Rust gate and commit**

```bash
git add src/cli tests
git commit -m "feat(cli): scope stats and export to an account and report the account in doctor

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Mod: send `auth`, show the account, spend and budget bars, wording

**Files:**
- Modify: `mod/hooks/register.tsx` (`session.start` forward adds `auth`)
- Modify: `mod/types/index.d.ts` (`AccountRef`, `BudgetView`, `Live.account`, `Live.budget`)
- Modify: `mod/hooks/client.ts` (`parseSnapshot` defaults `account` and `budget` to `null`)
- Modify: `mod/hooks/format.ts` (`WINDOWS.budget = 'Budget'`)
- Modify: `mod/hooks/state.ts` (`WINDOW_ORDER`, headline account line, limits section, cache wording)
- Test: `mod/tests/state.test.ts`, `mod/tests/client.test.ts`, `mod/tests/fixtures.ts`

**Interfaces:**
- Consumes: snapshot `live.account`, `live.budget`, windows `spend_limit` and `budget`.
- Produces:
  - `export type AccountRef = { id: string; label: string; kind: 'oauth' | 'api' | 'cloud' | 'gateway' }`
  - `export type BudgetView = { amount_usd: number; local_usd: number; period_start_ms: number; resets_at_ms: number }`
  - `export async function authOf($: EngineInterface): Promise<{ provider: string; base_url: string | null; api_key: boolean }>` in `register.tsx`

- [ ] **Step 1: Write the failing tests**

In `mod/tests/fixtures.ts`, `snapshot()` gains `account: null, budget: null` in `live`.
In `mod/tests/state.test.ts`:

```ts
describe('accounts and budgets', () => {
  const enterprise = { id: 'oauth:work', label: 'Acme · Enterprise', kind: 'oauth' as const }
  const budget = { amount_usd: 700, local_usd: 23.1, period_start_ms: Date.parse('2026-10-01T00:00:00Z'), resets_at_ms: Date.parse('2026-11-01T00:00:00Z') }
  const spend = (used: number) => ({
    window: 'spend_limit',
    used_pct: used,
    resets_at_ms: budget.resets_at_ms,
    sampled_at_ms: NOW_MS - MINUTE_MS,
    reset_since_sample: false,
    pace: { pct_per_hour: 0.01, projected_pct: 31, full_at_ms: null },
    calibration: { family: 'opus', pairs: 0, needed: 20, pct_per_usd: null },
  })

  it('names the account under the headline', () => {
    const lines = screen(input({ width: 60, snapshot: snapshot({ account: enterprise, limits: [] }) }))
    expect(text(line(lines, 'account')).trim()).toBe('Acme · Enterprise')
  })

  it('shows the spend limit in dollars when a budget is set', () => {
    const lines = screen(input({ width: 60, snapshot: snapshot({ account: enterprise, budget, limits: [spend(4)] }) }))
    expect(text(line(lines, 'limits')).trim().startsWith('Limits')).toBe(true)
    expect(text(line(lines, 'limit-spend_limit-detail')).trim()).toBe('$28.00 of $700 · resets in 20d · on pace for 31%')
    expect(text(line(lines, 'limit-spend_limit-here')).trim()).toBe('Claude Code here: $23.10')
  })

  it('hints at the budget command when no budget is set', () => {
    const lines = screen(input({ width: 60, snapshot: snapshot({ account: enterprise, limits: [spend(4)] }) }))
    expect(text(line(lines, 'limit-spend_limit-hint')).trim()).toBe('claude-plus budget set <usd> shows dollars')
  })

  it('an_exceeded_spend_limit_draws_full_with_its_real_percent', () => {
    const lines = screen(input({ width: 60, snapshot: snapshot({ account: enterprise, budget, limits: [spend(104)] }) }))
    const meter = line(lines, 'limit-spend_limit')
    expect(text(meter)).toContain('104%')
    expect(meter.segments[1]?.color).toBe('warning')
    expect(text(line(lines, 'limit-spend_limit-detail')).trim().startsWith('$728.00 of $700')).toBe(true)
  })

  it('draws a budget window as this machine only and aligns its label', () => {
    const own = { ...spend(3.3), window: 'budget' }
    const lines = screen(input({ width: 60, snapshot: snapshot({ account: enterprise, budget, limits: [own] }) }))
    expect(text(line(lines, 'limit-budget')).startsWith('  Budget ')).toBe(true)
    expect(text(line(lines, 'limit-budget-detail')).trim()).toBe('$23.10 of $700 here · resets in 20d · on pace for 31%')
  })

  it('says the cache lifetime is assumed from the account', () => {
    const lines = screen(input({ width: 60, expanded: ['cache'], snapshot: null }))
    expect(text(line(lines, 'explain-cache'))).toContain('assumed from your account')
  })
})
```

`resets in 20d` relies on `NOW_MS` being `2026-10-11T09:00:00Z` in the fixtures (it is) and on `duration` rounding 20 days 15 hours to `20d`; if `duration` prints `20d 15h`, use the printed form in both expectations, since the detail reuses `duration`.
Update the existing expectations that contain `Plan limits` to `Limits` (the padding of the heading line stays the same width).

In `mod/tests/client.test.ts`: `parseSnapshot` of a v1 body without `account` and `budget` returns `live.account === null` and `live.budget === null`.

In `mod/tests/register.test.ts`, follow the existing `session.start` test and assert the forwarded event's `data.auth` is `{ provider: 'anthropic', base_url: null, api_key: false }` with no env set, and `{ provider: 'bedrock', base_url: null, api_key: false }` with `CLAUDE_CODE_USE_BEDROCK=1` set through the test harness's env mock.

- [ ] **Step 2: Run to see them fail**

Run: `claude plugin test mod`
Expected: the new tests fail.

- [ ] **Step 3: Implement**

`register.tsx`:

```ts
async function authOf($: EngineInterface): Promise<{ provider: string; base_url: string | null; api_key: boolean }> {
  const set = (value: string | undefined) => value !== undefined && value !== '' && value !== '0'
  const provider = set(await $.env.get('CLAUDE_CODE_USE_BEDROCK'))
    ? 'bedrock'
    : set(await $.env.get('CLAUDE_CODE_USE_VERTEX'))
      ? 'vertex'
      : set(await $.env.get('CLAUDE_CODE_USE_FOUNDRY'))
        ? 'foundry'
        : 'anthropic'
  const baseUrl = await $.env.get('ANTHROPIC_BASE_URL')
  const apiKey = set(await $.env.get('ANTHROPIC_API_KEY')) || set(await $.env.get('ANTHROPIC_AUTH_TOKEN'))
  return { provider, base_url: baseUrl === undefined || baseUrl === '' ? null : baseUrl, api_key: apiKey }
}
```

and the forward becomes `quietly((async () => forward($, 'session.start', { cwd: e.cwd, surface: e.surface, interactive: e.isInteractive, auth: await authOf($) }))())`.
Each `$.env.get` takes a string literal, as `claude plugin validate` requires.

`client.ts` `parseSnapshot`: before returning, `if (!('account' in live)) live.account = null` and the same for `budget`.

`state.ts`:
- `WINDOW_ORDER = ['five_hour', 'seven_day', 'spend_limit', 'budget']`.
- `headline` pushes, after the headline line, `prose('account', snapshot.live.account.label, { dim: true })` when `snapshot?.live.account` is set, cut with `fitting(width, label, label.slice(0, width))`.
- `limits()`: heading text `Limits`; the meter label pads to `Math.max(5, ...windows.map(w => windowName(w.window).length))`.
- For a `spend_limit` window with `snapshot.live.budget` set, the detail is `${money(budget.amount_usd * used / 100)} of ${whole(budget.amount_usd)} · ${resets} · ${pace}` built from the same parts as `limitDetail`, followed by a dim line `limit-spend_limit-here`: `Claude Code here: ${money(budget.local_usd)}`; without a budget the existing detail stays and a dim line `limit-spend_limit-hint` reads `claude-plus budget set <usd> shows dollars`.
- For a `budget` window the detail is `${money(budget.local_usd)} of ${whole(budget.amount_usd)} here · ...`.
- `whole` formats a budget amount without cents when it is a whole number (`$700`), else with two decimals; add it to `format.ts` next to `money`.
- The meter of any window at or above 100 is drawn full (the `meter` helper already clamps the ratio; check `bar` clamps at 1 and add `Math.min(1, ...)` if it does not), coloured `warning` from `LIMIT_WARNING`.
- `cacheWhy`: `assumed from your plan` becomes `assumed from your account`.
- The `limits-learning` line is only shown for `five_hour`, as today.

Check every new line at width 24 (`MIN_CONTENT`) in a test: the detail lines must fall back to their `short` forms (`$28 of $700 · 20d · 31%`) when the long one does not fit, as `limitDetail` does with `compact`.

- [ ] **Step 4: Run the mod gate**

Run: `claude plugin validate mod && claude plugin test mod && (cd mod && npx --offline -p typescript@6.0.3 tsc --noEmit -p .)`
Expected: PASS; `validate` lists the six env names read.

- [ ] **Step 5: Commit**

```bash
git add mod
git commit -m "feat(mod): send the session's auth, name its account and show spend limits and budgets in dollars

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 11: Mod: History account cycle and budget alerts

**Files:**
- Modify: `mod/types/index.d.ts` (`Chart.account`, `HistoryReport.account`, `HistoryReport.accounts`, `Alerted.spendFor`, `Alerted.spendHighFor`)
- Modify: `mod/hooks/client.ts` (`historyPath(range, account)`)
- Modify: `mod/hooks/history.ts` (`accountLabel`, `accountChoices`, `nextAccount`, title)
- Modify: `mod/hooks/register.tsx` (control `a`, clear History caches on account change, pass account)
- Modify: `mod/hooks/alerts.ts` (spend alerts)
- Test: `mod/tests/history.test.ts`, `mod/tests/alerts.test.ts`, `mod/tests/client.test.ts`

**Interfaces:**
- Consumes: History route `account=` (Task 6), snapshot `live.account` (Task 5).
- Produces:
  - `Chart = { range: HistoryRange; metric: Metric; account?: string }` where `undefined` and `'current'` mean the session's account, `'all'` all accounts, any other value an account id
  - `historyPath(range, account: string | null)` with `account === null` meaning all
  - `resolvedAccount(chart: Chart, snapshot: Snapshot | null): string | null`
  - `nextAccount(chart: Chart, report: HistoryReport | null, snapshot: Snapshot | null): string`
  - Alerts kind `'spend'`

- [ ] **Step 1: Write the failing tests**

`client.test.ts`:

```ts
it('asks History for an account with its id encoded', () => {
  expect(historyPath('7d', null)).toBe('/history?range=7d&account=all')
  expect(historyPath('30d', 'gateway:https://gw:443')).toBe('/history?range=30d&account=gateway%3Ahttps%3A%2F%2Fgw%3A443')
})
```

`history.test.ts`:

```ts
describe('accounts', () => {
  const accounts = [{ id: 'oauth:me', label: 'Me · Max', kind: 'oauth' as const }, { id: 'oauth:work', label: 'Acme · Enterprise', kind: 'oauth' as const }]
  const current = snapshot({ account: accounts[1]! })
  const shown = { ...historyReport('7d'), account: 'oauth:work', accounts }

  it('resolves current to the session account and all to null', () => {
    expect(resolvedAccount({ range: '7d', metric: 'cost' }, current)).toBe('oauth:work')
    expect(resolvedAccount({ range: '7d', metric: 'cost', account: 'all' }, current)).toBe(null)
    expect(resolvedAccount({ range: '7d', metric: 'cost' }, snapshot({ account: null }))).toBe(null)
  })

  it('cycles current, the other accounts, then all', () => {
    const chart = (account?: string) => ({ range: '7d' as const, metric: 'cost' as const, account })
    expect(nextAccount(chart(), shown, current)).toBe('oauth:me')
    expect(nextAccount(chart('oauth:me'), shown, current)).toBe('all')
    expect(nextAccount(chart('all'), shown, current)).toBe('current')
  })

  it('offers the control only with two accounts or more', () => {
    expect(accountChoices({ range: '7d', metric: 'cost' }, { ...shown, accounts: [accounts[1]!] }, current)).toEqual([])
    expect(accountChoices({ range: '7d', metric: 'cost' }, shown, current).map(c => c.text)).toEqual(['Acme · Enterprise', 'Me · Max', 'All'])
  })
})
```

`alerts.test.ts`:

```ts
it('warns once a period when the spend pace goes past the limit and once at 90%', () => {
  const base = { nowMs: NOW_MS, local: local(), daemon: ok, known: EMPTY_ALERTED }
  const over = snapshot({ limits: [{ ...spendWindow(40), pace: { pct_per_hour: 0.2, projected_pct: 130, full_at_ms: NOW_MS + 10 * DAY_MS } }] })
  const first = alertsOf({ ...base, snapshot: over })
  expect(first.alerts.map(a => a.kind)).toEqual(['spend'])
  expect(first.alerts[0]?.text).toBe('At this pace the spend limit runs out in 10d, before its reset in 20d')
  expect(alertsOf({ ...base, snapshot: over, known: first.next }).alerts).toEqual([])
  const high = alertsOf({ ...base, snapshot: snapshot({ limits: [spendWindow(91)] }), known: first.next })
  expect(high.alerts.map(a => a.text)).toEqual(['91% of the spend limit is used, it resets in 20d'])
})
```

Define `spendWindow(used)` and `DAY_MS` in the test file from the fixtures' `NOW_MS`, with `resets_at_ms` on 2026-11-01T00:00:00Z; adapt the `20d` text to `duration`'s real output as in Task 10.

- [ ] **Step 2: Run to see them fail**

Run: `claude plugin test mod`
Expected: the new tests fail.

- [ ] **Step 3: Implement**

`client.ts`:

```ts
export function historyPath(range: HistoryRange, account: string | null): string {
  return `/history?range=${range}&account=${account === null ? 'all' : encodeURIComponent(account)}`
}
```

and `fetchHistory(io, socket, range, account)`; `parseHistory` defaults `account` to `null` and `accounts` to `[]` for older daemons.

`history.ts`:

```ts
export function resolvedAccount(chart: Chart, snapshot: Snapshot | null): string | null {
  const chosen = chart.account ?? 'current'
  if (chosen === 'all') return null
  if (chosen === 'current') return snapshot?.live.account?.id ?? null
  return chosen
}

function cycle(report: HistoryReport | null, snapshot: Snapshot | null): string[] {
  const own = snapshot?.live.account?.id ?? null
  const others = (report?.accounts ?? []).map(account => account.id).filter(id => id !== own)
  return [...(own === null ? [] : ['current']), ...others, 'all']
}

export function nextAccount(chart: Chart, report: HistoryReport | null, snapshot: Snapshot | null): string {
  const order = cycle(report, snapshot)
  const index = order.indexOf(chart.account ?? 'current')
  return order[(index + 1) % order.length] ?? 'all'
}

export function accountChoices(chart: Chart, report: HistoryReport | null, snapshot: Snapshot | null): Choice[] {
  if ((report?.accounts.length ?? 0) < 2) return []
  const label = (id: string) =>
    id === 'all' ? 'All' : id === 'current' ? snapshot?.live.account?.label ?? 'Current' : report?.accounts.find(a => a.id === id)?.label ?? id
  const active = chart.account ?? 'current'
  return cycle(report, snapshot).map(id => ({ text: label(id), isActive: id === active }))
}

export function accountLabel(chart: Chart, report: HistoryReport | null, snapshot: Snapshot | null): string {
  return accountChoices(chart, report, snapshot).find(choice => choice.isActive)?.text ?? 'All'
}
```

The History screen title gains ` · ${accountLabel(...)}` when `accountChoices` is not empty.

`register.tsx`:
- `pollHistory` passes `resolvedAccount(chart, await read($, snapshot))` and drops a reply whose `report.account` differs from the account it asked for.
- A new `cycleAccount($)` sets `chart.account` to `nextAccount(...)`, then clears the `history` and `failures` atoms and polls immediately.
- `controls` gains `{ key: 'account', hotkey: 'a', label: accountLabel(...), choices: accountChoices(...), onPress: () => quietly(cycleAccount($)) }` when `accountChoices` is not empty.
- When the session's account changes (a new `snapshot.live.account.id` differs from the one History was fetched for) and `chart.account` is `current`, clear the History caches too.

`alerts.ts`, after the 5h pace alert:

```ts
  const spend = limitsOf({ ...input, snapshot }).find(limit => limit.window === 'spend_limit' || limit.window === 'budget')
  const spendResets = spend?.resets_at_ms ?? null
  const spendFull = spend?.pace?.full_at_ms ?? null
  const fresh = (seen: number | null) => spendResets !== null && (seen === null || Math.abs(seen - spendResets) > RESET_TOLERANCE_MS)
  const name = spend?.window === 'budget' ? 'budget' : 'spend limit'
  if (spendFull !== null && spendResets !== null && spendFull < spendResets && fresh(known.spendFor)) {
    alerts.push({ kind: 'spend', text: `At this pace the ${name} runs out in ${duration(Math.max(0, spendFull - input.nowMs))}, before its reset in ${duration(spendResets - input.nowMs)}` })
    next.spendFor = spendResets
  }
  if (spend !== undefined && spend.used_pct >= 90 && spendResets !== null && fresh(known.spendHighFor)) {
    alerts.push({ kind: 'spend', text: `${percent(spend.used_pct)} of the ${name} is used, it resets in ${duration(spendResets - input.nowMs)}` })
    next.spendHighFor = spendResets
  }
```

`EMPTY_ALERTED` gains `spendFor: null, spendHighFor: null`; `ownAlerts` keeps them across sessions of the same day because they are keyed by the period's reset, not the session: change `ownAlerts` so a new session keeps `spendFor` and `spendHighFor` from `known`.
`Alert.kind` gains `'spend'`; the toast code in `register.tsx` treats it like `'pace'`, and the `alerts` option description in `mod/.claude-plugin/plugin.json` adds "or a spend limit or budget would run out".

- [ ] **Step 4: Run the mod gate**

Run: `claude plugin validate mod && claude plugin test mod && (cd mod && npx --offline -p typescript@6.0.3 tsc --noEmit -p .)`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add mod
git commit -m "feat(mod): switch History between accounts and warn before a spend limit or budget runs out

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 12: Docs and the end-to-end check

**Files:**
- Modify: `README.md`
- Create: `docs/manual-checks/m8-accounts.md`

**Interfaces:**
- Consumes: everything above.

- [ ] **Step 1: README**

Make these edits, one sentence per line, no em dash:
- The intro's "how fast it uses your subscription windows" becomes "how fast it uses your plan's limits, from a subscription's 5h and 7d windows to an enterprise monthly spend limit".
- Under "### 1. Live", say that the account the session bills to is named under the model, and that an enterprise spend limit shows in dollars once a budget is set.
- New section `## Accounts and budgets` after "### Alerts":
  - claude-plus keeps each account apart: logging out of a company account and into a private one never mixes their limits, History or tips.
  - It learns which account is signed in from Claude Code's `.claude.json`, read only, and an API key, a gateway (`ANTHROPIC_BASE_URL`) or Bedrock, Vertex or Foundry each count as their own account.
  - Sessions from before the upgrade show as Unknown under All accounts.
  - `claude-plus budget set 700` sets a monthly budget for the current account, reset on the 1st at 00:00 UTC; with a reported spend limit it turns the percent into dollars, and without one it is measured against what Claude Code on this machine spent.
  - Prices default to Anthropic's own; an optional `~/.claude-plus/prices.toml` with only the models whose prices differ overrides them, for example:
    ```toml
    [models.claude-opus-5-5]
    input = 3
    output = 15
    cache_read = 0.15
    ```
- The Commands table gains `claude-plus accounts [rename <account> <label>]`, `claude-plus budget set <usd> | show | clear [--account <account>]`, and `--account` on `stats` and `export`.
- The History paragraph mentions `a` to switch accounts.

- [ ] **Step 2: Manual check**

Create `docs/manual-checks/m8-accounts.md` in the style of `docs/manual-checks/m7-packaging.md`, with these checks, each with the exact command and what to see:
1. Signed in with the private subscription, start a session, send one prompt: the pane names `<org> · Max` under the model and shows 5h and 7d bars.
2. `/logout`, `/login` with the company account, start a new session, send one prompt: the pane names `<org> · Enterprise`, shows only `Spend` with the percent and the hint `claude-plus budget set <usd> shows dollars`, and no 7d bar from the private account.
3. `claude-plus budget set 700`: within a few seconds the Spend detail reads `$x of $700 · resets in …` and `Claude Code here: $y`, and the dollar figure matches the claude.ai usage page to within one percent of $700.
4. History view, press `a`: the title cycles through the company account, the private one and All, and the totals change accordingly.
5. `claude-plus accounts` lists both, with this month's spend and the budget on the company row.
6. `claude-plus stats --days 1` shows the company account, and `--account all` shows both sessions.
7. Switch back to the private login and start a session: its 5h bar continues from the value it had, with no reset and no "Learning" restart.
8. Look at every new line at 24, 60 and 144 columns in the dark and light themes; nothing is cut mid-word or misaligned.

- [ ] **Step 3: Run the full gate**

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check
claude plugin validate mod && claude plugin test mod && (cd mod && npx --offline -p typescript@6.0.3 tsc --noEmit -p .)
rg -n "\x{2014}" README.md docs/manual-checks/m8-accounts.md src mod/hooks || true
```

Expected: all green, and the grep prints nothing.

- [ ] **Step 4: Commit**

```bash
git add README.md docs/manual-checks/m8-accounts.md
git commit -m "docs: explain accounts, budgets and the price file, and add the accounts manual check

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
