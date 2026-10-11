use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    Loading, Roots, Scope, Seen, Settings, Source, SourceKind, display, frontmatter, is_markdown,
    read_json, read_text, sorted_entries,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plugin {
    pub short: String,
    pub dir: PathBuf,
}

pub fn plugins(roots: &Roots, settings: &Settings, sources: &mut Vec<Source>) -> Vec<Plugin> {
    let installed = read_json(&roots.claude_dir.join("plugins/installed_plugins.json"));
    let mut wanted = settings.enabled_plugins.clone();
    wanted.sort();
    let mut enabled = Vec::new();
    for (key, is_enabled) in wanted {
        let Some((scope, dir)) = installed
            .as_ref()
            .and_then(|installed| install_of(installed, &key, roots.project.as_deref()))
        else {
            continue;
        };
        let short = key.split('@').next().unwrap_or(&key).to_string();
        sources.push(Source {
            kind: SourceKind::Plugin,
            scope,
            name: key.clone(),
            alias: None,
            path: Some(display(&dir)),
            plugin: Some(short.clone()),
            loading: if is_enabled {
                Loading::Always
            } else {
                Loading::Off
            },
            chars: 0,
            file_chars: 0,
        });
        if is_enabled {
            enabled.push(Plugin { short, dir });
        }
    }
    enabled
}

pub fn skills(roots: &Roots, plugins: &[Plugin], seen: &mut Seen, sources: &mut Vec<Source>) {
    let mut add = |scope: Scope, dir: &Path, plugin: Option<&str>| {
        for skill_dir in sorted_entries(dir) {
            let file = skill_dir.join("SKILL.md");
            if !file.is_file() || !seen.first(&file) {
                continue;
            }
            let Some(text) = read_text(&file) else {
                continue;
            };
            let fields = frontmatter(text.content.as_deref().unwrap_or(""));
            let name = qualified(plugin, &file_name(&skill_dir));
            let alias = fields
                .get("name")
                .map(|own| qualified(plugin, own))
                .filter(|alias| *alias != name);
            let listed = fields
                .get("disable-model-invocation")
                .is_none_or(|value| value != "true");
            let (loading, chars) = if listed {
                let listed_as = match &alias {
                    Some(alias) => format!("{name} ({alias})"),
                    None => name.clone(),
                };
                (
                    Loading::Listing,
                    listing_chars(&listed_as, fields.get("description")),
                )
            } else {
                (Loading::OnDemand, text.chars)
            };
            sources.push(Source {
                kind: SourceKind::Skill,
                scope,
                name,
                alias,
                path: Some(display(&file)),
                plugin: plugin.map(String::from),
                loading,
                chars,
                file_chars: text.chars,
            });
        }
    };
    add(Scope::User, &roots.claude_dir.join("skills"), None);
    if let Some(project) = &roots.project {
        add(Scope::Project, &project.join(".claude/skills"), None);
    }
    for plugin in plugins {
        add(
            Scope::Plugin,
            &plugin.dir.join("skills"),
            Some(&plugin.short),
        );
    }
}

pub fn agents(roots: &Roots, plugins: &[Plugin], seen: &mut Seen, sources: &mut Vec<Source>) {
    for (scope, dir, plugin) in definition_dirs(roots, plugins, "agents") {
        for file in definitions(&dir, seen) {
            let Some(text) = read_text(&file) else {
                continue;
            };
            let fields = frontmatter(text.content.as_deref().unwrap_or(""));
            let own = fields.get("name").cloned().unwrap_or_else(|| stem(&file));
            let name = qualified(plugin.as_deref(), &own);
            sources.push(Source {
                kind: SourceKind::Agent,
                scope,
                chars: listing_chars(&name, fields.get("description")),
                name,
                alias: None,
                path: Some(display(&file)),
                plugin: plugin.clone(),
                loading: Loading::Listing,
                file_chars: text.chars,
            });
        }
    }
}

pub fn output_styles(
    roots: &Roots,
    plugins: &[Plugin],
    settings: &Settings,
    seen: &mut Seen,
    sources: &mut Vec<Source>,
) {
    for (scope, dir, plugin) in definition_dirs(roots, plugins, "output-styles") {
        for file in definitions(&dir, seen) {
            let Some(text) = read_text(&file) else {
                continue;
            };
            let fields = frontmatter(text.content.as_deref().unwrap_or(""));
            let own = fields.get("name").cloned().unwrap_or_else(|| stem(&file));
            let name = qualified(plugin.as_deref(), &own);
            let active = settings.output_style.as_deref().is_some_and(|style| {
                style.eq_ignore_ascii_case(&name) || style.eq_ignore_ascii_case(&own)
            });
            sources.push(Source {
                kind: SourceKind::OutputStyle,
                scope,
                name,
                alias: None,
                path: Some(display(&file)),
                plugin: plugin.clone(),
                loading: if active {
                    Loading::Always
                } else {
                    Loading::Off
                },
                chars: text.chars,
                file_chars: text.chars,
            });
        }
    }
}

