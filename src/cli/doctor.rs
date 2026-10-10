use anyhow::Result;

use super::format::{plural, usd};
use super::labeled;
use crate::ingest::discover;
use crate::paths::Paths;
use crate::pricing::PriceTable;

pub fn run(paths: &Paths, prices: &PriceTable) -> Result<()> {
    let files = discover(&paths.projects_dir())?;
    println!("✻ claude-plus doctor");
    println!(
        "{}",
        labeled(
            "Claude dir:",
            &format!(
                "{} · {}",
                paths.claude_dir.display(),
                plural(files.len() as u64, "transcript file", "transcript files")
            )
        )
    );
    println!(
        "{}",
        labeled("Data dir:", &paths.data_dir.display().to_string())
    );
    let (retention, hint) = retention(paths);
    println!("{}", labeled("Retention:", &retention));
    if let Some(hint) = hint {
        println!("{}", labeled("", hint));
    }
    println!(
        "{}",
        labeled(
            "Price table:",
            &format!("bundled prices {}", prices.version())
        )
    );

    let store = super::open_store(paths)?;
    let totals = store.totals()?;
    let imported = format!(
        "{} in {} · {}",
        plural(totals.requests, "request", "requests"),
        plural(store.file_count()?, "file", "files"),
        plural(
            store.parse_error_total()?,
            "unparsable line",
            "unparsable lines"
        )
    );
    println!("{}", labeled("Imported:", &imported));
    for (model, requests) in store.unpriced_models()? {
        let value = format!("{model} ({})", plural(requests, "request", "requests"));
        println!("{}", labeled("Unpriced:", &value));
    }

    let deltas = store.cost_state_deltas()?;
    let comparable: Vec<_> = deltas.iter().filter(|d| d.claude_usd > 0.0).collect();
    let ours: f64 = comparable.iter().map(|d| d.ours_usd).sum();
    let claude: f64 = comparable.iter().map(|d| d.claude_usd).sum();
    let cross_check = format!(
        "{} · ours {} · Claude Code {}",
        plural(comparable.len() as u64, "session", "sessions"),
        usd(ours),
        usd(claude)
    );
    println!("{}", labeled("Cross-check:", &cross_check));
    Ok(())
}

fn retention(paths: &Paths) -> (String, Option<&'static str>) {
    let configured = std::fs::read_to_string(paths.settings_file())
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|settings| {
            settings
                .get("cleanupPeriodDays")
                .and_then(serde_json::Value::as_u64)
        });
    match configured {
        Some(days) => (format!("cleanupPeriodDays: {days}"), None),
        None => (
            "cleanupPeriodDays: 30 (default)".to_string(),
            Some("Claude Code deletes older transcripts, claude-plus keeps its own copy"),
        ),
    }
}
