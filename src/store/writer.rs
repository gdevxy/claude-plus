use anyhow::Result;
use rusqlite::{OptionalExtension, Transaction, params};

use super::{GENERATION_MARK, REINDEX_PENDING};
use crate::transcript::TokenCounts;

const PARENT_LOOKBACK: i64 = 256;

#[derive(Debug, Clone, Copy)]
pub struct FileProgress {
    pub offset: u64,
    pub lines: u64,
    pub size: u64,
    pub mtime: i64,
    pub head_len: u64,
    pub head_hash: u64,
}

pub struct Writer<'a> {
    pub(super) tx: Transaction<'a>,
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: String,
    pub project: String,
    pub cwd: Option<String>,
    pub ts: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct RequestRow {
    pub key: String,
    pub session_id: String,
    pub ts: i64,
    pub sent_ts: Option<i64>,
    pub model: String,
    pub tokens: TokenCounts,
    pub cost_usd: Option<f64>,
    pub is_sidechain: bool,
    pub agent_id: Option<String>,
    pub skill: Option<String>,
    pub mcp_server: Option<String>,
    pub effort: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ToolUseRow {
    pub id: String,
    pub session_id: String,
    pub ts: i64,
    pub name: String,
    pub input_chars: u64,
    pub target: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PromptRow {
    pub uuid: String,
    pub session_id: String,
    pub ts: i64,
}

#[derive(Debug, Clone)]
pub struct ToolResultRow {
    pub tool_use_id: String,
    pub session_id: String,
    pub ts: i64,
    pub chars: u64,
    pub images: u32,
    pub is_error: bool,
    pub is_sidechain: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestWrite {
    Inserted,
    Updated,
    Unchanged,
}

#[derive(Debug, Clone)]
pub struct AttachmentRow {
    pub file_id: i64,
    pub line_no: u64,
    pub session_id: String,
    pub ts: Option<i64>,
    pub kind: String,
    pub chars: u64,
    pub names: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CompactionRow {
    pub file_id: i64,
    pub line_no: u64,
    pub session_id: String,
    pub ts: Option<i64>,
    pub trigger: Option<String>,
    pub pre_tokens: Option<u64>,
    pub post_tokens: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct CostStateRow {
    pub file_id: i64,
    pub line_no: u64,
    pub session_id: String,
    pub total_cost_usd: Option<f64>,
    pub model_count: u64,
    pub json: String,
}

impl Writer<'_> {
    fn exec(&self, sql: &str, params: impl rusqlite::Params) -> Result<usize> {
        Ok(self.tx.prepare_cached(sql)?.execute(params)?)
    }

    pub fn file(&mut self, path: &str, size: u64, mtime: i64) -> Result<i64> {
        let existing: Option<i64> = self
            .tx
            .prepare_cached("SELECT id FROM files WHERE path = ?1")?
            .query_row([path], |r| r.get(0))
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        self.exec(
            "INSERT INTO files (path, size, mtime) VALUES (?1, ?2, ?3)",
            params![path, size as i64, mtime],
        )?;
        Ok(self.tx.last_insert_rowid())
    }

    pub fn advance_file(
        &mut self,
        file_id: i64,
        progress: &FileProgress,
        new_parse_errors: u64,
    ) -> Result<()> {
        self.exec(
            "UPDATE files SET offset = ?2, lines = ?3, size = ?4, mtime = ?5, head_len = ?6, head_hash = ?7, parse_errors = parse_errors + ?8 WHERE id = ?1",
            params![
                file_id,
                progress.offset as i64,
                progress.lines as i64,
                progress.size as i64,
                progress.mtime,
                progress.head_len as i64,
                progress.head_hash as i64,
                new_parse_errors as i64
            ],
        )?;
        Ok(())
    }

    pub fn retire_file(&mut self, file_id: i64) -> Result<String> {
        let path: String = self
            .tx
            .prepare_cached("SELECT path FROM files WHERE id = ?1")?
            .query_row([file_id], |r| r.get(0))?;
        let prefix = format!("{path}{GENERATION_MARK}");
        let mut taken = self
            .tx
            .prepare_cached("SELECT substr(path, ?2) FROM files WHERE substr(path, 1, ?3) = ?1")?;
        let highest = taken
            .query_map(
                params![prefix, prefix.len() as i64 + 1, prefix.len() as i64],
                |r| r.get::<_, String>(0),
            )?
            .filter_map(|suffix| suffix.ok()?.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        let retired = format!("{prefix}{}", highest + 1);
        self.exec(
            "UPDATE files SET path = ?2 WHERE id = ?1",
            params![file_id, retired],
        )?;
        self.exec("DELETE FROM attachments WHERE file_id = ?1", [file_id])?;
        self.exec("DELETE FROM compactions WHERE file_id = ?1", [file_id])?;
        Ok(retired)
    }

    pub fn raw(&mut self, file_id: i64, line_no: u64, kind: &str, json: &str) -> Result<()> {
        self.exec(
            "INSERT OR REPLACE INTO raw_records (file_id, line_no, kind, json) VALUES (?1, ?2, ?3, ?4)",
            params![file_id, line_no as i64, kind, json],
        )?;
        Ok(())
    }

    pub fn written_before(
        &mut self,
        file_id: i64,
        line_no: u64,
        uuid: &str,
    ) -> Result<Option<i64>> {
        let mut statement = self.tx.prepare_cached(
            "SELECT json FROM raw_records WHERE file_id = ?1 AND line_no < ?2 ORDER BY line_no DESC LIMIT ?3",
        )?;
        let rows = statement.query_map(params![file_id, line_no as i64, PARENT_LOOKBACK], |r| {
            r.get::<_, String>(0)
        })?;
        for json in rows {
            let record: serde_json::Value = serde_json::from_str(&json?)?;
            if record.get("uuid").and_then(serde_json::Value::as_str) == Some(uuid) {
                return Ok(record
                    .get("timestamp")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
                    .map(|ts| ts.timestamp()));
            }
        }
        Ok(None)
    }

    pub fn session(&mut self, row: &SessionRow) -> Result<()> {
        self.exec(
            "INSERT INTO sessions (id, project, cwd, started_at, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(id) DO UPDATE SET
               cwd = COALESCE(sessions.cwd, excluded.cwd),
               started_at = CASE WHEN excluded.started_at IS NULL THEN sessions.started_at
                                 WHEN sessions.started_at IS NULL THEN excluded.started_at
                                 ELSE MIN(sessions.started_at, excluded.started_at) END,
               last_seen_at = CASE WHEN excluded.last_seen_at IS NULL THEN sessions.last_seen_at
                                   WHEN sessions.last_seen_at IS NULL THEN excluded.last_seen_at
                                   ELSE MAX(sessions.last_seen_at, excluded.last_seen_at) END",
            params![row.id, row.project, row.cwd, row.ts],
        )?;
        Ok(())
    }

    pub fn title(&mut self, session_id: &str, title: &str, is_custom: bool) -> Result<()> {
        self.exec(
            "UPDATE sessions SET title = ?2, title_is_custom = ?3 WHERE id = ?1 AND (?3 = 1 OR title_is_custom = 0)",
            params![session_id, title, is_custom],
        )?;
        Ok(())
    }

    pub fn request(&mut self, row: &RequestRow) -> Result<RequestWrite> {
        let t = &row.tokens;
        let stored_output: Option<i64> = self
            .tx
            .prepare_cached("SELECT output FROM requests WHERE key = ?1")?
            .query_row([&row.key], |r| r.get(0))
            .optional()?;
        if stored_output.is_some_and(|output| output >= t.output as i64) {
            return Ok(RequestWrite::Unchanged);
        }
        self.exec(
            "INSERT INTO requests (key, session_id, ts, model, input, output, thinking, cache_read,
               cache_write_5m, cache_write_1h, cost_usd, is_sidechain, agent_id, skill, mcp_server, effort, sent_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
             ON CONFLICT(key) DO UPDATE SET
               input = excluded.input, output = excluded.output, thinking = excluded.thinking,
               cache_read = excluded.cache_read, cache_write_5m = excluded.cache_write_5m,
               cache_write_1h = excluded.cache_write_1h, cost_usd = excluded.cost_usd",
            params![
                row.key, row.session_id, row.ts, row.model, t.input as i64, t.output as i64, t.thinking as i64,
                t.cache_read as i64, t.cache_write_5m as i64, t.cache_write_1h as i64, row.cost_usd,
                row.is_sidechain, row.agent_id, row.skill, row.mcp_server, row.effort, row.sent_ts
            ],
        )?;
        Ok(if stored_output.is_some() {
            RequestWrite::Updated
        } else {
            RequestWrite::Inserted
        })
    }

    pub fn tool_use(&mut self, row: &ToolUseRow) -> Result<()> {
        self.exec(
            "INSERT OR IGNORE INTO tool_uses (id, session_id, ts, name, input_chars, target) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![row.id, row.session_id, row.ts, row.name, row.input_chars as i64, row.target],
        )?;
        Ok(())
    }

    pub fn prompt(&mut self, row: &PromptRow) -> Result<()> {
        self.exec(
            "INSERT OR IGNORE INTO prompts (uuid, session_id, ts) VALUES (?1, ?2, ?3)",
            params![row.uuid, row.session_id, row.ts],
        )?;
        Ok(())
    }

    pub fn tool_result(&mut self, row: &ToolResultRow) -> Result<()> {
        self.exec(
            "INSERT OR IGNORE INTO tool_results (tool_use_id, session_id, ts, chars, images, is_error, is_sidechain)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![row.tool_use_id, row.session_id, row.ts, row.chars as i64, row.images, row.is_error, row.is_sidechain],
        )?;
        Ok(())
    }

    pub fn attachment(&mut self, row: &AttachmentRow) -> Result<()> {
        self.exec(
            "INSERT OR REPLACE INTO attachments (file_id, line_no, session_id, ts, kind, chars, names, removed_names)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![row.file_id, row.line_no as i64, row.session_id, row.ts, row.kind, row.chars as i64, serde_json::to_string(&row.names)?, serde_json::to_string(&row.removed)?],
        )?;
        Ok(())
    }

    pub fn compaction(&mut self, row: &CompactionRow) -> Result<()> {
        self.exec(
            "INSERT OR REPLACE INTO compactions (file_id, line_no, session_id, ts, trigger, pre_tokens, post_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![row.file_id, row.line_no as i64, row.session_id, row.ts, row.trigger, row.pre_tokens.map(|v| v as i64), row.post_tokens.map(|v| v as i64)],
        )?;
        Ok(())
    }

    pub fn cost_state(&mut self, row: &CostStateRow) -> Result<()> {
        self.exec(
            "INSERT OR REPLACE INTO cost_states (file_id, line_no, session_id, total_cost_usd, model_count, json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![row.file_id, row.line_no as i64, row.session_id, row.total_cost_usd, row.model_count as i64, row.json],
        )?;
        Ok(())
    }

    pub fn clear_derived(&mut self) -> Result<()> {
        for table in super::schema::DERIVED_TABLES {
            self.tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        Ok(())
    }

    pub fn mark_reindex_pending(&mut self, pending: bool) -> Result<()> {
        if pending {
            self.exec(
                "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, '1')",
                [REINDEX_PENDING],
            )?;
        } else {
            self.exec("DELETE FROM meta WHERE key = ?1", [REINDEX_PENDING])?;
        }
        Ok(())
    }

    pub fn checkpoint(&self, path: &str) -> Result<Option<super::FileCheckpoint>> {
        super::checkpoint(&self.tx, path)
    }

    pub fn raw_files(&self) -> Result<Vec<(i64, String)>> {
        super::raw_files(&self.tx)
    }

    pub fn raw_records(&self, file_id: i64, kinds: &[&str]) -> Result<Vec<super::RawRow>> {
        super::raw_records(&self.tx, file_id, kinds)
    }

    pub fn commit(self) -> Result<()> {
        self.tx.commit()?;
        Ok(())
    }
}
