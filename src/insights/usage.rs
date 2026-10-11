use std::collections::BTreeMap;

use super::words::{count, grouped, model_name, money, tokens};
use super::{Confidence, Draft, Finding, InsightContext, Rule, RuleId, is_main, listed, median};
use crate::context_model::TextKind;
use crate::forecast::{alternative_for, family};
use crate::store::{LargeResult, SpentRequest};

pub const LARGE_RESULT_CHARS: u64 = 20_000;
pub const KEPT_RESULT_CHARS: u64 = 10_000;
pub const LATE_PROMPT_TOKENS: u64 = 100_000;
pub const MIN_LATE_TURNS: usize = 10;
pub const REGROWTH: f64 = 0.5;
pub const QUICK_REQUESTS: usize = 3;
pub const QUICK_OUTPUT_TOKENS: u64 = 1_000;
pub const TOP_TIER: &[&str] = &["opus", "fable", "mythos"];
pub const MIN_TASKS: usize = 10;
pub const OUTLIER_FACTOR: f64 = 5.0;
pub const MIN_OUTLIER_USD: f64 = 1.0;
pub const TARGET_CHARS: usize = 30;

pub struct Task<'a> {
    pub session_id: &'a str,
    pub requests: Vec<&'a SpentRequest>,
}

impl Task<'_> {
    fn main(&self) -> impl Iterator<Item = &&SpentRequest> {
        self.requests.iter().filter(|r| is_main(r))
    }

    fn is_quick(&self) -> bool {
        let main: Vec<&&SpentRequest> = self.main().collect();
        main.len() <= QUICK_REQUESTS
            && main.iter().map(|r| r.point.tokens.output).sum::<u64>() <= QUICK_OUTPUT_TOKENS
            && main
                .iter()
                .all(|r| TOP_TIER.contains(&family(&r.point.model)))
    }

    fn cost(&self) -> f64 {
        self.requests.iter().filter_map(|r| r.cost_usd).sum()
    }
}

pub fn tasks<'a>(ctx: &InsightContext<'a>) -> Vec<Task<'a>> {
    let mut starts: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    for prompt in ctx.prompts {
        starts
            .entry(&prompt.session_id)
            .or_default()
            .push(prompt.ts);
    }
    let mut tasks: BTreeMap<(&str, i64), Vec<&SpentRequest>> = BTreeMap::new();
    for request in ctx.requests {
        let Some(times) = starts.get(request.session_id.as_str()) else {
            continue;
        };
        let at = times.partition_point(|ts| *ts <= request.point.ts);
        if at > 0 {
            tasks
                .entry((&request.session_id, times[at - 1]))
                .or_default()
                .push(request);
        }
    }
    tasks
        .into_iter()
        .map(|((session_id, _), requests)| Task {
            session_id,
            requests,
        })
        .collect()
}

pub struct LargeToolResults;

#[derive(Default)]
struct Source {
    results: u64,
    tokens: u64,
    usd: f64,
}

