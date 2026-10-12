use std::collections::BTreeMap;

use super::words::{count, money, tokens};
use super::{Confidence, DAY_S, Draft, Finding, InsightContext, Rule, RuleId, is_main};
use crate::cache_model::{FIVE_MINUTES_MS, MIN_REBUILD_PREFIX, ONE_HOUR_MS, Ttl, read_instead};
use crate::pricing::normalize_model;
use crate::store::SpentRequest;
use crate::transcript::TokenCounts;

pub struct CacheTtl;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Bucket {
    Main,
    Subagents,
}

#[derive(Default)]
struct Replay {
    turns: u64,
    written_1h: u64,
    written_5m: u64,
    recent_1h: u64,
    recent_5m: u64,
    premium_1h_usd: f64,
    premium_5m_usd: f64,
    paused_warm: u64,
    paused_warm_usd: f64,
    paused_rebuilds: u64,
    paused_rebuild_usd: f64,
}

struct Switch {
    bucket: Bucket,
    to_5m: bool,
    net_usd: f64,
    evidence: Vec<String>,
}

impl Rule for CacheTtl {
    fn evaluate(&self, ctx: &InsightContext) -> Vec<Finding> {
        let mut threads: BTreeMap<(Bucket, &str, Option<&str>), Vec<&SpentRequest>> =
            BTreeMap::new();
        for request in ctx.requests {
            let bucket = if is_main(request) {
                Bucket::Main
            } else {
                Bucket::Subagents
            };
            threads
                .entry((
                    bucket,
                    request.session_id.as_str(),
                    request.point.agent_id.as_deref(),
                ))
                .or_default()
                .push(request);
        }
        let mut latest: BTreeMap<Bucket, i64> = BTreeMap::new();
        for ((bucket, _, _), thread) in &threads {
            let last = thread.iter().map(|r| r.point.ts).max().unwrap_or(i64::MIN);
            let entry = latest.entry(*bucket).or_insert(last);
            *entry = (*entry).max(last);
        }
        let mut replays: BTreeMap<Bucket, Replay> = BTreeMap::new();
        for ((bucket, session, _), mut thread) in threads {
            thread.sort_by_key(|r| r.point.ts);
            let recent_since = latest.get(&bucket).map_or(i64::MIN, |ts| ts - DAY_S);
            let replay = replays.entry(bucket).or_default();
            replay_thread(ctx, bucket, session, &thread, recent_since, replay);
        }
        let mut switches: Vec<Switch> = replays
            .into_iter()
            .filter_map(|(bucket, replay)| switch(bucket, &replay))
            .filter(|s| s.net_usd > 0.0)
            .collect();
        switches.sort_by(|a, b| b.net_usd.total_cmp(&a.net_usd));
        let Some(top) = switches.first() else {
            return Vec::new();
        };
        let draft = Draft {
            rule: RuleId::CacheTtl,
            tip: tip(top.bucket, top.to_5m).into(),
            evidence: switches.iter().flat_map(|s| s.evidence.clone()).collect(),
            confidence: Confidence::Medium,
            how: "Replays the last 14 days with the other cache lifetime, turn by turn, \
                  for the main thread and for subagents apart. \
                  A turn sent 5 to 60 minutes after the one before reads a 1-hour cache but rewrites a 5-minute one, \
                  and the 1-hour cache costs more to write. \
                  Set \"promptCacheTtl\" (main thread) or \"subagentPromptCacheTtl\" (subagents) to \"5m\" or \"1h\" \
                  in ~/.claude/settings.json, from Claude Code 2.1.242."
                .into(),
        };
        let net = switches.iter().map(|s| s.net_usd).sum();
        ctx.priced(draft, net).into_iter().collect()
    }
}

