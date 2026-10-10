pub mod control;
pub mod lock;
pub mod log;
pub mod writer;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::signal::unix::{SignalKind, signal};

use crate::api::{self, Shutdown, State};
use crate::ingest::TranscriptFile;
use crate::ingest::watch::{Batch, Changes, watch};
use crate::paths::Paths;
use crate::pricing::PriceTable;
use crate::store::Store;
use lock::{AlreadyRunning, InstanceLock};
use log::Log;

pub const RESCAN_EVERY: Duration = Duration::from_secs(60);
const REWATCH_WINDOW: Duration = Duration::from_secs(2);
const REWATCH_EVERY: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Foreground,
    Detached,
}

pub fn run(paths: &Paths, prices: PriceTable, mode: Mode, rescan_every: Duration) -> Result<()> {
    if mode == Mode::Detached {
        let _ = rustix::process::setsid();
    }
    api::check_socket_path(&paths.socket())?;
    paths.ensure_data_dir()?;
    let log = Arc::new(Log::open(&paths.daemon_log(), mode == Mode::Foreground)?);
    let result = run_locked(paths, prices, mode, rescan_every, &log);
    if let Err(error) = &result
        && mode == Mode::Detached
    {
        log.error(format!("{error:#}"));
    }
    result
}

fn run_locked(
    paths: &Paths,
    prices: PriceTable,
    mode: Mode,
    rescan_every: Duration,
    log: &Arc<Log>,
) -> Result<()> {
    let _lock = match InstanceLock::acquire(&paths.daemon_lock()) {
        Err(error) if mode == Mode::Detached && error.is::<AlreadyRunning>() => {
            log.info(format!("{error}, leaving it in charge"));
            return Ok(());
        }
        lock => lock?,
    };
    let store = Store::open(&paths.database())?;
    let reader = Store::open_read_only(&paths.database())?;
    let listener = api::listen(&paths.socket())?;
    let (writer, writer_thread) = writer::spawn(store, prices, paths.projects_dir(), log.clone())?;
    let state = Arc::new(State {
        writer: writer.clone(),
        reader: Mutex::new(reader),
        started: Instant::now(),
        claude_dir: paths.claude_dir.clone(),
        watching: AtomicBool::new(false),
        shutdown: Shutdown::default(),
        log: log.clone(),
        request_timeout: api::REQUEST_TIMEOUT,
    });
    let served = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let mut terminate = signal(SignalKind::terminate())?;
            let mut interrupt = signal(SignalKind::interrupt())?;
            let shutdown = state.shutdown.clone();
            let signal_log = log.clone();
            let signals = tokio::spawn(async move {
                let name = tokio::select! {
                    _ = terminate.recv() => "SIGTERM",
                    _ = interrupt.recv() => "SIGINT",
                };
                signal_log.info(format!("{name} received, stopping"));
                shutdown.trigger();
            });
            let tail = tokio::spawn(tail(paths.projects_dir(), state.clone(), rescan_every));
            log.info(format!(
                "claude-plus daemon {} listening on {} (pid {})",
                env!("CARGO_PKG_VERSION"),
                paths.socket().display(),
                std::process::id()
            ));
            writer.scan();
            let served = api::serve(listener, state.clone()).await;
            tail.abort();
            signals.abort();
            served
        });
    writer.stop();
    let _ = writer_thread.join();
    let _ = std::fs::remove_file(paths.socket());
    log.info("stopped");
    served
}

async fn tail(projects_dir: PathBuf, state: Arc<State>, rescan_every: Duration) {
    let mut rescan =
        tokio::time::interval_at(tokio::time::Instant::now() + rescan_every, rescan_every);
    rescan.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut changes: Option<Changes> = None;
    let mut warned = false;
    let mut rewatch_until: Option<tokio::time::Instant> = None;
    let mut scan_once_watched = false;
    loop {
        if changes.is_none() {
            match watch(&projects_dir) {
                Ok(watching) => {
                    state
                        .log
                        .info(format!("watching {}", projects_dir.display()));
                    state.watching.store(true, Ordering::Relaxed);
                    changes = Some(watching);
                    warned = false;
                    rewatch_until = None;
                    if std::mem::take(&mut scan_once_watched) {
                        state.writer.scan();
                    }
                }
                Err(error) if !warned => {
                    state.log.warn(format!(
                        "{error:#}, retrying every {} s",
                        rescan_every.as_secs()
                    ));
                    warned = true;
                }
                Err(_) => {}
            }
        }
        let rewatching = changes.is_none()
            && rewatch_until.is_some_and(|until| tokio::time::Instant::now() < until);
        tokio::select! {
            batch = next_batch(&mut changes) => match batch {
                Some(batch) => {
                    if !batch.paths.is_empty() {
                        let files = batch.paths.iter().map(|path| TranscriptFile::from_path(path)).collect();
                        state.writer.ingest(files, batch.noticed_at);
                    }
                    if batch.root_lost {
                        state.log.warn(format!("lost the watch on {}, watching it again", projects_dir.display()));
                        state.watching.store(false, Ordering::Relaxed);
                        changes = None;
                        rewatch_until = Some(tokio::time::Instant::now() + REWATCH_WINDOW);
                        scan_once_watched = true;
                    } else if batch.rescan {
                        state.writer.scan();
                    }
                }
                None => {
                    state.watching.store(false, Ordering::Relaxed);
                    changes = None;
                }
            },
            _ = rescan.tick() => state.writer.scan(),
            () = tokio::time::sleep(REWATCH_EVERY), if rewatching => {}
        }
    }
}

async fn next_batch(changes: &mut Option<Changes>) -> Option<Batch> {
    match changes {
        Some(changes) => changes.next_batch().await,
        None => std::future::pending().await,
    }
}
