use std::os::unix::fs::PermissionsExt;
use std::thread::JoinHandle;

use serde_json::json;

use super::client::{Client, ClientError};
use super::*;
use crate::daemon::log::Log;
use crate::daemon::writer;
use crate::history::Range;
use crate::pricing::PriceTable;

const SESSION: &str = "11111111-1111-1111-1111-111111111111";
const TEST_REQUEST_TIMEOUT: Duration = Duration::from_millis(200);

struct Server {
    temp: tempfile::TempDir,
    socket: PathBuf,
    shutdown: Shutdown,
    writer: WriterHandle,
    state: Arc<State>,
    serving: Option<JoinHandle<Result<()>>>,
}

impl Server {
    fn start() -> Self {
        Self::start_with(|_| {})
    }

    fn start_with(prepare: impl FnOnce(&mut Store)) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let projects = temp.path().join("claude/projects");
        std::fs::create_dir_all(projects.join("-work-demo")).unwrap();
        std::fs::write(
            projects.join(format!("-work-demo/{SESSION}.jsonl")),
            format!(
                r#"{{"type":"assistant","sessionId":"{SESSION}","timestamp":"2026-10-01T10:00:05Z","requestId":"req_opus","message":{{"model":"claude-opus-5-5","content":[],"usage":{{"input_tokens":10,"output_tokens":1000,"cache_read_input_tokens":20000,"cache_creation_input_tokens":5000,"cache_creation":{{"ephemeral_1h_input_tokens":5000,"ephemeral_5m_input_tokens":0}}}}}}}}"#
            ) + "\n",
        )
        .unwrap();
        let database = temp.path().join("data.db");
        let mut store = Store::open(&database).unwrap();
        prepare(&mut store);
        let reader = Store::open_read_only(&database).unwrap();
        let history_reader = Store::open_read_only(&database).unwrap();
        let insights_reader = Store::open_read_only(&database).unwrap();
        let log = Arc::new(Log::open(&temp.path().join("daemon.log"), false).unwrap());
        let (writer, _) = writer::spawn(
            store,
            PriceTable::bundled(),
            projects,
            temp.path().join("claude.json"),
            temp.path().join("desktop/claude-code-sessions"),
            log.clone(),
        )
        .unwrap();
        let socket = temp.path().join("daemon.sock");
        let listener = listen(&socket).unwrap();
        let shutdown = Shutdown::default();
        let state = Arc::new(State {
            writer: writer.clone(),
            reader: Mutex::new(reader),
            history_reader: Mutex::new(history_reader),
            insights_reader: Mutex::new(insights_reader),
            prices: PriceTable::bundled(),
            started: Instant::now(),
            home: temp.path().join("home"),
            claude_dir: temp.path().join("claude"),
            claude_json: temp.path().join("claude.json"),
            scans: context::Scans::default(),
            watching: AtomicBool::new(true),
            shutdown: shutdown.clone(),
            log,
            request_timeout: TEST_REQUEST_TIMEOUT,
        });
        let serving = std::thread::spawn({
            let state = state.clone();
            move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(serve(listener, state))
            }
        });
        Self {
            temp,
            socket,
            shutdown,
            writer,
            state,
            serving: Some(serving),
        }
    }

    fn client(&self) -> Client {
        Client::new(&self.socket)
    }

    fn login(&self, claude_json: &str) {
        std::fs::write(&self.state.claude_json, claude_json).unwrap();
    }

    fn send(&self, session: &str, kind: &str, ts_ms: i64, data: serde_json::Value) {
        let accepted = self
            .client()
            .send_events(&json!({"events": [
                {"session": session, "kind": kind, "ts_ms": ts_ms, "data": data}
            ]}))
            .unwrap();
        assert_eq!(accepted.accepted, 1);
    }

    fn start_session(&self, session: &str, ts_ms: i64) {
        self.start_session_with(
            session,
            ts_ms,
            json!({"provider": "anthropic", "base_url": null, "api_key": false}),
        );
    }

    fn start_session_with(&self, session: &str, ts_ms: i64, auth: serde_json::Value) {
        self.send(
            session,
            "session.start",
            ts_ms,
            json!({"cwd": "/w", "auth": auth}),
        );
    }

    fn measure(&self, session: &str, ts_ms: i64, rate_limits: serde_json::Value) {
        self.send(
            session,
            "session.measure",
            ts_ms,
            json!({"rateLimits": rate_limits}),
        );
    }

    fn set_budget(&self, account_id: &str, amount_usd: f64) {
        let mut store = Store::open(&self.temp.path().join("data.db")).unwrap();
        let mut writer = store.writer().unwrap();
        writer.set_budget(account_id, amount_usd, 0).unwrap();
        writer.commit().unwrap();
    }

    fn raw(&self, method: Method, path: &str, body: &str) -> (u16, serde_json::Value) {
        let (status, bytes) = self
            .client()
            .raw(method, path, Some(body.as_bytes().to_vec()))
            .unwrap();
        (status.as_u16(), serde_json::from_slice(&bytes).unwrap())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown.trigger();
        self.writer.stop();
        if let Some(serving) = self.serving.take() {
            serving.join().unwrap().unwrap();
        }
    }
}

