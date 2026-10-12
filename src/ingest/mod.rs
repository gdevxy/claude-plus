mod discover;
mod lines;
pub mod watch;

use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;

use crate::pricing::PriceTable;
use crate::store::{
    AgentRow, AttachmentRow, Busy, CompactionRow, CostStateRow, FileCheckpoint, FileProgress,
    PromptRow, RequestRow, RequestWrite, SessionRow, Store, ToolResultRow, ToolUseRow, Writer,
};
use crate::transcript::location::Location;
use crate::transcript::redact::redact;
use crate::transcript::{Parsed, Record, parse_line};

pub use discover::{TranscriptFile, discover, is_transcript};

pub const BATCH_LINES: u64 = 5000;
pub const AGENT_TYPE_CHARS: usize = 64;
pub const SKIPPED_RAW_KINDS: &[&str] = &[
    "file-history-snapshot",
    "file-history-delta",
    "queue-operation",
    "atis-latch",
];

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IngestReport {
    pub files_seen: u64,
    pub files_changed: u64,
    pub lines: u64,
    pub requests_inserted: u64,
    pub duplicate_requests: u64,
    pub unpriced_requests: u64,
    pub parse_errors: u64,
    pub files_vanished: u64,
    pub failures: Vec<(String, String)>,
    pub sessions_touched: BTreeSet<String>,
    #[serde(skip)]
    pub reconcile_error: Option<String>,
}

impl IngestReport {
    pub fn add(&mut self, other: &IngestReport) {
        self.files_seen += other.files_seen;
        self.files_changed += other.files_changed;
        self.lines += other.lines;
        self.requests_inserted += other.requests_inserted;
        self.duplicate_requests += other.duplicate_requests;
        self.unpriced_requests += other.unpriced_requests;
        self.parse_errors += other.parse_errors;
        self.files_vanished += other.files_vanished;
        self.failures.extend(other.failures.iter().cloned());
        self.sessions_touched
            .extend(other.sessions_touched.iter().cloned());
        if other.reconcile_error.is_some() {
            self.reconcile_error.clone_from(&other.reconcile_error);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Rebuilt {
    pub requests: u64,
    pub records: u64,
}

#[derive(Debug, thiserror::Error)]
#[error("stopped before finishing, the rest is picked up on the next run")]
pub struct Cancelled;

pub struct Ingestor<'a> {
    store: &'a mut Store,
    prices: &'a PriceTable,
    batch_lines: u64,
    cancel: Option<&'a AtomicBool>,
    pause: Option<&'a mut dyn FnMut(&mut Store)>,
}

impl<'a> Ingestor<'a> {
    pub fn new(store: &'a mut Store, prices: &'a PriceTable) -> Self {
        Self {
            store,
            prices,
            batch_lines: BATCH_LINES,
            cancel: None,
            pause: None,
        }
    }

    pub fn cancelled_by(mut self, flag: &'a AtomicBool) -> Self {
        self.cancel = Some(flag);
        self
    }

    pub fn pausing_with(mut self, hook: &'a mut dyn FnMut(&mut Store)) -> Self {
        self.pause = Some(hook);
        self
    }

    fn pause(&mut self) {
        if let Some(hook) = &mut self.pause {
            hook(self.store);
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_some_and(|flag| flag.load(Ordering::Relaxed))
    }

    pub fn ingest_all(
        &mut self,
        projects_dir: &Path,
        on_file: &mut dyn FnMut(usize, usize),
    ) -> Result<IngestReport> {
        let files = discover(projects_dir)?;
        self.ingest_listed(&files, on_file)
    }

    pub(crate) fn ingest_listed(
        &mut self,
        files: &[TranscriptFile],
        on_file: &mut dyn FnMut(usize, usize),
    ) -> Result<IngestReport> {
        let mut report = IngestReport::default();
        for (index, file) in files.iter().enumerate() {
            if self.is_cancelled() {
                break;
            }
            match self.ingest_file(file) {
                Ok(file_report) => report.add(&file_report),
                Err(error) if error.is::<Busy>() => return Err(error),
                Err(error) if is_not_found(&error) => report.files_vanished += 1,
                Err(error) => report.failures.push((
                    file.path.to_string_lossy().into_owned(),
                    format!("{error:#}"),
                )),
            }
            on_file(index + 1, files.len());
            self.pause();
        }
        Ok(report)
    }

    pub fn reindex(&mut self, on_file: &mut dyn FnMut(usize, usize)) -> Result<Rebuilt> {
        let files = self.store.raw_files()?;
        let mut report = IngestReport::default();
        let mut records = 0u64;
        let mut next = 0;
        self.store.begin_reindex()?;
        let mut writer = self.store.writer()?;
        loop {
            let mut in_batch = 0;
            while next < files.len() && in_batch < self.batch_lines {
                for row in writer.raw_records(files[next].0, &[])? {
                    let Ok(parsed) = parse_line(&row.json) else {
                        continue;
                    };
                    if row.retired && is_live_only(&parsed.record) {
                        continue;
                    }
                    derive(
                        &mut writer,
                        self.prices,
                        &row.location,
                        row.file_id,
                        row.line_no,
                        &parsed,
                        &mut report,
                    )?;
                    records += 1;
                    in_batch += 1;
                }
                next += 1;
                on_file(next, files.len());
            }
            if next == files.len() {
                writer.reconcile(self.prices)?;
                writer.mark_reindex_pending(false)?;
                writer.commit()?;
                return Ok(Rebuilt {
                    requests: report.requests_inserted,
                    records,
                });
            }
            writer.commit()?;
            if self.is_cancelled() {
                return Err(Cancelled.into());
            }
            self.pause();
            writer = self.store.writer()?;
        }
    }

    pub fn finish_interrupted_reindex(
        &mut self,
        on_file: &mut dyn FnMut(usize, usize),
    ) -> Result<Option<Rebuilt>> {
        if !self.store.reindex_pending()? {
            return Ok(None);
        }
        self.reindex(on_file).map(Some)
    }

    pub fn ingest_file(&mut self, file: &TranscriptFile) -> Result<IngestReport> {
        self.ingest_file_limited(file, u64::MAX)
    }

    pub(crate) fn ingest_file_limited(
        &mut self,
        file: &TranscriptFile,
        max_lines: u64,
    ) -> Result<IngestReport> {
        let mut report = IngestReport {
            files_seen: 1,
            ..Default::default()
        };
        if let Some(agent_id) = &file.location.agent_id {
            self.record_agent_type(file, agent_id)?;
        }
        let metadata = std::fs::metadata(&file.path)?;
        let size = metadata.len();
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        let path = file.path.to_string_lossy().into_owned();
        if is_current(self.store.checkpoint(&path)?.as_ref(), size, mtime) {
            return Ok(report);
        }
        let mut writer = self.store.writer()?;
        let checkpoint = writer.checkpoint(&path)?;
        if is_current(checkpoint.as_ref(), size, mtime) {
            return Ok(report);
        }

        let replaced = match &checkpoint {
            Some(cp) if size < cp.offset => true,
            Some(cp) if cp.offset > 0 && cp.head_len > 0 => {
                lines::head_fingerprint(&file.path, cp.head_len)? != (cp.head_len, cp.head_hash)
            }
            _ => false,
        };
        let (head_len, head_hash) =
            lines::head_fingerprint(&file.path, lines::HEAD_BYTES.min(size))?;

        if let Some(cp) = checkpoint.as_ref().filter(|_| replaced) {
            writer.retire_file(cp.id)?;
        }
        let file_id = writer.file(&path, size, mtime)?;
        let (mut offset, mut line_no) = match &checkpoint {
            Some(cp) if !replaced => (cp.offset, cp.lines),
            _ => (0, 0),
        };

        let progress = |offset, lines| FileProgress {
            offset,
            lines,
            size,
            mtime,
            head_len,
            head_hash,
        };
        let mut batch_errors = 0;
        let mut in_batch = 0;
        for line in lines::complete_lines(&file.path, offset)? {
            if report.lines >= max_lines {
                break;
            }
            let line = line?;
            line_no += 1;
            report.lines += 1;
            offset = line.end_offset;
            if !line.text.trim().is_empty() {
                match parse_line(&line.text) {
                    Ok(parsed) => {
                        record_raw(&mut writer, file_id, line_no, &parsed)?;
                        derive(
                            &mut writer,
                            self.prices,
                            &file.location,
                            file_id,
                            line_no,
                            &parsed,
                            &mut report,
                        )?;
                    }
                    Err(_) => {
                        report.parse_errors += 1;
                        batch_errors += 1;
                    }
                }
            }
            in_batch += 1;
            if in_batch == self.batch_lines {
                writer.advance_file(file_id, &progress(offset, line_no), batch_errors)?;
                reconcile_touched(&mut writer, self.prices, &mut report);
                writer.commit()?;
                if self.is_cancelled() {
                    report.files_changed = 1;
                    return Ok(report);
                }
                self.pause();
                writer = self.store.writer()?;
                batch_errors = 0;
                in_batch = 0;
            }
        }
        writer.advance_file(file_id, &progress(offset, line_no), batch_errors)?;
        reconcile_touched(&mut writer, self.prices, &mut report);
        writer.commit()?;
        report.files_changed = u64::from(replaced || report.lines > 0);
        Ok(report)
    }
}

impl Ingestor<'_> {
    fn record_agent_type(&mut self, file: &TranscriptFile, agent_id: &str) -> Result<()> {
        if self.store.knows_agent(agent_id)? {
            return Ok(());
        }
        let Some(agent_type) = agent_type(&file.path.with_extension("meta.json")) else {
            return Ok(());
        };
        let mut writer = self.store.writer()?;
        writer.agent(&AgentRow {
            agent_id: agent_id.to_string(),
            session_id: file.location.session_hint.clone(),
            agent_type,
        })?;
        writer.commit()
    }
}

fn agent_type(meta: &Path) -> Option<String> {
    let text = std::fs::read_to_string(meta).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let name = value.get("agentType")?.as_str()?.trim();
    (!name.is_empty()).then(|| name.chars().take(AGENT_TYPE_CHARS).collect())
}

fn reconcile_touched(writer: &mut Writer, prices: &PriceTable, report: &mut IngestReport) {
    if let Err(error) = writer.reconcile_sessions(prices, &report.sessions_touched) {
        let error = error.context("reconciling with Claude Code counters");
        report.reconcile_error = Some(format!("{error:#}"));
    }
}

fn is_current(checkpoint: Option<&FileCheckpoint>, size: u64, mtime: i64) -> bool {
    checkpoint.is_some_and(|cp| cp.size == size && cp.mtime == mtime && cp.offset == size)
}

fn is_live_only(record: &Record) -> bool {
    matches!(record, Record::Attachment(_) | Record::CompactBoundary(_))
}

fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.kind() == ErrorKind::NotFound)
}

