use chrono::{DateTime, Utc};
use serde_json::json;

use super::*;
use crate::store::{
    AccountKind, AccountRow, AttributionSource, CompactionRow, RequestRow, SessionRow,
};

const OPUS: &str = "claude-opus-5-5";
const ME: &str = "oauth:me";
const WORK: &str = "oauth:work";

fn now_ms() -> i64 {
    utc("2026-10-11T09:00:00Z") * 1000
}

fn me() -> Scope {
    Scope::Account(ME.into())
}

fn account(w: &mut crate::store::Writer, id: &str, label: &str, first_seen: &str) {
    w.upsert_account(
        &AccountRow {
            id: id.into(),
            kind: AccountKind::Oauth,
            label: label.into(),
            organization_type: None,
            billing_type: None,
        },
        utc(first_seen) * 1000,
    )
    .unwrap();
}

fn utc(text: &str) -> i64 {
    DateTime::parse_from_rfc3339(text).unwrap().timestamp()
}

struct Spent {
    key: &'static str,
    session: &'static str,
    at: &'static str,
    model: &'static str,
    agent: Option<&'static str>,
    tokens: TokenCounts,
    cost_usd: Option<f64>,
}

fn tokens(input: u64, output: u64, read: u64, write_5m: u64, write_1h: u64) -> TokenCounts {
    TokenCounts {
        input,
        output,
        thinking: 0,
        cache_read: read,
        cache_write_5m: write_5m,
        cache_write_1h: write_1h,
    }
}

pub(super) fn fixture() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let mut w = store.writer().unwrap();
    for (id, project, cwd) in [
        ("s1", "-work-app", "/work/app"),
        ("s2", "-work-shop", "/work/shop"),
    ] {
        w.session(&SessionRow {
            id: id.into(),
            project: project.into(),
            cwd: Some(cwd.into()),
            ts: Some(utc("2026-10-01T00:00:00Z")),
        })
        .unwrap();
    }
    w.title("s1", "Fix the cache countdown", false).unwrap();
    account(&mut w, ME, "Me", "2026-09-01T00:00:00Z");
    for session in ["s1", "s2"] {
        w.attribute(session, ME, AttributionSource::Start).unwrap();
    }
    let spent = [
        Spent {
            key: "r0",
            session: "s1",
            at: "2026-10-03T10:00:00Z",
            model: OPUS,
            agent: None,
            tokens: tokens(10, 500, 0, 0, 50_000),
            cost_usd: Some(1.0),
        },
        Spent {
            key: "r1",
            session: "s1",
            at: "2026-10-10T10:00:00Z",
            model: OPUS,
            agent: None,
            tokens: tokens(10, 1_000, 0, 0, 50_000),
            cost_usd: Some(2.0),
        },
        Spent {
            key: "r2",
            session: "s1",
            at: "2026-10-10T10:05:00Z",
            model: OPUS,
            agent: None,
            tokens: tokens(10, 500, 50_000, 0, 1_000),
            cost_usd: Some(0.5),
        },
        Spent {
            key: "r3",
            session: "s1",
            at: "2026-10-10T12:00:00Z",
            model: OPUS,
            agent: None,
            tokens: tokens(10, 500, 0, 0, 51_000),
            cost_usd: Some(2.0),
        },
        Spent {
            key: "r4",
            session: "s1",
            at: "2026-10-11T08:00:00Z",
            model: "claude-haiku-5-5",
            agent: Some("a1"),
            tokens: tokens(2_000, 100, 0, 0, 0),
            cost_usd: Some(0.25),
        },
        Spent {
            key: "r5",
            session: "s2",
            at: "2026-10-06T09:00:00Z",
            model: "claude-sonnet-5-5",
            agent: None,
            tokens: tokens(100, 400, 0, 8_000, 0),
            cost_usd: Some(1.0),
        },
        Spent {
            key: "r6",
            session: "s2",
            at: "2026-10-06T09:01:00Z",
            model: "claude-mystery-1",
            agent: None,
            tokens: tokens(100, 100, 0, 0, 0),
            cost_usd: None,
        },
    ];
    for row in spent {
        w.request(&RequestRow {
            key: row.key.into(),
            session_id: row.session.into(),
            ts: utc(row.at),
            sent_ts: None,
            model: row.model.into(),
            tokens: row.tokens,
            cost_usd: row.cost_usd,
            is_sidechain: row.agent.is_some(),
            agent_id: row.agent.map(String::from),
            skill: None,
            mcp_server: None,
            effort: None,
        })
        .unwrap();
    }
    w.adjustment(&SpentAdjustment {
        session_id: "s2".into(),
        ts: utc("2026-10-06T10:00:00Z"),
        model: "claude-sonnet-5-5".into(),
        tokens: tokens(0, 100, 1_000, 0, 0),
        cost_usd: Some(0.25),
    })
    .unwrap();
    let main = w.file("/p/-work-app/s1.jsonl", 0, 0).unwrap();
    w.compaction(&CompactionRow {
        file_id: main,
        line_no: 7,
        session_id: "s1".into(),
        ts: Some(utc("2026-10-10T13:00:00Z")),
        trigger: Some("auto".into()),
        pre_tokens: None,
        post_tokens: None,
    })
    .unwrap();
    for (at, used_pct) in [
        ("2026-10-10T10:05:00Z", 12.0),
        ("2026-10-10T12:00:00Z", 30.5),
        ("2026-10-11T08:00:00Z", 4.0),
    ] {
        w.limit_sample(&LimitSample {
            ts_ms: utc(at) * 1000,
            session_id: "s1".into(),
            account_id: Some(ME.into()),
            model: Some(OPUS.into()),
            window: FIVE_HOUR.into(),
            used_pct,
            resets_at_ms: None,
        })
        .unwrap();
    }
    w.set_meta(
        limits::FIT_KEY,
        r#"[{"account":"oauth:me","window":"five_hour","family":"opus","pairs":25,"pct_per_usd":2.0},{"account":"oauth:me","window":"five_hour","family":"sonnet","pairs":5,"pct_per_usd":9.0}]"#,
    )
    .unwrap();
    w.commit().unwrap();
    store
}

