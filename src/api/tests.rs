use std::os::unix::fs::PermissionsExt;
use std::thread::JoinHandle;

use serde_json::json;

use super::client::{Client, ClientError};
use super::*;
use crate::daemon::log::Log;
use crate::daemon::writer;
use crate::pricing::PriceTable;

const SESSION: &str = "11111111-1111-1111-1111-111111111111";
const TEST_REQUEST_TIMEOUT: Duration = Duration::from_millis(200);

struct Server {
    temp: tempfile::TempDir,
    socket: PathBuf,
    shutdown: Shutdown,
    writer: WriterHandle,
    serving: Option<JoinHandle<Result<()>>>,
}

impl Server {
    fn start() -> Self {
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
        let store = Store::open(&database).unwrap();
        let reader = Store::open_read_only(&database).unwrap();
        let log = Arc::new(Log::open(&temp.path().join("daemon.log"), false).unwrap());
        let (writer, _) =
            writer::spawn(store, PriceTable::bundled(), projects, log.clone()).unwrap();
        let socket = temp.path().join("daemon.sock");
        let listener = listen(&socket).unwrap();
        let shutdown = Shutdown::default();
        let state = Arc::new(State {
            writer: writer.clone(),
            reader: Mutex::new(reader),
            prices: PriceTable::bundled(),
            started: Instant::now(),
            claude_dir: temp.path().join("claude"),
            watching: AtomicBool::new(true),
            shutdown: shutdown.clone(),
            log,
            request_timeout: TEST_REQUEST_TIMEOUT,
        });
        let serving = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(serve(listener, state))
        });
        Self {
            temp,
            socket,
            shutdown,
            writer,
            serving: Some(serving),
        }
    }

    fn client(&self) -> Client {
        Client::new(&self.socket)
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
        (1, env!("CARGO_PKG_VERSION"), 9, std::process::id())
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
    server.client().import().unwrap();
    let step_ms = epoch_ms(SystemTime::now()) - 1000;
    server
        .client()
        .send_events(&json!({"events": [
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
        (2, 25_300, 2_661, 900)
    );
    assert_eq!(live.limits.len(), 1);
    assert_eq!(
        (live.limits[0].window.as_str(), live.limits[0].used_pct),
        ("five_hour", 54.0)
    );
    assert_eq!(live.limits[0].calibration.family, "opus");
    assert_eq!(live.limits[0].calibration.pct_per_usd, None);
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
        (
            Method::GET,
            "/history",
            "",
            404,
            "no route for GET /history",
        ),
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
fn snapshots_are_refused_while_the_store_is_being_rebuilt() {
    let server = Server::start();
    server.client().import().unwrap();
    let mut store = Store::open(&server.temp.path().join("data.db")).unwrap();
    let mut writer = store.writer().unwrap();
    writer.mark_reindex_pending(true).unwrap();
    writer.commit().unwrap();
    assert_eq!(
        server.raw(Method::GET, &format!("/snapshot?session={SESSION}"), ""),
        (503, json!({"error": "the store is being rebuilt"}))
    );
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