impl Rule for LargeToolResults {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        let mut sources: BTreeMap<(String, String), Source> = BTreeMap::new();
        for result in ctx.tool_results {
            let kept = ctx.ratios.tokens(
                result.chars.saturating_sub(KEPT_RESULT_CHARS),
                TextKind::Code,
            );
            let until = ctx
                .compactions
                .iter()
                .filter(|c| c.main && c.session_id == result.session_id && c.ts > result.ts)
                .map(|c| c.ts)
                .min()
                .unwrap_or(i64::MAX);
            let usd: f64 = ctx
                .main_requests()
                .filter(|r| r.session_id == result.session_id)
                .filter(|r| r.point.ts > result.ts && r.point.ts < until)
                .map(|r| ctx.carried_usd(r, kept))
                .sum();
            let tool = result.tool.clone().unwrap_or_else(|| "A tool".into());
            let source = sources.entry((tool, target_of(ctx, result))).or_default();
            source.results += 1;
            source.tokens += ctx.ratios.tokens(result.chars, TextKind::Code);
            source.usd += usd;
        }
        let mut ordered: Vec<(&(String, String), &Source)> = sources.iter().collect();
        ordered.sort_by(|a, b| b.1.usd.total_cmp(&a.1.usd).then_with(|| a.0.cmp(b.0)));
        let Some(((top, _), _)) = ordered.first() else {
            return Vec::new();
        };
        let tip = match top.as_str() {
            "Read" => "read big files in parts instead of whole",
            "Bash" => "trim long command output with head, tail or grep",
            _ => "ask for smaller tool results",
        };
        let usd = ordered.iter().map(|(_, source)| source.usd).sum();
        let draft = Draft {
            rule: RuleId::LargeToolResults,
            tip: tip.into(),
            evidence: listed(
                &ordered,
                |((tool, target), source)| {
                    let what = match target.is_empty() {
                        true => tool.clone(),
                        false => format!("{tool} {target}"),
                    };
                    format!(
                        "{what}: {}, ~{} tokens each",
                        count(source.results, "result", "results"),
                        tokens(source.tokens / source.results)
                    )
                },
                "sources",
            ),
            confidence: Confidence::Medium,
            how: format!(
                "Looks at main-thread tool results of {} characters or more in the last 14 days. \
                 Everything past the first {} characters is priced on each later request until the next compaction, \
                 at what each request paid for the front of its prompt: a cache read while the cache was warm, a cache write when it was rewritten.",
                grouped(LARGE_RESULT_CHARS),
                grouped(KEPT_RESULT_CHARS)
            ),
        };
        ctx.priced(draft, usd).into_iter().collect()
    }
}

fn target_of(ctx: &InsightContext, result: &LargeResult) -> String {
    let Some(target) = result.target.as_deref() else {
        return String::new();
    };
    let cwd = ctx
        .session(&result.session_id)
        .and_then(|s| s.cwd.as_deref())
        .map(|cwd| format!("{}/", cwd.trim_end_matches('/')));
    let relative = cwd
        .and_then(|cwd| target.strip_prefix(cwd.as_str()).map(String::from))
        .unwrap_or_else(|| target.to_string());
    let line = relative.lines().next().unwrap_or("").trim();
    let chars: Vec<char> = line.chars().collect();
    if chars.len() <= TARGET_CHARS {
        return line.to_string();
    }
    let kept = TARGET_CHARS - 1;
    match line.contains(' ') {
        true => format!("{}…", chars[..kept].iter().collect::<String>().trim_end()),
        false => format!(
            "…{}",
            chars[chars.len() - kept..].iter().collect::<String>()
        ),
    }
}

pub struct LateCompaction;

impl Rule for LateCompaction {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        let mut sessions: BTreeMap<&str, (usize, u64, f64)> = BTreeMap::new();
        for request in ctx.main_requests() {
            let prompt = request.point.tokens.prompt_total();
            if prompt < LATE_PROMPT_TOKENS {
                continue;
            }
            let session = sessions.entry(&request.session_id).or_default();
            session.0 += 1;
            session.1 = session.1.max(prompt);
            session.2 += REGROWTH * ctx.carried_usd(request, prompt - LATE_PROMPT_TOKENS);
        }
        let mut late: Vec<(&str, (usize, u64, f64))> = sessions
            .into_iter()
            .filter(|(_, (turns, _, _))| *turns >= MIN_LATE_TURNS)
            .collect();
        late.sort_by(|a, b| b.1.2.total_cmp(&a.1.2).then_with(|| a.0.cmp(b.0)));
        if late.is_empty() {
            return Vec::new();
        }
        let usd = late.iter().map(|(_, (_, _, usd))| usd).sum();
        let draft = Draft {
            rule: RuleId::LateCompaction,
            tip: "run /compact once a session passes 100k tokens".into(),
            evidence: listed(
                &late,
                |(session, (turns, peak, _))| {
                    format!(
                        "{}: {} above 100k, peak ~{}",
                        ctx.title(session),
                        count(*turns as u64, "turn", "turns"),
                        tokens(*peak)
                    )
                },
                "sessions",
            ),
            confidence: Confidence::Medium,
            how: format!(
                "Finds sessions of the last 14 days with {MIN_LATE_TURNS} or more main-thread turns above 100k prompt tokens. \
                 The saving prices half of the tokens past 100k on those turns: an earlier /compact summarizes them away, and the context grows back after it."
            ),
        };
        ctx.priced(draft, usd).into_iter().collect()
    }
}

