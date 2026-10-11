use std::collections::{BTreeMap, BTreeSet};

use super::words::{count, duration, money, tokens};
use super::{
    Confidence, Draft, Finding, InsightContext, MIN_WEEKLY_USD, Rebuilt, Rule, RuleId,
    WEEKS_IN_WINDOW, median,
};
use crate::cache_model::RebuildCause;
use crate::store::PrefixChange;

pub const SETUP_KINDS: &[&str] = &[
    "skill_listing",
    "instructions",
    "nested_memory",
    "agent_listing_delta",
];
pub const MCP_TOOL_PREFIX: &str = "mcp__";

pub fn mcp_changes(changes: &[PrefixChange]) -> Vec<bool> {
    let mut tools: BTreeSet<&str> = BTreeSet::new();
    let mut servers: BTreeSet<&str> = BTreeSet::new();
    changes
        .iter()
        .map(|change| {
            let (known, only_mcp) = match change.kind.as_str() {
                "deferred_tools_delta" => (&mut tools, true),
                "mcp_instructions_delta" => (&mut servers, false),
                _ => return false,
            };
            let counted = |name: &&String| !only_mcp || name.starts_with(MCP_TOOL_PREFIX);
            let mut changed = false;
            for name in change.names.iter().filter(counted) {
                changed |= known.insert(name);
            }
            for name in change.removed.iter().filter(counted) {
                changed |= known.remove(name.as_str());
            }
            changed
        })
        .collect()
}

pub struct IdleRebuilds;

impl Rule for IdleRebuilds {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        let idle: Vec<&Rebuilt> = ctx
            .rebuilds
            .iter()
            .filter(|r| r.rebuild.cause == RebuildCause::Idle)
            .collect();
        let Some(gap) = median(idle.iter().map(|r| r.gap_ms as f64).collect()) else {
            return Vec::new();
        };
        let usd: f64 = idle.iter().filter_map(|r| r.rebuild.extra_usd).sum();
        let sessions: BTreeSet<&str> = idle.iter().map(|r| r.session_id.as_str()).collect();
        let rewritten = idle.iter().map(|r| r.rebuild.tokens).sum::<u64>() / idle.len() as u64;
        let ttl = most_common(idle.iter().map(|r| r.ttl_ms));
        let draft = Draft {
            rule: RuleId::IdleRebuilds,
            tip: "start fresh after a break instead of rewarming a long context".into(),
            evidence: vec![
                format!(
                    "{} after a break, in {}, {}",
                    count(idle.len() as u64, "cache rebuild", "cache rebuilds"),
                    count(sessions.len() as u64, "session", "sessions"),
                    money(usd)
                ),
                format!(
                    "Typical break {}, the cache lasts {}",
                    duration(gap as i64),
                    duration(ttl)
                ),
                format!("Each rebuild rewrote ~{} tokens", tokens(rewritten)),
            ],
            confidence: Confidence::High,
            how: format!(
                "Counts main-thread requests that rewrote the cache after a gap longer than its {} lifetime in the last 14 days. \
                 The saving is what those rewrites cost above a warm cache read. \
                 Coming back with /clear or a new session rebuilds only a small context.",
                duration(ttl)
            ),
        };
        ctx.priced(draft, usd).into_iter().collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Bust {
    ModelSwitch,
    McpChange,
    SetupChange,
    NoKnownCause,
}

impl Bust {
    fn of(rebuilt: &Rebuilt) -> Option<Self> {
        let setup = rebuilt
            .kinds
            .iter()
            .any(|k| SETUP_KINDS.contains(&k.as_str()));
        match rebuilt.rebuild.cause {
            RebuildCause::ModelSwitch => Some(Bust::ModelSwitch),
            RebuildCause::PrefixChanged if rebuilt.mcp_changed => Some(Bust::McpChange),
            RebuildCause::PrefixChanged if setup => Some(Bust::SetupChange),
            RebuildCause::PrefixChanged | RebuildCause::Unknown => Some(Bust::NoKnownCause),
            RebuildCause::Idle => None,
        }
    }

    fn line(self, times: u64, usd: f64) -> String {
        let what = match self {
            Bust::ModelSwitch => count(times, "model switch", "model switches"),
            Bust::McpChange => count(times, "MCP tool change", "MCP tool changes"),
            Bust::SetupChange => count(
                times,
                "change to CLAUDE.md, skills or agents",
                "changes to CLAUDE.md, skills or agents",
            ),
            Bust::NoKnownCause => {
                return format!(
                    "{} mid-session with no cause in the transcripts, {}",
                    count(times, "rewrite", "rewrites"),
                    money(usd)
                );
            }
        };
        format!("{what} mid-session, {}", money(usd))
    }

