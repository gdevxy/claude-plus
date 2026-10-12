use std::collections::BTreeMap;

use super::usage::TOP_TIER;
use super::words::{duration, model_name, times};
use super::{Confidence, Draft, Finding, InsightContext, Rule, RuleId, is_main, median};
use crate::cache_model::ONE_HOUR_MS;
use crate::forecast::{alternative_for, family};
use crate::history::FIVE_HOUR;
use crate::limits::instances;
use crate::pricing::normalize_model;
use crate::store::LimitSample;

pub const SEVEN_DAY: &str = "seven_day";
pub const CAPPED_PCT: f64 = 100.0;
pub const SUBAGENT_LED: f64 = 0.5;
const DAY_MS: i64 = 24 * ONE_HOUR_MS;

pub struct LimitHits;

struct Hit {
    window: &'static str,
    start_ms: i64,
    capped_ms: i64,
    waited_ms: i64,
}

impl Rule for LimitHits {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        let hits: Vec<Hit> = instances(ctx.limit_samples)
            .into_iter()
            .filter_map(|instance| hit(ctx, instance))
            .collect();
        if hits.is_empty() {
            return Vec::new();
        }
        let mut evidence = Vec::new();
        for (window, name) in [(FIVE_HOUR, "5-hour"), (SEVEN_DAY, "weekly")] {
            let waits: Vec<f64> = hits
                .iter()
                .filter(|h| h.window == window)
                .map(|h| h.waited_ms as f64)
                .collect();
            let ran_out = waits.len() as u64;
            let Some(wait) = median(waits) else {
                continue;
            };
            evidence.push(format!(
                "The {name} limit ran out {}, {}{} before it reset",
                times(ran_out),
                if ran_out > 1 { "a typical " } else { "" },
                duration(wait as i64)
            ));
        }
        let mut by_model: BTreeMap<&str, f64> = BTreeMap::new();
        let mut subagents_by_model: BTreeMap<&str, f64> = BTreeMap::new();
        let (mut spent, mut subagents) = (0.0, 0.0);
        for request in ctx.requests {
            let at_ms = request.point.ts * 1000;
            if !hits
                .iter()
                .any(|h| h.start_ms <= at_ms && at_ms <= h.capped_ms)
            {
                continue;
            }
            let paid = request.cost_usd.unwrap_or(0.0);
            let model = normalize_model(&request.point.model);
            spent += paid;
            *by_model.entry(model).or_default() += paid;
            if !is_main(request) {
                subagents += paid;
                *subagents_by_model.entry(model).or_default() += paid;
            }
        }
        let model = leading(&by_model);
        if let Some((model, usd)) = model.filter(|_| spent > 0.0) {
            evidence.push(format!(
                "{} drew {:.0}% of the spend before it ran out",
                model_name(model),
                usd / spent * 100.0
            ));
        }
        if spent > 0.0 && subagents > 0.0 {
            evidence.push(format!(
                "Subagents drew {:.0}% of it",
                subagents / spent * 100.0
            ));
        }
        subagents_by_model.retain(|model, _| TOP_TIER.contains(&family(model)));
        let subagents_top: f64 = subagents_by_model.values().sum();
        let subagent_cheaper = leading(&subagents_by_model)
            .filter(|_| spent > 0.0 && subagents_top / spent >= SUBAGENT_LED)
            .and_then(|(model, _)| alternative_for(model));
        let cheaper = model
            .map(|(model, _)| model)
            .filter(|model| TOP_TIER.contains(&family(model)))
            .and_then(alternative_for);
        let tip = match (subagent_cheaper, cheaper) {
            (Some(alternative), _) => format!(
                "run subagents on {} so your limits last",
                model_name(alternative)
            ),
            (None, Some(alternative)) => format!(
                "move routine work to {} so your limits last",
                model_name(alternative)
            ),
            (None, None) if hits.iter().any(|h| h.window == SEVEN_DAY) => {
                "pace heavy work across the week so the weekly limit lasts".into()
            }
            (None, None) => "spread heavy work across 5-hour windows so the limit lasts".into(),
        };
        vec![
            Draft {
                rule: RuleId::LimitHits,
                tip,
                evidence,
                confidence: Confidence::Medium,
                how: "Reads the 5-hour and weekly limits Claude Code reported for this account in the last 14 days. \
                      A limit ran out when it reached 100%, and the wait runs from then to its reset. \
                      The spend is what this account's requests cost at API prices in each window before its limit ran out, \
                      with the window's start estimated from its reset time. \
                      A limit that ran out stops your work, so this tip comes first."
                    .into(),
            }
            .into_finding(),
        ]
    }
}

