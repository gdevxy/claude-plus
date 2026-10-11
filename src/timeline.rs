use std::collections::HashSet;

use serde_json::Value;

use crate::pricing::normalize_model;
use crate::store::{RequestPoint, StoredEvent};
use crate::transcript::TokenCounts;

pub const STEP_KIND: &str = "turn.step";
pub const MATCH_WINDOW_MS: i64 = 300_000;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Thread {
    Main,
    Agent(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Transcript,
    Live,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Observed {
    pub ts_ms: i64,
    pub model: String,
    pub thread: Thread,
    pub tokens: TokenCounts,
    pub effort: Option<String>,
    pub source: Source,
}

impl Observed {
    pub fn touches_cache(&self) -> bool {
        self.tokens.cache_read > 0 || self.tokens.cache_write() > 0
    }

    pub fn prefix(&self) -> u64 {
        self.tokens.cache_read + self.tokens.cache_write()
    }

    fn identity(&self) -> (i64, Thread, String, u64, u64, u64) {
        let (thread, model, input, read, write) = self.fingerprint();
        (self.ts_ms, thread, model.to_string(), input, read, write)
    }

    fn fingerprint(&self) -> (Thread, &str, u64, u64, u64) {
        (
            self.thread.clone(),
            normalize_model(&self.model),
            self.tokens.input,
            self.tokens.cache_read,
            self.tokens.cache_write(),
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Timeline {
    pub requests: Vec<Observed>,
}

impl Timeline {
    pub fn build(requests: &[RequestPoint], steps: &[StoredEvent]) -> Self {
        let mut observed: Vec<Observed> = requests.iter().map(from_transcript).collect();
        let mut matched = vec![false; observed.len()];
        let mut live = Vec::new();
        let mut seen = HashSet::new();
        for (step, answered_ms) in steps.iter().filter_map(from_step) {
            if !seen.insert(step.identity()) {
                continue;
            }
            let twin = observed.iter().enumerate().position(|(index, request)| {
                !matched[index]
                    && request.fingerprint() == step.fingerprint()
                    && (requests[index].ts * 1000 - answered_ms).abs() <= MATCH_WINDOW_MS
            });
            match twin {
                Some(index) => {
                    matched[index] = true;
                    observed[index].ts_ms = observed[index].ts_ms.min(step.ts_ms);
                }
                None => live.push(step),
            }
        }
        live.retain(|step| {
            observed
                .iter()
                .filter(|request| request.thread == step.thread)
                .all(|request| request.ts_ms <= step.ts_ms)
        });
        observed.extend(live);
        observed.sort_by_key(|request| request.ts_ms);
        Self { requests: observed }
    }

    pub fn main(&self) -> Vec<&Observed> {
        self.requests
            .iter()
            .filter(|request| request.thread == Thread::Main)
            .collect()
    }

    pub fn latest_main(&self) -> Option<&Observed> {
        self.requests
            .iter()
            .rev()
            .find(|request| request.thread == Thread::Main)
    }

    pub fn steps_from_ms(requests: &[RequestPoint]) -> i64 {
        requests
            .last()
            .map_or(0, |latest| latest.ts * 1000 - MATCH_WINDOW_MS)
    }
}

fn from_transcript(point: &RequestPoint) -> Observed {
    Observed {
        ts_ms: point
            .sent
            .filter(|sent| *sent <= point.ts)
            .unwrap_or(point.ts)
            * 1000,
        model: point.model.clone(),
        thread: match (&point.agent_id, point.is_sidechain) {
            (Some(agent), _) => Thread::Agent(agent.clone()),
            (None, true) => Thread::Agent(String::new()),
            (None, false) => Thread::Main,
        },
        tokens: point.tokens,
        effort: point.effort.clone(),
        source: Source::Transcript,
    }
}

fn from_step(event: &StoredEvent) -> Option<(Observed, i64)> {
    let data: Value = serde_json::from_str(&event.json).ok()?;
    let usage = data.get("usage")?;
    let count = |field: &str| usage.get(field).and_then(Value::as_u64).unwrap_or(0);
    let tokens = TokenCounts {
        input: count("input_tokens"),
        output: count("output_tokens"),
        cache_read: count("cache_read_input_tokens"),
        cache_write_5m: count("cache_creation_input_tokens"),
        ..Default::default()
    };
    if tokens.is_empty() {
        return None;
    }
    let model = usage
        .get("model")
        .or_else(|| data.get("model"))
        .and_then(Value::as_str)?
        .to_string();
    let sent_ms = data
        .get("sent_ms")
        .and_then(Value::as_i64)
        .filter(|sent| (0..=event.ts_ms).contains(sent))
        .unwrap_or(event.ts_ms);
    let observed = Observed {
        ts_ms: sent_ms,
        model,
        thread: match data.get("agentId").and_then(Value::as_str) {
            Some(agent) => Thread::Agent(agent.to_string()),
            None => Thread::Main,
        },
        tokens,
        effort: data.get("effort").and_then(|effort| match effort {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        }),
        source: Source::Live,
    };
    Some((observed, event.ts_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(ts: i64, input: u64, read: u64, write_1h: u64, agent: Option<&str>) -> RequestPoint {
        RequestPoint {
            ts,
            sent: None,
            model: "claude-opus-5-5".into(),
            tokens: TokenCounts {
                input,
                output: 100,
                cache_read: read,
                cache_write_1h: write_1h,
                ..Default::default()
            },
            is_sidechain: agent.is_some(),
            agent_id: agent.map(String::from),
            effort: Some("high".into()),
        }
    }

    #[test]
    fn a_transcript_row_counts_from_when_it_was_sent_never_later() {
        let sent = RequestPoint {
            sent: Some(940),
            ..point(1000, 10, 20_000, 0, None)
        };
        let impossible = RequestPoint {
            sent: Some(2100),
            ..point(2000, 10, 20_000, 0, None)
        };
        let timeline = Timeline::build(&[sent, impossible], &[]);
        let anchors: Vec<i64> = timeline.requests.iter().map(|r| r.ts_ms).collect();
        assert_eq!(anchors, vec![940_000, 2_000_000]);
    }

    fn step(ts_ms: i64, input: u64, read: u64, write: u64, agent: Option<&str>) -> StoredEvent {
        let agent = agent.map_or(String::new(), |a| format!(r#","agentId":"{a}""#));
        StoredEvent {
            ts_ms,
            received_ms: ts_ms,
            json: format!(
                r#"{{"turnId":"t1","index":0,"model":"claude-opus-5-5","effort":"xhigh"{agent},"usage":{{"model":"claude-opus-5-5","input_tokens":{input},"output_tokens":7,"cache_read_input_tokens":{read},"cache_creation_input_tokens":{write}}}}}"#
            ),
        }
    }

    #[test]
    fn a_step_already_in_the_transcript_is_not_counted_twice() {
        let timeline = Timeline::build(
            &[point(1_000, 5, 9000, 300, None)],
            &[step(1_000_400, 5, 9000, 300, None)],
        );
        assert_eq!(timeline.requests.len(), 1);
        assert_eq!(timeline.requests[0].source, Source::Transcript);
        assert_eq!(timeline.requests[0].ts_ms, 1_000_000);
    }

    #[test]
    fn a_step_matches_its_transcript_row_however_long_ago_the_prompt_was() {
        let thought_long = RequestPoint {
            sent: Some(400),
            ..point(1_000, 5, 9000, 300, None)
        };
        let timeline = Timeline::build(&[thought_long], &[step(1_000_400, 5, 9000, 300, None)]);
        assert_eq!(timeline.requests.len(), 1);
        assert_eq!(timeline.requests[0].ts_ms, 400_000);
    }

    #[test]
    fn a_step_newer_than_the_transcript_joins_as_live() {
        let timeline = Timeline::build(
            &[point(1_000, 5, 9000, 300, None)],
            &[
                step(1_000_400, 5, 9000, 300, None),
                step(1_030_000, 8, 9300, 50, None),
            ],
        );
        let latest = timeline.latest_main().unwrap();
        assert_eq!(latest.source, Source::Live);
        assert_eq!(latest.ts_ms, 1_030_000);
        assert_eq!(latest.tokens.cache_write(), 50);
        assert_eq!(latest.effort.as_deref(), Some("xhigh"));
    }

    #[test]
    fn a_step_reported_twice_is_counted_once() {
        let timeline = Timeline::build(
            &[point(1_000, 5, 9000, 300, None)],
            &[
                step(1_000_400, 5, 9000, 300, None),
                step(1_000_400, 5, 9000, 300, None),
                step(1_030_000, 8, 9300, 50, None),
                step(1_030_000, 8, 9300, 50, None),
                step(1_030_000, 8, 9300, 50, Some("a1")),
            ],
        );
        assert_eq!(
            timeline
                .requests
                .iter()
                .map(|r| (r.ts_ms, r.source, r.thread.clone()))
                .collect::<Vec<_>>(),
            vec![
                (1_000_000, Source::Transcript, Thread::Main),
                (1_030_000, Source::Live, Thread::Main),
                (1_030_000, Source::Live, Thread::Agent("a1".into())),
            ]
        );
    }

    fn sent(mut event: StoredEvent, sent_ms: i64) -> StoredEvent {
        event.json = event
            .json
            .replacen('{', &format!(r#"{{"sent_ms":{sent_ms},"#), 1);
        event
    }

    #[test]
    fn a_step_counts_from_when_its_request_was_sent() {
        let timeline = Timeline::build(
            &[point(1_000, 5, 9000, 300, None)],
            &[
                sent(step(1_000_400, 5, 9000, 300, None), 999_200),
                sent(step(1_090_000, 8, 9300, 50, None), 1_030_000),
                step(1_100_000, 9, 9350, 10, None),
            ],
        );
        assert_eq!(
            timeline
                .requests
                .iter()
                .map(|r| (r.ts_ms, r.source))
                .collect::<Vec<_>>(),
            vec![
                (999_200, Source::Transcript),
                (1_030_000, Source::Live),
                (1_100_000, Source::Live),
            ]
        );
    }

    #[test]
    fn an_unmatched_step_older_than_the_transcript_is_dropped() {
        let timeline = Timeline::build(
            &[point(1_000, 5, 9000, 300, None)],
            &[step(990_000, 1, 2, 3, None)],
        );
        assert_eq!(timeline.requests.len(), 1);
    }

    #[test]
    fn threads_keep_subagents_apart_from_the_main_conversation() {
        let timeline = Timeline::build(
            &[
                point(1_000, 5, 9000, 300, None),
                point(1_010, 5, 100, 0, Some("a1")),
            ],
            &[
                step(1_020_000, 2, 100, 10, Some("a1")),
                step(1_005_000, 4, 9300, 20, None),
            ],
        );
        assert_eq!(
            timeline
                .requests
                .iter()
                .map(|r| (r.ts_ms, r.thread.clone()))
                .collect::<Vec<_>>(),
            vec![
                (1_000_000, Thread::Main),
                (1_005_000, Thread::Main),
                (1_010_000, Thread::Agent("a1".into())),
                (1_020_000, Thread::Agent("a1".into())),
            ]
        );
        assert_eq!(timeline.main().len(), 2);
    }

    #[test]
    fn steps_are_read_from_a_window_before_the_latest_request() {
        assert_eq!(Timeline::steps_from_ms(&[]), 0);
        assert_eq!(
            Timeline::steps_from_ms(&[point(900, 1, 1, 1, None), point(1_000, 1, 1, 1, None)]),
            1_000_000 - MATCH_WINDOW_MS
        );
    }

    #[test]
    fn steps_without_usage_or_model_are_ignored() {
        let empty = StoredEvent {
            ts_ms: 5,
            received_ms: 5,
            json: r#"{"usage":{"model":"m","input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}"#.into(),
        };
        let broken = StoredEvent {
            ts_ms: 6,
            received_ms: 6,
            json: r#"{"usage":null}"#.into(),
        };
        assert_eq!(Timeline::build(&[], &[empty, broken]), Timeline::default());
    }
}