pub struct ModelFit;

impl Rule for ModelFit {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        let mut by_session: BTreeMap<&str, Vec<Task>> = BTreeMap::new();
        for task in tasks(ctx) {
            by_session.entry(task.session_id).or_default().push(task);
        }
        let quick_sessions: Vec<&Vec<Task>> = by_session
            .values()
            .filter(|tasks| tasks.iter().all(Task::is_quick))
            .collect();
        let quick: Vec<&SpentRequest> = quick_sessions
            .iter()
            .flat_map(|tasks| tasks.iter().flat_map(|task| task.main().copied()))
            .collect();
        let mut spend: BTreeMap<&str, f64> = BTreeMap::new();
        let (mut current, mut cheaper) = (0.0, 0.0);
        for request in &quick {
            let model = request.point.model.as_str();
            let (Some(paid), Some(alternative)) = (request.cost_usd, alternative_for(model)) else {
                continue;
            };
            let Some(repriced) = ctx.prices.cost(alternative, &request.point.tokens) else {
                continue;
            };
            *spend.entry(model).or_default() += paid;
            current += paid;
            cheaper += repriced;
        }
        let Some(model) = spend
            .iter()
            .max_by(|a, b| a.1.total_cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(model, _)| *model)
        else {
            return Vec::new();
        };
        let alternative = alternative_for(model).unwrap_or_default();
        let mut evidence = vec![
            format!(
                "{} of quick tasks ran on {}",
                count(quick_sessions.len() as u64, "session", "sessions"),
                model_name(model)
            ),
            format!(
                "The same tokens on {} would cost {} instead of {}",
                model_name(alternative),
                money(cheaper),
                money(current)
            ),
        ];
        let mut efforts: BTreeMap<&str, usize> = BTreeMap::new();
        for request in &quick {
            if let Some(effort) = request.point.effort.as_deref() {
                *efforts.entry(effort).or_default() += 1;
            }
        }
        if let Some((effort, times)) = efforts.into_iter().max_by_key(|(_, n)| *n) {
            evidence.push(format!(
                "{} of {} at {effort} effort",
                times,
                count(quick.len() as u64, "request", "requests")
            ));
        }
        let draft = Draft {
            rule: RuleId::ModelFit,
            tip: format!(
                "use {} for quick questions and small edits",
                model_name(alternative)
            ),
            evidence,
            confidence: Confidence::Medium,
            how: format!(
                "Finds sessions of the last 14 days where every task, from a prompt to the next one, \
                 took at most {QUICK_REQUESTS} main-thread requests and {} output tokens on a top-tier model, \
                 and reprices those requests on {} at the same token counts.",
                tokens(QUICK_OUTPUT_TOKENS),
                model_name(alternative)
            ),
        };
        ctx.priced(draft, current - cheaper).into_iter().collect()
    }
}

pub struct CostlyTasks;

