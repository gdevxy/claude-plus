use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{
    Loading, MemoryDir, Roots, Scan, Scope, Source, SourceKind, display, is_markdown, read_text,
    sorted_entries,
};

pub const MEMORY_INDEX: &str = "MEMORY.md";
pub const MEMORY_INDEX_LINES: usize = 200;
pub const NESTED_MAX_DEPTH: usize = 6;
pub const NESTED_MAX_ENTRIES: usize = 20_000;
pub const RULES_MAX_DEPTH: usize = 4;
const SKIPPED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "vendor",
    "dist",
    "build",
    "__pycache__",
];
const NESTED_NAMES: &[&str] = &["CLAUDE.md", "CLAUDE.local.md"];

pub fn claude_md(roots: &Roots, sources: &mut Vec<Source>) {
    let mut seen = HashSet::new();
    let mut add = |scope: Scope, name: String, path: &Path, loading: Loading| {
        if !seen.insert(identity(path)) {
            return;
        }
        if let Some(text) = read_text(path) {
            sources.push(Source {
                kind: SourceKind::ClaudeMd,
                scope,
                name,
                path: Some(display(path)),
                plugin: None,
                loading,
                chars: text.chars,
                file_chars: text.chars,
            });
        }
    };
    let user = &roots.claude_dir;
    add(
        Scope::User,
        "CLAUDE.md".into(),
        &user.join("CLAUDE.md"),
        Loading::Always,
    );
    for rule in rules(&user.join("rules")) {
        add(
            Scope::User,
            relative(&rule, user, 0),
            &rule,
            Loading::Always,
        );
    }
    let Some(project) = &roots.project else {
        return;
    };
    for (depth, dir) in project.ancestors().enumerate() {
        for (scope, file) in [
            (Scope::Project, "CLAUDE.md"),
            (Scope::Project, ".claude/CLAUDE.md"),
            (Scope::Local, "CLAUDE.local.md"),
        ] {
            let path = dir.join(file);
            add(scope, relative(&path, dir, depth), &path, Loading::Always);
        }
        for rule in rules(&dir.join(".claude/rules")) {
            add(
                Scope::Project,
                relative(&rule, dir, depth),
                &rule,
                Loading::Always,
            );
        }
    }
    for path in nested(project) {
        add(
            Scope::Nested,
            relative(&path, project, 0),
            &path,
            Loading::OnDemand,
        );
    }
}

pub fn memory(roots: &Roots, scan: &mut Scan) {
    let projects = roots.claude_dir.join("projects");
    for project_dir in sorted_entries(&projects) {
        let dir = project_dir.join("memory");
        let files: Vec<PathBuf> = sorted_entries(&dir)
            .into_iter()
            .filter(|p| is_markdown(p))
            .collect();
        if files.is_empty() {
            continue;
        }
        let chars = files
            .iter()
            .filter_map(|file| read_text(file))
            .map(|text| text.chars)
            .sum();
        scan.memory_dirs.push(MemoryDir {
            project: name_of(&project_dir),
            path: display(&dir),
            files: files.len() as u64,
            chars,
        });
    }
    let Some(project) = &roots.project else {
        return;
    };
    let dir = projects.join(project_key(project)).join("memory");
    for file in sorted_entries(&dir).into_iter().filter(|p| is_markdown(p)) {
        let Some(text) = read_text(&file) else {
            continue;
        };
        let name = name_of(&file);
        let (loading, chars) = if name == MEMORY_INDEX {
            (
                Loading::Always,
                index_chars(text.content.as_deref(), text.chars),
            )
        } else {
            (Loading::OnDemand, text.chars)
        };
        scan.sources.push(Source {
            kind: SourceKind::Memory,
            scope: Scope::Project,
            name,
            path: Some(display(&file)),
            plugin: None,
            loading,
            chars,
            file_chars: text.chars,
        });
    }
}

pub fn project_key(project: &Path) -> String {
    project
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn index_chars(content: Option<&str>, chars: u64) -> u64 {
    match content {
        Some(content) => content
            .split_inclusive('\n')
            .take(MEMORY_INDEX_LINES)
            .map(|line| line.chars().count() as u64)
            .sum(),
        None => chars,
    }
}

fn rules(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = walkdir::WalkDir::new(dir)
        .max_depth(RULES_MAX_DEPTH)
        .into_iter()
        .filter_map(Result::ok)
        .map(walkdir::DirEntry::into_path)
        .filter(|path| is_markdown(path))
        .collect();
    found.sort();
    found
}

fn nested(project: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = walkdir::WalkDir::new(project)
        .min_depth(1)
        .max_depth(NESTED_MAX_DEPTH)
        .into_iter()
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !(entry.file_type().is_dir()
                && (name.starts_with('.') || SKIPPED_DIRS.contains(&name.as_ref())))
        })
        .take(NESTED_MAX_ENTRIES)
        .filter_map(Result::ok)
        .filter(|entry| entry.depth() >= 2 && entry.file_type().is_file())
        .filter(|entry| NESTED_NAMES.contains(&entry.file_name().to_string_lossy().as_ref()))
        .map(walkdir::DirEntry::into_path)
        .collect();
    found.sort();
    found
}

