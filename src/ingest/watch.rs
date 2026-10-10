use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::Instant;

use super::discover::is_transcript;

pub const QUIET: Duration = Duration::from_millis(200);
pub const MAX_WAIT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub paths: BTreeSet<PathBuf>,
    pub noticed_at: std::time::Instant,
}

pub struct Changes {
    receiver: UnboundedReceiver<PathBuf>,
    root: PathBuf,
    _watcher: RecommendedWatcher,
}

impl Changes {
    pub async fn next_batch(&mut self) -> Option<Batch> {
        loop {
            let batch = debounce(&mut self.receiver, QUIET, MAX_WAIT).await?;
            let root = self.root.clone();
            let paths = tokio::task::spawn_blocking(move || expand(&root, batch.paths))
                .await
                .ok()?;
            if !paths.is_empty() {
                return Some(Batch {
                    paths,
                    noticed_at: batch.noticed_at,
                });
            }
        }
    }
}

pub fn watch(projects_dir: &Path) -> Result<Changes> {
    let canonical = std::fs::canonicalize(projects_dir)
        .with_context(|| format!("cannot watch {}", projects_dir.display()))?;
    let root = projects_dir.to_path_buf();
    let (sender, receiver) = unbounded_channel();
    let watched = canonical.clone();
    let forwarded_root = root.clone();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
        if let Ok(event) = event {
            forward(&event, &watched, &forwarded_root, &sender);
        }
    })?;
    watcher
        .watch(&canonical, RecursiveMode::Recursive)
        .with_context(|| format!("cannot watch {}", projects_dir.display()))?;
    Ok(Changes {
        receiver,
        root,
        _watcher: watcher,
    })
}

fn forward(event: &Event, canonical: &Path, root: &Path, sender: &UnboundedSender<PathBuf>) {
    if matches!(event.kind, EventKind::Access(_)) {
        return;
    }
    for path in &event.paths {
        let Some(path) = under_root(path, canonical, root) else {
            continue;
        };
        let created_dir = matches!(event.kind, EventKind::Create(_)) && path.is_dir();
        if created_dir || is_transcript(root, &path) {
            let _ = sender.send(path);
        }
    }
}

fn expand(root: &Path, paths: BTreeSet<PathBuf>) -> BTreeSet<PathBuf> {
    let mut files = BTreeSet::new();
    for path in paths {
        if path.is_dir() {
            files.extend(
                walkdir::WalkDir::new(&path)
                    .into_iter()
                    .filter_map(Result::ok)
                    .map(walkdir::DirEntry::into_path)
                    .filter(|file| is_transcript(root, file)),
            );
        } else if is_transcript(root, &path) {
            files.insert(path);
        }
    }
    files
}

fn under_root(path: &Path, canonical: &Path, root: &Path) -> Option<PathBuf> {
    if let Ok(relative) = path.strip_prefix(canonical) {
        return Some(root.join(relative));
    }
    path.strip_prefix(root).ok().map(|_| path.to_path_buf())
}

