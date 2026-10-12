use std::collections::{BTreeMap, BTreeSet};

use super::usage::TOP_TIER;
use super::words::{count, model_name, money};
use super::{Confidence, Draft, Finding, InsightContext, Rule, RuleId, is_main, listed};
use crate::forecast::{alternative_for, family};
use crate::pricing::normalize_model;

pub const READ_ONLY_AGENTS: &[&str] = &["Explore", "Plan", "claude-code-guide", "statusline-setup"];
pub const UNKNOWN_AGENT: &str = "untyped";
pub const FORK_AGENT: &str = "fork";

pub struct SubagentModelFit;

#[derive(Default)]
struct Spent<'a> {
    runs: BTreeSet<&'a str>,
    paid: f64,
    cheaper: f64,
    by_model: BTreeMap<&'a str, f64>,
}

impl Spent<'_> {
    fn saving(&self) -> f64 {
        self.paid - self.cheaper
    }

    fn model(&self) -> &str {
        self.by_model
            .iter()
            .max_by(|a, b| a.1.total_cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map_or("", |(model, _)| model)
    }
}

impl Rule for SubagentModelFit {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        let mut by_type: BTreeMap<&str, Spent> = BTreeMap::new();
        let (mut all, mut subagents) = (0.0, 0.0);
        for request in ctx.requests {
            let paid = request.cost_usd.unwrap_or(0.0);
            all += paid;
            if is_main(request) {
                continue;
            }
            subagents += paid;
            let Some(agent) = request.point.agent_id.as_deref() else {
                continue;
            };
            let model = normalize_model(&request.point.model);
            if !TOP_TIER.contains(&family(model)) {
                continue;
            }
            let kind = ctx
                .agent_types
                .get(agent)
                .map_or(UNKNOWN_AGENT, String::as_str);
            if kind == FORK_AGENT {
                continue;
            }
            let Some(cheaper) = alternative_for(model)
                .and_then(|alternative| ctx.prices.cost(alternative, &request.point.tokens))
            else {
                continue;
            };
            let spent = by_type.entry(kind).or_default();
            spent.runs.insert(agent);
            spent.paid += paid;
            spent.cheaper += cheaper;
            *spent.by_model.entry(model).or_default() += paid;
        }
        let mut ordered: Vec<(&str, Spent)> = by_type.into_iter().collect();
        ordered.sort_by(|a, b| b.1.saving().total_cmp(&a.1.saving()).then(a.0.cmp(b.0)));
        let Some((top, spent)) = ordered.first() else {
            return Vec::new();
        };
        let alternative = alternative_for(spent.model()).unwrap_or_default();
        let mut evidence = listed(
            &ordered,
            |(kind, spent)| {
                let runs = count(spent.runs.len() as u64, "run", "runs");
                let (paid, cheaper) = (money(spent.paid), money(spent.cheaper));
                match spent.by_model.len() {
                    1 => {
                        let model = spent.model();
                        format!(
                            "{kind}: {runs} on {} cost {paid}, {cheaper} on {}",
                            model_name(model),
                            model_name(alternative_for(model).unwrap_or_default())
                        )
                    }
                    _ => format!("{kind}: {runs} cost {paid}, {cheaper} on cheaper models"),
                }
            },
            "agent types",
        );
        if all > 0.0 {
            evidence.push(format!(
                "Subagents drew {:.0}% of your spend",
                subagents / all * 100.0
            ));
        }
        let draft = Draft {
            rule: RuleId::SubagentModelFit,
            tip: match *top {
                UNKNOWN_AGENT => format!("run subagents on {}", model_name(alternative)),
                kind => format!("run {kind} subagents on {}", model_name(alternative)),
            },
            evidence,
            confidence: if READ_ONLY_AGENTS.contains(top) {
                Confidence::Medium
            } else {
                Confidence::Low
            },
            how: "Reprices the subagent requests of the last 14 days that ran on Opus, Fable or Mythos \
                  on the next model down at the same token counts, by the agent type each subagent's .meta.json names. \
                  A cheaper model may take more turns or do worse on hard work, so check its results first. \
                  Set model: in the agent's file under .claude/agents, add your own agent named Explore to replace the built-in one, \
                  or set CLAUDE_CODE_SUBAGENT_MODEL for the agents that name no model."
                .into(),
        };
        let saving = ordered.iter().map(|(_, spent)| spent.saving()).sum();
        ctx.priced(draft, saving).into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::fixture::*;

