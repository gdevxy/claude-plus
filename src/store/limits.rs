use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use super::{Store, Writer};

pub const RESETS_AT_TOLERANCE_MS: i64 = 60_000;
const SPEND_SINCE: &str = "SELECT ts, session_id, model, cost_usd FROM requests
     WHERE ts >= (?1 + 999) / 1000 AND cost_usd IS NOT NULL ORDER BY ts";

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

impl LimitSample {
    pub fn repeats(&self, other: &LimitSample) -> bool {
        self.window == other.window
            && self.account_id == other.account_id
            && self.used_pct == other.used_pct
            && match (self.resets_at_ms, other.resets_at_ms) {
                (Some(a), Some(b)) => (a - b).abs() <= RESETS_AT_TOLERANCE_MS,
                (None, None) => true,
                _ => false,
            }
    }
}

impl Writer<'_> {
    pub fn limit_sample(&mut self, sample: &LimitSample) -> Result<bool> {
        let previous = self
            .tx
            .prepare_cached(
                "SELECT ts_ms, session_id, account_id, model, window, used_pct, resets_at_ms FROM limit_samples
                 WHERE window = ?1 AND account_id IS ?3 AND ts_ms <= ?2 ORDER BY ts_ms DESC, id DESC LIMIT 1",
            )?
            .query_row(params![sample.window, sample.ts_ms, sample.account_id], read_sample)
            .optional()?;
        if previous.is_some_and(|previous| previous.repeats(sample)) {
            return Ok(false);
        }
        self.tx
            .prepare_cached(
                "INSERT INTO limit_samples (ts_ms, session_id, account_id, model, window, used_pct, resets_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?
            .execute(params![
                sample.ts_ms,
                sample.session_id,
                sample.account_id,
                sample.model,
                sample.window,
                sample.used_pct,
                sample.resets_at_ms
            ])?;
        Ok(true)
    }

    pub fn set_meta(&mut self, key: &str, value: &str) -> Result<()> {
        self.tx
            .prepare_cached("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)")?
            .execute([key, value])?;
        Ok(())
    }
}

impl Store {
    pub fn limit_samples_since(&self, since_ms: i64) -> Result<Vec<LimitSample>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT ts_ms, session_id, account_id, model, window, used_pct, resets_at_ms FROM limit_samples
             WHERE ts_ms >= ?1 ORDER BY account_id, window, ts_ms, id",
        )?;
        let rows = statement.query_map([since_ms], read_sample)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn spend_since(&self, since_ms: i64) -> Result<Vec<SpendPoint>> {
        let mut statement = self.conn.prepare_cached(SPEND_SINCE)?;
        let rows = statement.query_map([since_ms], |r| {
            Ok(SpendPoint {
                ts_ms: r.get::<_, i64>(0)? * 1000,
                session_id: r.get(1)?,
                model: r.get(2)?,
                cost_usd: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .prepare_cached("SELECT value FROM meta WHERE key = ?1")?
            .query_row([key], |r| r.get(0))
            .optional()?)
    }
}

fn read_sample(r: &rusqlite::Row) -> rusqlite::Result<LimitSample> {
    Ok(LimitSample {
        ts_ms: r.get(0)?,
        session_id: r.get(1)?,
        account_id: r.get(2)?,
        model: r.get(3)?,
        window: r.get(4)?,
        used_pct: r.get(5)?,
        resets_at_ms: r.get(6)?,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::RequestRow;
    use crate::transcript::TokenCounts;

    fn sample(ts_ms: i64, window: &str, used_pct: f64, resets_at_ms: Option<i64>) -> LimitSample {
        LimitSample {
            ts_ms,
            session_id: "s1".into(),
            account_id: None,
            model: Some("claude-opus-5-5".into()),
            window: window.into(),
            used_pct,
            resets_at_ms,
        }
    }

    #[test]
    fn a_sample_is_kept_only_when_its_window_moved() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        assert!(
            w.limit_sample(&sample(1_000, "five_hour", 10.0, Some(9_000_000)))
                .unwrap()
        );
        assert!(
            !w.limit_sample(&sample(2_000, "five_hour", 10.0, Some(9_030_000)))
                .unwrap()
        );
        assert!(
            w.limit_sample(&sample(3_000, "seven_day", 10.0, Some(9_000_000)))
                .unwrap()
        );
        assert!(
            w.limit_sample(&sample(4_000, "five_hour", 11.0, Some(9_000_000)))
                .unwrap()
        );
        assert!(
            w.limit_sample(&sample(5_000, "five_hour", 11.0, Some(27_000_000)))
                .unwrap()
        );
        assert!(
            w.limit_sample(&sample(6_000, "spend_limit", 3.5, None))
                .unwrap()
        );
        assert!(
            !w.limit_sample(&sample(7_000, "spend_limit", 3.5, None))
                .unwrap()
        );
        w.commit().unwrap();
        let kept: Vec<(i64, String)> = store
            .limit_samples_since(2_000)
            .unwrap()
            .into_iter()
            .map(|s| (s.ts_ms, s.window))
            .collect();
        assert_eq!(
            kept,
            vec![
                (4_000, "five_hour".into()),
                (5_000, "five_hour".into()),
                (3_000, "seven_day".into()),
                (6_000, "spend_limit".into())
            ]
        );
    }

    #[test]
    fn the_same_reading_from_another_account_is_kept() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let mine = sample(1_000, "five_hour", 10.0, Some(9_000_000));
        let theirs = LimitSample {
            account_id: Some("oauth:b".into()),
            ts_ms: 2_000,
            ..mine.clone()
        };
        assert!(w.limit_sample(&mine).unwrap());
        assert!(w.limit_sample(&theirs).unwrap());
        w.commit().unwrap();
        assert_eq!(store.limit_samples_since(0).unwrap().len(), 2);
    }

    #[test]
    fn spend_is_read_through_the_time_index() {
        let store = Store::open_in_memory().unwrap();
        let plan: Vec<String> = store
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {SPEND_SINCE}"))
            .unwrap()
            .query_map([0], |r| r.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            plan.iter()
                .any(|step| step.starts_with("SEARCH requests USING INDEX requests_by_ts (ts>?)")),
            "{plan:?}"
        );
    }

    #[test]
    fn spend_lists_priced_requests_of_every_session_from_a_time_on() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        for (key, session, ts, cost) in [
            ("a", "s1", 10, Some(0.5)),
            ("b", "s2", 20, Some(0.25)),
            ("c", "s1", 30, None),
            ("d", "s1", 5, Some(9.0)),
        ] {
            w.request(&RequestRow {
                key: key.into(),
                session_id: session.into(),
                ts,
                sent_ts: None,
                model: "claude-opus-5-5".into(),
                tokens: TokenCounts::default(),
                cost_usd: cost,
                is_sidechain: false,
                agent_id: None,
                skill: None,
                mcp_server: None,
                effort: None,
            })
            .unwrap();
        }
        w.set_meta("limits_fit", "{}").unwrap();
        w.commit().unwrap();
        let spend: Vec<(i64, f64)> = store
            .spend_since(10_000)
            .unwrap()
            .into_iter()
            .map(|p| (p.ts_ms, p.cost_usd))
            .collect();
        assert_eq!(spend, vec![(10_000, 0.5), (20_000, 0.25)]);
        assert_eq!(store.spend_since(10_001).unwrap().len(), 1);
        assert_eq!(store.meta("limits_fit").unwrap().as_deref(), Some("{}"));
        assert_eq!(store.meta("nothing").unwrap(), None);
    }
}