fn definitions(dir: &Path, seen: &mut Seen) -> Vec<PathBuf> {
    sorted_entries(dir)
        .into_iter()
        .filter(|path| is_markdown(path) && seen.first(path))
        .collect()
}

fn definition_dirs(
    roots: &Roots,
    plugins: &[Plugin],
    folder: &str,
) -> Vec<(Scope, PathBuf, Option<String>)> {
    let mut dirs = vec![(Scope::User, roots.claude_dir.join(folder), None)];
    if let Some(project) = &roots.project {
        dirs.push((Scope::Project, project.join(".claude").join(folder), None));
    }
    for plugin in plugins {
        dirs.push((
            Scope::Plugin,
            plugin.dir.join(folder),
            Some(plugin.short.clone()),
        ));
    }
    dirs
}

fn install_of(installed: &Value, key: &str, project: Option<&Path>) -> Option<(Scope, PathBuf)> {
    installed
        .pointer("/plugins")?
        .get(key)?
        .as_array()?
        .iter()
        .find_map(|entry| {
            let dir = PathBuf::from(entry.get("installPath")?.as_str()?);
            let scope = match entry.get("scope").and_then(Value::as_str) {
                Some("user") | None => Scope::User,
                Some(other) => {
                    let at = entry.get("projectPath").and_then(Value::as_str)?;
                    if project != Some(Path::new(at)) {
                        return None;
                    }
                    if other == "local" {
                        Scope::Local
                    } else {
                        Scope::Project
                    }
                }
            };
            Some((scope, dir))
        })
}

fn listing_chars(name: &str, description: Option<&String>) -> u64 {
    let line = format!(
        "- {name}: {}",
        description.map(String::as_str).unwrap_or("")
    );
    line.chars().count() as u64
}

