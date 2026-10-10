use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenCounts {
    pub input: u64,
    pub output: u64,
    pub thinking: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
}

impl TokenCounts {
    pub fn from_usage(usage: &Value) -> Self {
        let total_write = number(usage, "/cache_creation_input_tokens");
        let write_1h = number(usage, "/cache_creation/ephemeral_1h_input_tokens");
        let write_5m = usage
            .pointer("/cache_creation/ephemeral_5m_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| total_write.saturating_sub(write_1h));
        Self {
            input: number(usage, "/input_tokens"),
            output: number(usage, "/output_tokens"),
            thinking: number(usage, "/output_tokens_details/thinking_tokens"),
            cache_read: number(usage, "/cache_read_input_tokens"),
            cache_write_5m: write_5m,
            cache_write_1h: write_1h,
        }
    }

    pub fn cache_write(&self) -> u64 {
        self.cache_write_5m.saturating_add(self.cache_write_1h)
    }

    pub fn prompt_total(&self) -> u64 {
        self.input
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write())
    }

    pub fn is_empty(&self) -> bool {
        self.prompt_total() == 0 && self.output == 0
    }
}

fn number(value: &Value, pointer: &str) -> u64 {
    value.pointer(pointer).and_then(Value::as_u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_split_cache_writes() {
        let usage = json!({
            "input_tokens": 2, "output_tokens": 201, "cache_read_input_tokens": 22518,
            "cache_creation_input_tokens": 20231,
            "cache_creation": {"ephemeral_1h_input_tokens": 20231, "ephemeral_5m_input_tokens": 0},
            "output_tokens_details": {"thinking_tokens": 143}
        });
        let counts = TokenCounts::from_usage(&usage);
        assert_eq!(
            counts,
            TokenCounts {
                input: 2,
                output: 201,
                thinking: 143,
                cache_read: 22518,
                cache_write_5m: 0,
                cache_write_1h: 20231,
            }
        );
        assert_eq!(counts.cache_write(), 20231);
        assert_eq!(counts.prompt_total(), 2 + 22518 + 20231);
    }

    #[test]
    fn unsplit_cache_writes_count_as_five_minutes() {
        let counts = TokenCounts::from_usage(&json!({"cache_creation_input_tokens": 500}));
        assert_eq!(counts.cache_write_5m, 500);
        assert_eq!(counts.cache_write_1h, 0);
    }

    #[test]
    fn cache_writes_saturate_instead_of_overflowing() {
        let counts = TokenCounts::from_usage(&json!({
            "cache_creation_input_tokens": u64::MAX,
            "cache_creation": {"ephemeral_1h_input_tokens": u64::MAX, "ephemeral_5m_input_tokens": u64::MAX}
        }));
        assert_eq!(counts.cache_write(), u64::MAX);
        assert_eq!(counts.prompt_total(), u64::MAX);
    }

    #[test]
    fn null_usage_fields_are_zero() {
        let counts =
            TokenCounts::from_usage(&json!({"input_tokens": null, "cache_creation": null}));
        assert!(counts.is_empty());
    }
}
