mod discover;
mod lines;

use std::path::Path;

use anyhow::Result;

use crate::pricing::PriceTable;
use crate::store::{Store, Writer};
use crate::transcript::Parsed;

pub use discover::{TranscriptFile, discover};

pub const BATCH_LINES: u64 = 5000;
pub const RAW_STRING_LIMIT: usize = 2048;
pub const SKIPPED_RAW_KINDS: &[&str] = &[
    "file-history-snapshot",
    "file-history-delta",
    "queue-operation",
    "atis-latch",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IngestReport {
    pub files_seen: u64,
    pub files_changed: u64,
    pub lines: u64,
    pub requests_inserted: u64,
    pub duplicate_requests: u64,
    pub unpriced_requests: u64,
    pub parse_errors: u64,
}

use crate::store::{
    AttachmentRow, CompactionRow, CostStateRow, RequestRow, RequestWrite, SessionRow,
    ToolResultRow, ToolUseRow,
};
use crate::transcript::{Record, parse_line, text::shrink};

impl IngestReport {
    pub fn add(&mut self, other: &IngestReport) {
        self.files_seen += other.files_seen;
        self.files_changed += other.files_changed;
        self.lines += other.lines;
        self.requests_inserted += other.requests_inserted;
        self.duplicate_requests += other.duplicate_requests;
        self.unpriced_requests += other.unpriced_requests;
        self.parse_errors += other.parse_errors;
    }
}

pub struct Ingestor<'a> {
    store: &'a mut Store,
    prices: &'a PriceTable,
}

impl<'a> Ingestor<'a> {
    pub fn new(store: &'a mut Store, prices: &'a PriceTable) -> Self {
        Self { store, prices }
    }

    pub fn ingest_all(
        &mut self,
        projects_dir: &Path,
        on_file: &mut dyn FnMut(usize, usize),
    ) -> Result<IngestReport> {
        let files = discover(projects_dir)?;
        let mut report = IngestReport::default();
        for (index, file) in files.iter().enumerate() {
            report.add(&self.ingest_file(file)?);
            on_file(index + 1, files.len());
        }
        Ok(report)
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
        let metadata = std::fs::metadata(&file.path)?;
        let size = metadata.len();
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let path = file.path.to_string_lossy().into_owned();
        let checkpoint = self.store.checkpoint(&path)?;
        if let Some(cp) = &checkpoint
            && cp.size == size
            && cp.mtime == mtime
            && cp.offset == size
        {
            return Ok(report);
        }

        let mut writer = self.store.writer()?;
        let file_id = writer.file(&path, size, mtime)?;
        let (mut offset, mut line_no) = match &checkpoint {
            Some(cp) if size >= cp.offset => (cp.offset, cp.lines),
            Some(_) => {
                writer.reset_file(file_id)?;
                (0, 0)
            }
            None => (0, 0),
        };
        report.files_changed = 1;

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
            if line.text.trim().is_empty() {
                continue;
            }
            match parse_line(&line.text) {
                Ok(parsed) => apply(
                    &mut writer,
                    self.prices,
                    file,
                    file_id,
                    line_no,
                    &parsed,
                    &mut report,
                )?,
                Err(_) => {
                    report.parse_errors += 1;
                    batch_errors += 1;
                }
            }
            in_batch += 1;
            if in_batch == BATCH_LINES {
                writer.advance_file(file_id, offset, line_no, size, mtime, batch_errors)?;
                writer.commit()?;
                writer = self.store.writer()?;
                batch_errors = 0;
                in_batch = 0;
            }
        }
        writer.advance_file(file_id, offset, line_no, size, mtime, batch_errors)?;
        writer.commit()?;
        Ok(report)
    }
}

pub fn apply(
    writer: &mut Writer,
    prices: &PriceTable,
    file: &TranscriptFile,
    file_id: i64,
    line_no: u64,
    parsed: &Parsed,
    report: &mut IngestReport,
) -> Result<()> {
    let kind = parsed.record.kind();
    if !SKIPPED_RAW_KINDS.contains(&kind) {
        writer.raw(
            file_id,
            line_no,
            kind,
            &shrink(&parsed.raw, RAW_STRING_LIMIT).to_string(),
        )?;
    }
    let common = parsed.record.common();
    let session_id = common
        .and_then(|c| c.session_id.clone())
        .unwrap_or_else(|| file.session_hint.clone());
    let ts = common.and_then(|c| c.timestamp).map(|t| t.timestamp());
    if common.is_some() {
        writer.session(&SessionRow {
            id: session_id.clone(),
            project: file.project.clone(),
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
            let write = writer.request(&RequestRow {
                key,
                session_id: session_id.clone(),
                ts: ts.unwrap_or(0),
                model: a.model.clone(),
                tokens: a.usage,
                cost_usd,
                is_sidechain: a.common.is_sidechain || file.agent_id.is_some(),
                agent_id: a.common.agent_id.clone().or_else(|| file.agent_id.clone()),
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
            for tool in &a.tool_uses {
                writer.tool_use(&ToolUseRow {
                    id: tool.id.clone(),
                    session_id: session_id.clone(),
                    ts: ts.unwrap_or(0),
                    name: tool.name.clone(),
                    input_chars: tool.input_chars,
                })?;
            }
        }
        Record::User(u) => {
            for result in &u.tool_results {
                writer.tool_result(&ToolResultRow {
                    tool_use_id: result.tool_use_id.clone(),
                    session_id: session_id.clone(),
                    ts: ts.unwrap_or(0),
                    chars: result.chars,
                    images: result.images,
                    is_error: result.is_error,
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
                .unwrap_or_else(|| file.session_hint.clone());
            writer.cost_state(&CostStateRow {
                file_id,
                line_no,
                session_id,
                total_cost_usd: c.total_cost_usd,
                json: parsed.raw.to_string(),
            })?;
        }
        Record::Title(t) => {
            let session_id = t
                .session_id
                .clone()
                .unwrap_or_else(|| file.session_hint.clone());
            writer.session(&SessionRow {
                id: session_id.clone(),
                project: file.project.clone(),
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
            let prices = PriceTable::bundled();
            Ingestor::new(store, &prices)
                .ingest_all(&self.projects, &mut |_, _| {})
                .unwrap()
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
                files[0].project.as_str(),
                files[0].session_hint.as_str(),
                files[0].agent_id.as_deref()
            ),
            ("-w-proj", "s1", None)
        );
        assert_eq!(
            (files[1].session_hint.as_str(), files[1].agent_id.as_deref()),
            ("s1", Some("a1"))
        );
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
        assert_eq!((first.parse_errors, first.requests_inserted), (0, 0));
        fx.append("-w-proj/s1.jsonl", &format!("{tail}\n"));
        let second = fx.run(&mut store);
        assert_eq!((second.parse_errors, second.requests_inserted), (0, 1));
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
}