fn report(range: Range) -> HistoryReport {
    read(
        &fixture(),
        &PriceTable::bundled(),
        range,
        now_ms(),
        &Utc,
        &me(),
    )
    .unwrap()
}

fn rounded(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Number(number) if number.is_f64() => {
            json!((number.as_f64().unwrap() * 1e6).round() / 1e6)
        }
        serde_json::Value::Array(items) => items.into_iter().map(rounded).collect(),
        serde_json::Value::Object(fields) => fields
            .into_iter()
            .map(|(key, value)| (key, rounded(value)))
            .collect(),
        other => other,
    }
}

fn quiet_day(start_ms: i64, label: &str) -> serde_json::Value {
    json!({"start_ms": start_ms, "label": label, "requests": 0, "unpriced": 0, "cost_usd": 0.0, "reconciled_usd": 0.0, "tokens": 0, "pct_5h": 0.0, "models": [], "prompt_tokens": 0, "cache_read_tokens": 0, "rebuilds": 0, "rebuild_usd": 0.0, "compactions": 0, "peak_5h_pct": null})
}

#[test]
fn the_week_report_matches_the_contract() {
    let day = 86_400_000;
    let oct5 = utc("2026-10-05T00:00:00Z") * 1000;
    assert_eq!(
        rounded(serde_json::to_value(report(Range::Week)).unwrap()),
        json!({
            "v": 1,
            "range": "7d",
            "bucket": "day",
            "generated_at_ms": utc("2026-10-11T09:00:00Z") * 1000,
            "since_ms": oct5,
            "until_ms": oct5 + 7 * day,
            "account": "oauth:me",
            "accounts": [{"id": "oauth:me", "label": "Me", "kind": "oauth"}],
            "calibration": {"calibrated": true, "pairs": 25, "needed": 20},
            "totals": {"requests": 6, "unpriced": 1, "cost_usd": 6.0, "reconciled_usd": 0.25, "tokens": 165_930, "pct_5h": 12.0, "sessions": 2, "hit_ratio": 0.308204, "saved_usd": 0.19, "rebuilds": 2, "rebuild_usd": 0.7878, "idle_rebuilds": 2, "compactions": 1, "peak_5h_pct": 30.5},
            "buckets": [
                quiet_day(oct5, "Oct 5"),
                {"start_ms": oct5 + day, "label": "Oct 6", "requests": 2, "unpriced": 1, "cost_usd": 1.25, "reconciled_usd": 0.25, "tokens": 9_800, "pct_5h": 2.5, "models": [
                    {"model": "claude-sonnet-5-5", "requests": 1, "unpriced": 0, "cost_usd": 1.25, "reconciled_usd": 0.25, "tokens": 9_600, "pct_5h": 2.5},
                    {"model": "claude-mystery-1", "requests": 1, "unpriced": 1, "cost_usd": 0.0, "reconciled_usd": 0.0, "tokens": 200, "pct_5h": 0.0}
                ], "prompt_tokens": 8_200, "cache_read_tokens": 0, "rebuilds": 0, "rebuild_usd": 0.0, "compactions": 0, "peak_5h_pct": null},
                quiet_day(oct5 + 2 * day, "Oct 7"),
                quiet_day(oct5 + 3 * day, "Oct 8"),
                quiet_day(oct5 + 4 * day, "Oct 9"),
                {"start_ms": oct5 + 5 * day, "label": "Oct 10", "requests": 3, "unpriced": 0, "cost_usd": 4.5, "reconciled_usd": 0.0, "tokens": 154_030, "pct_5h": 9.0, "models": [
                    {"model": "claude-opus-5-5", "requests": 3, "unpriced": 0, "cost_usd": 4.5, "reconciled_usd": 0.0, "tokens": 154_030, "pct_5h": 9.0}
                ], "prompt_tokens": 152_030, "cache_read_tokens": 50_000, "rebuilds": 2, "rebuild_usd": 0.7878, "compactions": 1, "peak_5h_pct": 30.5},
                {"start_ms": oct5 + 6 * day, "label": "Oct 11", "requests": 1, "unpriced": 0, "cost_usd": 0.25, "reconciled_usd": 0.0, "tokens": 2_100, "pct_5h": 0.5, "models": [
                    {"model": "claude-haiku-5-5", "requests": 1, "unpriced": 0, "cost_usd": 0.25, "reconciled_usd": 0.0, "tokens": 2_100, "pct_5h": 0.5}
                ], "prompt_tokens": 2_000, "cache_read_tokens": 0, "rebuilds": 0, "rebuild_usd": 0.0, "compactions": 0, "peak_5h_pct": 4.0}
            ],
            "models": [
                {"model": "claude-opus-5-5", "requests": 3, "unpriced": 0, "cost_usd": 4.5, "reconciled_usd": 0.0, "tokens": 154_030, "pct_5h": 9.0},
                {"model": "claude-sonnet-5-5", "requests": 1, "unpriced": 0, "cost_usd": 1.25, "reconciled_usd": 0.25, "tokens": 9_600, "pct_5h": 2.5},
                {"model": "claude-haiku-5-5", "requests": 1, "unpriced": 0, "cost_usd": 0.25, "reconciled_usd": 0.0, "tokens": 2_100, "pct_5h": 0.5},
                {"model": "claude-mystery-1", "requests": 1, "unpriced": 1, "cost_usd": 0.0, "reconciled_usd": 0.0, "tokens": 200, "pct_5h": 0.0}
            ],
            "projects": [
                {"name": "app", "path": "/work/app", "sessions": 1, "requests": 4, "unpriced": 0, "cost_usd": 4.75, "reconciled_usd": 0.0, "tokens": 156_130, "pct_5h": 9.5},
                {"name": "shop", "path": "/work/shop", "sessions": 1, "requests": 2, "unpriced": 1, "cost_usd": 1.25, "reconciled_usd": 0.25, "tokens": 9_800, "pct_5h": 2.5}
            ],
            "sessions": [
                {"id": "s1", "title": "Fix the cache countdown", "project": "app", "last_ms": utc("2026-10-11T08:00:00Z") * 1000, "requests": 4, "unpriced": 0, "cost_usd": 4.75, "reconciled_usd": 0.0, "tokens": 156_130, "pct_5h": 9.5},
                {"id": "s2", "title": null, "project": "shop", "last_ms": utc("2026-10-06T10:00:00Z") * 1000, "requests": 2, "unpriced": 1, "cost_usd": 1.25, "reconciled_usd": 0.25, "tokens": 9_800, "pct_5h": 2.5}
            ]
        })
    );
}

