use serde_json::Value;

use super::wire::Rejected;
use crate::store::EventRow;

pub const MAX_EVENTS: usize = 256;
pub const KINDS: &[&str] = &[
    "session.start",
    "session.end",
    "session.measure",
    "session.usage",
    "session.compact",
    "turn.step",
    "turn.complete",
];
pub const SESSION_RULE: &str = "session must be 1 to 128 letters, digits, - or _";
pub const ID_RULE: &str = "id must be 1 to 128 letters, digits, -, _, . or :";

#[derive(Debug, Clone, PartialEq)]
pub struct Validated {
    pub rows: Vec<EventRow>,
    pub rejected: Vec<Rejected>,
}

pub fn validate(body: &Value, received_ms: i64) -> Result<Validated, String> {
    let Some(events) = body.get("events").and_then(Value::as_array) else {
        return Err("body must be a JSON object with an events array".into());
    };
    if events.len() > MAX_EVENTS {
        return Err(format!("at most {MAX_EVENTS} events per request"));
    }
    let mut validated = Validated {
        rows: Vec::new(),
        rejected: Vec::new(),
    };
    for (index, event) in events.iter().enumerate() {
        match row(event, received_ms) {
            Ok(row) => validated.rows.push(row),
            Err(error) => validated.rejected.push(Rejected { index, error }),
        }
    }
    Ok(validated)
}

pub fn is_session_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn is_client_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

fn row(event: &Value, received_ms: i64) -> Result<EventRow, String> {
    if !event.is_object() {
        return Err("event must be a JSON object".into());
    }
    let session = event
        .get("session")
        .and_then(Value::as_str)
        .filter(|id| is_session_id(id))
        .ok_or(SESSION_RULE)?;
    let kind = event.get("kind").and_then(Value::as_str).unwrap_or("");
    if !KINDS.contains(&kind) {
        return Err(format!("unknown kind \"{kind}\""));
    }
    let ts_ms = event
        .get("ts_ms")
        .and_then(Value::as_i64)
        .filter(|ts| *ts >= 0)
        .ok_or("ts_ms must be a non-negative integer of milliseconds")?;
    let data = event
        .get("data")
        .filter(|data| data.is_object())
        .ok_or("data must be a JSON object")?;
    let client_id = match event.get("id") {
        None => None,
        Some(id) => Some(id.as_str().filter(|id| is_client_id(id)).ok_or(ID_RULE)?),
    };
    Ok(EventRow {
        client_id: client_id.map(str::to_string),
        session_id: session.to_string(),
        kind: kind.to_string(),
        ts_ms,
        received_ms,
        json: data.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn measure(session: &str) -> Value {
        json!({"session": session, "kind": "session.measure", "ts_ms": 1_791_021_500_000_i64, "data": {"contextPercent": 41}})
    }

    #[test]
    fn valid_events_become_rows_and_invalid_ones_are_named_by_index() {
        let body = json!({"events": [
            measure("11111111-1111-1111-1111-111111111111"),
            measure("../etc"),
            {"session": "s1", "kind": "turn.exploded", "ts_ms": 1, "data": {}},
            {"session": "s1", "kind": "turn.step", "ts_ms": -1, "data": {}},
            {"session": "s1", "kind": "turn.step", "ts_ms": 1.5, "data": {}},
            {"session": "s1", "kind": "turn.step", "ts_ms": 1, "data": [1]},
            "not an object",
            {"session": "s1", "kind": "turn.complete", "ts_ms": 2, "data": {}, "extra": true}
        ]});
        let validated = validate(&body, 99).unwrap();
        assert_eq!(
            validated.rows,
            vec![
                EventRow {
                    client_id: None,
                    session_id: "11111111-1111-1111-1111-111111111111".into(),
                    kind: "session.measure".into(),
                    ts_ms: 1_791_021_500_000,
                    received_ms: 99,
                    json: r#"{"contextPercent":41}"#.into(),
                },
                EventRow {
                    client_id: None,
                    session_id: "s1".into(),
                    kind: "turn.complete".into(),
                    ts_ms: 2,
                    received_ms: 99,
                    json: "{}".into(),
                }
            ]
        );
        let rejected: Vec<(usize, &str)> = validated
            .rejected
            .iter()
            .map(|r| (r.index, r.error.as_str()))
            .collect();
        assert_eq!(
            rejected,
            vec![
                (1, SESSION_RULE),
                (2, "unknown kind \"turn.exploded\""),
                (3, "ts_ms must be a non-negative integer of milliseconds"),
                (4, "ts_ms must be a non-negative integer of milliseconds"),
                (5, "data must be a JSON object"),
                (6, "event must be a JSON object"),
            ]
        );
    }

    #[test]
    fn an_event_may_carry_a_client_id_to_be_stored_once() {
        let body = json!({"events": [
            {"id": "k3j-x8f.9:12_a", "session": "s1", "kind": "turn.step", "ts_ms": 1, "data": {}},
            {"session": "s1", "kind": "turn.step", "ts_ms": 2, "data": {}},
            {"id": "", "session": "s1", "kind": "turn.step", "ts_ms": 3, "data": {}},
            {"id": 7, "session": "s1", "kind": "turn.step", "ts_ms": 4, "data": {}},
            {"id": "a b", "session": "s1", "kind": "turn.step", "ts_ms": 5, "data": {}},
            {"id": "x".repeat(129), "session": "s1", "kind": "turn.step", "ts_ms": 6, "data": {}}
        ]});
        let validated = validate(&body, 9).unwrap();
        assert_eq!(
            validated
                .rows
                .iter()
                .map(|row| row.client_id.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("k3j-x8f.9:12_a"), None]
        );
        assert_eq!(
            validated
                .rejected
                .iter()
                .map(|r| (r.index, r.error.as_str()))
                .collect::<Vec<_>>(),
            vec![(2, ID_RULE), (3, ID_RULE), (4, ID_RULE), (5, ID_RULE)]
        );
    }

    #[test]
    fn the_batch_itself_must_be_well_formed() {
        assert_eq!(
            validate(&json!([measure("s1")]), 0).unwrap_err(),
            "body must be a JSON object with an events array"
        );
        let too_many = json!({"events": vec![measure("s1"); MAX_EVENTS + 1]});
        assert_eq!(
            validate(&too_many, 0).unwrap_err(),
            "at most 256 events per request"
        );
        let full = json!({"events": vec![measure("s1"); MAX_EVENTS]});
        assert_eq!(validate(&full, 0).unwrap().rows.len(), MAX_EVENTS);
    }

    #[test]
    fn session_ids_are_short_and_plain() {
        assert!(is_session_id("11111111-1111-1111-1111-111111111111"));
        assert!(is_session_id(&"a".repeat(128)));
        for id in ["", "a b", "a/b", "é", &"a".repeat(129)] {
            assert!(!is_session_id(id), "{id:?}");
        }
    }
}
