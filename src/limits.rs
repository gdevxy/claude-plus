use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use chrono::{DateTime, Datelike, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::forecast::family;
use crate::store::{LimitSample, RESETS_AT_TOLERANCE_MS, SpendPoint, Store};

pub const FIT_KEY: &str = "limits_fit";
pub const FIT_DAYS: i64 = 14;
pub const SETTLE_MS: i64 = 120_000;
pub const PAIRS_NEEDED: u32 = 20;
pub const DOMINANT_SHARE: f64 = 0.8;
pub const PACE_SPAN_MS: i64 = 30 * 60_000;
pub const MIN_PACE_SPAN_MS: i64 = 5 * 60_000;
pub const RESET_DROP_PCT: f64 = 0.5;
pub const SPEND_LIMIT: &str = "spend_limit";
pub const BUDGET: &str = "budget";
const DAY_MS: i64 = 86_400_000;
const HOUR_MS: f64 = 3_600_000.0;
const WINDOW_ORDER: &[&str] = &["five_hour", "seven_day", SPEND_LIMIT, BUDGET];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fit {
    #[serde(default)]
    pub account: Option<String>,
    pub window: String,
    pub family: String,
    pub pairs: u32,
    pub pct_per_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pace {
    pub pct_per_hour: f64,
    pub projected_pct: Option<f64>,
    pub full_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    pub family: String,
    pub pairs: u32,
    pub needed: u32,
    pub pct_per_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitWindow {
    pub window: String,
    pub used_pct: f64,
    pub resets_at_ms: Option<i64>,
    pub sampled_at_ms: i64,
    pub reset_since_sample: bool,
    pub pace: Option<Pace>,
    pub calibration: Calibration,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetView {
    pub amount_usd: f64,
    pub local_usd: f64,
    pub period_start_ms: i64,
    pub resets_at_ms: i64,
}

pub fn samples_from_measure(session_id: &str, ts_ms: i64, data: &Value) -> Vec<LimitSample> {
    let model = data.get("model").and_then(Value::as_str).map(String::from);
    data.get("rateLimits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|limit| {
            Some(LimitSample {
                ts_ms,
                session_id: session_id.to_string(),
                account_id: None,
                model: model.clone(),
                window: limit.get("kind")?.as_str()?.to_string(),
                used_pct: limit.get("percentUsed")?.as_f64()?,
                resets_at_ms: limit
                    .get("resetsAt")
                    .and_then(Value::as_str)
                    .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                    .map(|at| at.timestamp_millis()),
            })
        })
        .collect()
}

pub fn instances(samples: &[LimitSample]) -> Vec<&[LimitSample]> {
    let mut instances = Vec::new();
    let mut start = 0;
    for index in 1..samples.len() {
        let (previous, sample) = (&samples[index - 1], &samples[index]);
        let moved_reset = match (previous.resets_at_ms, sample.resets_at_ms) {
            (Some(a), Some(b)) => (a - b).abs() > RESETS_AT_TOLERANCE_MS,
            (a, b) => a.is_some() != b.is_some(),
        };
        let dropped = sample.used_pct + RESET_DROP_PCT < previous.used_pct;
        let passed_reset = previous
            .resets_at_ms
            .is_some_and(|resets| sample.ts_ms > resets);
        if sample.account_id != previous.account_id
            || sample.window != previous.window
            || moved_reset
            || dropped
            || passed_reset
        {
            instances.push(&samples[start..index]);
            start = index;
        }
    }
    if start < samples.len() {
        instances.push(&samples[start..]);
    }
    instances
}

pub fn fit(
    samples: &[LimitSample],
    spend: &[SpendPoint],
    accounts: &HashMap<String, String>,
    settled_before_ms: i64,
) -> Vec<Fit> {
    let mut ratios: BTreeMap<(Option<String>, String, String), Vec<f64>> = BTreeMap::new();
    let mut spend_by_account: HashMap<Option<&str>, Vec<&SpendPoint>> = HashMap::new();
    for point in spend {
        let account = accounts.get(&point.session_id).map(String::as_str);
        spend_by_account.entry(account).or_default().push(point);
    }
    for instance in instances(samples) {
        let account = instance[0].account_id.as_deref();
        let own = spend_by_account
            .get(&account)
            .map_or(&[][..], Vec::as_slice);
        for pair in instance.windows(2) {
            let (from, to) = (&pair[0], &pair[1]);
            let gained = to.used_pct - from.used_pct;
            if to.ts_ms > settled_before_ms || gained <= 0.0 {
                continue;
            }
            let Some((family, usd)) = dominant_spend(own, from.ts_ms, to.ts_ms) else {
                continue;
            };
            ratios
                .entry((
                    account.map(String::from),
                    to.window.clone(),
                    family.to_string(),
                ))
                .or_default()
                .push(gained / usd);
        }
    }
    ratios
        .into_iter()
        .map(|((account, window, family), ratios)| Fit {
            account,
            window,
            family,
            pairs: ratios.len() as u32,
            pct_per_usd: trimmed_median(ratios),
        })
        .collect()
}

pub fn fits_of(fits: &[Fit], account: Option<&str>) -> Vec<Fit> {
    fits.iter()
        .filter(|fit| fit.account.as_deref() == account)
        .cloned()
        .collect()
}

fn dominant_spend(
    spend: &[&SpendPoint],
    after_ms: i64,
    until_ms: i64,
) -> Option<(&'static str, f64)> {
    let start = spend.partition_point(|point| point.ts_ms <= after_ms);
    let end = spend.partition_point(|point| point.ts_ms <= until_ms);
    let mut by_family: BTreeMap<&'static str, f64> = BTreeMap::new();
    for point in spend[start..end].iter().copied() {
        *by_family.entry(family(&point.model)).or_default() += point.cost_usd;
    }
    let total: f64 = by_family.values().sum();
    let (family, top) = by_family.into_iter().max_by(|a, b| a.1.total_cmp(&b.1))?;
    (total > 0.0 && top / total >= DOMINANT_SHARE).then_some((family, total))
}

fn trimmed_median(mut ratios: Vec<f64>) -> f64 {
    ratios.sort_by(f64::total_cmp);
    let trim = ratios.len() / 10;
    let kept = &ratios[trim..ratios.len() - trim];
    let middle = kept.len() / 2;
    if kept.len() % 2 == 1 {
        kept[middle]
    } else {
        (kept[middle - 1] + kept[middle]) / 2.0
    }
}

pub fn pace(instance: &[LimitSample], now_ms: i64) -> Option<Pace> {
    let latest = instance.last()?;
    let start = now_ms - PACE_SPAN_MS;
    let anchor = instance
        .iter()
        .rev()
        .find(|sample| sample.ts_ms <= start)
        .unwrap_or(&instance[0]);
    let elapsed = now_ms - anchor.ts_ms.max(start);
    if elapsed < MIN_PACE_SPAN_MS {
        return None;
    }
    let per_ms = (latest.used_pct - anchor.used_pct) / elapsed as f64;
    let projected_pct = latest
        .resets_at_ms
        .map(|resets| latest.used_pct + per_ms * (resets - now_ms).max(0) as f64);
    let full_at_ms =
        projected_pct.and_then(|projected| full_at(latest.used_pct, projected, per_ms, now_ms));
    Some(Pace {
        pct_per_hour: per_ms * HOUR_MS,
        projected_pct,
        full_at_ms,
    })
}

pub fn month_start_utc(ms: i64) -> i64 {
    let at = utc(ms);
    first_of_month(at.year(), at.month())
}

pub fn next_month_start_utc(ms: i64) -> i64 {
    let at = utc(ms);
    let (year, month) = if at.month() == 12 {
        (at.year() + 1, 1)
    } else {
        (at.year(), at.month() + 1)
    };
    first_of_month(year, month)
}

fn utc(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .expect("a representable time")
}

fn first_of_month(year: i32, month: u32) -> i64 {
    Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0)
        .single()
        .expect("the first of a month exists")
        .timestamp_millis()
}

pub fn monthly_pace(
    used_pct: f64,
    period_start_ms: i64,
    resets_at_ms: i64,
    now_ms: i64,
) -> Option<Pace> {
    let elapsed = now_ms - period_start_ms;
    if elapsed < DAY_MS {
        return None;
    }
    let per_ms = used_pct / elapsed as f64;
    let projected = used_pct + per_ms * (resets_at_ms - now_ms).max(0) as f64;
    Some(Pace {
        pct_per_hour: per_ms * HOUR_MS,
        projected_pct: Some(projected),
        full_at_ms: full_at(used_pct, projected, per_ms, now_ms),
    })
}

fn full_at(used_pct: f64, projected_pct: f64, per_ms: f64, now_ms: i64) -> Option<i64> {
    (used_pct < 100.0 && projected_pct > 100.0 && per_ms > 0.0)
        .then(|| now_ms + ((100.0 - used_pct) / per_ms).round() as i64)
}

fn window_pace(instance: &[LimitSample], now_ms: i64) -> Option<Pace> {
    let sample = instance.last()?;
    match sample.resets_at_ms {
        Some(resets) if sample.window == SPEND_LIMIT => {
            monthly_pace(sample.used_pct, month_start_utc(resets - 1), resets, now_ms)
        }
        _ => pace(instance, now_ms),
    }
}

pub fn windows(
    samples: &[LimitSample],
    fits: &[Fit],
    model: Option<&str>,
    now_ms: i64,
    reported: &[&str],
    account: Option<&str>,
) -> Vec<LimitWindow> {
    if account.is_none() {
        return Vec::new();
    }
    let family = model.map_or("other", family);
    let mut latest: BTreeMap<&str, &[LimitSample]> = BTreeMap::new();
    for instance in instances(samples) {
        if instance[0].account_id.as_deref() == account {
            latest.insert(&instance[0].window, instance);
        }
    }
    let mut windows: Vec<LimitWindow> = latest
        .into_values()
        .filter(|instance| {
            let sample = instance.last().expect("instances are never empty");
            sample.resets_at_ms.is_some() || reported.contains(&sample.window.as_str())
        })
        .map(|instance| {
            let sample = instance.last().expect("instances are never empty");
            let reset = sample.resets_at_ms.is_some_and(|resets| resets <= now_ms);
            let fit = fits.iter().find(|fit| {
                fit.account.as_deref() == account
                    && fit.window == sample.window
                    && fit.family == family
            });
            let pairs = fit.map_or(0, |fit| fit.pairs);
            LimitWindow {
                window: sample.window.clone(),
                used_pct: if reset { 0.0 } else { sample.used_pct },
                resets_at_ms: sample.resets_at_ms.filter(|_| !reset),
                sampled_at_ms: sample.ts_ms,
                reset_since_sample: reset,
                pace: if reset {
                    None
                } else {
                    window_pace(instance, now_ms)
                },
                calibration: Calibration {
                    family: family.to_string(),
                    pairs,
                    needed: PAIRS_NEEDED,
                    pct_per_usd: fit
                        .filter(|fit| fit.pairs >= PAIRS_NEEDED)
                        .map(|fit| fit.pct_per_usd),
                },
            }
        })
        .collect();
    windows.sort_by_key(|window| {
        (
            WINDOW_ORDER
                .iter()
                .position(|known| *known == window.window)
                .unwrap_or(WINDOW_ORDER.len()),
            window.window.clone(),
        )
    });
    windows
}

pub fn budget_window(budget: &BudgetView, model: Option<&str>, now_ms: i64) -> LimitWindow {
    let used_pct = budget.local_usd / budget.amount_usd * 100.0;
    LimitWindow {
        window: BUDGET.into(),
        used_pct,
        resets_at_ms: Some(budget.resets_at_ms),
        sampled_at_ms: now_ms,
        reset_since_sample: false,
        pace: monthly_pace(
            used_pct,
            budget.period_start_ms,
            budget.resets_at_ms,
            now_ms,
        ),
        calibration: Calibration {
            family: model.map_or("other", family).to_string(),
            pairs: 0,
            needed: PAIRS_NEEDED,
            pct_per_usd: None,
        },
    }
}

pub fn refresh_fit(store: &mut Store, now_ms: i64) -> Result<Vec<Fit>> {
    let since = now_ms - FIT_DAYS * DAY_MS;
    let samples = store.limit_samples_since(since)?;
    let spend = store.spend_since(since)?;
    let fits = fit(
        &samples,
        &spend,
        &store.session_accounts()?,
        now_ms - SETTLE_MS,
    );
    let mut writer = store.writer()?;
    writer.set_meta(FIT_KEY, &serde_json::to_string(&fits)?)?;
    writer.commit()?;
    Ok(fits)
}

pub fn stored_fit(store: &Store) -> Result<Vec<Fit>> {
    Ok(match store.meta(FIT_KEY)? {
        Some(json) => serde_json::from_str(&json).unwrap_or_default(),
        None => Vec::new(),
    })
}

pub fn recent_samples(store: &Store, now_ms: i64) -> Result<Vec<LimitSample>> {
    store.limit_samples_since(now_ms - 8 * DAY_MS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    const T0: i64 = 1_791_000_000_000;
    const MINUTE: i64 = 60_000;

    fn sample(minute: i64, window: &str, used_pct: f64, resets_minute: i64) -> LimitSample {
        LimitSample {
            ts_ms: T0 + minute * MINUTE,
            session_id: "s1".into(),
            account_id: None,
            model: Some("claude-opus-5-5".into()),
            window: window.into(),
            used_pct,
            resets_at_ms: Some(T0 + resets_minute * MINUTE),
        }
    }

    fn spend(minute: i64, model: &str, cost_usd: f64) -> SpendPoint {
        SpendPoint {
            ts_ms: T0 + minute * MINUTE,
            session_id: "s1".into(),
            model: model.into(),
            cost_usd,
        }
    }

    fn of(
        account: &str,
        minute: i64,
        window: &str,
        used_pct: f64,
        resets_minute: i64,
    ) -> LimitSample {
        LimitSample {
            account_id: Some(account.into()),
            ..sample(minute, window, used_pct, resets_minute)
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn a_measure_becomes_one_sample_per_window() {
        let data = json!({
            "model": "claude-opus-5-5",
            "rateLimits": [
                {"kind": "five_hour", "percentUsed": 54.5, "resetsAt": "2026-10-11T10:00:00Z"},
                {"kind": "spend_limit", "percentUsed": 3},
                {"kind": "broken"}
            ]
        });
        let samples = samples_from_measure("s1", 42, &data);
        assert_eq!(
            samples,
            vec![
                LimitSample {
                    ts_ms: 42,
                    session_id: "s1".into(),
                    account_id: None,
                    model: Some("claude-opus-5-5".into()),
                    window: "five_hour".into(),
                    used_pct: 54.5,
                    resets_at_ms: Some(1_791_712_800_000),
                },
                LimitSample {
                    ts_ms: 42,
                    session_id: "s1".into(),
                    account_id: None,
                    model: Some("claude-opus-5-5".into()),
                    window: "spend_limit".into(),
                    used_pct: 3.0,
                    resets_at_ms: None,
                }
            ]
        );
        assert_eq!(samples_from_measure("s1", 1, &json!({})), vec![]);
    }

    #[test]
    fn a_drop_or_a_new_reset_time_starts_a_new_window() {
        let samples = vec![
            sample(0, "five_hour", 10.0, 300),
            sample(10, "five_hour", 12.0, 300),
            sample(20, "five_hour", 1.0, 300),
            sample(30, "five_hour", 2.0, 600),
            sample(31, "five_hour", 2.0, 600),
            sample(5, "seven_day", 40.0, 9000),
        ];
        let lengths: Vec<usize> = instances(&samples).iter().map(|i| i.len()).collect();
        assert_eq!(lengths, vec![2, 1, 2, 1]);
    }

    #[test]
    fn a_reading_after_the_previous_reset_time_starts_a_new_window() {
        let samples = vec![
            sample(0, "five_hour", 50.0, 100),
            sample(90, "five_hour", 55.0, 100),
            sample(120, "five_hour", 56.0, 100),
        ];
        let lengths: Vec<usize> = instances(&samples).iter().map(|i| i.len()).collect();
        assert_eq!(lengths, vec![2, 1]);
    }

    #[test]
    fn a_window_without_a_reset_time_is_shown_only_while_this_session_reports_it() {
        let mut spend = of("oauth:me", 0, "spend_limit", 3.0, 0);
        spend.resets_at_ms = None;
        let samples = vec![of("oauth:me", 0, "five_hour", 50.0, 400), spend];
        let shown = |reported: &[&str]| -> Vec<String> {
            windows(&samples, &[], None, T0 + MINUTE, reported, Some("oauth:me"))
                .into_iter()
                .map(|w| w.window)
                .collect()
        };
        assert_eq!(shown(&[]), vec!["five_hour"]);
        assert_eq!(shown(&["spend_limit"]), vec!["five_hour", "spend_limit"]);
    }

    #[test]
    fn the_fit_pairs_point_gains_with_dollars_of_the_dominant_family() {
        let mut samples = Vec::new();
        let mut spent = Vec::new();
        for i in 0..25 {
            samples.push(sample(i * 10, "five_hour", i as f64, 400));
            spent.push(spend(i * 10 + 5, "claude-opus-5-5", 2.0));
        }
        spent[3] = spend(35, "claude-opus-5-5", 200.0);
        spent.push(spend(36, "claude-haiku-5-5", 0.01));
        let fits = fit(&samples, &spent, &HashMap::new(), T0 + 1000 * MINUTE);
        assert_eq!(fits.len(), 1);
        assert_eq!(
            (
                fits[0].window.as_str(),
                fits[0].family.as_str(),
                fits[0].pairs
            ),
            ("five_hour", "opus", 24)
        );
        assert!(close(fits[0].pct_per_usd, 0.5));
    }

    #[test]
    fn mixed_spend_unsettled_pairs_and_resets_make_no_pairs() {
        let samples = vec![
            sample(0, "five_hour", 1.0, 300),
            sample(10, "five_hour", 2.0, 300),
            sample(20, "five_hour", 3.0, 300),
            sample(30, "five_hour", 0.0, 600),
            sample(40, "five_hour", 1.0, 600),
        ];
        let spent = vec![
            spend(5, "claude-opus-5-5", 1.0),
            spend(6, "claude-sonnet-5-5", 1.0),
            spend(15, "claude-opus-5-5", 1.0),
            spend(25, "claude-opus-5-5", 1.0),
            spend(35, "claude-opus-5-5", 1.0),
        ];
        let none = HashMap::new();
        assert_eq!(fit(&samples, &spent, &none, T0 + 15 * MINUTE), vec![]);
        let settled = fit(&samples, &spent, &none, T0 + 100 * MINUTE);
        assert_eq!(settled[0].pairs, 2);
    }

    #[test]
    fn the_pace_extrapolates_the_last_half_hour_to_the_reset() {
        let instance = vec![
            sample(0, "five_hour", 20.0, 300),
            sample(40, "five_hour", 30.0, 300),
            sample(60, "five_hour", 40.0, 300),
            sample(69, "five_hour", 46.0, 300),
        ];
        let pace = pace(&instance, T0 + 70 * MINUTE).unwrap();
        assert!(close(pace.pct_per_hour, 32.0));
        assert!(close(
            pace.projected_pct.unwrap(),
            46.0 + 32.0 * 230.0 / 60.0
        ));
        assert_eq!(
            pace.full_at_ms,
            Some(T0 + 70 * MINUTE + (54.0 / 32.0 * 3_600_000.0f64).round() as i64)
        );
    }

    #[test]
    fn a_young_window_paces_from_its_first_sample_and_a_newborn_one_not_at_all() {
        let young = vec![
            sample(50, "five_hour", 0.0, 100),
            sample(60, "five_hour", 5.0, 100),
        ];
        let pace_young = pace(&young, T0 + 60 * MINUTE).unwrap();
        assert!(close(pace_young.pct_per_hour, 30.0));
        assert!(close(pace_young.projected_pct.unwrap(), 25.0));
        assert_eq!(pace_young.full_at_ms, None);
        let newborn = vec![
            sample(58, "five_hour", 0.0, 350),
            sample(60, "five_hour", 1.0, 350),
        ];
        assert_eq!(pace(&newborn, T0 + 60 * MINUTE), None);
    }

    #[test]
    fn windows_report_the_latest_reading_with_its_calibration() {
        let samples = vec![
            of("oauth:me", 0, "five_hour", 50.0, 100),
            of("oauth:me", 1, "seven_day", 12.0, 5000),
            of("oauth:me", 2, "spend_limit", 3.0, 90_000),
        ];
        let fits = vec![
            Fit {
                account: Some("oauth:work".into()),
                window: "five_hour".into(),
                family: "opus".into(),
                pairs: 40,
                pct_per_usd: 9.0,
            },
            Fit {
                account: Some("oauth:me".into()),
                window: "five_hour".into(),
                family: "opus".into(),
                pairs: 20,
                pct_per_usd: 0.5,
            },
            Fit {
                account: Some("oauth:me".into()),
                window: "seven_day".into(),
                family: "opus".into(),
                pairs: 19,
                pct_per_usd: 0.1,
            },
        ];
        let windows = windows(
            &samples,
            &fits,
            Some("claude-opus-5-5"),
            T0 + 200 * MINUTE,
            &[],
            Some("oauth:me"),
        );
        assert_eq!(
            windows
                .iter()
                .map(|w| w.window.as_str())
                .collect::<Vec<_>>(),
            vec!["five_hour", "seven_day", "spend_limit"]
        );
        let five = &windows[0];
        assert!(five.reset_since_sample);
        assert_eq!(
            (five.used_pct, five.resets_at_ms, &five.pace),
            (0.0, None, &None)
        );
        assert_eq!(five.calibration.pct_per_usd, Some(0.5));
        let seven = &windows[1];
        assert!(!seven.reset_since_sample);
        assert_eq!(
            (
                seven.used_pct,
                seven.calibration.pairs,
                seven.calibration.pct_per_usd
            ),
            (12.0, 19, None)
        );
        assert_eq!(windows[2].calibration.family, "opus");
    }

    #[test]
    fn switching_accounts_and_back_resumes_the_same_instance() {
        let mut samples = vec![
            of("oauth:me", 0, "five_hour", 60.0, 300),
            of("oauth:me", 40, "five_hour", 62.0, 300),
            of("oauth:work", 10, "five_hour", 5.0, 200),
            of("oauth:work", 20, "five_hour", 6.0, 200),
        ];
        samples.sort_by(|a, b| {
            (&a.account_id, &a.window, a.ts_ms).cmp(&(&b.account_id, &b.window, b.ts_ms))
        });
        let split: Vec<Vec<f64>> = instances(&samples)
            .iter()
            .map(|instance| instance.iter().map(|s| s.used_pct).collect())
            .collect();
        assert_eq!(split, vec![vec![60.0, 62.0], vec![5.0, 6.0]]);
    }

    #[test]
    fn a_change_of_account_alone_starts_a_new_instance() {
        let samples = vec![
            of("oauth:me", 0, "five_hour", 10.0, 300),
            of("oauth:work", 10, "five_hour", 12.0, 300),
        ];
        let lengths: Vec<usize> = instances(&samples).iter().map(|i| i.len()).collect();
        assert_eq!(lengths, vec![1, 1]);
    }

    #[test]
    fn a_fit_only_counts_spend_of_its_own_account() {
        let samples = vec![
            of("oauth:me", 0, "five_hour", 10.0, 300),
            of("oauth:me", 10, "five_hour", 12.0, 300),
        ];
        let spend = vec![
            SpendPoint {
                ts_ms: T0 + 5 * MINUTE,
                session_id: "mine".into(),
                model: "claude-opus-5-5".into(),
                cost_usd: 1.0,
            },
            SpendPoint {
                ts_ms: T0 + 6 * MINUTE,
                session_id: "work".into(),
                model: "claude-opus-5-5".into(),
                cost_usd: 9.0,
            },
        ];
        let accounts = HashMap::from([
            ("mine".to_string(), "oauth:me".to_string()),
            ("work".to_string(), "oauth:work".to_string()),
        ]);
        let fits = fit(&samples, &spend, &accounts, i64::MAX);
        assert_eq!(fits.len(), 1);
        assert_eq!(fits[0].account.as_deref(), Some("oauth:me"));
        assert!(close(fits[0].pct_per_usd, 2.0), "{}", fits[0].pct_per_usd);
    }

    #[test]
    fn fits_of_keeps_only_the_asked_account() {
        let fit_for = |account: Option<&str>| Fit {
            account: account.map(String::from),
            window: "five_hour".into(),
            family: "opus".into(),
            pairs: 1,
            pct_per_usd: 1.0,
        };
        let fits = vec![
            fit_for(Some("oauth:me")),
            fit_for(Some("oauth:work")),
            fit_for(None),
        ];
        assert_eq!(
            fits_of(&fits, Some("oauth:work")),
            vec![fit_for(Some("oauth:work"))]
        );
        assert_eq!(fits_of(&fits, None), vec![fit_for(None)]);
    }

    #[test]
    fn a_stored_fit_without_an_account_still_reads() {
        let fits: Vec<Fit> = serde_json::from_str(
            r#"[{"window":"five_hour","family":"opus","pairs":3,"pct_per_usd":0.5}]"#,
        )
        .unwrap();
        assert_eq!(fits[0].account, None);
    }

    #[test]
    fn windows_show_only_the_asked_account() {
        let samples = vec![
            of("oauth:me", 0, "seven_day", 40.0, 9000),
            of("oauth:work", 1, "spend_limit", 4.0, 50_000),
        ];
        let shown = |account| -> Vec<String> {
            windows(&samples, &[], None, T0 + 2 * MINUTE, &[], account)
                .into_iter()
                .map(|w| w.window)
                .collect()
        };
        assert_eq!(shown(Some("oauth:work")), vec!["spend_limit"]);
        assert_eq!(shown(Some("oauth:me")), vec!["seven_day"]);
        assert_eq!(shown(None), Vec::<String>::new());
    }

    #[test]
    fn monthly_pace_waits_a_full_day_and_uses_utc_months() {
        let at = |text: &str| {
            DateTime::parse_from_rfc3339(text)
                .unwrap()
                .timestamp_millis()
        };
        let oct_1 = at("2026-10-01T00:00:00Z");
        let nov_1 = at("2026-11-01T00:00:00Z");
        let oct_11 = at("2026-10-11T12:00:00Z");
        assert_eq!(month_start_utc(oct_11), oct_1);
        assert_eq!(next_month_start_utc(oct_11), nov_1);
        assert_eq!(month_start_utc(nov_1 - 1), oct_1);
        assert_eq!(month_start_utc(nov_1), nov_1);
        assert_eq!(
            next_month_start_utc(at("2026-12-31T23:59:59Z")),
            at("2027-01-01T00:00:00Z")
        );
        assert_eq!(monthly_pace(4.0, oct_1, nov_1, oct_1 + DAY_MS - 1), None);
        let pace = monthly_pace(4.0, oct_1, nov_1, oct_11).unwrap();
        let elapsed_days = 10.5;
        let projected = 4.0 / elapsed_days * 31.0;
        assert!((pace.projected_pct.unwrap() - projected).abs() < 1e-6);
        assert_eq!(pace.full_at_ms, None);
        let hot = monthly_pace(50.0, oct_1, nov_1, oct_11).unwrap();
        assert!(hot.full_at_ms.unwrap() < nov_1);
    }

    #[test]
    fn a_budget_window_shows_an_overrun_above_a_hundred_and_paces_over_its_month() {
        let at = |text: &str| {
            DateTime::parse_from_rfc3339(text)
                .unwrap()
                .timestamp_millis()
        };
        let oct_11 = at("2026-10-11T12:00:00Z");
        let budget = BudgetView {
            amount_usd: 200.0,
            local_usd: 250.0,
            period_start_ms: at("2026-10-01T00:00:00Z"),
            resets_at_ms: at("2026-11-01T00:00:00Z"),
        };
        let window = budget_window(&budget, Some("claude-opus-5-5"), oct_11);
        assert_eq!(window.window, BUDGET);
        assert_eq!(window.used_pct, 125.0);
        assert_eq!(window.resets_at_ms, Some(budget.resets_at_ms));
        assert_eq!(window.sampled_at_ms, oct_11);
        assert!(!window.reset_since_sample);
        assert_eq!(
            window.pace,
            monthly_pace(125.0, budget.period_start_ms, budget.resets_at_ms, oct_11)
        );
        assert_eq!(
            window.calibration,
            Calibration {
                family: "opus".into(),
                pairs: 0,
                needed: PAIRS_NEEDED,
                pct_per_usd: None,
            }
        );
    }

    #[test]
    fn a_spend_limit_window_paces_over_its_month() {
        let resets = DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z")
            .unwrap()
            .timestamp_millis();
        let now = DateTime::parse_from_rfc3339("2026-10-11T12:00:00Z")
            .unwrap()
            .timestamp_millis();
        let samples = vec![LimitSample {
            ts_ms: now - 60_000,
            session_id: "s1".into(),
            account_id: Some("oauth:work".into()),
            model: None,
            window: SPEND_LIMIT.into(),
            used_pct: 4.0,
            resets_at_ms: Some(resets),
        }];
        let shown = windows(&samples, &[], None, now, &[], Some("oauth:work"));
        let projected = shown[0].pace.as_ref().unwrap().projected_pct.unwrap();
        assert!((projected - 4.0 / 10.5 * 31.0).abs() < 1e-6, "{projected}");
    }

    #[test]
    fn a_used_up_spend_limit_is_not_paced_to_fill_again() {
        let at = |text: &str| {
            DateTime::parse_from_rfc3339(text)
                .unwrap()
                .timestamp_millis()
        };
        let (oct_1, nov_1, oct_11) = (
            at("2026-10-01T00:00:00Z"),
            at("2026-11-01T00:00:00Z"),
            at("2026-10-11T12:00:00Z"),
        );
        let over = monthly_pace(104.0, oct_1, nov_1, oct_11).unwrap();
        assert!(over.projected_pct.unwrap() > 100.0);
        assert_eq!(over.full_at_ms, None);
        assert_eq!(
            monthly_pace(100.0, oct_1, nov_1, oct_11)
                .unwrap()
                .full_at_ms,
            None
        );
        let samples = vec![LimitSample {
            ts_ms: oct_11 - 60_000,
            session_id: "s1".into(),
            account_id: Some("oauth:work".into()),
            model: None,
            window: SPEND_LIMIT.into(),
            used_pct: 104.0,
            resets_at_ms: Some(nov_1),
        }];
        let shown = windows(&samples, &[], None, oct_11, &[], Some("oauth:work"));
        assert_eq!(shown[0].pace.as_ref().unwrap().full_at_ms, None);
    }

    #[test]
    fn a_projection_of_exactly_a_hundred_does_not_warn() {
        let start = 0;
        let resets = 4 * 1024 * MINUTE;
        let now = 2 * 1024 * MINUTE;
        let pace = monthly_pace(50.0, start, resets, now).unwrap();
        assert_eq!(pace.projected_pct, Some(100.0));
        assert_eq!(pace.full_at_ms, None);
    }

    #[test]
    fn a_full_five_hour_window_is_not_paced_to_fill_again() {
        let instance = vec![
            sample(30, "five_hour", 90.0, 300),
            sample(60, "five_hour", 100.0, 300),
        ];
        let pace = pace(&instance, T0 + 60 * MINUTE).unwrap();
        assert!(pace.projected_pct.unwrap() > 100.0);
        assert_eq!(pace.full_at_ms, None);
    }

    #[test]
    fn the_fit_is_kept_in_meta_and_read_back() {
        let mut store = Store::open_in_memory().unwrap();
        assert_eq!(stored_fit(&store).unwrap(), vec![]);
        let fits = refresh_fit(&mut store, T0).unwrap();
        assert_eq!(fits, vec![]);
        assert_eq!(store.meta(FIT_KEY).unwrap().as_deref(), Some("[]"));
    }
}