fn replay_thread(
    ctx: &InsightContext,
    bucket: Bucket,
    session: &str,
    thread: &[&SpentRequest],
    recent_since: i64,
    replay: &mut Replay,
) {
    let mut moved_by_index = vec![0u64; thread.len()];
    let mut cached_1h = None;
    for (index, pair) in thread.windows(2).enumerate() {
        let (previous, request) = (pair[0], pair[1]);
        replay.turns += 1;
        if previous.point.tokens.cache_write() > 0 {
            cached_1h = Some(previous.point.tokens.cache_write_1h > 0);
        }
        let prefix = previous.point.tokens.cache_read + previous.point.tokens.cache_write();
        let gap_ms = (sent(request) - sent(previous)) * 1000;
        let paused = (FIVE_MINUTES_MS..ONE_HOUR_MS).contains(&gap_ms);
        let same_model =
            normalize_model(&previous.point.model) == normalize_model(&request.point.model);
        let Some(cached_1h) = cached_1h.filter(|_| {
            paused
                && same_model
                && prefix >= MIN_REBUILD_PREFIX
                && !compacted_between(ctx, bucket, session, previous.point.ts, request.point.ts)
        }) else {
            continue;
        };
        let tokens = &request.point.tokens;
        let total = tokens.prompt_total();
        let model = request.point.model.as_str();
        let rebuilt = tokens.cache_read * 10 < prefix && tokens.cache_write() * 2 >= prefix;
        if rebuilt && !cached_1h {
            let moved = tokens.cache_write().min(prefix);
            moved_by_index[index + 1] = moved.min(tokens.cache_write_5m);
            let paid = ctx.prices.cost(model, tokens).unwrap_or(0.0);
            let warm = ctx
                .prices
                .cost(model, &read_instead(tokens, moved))
                .unwrap_or(0.0);
            replay.paused_rebuilds += 1;
            replay.paused_rebuild_usd += paid - warm;
        } else if !rebuilt && cached_1h {
            let moved = tokens.cache_read.min(prefix);
            let rewrite = cost_of(ctx, model, total, Ttl::FiveMinutes.written(moved));
            let read = cost_of(
                ctx,
                model,
                total,
                TokenCounts {
                    cache_read: moved,
                    ..Default::default()
                },
            );
            replay.paused_warm += 1;
            replay.paused_warm_usd += rewrite - read;
        }
    }
    for (request, moved) in thread.iter().zip(moved_by_index) {
        let tokens = &request.point.tokens;
        let total = tokens.prompt_total();
        let model = request.point.model.as_str();
        replay.written_1h += tokens.cache_write_1h;
        replay.written_5m += tokens.cache_write_5m;
        if request.point.ts >= recent_since {
            replay.recent_1h += tokens.cache_write_1h;
            replay.recent_5m += tokens.cache_write_5m;
        }
        replay.premium_1h_usd += premium(ctx, model, total, tokens.cache_write_1h);
        replay.premium_5m_usd += premium(ctx, model, total, tokens.cache_write_5m - moved);
    }
}

impl Replay {
    fn uses_1h(&self) -> bool {
        let (written_1h, written_5m) = match self.recent_1h + self.recent_5m {
            0 => (self.written_1h, self.written_5m),
            _ => (self.recent_1h, self.recent_5m),
        };
        written_1h > 0 && written_1h >= written_5m
    }
}

fn switch(bucket: Bucket, replay: &Replay) -> Option<Switch> {
    let who = match bucket {
        Bucket::Main => "Main thread",
        Bucket::Subagents => "Subagents",
    };
    if replay.uses_1h() {
        return Some(Switch {
            bucket,
            to_5m: true,
            net_usd: replay.premium_1h_usd - replay.paused_warm_usd,
            evidence: vec![
                format!(
                    "{who}: {}\u{a0}tokens written at the 1-hour rate, {} above the 5-minute rate",
                    tokens(replay.written_1h),
                    money(replay.premium_1h_usd)
                ),
                format!(
                    "{} of {} came 5 to 60 minutes after the one before, rewriting {} would cost {}",
                    replay.paused_warm,
                    count(replay.turns, "turn", "turns"),
                    if replay.paused_warm == 1 {
                        "it"
                    } else {
                        "them"
                    },
                    money(replay.paused_warm_usd)
                ),
            ],
        });
    }
    (replay.written_5m > 0).then(|| Switch {
        bucket,
        to_5m: false,
        net_usd: replay.paused_rebuild_usd - replay.premium_5m_usd,
        evidence: vec![
            format!(
                "{who}: {} after 5 to 60-minute pauses, {}",
                count(replay.paused_rebuilds, "rebuild", "rebuilds"),
                money(replay.paused_rebuild_usd)
            ),
            format!(
                "The 1-hour rate on the other cache writes would add {}",
                money(replay.premium_5m_usd)
            ),
        ],
    })
}