#[test]
fn a_day_counts_hours_and_leaves_older_requests_out() {
    let report = report(Range::Day);
    assert_eq!(
        (report.bucket, report.buckets.len()),
        (BucketSize::Hour, 24)
    );
    assert_eq!(
        (
            report.buckets[0].label.as_str(),
            report.buckets[23].label.as_str()
        ),
        ("10:00", "09:00")
    );
    let busy: Vec<(usize, u64)> = report
        .buckets
        .iter()
        .enumerate()
        .filter(|(_, bucket)| bucket.spend.requests > 0)
        .map(|(index, bucket)| (index, bucket.spend.requests))
        .collect();
    assert_eq!(busy, [(0, 2), (2, 1), (22, 1)]);
    assert_eq!(
        (
            report.totals.spend.cost_usd,
            report.totals.rebuilds,
            report.totals.sessions
        ),
        (4.75, 2, 1)
    );
    assert_eq!(report.buckets[3].compactions, 1);
}

#[test]
fn without_a_calibrated_five_hour_fit_no_percent_is_shown() {
    let mut store = fixture();
    let mut w = store.writer().unwrap();
    w.set_meta(
        limits::FIT_KEY,
        r#"[{"account":"oauth:me","window":"five_hour","family":"opus","pairs":12,"pct_per_usd":2.0}]"#,
    )
    .unwrap();
    w.commit().unwrap();
    let report = read(
        &store,
        &PriceTable::bundled(),
        Range::Week,
        now_ms(),
        &Utc,
        &me(),
    )
    .unwrap();
    assert_eq!(
        report.calibration,
        Calibration {
            calibrated: false,
            pairs: 12,
            needed: 20
        }
    );
    assert_eq!(report.totals.spend.pct_5h, None);
    assert!(
        report
            .buckets
            .iter()
            .all(|bucket| bucket.spend.pct_5h.is_none())
    );
    assert!(
        report
            .models
            .iter()
            .all(|model| model.spend.pct_5h.is_none())
    );
}

