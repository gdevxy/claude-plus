pub mod calendar;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Range {
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
    #[serde(rename = "90d")]
    Quarter,
}

impl Range {
    pub const ALL: [Range; 4] = [Range::Day, Range::Week, Range::Month, Range::Quarter];

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|range| range.as_str() == text)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Range::Day => "24h",
            Range::Week => "7d",
            Range::Month => "30d",
            Range::Quarter => "90d",
        }
    }

    pub fn days(self) -> u64 {
        match self {
            Range::Day => 1,
            Range::Week => 7,
            Range::Month => 30,
            Range::Quarter => 90,
        }
    }

    pub fn bucket(self) -> BucketSize {
        match self {
            Range::Day => BucketSize::Hour,
            _ => BucketSize::Day,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BucketSize {
    Hour,
    Day,
}