    fn tip(self) -> &'static str {
        match self {
            Bust::ModelSwitch => {
                "pick the model when a session starts rather than switching midway"
            }
            Bust::McpChange => "keep MCP servers connected for the whole session",
            Bust::SetupChange => "edit CLAUDE.md, skills and agents between sessions",
            Bust::NoKnownCause => {
                "watch for warm caches rewritten mid-session, the transcripts show no cause"
            }
        }
    }
}

pub struct CacheBusts;

impl Rule for CacheBusts {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        let mut busts: BTreeMap<Bust, (u64, f64)> = BTreeMap::new();
        for rebuilt in ctx.rebuilds {
            if let Some(bust) = Bust::of(rebuilt) {
                let entry = busts.entry(bust).or_default();
                entry.0 += 1;
                entry.1 += rebuilt.rebuild.extra_usd.unwrap_or(0.0);
            }
        }
        let mut ordered: Vec<(Bust, (u64, f64))> = busts.into_iter().collect();
        ordered.sort_by(|a, b| b.1.1.total_cmp(&a.1.1).then_with(|| a.0.cmp(&b.0)));
        let evidence = ordered
            .iter()
            .map(|(bust, (times, usd))| bust.line(*times, *usd))
            .collect();
        let known: Vec<&(Bust, (u64, f64))> = ordered
            .iter()
            .filter(|(bust, _)| *bust != Bust::NoKnownCause)
            .collect();
        let Some((top, _)) = known.first() else {
            let unexplained: f64 = ordered.iter().map(|(_, (_, usd))| usd).sum();
            if unexplained / WEEKS_IN_WINDOW < MIN_WEEKLY_USD {
                return Vec::new();
            }
            return vec![
                Draft {
                    rule: RuleId::CacheBusts,
                    tip: Bust::NoKnownCause.tip().into(),
                    evidence,
                    confidence: Confidence::Low,
                    how: "Counts main-thread requests in the last 14 days that rewrote a warm cache \
                          although the model, the MCP tools, CLAUDE.md, skills and agents stayed the same between them. \
                          The transcripts do not say why, so no saving is claimed."
                        .into(),
                }
                .into_finding(),
            ];
        };
        let usd = known.iter().map(|(_, (_, usd))| usd).sum();
        let draft = Draft {
            rule: RuleId::CacheBusts,
            tip: top.tip().into(),
            evidence,
            confidence: Confidence::Medium,
            how: "Counts main-thread requests in the last 14 days that rewrote a warm cache because the model changed, \
                  an MCP server added or removed tools or CLAUDE.md, skills or agents changed. \
                  The saving is what those rewrites cost above a warm cache read. \
                  Rewrites with no cause in the transcripts are listed but not counted."
                .into(),
        };
        ctx.priced(draft, usd).into_iter().collect()
    }
}

