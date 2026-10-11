mod extensions;
mod frontmatter;
mod instructions;
mod mcp;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use frontmatter::frontmatter;

pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    pub claude_dir: PathBuf,
    pub claude_json: PathBuf,
    pub project: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    ClaudeMd,
    Memory,
    Skill,
    Agent,
    OutputStyle,
    McpServer,
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    User,
    Project,
    Local,
    Nested,
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Loading {
    Always,
    Listing,
    OnDemand,
    Off,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub kind: SourceKind,
    pub scope: Scope,
    pub name: String,
    pub path: Option<String>,
    pub plugin: Option<String>,
    pub loading: Loading,
    pub chars: u64,
    pub file_chars: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryDir {
    pub project: String,
    pub path: String,
    pub files: u64,
    pub chars: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scan {
    pub sources: Vec<Source>,
    pub memory_dirs: Vec<MemoryDir>,
}

pub fn scan(roots: &Roots) -> Scan {
    let mut scan = Scan::default();
    let mut seen = Seen::default();
    instructions::claude_md(roots, &mut seen, &mut scan.sources);
    instructions::memory(roots, &mut seen, &mut scan);
    let settings = Settings::read(roots);
    let plugins = extensions::plugins(roots, &settings, &mut scan.sources);
    extensions::skills(roots, &plugins, &mut seen, &mut scan.sources);
    extensions::agents(roots, &plugins, &mut seen, &mut scan.sources);
    extensions::output_styles(roots, &plugins, &settings, &mut seen, &mut scan.sources);
    mcp::servers(roots, &plugins, &mut scan.sources);
    scan
}

#[derive(Debug, Default)]
pub(crate) struct Seen(HashSet<PathBuf>);

impl Seen {
    pub fn first(&mut self, path: &Path) -> bool {
        self.0
            .insert(std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Settings {
    pub enabled_plugins: Vec<(String, bool)>,
    pub output_style: Option<String>,
}

impl Settings {
    fn read(roots: &Roots) -> Self {
        let mut files = vec![roots.claude_dir.join("settings.json")];
        if let Some(project) = &roots.project {
            files.push(project.join(".claude/settings.json"));
            files.push(project.join(".claude/settings.local.json"));
        }
        let mut settings = Self::default();
        for value in files.iter().filter_map(|file| read_json(file)) {
            if let Some(plugins) = value.get("enabledPlugins").and_then(Value::as_object) {
                for (key, enabled) in plugins {
                    let enabled = enabled.as_bool().unwrap_or(false);
                    settings.enabled_plugins.retain(|(known, _)| known != key);
                    settings.enabled_plugins.push((key.clone(), enabled));
                }
            }
            if let Some(style) = value.get("outputStyle").and_then(Value::as_str) {
                settings.output_style = Some(style.to_string());
            }
        }
        settings
    }
}

pub(crate) struct Text {
    pub content: Option<String>,
    pub chars: u64,
}

pub(crate) fn read_text(path: &Path) -> Option<Text> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Some(Text {
            content: None,
            chars: metadata.len(),
        });
    }
    let bytes = std::fs::read(path).ok()?;
    let content = String::from_utf8_lossy(&bytes).into_owned();
    Some(Text {
        chars: content.chars().count() as u64,
        content: Some(content),
    })
}

pub(crate) fn read_json(path: &Path) -> Option<Value> {
    let text = read_text(path)?.content?;
    serde_json::from_str(&text).ok()
}

pub(crate) fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    paths.sort();
    paths
}

pub(crate) fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "md") && path.is_file()
}

