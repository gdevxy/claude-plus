use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::extensions::Plugin;
use super::{Loading, Roots, Scan, Scope, Source, SourceKind, display, read_json, read_text_up_to};

pub const CLAUDE_JSON_MAX_BYTES: u64 = 64 * 1024 * 1024;

pub fn servers(roots: &Roots, plugins: &[Plugin], scan: &mut Scan) {
    let text = read_text_up_to(&roots.claude_json, CLAUDE_JSON_MAX_BYTES);
    scan.claude_json_oversized = text.as_ref().is_some_and(|text| text.content.is_none());
    let sources = &mut scan.sources;
    let claude_json = text.and_then(|text| serde_json::from_str::<Value>(&text.content?).ok());
    let project_entry = roots.project.as_ref().and_then(|project| {
        claude_json
            .as_ref()?
            .pointer("/projects")?
            .get(project.to_string_lossy().as_ref())
            .cloned()
    });
    let mut add = |scope: Scope,
                   names: BTreeSet<String>,
                   origin: &Path,
                   plugin: Option<&str>,
                   off: &BTreeSet<String>| {
        for name in names {
            sources.push(Source {
                kind: SourceKind::McpServer,
                scope,
                loading: if off.contains(&name) {
                    Loading::Off
                } else {
                    Loading::Unknown
                },
                name,
                alias: None,
                path: Some(display(origin)),
                plugin: plugin.map(String::from),
                chars: 0,
                file_chars: 0,
            });
        }
    };
    let none = BTreeSet::new();
    add(
        Scope::User,
        names_in(claude_json.as_ref()),
        &roots.claude_json,
        None,
        &none,
    );
    add(
        Scope::Local,
        names_in(project_entry.as_ref()),
        &roots.claude_json,
        None,
        &none,
    );
    if let Some(project) = &roots.project {
        let file = project.join(".mcp.json");
        let disabled = strings(project_entry.as_ref(), "disabledMcpjsonServers");
        add(
            Scope::Project,
            names_in(read_json(&file).as_ref()),
            &file,
            None,
            &disabled,
        );
    }
    for plugin in plugins {
        let manifest = read_json(&plugin.dir.join(".claude-plugin/plugin.json"));
        let mut names = names_in(read_json(&plugin.dir.join(".mcp.json")).as_ref());
        match manifest.as_ref().and_then(|m| m.get("mcpServers")) {
            Some(Value::Object(inline)) => names.extend(inline.keys().cloned()),
            Some(Value::String(relative)) => {
                if let Some(file) = inside(&plugin.dir, relative) {
                    names.extend(names_in(read_json(&file).as_ref()));
                }
            }
            _ => {}
        }
        add(
            Scope::Plugin,
            names,
            &plugin.dir,
            Some(&plugin.short),
            &none,
        );
    }
}

fn inside(dir: &Path, relative: &str) -> Option<PathBuf> {
    let root = std::fs::canonicalize(dir).ok()?;
    let file = std::fs::canonicalize(dir.join(relative)).ok()?;
    file.starts_with(&root).then_some(file)
}

fn names_in(config: Option<&Value>) -> BTreeSet<String> {
    config
        .and_then(|config| config.get("mcpServers"))
        .and_then(Value::as_object)
        .map(|servers| servers.keys().cloned().collect())
        .unwrap_or_default()
}