fn measure(ts_ms: i64, percent: u64) -> serde_json::Value {
    json!({"session": SESSION, "kind": "session.measure", "ts_ms": ts_ms, "data": {"contextPercent": percent}})
}

#[test]
fn health_reports_version_schema_and_ingest_state() {
    let server = Server::start();
    let health = server.client().health().unwrap();
    assert_eq!(
        (health.v, health.version.as_str(), health.schema, health.pid),
        (1, env!("CARGO_PKG_VERSION"), 18, std::process::id())
    );
    assert_eq!(
        health.claude_dir,
        server.temp.path().join("claude").display().to_string()
    );
    assert!(health.ingest.watching);
    assert_eq!(health.ingest.last_ingest_ms, None);
    server.client().import().unwrap();
    let health = server.client().health().unwrap();
    assert_eq!((health.ingest.lag_ms, health.ingest.backlog), (0, 0));
    assert!(health.ingest.last_ingest_ms.is_some());
    assert_eq!(health.ingest.progress, None);
}

#[test]
fn the_socket_is_private_and_a_stale_one_is_replaced() {
    let server = Server::start();
    let mode = std::fs::metadata(&server.socket)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    let stale = server.temp.path().join("stale.sock");
    std::fs::write(&stale, "").unwrap();
    assert!(listen(&stale).is_ok());
}

#[test]
fn a_socket_path_beyond_the_platform_limit_is_explained() {
    let temp = tempfile::tempdir().unwrap();
    let socket = temp.path().join("d".repeat(120)).join("daemon.sock");
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    let error = listen(&socket).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "cannot listen on {}: the socket path is {} bytes and this system allows {MAX_SOCKET_PATH}; set CLAUDE_PLUS_HOME to a shorter directory",
            socket.display(),
            socket.as_os_str().len()
        )
    );
    assert!(!socket.exists());
}

#[test]
fn the_longest_allowed_socket_path_binds() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().as_os_str().len() + "/".len() + "/daemon.sock".len();
    let socket = temp
        .path()
        .join("d".repeat(MAX_SOCKET_PATH - base))
        .join("daemon.sock");
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    assert_eq!(socket.as_os_str().len(), MAX_SOCKET_PATH);
    listen(&socket).unwrap();
    assert!(socket.exists());
}

#[test]
fn events_are_stored_and_the_latest_measure_reaches_the_snapshot() {
    let server = Server::start();
    let accepted = server
        .client()
        .send_events(&json!({"events": [
            measure(1_791_021_500_000, 41),
            measure(1_791_021_400_000, 30),
            {"session": SESSION, "kind": "turn.boom", "ts_ms": 1, "data": {}}
        ]}))
        .unwrap();
    assert_eq!(accepted.accepted, 2);
    assert_eq!(
        accepted.rejected,
        vec![wire::Rejected {
            index: 2,
            error: "unknown kind \"turn.boom\"".into()
        }]
    );
    let snapshot = server.client().snapshot(SESSION).unwrap();
    let measure = snapshot.measure.unwrap();
    assert_eq!(measure.ts_ms, 1_791_021_500_000);
    assert_eq!(measure.data, json!({"contextPercent": 41}));
    assert!(snapshot.known);
    assert_eq!(snapshot.totals.requests, 0);
}

