use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::transcript::TokenCounts;

const BUNDLED: &str = include_str!("../../assets/prices.toml");
const PER_MILLION: f64 = 1_000_000.0;

#[derive(Debug, Clone, Deserialize)]
pub struct PriceTable {
    version: String,
    #[serde(default = "default_write_5m")]
    cache_write_5m_multiplier: f64,
    #[serde(default = "default_write_1h")]
    cache_write_1h_multiplier: f64,
    #[serde(default)]
    models: BTreeMap<String, ModelPrice>,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelPrice {
    input: f64,
    output: f64,
    cache_read: f64,
    #[serde(default)]
    tiers: Vec<Tier>,
}

#[derive(Debug, Clone, Deserialize)]
struct Tier {
    above_prompt_tokens: u64,
    input: f64,
    output: f64,
    cache_read: f64,
}

struct Rates {
    input: f64,
    output: f64,
    cache_read: f64,
}

fn default_write_5m() -> f64 {
    1.25
}

fn default_write_1h() -> f64 {
    2.0
}

impl PriceTable {
    pub fn bundled() -> Self {
        Self::from_toml(BUNDLED).expect("bundled price table is valid")
    }

    pub fn from_toml(source: &str) -> Result<Self> {
        toml::from_str(source).context("invalid price table")
    }

    pub fn merged_with(mut self, overrides: PriceTable) -> Self {
        self.version = format!("{}+{}", self.version, overrides.version);
        self.models.extend(overrides.models);
        self
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn knows(&self, model: &str) -> bool {
        self.models.contains_key(normalize_model(model))
    }

    pub fn cost(&self, model: &str, tokens: &TokenCounts) -> Option<f64> {
        let price = self.models.get(normalize_model(model))?;
        let rates = price.rates_for(tokens.prompt_total());
        let usd = tokens.input as f64 * rates.input
            + tokens.cache_write_5m as f64 * rates.input * self.cache_write_5m_multiplier
            + tokens.cache_write_1h as f64 * rates.input * self.cache_write_1h_multiplier
            + tokens.cache_read as f64 * rates.cache_read
            + tokens.output as f64 * rates.output;
        Some(usd / PER_MILLION)
    }
}

impl ModelPrice {
    fn rates_for(&self, prompt_tokens: u64) -> Rates {
        self.tiers
            .iter()
            .filter(|tier| prompt_tokens > tier.above_prompt_tokens)
            .max_by_key(|tier| tier.above_prompt_tokens)
            .map(|tier| Rates {
                input: tier.input,
                output: tier.output,
                cache_read: tier.cache_read,
            })
            .unwrap_or(Rates {
                input: self.input,
                output: self.output,
                cache_read: self.cache_read,
            })
    }
}

pub fn normalize_model(model: &str) -> &str {
    let without_bracket = model.split('[').next().unwrap_or(model);
    match without_bracket.rsplit_once('-') {
        Some((base, suffix)) if suffix.len() == 8 && suffix.bytes().all(|b| b.is_ascii_digit()) => {
            base
        }
        _ => without_bracket,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write_5m: u64,
        cache_write_1h: u64,
    ) -> TokenCounts {
        TokenCounts {
            input,
            output,
            thinking: 0,
            cache_read,
            cache_write_5m,
            cache_write_1h,
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn bundled_table_parses() {
        let table = PriceTable::bundled();
        assert_eq!(table.version(), "2026-10-10");
        assert!(table.knows("claude-opus-5-5"));
    }

    #[test]
    fn opus_with_one_hour_cache() {
        let cost = PriceTable::bundled()
            .cost("claude-opus-5-5", &tokens(10, 1000, 20_000, 0, 5000))
            .unwrap();
        assert!(close(cost, 0.00004 + 0.02 + 0.004 + 0.04), "{cost}");
    }

    #[test]
    fn sonnet_with_five_minute_cache() {
        let cost = PriceTable::bundled()
            .cost("claude-sonnet-5-5", &tokens(5, 200, 0, 1000, 0))
            .unwrap();
        assert!(close(cost, 0.00001 + 0.002 + 0.0025), "{cost}");
    }

    #[test]
    fn haiku_5_5_switches_tier_above_100k_prompt_tokens() {
        let table = PriceTable::bundled();
        let small = table
            .cost("claude-haiku-5-5", &tokens(100_000, 0, 0, 0, 0))
            .unwrap();
        assert!(close(small, 0.01), "{small}");
        let large = table
            .cost("claude-haiku-5-5", &tokens(100_001, 0, 0, 0, 0))
            .unwrap();
        assert!(close(large, 100_001.0 * 0.50 / 1e6), "{large}");
    }

    #[test]
    fn dated_and_bracketed_ids_normalize() {
        assert_eq!(
            normalize_model("claude-haiku-4-5-20251001"),
            "claude-haiku-4-5"
        );
        assert_eq!(normalize_model("claude-opus-4-6[1m]"), "claude-opus-4-6");
        assert_eq!(normalize_model("claude-opus-5-5"), "claude-opus-5-5");
        let cost = PriceTable::bundled()
            .cost("claude-haiku-4-5-20251001", &tokens(100, 50, 0, 0, 0))
            .unwrap();
        assert!(close(cost, 0.0001 + 0.00025), "{cost}");
    }

    #[test]
    fn unknown_models_have_no_price() {
        assert_eq!(
            PriceTable::bundled().cost("gpt-9", &tokens(1, 1, 0, 0, 0)),
            None
        );
        assert_eq!(
            PriceTable::bundled().cost("<synthetic>", &tokens(1, 1, 0, 0, 0)),
            None
        );
    }

    #[test]
    fn overrides_replace_individual_models() {
        let overrides = PriceTable::from_toml(
            "version = \"local\"\n[models.claude-opus-5-5]\ninput = 1.0\noutput = 1.0\ncache_read = 1.0\n",
        )
        .unwrap();
        let table = PriceTable::bundled().merged_with(overrides);
        let cost = table
            .cost("claude-opus-5-5", &tokens(1_000_000, 0, 0, 0, 0))
            .unwrap();
        assert!(close(cost, 1.0));
        assert!(table.knows("claude-sonnet-5-5"));
        assert_eq!(table.version(), "2026-10-10+local");
    }
}
