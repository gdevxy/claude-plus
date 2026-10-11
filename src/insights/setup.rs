use std::collections::BTreeMap;

use super::words::{count, times, tokens};
use super::{Confidence, Draft, Finding, InsightContext, Rule, RuleId, listed};
use crate::context_model::{Item, ItemKind, TextKind};
use crate::paths::Paths;

pub const DUPLICATE_MIN_CHARS: usize = 24;
pub const LARGE_MEMORY_CHARS: u64 = 40_000;
pub const WANTED_RETENTION_DAYS: u64 = 90;

pub struct UnusedMcpServers;

impl Rule for UnusedMcpServers {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        unused(ctx, ItemKind::McpServer, |items| match items {
            [one] => format!("turn off the {} MCP server, unused for 14 days", one.name),
            _ => format!(
                "turn off {} you have not used in 14 days",
                count(items.len() as u64, "MCP server", "MCP servers")
            ),
        })
        .into_iter()
        .collect()
    }
}

pub struct UnusedSkills;

impl Rule for UnusedSkills {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        unused(ctx, ItemKind::Skill, |items| match items {
            [one] => format!("retire the {} skill, unused for 14 days", one.name),
            _ => format!(
                "retire {} you have not used in 14 days",
                count(items.len() as u64, "skill", "skills")
            ),
        })
        .into_iter()
        .collect()
    }
}

fn unused(
    ctx: &InsightContext,
    kind: ItemKind,
    tip: impl Fn(&[&Item]) -> String,
) -> Option<Finding> {
    let items: Vec<&Item> = ctx.dead_weight.iter().filter(|i| i.kind == kind).collect();
    if items.is_empty() {
        return None;
    }
    let carried: u64 = items.iter().map(|i| i.tokens).sum();
    let month = |item: &Item| {
        ctx.month_items
            .iter()
            .find(|m| m.kind == item.kind && m.name == item.name)
            .and_then(|m| m.uses_14d)
            .unwrap_or(0)
    };
    let mut evidence = listed(
        &items,
        |item| match month(item) {
            0 => format!(
                "{}: ~{} tokens per request, unused for 30 days",
                item.name,
                tokens(item.tokens)
            ),
            uses => format!(
                "{}: ~{} tokens per request, used {} in 30 days",
                item.name,
                tokens(item.tokens),
                times(uses)
            ),
        },
        if kind == ItemKind::Skill {
            "skills"
        } else {
            "servers"
        },
    );
    evidence.push(format!(
        "Carried on {} in this project over 14 days",
        count(ctx.project_requests() as u64, "request", "requests")
    ));
    let measured = items.iter().all(|i| !i.estimated);
    let draft = Draft {
        rule: match kind {
            ItemKind::Skill => RuleId::UnusedSkills,
            _ => RuleId::UnusedMcpServers,
        },
        tip: tip(&items),
        evidence,
        confidence: if measured {
            Confidence::High
        } else {
            Confidence::Medium
        },
        how: format!(
            "{} Every main-thread request in this project carries {}, and the saving prices them at what each request paid for the front of its prompt: a cache read while the cache was warm, a cache write when it was rewritten. {}",
            if measured {
                "Token sizes come from Claude Code's own /context breakdown."
            } else {
                "Token sizes are estimated from the config files until /context runs."
            },
            if kind == ItemKind::Skill {
                "skill listings"
            } else {
                "MCP tool definitions"
            },
            if kind == ItemKind::Skill {
                "Disable the plugin that brings a skill or move the skill folder away."
            } else {
                "Turn a server off with /mcp or remove it from its .mcp.json."
            }
        ),
    };
    ctx.priced(draft, ctx.carrying_usd(carried))
}

pub struct OversizedMemory;

