use std::collections::BTreeMap;

use super::sources::{Argument, COMMAND_TOOLS, Source};
use super::words::{count, grouped, model_name, money, tokens};
use super::{Confidence, Draft, Finding, InsightContext, Rule, RuleId, is_main, listed, median};
use crate::context_model::TextKind;
use crate::forecast::{alternative_for, family};
use crate::store::SpentRequest;
use crate::transcript::TokenCounts;

pub const LARGE_RESULT_CHARS: u64 = 20_000;
pub const KEPT_RESULT_CHARS: u64 = 10_000;
pub const COMPACT_THRESHOLDS: &[u64] = &[
    50_000, 100_000, 150_000, 200_000, 250_000, 300_000, 350_000, 400_000, 500_000, 600_000,
    700_000, 800_000, 900_000,
];
pub const NEAR_BEST: f64 = 0.9;
pub const RESTART_CONTEXT_TOKENS: u64 = 68_000;
pub const RESTART_REWRITE_TOKENS: u64 = 40_000;
pub const SUMMARY_TOKENS: u64 = 20_000;
pub const QUICK_REQUESTS: usize = 3;
pub const QUICK_OUTPUT_TOKENS: u64 = 1_000;
pub const TOP_TIER: &[&str] = &["opus", "fable", "mythos"];
pub const MIN_TASKS: usize = 10;
pub const OUTLIER_FACTOR: f64 = 5.0;
pub const MIN_OUTLIER_USD: f64 = 1.0;

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
struct Carried {
    results: u64,
    tokens: u64,
    usd: f64,
}

impl Rule for LargeToolResults {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        let mut sources: BTreeMap<Source, Carried> = BTreeMap::new();
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
            let cwd = ctx
                .session(&result.session_id)
                .and_then(|s| s.cwd.as_deref());
            let source = Source::of(result.tool.as_deref(), result.target.as_deref(), cwd);
            let carried = sources.entry(source).or_default();
            carried.results += 1;
            carried.tokens += ctx.ratios.tokens(result.chars, TextKind::Code);
            carried.usd += usd;
        }
        let mut ordered: Vec<(&Source, &Carried)> = sources.iter().collect();
        ordered.sort_by(|a, b| b.1.usd.total_cmp(&a.1.usd).then_with(|| a.0.cmp(b.0)));
        let Some((top, _)) = ordered
            .iter()
            .find(|(source, _)| !matches!(source, Source::Unnamed { .. }))
            .or(ordered.first())
        else {
            return Vec::new();
        };
        let usd = ordered.iter().map(|(_, carried)| carried.usd).sum();
        let draft = Draft {
            rule: RuleId::LargeToolResults,
            tip: tip_for(top),
            evidence: listed(
                &ordered,
                |(source, carried)| {
                    let size = tokens(carried.tokens / carried.results);
                    match carried.results {
                        1 => format!("{}: 1 result, ~{size} tokens", source.label()),
                        n => format!("{}: {n} results, ~{size} tokens each", source.label()),
                    }
                },
                "sources",
            ),
            confidence: Confidence::Medium,
            how: format!(
                "Looks at main-thread tool results of {} characters or more in the last 14 days, \
                 grouped by file, or by a command's program and its first file or argument. \
                 Everything past the first {} characters is priced on each later request until the next compaction, \
                 at what each request paid for the front of its prompt: a cache read while the cache was warm, a cache write when it was rewritten.",
                grouped(LARGE_RESULT_CHARS),
                grouped(KEPT_RESULT_CHARS)
            ),
        };
        ctx.priced(draft, usd).into_iter().collect()
    }
}

fn tip_for(source: &Source) -> String {
    if let Some(file) = source.file() {
        let name = file.rsplit('/').next().unwrap_or(file);
        return format!("read big files like {name} in parts instead of whole");
    }
    match source {
        Source::Command {
            program,
            argument: Some(Argument::Word(word)),
            ..
        } => format!("trim the output of {program} {word} with head, tail or grep"),
        Source::Command { program, .. } => {
            format!("trim the output of {program} with head, tail or grep")
        }
        Source::Unnamed { tool } if COMMAND_TOOLS.contains(&tool.as_str()) => {
            "trim long command output with head, tail or grep".into()
        }
        other => format!("ask {} for smaller results", other.tool()),
    }
}