#[test]
fn the_snapshot_carries_session_totals_from_the_store() {
    let server = Server::start();
    let outcome = server.client().import().unwrap();
    assert_eq!(outcome.report.requests_inserted, 1);
    assert_eq!(outcome.reconcile_error, None);
    let snapshot = server.client().snapshot(SESSION).unwrap();
    assert_eq!(snapshot.v, 1);
    assert_eq!(snapshot.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(snapshot.session, SESSION);
    assert!((snapshot.totals.cost_usd - 0.06404).abs() < 1e-9);
    assert_eq!(snapshot.totals.requests, 1);
    assert_eq!(snapshot.totals.last_request_ms, Some(1_790_848_805_000));
    assert_eq!(snapshot.totals.tokens.cache_write_1h, 5000);
    assert_eq!(snapshot.totals.models[0].model, "claude-opus-5-5");
    assert_eq!(snapshot.measure, None);
    let now = epoch_ms(SystemTime::now());
    assert!((now - 5000..=now).contains(&snapshot.generated_at_ms));
}

#[test]
fn the_snapshot_reports_the_cache_forecast_and_limits() {
    let server = Server::start();
    std::fs::write(
        server.temp.path().join("claude.json"),
        r#"{"oauthAccount":{"accountUuid":"u-1","organizationType":"claude_max"}}"#,
    )
    .unwrap();
    server.client().import().unwrap();
    let step_ms = epoch_ms(SystemTime::now()) - 1000;
    server
        .client()
        .send_events(&json!({"events": [
            {"session": SESSION, "kind": "session.start", "ts_ms": step_ms - 1000, "data": {
                "cwd": "/work/demo", "auth": {"provider": "anthropic", "base_url": null, "api_key": false}
            }},
            {"session": SESSION, "kind": "session.measure", "ts_ms": step_ms, "data": {
                "model": "claude-opus-5-5",
                "rateLimits": [{"kind": "five_hour", "percentUsed": 54, "resetsAt": "2099-01-01T00:00:00Z"}]
            }},
            {"session": SESSION, "kind": "turn.step", "ts_ms": step_ms, "data": {
                "turnId": "t1", "index": 0, "model": "claude-opus-5-5", "effort": "high",
                "usage": {"model": "claude-opus-5-5", "input_tokens": 12, "output_tokens": 800,
                          "cache_read_input_tokens": 25000, "cache_creation_input_tokens": 300}
            }}
        ]}))
        .unwrap();
    let live = server.client().snapshot(SESSION).unwrap().live;
    assert_eq!(live.model.as_deref(), Some("claude-opus-5-5"));
    assert_eq!(live.effort.as_deref(), Some("high"));
    assert_eq!(live.cache.ttl, crate::cache_model::Ttl::OneHour);
    assert_eq!(
        live.cache.ttl_source,
        crate::cache_model::TtlSource::Observed
    );
    assert_eq!(live.cache.last_touch_ms, Some(step_ms));
    assert_eq!(live.cache.expires_at_ms, Some(step_ms + 3_600_000));
    assert_eq!(live.cache.state, crate::cache_model::CacheState::Warm);
    let forecast = live.forecast.unwrap();
    assert_eq!(
        (
            forecast.samples,
            forecast.prefix_tokens,
            forecast.new_tokens,
            forecast.output_tokens
        ),
        (2, 25_300, 312, 900)
    );
    let close = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() < 1e-9);
    assert!(close(
        forecast.warm_usd,
        (25_300.0 * 0.2 + 312.0 * 8.0 + 900.0 * 20.0) / 1e6
    ));
    assert!(close(
        forecast.cold_usd,
        (25_612.0 * 8.0 + 900.0 * 20.0) / 1e6
    ));
    assert!(close(live.cache.hit_ratio, 45_000.0 / 50_322.0));
    assert!(close(
        Some(live.cache.saved_usd),
        45_000.0 * (4.0 - 0.2) / 1e6
    ));
    assert_eq!((live.cache.rebuilds, live.cache.rebuild_usd), (0, 0.0));
    assert_eq!(live.limits.len(), 1);
    assert_eq!(
        (live.limits[0].window.as_str(), live.limits[0].used_pct),
        ("five_hour", 54.0)
    );
    assert_eq!(live.limits[0].calibration.family, "opus");
    assert_eq!(live.limits[0].calibration.pct_per_usd, None);
}

fn step(id: Option<&str>, ts_ms: i64, input: u64, read: u64, write: u64) -> serde_json::Value {
    let mut event = json!({"session": SESSION, "kind": "turn.step", "ts_ms": ts_ms, "data": {
        "turnId": "t1", "index": 0, "model": "claude-opus-5-5", "effort": "high",
        "usage": {"model": "claude-opus-5-5", "input_tokens": input, "output_tokens": 800,
                  "cache_read_input_tokens": read, "cache_creation_input_tokens": write}
    }});
    if let Some(id) = id {
        event["id"] = json!(id);
    }
    event
}

#[test]
fn a_batch_sent_again_is_stored_once_and_counted_once() {
    let server = Server::start();
    server.client().import().unwrap();
    let step_ms = epoch_ms(SystemTime::now()) - 1000;
    let batch = json!({"events": [step(Some("m1-1"), step_ms, 12, 25_000, 300)]});
    assert_eq!(server.client().send_events(&batch).unwrap().accepted, 1);
    let once = server.client().snapshot(SESSION).unwrap().live;
    assert_eq!(server.client().send_events(&batch).unwrap().accepted, 1);
    let twice = server.client().snapshot(SESSION).unwrap().live;
    assert_eq!(twice, once);
    assert_eq!(once.forecast.unwrap().samples, 2);
    let store = Store::open_read_only(&server.temp.path().join("data.db")).unwrap();
    assert_eq!(store.dump("events").len(), 1);
}