fn tip(bucket: Bucket, to_5m: bool) -> &'static str {
    match (bucket, to_5m) {
        (Bucket::Main, true) => {
            "use the 5-minute cache, your turns rarely pause long enough to need an hour"
        }
        (Bucket::Main, false) => "use the 1-hour cache, your pauses often outlast 5 minutes",
        (Bucket::Subagents, true) => {
            "use the 5-minute cache for subagents, they rarely pause long enough to need an hour"
        }
        (Bucket::Subagents, false) => {
            "use the 1-hour cache for subagents, they often pause past 5 minutes"
        }
    }
}

fn sent(request: &SpentRequest) -> i64 {
    request
        .point
        .sent
        .filter(|sent| *sent <= request.point.ts)
        .unwrap_or(request.point.ts)
}

fn compacted_between(
    ctx: &InsightContext,
    bucket: Bucket,
    session: &str,
    after: i64,
    until: i64,
) -> bool {
    bucket == Bucket::Main
        && ctx
            .compactions
            .iter()
            .any(|c| c.main && c.session_id == session && after < c.ts && c.ts <= until)
}

fn cost_of(ctx: &InsightContext, model: &str, total: u64, part: TokenCounts) -> f64 {
    ctx.prices.cost_within(model, &part, total).unwrap_or(0.0)
}

