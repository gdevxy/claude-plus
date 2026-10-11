use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::breakdown::Breakdown;
use super::ratios::{Ratios, TextKind};
use crate::config_scan::{Loading, Scan, Scope, SourceKind};

pub const MAX_ITEMS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    MemoryFile,
    Skill,
    McpServer,
    Agent,
    OutputStyle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub kind: ItemKind,
    pub name: String,
    pub detail: Option<String>,
    pub tokens: u64,
    pub estimated: bool,
    pub uses_session: Option<u64>,
    pub uses_14d: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Uses {
    pub session_skills: BTreeMap<String, u64>,
    pub recent_skills: BTreeMap<String, u64>,
    pub session_tools: BTreeMap<String, u64>,
    pub recent_tools: BTreeMap<String, u64>,
}

pub fn items(
    breakdown: Option<&Breakdown>,
    scan: &Scan,
    ratios: &Ratios,
    uses: &Uses,
) -> Vec<Item> {
    let mut items = match breakdown {
        Some(breakdown) => measured(breakdown, scan, uses),
        None => estimated(scan, ratios, uses),
    };
    items.extend(
        scan.sources
            .iter()
            .filter(|s| s.kind == SourceKind::OutputStyle && s.loading == Loading::Always)
            .map(|style| Item {
                kind: ItemKind::OutputStyle,
                name: style.name.clone(),
                detail: Some(scope_label(style.scope).into()),
                tokens: ratios.tokens(style.chars, TextKind::Prose),
                estimated: true,
                uses_session: None,
                uses_14d: None,
            }),
    );
    items.sort_by(|a, b| {
        b.tokens
            .cmp(&a.tokens)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.name.cmp(&b.name))
    });
    items
}

pub fn sized(items: &[Item]) -> Vec<Item> {
    items
        .iter()
        .filter(|item| item.tokens > 0)
        .take(MAX_ITEMS)
        .cloned()
        .collect()
}

pub fn dead_weight(items: &[Item]) -> Vec<Item> {
    items
        .iter()
        .filter(|item| item.tokens > 0 && item.uses_14d == Some(0))
        .cloned()
        .collect()
}

fn measured(breakdown: &Breakdown, scan: &Scan, uses: &Uses) -> Vec<Item> {
    let mut items: Vec<Item> = breakdown
        .memory_files
        .iter()
        .map(|file| Item {
            kind: ItemKind::MemoryFile,
            name: scan
                .sources
                .iter()
                .find(|s| s.path.as_deref() == Some(file.path.as_str()))
                .map(|s| s.name.clone())
                .unwrap_or_else(|| file_name(&file.path)),
            detail: Some(file.kind.clone()).filter(|kind| !kind.is_empty()),
            tokens: file.tokens,
            estimated: false,
            uses_session: None,
            uses_14d: None,
        })
        .collect();
    items.extend(breakdown.skills.iter().map(|skill| Item {
        kind: ItemKind::Skill,
        name: skill.name.clone(),
        detail: skill.plugin.clone(),
        tokens: skill.tokens,
        estimated: false,
        uses_session: Some(count(&uses.session_skills, &skill.name)),
        uses_14d: Some(count(&uses.recent_skills, &skill.name)),
    }));
    let mut servers: BTreeMap<&str, (u64, Vec<&str>)> = BTreeMap::new();
    for tool in &breakdown.mcp_tools {
        let server = servers.entry(tool.server.as_str()).or_default();
        if tool.loaded {
            server.0 += tool.tokens;
        }
        server.1.push(tool.name.as_str());
    }
    items.extend(servers.into_iter().map(|(server, (tokens, tools))| Item {
        kind: ItemKind::McpServer,
        name: server.to_string(),
        detail: Some(format!(
            "{} tool{}",
            tools.len(),
            if tools.len() == 1 { "" } else { "s" }
        )),
        tokens,
        estimated: false,
        uses_session: Some(tools.iter().map(|t| count(&uses.session_tools, t)).sum()),
        uses_14d: Some(tools.iter().map(|t| count(&uses.recent_tools, t)).sum()),
    }));
    items.extend(breakdown.agents.iter().map(|agent| Item {
        kind: ItemKind::Agent,
        name: agent.name.clone(),
        detail: None,
        tokens: agent.tokens,
        estimated: false,
        uses_session: None,
        uses_14d: None,
    }));
    items
}

fn estimated(scan: &Scan, ratios: &Ratios, uses: &Uses) -> Vec<Item> {
    scan.sources
        .iter()
        .filter_map(|source| {
            let kind = match (source.kind, source.loading) {
                (SourceKind::ClaudeMd | SourceKind::Memory, Loading::Always) => {
                    ItemKind::MemoryFile
                }
                (SourceKind::Skill, Loading::Listing) => ItemKind::Skill,
                (SourceKind::Agent, Loading::Listing) => ItemKind::Agent,
                _ => return None,
            };
            let skill = kind == ItemKind::Skill;
            Some(Item {
                kind,
                name: source.name.clone(),
                detail: if skill {
                    source.plugin.clone()
                } else {
                    Some(scope_label(source.scope).into())
                },
                tokens: ratios.tokens(source.chars, TextKind::Prose),
                estimated: true,
                uses_session: skill.then(|| count(&uses.session_skills, &source.name)),
                uses_14d: skill.then(|| count(&uses.recent_skills, &source.name)),
            })
        })
        .collect()
}

fn count(uses: &BTreeMap<String, u64>, name: &str) -> u64 {
    uses.get(name).copied().unwrap_or(0)
}

fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn scope_label(scope: Scope) -> &'static str {
    match scope {
        Scope::User => "User",
        Scope::Project => "Project",
        Scope::Local => "Local",
        Scope::Nested => "Nested",
        Scope::Plugin => "Plugin",
    }
}
#[cfg(test)]
mod tests {
    use super::super::breakdown::{parse, tests::USAGE};
    use super::*;
    use crate::config_scan::Source;

    fn source(
        kind: SourceKind,
        scope: Scope,
        name: &str,
        path: Option<&str>,
        loading: Loading,
        chars: u64,
    ) -> Source {
        Source {
            kind,
            scope,
            name: name.into(),
            path: path.map(String::from),
            plugin: None,
            loading,
            chars,
            file_chars: chars,
        }
    }

    fn scan() -> Scan {
        Scan {
            sources: vec![
                source(
                    SourceKind::ClaudeMd,
                    Scope::User,
                    "CLAUDE.md",
                    Some("/home/ada/.claude/CLAUDE.md"),
                    Loading::Always,
                    1400,
                ),
                source(
                    SourceKind::ClaudeMd,
                    Scope::Nested,
                    "src/CLAUDE.md",
                    Some("/work/app/src/CLAUDE.md"),
                    Loading::OnDemand,
                    9000,
                ),
                source(
                    SourceKind::Skill,
                    Scope::User,
                    "tidy",
                    None,
                    Loading::Listing,
                    350,
                ),
                source(
                    SourceKind::Skill,
                    Scope::User,
                    "manual",
                    None,
                    Loading::OnDemand,
                    7000,
                ),
                source(
                    SourceKind::Agent,
                    Scope::User,
                    "reviewer",
                    None,
                    Loading::Listing,
                    70,
                ),
                source(
                    SourceKind::OutputStyle,
                    Scope::User,
                    "Terse",
                    None,
                    Loading::Always,
                    700,
                ),
                source(
                    SourceKind::OutputStyle,
                    Scope::User,
                    "Chatty",
                    None,
                    Loading::Off,
                    7000,
                ),
                source(
                    SourceKind::McpServer,
                    Scope::User,
                    "linear",
                    None,
                    Loading::Unknown,
                    0,
                ),
            ],
            memory_dirs: vec![],
        }
    }

    fn uses() -> Uses {
        Uses {
            session_skills: BTreeMap::from([("superpowers:brainstorming".into(), 4)]),
            recent_skills: BTreeMap::from([("superpowers:brainstorming".into(), 9)]),
            session_tools: BTreeMap::from([("mcp__linear__search".into(), 2)]),
            recent_tools: BTreeMap::from([
                ("mcp__linear__search".into(), 3),
                ("mcp__linear__create_issue".into(), 1),
            ]),
        }
    }

    type Row<'a> = (
        ItemKind,
        &'a str,
        Option<&'a str>,
        u64,
        bool,
        Option<u64>,
        Option<u64>,
    );

    fn rows(items: &[Item]) -> Vec<Row<'_>> {
        items
            .iter()
            .map(|i| {
                (
                    i.kind,
                    i.name.as_str(),
                    i.detail.as_deref(),
                    i.tokens,
                    i.estimated,
                    i.uses_session,
                    i.uses_14d,
                )
            })
            .collect()
    }

    #[test]
    fn a_live_breakdown_gives_exact_items_with_their_uses() {
        let breakdown = parse(1, USAGE).unwrap();
        let all = items(Some(&breakdown), &scan(), &Ratios::default(), &uses());
        assert_eq!(
            rows(&all),
            vec![
                (
                    ItemKind::McpServer,
                    "linear",
                    Some("2 tools"),
                    4100,
                    false,
                    Some(2),
                    Some(4)
                ),
                (
                    ItemKind::Skill,
                    "superpowers:brainstorming",
                    Some("superpowers"),
                    2300,
                    false,
                    Some(4),
                    Some(9)
                ),
                (
                    ItemKind::MemoryFile,
                    "CLAUDE.md",
                    Some("Project"),
                    1600,
                    false,
                    None,
                    None
                ),
                (
                    ItemKind::MemoryFile,
                    "CLAUDE.md",
                    Some("User"),
                    400,
                    false,
                    None,
                    None
                ),
                (ItemKind::Skill, "tidy", None, 300, false, Some(0), Some(0)),
                (
                    ItemKind::OutputStyle,
                    "Terse",
                    Some("User"),
                    200,
                    true,
                    None,
                    None
                ),
                (ItemKind::Agent, "reviewer", None, 120, false, None, None),
                (
                    ItemKind::McpServer,
                    "docs",
                    Some("1 tool"),
                    0,
                    false,
                    Some(0),
                    Some(0)
                ),
            ]
        );
        assert_eq!(sized(&all).len(), 7);
        assert_eq!(
            rows(&dead_weight(&all)),
            vec![(ItemKind::Skill, "tidy", None, 300, false, Some(0), Some(0))]
        );
    }

    #[test]
    fn without_a_breakdown_items_are_estimated_from_the_scan() {
        let all = items(None, &scan(), &Ratios::default(), &Uses::default());
        assert_eq!(
            rows(&all),
            vec![
                (
                    ItemKind::MemoryFile,
                    "CLAUDE.md",
                    Some("User"),
                    400,
                    true,
                    None,
                    None
                ),
                (
                    ItemKind::OutputStyle,
                    "Terse",
                    Some("User"),
                    200,
                    true,
                    None,
                    None
                ),
                (ItemKind::Skill, "tidy", None, 100, true, Some(0), Some(0)),
                (
                    ItemKind::Agent,
                    "reviewer",
                    Some("User"),
                    20,
                    true,
                    None,
                    None
                ),
            ]
        );
        assert_eq!(dead_weight(&all).len(), 1);
    }
}
