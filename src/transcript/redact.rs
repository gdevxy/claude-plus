use serde_json::{Map, Value, json};

use super::record::ATTACHMENT_METADATA_KEYS;
use super::text::{SHRUNK_KEY, shrink};

pub const RAW_STRING_LIMIT: usize = 2048;
pub const TOOL_INPUT_LIMIT: usize = 256;

const STRUCTURE_KEYS: &[&str] = &["type"];
const CONTENT_BLOCKS: &[&str] = &["text", "thinking", "redacted_thinking", "image", "document"];
const TOOL_USE_BLOCKS: &[&str] = &["tool_use", "server_tool_use"];

pub fn redact(raw: &Value) -> Value {
    let mut value = raw.clone();
    if let Some(record) = value.as_object_mut() {
        match record.get("type").and_then(Value::as_str) {
            Some("assistant") => redact_message(record),
            Some("user") => {
                redact_message(record);
                redact_fields(record, &["toolUseResult"], STRUCTURE_KEYS);
            }
            Some("attachment") => {
                redact_fields(record, &["attachment"], ATTACHMENT_METADATA_KEYS);
                redact_fields(record, &["rendered", "renderedInHumanTurn"], STRUCTURE_KEYS);
            }
            Some("system") => redact_fields(record, &["content", "prompt"], STRUCTURE_KEYS),
            Some("last-prompt") => redact_fields(record, &["lastPrompt"], STRUCTURE_KEYS),
            _ => {}
        }
    }
    shrink(&value, RAW_STRING_LIMIT)
}

fn redact_message(record: &mut Map<String, Value>) {
    let Some(content) = record
        .get_mut("message")
        .and_then(Value::as_object_mut)
        .and_then(|message| message.get_mut("content"))
    else {
        return;
    };
    match content {
        Value::Array(blocks) => blocks.iter_mut().for_each(redact_block),
        other => *other = lengths(other, STRUCTURE_KEYS),
    }
}

fn redact_block(block: &mut Value) {
    let kind = block.get("type").and_then(Value::as_str).unwrap_or("");
    if CONTENT_BLOCKS.contains(&kind) {
        *block = lengths(block, STRUCTURE_KEYS);
    } else if TOOL_USE_BLOCKS.contains(&kind) {
        if let Some(input) = block.get_mut("input") {
            *input = shrink(input, TOOL_INPUT_LIMIT);
        }
    } else if kind == "tool_result"
        && let Some(object) = block.as_object_mut()
    {
        redact_fields(object, &["content"], STRUCTURE_KEYS);
    }
}

fn redact_fields(record: &mut Map<String, Value>, fields: &[&str], kept: &[&str]) {
    for field in fields {
        if let Some(value) = record.get_mut(*field) {
            *value = lengths(value, kept);
        }
    }
}