fn strings(config: Option<&Value>, key: &str) -> BTreeSet<String> {
    config
        .and_then(|config| config.get(key))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::super::fixture::Home;
    use super::super::scan;
    use super::*;

    #[test]
    fn servers_come_from_the_user_the_project_entry_the_mcp_file_and_plugins() {
        let home = Home::new();
        let app = home.path("work/app").display().to_string();
        home.write(
            "home/.claude.json",
            &serde_json::json!({
                "mcpServers": {"context7": {"command": "npx"}, "serena": {}},
                "projects": {app.clone(): {"mcpServers": {"db": {}}, "disabledMcpjsonServers": ["legacy"]}}
            })
            .to_string(),
        );
        home.write(
            "work/app/.mcp.json",
            r#"{"mcpServers":{"legacy":{},"linear":{}}}"#,
        );
        home.write(
            "home/.claude/plugins/installed_plugins.json",
            &serde_json::json!({"plugins": {
                "atlassian@official": [{"scope": "user", "installPath": home.path("plugins/atlassian")}],
                "docs@official": [{"scope": "user", "installPath": home.path("plugins/docs")}]
            }})
            .to_string(),
        );
        home.write(
            "home/.claude/settings.json",
            r#"{"enabledPlugins":{"atlassian@official":true,"docs@official":true}}"#,
        );
        home.write(
            "plugins/atlassian/.mcp.json",
            r#"{"mcpServers":{"atlassian":{}}}"#,
        );
        home.write(
            "plugins/atlassian/.claude-plugin/plugin.json",
            r#"{"name":"atlassian","mcpServers":"./.mcp.json"}"#,
        );
        home.write(
            "plugins/docs/.claude-plugin/plugin.json",
            r#"{"name":"docs","mcpServers":{"docs-search":{}}}"#,
        );
        let found = scan(&home.roots(Some("work/app")));
        let servers: Vec<(Scope, &str, Loading, Option<&str>)> = found
            .sources
            .iter()
            .filter(|s| s.kind == SourceKind::McpServer)
            .map(|s| (s.scope, s.name.as_str(), s.loading, s.plugin.as_deref()))
            .collect();
        assert_eq!(
            servers,
            vec![
                (Scope::User, "context7", Loading::Unknown, None),
                (Scope::User, "serena", Loading::Unknown, None),
                (Scope::Local, "db", Loading::Unknown, None),
                (Scope::Project, "legacy", Loading::Off, None),
                (Scope::Project, "linear", Loading::Unknown, None),
                (
                    Scope::Plugin,
                    "atlassian",
                    Loading::Unknown,
                    Some("atlassian")
                ),
                (Scope::Plugin, "docs-search", Loading::Unknown, Some("docs")),
            ]
        );
    }

    #[test]
    fn a_claude_json_grown_past_the_file_cap_still_lists_its_servers() {
        let home = Home::new();
        let history = "x".repeat(super::super::MAX_FILE_BYTES as usize);
        home.write(
            "home/.claude.json",
            &serde_json::json!({"mcpServers": {"context7": {}}, "history": history}).to_string(),
        );
        let found = scan(&home.roots(None));
        let servers: Vec<&str> = found.sources.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(servers, vec!["context7"]);
        assert!(!found.claude_json_oversized);
    }

    #[test]
    fn a_claude_json_past_the_hard_limit_is_reported_on_the_scan() {
        let home = Home::new();
        let path = home.write("home/.claude.json", "{}");
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(CLAUDE_JSON_MAX_BYTES + 1)
            .unwrap();
        let found = scan(&home.roots(None));
        assert!(found.sources.is_empty());
        assert!(found.claude_json_oversized);
    }

    #[test]
    fn a_plugin_manifest_cannot_point_outside_its_plugin() {
        let home = Home::new();
        home.write(
            "home/.claude/plugins/installed_plugins.json",
            &serde_json::json!({"plugins": {
                "up@m": [{"scope": "user", "installPath": home.path("plugins/up")}],
                "abs@m": [{"scope": "user", "installPath": home.path("plugins/abs")}]
            }})
            .to_string(),
        );
        home.write(
            "home/.claude/settings.json",
            r#"{"enabledPlugins":{"up@m":true,"abs@m":true}}"#,
        );
        let outside = home.write("secret.json", r#"{"mcpServers":{"leaked":{}}}"#);
        home.write(
            "plugins/up/.claude-plugin/plugin.json",
            r#"{"mcpServers":"../../secret.json"}"#,
        );
        home.write(
            "plugins/abs/.claude-plugin/plugin.json",
            &serde_json::json!({"mcpServers": outside}).to_string(),
        );
        let found = scan(&home.roots(None));
        assert!(
            found
                .sources
                .iter()
                .all(|s| s.kind != SourceKind::McpServer)
        );
    }

    #[test]
    fn unreadable_or_malformed_config_is_skipped() {
        let home = Home::new();
        home.write("home/.claude.json", "{not json");
        home.write("work/app/.mcp.json", r#"{"mcpServers":[1,2]}"#);
        let found = scan(&home.roots(Some("work/app")));
        assert!(found.sources.is_empty());
    }
}
