use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::ingest::IngestReport;
use crate::store::{ModelTotal, SessionTotals, StoredEvent};
use crate::transcript::TokenCounts;

pub const API_VERSION: u32 = 1;
pub const SNAPSHOT_VERSION: u32 = 1;
pub const MEASURE_KIND: &str = "session.measure";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Health {
    pub v: u32,
    pub version: String,
    pub schema: usize,
    pub pid: u32,
    pub uptime_s: u64,
    pub claude_dir: String,
    pub ingest: IngestHealth,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IngestHealth {
    pub lag_ms: u64,
    pub backlog: usize,
    pub last_ingest_ms: Option<i64>,
    pub progress: Option<Progress>,
    pub watching: bool,
    pub last_error: Option<String>,
    #[serde(default)]
    pub rebuilding: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventsAccepted {
    pub accepted: usize,
    pub rejected: Vec<Rejected>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejected {
    pub index: usize,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportOutcome {
    pub report: IngestReport,
    pub reconcile_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub v: u32,
    pub session: String,
    pub known: bool,
    pub generated_at_ms: i64,
    pub totals: SnapshotTotals,
    pub measure: Option<Measure>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotTotals {
    pub cost_usd: f64,
    pub reconciled_usd: f64,
    pub unpriced_requests: u64,
    pub requests: u64,
    pub subagents: u64,
    pub last_request_ms: Option<i64>,
    pub tokens: Tokens,
    pub models: Vec<ModelCost>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub thinking: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCost {
    pub model: String,
    pub requests: u64,
    pub cost_usd: f64,
    pub unpriced_requests: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measure {
    pub ts_ms: i64,
    pub received_at_ms: i64,
    pub data: serde_json::Value,
}

impl Snapshot {
    pub fn of(
        session: &str,
        totals: SessionTotals,
        measure: Option<StoredEvent>,
        now_ms: i64,
    ) -> Result<Self> {
        let measure = match measure {
            Some(event) => Some(Measure {
                ts_ms: event.ts_ms,
                received_at_ms: event.received_ms,
                data: serde_json::from_str(&event.json)?,
            }),
            None => None,
        };
        Ok(Self {
            v: SNAPSHOT_VERSION,
            session: session.to_string(),
            known: totals.known || measure.is_some(),
            generated_at_ms: now_ms,
            totals: SnapshotTotals {
                cost_usd: totals.cost_usd,
                reconciled_usd: totals.reconciled_usd,
                unpriced_requests: totals.unpriced_requests,
                requests: totals.requests,
                subagents: totals.subagents,
                last_request_ms: totals.last_request_ts.map(|ts| ts * 1000),
                tokens: totals.tokens.into(),
                models: totals.models.into_iter().map(ModelCost::from).collect(),
            },
            measure,
        })
    }
}

impl From<TokenCounts> for Tokens {
    fn from(t: TokenCounts) -> Self {
        Self {
            input: t.input,
            output: t.output,
            thinking: t.thinking,
            cache_read: t.cache_read,
            cache_write_5m: t.cache_write_5m,
            cache_write_1h: t.cache_write_1h,
        }
    }
}

impl From<ModelTotal> for ModelCost {
    fn from(m: ModelTotal) -> Self {
        Self {
            model: m.model,
            requests: m.requests,
            cost_usd: m.cost_usd,
            unpriced_requests: m.unpriced_requests,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNAPSHOT_V1: &str = r#"{
  "v": 1,
  "session": "11111111-1111-1111-1111-111111111111",
  "known": true,
  "generated_at_ms": 1791021600000,
  "totals": {
    "cost_usd": 4.18,
    "reconciled_usd": 0.5,
    "unpriced_requests": 1,
    "requests": 42,
    "subagents": 3,
    "last_request_ms": 1791021000000,
    "tokens": {
      "input": 1200,
      "output": 35000,
      "thinking": 9000,
      "cache_read": 2400000,
      "cache_write_5m": 1500,
      "cache_write_1h": 180000
    },
    "models": [
      {
        "model": "claude-opus-5-5",
        "requests": 40,
        "cost_usd": 4.0,
        "unpriced_requests": 0
      },
      {
        "model": "claude-mystery-9",
        "requests": 2,
        "cost_usd": 0.0,
        "unpriced_requests": 1
      }
    ]
  },
  "measure": {
    "ts_ms": 1791021500000,
    "received_at_ms": 1791021500012,
    "data": {
      "contextPercent": 41
    }
  }
}"#;

    fn totals() -> SessionTotals {
        SessionTotals {
            known: true,
            cost_usd: 4.18,
            reconciled_usd: 0.5,
            unpriced_requests: 1,
            requests: 42,
            subagents: 3,
            last_request_ts: Some(1_791_021_000),
            tokens: TokenCounts {
                input: 1200,
                output: 35_000,
                thinking: 9000,
                cache_read: 2_400_000,
                cache_write_5m: 1500,
                cache_write_1h: 180_000,
            },
            models: vec![
                ModelTotal {
                    model: "claude-opus-5-5".into(),
                    requests: 40,
                    cost_usd: 4.0,
                    unpriced_requests: 0,
                },
                ModelTotal {
                    model: "claude-mystery-9".into(),
                    requests: 2,
                    cost_usd: 0.0,
                    unpriced_requests: 1,
                },
            ],
        }
    }

    #[test]
    fn snapshot_v1_has_the_documented_shape() {
        let snapshot = Snapshot::of(
            "11111111-1111-1111-1111-111111111111",
            totals(),
            Some(StoredEvent {
                ts_ms: 1_791_021_500_000,
                received_ms: 1_791_021_500_012,
                json: r#"{"contextPercent":41}"#.into(),
            }),
            1_791_021_600_000,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_string_pretty(&snapshot).unwrap(),
            SNAPSHOT_V1
        );
        let parsed: Snapshot = serde_json::from_str(SNAPSHOT_V1).unwrap();
        assert_eq!(parsed, snapshot);
    }

    #[test]
    fn a_session_known_only_from_mod_events_is_known() {
        let snapshot = Snapshot::of(
            "s9",
            SessionTotals::default(),
            Some(StoredEvent {
                ts_ms: 5,
                received_ms: 6,
                json: "{}".into(),
            }),
            7,
        )
        .unwrap();
        assert!(snapshot.known);
        assert_eq!(snapshot.totals.last_request_ms, None);
        let unknown = Snapshot::of("s9", SessionTotals::default(), None, 7).unwrap();
        assert!(!unknown.known);
        assert_eq!(unknown.measure, None);
    }
}
