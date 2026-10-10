pub mod client;
pub mod events;
pub mod live;
pub mod wire;

use std::convert::Infallible;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::body::{Body, Bytes};
use hyper::header::{ALLOW, CONTENT_TYPE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use serde::Serialize;
use tokio::sync::watch;

use crate::daemon::log::Log;
use crate::daemon::writer::WriterHandle;
use crate::pricing::PriceTable;
use crate::store::Store;
use live::Live;
use wire::{
    API_VERSION, ErrorBody, EventsAccepted, Health, ImportOutcome, IngestHealth, MEASURE_KIND,
    Progress, Snapshot,
};

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
pub const MAX_SOCKET_PATH: usize = if cfg!(target_os = "linux") { 107 } else { 103 };
const WRITER_EXITED: &str = "the claude-plus writer stopped unexpectedly";
const REBUILDING: &str = "the store is being rebuilt";
const DRAIN: Duration = Duration::from_secs(2);
const ROUTES: &[(&str, Method)] = &[
    ("/health", Method::GET),
    ("/snapshot", Method::GET),
    ("/events", Method::POST),
    ("/import", Method::POST),
    ("/reindex", Method::POST),
    ("/shutdown", Method::POST),
];

#[derive(Clone)]
pub struct Shutdown {
    sender: Arc<watch::Sender<bool>>,
}

impl Default for Shutdown {
    fn default() -> Self {
        Self {
            sender: Arc::new(watch::channel(false).0),
        }
    }
}

impl Shutdown {
    pub fn trigger(&self) {
        self.sender.send_replace(true);
    }

    pub async fn triggered(&self) {
        let mut receiver = self.sender.subscribe();
        let _ = receiver.wait_for(|stopping| *stopping).await;
    }
}

pub struct State {
    pub writer: WriterHandle,
    pub reader: Mutex<Store>,
    pub prices: PriceTable,
    pub started: Instant,
    pub claude_dir: PathBuf,
    pub watching: AtomicBool,
    pub shutdown: Shutdown,
    pub log: Arc<Log>,
    pub request_timeout: Duration,
}

pub fn socket_path_problem(socket: &Path) -> Option<String> {
    let length = socket.as_os_str().len();
    (length > MAX_SOCKET_PATH).then(|| {
        format!(
            "the socket path is {length} bytes and this system allows {MAX_SOCKET_PATH}; set CLAUDE_PLUS_HOME to a shorter directory"
        )
    })
}

pub fn check_socket_path(socket: &Path) -> Result<()> {
    match socket_path_problem(socket) {
        Some(problem) => bail!("cannot listen on {}: {problem}", socket.display()),
        None => Ok(()),
    }
}

pub fn listen(socket: &Path) -> Result<UnixListener> {
    check_socket_path(socket)?;
    match std::fs::remove_file(socket) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(error).with_context(|| format!("cannot remove {}", socket.display()));
        }
        _ => {}
    }
    let listener = UnixListener::bind(socket)
        .with_context(|| format!("cannot listen on {}", socket.display()))?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("cannot restrict {} to its owner", socket.display()))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

pub async fn serve(listener: UnixListener, state: Arc<State>) -> Result<()> {
    let listener = tokio::net::UnixListener::from_std(listener)?;
    let graceful = GracefulShutdown::new();
    let mut writer_exited = false;
    loop {
        tokio::select! {
            biased;
            () = state.shutdown.triggered() => break,
            () = state.writer.exited() => {
                state.log.error(format!("{WRITER_EXITED}, stopping"));
                writer_exited = true;
                break;
            }
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else {
                    continue;
                };
                let mut builder = http1::Builder::new();
                builder.timer(TokioTimer::new()).header_read_timeout(state.request_timeout);
                let state = state.clone();
                let service = service_fn(move |request| {
                    let state = state.clone();
                    async move { Ok::<_, Infallible>(handle(state, request).await) }
                });
                let connection = builder
                    .serve_connection(TokioIo::new(stream), service);
                let connection = graceful.watch(connection);
                tokio::spawn(async move {
                    let _ = connection.await;
                });
            }
        }
    }
    state.writer.cancel();
    let _ = tokio::time::timeout(DRAIN, graceful.shutdown()).await;
    if writer_exited {
        bail!(WRITER_EXITED);
    }
    Ok(())
}

type Reply = Response<Full<Bytes>>;

pub async fn handle<B>(state: Arc<State>, request: Request<B>) -> Reply
where
    B: Body<Data = Bytes>,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    match (&method, path.as_str()) {
        (&Method::GET, "/health") => json(StatusCode::OK, &health(&state)),
        (&Method::GET, "/snapshot") => snapshot(state, request.uri().query()).await,
        (&Method::POST, "/events") => events(state, request.into_body()).await,
        (&Method::POST, "/import") => {
            state.log.info("import requested over the socket");
            import(&state).await
        }
        (&Method::POST, "/reindex") => {
            state.log.info("reindex requested over the socket");
            reindex(&state).await
        }
        (&Method::POST, "/shutdown") => {
            state.log.info("shutdown requested over the socket");
            state.writer.cancel();
            state.shutdown.trigger();
            json(StatusCode::ACCEPTED, &serde_json::json!({"stopping": true}))
        }
        _ => not_routed(&method, &path),
    }
}

async fn import(state: &State) -> Reply {
    match state.writer.import().await {
        Ok(outcome) => json(
            StatusCode::OK,
            &ImportOutcome {
                report: outcome.report,
                reconcile_error: outcome.reconcile_error,
            },
        ),
        Err(error) => failure(StatusCode::SERVICE_UNAVAILABLE, format!("{error:#}")),
    }
}

