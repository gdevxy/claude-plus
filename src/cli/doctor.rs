use anyhow::Result;

use super::format::{plural, usd};
use super::labeled;
use crate::ingest::discover;
use crate::paths::Paths;
use crate::pricing::PriceTable;
use crate::store::Store;

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

    match readable_store(paths)? {
        Some(store) => print_database(&store),
        None => Ok(()),
    }
}

fn readable_store(paths: &Paths) -> Result<Option<Store>> {
    let database = paths.database();
    if !database.exists() {
        println!(
            "{}",
            labeled("Database:", "not created yet, run claude-plus import")
        );
        return Ok(None);
    }
    let store = match Store::open_read_only(&database) {
        Ok(store) => store,
        Err(error) => {
            println!("{}", labeled("Database:", &format!("{error:#}")));
            return Ok(None);
        }
    };
    let version = store.schema_version()?;
    if version < Store::latest_schema_version() {
        let value = format!("schema v{version}, run claude-plus import to upgrade");
        println!("{}", labeled("Database:", &value));
        return Ok(None);
    }
    if store.rederive_pending()? {
        println!(
            "{}",
            labeled("Database:", "rebuild pending, run claude-plus import")
        );
        return Ok(None);
    }
    Ok(Some(store))
}

fn print_database(store: &Store) -> Result<()> {
    let reconciliation = store.reconciliation_summary()?;
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
    let unpriced_requests = store.unpriced_models()?;
    for (model, requests) in &unpriced_requests {
        let value = format!("{model} ({})", plural(*requests, "request", "requests"));
        println!("{}", labeled("Unpriced:", &value));
    }
    let counter_only = reconciliation
        .unpriced_models
        .iter()
        .filter(|model| !unpriced_requests.iter().any(|(listed, _)| listed == *model));
    for model in counter_only {
        let value = format!("{model} (Claude Code counter only)");
        println!("{}", labeled("Unpriced:", &value));
    }

    let deltas = store.cost_state_deltas()?;
    let comparable: Vec<_> = deltas
        .iter()
        .filter_map(|d| Some((d, d.claude_usd.filter(|usd| *usd > 0.0)?)))
        .collect();
    let transcripts: f64 = comparable.iter().map(|(d, _)| d.ours_usd).sum();
    let claude: f64 = comparable.iter().map(|(_, claude)| claude).sum();
    let reconciled: f64 = comparable
        .iter()
        .map(|(d, _)| d.ours_usd + d.reconciled_usd)
        .sum();
    let cross_check = format!(
        "{} · transcripts {} · Claude Code {} · reconciled {}",
        plural(comparable.len() as u64, "session", "sessions"),
        usd(transcripts),
        usd(claude),
        usd(reconciled)
    );
    println!("{}", labeled("Cross-check:", &cross_check));
    let uncounted = reconciliation.sessions_without_counter;
    if uncounted > 0 {
        let value = format!(
            "{}, {} subagent output may be understated",
            plural(
                uncounted,
                "session with subagents has no Claude Code counter",
                "sessions with subagents have no Claude Code counter"
            ),
            if uncounted == 1 { "its" } else { "their" }
        );
        println!("{}", labeled("Uncounted:", &value));
    }
    if reconciliation.sessions_over > 0 {
        let value = plural(
            reconciliation.sessions_over,
            "session where transcripts exceed Claude Code's counter",
            "sessions where transcripts exceed Claude Code's counter",
        );
        println!("{}", labeled("Over:", &value));
    }
    Ok(())
}

const DEFAULT_RETENTION_DAYS: u64 = 30;
const RETENTION_HINT: &str =
    "Claude Code deletes older transcripts, claude-plus keeps its own copy";

fn retention(paths: &Paths) -> (String, Option<&'static str>) {
    let configured = std::fs::read_to_string(paths.settings_file())
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|settings| {
            settings
                .get("cleanupPeriodDays")
                .and_then(serde_json::Value::as_u64)
        });
    let days = configured.unwrap_or(DEFAULT_RETENTION_DAYS);
    let value = match configured {
        Some(days) => format!("cleanupPeriodDays: {days}"),
        None => format!("cleanupPeriodDays: {days} (default)"),
    };
    let hint = (days <= DEFAULT_RETENTION_DAYS).then_some(RETENTION_HINT);
    (value, hint)
}