    fn close(actual: Option<f64>, expected: f64) {
        let actual = actual.unwrap();
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    #[test]
    fn top_tier_subagents_are_repriced_by_agent_type_and_the_costliest_type_names_the_tip() {
        let mut fixture = Fixture::new();
        fixture.typed(
            "a1",
            "Explore",
            SINCE + 10,
            OPUS,
            counts(100_000, 0, 0, 1_000),
        );
        fixture.typed(
            "a1",
            "Explore",
            SINCE + 20,
            OPUS,
            counts(100_000, 0, 0, 1_000),
        );
        fixture.typed(
            "a2",
            "Explore",
            SINCE + 30,
            OPUS,
            counts(100_000, 0, 0, 1_000),
        );
        fixture.typed(
            "a3",
            "reviewer",
            SINCE + 40,
            OPUS,
            counts(50_000, 0, 0, 1_000),
        );
        fixture.typed(
            "a4",
            "Explore",
            SINCE + 50,
            SONNET,
            counts(100_000, 0, 0, 1_000),
        );
        fixture.push_agent("a5", SINCE + 60, OPUS, counts(10_000, 0, 0, 0));
        fixture.request("s1", SINCE + 70, OPUS, counts(100_000, 0, 0, 1_000));
        let findings = SubagentModelFit.evaluate(&fixture.ctx());
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(finding.rule, RuleId::SubagentModelFit);
        assert_eq!(finding.tip, "run Explore subagents on Sonnet\u{a0}5.5");
        assert_eq!(finding.confidence, Confidence::Medium);
        close(finding.saving_usd, (0.63 + 0.11 + 0.02) / 2.0);
        assert_eq!(
            finding.evidence,
            [
                "Explore: 2\u{a0}runs on Opus\u{a0}5.5 cost $1.26, $0.63 on Sonnet\u{a0}5.5",
                "reviewer: 1\u{a0}run on Opus\u{a0}5.5 cost $0.22, $0.11 on Sonnet\u{a0}5.5",
                "untyped: 1\u{a0}run on Opus\u{a0}5.5 cost $0.04, $0.02 on Sonnet\u{a0}5.5",
                "Subagents drew 80% of your spend",
            ]
        );
    }

    #[test]
    fn agents_that_write_code_get_a_low_confidence_tip_and_untyped_ones_a_generic_one() {
        let mut fixture = Fixture::new();
        fixture.typed(
            "a1",
            "general-purpose",
            SINCE,
            OPUS,
            counts(100_000, 0, 0, 1_000),
        );
        let finding = &SubagentModelFit.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "run general-purpose subagents on Sonnet\u{a0}5.5"
        );
        assert_eq!(finding.confidence, Confidence::Low);
        let mut untyped = Fixture::new();
        untyped.push_agent("a9", SINCE, OPUS, counts(100_000, 0, 0, 1_000));
        let finding = &SubagentModelFit.evaluate(&untyped.ctx())[0];
        assert_eq!(finding.tip, "run subagents on Sonnet\u{a0}5.5");
        assert_eq!(finding.confidence, Confidence::Low);
    }

    #[test]
    fn a_type_on_several_top_tier_models_is_priced_without_naming_one_alternative() {
        let mut fixture = Fixture::new();
        let input = counts(100_000, 0, 0, 0);
        fixture.typed("a1", "reviewer", SINCE, "claude-fable-5-1", input);
        fixture.typed("a2", "reviewer", SINCE + 10, OPUS, input);
        fixture.typed("a3", "Explore", SINCE + 20, OPUS, input);
        fixture.typed(
            "a3",
            "Explore",
            SINCE + 30,
            "claude-opus-5-5-20261001",
            input,
        );
        fixture.typed("f1", "fork", SINCE + 40, OPUS, counts(1_000_000, 0, 0, 0));
        fixture.request("s1", SINCE + 50, SONNET, input);
        fixture.requests.last_mut().unwrap().point.is_sidechain = true;
        fixture.request("s1", SINCE + 60, OPUS, input);
        let finding = &SubagentModelFit.evaluate(&fixture.ctx())[0];
        assert_eq!(finding.tip, "run reviewer subagents on Opus\u{a0}5.5");
        assert_eq!(finding.confidence, Confidence::Low);
        close(finding.saving_usd, (0.8 + 0.4) / 2.0);
        assert_eq!(
            finding.evidence,
            [
                "reviewer: 2\u{a0}runs cost $1.40, $0.60 on cheaper models",
                "Explore: 1\u{a0}run on Opus\u{a0}5.5 cost $0.80, $0.40 on Sonnet\u{a0}5.5",
                "Subagents drew 94% of your spend",
            ]
        );
    }

    #[test]
    fn fork_subagents_are_never_repriced() {
        let mut fixture = Fixture::new();
        fixture.typed("f1", "fork", SINCE, OPUS, counts(1_000_000, 0, 0, 1_000));
        assert!(SubagentModelFit.evaluate(&fixture.ctx()).is_empty());
    }

    #[test]
    fn subagents_already_on_a_cheaper_model_or_spending_pennies_give_no_tip() {
        let mut fixture = Fixture::new();
        fixture.typed(
            "a1",
            "Explore",
            SINCE,
            SONNET,
            counts(1_000_000, 0, 0, 1_000),
        );
        fixture.request("s1", SINCE + 10, OPUS, counts(1_000_000, 0, 0, 1_000));
        assert!(SubagentModelFit.evaluate(&fixture.ctx()).is_empty());
        fixture.typed("a2", "Explore", SINCE + 20, OPUS, counts(90_000, 0, 0, 0));
        assert!(SubagentModelFit.evaluate(&fixture.ctx()).is_empty());
        assert!(SubagentModelFit.evaluate(&Fixture::new().ctx()).is_empty());
    }
}