fn qualified(plugin: Option<&str>, name: &str) -> String {
    match plugin {
        Some(plugin) => format!("{plugin}:{name}"),
        None => name.to_string(),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::super::fixture::Home;
    use super::super::scan;
    use super::*;

    fn rows(sources: &[Source], kind: SourceKind) -> Vec<(Scope, &str, Loading, u64)> {
        sources
            .iter()
            .filter(|s| s.kind == kind)
            .map(|s| (s.scope, s.name.as_str(), s.loading, s.chars))
            .collect()
    }

    fn installed(home: &Home) {
        let json = serde_json::json!({"version": 2, "plugins": {
            "superpowers@official": [{"scope": "user", "installPath": home.path("plugins/superpowers")}],
            "kata@official": [{"scope": "local", "projectPath": home.path("work/other"), "installPath": home.path("plugins/kata-other")},
                              {"scope": "local", "projectPath": home.path("work/app"), "installPath": home.path("plugins/kata")}],
            "lint@official": [{"scope": "project", "projectPath": home.path("work/other"), "installPath": home.path("plugins/lint")}],
            "off@official": [{"scope": "user", "installPath": home.path("plugins/off")}]
        }});
        home.write(
            "home/.claude/plugins/installed_plugins.json",
            &json.to_string(),
        );
        home.write(
            "home/.claude/settings.json",
            r#"{"enabledPlugins":{"superpowers@official":true,"kata@official":true,"lint@official":true,"off@official":false,"ghost@official":true},"outputStyle":"Terse"}"#,
        );
    }

    #[test]
    fn enabled_plugins_installed_for_this_project_are_found() {
        let home = Home::new();
        installed(&home);
        let found = scan(&home.roots(Some("work/app")));
        assert_eq!(
            rows(&found.sources, SourceKind::Plugin),
            vec![
                (Scope::Local, "kata@official", Loading::Always, 0),
                (Scope::User, "off@official", Loading::Off, 0),
                (Scope::User, "superpowers@official", Loading::Always, 0),
            ]
        );
        assert_eq!(
            found.sources[0].path.as_deref(),
            Some(home.path("plugins/kata").display().to_string().as_str())
        );
    }

    #[test]
    fn skills_are_listed_by_their_directory_and_measured_whole() {
        let home = Home::new();
        installed(&home);
        home.write(
            "home/.claude/skills/tidy/SKILL.md",
            "---\nname: tidy\ndescription: Tidy up the workspace\n---\nLong body here",
        );
        home.write("home/.claude/skills/nameless/SKILL.md", "no frontmatter");
        home.write(
            "home/.claude/skills/manual/SKILL.md",
            "---\nname: manual\ndescription: x\ndisable-model-invocation: true\n---\n",
        );
        home.write("home/.claude/skills/empty-dir/README.md", "not a skill");
        home.write(
            "work/app/.claude/skills/deploy/SKILL.md",
            "---\ndescription: Ship it\n---\n",
        );
        home.write(
            "plugins/superpowers/skills/brainstorming/SKILL.md",
            "---\nname: brainstorming\ndescription: \"Think first\"\n---\nbody",
        );
        home.write(
            "plugins/off/skills/hidden/SKILL.md",
            "---\nname: hidden\ndescription: never\n---\n",
        );
        let found = scan(&home.roots(Some("work/app")));
        assert_eq!(
            rows(&found.sources, SourceKind::Skill),
            vec![
                (Scope::User, "manual", Loading::OnDemand, 67),
                (Scope::User, "nameless", Loading::Listing, 12),
                (Scope::User, "tidy", Loading::Listing, 29),
                (Scope::Project, "deploy", Loading::Listing, 17),
                (
                    Scope::Plugin,
                    "superpowers:brainstorming",
                    Loading::Listing,
                    40
                ),
            ]
        );
        let tidy = found.sources.iter().find(|s| s.name == "tidy").unwrap();
        assert_eq!(tidy.file_chars, 68);
        let plugin = found
            .sources
            .iter()
            .find(|s| s.name.starts_with("superpowers:"))
            .unwrap();
        assert_eq!(plugin.plugin.as_deref(), Some("superpowers"));
    }

    #[test]
    fn a_skill_is_named_by_its_directory_as_claude_code_lists_it() {
        let home = Home::new();
        installed(&home);
        home.write(
            "home/.claude/skills/spring-boot-4-migration-skill/SKILL.md",
            "---\nname: migrating-to-spring-boot-4\ndescription: Migrate\n---\n",
        );
        home.write(
            "plugins/superpowers/skills/billing-history/SKILL.md",
            "---\nname: paddle-billing-history\ndescription: Bills\n---\n",
        );
        let found = scan(&home.roots(Some("work/app")));
        let skills: Vec<(&str, Option<&str>, u64)> = found
            .sources
            .iter()
            .filter(|s| s.kind == SourceKind::Skill)
            .map(|s| (s.name.as_str(), s.alias.as_deref(), s.chars))
            .collect();
        assert_eq!(
            skills,
            vec![
                (
                    "spring-boot-4-migration-skill",
                    Some("migrating-to-spring-boot-4"),
                    "- spring-boot-4-migration-skill (migrating-to-spring-boot-4): Migrate".len()
                        as u64
                ),
                (
                    "superpowers:billing-history",
                    Some("superpowers:paddle-billing-history"),
                    "- superpowers:billing-history (superpowers:paddle-billing-history): Bills"
                        .len() as u64
                ),
            ]
        );
    }

    #[test]
    fn agents_and_output_styles_come_from_user_project_and_plugins() {
        let home = Home::new();
        installed(&home);
        home.write(
            "home/.claude/agents/reviewer.md",
            "---\nname: reviewer\ndescription: Reviews code\n---\nYou review.",
        );
        home.write(
            "plugins/kata/agents/coach.md",
            "---\ndescription: Coaches\n---\n",
        );
        home.write(
            "home/.claude/output-styles/terse.md",
            "---\nname: Terse\n---\nBe terse.",
        );
        home.write("work/app/.claude/output-styles/chatty.md", "Talk a lot.");
        let found = scan(&home.roots(Some("work/app")));
        assert_eq!(
            rows(&found.sources, SourceKind::Agent),
            vec![
                (Scope::User, "reviewer", Loading::Listing, 24),
                (Scope::Plugin, "kata:coach", Loading::Listing, 21),
            ]
        );
        assert_eq!(
            rows(&found.sources, SourceKind::OutputStyle),
            vec![
                (Scope::User, "Terse", Loading::Always, 29),
                (Scope::Project, "chatty", Loading::Off, 11),
            ]
        );
    }
}