fn premium(ctx: &InsightContext, model: &str, total: u64, written: u64) -> f64 {
    if written == 0 {
        return 0.0;
    }
    cost_of(ctx, model, total, Ttl::OneHour.written(written))
        - cost_of(ctx, model, total, Ttl::FiveMinutes.written(written))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::fixture::*;

    fn written_1h(cache_read: u64, cache_write_1h: u64) -> TokenCounts {
        TokenCounts {
            input: 10,
            output: 10,
            cache_read,
            cache_write_1h,
            ..Default::default()
        }
    }

    fn close(actual: Option<f64>, expected: f64) {
        let actual = actual.unwrap();
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    fn short_main_turns(fixture: &mut Fixture) {
        fixture.request("s1", SINCE, OPUS, written_1h(0, 100_000));
        for turn in 1..=20 {
            fixture.request("s1", SINCE + turn * 60, OPUS, written_1h(100_000, 10_000));
        }
        fixture.request(
            "s1",
            SINCE + 20 * 60 + 1_200,
            OPUS,
            written_1h(100_000, 10_000),
        );
    }

    fn paused_subagent(fixture: &mut Fixture) {
        fixture.subagent("s1", SINCE, OPUS, counts(10, 0, 100_000, 10));
        fixture.subagent("s1", SINCE + 600, OPUS, counts(10, 0, 100_000, 10));
        fixture.subagent("s1", SINCE + 2_400, OPUS, counts(10, 0, 100_000, 10));
        fixture.subagent("s1", SINCE + 2_460, OPUS, counts(10, 100_000, 1_000, 10));
    }

    #[test]
    fn a_main_thread_that_rarely_pauses_is_told_to_use_the_five_minute_cache() {
        let mut fixture = Fixture::new();
        short_main_turns(&mut fixture);
        let findings = CacheTtl.evaluate(&fixture.ctx());
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(finding.rule, RuleId::CacheTtl);
        assert_eq!(
            finding.tip,
            "use the 5-minute cache, your turns rarely pause long enough to need an hour"
        );
        let premium = 310_000.0 * (8.0 - 5.0) / 1e6;
        let rewrite = 100_000.0 * (5.0 - 0.2) / 1e6;
        close(finding.saving_usd, (premium - rewrite) / 2.0);
        assert_eq!(finding.confidence, Confidence::Medium);
        assert_eq!(
            finding.evidence,
            [
                "Main thread: 310k\u{a0}tokens written at the 1-hour rate, $0.93 above the 5-minute rate",
                "1 of 21\u{a0}turns came 5 to 60 minutes after the one before, rewriting it would cost $0.48",
            ]
        );
    }

    #[test]
    fn subagents_that_pause_past_five_minutes_are_told_to_use_the_one_hour_cache() {
        let mut fixture = Fixture::new();
        paused_subagent(&mut fixture);
        let finding = &CacheTtl.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "use the 1-hour cache for subagents, they often pause past 5 minutes"
        );
        let rebuilds = 2.0 * 100_000.0 * (5.0 - 0.2) / 1e6;
        let premium = 101_000.0 * (8.0 - 5.0) / 1e6;
        close(finding.saving_usd, (rebuilds - premium) / 2.0);
        assert_eq!(
            finding.evidence,
            [
                "Subagents: 2\u{a0}rebuilds after 5 to 60-minute pauses, $0.96",
                "The 1-hour rate on the other cache writes would add $0.30",
            ]
        );
    }

    #[test]
    fn both_switches_add_up_and_the_larger_one_names_the_tip() {
        let mut fixture = Fixture::new();
        short_main_turns(&mut fixture);
        paused_subagent(&mut fixture);
        let finding = &CacheTtl.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "use the 1-hour cache for subagents, they often pause past 5 minutes"
        );
        assert_eq!(finding.evidence.len(), 4);
        assert!(finding.evidence[2].starts_with("Main thread: "));
        let main = (310_000.0 * 3.0 - 100_000.0 * 4.8) / 1e6;
        let subagents = (2.0 * 100_000.0 * 4.8 - 101_000.0 * 3.0) / 1e6;
        close(finding.saving_usd, (main + subagents) / 2.0);
    }

    #[test]
    fn a_switch_to_the_one_hour_cache_already_made_is_not_suggested_again() {
        let mut fixture = Fixture::new();
        fixture.request("s1", SINCE, OPUS, counts(10, 0, 100_000, 10));
        for pause in 1..=5 {
            fixture.request("s1", SINCE + pause * 600, OPUS, counts(10, 0, 100_000, 10));
        }
        let recent = NOW_S - 3 * 3_600;
        fixture.request("s2", recent, OPUS, written_1h(0, 100_000));
        fixture.request("s2", recent + 1_200, OPUS, written_1h(100_000, 0));
        fixture.request("s2", recent + 2_400, OPUS, written_1h(0, 100_000));
        fixture.request("s2", recent + 3_000, OPUS, written_1h(100_000, 0));
        assert!(CacheTtl.evaluate(&fixture.ctx()).is_empty());
    }

    #[test]
    fn a_rebuild_after_a_pause_counts_only_when_the_cache_lived_five_minutes() {
        let mut fixture = Fixture::new();
        fixture.request("s1", SINCE, OPUS, written_1h(0, 100_000));
        fixture.request("s1", SINCE + 1_200, OPUS, counts(10, 0, 100_000, 10));
        fixture.request("s1", SINCE + 1_260, OPUS, counts(10, 100_000, 0, 10));
        fixture.request("s1", SINCE + 1_860, OPUS, counts(10, 0, 100_000, 10));
        fixture.request("s1", SINCE + 2_460, OPUS, counts(10, 0, 100_000, 10));
        let finding = &CacheTtl.evaluate(&fixture.ctx())[0];
        assert_eq!(
            finding.tip,
            "use the 1-hour cache, your pauses often outlast 5 minutes"
        );
        assert_eq!(
            finding.evidence,
            [
                "Main thread: 2\u{a0}rebuilds after 5 to 60-minute pauses, $0.96",
                "The 1-hour rate on the other cache writes would add $0.30",
            ]
        );
    }

    #[test]
    fn a_pause_runs_from_when_the_turn_before_was_sent() {
        let mut fixture = Fixture::new();
        fixture.request("s1", SINCE + 180, OPUS, written_1h(0, 100_000));
        fixture.requests.last_mut().unwrap().point.sent = Some(SINCE);
        fixture.request("s1", SINCE + 400, OPUS, written_1h(100_000, 0));
        fixture.requests.last_mut().unwrap().point.sent = Some(SINCE + 360);
        assert!(CacheTtl.evaluate(&fixture.ctx()).is_empty());
    }

    #[test]
    fn pauses_that_cost_more_than_the_premium_keep_the_one_hour_cache() {
        let paused = |gap: i64, model: &str, compacted: bool| {
            let mut fixture = Fixture::new();
            fixture.request("s1", SINCE, OPUS, written_1h(0, 100_000));
            fixture.request("s1", SINCE + gap, model, written_1h(100_000, 0));
            if compacted {
                fixture.compaction("s1", SINCE + 10);
            }
            CacheTtl.evaluate(&fixture.ctx())
        };
        assert!(paused(1_200, OPUS, false).is_empty());
        for kept in [
            paused(3_600, OPUS, false),
            paused(NOW_S - SINCE - 100, OPUS, false),
            paused(1_200, SONNET, false),
            paused(1_200, OPUS, true),
        ] {
            close(kept[0].saving_usd, 100_000.0 * 3.0 / 1e6 / 2.0);
        }
        assert!(CacheTtl.evaluate(&Fixture::new().ctx()).is_empty());
    }
}
