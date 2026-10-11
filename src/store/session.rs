use anyhow::Result;

use super::Store;
use crate::transcript::TokenCounts;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionTotals {
    pub known: bool,
    pub cost_usd: f64,
    pub reconciled_usd: f64,
    pub unpriced_requests: u64,
    pub requests: u64,
    pub subagents: u64,
    pub last_request_ts: Option<i64>,
    pub tokens: TokenCounts,
    pub models: Vec<ModelTotal>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelTotal {
    pub model: String,
    pub requests: u64,
    pub cost_usd: f64,
    pub unpriced_requests: u64,
}

impl Store {
    pub fn session_totals(&self, session_id: &str) -> Result<SessionTotals> {
        let mut totals = SessionTotals {
            known: self
                .conn
                .prepare_cached("SELECT 1 FROM sessions WHERE id = ?1")?
                .exists([session_id])?,
            ..Default::default()
        };
        let mut requests = self.conn.prepare_cached(
            "SELECT model, COUNT(*), COALESCE(SUM(cost_usd), 0), SUM(cost_usd IS NULL), SUM(input),
                    SUM(output), SUM(thinking), SUM(cache_read), SUM(cache_write_5m),
                    SUM(cache_write_1h), MAX(ts)
             FROM requests WHERE session_id = ?1 GROUP BY model",
        )?;
        let mut rows = requests.query([session_id])?;
        while let Some(r) = rows.next()? {
            let count = |index| -> rusqlite::Result<u64> { Ok(r.get::<_, i64>(index)? as u64) };
            let model = ModelTotal {
                model: r.get(0)?,
                requests: count(1)?,
                cost_usd: r.get(2)?,
                unpriced_requests: count(3)?,
            };
            totals.known = true;
            totals.requests += model.requests;
            totals.cost_usd += model.cost_usd;
            totals.unpriced_requests += model.unpriced_requests;
            add(
                &mut totals.tokens,
                TokenCounts {
                    input: count(4)?,
                    output: count(5)?,
                    thinking: count(6)?,
                    cache_read: count(7)?,
                    cache_write_5m: count(8)?,
                    cache_write_1h: count(9)?,
                },
            );
            totals.last_request_ts = totals.last_request_ts.max(r.get(10)?);
            totals.models.push(model);
        }
        let mut adjustments = self.conn.prepare_cached(
            "SELECT model, COALESCE(SUM(cost_usd), 0), SUM(input), SUM(output), SUM(cache_read),
                    SUM(cache_write)
             FROM adjustments WHERE session_id = ?1 GROUP BY model",
        )?;
        let mut rows = adjustments.query([session_id])?;
        while let Some(r) = rows.next()? {
            let count = |index| -> rusqlite::Result<u64> { Ok(r.get::<_, i64>(index)? as u64) };
            let model: String = r.get(0)?;
            let cost_usd: f64 = r.get(1)?;
            totals.cost_usd += cost_usd;
            totals.reconciled_usd += cost_usd;
            add(
                &mut totals.tokens,
                TokenCounts {
                    input: count(2)?,
                    output: count(3)?,
                    cache_read: count(4)?,
                    cache_write_5m: count(5)?,
                    ..Default::default()
                },
            );
            match totals.models.iter_mut().find(|m| m.model == model) {
                Some(existing) => existing.cost_usd += cost_usd,
                None => totals.models.push(ModelTotal {
                    model,
                    cost_usd,
                    ..Default::default()
                }),
            }
        }
        totals.models.sort_by(|a, b| {
            b.cost_usd
                .total_cmp(&a.cost_usd)
                .then_with(|| a.model.cmp(&b.model))
        });
        totals.subagents = self.conn.query_row(
            "SELECT COUNT(DISTINCT agent_id) FROM requests
             WHERE session_id = ?1 AND is_sidechain = 1 AND agent_id IS NOT NULL",
            [session_id],
            |r| r.get::<_, i64>(0),
        )? as u64;
        Ok(totals)
    }
}

fn add(total: &mut TokenCounts, more: TokenCounts) {
    total.input += more.input;
    total.output += more.output;
    total.thinking += more.thinking;
    total.cache_read += more.cache_read;
    total.cache_write_5m += more.cache_write_5m;
    total.cache_write_1h += more.cache_write_1h;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pricing::PriceTable;
    use crate::store::{CostStateRow, RequestRow, SessionRow};

    fn request(key: &str, ts: i64, model: &str, agent: Option<&str>) -> RequestRow {
        let tokens = TokenCounts {
            input: 10,
            output: 100,
            thinking: 40,
            cache_read: 1000,
            cache_write_5m: 200,
            cache_write_1h: 300,
        };
        RequestRow {
            key: key.into(),
            session_id: "s1".into(),
            ts,
            sent_ts: None,
            model: model.into(),
            tokens,
            cost_usd: PriceTable::bundled().cost(model, &tokens),
            is_sidechain: agent.is_some(),
            agent_id: agent.map(String::from),
            skill: None,
            mcp_server: None,
            effort: None,
        }
    }

    #[test]
    fn totals_add_reconciled_usage_per_model_and_count_subagents() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let file = w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        w.session(&SessionRow {
            id: "s1".into(),
            project: "proj".into(),
            cwd: None,
            ts: Some(100),
        })
        .unwrap();
        w.request(&request("a", 100, "claude-sonnet-5-5", None))
            .unwrap();
        w.request(&request("b", 160, "claude-sonnet-5-5", Some("a1")))
            .unwrap();
        w.request(&request("c", 130, "claude-sonnet-5-5", Some("a1")))
            .unwrap();
        w.request(&request("d", 140, "claude-mystery-9", Some("a2")))
            .unwrap();
        w.request(&RequestRow {
            session_id: "other".into(),
            ..request("e", 999, "claude-sonnet-5-5", None)
        })
        .unwrap();
        w.cost_state(&CostStateRow {
            file_id: file,
            line_no: 9,
            session_id: "s1".into(),
            total_cost_usd: Some(1.0),
            model_count: 2,
            json: r#"{"type":"cost-state","sessionId":"s1","modelUsage":{"claude-sonnet-5-5":{"inputTokens":30,"outputTokens":1300,"cacheReadInputTokens":3000,"cacheCreationInputTokens":1500},"claude-haiku-4-5-20251001":{"inputTokens":1000,"outputTokens":20}}}"#.into(),
        })
        .unwrap();
        w.commit().unwrap();
        store.reconcile(&PriceTable::bundled()).unwrap();

        let totals = store.session_totals("s1").unwrap();
        let sonnet_requests =
            3.0 * (10.0 * 2.0 + 100.0 * 10.0 + 1000.0 * 0.1 + 200.0 * 2.5 + 300.0 * 4.0) / 1e6;
        let sonnet_reconciled = 1000.0 * 10.0 / 1e6;
        let haiku_reconciled = (1000.0 * 1.0 + 20.0 * 5.0) / 1e6;
        assert!(totals.known);
        assert_eq!(
            (totals.requests, totals.unpriced_requests, totals.subagents),
            (4, 1, 2)
        );
        assert_eq!(totals.last_request_ts, Some(160));
        assert!((totals.reconciled_usd - (sonnet_reconciled + haiku_reconciled)).abs() < 1e-12);
        assert!(
            (totals.cost_usd - (sonnet_requests + sonnet_reconciled + haiku_reconciled)).abs()
                < 1e-12,
            "{totals:?}"
        );
        assert_eq!(
            totals.tokens,
            TokenCounts {
                input: 40 + 1000,
                output: 400 + 1000 + 20,
                thinking: 160,
                cache_read: 4000,
                cache_write_5m: 800,
                cache_write_1h: 1200,
            }
        );
        let models: Vec<(&str, u64, u64)> = totals
            .models
            .iter()
            .map(|m| (m.model.as_str(), m.requests, m.unpriced_requests))
            .collect();
        assert_eq!(
            models,
            vec![
                ("claude-sonnet-5-5", 3, 0),
                ("claude-haiku-4-5-20251001", 0, 0),
                ("claude-mystery-9", 1, 1)
            ]
        );
    }

    #[test]
    fn an_unknown_session_has_empty_totals() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(
            store.session_totals("nobody").unwrap(),
            SessionTotals::default()
        );
    }

    #[test]
    fn a_session_seen_without_requests_is_known() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.session(&SessionRow {
            id: "s1".into(),
            project: "proj".into(),
            cwd: None,
            ts: None,
        })
        .unwrap();
        w.commit().unwrap();
        let totals = store.session_totals("s1").unwrap();
        assert!(totals.known);
        assert_eq!(totals.requests, 0);
    }
}