impl Rule for CostlyTasks {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        let all = tasks(ctx);
        if all.len() < MIN_TASKS {
            return Vec::new();
        }
        let Some(typical) = median(all.iter().map(Task::cost).collect()) else {
            return Vec::new();
        };
        let mut outliers: Vec<&Task> = all
            .iter()
            .filter(|t| t.cost() >= MIN_OUTLIER_USD && t.cost() >= OUTLIER_FACTOR * typical)
            .collect();
        outliers.sort_by(|a, b| b.cost().total_cmp(&a.cost()));
        if outliers.is_empty() {
            return Vec::new();
        }
        let mut evidence = listed(
            &outliers,
            |task| {
                format!(
                    "{}: {}, {}",
                    ctx.title(task.session_id),
                    money(task.cost()),
                    count(task.requests.len() as u64, "request", "requests")
                )
            },
            "tasks",
        );
        evidence.push(format!(
            "Your typical task costs {} across {}",
            money(typical),
            count(all.len() as u64, "task", "tasks")
        ));
        vec![
            Draft {
                rule: RuleId::CostlyTasks,
                tip: "break your biggest asks into smaller steps".into(),
                evidence,
                confidence: Confidence::Low,
                how: format!(
                    "A task runs from one prompt to the next, subagents included. \
                     These cost at least {} and {OUTLIER_FACTOR:.0} times your median task of the last 14 days. \
                     Smaller steps keep context short and let you stop early.",
                    money(MIN_OUTLIER_USD)
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

    fn close(actual: Option<f64>, expected: f64) {
        let actual = actual.unwrap();
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    #[test]
    fn tasks_run_from_a_prompt_to_the_next_one_with_their_subagents() {
        let mut fixture = Fixture::new();
        fixture.prompt("s1", SINCE + 10);
        fixture.prompt("s1", SINCE + 100);
        fixture.prompt("s2", SINCE + 50);
        fixture.request("s1", SINCE + 5, OPUS, counts(10, 0, 0, 10));
        fixture.request("s1", SINCE + 10, OPUS, counts(10, 0, 0, 10));
        fixture.subagent("s1", SINCE + 20, OPUS, counts(10, 0, 0, 10));
        fixture.request("s1", SINCE + 120, OPUS, counts(10, 0, 0, 10));
        fixture.request("s2", SINCE + 60, OPUS, counts(10, 0, 0, 10));
        fixture.request("s3", SINCE + 60, OPUS, counts(10, 0, 0, 10));
        let ctx = fixture.ctx();
        let shape: Vec<(&str, Vec<i64>)> = tasks(&ctx)
            .iter()
            .map(|t| {
                (
                    t.session_id,
                    t.requests.iter().map(|r| r.point.ts - SINCE).collect(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [("s1", vec![10, 20]), ("s1", vec![120]), ("s2", vec![60])]
        );
    }

    #[test]
    fn large_tool_results_are_priced_until_the_next_compaction_and_grouped_by_target() {
        let mut fixture = Fixture::new();
        fixture.session("s1", PROJECT, Some("Big refactor"));
        for n in 1..=100 {
            fixture.request("s1", SINCE + n * 10, OPUS, counts(2_000, 100_000, 0, 10));
        }
        fixture.large(
            "s1",
            SINCE + 5,
            40_000,
            "Read",
            Some("/work/demo/src/big.rs"),
        );
        fixture.large(
            "s1",
            SINCE + 205,
            40_000,
            "Read",
            Some("/work/demo/src/big.rs"),
        );
        fixture.large(
            "s1",
            SINCE + 505,
            31_000,
            "Bash",
            Some("cargo test --workspace --all-features -- --nocapture --test-threads 1"),
        );
        fixture.compaction("s1", SINCE + 600);
        let finding = &LargeToolResults.evaluate(&fixture.ctx())[0];
        assert_eq!(finding.tip, "read big files in parts instead of whole");
        assert_eq!(
            finding.evidence,
            [
                "Read src/big.rs: 2 results, ~13.3k tokens each",
                "Bash cargo test --workspace --all-…: 1 result, ~10.3k tokens each",
            ]
        );
        let read = 10_000.0 * (59.0 + 39.0) * 0.2 / 1e6;
        let bash = 7_000.0 * 9.0 * 0.2 / 1e6;
        close(finding.saving_usd, (read + bash) / 2.0);
    }

    #[test]
    fn long_targets_keep_the_head_of_a_command_and_the_tail_of_a_path() {
        let mut fixture = Fixture::new();
        fixture.session("s1", PROJECT, None);
        for target in [
            "/work/demo/docs/plans/2026-10-11-m6-insights-and-alerts.md",
            "/elsewhere/src/main.rs",
            "rg --files-with-matches --hidden 'fn main' /work/demo",
        ] {
            fixture.large("s1", SINCE, 20_000, "Read", Some(target));
        }
        let ctx = fixture.ctx();
        let shown: Vec<String> = fixture
            .tool_results
            .iter()
            .map(|result| target_of(&ctx, result))
            .collect();
        assert_eq!(
            shown,
            [
                "…-11-m6-insights-and-alerts.md",
                "/elsewhere/src/main.rs",
                "rg --files-with-matches --hid…",
            ]
        );
        assert!(
            shown
                .iter()
                .all(|target| target.chars().count() <= TARGET_CHARS)
        );
    }

    #[test]
    fn sessions_that_stayed_above_100k_for_ten_turns_suggest_compacting_earlier() {
        let mut fixture = Fixture::new();
        fixture.session("s1", PROJECT, Some("Long debugging"));
        for n in 0..10 {
            fixture.request("s1", SINCE + n, OPUS, counts(0, 300_000 + n as u64, 0, 10));
        }
        for n in 0..9 {
            fixture.request("s2", SINCE + n, OPUS, counts(0, 500_000, 0, 10));
        }
        let finding = &LateCompaction.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.evidence,
            ["Long debugging: 10 turns above 100k, peak ~300k"]
        );
        let carried: f64 = (0..10).map(|n| (200_000 + n) as f64).sum();
        close(finding.saving_usd, REGROWTH * carried * 0.2 / 1e6 / 2.0);
        fixture.requests.truncate(9);
        assert!(LateCompaction.evaluate(&fixture.ctx()).is_empty());
    }

    #[test]
    fn sessions_of_quick_tasks_on_a_top_tier_model_are_repriced_on_its_alternative() {
        let mut fixture = Fixture::new();
        for session in ["s1", "s2"] {
            for n in 0..20 {
                let at = SINCE + n * 100;
                fixture.prompt(session, at);
                fixture.request(session, at + 1, OPUS, counts(20_000, 0, 0, 500));
                fixture.effort("high");
            }
        }
        fixture.prompt("s3", SINCE);
        for n in 0..5 {
            fixture.request("s3", SINCE + n, OPUS, counts(20_000, 0, 0, 500));
        }
        let finding = &ModelFit.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "use Sonnet 5.5 for quick questions and small edits"
        );
        assert_eq!(
            finding.evidence,
            [
                "2 sessions of quick tasks ran on Opus 5.5",
                "The same tokens on Sonnet 5.5 would cost $1.80 instead of $3.60",
                "40 of 40 requests at high effort",
            ]
        );
        close(finding.saving_usd, 0.9);
    }

    #[test]
    fn quick_tasks_on_a_cheaper_model_need_no_tip() {
        let mut fixture = Fixture::new();
        fixture.prompt("s1", SINCE);
        fixture.request("s1", SINCE + 1, SONNET, counts(20_000, 0, 0, 500));
        assert!(ModelFit.evaluate(&fixture.ctx()).is_empty());
    }

    #[test]
    fn tasks_far_above_the_median_are_named_without_a_saving() {
        let mut fixture = Fixture::new();
        fixture.session("s1", PROJECT, Some("Migrate the schema"));
        for n in 0..10 {
            fixture.prompt("s1", SINCE + n * 100);
            fixture.request("s1", SINCE + n * 100 + 1, OPUS, counts(10_000, 0, 0, 0));
        }
        fixture.prompt("s1", SINCE + 2_000);
        fixture.request("s1", SINCE + 2_001, OPUS, counts(400_000, 0, 0, 0));
        fixture.subagent("s1", SINCE + 2_002, OPUS, counts(100_000, 0, 0, 0));
        let finding = &CostlyTasks.evaluate(&fixture.ctx())[0];
        assert_eq!(finding.saving_usd, None);
        assert_eq!(finding.confidence, Confidence::Low);
        assert_eq!(
            finding.evidence,
            [
                "Migrate the schema: $2.00, 2 requests",
                "Your typical task costs $0.04 across 11 tasks",
            ]
        );
        fixture.prompts.truncate(9);
        fixture.requests.truncate(9);
        assert!(CostlyTasks.evaluate(&fixture.ctx()).is_empty());
    }
}
