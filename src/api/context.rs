use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;

use super::{REBUILDING, Reply, State, epoch_ms, failure, json, session_of};
use crate::config_scan::{self, Roots, Scan};
use crate::context_model::{self, ContextReport, Request};
use hyper::StatusCode;

pub const SCAN_TTL: Duration = Duration::from_secs(30);
pub const SCAN_IDLE: Duration = Duration::from_secs(600);
pub const SCAN_CACHE_SIZE: usize = 16;
pub const WARM_EVERY: Duration = Duration::from_secs(1);

#[derive(Default)]
pub struct Scans {
    entries: Mutex<Vec<Cached>>,
}

struct Cached {
    roots: Roots,
    complete: bool,
    scanned_at: Instant,
    used_at: Instant,
    at_ms: i64,
    scan: Arc<Scan>,
}

impl Scans {
    pub fn get(&self, roots: &Roots, now: Instant) -> (Arc<Scan>, i64) {
        if let Some(cached) = self.lock().iter_mut().find(|cached| cached.roots == *roots) {
            cached.used_at = now;
            return (cached.scan.clone(), cached.at_ms);
        }
        let scan = Arc::new(config_scan::scan_quick(roots));
        let at_ms = epoch_ms(SystemTime::now());
        let mut entries = self.lock();
        entries.retain(|cached| cached.roots != *roots);
        entries.push(Cached {
            roots: roots.clone(),
            complete: false,
            scanned_at: now,
            used_at: now,
            at_ms,
            scan: scan.clone(),
        });
        if entries.len() > SCAN_CACHE_SIZE
            && let Some(oldest) = (0..entries.len()).min_by_key(|&i| entries[i].used_at)
        {
            entries.remove(oldest);
        }
        (scan, at_ms)
    }

    pub fn due(&self, now: Instant) -> Vec<Roots> {
        self.lock()
            .iter()
            .filter(|cached| now.saturating_duration_since(cached.used_at) < SCAN_IDLE)
            .filter(|cached| {
                !cached.complete || now.saturating_duration_since(cached.scanned_at) >= SCAN_TTL
            })
            .map(|cached| cached.roots.clone())
            .collect()
    }

    pub fn refresh(&self, now: Instant) {
        for roots in self.due(now) {
            let scan = Arc::new(config_scan::scan(&roots));
            let at_ms = epoch_ms(SystemTime::now());
            if let Some(cached) = self.lock().iter_mut().find(|c| c.roots == roots) {
                cached.complete = true;
                cached.scanned_at = now;
                cached.at_ms = at_ms;
                cached.scan = scan;
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Cached>> {
        self.entries.lock().unwrap_or_else(|p| p.into_inner())
    }
}

pub async fn warm(state: Arc<State>) {
    let mut every = tokio::time::interval(WARM_EVERY);
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        every.tick().await;
        if state.scans.due(Instant::now()).is_empty() {
            continue;
        }
        let state = state.clone();
        let _ = tokio::task::spawn_blocking(move || state.scans.refresh(Instant::now())).await;
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
            home: state.home.clone(),
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
            home: temp.path().join("home"),
            claude_dir: temp.path().join("claude"),
            claude_json: temp.path().join("claude.json"),
            project: project.map(|p| temp.path().join(p)),
        }
    }

    fn project(temp: &tempfile::TempDir) -> Roots {
        std::fs::create_dir_all(temp.path().join("claude")).unwrap();
        std::fs::create_dir_all(temp.path().join("work/api")).unwrap();
        std::fs::write(temp.path().join("work/CLAUDE.md"), "first").unwrap();
        std::fs::write(temp.path().join("work/api/CLAUDE.md"), "nested").unwrap();
        roots(temp, Some("work"))
    }

    fn names(scan: &Scan) -> Vec<&str> {
        scan.sources.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn a_new_project_answers_without_the_nested_walk_and_is_completed_in_the_background() {
        let temp = tempfile::tempdir().unwrap();
        let work = project(&temp);
        let scans = Scans::default();
        let start = Instant::now();
        let (quick, _) = scans.get(&work, start);
        assert_eq!(names(&quick), vec!["CLAUDE.md"]);
        assert_eq!(scans.due(start), vec![work.clone()]);
        scans.refresh(start);
        assert!(scans.due(start).is_empty());
        let (full, _) = scans.get(&work, start + Duration::from_secs(1));
        assert_eq!(names(&full), vec!["CLAUDE.md", "api/CLAUDE.md"]);
    }

    #[test]
    fn a_stale_scan_is_served_at_once_and_refreshed_in_the_background() {
        let temp = tempfile::tempdir().unwrap();
        let work = project(&temp);
        let scans = Scans::default();
        let start = Instant::now();
        scans.get(&work, start);
        scans.refresh(start);
        let (first, _) = scans.get(&work, start);
        std::fs::write(temp.path().join("work/CLAUDE.md"), "second, longer").unwrap();
        let later = start + SCAN_TTL;
        let (stale, _) = scans.get(&work, later);
        assert!(Arc::ptr_eq(&first, &stale));
        assert_eq!(scans.due(later), vec![work.clone()]);
        scans.refresh(later);
        let (fresh, _) = scans.get(&work, later);
        assert_eq!(fresh.sources[0].chars, 14);
    }

    #[test]
    fn a_project_nobody_asked_for_lately_is_not_refreshed() {
        let temp = tempfile::tempdir().unwrap();
        let work = project(&temp);
        let scans = Scans::default();
        let start = Instant::now();
        scans.get(&work, start);
        scans.refresh(start);
        assert!(scans.due(start + SCAN_IDLE + SCAN_TTL).is_empty());
    }

    #[test]
    fn the_cache_keeps_the_sixteen_projects_used_last() {
        let temp = tempfile::tempdir().unwrap();
        let scans = Scans::default();
        let now = Instant::now();
        scans.get(&roots(&temp, Some("p0")), now);
        for index in 1..20 {
            let at = now + Duration::from_millis(index);
            scans.get(&roots(&temp, Some(&format!("p{index}"))), at);
            scans.get(&roots(&temp, Some("p0")), at);
        }
        let kept: Vec<Roots> = scans
            .entries
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.roots.clone())
            .collect();
        assert_eq!(kept.len(), SCAN_CACHE_SIZE);
        assert!(kept.contains(&roots(&temp, Some("p0"))));
    }
}