async fn reindex(state: &State) -> Reply {
    match state.writer.reindex().await {
        Ok(rebuilt) => json(StatusCode::OK, &rebuilt),
        Err(error) => failure(StatusCode::SERVICE_UNAVAILABLE, format!("{error:#}")),
    }
}

fn not_routed(method: &Method, path: &str) -> Reply {
    match ROUTES.iter().find(|(route, _)| *route == path) {
        Some((_, allowed)) => {
            let mut reply = failure(
                StatusCode::METHOD_NOT_ALLOWED,
                format!("use {allowed} for {path}"),
            );
            reply
                .headers_mut()
                .insert(ALLOW, allowed.as_str().parse().expect("valid header"));
            reply
        }
        None => failure(
            StatusCode::NOT_FOUND,
            format!("no route for {method} {path}"),
        ),
    }
}

fn health(state: &State) -> Health {
    let status = state.writer.status();
    Health {
        v: API_VERSION,
        version: env!("CARGO_PKG_VERSION").to_string(),
        schema: Store::latest_schema_version(),
        pid: std::process::id(),
        uptime_s: state.started.elapsed().as_secs(),
        claude_dir: state.claude_dir.display().to_string(),
        ingest: IngestHealth {
            lag_ms: status.lag(Instant::now()).as_millis() as u64,
            backlog: status.backlog(),
            last_ingest_ms: status.last_ingest.map(epoch_ms),
            progress: status
                .progress
                .map(|(done, total)| Progress { done, total }),
            watching: state.watching.load(Ordering::Relaxed),
            last_error: status.last_error.as_ref().map(|f| f.message.clone()),
            last_error_at_ms: status.last_error.as_ref().map(|f| epoch_ms(f.at)),
            rebuilding: rebuilding(state),
        },
    }
}

fn rebuilding(state: &State) -> bool {
    let reader = state.reader.lock().unwrap_or_else(|p| p.into_inner());
    reader.reindex_pending().unwrap_or(false)
}

async fn snapshot(state: Arc<State>, query: Option<&str>) -> Reply {
    let session = query
        .unwrap_or("")
        .split('&')
        .find_map(|pair| pair.strip_prefix("session="))
        .map(str::to_string);
    let Some(session) = session else {
        return failure(
            StatusCode::BAD_REQUEST,
            "session query parameter is required",
        );
    };
    if !events::is_session_id(&session) {
        return failure(StatusCode::BAD_REQUEST, events::SESSION_RULE);
    }
    let built = tokio::task::spawn_blocking(move || -> Result<Option<Snapshot>> {
        let reader = state.reader.lock().unwrap_or_else(|p| p.into_inner());
        reader.reading(|reader| {
            if reader.reindex_pending()? {
                return Ok(None);
            }
            let now_ms = epoch_ms(SystemTime::now());
            let totals = reader.session_totals(&session)?;
            let measure = reader.latest_event(&session, MEASURE_KIND)?;
            let live = Live::read(reader, &state.prices, &session, measure.as_ref(), now_ms)?;
            Snapshot::of(&session, totals, measure, live, now_ms).map(Some)
        })
    })
    .await;
    match built {
        Ok(Ok(Some(snapshot))) => json(StatusCode::OK, &snapshot),
        Ok(Ok(None)) => failure(StatusCode::SERVICE_UNAVAILABLE, REBUILDING),
        Ok(Err(error)) => failure(StatusCode::INTERNAL_SERVER_ERROR, format!("{error:#}")),
        Err(error) => failure(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn events<B>(state: Arc<State>, body: B) -> Reply
where
    B: Body<Data = Bytes>,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let collected = Limited::new(body, MAX_BODY_BYTES).collect();
    let Ok(collected) = tokio::time::timeout(state.request_timeout, collected).await else {
        return failure(
            StatusCode::REQUEST_TIMEOUT,
            format!(
                "the request body did not arrive within {} ms",
                state.request_timeout.as_millis()
            ),
        );
    };
    let bytes = match collected {
        Ok(collected) => collected.to_bytes(),
        Err(error) if error.is::<LengthLimitError>() => {
            return failure(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request body is larger than 1 MiB",
            );
        }
        Err(error) => return failure(StatusCode::BAD_REQUEST, error.to_string()),
    };
    let body: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(body) => body,
        Err(error) => {
            return failure(
                StatusCode::BAD_REQUEST,
                format!("body is not valid JSON: {error}"),
            );
        }
    };
    let validated = match events::validate(&body, epoch_ms(SystemTime::now())) {
        Ok(validated) => validated,
        Err(message) => return failure(StatusCode::BAD_REQUEST, message),
    };
    let accepted = validated.rows.len();
    if accepted > 0
        && let Err(error) = state.writer.store_events(validated.rows).await
    {
        return failure(StatusCode::SERVICE_UNAVAILABLE, format!("{error:#}"));
    }
    json(
        StatusCode::OK,
        &EventsAccepted {
            accepted,
            rejected: validated.rejected,
        },
    )
}

fn json(status: StatusCode, body: &impl Serialize) -> Reply {
    let bytes = serde_json::to_vec(body).expect("wire types serialize");
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(bytes)))
        .expect("valid response")
}

fn failure(status: StatusCode, message: impl Into<String>) -> Reply {
    json(
        status,
        &ErrorBody {
            error: message.into(),
        },
    )
}

pub fn epoch_ms(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
