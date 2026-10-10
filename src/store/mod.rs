mod schema;
mod writer;

use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, FixedOffset, NaiveDate};
use rusqlite::{Connection, OptionalExtension};

pub use writer::*;

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
    pub unpriced: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CostStateDelta {
    pub session_id: String,
    pub ours_usd: f64,
    pub claude_usd: f64,
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
        let conn =
            Connection::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::migrated(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::migrated(Connection::open_in_memory()?)
    }

    fn migrated(mut conn: Connection) -> Result<Self> {
        let current: usize =
            conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))? as usize;
        let latest = schema::MIGRATIONS.len();
        if current > latest {
            bail!(
                "database schema v{current} is newer than this claude-plus supports (v{latest}); upgrade claude-plus"
            );
        }
        if current < latest {
            let tx = conn.transaction()?;
            for migration in &schema::MIGRATIONS[current..] {
                tx.execute_batch(migration)?;
            }
            tx.pragma_update(None, "user_version", latest as i64)?;
            tx.commit()?;
        }
        Ok(Self { conn })
    }

    pub fn writer(&mut self) -> Result<Writer<'_>> {
        Ok(Writer {
            tx: self.conn.transaction()?,
        })
    }

    pub fn checkpoint(&self, path: &str) -> Result<Option<FileCheckpoint>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, offset, lines, size, mtime, head_len, head_hash FROM files WHERE path = ?1",
                [path],
                |r| {
                    Ok(FileCheckpoint {
                        id: r.get(0)?,
                        offset: r.get::<_, i64>(1)? as u64,
                        lines: r.get::<_, i64>(2)? as u64,
                        size: r.get::<_, i64>(3)? as u64,
                        mtime: r.get(4)?,
                        head_len: r.get::<_, i64>(5)? as u64,
                        head_hash: r.get::<_, i64>(6)? as u64,
                    })
                },
            )
            .optional()?)
    }

    pub fn totals(&self) -> Result<Totals> {
        Ok(self.conn.query_row(
            "SELECT (SELECT COUNT(DISTINCT session_id) FROM requests), COUNT(*), COALESCE(SUM(cost_usd), 0),
                    SUM(CASE WHEN cost_usd IS NULL THEN 1 ELSE 0 END), MIN(ts), MAX(ts) FROM requests",
            [],
            |r| {
                Ok(Totals {
                    sessions: r.get::<_, i64>(0)? as u64,
                    requests: r.get::<_, i64>(1)? as u64,
                    priced_cost_usd: r.get(2)?,
                    unpriced_requests: r.get::<_, Option<i64>>(3)?.unwrap_or(0) as u64,
                    first_ts: r.get(4)?,
                    last_ts: r.get(5)?,
                })
            },
        )?)
    }

    pub fn daily_costs(&self, since_ts: i64, offset_seconds: i32) -> Result<Vec<DailyCost>> {
        let offset = FixedOffset::east_opt(offset_seconds).context("invalid UTC offset")?;
        let mut statement = self
            .conn
            .prepare("SELECT ts, model, cost_usd FROM requests WHERE ts >= ?1 ORDER BY ts")?;
        let mut grouped: std::collections::BTreeMap<(NaiveDate, String), DailyCost> =
            Default::default();
        let rows = statement.query_map([since_ts], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<f64>>(2)?,
            ))
        })?;
        for row in rows {
            let (ts, model, cost) = row?;
            let day = DateTime::from_timestamp(ts, 0)
                .context("timestamp out of range")?
                .with_timezone(&offset)
                .date_naive();
            let entry = grouped.entry((day, model.clone())).or_insert(DailyCost {
                day,
                model,
                requests: 0,
                cost_usd: 0.0,
                unpriced: 0,
            });
            entry.requests += 1;
            match cost {
                Some(usd) => entry.cost_usd += usd,
                None => entry.unpriced += 1,
            }
        }
        Ok(grouped.into_values().collect())
    }

    pub fn unpriced_models(&self) -> Result<Vec<(String, u64)>> {
        let mut statement = self.conn.prepare(
            "SELECT model, COUNT(*) FROM requests WHERE cost_usd IS NULL GROUP BY model ORDER BY COUNT(*) DESC",
        )?;
        let rows = statement.query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn cost_state_deltas(&self) -> Result<Vec<CostStateDelta>> {
        let mut statement = self.conn.prepare(
            "SELECT c.session_id, (SELECT COALESCE(SUM(cost_usd), 0) FROM requests r WHERE r.session_id = c.session_id), c.total_cost_usd
             FROM cost_states c
             WHERE NOT EXISTS (SELECT 1 FROM cost_states later WHERE later.session_id = c.session_id
                               AND (later.file_id, later.line_no) > (c.file_id, c.line_no))
             ORDER BY c.session_id",
        )?;
        let rows = statement.query_map([], |r| {
            Ok(CostStateDelta {
                session_id: r.get(0)?,
                ours_usd: r.get(1)?,
                claude_usd: r.get(2)?,
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

    pub fn file_count(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get::<_, i64>(0))? as u64)
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
    let location = crate::ingest::TranscriptFile::from_path(Path::new(&path));
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

        let utc = store.daily_costs(0, 0).unwrap();
        assert_eq!(utc.len(), 2);
        assert_eq!(utc[0].day, NaiveDate::from_ymd_opt(2026, 10, 1).unwrap());
        assert_eq!(
            (utc[0].model.as_str(), utc[0].requests, utc[0].cost_usd),
            ("claude-opus-5-5", 2, 2.0)
        );
        assert_eq!((utc[1].model.as_str(), utc[1].unpriced), ("mystery", 1));

        let zurich_summer = store.daily_costs(0, 2 * 3600).unwrap();
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
            total_cost_usd: 0.20,
            json: "{}".into(),
        })
        .unwrap();
        w.cost_state(&CostStateRow {
            file_id: file,
            line_no: 9,
            session_id: "s1".into(),
            total_cost_usd: 0.75,
            json: "{}".into(),
        })
        .unwrap();
        w.commit().unwrap();
        assert_eq!(
            store.cost_state_deltas().unwrap(),
            vec![CostStateDelta {
                session_id: "s1".into(),
                ours_usd: 0.75,
                claude_usd: 0.75
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
    fn reset_file_removes_rows_keyed_by_file() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let file = w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        w.raw(file, 1, "attachment", "{}").unwrap();
        w.attachment(&AttachmentRow {
            file_id: file,
            line_no: 1,
            session_id: "s".into(),
            ts: None,
            kind: "k".into(),
            chars: 1,
            names: vec![],
        })
        .unwrap();
        w.compaction(&CompactionRow {
            file_id: file,
            line_no: 2,
            session_id: "s".into(),
            ts: None,
            trigger: None,
            pre_tokens: None,
            post_tokens: None,
        })
        .unwrap();
        w.cost_state(&CostStateRow {
            file_id: file,
            line_no: 3,
            session_id: "s".into(),
            total_cost_usd: 1.0,
            json: "{}".into(),
        })
        .unwrap();
        w.advance_file(
            file,
            &FileProgress {
                offset: 50,
                lines: 3,
                size: 50,
                mtime: 1,
                head_len: 50,
                head_hash: 9,
            },
            2,
        )
        .unwrap();
        w.reset_file(file).unwrap();
        w.commit().unwrap();
        for table in ["raw_records", "attachments", "compactions", "cost_states"] {
            let count: i64 = store
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "{table}");
        }
        let checkpoint = store.checkpoint("/p/proj/s1.jsonl").unwrap().unwrap();
        assert_eq!((checkpoint.offset, checkpoint.lines), (0, 0));
        assert_eq!(store.parse_error_total().unwrap(), 0);
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

    #[test]
    fn newer_schema_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 99)
            .unwrap();
        let error = Store::open(&path).err().expect("must refuse").to_string();
        assert_eq!(
            error,
            format!(
                "database schema v99 is newer than this claude-plus supports (v{}); upgrade claude-plus",
                schema::MIGRATIONS.len()
            )
        );
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