#[test]
fn a_step_already_in_the_transcript_and_its_repeat_are_not_counted_again() {
    let server = Server::start();
    server.client().import().unwrap();
    let transcript_ms = 1_790_848_805_000;
    let repeated = step(None, transcript_ms + 400, 10, 20_000, 5000);
    server
        .client()
        .send_events(&json!({"events": [repeated.clone(), repeated]}))
        .unwrap();
    let live = server.client().snapshot(SESSION).unwrap().live;
    assert_eq!(live.forecast.unwrap().samples, 1);
    assert_eq!(live.cache.last_touch_ms, Some(transcript_ms));
    assert_eq!(live.cache.hit_ratio, Some(20_000.0 / 25_010.0));
}

#[test]
fn while_an_answer_streams_the_countdown_counts_from_the_prompt() {
    let server = Server::start();
    let transcript = server
        .temp
        .path()
        .join(format!("claude/projects/-work-demo/{SESSION}.jsonl"));
    let mut lines = std::fs::read_to_string(&transcript).unwrap();
    lines += &format!(
        r#"{{"type":"user","sessionId":"{SESSION}","uuid":"p1","timestamp":"2026-10-01T10:10:00Z","message":{{"role":"user","content":"go"}}}}"#
    );
    lines += "\n";
    lines += &format!(
        r#"{{"type":"assistant","sessionId":"{SESSION}","uuid":"a1","parentUuid":"p1","timestamp":"2026-10-01T10:11:00Z","requestId":"req_streaming","message":{{"model":"claude-opus-5-5","content":[{{"type":"text","text":"so"}}],"usage":{{"input_tokens":3,"output_tokens":1,"cache_read_input_tokens":25000,"cache_creation_input_tokens":0}}}}}}"#
    );
    lines += "\n";
    std::fs::write(&transcript, lines).unwrap();
    server.client().import().unwrap();
    let cache = server.client().snapshot(SESSION).unwrap().live.cache;
    let prompt_ms = 1_790_848_800_000 + 600_000;
    assert_eq!(cache.last_touch_ms, Some(prompt_ms));
    assert_eq!(cache.expires_at_ms, Some(prompt_ms + 3_600_000));
}

#[test]
fn the_cache_countdown_starts_when_the_request_was_sent() {
    let server = Server::start();
    server.client().import().unwrap();
    let answered_ms = epoch_ms(SystemTime::now()) - 1000;
    let sent_ms = answered_ms - 90_000;
    let mut slow = step(Some("m1-2"), answered_ms, 12, 25_000, 300);
    slow["data"]["sent_ms"] = json!(sent_ms);
    server
        .client()
        .send_events(&json!({"events": [slow]}))
        .unwrap();
    let cache = server.client().snapshot(SESSION).unwrap().live.cache;
    assert_eq!(cache.last_touch_ms, Some(sent_ms));
    assert_eq!(cache.expires_at_ms, Some(sent_ms + 3_600_000));
}

#[test]
fn only_this_session_s_own_limits_say_it_runs_on_a_subscription() {
    let server = Server::start();
    let now_ms = epoch_ms(SystemTime::now());
    let other = "22222222-2222-2222-2222-222222222222";
    let mut metered = step(None, now_ms - 1000, 12, 0, 3000);
    metered["session"] = json!(other);
    server
        .client()
        .send_events(&json!({"events": [
            {"session": SESSION, "kind": "session.measure", "ts_ms": now_ms - 2000, "data": {
                "rateLimits": [{"kind": "five_hour", "percentUsed": 10, "resetsAt": "2099-01-01T00:00:00Z"}]
            }},
            metered
        ]}))
        .unwrap();
    let cache = server.client().snapshot(other).unwrap().live.cache;
    assert_eq!(
        (cache.ttl, cache.ttl_source),
        (
            crate::cache_model::Ttl::FiveMinutes,
            crate::cache_model::TtlSource::Assumed
        )
    );
}

