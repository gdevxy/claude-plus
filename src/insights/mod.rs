mod cache;
mod report;
mod setup;
pub mod sources;
mod subagents;
mod ttl;
mod usage;
pub mod words;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::cache_model::Rebuild;
use crate::context_model::{Item, Ratios};
use crate::history::Rates;
use crate::pricing::PriceTable;
use crate::store::{CompactionPoint, LargeResult, PromptPoint, SessionInfo, SpentRequest};
use crate::timeline::Observed;
use crate::transcript::TokenCounts;

pub use cache::{CacheBusts, IdleRebuilds};
pub use report::{Day, InsightsReport, Recap, Request, Wins, days, read, recap, wins};
pub use setup::{OversizedMemory, Retention, UnusedMcpServers, UnusedSkills};
pub use subagents::SubagentModelFit;
pub use ttl::CacheTtl;
pub use usage::{CostlyTasks, LargeToolResults, LateCompaction, ModelFit, Task, tasks};

pub const INSIGHTS_VERSION: u32 = 1;
pub const WINDOW_DAYS: i64 = 14;
pub const WEEKS_IN_WINDOW: f64 = WINDOW_DAYS as f64 / 7.0;
pub const MIN_WEEKLY_USD: f64 = 0.10;
pub const SHOWN_EVIDENCE: usize = 3;
pub const DAY_S: i64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleId {
    IdleRebuilds,
    UnusedMcpServers,
    UnusedSkills,
    OversizedMemory,
    LargeToolResults,
    LateCompaction,
    ModelFit,
    SubagentModelFit,
    CacheBusts,
    CacheTtl,
    CostlyTasks,
    Retention,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub rule: RuleId,
    pub tip: String,
    pub evidence: Vec<String>,
    pub saving_usd: Option<f64>,
    pub saving_pct_5h: Option<f64>,
    pub confidence: Confidence,
    pub how: String,
}

pub trait Rule {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rebuilt {
    pub session_id: String,
    pub request: Observed,
    pub rebuild: Rebuild,
    pub gap_ms: i64,
    pub ttl_ms: i64,
    pub kinds: Vec<String>,
    pub mcp_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryText {
    pub name: String,
    pub text: String,
}

pub struct InsightContext<'a> {
    pub now_ms: i64,
    pub since_ts: i64,
    pub prices: &'a PriceTable,
    pub rates: &'a Rates,
    pub requests: &'a [SpentRequest],
    pub agent_types: &'a BTreeMap<String, String>,
    pub sessions: &'a [SessionInfo],
    pub project: Option<&'a str>,
    pub prompts: &'a [PromptPoint],
    pub rebuilds: &'a [Rebuilt],
    pub tool_results: &'a [LargeResult],
    pub compactions: &'a [CompactionPoint],
    pub dead_weight: &'a [Item],
    pub month_items: &'a [Item],
    pub memory: &'a [MemoryText],
    pub ratios: &'a Ratios,
    pub retention_days: Option<u64>,
}

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(IdleRebuilds),
        Box::new(UnusedMcpServers),
        Box::new(UnusedSkills),
        Box::new(OversizedMemory),
        Box::new(LargeToolResults),
        Box::new(LateCompaction),
        Box::new(ModelFit),
        Box::new(SubagentModelFit),
        Box::new(CacheBusts),
        Box::new(CacheTtl),
        Box::new(CostlyTasks),
        Box::new(Retention),
    ]
}

pub fn evaluate(ctx: &InsightContext) -> Vec<Finding> {
    rank(rules().iter().flat_map(|rule| rule.evaluate(ctx)).collect())
}

pub fn rank(mut findings: Vec<Finding>) -> Vec<Finding> {
    findings.sort_by(|a, b| {
        b.saving_usd
            .unwrap_or(-1.0)
            .total_cmp(&a.saving_usd.unwrap_or(-1.0))
            .then_with(|| a.rule.cmp(&b.rule))
    });
    findings
}

pub fn front_of_prompt(tokens: &TokenCounts, carried: u64) -> TokenCounts {
    let mut left = carried;
    let mut take = |available: u64| {
        let taken = left.min(available);
        left -= taken;
        taken
    };
    TokenCounts {
        cache_read: take(tokens.cache_read),
        cache_write_1h: take(tokens.cache_write_1h),
        cache_write_5m: take(tokens.cache_write_5m),
        input: take(tokens.input),
        ..Default::default()
    }
}

