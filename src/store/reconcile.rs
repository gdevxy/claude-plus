use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::{Connection, params};

use super::{Store, Writer};
use crate::pricing::{PriceTable, normalize_model};
use crate::transcript::{ModelUsage, Record, TokenCounts, parse_line};

const OVERAGE_TOLERANCE: f64 = 1.01;
const BUCKET_SECONDS: i64 = 3600;
pub(super) const LATEST_COST_STATE: &str = "NOT EXISTS (SELECT 1 FROM cost_states later
    WHERE later.session_id = c.session_id AND (later.file_id, later.line_no) > (c.file_id, c.line_no))";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reconciliation {
    pub sessions_adjusted: u64,
    pub adjusted_usd: f64,
    pub unpriced_models: Vec<String>,
    pub sessions_over: u64,
    pub sessions_without_counter: u64,
}

impl Store {
    pub fn reconcile(&mut self, prices: &PriceTable) -> Result<Reconciliation> {
        let tx = self.conn.transaction()?;
        let reconciliation = rebuild(&tx, prices)?;
        tx.commit()?;
        Ok(reconciliation)
    }

    pub fn reconciliation_summary(&self) -> Result<Reconciliation> {
        summary(&self.conn)
    }
}

impl Writer<'_> {
    pub fn reconcile(&mut self, prices: &PriceTable) -> Result<Reconciliation> {
        rebuild(&self.tx, prices)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Tokens {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
}

impl Tokens {
    fn from_counter(usage: &ModelUsage) -> Self {
        Self {
            input: usage.input,
            output: usage.output,
            cache_read: usage.cache_read,
            cache_write: usage.cache_write,
        }
    }

    fn fields(self) -> [u64; 4] {
        [self.input, self.output, self.cache_read, self.cache_write]
    }

    fn from_fields([input, output, cache_read, cache_write]: [u64; 4]) -> Self {
        Self {
            input,
            output,
            cache_read,
            cache_write,
        }
    }

    fn add(&mut self, other: Tokens) {
        *self = Self::from_fields(std::array::from_fn(|i| {
            self.fields()[i].saturating_add(other.fields()[i])
        }));
    }

    fn deficit_to(self, theirs: Tokens) -> Tokens {
        Self::from_fields(std::array::from_fn(|i| {
            theirs.fields()[i].saturating_sub(self.fields()[i])
        }))
    }

    fn is_zero(self) -> bool {
        self.fields() == [0; 4]
    }

    fn priced(self, prices: &PriceTable, model: &str) -> Option<f64> {
        prices.base_cost(
            model,
            &TokenCounts {
                input: self.input,
                output: self.output,
                cache_read: self.cache_read,
                cache_write_5m: self.cache_write,
                ..Default::default()
            },
        )
    }
}

struct Counter {
    session_id: String,
    model_usage: BTreeMap<String, ModelUsage>,
}

struct Request {
    model: String,
    ts: i64,
    tokens: Tokens,
}

struct Bucket {
    requests: u64,
    ts: i64,
    model: String,
}

#[derive(Default)]
struct Ours {
    tokens: Tokens,
    buckets: BTreeMap<i64, Bucket>,
}

struct Theirs {
    tokens: Tokens,
    model: String,
}

#[derive(Debug)]
struct Adjustment {
    model: String,
    ts: i64,
    tokens: Tokens,
}

fn rebuild(conn: &Connection, prices: &PriceTable) -> Result<Reconciliation> {
    conn.execute("DELETE FROM adjustments", [])?;
    let mut insert = conn.prepare(
        "INSERT INTO adjustments (session_id, model, ts, input, output, cache_read, cache_write, cost_usd)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    for counter in latest_counters(conn)? {
        let requests = session_requests(conn, &counter.session_id)?;
        for adjustment in adjustments(&counter.model_usage, &requests) {
            let t = adjustment.tokens;
            insert.execute(params![
                counter.session_id,
                adjustment.model,
                adjustment.ts,
                t.input as i64,
                t.output as i64,
                t.cache_read as i64,
                t.cache_write as i64,
                t.priced(prices, &adjustment.model)
            ])?;
        }
    }
    summary(conn)
}

fn summary(conn: &Connection) -> Result<Reconciliation> {
    let (sessions_adjusted, adjusted_usd) = conn.query_row(
        "SELECT COUNT(DISTINCT session_id), COALESCE(SUM(cost_usd), 0) FROM adjustments",
        [],
        |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)),
    )?;
    let mut unpriced = conn
        .prepare("SELECT DISTINCT model FROM adjustments WHERE cost_usd IS NULL ORDER BY model")?;
    let unpriced_models = unpriced
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let sessions_over = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM cost_states c
             WHERE {LATEST_COST_STATE} AND c.total_cost_usd > 0
               AND (SELECT COALESCE(SUM(cost_usd), 0) FROM requests r WHERE r.session_id = c.session_id)
                   > c.total_cost_usd * ?1"
        ),
        [OVERAGE_TOLERANCE],
        |r| r.get::<_, i64>(0),
    )? as u64;
    let sessions_without_counter = conn.query_row(
        "SELECT COUNT(DISTINCT session_id) FROM requests r
         WHERE is_sidechain = 1 AND NOT EXISTS (SELECT 1 FROM cost_states c WHERE c.session_id = r.session_id)",
        [],
        |r| r.get::<_, i64>(0),
    )? as u64;
    Ok(Reconciliation {
        sessions_adjusted,
        adjusted_usd,
        unpriced_models,
        sessions_over,
        sessions_without_counter,
    })
}

