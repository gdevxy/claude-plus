use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Result, anyhow};
use tokio::sync::oneshot;

use super::log::Log;
use crate::ingest::{IngestReport, Ingestor, Rebuilt, TranscriptFile};
use crate::pricing::PriceTable;
use crate::store::{EventRow, Store};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScanOutcome {
    pub report: IngestReport,
    pub reconcile_error: Option<String>,
}

impl ScanOutcome {
    fn of(report: IngestReport) -> Self {
        Self {
            reconcile_error: report.reconcile_error.clone(),
            report,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct IngestStatus {
    backlog: VecDeque<Instant>,
    pub last_ingest: Option<SystemTime>,
    pub progress: Option<(usize, usize)>,
    pub last_error: Option<String>,
}

impl IngestStatus {
    pub fn lag(&self, now: Instant) -> Duration {
        self.backlog.front().map_or(Duration::ZERO, |oldest| {
            now.saturating_duration_since(*oldest)
        })
    }

    pub fn backlog(&self) -> usize {
        self.backlog.len()
    }
}

type Reply<T> = oneshot::Sender<Result<T, String>>;

enum Job {
    Files(Vec<TranscriptFile>),
    Scan(Option<(u64, Reply<ScanOutcome>)>),
    Events(Vec<EventRow>, Reply<()>),
    Reindex(Reply<Rebuilt>),
    Stop,
    #[cfg(test)]
    Hold(mpsc::Receiver<()>),
}

#[derive(Default)]
struct Shared {
    status: Mutex<IngestStatus>,
    collectors: Mutex<BTreeMap<u64, IngestReport>>,
    next_collector: AtomicU64,
}

#[derive(Clone)]
pub struct WriterHandle {
    jobs: mpsc::Sender<Job>,
    shared: Arc<Shared>,
    cancel: Arc<AtomicBool>,
}

const STOPPING: &str = "the claude-plus daemon is stopping";

impl WriterHandle {
    pub fn ingest(&self, files: Vec<TranscriptFile>, noticed_at: Instant) {
        self.enqueue_ingest(Job::Files(files), noticed_at);
    }

    pub fn scan(&self) {
        self.enqueue_ingest(Job::Scan(None), Instant::now());
    }

    pub async fn import(&self) -> Result<ScanOutcome> {
        let id = self.shared.next_collector.fetch_add(1, Ordering::Relaxed);
        lock(&self.shared.collectors).insert(id, IngestReport::default());
        let (reply, answer) = oneshot::channel();
        self.enqueue_ingest(Job::Scan(Some((id, reply))), Instant::now());
        let outcome = wait(answer).await;
        lock(&self.shared.collectors).remove(&id);
        outcome
    }

    pub async fn store_events(&self, rows: Vec<EventRow>) -> Result<()> {
        let (reply, answer) = oneshot::channel();
        self.send(Job::Events(rows, reply));
        wait(answer).await
    }

    pub async fn reindex(&self) -> Result<Rebuilt> {
        let (reply, answer) = oneshot::channel();
        self.send(Job::Reindex(reply));
        wait(answer).await
    }

    pub fn status(&self) -> IngestStatus {
        lock(&self.shared.status).clone()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn stop(&self) {
        self.cancel();
        self.send(Job::Stop);
    }

    fn enqueue_ingest(&self, job: Job, noticed_at: Instant) {
        let mut status = lock(&self.shared.status);
        if self.jobs.send(job).is_ok() {
            status.backlog.push_back(noticed_at);
        }
    }

    fn send(&self, job: Job) {
        let _ = self.jobs.send(job);
    }

    #[cfg(test)]
    pub(crate) fn hold(&self) -> mpsc::Sender<()> {
        let (release, held) = mpsc::channel();
        self.send(Job::Hold(held));
        release
    }
}

async fn wait<T>(answer: oneshot::Receiver<Result<T, String>>) -> Result<T> {
    match answer.await {
        Ok(result) => result.map_err(|message| anyhow!(message)),
        Err(_) => Err(anyhow!(STOPPING)),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn spawn(
    store: Store,
    prices: PriceTable,
    projects_dir: PathBuf,
    log: Arc<Log>,
) -> Result<(WriterHandle, JoinHandle<()>)> {
    let (jobs, queue) = mpsc::channel();
    let handle = WriterHandle {
        jobs,
        shared: Arc::new(Shared::default()),
        cancel: Arc::new(AtomicBool::new(false)),
    };
    let mut worker = Worker {
        store,
        prices,
        projects_dir,
        log,
        shared: handle.shared.clone(),
        cancel: handle.cancel.clone(),
    };
    let thread = std::thread::Builder::new()
        .name("claude-plus-writer".into())
        .spawn(move || worker.run(queue))?;
    Ok((handle, thread))
}

struct Worker {
    store: Store,
    prices: PriceTable,
    projects_dir: PathBuf,
    log: Arc<Log>,
    shared: Arc<Shared>,
    cancel: Arc<AtomicBool>,
}

impl Worker {
    fn run(&mut self, queue: mpsc::Receiver<Job>) {
        let mut deferred = VecDeque::new();
        loop {
            let Some(job) = deferred.pop_front().or_else(|| queue.recv().ok()) else {
                break;
            };
            let mut drain = |store: &mut Store| {
                while let Ok(job) = queue.try_recv() {
                    match job {
                        Job::Events(rows, reply) => answer(reply, store_events(store, &rows)),
                        #[cfg(test)]
                        Job::Hold(held) => {
                            let _ = held.recv();
                        }
                        other => deferred.push_back(other),
                    }
                }
            };
            match job {
                Job::Files(files) => {
                    let outcome = self.ingest_files(&files, &mut drain);
                    self.finish_ingest(outcome, None);
                }
                Job::Scan(reply) => {
                    let outcome = self.scan(&mut drain);
                    self.finish_ingest(outcome, reply);
                }
                Job::Events(rows, reply) => answer(reply, store_events(&mut self.store, &rows)),
                Job::Reindex(reply) => {
                    let rebuilt = self.reindex(&mut drain);
                    answer(reply, self.unless_cancelled(rebuilt));
                }
                Job::Stop => break,
                #[cfg(test)]
                Job::Hold(held) => {
                    let _ = held.recv();
                }
            }
        }
    }

    fn ingestor<'a>(&'a mut self, drain: &'a mut dyn FnMut(&mut Store)) -> Ingestor<'a> {
        Ingestor::new(&mut self.store, &self.prices)
            .cancelled_by(&self.cancel)
            .pausing_with(drain)
    }

    fn ingest_files(
        &mut self,
        files: &[TranscriptFile],
        drain: &mut dyn FnMut(&mut Store),
    ) -> Result<ScanOutcome> {
        let report = self.ingestor(drain).ingest_listed(files, &mut |_, _| {})?;
        Ok(ScanOutcome::of(report))
    }

    fn scan(&mut self, drain: &mut dyn FnMut(&mut Store)) -> Result<ScanOutcome> {
        if self.is_cancelled() {
            return Err(anyhow!(STOPPING));
        }
        let shared = self.shared.clone();
        let mut progress = |done: usize, total: usize| {
            lock(&shared.status).progress = Some((done, total));
        };
        let projects_dir = self.projects_dir.clone();
        let mut ingestor = self.ingestor(drain);
        ingestor.finish_interrupted_reindex(&mut progress)?;
        let report = ingestor.ingest_all(&projects_dir, &mut progress)?;
        Ok(ScanOutcome::of(report))
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn unless_cancelled<T>(&self, outcome: Result<T>) -> Result<T> {
        if self.is_cancelled() {
            Err(anyhow!(STOPPING))
        } else {
            outcome
        }
    }

    fn finish_ingest(
        &mut self,
        outcome: Result<ScanOutcome>,
        reply: Option<(u64, Reply<ScanOutcome>)>,
    ) {
        if self.is_cancelled() {
            let mut status = lock(&self.shared.status);
            status.backlog.pop_front();
            status.progress = None;
            if let Some((id, reply)) = reply {
                lock(&self.shared.collectors).remove(&id);
                let _ = reply.send(Err(STOPPING.into()));
            }
            return;
        }
        let outcome = outcome.map_err(|error| format!("{error:#}"));
        let problem = match &outcome {
            Ok(done) => done
                .reconcile_error
                .as_ref()
                .map(|error| format!("reconcile failed: {error}")),
            Err(error) => Some(format!("ingest failed: {error}")),
        };
        if let Some(problem) = &problem {
            self.log.warn(problem);
        }
        {
            let mut status = lock(&self.shared.status);
            status.backlog.pop_front();
            status.progress = None;
            status.last_error = problem;
            if let Ok(done) = &outcome {
                status.last_ingest = Some(SystemTime::now());
                for collected in lock(&self.shared.collectors).values_mut() {
                    collected.add(&done.report);
                }
            }
        }
        if let Some((id, reply)) = reply {
            let collected = lock(&self.shared.collectors).remove(&id);
            let _ = reply.send(outcome.map(|done| ScanOutcome {
                report: collected.unwrap_or(done.report),
                reconcile_error: done.reconcile_error,
            }));
        }
    }

    fn reindex(&mut self, drain: &mut dyn FnMut(&mut Store)) -> Result<Rebuilt> {
        if self.is_cancelled() {
            return Err(anyhow!(STOPPING));
        }
        let shared = self.shared.clone();
        let rebuilt = self.ingestor(drain).reindex(&mut |done, total| {
            lock(&shared.status).progress = Some((done, total));
        });
        lock(&self.shared.status).progress = None;
        rebuilt
    }
}

fn store_events(store: &mut Store, rows: &[EventRow]) -> Result<()> {
    let mut writer = store.writer()?;
    for row in rows {
        writer.event(row)?;
    }
    writer.commit()
}

fn answer<T>(reply: Reply<T>, result: Result<T>) {
    let _ = reply.send(result.map_err(|error| format!("{error:#}")));
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPUS: &str = r#"{"input_tokens":10,"output_tokens":1000}"#;

    fn line(session: &str, request: &str) -> String {
        format!(
            r#"{{"type":"assistant","sessionId":"{session}","timestamp":"2026-10-01T10:00:00Z","requestId":"{request}","message":{{"model":"claude-opus-5-5","content":[],"usage":{OPUS}}}}}"#
        ) + "\n"
    }

    struct Fixture {
        temp: tempfile::TempDir,
        projects: PathBuf,
        handle: WriterHandle,
        thread: Option<JoinHandle<()>>,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let projects = temp.path().join("projects");
            std::fs::create_dir_all(projects.join("proj")).unwrap();
            let store = Store::open(&temp.path().join("data.db")).unwrap();
            let log = Arc::new(Log::open(&temp.path().join("daemon.log"), false).unwrap());
            let (handle, thread) =
                spawn(store, PriceTable::bundled(), projects.clone(), log).unwrap();
            Self {
                temp,
                projects,
                handle,
                thread: Some(thread),
            }
        }

        fn write(&self, name: &str, content: &str) -> TranscriptFile {
            let path = self.projects.join("proj").join(name);
            std::fs::write(&path, content).unwrap();
            TranscriptFile::from_path(&path)
        }

        fn reader(&self) -> Store {
            Store::open_read_only(&self.temp.path().join("data.db")).unwrap()
        }

        fn stop(&mut self) {
            self.handle.stop();
            self.thread.take().unwrap().join().unwrap();
        }
    }

    #[tokio::test]
    async fn an_import_reports_everything_ingested_while_it_waited() {
        let fx = Fixture::new();
        let release = fx.handle.hold();
        let a = fx.write("a.jsonl", &line("a", "ra"));
        fx.handle.ingest(vec![a], Instant::now());
        fx.write("b.jsonl", &line("b", "rb"));
        let import = tokio::spawn({
            let handle = fx.handle.clone();
            async move { handle.import().await }
        });
        while fx.handle.status().backlog() < 2 {
            tokio::task::yield_now().await;
        }
        assert!(fx.handle.status().lag(Instant::now()) > Duration::ZERO);
        release.send(()).unwrap();
        let outcome = import.await.unwrap().unwrap();
        assert_eq!(
            (
                outcome.report.files_changed,
                outcome.report.requests_inserted
            ),
            (2, 2)
        );
        assert_eq!(outcome.reconcile_error, None);
        let status = fx.handle.status();
        assert_eq!((status.backlog(), status.progress), (0, None));
        assert_eq!(status.lag(Instant::now()), Duration::ZERO);
        assert!(status.last_ingest.is_some());
        assert_eq!(fx.reader().totals().unwrap().requests, 2);
    }

    #[tokio::test]
    async fn events_are_stored_in_one_transaction() {
        let fx = Fixture::new();
        let rows: Vec<EventRow> = (1..=3)
            .map(|ts_ms| EventRow {
                session_id: "s1".into(),
                kind: "session.measure".into(),
                ts_ms,
                received_ms: 10,
                json: format!(r#"{{"n":{ts_ms}}}"#),
            })
            .collect();
        fx.handle.store_events(rows).await.unwrap();
        let latest = fx.reader().latest_event("s1", "session.measure").unwrap();
        assert_eq!(latest.unwrap().json, r#"{"n":3}"#);
    }

    #[tokio::test]
    async fn events_posted_during_a_scan_are_stored_between_its_files() {
        let fx = Fixture::new();
        fx.write("a.jsonl", &line("a", "ra"));
        fx.write("b.jsonl", &line("b", "rb"));
        let release = fx.handle.hold();
        let import = tokio::spawn({
            let handle = fx.handle.clone();
            async move { handle.import().await }
        });
        while fx.handle.status().backlog() < 1 {
            tokio::task::yield_now().await;
        }
        let row = EventRow {
            session_id: "a".into(),
            kind: "session.measure".into(),
            ts_ms: 1,
            received_ms: 1,
            json: "{}".into(),
        };
        let events = tokio::spawn({
            let handle = fx.handle.clone();
            async move { handle.store_events(vec![row]).await }
        });
        tokio::task::yield_now().await;
        let mid_scan = fx.handle.hold();
        release.send(()).unwrap();
        events.await.unwrap().unwrap();
        assert!(!import.is_finished());
        assert_eq!(fx.reader().totals().unwrap().requests, 1);
        mid_scan.send(()).unwrap();
        let outcome = import.await.unwrap().unwrap();
        assert_eq!(outcome.report.requests_inserted, 2);
        assert!(
            fx.reader()
                .latest_event("a", "session.measure")
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn reindex_runs_in_the_writer() {
        let fx = Fixture::new();
        fx.write("a.jsonl", &line("a", "ra"));
        fx.handle.import().await.unwrap();
        let rebuilt = fx.handle.reindex().await.unwrap();
        assert_eq!(
            rebuilt,
            Rebuilt {
                requests: 1,
                records: 1
            }
        );
    }

    #[tokio::test]
    async fn requests_queued_behind_a_stop_are_answered_with_an_error() {
        let mut fx = Fixture::new();
        let release = fx.handle.hold();
        let import = tokio::spawn({
            let handle = fx.handle.clone();
            async move { handle.import().await }
        });
        while fx.handle.status().backlog() < 1 {
            tokio::task::yield_now().await;
        }
        fx.handle.stop();
        release.send(()).unwrap();
        let error = import.await.unwrap().unwrap_err();
        assert_eq!(error.to_string(), STOPPING);
        fx.thread.take().unwrap().join().unwrap();
        let mut stopped = Fixture::new();
        stopped.stop();
        let error = stopped.handle.reindex().await.unwrap_err();
        assert_eq!(error.to_string(), STOPPING);
    }

    #[tokio::test]
    async fn a_cancelled_import_is_answered_as_stopping_while_events_are_still_stored() {
        let mut fx = Fixture::new();
        fx.write("a.jsonl", &line("a", "ra"));
        let release = fx.handle.hold();
        let import = tokio::spawn({
            let handle = fx.handle.clone();
            async move { handle.import().await }
        });
        while fx.handle.status().backlog() < 1 {
            tokio::task::yield_now().await;
        }
        fx.handle.cancel();
        release.send(()).unwrap();
        let error = import.await.unwrap().unwrap_err();
        assert_eq!(error.to_string(), STOPPING);
        let row = EventRow {
            session_id: "a".into(),
            kind: "session.measure".into(),
            ts_ms: 1,
            received_ms: 1,
            json: "{}".into(),
        };
        fx.handle.store_events(vec![row]).await.unwrap();
        assert!(
            fx.reader()
                .latest_event("a", "session.measure")
                .unwrap()
                .is_some()
        );
        assert_eq!(fx.handle.reindex().await.unwrap_err().to_string(), STOPPING);
        fx.stop();
    }
}
