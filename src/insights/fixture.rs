use super::{DAY_S, InsightContext, MemoryText, Rebuilt, WINDOW_DAYS};
use crate::context_model::{Item, Ratios};
use crate::history::Rates;
use crate::limits::Fit;
use crate::pricing::PriceTable;
use crate::store::{
    CompactionPoint, LargeResult, PromptPoint, RequestPoint, SessionInfo, SpentRequest,
};
use crate::transcript::TokenCounts;

pub(crate) const NOW_S: i64 = 1_791_100_000;
pub(crate) const SINCE: i64 = NOW_S - WINDOW_DAYS * DAY_S;
pub(crate) const PROJECT: &str = "-work-demo";
pub(crate) const OPUS: &str = "claude-opus-5-5";
pub(crate) const SONNET: &str = "claude-sonnet-5-5";

pub(crate) struct Fixture {
    pub prices: PriceTable,
    pub rates: Rates,
    pub requests: Vec<SpentRequest>,
    pub sessions: Vec<SessionInfo>,
    pub project: Option<String>,
    pub prompts: Vec<PromptPoint>,
    pub rebuilds: Vec<Rebuilt>,
    pub tool_results: Vec<LargeResult>,
    pub compactions: Vec<CompactionPoint>,
    pub dead_weight: Vec<Item>,
    pub month_items: Vec<Item>,
    pub memory: Vec<MemoryText>,
    pub ratios: Ratios,
    pub retention_days: Option<u64>,
}

pub(crate) fn tokens(input: u64, cache_read: u64, cache_write_5m: u64, output: u64) -> TokenCounts {
    TokenCounts {
        input,
        cache_read,
        cache_write_5m,
        output,
        ..Default::default()
    }
}

impl Fixture {
    pub fn new() -> Self {
        Self {
            prices: PriceTable::bundled(),
            rates: Rates::of(&[]),
            requests: Vec::new(),
            sessions: Vec::new(),
            project: Some(PROJECT.into()),
            prompts: Vec::new(),
            rebuilds: Vec::new(),
            tool_results: Vec::new(),
            compactions: Vec::new(),
            dead_weight: Vec::new(),
            month_items: Vec::new(),
            memory: Vec::new(),
            ratios: Ratios::default(),
            retention_days: Some(90),
        }
    }

    pub fn ctx(&self) -> InsightContext<'_> {
        InsightContext {
            now_ms: NOW_S * 1000,
            since_ts: SINCE,
            prices: &self.prices,
            rates: &self.rates,
            requests: &self.requests,
            sessions: &self.sessions,
            project: self.project.as_deref(),
            prompts: &self.prompts,
            rebuilds: &self.rebuilds,
            tool_results: &self.tool_results,
            compactions: &self.compactions,
            dead_weight: &self.dead_weight,
            month_items: &self.month_items,
            memory: &self.memory,
            ratios: &self.ratios,
            retention_days: self.retention_days,
        }
    }

    pub fn calibrated(&mut self, pct_per_usd: f64) {
        self.rates = Rates::of(&[Fit {
            window: "five_hour".into(),
            family: "opus".into(),
            pairs: 30,
            pct_per_usd,
        }]);
    }

    pub fn session(&mut self, id: &str, project: &str, title: Option<&str>) {
        self.sessions.push(SessionInfo {
            id: id.into(),
            project: project.into(),
            cwd: Some("/work/demo".into()),
            title: title.map(String::from),
        });
    }

    pub fn request(&mut self, session: &str, ts: i64, model: &str, tokens: TokenCounts) {
        self.push(session, ts, model, tokens, None);
    }

    pub fn subagent(&mut self, session: &str, ts: i64, model: &str, tokens: TokenCounts) {
        self.push(session, ts, model, tokens, Some("a1"));
    }

    fn push(
        &mut self,
        session: &str,
        ts: i64,
        model: &str,
        tokens: TokenCounts,
        agent: Option<&str>,
    ) {
        self.requests.push(SpentRequest {
            session_id: session.into(),
            cost_usd: self.prices.cost(model, &tokens),
            point: RequestPoint {
                ts,
                sent: None,
                model: model.into(),
                tokens,
                is_sidechain: agent.is_some(),
                agent_id: agent.map(String::from),
                effort: None,
            },
        });
    }
}
