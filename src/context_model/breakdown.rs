use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategoryKind {
    Used,
    Free,
    Buffer,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Category {
    pub name: String,
    pub tokens: u64,
    pub color: String,
    pub kind: CategoryKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryFile {
    pub path: String,
    pub kind: String,
    pub tokens: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct McpTool {
    pub name: String,
    pub server: String,
    pub tokens: u64,
    pub loaded: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub source: String,
    pub plugin: Option<String>,
    pub tokens: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Breakdown {
    pub ts_ms: i64,
    pub model: String,
    pub total_tokens: u64,
    pub max_tokens: u64,
    pub percentage: f64,
    pub auto_compact_threshold: Option<u64>,
    pub categories: Vec<Category>,
    pub memory_files: Vec<MemoryFile>,
    pub mcp_tools: Vec<McpTool>,
    pub agents: Vec<Entry>,
    pub skills: Vec<Entry>,
    pub skills_total: u64,
    pub skills_included: u64,
}

pub fn parse(ts_ms: i64, json: &str) -> Option<Breakdown> {
    let data: Value = serde_json::from_str(json).ok()?;
    let categories = data
        .get("categories")?
        .as_array()?
        .iter()
        .filter_map(category)
        .collect();
    let skills = data.get("skills");
    Some(Breakdown {
        ts_ms,
        model: text(&data, "model").unwrap_or_default(),
        total_tokens: count(&data, "totalTokens"),
        max_tokens: count(&data, "rawMaxTokens").max(count(&data, "maxTokens")),
        percentage: data
            .get("percentage")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        auto_compact_threshold: data
            .get("autoCompactThreshold")
            .and_then(Value::as_f64)
            .map(|v| v.max(0.0).round() as u64),
        categories,
        memory_files: each(&data, "memoryFiles", |item| {
            Some(MemoryFile {
                path: text(item, "path")?,
                kind: text(item, "type").unwrap_or_default(),
                tokens: count(item, "tokens"),
            })
        }),
        mcp_tools: each(&data, "mcpTools", |item| {
            Some(McpTool {
                name: text(item, "name")?,
                server: text(item, "serverName").unwrap_or_default(),
                tokens: count(item, "tokens"),
                loaded: item
                    .get("isLoaded")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            })
        }),
        agents: each(&data, "agents", |item| {
            Some(Entry {
                name: text(item, "agentType")?,
                source: text(item, "source").unwrap_or_default(),
                plugin: None,
                tokens: count(item, "tokens"),
            })
        }),
        skills: skills
            .map(|skills| {
                each(skills, "skillFrontmatter", |item| {
                    Some(Entry {
                        name: text(item, "name")?,
                        source: text(item, "source").unwrap_or_default(),
                        plugin: text(item, "pluginName"),
                        tokens: count(item, "tokens"),
                    })
                })
            })
            .unwrap_or_default(),
        skills_total: skills.map_or(0, |s| count(s, "totalSkills")),
        skills_included: skills.map_or(0, |s| count(s, "includedSkills")),
    })
}

fn category(item: &Value) -> Option<Category> {
    let deferred = item
        .get("isDeferred")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let kind = match item.get("kind").and_then(Value::as_str) {
        _ if deferred => CategoryKind::Deferred,
        Some("free") => CategoryKind::Free,
        Some("buffer") => CategoryKind::Buffer,
        Some("deferred") => CategoryKind::Deferred,
        _ => CategoryKind::Used,
    };
    Some(Category {
        name: text(item, "name")?,
        tokens: count(item, "tokens"),
        color: text(item, "color").unwrap_or_default(),
        kind,
    })
}

fn each<T>(data: &Value, key: &str, read: impl Fn(&Value) -> Option<T>) -> Vec<T> {
    data.get(key)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(read).collect())
        .unwrap_or_default()
}

fn text(data: &Value, key: &str) -> Option<String> {
    data.get(key)?.as_str().map(String::from)
}

fn count(data: &Value, key: &str) -> u64 {
    data.get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .map_or(0, |v| v.max(0.0).round() as u64)
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const USAGE: &str = r#"{
        "model": "claude-opus-5-5", "totalTokens": 46000, "maxTokens": 200000, "rawMaxTokens": 200000,
        "percentage": 23, "autocompactSource": "auto", "autoCompactThreshold": 167000, "isAutoCompactEnabled": true,
        "categories": [
            {"name": "System prompt", "tokens": 3100, "color": "promptBorder", "isDeferred": false, "kind": "used"},
            {"name": "System tools", "tokens": 14200, "color": "inactive", "isDeferred": false, "kind": "used"},
            {"name": "MCP tools", "tokens": 4100, "color": "permission", "isDeferred": false, "kind": "used"},
            {"name": "MCP tools (deferred)", "tokens": 8400, "color": "inactive", "isDeferred": true, "kind": "deferred"},
            {"name": "Memory files", "tokens": 2000.4, "color": "claude", "isDeferred": false, "kind": "used"},
            {"name": "Skills", "tokens": 2600, "color": "warning", "isDeferred": false, "kind": "used"},
            {"name": "Messages", "tokens": 20000, "color": "purple_FOR_SUBAGENTS_ONLY", "isDeferred": false, "kind": "used"},
            {"name": "Free space", "tokens": 121000, "color": "promptBorder", "isDeferred": false, "kind": "free"},
            {"name": "Autocompact buffer", "tokens": 33000, "color": "inactive", "isDeferred": false, "kind": "buffer"},
            {"tokens": 1}
        ],
        "memoryFiles": [
            {"path": "/home/ada/.claude/CLAUDE.md", "type": "User", "tokens": 400},
            {"path": "/work/app/CLAUDE.md", "type": "Project", "tokens": 1600}
        ],
        "mcpTools": [
            {"name": "mcp__linear__create_issue", "serverName": "linear", "tokens": 2500, "isLoaded": true},
            {"name": "mcp__linear__search", "serverName": "linear", "tokens": 1600, "isLoaded": true},
            {"name": "mcp__docs__find", "serverName": "docs", "tokens": 900, "isLoaded": false}
        ],
        "agents": [{"agentType": "reviewer", "source": "userSettings", "tokens": 120}],
        "skills": {"totalSkills": 3, "includedSkills": 2, "tokens": 2600, "skillFrontmatter": [
            {"name": "tidy", "source": "userSettings", "tokens": 300},
            {"name": "superpowers:brainstorming", "source": "plugin", "pluginName": "superpowers", "tokens": 2300}
        ]},
        "apiUsage": {"input_tokens": 3, "output_tokens": 500, "cache_read_input_tokens": 40000, "cache_creation_input_tokens": 6000}
    }"#;

    #[test]
    fn a_full_breakdown_is_read_with_rounded_counts_and_kinds() {
        let breakdown = parse(7, USAGE).unwrap();
        assert_eq!(
            (
                breakdown.ts_ms,
                breakdown.model.as_str(),
                breakdown.total_tokens,
                breakdown.max_tokens
            ),
            (7, "claude-opus-5-5", 46_000, 200_000)
        );
        assert_eq!(breakdown.auto_compact_threshold, Some(167_000));
        let kinds: Vec<(&str, u64, CategoryKind)> = breakdown
            .categories
            .iter()
            .map(|c| (c.name.as_str(), c.tokens, c.kind))
            .collect();
        assert_eq!(
            kinds[3],
            ("MCP tools (deferred)", 8400, CategoryKind::Deferred)
        );
        assert_eq!(kinds[4], ("Memory files", 2000, CategoryKind::Used));
        assert_eq!(kinds[7].2, CategoryKind::Free);
        assert_eq!(kinds[8].2, CategoryKind::Buffer);
        assert_eq!(kinds.len(), 9);
        assert_eq!(breakdown.memory_files[1].kind, "Project");
        assert_eq!(breakdown.mcp_tools.len(), 3);
        assert!(!breakdown.mcp_tools[2].loaded);
        assert_eq!(breakdown.agents[0].name, "reviewer");
        assert_eq!(breakdown.skills[1].plugin.as_deref(), Some("superpowers"));
        assert_eq!((breakdown.skills_total, breakdown.skills_included), (3, 2));
    }

    #[test]
    fn anything_but_a_breakdown_reads_as_none_and_missing_lists_as_empty() {
        assert_eq!(parse(1, "not json"), None);
        assert_eq!(parse(1, r#"{"totalTokens": 5}"#), None);
        let bare = parse(
            1,
            r#"{"categories": [], "totalTokens": -4, "maxTokens": "x"}"#,
        )
        .unwrap();
        assert_eq!((bare.total_tokens, bare.max_tokens), (0, 0));
        assert!(bare.memory_files.is_empty() && bare.skills.is_empty());
        assert_eq!(bare.auto_compact_threshold, None);
    }
}
