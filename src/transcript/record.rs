use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::TokenCounts;
use super::redact::TOOL_INPUT_LIMIT;
use super::text::{leading_text, non_prompt_prefix, string_chars, string_chars_except};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("line is not a JSON object")]
    NotAnObject,
}

#[derive(Debug, Clone)]
pub struct Parsed {
    pub record: Record,
    pub raw: Value,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Common {
    pub session_id: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
    pub is_sidechain: bool,
    pub agent_id: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    Assistant(AssistantRecord),
    User(UserRecord),
    Attachment(AttachmentRecord),
    CompactBoundary(CompactRecord),
    CostState(CostStateRecord),
    Title(TitleRecord),
    Other { kind: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssistantRecord {
    pub common: Common,
    pub dedup_key: Option<String>,
    pub model: String,
    pub usage: TokenCounts,
    pub tool_uses: Vec<ToolUse>,
    pub skill: Option<String>,
    pub mcp_server: Option<String>,
    pub effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolUse {
    pub id: String,
    pub name: String,
    pub input_chars: u64,
    pub target: Option<String>,
}

pub const TARGET_KEYS: &[&str] = &[
    "file_path",
    "notebook_path",
    "command",
    "url",
    "pattern",
    "path",
];

#[derive(Debug, Clone, PartialEq)]
pub struct UserRecord {
    pub common: Common,
    pub is_prompt: bool,
    pub text_chars: u64,
    pub tool_results: Vec<ToolResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub chars: u64,
    pub images: u32,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentRecord {
    pub common: Common,
    pub kind: String,
    pub chars: u64,
    pub names: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompactRecord {
    pub common: Common,
    pub trigger: Option<String>,
    pub pre_tokens: Option<u64>,
    pub post_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CostStateRecord {
    pub session_id: Option<String>,
    pub total_cost_usd: Option<f64>,
    pub model_usage: BTreeMap<String, ModelUsage>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ModelUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TitleRecord {
    pub session_id: Option<String>,
    pub title: String,
    pub is_custom: bool,
}

const SYNTHETIC_MODEL: &str = "<synthetic>";

pub(crate) const ATTACHMENT_METADATA_KEYS: &[&str] = &[
    "type",
    "names",
    "addedNames",
    "removedNames",
    "readdedNames",
    "name",
    "path",
    "filePath",
    "filename",
    "displayPath",
];

impl AssistantRecord {
    pub fn is_billable(&self) -> bool {
        self.model != SYNTHETIC_MODEL && !self.usage.is_empty()
    }
}

impl Record {
    pub fn kind(&self) -> &str {
        match self {
            Record::Assistant(_) => "assistant",
            Record::User(_) => "user",
            Record::Attachment(_) => "attachment",
            Record::CompactBoundary(_) => "compact_boundary",
            Record::CostState(_) => "cost-state",
            Record::Title(_) => "title",
            Record::Other { kind } => kind,
        }
    }

    pub fn common(&self) -> Option<&Common> {
        match self {
            Record::Assistant(r) => Some(&r.common),
            Record::User(r) => Some(&r.common),
            Record::Attachment(r) => Some(&r.common),
            Record::CompactBoundary(r) => Some(&r.common),
            _ => None,
        }
    }
}

pub fn parse_line(line: &str) -> Result<Parsed, ParseError> {
    parse_value(serde_json::from_str(line)?)
}

pub fn parse_value(raw: Value) -> Result<Parsed, ParseError> {
    if !raw.is_object() {
        return Err(ParseError::NotAnObject);
    }
    let record = match text(&raw, "/type").as_deref() {
        Some("assistant") => Record::Assistant(assistant(&raw)),
        Some("user") => Record::User(user(&raw)),
        Some("attachment") => Record::Attachment(attachment(&raw)),
        Some("system") if text(&raw, "/subtype").as_deref() == Some("compact_boundary") => {
            Record::CompactBoundary(compaction(&raw))
        }
        Some("cost-state") => Record::CostState(cost_state(&raw)),
        Some("custom-title") => title(&raw, "/customTitle", true),
        Some("ai-title") => title(&raw, "/aiTitle", false),
        Some(kind) => Record::Other {
            kind: kind.to_string(),
        },
        None => Record::Other {
            kind: "unknown".to_string(),
        },
    };
    Ok(Parsed { record, raw })
}

fn common(raw: &Value) -> Common {
    Common {
        session_id: text(raw, "/sessionId").or_else(|| text(raw, "/session_id")),
        timestamp: text(raw, "/timestamp")
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&Utc)),
        is_sidechain: raw
            .pointer("/isSidechain")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        agent_id: text(raw, "/agentId"),
        cwd: text(raw, "/cwd"),
    }
}

fn assistant(raw: &Value) -> AssistantRecord {
    let tool_uses = blocks(raw)
        .filter(|block| text(block, "/type").as_deref() == Some("tool_use"))
        .filter_map(|block| {
            Some(ToolUse {
                id: text(block, "/id")?,
                name: text(block, "/name")?,
                input_chars: block.get("input").map(string_chars).unwrap_or(0),
                target: block.get("input").and_then(target),
            })
        })
        .collect();
    AssistantRecord {
        common: common(raw),
        dedup_key: text(raw, "/requestId")
            .or_else(|| text(raw, "/message/id"))
            .or_else(|| text(raw, "/uuid")),
        model: text(raw, "/message/model").unwrap_or_default(),
        usage: raw
            .pointer("/message/usage")
            .map(TokenCounts::from_usage)
            .unwrap_or_default(),
        tool_uses,
        skill: text(raw, "/attributionSkill"),
        mcp_server: text(raw, "/attributionMcpServer"),
        effort: text(raw, "/perTurnEffort").or_else(|| text(raw, "/effort")),
    }
}

fn target(input: &Value) -> Option<String> {
    TARGET_KEYS.iter().find_map(|key| {
        let value = input.get(*key)?.as_str()?;
        (!value.is_empty() && value.len() <= TOOL_INPUT_LIMIT).then(|| value.to_string())
    })
}

fn user(raw: &Value) -> UserRecord {
    let is_meta = raw
        .pointer("/isMeta")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let content = raw.pointer("/message/content");
    let tool_results: Vec<ToolResult> = blocks(raw)
        .filter(|block| text(block, "/type").as_deref() == Some("tool_result"))
        .filter_map(|block| {
            Some(ToolResult {
                tool_use_id: text(block, "/tool_use_id")?,
                chars: block.get("content").map(result_chars).unwrap_or(0),
                images: block.get("content").map(image_count).unwrap_or(0),
                is_error: block
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect();
    let text_chars = match content {
        Some(content) if !content.is_array() => string_chars(content),
        _ => blocks(raw)
            .filter(|block| text(block, "/type").as_deref() == Some("text"))
            .map(|block| block.get("text").map(string_chars).unwrap_or(0))
            .fold(0, u64::saturating_add),
    };
    let flag = |pointer: &str| {
        raw.pointer(pointer)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    let is_prompt = !is_meta
        && !flag("/isCompactSummary")
        && !flag("/isVisibleInTranscriptOnly")
        && tool_results.is_empty()
        && text_chars > 0
        && non_prompt_prefix(first_text(raw)).is_none();
    UserRecord {
        common: common(raw),
        is_prompt,
        text_chars,
        tool_results,
    }
}

fn attachment(raw: &Value) -> AttachmentRecord {
    let body = raw.get("attachment").cloned().unwrap_or(Value::Null);
    let mut names = strings(&body, "/names");
    names.extend(strings(&body, "/addedNames"));
    names.extend(strings(&body, "/readdedNames"));
    names.extend(field_of_each(&body, "/files", "/path"));
    names.extend(field_of_each(&body, "/entries", "/name"));
    if let Some(path) = text(&body, "/path") {
        names.push(path);
    }
    AttachmentRecord {
        common: common(raw),
        kind: text(&body, "/type").unwrap_or_else(|| "unknown".to_string()),
        chars: string_chars_except(&body, ATTACHMENT_METADATA_KEYS),
        names,
        removed: strings(&body, "/removedNames"),
    }
}

fn compaction(raw: &Value) -> CompactRecord {
    CompactRecord {
        common: common(raw),
        trigger: text(raw, "/compactMetadata/trigger"),
        pre_tokens: raw
            .pointer("/compactMetadata/preTokens")
            .and_then(Value::as_u64),
        post_tokens: raw
            .pointer("/compactMetadata/postTokens")
            .and_then(Value::as_u64),
    }
}

fn cost_state(raw: &Value) -> CostStateRecord {
    let count = |usage: &Value, field: &str| usage.get(field).and_then(Value::as_u64).unwrap_or(0);
    let model_usage = raw
        .pointer("/modelUsage")
        .and_then(Value::as_object)
        .map(|models| {
            models
                .iter()
                .filter(|(_, usage)| usage.is_object())
                .map(|(model, usage)| {
                    let counters = ModelUsage {
                        input: count(usage, "inputTokens"),
                        output: count(usage, "outputTokens"),
                        cache_read: count(usage, "cacheReadInputTokens"),
                        cache_write: count(usage, "cacheCreationInputTokens"),
                        cost_usd: usage.get("costUSD").and_then(Value::as_f64).unwrap_or(0.0),
                    };
                    (model.clone(), counters)
                })
                .collect()
        })
        .unwrap_or_default();
    CostStateRecord {
        session_id: text(raw, "/sessionId"),
        total_cost_usd: raw.pointer("/totalCostUSD").and_then(Value::as_f64),
        model_usage,
    }
}

fn title(raw: &Value, pointer: &str, is_custom: bool) -> Record {
    match text(raw, pointer) {
        Some(title) => Record::Title(TitleRecord {
            session_id: text(raw, "/sessionId"),
            title,
            is_custom,
        }),
        None => Record::Other {
            kind: text(raw, "/type").unwrap_or_default(),
        },
    }
}

fn blocks(raw: &Value) -> impl Iterator<Item = &Value> {
    raw.pointer("/message/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn text(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn strings(value: &Value, pointer: &str) -> Vec<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn field_of_each(value: &Value, array: &str, field: &str) -> Vec<String> {
    value
        .pointer(array)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(|item| text(item, field)).collect())
        .unwrap_or_default()
}

fn first_text(raw: &Value) -> &str {
    match raw.pointer("/message/content") {
        Some(Value::Array(_)) => blocks(raw)
            .find(|block| text(block, "/type").as_deref() == Some("text"))
            .and_then(|block| block.get("text"))
            .and_then(leading_text),
        content => content.and_then(leading_text),
    }
    .unwrap_or_default()
}

fn result_chars(content: &Value) -> u64 {
    match content {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match text(item, "/type").as_deref() {
                Some("text") => item.get("text"),
                Some("tool_reference") => item.get("tool_name"),
                Some("document") if text(item, "/source/type").as_deref() == Some("text") => {
                    item.pointer("/source/data")
                }
                _ => None,
            })
            .map(string_chars)
            .fold(0, u64::saturating_add),
        other => string_chars(other),
    }
}

fn image_count(content: &Value) -> u32 {
    content.as_array().map_or(0, |items| {
        items
            .iter()
            .filter(|item| text(item, "/type").as_deref() == Some("image"))
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    })
}
