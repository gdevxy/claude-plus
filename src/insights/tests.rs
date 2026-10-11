use super::fixture::*;
use super::*;
use crate::transcript::TokenCounts;

fn finding(rule: RuleId, saving_usd: Option<f64>) -> Finding {
    Finding {
        rule,
        tip: String::new(),
        evidence: Vec::new(),
        saving_usd,
        saving_pct_5h: None,
        confidence: Confidence::High,
        how: String::new(),
    }
}

fn draft() -> Draft {
    Draft {
        rule: RuleId::IdleRebuilds,
        tip: "Tip".into(),
        evidence: vec!["line".into()],
        confidence: Confidence::Medium,
        how: "how".into(),
    }
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

#[test]
fn findings_rank_by_weekly_saving_then_rule_with_unpriced_ones_last() {
    let ranked = rank(vec![
        finding(RuleId::Retention, None),
        finding(RuleId::CacheBusts, Some(1.0)),
        finding(RuleId::CostlyTasks, None),
        finding(RuleId::UnusedSkills, Some(4.0)),
        finding(RuleId::IdleRebuilds, Some(1.0)),
    ]);
    let order: Vec<RuleId> = ranked.iter().map(|f| f.rule).collect();
    assert_eq!(
        order,
        [
            RuleId::UnusedSkills,
            RuleId::IdleRebuilds,
            RuleId::CacheBusts,
            RuleId::CostlyTasks,
            RuleId::Retention,
        ]
    );
}

#[test]
fn carried_tokens_cost_what_the_front_of_each_prompt_paid() {
    let mut fixture = Fixture::new();
    fixture.request("s1", SINCE + 10, OPUS, counts(1_000, 99_000, 0, 5_000));
    fixture.request("s1", SINCE + 20, OPUS, counts(0, 0, 100_000, 5_000));
    fixture.request(
        "s1",
        SINCE + 30,
        OPUS,
        TokenCounts {
            input: 50_000,
            cache_read: 4_000,
            cache_write_1h: 3_000,
            cache_write_5m: 2_000,
            output: 5_000,
            ..Default::default()
        },
    );
    fixture.request("s1", SINCE + 40, OPUS, counts(6_000, 0, 0, 5_000));
    fixture.request("s1", SINCE + 50, HAIKU, counts(0, 150_000, 0, 5_000));
    let ctx = fixture.ctx();
    let usd = |n: usize| ctx.carried_usd(&fixture.requests[n], 10_000);
    close(usd(0), 10_000.0 * 0.2 / 1e6);
    close(usd(1), 10_000.0 * 5.0 / 1e6);
    close(
        usd(2),
        (4_000.0 * 0.2 + 3_000.0 * 8.0 + 2_000.0 * 5.0 + 1_000.0 * 4.0) / 1e6,
    );
    close(usd(3), 6_000.0 * 4.0 / 1e6);
    close(usd(4), 10_000.0 * 0.05 / 1e6);
}

#[test]
fn carrying_tokens_costs_them_on_every_main_request_of_the_project() {
    let mut fixture = Fixture::new();
    fixture.session("s1", PROJECT, None);
    fixture.session("s2", "-work-other", None);
    fixture.request("s1", SINCE + 5, OPUS, counts(0, 0, 100_000, 10));
    fixture.request("s1", SINCE + 10, OPUS, counts(0, 100_000, 0, 10));
    fixture.request("s1", SINCE + 20, OPUS, counts(0, 100_000, 0, 10));
    fixture.subagent("s1", SINCE + 30, OPUS, counts(0, 100_000, 0, 10));
    fixture.request("s2", SINCE + 40, OPUS, counts(0, 100_000, 0, 10));
    let ctx = fixture.ctx();
    close(
        ctx.carrying_usd(10_000),
        10_000.0 * 5.0 / 1e6 + 2.0 * 10_000.0 * 0.2 / 1e6,
    );
    assert_eq!(ctx.project_requests(), 3);
    fixture.project = None;
    close(fixture.ctx().carrying_usd(10_000), 0.0);
}

#[test]
fn a_priced_finding_reports_a_weekly_saving_with_its_share_of_a_five_hour_window() {
    let mut fixture = Fixture::new();
    fixture.request("s1", SINCE + 10, OPUS, counts(0, 100_000, 0, 10));
    assert_eq!(
        fixture.ctx().priced(draft(), 3.0),
        Some(Finding {
            rule: RuleId::IdleRebuilds,
            tip: "Tip".into(),
            evidence: vec!["line".into()],
            saving_usd: Some(1.5),
            saving_pct_5h: None,
            confidence: Confidence::Medium,
            how: "how".into(),
        })
    );
    fixture.calibrated(2.0);
    let priced = fixture.ctx().priced(draft(), 3.0).unwrap();
    assert_eq!(priced.saving_pct_5h, Some(3.0));
    assert_eq!(fixture.ctx().priced(draft(), 0.19), None);
}

#[test]
fn the_dominant_model_is_the_one_the_main_thread_spent_most_on() {
    let mut fixture = Fixture::new();
    assert_eq!(fixture.ctx().dominant_model(), "");
    fixture.request("s1", SINCE + 10, SONNET, counts(100_000, 0, 0, 10));
    fixture.request("s1", SINCE + 20, OPUS, counts(10_000, 0, 0, 10));
    fixture.subagent("s1", SINCE + 30, OPUS, counts(900_000, 0, 0, 10));
    assert_eq!(fixture.ctx().dominant_model(), SONNET);
}

#[test]
fn long_evidence_lists_show_three_lines_and_count_the_rest() {
    let five = [1, 2, 3, 4, 5];
    assert_eq!(
        listed(&five, |n| format!("item {n}"), "items"),
        ["item 1", "item 2", "item 3", "and 2\u{a0}more items"]
    );
    assert_eq!(
        listed(&five[..3], |n| n.to_string(), "items"),
        ["1", "2", "3"]
    );
}

#[test]
fn sessions_without_a_title_are_named_by_their_id() {
    let mut fixture = Fixture::new();
    fixture.session("abcdef1234", PROJECT, None);
    fixture.session("s2", PROJECT, Some("Fix the login flow"));
    let ctx = fixture.ctx();
    assert_eq!(ctx.title("abcdef1234"), "Untitled abcdef12");
    assert_eq!(ctx.title("s2"), "Fix the login flow");
}

#[test]
fn the_median_of_an_even_count_averages_the_middle_pair() {
    assert_eq!(median(vec![]), None);
    assert_eq!(median(vec![5.0, 1.0, 3.0]), Some(3.0));
    assert_eq!(median(vec![4.0, 1.0, 3.0, 2.0]), Some(2.5));
}
