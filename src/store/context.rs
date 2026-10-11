use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use super::{GENERATION_MARK, Store};
use crate::transcript::{Record, parse_line};

pub const PROMPT_SEARCH_LIMIT: i64 = 500;
const MAIN_THREAD: &str = "f.path NOT LIKE '%/subagents/%' AND instr(f.path, '#gen-') = 0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentPoint {
    pub ts: i64,
    pub kind: String,
    pub chars: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResultPoint {
    pub tool: Option<String>,
    pub ts: i64,
    pub chars: u64,
    pub images: u32,
}

impl Store {
    pub fn main_attachments(&self, session_id: &str) -> Result<Vec<AttachmentPoint>> {
        let mut statement = self.conn.prepare_cached(&main_attachments_sql())?;
        let rows = statement.query_map([session_id], |r| {
            Ok(AttachmentPoint {
                ts: r.get(0)?,
                kind: r.get(1)?,
                chars: r.get::<_, i64>(2)? as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn main_compactions(&self, session_id: &str) -> Result<Vec<i64>> {
        let mut statement = self.conn.prepare_cached(&main_compactions_sql())?;
        let rows = statement.query_map([session_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn first_prompt_chars(&self, session_id: &str) -> Result<Option<u64>> {
        let suffix = format!("/{session_id}.jsonl");
        let mut files = self
            .conn
            .prepare_cached("SELECT id, path FROM files WHERE path LIKE '%.jsonl' ORDER BY id")?;
        let ids: Vec<i64> = files
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .filter_map(rusqlite::Result::ok)
            .filter(|(_, path)| path.ends_with(&suffix) && !path.contains(GENERATION_MARK))
            .map(|(id, _)| id)
            .collect();
        let mut users = self.conn.prepare_cached(
            "SELECT json FROM raw_records WHERE file_id = ?1 AND kind = 'user' ORDER BY line_no LIMIT ?2",
        )?;
        for id in ids {
            let rows =
                users.query_map(params![id, PROMPT_SEARCH_LIMIT], |r| r.get::<_, String>(0))?;
            for json in rows {
                if let Ok(parsed) = parse_line(&json?)
                    && let Record::User(user) = parsed.record
                    && user.is_prompt
                {
                    return Ok(Some(user.text_chars));
                }
            }
        }
        Ok(None)
    }

    pub fn session_cwd(&self, session_id: &str) -> Result<Option<String>> {
        let stored: Option<String> = self
            .conn
            .prepare_cached("SELECT cwd FROM sessions WHERE id = ?1")?
            .query_row([session_id], |r| r.get(0))
            .optional()?
            .flatten();
        if stored.is_some() {
            return Ok(stored);
        }
        let started = self.latest_event(session_id, "session.start")?;
        Ok(started
            .and_then(|event| serde_json::from_str::<serde_json::Value>(&event.json).ok())
            .and_then(|data| data.get("cwd")?.as_str().map(String::from)))
    }

    pub fn skill_uses_in_session(&self, session_id: &str) -> Result<BTreeMap<String, u64>> {
        self.counted(SKILL_USES_IN_SESSION, params![session_id])
    }

    pub fn skill_uses_since(&self, since_ts: i64) -> Result<BTreeMap<String, u64>> {
        self.counted(SKILL_USES_SINCE, params![since_ts])
    }

    pub fn mcp_tool_uses_in_session(&self, session_id: &str) -> Result<BTreeMap<String, u64>> {
        self.counted(MCP_TOOL_USES_IN_SESSION, params![session_id])
    }

    pub fn mcp_tool_uses_since(&self, since_ts: i64) -> Result<BTreeMap<String, u64>> {
        self.counted(MCP_TOOL_USES_SINCE, params![since_ts])
    }

    fn counted(&self, sql: &str, params: impl rusqlite::Params) -> Result<BTreeMap<String, u64>> {
        let mut statement = self.conn.prepare_cached(sql)?;
        let rows = statement.query_map(params, |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn largest_tool_results(
        &self,
        session_id: &str,
        since_ts: i64,
        limit: usize,
    ) -> Result<Vec<ToolResultPoint>> {
        let mut statement = self.conn.prepare_cached(LARGEST_TOOL_RESULTS)?;
        let rows = statement.query_map(params![session_id, since_ts, limit as i64], |r| {
            Ok(ToolResultPoint {
                tool: r.get(0)?,
                ts: r.get(1)?,
                chars: r.get::<_, i64>(2)? as u64,
                images: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

fn main_attachments_sql() -> String {
    format!(
        "SELECT a.ts, a.kind, a.chars FROM attachments a JOIN files f ON f.id = a.file_id
         WHERE a.session_id = ?1 AND a.ts IS NOT NULL AND {MAIN_THREAD}
         ORDER BY a.ts, a.file_id, a.line_no"
    )
}

fn main_compactions_sql() -> String {
    format!(
        "SELECT c.ts FROM compactions c JOIN files f ON f.id = c.file_id
         WHERE c.session_id = ?1 AND c.ts IS NOT NULL AND {MAIN_THREAD} ORDER BY c.ts"
    )
}

#[cfg(test)]
fn context_statements() -> Vec<String> {
    vec![
        main_attachments_sql(),
        main_compactions_sql(),
        SKILL_USES_IN_SESSION.to_string(),
        SKILL_USES_SINCE.to_string(),
        MCP_TOOL_USES_IN_SESSION.to_string(),
        MCP_TOOL_USES_SINCE.to_string(),
        LARGEST_TOOL_RESULTS.to_string(),
    ]
}

const SKILL_USES_IN_SESSION: &str = "SELECT skill, COUNT(*) FROM requests
     WHERE session_id = ?1 AND skill IS NOT NULL GROUP BY skill";

const SKILL_USES_SINCE: &str = "SELECT skill, COUNT(*) FROM requests
     WHERE ts >= ?1 AND skill IS NOT NULL GROUP BY skill";

const MCP_TOOL_USES_IN_SESSION: &str = "SELECT name, COUNT(*) FROM tool_uses
     WHERE session_id = ?1 AND name >= 'mcp__' AND name < 'mcp_`' GROUP BY name";

const MCP_TOOL_USES_SINCE: &str = "SELECT name, COUNT(*) FROM tool_uses
     WHERE ts >= ?1 AND name >= 'mcp__' AND name < 'mcp_`' GROUP BY name";

const LARGEST_TOOL_RESULTS: &str = "SELECT u.name, r.ts, r.chars, r.images FROM tool_results r
     LEFT JOIN tool_uses u ON u.id = r.tool_use_id
     WHERE r.session_id = ?1 AND r.is_sidechain = 0 AND r.ts >= ?2
     ORDER BY r.chars DESC, r.ts LIMIT ?3";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{
        AttachmentRow, CompactionRow, EventRow, RequestRow, SessionRow, ToolResultRow, ToolUseRow,
    };
    use crate::transcript::TokenCounts;

    fn attachment(file_id: i64, line_no: u64, ts: Option<i64>, kind: &str) -> AttachmentRow {
        AttachmentRow {
            file_id,
            line_no,
            session_id: "s1".into(),
            ts,
            kind: kind.into(),
            chars: 100 + line_no,
            names: vec![],
        }
    }

    #[test]
    fn main_thread_attachments_and_compactions_leave_subagents_and_old_generations_out() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let main = w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        let agent = w.file("/p/proj/s1/subagents/agent-a1.jsonl", 0, 0).unwrap();
        let retired = w.file("/p/proj/s1.jsonl#gen-1", 0, 0).unwrap();
        w.attachment(&attachment(main, 2, Some(20), "nested_memory"))
            .unwrap();
        w.attachment(&attachment(main, 1, Some(10), "skill_listing"))
            .unwrap();
        w.attachment(&attachment(main, 3, None, "instructions"))
            .unwrap();
        w.attachment(&attachment(agent, 1, Some(15), "skill_listing"))
            .unwrap();
        w.attachment(&attachment(retired, 1, Some(5), "skill_listing"))
            .unwrap();
        for (file_id, ts) in [(main, 30), (agent, 25)] {
            w.compaction(&CompactionRow {
                file_id,
                line_no: 9,
                session_id: "s1".into(),
                ts: Some(ts),
                trigger: Some("auto".into()),
                pre_tokens: None,
                post_tokens: None,
            })
            .unwrap();
        }
        w.commit().unwrap();
        assert_eq!(
            store.main_attachments("s1").unwrap(),
            vec![
                AttachmentPoint {
                    ts: 10,
                    kind: "skill_listing".into(),
                    chars: 101
                },
                AttachmentPoint {
                    ts: 20,
                    kind: "nested_memory".into(),
                    chars: 102
                },
            ]
        );
        assert_eq!(store.main_compactions("s1").unwrap(), vec![30]);
    }

    fn plan(store: &Store, sql: &str) -> String {
        let mut statement = store
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap();
        let nulls = vec![rusqlite::types::Null; statement.parameter_count()];
        let details: Vec<String> = statement
            .query_map(rusqlite::params_from_iter(nulls), |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        details.join("; ")
    }

    #[test]
    fn the_context_reads_search_indexes_instead_of_scanning_tables() {
        let store = Store::open_in_memory().unwrap();
        for sql in context_statements() {
            let plan = plan(&store, &sql);
            assert!(!plan.contains("SCAN"), "{sql}\n=> {plan}");
        }
    }

    #[test]
    fn the_first_prompt_is_the_first_user_record_that_a_person_typed() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let agent = w.file("/p/proj/s1/subagents/agent-a1.jsonl", 0, 0).unwrap();
        let main = w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        w.raw(
            agent,
            1,
            "user",
            r#"{"type":"user","message":{"role":"user","content":{"$chars":999}}}"#,
        )
        .unwrap();
        w.raw(
            main,
            1,
            "user",
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":{"$chars":50}}}"#,
        )
        .unwrap();
        w.raw(main, 2, "user", r#"{"type":"user","message":{"role":"user","content":{"$chars":40,"$prefix":"<command-"}}}"#)
            .unwrap();
        w.raw(
            main,
            3,
            "user",
            r#"{"type":"user","message":{"role":"user","content":{"$chars":1200}}}"#,
        )
        .unwrap();
        w.raw(
            main,
            4,
            "user",
            r#"{"type":"user","message":{"role":"user","content":{"$chars":7}}}"#,
        )
        .unwrap();
        w.commit().unwrap();
        assert_eq!(store.first_prompt_chars("s1").unwrap(), Some(1200));
        assert_eq!(store.first_prompt_chars("s2").unwrap(), None);
    }

    #[test]
    fn the_session_directory_comes_from_the_transcript_then_the_mod() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.session(&SessionRow {
            id: "s1".into(),
            project: "-w-app".into(),
            cwd: Some("/w/app".into()),
            ts: Some(1),
        })
        .unwrap();
        w.event(&EventRow {
            client_id: None,
            session_id: "s2".into(),
            kind: "session.start".into(),
            ts_ms: 5,
            received_ms: 5,
            json: r#"{"cwd":"/w/new"}"#.into(),
        })
        .unwrap();
        w.commit().unwrap();
        assert_eq!(store.session_cwd("s1").unwrap().as_deref(), Some("/w/app"));
        assert_eq!(store.session_cwd("s2").unwrap().as_deref(), Some("/w/new"));
        assert_eq!(store.session_cwd("s3").unwrap(), None);
    }

    fn request(key: &str, session: &str, ts: i64, skill: Option<&str>) -> RequestRow {
        RequestRow {
            key: key.into(),
            session_id: session.into(),
            ts,
            model: "claude-opus-5-5".into(),
            tokens: TokenCounts::default(),
            cost_usd: Some(0.0),
            is_sidechain: false,
            agent_id: None,
            skill: skill.map(String::from),
            mcp_server: None,
            effort: None,
            sent_ts: None,
        }
    }

    #[test]
    fn skill_and_mcp_tool_uses_are_counted_per_session_or_everywhere_since_a_time() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.request(&request("a", "s1", 100, Some("tidy"))).unwrap();
        w.request(&request("b", "s1", 200, Some("tidy"))).unwrap();
        w.request(&request("c", "s2", 300, Some("tidy"))).unwrap();
        w.request(&request("d", "s2", 50, Some("old"))).unwrap();
        w.request(&request("e", "s1", 300, None)).unwrap();
        for (id, session, ts, name) in [
            ("t1", "s1", 100, "mcp__linear__create"),
            ("t2", "s1", 110, "mcp__linear__create"),
            ("t3", "s2", 120, "mcp__docs__find"),
            ("t4", "s1", 130, "Read"),
            ("t5", "s1", 140, "mcpXXfake"),
            ("t6", "s2", 10, "mcp__docs__find"),
        ] {
            w.tool_use(&ToolUseRow {
                id: id.into(),
                session_id: session.into(),
                ts,
                name: name.into(),
                input_chars: 1,
            })
            .unwrap();
        }
        w.commit().unwrap();
        assert_eq!(
            store.skill_uses_in_session("s1").unwrap(),
            BTreeMap::from([("tidy".to_string(), 2)])
        );
        assert_eq!(
            store.skill_uses_since(100).unwrap(),
            BTreeMap::from([("tidy".to_string(), 3)])
        );
        assert_eq!(
            store.mcp_tool_uses_in_session("s1").unwrap(),
            BTreeMap::from([("mcp__linear__create".to_string(), 2)])
        );
        assert_eq!(
            store.mcp_tool_uses_since(100).unwrap(),
            BTreeMap::from([
                ("mcp__docs__find".to_string(), 1),
                ("mcp__linear__create".to_string(), 2)
            ])
        );
    }

    #[test]
    fn the_largest_tool_results_are_the_main_threads_since_its_last_compaction() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.tool_use(&ToolUseRow {
            id: "t1".into(),
            session_id: "s1".into(),
            ts: 10,
            name: "Read".into(),
            input_chars: 1,
        })
        .unwrap();
        for (id, session, ts, chars, is_sidechain) in [
            ("t1", "s1", 20, 9000, false),
            ("t2", "s1", 20, 400, false),
            ("t3", "s2", 20, 99_999, false),
            ("t4", "s1", 20, 3000, false),
            ("t5", "s1", 20, 60_000, true),
            ("t6", "s1", 5, 50_000, false),
        ] {
            w.tool_result(&ToolResultRow {
                tool_use_id: id.into(),
                session_id: session.into(),
                ts,
                chars,
                images: 0,
                is_error: false,
                is_sidechain,
            })
            .unwrap();
        }
        w.commit().unwrap();
        let largest = store.largest_tool_results("s1", 10, 2).unwrap();
        assert_eq!(
            largest,
            vec![
                ToolResultPoint {
                    tool: Some("Read".into()),
                    ts: 20,
                    chars: 9000,
                    images: 0
                },
                ToolResultPoint {
                    tool: None,
                    ts: 20,
                    chars: 3000,
                    images: 0
                },
            ]
        );
    }
}