pub(crate) fn display(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
pub(crate) mod fixture {
    use std::path::{Path, PathBuf};

    use super::Roots;

    pub struct Home {
        pub temp: tempfile::TempDir,
    }

    impl Home {
        pub fn new() -> Self {
            Self {
                temp: tempfile::tempdir().unwrap(),
            }
        }

        pub fn path(&self, relative: &str) -> PathBuf {
            self.temp.path().join(relative)
        }

        pub fn write(&self, relative: &str, content: &str) -> PathBuf {
            let path = self.path(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, content).unwrap();
            path
        }

        pub fn roots(&self, project: Option<&str>) -> Roots {
            Roots {
                claude_dir: self.path("home/.claude"),
                claude_json: self.path("home/.claude.json"),
                project: project.map(|p| self.path(p)),
            }
        }
    }

    pub fn shown(path: &Path) -> String {
        path.display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::Home;
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_read_only_claude_dir_is_scanned_and_left_untouched() {
        use std::os::unix::fs::PermissionsExt;
        let home = Home::new();
        home.write("home/.claude/CLAUDE.md", "rules");
        home.write(
            "home/.claude/skills/tidy/SKILL.md",
            "---\nname: tidy\ndescription: Tidy up\n---\nbody",
        );
        home.write("home/.claude/settings.json", r#"{"outputStyle":"Plain"}"#);
        let claude = home.path("home/.claude");
        let before: Vec<_> = walkdir::WalkDir::new(&claude)
            .into_iter()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.path().to_path_buf(),
                    e.metadata().unwrap().modified().unwrap(),
                )
            })
            .collect();
        for (path, _) in &before {
            let mode = if path.is_dir() { 0o500 } else { 0o400 };
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let scan = scan(&home.roots(None));
        for (path, _) in before.iter().rev() {
            let mode = if path.is_dir() { 0o700 } else { 0o600 };
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let after: Vec<_> = walkdir::WalkDir::new(&claude)
            .into_iter()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.path().to_path_buf(),
                    e.metadata().unwrap().modified().unwrap(),
                )
            })
            .collect();
        assert_eq!(after, before);
        assert_eq!(
            scan.sources
                .iter()
                .map(|s| (s.kind, s.name.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (SourceKind::ClaudeMd, "CLAUDE.md"),
                (SourceKind::Skill, "tidy")
            ]
        );
    }

    #[test]
    fn a_project_at_the_home_directory_counts_every_source_once() {
        let home = Home::new();
        home.write("home/.claude/CLAUDE.md", "rules");
        home.write("home/.claude/rules/style.md", "be terse");
        home.write(
            "home/.claude/skills/tidy/SKILL.md",
            "---\nname: tidy\ndescription: Tidy up\n---\nbody",
        );
        home.write(
            "home/.claude/agents/reviewer.md",
            "---\nname: reviewer\ndescription: Reviews\n---\n",
        );
        home.write("home/.claude/output-styles/terse.md", "Be terse.");
        let found = scan(&home.roots(Some("home")));
        let mut names: Vec<(SourceKind, &str)> = found
            .sources
            .iter()
            .map(|s| (s.kind, s.name.as_str()))
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                (SourceKind::ClaudeMd, "CLAUDE.md"),
                (SourceKind::ClaudeMd, "rules/style.md"),
                (SourceKind::Skill, "tidy"),
                (SourceKind::Agent, "reviewer"),
                (SourceKind::OutputStyle, "terse"),
            ]
        );
        assert!(found.sources.iter().all(|s| s.scope == Scope::User));
    }

    #[test]
    fn an_empty_home_scans_to_nothing() {
        let home = Home::new();
        assert_eq!(scan(&home.roots(Some("work/app"))), Scan::default());
    }

    #[test]
    fn oversized_files_count_their_bytes_without_being_read() {
        let home = Home::new();
        let path = home.write("big.md", "");
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        let text = read_text(&path).unwrap();
        assert_eq!(
            (text.content.is_none(), text.chars),
            (true, MAX_FILE_BYTES + 1)
        );
        assert!(read_text(&home.path("missing.md")).is_none());
        assert!(read_text(home.temp.path()).is_none());
    }

    #[test]
    fn settings_merge_user_then_project_then_local() {
        let home = Home::new();
        home.write(
            "home/.claude/settings.json",
            r#"{"enabledPlugins":{"a@m":true,"b@m":true},"outputStyle":"Explanatory"}"#,
        );
        home.write(
            "work/app/.claude/settings.json",
            r#"{"enabledPlugins":{"b@m":false}}"#,
        );
        home.write(
            "work/app/.claude/settings.local.json",
            r#"{"outputStyle":"Plain","enabledPlugins":{"c@m":"yes"}}"#,
        );
        let settings = Settings::read(&home.roots(Some("work/app")));
        assert_eq!(
            settings.enabled_plugins,
            vec![
                ("a@m".to_string(), true),
                ("b@m".to_string(), false),
                ("c@m".to_string(), false)
            ]
        );
        assert_eq!(settings.output_style.as_deref(), Some("Plain"));
    }
}
