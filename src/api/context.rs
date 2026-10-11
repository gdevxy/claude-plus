use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;

use super::{REBUILDING, Reply, State, epoch_ms, failure, json, session_of};
use crate::config_scan::{self, Roots, Scan};
use crate::context_model::{self, ContextReport, Request};
use hyper::StatusCode;

pub const SCAN_TTL: Duration = Duration::from_secs(30);
pub const SCAN_CACHE_SIZE: usize = 16;

#[derive(Default)]
pub struct Scans {
    entries: Mutex<Vec<Cached>>,
}

struct Cached {
    roots: Roots,
    at: Instant,
    at_ms: i64,
    scan: Arc<Scan>,
}

impl Scans {
    pub fn get(&self, roots: &Roots, now: Instant) -> (Arc<Scan>, i64) {
        {
            let entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(cached) = entries
                .iter()
                .find(|cached| cached.roots == *roots && now.duration_since(cached.at) < SCAN_TTL)
            {
                return (cached.scan.clone(), cached.at_ms);
            }
        }
        let scan = Arc::new(config_scan::scan(roots));
        let at_ms = epoch_ms(SystemTime::now());
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        entries.retain(|cached| cached.roots != *roots);
        entries.push(Cached {
            roots: roots.clone(),
            at: now,
            at_ms,
            scan: scan.clone(),
        });
        if entries.len() > SCAN_CACHE_SIZE {
            entries.remove(0);
        }
        (scan, at_ms)
    }
}

pub async fn context(state: Arc<State>, query: Option<&str>) -> Reply {
    let session = match session_of(query) {
        Ok(session) => session,
        Err(problem) => return failure(StatusCode::BAD_REQUEST, problem),
    };
    let built = tokio::task::spawn_blocking(move || -> Result<Option<ContextReport>> {
        let project_dir = {
            let reader = state.reader.lock().unwrap_or_else(|p| p.into_inner());
            if reader.reindex_pending()? {
                return Ok(None);
            }
            reader.session_cwd(&session)?
        };
        let roots = Roots {
            claude_dir: state.claude_dir.clone(),
            claude_json: state.claude_json.clone(),
            project: project_dir.as_ref().map(PathBuf::from),
        };
        let (scan, scanned_at_ms) = state.scans.get(&roots, Instant::now());
        let reader = state.reader.lock().unwrap_or_else(|p| p.into_inner());
        reader.reading(|reader| {
            if reader.reindex_pending()? {
                return Ok(None);
            }
            context_model::read(
                reader,
                &Request {
                    session: &session,
                    project_dir: project_dir.as_deref(),
                    scan: &scan,
                    scanned_at_ms,
                    now_ms: epoch_ms(SystemTime::now()),
                },
            )
            .map(Some)
        })
    })
    .await;
    match built {
        Ok(Ok(Some(report))) => json(StatusCode::OK, &report),
        Ok(Ok(None)) => failure(StatusCode::SERVICE_UNAVAILABLE, REBUILDING),
        Ok(Err(error)) => failure(StatusCode::INTERNAL_SERVER_ERROR, format!("{error:#}")),
        Err(error) => failure(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn roots(temp: &tempfile::TempDir, project: Option<&str>) -> Roots {
        Roots {
            claude_dir: temp.path().join("claude"),
            claude_json: temp.path().join("claude.json"),
            project: project.map(|p| temp.path().join(p)),
        }
    }

    #[test]
    fn a_scan_is_reused_for_thirty_seconds_per_project() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("claude")).unwrap();
        std::fs::write(temp.path().join("claude/CLAUDE.md"), "first").unwrap();
        let scans = Scans::default();
        let start = Instant::now();
        let (first, _) = scans.get(&roots(&temp, None), start);
        std::fs::write(temp.path().join("claude/CLAUDE.md"), "second, longer").unwrap();
        let (cached, _) = scans.get(&roots(&temp, None), start + Duration::from_secs(29));
        assert!(Arc::ptr_eq(&first, &cached));
        let (other, _) = scans.get(&roots(&temp, Some("work")), start + Duration::from_secs(29));
        assert!(!Arc::ptr_eq(&first, &other));
        let (fresh, _) = scans.get(&roots(&temp, None), start + SCAN_TTL);
        assert_eq!(fresh.sources[0].chars, 14);
    }

    #[test]
    fn the_cache_keeps_at_most_sixteen_projects() {
        let temp = tempfile::tempdir().unwrap();
        let scans = Scans::default();
        let now = Instant::now();
        for index in 0..20 {
            scans.get(&roots(&temp, Some(&format!("p{index}"))), now);
        }
        assert_eq!(scans.entries.lock().unwrap().len(), SCAN_CACHE_SIZE);
    }
}
