use std::collections::BTreeMap;

use super::{DAY_S, InsightContext, MemoryText, Rebuilt, WINDOW_DAYS};
use crate::cache_model::FIVE_MINUTES_MS;
use crate::cache_model::{Rebuild, RebuildCause};
use crate::context_model::{Item, ItemKind, Ratios};
use crate::history::Rates;
use crate::limits::Fit;
use crate::pricing::PriceTable;
use crate::store::{
    CompactionPoint, LargeResult, PromptPoint, RequestPoint, SessionInfo, SpentRequest,
};
use crate::timeline::{Observed, Source, Thread};
use crate::transcript::TokenCounts;

pub(crate) const NOW_S: i64 = 1_791_100_000;
pub(crate) const SINCE: i64 = NOW_S - WINDOW_DAYS * DAY_S;
pub(crate) const PROJECT: &str = "-work-demo";
pub(crate) const OPUS: &str = "claude-opus-5-5";
pub(crate) const SONNET: &str = "claude-sonnet-5-5";
pub(crate) const HAIKU: &str = "claude-haiku-5-5";

pub(crate) struct Fixture {
    pub prices: PriceTable,
    pub rates: Rates,
    pub requests: Vec<SpentRequest>,
    pub agent_types: BTreeMap<String, String>,
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

pub(crate) fn counts(input: u64, cache_read: u64, cache_write_5m: u64, output: u64) -> TokenCounts {
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
            agent_types: BTreeMap::new(),
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
            agent_types: &self.agent_types,
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
            account: None,
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

    pub fn rebuilt(
        &mut self,
        session: &str,
        cause: RebuildCause,
        usd: f64,
        gap_ms: i64,
        kinds: &[&str],
    ) {
        let ts_ms = (SINCE + 3_600) * 1000;
        self.rebuilds.push(Rebuilt {
            session_id: session.into(),
            request: Observed {
                ts_ms,
                model: OPUS.into(),
                thread: Thread::Main,
                tokens: counts(0, 0, 80_000, 10),
                effort: None,
                source: Source::Transcript,
            },
            rebuild: Rebuild {
                ts_ms,
                cause,
                tokens: 80_000,
                extra_usd: Some(usd),
            },
            gap_ms,
            ttl_ms: FIVE_MINUTES_MS,
            kinds: kinds.iter().map(|k| k.to_string()).collect(),
            mcp_changed: false,
        });
    }

    pub fn mcp_changed(&mut self) {
        if let Some(last) = self.rebuilds.last_mut() {
            last.mcp_changed = true;
        }
    }

    pub fn prompt(&mut self, session: &str, ts: i64) {
        self.prompts.push(PromptPoint {
            session_id: session.into(),
            ts,
        });
    }

    pub fn effort(&mut self, effort: &str) {
        if let Some(last) = self.requests.last_mut() {
            last.point.effort = Some(effort.into());
        }
    }

    pub fn large(&mut self, session: &str, ts: i64, chars: u64, tool: &str, target: Option<&str>) {
        self.tool_results.push(LargeResult {
            session_id: session.into(),
            ts,
            chars,
            tool: Some(tool.into()),
            target: target.map(String::from),
        });
    }

    pub fn compaction(&mut self, session: &str, ts: i64) {
        self.compactions.push(CompactionPoint {
            session_id: session.into(),
            ts,
            main: true,
            post_tokens: None,
        });
    }

    pub fn compacted(&mut self, session: &str, ts: i64, post_tokens: u64) {
        self.compactions.push(CompactionPoint {
            session_id: session.into(),
            ts,
            main: true,
            post_tokens: Some(post_tokens),
        });
    }

    pub fn request(&mut self, session: &str, ts: i64, model: &str, tokens: TokenCounts) {
        self.push(session, ts, model, tokens, None);
    }

    pub fn subagent(&mut self, session: &str, ts: i64, model: &str, tokens: TokenCounts) {
        self.push(session, ts, model, tokens, Some("a1"));
    }

    pub fn push_agent(&mut self, agent: &str, ts: i64, model: &str, tokens: TokenCounts) {
        self.push("s1", ts, model, tokens, Some(agent));
    }

    pub fn typed(
        &mut self,
        agent: &str,
        agent_type: &str,
        ts: i64,
        model: &str,
        tokens: TokenCounts,
    ) {
        self.push("s1", ts, model, tokens, Some(agent));
        self.agent_types.insert(agent.into(), agent_type.into());
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

pub(crate) fn item(kind: ItemKind, name: &str, tokens: u64, estimated: bool, uses: u64) -> Item {
    Item {
        kind,
        name: name.into(),
        detail: None,
        tokens,
        estimated,
        uses_session: Some(0),
        uses_14d: Some(uses),
    }
}

pub(crate) fn memory(name: &str, text: &str) -> MemoryText {
    MemoryText {
        name: name.into(),
        text: text.into(),
    }
}
