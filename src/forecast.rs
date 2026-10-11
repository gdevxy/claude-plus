use serde::{Deserialize, Serialize};

use crate::cache_model::Ttl;
use crate::pricing::{PriceTable, normalize_model};
use crate::timeline::Observed;
use crate::transcript::TokenCounts;

pub const RECENT: usize = 10;
pub const WARM_READ_SHARE: f64 = 0.9;
const ALTERNATIVES: &[(&str, &str)] = &[
    ("opus", "claude-sonnet-5-5"),
    ("fable", "claude-opus-5-5"),
    ("mythos", "claude-opus-5-5"),
    ("sonnet", "claude-haiku-5-5"),
    ("haiku", "claude-sonnet-5-5"),
];
const FAMILIES: &[&str] = &["opus", "sonnet", "haiku", "fable", "mythos"];
const LINEUP: &[&str] = &[
    "claude-fable-5-1",
    "claude-opus-5-5",
    "claude-sonnet-5-5",
    "claude-haiku-5-5",
];
const TIERS: &[&str] = &["mythos", "fable", "opus", "sonnet", "haiku"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Forecast {
    pub model: String,
    pub samples: u32,
    pub prefix_tokens: u64,
    pub new_tokens: u64,
    pub output_tokens: u64,
    pub warm_usd: Option<f64>,
    pub cold_usd: Option<f64>,
    pub warm_pct_5h: Option<f64>,
    pub cold_pct_5h: Option<f64>,
    #[serde(default)]
    pub lineup: Vec<ModelCost>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCost {
    pub model: String,
    pub warm_usd: f64,
    pub cold_usd: f64,
}

pub fn family(model: &str) -> &'static str {
    let model = normalize_model(model);
    FAMILIES
        .iter()
        .find(|family| model.split('-').any(|part| part == **family))
        .copied()
        .unwrap_or("other")
}

pub fn alternative_for(model: &str) -> Option<&'static str> {
    let family = family(model);
    ALTERNATIVES
        .iter()
        .find(|(from, _)| *from == family)
        .map(|(_, to)| *to)
}

fn lineup(current: &str) -> Vec<&str> {
    let mut models: Vec<&str> = LINEUP
        .iter()
        .copied()
        .filter(|model| family(model) != family(current))
        .collect();
    models.push(current);
    models.sort_by_key(|model| {
        TIERS
            .iter()
            .position(|tier| *tier == family(model))
            .unwrap_or(TIERS.len())
    });
    models
}

pub fn next_turn(
    main: &[&Observed],
    prices: &PriceTable,
    ttl: Ttl,
    pct_per_usd_5h: Option<f64>,
) -> Option<Forecast> {
    let latest = main.last()?;
    let start = main.len().saturating_sub(RECENT);
    let recent = &main[start..];
    let prefix_tokens = latest.prefix();
    let warm: Vec<&Observed> = (start.max(1)..main.len())
        .filter(|&index| is_warm(main[index - 1], main[index]))
        .map(|index| main[index])
        .collect();
    let sampled = if warm.is_empty() { recent } else { &warm };
    let new_tokens = median(
        sampled
            .iter()
            .map(|r| r.tokens.input + r.tokens.cache_write()),
    );
    let output_tokens = median(recent.iter().map(|r| r.tokens.output));
    let warm = TokenCounts {
        cache_read: prefix_tokens,
        output: output_tokens,
        ..ttl.written(new_tokens)
    };
    let cold = TokenCounts {
        output: output_tokens,
        ..ttl.written(prefix_tokens + new_tokens)
    };
    let warm_usd = prices.cost(&latest.model, &warm);
    let cold_usd = prices.cost(&latest.model, &cold);
    let percent = |usd: Option<f64>| usd.zip(pct_per_usd_5h).map(|(usd, rate)| usd * rate);
    let lineup = lineup(&latest.model)
        .into_iter()
        .filter_map(|model| {
            Some(ModelCost {
                model: model.to_string(),
                warm_usd: prices.cost(model, &warm)?,
                cold_usd: prices.cost(model, &cold)?,
            })
        })
        .collect();
    Some(Forecast {
        model: latest.model.clone(),
        samples: recent.len() as u32,
        prefix_tokens,
        new_tokens,
        output_tokens,
        warm_usd,
        cold_usd,
        warm_pct_5h: percent(warm_usd),
        cold_pct_5h: percent(cold_usd),
        lineup,
    })
}

fn is_warm(previous: &Observed, request: &Observed) -> bool {
    let prefix = previous.prefix();
    prefix > 0 && request.tokens.cache_read as f64 >= prefix as f64 * WARM_READ_SHARE
}