pub struct LateCompaction;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Restart {
    context: u64,
    rewrite: u64,
    summary: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Outcome {
    net_usd: f64,
    compactions: u64,
    late_turns: u64,
}

impl Restart {
    fn observed(ctx: &InsightContext) -> Self {
        let mut context = Vec::new();
        let mut rewrite = Vec::new();
        let mut summary = Vec::new();
        for compaction in ctx.compactions.iter().filter(|c| c.main) {
            summary.extend(compaction.post_tokens.map(|n| n as f64));
            let first = ctx
                .main_requests()
                .filter(|r| r.session_id == compaction.session_id && r.point.ts > compaction.ts)
                .min_by_key(|r| r.point.ts);
            if let Some(first) = first {
                let tokens = &first.point.tokens;
                context.push(tokens.prompt_total() as f64);
                rewrite.push((tokens.input + tokens.cache_write()) as f64);
            }
        }
        let typical =
            |values: Vec<f64>, fallback: u64| median(values).map_or(fallback, |m| m as u64);
        Restart {
            context: typical(context, RESTART_CONTEXT_TOKENS),
            rewrite: typical(rewrite, RESTART_REWRITE_TOKENS),
            summary: typical(summary, SUMMARY_TOKENS),
        }
    }
}

struct Policy<'a> {
    threshold: u64,
    sessions: Vec<(&'a str, Outcome)>,
}

impl Policy<'_> {
    fn net_usd(&self) -> f64 {
        self.sessions
            .iter()
            .map(|(_, outcome)| outcome.net_usd)
            .sum()
    }
}

fn simulate(
    ctx: &InsightContext,
    segments: &[Vec<&SpentRequest>],
    restart: Restart,
    threshold: u64,
) -> Outcome {
    let mut outcome = Outcome::default();
    for turns in segments {
        let one_hour = turns.iter().any(|r| r.point.tokens.cache_write_1h > 0);
        let mut removed = 0u64;
        let mut rewrite_next = false;
        for request in turns {
            let tokens = &request.point.tokens;
            let model = request.point.model.as_str();
            let whole = tokens.prompt_total();
            if whole >= threshold {
                outcome.late_turns += 1;
            }
            outcome.net_usd += ctx.carried_usd(request, removed);
            if rewrite_next {
                let written = TokenCounts {
                    cache_write_1h: if one_hour { restart.rewrite } else { 0 },
                    cache_write_5m: if one_hour { 0 } else { restart.rewrite },
                    ..Default::default()
                };
                let read = TokenCounts {
                    cache_read: restart.rewrite,
                    ..Default::default()
                };
                let premium = ctx
                    .prices
                    .cost_within(model, &written, whole)
                    .unwrap_or(0.0)
                    - ctx.prices.cost_within(model, &read, whole).unwrap_or(0.0);
                outcome.net_usd -= premium;
                rewrite_next = false;
            }
            let context = whole.saturating_sub(removed);
            if context >= threshold && context > restart.context {
                let summarized = TokenCounts {
                    cache_read: context,
                    output: restart.summary,
                    ..Default::default()
                };
                outcome.net_usd -= ctx.prices.cost(model, &summarized).unwrap_or(0.0);
                outcome.compactions += 1;
                removed += context - restart.context;
                rewrite_next = true;
            }
        }
    }
    outcome
}

fn segments<'a>(ctx: &InsightContext<'a>, session: &str) -> Vec<Vec<&'a SpentRequest>> {
    let mut turns: Vec<&SpentRequest> = ctx
        .requests
        .iter()
        .filter(|r| is_main(r) && r.session_id == session)
        .collect();
    turns.sort_by_key(|r| r.point.ts);
    let mut cuts: Vec<i64> = ctx
        .compactions
        .iter()
        .filter(|c| c.main && c.session_id == session)
        .map(|c| c.ts)
        .collect();
    cuts.sort();
    let mut segments: Vec<Vec<&SpentRequest>> = vec![Vec::new()];
    let mut next_cut = cuts.iter().peekable();
    for turn in turns {
        let mut cut = false;
        while next_cut.peek().is_some_and(|ts| **ts < turn.point.ts) {
            next_cut.next();
            cut = true;
        }
        if cut && segments.last().is_some_and(|s| !s.is_empty()) {
            segments.push(Vec::new());
        }
        if let Some(last) = segments.last_mut() {
            last.push(turn);
        }
    }
    segments
}

