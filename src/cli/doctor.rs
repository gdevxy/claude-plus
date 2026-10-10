use anyhow::Result;

use super::format::{plural, usd};
use crate::ingest::discover;
use crate::paths::Paths;
use crate::pricing::PriceTable;

pub fn run(paths: &Paths, prices: &PriceTable) -> Result<()> {
    let files = discover(&paths.projects_dir())?;
    println!("✻ claude-plus doctor");
    println!(
        "  Claude dir:  {} · {}",
        paths.claude_dir.display(),
        plural(files.len() as u64, "transcript file", "transcript files")
    );
    println!("  Data dir:    {}", paths.data_dir.display());
    println!("  Retention:   {}", retention(paths));
    println!("  Price table: bundled prices {}", prices.version());

    let store = super::open_store(paths)?;
    let totals = store.totals()?;
    println!(
        "  Imported:    {} in {} · {}",
        plural(totals.requests, "request", "requests"),
        plural(store.file_count()?, "file", "files"),
        plural(
            store.parse_error_total()?,
            "unparsable line",
            "unparsable lines"
        )
    );
    for (model, requests) in store.unpriced_models()? {
        println!(
            "  Unknown price: {model} ({})",
            plural(requests, "request", "requests")
        );
    }

    let deltas = store.cost_state_deltas()?;
    let comparable: Vec<_> = deltas.iter().filter(|d| d.claude_usd > 0.0).collect();
    let ours: f64 = comparable.iter().map(|d| d.ours_usd).sum();
    let claude: f64 = comparable.iter().map(|d| d.claude_usd).sum();
    println!(
        "  Claude Code cost cross-check: {} sessions · ours {} · Claude Code {}",
        comparable.len(),
        usd(ours),
        usd(claude)
    );
    Ok(())
}

fn retention(paths: &Paths) -> String {
    let configured = std::fs::read_to_string(paths.settings_file())
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|settings| {
            settings
                .get("cleanupPeriodDays")
                .and_then(serde_json::Value::as_u64)
        });
    match configured {
        Some(days) => format!("cleanupPeriodDays: {days}"),
        None => "cleanupPeriodDays: 30 (default)\n               Claude Code deletes older transcripts, claude-plus keeps its own copy".to_string(),
    }
}