pub async fn debounce(
    receiver: &mut UnboundedReceiver<PathBuf>,
    quiet: Duration,
    max_wait: Duration,
) -> Option<Batch> {
    let first = receiver.recv().await?;
    let noticed_at = Instant::now();
    let deadline = noticed_at + max_wait;
    let mut paths = BTreeSet::from([first]);
    loop {
        let flush_at = (Instant::now() + quiet).min(deadline);
        tokio::select! {
            next = receiver.recv() => match next {
                Some(path) => {
                    paths.insert(path);
                }
                None => break,
            },
            () = tokio::time::sleep_until(flush_at) => break,
        }
    }
    Some(Batch {
        paths,
        noticed_at: noticed_at.into_std(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn path(name: &str) -> PathBuf {
        PathBuf::from(format!("/p/proj/{name}.jsonl"))
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_is_merged_once_it_goes_quiet() {
        let (sender, mut receiver) = unbounded_channel();
        for name in ["a", "b", "a"] {
            sender.send(path(name)).unwrap();
        }
        let start = Instant::now();
        let batch = debounce(&mut receiver, QUIET, MAX_WAIT).await.unwrap();
        assert_eq!(batch.paths, BTreeSet::from([path("a"), path("b")]));
        assert_eq!(start.elapsed(), QUIET);
    }

    #[tokio::test(start_paused = true)]
    async fn a_steady_stream_is_flushed_at_the_max_wait() {
        let (sender, mut receiver) = unbounded_channel();
        let writer = tokio::spawn(async move {
            for index in 0.. {
                if sender.send(path(&format!("s{index}"))).is_err() {
                    break;
                }
                tokio::time::sleep(QUIET * 3 / 4).await;
            }
        });
        let start = Instant::now();
        let batch = debounce(&mut receiver, QUIET, MAX_WAIT).await.unwrap();
        assert_eq!(start.elapsed(), MAX_WAIT);
        assert_eq!(batch.paths.len(), 7);
        drop(receiver);
        writer.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_closed_source_ends_the_stream() {
        let (sender, mut receiver) = unbounded_channel::<PathBuf>();
        drop(sender);
        assert_eq!(debounce(&mut receiver, QUIET, MAX_WAIT).await, None);
    }

    #[test]
    fn paths_reported_under_the_canonical_root_map_back_to_the_configured_root() {
        let canonical = Path::new("/private/var/t/projects");
        let root = Path::new("/var/t/projects");
        assert_eq!(
            under_root(
                Path::new("/private/var/t/projects/p/s.jsonl"),
                canonical,
                root
            ),
            Some(PathBuf::from("/var/t/projects/p/s.jsonl"))
        );
        assert_eq!(
            under_root(Path::new("/var/t/projects/p/s.jsonl"), canonical, root),
            Some(PathBuf::from("/var/t/projects/p/s.jsonl"))
        );
        assert_eq!(
            under_root(Path::new("/elsewhere/s.jsonl"), canonical, root),
            None
        );
    }

    #[test]
    fn created_directories_are_expanded_to_the_transcripts_they_hold_at_flush() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("projects");
        let session_dir = root.join("proj/s1");
        let subagent = session_dir.join("subagents/agent-a1.jsonl");
        let journal = session_dir.join("subagents/workflows/wf/journal.jsonl");
        let session = root.join("proj/s1.jsonl");
        std::fs::create_dir_all(journal.parent().unwrap()).unwrap();
        for file in [&subagent, &journal, &session] {
            std::fs::write(file, "{}\n").unwrap();
        }
        let gone = root.join("proj/gone");
        let expanded = expand(&root, BTreeSet::from([session_dir, session.clone(), gone]));
        assert_eq!(expanded, BTreeSet::from([session, subagent]));
    }

    async fn collect_until(changes: &mut Changes, wanted: &BTreeSet<PathBuf>) -> BTreeSet<PathBuf> {
        let mut seen = BTreeSet::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !wanted.is_subset(&seen) {
            match tokio::time::timeout_at(deadline, changes.next_batch()).await {
                Ok(Some(batch)) => seen.extend(batch.paths),
                _ => break,
            }
        }
        seen
    }

    #[tokio::test]
    async fn new_sessions_subagents_and_appends_are_noticed() {
        let temp = tempfile::tempdir().unwrap();
        let projects = temp.path().join("projects");
        std::fs::create_dir_all(projects.join("proj")).unwrap();
        let existing = projects.join("proj/s0.jsonl");
        std::fs::write(&existing, "{}\n").unwrap();
        let mut changes = watch(&projects).unwrap();

        let session = projects.join("proj/s1.jsonl");
        let subagent = projects.join("proj/s1/subagents/agent-a1.jsonl");
        let workflow = projects.join("proj/s1/subagents/workflows/wf/agent-w1.jsonl");
        let journal = projects.join("proj/s1/subagents/workflows/wf/journal.jsonl");
        let fresh_project = projects.join("other/s2.jsonl");
        std::fs::write(&session, "{}\n").unwrap();
        std::fs::create_dir_all(workflow.parent().unwrap()).unwrap();
        std::fs::write(&subagent, "{}\n").unwrap();
        std::fs::write(&workflow, "{}\n").unwrap();
        std::fs::write(&journal, "{}\n").unwrap();
        std::fs::create_dir_all(fresh_project.parent().unwrap()).unwrap();
        std::fs::write(&fresh_project, "{}\n").unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&existing)
            .unwrap()
            .write_all(b"{}\n")
            .unwrap();

        let wanted = BTreeSet::from([session, subagent, workflow, fresh_project, existing]);
        let seen = collect_until(&mut changes, &wanted).await;
        assert!(
            wanted.is_subset(&seen),
            "missing {:?}",
            wanted.difference(&seen)
        );
        assert!(!seen.contains(&journal));
    }
}
