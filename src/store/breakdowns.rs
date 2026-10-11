use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use super::{EventRow, Store, Writer};

#[derive(Debug, Clone, PartialEq)]
pub struct StoredBreakdown {
    pub session_id: String,
    pub ts_ms: i64,
    pub received_ms: i64,
    pub json: String,
}

impl Writer<'_> {
    pub fn live_breakdown(&mut self, row: &EventRow) -> Result<bool> {
        let changed = self
            .tx
            .prepare_cached(
                "INSERT INTO live_breakdowns (session_id, ts_ms, received_ms, json) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(session_id) DO UPDATE SET
                     ts_ms = excluded.ts_ms, received_ms = excluded.received_ms, json = excluded.json
                 WHERE excluded.ts_ms >= live_breakdowns.ts_ms",
            )?
            .execute(params![row.session_id, row.ts_ms, row.received_ms, row.json])?;
        Ok(changed > 0)
    }

    pub fn prune_breakdowns(&mut self, before_ms: i64) -> Result<usize> {
        Ok(self
            .tx
            .prepare_cached("DELETE FROM live_breakdowns WHERE ts_ms < ?1")?
            .execute([before_ms])?)
    }
}

impl Store {
    pub fn live_breakdown(&self, session_id: &str) -> Result<Option<StoredBreakdown>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT session_id, ts_ms, received_ms, json FROM live_breakdowns WHERE session_id = ?1",
            )?
            .query_row([session_id], read_breakdown)
            .optional()?)
    }

    pub fn recent_breakdowns(&self, limit: usize) -> Result<Vec<StoredBreakdown>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT session_id, ts_ms, received_ms, json FROM live_breakdowns
             ORDER BY ts_ms DESC, session_id LIMIT ?1",
        )?;
        let rows = statement.query_map([limit as i64], read_breakdown)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

fn read_breakdown(r: &rusqlite::Row) -> rusqlite::Result<StoredBreakdown> {
    Ok(StoredBreakdown {
        session_id: r.get(0)?,
        ts_ms: r.get(1)?,
        received_ms: r.get(2)?,
        json: r.get(3)?,
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    fn usage(session_id: &str, ts_ms: i64, json: &str) -> EventRow {
        EventRow {
            client_id: None,
            session_id: session_id.into(),
            kind: "session.usage".into(),
            ts_ms,
            received_ms: ts_ms + 3,
            json: json.into(),
        }
    }

    #[test]
    fn the_newest_breakdown_of_a_session_replaces_older_ones() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        assert!(w.live_breakdown(&usage("s1", 200, r#"{"n":2}"#)).unwrap());
        assert!(!w.live_breakdown(&usage("s1", 100, r#"{"n":1}"#)).unwrap());
        assert!(w.live_breakdown(&usage("s1", 200, r#"{"n":3}"#)).unwrap());
        assert!(w.live_breakdown(&usage("s2", 150, r#"{"n":4}"#)).unwrap());
        w.commit().unwrap();
        assert_eq!(
            store.live_breakdown("s1").unwrap(),
            Some(StoredBreakdown {
                session_id: "s1".into(),
                ts_ms: 200,
                received_ms: 203,
                json: r#"{"n":3}"#.into(),
            })
        );
        assert_eq!(store.live_breakdown("s3").unwrap(), None);
        let recent: Vec<String> = store
            .recent_breakdowns(5)
            .unwrap()
            .into_iter()
            .map(|b| b.session_id)
            .collect();
        assert_eq!(recent, vec!["s1", "s2"]);
        assert_eq!(store.recent_breakdowns(1).unwrap().len(), 1);
    }

    #[test]
    fn breakdowns_older_than_the_transcripts_are_pruned() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.live_breakdown(&usage("old", 100, "{}")).unwrap();
        w.live_breakdown(&usage("kept", 200, "{}")).unwrap();
        assert_eq!(w.prune_breakdowns(200).unwrap(), 1);
        w.commit().unwrap();
        let left: Vec<String> = store
            .recent_breakdowns(5)
            .unwrap()
            .into_iter()
            .map(|b| b.session_id)
            .collect();
        assert_eq!(left, vec!["kept"]);
    }

    #[test]
    fn breakdowns_are_not_derived_and_survive_a_reindex_clear() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.live_breakdown(&usage("s1", 1, "{}")).unwrap();
        w.clear_derived().unwrap();
        w.commit().unwrap();
        assert_eq!(store.dump("live_breakdowns").len(), 1);
    }
}
