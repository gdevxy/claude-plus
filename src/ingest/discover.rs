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
        if name(parent) == "subagents" {
            let session_dir = parent.and_then(Path::parent);
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
}
