use anyhow::Result;
use rusqlite::types::Value;
use rusqlite::{OptionalExtension, params};

use super::context::MAIN_THREAD;
use super::timeline::{POINT_COLUMNS, point_of};
use super::{LimitSample, PREFIX_KINDS, RequestPoint, Store};
use crate::transcript::TokenCounts;

#[derive(Debug, Clone, PartialEq)]
pub struct SpentRequest {
    pub session_id: String,
    pub point: RequestPoint,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpentAdjustment {
    pub session_id: String,
    pub ts: i64,
    pub model: String,
    pub tokens: TokenCounts,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub id: String,
    pub project: String,
    pub cwd: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionPoint {
    pub session_id: String,
    pub ts: i64,
    pub main: bool,
}

#[cfg(test)]
impl super::Writer<'_> {
    pub(crate) fn adjustment(&mut self, row: &SpentAdjustment) -> Result<()> {
        self.tx.execute(
            "INSERT INTO adjustments (session_id, model, ts, input, output, cache_read, cache_write, cost_usd)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                row.session_id,
                row.model,
                row.ts,
                row.tokens.input as i64,
                row.tokens.output as i64,
                row.tokens.cache_read as i64,
                row.tokens.cache_write() as i64,
                row.cost_usd
            ],
        )?;
        Ok(())
    }
}

const REQUESTS_BETWEEN: &str =
    "FROM requests WHERE ts >= ?1 AND ts < ?2 ORDER BY session_id, ts, key";

const LAST_MAIN_REQUEST_BEFORE: &str = "FROM requests
     WHERE session_id = ?1 AND ts < ?2 AND is_sidechain = 0 ORDER BY ts DESC, key DESC LIMIT 1";

const ADJUSTMENTS_BETWEEN: &str =
    "SELECT session_id, ts, model, input, output, cache_read, cache_write, cost_usd
     FROM adjustments WHERE ts >= ?1 AND ts < ?2 ORDER BY session_id, ts, model";

const SESSIONS_SPENDING_BETWEEN: &str = "SELECT id, project, cwd, title FROM sessions WHERE id IN (
         SELECT session_id FROM requests WHERE ts >= ?1 AND ts < ?2
         UNION SELECT session_id FROM adjustments WHERE ts >= ?1 AND ts < ?2)
     ORDER BY id";

const LIMIT_SAMPLES_BETWEEN: &str =
    "SELECT ts_ms, session_id, model, window, used_pct, resets_at_ms
     FROM limit_samples WHERE window = ?1 AND ts_ms >= ?2 AND ts_ms < ?3 ORDER BY ts_ms, id";

fn prefix_changes_sql() -> String {
    let kinds = vec!["?"; PREFIX_KINDS.len()].join(", ");
    format!(
        "SELECT ts FROM attachments
         WHERE session_id = ? AND ts >= ? AND ts < ? AND kind IN ({kinds}) ORDER BY ts"
    )
}

fn compactions_sql() -> String {
    format!(
        "SELECT c.ts, {MAIN_THREAD} FROM compactions c JOIN files f ON f.id = c.file_id
         WHERE c.session_id = ?1 AND c.ts >= ?2 AND c.ts < ?3 ORDER BY c.ts, c.line_no"
    )
}

#[cfg(test)]
fn history_statements() -> Vec<String> {
    vec![
        format!("SELECT {POINT_COLUMNS} {REQUESTS_BETWEEN}"),
        format!("SELECT {POINT_COLUMNS} {LAST_MAIN_REQUEST_BEFORE}"),
        ADJUSTMENTS_BETWEEN.to_string(),
        SESSIONS_SPENDING_BETWEEN.to_string(),
        LIMIT_SAMPLES_BETWEEN.to_string(),
        prefix_changes_sql(),
        compactions_sql(),
    ]
}