fn lengths(value: &Value, kept: &[&str]) -> Value {
    match value {
        Value::String(s) => json!({ SHRUNK_KEY: s.chars().count() }),
        Value::Array(items) => Value::Array(items.iter().map(|item| lengths(item, kept)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| {
                    let item = if kept.contains(&key.as_str()) {
                        item.clone()
                    } else {
                        lengths(item, kept)
                    };
                    (key.clone(), item)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::{Record, parse_value};
    use serde_json::json;

    const SECRET: &str = "hunter2";

    fn secret(len: usize) -> String {
        format!("{SECRET}{}", "x".repeat(len.saturating_sub(SECRET.len())))
    }

    fn assistant() -> Value {
        json!({"type":"assistant","sessionId":"s1","requestId":"r1","message":{"model":"claude-opus-5-5","content":[
            {"type":"thinking","thinking":secret(40),"signature":secret(300)},
            {"type":"text","text":secret(12)},
            {"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"cargo test","description":secret(300),"nested":["--release"]}},
            {"type":"tool_use","id":"toolu_2","name":"Read","input":{"file_path":"/w/src/main.rs"}}
        ],"usage":{"input_tokens":1,"output_tokens":2}}})
    }

    fn user_with_tool_results() -> Value {
        json!({"type":"user","sessionId":"s1","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"toolu_1","content":secret(5000),"is_error":true},
            {"type":"tool_result","tool_use_id":"toolu_2","content":[
                {"type":"text","text":secret(30)},
                {"type":"image","source":{"type":"base64","media_type":"image/png","data":secret(900)}},
                {"type":"tool_reference","tool_name":"mcp__x__y"}
            ]}
        ]},"toolUseResult":{"stdout":secret(50),"stderr":"","interrupted":false,"usage":{"output_tokens":7}}})
    }

    fn prompt() -> Value {
        json!({"type":"user","sessionId":"s1","message":{"role":"user","content":secret(25)}})
    }

    fn attachment() -> Value {
        json!({"type":"attachment","sessionId":"s1","attachment":{
            "type":"instructions",
            "files":[{"path":"/h/CLAUDE.md","type":"User","content":secret(70)}],
            "names":["alpha"],"addedNames":["beta"],"entries":[{"name":"gamma","body":secret(9)}],
            "path":"/h/notes.md","content":secret(3000)
        },"rendered":[{"content":secret(15)}]})
    }

    fn system() -> Value {
        json!({"type":"system","subtype":"local_command","sessionId":"s1","content":secret(40),"level":"info"})
    }

    fn last_prompt() -> Value {
        json!({"type":"last-prompt","sessionId":"s1","lastPrompt":secret(10)})
    }

    fn samples() -> Vec<Value> {
        vec![
            assistant(),
            user_with_tool_results(),
            prompt(),
            attachment(),
            system(),
            last_prompt(),
        ]
    }

    #[test]
    fn content_strings_keep_only_their_length() {
        for sample in samples() {
            let redacted = redact(&sample).to_string();
            assert!(!redacted.contains(SECRET), "{redacted}");
        }
    }

    #[test]
    fn tool_inputs_keep_short_strings() {
        let redacted = redact(&assistant());
        let content = &redacted["message"]["content"];
        assert_eq!(content[2]["input"]["command"], "cargo test");
        assert_eq!(content[2]["input"]["description"], json!({"$chars": 300}));
        assert_eq!(content[2]["input"]["nested"], json!(["--release"]));
        assert_eq!(content[3]["input"]["file_path"], "/w/src/main.rs");
        assert_eq!(content[2]["name"], "Bash");
        assert_eq!(content[2]["id"], "toolu_1");
    }

    #[test]
    fn attachment_names_and_paths_survive() {
        let redacted = redact(&attachment());
        let body = &redacted["attachment"];
        assert_eq!(body["type"], "instructions");
        assert_eq!(body["files"][0]["path"], "/h/CLAUDE.md");
        assert_eq!(body["files"][0]["content"], json!({"$chars": 70}));
        assert_eq!(body["names"], json!(["alpha"]));
        assert_eq!(body["addedNames"], json!(["beta"]));
        assert_eq!(body["entries"][0]["name"], "gamma");
        assert_eq!(body["path"], "/h/notes.md");
        assert_eq!(redacted["rendered"][0]["content"], json!({"$chars": 15}));
    }

    #[test]
    fn identifiers_and_numbers_survive() {
        let redacted = redact(&user_with_tool_results());
        let content = &redacted["message"]["content"];
        assert_eq!(content[0]["tool_use_id"], "toolu_1");
        assert_eq!(content[0]["is_error"], true);
        assert_eq!(content[1]["content"][1]["type"], "image");
        assert_eq!(redacted["toolUseResult"]["usage"]["output_tokens"], 7);
        assert_eq!(redacted["sessionId"], "s1");
    }

    #[test]
    fn redaction_is_idempotent() {
        for sample in samples() {
            let once = redact(&sample);
            assert_eq!(redact(&once), once);
        }
    }

    #[derive(Debug, PartialEq)]
    enum Derived {
        Assistant(
            Vec<crate::transcript::ToolUse>,
            crate::transcript::TokenCounts,
        ),
        User(Vec<crate::transcript::ToolResult>, u64),
        Attachment(String, u64, Vec<String>),
        Other(String),
    }

    fn derived(value: Value) -> Derived {
        match parse_value(value).unwrap().record {
            Record::Assistant(a) => Derived::Assistant(a.tool_uses, a.usage),
            Record::User(u) => Derived::User(u.tool_results, u.text_chars),
            Record::Attachment(a) => Derived::Attachment(a.kind, a.chars, a.names),
            other => Derived::Other(other.kind().to_string()),
        }
    }

    #[test]
    fn derived_counts_are_identical_after_redaction() {
        for sample in samples() {
            assert_eq!(
                derived(redact(&sample)),
                derived(sample.clone()),
                "{sample}"
            );
        }
    }
}