#[test]
fn an_empty_store_reports_every_span_with_nothing_in_it() {
    let report = read(
        &Store::open_in_memory().unwrap(),
        &PriceTable::bundled(),
        Range::Month,
        now_ms(),
        &Utc,
        &Scope::All,
    )
    .unwrap();
    assert_eq!(report.buckets.len(), 30);
    assert_eq!(
        (
            report.totals.spend.requests,
            report.totals.hit_ratio,
            report.totals.peak_5h_pct
        ),
        (0, None, None)
    );
    assert!(report.models.is_empty() && report.projects.is_empty() && report.sessions.is_empty());
    assert!(!report.calibration.calibrated);
}

#[test]
fn projects_with_the_same_folder_name_show_their_parent_too() {
    let info = |id: &str, project: &str, cwd: Option<&str>| SessionInfo {
        id: id.into(),
        project: project.into(),
        cwd: cwd.map(String::from),
        title: None,
    };
    let names = project_names(&[
        info("a", "-work-app", Some("/work/app")),
        info("b", "-home-app", Some("/home/app/")),
        info("c", "-work-shop", Some("/work/shop")),
        info("d", "-gone", None),
    ]);
    assert_eq!(names["-work-app"].0, "work/app");
    assert_eq!(names["-home-app"].0, "home/app");
    assert_eq!(
        names["-work-shop"],
        ("shop".to_string(), Some("/work/shop".to_string()))
    );
    assert!(!names.contains_key("-gone"));
}