#[test]
fn a_snapshot_shows_only_its_own_account_s_windows() {
    let server = Server::start();
    let now_ms = epoch_ms(SystemTime::now());
    server.login(r#"{"oauthAccount":{"accountUuid":"me","organizationName":"Me","organizationType":"claude_max"}}"#);
    server.start_session(SESSION, now_ms - 9000);
    server.measure(
        SESSION,
        now_ms - 8000,
        json!([{"kind": "seven_day", "percentUsed": 40, "resetsAt": "2099-01-01T00:00:00Z"}]),
    );
    server.login(r#"{"oauthAccount":{"accountUuid":"work","organizationName":"Acme","organizationType":"claude_enterprise"}}"#);
    let other = "22222222-2222-2222-2222-222222222222";
    server.start_session(other, now_ms - 2000);
    server.measure(
        other,
        now_ms - 1000,
        json!([{"kind": "spend_limit", "percentUsed": 4, "resetsAt": "2099-01-01T00:00:00Z"}]),
    );
    let live = server.client().snapshot(other).unwrap().live;
    assert_eq!(
        live.limits
            .iter()
            .map(|w| w.window.as_str())
            .collect::<Vec<_>>(),
        vec!["spend_limit"]
    );
    let account = live.account.unwrap();
    assert_eq!(
        (account.id.as_str(), account.label.as_str()),
        ("oauth:work", "Acme · Enterprise")
    );
}

#[test]
fn a_budget_without_a_reported_limit_is_its_own_window() {
    let server = Server::start();
    let now_ms = epoch_ms(SystemTime::now());
    server.start_session_with(
        SESSION,
        now_ms - 2000,
        json!({"provider": "anthropic", "base_url": "https://llm.corp", "api_key": true}),
    );
    server.set_budget("gateway:https://llm.corp:443", 700.0);
    let live = server.client().snapshot(SESSION).unwrap().live;
    let budget = live.budget.unwrap();
    assert_eq!(budget.amount_usd, 700.0);
    assert_eq!(
        budget.resets_at_ms,
        crate::limits::next_month_start_utc(now_ms)
    );
    assert_eq!(
        live.limits
            .iter()
            .map(|w| w.window.as_str())
            .collect::<Vec<_>>(),
        vec!["budget"]
    );
}

#[test]
fn a_budget_of_nothing_is_not_shown() {
    let server = Server::start();
    let now_ms = epoch_ms(SystemTime::now());
    server.start_session_with(
        SESSION,
        now_ms - 2000,
        json!({"provider": "anthropic", "base_url": "https://llm.corp", "api_key": true}),
    );
    server.set_budget("gateway:https://llm.corp:443", 0.0);
    let live = server.client().snapshot(SESSION).unwrap().live;
    assert_eq!((live.budget, live.limits), (None, vec![]));
}

#[test]
fn a_reported_spend_limit_wins_over_the_budget_window() {
    let server = Server::start();
    let now_ms = epoch_ms(SystemTime::now());
    server.login(r#"{"oauthAccount":{"accountUuid":"work","organizationName":"Acme","organizationType":"claude_enterprise"}}"#);
    server.start_session(SESSION, now_ms - 2000);
    server.measure(
        SESSION,
        now_ms - 1000,
        json!([{"kind": "spend_limit", "percentUsed": 4, "resetsAt": "2099-01-01T00:00:00Z"}]),
    );
    server.set_budget("oauth:work", 700.0);
    let live = server.client().snapshot(SESSION).unwrap().live;
    assert_eq!(
        live.limits
            .iter()
            .map(|w| w.window.as_str())
            .collect::<Vec<_>>(),
        vec!["spend_limit"]
    );
    assert_eq!(live.limits[0].used_pct, 4.0);
    assert_eq!(live.budget.unwrap().amount_usd, 700.0);
}

#[test]
fn the_context_route_joins_the_live_breakdown_the_transcript_and_the_config() {
    let server = Server::start();
    let work = server.temp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("CLAUDE.md"), "x".repeat(700)).unwrap();
    std::fs::create_dir_all(server.temp.path().join("claude/skills/tidy")).unwrap();
    std::fs::write(
        server.temp.path().join("claude/skills/tidy/SKILL.md"),
        "---\nname: tidy\ndescription: Tidy up\n---\n",
    )
    .unwrap();
    server.client().import().unwrap();
    let now = epoch_ms(SystemTime::now());
    server
        .client()
        .send_events(&json!({"events": [
            {"session": SESSION, "kind": "session.start", "ts_ms": now - 2000, "data": {"cwd": work.display().to_string()}},
            {"session": SESSION, "kind": "session.usage", "ts_ms": now - 1000, "data": {
                "model": "claude-opus-5-5", "totalTokens": 25300, "maxTokens": 200000, "rawMaxTokens": 200000, "percentage": 13,
                "categories": [{"name": "Messages", "tokens": 25300, "color": "claude", "isDeferred": false, "kind": "used"}],
                "memoryFiles": [{"path": work.join("CLAUDE.md").display().to_string(), "type": "Project", "tokens": 200}],
                "mcpTools": [], "agents": []
            }}
        ]}))
        .unwrap();
    let report = server.client().context(SESSION).unwrap();
    assert_eq!(report.v, 1);
    assert_eq!(report.project_dir, Some(work.display().to_string()));
    let live = report.live.unwrap();
    assert_eq!((live.ts_ms, live.total_tokens), (now - 1000, 25_300));
    assert_eq!(report.history.unwrap().measured_tokens, 25_010);
    assert_eq!(
        report
            .items
            .iter()
            .map(|item| (item.name.as_str(), item.tokens))
            .collect::<Vec<_>>(),
        vec![("CLAUDE.md", 200)]
    );
    let groups: Vec<(crate::config_scan::SourceKind, u64)> = report
        .config
        .groups
        .iter()
        .map(|group| (group.kind, group.count))
        .collect();
    assert_eq!(
        groups,
        vec![
            (crate::config_scan::SourceKind::ClaudeMd, 1),
            (crate::config_scan::SourceKind::Skill, 1)
        ]
    );
}

#[test]
fn the_history_route_reports_the_asked_range_in_local_time() {
    let server = Server::start();
    let hour_ago = chrono::Utc::now() - chrono::Duration::hours(1);
    std::fs::write(
        server
            .temp
            .path()
            .join("claude/projects/-work-demo/22222222-2222-2222-2222-222222222222.jsonl"),
        format!(
            r#"{{"type":"assistant","sessionId":"22222222-2222-2222-2222-222222222222","timestamp":"{}","requestId":"req_recent","message":{{"model":"claude-sonnet-5-5","content":[],"usage":{{"input_tokens":10,"output_tokens":100}}}}}}"#,
            hour_ago.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        ) + "\n",
    )
    .unwrap();
    server.client().import().unwrap();
    let day = server.client().history(Range::Day).unwrap();
    assert_eq!((day.v, day.range, day.buckets.len()), (1, Range::Day, 24));
    assert_eq!(day.totals.spend.requests, 1);
    assert_eq!(day.projects[0].name, "-work-demo");
    let quarter = server.client().history(Range::Quarter).unwrap();
    assert_eq!((quarter.range, quarter.buckets.len()), (Range::Quarter, 90));
    assert!(quarter.totals.spend.requests >= 1);
    let (status, body) = server.raw(Method::GET, "/history", "");
    assert_eq!((status, body["range"].as_str()), (200, Some("7d")));
}

#[test]
fn live_history_and_insights_count_the_same_main_thread_rebuilds_past_a_subagent_s_compaction() {
    let server = Server::start();
    let session = "33333333-3333-3333-3333-333333333333";
    let start = chrono::Utc::now() - chrono::Duration::days(1);
    let at = |seconds: i64| {
        (start + chrono::Duration::seconds(seconds))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    };
    let request = |seconds: i64, read: u64, write: u64| {
        json!({"type": "assistant", "sessionId": session, "timestamp": at(seconds),
               "requestId": format!("req_{seconds}"),
               "message": {"model": "claude-opus-5-5", "content": [], "usage": {
                   "input_tokens": 10, "output_tokens": 100, "cache_read_input_tokens": read,
                   "cache_creation_input_tokens": write,
                   "cache_creation": {"ephemeral_5m_input_tokens": write, "ephemeral_1h_input_tokens": 0}}}})
    };
    let lines = |records: &[serde_json::Value]| {
        records
            .iter()
            .map(|record| format!("{record}\n"))
            .collect::<String>()
    };
    let projects = server.temp.path().join("claude/projects/-work-demo");
    std::fs::write(
        projects.join(format!("{session}.jsonl")),
        lines(&[
            request(0, 0, 10_000),
            request(30, 10_000, 290_000),
            request(1_200, 0, 300_000),
            request(1_260, 0, 300_000),
        ]),
    )
    .unwrap();
    let subagents = projects.join(format!("{session}/subagents"));
    std::fs::create_dir_all(&subagents).unwrap();
    std::fs::write(
        subagents.join("agent-a.jsonl"),
        lines(&[
            json!({"type": "system", "subtype": "compact_boundary", "sessionId": session,
                   "isSidechain": true, "agentId": "a", "timestamp": at(600),
                   "compactMetadata": {"trigger": "auto", "preTokens": 180_000, "postTokens": 20_000}}),
            json!({"type": "attachment", "sessionId": session, "isSidechain": true, "agentId": "a",
                   "timestamp": at(1_230),
                   "attachment": {"type": "deferred_tools_record", "entries": []}}),
        ]),
    )
    .unwrap();
    server.client().import().unwrap();
    let live = server.client().snapshot(session).unwrap().live.cache;
    assert_eq!(live.rebuilds, 2);
    assert_eq!(
        live.last_rebuild.map(|r| r.cause),
        Some(crate::cache_model::RebuildCause::Unknown)
    );
    let history = server.client().history(Range::Month).unwrap().totals;
    assert_eq!((history.rebuilds, history.idle_rebuilds), (2, 1));
    let insights = server.client().insights(session).unwrap();
    let evidence = |rule| {
        insights
            .findings
            .iter()
            .find(|f| f.rule == rule)
            .map(|f| f.evidence[0].clone())
    };
    assert_eq!(
        evidence(crate::insights::RuleId::IdleRebuilds),
        Some("1\u{a0}cache rebuild after a break, in 1\u{a0}session, $1.44".into())
    );
    assert_eq!(
        evidence(crate::insights::RuleId::CacheBusts),
        Some("1\u{a0}rewrite mid-session with no cause in the transcripts, $1.44".into())
    );
}

#[test]
fn the_insights_route_answers_with_ranked_findings_wins_and_a_recap() {
    let server = Server::start();
    std::fs::write(
        server.temp.path().join("claude/settings.json"),
        r#"{"cleanupPeriodDays": 30}"#,
    )
    .unwrap();
    server.client().import().unwrap();
    let report = server.client().insights(SESSION).unwrap();
    assert_eq!((report.v, report.session.as_str()), (1, SESSION));
    assert_eq!(
        report.findings.last().map(|f| f.rule),
        Some(crate::insights::RuleId::Retention)
    );
    let (status, body) = server.raw(Method::GET, &format!("/insights?session={SESSION}"), "");
    assert_eq!(status, 200);
    let keys = |value: &serde_json::Value| {
        let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    assert_eq!(
        keys(&body),
        [
            "findings",
            "generated_at_ms",
            "recap",
            "requests",
            "session",
            "sessions",
            "since_ms",
            "v",
            "wins"
        ]
    );
    assert_eq!(keys(&body["wins"]), ["hit_streak_days", "saved_usd_7d"]);
    let retention = body["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["rule"] == "retention")
        .unwrap();
    assert_eq!(
        keys(retention),
        [
            "confidence",
            "evidence",
            "how",
            "rule",
            "saving_pct_5h",
            "saving_usd",
            "tip"
        ]
    );
    assert_eq!(
        (&retention["confidence"], &retention["saving_usd"]),
        (&json!("high"), &json!(null))
    );
    assert_eq!(
        retention["evidence"][0],
        "cleanupPeriodDays is 30 in ~/.claude/settings.json"
    );
}

#[test]
fn bad_requests_are_answered_with_a_reason() {
    let server = Server::start();
    let cases = [
        (
            Method::GET,
            "/snapshot",
            "",
            400,
            "session query parameter is required",
        ),
        (
            Method::GET,
            "/snapshot?session=a%2Fb",
            "",
            400,
            events::SESSION_RULE,
        ),
        (
            Method::GET,
            "/context",
            "",
            400,
            "session query parameter is required",
        ),
        (
            Method::GET,
            "/context?session=a%2Fb",
            "",
            400,
            events::SESSION_RULE,
        ),
        (Method::POST, "/context", "", 405, "use GET for /context"),
        (
            Method::GET,
            "/history?range=1y",
            "",
            400,
            "range must be 24h, 7d, 30d or 90d",
        ),
        (Method::POST, "/history", "", 405, "use GET for /history"),
        (
            Method::GET,
            "/insights",
            "",
            400,
            "session query parameter is required",
        ),
        (Method::POST, "/insights", "", 405, "use GET for /insights"),
        (
            Method::POST,
            "/events",
            "{not json",
            400,
            "body is not valid JSON: key must be a string at line 1 column 2",
        ),
        (
            Method::POST,
            "/events",
            "[]",
            400,
            "body must be a JSON object with an events array",
        ),
        (Method::GET, "/events", "", 405, "use POST for /events"),
        (Method::POST, "/health", "", 405, "use GET for /health"),
        (Method::GET, "/budget", "", 404, "no route for GET /budget"),
    ];
    for (method, path, body, status, error) in cases {
        assert_eq!(
            server.raw(method, path, body),
            (status, json!({"error": error})),
            "{path}"
        );
    }
}

#[test]
fn oversized_bodies_are_refused() {
    let server = Server::start();
    let body = format!(
        r#"{{"events": [], "pad": "{}"}}"#,
        "x".repeat(MAX_BODY_BYTES)
    );
    assert_eq!(
        server.raw(Method::POST, "/events", &body),
        (413, json!({"error": "request body is larger than 1 MiB"}))
    );
}

#[test]
fn reindex_runs_over_the_socket() {
    let server = Server::start();
    server.client().import().unwrap();
    let rebuilt = server.client().reindex().unwrap();
    assert_eq!((rebuilt.requests, rebuilt.records), (1, 1));
}

#[test]
fn the_shutdown_route_stops_the_server() {
    let mut server = Server::start();
    server.client().shutdown().unwrap();
    server.serving.take().unwrap().join().unwrap().unwrap();
    let error = server.client().health().unwrap_err();
    assert!(
        matches!(
            error,
            ClientError::NotRunning(_) | ClientError::Transport(_)
        ),
        "{error}"
    );
}

#[test]
fn a_missing_daemon_is_reported_as_not_running() {
    let temp = tempfile::tempdir().unwrap();
    let socket = temp.path().join("daemon.sock");
    let error = Client::new(&socket).health().unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("no claude-plus daemon is listening on {}", socket.display())
    );
}

#[test]
fn health_answers_while_a_snapshot_holds_the_reader() {
    let server = Server::start();
    let held = server.state.reader.lock().unwrap();
    let health = server
        .client()
        .with_timeout(Duration::from_millis(500))
        .health();
    drop(held);
    assert!(health.is_ok(), "{health:?}");
}

#[test]
fn a_snapshot_does_not_wait_for_a_history_report() {
    let server = Server::start();
    let building = server.state.history_reader.lock().unwrap();
    let snapshot = server
        .client()
        .with_timeout(Duration::from_millis(500))
        .snapshot(SESSION);
    drop(building);
    assert!(snapshot.is_ok(), "{snapshot:?}");
}

#[test]
fn a_history_report_does_not_wait_for_a_snapshot() {
    let server = Server::start();
    let held = server.state.reader.lock().unwrap();
    let history = server
        .client()
        .with_timeout(Duration::from_millis(500))
        .history(Range::Quarter);
    drop(held);
    assert!(history.is_ok(), "{history:?}");
}

#[test]
fn a_snapshot_does_not_wait_for_an_insights_report() {
    let server = Server::start();
    let building = server.state.insights_reader.lock().unwrap();
    let snapshot = server
        .client()
        .with_timeout(Duration::from_millis(500))
        .snapshot(SESSION);
    drop(building);
    assert!(snapshot.is_ok(), "{snapshot:?}");
}

#[test]
fn an_insights_report_does_not_wait_for_a_snapshot() {
    let server = Server::start();
    let held = server.state.reader.lock().unwrap();
    let insights = server
        .client()
        .with_timeout(Duration::from_millis(500))
        .insights(SESSION);
    drop(held);
    assert!(insights.is_ok(), "{insights:?}");
}

#[test]
fn snapshots_and_context_are_refused_while_the_store_is_being_rebuilt() {
    let server = Server::start_with(|store| {
        let mut writer = store.writer().unwrap();
        writer.mark_reindex_pending(true).unwrap();
        writer.commit().unwrap();
    });
    for route in ["snapshot", "context", "history", "insights"] {
        assert_eq!(
            server.raw(Method::GET, &format!("/{route}?session={SESSION}"), ""),
            (503, json!({"error": "the store is being rebuilt"})),
            "{route}"
        );
    }
    assert!(server.client().health().unwrap().ingest.rebuilding);
    server.client().import().unwrap();
    assert!(!server.client().health().unwrap().ingest.rebuilding);
    assert_eq!(
        server.client().snapshot(SESSION).unwrap().totals.requests,
        1
    );
}

#[test]
fn an_import_cut_short_by_shutdown_is_answered_as_stopping() {
    let mut server = Server::start();
    let release = server.writer.hold();
    let socket = server.socket.clone();
    let import = std::thread::spawn(move || Client::new(&socket).import());
    while server.writer.status().backlog() < 1 {
        std::thread::yield_now();
    }
    server.client().shutdown().unwrap();
    let started = Instant::now();
    release.send(()).unwrap();
    let error = import.join().unwrap().unwrap_err();
    assert_eq!(error.to_string(), "the claude-plus daemon is stopping");
    server.serving.take().unwrap().join().unwrap().unwrap();
    assert!(started.elapsed() < DRAIN, "{:?}", started.elapsed());
}

fn answer_to(server: &Server, partial_request: &str) -> (String, Duration) {
    use std::io::{Read, Write};
    let mut stream = std::os::unix::net::UnixStream::connect(&server.socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.write_all(partial_request.as_bytes()).unwrap();
    let started = Instant::now();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    (answer, started.elapsed())
}

#[test]
fn a_client_that_never_finishes_its_headers_is_disconnected() {
    let server = Server::start();
    let (answer, waited) = answer_to(&server, "GET /health HTTP/1.1\r\nhost: x\r\n");
    assert_eq!(answer, "");
    assert!(waited < Duration::from_secs(2), "{waited:?}");
}

#[test]
fn an_idle_kept_alive_connection_is_closed() {
    let server = Server::start();
    let (answer, waited) = answer_to(&server, "GET /health HTTP/1.1\r\nhost: x\r\n\r\n");
    assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer:?}");
    assert!(waited < Duration::from_secs(2), "{waited:?}");
}

#[test]
fn a_body_that_never_arrives_is_answered_with_a_timeout() {
    let server = Server::start();
    let (answer, waited) = answer_to(
        &server,
        "POST /events HTTP/1.1\r\nhost: x\r\ncontent-length: 100\r\n\r\n{",
    );
    assert!(answer.starts_with("HTTP/1.1 408 "), "{answer:?}");
    assert!(
        answer.ends_with(r#"{"error":"the request body did not arrive within 200 ms"}"#),
        "{answer:?}"
    );
    assert!(waited < Duration::from_secs(2), "{waited:?}");
}

#[test]
fn the_server_stops_when_the_writer_dies() {
    let mut server = Server::start();
    server.writer.panic_for_test();
    let error = server.serving.take().unwrap().join().unwrap().unwrap_err();
    assert_eq!(
        error.to_string(),
        "the claude-plus writer stopped unexpectedly"
    );
    assert!(server.client().health().is_err());
}