pub fn record_raw(writer: &mut Writer, file_id: i64, line_no: u64, parsed: &Parsed) -> Result<()> {
    let kind = parsed.record.kind();
    if !SKIPPED_RAW_KINDS.contains(&kind) {
        writer.raw(file_id, line_no, kind, &redact(&parsed.raw).to_string())?;
    }
    Ok(())
}

pub fn derive(
    writer: &mut Writer,
    prices: &PriceTable,
    location: &Location,
    file_id: i64,
    line_no: u64,
    parsed: &Parsed,
    report: &mut IngestReport,
) -> Result<()> {
    let common = parsed.record.common();
    let session_id = common
        .and_then(|c| c.session_id.clone())
        .unwrap_or_else(|| location.session_hint.clone());
    let ts = common.and_then(|c| c.timestamp).map(|t| t.timestamp());
    if common.is_some() {
        writer.session(&SessionRow {
            id: session_id.clone(),
            project: location.project.clone(),
            cwd: common.and_then(|c| c.cwd.clone()),
            ts,
        })?;
    }

    match &parsed.record {
        Record::Assistant(a) if a.is_billable() => {
            let Some(key) = a.dedup_key.clone() else {
                return Ok(());
            };
            let cost_usd = prices.cost(&a.model, &a.usage);
            let sent_ts = match parsed
                .raw
                .get("parentUuid")
                .and_then(serde_json::Value::as_str)
            {
                Some(parent) => writer.written_before(file_id, line_no, parent)?,
                None => None,
            };
            let write = writer.request(&RequestRow {
                key,
                session_id: session_id.clone(),
                ts: ts.unwrap_or(0),
                sent_ts,
                model: a.model.clone(),
                tokens: a.usage,
                cost_usd,
                is_sidechain: a.common.is_sidechain || location.agent_id.is_some(),
                agent_id: a
                    .common
                    .agent_id
                    .clone()
                    .or_else(|| location.agent_id.clone()),
                skill: a.skill.clone(),
                mcp_server: a.mcp_server.clone(),
                effort: a.effort.clone(),
            })?;
            match write {
                RequestWrite::Inserted => {
                    report.requests_inserted += 1;
                    if cost_usd.is_none() {
                        report.unpriced_requests += 1;
                    }
                }
                RequestWrite::Updated | RequestWrite::Unchanged => report.duplicate_requests += 1,
            }
            if write != RequestWrite::Unchanged {
                report.sessions_touched.insert(session_id.clone());
            }
            for tool in &a.tool_uses {
                writer.tool_use(&ToolUseRow {
                    id: tool.id.clone(),
                    session_id: session_id.clone(),
                    ts: ts.unwrap_or(0),
                    name: tool.name.clone(),
                    input_chars: tool.input_chars,
                    target: tool.target.clone(),
                })?;
            }
        }
        Record::User(u) => {
            let main_thread = !u.common.is_sidechain && location.agent_id.is_none();
            if u.is_prompt
                && main_thread
                && let (Some(uuid), Some(ts)) = (
                    parsed.raw.get("uuid").and_then(serde_json::Value::as_str),
                    ts,
                )
            {
                writer.prompt(&PromptRow {
                    uuid: uuid.to_string(),
                    session_id: session_id.clone(),
                    ts,
                })?;
            }
            for result in &u.tool_results {
                writer.tool_result(&ToolResultRow {
                    tool_use_id: result.tool_use_id.clone(),
                    session_id: session_id.clone(),
                    ts: ts.unwrap_or(0),
                    chars: result.chars,
                    images: result.images,
                    is_error: result.is_error,
                    is_sidechain: u.common.is_sidechain || location.agent_id.is_some(),
                })?;
            }
        }
        Record::Attachment(a) => {
            writer.attachment(&AttachmentRow {
                file_id,
                line_no,
                session_id,
                ts,
                kind: a.kind.clone(),
                chars: a.chars,
                names: a.names.clone(),
                removed: a.removed.clone(),
            })?;
        }
        Record::CompactBoundary(c) => {
            writer.compaction(&CompactionRow {
                file_id,
                line_no,
                session_id,
                ts,
                trigger: c.trigger.clone(),
                pre_tokens: c.pre_tokens,
                post_tokens: c.post_tokens,
            })?;
        }
        Record::CostState(c) => {
            let session_id = c
                .session_id
                .clone()
                .unwrap_or_else(|| location.session_hint.clone());
            report.sessions_touched.insert(session_id.clone());
            writer.cost_state(&CostStateRow {
                file_id,
                line_no,
                session_id,
                total_cost_usd: c.total_cost_usd,
                model_count: c.model_usage.len() as u64,
                json: redact(&parsed.raw).to_string(),
            })?;
        }
        Record::Title(t) => {
            let session_id = t
                .session_id
                .clone()
                .unwrap_or_else(|| location.session_hint.clone());
            writer.session(&SessionRow {
                id: session_id.clone(),
                project: location.project.clone(),
                cwd: None,
                ts: None,
            })?;
            writer.title(&session_id, &t.title, t.is_custom)?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Reconciliation;
    use std::io::Write;
    use std::path::PathBuf;

    fn assistant(session: &str, request: &str, ts: &str, model: &str, usage: &str) -> String {
        format!(
            r#"{{"type":"assistant","sessionId":"{session}","timestamp":"{ts}","requestId":"{request}","message":{{"model":"{model}","content":[{{"type":"text","text":"hi"}}],"usage":{usage}}}}}"#
        )
    }

    const OPUS_USAGE: &str = r#"{"input_tokens":10,"output_tokens":1000,"cache_read_input_tokens":20000,"cache_creation_input_tokens":5000,"cache_creation":{"ephemeral_1h_input_tokens":5000,"ephemeral_5m_input_tokens":0}}"#;

    struct Fixture {
        _temp: tempfile::TempDir,
        projects: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let projects = temp.path().join("projects");
            std::fs::create_dir_all(projects.join("-w-proj")).unwrap();
            Self {
                _temp: temp,
                projects,
            }
        }

        fn write(&self, relative: &str, content: &str) -> PathBuf {
            let path = self.projects.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, content).unwrap();
            path
        }

        fn append(&self, relative: &str, content: &str) {
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(self.projects.join(relative))
                .unwrap();
            file.write_all(content.as_bytes()).unwrap();
        }

        fn run(&self, store: &mut Store) -> IngestReport {
            self.run_reconciled(store).0
        }

        fn run_reconciled(&self, store: &mut Store) -> (IngestReport, Reconciliation) {
            let prices = PriceTable::bundled();
            let report = Ingestor::new(store, &prices)
                .ingest_all(&self.projects, &mut |_, _| {})
                .unwrap();
            (report, store.reconciliation_summary().unwrap())
        }
    }

    #[test]
    fn discovers_sessions_and_subagents() {
        let fx = Fixture::new();
        fx.write("-w-proj/s1.jsonl", "");
        fx.write("-w-proj/s1/subagents/agent-a1.jsonl", "");
        fx.write("-w-proj/s1/subagents/agent-a1.meta.json", "{}");
        fx.write("-w-proj/notes.txt", "");
        let files = discover(&fx.projects).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(
            (
                files[0].location.project.as_str(),
                files[0].location.session_hint.as_str(),
                files[0].location.agent_id.as_deref()
            ),
            ("-w-proj", "s1", None)
        );
        assert_eq!(
            (
                files[1].location.session_hint.as_str(),
                files[1].location.agent_id.as_deref()
            ),
            ("s1", Some("a1"))
        );
    }

    #[test]
    fn a_subagent_type_is_read_from_its_meta_file_once_it_exists() {
        let fx = Fixture::new();
        let line = assistant(
            "s1",
            "req_1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        fx.write("-w-proj/s1/subagents/agent-a1.jsonl", &format!("{line}\n"));
        fx.write("-w-proj/s1/subagents/agent-a2.jsonl", &format!("{line}\n"));
        fx.write("-w-proj/s1/subagents/agent-a2.meta.json", "{not json");
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        assert!(!store.knows_agent("a1").unwrap());
        assert!(!store.knows_agent("a2").unwrap());
        fx.write(
            "-w-proj/s1/subagents/agent-a1.meta.json",
            r#"{"agentType":"Explore","description":"Find the parser","toolUseId":"toolu_1"}"#,
        );
        fx.run(&mut store);
        let since = chrono::DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
            .unwrap()
            .timestamp();
        let types = store.agent_types_between(since, since + 86_400).unwrap();
        assert_eq!(
            types.into_iter().collect::<Vec<_>>(),
            [("a1".to_string(), "Explore".to_string())]
        );
        fx.write(
            "-w-proj/s1/subagents/agent-a1.meta.json",
            r#"{"agentType":"reviewer"}"#,
        );
        fx.run(&mut store);
        let kept = store.agent_types_between(since, since + 86_400).unwrap();
        assert_eq!(kept.get("a1").map(String::as_str), Some("Explore"));
    }

    #[test]
    fn workflow_agents_keep_their_type_and_a_long_type_is_cut() {
        let fx = Fixture::new();
        let line = assistant(
            "s1",
            "req_1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        let run = "-w-proj/s1/subagents/workflows/wf_1";
        fx.write(&format!("{run}/agent-w1.jsonl"), &format!("{line}\n"));
        let long = "x".repeat(100);
        fx.write(
            &format!("{run}/agent-w1.meta.json"),
            &format!(r#"{{"agentType":"{long}"}}"#),
        );
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        let since = chrono::DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
            .unwrap()
            .timestamp();
        let types = store.agent_types_between(since, since + 86_400).unwrap();
        assert_eq!(types.get("w1").map(String::len), Some(AGENT_TYPE_CHARS));
    }

    #[test]
    fn import_reconciles_with_claude_counters() {
        let fx = Fixture::new();
        let line = assistant(
            "s1",
            "req_1",
            "2026-10-01T10:00:00Z",
            "claude-sonnet-5-5",
            r#"{"input_tokens":1,"output_tokens":3}"#,
        );
        let counter = r#"{"type":"cost-state","sessionId":"s1","totalCostUSD":0.01,"modelUsage":{"claude-sonnet-5-5":{"inputTokens":1,"outputTokens":1003}}}"#;
        fx.write("-w-proj/s1.jsonl", &format!("{line}\n{counter}\n"));
        let mut store = Store::open_in_memory().unwrap();
        let (_, reconciliation) = fx.run_reconciled(&mut store);
        assert_eq!(reconciliation.sessions_adjusted, 1);
        assert!((reconciliation.adjusted_usd - 0.01).abs() < 1e-12);
        assert!((store.totals().unwrap().reconciled_usd - 0.01).abs() < 1e-12);
    }

    const SONNET_LINE_USAGE: &str = r#"{"input_tokens":1,"output_tokens":3}"#;

    fn sonnet_counter(output: u64, total: f64) -> String {
        format!(
            r#"{{"type":"cost-state","sessionId":"s1","totalCostUSD":{total},"modelUsage":{{"claude-sonnet-5-5":{{"inputTokens":1,"outputTokens":{output}}}}}}}"#
        )
    }

    fn ingest_only(fx: &Fixture, store: &mut Store) -> IngestReport {
        let prices = PriceTable::bundled();
        Ingestor::new(store, &prices)
            .ingest_all(&fx.projects, &mut |_, _| {})
            .unwrap()
    }

    #[test]
    fn an_ingest_rebuilds_the_adjustments_of_its_sessions_in_the_same_commit() {
        let fx = Fixture::new();
        let line = assistant(
            "s1",
            "req_1",
            "2026-10-01T10:00:00Z",
            "claude-sonnet-5-5",
            SONNET_LINE_USAGE,
        );
        fx.write(
            "-w-proj/s1.jsonl",
            &format!("{line}\n{}\n", sonnet_counter(1003, 0.01)),
        );
        let mut store = Store::open_in_memory().unwrap();
        let report = ingest_only(&fx, &mut store);
        assert_eq!(report.reconcile_error, None);
        assert!((store.totals().unwrap().reconciled_usd - 0.01).abs() < 1e-12);
    }

    #[test]
    fn a_reconcile_failure_keeps_the_ingest_and_the_previous_adjustments() {
        let fx = Fixture::new();
        let line = assistant(
            "s1",
            "req_1",
            "2026-10-01T10:00:00Z",
            "claude-sonnet-5-5",
            SONNET_LINE_USAGE,
        );
        fx.write(
            "-w-proj/s1.jsonl",
            &format!("{line}\n{}\n", sonnet_counter(1003, 0.01)),
        );
        let database = fx._temp.path().join("data.db");
        let mut store = Store::open(&database).unwrap();
        ingest_only(&fx, &mut store);
        rusqlite::Connection::open(&database)
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER no_adjustments BEFORE INSERT ON adjustments
                 BEGIN SELECT RAISE(ABORT, 'adjustments are locked'); END;",
            )
            .unwrap();
        fx.append(
            "-w-proj/s1.jsonl",
            &format!("{}\n", sonnet_counter(2003, 0.02)),
        );
        let report = ingest_only(&fx, &mut store);
        assert_eq!(report.lines, 1);
        assert_eq!(
            report.reconcile_error.as_deref(),
            Some(
                "reconciling with Claude Code counters: adjustments are locked: Error code 1811: constraint failed"
            )
        );
        assert!((store.totals().unwrap().reconciled_usd - 0.01).abs() < 1e-12);
        assert_eq!(ingest_only(&fx, &mut store).lines, 0);
    }

    fn adjusted_outputs(store: &Store) -> Vec<(String, String)> {
        store
            .dump("adjustments")
            .iter()
            .map(|row| {
                let cells: Vec<&str> = row.split('|').collect();
                (cells[0].to_string(), cells[4].to_string())
            })
            .collect()
    }

    #[test]
    fn reconcile_rebuilds_only_the_sessions_an_import_touched() {
        let fx = Fixture::new();
        let request = |session: &str, id: &str, output: u64| {
            assistant(
                session,
                id,
                "2026-10-01T10:00:00Z",
                "claude-sonnet-5-5",
                &format!(r#"{{"input_tokens":1,"output_tokens":{output}}}"#),
            )
        };
        let counter = |session: &str, output: u64| {
            format!(
                r#"{{"type":"cost-state","sessionId":"{session}","totalCostUSD":0.02,"modelUsage":{{"claude-sonnet-5-5":{{"inputTokens":1,"outputTokens":{output}}}}}}}"#
            )
        };
        for session in ["s1", "s2"] {
            fx.write(
                &format!("-w-proj/{session}.jsonl"),
                &format!(
                    "{}\n{}\n",
                    request(session, session, 3),
                    counter(session, 1003)
                ),
            );
        }
        let mut store = Store::open_in_memory().unwrap();
        let (first, _) = fx.run_reconciled(&mut store);
        assert_eq!(
            first.sessions_touched,
            BTreeSet::from(["s1".to_string(), "s2".to_string()])
        );

        let s2 = fx.projects.join("-w-proj/s2.jsonl");
        let mut w = store.writer().unwrap();
        let file_id = w.file(&s2.to_string_lossy(), 0, 0).unwrap();
        w.cost_state(&CostStateRow {
            file_id,
            line_no: 99,
            session_id: "s2".into(),
            total_cost_usd: Some(0.05),
            model_count: 1,
            json: counter("s2", 5003),
        })
        .unwrap();
        w.commit().unwrap();
        fx.append(
            "-w-proj/s1.jsonl",
            &format!("{}\n", request("s1", "s1-b", 500)),
        );

        let (second, reconciliation) = fx.run_reconciled(&mut store);
        assert_eq!(second.sessions_touched, BTreeSet::from(["s1".to_string()]));
        assert_eq!(reconciliation.sessions_adjusted, 2);
        assert_eq!(
            adjusted_outputs(&store),
            vec![
                (r#"Text("s1")"#.into(), "Integer(500)".into()),
                (r#"Text("s2")"#.into(), "Integer(1000)".into())
            ]
        );
        store.reconcile(&PriceTable::bundled()).unwrap();
        assert_eq!(adjusted_outputs(&store)[1].1, "Integer(5000)");
    }

    #[test]
    fn unchanged_files_are_skipped_without_the_write_lock() {
        let fx = Fixture::new();
        fx.write(
            "-w-proj/s1.jsonl",
            &format!(
                "{}\n",
                assistant(
                    "s1",
                    "r1",
                    "2026-10-01T10:00:00Z",
                    "claude-opus-5-5",
                    OPUS_USAGE
                )
            ),
        );
        let database = fx._temp.path().join("data.db");
        let mut store = Store::open(&database).unwrap();
        fx.run(&mut store);
        let mut holder = Store::open(&database).unwrap();
        let _held = holder.writer().unwrap();
        store.set_busy_timeout(std::time::Duration::from_millis(10));
        let (report, _) = fx.run_reconciled(&mut store);
        assert_eq!(
            (report.files_seen, report.files_changed, report.lines),
            (1, 0, 0)
        );
        assert!(report.sessions_touched.is_empty());
    }

    #[test]
    fn duplicate_requests_across_records_and_files_count_once() {
        let fx = Fixture::new();
        let line = assistant(
            "s1",
            "req_1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        fx.write("-w-proj/s1.jsonl", &format!("{line}\n{line}\n{line}\n"));
        fx.write("-w-proj/s2.jsonl", &format!("{line}\n"));
        let mut store = Store::open_in_memory().unwrap();
        let report = fx.run(&mut store);
        assert_eq!(
            (report.requests_inserted, report.duplicate_requests),
            (1, 3)
        );
        let totals = store.totals().unwrap();
        assert_eq!(totals.requests, 1);
        assert!(
            (totals.priced_cost_usd - 0.06404).abs() < 1e-9,
            "{}",
            totals.priced_cost_usd
        );
    }

    #[test]
    fn streamed_output_keeps_final_usage() {
        let fx = Fixture::new();
        let partial = assistant(
            "s1",
            "req_1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            r#"{"input_tokens":10,"output_tokens":1}"#,
        );
        let final_usage = assistant(
            "s1",
            "req_1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            r#"{"input_tokens":10,"output_tokens":1000}"#,
        );
        fx.write(
            "-w-proj/s1.jsonl",
            &format!("{partial}\n{final_usage}\n{partial}\n"),
        );
        let mut store = Store::open_in_memory().unwrap();
        let report = fx.run(&mut store);
        assert_eq!(
            (report.requests_inserted, report.duplicate_requests),
            (1, 2)
        );
        let cost = store.totals().unwrap().priced_cost_usd;
        assert!(
            (cost - (10.0 * 4.0 + 1000.0 * 20.0) / 1e6).abs() < 1e-12,
            "{cost}"
        );
    }

    #[test]
    fn truncated_last_line_is_picked_up_later() {
        let fx = Fixture::new();
        let line = assistant(
            "s1",
            "req_1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        let (head, tail) = line.split_at(40);
        fx.write("-w-proj/s1.jsonl", head);
        let mut store = Store::open_in_memory().unwrap();
        let first = fx.run(&mut store);
        assert_eq!(
            (
                first.parse_errors,
                first.requests_inserted,
                first.files_changed
            ),
            (0, 0, 0)
        );
        fx.append("-w-proj/s1.jsonl", &format!("{tail}\n"));
        let second = fx.run(&mut store);
        assert_eq!(
            (
                second.parse_errors,
                second.requests_inserted,
                second.files_changed
            ),
            (0, 1, 1)
        );
    }

    #[test]
    fn unchanged_files_are_skipped() {
        let fx = Fixture::new();
        fx.write(
            "-w-proj/s1.jsonl",
            &format!(
                "{}\n",
                assistant(
                    "s1",
                    "r",
                    "2026-10-01T10:00:00Z",
                    "claude-opus-5-5",
                    OPUS_USAGE
                )
            ),
        );
        let mut store = Store::open_in_memory().unwrap();
        assert_eq!(fx.run(&mut store).files_changed, 1);
        let again = fx.run(&mut store);
        assert_eq!(
            (again.files_seen, again.files_changed, again.lines),
            (1, 0, 0)
        );
    }

    #[test]
    fn resume_from_checkpoint_after_partial_import() {
        let fx = Fixture::new();
        let mut content = String::new();
        for i in 0..(BATCH_LINES + 10) {
            content.push_str(&assistant(
                "s1",
                &format!("r{i}"),
                "2026-10-01T10:00:00Z",
                "claude-opus-5-5",
                OPUS_USAGE,
            ));
            content.push('\n');
        }
        let path = fx.write("-w-proj/s1.jsonl", &content);
        let mut store = Store::open_in_memory().unwrap();
        let prices = PriceTable::bundled();
        let file = TranscriptFile::from_path(&path);
        Ingestor::new(&mut store, &prices)
            .ingest_file_limited(&file, BATCH_LINES)
            .unwrap();
        let checkpoint_after_first_batch =
            store.checkpoint(&path.to_string_lossy()).unwrap().unwrap();
        assert_eq!(checkpoint_after_first_batch.lines, BATCH_LINES);
        let report = fx.run(&mut store);
        assert_eq!(report.requests_inserted, 10);
        assert_eq!(store.totals().unwrap().requests, BATCH_LINES + 10);
        let line_numbers: Vec<u64> = store
            .raw_records(checkpoint_after_first_batch.id, &[])
            .unwrap()
            .iter()
            .map(|row| row.line_no)
            .collect();
        assert_eq!(line_numbers, (1..=BATCH_LINES + 10).collect::<Vec<_>>());
    }

    #[test]
    fn blank_lines_count_towards_a_batch() {
        let fx = Fixture::new();
        let line = |id: &str| {
            assistant(
                "s1",
                id,
                "2026-10-01T10:00:00Z",
                "claude-opus-5-5",
                OPUS_USAGE,
            )
        };
        let path = fx.write(
            "-w-proj/s1.jsonl",
            &format!("{}\n\n{}\n", line("r1"), line("boom")),
        );
        let database = fx._temp.path().join("data.db");
        let mut store = Store::open(&database).unwrap();
        rusqlite::Connection::open(&database)
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_on_boom BEFORE INSERT ON raw_records WHEN NEW.json LIKE '%boom%'
                 BEGIN SELECT RAISE(ABORT, 'boom'); END;",
            )
            .unwrap();
        let prices = PriceTable::bundled();
        let mut ingestor = Ingestor::new(&mut store, &prices);
        ingestor.batch_lines = 2;
        assert!(
            ingestor
                .ingest_file(&TranscriptFile::from_path(&path))
                .is_err()
        );
        let checkpoint = store.checkpoint(&path.to_string_lossy()).unwrap().unwrap();
        assert_eq!(checkpoint.lines, 2);
        assert_eq!(store.totals().unwrap().requests, 1);
    }

    #[test]
    fn sub_second_rewrites_of_the_same_size_are_noticed() {
        let fx = Fixture::new();
        let a = assistant(
            "s1",
            "r1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        let path = fx.write("-w-proj/s1.jsonl", &format!("{a}\n"));
        let second = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_791_018_000);
        let set_mtime = |time| {
            std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(time)
                .unwrap()
        };
        set_mtime(second);
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        fx.write("-w-proj/s1.jsonl", &format!("{}\n", a.replace("r1", "r2")));
        set_mtime(second + std::time::Duration::from_millis(300));
        assert_eq!(fx.run(&mut store).requests_inserted, 1);
    }

    #[test]
    fn subagent_records_without_session_id_use_the_parent_session() {
        let fx = Fixture::new();
        let line = assistant(
            "ignored",
            "r1",
            "2026-10-01T10:00:00Z",
            "claude-haiku-4-5-20251001",
            r#"{"input_tokens":100,"output_tokens":50}"#,
        )
        .replace(r#""sessionId":"ignored","#, "");
        fx.write("-w-proj/s9/subagents/agent-a1.jsonl", &format!("{line}\n"));
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        let requests = store.dump("requests");
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0].starts_with(r#"Text("r1")|Text("s9")|"#),
            "{}",
            requests[0]
        );
        assert!(requests[0].ends_with(r#"|Integer(1)|Text("a1")|Null|Null|Null|Null"#));
    }

    #[test]
    fn replaced_file_is_reread_without_double_counting() {
        let fx = Fixture::new();
        let a = assistant(
            "s1",
            "r1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        let b = assistant(
            "s1",
            "r2",
            "2026-10-01T10:01:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        fx.write("-w-proj/s1.jsonl", &format!("{a}\n{b}\n"));
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        fx.write("-w-proj/s1.jsonl", &format!("{b}\n"));
        let report = fx.run(&mut store);
        assert_eq!(
            (report.requests_inserted, report.duplicate_requests),
            (0, 1)
        );
        assert_eq!(store.totals().unwrap().requests, 2);
    }

    #[test]
    fn synthetic_and_unknown_models() {
        let fx = Fixture::new();
        let synthetic = assistant(
            "s1",
            "r1",
            "2026-10-01T10:00:00Z",
            "<synthetic>",
            OPUS_USAGE,
        );
        let unknown = assistant(
            "s1",
            "r2",
            "2026-10-01T10:00:00Z",
            "claude-future-9",
            OPUS_USAGE,
        );
        fx.write("-w-proj/s1.jsonl", &format!("{synthetic}\n{unknown}\n"));
        let mut store = Store::open_in_memory().unwrap();
        let report = fx.run(&mut store);
        assert_eq!((report.requests_inserted, report.unpriced_requests), (1, 1));
        assert_eq!(
            store.unpriced_models().unwrap(),
            vec![("claude-future-9".to_string(), 1)]
        );
    }

    #[test]
    fn non_utf8_bytes_are_tolerated() {
        let fx = Fixture::new();
        let path = fx.projects.join("-w-proj/s1.jsonl");
        let mut bytes = b"\xff\xfe garbage\n".to_vec();
        bytes.extend(
            assistant(
                "s1",
                "r1",
                "2026-10-01T10:00:00Z",
                "claude-opus-5-5",
                OPUS_USAGE,
            )
            .as_bytes(),
        );
        bytes.push(b'\n');
        std::fs::write(&path, bytes).unwrap();
        let mut store = Store::open_in_memory().unwrap();
        let report = fx.run(&mut store);
        assert_eq!((report.parse_errors, report.requests_inserted), (1, 1));
    }

    #[test]
    fn subagent_requests_belong_to_parent_session() {
        let fx = Fixture::new();
        let usage = r#"{"input_tokens":100,"output_tokens":50}"#;
        let line = assistant(
            "s1",
            "r9",
            "2026-10-01T10:00:00Z",
            "claude-haiku-4-5-20251001",
            usage,
        )
        .replacen(
            r#""type":"assistant","#,
            r#""type":"assistant","isSidechain":true,"#,
            1,
        );
        fx.write("-w-proj/s1/subagents/agent-a1.jsonl", &format!("{line}\n"));
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        let totals = store.totals().unwrap();
        assert_eq!((totals.sessions, totals.requests), (1, 1));
        assert!((totals.priced_cost_usd - 0.00035).abs() < 1e-12);
    }

    #[test]
    fn tool_results_attachments_and_raw_are_stored() {
        let fx = Fixture::new();
        let big = "y".repeat(5000);
        let lines = [
            r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-10-01T10:00:00Z","requestId":"r1","message":{"model":"claude-opus-5-5","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/a"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#.to_string(),
            format!(r#"{{"type":"user","sessionId":"s1","timestamp":"2026-10-01T10:00:01Z","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","content":"{big}"}}]}}}}"#),
            r#"{"type":"attachment","sessionId":"s1","timestamp":"2026-10-01T09:59:00Z","attachment":{"type":"skill_listing","content":"- a: b","names":["a"]}}"#.to_string(),
            r#"{"type":"file-history-snapshot","messageId":"m","snapshot":{}}"#.to_string(),
        ];
        fx.write("-w-proj/s1.jsonl", &(lines.join("\n") + "\n"));
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        let (file_id, _) = store.raw_files().unwrap()[0].clone();
        let raw = store.raw_records(file_id, &[]).unwrap();
        assert_eq!(
            raw.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(),
            vec!["assistant", "user", "attachment"]
        );
        assert!(raw[1].json.len() < 1000, "tool output must be shrunk");
        assert!(raw[1].json.contains(r#""$chars":5000"#));
    }

    #[test]
    fn a_subagent_tool_result_is_stored_as_a_sidechain() {
        let fx = Fixture::new();
        let result = |id: &str| {
            format!(
                r#"{{"type":"user","sessionId":"s1","timestamp":"2026-10-01T10:00:01Z","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"{id}","content":"ok"}}]}}}}"#
            )
        };
        fx.write("-w-proj/s1.jsonl", &format!("{}\n", result("main")));
        fx.write(
            "-w-proj/s1/subagents/agent-a1.jsonl",
            &format!("{}\n", result("agent")),
        );
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        let flags: Vec<String> = store
            .dump("tool_results")
            .into_iter()
            .map(|row| {
                let cells: Vec<&str> = row.split('|').collect();
                format!("{} {}", cells[0], cells[6])
            })
            .collect();
        assert_eq!(
            flags,
            vec![r#"Text("agent") Integer(1)"#, r#"Text("main") Integer(0)"#]
        );
    }

    const DERIVED: &[&str] = &[
        "requests",
        "tool_uses",
        "tool_results",
        "attachments",
        "compactions",
        "cost_states",
        "sessions",
        "adjustments",
        "prompts",
    ];

    #[test]
    fn main_thread_prompts_and_tool_targets_are_derived_once() {
        let fx = Fixture::new();
        let prompt = |uuid: &str, ts: &str, text: &str| {
            serde_json::json!({"type":"user","uuid":uuid,"sessionId":"s1","timestamp":ts,"message":{"role":"user","content":text}}).to_string()
        };
        let lines = [
            prompt("u1", "2026-10-01T10:00:00Z", "Fix the countdown"),
            serde_json::json!({"type":"user","uuid":"u2","isMeta":true,"sessionId":"s1","timestamp":"2026-10-01T10:00:01Z","message":{"role":"user","content":"meta"}}).to_string(),
            prompt("u3", "2026-10-01T10:00:02Z", "<command-name>/clear</command-name>"),
            serde_json::json!({"type":"assistant","sessionId":"s1","timestamp":"2026-10-01T10:00:03Z","requestId":"r1","message":{"model":"claude-opus-5-5","content":[
                {"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/w/a.rs","limit":20}},
                {"type":"tool_use","id":"t2","name":"Bash","input":{"command":"x".repeat(300),"description":"long"}},
                {"type":"tool_use","id":"t3","name":"Grep","input":{"pattern":"fn main","path":"/w"}}
            ],"usage":{"input_tokens":1,"output_tokens":5}}}).to_string(),
            prompt("u4", "2026-10-01T10:05:00Z", "Now the tests"),
        ];
        let path = fx.write("-w-proj/s1.jsonl", &(lines.join("\n") + "\n"));
        fx.write(
            "-w-proj/s1/subagents/agent-a1.jsonl",
            &format!(
                "{}\n",
                prompt("u5", "2026-10-01T10:00:04Z", "Explore the repo")
            ),
        );
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        let prompts = |store: &Store| -> Vec<i64> {
            store
                .prompts_between(0, i64::MAX)
                .unwrap()
                .iter()
                .map(|p| p.ts)
                .collect()
        };
        assert_eq!(prompts(&store), vec![1_790_848_800, 1_790_849_100]);
        assert_eq!(
            store.dump("tool_uses"),
            vec![
                r#"Text("t1")|Text("s1")|Integer(1790848803)|Text("Read")|Integer(7)|Text("/w/a.rs")"#,
                r#"Text("t2")|Text("s1")|Integer(1790848803)|Text("Bash")|Integer(304)|Null"#,
                r#"Text("t3")|Text("s1")|Integer(1790848803)|Text("Grep")|Integer(9)|Text("fn main")"#,
            ]
        );
        let shorter = &lines[..4];
        fx.write("-w-proj/s1.jsonl", &(shorter.join("\n") + "\n"));
        set_mtime_offset(&path, 10);
        fx.run(&mut store);
        fx.write("-w-proj/s1.jsonl", &(lines.join("\n") + "\n"));
        set_mtime_offset(&path, 20);
        fx.run(&mut store);
        assert_eq!(prompts(&store), vec![1_790_848_800, 1_790_849_100]);
        reindex(&mut store);
        assert_eq!(prompts(&store), vec![1_790_848_800, 1_790_849_100]);
    }

    #[test]
    fn a_request_counts_as_sent_when_the_entry_it_answers_was_written() {
        let fx = Fixture::new();
        let lines = [
            serde_json::json!({"type":"user","sessionId":"s1","uuid":"u1","timestamp":"2026-10-01T09:59:00Z","message":{"role":"user","content":"go"}}),
            serde_json::json!({"type":"attachment","sessionId":"s1","uuid":"u2","parentUuid":"u1","timestamp":"2026-10-01T09:59:01Z","attachment":{"type":"instructions","names":["a"]}}),
            serde_json::json!({"type":"assistant","sessionId":"s1","uuid":"u3","parentUuid":"u2","timestamp":"2026-10-01T10:00:00Z","requestId":"r1","message":{"model":"claude-opus-5-5","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":1,"output_tokens":5}}}),
            serde_json::json!({"type":"assistant","sessionId":"s1","uuid":"u4","timestamp":"2026-10-01T10:05:00Z","requestId":"r2","message":{"model":"claude-opus-5-5","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":1,"output_tokens":5}}}),
        ];
        let content: String = lines.iter().map(|line| format!("{line}\n")).collect();
        fx.write("-w-proj/s1.jsonl", &content);
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        let sent = |store: &Store| -> Vec<(i64, Option<i64>)> {
            store
                .session_requests("s1")
                .unwrap()
                .iter()
                .map(|r| (r.ts, r.sent))
                .collect()
        };
        let parent = 1_790_848_740;
        let expected = vec![(parent + 60, Some(parent + 1)), (parent + 360, None)];
        assert_eq!(sent(&store), expected);
        reindex(&mut store);
        assert_eq!(sent(&store), expected);
    }

    #[test]
    fn reindex_derives_the_same_rows_from_redacted_raw() {
        let fx = Fixture::new();
        let secret = "s3cr3t-".repeat(400);
        let short = "s3cr3t-short";
        let lines = [
            serde_json::json!({"type":"user","uuid":"u0","sessionId":"s1","timestamp":"2026-10-01T09:59:00Z","message":{"role":"user","content":format!("{short} {secret}")}}),
            serde_json::json!({"type":"attachment","sessionId":"s1","timestamp":"2026-10-01T09:59:01Z","attachment":{"type":"instructions","files":[{"path":"/h/CLAUDE.md","type":"User","content":short}],"names":["a"],"content":secret}}),
            serde_json::json!({"type":"assistant","sessionId":"s1","timestamp":"2026-10-01T10:00:00Z","requestId":"r1","message":{"model":"claude-opus-5-5","content":[
                {"type":"thinking","thinking":short,"signature":secret},
                {"type":"text","text":short},
                {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls -la","description":secret}},
                {"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"/w/a.rs"}}
            ],"usage":{"input_tokens":1,"output_tokens":5}}}),
            serde_json::json!({"type":"user","sessionId":"s1","timestamp":"2026-10-01T10:00:01Z","message":{"role":"user","content":[
                {"type":"tool_result","tool_use_id":"t1","content":short},
                {"type":"tool_result","tool_use_id":"t2","is_error":true,"content":[
                    {"type":"text","text":secret},
                    {"type":"image","source":{"type":"base64","media_type":"image/png","data":secret}},
                    {"type":"tool_reference","tool_name":"mcp__x__y"}
                ]}
            ]},"toolUseResult":{"stdout":short}}),
            serde_json::json!({"type":"system","subtype":"compact_boundary","sessionId":"s1","timestamp":"2026-10-01T10:00:02Z","content":short,"compactMetadata":{"trigger":"auto","preTokens":9,"postTokens":3}}),
            serde_json::json!({"type":"cost-state","sessionId":"s1","totalCostUSD":1.0,"modelUsage":{"claude-opus-5-5":{"inputTokens":1,"outputTokens":50}}}),
            serde_json::json!({"type":"last-prompt","sessionId":"s1","lastPrompt":short}),
        ];
        let content: String = lines.iter().map(|line| format!("{line}\n")).collect();
        fx.write("-w-proj/s1.jsonl", &content);
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        let (file_id, _) = store.raw_files().unwrap()[0].clone();
        for row in store.raw_records(file_id, &[]).unwrap() {
            assert!(!row.json.contains("s3cr3t"), "{}", row.json);
        }
        let imported: Vec<Vec<String>> = DERIVED.iter().map(|t| store.dump(t)).collect();
        assert_eq!(imported[1].len(), 2);
        assert_eq!(imported[2].len(), 2);
        assert_eq!(imported[3].len(), 1);
        assert_eq!(imported[8].len(), 1);
        assert!(!imported.concat().concat().contains("s3cr3t"));

        reindex(&mut store);
        let rebuilt: Vec<Vec<String>> = DERIVED.iter().map(|t| store.dump(t)).collect();
        assert_eq!(rebuilt, imported);
        let raw_after = store.raw_records(file_id, &[]).unwrap();
        assert_eq!(raw_after.len(), lines.len());
    }

    #[test]
    fn reindex_never_writes_raw_records() {
        let fx = Fixture::new();
        let line = assistant(
            "s1",
            "r1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        fx.write("-w-proj/s1.jsonl", &format!("{line}\n"));
        let database = fx._temp.path().join("data.db");
        let mut store = Store::open(&database).unwrap();
        fx.run(&mut store);
        rusqlite::Connection::open(&database)
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER raw_is_read_only BEFORE INSERT ON raw_records
                 BEGIN SELECT RAISE(ABORT, 'raw rewritten'); END;",
            )
            .unwrap();
        assert_eq!(reindex(&mut store).requests, 1);
    }

    #[test]
    fn a_busy_store_aborts_the_import() {
        let fx = Fixture::new();
        let line = assistant(
            "s1",
            "r1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        fx.write("-w-proj/s1.jsonl", &format!("{line}\n"));
        fx.write("-w-proj/s2.jsonl", &format!("{line}\n"));
        let database = fx._temp.path().join("data.db");
        let mut holder = Store::open(&database).unwrap();
        let mut store = Store::open(&database).unwrap();
        store.set_busy_timeout(std::time::Duration::from_millis(10));
        let _held = holder.writer().unwrap();
        let prices = PriceTable::bundled();
        let error = Ingestor::new(&mut store, &prices)
            .ingest_all(&fx.projects, &mut |_, _| {})
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "another claude-plus is writing, try again in a moment"
        );
    }

    fn set_mtime_offset(path: &std::path::Path, seconds: u64) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(seconds))
            .unwrap();
    }

    fn raw_json(store: &Store, path: &str) -> String {
        let (file_id, _) = store
            .raw_files()
            .unwrap()
            .into_iter()
            .find(|(_, stored)| stored == path)
            .unwrap();
        store
            .raw_records(file_id, &[])
            .unwrap()
            .iter()
            .map(|r| r.json.clone())
            .collect()
    }

    #[test]
    fn same_size_replacement_is_reread() {
        let fx = Fixture::new();
        let a = assistant(
            "s1",
            "r1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        let b = a.replace("r1", "r2");
        let path = fx.write("-w-proj/s1.jsonl", &format!("{a}\n"));
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        fx.write("-w-proj/s1.jsonl", &format!("{b}\n"));
        set_mtime_offset(&path, 10);
        let report = fx.run(&mut store);
        assert_eq!(report.requests_inserted, 1);
        let live = path.to_string_lossy().into_owned();
        assert!(!raw_json(&store, &live).contains("r1"));
        assert!(raw_json(&store, &live).contains("r2"));
        assert!(raw_json(&store, &format!("{live}#gen-1")).contains("r1"));
    }

    type Snapshot = (crate::store::Totals, Vec<crate::store::DailyCost>);

    fn snapshot(store: &Store) -> Snapshot {
        (
            store.totals().unwrap(),
            store.daily_costs(0, i64::MAX, &chrono::Utc, None).unwrap(),
        )
    }

    fn reindex(store: &mut Store) -> Rebuilt {
        let prices = PriceTable::bundled();
        Ingestor::new(store, &prices)
            .reindex(&mut |_, _| {})
            .unwrap()
    }

    #[test]
    fn reindex_matches_import_across_replacement_and_deletion() {
        let fx = Fixture::new();
        let r1 = assistant(
            "s1",
            "r1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        let r2 = r1.replace("r1", "r2");
        let r3 = assistant(
            "s2",
            "r3",
            "2026-10-02T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        let replaced = fx.write("-w-proj/s1.jsonl", &format!("{r1}\n"));
        let deleted = fx.write("-w-proj/s2.jsonl", &format!("{r3}\n"));
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        fx.write("-w-proj/s1.jsonl", &format!("{r2}\n"));
        set_mtime_offset(&replaced, 10);
        std::fs::remove_file(deleted).unwrap();
        fx.run(&mut store);
        let imported = snapshot(&store);
        assert_eq!(imported.0.requests, 3);

        let rebuilt = reindex(&mut store);
        assert_eq!((rebuilt.requests, rebuilt.records), (3, 3));
        assert_eq!(snapshot(&store), imported);
    }

    #[test]
    fn replaced_files_keep_one_attachment_and_one_compaction() {
        let fx = Fixture::new();
        let attachment = r#"{"type":"attachment","sessionId":"s1","timestamp":"2026-10-01T09:59:00Z","attachment":{"type":"skill_listing","content":"- a: b","names":["a"]}}"#;
        let compaction = r#"{"type":"system","subtype":"compact_boundary","sessionId":"s1","timestamp":"2026-10-01T10:00:02Z","compactMetadata":{"trigger":"auto","preTokens":9,"postTokens":3}}"#;
        let request = assistant(
            "s1",
            "r1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        let path = fx.write(
            "-w-proj/s1.jsonl",
            &format!("{attachment}\n{compaction}\n{request}\n"),
        );
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        fx.write(
            "-w-proj/s1.jsonl",
            &format!(
                "{attachment}\n{compaction}\n{}\n",
                request.replace("r1", "r2")
            ),
        );
        set_mtime_offset(&path, 10);
        fx.run(&mut store);
        let counts = |store: &Store| {
            (
                store.dump("attachments").len(),
                store.dump("compactions").len(),
                store.totals().unwrap().requests,
            )
        };
        assert_eq!(counts(&store), (1, 1, 2));
        reindex(&mut store);
        assert_eq!(counts(&store), (1, 1, 2));
    }

    #[test]
    fn replaced_generations_are_never_reingested() {
        let fx = Fixture::new();
        let a = assistant(
            "s1",
            "r1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        let path = fx.write("-w-proj/s1.jsonl", &format!("{a}\n"));
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        for (generation, request) in ["r2", "r3"].iter().enumerate() {
            fx.write(
                "-w-proj/s1.jsonl",
                &format!("{}\n", a.replace("r1", request)),
            );
            set_mtime_offset(&path, 10 * (generation as u64 + 1));
            assert_eq!(fx.run(&mut store).requests_inserted, 1);
        }
        let again = fx.run(&mut store);
        assert_eq!((again.files_seen, again.files_changed), (1, 0));
        let live = path.to_string_lossy().into_owned();
        assert_eq!(
            store
                .raw_files()
                .unwrap()
                .into_iter()
                .map(|(_, path)| path)
                .collect::<Vec<_>>(),
            vec![format!("{live}#gen-1"), format!("{live}#gen-2"), live]
        );
        assert_eq!(store.totals().unwrap().requests, 3);
    }

    #[test]
    fn larger_replacement_is_reread() {
        let fx = Fixture::new();
        let a = assistant(
            "s1",
            "r1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        let b = assistant(
            "s1",
            "r2",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        let c = assistant(
            "s1",
            "r3",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        let path = fx.write("-w-proj/s1.jsonl", &format!("{a}\n"));
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        fx.write("-w-proj/s1.jsonl", &format!("{b}\n{c}\n"));
        let report = fx.run(&mut store);
        assert_eq!((report.requests_inserted, report.parse_errors), (2, 0));
        let live = path.to_string_lossy().into_owned();
        assert!(!raw_json(&store, &live).contains("r1"));
        assert!(raw_json(&store, &format!("{live}#gen-1")).contains("r1"));
    }

    #[test]
    fn small_file_growth_is_not_a_replacement() {
        let fx = Fixture::new();
        let line = r#"{"type":"queue-operation"}"#;
        let path = fx.write("-w-proj/s1.jsonl", &format!("{line}\n"));
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        for expected_lines in [2, 3] {
            fx.append("-w-proj/s1.jsonl", &format!("{line}\n"));
            set_mtime_offset(&path, expected_lines * 10);
            let report = fx.run(&mut store);
            assert_eq!((report.lines, report.parse_errors), (1, 0));
            let checkpoint = store.checkpoint(&path.to_string_lossy()).unwrap().unwrap();
            assert_eq!(checkpoint.lines, expected_lines);
        }
    }

    #[test]
    fn reindex_commits_in_batches_and_resumes_after_an_interruption() {
        let fx = Fixture::new();
        let line = |session: &str, id: &str| {
            assistant(
                session,
                id,
                "2026-10-01T10:00:00Z",
                "claude-opus-5-5",
                OPUS_USAGE,
            )
        };
        fx.write("-w-proj/s1.jsonl", &format!("{}\n", line("s1", "r1")));
        fx.write("-w-proj/s2.jsonl", &format!("{}\n", line("s2", "boom")));
        let database = fx._temp.path().join("data.db");
        let mut store = Store::open(&database).unwrap();
        fx.run(&mut store);
        let imported = snapshot(&store);
        let conn = rusqlite::Connection::open(&database).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_on_boom BEFORE INSERT ON requests WHEN NEW.key = 'boom'
             BEGIN SELECT RAISE(ABORT, 'boom'); END;",
        )
        .unwrap();
        let prices = PriceTable::bundled();
        let mut ingestor = Ingestor::new(&mut store, &prices);
        ingestor.batch_lines = 1;
        assert!(ingestor.reindex(&mut |_, _| {}).is_err());
        assert_eq!(store.totals().unwrap().requests, 1);
        assert!(store.reindex_pending().unwrap());

        conn.execute_batch("DROP TRIGGER fail_on_boom").unwrap();
        let rebuilt = Ingestor::new(&mut store, &prices)
            .finish_interrupted_reindex(&mut |_, _| {})
            .unwrap();
        assert_eq!(
            rebuilt,
            Some(Rebuilt {
                requests: 2,
                records: 2
            })
        );
        assert!(!store.reindex_pending().unwrap());
        assert_eq!(snapshot(&store), imported);
        let again = Ingestor::new(&mut store, &prices)
            .finish_interrupted_reindex(&mut |_, _| {})
            .unwrap();
        assert_eq!(again, None);
    }

    #[test]
    fn a_cancelled_reindex_stays_pending() {
        let fx = Fixture::new();
        for session in ["s1", "s2"] {
            fx.write(
                &format!("-w-proj/{session}.jsonl"),
                &format!(
                    "{}\n",
                    assistant(
                        session,
                        session,
                        "2026-10-01T10:00:00Z",
                        "claude-opus-5-5",
                        OPUS_USAGE
                    )
                ),
            );
        }
        let mut store = Store::open_in_memory().unwrap();
        fx.run(&mut store);
        let prices = PriceTable::bundled();
        let cancel = AtomicBool::new(true);
        let mut ingestor = Ingestor::new(&mut store, &prices).cancelled_by(&cancel);
        ingestor.batch_lines = 1;
        let error = ingestor.reindex(&mut |_, _| {}).unwrap_err();
        assert!(error.is::<Cancelled>());
        assert!(store.reindex_pending().unwrap());
        assert_eq!(store.totals().unwrap().requests, 1);
    }

    #[test]
    fn a_cancelled_import_stops_at_a_batch_boundary_and_resumes() {
        let fx = Fixture::new();
        let lines = |session: &str| -> String {
            (0..3)
                .map(|i| {
                    assistant(
                        session,
                        &format!("{session}-r{i}"),
                        "2026-10-01T10:00:00Z",
                        "claude-opus-5-5",
                        OPUS_USAGE,
                    ) + "\n"
                })
                .collect()
        };
        let path = fx.write("-w-proj/s1.jsonl", &lines("s1"));
        fx.write("-w-proj/s2.jsonl", &lines("s2"));
        let mut store = Store::open_in_memory().unwrap();
        let prices = PriceTable::bundled();
        let cancel = AtomicBool::new(true);
        let files = discover(&fx.projects).unwrap();
        let mut ingestor = Ingestor::new(&mut store, &prices).cancelled_by(&cancel);
        ingestor.batch_lines = 2;
        let skipped = ingestor.ingest_listed(&files, &mut |_, _| {}).unwrap();
        assert_eq!(skipped.files_seen, 0);
        let partial = ingestor.ingest_file(&files[0]).unwrap();
        assert_eq!((partial.lines, partial.files_changed), (2, 1));
        let checkpoint = store.checkpoint(&path.to_string_lossy()).unwrap().unwrap();
        assert_eq!(checkpoint.lines, 2);
        let rest = fx.run(&mut store);
        assert_eq!(rest.requests_inserted, 4);
        assert_eq!(store.totals().unwrap().requests, 6);
    }

    #[test]
    fn the_pause_hook_runs_between_batches_and_files_outside_a_transaction() {
        let fx = Fixture::new();
        let lines = |session: &str| -> String {
            (0..3)
                .map(|i| {
                    assistant(
                        session,
                        &format!("{session}-r{i}"),
                        "2026-10-01T10:00:00Z",
                        "claude-opus-5-5",
                        OPUS_USAGE,
                    ) + "\n"
                })
                .collect()
        };
        fx.write("-w-proj/s1.jsonl", &lines("s1"));
        fx.write("-w-proj/s2.jsonl", &lines("s2"));
        let mut store = Store::open_in_memory().unwrap();
        let prices = PriceTable::bundled();
        let files = discover(&fx.projects).unwrap();
        let mut pauses = 0;
        let mut hook = |store: &mut Store| {
            pauses += 1;
            let mut writer = store.writer().unwrap();
            writer
                .event(&crate::store::EventRow {
                    client_id: None,
                    session_id: "s1".into(),
                    kind: "session.measure".into(),
                    ts_ms: pauses,
                    received_ms: pauses,
                    json: "{}".into(),
                })
                .unwrap();
            writer.commit().unwrap();
        };
        let mut ingestor = Ingestor::new(&mut store, &prices).pausing_with(&mut hook);
        ingestor.batch_lines = 2;
        let report = ingestor.ingest_listed(&files, &mut |_, _| {}).unwrap();
        assert_eq!(report.requests_inserted, 6);
        assert_eq!(pauses, 4);
        let latest = store.latest_event("s1", "session.measure").unwrap();
        assert_eq!(latest.unwrap().ts_ms, 4);
    }

    #[test]
    fn vanished_file_is_skipped() {
        let fx = Fixture::new();
        let missing = TranscriptFile::from_path(&fx.projects.join("-w-proj/gone.jsonl"));
        let mut store = Store::open_in_memory().unwrap();
        let prices = PriceTable::bundled();
        let report = Ingestor::new(&mut store, &prices)
            .ingest_listed(&[missing], &mut |_, _| {})
            .unwrap();
        assert_eq!((report.files_vanished, report.failures.len()), (1, 0));
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_file_is_reported_and_others_continue() {
        use std::os::unix::fs::PermissionsExt;
        let fx = Fixture::new();
        let line = assistant(
            "s1",
            "r1",
            "2026-10-01T10:00:00Z",
            "claude-opus-5-5",
            OPUS_USAGE,
        );
        let locked = fx.write("-w-proj/a.jsonl", &format!("{line}\n"));
        fx.write(
            "-w-proj/b.jsonl",
            &format!("{}\n", line.replace("r1", "r2")),
        );
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::File::open(&locked).is_ok() {
            return;
        }
        let mut store = Store::open_in_memory().unwrap();
        let report = fx.run(&mut store);
        assert_eq!(report.requests_inserted, 1);
        assert_eq!(report.failures.len(), 1);
        assert!(report.failures[0].0.ends_with("a.jsonl"));
    }
}
