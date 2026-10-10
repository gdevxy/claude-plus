use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptFile {
    pub path: PathBuf,
    pub project: String,
    pub session_hint: String,
    pub agent_id: Option<String>,
}

impl TranscriptFile {
    pub fn from_path(path: &Path) -> Self {
        let name = |p: Option<&Path>| {
            p.and_then(Path::file_name)
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parent = path.parent();
        let subagents_dir = if name(parent) == "subagents" {
            parent
        } else if is_workflow_agent_dir(parent) {
            parent.and_then(Path::parent).and_then(Path::parent)
        } else {
            None
        };
        if let Some(subagents_dir) = subagents_dir {
            let session_dir = subagents_dir.parent();
            Self {
                path: path.to_path_buf(),
                project: name(session_dir.and_then(Path::parent)),
                session_hint: name(session_dir),
                agent_id: Some(stem.trim_start_matches("agent-").to_string()),
            }
        } else {
            Self {
                path: path.to_path_buf(),
                project: name(parent),
                session_hint: stem,
                agent_id: None,
            }
        }
    }
}

pub fn discover(projects_dir: &Path) -> anyhow::Result<Vec<TranscriptFile>> {
    if !projects_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files: Vec<TranscriptFile> = walkdir::WalkDir::new(projects_dir)
        .min_depth(2)
        .max_depth(6)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jsonl"))
        .filter(|entry| is_session_file(projects_dir, entry.path()))
        .map(|entry| TranscriptFile::from_path(entry.path()))
        .collect();
    files.sort_by(|a, b| a.path.as_os_str().cmp(b.path.as_os_str()));
    Ok(files)
}

fn is_session_file(root: &Path, path: &Path) -> bool {
    let depth = path
        .strip_prefix(root)
        .map(|p| p.components().count())
        .unwrap_or(0);
    let parent = path.parent();
    let in_subagents = parent
        .and_then(Path::file_name)
        .is_some_and(|n| n == "subagents");
    let is_agent = path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with("agent-"));
    depth == 2
        || (depth == 4 && in_subagents)
        || (depth == 6 && is_agent && is_workflow_agent_dir(parent))
}

fn is_workflow_agent_dir(run_dir: Option<&Path>) -> bool {
    let workflows = run_dir.and_then(Path::parent);
    let subagents = workflows.and_then(Path::parent);
    workflows
        .and_then(Path::file_name)
        .is_some_and(|n| n == "workflows")
        && subagents
            .and_then(Path::file_name)
            .is_some_and(|n| n == "subagents")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_file_yields_project_and_session() {
        let file = TranscriptFile::from_path(Path::new("/h/projects/proj/sess-1.jsonl"));
        assert_eq!(file.project, "proj");
        assert_eq!(file.session_hint, "sess-1");
        assert_eq!(file.agent_id, None);
    }

    #[test]
    fn subagent_file_yields_parent_session_and_agent_id() {
        let file = TranscriptFile::from_path(Path::new(
            "/h/projects/proj/sess-1/subagents/agent-abc.jsonl",
        ));
        assert_eq!(file.project, "proj");
        assert_eq!(file.session_hint, "sess-1");
        assert_eq!(file.agent_id.as_deref(), Some("abc"));
    }

    #[test]
    fn workflow_agent_file_yields_parent_session_and_agent_id() {
        let file = TranscriptFile::from_path(Path::new(
            "/h/projects/proj/sess-1/subagents/workflows/wf_1/agent-w1.jsonl",
        ));
        assert_eq!(file.project, "proj");
        assert_eq!(file.session_hint, "sess-1");
        assert_eq!(file.agent_id.as_deref(), Some("w1"));
    }

    #[test]
    fn discovers_workflow_agents_but_not_journals() {
        let temp = tempfile::tempdir().unwrap();
        let projects = temp.path();
        let run = projects.join("proj/sess-1/subagents/workflows/wf_1");
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(projects.join("proj/sess-1.jsonl"), "").unwrap();
        std::fs::write(run.join("agent-w1.jsonl"), "").unwrap();
        std::fs::write(run.join("journal.jsonl"), "").unwrap();
        let found: Vec<_> = discover(projects)
            .unwrap()
            .into_iter()
            .map(|file| (file.session_hint, file.agent_id))
            .collect();
        assert_eq!(
            found,
            vec![
                ("sess-1".to_string(), None),
                ("sess-1".to_string(), Some("w1".to_string()))
            ]
        );
    }
}
