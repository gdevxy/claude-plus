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

    pub fn session_started_at(&self, session_id: &str) -> Result<Option<i64>> {
        Ok(self
            .tx
            .prepare_cached("SELECT started_at FROM sessions WHERE id = ?1")?
            .query_row([session_id], |r| r.get(0))
            .optional()?
            .flatten())
    }

    pub fn account_is_current(&self, row: &AccountRow) -> Result<bool> {
        Ok(self
            .tx
            .prepare_cached(
                "SELECT 1 FROM accounts
                 WHERE id = ?1 AND (label_is_custom = 1 OR label = ?2)
                   AND organization_type IS ?3 AND billing_type IS ?4",
            )?
            .exists(params![
                row.id,
                row.label,
                row.organization_type,
                row.billing_type
            ])?)
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
            .prepare_cached(&format!(
                "SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = ?1"
            ))?
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

    #[cfg(test)]
    pub fn total_changes(&self) -> u64 {
        self.conn.total_changes()
    }

    pub fn unattributed_sessions(&self) -> Result<Vec<(String, i64)>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT s.id, s.started_at FROM sessions s
             LEFT JOIN session_accounts a ON a.session_id = s.id
             WHERE a.session_id IS NULL AND s.started_at IS NOT NULL
               AND s.started_at * 1000 >= (SELECT MIN(from_ms) FROM account_periods)
             ORDER BY s.started_at, s.id",
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
            .prepare_cached(
                "SELECT account_id, amount_usd, updated_ms FROM budgets WHERE account_id = ?1",
            )?
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
    let kind = AccountKind::parse(&kind).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            format!("unknown account kind {kind:?}").into(),
        )
    })?;
    Ok(AccountInfo {
        id: r.get(0)?,
        kind,
        label: r.get(2)?,
        organization_type: r.get(3)?,
        first_seen_ms: r.get(4)?,
    })
}

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
            ts: Some(started_at),
        })
        .unwrap();
        w.commit().unwrap();
    }

    #[test]
    fn periods_open_and_close_and_answer_which_account_was_active() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("a", "Acme · Enterprise"), 1_000)
            .unwrap();
        w.upsert_account(&oauth("b", "Me · Max"), 1_000).unwrap();
        w.open_period("oauth:a", 1_000).unwrap();
        assert_eq!(
            w.close_open_period(5_000).unwrap().as_deref(),
            Some("oauth:a")
        );
        w.open_period("oauth:b", 5_000).unwrap();
        w.commit().unwrap();
        assert_eq!(store.account_at(999).unwrap(), None);
        assert_eq!(store.account_at(1_000).unwrap().as_deref(), Some("oauth:a"));
        assert_eq!(store.account_at(4_999).unwrap().as_deref(), Some("oauth:a"));
        assert_eq!(store.account_at(5_000).unwrap().as_deref(), Some("oauth:b"));
        assert_eq!(
            store.account_at(i64::MAX).unwrap().as_deref(),
            Some("oauth:b")
        );
        assert_eq!(store.open_account().unwrap().as_deref(), Some("oauth:b"));
    }

    #[test]
    fn a_start_attribution_replaces_a_timeline_one_but_not_the_reverse() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("a", "A"), 0).unwrap();
        w.upsert_account(&oauth("b", "B"), 0).unwrap();
        assert!(
            w.attribute("s1", "oauth:a", AttributionSource::Timeline)
                .unwrap()
        );
        assert!(
            w.attribute("s1", "oauth:b", AttributionSource::Start)
                .unwrap()
        );
        assert!(
            !w.attribute("s1", "oauth:a", AttributionSource::Timeline)
                .unwrap()
        );
        assert!(
            !w.attribute("s1", "oauth:a", AttributionSource::Start)
                .unwrap()
        );
        w.commit().unwrap();
        assert_eq!(
            store.session_account("s1").unwrap().as_deref(),
            Some("oauth:b")
        );
    }

    #[test]
    fn unattributed_sessions_are_those_with_a_start_and_no_account() {
        let mut store = Store::open_in_memory().unwrap();
        session(&mut store, "s1", 10);
        session(&mut store, "s2", 20);
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("a", "A"), 0).unwrap();
        w.open_period("oauth:a", 0).unwrap();
        w.attribute("s1", "oauth:a", AttributionSource::Timeline)
            .unwrap();
        w.commit().unwrap();
        assert_eq!(
            store.unattributed_sessions().unwrap(),
            vec![("s2".to_string(), 20)]
        );
        assert_eq!(
            store.latest_session_account().unwrap().as_deref(),
            Some("oauth:a")
        );
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
        w.upsert_account(&oauth("a", "Acme · Enterprise"), 1)
            .unwrap();
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
        w.upsert_account(&oauth("a", "Acme · Enterprise"), 1)
            .unwrap();
        w.upsert_account(&oauth("a", "Acme Corp · Enterprise"), 2)
            .unwrap();
        w.commit().unwrap();
        assert_eq!(
            store.account("oauth:a").unwrap().unwrap().label,
            "Acme Corp · Enterprise"
        );
    }

    #[test]
    fn sessions_before_the_first_period_are_never_pending() {
        let mut store = Store::open_in_memory().unwrap();
        session(&mut store, "old", 10);
        session(&mut store, "new", 200);
        assert_eq!(store.unattributed_sessions().unwrap(), vec![]);
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("a", "A"), 0).unwrap();
        w.open_period("oauth:a", 100_000).unwrap();
        w.commit().unwrap();
        assert_eq!(
            store.unattributed_sessions().unwrap(),
            vec![("new".to_string(), 200)]
        );
    }

    #[test]
    fn an_unknown_account_kind_is_an_error() {
        let store = Store::open_in_memory().unwrap();
        store
            .conn
            .execute(
                "INSERT INTO accounts (id, kind, label, first_seen_ms) VALUES ('x', 'martian', 'X', 0)",
                [],
            )
            .unwrap();
        assert!(store.accounts().is_err());
        assert!(store.account("x").is_err());
    }

    #[test]
    fn clear_derived_keeps_accounts_periods_attributions_and_budgets() {
        let mut store = Store::open_in_memory().unwrap();
        session(&mut store, "s1", 10);
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("a", "A"), 0).unwrap();
        w.open_period("oauth:a", 0).unwrap();
        w.attribute("s1", "oauth:a", AttributionSource::Start)
            .unwrap();
        w.set_budget("oauth:a", 50.0, 1).unwrap();
        w.commit().unwrap();
        let mut w = store.writer().unwrap();
        w.clear_derived().unwrap();
        w.commit().unwrap();
        assert_eq!(store.accounts().unwrap().len(), 1);
        assert_eq!(store.open_account().unwrap().as_deref(), Some("oauth:a"));
        assert_eq!(
            store.session_account("s1").unwrap().as_deref(),
            Some("oauth:a")
        );
        assert!(store.budget("oauth:a").unwrap().is_some());
    }
}