fn latest_counters(conn: &Connection) -> Result<Vec<Counter>> {
    let mut statement = conn.prepare(&format!(
        "SELECT c.session_id, c.json FROM cost_states c WHERE {LATEST_COST_STATE} ORDER BY c.session_id"
    ))?;
    let rows = statement.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut counters = Vec::new();
    for row in rows {
        let (session_id, json) = row?;
        let model_usage = match parse_line(&json).map(|parsed| parsed.record) {
            Ok(Record::CostState(state)) => state.model_usage,
            _ => BTreeMap::new(),
        };
        counters.push(Counter {
            session_id,
            model_usage,
        });
    }
    Ok(counters)
}

fn session_requests(conn: &Connection, session_id: &str) -> Result<Vec<Request>> {
    let mut statement = conn.prepare_cached(
        "SELECT model, ts, input, output, cache_read, cache_write_5m + cache_write_1h
         FROM requests WHERE session_id = ?1 ORDER BY ts, key",
    )?;
    let rows = statement.query_map([session_id], |r| {
        Ok(Request {
            model: r.get(0)?,
            ts: r.get(1)?,
            tokens: Tokens {
                input: r.get::<_, i64>(2)? as u64,
                output: r.get::<_, i64>(3)? as u64,
                cache_read: r.get::<_, i64>(4)? as u64,
                cache_write: r.get::<_, i64>(5)? as u64,
            },
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn adjustments(
    model_usage: &BTreeMap<String, ModelUsage>,
    requests: &[Request],
) -> Vec<Adjustment> {
    let Some(latest_ts) = requests.iter().map(|r| r.ts).max() else {
        return Vec::new();
    };
    let mut theirs: BTreeMap<&str, Theirs> = BTreeMap::new();
    for (key, usage) in model_usage {
        theirs
            .entry(normalize_model(key))
            .or_insert_with(|| Theirs {
                tokens: Tokens::default(),
                model: key.split('[').next().unwrap_or(key).to_string(),
            })
            .tokens
            .add(Tokens::from_counter(usage));
    }
    let mut ours: BTreeMap<&str, Ours> = BTreeMap::new();
    for request in requests {
        let group = ours.entry(normalize_model(&request.model)).or_default();
        group.tokens.add(request.tokens);
        let bucket = group
            .buckets
            .entry(request.ts.div_euclid(BUCKET_SECONDS))
            .or_insert_with(|| Bucket {
                requests: 0,
                ts: request.ts,
                model: request.model.clone(),
            });
        bucket.requests += 1;
        bucket.ts = request.ts;
        bucket.model = request.model.clone();
    }

    let mut adjustments = Vec::new();
    for (normalized, counter) in &theirs {
        let own = ours.get(normalized);
        let deficit = own
            .map_or_else(Tokens::default, |own| own.tokens)
            .deficit_to(counter.tokens);
        if deficit.is_zero() {
            continue;
        }
        match own {
            Some(own) => adjustments.extend(spread(deficit, &own.buckets)),
            None => adjustments.push(Adjustment {
                model: counter.model.clone(),
                ts: latest_ts,
                tokens: deficit,
            }),
        }
    }
    adjustments
}

fn spread(deficit: Tokens, buckets: &BTreeMap<i64, Bucket>) -> Vec<Adjustment> {
    let total: u64 = buckets.values().map(|b| b.requests).sum();
    let mut remaining = deficit.fields();
    let last = buckets.len() - 1;
    buckets
        .values()
        .enumerate()
        .map(|(index, bucket)| {
            let parts = std::array::from_fn(|field| {
                let part = if index == last {
                    remaining[field]
                } else {
                    (u128::from(deficit.fields()[field]) * u128::from(bucket.requests)
                        / u128::from(total)) as u64
                };
                remaining[field] -= part;
                part
            });
            Adjustment {
                model: bucket.model.clone(),
                ts: bucket.ts,
                tokens: Tokens::from_fields(parts),
            }
        })
        .filter(|adjustment| !adjustment.tokens.is_zero())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{CostStateRow, RequestRow};
    use crate::transcript::TokenCounts;

    const T: i64 = 1_791_018_000;

    struct Counter<'a> {
        model: &'a str,
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
    }

    fn counter(model: &str, input: u64, output: u64, cache_read: u64) -> Counter<'_> {
        Counter {
            model,
            input,
            output,
            cache_read,
            cache_write: 0,
        }
    }

    fn tokens(input: u64, output: u64, cache_read: u64) -> TokenCounts {
        TokenCounts {
            input,
            output,
            cache_read,
            ..Default::default()
        }
    }

    struct Session<'a> {
        store: &'a mut Store,
        id: &'a str,
        prices: PriceTable,
        next_line: u64,
    }

    impl<'a> Session<'a> {
        fn new(store: &'a mut Store, id: &'a str) -> Self {
            Self {
                store,
                id,
                prices: PriceTable::bundled(),
                next_line: 0,
            }
        }

        fn request(&mut self, ts: i64, model: &str, tokens: TokenCounts, sidechain: bool) {
            self.next_line += 1;
            let row = RequestRow {
                key: format!("{}-{}", self.id, self.next_line),
                session_id: self.id.into(),
                ts,
                model: model.into(),
                tokens,
                cost_usd: self.prices.cost(model, &tokens),
                is_sidechain: sidechain,
                agent_id: None,
                skill: None,
                mcp_server: None,
                effort: None,
            };
            let mut w = self.store.writer().unwrap();
            w.request(&row).unwrap();
            w.commit().unwrap();
        }

        fn counter(&mut self, total_cost_usd: f64, counters: &[Counter]) {
            self.next_line += 1;
            let model_usage: serde_json::Map<String, serde_json::Value> = counters
                .iter()
                .map(|c| {
                    (
                        c.model.to_string(),
                        serde_json::json!({
                            "inputTokens": c.input,
                            "outputTokens": c.output,
                            "thinkingTokens": 7,
                            "cacheReadInputTokens": c.cache_read,
                            "cacheCreationInputTokens": c.cache_write,
                            "costUSD": 0.0
                        }),
                    )
                })
                .collect();
            let json = serde_json::json!({
                "type": "cost-state",
                "sessionId": self.id,
                "totalCostUSD": total_cost_usd,
                "modelUsage": model_usage
            });
            let mut w = self.store.writer().unwrap();
            let file = w.file(&format!("/p/proj/{}.jsonl", self.id), 0, 0).unwrap();
            w.cost_state(&CostStateRow {
                file_id: file,
                line_no: self.next_line,
                session_id: self.id.into(),
                total_cost_usd,
                json: json.to_string(),
            })
            .unwrap();
            w.commit().unwrap();
        }
    }

    #[derive(Debug, PartialEq)]
    struct Row {
        session_id: String,
        model: String,
        ts: i64,
        tokens: [u64; 4],
        cost_usd: Option<f64>,
    }

    fn rows(store: &Store) -> Vec<Row> {
        let mut statement = store
            .conn
            .prepare(
                "SELECT session_id, model, ts, input, output, cache_read, cache_write, cost_usd
                 FROM adjustments ORDER BY session_id, model, ts",
            )
            .unwrap();
        statement
            .query_map([], |r| {
                Ok(Row {
                    session_id: r.get(0)?,
                    model: r.get(1)?,
                    ts: r.get(2)?,
                    tokens: [
                        r.get::<_, i64>(3)? as u64,
                        r.get::<_, i64>(4)? as u64,
                        r.get::<_, i64>(5)? as u64,
                        r.get::<_, i64>(6)? as u64,
                    ],
                    cost_usd: r.get(7)?,
                })
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    fn close(actual: f64, expected: f64) -> bool {
        (actual - expected).abs() < 1e-12
    }

    #[test]
    fn fills_output_deficit_from_claude_counter() {
        let mut store = Store::open_in_memory().unwrap();
        let mut s = Session::new(&mut store, "s2");
        s.request(T + 10, "claude-sonnet-5-5", tokens(5, 100, 0), false);
        s.request(T + 30, "claude-sonnet-5-5", tokens(5, 3, 1000), true);
        s.counter(
            0.01225,
            &[
                counter("claude-sonnet-5-5", 10, 1103, 1000),
                counter("claude-haiku-4-5-20251001", 1000, 20, 0),
            ],
        );
        let result = store.reconcile(&PriceTable::bundled()).unwrap();
        assert_eq!(result.sessions_adjusted, 1);
        assert!(
            close(result.adjusted_usd, 0.0111),
            "{}",
            result.adjusted_usd
        );
        assert_eq!(
            (
                result.sessions_over,
                result.sessions_without_counter,
                result.unpriced_models.len()
            ),
            (0, 0, 0)
        );

        let adjustments = rows(&store);
        assert_eq!(adjustments.len(), 2);
        assert_eq!(
            (
                adjustments[0].model.as_str(),
                adjustments[0].ts,
                adjustments[0].tokens
            ),
            ("claude-haiku-4-5-20251001", T + 30, [1000, 20, 0, 0])
        );
        assert!(close(adjustments[0].cost_usd.unwrap(), 0.0011));
        assert_eq!(
            (
                adjustments[1].model.as_str(),
                adjustments[1].ts,
                adjustments[1].tokens
            ),
            ("claude-sonnet-5-5", T + 30, [0, 1000, 0, 0])
        );
        assert!(close(adjustments[1].cost_usd.unwrap(), 0.01));

        let totals = store.totals().unwrap();
        assert!(close(totals.reconciled_usd, 0.0111), "{totals:?}");
        assert!(close(totals.priced_cost_usd, 0.01225), "{totals:?}");

        let daily = store.daily_costs(0, i64::MAX, &chrono::Utc).unwrap();
        assert_eq!(daily.len(), 2);
        assert_eq!(
            (daily[0].model.as_str(), daily[0].requests),
            ("claude-haiku-4-5-20251001", 0)
        );
        assert!(close(daily[0].cost_usd, 0.0011));
        assert!(close(daily[0].reconciled_usd, 0.0011));
        assert_eq!(daily[1].requests, 2);
        assert!(close(daily[1].cost_usd, 0.01115), "{}", daily[1].cost_usd);
        assert!(close(daily[1].reconciled_usd, 0.01));

        let deltas = store.cost_state_deltas().unwrap();
        assert!(close(deltas[0].ours_usd, 0.00115), "{deltas:?}");
        assert!(close(deltas[0].reconciled_usd, 0.0111), "{deltas:?}");
    }

    #[test]
    fn spreads_deficit_by_hour_bucket_request_count() {
        let mut store = Store::open_in_memory().unwrap();
        let mut s = Session::new(&mut store, "s1");
        s.request(T + 100, "claude-sonnet-5-5", tokens(0, 1, 0), false);
        s.request(T + 3700, "claude-sonnet-5-5", tokens(0, 1, 0), false);
        s.request(T + 3800, "claude-sonnet-5-5", tokens(0, 1, 0), false);
        s.counter(1.0, &[counter("claude-sonnet-5-5", 0, 13, 0)]);
        store.reconcile(&PriceTable::bundled()).unwrap();
        let adjustments = rows(&store);
        assert_eq!(
            adjustments
                .iter()
                .map(|row| (row.ts, row.tokens))
                .collect::<Vec<_>>(),
            vec![(T + 100, [0, 3, 0, 0]), (T + 3800, [0, 7, 0, 0])]
        );
    }

    #[test]
    fn merges_context_window_variants() {
        let mut store = Store::open_in_memory().unwrap();
        let mut s = Session::new(&mut store, "s1");
        s.request(T, "claude-opus-5", tokens(1, 10, 0), false);
        s.counter(
            1.0,
            &[
                counter("claude-opus-5", 1, 50, 0),
                counter("claude-opus-5[1m]", 0, 40, 0),
            ],
        );
        store.reconcile(&PriceTable::bundled()).unwrap();
        let adjustments = rows(&store);
        assert_eq!(adjustments.len(), 1);
        assert_eq!(
            (adjustments[0].model.as_str(), adjustments[0].tokens),
            ("claude-opus-5", [0, 80, 0, 0])
        );
    }

    #[test]
    fn prices_missing_cache_writes_at_the_five_minute_rate() {
        let mut store = Store::open_in_memory().unwrap();
        let mut s = Session::new(&mut store, "s1");
        s.request(T, "claude-sonnet-5-5", tokens(1, 1, 0), false);
        s.counter(
            1.0,
            &[Counter {
                cache_write: 1000,
                ..counter("claude-sonnet-5-5", 1, 1, 0)
            }],
        );
        store.reconcile(&PriceTable::bundled()).unwrap();
        let adjustments = rows(&store);
        assert_eq!(adjustments[0].tokens, [0, 0, 0, 1000]);
        assert!(close(adjustments[0].cost_usd.unwrap(), 0.0025));
    }

    #[test]
    fn prices_adjustments_at_the_base_tier() {
        let mut store = Store::open_in_memory().unwrap();
        let mut s = Session::new(&mut store, "s1");
        s.request(T, "claude-haiku-5-5", tokens(1, 1, 0), false);
        s.counter(1.0, &[counter("claude-haiku-5-5", 1, 1, 200_001)]);
        let result = store.reconcile(&PriceTable::bundled()).unwrap();
        let expected = 200_001.0 * 0.01 / 1e6;
        assert!(
            close(result.adjusted_usd, expected),
            "{}",
            result.adjusted_usd
        );
        assert!(close(rows(&store)[0].cost_usd.unwrap(), expected));
    }

    #[test]
    fn summary_reads_back_what_was_reconciled() {
        let mut store = Store::open_in_memory().unwrap();
        let mut over = Session::new(&mut store, "over");
        over.request(T, "claude-sonnet-5-5", tokens(1000, 1000, 0), false);
        over.counter(0.005, &[]);
        let mut short = Session::new(&mut store, "short");
        short.request(T, "claude-sonnet-5-5", tokens(1, 1, 0), false);
        short.counter(
            1.0,
            &[
                counter("claude-sonnet-5-5", 1, 1001, 0),
                counter("claude-mystery-9", 1, 1, 0),
            ],
        );
        Session::new(&mut store, "uncounted").request(
            T,
            "claude-sonnet-5-5",
            tokens(1, 1, 0),
            true,
        );
        assert_eq!(
            store.reconciliation_summary().unwrap(),
            Reconciliation {
                sessions_over: 1,
                sessions_without_counter: 1,
                ..Default::default()
            }
        );
        let reconciled = store.reconcile(&PriceTable::bundled()).unwrap();
        assert_eq!(
            (
                reconciled.sessions_adjusted,
                reconciled.sessions_over,
                reconciled.sessions_without_counter,
                reconciled.unpriced_models.clone()
            ),
            (1, 1, 1, vec!["claude-mystery-9".to_string()])
        );
        assert!(close(reconciled.adjusted_usd, 0.01));
        assert_eq!(store.reconciliation_summary().unwrap(), reconciled);
    }

    #[test]
    fn never_adjusts_downwards_and_counts_overage() {
        let mut store = Store::open_in_memory().unwrap();
        let mut s = Session::new(&mut store, "s1");
        s.request(T, "claude-sonnet-5-5", tokens(1000, 1000, 0), false);
        s.counter(0.005, &[counter("claude-sonnet-5-5", 500, 500, 0)]);
        let result = store.reconcile(&PriceTable::bundled()).unwrap();
        assert_eq!(rows(&store), vec![]);
        assert_eq!((result.sessions_adjusted, result.sessions_over), (0, 1));
        assert_eq!(result.adjusted_usd, 0.0);
    }

    #[test]
    fn overage_within_one_percent_is_tolerated() {
        let mut store = Store::open_in_memory().unwrap();
        let mut s = Session::new(&mut store, "s1");
        s.request(T, "claude-sonnet-5-5", tokens(0, 1000, 0), false);
        s.counter(0.00995, &[]);
        let result = store.reconcile(&PriceTable::bundled()).unwrap();
        assert_eq!(result.sessions_over, 0);
    }

    #[test]
    fn is_idempotent() {
        let mut store = Store::open_in_memory().unwrap();
        let mut s = Session::new(&mut store, "s1");
        s.request(T, "claude-sonnet-5-5", tokens(1, 1, 0), false);
        s.request(T + 7200, "claude-sonnet-5-5", tokens(1, 1, 0), true);
        s.counter(1.0, &[counter("claude-sonnet-5-5", 2, 999, 5)]);
        let prices = PriceTable::bundled();
        let first = store.reconcile(&prices).unwrap();
        let first_rows = rows(&store);
        let second = store.reconcile(&prices).unwrap();
        assert_eq!(first, second);
        assert_eq!(store.reconciliation_summary().unwrap(), first);
        assert_eq!(first_rows, rows(&store));
        assert_eq!(first_rows.len(), 2);
    }

    #[test]
    fn uses_the_latest_counter_and_skips_empty_model_usage() {
        let mut store = Store::open_in_memory().unwrap();
        let mut s = Session::new(&mut store, "s1");
        s.request(T, "claude-sonnet-5-5", tokens(1, 1, 0), false);
        s.counter(1.0, &[counter("claude-sonnet-5-5", 1, 500, 0)]);
        s.counter(1.0, &[]);
        let result = store.reconcile(&PriceTable::bundled()).unwrap();
        assert_eq!(rows(&store), vec![]);
        assert_eq!(result.sessions_adjusted, 0);
    }

    #[test]
    fn session_without_requests_gets_no_rows() {
        let mut store = Store::open_in_memory().unwrap();
        Session::new(&mut store, "s1").counter(1.0, &[counter("claude-sonnet-5-5", 1, 500, 0)]);
        store.reconcile(&PriceTable::bundled()).unwrap();
        assert_eq!(rows(&store), vec![]);
    }

    #[test]
    fn counts_subagent_sessions_without_counter() {
        let mut store = Store::open_in_memory().unwrap();
        let mut uncounted = Session::new(&mut store, "uncounted");
        uncounted.request(T, "claude-sonnet-5-5", tokens(1, 1, 0), false);
        uncounted.request(T + 1, "claude-sonnet-5-5", tokens(1, 1, 0), true);
        uncounted.request(T + 2, "claude-sonnet-5-5", tokens(1, 1, 0), true);
        let mut counted = Session::new(&mut store, "counted");
        counted.request(T, "claude-sonnet-5-5", tokens(1, 1, 0), true);
        counted.counter(1.0, &[]);
        Session::new(&mut store, "plain").request(T, "claude-sonnet-5-5", tokens(1, 1, 0), false);
        let result = store.reconcile(&PriceTable::bundled()).unwrap();
        assert_eq!(result.sessions_without_counter, 1);
    }

    #[test]
    fn unknown_model_counter_stays_unpriced() {
        let mut store = Store::open_in_memory().unwrap();
        let mut s = Session::new(&mut store, "s1");
        s.request(T, "claude-sonnet-5-5", tokens(1, 1, 0), false);
        s.request(T + 50, "claude-sonnet-5-5", tokens(1, 1, 0), false);
        s.counter(
            1.0,
            &[
                counter("claude-sonnet-5-5", 2, 2, 0),
                counter("claude-mystery-9", 10, 10, 0),
            ],
        );
        let result = store.reconcile(&PriceTable::bundled()).unwrap();
        assert_eq!(result.unpriced_models, vec!["claude-mystery-9".to_string()]);
        assert_eq!(result.sessions_adjusted, 1);
        assert_eq!(
            rows(&store),
            vec![Row {
                session_id: "s1".into(),
                model: "claude-mystery-9".into(),
                ts: T + 50,
                tokens: [10, 10, 0, 0],
                cost_usd: None,
            }]
        );
        assert_eq!(store.totals().unwrap().reconciled_usd, 0.0);
    }
}