fn most_common(values: impl Iterator<Item = i64>) -> i64 {
    let mut counts: BTreeMap<i64, usize> = BTreeMap::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map_or(0, |(value, _)| value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::fixture::*;

    #[test]
    fn idle_rebuilds_report_their_cost_typical_break_and_size() {
        let mut fixture = Fixture::new();
        fixture.rebuilt("s1", RebuildCause::Idle, 0.40, 12 * 60_000, &[]);
        fixture.rebuilt("s1", RebuildCause::Idle, 0.30, 14 * 60_000, &[]);
        fixture.rebuilt("s2", RebuildCause::Idle, 0.50, 90 * 60_000, &[]);
        fixture.rebuilt("s2", RebuildCause::ModelSwitch, 2.00, 60_000, &[]);
        let findings = IdleRebuilds.evaluate(&fixture.ctx());
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(finding.rule, RuleId::IdleRebuilds);
        assert_eq!(finding.saving_usd, Some(0.6));
        assert_eq!(finding.confidence, Confidence::High);
        assert_eq!(
            finding.evidence,
            [
                "3 cache rebuilds after a break, in 2 sessions, $1.20",
                "Typical break 14m, the cache lasts 5m",
                "Each rebuild rewrote ~80k tokens",
            ]
        );
    }

    #[test]
    fn no_idle_rebuild_or_a_trivial_cost_means_no_tip() {
        let mut fixture = Fixture::new();
        assert!(IdleRebuilds.evaluate(&fixture.ctx()).is_empty());
        fixture.rebuilt("s1", RebuildCause::Idle, 0.19, 12 * 60_000, &[]);
        assert!(IdleRebuilds.evaluate(&fixture.ctx()).is_empty());
    }

    #[test]
    fn cache_busts_name_each_cause_and_lead_with_the_costliest() {
        let mut fixture = Fixture::new();
        fixture.rebuilt("s1", RebuildCause::ModelSwitch, 0.30, 60_000, &[]);
        fixture.rebuilt(
            "s1",
            RebuildCause::PrefixChanged,
            0.50,
            60_000,
            &["deferred_tools_delta"],
        );
        fixture.mcp_changed();
        fixture.rebuilt(
            "s1",
            RebuildCause::PrefixChanged,
            0.40,
            60_000,
            &["mcp_instructions_delta", "skill_listing"],
        );
        fixture.mcp_changed();
        fixture.rebuilt(
            "s2",
            RebuildCause::PrefixChanged,
            0.20,
            60_000,
            &["instructions"],
        );
        fixture.rebuilt("s2", RebuildCause::Idle, 3.00, 600_000, &[]);
        fixture.rebuilt("s2", RebuildCause::Unknown, 0.15, 60_000, &[]);
        let findings = CacheBusts.evaluate(&fixture.ctx());
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(
            finding.tip,
            "keep MCP servers connected for the whole session"
        );
        assert_eq!(finding.saving_usd, Some(0.7));
        assert_eq!(finding.confidence, Confidence::Medium);
        assert_eq!(
            finding.evidence,
            [
                "2 MCP tool changes mid-session, $0.90",
                "1 model switch mid-session, $0.30",
                "1 change to CLAUDE.md, skills or agents mid-session, $0.20",
                "1 rewrite mid-session with no cause in the transcripts, $0.15",
            ]
        );
    }

    #[test]
    fn mcp_records_without_a_tool_change_leave_the_bust_to_the_other_changes() {
        let mut fixture = Fixture::new();
        fixture.rebuilt(
            "s1",
            RebuildCause::PrefixChanged,
            0.60,
            60_000,
            &["deferred_tools_record", "instructions"],
        );
        let finding = &CacheBusts.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "edit CLAUDE.md, skills and agents between sessions"
        );
        assert_eq!(
            finding.evidence,
            ["1 change to CLAUDE.md, skills or agents mid-session, $0.60"]
        );
    }

    #[test]
    fn busts_with_no_cause_in_the_transcripts_claim_no_saving() {
        let mut fixture = Fixture::new();
        fixture.rebuilt(
            "s1",
            RebuildCause::PrefixChanged,
            2.15,
            60_000,
            &["deferred_tools_record", "deferred_tools_delta"],
        );
        fixture.rebuilt("s1", RebuildCause::Unknown, 0.50, 60_000, &[]);
        let findings = CacheBusts.evaluate(&fixture.ctx());
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(
            finding.tip,
            "watch for warm caches rewritten mid-session, the transcripts show no cause"
        );
        assert_eq!(finding.saving_usd, None);
        assert_eq!(finding.confidence, Confidence::Low);
        assert_eq!(
            finding.evidence,
            ["2 rewrites mid-session with no cause in the transcripts, $2.65"]
        );
        let mut trivial = Fixture::new();
        trivial.rebuilt("s1", RebuildCause::Unknown, 0.19, 60_000, &[]);
        assert!(CacheBusts.evaluate(&trivial.ctx()).is_empty());
    }

    fn change(kind: &str, names: &[&str], removed: &[&str]) -> PrefixChange {
        PrefixChange {
            ts: 0,
            kind: kind.into(),
            names: names.iter().map(|n| n.to_string()).collect(),
            removed: removed.iter().map(|n| n.to_string()).collect(),
        }
    }

    #[test]
    fn only_a_delta_that_adds_or_removes_mcp_tools_or_servers_changes_them() {
        let changes = [
            change("mcp_instructions_delta", &["linear"], &[]),
            change(
                "deferred_tools_delta",
                &["WebFetch", "mcp__linear__search"],
                &[],
            ),
            change("deferred_tools_record", &["mcp__docs__find"], &[]),
            change("deferred_tools_delta", &["mcp__linear__search"], &[]),
            change("mcp_instructions_delta", &["linear"], &[]),
            change("deferred_tools_delta", &["WebSearch"], &[]),
            change("deferred_tools_delta", &[], &["mcp__linear__search"]),
            change("deferred_tools_delta", &["mcp__linear__search"], &[]),
            change("mcp_instructions_delta", &["docs"], &[]),
            change("mcp_instructions_delta", &[], &["docs"]),
            change("instructions", &["mcp__x__y"], &[]),
        ];
        assert_eq!(
            mcp_changes(&changes),
            [
                true, true, false, false, false, false, true, true, true, true, false
            ]
        );
    }

    #[test]
    fn model_switches_alone_suggest_picking_the_model_up_front() {
        let mut fixture = Fixture::new();
        fixture.rebuilt("s1", RebuildCause::ModelSwitch, 0.80, 60_000, &[]);
        let findings = CacheBusts.evaluate(&fixture.ctx());
        assert_eq!(
            findings[0].tip,
            "pick the model when a session starts rather than switching midway"
        );
        assert!(CacheBusts.evaluate(&Fixture::new().ctx()).is_empty());
    }
}
