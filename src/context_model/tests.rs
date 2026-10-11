use serde_json::json;

use super::breakdown::tests::USAGE;
use super::*;
use crate::config_scan::{Scope, Source};
use crate::store::{AttachmentRow, EventRow, RequestRow, ToolResultRow, ToolUseRow};
use crate::transcript::TokenCounts;

const SESSION: &str = "s1";
const NOW_MS: i64 = 1_791_021_600_000;
const FIRST_TS: i64 = 1_791_021_000;

fn scan() -> Scan {
    let source = |kind, scope, name: &str, path: Option<&str>, loading, chars| Source {
        kind,
        scope,
        name: name.into(),
        alias: None,
        path: path.map(String::from),
        plugin: None,
        loading,
        chars,
        file_chars: chars,
    };
    Scan {
        sources: vec![
            source(
                SourceKind::ClaudeMd,
                Scope::User,
                "CLAUDE.md",
                Some("/home/ada/.claude/CLAUDE.md"),
                Loading::Always,
                1400,
            ),
            source(
                SourceKind::ClaudeMd,
                Scope::Nested,
                "src/CLAUDE.md",
                Some("/work/app/src/CLAUDE.md"),
                Loading::OnDemand,
                7000,
            ),
            source(
                SourceKind::Skill,
                Scope::User,
                "tidy",
                None,
                Loading::Listing,
                350,
            ),
            source(
                SourceKind::McpServer,
                Scope::User,
                "linear",
                None,
                Loading::Unknown,
                0,
            ),
        ],
        memory_dirs: vec![],
        ..Scan::default()
    }
}

fn store() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let mut w = store.writer().unwrap();
    let file = w.file("/p/-work-app/s1.jsonl", 0, 0).unwrap();
    w.raw(
        file,
        1,
        "user",
        r#"{"type":"user","message":{"role":"user","content":{"$chars":350}}}"#,
    )
    .unwrap();
    for (line_no, ts, kind, chars) in [
        (2, FIRST_TS - 1, "skill_listing", 7000),
        (3, FIRST_TS - 1, "instructions", 3500),
        (5, FIRST_TS + 60, "nested_memory", 1750),
    ] {
        w.attachment(&AttachmentRow {
            file_id: file,
            line_no,
            session_id: SESSION.into(),
            ts: Some(ts),
            kind: kind.into(),
            chars,
            names: vec![],
        })
        .unwrap();
    }
    for (key, ts, prompt, skill) in [
        ("r1", FIRST_TS, 20_000, None),
        ("r2", FIRST_TS + 120, 46_000, Some("tidy")),
    ] {
        w.request(&RequestRow {
            key: key.into(),
            session_id: SESSION.into(),
            ts,
            model: "claude-opus-5-5".into(),
            tokens: TokenCounts {
                input: 0,
                output: 500,
                thinking: 0,
                cache_read: prompt,
                cache_write_5m: 0,
                cache_write_1h: 0,
            },
            cost_usd: Some(0.1),
            is_sidechain: false,
            agent_id: None,
            skill: skill.map(String::from),
            mcp_server: None,
            effort: None,
            sent_ts: None,
        })
        .unwrap();
    }
    w.tool_use(&ToolUseRow {
        id: "t1".into(),
        session_id: SESSION.into(),
        ts: FIRST_TS + 100,
        name: "mcp__linear__search".into(),
        input_chars: 10,
    })
    .unwrap();
    w.tool_result(&ToolResultRow {
        tool_use_id: "t1".into(),
        session_id: SESSION.into(),
        ts: FIRST_TS + 101,
        chars: 9000,
        images: 0,
        is_error: false,
        is_sidechain: false,
    })
    .unwrap();
    w.live_breakdown(&EventRow {
        client_id: None,
        session_id: SESSION.into(),
        kind: "session.usage".into(),
        ts_ms: NOW_MS - 30_000,
        received_ms: NOW_MS - 29_990,
        json: USAGE.into(),
    })
    .unwrap();
    w.commit().unwrap();
    store
}

fn request<'a>(scan: &'a Scan) -> Request<'a> {
    Request {
        session: SESSION,
        project_dir: Some("/work/app"),
        scan,
        scanned_at_ms: NOW_MS - 5_000,
        now_ms: NOW_MS,
    }
}

