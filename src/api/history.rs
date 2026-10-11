use std::sync::Arc;
use std::time::SystemTime;

use anyhow::Result;
use hyper::StatusCode;

use super::{REBUILDING, Reply, State, epoch_ms, failure, json};
use crate::history::{self, HistoryReport, Range};

pub const RANGE_RULE: &str = "range must be 24h, 7d, 30d or 90d";

pub fn range_of(query: Option<&str>) -> Result<Range, &'static str> {
    match query
        .unwrap_or("")
        .split('&')
        .find_map(|pair| pair.strip_prefix("range="))
    {
        None => Ok(Range::Week),
        Some(text) => Range::parse(text).ok_or(RANGE_RULE),
    }
}

pub async fn history(state: Arc<State>, query: Option<&str>) -> Reply {
    let range = match range_of(query) {
        Ok(range) => range,
        Err(problem) => return failure(StatusCode::BAD_REQUEST, problem),
    };
    let built = tokio::task::spawn_blocking(move || -> Result<Option<HistoryReport>> {
        let reader = state.reader.lock().unwrap_or_else(|p| p.into_inner());
        reader.reading(|reader| {
            if reader.reindex_pending()? {
                return Ok(None);
            }
            let now_ms = epoch_ms(SystemTime::now());
            history::read(reader, &state.prices, range, now_ms, &chrono::Local).map(Some)
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

    #[test]
    fn the_range_defaults_to_a_week_and_rejects_anything_else() {
        assert_eq!(range_of(None), Ok(Range::Week));
        assert_eq!(range_of(Some("x=1")), Ok(Range::Week));
        assert_eq!(range_of(Some("x=1&range=24h")), Ok(Range::Day));
        assert_eq!(range_of(Some("range=90d")), Ok(Range::Quarter));
        assert_eq!(range_of(Some("range=1y")), Err(RANGE_RULE));
        assert_eq!(range_of(Some("range=")), Err(RANGE_RULE));
    }
}