impl Store {
    pub fn requests_between(&self, since_ts: i64, until_ts: i64) -> Result<Vec<SpentRequest>> {
        let mut statement = self.conn.prepare_cached(&format!(
            "SELECT {POINT_COLUMNS}, session_id, cost_usd {REQUESTS_BETWEEN}"
        ))?;
        let rows = statement.query_map([since_ts, until_ts], |r| {
            Ok(SpentRequest {
                point: point_of(r)?,
                session_id: r.get(12)?,
                cost_usd: r.get(13)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn last_main_request_before(
        &self,
        session_id: &str,
        before_ts: i64,
    ) -> Result<Option<RequestPoint>> {
        Ok(self
            .conn
            .prepare_cached(&format!(
                "SELECT {POINT_COLUMNS} {LAST_MAIN_REQUEST_BEFORE}"
            ))?
            .query_row(params![session_id, before_ts], point_of)
            .optional()?)
    }

    pub fn adjustments_between(
        &self,
        since_ts: i64,
        until_ts: i64,
    ) -> Result<Vec<SpentAdjustment>> {
        let mut statement = self.conn.prepare_cached(ADJUSTMENTS_BETWEEN)?;
        let rows = statement.query_map([since_ts, until_ts], |r| {
            let count = |index| -> rusqlite::Result<u64> { Ok(r.get::<_, i64>(index)? as u64) };
            Ok(SpentAdjustment {
                session_id: r.get(0)?,
                ts: r.get(1)?,
                model: r.get(2)?,
                tokens: TokenCounts {
                    input: count(3)?,
                    output: count(4)?,
                    cache_read: count(5)?,
                    cache_write_5m: count(6)?,
                    ..Default::default()
                },
                cost_usd: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn sessions_spending_between(
        &self,
        since_ts: i64,
        until_ts: i64,
    ) -> Result<Vec<SessionInfo>> {
        let mut statement = self.conn.prepare_cached(SESSIONS_SPENDING_BETWEEN)?;
        let rows = statement.query_map([since_ts, until_ts], |r| {
            Ok(SessionInfo {
                id: r.get(0)?,
                project: r.get(1)?,
                cwd: r.get(2)?,
                title: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn limit_samples_between(
        &self,
        window: &str,
        since_ms: i64,
        until_ms: i64,
    ) -> Result<Vec<LimitSample>> {
        let mut statement = self.conn.prepare_cached(LIMIT_SAMPLES_BETWEEN)?;
        let rows = statement.query_map(params![window, since_ms, until_ms], |r| {
            Ok(LimitSample {
                ts_ms: r.get(0)?,
                session_id: r.get(1)?,
                model: r.get(2)?,
                window: r.get(3)?,
                used_pct: r.get(4)?,
                resets_at_ms: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn prefix_changes_between(
        &self,
        session_id: &str,
        since_ts: i64,
        until_ts: i64,
    ) -> Result<Vec<i64>> {
        let mut statement = self.conn.prepare_cached(&prefix_changes_sql())?;
        let mut values: Vec<Value> = vec![
            session_id.to_string().into(),
            since_ts.into(),
            until_ts.into(),
        ];
        values.extend(
            PREFIX_KINDS
                .iter()
                .map(|kind| Value::from(kind.to_string())),
        );
        let rows = statement.query_map(rusqlite::params_from_iter(values), |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn compactions_between(
        &self,
        session_id: &str,
        since_ts: i64,
        until_ts: i64,
    ) -> Result<Vec<CompactionPoint>> {
        let mut statement = self.conn.prepare_cached(&compactions_sql())?;
        let rows = statement.query_map(params![session_id, since_ts, until_ts], |r| {
            Ok(CompactionPoint {
                session_id: session_id.to_string(),
                ts: r.get(0)?,
                main: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{AttachmentRow, CompactionRow, RequestRow, SessionRow};

    fn plan(store: &Store, sql: &str) -> String {
        let mut statement = store
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap();
        let nulls = vec![rusqlite::types::Null; statement.parameter_count()];
        let details: Vec<String> = statement
            .query_map(rusqlite::params_from_iter(nulls), |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        details.join("; ")
    }

    #[test]
    fn the_history_reads_search_indexes_instead_of_scanning_tables() {
        let store = Store::open_in_memory().unwrap();
        for sql in history_statements() {
            let plan = plan(&store, &sql);
            assert!(!plan.contains("SCAN"), "{sql}\n=> {plan}");
        }
    }

    #[test]
    fn limit_samples_in_a_span_are_read_for_one_window() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        for (ts_ms, window, used_pct) in [
            (100, "five_hour", 10.0),
            (200, "seven_day", 3.0),
            (300, "five_hour", 20.0),
            (400, "five_hour", 30.0),
        ] {
            w.limit_sample(&LimitSample {
                ts_ms,
                session_id: "s1".into(),
                model: None,
                window: window.into(),
                used_pct,
                resets_at_ms: None,
            })
            .unwrap();
        }
        w.commit().unwrap();
        let read: Vec<(i64, f64)> = store
            .limit_samples_between("five_hour", 100, 400)
            .unwrap()
            .iter()
            .map(|sample| (sample.ts_ms, sample.used_pct))
            .collect();
        assert_eq!(read, [(100, 10.0), (300, 20.0)]);
    }

    fn request(
        key: &str,
        session: &str,
        ts: i64,
        agent: Option<&str>,
        cost: Option<f64>,
    ) -> RequestRow {
        RequestRow {
            key: key.into(),
            session_id: session.into(),
            ts,
            sent_ts: None,
            model: "claude-opus-5-5".into(),
            tokens: TokenCounts {
                input: 3,
                output: 40,
                thinking: 5,
                cache_read: 1000,
                cache_write_5m: 0,
                cache_write_1h: 200,
            },
            cost_usd: cost,
            is_sidechain: agent.is_some(),
            agent_id: agent.map(String::from),
            skill: None,
            mcp_server: None,
            effort: None,
        }
    }

    fn filled() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        for (id, cwd) in [
            ("s1", Some("/work/app")),
            ("s2", None),
            ("s3", Some("/work/old")),
        ] {
            w.session(&SessionRow {
                id: id.into(),
                project: format!("-p-{id}"),
                cwd: cwd.map(String::from),
                ts: Some(10),
            })
            .unwrap();
        }
        w.title("s1", "Fix the cache countdown", false).unwrap();
        for row in [
            request("r0", "s1", 50, None, Some(0.5)),
            request("r1", "s1", 100, None, Some(1.0)),
            request("r2", "s1", 150, Some("a1"), None),
            request("r3", "s2", 120, None, Some(2.0)),
            request("r4", "s1", 300, None, Some(4.0)),
            request("r5", "s3", 20, None, Some(8.0)),
        ] {
            w.request(&row).unwrap();
        }
        for (session, ts, cost_usd) in [("s2", 130, 0.25), ("s3", 30, 0.1)] {
            w.adjustment(&SpentAdjustment {
                session_id: session.into(),
                ts,
                model: "claude-opus-5-5".into(),
                tokens: TokenCounts {
                    input: 1,
                    output: 2,
                    cache_read: 3,
                    cache_write_5m: 4,
                    ..Default::default()
                },
                cost_usd: Some(cost_usd),
            })
            .unwrap();
        }
        let main = w.file("/p/-p-s1/s1.jsonl", 0, 0).unwrap();
        let agent = w
            .file("/p/-p-s1/s1/subagents/agent-a1.jsonl", 0, 0)
            .unwrap();
        for (file_id, line_no, ts, kind) in [
            (main, 1, 110, "skill_listing"),
            (main, 2, 115, "date"),
            (main, 3, 40, "instructions"),
        ] {
            w.attachment(&AttachmentRow {
                file_id,
                line_no,
                session_id: "s1".into(),
                ts: Some(ts),
                kind: kind.into(),
                chars: 10,
                names: vec![],
            })
            .unwrap();
        }
        for (file_id, ts) in [(main, 140), (agent, 145)] {
            w.compaction(&CompactionRow {
                file_id,
                line_no: 9,
                session_id: "s1".into(),
                ts: Some(ts),
                trigger: None,
                pre_tokens: None,
                post_tokens: None,
            })
            .unwrap();
        }
        w.commit().unwrap();
        store
    }

    #[test]
    fn requests_and_adjustments_in_a_span_come_with_their_sessions() {
        let store = filled();
        let requests = store.requests_between(100, 300).unwrap();
        let keys: Vec<(&str, i64, Option<f64>)> = requests
            .iter()
            .map(|r| (r.session_id.as_str(), r.point.ts, r.cost_usd))
            .collect();
        assert_eq!(
            keys,
            [
                ("s1", 100, Some(1.0)),
                ("s1", 150, None),
                ("s2", 120, Some(2.0))
            ]
        );
        assert_eq!(requests[1].point.agent_id.as_deref(), Some("a1"));
        assert_eq!(requests[0].point.tokens.cache_write_1h, 200);
        let adjustments = store.adjustments_between(100, 300).unwrap();
        assert_eq!(adjustments.len(), 1);
        assert_eq!(
            (
                adjustments[0].session_id.as_str(),
                adjustments[0].tokens.cache_write_5m,
                adjustments[0].cost_usd
            ),
            ("s2", 4, Some(0.25))
        );
        let sessions = store.sessions_spending_between(100, 300).unwrap();
        assert_eq!(
            sessions,
            [
                SessionInfo {
                    id: "s1".into(),
                    project: "-p-s1".into(),
                    cwd: Some("/work/app".into()),
                    title: Some("Fix the cache countdown".into())
                },
                SessionInfo {
                    id: "s2".into(),
                    project: "-p-s2".into(),
                    cwd: None,
                    title: None
                },
            ]
        );
        assert_eq!(store.sessions_spending_between(25, 40).unwrap()[0].id, "s3");
    }

    #[test]
    fn the_main_request_before_a_span_skips_subagents() {
        let store = filled();
        assert_eq!(
            store
                .last_main_request_before("s1", 100)
                .unwrap()
                .map(|p| p.ts),
            Some(50)
        );
        assert_eq!(
            store
                .last_main_request_before("s1", 200)
                .unwrap()
                .map(|p| p.ts),
            Some(100)
        );
        assert_eq!(store.last_main_request_before("s1", 50).unwrap(), None);
    }

    #[test]
    fn prefix_changes_and_compactions_in_a_span_tell_the_main_thread_apart() {
        let store = filled();
        assert_eq!(store.prefix_changes_between("s1", 50, 200).unwrap(), [110]);
        assert_eq!(
            store.prefix_changes_between("s2", 0, 200).unwrap(),
            [] as [i64; 0]
        );
        assert_eq!(store.compactions_between("s1", 141, 200).unwrap().len(), 1);
        assert_eq!(
            store.compactions_between("s1", 0, 200).unwrap(),
            [
                CompactionPoint {
                    session_id: "s1".into(),
                    ts: 140,
                    main: true
                },
                CompactionPoint {
                    session_id: "s1".into(),
                    ts: 145,
                    main: false
                },
            ]
        );
    }
}