impl Rule for LateCompaction {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        let restart = Restart::observed(ctx);
        let sessions: Vec<&str> = ctx
            .main_requests()
            .map(|r| r.session_id.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let split: Vec<(&str, Vec<Vec<&SpentRequest>>)> = sessions
            .iter()
            .map(|session| (*session, segments(ctx, session)))
            .collect();
        let policies: Vec<Policy> = COMPACT_THRESHOLDS
            .iter()
            .map(|threshold| Policy {
                threshold: *threshold,
                sessions: split
                    .iter()
                    .map(|(session, segments)| {
                        (*session, simulate(ctx, segments, restart, *threshold))
                    })
                    .filter(|(_, outcome)| outcome.compactions > 0)
                    .collect(),
            })
            .collect();
        let best = policies.iter().map(Policy::net_usd).fold(0.0, f64::max);
        let Some(chosen) = policies
            .into_iter()
            .rev()
            .find(|policy| best > 0.0 && policy.net_usd() >= NEAR_BEST * best)
        else {
            return Vec::new();
        };
        let (threshold, usd) = (chosen.threshold, chosen.net_usd());
        let mut late: Vec<(&str, Outcome)> = chosen
            .sessions
            .into_iter()
            .filter(|(_, outcome)| outcome.net_usd > 0.0)
            .collect();
        late.sort_by(|a, b| {
            b.1.net_usd
                .total_cmp(&a.1.net_usd)
                .then_with(|| a.0.cmp(b.0))
        });
        let peak = |session: &str| {
            ctx.main_requests()
                .filter(|r| r.session_id == session)
                .map(|r| r.point.tokens.prompt_total())
                .max()
                .unwrap_or(0)
        };
        let draft = Draft {
            rule: RuleId::LateCompaction,
            tip: format!(
                "run /compact once a session passes {} tokens",
                tokens(threshold)
            ),
            evidence: listed(
                &late,
                |(session, outcome)| {
                    format!(
                        "{}: {} above {}, peak ~{}",
                        ctx.title(session),
                        count(outcome.late_turns, "turn", "turns"),
                        tokens(threshold),
                        tokens(peak(session))
                    )
                },
                "sessions",
            ),
            confidence: Confidence::Medium,
            how: format!(
                "Replays the main-thread turns of the last 14 days as if you ran /compact whenever context passed {threshold}. \
                 Each /compact is priced as its own request, reading the whole context and writing a ~{} token summary, \
                 plus a cache rewrite of ~{} tokens on the next turn; the turns after it carry ~{} tokens of context and grow back as they did. \
                 Sizes come from your own compactions when there are any. \
                 {threshold} is the highest threshold that saves within {:.0}% of the best one, \
                 because every /compact also forgets detail that the saving cannot price.",
                tokens(restart.summary),
                tokens(restart.rewrite),
                tokens(restart.context),
                (1.0 - NEAR_BEST) * 100.0,
                threshold = tokens(threshold),
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
            Some("cd /work/demo/sdd/open-edition && sed -n 1,200p progress.md"),
        );
        fixture.large(
            "s1",
            SINCE + 515,
            31_000,
            "Bash",
            Some("cd /work/demo/sdd/open-edition; cat lane-rules.md"),
        );
        fixture.compaction("s1", SINCE + 600);
        let finding = &LargeToolResults.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "read big files like big.rs in parts instead of whole"
        );
        assert_eq!(
            finding.evidence,
            [
                "Read src/big.rs: 2 results, ~13.3k tokens each",
                "Bash sed …/open-edition/progress.md: 1 result, ~10.3k tokens",
                "Bash cat …pen-edition/lane-rules.md: 1 result, ~10.3k tokens",
            ]
        );
        let read = 10_000.0 * (59.0 + 39.0) * 0.2 / 1e6;
        let sed = 7_000.0 * 9.0 * 0.2 / 1e6;
        let cat = 7_000.0 * 8.0 * 0.2 / 1e6;
        close(finding.saving_usd, (read + sed + cat) / 2.0);
    }

