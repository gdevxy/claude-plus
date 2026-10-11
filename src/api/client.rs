use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::header::{CONTENT_TYPE, HOST};
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use serde::de::DeserializeOwned;
use tokio::net::UnixStream;

use super::wire::{ErrorBody, EventsAccepted, Health, ImportOutcome, Snapshot};
use crate::context_model::ContextReport;
use crate::history::{HistoryReport, Range};
use crate::ingest::Rebuilt;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);
const LONG_TIMEOUT: Duration = Duration::from_secs(3600);

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("no claude-plus daemon is listening on {0}")]
    NotRunning(String),
    #[error("the claude-plus daemon did not answer within {0} ms")]
    Timeout(u128),
    #[error("{message}")]
    Status { status: u16, message: String },
    #[error("talking to the claude-plus daemon failed: {0}")]
    Transport(String),
}

pub struct Client {
    socket: PathBuf,
    timeout: Duration,
    runtime: tokio::runtime::Runtime,
}

impl Client {
    pub fn new(socket: &Path) -> Self {
        Self {
            socket: socket.to_path_buf(),
            timeout: DEFAULT_TIMEOUT,
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread runtime always builds"),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn health(&self) -> Result<Health, ClientError> {
        self.call(Method::GET, "/health", None, self.timeout)
    }

    pub fn snapshot(&self, session: &str) -> Result<Snapshot, ClientError> {
        self.call(
            Method::GET,
            &format!("/snapshot?session={session}"),
            None,
            self.timeout,
        )
    }

    pub fn context(&self, session: &str) -> Result<ContextReport, ClientError> {
        self.call(
            Method::GET,
            &format!("/context?session={session}"),
            None,
            self.timeout,
        )
    }

    pub fn history(&self, range: Range) -> Result<HistoryReport, ClientError> {
        self.call(
            Method::GET,
            &format!("/history?range={}", range.as_str()),
            None,
            self.timeout,
        )
    }

    pub fn send_events(&self, body: &serde_json::Value) -> Result<EventsAccepted, ClientError> {
        self.call(
            Method::POST,
            "/events",
            Some(body.to_string().into_bytes()),
            self.timeout,
        )
    }

    pub fn import(&self) -> Result<ImportOutcome, ClientError> {
        self.call(Method::POST, "/import", Some(Vec::new()), LONG_TIMEOUT)
    }

    pub fn reindex(&self) -> Result<Rebuilt, ClientError> {
        self.call(Method::POST, "/reindex", Some(Vec::new()), LONG_TIMEOUT)
    }

    pub fn shutdown(&self) -> Result<(), ClientError> {
        self.call::<serde_json::Value>(Method::POST, "/shutdown", Some(Vec::new()), self.timeout)
            .map(|_| ())
    }

    pub fn raw(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<(StatusCode, Bytes), ClientError> {
        self.runtime
            .block_on(exchange(&self.socket, method, path, body, self.timeout))
    }

    fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<T, ClientError> {
        let (status, bytes) =
            self.runtime
                .block_on(exchange(&self.socket, method, path, body, timeout))?;
        if !status.is_success() {
            let message = serde_json::from_slice::<ErrorBody>(&bytes)
                .map(|body| body.error)
                .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned());
            return Err(ClientError::Status {
                status: status.as_u16(),
                message,
            });
        }
        serde_json::from_slice(&bytes).map_err(|error| ClientError::Transport(error.to_string()))
    }
}

async fn exchange(
    socket: &Path,
    method: Method,
    path: &str,
    body: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<(StatusCode, Bytes), ClientError> {
    let transport = |error: &dyn std::fmt::Display| ClientError::Transport(error.to_string());
    let attempt = async {
        let stream = UnixStream::connect(socket)
            .await
            .map_err(|error| match error.kind() {
                ErrorKind::NotFound | ErrorKind::ConnectionRefused => {
                    ClientError::NotRunning(socket.display().to_string())
                }
                _ => transport(&error),
            })?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|e| transport(&e))?;
        tokio::spawn(connection);
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(HOST, "claude-plus");
        if body.is_some() {
            request = request.header(CONTENT_TYPE, "application/json");
        }
        let request = request
            .body(Full::new(Bytes::from(body.unwrap_or_default())))
            .map_err(|e| transport(&e))?;
        let response = sender
            .send_request(request)
            .await
            .map_err(|e| transport(&e))?;
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|e| transport(&e))?
            .to_bytes();
        Ok((status, bytes))
    };
    tokio::time::timeout(timeout, attempt)
        .await
        .unwrap_or(Err(ClientError::Timeout(timeout.as_millis())))
}