#[test]
fn ranges_read_and_write_as_the_api_names_them() {
    assert_eq!(Range::ALL.map(Range::as_str), ["24h", "7d", "30d", "90d"]);
    assert_eq!(Range::parse("30d"), Some(Range::Month));
    assert_eq!(Range::parse("1y"), None);
    assert_eq!(serde_json::to_string(&Range::Quarter).unwrap(), r#""90d""#);
}

fn two_account_store() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let mut w = store.writer().unwrap();
    account(&mut w, "oauth:gone", "Gone", "2026-01-01T00:00:00Z");
    account(&mut w, ME, "Me", "2026-09-01T00:00:00Z");
    account(&mut w, WORK, "Work", "2026-09-15T00:00:00Z");
    let spent = [
        (
            "mine",
            ME,
            ["2026-10-06T10:00:00Z", "2026-10-07T10:00:00Z"].as_slice(),
        ),
        (
            "work",
            WORK,
            [
                "2026-10-08T10:00:00Z",
                "2026-10-09T10:00:00Z",
                "2026-10-10T10:00:00Z",
            ]
            .as_slice(),
        ),
    ];
    for (session, id, times) in spent {
        w.session(&SessionRow {
            id: session.into(),
            project: format!("-{session}"),
            cwd: Some(format!("/{session}")),
            ts: Some(utc("2026-10-06T00:00:00Z")),
        })
        .unwrap();
        w.attribute(session, id, AttributionSource::Start).unwrap();
        for (index, at) in times.iter().enumerate() {
            w.request(&RequestRow {
                key: format!("{session}-{index}"),
                session_id: session.into(),
                ts: utc(at),
                sent_ts: None,
                model: OPUS.into(),
                tokens: tokens(10, 100, 0, 0, 0),
                cost_usd: Some(1.0),
                is_sidechain: false,
                agent_id: None,
                skill: None,
                mcp_server: None,
                effort: None,
            })
            .unwrap();
        }
    }
    w.limit_sample(&LimitSample {
        ts_ms: utc("2026-10-10T10:00:00Z") * 1000,
        session_id: "work".into(),
        account_id: Some(WORK.into()),
        model: Some(OPUS.into()),
        window: FIVE_HOUR.into(),
        used_pct: 40.0,
        resets_at_ms: None,
    })
    .unwrap();
    w.set_meta(
        limits::FIT_KEY,
        r#"[{"account":"oauth:work","window":"five_hour","family":"opus","pairs":25,"pct_per_usd":2.0}]"#,
    )
    .unwrap();
    w.commit().unwrap();
    store
}

#[test]
fn history_of_one_account_counts_only_its_sessions_and_lists_both() {
    let store = two_account_store();
    let prices = PriceTable::bundled();
    let work = read(
        &store,
        &prices,
        Range::Week,
        now_ms(),
        &Utc,
        &Scope::Account(WORK.into()),
    )
    .unwrap();
    assert_eq!(work.totals.spend.requests, 3);
    assert_eq!(work.totals.sessions, 1);
    assert_eq!(work.account.as_deref(), Some(WORK));
    assert_eq!(
        work.accounts
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        vec![ME, WORK]
    );
    assert!(work.calibration.calibrated);
    assert_eq!(work.totals.peak_5h_pct, Some(40.0));
    assert_eq!(work.totals.spend.pct_5h, Some(6.0));
    let all = read(&store, &prices, Range::Week, now_ms(), &Utc, &Scope::All).unwrap();
    assert_eq!(all.totals.spend.requests, 5);
    assert_eq!(all.account, None);
    assert_eq!(all.accounts, work.accounts);
    assert!(!all.calibration.calibrated);
    assert_eq!(all.totals.peak_5h_pct, None);
    assert_eq!(all.totals.spend.pct_5h, None);
}

#[test]
fn an_account_with_nothing_spent_still_lists_itself() {
    let store = two_account_store();
    let gone = read(
        &store,
        &PriceTable::bundled(),
        Range::Week,
        now_ms(),
        &Utc,
        &Scope::Account("oauth:gone".into()),
    )
    .unwrap();
    assert_eq!(gone.totals.spend.requests, 0);
    assert_eq!(
        gone.accounts
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        vec!["oauth:gone", ME, WORK]
    );
}

#[test]
fn a_report_from_an_older_daemon_without_accounts_still_reads() {
    let mut value = serde_json::to_value(report(Range::Week)).unwrap();
    let object = value.as_object_mut().unwrap();
    object.remove("account");
    object.remove("accounts");
    let older: HistoryReport = serde_json::from_value(value).unwrap();
    assert_eq!(older.account, None);
    assert!(older.accounts.is_empty());
}