fn hit(ctx: &InsightContext, instance: &[LimitSample]) -> Option<Hit> {
    let first = instance.first()?;
    let window = [FIVE_HOUR, SEVEN_DAY]
        .into_iter()
        .find(|window| *window == first.window)?;
    let capped = instance.iter().find(|s| s.used_pct >= CAPPED_PCT)?;
    let resets = instance.iter().rev().find_map(|s| s.resets_at_ms)?;
    (capped.ts_ms >= ctx.since_ts * 1000).then(|| Hit {
        window,
        start_ms: resets - length(window),
        capped_ms: capped.ts_ms,
        waited_ms: (resets.min(ctx.now_ms) - capped.ts_ms).max(0),
    })
}

fn leading<'a>(by_model: &BTreeMap<&'a str, f64>) -> Option<(&'a str, f64)> {
    by_model
        .iter()
        .max_by(|a, b| a.1.total_cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(model, usd)| (*model, *usd))
}

fn length(window: &str) -> i64 {
    match window {
        SEVEN_DAY => 7 * DAY_MS,
        _ => 5 * ONE_HOUR_MS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::fixture::*;

    const HOUR: i64 = 3_600;
    const DAY: i64 = 24 * HOUR;

    #[test]
    fn five_hour_limits_that_ran_out_name_the_wait_the_model_and_the_subagents_behind_them() {
        let mut fixture = Fixture::new();
        fixture.limit(FIVE_HOUR, SINCE + 5 * HOUR + 60, 40.0, SINCE + 10 * HOUR);
        fixture.limit(FIVE_HOUR, SINCE + 7 * HOUR, 100.0, SINCE + 10 * HOUR);
        fixture.limit(
            FIVE_HOUR,
            SINCE + 2 * DAY + HOUR,
            30.0,
            SINCE + 2 * DAY + 5 * HOUR,
        );
        fixture.limit(
            FIVE_HOUR,
            SINCE + 2 * DAY + 4 * HOUR,
            100.0,
            SINCE + 2 * DAY + 5 * HOUR,
        );
        fixture.subagent("s1", SINCE + 6 * HOUR, OPUS, counts(100_000, 0, 0, 0));
        fixture.request("s1", SINCE + 6 * HOUR + 60, OPUS, counts(50_000, 0, 0, 0));
        fixture.subagent(
            "s1",
            SINCE + 2 * DAY + 2 * HOUR,
            OPUS,
            counts(100_000, 0, 0, 0),
        );
        fixture.request("s1", SINCE + DAY, SONNET, counts(1_000_000, 0, 0, 0));
        let findings = LimitHits.evaluate(&fixture.ctx());
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(finding.rule, RuleId::LimitHits);
        assert_eq!(
            finding.tip,
            "run subagents on Sonnet\u{a0}5.5 so your limits last"
        );
        assert_eq!(finding.saving_usd, None);
        assert_eq!(finding.confidence, Confidence::Medium);
        assert_eq!(
            finding.evidence,
            [
                "The 5-hour limit ran out twice, a typical 2h before it reset",
                "Opus\u{a0}5.5 drew 100% of the spend before it ran out",
                "Subagents drew 80% of it",
            ]
        );
    }

    #[test]
    fn a_weekly_limit_spent_on_a_cheaper_model_suggests_pacing_the_week() {
        let mut fixture = Fixture::new();
        fixture.limit(SEVEN_DAY, SINCE + 8 * DAY, 90.0, SINCE + 10 * DAY);
        fixture.limit(SEVEN_DAY, SINCE + 9 * DAY, 100.0, SINCE + 10 * DAY);
        fixture.request("s1", SINCE + 8 * DAY + 10, SONNET, counts(100_000, 0, 0, 0));
        let finding = &LimitHits.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "pace heavy work across the week so the weekly limit lasts"
        );
        assert_eq!(
            finding.evidence,
            [
                "The weekly limit ran out once, 1d before it reset",
                "Sonnet\u{a0}5.5 drew 100% of the spend before it ran out",
            ]
        );
    }

    #[test]
    fn a_limit_still_out_counts_its_wait_until_now() {
        let mut fixture = Fixture::new();
        fixture.limit(FIVE_HOUR, NOW_S - 2 * HOUR, 80.0, NOW_S + 2 * HOUR);
        fixture.limit(FIVE_HOUR, NOW_S - HOUR, 100.0, NOW_S + 2 * HOUR);
        fixture.request("s1", NOW_S - 90 * 60, OPUS, counts(100_000, 0, 0, 0));
        let finding = &LimitHits.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "move routine work to Sonnet\u{a0}5.5 so your limits last"
        );
        assert_eq!(
            finding.evidence[0],
            "The 5-hour limit ran out once, 1h before it reset"
        );
    }

    #[test]
    fn subagents_that_drew_the_limit_down_on_a_cheaper_model_leave_the_tip_to_the_main_thread() {
        let mut fixture = Fixture::new();
        fixture.limit(FIVE_HOUR, SINCE + 7 * HOUR, 100.0, SINCE + 10 * HOUR);
        let spend = [
            (None, OPUS, 45.0),
            (Some("a1"), OPUS, 10.0),
            (Some("a2"), SONNET, 45.0),
        ];
        for (offset, (agent, model, usd)) in (0..).zip(spend) {
            match agent {
                Some(agent) => {
                    fixture.push_agent(agent, SINCE + 6 * HOUR + offset, model, counts(1, 0, 0, 0))
                }
                None => fixture.request("s1", SINCE + 6 * HOUR + offset, model, counts(1, 0, 0, 0)),
            }
            fixture.requests.last_mut().unwrap().cost_usd = Some(usd);
        }
        let finding = &LimitHits.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "move routine work to Sonnet\u{a0}5.5 so your limits last"
        );
        assert_eq!(
            finding.evidence[1..],
            [
                "Opus\u{a0}5.5 drew 55% of the spend before it ran out",
                "Subagents drew 55% of it",
            ]
        );
    }

    #[test]
    fn the_subagent_tip_names_the_model_the_subagents_drew_on() {
        let mut fixture = Fixture::new();
        fixture.limit(FIVE_HOUR, SINCE + 7 * HOUR, 100.0, SINCE + 10 * HOUR);
        let spend = [
            (None, OPUS, 40.0),
            (Some("a1"), "claude-fable-5-1", 35.0),
            (Some("a2"), "claude-opus-5-5-20261001", 25.0),
        ];
        for (offset, (agent, model, usd)) in (0..).zip(spend) {
            match agent {
                Some(agent) => {
                    fixture.push_agent(agent, SINCE + 6 * HOUR + offset, model, counts(1, 0, 0, 0))
                }
                None => fixture.request("s1", SINCE + 6 * HOUR + offset, model, counts(1, 0, 0, 0)),
            }
            fixture.requests.last_mut().unwrap().cost_usd = Some(usd);
        }
        let finding = &LimitHits.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "run subagents on Opus\u{a0}5.5 so your limits last"
        );
        assert_eq!(
            finding.evidence[1],
            "Opus\u{a0}5.5 drew 65% of the spend before it ran out"
        );
    }

    #[test]
    fn both_windows_out_on_a_cheaper_model_suggest_pacing_the_week() {
        let mut fixture = Fixture::new();
        fixture.limit(FIVE_HOUR, SINCE + 7 * HOUR, 100.0, SINCE + 10 * HOUR);
        fixture.limit(SEVEN_DAY, SINCE + 9 * DAY, 100.0, SINCE + 10 * DAY);
        fixture.request("s1", SINCE + 6 * HOUR, SONNET, counts(100_000, 0, 0, 0));
        let finding = &LimitHits.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "pace heavy work across the week so the weekly limit lasts"
        );
    }

    #[test]
    fn limits_that_never_ran_out_in_the_window_give_no_tip() {
        let mut fixture = Fixture::new();
        fixture.limit(FIVE_HOUR, SINCE - HOUR, 100.0, SINCE + HOUR);
        fixture.limit(FIVE_HOUR, SINCE + DAY, 99.0, SINCE + DAY + HOUR);
        fixture.limit("seven_day_opus", SINCE + 2 * DAY, 100.0, SINCE + 3 * DAY);
        fixture.limit(FIVE_HOUR, SINCE + 3 * DAY, 100.0, SINCE + 3 * DAY);
        fixture.limit_samples.last_mut().unwrap().resets_at_ms = None;
        assert!(LimitHits.evaluate(&fixture.ctx()).is_empty());
        assert!(LimitHits.evaluate(&Fixture::new().ctx()).is_empty());
    }
}