fn relative(path: &Path, base: &Path, depth: usize) -> String {
    let inner = path
        .strip_prefix(base)
        .unwrap_or(path)
        .display()
        .to_string();
    format!("{}{inner}", "../".repeat(depth))
}

fn identity(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::super::fixture::{Home, shown};
    use super::*;

    fn rows<'a>(home: &Home, sources: &'a [Source]) -> Vec<(Scope, &'a str, Loading, u64)> {
        let inside = shown(home.temp.path());
        sources
            .iter()
            .filter(|s| s.path.as_deref().is_some_and(|p| p.starts_with(&inside)))
            .map(|s| (s.scope, s.name.as_str(), s.loading, s.chars))
            .collect()
    }

    #[test]
    fn the_claude_md_chain_runs_from_the_user_up_the_project_and_into_nested_dirs() {
        let home = Home::new();
        home.write("home/.claude/CLAUDE.md", "user rules");
        home.write("home/.claude/rules/style.md", "be terse");
        home.write("work/CLAUDE.md", "monorepo");
        home.write("work/app/CLAUDE.md", "app rules");
        home.write("work/app/.claude/CLAUDE.md", "hidden");
        home.write("work/app/CLAUDE.local.md", "mine");
        home.write("work/app/.claude/rules/tests.md", "tdd");
        home.write("work/app/src/api/CLAUDE.md", "api notes");
        home.write("work/app/node_modules/x/CLAUDE.md", "skipped");
        home.write("work/app/.git/CLAUDE.md", "skipped");
        home.write("work/app/a/b/c/d/e/f/CLAUDE.md", "too deep");
        let mut sources = Vec::new();
        claude_md(&home.roots(Some("work/app")), &mut sources);
        let found = rows(&home, &sources);
        let expected = vec![
            (Scope::User, "CLAUDE.md", Loading::Always, 10),
            (Scope::User, "rules/style.md", Loading::Always, 8),
            (Scope::Project, "CLAUDE.md", Loading::Always, 9),
            (Scope::Project, ".claude/CLAUDE.md", Loading::Always, 6),
            (Scope::Local, "CLAUDE.local.md", Loading::Always, 4),
            (Scope::Project, ".claude/rules/tests.md", Loading::Always, 3),
            (Scope::Project, "../CLAUDE.md", Loading::Always, 8),
            (Scope::Nested, "src/api/CLAUDE.md", Loading::OnDemand, 9),
        ];
        assert_eq!(found, expected);
        assert_eq!(
            sources[0].path.as_deref(),
            Some(shown(&home.path("home/.claude/CLAUDE.md")).as_str())
        );
    }

    #[test]
    fn the_home_directory_as_an_ancestor_does_not_count_the_user_file_twice() {
        let home = Home::new();
        home.write("home/.claude/CLAUDE.md", "user rules");
        home.write("home/work/CLAUDE.md", "project");
        let mut sources = Vec::new();
        claude_md(&home.roots(Some("home/work")), &mut sources);
        assert_eq!(
            rows(&home, &sources),
            vec![
                (Scope::User, "CLAUDE.md", Loading::Always, 10),
                (Scope::Project, "CLAUDE.md", Loading::Always, 7),
            ]
        );
    }

    #[test]
    fn memory_loads_the_first_200_lines_of_its_index_and_the_rest_on_demand() {
        let home = Home::new();
        let project = home.path("work/app");
        let key = project_key(&project);
        let index: String = (0..250).map(|i| format!("- note {i:03}\n")).collect();
        home.write(
            &format!("home/.claude/projects/{key}/memory/MEMORY.md"),
            &index,
        );
        home.write(
            &format!("home/.claude/projects/{key}/memory/kata.md"),
            "kata notes",
        );
        home.write("home/.claude/projects/-other/memory/MEMORY.md", "elsewhere");
        home.write(
            "home/.claude/projects/-other/memory/notes.txt",
            "not markdown",
        );
        home.write("home/.claude/projects/-empty/memory/.keep", "");
        let mut scan = Scan::default();
        memory(&home.roots(Some("work/app")), &mut scan);
        assert_eq!(
            rows(&home, &scan.sources),
            vec![
                (Scope::Project, "MEMORY.md", Loading::Always, 200 * 11),
                (Scope::Project, "kata.md", Loading::OnDemand, 10),
            ]
        );
        assert_eq!(scan.sources[0].file_chars, 250 * 11);
        let dirs: Vec<(&str, u64, u64)> = scan
            .memory_dirs
            .iter()
            .map(|d| (d.project.as_str(), d.files, d.chars))
            .collect();
        assert_eq!(
            dirs,
            vec![("-other", 1, 9), (key.as_str(), 2, 250 * 11 + 10)]
        );
    }

    #[test]
    fn a_project_key_replaces_every_other_character_with_a_dash() {
        assert_eq!(
            project_key(Path::new("/Users/ada.l/my_app")),
            "-Users-ada-l-my-app"
        );
    }
}