pub fn listed<T>(items: &[T], line: impl Fn(&T) -> String, what: &str) -> Vec<String> {
    let mut lines: Vec<String> = items.iter().take(SHOWN_EVIDENCE).map(line).collect();
    if items.len() > SHOWN_EVIDENCE {
        lines.push(format!(
            "and {}\u{a0}more {what}",
            items.len() - SHOWN_EVIDENCE
        ));
    }
    lines
}

pub fn median(mut values: Vec<f64>) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    match values.len() {
        0 => None,
        n if n % 2 == 1 => Some(values[middle]),
        _ => Some((values[middle - 1] + values[middle]) / 2.0),
    }
}

pub fn is_main(request: &SpentRequest) -> bool {
    !request.point.is_sidechain && request.point.agent_id.is_none()
}

impl InsightContext<'_> {
    pub fn main_requests(&self) -> impl Iterator<Item = &SpentRequest> {
        self.requests.iter().filter(|r| is_main(r))
    }

    pub fn session(&self, session_id: &str) -> Option<&SessionInfo> {
        self.sessions.iter().find(|s| s.id == session_id)
    }

    pub fn in_project(&self, session_id: &str) -> bool {
        self.project.is_some_and(|project| {
            self.session(session_id)
                .is_some_and(|session| session.project == project)
        })
    }

    pub fn title(&self, session_id: &str) -> String {
        self.session(session_id)
            .and_then(|s| s.title.clone())
            .unwrap_or_else(|| {
                format!(
                    "Untitled {}",
                    session_id.chars().take(8).collect::<String>()
                )
            })
    }

    pub fn carried_usd(&self, request: &SpentRequest, carried: u64) -> f64 {
        self.carried(&request.point.model, &request.point.tokens, carried)
    }

    fn carried(&self, model: &str, tokens: &TokenCounts, carried: u64) -> f64 {
        self.prices
            .cost_within(
                model,
                &front_of_prompt(tokens, carried),
                tokens.prompt_total(),
            )
            .unwrap_or(0.0)
    }

    pub fn rewrite_premium(&self, request: &Observed, tokens: u64) -> f64 {
        let read = TokenCounts {
            cache_read: tokens,
            ..Default::default()
        };
        let warm = self
            .prices
            .cost_within(&request.model, &read, request.tokens.prompt_total())
            .unwrap_or(0.0);
        self.carried(&request.model, &request.tokens, tokens) - warm
    }

    pub fn carrying_usd(&self, tokens: u64) -> f64 {
        self.main_requests()
            .filter(|r| self.in_project(&r.session_id))
            .map(|r| self.carried_usd(r, tokens))
            .sum()
    }

    pub fn project_requests(&self) -> usize {
        self.main_requests()
            .filter(|r| self.in_project(&r.session_id))
            .count()
    }

    pub fn dominant_model(&self) -> &str {
        let mut spend: BTreeMap<&str, f64> = BTreeMap::new();
        for request in self.main_requests() {
            *spend.entry(request.point.model.as_str()).or_default() +=
                request.cost_usd.unwrap_or(0.0);
        }
        spend
            .into_iter()
            .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(a.0)))
            .map_or("", |(model, _)| model)
    }

    pub fn priced(&self, finding: Draft, window_usd: f64) -> Option<Finding> {
        let weekly = window_usd / WEEKS_IN_WINDOW;
        (weekly >= MIN_WEEKLY_USD).then(|| Finding {
            saving_usd: Some(weekly),
            saving_pct_5h: self.rates.pct(self.dominant_model(), Some(weekly)),
            ..finding.into_finding()
        })
    }
}

pub struct Draft {
    pub rule: RuleId,
    pub tip: String,
    pub evidence: Vec<String>,
    pub confidence: Confidence,
    pub how: String,
}

impl Draft {
    pub fn into_finding(self) -> Finding {
        Finding {
            rule: self.rule,
            tip: self.tip,
            evidence: self.evidence,
            saving_usd: None,
            saving_pct_5h: None,
            confidence: self.confidence,
            how: self.how,
        }
    }
}

#[cfg(test)]
pub(crate) mod fixture;
#[cfg(test)]
mod tests;
