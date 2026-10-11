pub mod calendar;
pub mod export;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::Result;
use chrono::TimeZone;
use serde::{Deserialize, Serialize};

use crate::accounts::AccountRef;
use crate::cache_model::{self, Causes, Rebuild, RebuildCause};
use crate::forecast::family;
use crate::limits::{self, Fit, PAIRS_NEEDED};
use crate::pricing::{PriceTable, normalize_model};
use crate::scope::Scope;
use crate::store::{
    CompactionPoint, LimitSample, RequestPoint, SessionInfo, SpentAdjustment, SpentRequest, Store,
};
use crate::timeline::{Thread, Timeline};
use crate::transcript::TokenCounts;
use calendar::{Span, Spans};

pub const HISTORY_VERSION: u32 = 1;
pub const TOP: usize = 5;
pub const FIVE_HOUR: &str = "five_hour";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Range {
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
    #[serde(rename = "90d")]
    Quarter,
}

impl Range {
    pub const ALL: [Range; 4] = [Range::Day, Range::Week, Range::Month, Range::Quarter];

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|range| range.as_str() == text)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Range::Day => "24h",
            Range::Week => "7d",
            Range::Month => "30d",
            Range::Quarter => "90d",
        }
    }

    pub fn days(self) -> u64 {
        match self {
            Range::Day => 1,
            Range::Week => 7,
            Range::Month => 30,
            Range::Quarter => 90,
        }
    }

    pub fn bucket(self) -> BucketSize {
        match self {
            Range::Day => BucketSize::Hour,
            _ => BucketSize::Day,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BucketSize {
    Hour,
    Day,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spend {
    pub requests: u64,
    pub unpriced: u64,
    pub cost_usd: f64,
    pub reconciled_usd: f64,
    pub tokens: u64,
    pub pct_5h: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    pub calibrated: bool,
    pub pairs: u32,
    pub needed: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Totals {
    #[serde(flatten)]
    pub spend: Spend,
    pub sessions: u64,
    pub hit_ratio: Option<f64>,
    pub saved_usd: f64,
    pub rebuilds: u64,
    pub rebuild_usd: f64,
    pub idle_rebuilds: u64,
    pub compactions: u64,
    pub peak_5h_pct: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSpend {
    pub model: String,
    #[serde(flatten)]
    pub spend: Spend,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bucket {
    pub start_ms: i64,
    pub label: String,
    #[serde(flatten)]
    pub spend: Spend,
    pub models: Vec<ModelSpend>,
    pub prompt_tokens: u64,
    pub cache_read_tokens: u64,
    pub rebuilds: u64,
    pub rebuild_usd: f64,
    pub compactions: u64,
    pub peak_5h_pct: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSpend {
    pub name: String,
    pub path: Option<String>,
    pub sessions: u64,
    #[serde(flatten)]
    pub spend: Spend,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSpend {
    pub id: String,
    pub title: Option<String>,
    pub project: String,
    pub last_ms: i64,
    #[serde(flatten)]
    pub spend: Spend,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryReport {
    pub v: u32,
    pub range: Range,
    pub bucket: BucketSize,
    pub generated_at_ms: i64,
    pub since_ms: i64,
    pub until_ms: i64,
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub accounts: Vec<AccountRef>,
    pub calibration: Calibration,
    pub totals: Totals,
    pub buckets: Vec<Bucket>,
    pub models: Vec<ModelSpend>,
    pub projects: Vec<ProjectSpend>,
    pub sessions: Vec<SessionSpend>,
}

pub struct Inputs<'a> {
    pub requests: &'a [SpentRequest],
    pub adjustments: &'a [SpentAdjustment],
    pub sessions: &'a [SessionInfo],
    pub rebuilds: &'a [Rebuild],
    pub samples: &'a [LimitSample],
    pub compactions: &'a [CompactionPoint],
    pub fits: &'a [Fit],
    pub prices: &'a PriceTable,
}

pub struct Rates {
    by_family: BTreeMap<String, f64>,
    fallback: Option<f64>,
    pairs: u32,
}

impl Rates {
    pub fn of(fits: &[Fit]) -> Self {
        let five_hour: Vec<&Fit> = fits.iter().filter(|fit| fit.window == FIVE_HOUR).collect();
        let calibrated: Vec<&&Fit> = five_hour
            .iter()
            .filter(|fit| fit.pairs >= PAIRS_NEEDED)
            .collect();
        Self {
            by_family: calibrated
                .iter()
                .map(|fit| (fit.family.clone(), fit.pct_per_usd))
                .collect(),
            fallback: calibrated
                .iter()
                .max_by_key(|fit| fit.pairs)
                .map(|fit| fit.pct_per_usd),
            pairs: five_hour.iter().map(|fit| fit.pairs).max().unwrap_or(0),
        }
    }

    pub fn pct(&self, model: &str, cost_usd: Option<f64>) -> Option<f64> {
        let rate = self
            .by_family
            .get(family(model))
            .copied()
            .or(self.fallback)?;
        Some(cost_usd.unwrap_or(0.0) * rate)
    }

    fn empty(&self) -> Spend {
        Spend {
            requests: 0,
            unpriced: 0,
            cost_usd: 0.0,
            reconciled_usd: 0.0,
            tokens: 0,
            pct_5h: self.fallback.map(|_| 0.0),
        }
    }
}

struct Item {
    cost_usd: Option<f64>,
    tokens: u64,
    pct_5h: Option<f64>,
    is_request: bool,
}

impl Spend {
    fn add(&mut self, item: &Item) {
        if item.is_request {
            self.requests += 1;
            if item.cost_usd.is_none() {
                self.unpriced += 1;
            }
        } else {
            self.reconciled_usd += item.cost_usd.unwrap_or(0.0);
        }
        self.cost_usd += item.cost_usd.unwrap_or(0.0);
        self.tokens += item.tokens;
        if let (Some(total), Some(pct)) = (self.pct_5h.as_mut(), item.pct_5h) {
            *total += pct;
        }
    }
}

pub fn read<Tz: TimeZone>(
    store: &Store,
    prices: &PriceTable,
    range: Range,
    now_ms: i64,
    tz: &Tz,
    scope: &Scope,
) -> Result<HistoryReport>
where
    Tz::Offset: std::fmt::Display,
{
    let spans = calendar::spans(range, now_ms.div_euclid(1000), tz)?;
    let (since, until) = (spans.since(), spans.until);
    let kept = scope.sessions(store)?;
    let requests: Vec<SpentRequest> = store
        .requests_between(since, until)?
        .into_iter()
        .filter(|r| scope.keeps(&kept, &r.session_id))
        .collect();
    let adjustments: Vec<SpentAdjustment> = store
        .adjustments_between(since, until)?
        .into_iter()
        .filter(|a| scope.keeps(&kept, &a.session_id))
        .collect();
    let spending = store.sessions_spending_between(since, until)?;
    let accounts = accounts_in(store, &spending, scope)?;
    let sessions: Vec<SessionInfo> = spending
        .into_iter()
        .filter(|s| scope.keeps(&kept, &s.id))
        .collect();
    let mut previous = HashMap::new();
    for session in sessions_of(&requests) {
        if let Some(point) = store.last_main_request_before(session, since)? {
            previous.insert(session, point);
        }
    }
    let mut causes = BTreeMap::new();
    for session in sessions_of(&requests) {
        let from = previous
            .get(session)
            .map_or(since, |point| point.ts.min(since));
        causes.insert(session, Causes::read(store, session, from, until)?);
    }
    let rebuilds = rebuilds_in(&requests, &previous, &causes, prices, now_ms)
        .into_iter()
        .filter(|rebuild| rebuild.ts_ms >= since * 1000)
        .collect::<Vec<_>>();
    let (samples, fits) = match scope.account() {
        None => (Vec::new(), Vec::new()),
        Some(account) => (
            store
                .limit_samples_between(FIVE_HOUR, since * 1000, until * 1000)?
                .into_iter()
                .filter(|s| s.account_id.as_deref() == Some(account))
                .collect(),
            limits::fits_of(&limits::stored_fit(store)?, Some(account)),
        ),
    };
    let in_range: Vec<CompactionPoint> = causes
        .into_values()
        .flat_map(|causes| causes.compactions)
        .filter(|compaction| compaction.ts >= since)
        .collect();
    let report = build(
        range,
        &spans,
        &Inputs {
            requests: &requests,
            adjustments: &adjustments,
            sessions: &sessions,
            rebuilds: &rebuilds,
            samples: &samples,
            compactions: &in_range,
            fits: &fits,
            prices,
        },
        now_ms,
    );
    Ok(HistoryReport {
        account: scope.account().map(String::from),
        accounts,
        ..report
    })
}

fn accounts_in(store: &Store, spending: &[SessionInfo], scope: &Scope) -> Result<Vec<AccountRef>> {
    let attributed = store.session_accounts()?;
    let mut shown: HashSet<&str> = spending
        .iter()
        .filter_map(|s| attributed.get(&s.id).map(String::as_str))
        .collect();
    shown.extend(scope.account());
    Ok(store
        .accounts()?
        .into_iter()
        .filter(|account| shown.contains(account.id.as_str()))
        .map(AccountRef::from)
        .collect())
}

fn sessions_of(requests: &[SpentRequest]) -> BTreeSet<&str> {
    requests.iter().map(|r| r.session_id.as_str()).collect()
}

fn rebuilds_in(
    requests: &[SpentRequest],
    previous: &HashMap<&str, RequestPoint>,
    causes: &BTreeMap<&str, Causes>,
    prices: &PriceTable,
    now_ms: i64,
) -> Vec<Rebuild> {
    let mut by_session: BTreeMap<&str, Vec<RequestPoint>> = BTreeMap::new();
    for request in requests {
        by_session
            .entry(&request.session_id)
            .or_insert_with(|| {
                previous
                    .get(request.session_id.as_str())
                    .cloned()
                    .into_iter()
                    .collect()
            })
            .push(request.point.clone());
    }
    let none = Causes::default();
    by_session
        .into_iter()
        .flat_map(|(session, points)| {
            let causes = causes.get(session).unwrap_or(&none);
            session_rebuilds(&points, causes, prices, now_ms)
        })
        .collect()
}

pub fn session_rebuilds(
    points: &[RequestPoint],
    causes: &Causes,
    prices: &PriceTable,
    now_ms: i64,
) -> Vec<Rebuild> {
    let timeline = Timeline::build(points, &[]);
    let main = timeline.main();
    let (ttl, _) = cache_model::ttl_of(&Thread::Main, &main, false);
    cache_model::rebuilds(&main, ttl, &causes.context(prices, false, now_ms))
}

struct Ledger<'a> {
    spans: &'a Spans,
    rates: &'a Rates,
    info: HashMap<&'a str, &'a SessionInfo>,
    buckets: Vec<Bucket>,
    by_bucket: Vec<BTreeMap<String, Spend>>,
    totals: Spend,
    models: BTreeMap<String, Spend>,
    projects: BTreeMap<String, (Spend, HashSet<String>)>,
    sessions: BTreeMap<String, (Spend, i64)>,
}

impl<'a> Ledger<'a> {
    fn new(spans: &'a Spans, rates: &'a Rates, sessions: &'a [SessionInfo]) -> Self {
        Self {
            spans,
            rates,
            info: sessions.iter().map(|s| (s.id.as_str(), s)).collect(),
            buckets: spans
                .spans
                .iter()
                .map(|span| empty_bucket(span, rates))
                .collect(),
            by_bucket: vec![BTreeMap::new(); spans.spans.len()],
            totals: rates.empty(),
            models: BTreeMap::new(),
            projects: BTreeMap::new(),
            sessions: BTreeMap::new(),
        }
    }

    fn add(&mut self, session: &str, ts: i64, model: &str, item: &Item) -> Option<usize> {
        let index = self.spans.index_of(ts)?;
        let model = normalize_model(model).to_string();
        let project = self.info.get(session).map_or("", |s| s.project.as_str());
        let rates = self.rates;
        self.buckets[index].spend.add(item);
        self.by_bucket[index]
            .entry(model.clone())
            .or_insert_with(|| rates.empty())
            .add(item);
        self.totals.add(item);
        self.models
            .entry(model)
            .or_insert_with(|| rates.empty())
            .add(item);
        let (spend, members) = self
            .projects
            .entry(project.to_string())
            .or_insert_with(|| (rates.empty(), HashSet::new()));
        spend.add(item);
        members.insert(session.to_string());
        let (spend, last) = self
            .sessions
            .entry(session.to_string())
            .or_insert_with(|| (rates.empty(), ts));
        spend.add(item);
        *last = (*last).max(ts);
        Some(index)
    }
}

pub fn build(range: Range, spans: &Spans, inputs: &Inputs, now_ms: i64) -> HistoryReport {
    let rates = Rates::of(inputs.fits);
    let mut ledger = Ledger::new(spans, &rates, inputs.sessions);
    let (mut read, mut prompt, mut saved_usd) = (0u64, 0u64, 0.0);
    for request in inputs.requests {
        let point = &request.point;
        let item = Item {
            cost_usd: request.cost_usd,
            tokens: all_tokens(&point.tokens),
            pct_5h: rates.pct(&point.model, request.cost_usd),
            is_request: true,
        };
        let Some(index) = ledger.add(&request.session_id, point.ts, &point.model, &item) else {
            continue;
        };
        ledger.buckets[index].prompt_tokens += point.tokens.prompt_total();
        ledger.buckets[index].cache_read_tokens += point.tokens.cache_read;
        read += point.tokens.cache_read;
        prompt += point.tokens.prompt_total();
        saved_usd +=
            cache_model::saved_usd(&point.model, &point.tokens, inputs.prices).unwrap_or(0.0);
    }
    for adjustment in inputs.adjustments {
        let item = Item {
            cost_usd: adjustment.cost_usd,
            tokens: all_tokens(&adjustment.tokens),
            pct_5h: rates.pct(&adjustment.model, adjustment.cost_usd),
            is_request: false,
        };
        ledger.add(
            &adjustment.session_id,
            adjustment.ts,
            &adjustment.model,
            &item,
        );
    }
    let Ledger {
        info,
        mut buckets,
        by_bucket,
        totals,
        models,
        projects,
        sessions,
        ..
    } = ledger;
    let order = ranked(models);
    for (bucket, spent) in buckets.iter_mut().zip(by_bucket) {
        bucket.models = order
            .iter()
            .filter_map(|model| {
                spent.get(&model.model).map(|spend| ModelSpend {
                    model: model.model.clone(),
                    spend: spend.clone(),
                })
            })
            .collect();
    }
    let (mut rebuilds, mut rebuild_usd, mut idle_rebuilds) = (0, 0.0, 0);
    for rebuild in inputs.rebuilds {
        let Some(index) = spans.index_of(rebuild.ts_ms.div_euclid(1000)) else {
            continue;
        };
        let extra = rebuild.extra_usd.unwrap_or(0.0);
        buckets[index].rebuilds += 1;
        buckets[index].rebuild_usd += extra;
        rebuilds += 1;
        rebuild_usd += extra;
        if rebuild.cause == RebuildCause::Idle {
            idle_rebuilds += 1;
        }
    }
    for sample in inputs.samples.iter().filter(|s| s.window == FIVE_HOUR) {
        if let Some(index) = spans.index_of(sample.ts_ms.div_euclid(1000)) {
            let peak = &mut buckets[index].peak_5h_pct;
            *peak = Some(peak.map_or(sample.used_pct, |p| p.max(sample.used_pct)));
        }
    }
    let mut compactions = 0;
    for compaction in inputs.compactions.iter().filter(|c| c.main) {
        if let Some(index) = spans.index_of(compaction.ts) {
            buckets[index].compactions += 1;
            compactions += 1;
        }
    }
    let peak_5h_pct = buckets
        .iter()
        .filter_map(|bucket| bucket.peak_5h_pct)
        .reduce(f64::max);
    let names = project_names(inputs.sessions);
    let projects = top(
        projects.into_iter().map(|(key, (spend, members))| {
            let (name, path) = names
                .get(&key)
                .cloned()
                .unwrap_or_else(|| (key.clone(), None));
            ProjectSpend {
                name,
                path,
                sessions: members.len() as u64,
                spend,
            }
        }),
        |project| &project.spend,
    );
    let session_count = sessions.len() as u64;
    let sessions = top(
        sessions.into_iter().map(|(id, (spend, last))| {
            let found = info.get(id.as_str());
            SessionSpend {
                title: found.and_then(|s| s.title.clone()),
                project: found
                    .and_then(|s| names.get(&s.project))
                    .map_or_else(String::new, |(name, _)| name.clone()),
                last_ms: last * 1000,
                id,
                spend,
            }
        }),
        |session| &session.spend,
    );
    HistoryReport {
        v: HISTORY_VERSION,
        range,
        bucket: range.bucket(),
        generated_at_ms: now_ms,
        since_ms: spans.since() * 1000,
        until_ms: spans.until * 1000,
        account: None,
        accounts: Vec::new(),
        calibration: Calibration {
            calibrated: rates.fallback.is_some(),
            pairs: rates.pairs,
            needed: PAIRS_NEEDED,
        },
        totals: Totals {
            spend: totals,
            sessions: session_count,
            hit_ratio: (prompt > 0).then(|| read as f64 / prompt as f64),
            saved_usd,
            rebuilds,
            rebuild_usd,
            idle_rebuilds,
            compactions,
            peak_5h_pct,
        },
        buckets,
        models: order.into_iter().take(TOP).collect(),
        projects,
        sessions,
    }
}

fn empty_bucket(span: &Span, rates: &Rates) -> Bucket {
    Bucket {
        start_ms: span.start * 1000,
        label: span.label.clone(),
        spend: rates.empty(),
        models: Vec::new(),
        prompt_tokens: 0,
        cache_read_tokens: 0,
        rebuilds: 0,
        rebuild_usd: 0.0,
        compactions: 0,
        peak_5h_pct: None,
    }
}

fn all_tokens(tokens: &TokenCounts) -> u64 {
    tokens.prompt_total() + tokens.output
}

fn by_spend(a: &Spend, b: &Spend) -> std::cmp::Ordering {
    b.cost_usd
        .total_cmp(&a.cost_usd)
        .then_with(|| b.tokens.cmp(&a.tokens))
}

fn ranked(models: BTreeMap<String, Spend>) -> Vec<ModelSpend> {
    let mut ranked: Vec<ModelSpend> = models
        .into_iter()
        .map(|(model, spend)| ModelSpend { model, spend })
        .collect();
    ranked.sort_by(|a, b| by_spend(&a.spend, &b.spend).then_with(|| a.model.cmp(&b.model)));
    ranked
}

fn top<T>(items: impl Iterator<Item = T>, spend: impl Fn(&T) -> &Spend) -> Vec<T> {
    let mut items: Vec<T> = items.collect();
    items.sort_by(|a, b| by_spend(spend(a), spend(b)));
    items.truncate(TOP);
    items
}

fn project_names(sessions: &[SessionInfo]) -> HashMap<String, (String, Option<String>)> {
    let mut paths: BTreeMap<&str, &str> = BTreeMap::new();
    for session in sessions {
        if let Some(cwd) = session.cwd.as_deref() {
            paths.entry(&session.project).or_insert(cwd);
        }
    }
    let tail = |path: &str, parts: usize| -> String {
        let mut pieces: Vec<&str> = path.trim_end_matches('/').rsplit('/').take(parts).collect();
        pieces.reverse();
        pieces.join("/")
    };
    paths
        .iter()
        .map(|(project, path)| {
            let short = tail(path, 1);
            let shared = paths
                .iter()
                .any(|(other, p)| other != project && tail(p, 1) == short);
            let name = if shared { tail(path, 2) } else { short };
            (project.to_string(), (name, Some(path.to_string())))
        })
        .collect()
}

#[cfg(test)]
mod tests;
