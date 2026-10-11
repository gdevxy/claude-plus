use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use chrono::{DateTime, TimeZone};
use serde::{Deserialize, Serialize};

use super::project_names;
use crate::store::{SessionInfo, SpentAdjustment, SpentRequest};
use crate::transcript::TokenCounts;

pub const EXPORT_VERSION: u32 = 1;
pub const CSV_HEADER: &str = "day,project,model,requests,unpriced,input,output,cache_read,cache_write_5m,cache_write_1h,cost_usd,reconciled_usd";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExportRow {
    pub day: String,
    pub project: String,
    pub model: String,
    pub requests: u64,
    pub unpriced: u64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub cost_usd: f64,
    pub reconciled_usd: f64,
}

#[derive(Serialize)]
struct Export<'a> {
    v: u32,
    rows: &'a [ExportRow],
}

pub fn rows<Tz: TimeZone>(
    requests: &[SpentRequest],
    adjustments: &[SpentAdjustment],
    sessions: &[SessionInfo],
    tz: &Tz,
) -> Vec<ExportRow> {
    let names = project_names(sessions);
    let projects: HashMap<&str, String> = sessions
        .iter()
        .map(|session| {
            let name = names
                .get(&session.project)
                .map_or_else(|| session.project.clone(), |(name, _)| name.clone());
            (session.id.as_str(), name)
        })
        .collect();
    let mut grouped = Grouped::new();
    let key = |session: &str, ts: i64, model: &str| -> Option<Key> {
        let day = DateTime::from_timestamp(ts, 0)?
            .with_timezone(tz)
            .date_naive()
            .to_string();
        let project = projects.get(session).cloned().unwrap_or_default();
        Some((day, project, model.to_string()))
    };
    for request in requests {
        let Some(key) = key(&request.session_id, request.point.ts, &request.point.model) else {
            continue;
        };
        let row = row_of(&mut grouped, key);
        row.requests += 1;
        add_tokens(row, &request.point.tokens);
        match request.cost_usd {
            Some(usd) => row.cost_usd += usd,
            None => row.unpriced += 1,
        }
    }
    for adjustment in adjustments {
        let Some(key) = key(&adjustment.session_id, adjustment.ts, &adjustment.model) else {
            continue;
        };
        let row = row_of(&mut grouped, key);
        add_tokens(row, &adjustment.tokens);
        let usd = adjustment.cost_usd.unwrap_or(0.0);
        row.cost_usd += usd;
        row.reconciled_usd += usd;
    }
    grouped
        .into_values()
        .map(|row| ExportRow {
            cost_usd: to_six_decimals(row.cost_usd),
            reconciled_usd: to_six_decimals(row.reconciled_usd),
            ..row
        })
        .collect()
}

fn to_six_decimals(usd: f64) -> f64 {
    (usd * 1e6).round() / 1e6
}

type Key = (String, String, String);
type Grouped = BTreeMap<Key, ExportRow>;

fn row_of(grouped: &mut Grouped, key: Key) -> &mut ExportRow {
    grouped
        .entry(key)
        .or_insert_with_key(|(day, project, model)| ExportRow {
            day: day.clone(),
            project: project.clone(),
            model: model.clone(),
            ..Default::default()
        })
}

fn add_tokens(row: &mut ExportRow, tokens: &TokenCounts) {
    row.input += tokens.input;
    row.output += tokens.output;
    row.cache_read += tokens.cache_read;
    row.cache_write_5m += tokens.cache_write_5m;
    row.cache_write_1h += tokens.cache_write_1h;
}

pub fn json(rows: &[ExportRow]) -> Result<String> {
    Ok(serde_json::to_string(&Export {
        v: EXPORT_VERSION,
        rows,
    })?)
}

pub fn csv(rows: &[ExportRow]) -> String {
    let mut out = format!("{CSV_HEADER}\n");
    for row in rows {
        let fields = [
            row.day.clone(),
            field(&row.project),
            field(&row.model),
            row.requests.to_string(),
            row.unpriced.to_string(),
            row.input.to_string(),
            row.output.to_string(),
            row.cache_read.to_string(),
            row.cache_write_5m.to_string(),
            row.cache_write_1h.to_string(),
            format!("{:.6}", row.cost_usd),
            format!("{:.6}", row.reconciled_usd),
        ];
        out.push_str(&fields.join(","));
        out.push('\n');
    }
    out
}