impl Rule for OversizedMemory {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        let mut seen: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for memory in ctx.memory {
            for line in memory.text.lines().map(str::trim) {
                if line.chars().count() >= DUPLICATE_MIN_CHARS {
                    seen.entry(line).or_default().push(&memory.name);
                }
            }
        }
        let repeats: Vec<(&str, usize)> = seen
            .iter()
            .filter(|(_, owners)| owners.len() > 1)
            .map(|(line, owners)| (*line, owners.len() - 1))
            .collect();
        let repeated_chars: u64 = repeats
            .iter()
            .map(|(line, extra)| (line.chars().count() * extra) as u64)
            .sum();
        let mut sized: Vec<(&str, u64)> = ctx
            .memory
            .iter()
            .map(|m| (m.name.as_str(), m.text.chars().count() as u64))
            .collect();
        sized.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        let excess: u64 = sized
            .iter()
            .map(|(_, chars)| chars.saturating_sub(LARGE_MEMORY_CHARS))
            .sum();
        if repeated_chars == 0 && excess == 0 {
            return Vec::new();
        }
        let prose = |chars: u64| ctx.ratios.tokens(chars, TextKind::Prose);
        let mut evidence = listed(
            &sized,
            |(name, chars)| format!("{name}: ~{} tokens on every request", tokens(prose(*chars))),
            "files",
        );
        let lines: usize = repeats.iter().map(|(_, extra)| extra).sum();
        if lines > 0 {
            evidence.push(format!(
                "{} repeated in your memory files, ~{} tokens",
                count(lines as u64, "line", "lines"),
                tokens(prose(repeated_chars))
            ));
        }
        let tip = match (lines, sized.first()) {
            (0, Some((name, _))) => format!("trim {name}, it rides on every request"),
            _ => format!(
                "remove {} from your memory files",
                count(lines as u64, "repeated line", "repeated lines")
            ),
        };
        let draft = Draft {
            rule: RuleId::OversizedMemory,
            tip,
            evidence,
            confidence: Confidence::Medium,
            how: format!(
                "Reads the CLAUDE.md and memory files Claude Code loads into every request in this project. \
                 The saving counts lines of {DUPLICATE_MIN_CHARS} characters or more that appear in more than one place \
                 and anything past 40,000 characters in a file, priced on every main-thread request of the last 14 days."
            ),
        };
        ctx.priced(draft, ctx.carrying_usd(prose(repeated_chars + excess)))
            .into_iter()
            .collect()
    }
}

pub struct Retention;

