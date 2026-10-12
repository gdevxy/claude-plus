use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::types::Value;
use rusqlite::{OptionalExtension, params};

use super::context::MAIN_THREAD;
use super::{PREFIX_KINDS, Store};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptPoint {
    pub session_id: String,
    pub ts: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LargeResult {
    pub session_id: String,
    pub ts: i64,
    pub chars: u64,
    pub tool: Option<String>,
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixChange {
    pub ts: i64,
    pub kind: String,
    pub names: Vec<String>,
    pub removed: Vec<String>,
}

const PROMPTS_BETWEEN: &str =
    "SELECT session_id, ts FROM prompts WHERE ts >= ?1 AND ts < ?2 ORDER BY session_id, ts, uuid";

const LARGE_RESULTS_BETWEEN: &str = "SELECT r.session_id, r.ts, r.chars, u.name, u.target
     FROM tool_results r LEFT JOIN tool_uses u ON u.id = r.tool_use_id
     WHERE r.ts >= ?1 AND r.ts < ?2 AND r.chars >= ?3 AND r.is_sidechain = 0
     ORDER BY r.session_id, r.ts, r.tool_use_id";

const LAST_PROMPT_BEFORE: &str = "SELECT session_id, ts FROM prompts WHERE session_id = ?1 AND ts < ?2 ORDER BY ts DESC, uuid DESC LIMIT 1";

const SESSION_PROJECT: &str = "SELECT project FROM sessions WHERE id = ?1";

const AGENT_TYPES_BETWEEN: &str = "SELECT DISTINCT a.agent_id, a.agent_type
     FROM requests r JOIN agents a ON a.agent_id = r.agent_id
     WHERE r.ts >= ?1 AND r.ts < ?2";

const KNOWS_AGENT: &str = "SELECT 1 FROM agents WHERE agent_id = ?1";

fn prefix_kinds_sql() -> String {
    let kinds = vec!["?"; PREFIX_KINDS.len()].join(", ");
    format!(
        "SELECT a.ts, a.kind, a.names, a.removed_names FROM attachments a JOIN files f ON f.id = a.file_id
         WHERE a.session_id = ? AND a.ts >= ? AND a.ts < ? AND a.kind IN ({kinds}) AND {MAIN_THREAD}
         ORDER BY a.ts, a.kind, a.file_id, a.line_no"
    )
}

#[cfg(test)]
fn insights_statements() -> Vec<String> {
    vec![
        PROMPTS_BETWEEN.to_string(),
        LAST_PROMPT_BEFORE.to_string(),
        LARGE_RESULTS_BETWEEN.to_string(),
        prefix_kinds_sql(),
        SESSION_PROJECT.to_string(),
        AGENT_TYPES_BETWEEN.to_string(),
        KNOWS_AGENT.to_string(),
    ]
}

impl Store {
    pub fn prompts_between(&self, since_ts: i64, until_ts: i64) -> Result<Vec<PromptPoint>> {
        let mut statement = self.conn.prepare_cached(PROMPTS_BETWEEN)?;
        let rows = statement.query_map([since_ts, until_ts], |r| {
            Ok(PromptPoint {
                session_id: r.get(0)?,
                ts: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn last_prompt_before(
        &self,
        session_id: &str,
        before_ts: i64,
    ) -> Result<Option<PromptPoint>> {
        Ok(self
            .conn
            .prepare_cached(LAST_PROMPT_BEFORE)?
            .query_row(params![session_id, before_ts], |r| {
                Ok(PromptPoint {
                    session_id: r.get(0)?,
                    ts: r.get(1)?,
                })
            })
            .optional()?)
    }

    pub fn large_tool_results_between(
        &self,
        since_ts: i64,
        until_ts: i64,
        min_chars: u64,
    ) -> Result<Vec<LargeResult>> {
        let mut statement = self.conn.prepare_cached(LARGE_RESULTS_BETWEEN)?;
        let rows = statement.query_map(params![since_ts, until_ts, min_chars as i64], |r| {
            Ok(LargeResult {
                session_id: r.get(0)?,
                ts: r.get(1)?,
                chars: r.get::<_, i64>(2)? as u64,
                tool: r.get(3)?,
                target: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn session_project(&self, session_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .prepare_cached(SESSION_PROJECT)?
            .query_row([session_id], |r| r.get(0))
            .optional()?)
    }

    pub fn agent_types_between(
        &self,
        since_ts: i64,
        until_ts: i64,
    ) -> Result<BTreeMap<String, String>> {
        let mut statement = self.conn.prepare_cached(AGENT_TYPES_BETWEEN)?;
        let rows = statement.query_map([since_ts, until_ts], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn knows_agent(&self, agent_id: &str) -> Result<bool> {
        Ok(self.conn.prepare_cached(KNOWS_AGENT)?.exists([agent_id])?)
    }

    pub fn prefix_kinds_between(
        &self,
        session_id: &str,
        since_ts: i64,
        until_ts: i64,
    ) -> Result<Vec<PrefixChange>> {
        let mut statement = self.conn.prepare_cached(&prefix_kinds_sql())?;
        let mut values: Vec<Value> = vec![
            session_id.to_string().into(),
            since_ts.into(),
            until_ts.into(),
        ];
        values.extend(
            PREFIX_KINDS
                .iter()
                .map(|kind| Value::from(kind.to_string())),
        );
        let rows = statement.query_map(rusqlite::params_from_iter(values), |r| {
            let list = |index| -> rusqlite::Result<Vec<String>> {
                Ok(serde_json::from_str(&r.get::<_, String>(index)?).unwrap_or_default())
            };
            Ok(PrefixChange {
                ts: r.get(0)?,
                kind: r.get(1)?,
                names: list(2)?,
                removed: list(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{AgentRow, AttachmentRow, PromptRow, RequestRow, ToolResultRow, ToolUseRow};
    use crate::transcript::TokenCounts;

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
    fn the_insights_reads_search_indexes_instead_of_scanning_tables() {
        let store = Store::open_in_memory().unwrap();
        for sql in insights_statements() {
            let plan = plan(&store, &sql);
            assert!(!plan.contains("SCAN"), "{sql}\n=> {plan}");
        }
    }

    #[test]
    fn prompts_of_a_span_are_read_by_session_then_time() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        for (uuid, session, ts) in [
            ("p1", "s2", 150),
            ("p2", "s1", 120),
            ("p3", "s1", 100),
            ("p4", "s1", 90),
            ("p5", "s1", 200),
        ] {
            w.prompt(&PromptRow {
                uuid: uuid.into(),
                session_id: session.into(),
                ts,
            })
            .unwrap();
        }
        w.prompt(&PromptRow {
            uuid: "p2".into(),
            session_id: "s1".into(),
            ts: 999,
        })
        .unwrap();
        w.commit().unwrap();
        let read: Vec<(String, i64)> = store
            .prompts_between(100, 200)
            .unwrap()
            .into_iter()
            .map(|p| (p.session_id, p.ts))
            .collect();
        assert_eq!(
            read,
            [("s1".into(), 100), ("s1".into(), 120), ("s2".into(), 150)]
        );
        assert_eq!(
            store.last_prompt_before("s1", 100).unwrap(),
            Some(PromptPoint {
                session_id: "s1".into(),
                ts: 90
            })
        );
        assert_eq!(store.last_prompt_before("s2", 150).unwrap(), None);
    }

    #[test]
    fn large_tool_results_of_the_main_thread_carry_their_tool_and_target() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        for (id, name, target) in [
            ("t1", "Read", Some("/w/big.rs")),
            ("t2", "Bash", None),
            ("t3", "Read", Some("/w/small.rs")),
        ] {
            w.tool_use(&ToolUseRow {
                id: id.into(),
                session_id: "s1".into(),
                ts: 100,
                name: name.into(),
                input_chars: 10,
                target: target.map(String::from),
            })
            .unwrap();
        }
        for (id, ts, chars, is_sidechain) in [
            ("t1", 101, 40_000, false),
            ("t2", 102, 30_000, false),
            ("t3", 103, 500, false),
            ("t4", 104, 50_000, false),
            ("t5", 105, 60_000, true),
        ] {
            w.tool_result(&ToolResultRow {
                tool_use_id: id.into(),
                session_id: "s1".into(),
                ts,
                chars,
                images: 0,
                is_error: false,
                is_sidechain,
            })
            .unwrap();
        }
        w.commit().unwrap();
        assert_eq!(
            store.large_tool_results_between(100, 200, 20_000).unwrap(),
            vec![
                LargeResult {
                    session_id: "s1".into(),
                    ts: 101,
                    chars: 40_000,
                    tool: Some("Read".into()),
                    target: Some("/w/big.rs".into()),
                },
                LargeResult {
                    session_id: "s1".into(),
                    ts: 102,
                    chars: 30_000,
                    tool: Some("Bash".into()),
                    target: None,
                },
                LargeResult {
                    session_id: "s1".into(),
                    ts: 104,
                    chars: 50_000,
                    tool: None,
                    target: None,
                },
            ]
        );
    }

    #[test]
    fn prefix_changes_of_a_span_name_their_kind_and_what_they_announced_on_the_main_thread() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let main = w.file("/p/s1.jsonl", 0, 0).unwrap();
        let agent = w.file("/p/s1/subagents/agent-a.jsonl", 0, 0).unwrap();
        for (file_id, line_no, ts, kind, names, removed) in [
            (
                main,
                1,
                100,
                "mcp_instructions_delta",
                vec!["linear"],
                vec![],
            ),
            (main, 2, 110, "date", vec![], vec![]),
            (agent, 1, 115, "deferred_tools_record", vec![], vec![]),
            (
                main,
                3,
                120,
                "deferred_tools_delta",
                vec!["mcp__a__x"],
                vec!["mcp__b__y"],
            ),
            (main, 4, 300, "instructions", vec![], vec![]),
        ] {
            w.attachment(&AttachmentRow {
                file_id,
                line_no,
                session_id: "s1".into(),
                ts: Some(ts),
                kind: kind.into(),
                chars: 10,
                names: names.into_iter().map(String::from).collect(),
                removed: removed.into_iter().map(String::from).collect(),
            })
            .unwrap();
        }
        w.commit().unwrap();
        assert_eq!(
            store.prefix_kinds_between("s1", 100, 300).unwrap(),
            vec![
                PrefixChange {
                    ts: 100,
                    kind: "mcp_instructions_delta".into(),
                    names: vec!["linear".into()],
                    removed: vec![],
                },
                PrefixChange {
                    ts: 120,
                    kind: "deferred_tools_delta".into(),
                    names: vec!["mcp__a__x".into()],
                    removed: vec!["mcp__b__y".into()],
                },
            ]
        );
    }

    #[test]
    fn agent_types_are_read_for_the_subagents_that_spent_in_a_span() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        for (agent_id, agent_type) in [("a1", "Explore"), ("a2", "reviewer"), ("a3", "Plan")] {
            w.agent(&AgentRow {
                agent_id: agent_id.into(),
                session_id: "s1".into(),
                agent_type: agent_type.into(),
            })
            .unwrap();
        }
        for (key, ts, agent_id) in [
            ("r1", 100, Some("a1")),
            ("r2", 110, Some("a1")),
            ("r3", 120, Some("a2")),
            ("r4", 130, None),
            ("r5", 300, Some("a3")),
            ("r6", 140, Some("a4")),
        ] {
            w.request(&RequestRow {
                key: key.into(),
                session_id: "s1".into(),
                ts,
                sent_ts: None,
                model: "claude-opus-5-5".into(),
                tokens: TokenCounts::default(),
                cost_usd: Some(0.0),
                is_sidechain: agent_id.is_some(),
                agent_id: agent_id.map(String::from),
                skill: None,
                mcp_server: None,
                effort: None,
            })
            .unwrap();
        }
        w.commit().unwrap();
        let types = store.agent_types_between(100, 200).unwrap();
        assert_eq!(
            types.into_iter().collect::<Vec<_>>(),
            [
                ("a1".to_string(), "Explore".to_string()),
                ("a2".to_string(), "reviewer".to_string()),
            ]
        );
        assert!(store.knows_agent("a3").unwrap());
        assert!(!store.knows_agent("a4").unwrap());
    }
}
