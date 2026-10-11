use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::breakdown::Breakdown;
use crate::config_scan::{Loading, Scan, SourceKind};

pub const DEFAULT_PROSE: f64 = 3.5;
pub const DEFAULT_CODE: f64 = 3.0;
pub const DEFAULT_SCHEMA: f64 = 2.5;
pub const MIN_PAIR_TOKENS: u64 = 25;
pub const MIN_PAIRS: usize = 3;
pub const MIN_FACTOR: f64 = 0.5;
pub const MAX_FACTOR: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextKind {
    Prose,
    Code,
    Schema,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Ratios {
    pub prose: f64,
    pub code: f64,
    pub schema: f64,
    pub calibrated: bool,
    pub pairs: usize,
}

impl Default for Ratios {
    fn default() -> Self {
        Self {
            prose: DEFAULT_PROSE,
            code: DEFAULT_CODE,
            schema: DEFAULT_SCHEMA,
            calibrated: false,
            pairs: 0,
        }
    }
}

impl Ratios {
    pub fn of(&self, kind: TextKind) -> f64 {
        match kind {
            TextKind::Prose => self.prose,
            TextKind::Code => self.code,
            TextKind::Schema => self.schema,
        }
    }

    pub fn tokens(&self, chars: u64, kind: TextKind) -> u64 {
        (chars as f64 / self.of(kind)).round() as u64
    }
}

pub fn calibrate(scan: &Scan, recent_first: &[Breakdown]) -> Ratios {
    let mut pairs: BTreeMap<&str, f64> = BTreeMap::new();
    for file in recent_first.iter().flat_map(|b| &b.memory_files) {
        if file.tokens < MIN_PAIR_TOKENS || pairs.contains_key(file.path.as_str()) {
            continue;
        }
        let counted = scan.sources.iter().find(|source| {
            matches!(source.kind, SourceKind::ClaudeMd | SourceKind::Memory)
                && source.loading != Loading::Off
                && source.chars > 0
                && source.path.as_deref() == Some(file.path.as_str())
        });
        if let Some(source) = counted {
            pairs.insert(&file.path, source.chars as f64 / file.tokens as f64);
        }
    }
    if pairs.len() < MIN_PAIRS {
        return Ratios::default();
    }
    let mut ratios: Vec<f64> = pairs.into_values().collect();
    ratios.sort_by(f64::total_cmp);
    let middle = ratios.len() / 2;
    let median = if ratios.len().is_multiple_of(2) {
        (ratios[middle - 1] + ratios[middle]) / 2.0
    } else {
        ratios[middle]
    };
    let factor = (median / DEFAULT_PROSE).clamp(MIN_FACTOR, MAX_FACTOR);
    Ratios {
        prose: DEFAULT_PROSE * factor,
        code: DEFAULT_CODE * factor,
        schema: DEFAULT_SCHEMA * factor,
        calibrated: true,
        pairs: ratios.len(),
    }
}
#[cfg(test)]
mod tests {
    use super::super::breakdown::{Entry, MemoryFile};
    use super::*;
    use crate::config_scan::{Scope, Source};

    fn source(kind: SourceKind, name: &str, path: Option<&str>, chars: u64) -> Source {
        Source {
            kind,
            scope: Scope::User,
            name: name.into(),
            alias: None,
            path: path.map(String::from),
            plugin: None,
            loading: Loading::Listing,
            chars,
            file_chars: chars,
        }
    }

    fn entry(name: &str, tokens: u64) -> Entry {
        Entry {
            name: name.into(),
            source: "userSettings".into(),
            plugin: None,
            tokens,
        }
    }

    fn breakdown(memory: Vec<(&str, u64)>, skills: Vec<Entry>, agents: Vec<Entry>) -> Breakdown {
        Breakdown {
            ts_ms: 1,
            model: String::new(),
            total_tokens: 0,
            max_tokens: 0,
            percentage: 0.0,
            auto_compact_threshold: None,
            categories: vec![],
            memory_files: memory
                .into_iter()
                .map(|(path, tokens)| MemoryFile {
                    path: path.into(),
                    kind: "User".into(),
                    tokens,
                })
                .collect(),
            mcp_tools: vec![],
            agents,
            skills,
            skills_total: 0,
            skills_included: 0,
        }
    }

    fn scan() -> Scan {
        Scan {
            sources: vec![
                source(
                    SourceKind::ClaudeMd,
                    "CLAUDE.md",
                    Some("/h/.claude/CLAUDE.md"),
                    4000,
                ),
                source(
                    SourceKind::Memory,
                    "MEMORY.md",
                    Some("/h/.claude/projects/-w/memory/MEMORY.md"),
                    900,
                ),
                source(SourceKind::Skill, "tidy", None, 120),
                source(SourceKind::Skill, "deploy", None, 400),
                source(SourceKind::Agent, "reviewer", None, 300),
            ],
            memory_dirs: vec![],
        }
    }

    #[test]
    fn without_enough_matching_items_the_documented_defaults_apply() {
        let only_two = breakdown(
            vec![("/h/.claude/CLAUDE.md", 1000)],
            vec![entry("tidy", 30)],
            vec![],
        );
        assert_eq!(calibrate(&scan(), &[only_two]), Ratios::default());
        assert_eq!(calibrate(&scan(), &[]), Ratios::default());
        let ratios = Ratios::default();
        assert_eq!((ratios.prose, ratios.code, ratios.schema), (3.5, 3.0, 2.5));
        assert_eq!(ratios.tokens(3500, TextKind::Prose), 1000);
        assert_eq!(ratios.tokens(10, TextKind::Schema), 4);
    }

    fn memory_scan() -> Scan {
        let mut scan = scan();
        for (index, chars) in [(1, 1200), (2, 2000)] {
            scan.sources.push(source(
                SourceKind::ClaudeMd,
                "CLAUDE.md",
                Some(&format!("/w/{index}/CLAUDE.md")),
                chars,
            ));
        }
        scan
    }

    #[test]
    fn the_median_of_counted_memory_files_scales_every_kind_by_the_same_factor() {
        let latest = breakdown(
            vec![
                ("/h/.claude/CLAUDE.md", 1000),
                ("/w/1/CLAUDE.md", 300),
                ("/elsewhere/CLAUDE.md", 50),
                ("/h/.claude/projects/-w/memory/MEMORY.md", 5),
            ],
            vec![entry("tidy", 40), entry("deploy", 100)],
            vec![entry("reviewer", 75)],
        );
        let older = breakdown(
            vec![("/h/.claude/CLAUDE.md", 10), ("/w/2/CLAUDE.md", 400)],
            vec![entry("tidy", 1)],
            vec![],
        );
        let ratios = calibrate(&memory_scan(), &[latest, older]);
        assert!(ratios.calibrated);
        assert_eq!(ratios.pairs, 3);
        assert!((ratios.prose - 4.0).abs() < 1e-9, "{ratios:?}");
        assert!((ratios.code - 3.0 * 4.0 / 3.5).abs() < 1e-9);
        assert!((ratios.schema - 2.5 * 4.0 / 3.5).abs() < 1e-9);
    }

    #[test]
    fn skills_and_agents_are_claude_codes_own_estimates_and_never_calibrate() {
        let estimates = breakdown(
            vec![("/h/.claude/CLAUDE.md", 1000)],
            vec![entry("tidy", 40), entry("deploy", 100)],
            vec![entry("reviewer", 75)],
        );
        assert_eq!(calibrate(&scan(), &[estimates]), Ratios::default());
    }

    #[test]
    fn a_wild_calibration_is_held_to_half_or_double_the_default() {
        let wild = breakdown(
            vec![
                ("/h/.claude/CLAUDE.md", 100),
                ("/w/1/CLAUDE.md", 30),
                ("/w/2/CLAUDE.md", 30),
            ],
            vec![],
            vec![],
        );
        let ratios = calibrate(&memory_scan(), &[wild]);
        assert_eq!(ratios.prose, DEFAULT_PROSE * MAX_FACTOR);
    }
}
