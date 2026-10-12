use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;
use chrono::TimeZone;
use serde::{Deserialize, Serialize};

use super::usage::LARGE_RESULT_CHARS;
use super::words::{count, money};
use super::{
    DAY_S, Finding, INSIGHTS_VERSION, InsightContext, MemoryText, Rebuilt, WINDOW_DAYS, evaluate,
    is_main,
};
use crate::cache_model::{self, Causes};
use crate::config_scan::{self, Loading, Scan, SourceKind};
use crate::context_model::{self, Uses, breakdown, items};
use crate::history::{Range, Rates, calendar};
use crate::limits;
use crate::pricing::PriceTable;
use crate::scope::Scope;
use crate::store::{
    CompactionPoint, PrefixChange, PromptPoint, SpentAdjustment, SpentRequest, Store,
};
use crate::timeline::{Thread, Timeline};

pub const MONTH_DAYS: i64 = 30;
pub const WEEK_DAYS: usize = 7;
pub const STREAK_HIT_RATIO: f64 = 0.9;
pub const STREAK_WIN_DAYS: u64 = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightsReport {
    pub v: u32,
    pub session: String,
    pub generated_at_ms: i64,
    pub since_ms: i64,
    pub requests: u64,
    pub sessions: u64,
    pub findings: Vec<Finding>,
    pub wins: Wins,
    pub recap: Option<Recap>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wins {
    pub saved_usd_7d: f64,
    pub hit_streak_days: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recap {
    pub day: String,
    pub start_ms: i64,
    pub cost_usd: f64,
    pub hit_ratio: Option<f64>,
    pub saved_usd: f64,
    pub win: Option<String>,
}

pub struct Request<'a> {
    pub session: &'a str,
    pub scan: &'a Scan,
    pub retention_days: Option<u64>,
    pub now_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Day {
    pub start: i64,
    pub label: String,
    pub requests: u64,
    pub cost_usd: f64,
    pub saved_usd: f64,
    pub read: u64,
    pub prompt: u64,
}

impl Day {
    fn hit_ratio(&self) -> Option<f64> {
        (self.prompt > 0).then(|| self.read as f64 / self.prompt as f64)
    }

    fn on_streak(&self) -> bool {
        self.hit_ratio()
            .is_some_and(|ratio| ratio >= STREAK_HIT_RATIO)
    }
}

pub fn days(
    spans: &calendar::Spans,
    requests: &[SpentRequest],
    adjustments: &[SpentAdjustment],
    prices: &PriceTable,
) -> Vec<Day> {
    let mut days: Vec<Day> = spans
        .spans
        .iter()
        .map(|span| Day {
            start: span.start,
            label: span.label.clone(),
            ..Default::default()
        })
        .collect();
    for request in requests {
        let Some(index) = spans.index_of(request.point.ts) else {
            continue;
        };
        let day = &mut days[index];
        let tokens = &request.point.tokens;
        day.requests += 1;
        day.cost_usd += request.cost_usd.unwrap_or(0.0);
        day.saved_usd +=
            cache_model::saved_usd(&request.point.model, tokens, prices).unwrap_or(0.0);
        day.read += tokens.cache_read;
        day.prompt += tokens.prompt_total();
    }
    for adjustment in adjustments {
        if let Some(index) = spans.index_of(adjustment.ts) {
            days[index].cost_usd += adjustment.cost_usd.unwrap_or(0.0);
        }
    }
    days
}

pub fn wins(days: &[Day]) -> Wins {
    let Some((today, before)) = days.split_last() else {
        return Wins {
            saved_usd_7d: 0.0,
            hit_streak_days: 0,
        };
    };
    let streak = before
        .iter()
        .rev()
        .filter(|day| day.requests > 0)
        .take_while(|day| day.on_streak())
        .count() as u64;
    Wins {
        saved_usd_7d: days.iter().rev().take(WEEK_DAYS).map(|d| d.saved_usd).sum(),
        hit_streak_days: streak + u64::from(today.on_streak()),
    }
}

pub fn recap(days: &[Day]) -> Option<Recap> {
    let (_, before) = days.split_last()?;
    let index = before.iter().rposition(|day| day.requests > 0)?;
    let day = &before[index];
    let streak = before[..=index]
        .iter()
        .rev()
        .filter(|day| day.requests > 0)
        .take_while(|day| day.on_streak())
        .count() as u64;
    let win = if streak >= STREAK_WIN_DAYS {
        Some(format!(
            "{} in a row above 90% cache hits",
            count(streak, "day", "days")
        ))
    } else if day.saved_usd >= 0.01 {
        Some(format!("cache hits saved you {}", money(day.saved_usd)))
    } else {
        None
    };
    Some(Recap {
        day: match index + 1 == before.len() {
            true => "Yesterday".into(),
            false => day.label.clone(),
        },
        start_ms: day.start * 1000,
        cost_usd: day.cost_usd,
        hit_ratio: day.hit_ratio(),
        saved_usd: day.saved_usd,
        win,
    })
}

fn rates_for(scope: &Scope, fits: &[limits::Fit]) -> Rates {
    match scope.account() {
        Some(account) => Rates::of(&limits::fits_of(fits, Some(account))),
        None => Rates::of(&[]),
    }
}

pub fn read<Tz: TimeZone>(
    store: &Store,
    prices: &PriceTable,
    request: &Request,
    tz: &Tz,
) -> Result<InsightsReport>
where
    Tz::Offset: std::fmt::Display,
{
    let now_s = request.now_ms.div_euclid(1000);
    let spans = calendar::spans(Range::Quarter, now_s, tz)?;
    let since = now_s - WINDOW_DAYS * DAY_S;
    let scope = match store.session_account(request.session)? {
        Some(id) => Scope::Account(id),
        None => Scope::All,
    };
    let kept = scope.sessions(store)?;
    let quarter: Vec<SpentRequest> = store
        .requests_between(spans.since().min(since), spans.until)?
        .into_iter()
        .filter(|r| scope.keeps(&kept, &r.session_id))
        .collect();
    let reconciled: Vec<SpentAdjustment> = store
        .adjustments_between(spans.since(), spans.until)?
        .into_iter()
        .filter(|a| scope.keeps(&kept, &a.session_id))
        .collect();
    let calendar_days = days(&spans, &quarter, &reconciled, prices);
    let requests: Vec<SpentRequest> = quarter
        .into_iter()
        .filter(|r| r.point.ts >= since && r.point.ts < spans.until)
        .collect();
    let sessions: Vec<_> = store
        .sessions_spending_between(since, spans.until)?
        .into_iter()
        .filter(|s| scope.keeps(&kept, &s.id))
        .collect();
    let (rebuilds, compactions) =
        rebuilds(store, prices, &requests, since, spans.until, request.now_ms)?;
    let agent_types = store.agent_types_between(since, spans.until)?;
    let project = store.session_project(request.session)?;
    let live = store
        .live_breakdown(request.session)?
        .and_then(|stored| breakdown::parse(stored.ts_ms, &stored.json));
    let recent: Vec<breakdown::Breakdown> = store
        .recent_breakdowns(context_model::CALIBRATION_BREAKDOWNS)?
        .into_iter()
        .filter_map(|stored| breakdown::parse(stored.ts_ms, &stored.json))
        .collect();
    let ratios = context_model::ratios::calibrate(request.scan, &recent);
    let session_uses = |recent_skills, recent_tools| -> Result<Uses> {
        Ok(Uses {
            session_skills: store.skill_uses_in_session(request.session)?,
            recent_skills,
            session_tools: store.mcp_tool_uses_in_session(request.session)?,
            recent_tools,
        })
    };
    let fortnight = session_uses(
        store.skill_uses_since(since)?,
        store.mcp_tool_uses_since(since)?,
    )?;
    let month_since = now_s - MONTH_DAYS * DAY_S;
    let monthly = session_uses(
        store.skill_uses_since(month_since)?,
        store.mcp_tool_uses_since(month_since)?,
    )?;
    let all = items::items(live.as_ref(), request.scan, &ratios, &fortnight);
    let dead_weight = items::dead_weight(&all, live.as_ref());
    let month_items = items::items(live.as_ref(), request.scan, &ratios, &monthly);
    let memory = memory_texts(request.scan);
    let rates = rates_for(&scope, &limits::stored_fit(store)?);
    let prompts: Vec<PromptPoint> = prompts_of(store, &requests, since, spans.until)?
        .into_iter()
        .filter(|p| scope.keeps(&kept, &p.session_id))
        .collect();
    let tool_results: Vec<_> = store
        .large_tool_results_between(since, spans.until, LARGE_RESULT_CHARS)?
        .into_iter()
        .filter(|r| scope.keeps(&kept, &r.session_id))
        .collect();
    let ctx = InsightContext {
        now_ms: request.now_ms,
        since_ts: since,
        prices,
        rates: &rates,
        requests: &requests,
        agent_types: &agent_types,
        sessions: &sessions,
        project: project.as_deref(),
        prompts: &prompts,
        rebuilds: &rebuilds,
        tool_results: &tool_results,
        compactions: &compactions,
        dead_weight: &dead_weight,
        month_items: &month_items,
        memory: &memory,
        ratios: &ratios,
        retention_days: request.retention_days,
    };
    Ok(InsightsReport {
        v: INSIGHTS_VERSION,
        session: request.session.to_string(),
        generated_at_ms: request.now_ms,
        since_ms: since * 1000,
        requests: requests.iter().filter(|r| is_main(r)).count() as u64,
        sessions: sessions.len() as u64,
        findings: evaluate(&ctx),
        wins: wins(&calendar_days),
        recap: recap(&calendar_days),
    })
}

fn prompts_of(
    store: &Store,
    requests: &[SpentRequest],
    since: i64,
    until: i64,
) -> Result<Vec<PromptPoint>> {
    let mut prompts = store.prompts_between(since, until)?;
    let sessions: BTreeSet<&str> = requests.iter().map(|r| r.session_id.as_str()).collect();
    for session in sessions {
        prompts.extend(store.last_prompt_before(session, since)?);
    }
    prompts.sort_by(|a, b| a.session_id.cmp(&b.session_id).then(a.ts.cmp(&b.ts)));
    Ok(prompts)
}

fn rebuilds(
    store: &Store,
    prices: &PriceTable,
    requests: &[SpentRequest],
    since: i64,
    until: i64,
    now_ms: i64,
) -> Result<(Vec<Rebuilt>, Vec<CompactionPoint>)> {
    let ids: BTreeSet<&str> = requests.iter().map(|r| r.session_id.as_str()).collect();
    let mut rebuilt = Vec::new();
    let mut compactions = Vec::new();
    for session in ids {
        let previous = store.last_main_request_before(session, since)?;
        let from = previous.as_ref().map_or(since, |p| p.ts.min(since));
        let mut points: Vec<_> = previous.into_iter().collect();
        points.extend(
            requests
                .iter()
                .filter(|r| r.session_id == session)
                .map(|r| r.point.clone()),
        );
        let causes = Causes::read(store, session, from, until)?;
        let timeline = Timeline::build(&points, &[]);
        let main = timeline.main();
        let (ttl, _) = cache_model::ttl_of(&Thread::Main, &main, false);
        let found =
            cache_model::rebuilt_requests(&main, ttl, &causes.context(prices, false, now_ms));
        for (request, rebuild) in found.into_iter().filter(|(_, r)| r.ts_ms >= since * 1000) {
            let before = main
                .iter()
                .rev()
                .find(|o| o.ts_ms < rebuild.ts_ms)
                .map_or(rebuild.ts_ms, |o| o.ts_ms);
            let between: Vec<&(PrefixChange, bool)> = causes
                .changes
                .iter()
                .filter(|(k, _)| before < k.ts * 1000 && k.ts * 1000 <= rebuild.ts_ms)
                .collect();
            let names: BTreeSet<String> = between.iter().map(|(k, _)| k.kind.clone()).collect();
            rebuilt.push(Rebuilt {
                session_id: session.to_string(),
                request: request.clone(),
                gap_ms: rebuild.ts_ms - before,
                ttl_ms: ttl.millis(),
                kinds: names.into_iter().collect(),
                mcp_changed: between.iter().any(|(_, changed)| *changed),
                rebuild,
            });
        }
        compactions.extend(causes.compactions.into_iter().filter(|c| c.ts >= since));
    }
    Ok((rebuilt, compactions))
}

fn memory_texts(scan: &Scan) -> Vec<MemoryText> {
    let mut seen = BTreeMap::new();
    for source in &scan.sources {
        let always = matches!(source.kind, SourceKind::ClaudeMd | SourceKind::Memory)
            && source.loading == Loading::Always;
        let Some(path) = source.path.as_deref().filter(|_| always) else {
            continue;
        };
        let Some(text) = config_scan::read_text(Path::new(path)).and_then(|t| t.content) else {
            continue;
        };
        let text = match source.kind {
            SourceKind::Memory => text
                .split_inclusive('\n')
                .take(config_scan::MEMORY_INDEX_LINES)
                .collect(),
            _ => text,
        };
        seen.entry(path.to_string()).or_insert(MemoryText {
            name: source.name.clone(),
            text,
        });
    }
    seen.into_values().collect()
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;
    use crate::config_scan::Roots;
    use crate::insights::RuleId;
    use crate::store::{
        AgentRow, AttachmentRow, AttributionSource, LimitSample, PromptRow, RequestPoint,
        RequestRow, SessionRow,
    };
    use crate::transcript::TokenCounts;

    const OPUS: &str = "claude-opus-5-5";

    fn utc(text: &str) -> i64 {
        DateTime::parse_from_rfc3339(text).unwrap().timestamp()
    }

    fn spent(at: &str, read: u64, write: u64) -> SpentRequest {
        let tokens = TokenCounts {
            input: 10,
            output: 100,
            cache_read: read,
            cache_write_5m: write,
            ..Default::default()
        };
        SpentRequest {
            session_id: "s1".into(),
            cost_usd: PriceTable::bundled().cost(OPUS, &tokens),
            point: RequestPoint {
                ts: utc(at),
                sent: None,
                model: OPUS.into(),
                tokens,
                is_sidechain: false,
                agent_id: None,
                effort: None,
            },
        }
    }

    fn adjusted(at: &str, usd: f64) -> SpentAdjustment {
        SpentAdjustment {
            session_id: "s1".into(),
            ts: utc(at),
            model: OPUS.into(),
            tokens: TokenCounts {
                output: 1_000,
                ..Default::default()
            },
            cost_usd: Some(usd),
        }
    }

    fn month(now: &str, requests: &[SpentRequest]) -> Vec<Day> {
        reconciled_month(now, requests, &[])
    }

    fn reconciled_month(
        now: &str,
        requests: &[SpentRequest],
        adjustments: &[SpentAdjustment],
    ) -> Vec<Day> {
        let spans = calendar::spans(Range::Month, utc(now), &Utc).unwrap();
        days(&spans, requests, adjustments, &PriceTable::bundled())
    }

    #[test]
    fn days_count_reconciled_cost_on_the_local_day_it_was_spread_to() {
        let request = spent("2026-10-10T08:00:00Z", 90_000, 10_000);
        let paid = request.cost_usd.unwrap();
        let days = reconciled_month(
            "2026-10-11T12:00:00Z",
            &[request],
            &[
                adjusted("2026-10-10T08:00:00Z", 1.25),
                adjusted("2026-10-10T23:59:59Z", 0.50),
                adjusted("2026-10-11T00:00:00Z", 2.00),
                adjusted("2026-09-01T00:00:00Z", 9.00),
            ],
        );
        let yesterday = &days[28];
        assert!((yesterday.cost_usd - (paid + 1.75)).abs() < 1e-9);
        assert_eq!(yesterday.requests, 1);
        assert_eq!(yesterday.prompt, 90_000 + 10_000 + 10);
        assert!((days[29].cost_usd - 2.0).abs() < 1e-9);
        assert_eq!(days[29].requests, 0);
    }

    #[test]
    fn days_add_up_cost_savings_and_hits_per_local_day() {
        let days = month(
            "2026-10-11T12:00:00Z",
            &[
                spent("2026-10-10T08:00:00Z", 90_000, 10_000),
                spent("2026-10-10T23:59:59Z", 90_000, 10_000),
                spent("2026-10-11T00:00:00Z", 0, 50_000),
                spent("2026-09-01T00:00:00Z", 0, 50_000),
            ],
        );
        assert_eq!(days.len(), 30);
        let yesterday = &days[28];
        assert_eq!(
            (yesterday.label.as_str(), yesterday.requests),
            ("Oct 10", 2)
        );
        assert_eq!(yesterday.read, 180_000);
        assert!((yesterday.saved_usd - 2.0 * 90_000.0 * 3.8 / 1e6).abs() < 1e-9);
        assert_eq!(days[29].requests, 1);
        assert_eq!(days.iter().map(|d| d.requests).sum::<u64>(), 3);
    }

    #[test]
    fn the_hit_streak_skips_idle_days_and_counts_today_only_once_it_qualifies() {
        let mut requests = vec![spent("2026-10-03T10:00:00Z", 10_000, 90_000)];
        for day in ["05", "06", "08", "09", "10"] {
            requests.push(spent(&format!("2026-10-{day}T10:00:00Z"), 95_000, 5_000));
        }
        requests.push(spent("2026-10-11T10:00:00Z", 10_000, 90_000));
        let wins_today_low = wins(&month("2026-10-11T12:00:00Z", &requests));
        assert_eq!(wins_today_low.hit_streak_days, 5);
        requests.push(spent("2026-10-11T11:00:00Z", 2_000_000, 0));
        let wins_today_high = wins(&month("2026-10-11T12:00:00Z", &requests));
        assert_eq!(wins_today_high.hit_streak_days, 6);
        let week: f64 = requests[1..]
            .iter()
            .map(|r| r.point.tokens.cache_read as f64 * 3.8 / 1e6)
            .sum();
        assert!((wins_today_high.saved_usd_7d - week).abs() < 1e-9);
    }

    #[test]
    fn the_recap_greets_with_the_last_active_day_and_its_biggest_win() {
        let streak: Vec<SpentRequest> = ["08", "09", "10"]
            .iter()
            .map(|day| spent(&format!("2026-10-{day}T10:00:00Z"), 95_000, 5_000))
            .collect();
        let recap_of = |requests: &[SpentRequest]| recap(&month("2026-10-11T12:00:00Z", requests));
        let with_streak = recap_of(&streak).unwrap();
        assert_eq!(with_streak.day, "Yesterday");
        assert_eq!(with_streak.start_ms, utc("2026-10-10T00:00:00Z") * 1000);
        assert_eq!(
            with_streak.win.as_deref(),
            Some("3\u{a0}days in a row above 90% cache hits")
        );
        let older = recap_of(&[spent("2026-10-07T10:00:00Z", 50_000, 50_000)]).unwrap();
        assert_eq!(older.day, "Oct 7");
        assert_eq!(older.hit_ratio, Some(50_000.0 / 100_010.0));
        assert_eq!(older.win.as_deref(), Some("cache hits saved you $0.19"));
        let cold = recap_of(&[spent("2026-10-07T10:00:00Z", 0, 50_000)]).unwrap();
        assert_eq!(cold.win, None);
        assert_eq!(recap_of(&[spent("2026-10-11T10:00:00Z", 0, 50_000)]), None);
        assert_eq!(recap_of(&[]), None);
    }

    #[test]
    fn the_report_reads_the_last_two_weeks_from_the_store_and_ranks_its_findings() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("claude")).unwrap();
        std::fs::create_dir_all(temp.path().join("work")).unwrap();
        std::fs::write(
            temp.path().join("work/CLAUDE.md"),
            "Use the store for every read.\n",
        )
        .unwrap();
        let scan = config_scan::scan(&Roots {
            home: temp.path().join("home"),
            claude_dir: temp.path().join("claude"),
            claude_json: temp.path().join("claude.json"),
            project: Some(temp.path().join("work")),
        });
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.session(&SessionRow {
            id: "s1".into(),
            project: "-work".into(),
            cwd: Some(temp.path().join("work").to_string_lossy().into_owned()),
            ts: Some(utc("2026-10-10T09:00:00Z")),
        })
        .unwrap();
        for (key, request) in [
            ("r1", spent("2026-10-10T10:00:00Z", 0, 20_000)),
            ("r1b", spent("2026-10-10T10:00:30Z", 20_000, 80_000)),
            ("r2", spent("2026-10-10T10:20:30Z", 0, 100_000)),
            ("r3", spent("2026-10-10T10:21:00Z", 100_000, 0)),
            ("r0", spent("2026-09-20T10:00:00Z", 0, 100_000)),
        ] {
            w.request(&RequestRow {
                key: key.into(),
                session_id: if key == "r0" {
                    "s0".into()
                } else {
                    request.session_id
                },
                ts: request.point.ts,
                sent_ts: None,
                model: request.point.model,
                tokens: request.point.tokens,
                cost_usd: request.cost_usd,
                is_sidechain: false,
                agent_id: None,
                skill: None,
                mcp_server: None,
                effort: None,
            })
            .unwrap();
        }
        for (uuid, at) in [
            ("p1", "2026-10-10T09:59:59Z"),
            ("p2", "2026-10-10T10:19:00Z"),
        ] {
            w.prompt(&PromptRow {
                uuid: uuid.into(),
                session_id: "s1".into(),
                ts: utc(at),
            })
            .unwrap();
        }
        w.adjustment(&SpentAdjustment {
            session_id: "s1".into(),
            ..adjusted("2026-10-10T10:30:00Z", 0.84)
        })
        .unwrap();
        w.commit().unwrap();
        let paid: f64 = [
            spent("2026-10-10T10:00:00Z", 0, 20_000),
            spent("2026-10-10T10:00:30Z", 20_000, 80_000),
            spent("2026-10-10T10:20:30Z", 0, 100_000),
            spent("2026-10-10T10:21:00Z", 100_000, 0),
        ]
        .iter()
        .map(|r| r.cost_usd.unwrap())
        .sum();
        let report = read(
            &store,
            &PriceTable::bundled(),
            &Request {
                session: "s1",
                scan: &scan,
                retention_days: None,
                now_ms: utc("2026-10-11T12:00:00Z") * 1000,
            },
            &Utc,
        )
        .unwrap();
        assert_eq!((report.v, report.requests, report.sessions), (1, 4, 1));
        assert_eq!(report.since_ms, utc("2026-09-27T12:00:00Z") * 1000);
        let rules: Vec<RuleId> = report.findings.iter().map(|f| f.rule).collect();
        assert_eq!(
            rules,
            [RuleId::ModelFit, RuleId::IdleRebuilds, RuleId::Retention]
        );
        assert_eq!(
            report.findings[1].evidence[1],
            "Typical break 20m, the cache lasts 5m"
        );
        assert!(
            (report.findings[1].saving_usd.unwrap() - (100_000.0 - 20_000.0) * 4.8 / 1e6 / 2.0)
                .abs()
                < 1e-9
        );
        let recap = report.recap.unwrap();
        assert_eq!(recap.day, "Yesterday");
        assert!((recap.cost_usd - (paid + 0.84)).abs() < 1e-9);
        assert_eq!(recap.win.as_deref(), Some("cache hits saved you $0.46"));
        assert_eq!(report.wins.hit_streak_days, 0);
    }

    fn rebuilt_after(announcements: &[(&str, &str, &[&str])]) -> Rebuilt {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        for (key, request) in [
            ("r1", spent("2026-10-10T10:00:00Z", 0, 100_000)),
            ("r2", spent("2026-10-10T10:02:00Z", 0, 100_000)),
        ] {
            w.request(&RequestRow {
                key: key.into(),
                session_id: request.session_id,
                ts: request.point.ts,
                sent_ts: None,
                model: request.point.model,
                tokens: request.point.tokens,
                cost_usd: request.cost_usd,
                is_sidechain: false,
                agent_id: None,
                skill: None,
                mcp_server: None,
                effort: None,
            })
            .unwrap();
        }
        let main = w.file("/p/-p/s1.jsonl", 0, 0).unwrap();
        let agent = w.file("/p/-p/s1/subagents/agent-a.jsonl", 0, 0).unwrap();
        for (line_no, (at, file, names)) in [("2026-10-10T09:59:00Z", "main", &["mcp__a__x"][..])]
            .into_iter()
            .chain(announcements.iter().copied())
            .enumerate()
        {
            w.attachment(&AttachmentRow {
                file_id: if file == "main" { main } else { agent },
                line_no: line_no as u64,
                session_id: "s1".into(),
                ts: Some(utc(at)),
                kind: "deferred_tools_delta".into(),
                chars: 10,
                names: names.iter().map(|n| n.to_string()).collect(),
                removed: vec![],
            })
            .unwrap();
        }
        w.commit().unwrap();
        let since = utc("2026-10-01T00:00:00Z");
        let until = utc("2026-10-11T00:00:00Z");
        let requests = store.requests_between(since, until).unwrap();
        let (mut found, _) = rebuilds(
            &store,
            &PriceTable::bundled(),
            &requests,
            since,
            until,
            until * 1000,
        )
        .unwrap();
        assert_eq!(found.len(), 1);
        found.remove(0)
    }

    #[test]
    fn a_bust_is_an_mcp_change_only_when_the_main_thread_announced_new_tools_between_requests() {
        let again = rebuilt_after(&[("2026-10-10T10:01:00Z", "main", &["mcp__a__x"])]);
        assert_eq!(again.kinds, ["deferred_tools_delta"]);
        assert!(!again.mcp_changed);
        let subagent = rebuilt_after(&[("2026-10-10T10:01:00Z", "agent", &["mcp__b__y"])]);
        assert!(subagent.kinds.is_empty());
        assert_eq!(subagent.rebuild.cause, cache_model::RebuildCause::Unknown);
        let added = rebuilt_after(&[("2026-10-10T10:01:00Z", "main", &["mcp__b__y"])]);
        assert!(added.mcp_changed);
    }

    #[test]
    fn the_idle_rebuild_saving_leaves_out_the_smallest_context_when_requests_were_sent_before_they_ended()
     {
        let temp = tempfile::tempdir().unwrap();
        let scan = config_scan::scan(&Roots {
            home: temp.path().join("home"),
            claude_dir: temp.path().join("claude"),
            claude_json: temp.path().join("claude.json"),
            project: None,
        });
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        for (key, request) in [
            ("r1", spent("2026-10-10T10:00:00Z", 0, 20_000)),
            ("r2", spent("2026-10-10T10:00:30Z", 20_000, 80_000)),
            ("r3", spent("2026-10-10T10:20:30Z", 0, 100_000)),
            ("r4", spent("2026-10-10T10:21:00Z", 100_000, 0)),
        ] {
            w.request(&RequestRow {
                key: key.into(),
                session_id: request.session_id,
                ts: request.point.ts,
                sent_ts: Some(request.point.ts - 6),
                model: request.point.model,
                tokens: request.point.tokens,
                cost_usd: request.cost_usd,
                is_sidechain: false,
                agent_id: None,
                skill: None,
                mcp_server: None,
                effort: None,
            })
            .unwrap();
        }
        w.commit().unwrap();
        let report = read(
            &store,
            &PriceTable::bundled(),
            &Request {
                session: "s1",
                scan: &scan,
                retention_days: Some(90),
                now_ms: utc("2026-10-11T12:00:00Z") * 1000,
            },
            &Utc,
        )
        .unwrap();
        let idle = report
            .findings
            .iter()
            .find(|f| f.rule == RuleId::IdleRebuilds)
            .unwrap();
        let saving = idle.saving_usd.unwrap();
        assert!(
            (saving - (100_000.0 - 20_000.0) * 4.8 / 1e6 / 2.0).abs() < 1e-9,
            "{saving}"
        );
    }

    #[test]
    fn insights_count_only_the_sessions_of_the_requesting_session_account() {
        let temp = tempfile::tempdir().unwrap();
        let scan = config_scan::scan(&Roots {
            home: temp.path().join("home"),
            claude_dir: temp.path().join("claude"),
            claude_json: temp.path().join("claude.json"),
            project: None,
        });
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let spending = [
            ("s1", "oauth:me", &["2026-10-10T10:00:00Z"][..]),
            (
                "s2",
                "oauth:work",
                &["2026-10-10T11:00:00Z", "2026-10-10T11:01:00Z"][..],
            ),
        ];
        for (session, account, times) in spending {
            w.session(&SessionRow {
                id: session.into(),
                project: "-work".into(),
                cwd: None,
                ts: Some(utc("2026-10-10T09:00:00Z")),
            })
            .unwrap();
            w.attribute(session, account, AttributionSource::Start)
                .unwrap();
            for (index, at) in times.iter().enumerate() {
                let request = spent(at, 0, 1_000);
                w.request(&RequestRow {
                    key: format!("{session}-{index}"),
                    session_id: session.into(),
                    ts: request.point.ts,
                    sent_ts: None,
                    model: request.point.model,
                    tokens: request.point.tokens,
                    cost_usd: request.cost_usd,
                    is_sidechain: false,
                    agent_id: None,
                    skill: None,
                    mcp_server: None,
                    effort: None,
                })
                .unwrap();
            }
        }
        w.commit().unwrap();
        let insights = |session: &str| {
            read(
                &store,
                &PriceTable::bundled(),
                &Request {
                    session,
                    scan: &scan,
                    retention_days: Some(90),
                    now_ms: utc("2026-10-11T12:00:00Z") * 1000,
                },
                &Utc,
            )
            .unwrap()
        };
        let one = spent("2026-10-10T10:00:00Z", 0, 1_000).cost_usd.unwrap();
        let mine = insights("s1");
        assert_eq!((mine.requests, mine.sessions), (1, 1));
        assert!((mine.recap.unwrap().cost_usd - one).abs() < 1e-9);
        let work = insights("s2");
        assert_eq!((work.requests, work.sessions), (2, 1));
        let unattributed = insights("s3");
        assert_eq!((unattributed.requests, unattributed.sessions), (3, 2));
    }

    #[test]
    fn an_unattributed_session_gets_no_five_hour_share_even_with_unattributed_fits() {
        let fit = |account: Option<&str>, pct_per_usd| limits::Fit {
            account: account.map(String::from),
            window: "five_hour".into(),
            family: "opus".into(),
            pairs: 30,
            pct_per_usd,
        };
        let fits = [fit(None, 2.0), fit(Some("oauth:me"), 3.0)];
        let all = rates_for(&Scope::All, &fits);
        assert_eq!(all.pct("claude-opus-5-5", Some(1.0)), None);
        let mine = rates_for(&Scope::Account("oauth:me".into()), &fits);
        assert_eq!(mine.pct("claude-opus-5-5", Some(1.0)), Some(3.0));
    }

    #[test]
    fn the_hit_streak_is_read_past_a_month() {
        let temp = tempfile::tempdir().unwrap();
        let scan = config_scan::scan(&Roots {
            home: temp.path().join("home"),
            claude_dir: temp.path().join("claude"),
            claude_json: temp.path().join("claude.json"),
            project: None,
        });
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let now = utc("2026-10-11T12:00:00Z");
        for day in 1..=40 {
            let request = spent("2026-10-11T10:00:00Z", 95_000, 5_000);
            w.request(&RequestRow {
                key: format!("r{day}"),
                session_id: request.session_id,
                ts: request.point.ts - day * DAY_S,
                sent_ts: None,
                model: request.point.model,
                tokens: request.point.tokens,
                cost_usd: request.cost_usd,
                is_sidechain: false,
                agent_id: None,
                skill: None,
                mcp_server: None,
                effort: None,
            })
            .unwrap();
        }
        w.commit().unwrap();
        let report = read(
            &store,
            &PriceTable::bundled(),
            &Request {
                session: "s1",
                scan: &scan,
                retention_days: Some(90),
                now_ms: now * 1000,
            },
            &Utc,
        )
        .unwrap();
        assert_eq!(report.wins.hit_streak_days, 40);
    }

    #[test]
    fn a_task_that_started_before_the_window_keeps_its_requests_in_it() {
        let temp = tempfile::tempdir().unwrap();
        let scan = config_scan::scan(&Roots {
            home: temp.path().join("home"),
            claude_dir: temp.path().join("claude"),
            claude_json: temp.path().join("claude.json"),
            project: None,
        });
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let now = utc("2026-10-11T12:00:00Z");
        w.prompt(&PromptRow {
            uuid: "p1".into(),
            session_id: "s1".into(),
            ts: now - 15 * DAY_S,
        })
        .unwrap();
        for (key, minutes) in [("r1", 1), ("r2", 2)] {
            let tokens = TokenCounts {
                input: 200_000,
                output: 500,
                ..Default::default()
            };
            w.request(&RequestRow {
                key: key.into(),
                session_id: "s1".into(),
                ts: now - DAY_S + minutes * 60,
                sent_ts: None,
                model: OPUS.into(),
                cost_usd: PriceTable::bundled().cost(OPUS, &tokens),
                tokens,
                is_sidechain: false,
                agent_id: None,
                skill: None,
                mcp_server: None,
                effort: None,
            })
            .unwrap();
        }
        w.commit().unwrap();
        let report = read(
            &store,
            &PriceTable::bundled(),
            &Request {
                session: "s1",
                scan: &scan,
                retention_days: Some(90),
                now_ms: now * 1000,
            },
            &Utc,
        )
        .unwrap();
        let rules: Vec<RuleId> = report.findings.iter().map(|f| f.rule).collect();
        assert_eq!(rules, [RuleId::ModelFit]);
    }

    fn account_store(subagent_type: Option<&str>, limits: &[(&str, &str, f64)]) -> Store {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.session(&SessionRow {
            id: "s1".into(),
            project: "-work".into(),
            cwd: None,
            ts: Some(utc("2026-10-10T09:00:00Z")),
        })
        .unwrap();
        w.attribute("s1", "oauth:me", AttributionSource::Start)
            .unwrap();
        if let Some(agent_type) = subagent_type {
            w.agent(&AgentRow {
                agent_id: "a1".into(),
                session_id: "s1".into(),
                agent_type: agent_type.into(),
            })
            .unwrap();
        }
        let tokens = TokenCounts {
            input: 1_000_000,
            ..Default::default()
        };
        w.request(&RequestRow {
            key: "r1".into(),
            session_id: "s1".into(),
            ts: utc("2026-10-10T09:30:00Z"),
            sent_ts: None,
            model: OPUS.into(),
            cost_usd: PriceTable::bundled().cost(OPUS, &tokens),
            tokens,
            is_sidechain: true,
            agent_id: Some("a1".into()),
            skill: None,
            mcp_server: None,
            effort: None,
        })
        .unwrap();
        for (account, at, used_pct) in limits {
            w.limit_sample(&LimitSample {
                ts_ms: utc(at) * 1000,
                session_id: "s1".into(),
                account_id: Some(account.to_string()),
                model: Some(OPUS.into()),
                window: "five_hour".into(),
                used_pct: *used_pct,
                resets_at_ms: Some(utc("2026-10-10T11:00:00Z") * 1000),
            })
            .unwrap();
        }
        w.commit().unwrap();
        store
    }

    fn report_for(store: &Store, session: &str) -> InsightsReport {
        let temp = tempfile::tempdir().unwrap();
        let scan = config_scan::scan(&Roots {
            home: temp.path().join("home"),
            claude_dir: temp.path().join("claude"),
            claude_json: temp.path().join("claude.json"),
            project: None,
        });
        read(
            store,
            &PriceTable::bundled(),
            &Request {
                session,
                scan: &scan,
                retention_days: Some(90),
                now_ms: utc("2026-10-11T12:00:00Z") * 1000,
            },
            &Utc,
        )
        .unwrap()
    }

    #[test]
    fn the_report_names_subagents_by_the_type_the_store_keeps() {
        let typed = report_for(&account_store(Some("Explore"), &[]), "s1");
        let rules: Vec<RuleId> = typed.findings.iter().map(|f| f.rule).collect();
        assert_eq!(rules, [RuleId::SubagentModelFit]);
        assert_eq!(
            typed.findings[0].tip,
            "run Explore subagents on Sonnet\u{a0}5.5"
        );
        let untyped = report_for(&account_store(None, &[]), "s1");
        assert_eq!(untyped.findings[0].tip, "run subagents on Sonnet\u{a0}5.5");
    }
}
