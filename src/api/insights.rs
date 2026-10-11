use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use anyhow::Result;
use hyper::StatusCode;

use super::{REBUILDING, Reply, State, epoch_ms, failure, json, session_of};
use crate::config_scan::Roots;
use crate::insights::{self, InsightsReport, Request};
use crate::paths;

pub async fn insights(state: Arc<State>, query: Option<&str>) -> Reply {
    let session = match session_of(query) {
        Ok(session) => session,
        Err(problem) => return failure(StatusCode::BAD_REQUEST, problem),
    };
    let built = tokio::task::spawn_blocking(move || -> Result<Option<InsightsReport>> {
        let project_dir = {
            let reader = state
                .insights_reader
                .lock()
                .unwrap_or_else(|p| p.into_inner());
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
        let (scan, _) = state.scans.get(&roots, Instant::now());
        let retention_days = paths::retention_days_in(&state.claude_dir);
        let reader = state
            .insights_reader
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        reader.reading(|reader| {
            if reader.reindex_pending()? {
                return Ok(None);
            }
            insights::read(
                reader,
                &state.prices,
                &Request {
                    session: &session,
                    scan: &scan,
                    retention_days,
                    now_ms: epoch_ms(SystemTime::now()),
                },
                &chrono::Local,
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
