use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::transcript::TokenCounts;

const BUNDLED: &str = include_str!("../../assets/prices.toml");
const PER_MILLION: f64 = 1_000_000.0;

#[derive(Debug, Clone, Deserialize)]
pub struct PriceTable {
    #[serde(default = "local_version")]
    version: String,
    cache_write_5m_multiplier: Option<f64>,
    cache_write_1h_multiplier: Option<f64>,
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

fn local_version() -> String {
    "local".into()
}

const DEFAULT_WRITE_5M: f64 = 1.25;
const DEFAULT_WRITE_1H: f64 = 2.0;

fn check_rate(name: &str, value: f64) -> Result<()> {
    if !value.is_finite() || value < 0.0 {
        bail!("{name} must be a finite non-negative rate, got {value}");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserPrices {
    Absent,
    Loaded,
    Invalid(String),
}

impl PriceTable {
    pub fn with_user_file(path: &Path) -> (Self, UserPrices) {
        let bundled = Self::bundled();
        match std::fs::read_to_string(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (bundled, UserPrices::Absent)
            }
            Err(error) => (
                bundled,
                UserPrices::Invalid(format!("cannot read {}: {error}", path.display())),
            ),
            Ok(text) => match Self::from_toml(&text) {
                Ok(overrides) => (bundled.merged_with(overrides), UserPrices::Loaded),
                Err(error) => (
                    bundled,
                    UserPrices::Invalid(format!("{}: {error:#}", path.display())),
                ),
            },
        }
    }

    pub fn bundled() -> Self {
        Self::from_toml(BUNDLED).expect("bundled price table is valid")
    }

    pub fn from_toml(source: &str) -> Result<Self> {
        let table: Self = toml::from_str(source).context("invalid price table")?;
        table.validate().context("invalid price table")?;
        Ok(table)
    }

    fn validate(&self) -> Result<()> {
        for (name, multiplier) in [
            ("cache_write_5m_multiplier", self.cache_write_5m_multiplier),
            ("cache_write_1h_multiplier", self.cache_write_1h_multiplier),
        ] {
            if let Some(value) = multiplier {
                check_rate(name, value)?;
            }
        }
        for (model, price) in &self.models {
            price.base_rates().check(model)?;
            for tier in &price.tiers {
                let label = format!("{model} tier above {}", tier.above_prompt_tokens);
                tier.rates().check(&label)?;
            }
        }
        Ok(())
    }

    pub fn merged_with(mut self, overrides: PriceTable) -> Self {
        self.version = format!("{}+{}", self.version, overrides.version);
        self.cache_write_5m_multiplier = overrides
            .cache_write_5m_multiplier
            .or(self.cache_write_5m_multiplier);
        self.cache_write_1h_multiplier = overrides
            .cache_write_1h_multiplier
            .or(self.cache_write_1h_multiplier);
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
        Some(self.priced(&price.rates_for(tokens.prompt_total()), tokens))
    }

    pub fn cost_within(
        &self,
        model: &str,
        part: &TokenCounts,
        whole_prompt_tokens: u64,
    ) -> Option<f64> {
        let price = self.models.get(normalize_model(model))?;
        Some(self.priced(&price.rates_for(whole_prompt_tokens), part))
    }

    pub fn base_cost(&self, model: &str, tokens: &TokenCounts) -> Option<f64> {
        let price = self.models.get(normalize_model(model))?;
        Some(self.priced(&price.base_rates(), tokens))
    }

    fn priced(&self, rates: &Rates, tokens: &TokenCounts) -> f64 {
        let write_5m = self.cache_write_5m_multiplier.unwrap_or(DEFAULT_WRITE_5M);
        let write_1h = self.cache_write_1h_multiplier.unwrap_or(DEFAULT_WRITE_1H);
        let usd = tokens.input as f64 * rates.input
            + tokens.cache_write_5m as f64 * rates.input * write_5m
            + tokens.cache_write_1h as f64 * rates.input * write_1h
            + tokens.cache_read as f64 * rates.cache_read
            + tokens.output as f64 * rates.output;
        usd / PER_MILLION
    }
}

impl ModelPrice {
    fn rates_for(&self, prompt_tokens: u64) -> Rates {
        self.tiers
            .iter()
            .filter(|tier| prompt_tokens > tier.above_prompt_tokens)
            .max_by_key(|tier| tier.above_prompt_tokens)
            .map(Tier::rates)
            .unwrap_or_else(|| self.base_rates())
    }

    fn base_rates(&self) -> Rates {
        Rates {
            input: self.input,
            output: self.output,
            cache_read: self.cache_read,
        }
    }
}

impl Tier {
    fn rates(&self) -> Rates {
        Rates {
            input: self.input,
            output: self.output,
            cache_read: self.cache_read,
        }
    }
}

impl Rates {
    fn check(&self, label: &str) -> Result<()> {
        for (field, value) in [
            ("input", self.input),
            ("output", self.output),
            ("cache_read", self.cache_read),
        ] {
            check_rate(&format!("{label} {field}"), value)?;
        }
        Ok(())
    }
}

pub fn normalize_model(model: &str) -> &str {
    let model = model.split('[').next().unwrap_or(model);
    let model = model.rsplit_once('/').map_or(model, |(_, name)| name);
    let model = model.split_once('@').map_or(model, |(name, _)| name);
    let model = model
        .find("anthropic.")
        .map_or(model, |at| &model[at + "anthropic.".len()..]);
    let model = strip_suffix_where(model, |suffix| {
        suffix
            .strip_prefix('v')
            .and_then(|rest| rest.split_once(':'))
            .is_some_and(|(major, minor)| is_digits(major) && is_digits(minor))
    });
    strip_suffix_where(model, |suffix| suffix.len() == 8 && is_digits(suffix))
}

fn strip_suffix_where(model: &str, matches: impl Fn(&str) -> bool) -> &str {
    match model.rsplit_once('-') {
        Some((base, suffix)) if matches(suffix) => base,
        _ => model,
    }
}

fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
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
    fn cloud_and_gateway_model_ids_find_their_price() {
        for id in [
            "us.anthropic.claude-opus-5-5-20260901-v1:0",
            "global.anthropic.claude-opus-5-5-v1:0",
            "anthropic.claude-opus-5-5-20260901-v2:0",
            "claude-opus-5-5@20260901",
            "anthropic/claude-opus-5-5",
            "bedrock/us.anthropic.claude-opus-5-5-20260901-v1:0",
            "claude-opus-5-5-20260901[1m]",
        ] {
            assert_eq!(normalize_model(id), "claude-opus-5-5", "{id}");
        }
        assert_eq!(normalize_model("gpt-5"), "gpt-5");
    }

    #[test]
    fn the_user_file_is_optional_merged_and_reported_when_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prices.toml");
        let (table, state) = PriceTable::with_user_file(&path);
        assert_eq!(state, UserPrices::Absent);
        assert_eq!(table.version(), PriceTable::bundled().version());
        std::fs::write(
            &path,
            "[models.claude-opus-5-5]\ninput = 3\noutput = 15\ncache_read = 0.1\n",
        )
        .unwrap();
        let (table, state) = PriceTable::with_user_file(&path);
        assert_eq!(state, UserPrices::Loaded);
        let cost = table
            .cost("claude-opus-5-5", &tokens(1_000_000, 0, 0, 0, 0))
            .unwrap();
        assert!(close(cost, 3.0), "{cost}");
        assert!(table.knows("claude-haiku-5-5"));
        std::fs::write(
            &path,
            "[models.x]\ninput = -1\noutput = 1\ncache_read = 1\n",
        )
        .unwrap();
        let (table, state) = PriceTable::with_user_file(&path);
        assert!(matches!(state, UserPrices::Invalid(_)));
        assert_eq!(table.version(), PriceTable::bundled().version());
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
    fn cache_read_exceptions_are_pinned() {
        let table = PriceTable::bundled();
        let reads = tokens(0, 0, 1_000_000, 0, 0);
        let sonnet = table.cost("claude-sonnet-5-5", &reads).unwrap();
        assert!(close(sonnet, 0.10), "{sonnet}");
        let fable = table.cost("claude-fable-5-1", &reads).unwrap();
        assert!(close(fable, 0.25), "{fable}");
    }

    #[test]
    fn haiku_5_5_tier_counts_cache_reads_and_writes() {
        let table = PriceTable::bundled();
        let below = tokens(50_000, 0, 40_000, 10_000, 0);
        let cost = table.cost("claude-haiku-5-5", &below).unwrap();
        let base = (50_000.0 * 0.1 + 40_000.0 * 0.01 + 10_000.0 * 0.1 * 1.25) / 1e6;
        assert!(close(cost, base), "{cost}");
        let above = tokens(50_000, 0, 40_000, 10_001, 0);
        let cost = table.cost("claude-haiku-5-5", &above).unwrap();
        let tiered = (50_000.0 * 0.5 + 40_000.0 * 0.05 + 10_001.0 * 0.5 * 1.25) / 1e6;
        assert!(close(cost, tiered), "{cost}");
    }

    #[test]
    fn overrides_can_change_cache_write_multipliers() {
        let overrides =
            PriceTable::from_toml("version = \"local\"\ncache_write_1h_multiplier = 3.0\n")
                .unwrap();
        let table = PriceTable::bundled().merged_with(overrides);
        let cost = table
            .cost("claude-opus-5-5", &tokens(0, 0, 0, 1_000_000, 1_000_000))
            .unwrap();
        assert!(close(cost, 4.0 * 1.25 + 4.0 * 3.0), "{cost}");
    }

    #[test]
    fn overrides_without_multipliers_keep_the_bundled_ones() {
        let custom = PriceTable::from_toml(
            "version = \"custom\"\ncache_write_1h_multiplier = 4.0\n[models.m]\ninput = 1\noutput = 1\ncache_read = 1\n",
        )
        .unwrap()
        .merged_with(PriceTable::from_toml("version = \"local\"\n").unwrap());
        let cost = custom.cost("m", &tokens(0, 0, 0, 0, 1_000_000)).unwrap();
        assert!(close(cost, 4.0), "{cost}");
    }

    #[test]
    fn invalid_rates_are_rejected() {
        for (field, value, shown) in [
            ("input", "-1.0", "-1"),
            ("output", "nan", "NaN"),
            ("cache_read", "inf", "inf"),
        ] {
            let source = "version = \"bad\"\n[models.m]\ninput = 1\noutput = 1\ncache_read = 1\n"
                .replace(&format!("{field} = 1"), &format!("{field} = {value}"));
            let error = PriceTable::from_toml(&source).unwrap_err();
            assert_eq!(
                format!("{error:#}"),
                format!(
                    "invalid price table: m {field} must be a finite non-negative rate, got {shown}"
                )
            );
        }
        let tier = "version = \"bad\"\n[models.m]\ninput = 1\noutput = 1\ncache_read = 1\n[[models.m.tiers]]\nabove_prompt_tokens = 10\ninput = 1\noutput = -2.0\ncache_read = 1\n";
        assert_eq!(
            format!("{:#}", PriceTable::from_toml(tier).unwrap_err()),
            "invalid price table: m tier above 10 output must be a finite non-negative rate, got -2"
        );
        let multiplier = "version = \"bad\"\ncache_write_5m_multiplier = -1.0\n";
        assert_eq!(
            format!("{:#}", PriceTable::from_toml(multiplier).unwrap_err()),
            "invalid price table: cache_write_5m_multiplier must be a finite non-negative rate, got -1"
        );
    }

    #[test]
    fn bundled_table_passes_validation() {
        assert!(PriceTable::from_toml(BUNDLED).is_ok());
    }

    #[test]
    fn base_cost_ignores_tiers() {
        let table = PriceTable::bundled();
        let large = tokens(100_001, 10, 0, 0, 0);
        let base = table.base_cost("claude-haiku-5-5", &large).unwrap();
        assert!(close(base, (100_001.0 * 0.1 + 10.0 * 0.5) / 1e6), "{base}");
        assert_eq!(
            table.base_cost("claude-sonnet-5-5", &tokens(5, 200, 0, 1000, 0)),
            table.cost("claude-sonnet-5-5", &tokens(5, 200, 0, 1000, 0))
        );
        assert_eq!(table.base_cost("gpt-9", &large), None);
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
