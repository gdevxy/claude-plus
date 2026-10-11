pub mod breakdown;
pub mod history;
pub mod items;
pub mod ratios;

use std::collections::BTreeMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config_scan::{Loading, Scan, SourceKind};
use crate::store::Store;
pub use breakdown::{Breakdown, Category, CategoryKind};
pub use history::{History, Part, Point, Share};
pub use items::{Item, ItemKind, Uses};
pub use ratios::{Ratios, TextKind};

pub const CONTEXT_VERSION: u32 = 1;
pub const CALIBRATION_BREAKDOWNS: usize = 10;
pub const RECENT_DAYS: i64 = 14;
pub const TOOL_RESULTS: usize = 5;
const DAY_S: i64 = 86_400;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextReport {
    pub v: u32,
    pub session: String,
    pub generated_at_ms: i64,
    pub project_dir: Option<String>,
    pub live: Option<LiveContext>,
    pub history: Option<History>,
    pub ratios: Ratios,
    pub items: Vec<Item>,
    pub dead_weight: Vec<Item>,
    pub tool_results: Vec<ToolResult>,
    pub config: ConfigSummary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveContext {
    pub ts_ms: i64,
    pub model: String,
    pub total_tokens: u64,
    pub max_tokens: u64,
    pub percentage: f64,
    pub auto_compact_threshold: Option<u64>,
    pub categories: Vec<Category>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool: Option<String>,
    pub ts_ms: i64,
    pub chars: u64,
    pub images: u32,
    pub tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSummary {
    pub scanned_at_ms: i64,
    pub groups: Vec<ConfigGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigGroup {
    pub kind: SourceKind,
    pub loading: Loading,
    pub count: u64,
    pub tokens: u64,
}

pub struct Request<'a> {
    pub session: &'a str,
    pub project_dir: Option<&'a str>,
    pub scan: &'a Scan,
    pub scanned_at_ms: i64,
    pub now_ms: i64,
}

pub fn read(store: &Store, request: &Request) -> Result<ContextReport> {
    let session = request.session;
    let live = store
        .live_breakdown(session)?
        .and_then(|stored| breakdown::parse(stored.ts_ms, &stored.json));
    let recent: Vec<Breakdown> = store
        .recent_breakdowns(CALIBRATION_BREAKDOWNS)?
        .into_iter()
        .filter_map(|stored| breakdown::parse(stored.ts_ms, &stored.json))
        .collect();
    let ratios = ratios::calibrate(request.scan, &recent);
    let history = history::compose(
        &store.session_requests(session)?,
        &store.main_attachments(session)?,
        &store.main_compactions(session)?,
        store.first_prompt_chars(session)?,
        &ratios,
    );
    let since = request.now_ms / 1000 - RECENT_DAYS * DAY_S;
    let uses = Uses {
        session_skills: store.skill_uses(Some(session), 0)?,
        recent_skills: store.skill_uses(None, since)?,
        session_tools: store.mcp_tool_uses(Some(session), 0)?,
        recent_tools: store.mcp_tool_uses(None, since)?,
    };
    let all = items::items(live.as_ref(), request.scan, &ratios, &uses);
    let tool_results = store
        .largest_tool_results(session, TOOL_RESULTS)?
        .into_iter()
        .map(|point| ToolResult {
            tokens: ratios.tokens(point.chars, TextKind::Code),
            tool: point.tool,
            ts_ms: point.ts * 1000,
            chars: point.chars,
            images: point.images,
        })
        .collect();
    Ok(ContextReport {
        v: CONTEXT_VERSION,
        session: session.to_string(),
        generated_at_ms: request.now_ms,
        project_dir: request.project_dir.map(String::from),
        live: live.map(|b| LiveContext {
            ts_ms: b.ts_ms,
            model: b.model,
            total_tokens: b.total_tokens,
            max_tokens: b.max_tokens,
            percentage: b.percentage,
            auto_compact_threshold: b.auto_compact_threshold,
            categories: b.categories,
        }),
        history,
        items: items::sized(&all),
        dead_weight: items::dead_weight(&all),
        tool_results,
        config: summary(request.scan, request.scanned_at_ms, &ratios),
        ratios,
    })
}

fn summary(scan: &Scan, scanned_at_ms: i64, ratios: &Ratios) -> ConfigSummary {
    let mut groups: BTreeMap<(SourceKind, Loading), (u64, u64)> = BTreeMap::new();
    for source in &scan.sources {
        let group = groups.entry((source.kind, source.loading)).or_default();
        group.0 += 1;
        group.1 += ratios.tokens(source.chars, TextKind::Prose);
    }
    ConfigSummary {
        scanned_at_ms,
        groups: groups
            .into_iter()
            .map(|((kind, loading), (count, tokens))| ConfigGroup {
                kind,
                loading,
                count,
                tokens,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests;