    #[test]
    fn the_tip_names_the_file_or_command_that_costs_most() {
        let tip_for = |tool: &str, target: Option<&str>| {
            let mut fixture = Fixture::new();
            fixture.session("s1", PROJECT, None);
            for n in 1..=100 {
                fixture.request("s1", SINCE + n * 10, OPUS, counts(0, 100_000, 0, 10));
            }
            fixture.large("s1", SINCE + 5, 80_000, tool, target);
            LargeToolResults.evaluate(&fixture.ctx())[0].tip.clone()
        };
        assert_eq!(
            tip_for("Bash", Some("cd /work/demo && sed -n 1,900p docs/plan.md")),
            "read big files like plan.md in parts instead of whole"
        );
        assert_eq!(
            tip_for("Bash", Some("cargo test --workspace -- --nocapture")),
            "trim the output of cargo test with head, tail or grep"
        );
        assert_eq!(
            tip_for("Bash", None),
            "trim long command output with head, tail or grep"
        );
        let mut fixture = Fixture::new();
        fixture.session("s1", PROJECT, None);
        for n in 1..=100 {
            fixture.request("s1", SINCE + n * 10, OPUS, counts(0, 100_000, 0, 10));
        }
        fixture.large("s1", SINCE + 5, 90_000, "Bash", None);
        fixture.large("s1", SINCE + 5, 60_000, "Read", Some("/work/demo/notes.md"));
        let finding = &LargeToolResults.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "read big files like notes.md in parts instead of whole"
        );
        assert_eq!(
            finding.evidence[0],
            "Bash commands too long to name: 1 result, ~30k tokens"
        );
        assert_eq!(
            tip_for("mcp__linear__search", Some("open bugs")),
            "ask linear search for smaller results"
        );
    }

    fn compacted_once_before(fixture: &mut Fixture) {
        fixture.compacted("s0", SINCE + 1, 2_000);
        fixture.request("s0", SINCE + 2, OPUS, counts(0, 10_000, 10_000, 0));
    }

    #[test]
    fn compacting_earlier_is_priced_net_of_its_own_request_and_the_rewrite_after_it() {
        let mut fixture = Fixture::new();
        fixture.session("s1", PROJECT, Some("Long debugging"));
        compacted_once_before(&mut fixture);
        for n in 0..100 {
            fixture.request("s1", SINCE + 100 + n, OPUS, counts(0, 400_000, 0, 0));
        }
        let finding = &LateCompaction.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "run /compact once a session passes 400k tokens"
        );
        assert_eq!(
            finding.evidence,
            ["Long debugging: 100 turns above 400k, peak ~400k"]
        );
        let summary_request = (400_000.0 * 0.2 + 2_000.0 * 20.0) / 1e6;
        let rewrite = 10_000.0 * (5.0 - 0.2) / 1e6;
        let lighter_turns = 99.0 * (400_000.0 - 20_000.0) * 0.2 / 1e6;
        close(
            finding.saving_usd,
            (lighter_turns - summary_request - rewrite) / 2.0,
        );
    }

    #[test]
    fn the_threshold_is_the_highest_one_near_the_best_and_counts_the_sessions_it_costs() {
        let mut fixture = Fixture::new();
        fixture.session("s1", PROJECT, Some("Long debugging"));
        compacted_once_before(&mut fixture);
        let mut turns = |session: &str, at: i64, n: i64, context: u64| {
            for k in 0..n {
                fixture.request(session, SINCE + at + k, OPUS, counts(0, context, 0, 0));
            }
        };
        turns("s1", 100, 100, 400_000);
        turns("s2", 300, 3, 400_000);
        turns("s3", 400, 10, 300_000);
        let finding = &LateCompaction.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "run /compact once a session passes 400k tokens"
        );
        assert_eq!(
            finding.evidence,
            ["Long debugging: 100 turns above 400k, peak ~400k"]
        );
        let compact = |context: f64| (context * 0.2 + 2_000.0 * 20.0) / 1e6 + 10_000.0 * 4.8 / 1e6;
        let lighter = |turns: f64, context: f64| turns * (context - 20_000.0) * 0.2 / 1e6;
        let s1 = lighter(99.0, 400_000.0) - compact(400_000.0);
        let s2 = lighter(2.0, 400_000.0) - compact(400_000.0);
        let s3 = lighter(9.0, 300_000.0) - compact(300_000.0);
        assert!(s2 < 0.0 && s3 > 0.0);
        assert!(s1 + s2 >= NEAR_BEST * (s1 + s2 + s3));
        close(finding.saving_usd, (s1 + s2) / 2.0);
    }

    #[test]
    fn a_compaction_that_costs_more_than_it_saves_is_no_tip() {
        let mut fixture = Fixture::new();
        fixture.session("s1", PROJECT, Some("Long debugging"));
        compacted_once_before(&mut fixture);
        for n in 0..3 {
            fixture.request("s1", SINCE + 100 + n, OPUS, counts(0, 400_000, 0, 0));
        }
        assert!(LateCompaction.evaluate(&fixture.ctx()).is_empty());
    }

    #[test]
    fn a_real_compaction_starts_the_context_over_in_both_cases() {
        let mut fixture = Fixture::new();
        fixture.session("s1", PROJECT, Some("Long debugging"));
        compacted_once_before(&mut fixture);
        for n in 0..3 {
            fixture.request("s1", SINCE + 100 + n, OPUS, counts(0, 400_000, 0, 0));
        }
        fixture.compacted("s1", SINCE + 103, 2_000);
        for n in 0..3 {
            fixture.request("s1", SINCE + 104 + n, OPUS, counts(0, 400_000, 0, 0));
        }
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
