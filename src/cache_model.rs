use std::collections::BTreeSet;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::pricing::{PriceTable, normalize_model};
use crate::store::{CompactionPoint, PrefixChange, Store};
use crate::timeline::{Observed, Source, Thread, Timeline};
use crate::transcript::TokenCounts;

pub const FIVE_MINUTES_MS: i64 = 300_000;
pub const ONE_HOUR_MS: i64 = 3_600_000;
pub const MIN_REBUILD_PREFIX: u64 = 2048;
pub const SETUP_KINDS: &[&str] = &[
    "skill_listing",
    "instructions",
    "nested_memory",
    "agent_listing_delta",
];
pub const MCP_TOOL_PREFIX: &str = "mcp__";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ttl {
    #[serde(rename = "5m")]
    FiveMinutes,
    #[serde(rename = "1h")]
    OneHour,
}

impl Ttl {
    pub fn millis(self) -> i64 {
        match self {
            Ttl::FiveMinutes => FIVE_MINUTES_MS,
            Ttl::OneHour => ONE_HOUR_MS,
        }
    }

    pub fn written(self, tokens: u64) -> TokenCounts {
        match self {
            Ttl::FiveMinutes => TokenCounts {
                cache_write_5m: tokens,
                ..Default::default()
            },
            Ttl::OneHour => TokenCounts {
                cache_write_1h: tokens,
                ..Default::default()
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TtlSource {
    Observed,
    Assumed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheState {
    Warm,
    Cold,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebuildCause {
    Idle,
    ModelSwitch,
    PrefixChanged,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rebuild {
    pub ts_ms: i64,
    pub cause: RebuildCause,
    pub tokens: u64,
    pub extra_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cache {
    pub state: CacheState,
    pub ttl: Ttl,
    pub ttl_source: TtlSource,
    pub last_touch_ms: Option<i64>,
    pub expires_at_ms: Option<i64>,
    pub hit_ratio: Option<f64>,
    pub saved_usd: f64,
    pub rebuilds: u64,
    pub rebuild_usd: f64,
    pub last_rebuild: Option<Rebuild>,
}

pub struct Context<'a> {
    pub prices: &'a PriceTable,
    pub prefix_changes: &'a [i64],
    pub compactions: &'a [i64],
    pub subscription: bool,
    pub now_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Causes {
    pub changes: Vec<(PrefixChange, bool)>,
    pub compactions: Vec<CompactionPoint>,
    prefix_changes: Vec<i64>,
    main_compactions: Vec<i64>,
}

impl Causes {
    pub fn read(store: &Store, session_id: &str, from_ts: i64, until_ts: i64) -> Result<Self> {
        let announced = store.prefix_kinds_between(session_id, i64::MIN, until_ts)?;
        let changed = mcp_changes(&announced);
        let changes = announced
            .into_iter()
            .zip(changed)
            .filter(|(change, _)| change.ts >= from_ts)
            .collect();
        Ok(Self::of(
            changes,
            store.compactions_between(session_id, from_ts, until_ts)?,
        ))
    }

    pub fn of(changes: Vec<(PrefixChange, bool)>, compactions: Vec<CompactionPoint>) -> Self {
        let prefix_changes = changes
            .iter()
            .filter(|(change, mcp)| *mcp || SETUP_KINDS.contains(&change.kind.as_str()))
            .map(|(change, _)| change.ts)
            .collect();
        let main_compactions = compactions
            .iter()
            .filter(|c| c.main)
            .map(|c| c.ts)
            .collect();
        Self {
            changes,
            compactions,
            prefix_changes,
            main_compactions,
        }
    }

    pub fn context<'a>(
        &'a self,
        prices: &'a PriceTable,
        subscription: bool,
        now_ms: i64,
    ) -> Context<'a> {
        Context {
            prices,
            prefix_changes: &self.prefix_changes,
            compactions: &self.main_compactions,
            subscription,
            now_ms,
        }
    }
}

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

pub fn ttl_of(thread: &Thread, requests: &[&Observed], subscription: bool) -> (Ttl, TtlSource) {
    let observed = requests
        .iter()
        .rev()
        .find(|request| request.source == Source::Transcript && request.tokens.cache_write() > 0);
    match observed {
        Some(request) if request.tokens.cache_write_1h > 0 => (Ttl::OneHour, TtlSource::Observed),
        Some(_) => (Ttl::FiveMinutes, TtlSource::Observed),
        None if *thread == Thread::Main && subscription => (Ttl::OneHour, TtlSource::Assumed),
        None => (Ttl::FiveMinutes, TtlSource::Assumed),
    }
}

pub fn model(timeline: &Timeline, context: &Context) -> Cache {
    let main = timeline.main();
    let (ttl, ttl_source) = ttl_of(&Thread::Main, &main, context.subscription);
    let last_touch_ms = main
        .iter()
        .rev()
        .find(|request| request.touches_cache())
        .map(|request| request.ts_ms);
    let expires_at_ms = last_touch_ms.map(|touched| touched + ttl.millis());
    let state = match expires_at_ms {
        None => CacheState::None,
        Some(expires) if context.now_ms < expires => CacheState::Warm,
        Some(_) => CacheState::Cold,
    };
    let rebuilds = rebuilds(&main, ttl, context);
    Cache {
        state,
        ttl,
        ttl_source,
        last_touch_ms,
        expires_at_ms,
        hit_ratio: hit_ratio(&timeline.requests),
        saved_usd: total(
            timeline
                .requests
                .iter()
                .filter_map(|request| saved_usd(&request.model, &request.tokens, context.prices)),
        ),
        rebuilds: rebuilds.len() as u64,
        rebuild_usd: total(rebuilds.iter().filter_map(|r| r.extra_usd)),
        last_rebuild: rebuilds.last().cloned(),
    }
}

fn total(values: impl Iterator<Item = f64>) -> f64 {
    values.fold(0.0, |sum, value| sum + value)
}

fn hit_ratio(requests: &[Observed]) -> Option<f64> {
    let (read, total) = requests
        .iter()
        .fold((0u64, 0u64), |(read, total), request| {
            (
                read + request.tokens.cache_read,
                total + request.tokens.prompt_total(),
            )
        });
    (total > 0).then(|| read as f64 / total as f64)
}

pub fn rebuilds(main: &[&Observed], ttl: Ttl, context: &Context) -> Vec<Rebuild> {
    rebuilt_requests(main, ttl, context)
        .into_iter()
        .map(|(_, rebuild)| rebuild)
        .collect()
}

pub fn rebuilt_requests<'a>(
    main: &[&'a Observed],
    ttl: Ttl,
    context: &Context,
) -> Vec<(&'a Observed, Rebuild)> {
    main.windows(2)
        .filter_map(|pair| Some((pair[1], rebuild(pair[0], pair[1], ttl, context)?)))
        .collect()
}

pub fn saved_usd(model: &str, tokens: &TokenCounts, prices: &PriceTable) -> Option<f64> {
    let uncached = TokenCounts {
        input: tokens.input + tokens.cache_read,
        cache_read: 0,
        ..*tokens
    };
    Some(prices.cost(model, &uncached)? - prices.cost(model, tokens)?)
}

fn rebuild(
    previous: &Observed,
    request: &Observed,
    ttl: Ttl,
    context: &Context,
) -> Option<Rebuild> {
    let prefix = previous.prefix();
    let is_rebuild = prefix >= MIN_REBUILD_PREFIX
        && request.tokens.cache_read * 10 < prefix
        && request.tokens.cache_write() * 2 >= prefix;
    let between = |times: &[i64]| {
        times.iter().any(|ts| {
            let at = ts * 1000;
            previous.ts_ms < at && at <= request.ts_ms
        })
    };
    if !is_rebuild || between(context.compactions) {
        return None;
    }
    let cause = if request.ts_ms - previous.ts_ms >= ttl.millis() {
        RebuildCause::Idle
    } else if normalize_model(&previous.model) != normalize_model(&request.model) {
        RebuildCause::ModelSwitch
    } else if between(context.prefix_changes) {
        RebuildCause::PrefixChanged
    } else {
        RebuildCause::Unknown
    };
    let tokens = request.tokens.cache_write().min(prefix);
    let extra_usd = context
        .prices
        .cost(&request.model, &request.tokens)
        .zip(
            context
                .prices
                .cost(&request.model, &read_instead(&request.tokens, tokens)),
        )
        .map(|(paid, warm)| paid - warm);
    Some(Rebuild {
        ts_ms: request.ts_ms,
        cause,
        tokens,
        extra_usd,
    })
}

pub fn read_instead(tokens: &TokenCounts, moved: u64) -> TokenCounts {
    let from_1h = if tokens.cache_write() == 0 {
        0
    } else {
        (moved as u128 * tokens.cache_write_1h as u128 / tokens.cache_write() as u128) as u64
    };
    TokenCounts {
        cache_read: tokens.cache_read + moved,
        cache_write_1h: tokens.cache_write_1h - from_1h,
        cache_write_5m: tokens.cache_write_5m - (moved - from_1h),
        ..*tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPUS: &str = "claude-opus-5-5";
    const T0: i64 = 1_791_000_000_000;

    fn at(seconds: i64, model: &str, read: u64, write_5m: u64, write_1h: u64) -> Observed {
        Observed {
            ts_ms: T0 + seconds * 1000,
            model: model.into(),
            thread: Thread::Main,
            tokens: TokenCounts {
                input: 10,
                output: 500,
                cache_read: read,
                cache_write_5m: write_5m,
                cache_write_1h: write_1h,
                ..Default::default()
            },
            effort: None,
            source: Source::Transcript,
        }
    }

    fn live(observed: Observed) -> Observed {
        Observed {
            source: Source::Live,
            ..observed
        }
    }

    fn cache(requests: Vec<Observed>, now_s: i64, subscription: bool) -> Cache {
        cache_with(requests, now_s, subscription, &[], &[])
    }

    fn cache_with(
        requests: Vec<Observed>,
        now_s: i64,
        subscription: bool,
        prefix_changes: &[i64],
        compactions: &[i64],
    ) -> Cache {
        let prices = PriceTable::bundled();
        model(
            &Timeline { requests },
            &Context {
                prices: &prices,
                prefix_changes,
                compactions,
                subscription,
                now_ms: T0 + now_s * 1000,
            },
        )
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn a_1h_write_keeps_the_cache_warm_for_an_hour_from_the_last_touch() {
        let requests = vec![at(0, OPUS, 0, 0, 20_000), at(600, OPUS, 20_000, 0, 500)];
        let warm = cache(requests.clone(), 600 + 3599, false);
        assert_eq!(
            (warm.ttl, warm.ttl_source),
            (Ttl::OneHour, TtlSource::Observed)
        );
        assert_eq!(warm.last_touch_ms, Some(T0 + 600_000));
        assert_eq!(warm.expires_at_ms, Some(T0 + 600_000 + ONE_HOUR_MS));
        assert_eq!(warm.state, CacheState::Warm);
        assert_eq!(cache(requests, 600 + 3600, false).state, CacheState::Cold);
    }

    #[test]
    fn a_read_refreshes_the_ttl_and_a_request_without_cache_does_not() {
        let requests = vec![
            at(0, OPUS, 0, 9000, 0),
            at(200, OPUS, 9000, 0, 0),
            at(250, OPUS, 0, 0, 0),
        ];
        let model = cache(requests, 260, true);
        assert_eq!(
            (model.ttl, model.ttl_source),
            (Ttl::FiveMinutes, TtlSource::Observed)
        );
        assert_eq!(model.last_touch_ms, Some(T0 + 200_000));
        assert_eq!(model.expires_at_ms, Some(T0 + 500_000));
    }

    #[test]
    fn without_a_transcript_write_the_ttl_is_assumed_from_the_plan_and_thread() {
        let steps = vec![live(at(0, OPUS, 0, 9000, 0))];
        let subscribed = cache(steps.clone(), 10, true);
        assert_eq!(
            (subscribed.ttl, subscribed.ttl_source),
            (Ttl::OneHour, TtlSource::Assumed)
        );
        let metered = cache(steps, 10, false);
        assert_eq!(
            (metered.ttl, metered.ttl_source),
            (Ttl::FiveMinutes, TtlSource::Assumed)
        );
        assert_eq!(
            ttl_of(&Thread::Agent("a1".into()), &[], true),
            (Ttl::FiveMinutes, TtlSource::Assumed)
        );
    }

    #[test]
    fn subagent_requests_do_not_move_the_main_countdown() {
        let mut agent = at(900, OPUS, 5000, 0, 0);
        agent.thread = Thread::Agent("a1".into());
        let model = cache(vec![at(0, OPUS, 0, 0, 9000), agent], 1000, true);
        assert_eq!(model.last_touch_ms, Some(T0));
    }

    #[test]
    fn a_session_without_cache_activity_has_no_state() {
        let model = cache(vec![at(0, OPUS, 0, 0, 0)], 1, true);
        assert_eq!(model.state, CacheState::None);
        assert_eq!(model.expires_at_ms, None);
        assert_eq!(cache(vec![], 1, true).hit_ratio, None);
    }

    #[test]
    fn empty_sums_are_written_as_a_plain_zero() {
        let json = serde_json::to_string(&cache(vec![], 1, true)).unwrap();
        assert!(json.contains(r#""saved_usd":0.0,"#), "{json}");
        assert!(json.contains(r#""rebuild_usd":0.0,"#), "{json}");
    }

    #[test]
    fn hit_ratio_and_savings_count_every_thread() {
        let mut agent = at(30, OPUS, 4000, 1000, 0);
        agent.thread = Thread::Agent("a1".into());
        let model = cache(vec![at(0, OPUS, 6000, 0, 2000), agent], 40, true);
        let read = 10_000.0;
        let prompt = read + 3000.0 + 20.0;
        assert!(close(model.hit_ratio.unwrap(), read / prompt));
        assert!(close(model.saved_usd, read * (4.0 - 0.2) / 1e6));
    }

    #[test]
    fn rebuilds_are_named_by_their_cause() {
        let idle = cache(
            vec![at(0, OPUS, 50_000, 0, 1000), at(4000, OPUS, 0, 0, 51_000)],
            4001,
            true,
        );
        assert_eq!(idle.rebuilds, 1);
        let last = idle.last_rebuild.unwrap();
        assert_eq!(
            (last.cause, last.tokens, last.ts_ms),
            (RebuildCause::Idle, 51_000, T0 + 4_000_000)
        );
        assert!(close(last.extra_usd.unwrap(), 51_000.0 * (8.0 - 0.2) / 1e6));
        assert!(close(idle.rebuild_usd, last.extra_usd.unwrap()));

        let switched = cache(
            vec![
                at(0, OPUS, 50_000, 0, 1000),
                at(60, "claude-sonnet-5-5", 0, 0, 51_000),
            ],
            61,
            true,
        );
        assert_eq!(
            switched.last_rebuild.unwrap().cause,
            RebuildCause::ModelSwitch
        );

        let requests = vec![at(0, OPUS, 50_000, 0, 1000), at(60, OPUS, 0, 0, 51_000)];
        let changed = cache_with(requests.clone(), 61, true, &[(T0 / 1000) + 30], &[]);
        assert_eq!(
            changed.last_rebuild.unwrap().cause,
            RebuildCause::PrefixChanged
        );
        let unknown = cache_with(requests.clone(), 61, true, &[(T0 / 1000) - 30], &[]);
        assert_eq!(unknown.last_rebuild.unwrap().cause, RebuildCause::Unknown);
        let compacted = cache_with(requests, 61, true, &[], &[(T0 / 1000) + 30]);
        assert_eq!((compacted.rebuilds, compacted.last_rebuild), (0, None));
    }

    #[test]
    fn a_rebuild_costs_what_its_own_request_paid_over_reading_the_prefix() {
        let mut long = at(4000, "claude-haiku-5-5", 0, 0, 60_000);
        long.tokens.input = 50_000;
        let haiku = cache(
            vec![at(0, "claude-haiku-5-5", 59_000, 0, 1000), long],
            4001,
            true,
        );
        assert!(close(
            haiku.last_rebuild.unwrap().extra_usd.unwrap(),
            60_000.0 * (0.5 * 2.0 - 0.05) / 1e6
        ));
        let five_minute = cache(
            vec![
                at(0, OPUS, 50_000, 0, 1000),
                at(4000, OPUS, 0, 51_000, 0),
                at(4100, OPUS, 51_000, 0, 100),
            ],
            4101,
            true,
        );
        assert_eq!(five_minute.ttl, Ttl::OneHour);
        assert!(close(
            five_minute.last_rebuild.unwrap().extra_usd.unwrap(),
            51_000.0 * (4.0 * 1.25 - 0.2) / 1e6
        ));
    }

    #[test]
    fn small_prefixes_and_partial_reads_are_not_rebuilds() {
        let small = cache(
            vec![at(0, OPUS, 1000, 0, 500), at(4000, OPUS, 0, 0, 1600)],
            4001,
            true,
        );
        assert_eq!(small.rebuilds, 0);
        let partial = cache(
            vec![
                at(0, OPUS, 50_000, 0, 1000),
                at(60, OPUS, 30_000, 0, 21_000),
            ],
            61,
            true,
        );
        assert_eq!(partial.rebuilds, 0);
    }

    #[test]
    fn an_unpriced_model_still_counts_its_rebuild_without_dollars() {
        let model = cache(
            vec![
                at(0, "claude-mystery-9", 50_000, 0, 1000),
                at(4000, "claude-mystery-9", 0, 0, 51_000),
            ],
            4001,
            true,
        );
        assert_eq!(model.rebuilds, 1);
        assert_eq!(model.last_rebuild.unwrap().extra_usd, None);
        assert_eq!((model.rebuild_usd, model.saved_usd), (0.0, 0.0));
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
    fn only_setup_records_and_real_mcp_changes_explain_a_rewrite_and_only_main_compactions_hide_one()
     {
        let at = |ts: i64, kind: &str, mcp: bool| {
            (
                PrefixChange {
                    ts,
                    ..change(kind, &[], &[])
                },
                mcp,
            )
        };
        let compaction = |ts: i64, main: bool| CompactionPoint {
            session_id: "s1".into(),
            ts,
            main,
            post_tokens: None,
        };
        let causes = Causes::of(
            vec![
                at(10, "deferred_tools_record", false),
                at(20, "deferred_tools_delta", false),
                at(30, "deferred_tools_delta", true),
                at(40, "nested_memory", false),
                at(50, "mcp_instructions_delta", true),
            ],
            vec![compaction(60, false), compaction(70, true)],
        );
        let prices = PriceTable::bundled();
        let context = causes.context(&prices, false, 0);
        assert_eq!(context.prefix_changes, [30, 40, 50]);
        assert_eq!(context.compactions, [70]);
        assert_eq!(causes.changes.len(), 5);
        assert_eq!(causes.compactions.len(), 2);
    }
}