fn field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, Utc};

    use super::*;
    use crate::store::RequestPoint;

    const OCT_10_23H: i64 = 1_791_673_200;

    fn request(session: &str, ts: i64, model: &str, cost_usd: Option<f64>) -> SpentRequest {
        SpentRequest {
            session_id: session.into(),
            point: RequestPoint {
                ts,
                sent: None,
                model: model.into(),
                tokens: TokenCounts {
                    input: 1,
                    output: 2,
                    thinking: 0,
                    cache_read: 3,
                    cache_write_5m: 4,
                    cache_write_1h: 5,
                },
                is_sidechain: false,
                agent_id: None,
                effort: None,
            },
            cost_usd,
        }
    }

    fn sessions() -> Vec<SessionInfo> {
        vec![
            SessionInfo {
                id: "s1".into(),
                project: "-work-app".into(),
                cwd: Some("/work/app".into()),
                title: None,
            },
            SessionInfo {
                id: "s2".into(),
                project: "-work-a-b".into(),
                cwd: Some("/work/a,b \"x\"".into()),
                title: None,
            },
        ]
    }

    #[test]
    fn rows_group_requests_and_adjustments_by_local_day_project_and_model() {
        let requests = [
            request("s1", OCT_10_23H, "claude-opus-5-5", Some(1.5)),
            request("s1", OCT_10_23H + 600, "claude-opus-5-5", Some(0.5)),
            request("s1", OCT_10_23H + 60, "claude-mystery-1", None),
            request("s2", OCT_10_23H, "claude-opus-5-5", Some(2.0)),
        ];
        let adjustments = [SpentAdjustment {
            session_id: "s1".into(),
            ts: OCT_10_23H,
            model: "claude-opus-5-5".into(),
            tokens: TokenCounts {
                output: 10,
                cache_write_5m: 7,
                ..Default::default()
            },
            cost_usd: Some(0.25),
        }];
        let utc = rows(&requests, &adjustments, &sessions(), &Utc);
        let keys: Vec<(&str, &str, &str, u64)> = utc
            .iter()
            .map(|r| {
                (
                    r.day.as_str(),
                    r.project.as_str(),
                    r.model.as_str(),
                    r.requests,
                )
            })
            .collect();
        assert_eq!(
            keys,
            [
                ("2026-10-10", "a,b \"x\"", "claude-opus-5-5", 1),
                ("2026-10-10", "app", "claude-mystery-1", 1),
                ("2026-10-10", "app", "claude-opus-5-5", 2),
            ]
        );
        let opus = &utc[2];
        assert_eq!(
            (
                opus.cost_usd,
                opus.reconciled_usd,
                opus.output,
                opus.cache_write_5m
            ),
            (2.25, 0.25, 14, 15)
        );
        assert_eq!(utc[1].unpriced, 1);
        let zurich = FixedOffset::east_opt(2 * 3600).unwrap();
        let later = rows(&requests, &adjustments, &sessions(), &zurich);
        assert!(later.iter().all(|row| row.day == "2026-10-11"));
    }

    #[test]
    fn json_and_csv_carry_the_same_costs_to_six_decimals() {
        let rows = rows(
            &[request(
                "s2",
                OCT_10_23H,
                "claude-opus-5-5",
                Some(628.3534275999997),
            )],
            &[SpentAdjustment {
                session_id: "s2".into(),
                ts: OCT_10_23H,
                model: "claude-opus-5-5".into(),
                tokens: TokenCounts::default(),
                cost_usd: Some(0.1 + 0.2),
            }],
            &sessions(),
            &Utc,
        );
        let json = json(&rows).unwrap();
        assert!(
            json.contains(r#""cost_usd":628.653428,"reconciled_usd":0.3}"#),
            "{json}"
        );
        assert!(csv(&rows).contains(",628.653428,0.300000"));
    }

    #[test]
    fn csv_quotes_what_needs_quoting_and_json_carries_a_version() {
        let rows = rows(
            &[request("s2", OCT_10_23H, "claude-opus-5-5", Some(2.0))],
            &[],
            &sessions(),
            &Utc,
        );
        assert_eq!(
            csv(&rows),
            format!(
                "{CSV_HEADER}\n2026-10-10,\"a,b \"\"x\"\"\",claude-opus-5-5,1,0,1,2,3,4,5,2.000000,0.000000\n"
            )
        );
        assert_eq!(
            json(&rows).unwrap(),
            r#"{"v":1,"rows":[{"day":"2026-10-10","project":"a,b \"x\"","model":"claude-opus-5-5","requests":1,"unpriced":0,"input":1,"output":2,"cache_read":3,"cache_write_5m":4,"cache_write_1h":5,"cost_usd":2.0,"reconciled_usd":0.0}]}"#
        );
        assert_eq!(csv(&[]), format!("{CSV_HEADER}\n"));
    }
}
