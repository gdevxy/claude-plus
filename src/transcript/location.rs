use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Location {
    pub project: String,
    pub session_hint: String,
    pub agent_id: Option<String>,
}

impl Location {
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
        match subagents_dir {
            Some(subagents_dir) => {
                let session_dir = subagents_dir.parent();
                Self {
                    project: name(session_dir.and_then(Path::parent)),
                    session_hint: name(session_dir),
                    agent_id: Some(stem.trim_start_matches("agent-").to_string()),
                }
            }
            None => Self {
                project: name(parent),
                session_hint: stem,
                agent_id: None,
            },
        }
    }
}

pub fn is_workflow_agent_dir(run_dir: Option<&Path>) -> bool {
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
        let location = Location::from_path(Path::new("/h/projects/proj/sess-1.jsonl"));
        assert_eq!(location.project, "proj");
        assert_eq!(location.session_hint, "sess-1");
        assert_eq!(location.agent_id, None);
    }

    #[test]
    fn subagent_file_yields_parent_session_and_agent_id() {
        let location = Location::from_path(Path::new(
            "/h/projects/proj/sess-1/subagents/agent-abc.jsonl",
        ));
        assert_eq!(location.project, "proj");
        assert_eq!(location.session_hint, "sess-1");
        assert_eq!(location.agent_id.as_deref(), Some("abc"));
    }

    #[test]
    fn workflow_agent_file_yields_parent_session_and_agent_id() {
        let location = Location::from_path(Path::new(
            "/h/projects/proj/sess-1/subagents/workflows/wf_1/agent-w1.jsonl",
        ));
        assert_eq!(location.project, "proj");
        assert_eq!(location.session_hint, "sess-1");
        assert_eq!(location.agent_id.as_deref(), Some("w1"));
    }
}
