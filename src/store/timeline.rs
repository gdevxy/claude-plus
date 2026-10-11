use anyhow::Result;

use super::{Store, StoredEvent};
use crate::transcript::TokenCounts;

pub const PREFIX_KINDS: &[&str] = &[
    "skill_listing",
    "instructions",
    "nested_memory",
    "mcp_instructions_delta",
    "deferred_tools_record",
    "deferred_tools_delta",
    "agent_listing_delta",
];

#[derive(Debug, Clone, PartialEq)]
pub struct RequestPoint {
    pub ts: i64,
    pub sent: Option<i64>,
    pub model: String,
    pub tokens: TokenCounts,
    pub is_sidechain: bool,
    pub agent_id: Option<String>,
    pub effort: Option<String>,
}

impl Store {
    pub fn session_requests(&self, session_id: &str) -> Result<Vec<RequestPoint>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT ts, model, input, output, thinking, cache_read, cache_write_5m, cache_write_1h,
                    is_sidechain, agent_id, effort, sent_ts
             FROM requests WHERE session_id = ?1 ORDER BY ts, key",
        )?;
        let rows = statement.query_map([session_id], |r| {
            let count = |index| -> rusqlite::Result<u64> { Ok(r.get::<_, i64>(index)? as u64) };
            Ok(RequestPoint {
                ts: r.get(0)?,
                sent: r.get(11)?,
                model: r.get(1)?,
                tokens: TokenCounts {
                    input: count(2)?,
                    output: count(3)?,
                    thinking: count(4)?,
                    cache_read: count(5)?,
                    cache_write_5m: count(6)?,
                    cache_write_1h: count(7)?,
                },
                is_sidechain: r.get(8)?,
                agent_id: r.get(9)?,
                effort: r.get(10)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn events_since(
        &self,
        session_id: &str,
        kind: &str,
        since_ms: i64,
    ) -> Result<Vec<StoredEvent>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT ts_ms, received_ms, json FROM events
             WHERE session_id = ?1 AND kind = ?2 AND ts_ms >= ?3 ORDER BY ts_ms, id",
        )?;
        let rows = statement.query_map(rusqlite::params![session_id, kind, since_ms], |r| {
            Ok(StoredEvent {
                ts_ms: r.get(0)?,
                received_ms: r.get(1)?,
                json: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn prefix_changes(&self, session_id: &str) -> Result<Vec<i64>> {
        let placeholders = vec!["?"; PREFIX_KINDS.len()].join(", ");
        let sql = format!(
            "SELECT ts FROM attachments WHERE session_id = ? AND ts IS NOT NULL AND kind IN ({placeholders}) ORDER BY ts"
        );
        let mut statement = self.conn.prepare_cached(&sql)?;
        let params = std::iter::once(session_id).chain(PREFIX_KINDS.iter().copied());
        let rows = statement.query_map(rusqlite::params_from_iter(params), |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn compaction_times(&self, session_id: &str) -> Result<Vec<i64>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT ts FROM compactions WHERE session_id = ?1 AND ts IS NOT NULL ORDER BY ts",
        )?;
        let rows = statement.query_map([session_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{AttachmentRow, CompactionRow, EventRow, RequestRow};

    fn request(key: &str, session: &str, ts: i64, agent: Option<&str>) -> RequestRow {
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
            cost_usd: Some(0.01),
            is_sidechain: agent.is_some(),
            agent_id: agent.map(String::from),
            skill: None,
            mcp_server: None,
            effort: Some("high".into()),
        }
    }

    #[test]
    fn session_requests_come_back_in_time_order_with_their_thread() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.request(&request("b", "s1", 200, Some("a1"))).unwrap();
        w.request(&request("a", "s1", 100, None)).unwrap();
        w.request(&request("c", "s2", 150, None)).unwrap();
        w.commit().unwrap();
        let points = store.session_requests("s1").unwrap();
        assert_eq!(
            points
                .iter()
                .map(|p| (p.ts, p.agent_id.as_deref(), p.is_sidechain))
                .collect::<Vec<_>>(),
            vec![(100, None, false), (200, Some("a1"), true)]
        );
        assert_eq!(points[0].tokens.cache_write_1h, 200);
        assert_eq!(points[0].effort.as_deref(), Some("high"));
    }

    #[test]
    fn events_since_keeps_one_kind_from_a_time_on() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        for (kind, ts_ms) in [
            ("turn.step", 10),
            ("turn.step", 30),
            ("session.measure", 40),
            ("turn.step", 20),
        ] {
            w.event(&EventRow {
                client_id: None,
                session_id: "s1".into(),
                kind: kind.into(),
                ts_ms,
                received_ms: ts_ms,
                json: format!(r#"{{"n":{ts_ms}}}"#),
            })
            .unwrap();
        }
        w.commit().unwrap();
        let since: Vec<i64> = store
            .events_since("s1", "turn.step", 20)
            .unwrap()
            .iter()
            .map(|e| e.ts_ms)
            .collect();
        assert_eq!(since, vec![20, 30]);
    }

    #[test]
    fn prefix_changes_and_compactions_are_listed_by_time() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let file = w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        for (line_no, kind, ts) in [
            (1, "deferred_tools_delta", Some(300)),
            (2, "environment", Some(100)),
            (3, "instructions", Some(50)),
            (4, "skill_listing", None),
        ] {
            w.attachment(&AttachmentRow {
                file_id: file,
                line_no,
                session_id: "s1".into(),
                ts,
                kind: kind.into(),
                chars: 10,
                names: vec![],
            })
            .unwrap();
        }
        w.compaction(&CompactionRow {
            file_id: file,
            line_no: 5,
            session_id: "s1".into(),
            ts: Some(400),
            trigger: Some("auto".into()),
            pre_tokens: Some(9),
            post_tokens: Some(3),
        })
        .unwrap();
        w.commit().unwrap();
        assert_eq!(store.prefix_changes("s1").unwrap(), vec![50, 300]);
        assert_eq!(store.compaction_times("s1").unwrap(), vec![400]);
        assert_eq!(store.prefix_changes("s2").unwrap(), Vec::<i64>::new());
    }
}
