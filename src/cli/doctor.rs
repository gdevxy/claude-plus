use anyhow::Result;

use super::format::{duration, plural, usd};
use super::labeled;
use crate::api;
use crate::daemon::control::{self, Status};
use crate::ingest::discover;
use crate::paths::Paths;
use crate::pricing::PriceTable;
use crate::setup::bundle::{self, PLUGIN_ID};
use crate::setup::claude::{self, ClaudeCode, Plugin};
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
    let (line, status) = daemon(paths);
    println!("{}", labeled("Daemon:", &line));
    let (claude_code, plus) = setup(&ClaudeCode::default());
    println!("{}", labeled("Claude Code:", &claude_code));
    println!("{}", labeled("Mod:", &plus));

    match readable_store(paths)? {
        Some(store) => print_database(&store, &status),
        None => Ok(()),
    }
}

fn daemon(paths: &Paths) -> (String, Status) {
    if let Some(problem) = api::socket_path_problem(&paths.socket()) {
        return (format!("cannot run, {problem}"), Status::NotRunning);
    }
    let status = match control::status(paths) {
        Ok(status) => status,
        Err(error) => return (format!("{error:#}"), Status::NotRunning),
    };
    let line = match &status {
        Status::Running(health) => format!(
            "running · pid {} · up {}",
            health.pid,
            duration(health.uptime_s)
        ),
        Status::NotRunning => "not running, start it with claude-plus daemon --ensure".into(),
        Status::Unresponsive { pid } => format!(
            "{} does not answer, see {}",
            pid.map_or("the lock holder".to_string(), |pid| format!("pid {pid}")),
            paths.daemon_log().display()
        ),
    };
    (line, status)
}

fn setup(claude: &ClaudeCode) -> (String, String) {
    let version = match claude.version() {
        Ok(Some(version)) => version,
        Ok(None) => {
            return (
                format!(
                    "not found on PATH, the pane needs {} or newer",
                    claude::MIN_VERSION
                ),
                "unknown without Claude Code".into(),
            );
        }
        Err(error) => return (format!("{error:#}"), "unknown".into()),
    };
    let plugin = claude.plugins().map(|plugins| {
        plugins
            .into_iter()
            .find(|plugin| plugin.id == PLUGIN_ID && plugin.scope == "user")
    });
    let plus = match plugin {
        Ok(plugin) => mod_state(plugin.as_ref()),
        Err(error) => format!("{error:#}"),
    };
    (claude_code_state(&version), plus)
}

fn claude_code_state(version: &str) -> String {
    if control::is_older(version, claude::MIN_VERSION) {
        format!(
            "{version}, older than the {} the pane needs, run claude update",
            claude::MIN_VERSION
        )
    } else {
        version.to_string()
    }
}

fn mod_state(plugin: Option<&Plugin>) -> String {
    let Some(plugin) = plugin else {
        return "not installed, run claude-plus install".into();
    };
    let version = plugin.loaded_version();
    let from = plugin
        .folder
        .as_deref()
        .map(|folder| format!(" · {folder}"))
        .unwrap_or_default();
    if !plugin.enabled {
        format!("plus v{version} is disabled, run claude-plus install to enable it")
    } else if version != bundle::VERSION {
        format!(
            "plus v{version}, run claude-plus install to match this v{}",
            bundle::VERSION
        )
    } else {
        format!("plus v{version}{from}")
    }
}

fn reindex_state(pending: bool, daemon: &Status) -> Option<&'static str> {
    match daemon {
        _ if !pending => None,
        Status::Running(health) if health.ingest.rebuilding => Some("running in the daemon"),
        _ => Some("interrupted, the next import finishes it"),
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
    Ok(Some(store))
}

fn print_database(store: &Store, daemon: &Status) -> Result<()> {
    let reconciliation = store.reconciliation_summary()?;
    let totals = store.totals()?;
    let files = store.file_counts()?;
    let retired = match files.retired {
        0 => String::new(),
        retired => format!(" ({retired} retired)"),
    };
    let imported = format!(
        "{} in {}{retired} · {}",
        plural(totals.requests, "request", "requests"),
        plural(files.live, "file", "files"),
        plural(
            store.parse_error_total()?,
            "unparsable line",
            "unparsable lines"
        )
    );
    println!("{}", labeled("Imported:", &imported));
    if let Some(state) = reindex_state(store.reindex_pending()?, daemon) {
        println!("{}", labeled("Reindex:", state));
    }
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

const RETENTION_HINT: &str =
    "Claude Code deletes older transcripts, claude-plus keeps its own copy";

fn retention(paths: &Paths) -> (String, Option<&'static str>) {
    let configured = paths.retention_days();
    let days = configured.unwrap_or(Paths::DEFAULT_RETENTION_DAYS);
    let value = match configured {
        Some(days) => format!("cleanupPeriodDays: {days}"),
        None => format!("cleanupPeriodDays: {days} (default)"),
    };
    let hint = (days <= Paths::DEFAULT_RETENTION_DAYS).then_some(RETENTION_HINT);
    (value, hint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::wire::{Health, IngestHealth};

    fn running(rebuilding: bool) -> Status {
        Status::Running(Health {
            v: 1,
            version: "0.1.0".into(),
            schema: 8,
            pid: 42,
            uptime_s: 1,
            claude_dir: "/c".into(),
            ingest: IngestHealth {
                lag_ms: 0,
                backlog: 0,
                last_ingest_ms: None,
                progress: None,
                watching: true,
                last_error: None,
                last_error_at_ms: None,
                rebuilding,
            },
        })
    }

    fn plus(version: &str, enabled: bool) -> Plugin {
        Plugin {
            id: PLUGIN_ID.into(),
            version: "0.0.1".into(),
            scope: "user".into(),
            enabled,
            folder: Some("/home/ada/.claude-plus/marketplace/mod".into()),
            folder_version: Some(version.into()),
        }
    }

    #[test]
    fn the_mod_line_says_what_loads_and_how_to_fix_it() {
        let this = bundle::VERSION;
        assert_eq!(
            mod_state(Some(&plus(this, true))),
            format!("plus v{this} · /home/ada/.claude-plus/marketplace/mod")
        );
        assert_eq!(mod_state(None), "not installed, run claude-plus install");
        assert_eq!(
            mod_state(Some(&plus(this, false))),
            format!("plus v{this} is disabled, run claude-plus install to enable it")
        );
        assert_eq!(
            mod_state(Some(&plus("0.0.9", true))),
            format!("plus v0.0.9, run claude-plus install to match this v{this}")
        );
    }

    #[test]
    fn an_old_claude_code_is_named_with_the_fix() {
        assert_eq!(claude_code_state("2.1.296"), "2.1.296");
        assert_eq!(claude_code_state("2.2.0"), "2.2.0");
        assert_eq!(
            claude_code_state("2.1.200"),
            "2.1.200, older than the 2.1.296 the pane needs, run claude update"
        );
    }

    #[test]
    fn a_pending_reindex_is_running_in_an_answering_daemon_and_interrupted_otherwise() {
        assert_eq!(reindex_state(false, &running(false)), None);
        assert_eq!(
            reindex_state(true, &running(true)),
            Some("running in the daemon")
        );
        assert_eq!(
            reindex_state(true, &Status::NotRunning),
            Some("interrupted, the next import finishes it")
        );
        assert_eq!(
            reindex_state(true, &Status::Unresponsive { pid: Some(7) }),
            Some("interrupted, the next import finishes it")
        );
    }
}