fn median(values: impl Iterator<Item = u64>) -> u64 {
    let mut values: Vec<u64> = values.collect();
    values.sort_unstable();
    match values.len() {
        0 => 0,
        n if n % 2 == 1 => values[n / 2],
        n => (values[n / 2 - 1] + values[n / 2]) / 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::{Source, Thread};

    fn request(model: &str, input: u64, read: u64, write: u64, output: u64) -> Observed {
        Observed {
            ts_ms: 0,
            model: model.into(),
            thread: Thread::Main,
            tokens: TokenCounts {
                input,
                output,
                cache_read: read,
                cache_write_1h: write,
                ..Default::default()
            },
            effort: None,
            source: Source::Transcript,
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn warm_and_cold_follow_the_spec_formulas() {
        let requests = [
            request("claude-opus-5-5", 10, 40_000, 1_000, 300),
            request("claude-opus-5-5", 20, 41_000, 2_010, 900),
            request("claude-opus-5-5", 30, 44_000, 2_000, 600),
        ];
        let main: Vec<&Observed> = requests.iter().collect();
        let forecast = next_turn(&main, &PriceTable::bundled(), Ttl::OneHour, Some(0.5)).unwrap();
        assert_eq!(
            (
                forecast.prefix_tokens,
                forecast.new_tokens,
                forecast.output_tokens,
                forecast.samples
            ),
            (46_000, 2_030, 600, 3)
        );
        let warm = (46_000.0 * 0.2 + 2_030.0 * 8.0 + 600.0 * 20.0) / 1e6;
        let cold = ((46_000.0 + 2_030.0) * 8.0 + 600.0 * 20.0) / 1e6;
        assert!(close(forecast.warm_usd.unwrap(), warm));
        assert!(close(forecast.cold_usd.unwrap(), cold));
        assert!(close(forecast.warm_pct_5h.unwrap(), warm * 0.5));
        assert!(close(forecast.cold_pct_5h.unwrap(), cold * 0.5));
        let sonnet = &forecast.lineup[2];
        assert_eq!(sonnet.model, "claude-sonnet-5-5");
        assert!(close(
            sonnet.cold_usd,
            ((46_000.0 + 2_030.0) * 4.0 + 600.0 * 10.0) / 1e6
        ));
        assert!(close(
            sonnet.warm_usd,
            (46_000.0 * 0.1 + 2_030.0 * 4.0 + 600.0 * 10.0) / 1e6
        ));
        let opus = &forecast.lineup[1];
        assert_eq!(opus.model, "claude-opus-5-5");
        assert!(close(opus.warm_usd, warm) && close(opus.cold_usd, cold));
    }

    #[test]
    fn new_tokens_come_from_warm_requests_once_there_are_any() {
        let opening = request("claude-opus-5-5", 10, 0, 40_000, 300);
        let warm = request("claude-opus-5-5", 20, 40_000, 1_000, 900);
        let early = [opening.clone(), warm];
        let main: Vec<&Observed> = early.iter().collect();
        let forecast = next_turn(&main, &PriceTable::bundled(), Ttl::OneHour, None).unwrap();
        assert_eq!((forecast.new_tokens, forecast.output_tokens), (1_020, 600));
        let alone = [opening];
        let main: Vec<&Observed> = alone.iter().collect();
        let forecast = next_turn(&main, &PriceTable::bundled(), Ttl::OneHour, None).unwrap();
        assert_eq!(forecast.new_tokens, 40_010);
    }

    #[test]
    fn only_the_last_ten_requests_set_the_medians() {
        let mut requests: Vec<Observed> = (0..15)
            .map(|i| request("claude-opus-5-5", 0, 1000, 1000 * i, 10 * i))
            .collect();
        requests.push(request("claude-opus-5-5", 0, 1000, 99_000, 9_990));
        let main: Vec<&Observed> = requests.iter().collect();
        let forecast = next_turn(&main, &PriceTable::bundled(), Ttl::FiveMinutes, None).unwrap();
        assert_eq!(forecast.samples, 10);
        assert_eq!((forecast.new_tokens, forecast.output_tokens), (10_500, 105));
        assert_eq!((forecast.warm_pct_5h, forecast.cold_pct_5h), (None, None));
    }

    #[test]
    fn an_unpriced_model_has_tokens_but_no_dollars() {
        let requests = [request("claude-mystery-9", 1, 2, 3, 4)];
        let main: Vec<&Observed> = requests.iter().collect();
        let forecast = next_turn(&main, &PriceTable::bundled(), Ttl::OneHour, Some(1.0)).unwrap();
        assert_eq!(
            (forecast.warm_usd, forecast.cold_usd, forecast.warm_pct_5h),
            (None, None, None)
        );
        assert_eq!(forecast.lineup.len(), 4);
        assert!(
            forecast
                .lineup
                .iter()
                .all(|cost| cost.model != "claude-mystery-9")
        );
        assert_eq!(
            next_turn(&[], &PriceTable::bundled(), Ttl::OneHour, None),
            None
        );
    }

    #[test]
    fn families_and_alternatives_are_read_from_the_model_id() {
        assert_eq!(family("claude-opus-5-5-20261001"), "opus");
        assert_eq!(family("claude-haiku-4-5[1m]"), "haiku");
        assert_eq!(family("claude-mystery-9"), "other");
        assert_eq!(alternative_for("claude-fable-5-1"), Some("claude-opus-5-5"));
        assert_eq!(
            alternative_for("claude-sonnet-5-5"),
            Some("claude-haiku-5-5")
        );
        assert_eq!(alternative_for("claude-mystery-9"), None);
    }

    #[test]
    fn the_lineup_runs_down_the_tiers_with_the_current_model_in_its_family_slot() {
        assert_eq!(
            lineup("claude-sonnet-5-5"),
            [
                "claude-fable-5-1",
                "claude-opus-5-5",
                "claude-sonnet-5-5",
                "claude-haiku-5-5"
            ]
        );
        assert_eq!(
            lineup("claude-opus-4-7"),
            [
                "claude-fable-5-1",
                "claude-opus-4-7",
                "claude-sonnet-5-5",
                "claude-haiku-5-5"
            ]
        );
        assert_eq!(
            lineup("claude-mythos-5-1"),
            [
                "claude-mythos-5-1",
                "claude-fable-5-1",
                "claude-opus-5-5",
                "claude-sonnet-5-5",
                "claude-haiku-5-5"
            ]
        );
        assert_eq!(lineup("claude-mystery-9").last(), Some(&"claude-mystery-9"));
    }
}