#[test]
fn context_v1_has_the_documented_shape() {
    let scan = scan();
    let report = read(&store(), &request(&scan)).unwrap();
    let expected = json!({
        "v": 1,
        "session": "s1",
        "generated_at_ms": NOW_MS,
        "project_dir": "/work/app",
        "live": {
            "ts_ms": NOW_MS - 30_000,
            "model": "claude-opus-5-5",
            "total_tokens": 46000,
            "max_tokens": 200000,
            "percentage": 23.0,
            "auto_compact_threshold": 167000,
            "categories": [
                {"name": "System prompt", "tokens": 3100, "color": "promptBorder", "kind": "used"},
                {"name": "System tools", "tokens": 14200, "color": "inactive", "kind": "used"},
                {"name": "MCP tools", "tokens": 4100, "color": "permission", "kind": "used"},
                {"name": "MCP tools (deferred)", "tokens": 8400, "color": "inactive", "kind": "deferred"},
                {"name": "Memory files", "tokens": 2000, "color": "claude", "kind": "used"},
                {"name": "Skills", "tokens": 2600, "color": "warning", "kind": "used"},
                {"name": "Messages", "tokens": 20000, "color": "purple_FOR_SUBAGENTS_ONLY", "kind": "used"},
                {"name": "Free space", "tokens": 121000, "color": "promptBorder", "kind": "free"},
                {"name": "Autocompact buffer", "tokens": 33000, "color": "inactive", "kind": "buffer"}
            ]
        },
        "history": {
            "first_request_ms": FIRST_TS * 1000,
            "measured_tokens": 20000,
            "first_message_tokens": 100,
            "scale": 1.0,
            "initial": [
                {"part": "system", "tokens": 16900},
                {"part": "memory", "tokens": 1000},
                {"part": "skills", "tokens": 2000}
            ],
            "now": [
                {"part": "system", "tokens": 16900},
                {"part": "memory", "tokens": 1500},
                {"part": "skills", "tokens": 2000},
                {"part": "messages", "tokens": 25600}
            ],
            "injected_later_tokens": 500,
            "compactions": 0,
            "points": [
                {"ts_ms": FIRST_TS * 1000, "prompt_tokens": 20000, "injected_tokens": 3000, "compacted": false},
                {"ts_ms": (FIRST_TS + 120) * 1000, "prompt_tokens": 46000, "injected_tokens": 3500, "compacted": false}
            ]
        },
        "ratios": {"prose": 3.5, "code": 3.0, "schema": 2.5, "calibrated": false, "pairs": 0},
        "items": [
            {"kind": "mcp_server", "name": "linear", "detail": "2 tools", "tokens": 4100, "estimated": false, "uses_session": 1, "uses_14d": 1},
            {"kind": "skill", "name": "superpowers:brainstorming", "detail": "superpowers", "tokens": 2300, "estimated": false, "uses_session": 0, "uses_14d": 0},
            {"kind": "memory_file", "name": "CLAUDE.md", "detail": "Project", "tokens": 1600, "estimated": false, "uses_session": null, "uses_14d": null},
            {"kind": "memory_file", "name": "CLAUDE.md", "detail": "User", "tokens": 400, "estimated": false, "uses_session": null, "uses_14d": null},
            {"kind": "skill", "name": "tidy", "detail": null, "tokens": 300, "estimated": false, "uses_session": 1, "uses_14d": 1},
            {"kind": "agent", "name": "reviewer", "detail": null, "tokens": 120, "estimated": false, "uses_session": null, "uses_14d": null}
        ],
        "dead_weight": [
            {"kind": "skill", "name": "superpowers:brainstorming", "detail": "superpowers", "tokens": 2300, "estimated": false, "uses_session": 0, "uses_14d": 0}
        ],
        "tool_results": [
            {"tool": "mcp__linear__search", "ts_ms": (FIRST_TS + 101) * 1000, "chars": 9000, "images": 0, "tokens": 3000}
        ],
        "config": {
            "scanned_at_ms": NOW_MS - 5_000,
            "groups": [
                {"kind": "claude_md", "loading": "always", "count": 1, "tokens": 400},
                {"kind": "claude_md", "loading": "on_demand", "count": 1, "tokens": 2000},
                {"kind": "skill", "loading": "listing", "count": 1, "tokens": 100},
                {"kind": "mcp_server", "loading": "unknown", "count": 1, "tokens": 0}
            ]
        }
    });
    assert_eq!(serde_json::to_value(&report).unwrap(), expected);
    let parsed: ContextReport = serde_json::from_value(expected).unwrap();
    assert_eq!(parsed, report);
}

#[test]
fn an_unknown_session_reports_the_configuration_alone() {
    let scan = scan();
    let store = Store::open_in_memory().unwrap();
    let report = read(
        &store,
        &Request {
            session: "s9",
            ..request(&scan)
        },
    )
    .unwrap();
    assert_eq!(
        (report.live.as_ref(), report.history.as_ref()),
        (None, None)
    );
    assert_eq!(report.items.len(), 2);
    assert!(report.items.iter().all(|item| item.estimated));
    assert!(report.tool_results.is_empty());
    assert_eq!(report.config.groups.len(), 4);
}

#[test]
fn a_breakdown_the_daemon_cannot_read_is_left_out() {
    let scan = scan();
    let mut store = Store::open_in_memory().unwrap();
    let mut w = store.writer().unwrap();
    w.live_breakdown(&EventRow {
        client_id: None,
        session_id: SESSION.into(),
        kind: "session.usage".into(),
        ts_ms: 1,
        received_ms: 1,
        json: r#"{"categories":"nope"}"#.into(),
    })
    .unwrap();
    w.commit().unwrap();
    let report = read(&store, &request(&scan)).unwrap();
    assert_eq!(report.live, None);
}
