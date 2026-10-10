mod reconcile;
mod schema;
mod upgrade;
mod writer;

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDate, TimeZone};
use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};

pub use reconcile::Reconciliation;
pub use upgrade::Upgrade;
pub use writer::*;

pub(crate) const GENERATION_MARK: &str = "#gen-";
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
#[error("another claude-plus is writing, try again in a moment")]
pub struct Busy;

pub struct Store {
    conn: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCheckpoint {
    pub id: i64,
    pub offset: u64,
    pub lines: u64,
    pub size: u64,
    pub mtime: i64,
    pub head_len: u64,
    pub head_hash: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Totals {
    pub sessions: u64,
    pub requests: u64,
    pub priced_cost_usd: f64,
    pub reconciled_usd: f64,
    pub unpriced_requests: u64,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DailyCost {
    pub day: NaiveDate,
    pub model: String,
    pub requests: u64,
    pub cost_usd: f64,
    pub reconciled_usd: f64,
    pub unpriced: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CostStateDelta {
    pub session_id: String,
    pub ours_usd: f64,
    pub reconciled_usd: f64,
    pub claude_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawRow {
    pub file_id: i64,
    pub line_no: u64,
    pub kind: String,
    pub json: String,
    pub project: String,
    pub session_hint: String,
    pub agent_id: Option<String>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_progress(path, &mut |_| {})
    }

    pub fn open_with_progress(path: &Path, on_upgrade: &mut dyn FnMut(Upgrade)) -> Result<Self> {
        Self::open_writable(path, on_upgrade).map_err(|error| naming(path, error))
    }

    fn open_writable(path: &Path, on_upgrade: &mut dyn FnMut(Upgrade)) -> Result<Self> {
        let conn = Connection::open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("cannot restrict {} to its owner", path.display()))?;
        }
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::migrated(conn, on_upgrade)
    }

    pub fn open_read_only(path: &Path) -> Result<Self> {
        let open = || -> Result<Self> {
            let conn = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            let store = Self { conn };
            store.refuse_newer_schema()?;
            Ok(store)
        };
        open().map_err(|error| naming(path, error))
    }

    pub fn schema_version(&self) -> Result<usize> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))? as usize)
    }

    pub fn latest_schema_version() -> usize {
        schema::MIGRATIONS.len()
    }

    fn refuse_newer_schema(&self) -> Result<()> {
        let current = self.schema_version()?;
        let latest = Self::latest_schema_version();
        if current > latest {
            bail!(
                "database schema v{current} is newer than this claude-plus supports (v{latest}); upgrade claude-plus"
            );
        }
        Ok(())
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::migrated(Connection::open_in_memory()?, &mut |_| {})
    }

    fn migrated(conn: Connection, on_upgrade: &mut dyn FnMut(Upgrade)) -> Result<Self> {
        let mut store = Self { conn };
        store.refuse_newer_schema()?;
        let current = store.schema_version()?;
        for (index, migration) in schema::MIGRATIONS.iter().enumerate().skip(current) {
            let version = index as i64 + 1;
            match migration {
                schema::Migration::Sql(sql) => {
                    let tx = begin_write(&mut store.conn)?;
                    tx.execute_batch(sql)?;
                    tx.pragma_update(None, "user_version", version)?;
                    tx.commit()?;
                }
                schema::Migration::Rust(step) => {
                    step(&mut store.conn, on_upgrade)?;
                    store.conn.pragma_update(None, "user_version", version)?;
                }
            }
        }
        upgrade::compact_if_pending(&store.conn, on_upgrade)?;
        Ok(store)
    }

    pub fn writer(&mut self) -> Result<Writer<'_>> {
        Ok(Writer {
            tx: begin_write(&mut self.conn)?,
        })
    }

    #[cfg(test)]
    pub(crate) fn set_busy_timeout(&self, timeout: std::time::Duration) {
        self.conn.busy_timeout(timeout).unwrap();
    }

    pub fn checkpoint(&self, path: &str) -> Result<Option<FileCheckpoint>> {
        checkpoint(&self.conn, path)
    }

    pub fn totals(&self) -> Result<Totals> {
        Ok(self.conn.query_row(
            "SELECT (SELECT COUNT(DISTINCT session_id) FROM requests), COUNT(*), COALESCE(SUM(cost_usd), 0),
                    SUM(CASE WHEN cost_usd IS NULL THEN 1 ELSE 0 END), MIN(ts), MAX(ts),
                    (SELECT COALESCE(SUM(cost_usd), 0) FROM adjustments) FROM requests",
            [],
            |r| {
                let reconciled_usd: f64 = r.get(6)?;
                Ok(Totals {
                    sessions: r.get::<_, i64>(0)? as u64,
                    requests: r.get::<_, i64>(1)? as u64,
                    priced_cost_usd: r.get::<_, f64>(2)? + reconciled_usd,
                    reconciled_usd,
                    unpriced_requests: r.get::<_, Option<i64>>(3)?.unwrap_or(0) as u64,
                    first_ts: r.get(4)?,
                    last_ts: r.get(5)?,
                })
            },
        )?)
    }

    pub fn daily_costs<Tz: TimeZone>(
        &self,
        since_ts: i64,
        until_ts: i64,
        tz: &Tz,
    ) -> Result<Vec<DailyCost>> {
        let mut grouped = BTreeMap::new();
        for (ts, model, cost) in self.costs_between("requests", since_ts, until_ts)? {
            let day = daily_entry(&mut grouped, ts, model, tz)?;
            day.requests += 1;
            match cost {
                Some(usd) => day.cost_usd += usd,
                None => day.unpriced += 1,
            }
        }
        for (ts, model, cost) in self.costs_between("adjustments", since_ts, until_ts)? {
            if let Some(usd) = cost {
                let day = daily_entry(&mut grouped, ts, model, tz)?;
                day.cost_usd += usd;
                day.reconciled_usd += usd;
            }
        }
        Ok(grouped.into_values().collect())
    }

    fn costs_between(
        &self,
        table: &str,
        since_ts: i64,
        until_ts: i64,
    ) -> Result<Vec<(i64, String, Option<f64>)>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT ts, model, cost_usd FROM {table} WHERE ts >= ?1 AND ts < ?2 ORDER BY ts"
        ))?;
        let rows = statement.query_map([since_ts, until_ts], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn unpriced_models(&self) -> Result<Vec<(String, u64)>> {
        let mut statement = self.conn.prepare(
            "SELECT model, COUNT(*) FROM requests WHERE cost_usd IS NULL GROUP BY model ORDER BY COUNT(*) DESC",
        )?;
        let rows = statement.query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn cost_state_deltas(&self) -> Result<Vec<CostStateDelta>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT c.session_id, (SELECT COALESCE(SUM(cost_usd), 0) FROM requests r WHERE r.session_id = c.session_id),
                    (SELECT COALESCE(SUM(cost_usd), 0) FROM adjustments a WHERE a.session_id = c.session_id), c.total_cost_usd
             FROM cost_states c
             WHERE {} ORDER BY c.session_id",
            reconcile::LATEST_COST_STATE
        ))?;
        let rows = statement.query_map([], |r| {
            Ok(CostStateDelta {
                session_id: r.get(0)?,
                ours_usd: r.get(1)?,
                reconciled_usd: r.get(2)?,
                claude_usd: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn raw_files(&self) -> Result<Vec<(i64, String)>> {
        raw_files(&self.conn)
    }

    pub fn raw_records(&self, file_id: i64, kinds: &[&str]) -> Result<Vec<RawRow>> {
        raw_records(&self.conn, file_id, kinds)
    }

    pub fn parse_error_total(&self) -> Result<u64> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(SUM(parse_errors), 0) FROM files",
            [],
            |r| r.get::<_, i64>(0),
        )? as u64)
    }

    #[cfg(test)]
    pub(crate) fn dump(&self, table: &str) -> Vec<String> {
        let mut statement = self
            .conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1, 2"))
            .unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |r| {
                (0..columns)
                    .map(|i| {
                        r.get::<_, rusqlite::types::Value>(i)
                            .map(|v| format!("{v:?}"))
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map(|cells| cells.join("|"))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    pub fn file_count(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get::<_, i64>(0))? as u64)
    }
}

fn naming(path: &Path, error: anyhow::Error) -> anyhow::Error {
    match error.downcast_ref::<rusqlite::Error>() {
        Some(sqlite) if !error.is::<Busy>() => {
            anyhow::anyhow!("cannot open {}: {sqlite}", path.display())
        }
        _ => error,
    }
}

pub(crate) fn begin_write(conn: &mut Connection) -> Result<Transaction<'_>> {
    conn.transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(friendly)
}

pub(crate) fn friendly(error: rusqlite::Error) -> anyhow::Error {
    if error.sqlite_error_code() == Some(ErrorCode::DatabaseBusy) {
        Busy.into()
    } else {
        error.into()
    }
}

pub(crate) fn checkpoint(conn: &Connection, path: &str) -> Result<Option<FileCheckpoint>> {
    Ok(conn
        .prepare_cached(
            "SELECT id, offset, lines, size, mtime, head_len, head_hash FROM files WHERE path = ?1",
        )?
        .query_row([path], |r| {
            Ok(FileCheckpoint {
                id: r.get(0)?,
                offset: r.get::<_, i64>(1)? as u64,
                lines: r.get::<_, i64>(2)? as u64,
                size: r.get::<_, i64>(3)? as u64,
                mtime: r.get(4)?,
                head_len: r.get::<_, i64>(5)? as u64,
                head_hash: r.get::<_, i64>(6)? as u64,
            })
        })
        .optional()?)
}

fn daily_entry<'a, Tz: TimeZone>(
    grouped: &'a mut BTreeMap<(NaiveDate, String), DailyCost>,
    ts: i64,
    model: String,
    tz: &Tz,
) -> Result<&'a mut DailyCost> {
    let day = DateTime::from_timestamp(ts, 0)
        .context("timestamp out of range")?
        .with_timezone(tz)
        .date_naive();
    Ok(grouped.entry((day, model.clone())).or_insert(DailyCost {
        day,
        model,
        requests: 0,
        cost_usd: 0.0,
        reconciled_usd: 0.0,
        unpriced: 0,
    }))
}

