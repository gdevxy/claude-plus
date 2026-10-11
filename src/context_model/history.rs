use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::ratios::{Ratios, TextKind};
use crate::store::{AttachmentPoint, RequestPoint};

pub const MAX_POINTS: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Part {
    System,
    Memory,
    Skills,
    Mcp,
    Agents,
    Files,
    Hooks,
    Reminders,
    Messages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Share {
    pub part: Part,
    pub tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    pub ts_ms: i64,
    pub prompt_tokens: u64,
    pub injected_tokens: u64,
    pub compacted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct History {
    pub first_request_ms: i64,
    pub measured_tokens: u64,
    pub first_message_tokens: u64,
    pub scale: f64,
    pub initial: Vec<Share>,
    pub now: Vec<Share>,
    pub injected_later_tokens: u64,
    pub compactions: u64,
    pub points: Vec<Point>,
}

pub fn classify(kind: &str) -> Option<(Part, TextKind)> {
    Some(match kind {
        "instructions" | "nested_memory" => (Part::Memory, TextKind::Prose),
        "skill_listing" | "invoked_skills" => (Part::Skills, TextKind::Prose),
        "mcp_instructions_delta" => (Part::Mcp, TextKind::Prose),
        "deferred_tools_delta" | "deferred_tools_record" => (Part::Mcp, TextKind::Schema),
        "agent_listing_delta" => (Part::Agents, TextKind::Prose),
        "file" | "edited_text_file" | "selected_lines_in_ide" | "compact_file_reference" => {
            (Part::Files, TextKind::Code)
        }
        "hook_additional_context" => (Part::Hooks, TextKind::Prose),
        "environment"
        | "date"
        | "model"
        | "session_context"
        | "total_tokens_reminder"
        | "silent_turn_reminder"
        | "task_status"
        | "auto_mode" => (Part::Reminders, TextKind::Prose),
        _ => return None,
    })
}

pub fn compose(
    requests: &[RequestPoint],
    attachments: &[AttachmentPoint],
    compactions: &[i64],
    first_prompt_chars: Option<u64>,
    ratios: &Ratios,
) -> Option<History> {
    let main: Vec<&RequestPoint> = requests
        .iter()
        .filter(|r| !r.is_sidechain && r.agent_id.is_none())
        .collect();
    let first = *main.first()?;
    let mut estimated: Vec<(i64, Part, u64)> = attachments
        .iter()
        .filter_map(|a| {
            classify(&a.kind).map(|(part, text)| (a.ts, part, ratios.tokens(a.chars, text)))
        })
        .collect();
    estimated.sort_by_key(|(ts, _, _)| *ts);
    let mut compactions = compactions.to_vec();
    compactions.sort_unstable();
    let initial_estimate: u64 = estimated
        .iter()
        .filter(|(ts, _, _)| *ts <= first.ts)
        .map(|(_, _, tokens)| tokens)
        .sum();
    let measured = first.tokens.prompt_total();
    let first_message = ratios.tokens(first_prompt_chars.unwrap_or(0), TextKind::Prose);
    let fixed = measured.saturating_sub(first_message);
    let (scale, system) = if initial_estimate > fixed {
        (fixed as f64 / initial_estimate as f64, 0)
    } else {
        (1.0, fixed - initial_estimate)
    };
    let scaled = |tokens: u64| (tokens as f64 * scale).round() as u64;
    let mut initial_parts: BTreeMap<Part, u64> = BTreeMap::new();
    let mut injected_later = 0;
    for (ts, part, tokens) in &estimated {
        if *ts <= first.ts {
            *initial_parts.entry(*part).or_default() += scaled(*tokens);
        } else {
            injected_later += scaled(*tokens);
        }
    }
    let mut points = Vec::with_capacity(main.len());
    let mut injected: BTreeMap<Part, u64> = BTreeMap::new();
    let (mut next_attachment, mut next_compaction) = (0, 0);
    for request in &main {
        let mut compacted = false;
        loop {
            let attachment_ts = estimated
                .get(next_attachment)
                .map(|(ts, _, _)| *ts)
                .filter(|ts| *ts <= request.ts);
            let compaction_ts = compactions
                .get(next_compaction)
                .copied()
                .filter(|ts| *ts <= request.ts);
            match (compaction_ts, attachment_ts) {
                (Some(c), a) if a.is_none_or(|a| c <= a) => {
                    injected.clear();
                    compacted = true;
                    next_compaction += 1;
                }
                (_, Some(_)) => {
                    let (_, part, tokens) = estimated[next_attachment];
                    *injected.entry(part).or_default() += scaled(tokens);
                    next_attachment += 1;
                }
                _ => break,
            }
        }
        points.push(Point {
            ts_ms: request.ts * 1000,
            prompt_tokens: request.tokens.prompt_total(),
            injected_tokens: injected.values().sum(),
            compacted,
        });
    }
    let last = points.last().copied()?;
    let mut now = vec![Share {
        part: Part::System,
        tokens: system,
    }];
    now.extend(
        injected
            .iter()
            .filter(|(_, t)| **t > 0)
            .map(|(part, tokens)| Share {
                part: *part,
                tokens: *tokens,
            }),
    );
    now.push(Share {
        part: Part::Messages,
        tokens: last
            .prompt_tokens
            .saturating_sub(system + last.injected_tokens),
    });
    let mut initial = vec![Share {
        part: Part::System,
        tokens: system,
    }];
    initial.extend(
        initial_parts
            .iter()
            .filter(|(_, t)| **t > 0)
            .map(|(part, tokens)| Share {
                part: *part,
                tokens: *tokens,
            }),
    );
    Some(History {
        first_request_ms: first.ts * 1000,
        measured_tokens: measured,
        first_message_tokens: first_message,
        scale,
        initial,
        now,
        injected_later_tokens: injected_later,
        compactions: compactions.iter().filter(|ts| **ts >= first.ts).count() as u64,
        points: thinned(points),
    })
}

fn thinned(points: Vec<Point>) -> Vec<Point> {
    if points.len() <= MAX_POINTS {
        return points;
    }
    let last = points.len() - 1;
    let mut keep: Vec<usize> = points
        .iter()
        .enumerate()
        .filter(|(i, p)| p.compacted && *i != 0 && *i != last)
        .map(|(i, _)| i)
        .collect();
    keep.truncate(MAX_POINTS - 2);
    keep.extend([0, last]);
    let rest: Vec<usize> = (0..=last).filter(|i| !keep.contains(i)).collect();
    let slots = MAX_POINTS - keep.len();
    let span = rest.len() - 1;
    keep.extend((0..slots).map(|j| rest[j * span / (slots - 1).max(1)]));
    keep.sort_unstable();
    keep.into_iter().map(|i| points[i]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::TokenCounts;

    fn request(ts: i64, prompt: u64, agent: Option<&str>) -> RequestPoint {
        RequestPoint {
            ts,
            sent: None,
            model: "claude-opus-5-5".into(),
            tokens: TokenCounts {
                input: 0,
                output: 100,
                thinking: 0,
                cache_read: prompt,
                cache_write_5m: 0,
                cache_write_1h: 0,
            },
            is_sidechain: agent.is_some(),
            agent_id: agent.map(String::from),
            effort: None,
        }
    }

    fn attachment(ts: i64, kind: &str, chars: u64) -> AttachmentPoint {
        AttachmentPoint {
            ts,
            kind: kind.into(),
            chars,
        }
    }

    fn shares(list: &[Share]) -> Vec<(Part, u64)> {
        list.iter().map(|s| (s.part, s.tokens)).collect()
    }

    #[test]
    fn the_first_request_splits_into_injected_context_and_the_system_prompt() {
        let requests = vec![request(100, 20_000, None), request(160, 26_000, None)];
        let attachments = vec![
            attachment(99, "skill_listing", 7_000),
            attachment(99, "instructions", 3_500),
            attachment(100, "deferred_tools_delta", 2_500),
            attachment(99, "queued_command", 50_000),
            attachment(150, "nested_memory", 1_750),
        ];
        let history = compose(&requests, &attachments, &[], Some(700), &Ratios::default()).unwrap();
        assert_eq!(history.measured_tokens, 20_000);
        assert_eq!(history.first_message_tokens, 200);
        assert_eq!(history.scale, 1.0);
        assert_eq!(
            shares(&history.initial),
            vec![
                (Part::System, 15_800),
                (Part::Memory, 1_000),
                (Part::Skills, 2_000),
                (Part::Mcp, 1_000)
            ]
        );
        assert_eq!(history.injected_later_tokens, 500);
        assert_eq!(
            shares(&history.now),
            vec![
                (Part::System, 15_800),
                (Part::Memory, 1_500),
                (Part::Skills, 2_000),
                (Part::Mcp, 1_000),
                (Part::Messages, 5_700)
            ]
        );
        assert_eq!(
            history.points,
            vec![
                Point {
                    ts_ms: 100_000,
                    prompt_tokens: 20_000,
                    injected_tokens: 4_000,
                    compacted: false
                },
                Point {
                    ts_ms: 160_000,
                    prompt_tokens: 26_000,
                    injected_tokens: 4_500,
                    compacted: false
                },
            ]
        );
    }

    #[test]
    fn estimates_above_the_measured_prompt_are_scaled_down_to_fit() {
        let requests = vec![request(100, 3_000, None)];
        let attachments = vec![
            attachment(90, "skill_listing", 14_000),
            attachment(90, "instructions", 7_000),
        ];
        let history = compose(&requests, &attachments, &[], None, &Ratios::default()).unwrap();
        assert_eq!(history.scale, 0.5);
        assert_eq!(
            shares(&history.initial),
            vec![
                (Part::System, 0),
                (Part::Memory, 1_000),
                (Part::Skills, 2_000)
            ]
        );
    }

    #[test]
    fn a_compaction_empties_the_injected_context_until_it_is_injected_again() {
        let requests = vec![
            request(100, 10_000, None),
            request(200, 60_000, None),
            request(300, 12_000, None),
        ];
        let attachments = vec![
            attachment(90, "skill_listing", 3_500),
            attachment(250, "skill_listing", 3_500),
            attachment(260, "file", 3_000),
        ];
        let history = compose(&requests, &attachments, &[250], None, &Ratios::default()).unwrap();
        assert_eq!(history.compactions, 1);
        let injected: Vec<(u64, bool)> = history
            .points
            .iter()
            .map(|p| (p.injected_tokens, p.compacted))
            .collect();
        assert_eq!(
            injected,
            vec![(1_000, false), (1_000, false), (2_000, true)]
        );
    }

    #[test]
    fn subagent_requests_are_not_the_conversation() {
        let requests = vec![request(50, 99_000, Some("a1")), request(100, 10_000, None)];
        let history = compose(&requests, &[], &[], None, &Ratios::default()).unwrap();
        assert_eq!(
            (history.first_request_ms, history.measured_tokens),
            (100_000, 10_000)
        );
        assert_eq!(
            compose(&requests[..1], &[], &[], None, &Ratios::default()),
            None
        );
        assert_eq!(compose(&[], &[], &[], None, &Ratios::default()), None);
    }

    #[test]
    fn a_long_session_keeps_sixty_points_and_every_compaction() {
        let requests: Vec<RequestPoint> = (0..500)
            .map(|i| request(i, 1_000 + i as u64, None))
            .collect();
        let compactions: Vec<i64> = (0..10).map(|i| 33 + i * 47).collect();
        let history = compose(&requests, &[], &compactions, None, &Ratios::default()).unwrap();
        assert_eq!(history.points.len(), MAX_POINTS);
        assert_eq!(history.points.first().unwrap().ts_ms, 0);
        assert_eq!(history.points.last().unwrap().ts_ms, 499_000);
        let kept: Vec<i64> = history
            .points
            .iter()
            .filter(|p| p.compacted)
            .map(|p| p.ts_ms / 1000)
            .collect();
        assert_eq!(kept, compactions);
        assert!(history.points.windows(2).all(|w| w[0].ts_ms < w[1].ts_ms));
    }

    #[test]
    fn every_listed_kind_has_a_part_and_bookkeeping_has_none() {
        for kind in [
            "instructions",
            "nested_memory",
            "skill_listing",
            "invoked_skills",
            "mcp_instructions_delta",
            "deferred_tools_delta",
            "deferred_tools_record",
            "agent_listing_delta",
        ] {
            assert!(classify(kind).is_some(), "{kind}");
        }
        for kind in [
            "queued_command",
            "prompt_snapshot",
            "hook_success",
            "thinking_drop",
            "command_permissions",
        ] {
            assert_eq!(classify(kind), None, "{kind}");
        }
    }
}
