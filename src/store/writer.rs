use anyhow::Result;
use rusqlite::{OptionalExtension, Transaction, params};

use crate::transcript::TokenCounts;

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
}

#[derive(Debug, Clone)]
pub struct ToolResultRow {
    pub tool_use_id: String,
    pub session_id: String,
    pub ts: i64,
    pub chars: u64,
    pub images: u32,
    pub is_error: bool,
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
    pub total_cost_usd: f64,
    pub json: String,
}

impl Writer<'_> {
    pub fn file(&mut self, path: &str, size: u64, mtime: i64) -> Result<i64> {
        let existing: Option<i64> = self
            .tx
            .query_row("SELECT id FROM files WHERE path = ?1", [path], |r| r.get(0))
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        self.tx.execute(
            "INSERT INTO files (path, size, mtime) VALUES (?1, ?2, ?3)",
            params![path, size as i64, mtime],
        )?;
        Ok(self.tx.last_insert_rowid())
    }

    pub fn advance_file(
        &mut self,
        file_id: i64,
        offset: u64,
        lines: u64,
        size: u64,
        mtime: i64,
        new_parse_errors: u64,
    ) -> Result<()> {
        self.tx.execute(
            "UPDATE files SET offset = ?2, lines = ?3, size = ?4, mtime = ?5, parse_errors = parse_errors + ?6 WHERE id = ?1",
            params![file_id, offset as i64, lines as i64, size as i64, mtime, new_parse_errors as i64],
        )?;
        Ok(())
    }

    pub fn reset_file(&mut self, file_id: i64) -> Result<()> {
        self.tx.execute(
            "UPDATE files SET offset = 0, lines = 0, parse_errors = 0 WHERE id = ?1",
            [file_id],
        )?;
        Ok(())
    }

    pub fn raw(&mut self, file_id: i64, line_no: u64, kind: &str, json: &str) -> Result<()> {
        self.tx.execute(
            "INSERT OR REPLACE INTO raw_records (file_id, line_no, kind, json) VALUES (?1, ?2, ?3, ?4)",
            params![file_id, line_no as i64, kind, json],
        )?;
        Ok(())
    }

    pub fn session(&mut self, row: &SessionRow) -> Result<()> {
        self.tx.execute(
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
        self.tx.execute(
            "UPDATE sessions SET title = ?2, title_is_custom = ?3 WHERE id = ?1 AND (?3 = 1 OR title_is_custom = 0)",
            params![session_id, title, is_custom],
        )?;
        Ok(())
    }

    pub fn request(&mut self, row: &RequestRow) -> Result<RequestWrite> {
        let t = &row.tokens;
        let stored_output: Option<i64> = self
            .tx
            .query_row(
                "SELECT output FROM requests WHERE key = ?1",
                [&row.key],
                |r| r.get(0),
            )
            .optional()?;
        if stored_output.is_some_and(|output| output >= t.output as i64) {
            return Ok(RequestWrite::Unchanged);
        }
        self.tx.execute(
            "INSERT INTO requests (key, session_id, ts, model, input, output, thinking, cache_read,
               cache_write_5m, cache_write_1h, cost_usd, is_sidechain, agent_id, skill, mcp_server, effort)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ON CONFLICT(key) DO UPDATE SET
               input = excluded.input, output = excluded.output, thinking = excluded.thinking,
               cache_read = excluded.cache_read, cache_write_5m = excluded.cache_write_5m,
               cache_write_1h = excluded.cache_write_1h, cost_usd = excluded.cost_usd",
            params![
                row.key, row.session_id, row.ts, row.model, t.input as i64, t.output as i64, t.thinking as i64,
                t.cache_read as i64, t.cache_write_5m as i64, t.cache_write_1h as i64, row.cost_usd,
                row.is_sidechain, row.agent_id, row.skill, row.mcp_server, row.effort
            ],
        )?;
        Ok(if stored_output.is_some() {
            RequestWrite::Updated
        } else {
            RequestWrite::Inserted
        })
    }

    pub fn tool_use(&mut self, row: &ToolUseRow) -> Result<()> {
        self.tx.execute(
            "INSERT OR IGNORE INTO tool_uses (id, session_id, ts, name, input_chars) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![row.id, row.session_id, row.ts, row.name, row.input_chars as i64],
        )?;
        Ok(())
    }

    pub fn tool_result(&mut self, row: &ToolResultRow) -> Result<()> {
        self.tx.execute(
            "INSERT OR IGNORE INTO tool_results (tool_use_id, session_id, ts, chars, images, is_error) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![row.tool_use_id, row.session_id, row.ts, row.chars as i64, row.images, row.is_error],
        )?;
        Ok(())
    }

    pub fn attachment(&mut self, row: &AttachmentRow) -> Result<()> {
        self.tx.execute(
            "INSERT OR REPLACE INTO attachments (file_id, line_no, session_id, ts, kind, chars, names)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![row.file_id, row.line_no as i64, row.session_id, row.ts, row.kind, row.chars as i64, serde_json::to_string(&row.names)?],
        )?;
        Ok(())
    }

    pub fn compaction(&mut self, row: &CompactionRow) -> Result<()> {
        self.tx.execute(
            "INSERT OR REPLACE INTO compactions (file_id, line_no, session_id, ts, trigger, pre_tokens, post_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![row.file_id, row.line_no as i64, row.session_id, row.ts, row.trigger, row.pre_tokens.map(|v| v as i64), row.post_tokens.map(|v| v as i64)],
        )?;
        Ok(())
    }

    pub fn cost_state(&mut self, row: &CostStateRow) -> Result<()> {
        self.tx.execute(
            "INSERT OR REPLACE INTO cost_states (file_id, line_no, session_id, total_cost_usd, json) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![row.file_id, row.line_no as i64, row.session_id, row.total_cost_usd, row.json],
        )?;
        Ok(())
    }

    pub fn commit(self) -> Result<()> {
        self.tx.commit()?;
        Ok(())
    }
}