fn live_path(path: &str) -> &str {
    match path.rsplit_once(GENERATION_MARK) {
        Some((live, generation))
            if !generation.is_empty() && generation.bytes().all(|b| b.is_ascii_digit()) =>
        {
            live
        }
        _ => path,
    }
}

pub(crate) fn raw_files(conn: &Connection) -> Result<Vec<(i64, String)>> {
    let mut statement = conn.prepare(
        "SELECT id, path FROM files WHERE id IN (SELECT DISTINCT file_id FROM raw_records) ORDER BY id",
    )?;
    let rows = statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub(crate) fn raw_records(conn: &Connection, file_id: i64, kinds: &[&str]) -> Result<Vec<RawRow>> {
    let path: String = conn.query_row("SELECT path FROM files WHERE id = ?1", [file_id], |r| {
        r.get(0)
    })?;
    let location = crate::ingest::TranscriptFile::from_path(Path::new(live_path(&path)));
    let mut sql = String::from("SELECT line_no, kind, json FROM raw_records WHERE file_id = ?1");
    if !kinds.is_empty() {
        let placeholders = vec!["?"; kinds.len()].join(",");
        sql.push_str(&format!(" AND kind IN ({placeholders})"));
    }
    sql.push_str(" ORDER BY line_no");
    let mut statement = conn.prepare(&sql)?;
    let params = std::iter::once(&file_id as &dyn rusqlite::ToSql)
        .chain(kinds.iter().map(|kind| kind as &dyn rusqlite::ToSql));
    let rows = statement.query_map(rusqlite::params_from_iter(params), |r| {
        Ok(RawRow {
            file_id,
            line_no: r.get::<_, i64>(0)? as u64,
            kind: r.get(1)?,
            json: r.get(2)?,
            project: location.project.clone(),
            session_hint: location.session_hint.clone(),
            agent_id: location.agent_id.clone(),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::TokenCounts;

    fn request(key: &str, ts: i64, model: &str, cost: Option<f64>) -> RequestRow {
        RequestRow {
            key: key.into(),
            session_id: "s1".into(),
            ts,
            model: model.into(),
            tokens: TokenCounts {
                input: 1,
                output: 2,
                ..Default::default()
            },
            cost_usd: cost,
            is_sidechain: false,
            agent_id: None,
            skill: None,
            mcp_server: None,
            effort: None,
        }
    }

    #[test]
    fn requests_are_deduplicated_by_key() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        assert_eq!(
            w.request(&request("r1", 100, "m", Some(1.0))).unwrap(),
            RequestWrite::Inserted
        );
        assert_eq!(
            w.request(&request("r1", 100, "m", Some(1.0))).unwrap(),
            RequestWrite::Unchanged
        );
        w.commit().unwrap();
        assert_eq!(store.totals().unwrap().requests, 1);
    }

    #[test]
    fn streamed_records_keep_the_highest_output() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let mut partial = request("r1", 100, "m", Some(1.0));
        partial.tokens.output = 10;
        let mut last = request("r1", 100, "m", Some(3.0));
        last.tokens.output = 900;
        assert_eq!(w.request(&partial).unwrap(), RequestWrite::Inserted);
        assert_eq!(w.request(&last).unwrap(), RequestWrite::Updated);
        assert_eq!(w.request(&partial).unwrap(), RequestWrite::Unchanged);
        w.commit().unwrap();
        let totals = store.totals().unwrap();
        assert_eq!((totals.requests, totals.priced_cost_usd), (1, 3.0));
    }

    #[test]
    fn checkpoints_round_trip() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let id = w.file("/p/a.jsonl", 10, 5).unwrap();
        assert_eq!(w.file("/p/a.jsonl", 10, 5).unwrap(), id);
        w.advance_file(
            id,
            &FileProgress {
                offset: 120,
                lines: 3,
                size: 200,
                mtime: 7,
                head_len: 64,
                head_hash: 0xdead_beef_dead_beef,
            },
            1,
        )
        .unwrap();
        w.commit().unwrap();
        assert_eq!(
            store.checkpoint("/p/a.jsonl").unwrap(),
            Some(FileCheckpoint {
                id,
                offset: 120,
                lines: 3,
                size: 200,
                mtime: 7,
                head_len: 64,
                head_hash: 0xdead_beef_dead_beef
            })
        );
        assert_eq!(store.parse_error_total().unwrap(), 1);
        assert_eq!(store.checkpoint("/p/missing.jsonl").unwrap(), None);
    }

    #[test]
    fn daily_costs_group_by_local_day_and_model() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let day1 = DateTime::parse_from_rfc3339("2026-10-01T23:30:00Z")
            .unwrap()
            .timestamp();
        w.request(&request("a", day1, "claude-opus-5-5", Some(1.5)))
            .unwrap();
        w.request(&request("b", day1 + 60, "claude-opus-5-5", Some(0.5)))
            .unwrap();
        w.request(&request("c", day1 + 120, "mystery", None))
            .unwrap();
        w.commit().unwrap();

        let utc = store.daily_costs(0, i64::MAX, &chrono::Utc).unwrap();
        assert_eq!(utc.len(), 2);
        assert_eq!(utc[0].day, NaiveDate::from_ymd_opt(2026, 10, 1).unwrap());
        assert_eq!(
            (utc[0].model.as_str(), utc[0].requests, utc[0].cost_usd),
            ("claude-opus-5-5", 2, 2.0)
        );
        assert_eq!((utc[1].model.as_str(), utc[1].unpriced), ("mystery", 1));

        let zurich_summer = store
            .daily_costs(
                0,
                i64::MAX,
                &chrono::FixedOffset::east_opt(2 * 3600).unwrap(),
            )
            .unwrap();
        assert_eq!(
            zurich_summer[0].day,
            NaiveDate::from_ymd_opt(2026, 10, 2).unwrap()
        );

        let totals = store.totals().unwrap();
        assert_eq!(
            (
                totals.requests,
                totals.unpriced_requests,
                totals.priced_cost_usd
            ),
            (3, 1, 2.0)
        );
        assert_eq!(
            store.unpriced_models().unwrap(),
            vec![("mystery".to_string(), 1)]
        );
    }

    #[test]
    fn daily_costs_exclude_requests_at_or_after_until() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.request(&request("a", 100, "m", Some(1.0))).unwrap();
        w.request(&request("b", 200, "m", Some(1.0))).unwrap();
        w.commit().unwrap();
        let rows = store.daily_costs(0, 200, &chrono::Utc).unwrap();
        assert_eq!(rows[0].requests, 1);
    }

    #[test]
    fn sessions_track_first_and_last_seen_and_titles() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        for ts in [Some(50), Some(10), None, Some(90)] {
            w.session(&SessionRow {
                id: "s1".into(),
                project: "p".into(),
                cwd: Some("/w".into()),
                ts,
            })
            .unwrap();
        }
        w.title("s1", "AI title", false).unwrap();
        w.title("s1", "My title", true).unwrap();
        w.title("s1", "Later AI title", false).unwrap();
        w.commit().unwrap();
        let (started, last, title): (i64, i64, String) = store
            .conn
            .query_row(
                "SELECT started_at, last_seen_at, title FROM sessions WHERE id = 's1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((started, last, title.as_str()), (10, 90, "My title"));
    }

    #[test]
    fn cost_state_deltas_compare_latest_claude_total_with_ours() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let file = w.file("/p/s1.jsonl", 0, 0).unwrap();
        w.request(&request("a", 1, "claude-opus-5-5", Some(0.25)))
            .unwrap();
        w.request(&request("b", 2, "claude-opus-5-5", Some(0.50)))
            .unwrap();
        w.cost_state(&CostStateRow {
            file_id: file,
            line_no: 3,
            session_id: "s1".into(),
            total_cost_usd: Some(0.20),
            model_count: 1,
            json: "{}".into(),
        })
        .unwrap();
        w.cost_state(&CostStateRow {
            file_id: file,
            line_no: 9,
            session_id: "s1".into(),
            total_cost_usd: Some(0.75),
            model_count: 1,
            json: "{}".into(),
        })
        .unwrap();
        w.commit().unwrap();
        assert_eq!(
            store.cost_state_deltas().unwrap(),
            vec![CostStateDelta {
                session_id: "s1".into(),
                ours_usd: 0.75,
                reconciled_usd: 0.0,
                claude_usd: Some(0.75)
            }]
        );
    }

    #[test]
    fn clear_derived_keeps_raw_and_files() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let file = w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        w.raw(file, 1, "assistant", "{}").unwrap();
        w.request(&request("a", 1, "m", Some(1.0))).unwrap();
        w.commit().unwrap();
        let mut w = store.writer().unwrap();
        w.clear_derived().unwrap();
        assert_eq!(
            w.raw_files().unwrap(),
            vec![(file, "/p/proj/s1.jsonl".to_string())]
        );
        let raw = w.raw_records(file, &["assistant"]).unwrap();
        assert_eq!(raw.len(), 1);
        assert_eq!(raw[0].project, "proj");
        w.commit().unwrap();
        assert_eq!(store.totals().unwrap().requests, 0);
    }

    #[test]
    fn abandoned_clear_derived_leaves_previous_data_intact() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        w.request(&request("a", 1, "m", Some(1.0))).unwrap();
        w.commit().unwrap();
        let mut w = store.writer().unwrap();
        w.clear_derived().unwrap();
        drop(w);
        assert_eq!(store.totals().unwrap().requests, 1);
    }

    #[test]
    fn raw_records_filter_by_kind_in_line_order() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let file = w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        w.raw(file, 3, "user", "{}").unwrap();
        w.raw(file, 2, "assistant", "{}").unwrap();
        w.raw(file, 1, "system", "{}").unwrap();
        w.commit().unwrap();
        let kinds = |wanted: &[&str]| -> Vec<(u64, String)> {
            store
                .raw_records(file, wanted)
                .unwrap()
                .into_iter()
                .map(|r| (r.line_no, r.kind))
                .collect()
        };
        assert_eq!(
            kinds(&["assistant", "user"]),
            vec![(2, "assistant".into()), (3, "user".into())]
        );
        assert_eq!(kinds(&[]).len(), 3);
    }

    #[test]
    fn retiring_a_file_keeps_its_rows_under_a_free_generation_path() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let first = w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        w.raw(first, 1, "attachment", "{}").unwrap();
        w.cost_state(&CostStateRow {
            file_id: first,
            line_no: 2,
            session_id: "s".into(),
            total_cost_usd: Some(1.0),
            model_count: 1,
            json: "{}".into(),
        })
        .unwrap();
        assert_eq!(w.retire_file(first).unwrap(), "/p/proj/s1.jsonl#gen-1");
        let second = w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        assert_ne!(second, first);
        w.raw(second, 1, "user", "{}").unwrap();
        assert_eq!(w.retire_file(second).unwrap(), "/p/proj/s1.jsonl#gen-2");
        w.commit().unwrap();

        assert_eq!(store.checkpoint("/p/proj/s1.jsonl").unwrap(), None);
        assert_eq!(
            store.raw_files().unwrap(),
            vec![
                (first, "/p/proj/s1.jsonl#gen-1".to_string()),
                (second, "/p/proj/s1.jsonl#gen-2".to_string())
            ]
        );
        let retired = store.raw_records(first, &[]).unwrap();
        assert_eq!(
            (
                retired[0].project.as_str(),
                retired[0].session_hint.as_str()
            ),
            ("proj", "s1")
        );
        let cost_states: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM cost_states", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cost_states, 1);
    }

    #[test]
    fn generation_numbers_ignore_similar_paths() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.file("/p/proj/s1.jsonl.bak#gen-7", 0, 0).unwrap();
        w.file("/p/proj/s10.jsonl#gen-3", 0, 0).unwrap();
        let live = w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        assert_eq!(w.retire_file(live).unwrap(), "/p/proj/s1.jsonl#gen-1");
    }

    #[test]
    fn reopening_a_file_store_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        {
            let mut store = Store::open(&path).unwrap();
            let mut w = store.writer().unwrap();
            w.request(&request("r1", 1, "m", Some(1.0))).unwrap();
            w.commit().unwrap();
        }
        assert_eq!(Store::open(&path).unwrap().totals().unwrap().requests, 1);
    }

    fn seeded(path: &Path, version: usize, seed: &str) {
        let conn = Connection::open(path).unwrap();
        for migration in &schema::MIGRATIONS[..version] {
            let schema::Migration::Sql(sql) = migration else {
                panic!("seeding stops at SQL migrations")
            };
            conn.execute_batch(sql).unwrap();
        }
        conn.execute_batch(seed).unwrap();
        conn.pragma_update(None, "user_version", version as i64)
            .unwrap();
    }

    #[test]
    fn upgrading_from_v3_classifies_stored_cost_states() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        seeded(
            &path,
            3,
            r#"INSERT INTO cost_states VALUES
                (1, 1, 's1', 0.5, '{"type":"cost-state","totalCostUSD":0.5,"modelUsage":{"m":{"outputTokens":1},"broken":7}}'),
                (1, 2, 's1', 0.0, '{"type":"cost-state","modelUsage":{}}'),
                (1, 3, 's1', 0.0, '{"type":"cost-state","totalCostUSD":0}');"#,
        );
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.schema_version().unwrap(),
            Store::latest_schema_version()
        );
        let rows: Vec<(i64, Option<f64>, i64)> = store
            .conn
            .prepare(
                "SELECT line_no, total_cost_usd, model_count FROM cost_states ORDER BY line_no",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![(1, Some(0.5), 1), (2, None, 0), (3, Some(0.0), 0)]
        );
    }

    #[test]
    fn upgrading_from_v4_redacts_stored_raw_records_and_compacts() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        seeded(
            &path,
            4,
            r#"INSERT INTO files (id, path) VALUES (1, '/p/proj/s1.jsonl');
               INSERT INTO raw_records VALUES
                 (1, 1, 'user', '{"type":"user","message":{"role":"user","content":"my password"}}'),
                 (1, 2, 'assistant', '{"type":"assistant","message":{"content":[{"type":"text","text":"secret answer"},{"type":"tool_use","id":"t","name":"Read","input":{"file_path":"/a"}}]}}'),
                 (1, 3, 'custom-title', '{"type":"custom-title","customTitle":"Keep me"}');
               INSERT INTO cost_states VALUES (1, 4, 's1', 1.0, 1, '{"type":"cost-state","note":"' || printf('%.3000c', 'x') || '"}');"#,
        );
        let mut events = Vec::new();
        let store = Store::open_with_progress(&path, &mut |event| events.push(event)).unwrap();
        assert_eq!(
            store.schema_version().unwrap(),
            Store::latest_schema_version()
        );
        let raw: Vec<String> = store
            .raw_records(1, &[])
            .unwrap()
            .into_iter()
            .map(|row| row.json)
            .collect();
        assert_eq!(
            raw,
            vec![
                r#"{"message":{"content":{"$chars":11},"role":"user"},"type":"user"}"#,
                r#"{"message":{"content":[{"text":{"$chars":13},"type":"text"},{"id":"t","input":{"file_path":"/a"},"name":"Read","type":"tool_use"}]},"type":"assistant"}"#,
                r#"{"customTitle":"Keep me","type":"custom-title"}"#,
            ]
        );
        assert_eq!(
            store.dump("cost_states"),
            vec![
                r#"Integer(1)|Integer(4)|Text("s1")|Real(1.0)|Integer(1)|Text("{\"note\":{\"$chars\":3000},\"type\":\"cost-state\"}")"#
            ]
        );
        assert_eq!(
            events,
            vec![
                Upgrade::Records { done: 0, total: 3 },
                Upgrade::Records { done: 3, total: 3 },
                Upgrade::Compacting
            ]
        );
        assert_eq!(store.dump("meta"), Vec::<String>::new());
    }

    #[test]
    fn a_second_writer_is_asked_to_retry() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        let mut first = Store::open(&path).unwrap();
        let mut second = Store::open(&path).unwrap();
        second.set_busy_timeout(std::time::Duration::from_millis(10));
        let _held = first.writer().unwrap();
        let error = second.writer().err().expect("must be busy");
        assert_eq!(
            error.to_string(),
            "another claude-plus is writing, try again in a moment"
        );
        assert!(error.is::<Busy>());
        let error = second
            .reconcile(&crate::pricing::PriceTable::bundled())
            .unwrap_err();
        assert!(error.is::<Busy>());
    }

    #[test]
    fn writer_reads_checkpoints_inside_its_transaction() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let id = w.file("/p/a.jsonl", 3, 4).unwrap();
        let checkpoint = w.checkpoint("/p/a.jsonl").unwrap().unwrap();
        assert_eq!(
            (checkpoint.id, checkpoint.size, checkpoint.mtime),
            (id, 3, 4)
        );
        assert_eq!(w.checkpoint("/p/b.jsonl").unwrap(), None);
    }

    #[test]
    fn newer_schema_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 99)
            .unwrap();
        let expected = format!(
            "database schema v99 is newer than this claude-plus supports (v{}); upgrade claude-plus",
            schema::MIGRATIONS.len()
        );
        let error = Store::open(&path).err().expect("must refuse").to_string();
        assert_eq!(error, expected);
        let error = Store::open_read_only(&path)
            .err()
            .expect("must refuse")
            .to_string();
        assert_eq!(error, expected);
    }

    #[test]
    fn read_only_open_neither_creates_nor_migrates() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        assert!(Store::open_read_only(&path).is_err());
        assert!(!path.exists());
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 1)
            .unwrap();
        let store = Store::open_read_only(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 1);
        assert_eq!(Store::latest_schema_version(), schema::MIGRATIONS.len());
    }

    #[cfg(unix)]
    #[test]
    fn database_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        Store::open(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
