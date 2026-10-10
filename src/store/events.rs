use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use super::{Store, Writer};

#[derive(Debug, Clone, PartialEq)]
pub struct EventRow {
    pub client_id: Option<String>,
    pub session_id: String,
    pub kind: String,
    pub ts_ms: i64,
    pub received_ms: i64,
    pub json: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredEvent {
    pub ts_ms: i64,
    pub received_ms: i64,
    pub json: String,
}

impl Writer<'_> {
    pub fn event(&mut self, row: &EventRow) -> Result<bool> {
        let inserted = self
            .tx
            .prepare_cached(
                "INSERT INTO events (client_id, session_id, kind, ts_ms, received_ms, json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT DO NOTHING",
            )?
            .execute(params![
                row.client_id,
                row.session_id,
                row.kind,
                row.ts_ms,
                row.received_ms,
                row.json
            ])?;
        Ok(inserted > 0)
    }
}

impl Store {
    pub fn latest_event(&self, session_id: &str, kind: &str) -> Result<Option<StoredEvent>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT ts_ms, received_ms, json FROM events WHERE session_id = ?1 AND kind = ?2
                 ORDER BY ts_ms DESC, id DESC LIMIT 1",
            )?
            .query_row([session_id, kind], |r| {
                Ok(StoredEvent {
                    ts_ms: r.get(0)?,
                    received_ms: r.get(1)?,
                    json: r.get(2)?,
                })
            })
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(session_id: &str, kind: &str, ts_ms: i64, json: &str) -> EventRow {
        EventRow {
            client_id: None,
            session_id: session_id.into(),
            kind: kind.into(),
            ts_ms,
            received_ms: ts_ms + 5,
            json: json.into(),
        }
    }

    #[test]
    fn the_latest_event_of_a_kind_wins_by_timestamp_then_arrival() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.event(&event("s1", "session.measure", 200, r#"{"n":2}"#))
            .unwrap();
        w.event(&event("s1", "session.measure", 100, r#"{"n":1}"#))
            .unwrap();
        w.event(&event("s1", "session.measure", 200, r#"{"n":3}"#))
            .unwrap();
        w.event(&event("s1", "turn.step", 900, r#"{"n":4}"#))
            .unwrap();
        w.event(&event("s2", "session.measure", 900, r#"{"n":5}"#))
            .unwrap();
        w.commit().unwrap();
        assert_eq!(
            store.latest_event("s1", "session.measure").unwrap(),
            Some(StoredEvent {
                ts_ms: 200,
                received_ms: 205,
                json: r#"{"n":3}"#.into()
            })
        );
        assert_eq!(store.latest_event("s3", "session.measure").unwrap(), None);
    }

    #[test]
    fn an_event_sent_again_under_its_client_id_is_stored_once() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let with_id = |session: &str, id: &str| EventRow {
            client_id: Some(id.into()),
            ..event(session, "turn.step", 1, "{}")
        };
        assert!(w.event(&with_id("s1", "a-1")).unwrap());
        assert!(!w.event(&with_id("s1", "a-1")).unwrap());
        assert!(w.event(&with_id("s1", "a-2")).unwrap());
        assert!(w.event(&with_id("s2", "a-1")).unwrap());
        assert!(w.event(&event("s1", "turn.step", 1, "{}")).unwrap());
        assert!(w.event(&event("s1", "turn.step", 1, "{}")).unwrap());
        w.commit().unwrap();
        assert_eq!(store.dump("events").len(), 5);
    }

    #[test]
    fn events_are_not_derived_and_survive_a_reindex_clear() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.event(&event("s1", "session.start", 1, "{}")).unwrap();
        w.clear_derived().unwrap();
        w.commit().unwrap();
        assert_eq!(store.dump("events").len(), 1);
    }
}
