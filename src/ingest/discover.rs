use std::path::{Path, PathBuf};

use crate::transcript::location::{Location, is_workflow_agent_dir};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptFile {
    pub path: PathBuf,
    pub location: Location,
}

impl TranscriptFile {
    pub fn from_path(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            location: Location::from_path(path),
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
        .filter(|entry| is_transcript(projects_dir, entry.path()))
        .map(|entry| TranscriptFile::from_path(entry.path()))
        .collect();
    files.sort_by(|a, b| a.path.as_os_str().cmp(b.path.as_os_str()));
    Ok(files)
}

pub fn is_transcript(projects_dir: &Path, path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "jsonl") && is_session_file(projects_dir, path)
}

fn is_session_file(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let depth = relative.components().count();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcripts_are_recognised_by_place_and_extension() {
        let root = Path::new("/h/projects");
        for path in [
            "/h/projects/p/s.jsonl",
            "/h/projects/p/s/subagents/agent-a.jsonl",
            "/h/projects/p/s/subagents/workflows/wf/agent-w.jsonl",
        ] {
            assert!(is_transcript(root, Path::new(path)), "{path}");
        }
        for path in [
            "/h/projects/p/s.json",
            "/h/projects/s.jsonl",
            "/h/projects/p/s/subagents/workflows/wf/journal.jsonl",
            "/h/projects/p/s/other/agent-a.jsonl",
            "/elsewhere/p/s.jsonl",
        ] {
            assert!(!is_transcript(root, Path::new(path)), "{path}");
        }
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
            .map(|file| (file.location.session_hint, file.location.agent_id))
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
