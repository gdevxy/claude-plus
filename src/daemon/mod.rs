pub mod control;
pub mod lock;
pub mod log;
pub mod writer;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::signal::unix::{Signal, SignalKind, signal};

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
    api::check_socket_path(&paths.socket())?;
    paths.ensure_data_dir()?;
    let log = Arc::new(Log::open(&paths.daemon_log(), mode == Mode::Foreground)?);
    log::log_panics(log.clone());
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
    let mut store = Store::open(&paths.database())?;
    prune_breakdowns(&mut store, paths, log)?;
    let reader = Store::open_read_only(&paths.database())?;
    let listener = api::listen(&paths.socket())?;
    let (writer, writer_thread) =
        writer::spawn(store, prices.clone(), paths.projects_dir(), log.clone())?;
    let state = Arc::new(State {
        writer: writer.clone(),
        reader: Mutex::new(reader),
        prices,
        started: Instant::now(),
        claude_dir: paths.claude_dir.clone(),
        claude_json: paths.claude_json.clone(),
        scans: api::context::Scans::default(),
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
            let socket = paths.socket();
            let signals = tokio::spawn(async move {
                let (name, _) = next_signal(&mut terminate, &mut interrupt).await;
                signal_log.info(format!("{name} received, stopping"));
                shutdown.trigger();
                let (name, number) = next_signal(&mut terminate, &mut interrupt).await;
                signal_log.warn(format!("{name} received again, exiting now"));
                let _ = std::fs::remove_file(&socket);
                std::process::exit(128 + number);
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
            writer.stop();
            join_before_aborting(move || writer_thread.join(), signals).await;
            served
        });
    let _ = std::fs::remove_file(paths.socket());
    log.info("stopped");
    served
}

const DAY_MS: i64 = 86_400_000;

fn breakdowns_kept_since(now_ms: i64, retention_days: Option<u64>) -> i64 {
    let days = retention_days.unwrap_or(Paths::DEFAULT_RETENTION_DAYS);
    i64::try_from(days)
        .ok()
        .and_then(|days| days.checked_mul(DAY_MS))
        .and_then(|kept| now_ms.checked_sub(kept))
        .unwrap_or(i64::MIN)
}

fn prune_breakdowns(store: &mut Store, paths: &Paths, log: &Log) -> Result<()> {
    let since = breakdowns_kept_since(
        api::epoch_ms(std::time::SystemTime::now()),
        paths.retention_days(),
    );
    let mut writer = store.writer()?;
    let pruned = writer.prune_breakdowns(since)?;
    writer.commit()?;
    if pruned > 0 {
        log.info(format!(
            "pruned {pruned} context breakdowns older than the transcripts"
        ));
    }
    Ok(())
}

async fn join_before_aborting<T: Send + 'static>(
    join: impl FnOnce() -> T + Send + 'static,
    signals: tokio::task::JoinHandle<()>,
) -> Option<T> {
    let joined = tokio::task::spawn_blocking(join).await.ok();
    signals.abort();
    joined
}

async fn next_signal(terminate: &mut Signal, interrupt: &mut Signal) -> (&'static str, i32) {
    tokio::select! {
        _ = terminate.recv() => ("SIGTERM", rustix::process::Signal::TERM.as_raw()),
        _ = interrupt.recv() => ("SIGINT", rustix::process::Signal::INT.as_raw()),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakdowns_are_kept_as_long_as_claude_code_keeps_transcripts() {
        let day_ms = 86_400_000;
        assert_eq!(breakdowns_kept_since(100 * day_ms, None), 70 * day_ms);
        assert_eq!(breakdowns_kept_since(100 * day_ms, Some(7)), 93 * day_ms);
        assert_eq!(breakdowns_kept_since(day_ms, Some(u64::MAX)), i64::MIN);
    }

    #[tokio::test]
    async fn signals_are_still_handled_while_the_writer_finishes() {
        let (second, signalled) = tokio::sync::oneshot::channel::<()>();
        let (handled, was_handled) = std::sync::mpsc::channel();
        let signals = tokio::spawn(async move {
            if signalled.await.is_ok() {
                let _ = handled.send(());
            }
        });
        let joined = join_before_aborting(
            move || {
                second.send(()).unwrap();
                was_handled.recv_timeout(Duration::from_secs(2)).is_ok()
            },
            signals,
        )
        .await;
        assert_eq!(joined, Some(true));
    }
}