impl Rule for Retention {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        let days = ctx.retention_days.unwrap_or(Paths::DEFAULT_RETENTION_DAYS);
        if days >= WANTED_RETENTION_DAYS {
            return Vec::new();
        }
        let setting = match ctx.retention_days {
            Some(days) => format!("cleanupPeriodDays is {days}"),
            None => format!("cleanupPeriodDays is {days}, the default"),
        };
        vec![
            Draft {
                rule: RuleId::Retention,
                tip: format!("keep {WANTED_RETENTION_DAYS} days of transcripts to match the History range"),
                evidence: vec![
                    setting,
                    "Claude Code deletes older transcripts, so /resume and a fresh import cannot reach them".into(),
                ],
                confidence: Confidence::High,
                how: format!(
                    "Add \"cleanupPeriodDays\": {WANTED_RETENTION_DAYS} to ~/.claude/settings.json. \
                     claude-plus keeps its own copy of what it has read either way."
                ),
            }
            .into_finding(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::fixture::*;

    const OPUS_READ: f64 = 0.2 / 1e6;
    const OPUS_WRITE_5M: f64 = 5.0 / 1e6;

    fn busy(fixture: &mut Fixture) {
        fixture.session("s1", PROJECT, None);
        fixture.request("s1", SINCE, OPUS, counts(0, 0, 100_000, 10));
        for n in 1..1_000 {
            fixture.request("s1", SINCE + n, OPUS, counts(0, 100_000, 0, 10));
        }
    }

    fn carried_on_busy(tokens: f64) -> f64 {
        tokens * OPUS_WRITE_5M + 999.0 * tokens * OPUS_READ
    }

    #[test]
    fn unused_mcp_servers_are_priced_on_every_request_of_the_project() {
        let mut fixture = Fixture::new();
        busy(&mut fixture);
        fixture.dead_weight = vec![
            item(ItemKind::McpServer, "github", 40_000, false, 0),
            item(ItemKind::McpServer, "linear", 20_000, false, 0),
            item(ItemKind::Skill, "deploy", 9_000, false, 0),
        ];
        fixture.month_items = vec![item(ItemKind::McpServer, "linear", 20_000, false, 2)];
        let findings = UnusedMcpServers.evaluate(&fixture.ctx());
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(
            finding.tip,
            "turn off 2 MCP servers you have not used in 14 days"
        );
        assert_eq!(finding.confidence, Confidence::High);
        assert!((finding.saving_usd.unwrap() - carried_on_busy(60_000.0) / 2.0).abs() < 1e-9);
        assert_eq!(
            finding.evidence,
            [
                "github: ~40k tokens per request, unused for 30 days",
                "linear: ~20k tokens per request, used twice in 30 days",
                "Carried on 1,000 requests in this project over 14 days",
            ]
        );
    }

    #[test]
    fn one_estimated_skill_is_named_with_medium_confidence() {
        let mut fixture = Fixture::new();
        busy(&mut fixture);
        fixture.dead_weight = vec![item(ItemKind::Skill, "deploy", 9_000, true, 0)];
        let finding = &UnusedSkills.evaluate(&fixture.ctx())[0];
        assert_eq!(finding.tip, "retire the deploy skill, unused for 14 days");
        assert_eq!(finding.confidence, Confidence::Medium);
        assert!(UnusedMcpServers.evaluate(&fixture.ctx()).is_empty());
    }

    #[test]
    fn dead_weight_carried_by_too_few_requests_is_not_worth_a_tip() {
        let mut fixture = Fixture::new();
        fixture.session("s1", PROJECT, None);
        fixture.request("s1", SINCE + 1, OPUS, counts(0, 100_000, 0, 10));
        fixture.dead_weight = vec![item(ItemKind::Skill, "deploy", 9_000, false, 0)];
        assert!(UnusedSkills.evaluate(&fixture.ctx()).is_empty());
    }

    #[test]
    fn repeated_memory_lines_and_oversized_files_are_priced_as_prose() {
        let mut fixture = Fixture::new();
        busy(&mut fixture);
        let rule = "Always run cargo fmt before committing any change.";
        let long = "x".repeat(LARGE_MEMORY_CHARS as usize + 10_500);
        let project = format!("# Rules\n{rule}\nshort line\n{long}");
        let excess = project.len() as u64 - LARGE_MEMORY_CHARS;
        fixture.memory = vec![
            memory("CLAUDE.md", &project),
            memory(
                "~/.claude/CLAUDE.md",
                &format!("{rule}\n  {rule}  \nshort line\n"),
            ),
        ];
        let finding = &OversizedMemory.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "remove 2 repeated lines from your memory files"
        );
        assert_eq!(
            finding.evidence,
            [
                "CLAUDE.md: ~14.4k tokens on every request",
                "~/.claude/CLAUDE.md: ~33 tokens on every request",
                "2 lines repeated in your memory files, ~29 tokens",
            ]
        );
        let carried = fixture
            .ratios
            .tokens(2 * rule.len() as u64 + excess, TextKind::Prose);
        let expected = carried_on_busy(carried as f64) / 2.0;
        assert!((finding.saving_usd.unwrap() - expected).abs() < 1e-9);
    }

    #[test]
    fn a_large_file_without_repeats_suggests_trimming_it() {
        let mut fixture = Fixture::new();
        busy(&mut fixture);
        fixture.memory = vec![memory("CLAUDE.md", &"y".repeat(400_000))];
        let finding = &OversizedMemory.evaluate(&fixture.ctx())[0];
        assert_eq!(finding.tip, "trim CLAUDE.md, it rides on every request");
        fixture.memory = vec![memory("CLAUDE.md", "small and unique")];
        assert!(OversizedMemory.evaluate(&fixture.ctx()).is_empty());
    }

    #[test]
    fn retention_shorter_than_ninety_days_suggests_keeping_more() {
        let mut fixture = Fixture::new();
        assert!(Retention.evaluate(&fixture.ctx()).is_empty());
        fixture.retention_days = None;
        let finding = &Retention.evaluate(&fixture.ctx())[0];
        assert_eq!(finding.saving_usd, None);
        assert_eq!(finding.confidence, Confidence::High);
        assert_eq!(
            finding.tip,
            "keep 90 days of transcripts to match the History range"
        );
        assert_eq!(finding.evidence[0], "cleanupPeriodDays is 30, the default");
        fixture.retention_days = Some(45);
        assert_eq!(
            Retention.evaluate(&fixture.ctx())[0].evidence[0],
            "cleanupPeriodDays is 45"
        );
    }
}
