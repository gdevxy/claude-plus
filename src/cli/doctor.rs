use std::path::Path;

use anyhow::Result;

use super::format::{count, duration, plural, usd};
use super::labeled;
use super::setup::path_warning;
use crate::accounts::{Login, read_login};
use crate::api;
use crate::daemon::control::{self, Status};
use crate::desktop::DesktopReader;
use crate::ingest::discover;
use crate::paths::Paths;
use crate::pricing::{PriceTable, UserPrices};
use crate::setup::bundle::{self, PLUGIN_ID};
use crate::setup::claude::{self, ClaudeCode, Plugin};
use crate::store::Store;

pub fn run(paths: &Paths, user_prices: &UserPrices) -> Result<()> {
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
    let (line, status) = daemon(paths);
    println!("{}", labeled("Daemon:", &line));
    let socket_ok = api::socket_path_problem(&paths.socket()).is_none();
    let pane_runs_this = std::env::current_exe()
        .is_ok_and(|exe| path_warning(&exe, std::env::var_os("PATH")).is_none());
    let (claude_code, plugin) = setup(&ClaudeCode::default());
    let plus = match &plugin {
        Ok(plugin) => mod_state(plugin.as_ref(), &paths.marketplace_dir(), pane_runs_this),
        Err(problem) => problem.clone(),
    };
    println!("{}", labeled("Claude Code:", &claude_code));
    println!("{}", labeled("Mod:", &plus));
    println!("{}", labeled("Account:", &account_line(paths)));
    println!("{}", labeled("Login:", &login_line(&paths.claude_json)));
    println!(
        "{}",
        labeled(
            "Prices:",
            &prices_line(PriceTable::bundled().version(), user_prices)
        )
    );
    println!(
        "{}",
        labeled("Desktop:", &desktop_line(&paths.desktop_sessions_dir()))
    );

    let upgraded_by_sessions = sessions_upgrade(
        plugin.ok().flatten().as_ref(),
        &status,
        socket_ok,
        pane_runs_this,
    );
    match readable_store(paths, upgraded_by_sessions)? {
        Some(store) => print_database(&store, &status),
        None => Ok(()),
    }
}

fn account_line(paths: &Paths) -> String {
    Store::open_read_only(&paths.database())
        .ok()
        .and_then(|store| {
            let id = super::accounts::current(&store).ok()??;
            super::accounts::described(&store, &id).ok()
        })
        .unwrap_or_else(|| "none seen yet".into())
}

fn login_line(claude_json: &Path) -> String {
    match read_login(claude_json) {
        Ok(Login::SignedIn(_)) => "signed in".into(),
        Ok(Login::SignedOut) => "signed out".into(),
        Err(error) if is_not_found(&error) => "no .claude.json yet (never signed in)".into(),
        Err(error) => format!(
            "cannot read {}: {}",
            claude_json.display(),
            error.root_cause()
        ),
    }
}

fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .root_cause()
        .downcast_ref::<std::io::Error>()
        .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
}

fn desktop_line(dir: &Path) -> String {
    let scan = DesktopReader::default().scan(dir);
    if !scan.found {
        return match scan.problem {
            Some(problem) => format!("cannot read {problem}"),
            None => format!("none found in {}", dir.display()),
        };
    }
    let mut line = format!(
        "{} · {}",
        dir.display(),
        plural(scan.files_read as u64, "session file", "session files")
    );
    if !scan.skipped.is_empty() {
        line.push_str(&format!(" · {} skipped", count(scan.skipped.len() as u64)));
    }
    if let Some(problem) = scan.problem {
        line.push_str(&format!(" · cannot read {problem}"));
    }
    line
}

fn prices_line(bundled_version: &str, user_prices: &UserPrices) -> String {
    let anthropic = format!("Anthropic's {bundled_version}");
    match user_prices {
        UserPrices::Absent => anthropic,
        UserPrices::Loaded => format!("{anthropic}, with your prices.toml"),
        UserPrices::Invalid(message) => format!("{anthropic}; {message}"),
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

fn setup(claude: &ClaudeCode) -> (String, Result<Option<Plugin>, String>) {
    let version = match claude.version() {
        Ok(Some(version)) => version,
        Ok(None) => {
            return (
                format!(
                    "not found on PATH, the pane needs {} or newer",
                    claude::MIN_VERSION
                ),
                Err("unknown without Claude Code".into()),
            );
        }
        Err(error) => return (format!("{error:#}"), Err("unknown".into())),
    };
    let plugin = claude
        .plugins()
        .map(|plugins| {
            plugins
                .into_iter()
                .find(|plugin| plugin.id == PLUGIN_ID && plugin.scope == "user")
        })
        .map_err(|error| format!("{error:#}"));
    (claude_code_state(&version), plugin)
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

const UPDATED_BY: &str = "the next Claude Code session or claude-plus daemon --ensure";

fn mod_state(plugin: Option<&Plugin>, marketplace: &Path, pane_runs_this: bool) -> String {
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
    } else if control::is_older(version, bundle::VERSION)
        && pane_runs_this
        && plugin.reads_from(marketplace)
    {
        format!(
            "plus v{version}, {UPDATED_BY} updates it to v{}",
            bundle::VERSION
        )
    } else if control::is_older(bundle::VERSION, version) {
        format!(
            "plus v{version} is newer than this claude-plus v{}, upgrade claude-plus",
            bundle::VERSION
        )
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

fn sessions_upgrade(
    plugin: Option<&Plugin>,
    daemon: &Status,
    socket_ok: bool,
    pane_runs_this: bool,
) -> bool {
    plugin.is_some_and(|plugin| plugin.enabled)
        && socket_ok
        && pane_runs_this
        && !matches!(daemon, Status::Unresponsive { .. })
}

fn schema_line(version: usize, upgraded_by_sessions: bool) -> String {
    if upgraded_by_sessions {
        format!("schema v{version}, {UPDATED_BY} upgrades it")
    } else {
        format!("schema v{version}, run claude-plus import to upgrade")
    }
}

fn readable_store(paths: &Paths, upgraded_by_sessions: bool) -> Result<Option<Store>> {
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
        println!(
            "{}",
            labeled("Database:", &schema_line(version, upgraded_by_sessions))
        );
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
    use std::os::unix::fs::PermissionsExt;

    fn desktop_file(root: &Path, account: &str, name: &str) {
        let dir = root
            .join(account)
            .join("7d3f0c2e-8b4a-4e6f-a1c9-0123456789ab");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), r#"{"cliSessionId":"s"}"#).unwrap();
    }

    #[test]
    fn a_partly_unreadable_desktop_folder_keeps_its_counts_and_names_what_it_cannot_read() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("sessions");
        let locked = "e08c8984-5a1e-4c1b-9d2f-0123456789ab";
        desktop_file(&root, "0a0b0c0d-1111-4222-8333-444455556666", "a.json");
        desktop_file(&root, "0a0b0c0d-1111-4222-8333-444455556666", "b.json");
        desktop_file(&root, locked, "c.json");
        let locked = root.join(locked);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let line = desktop_line(&root);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            line.starts_with(&format!(
                "{} · 2 session files · cannot read {}: ",
                root.display(),
                locked.display()
            )),
            "{line}"
        );
    }

    #[test]
    fn an_unreadable_desktop_folder_is_named_with_its_error() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("Claude");
        let root = parent.join("claude-code-sessions");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o000)).unwrap();
        let line = desktop_line(&root);
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            line.starts_with(&format!("cannot read {}: ", root.display())),
            "{line}"
        );
    }

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
        let ours = Path::new("/home/ada/.claude-plus/marketplace");
        assert_eq!(
            mod_state(Some(&plus(this, true)), ours, true),
            format!("plus v{this} · /home/ada/.claude-plus/marketplace/mod")
        );
        assert_eq!(
            mod_state(None, ours, true),
            "not installed, run claude-plus install"
        );
        assert_eq!(
            mod_state(Some(&plus("0.0.9", false)), ours, true),
            "plus v0.0.9 is disabled, run claude-plus install to enable it"
        );
        assert_eq!(
            mod_state(Some(&plus("0.0.9", true)), ours, true),
            format!(
                "plus v0.0.9, the next Claude Code session or claude-plus daemon --ensure updates it to v{this}"
            )
        );
        let install = format!("plus v0.0.9, run claude-plus install to match this v{this}");
        assert_eq!(
            mod_state(Some(&plus("0.0.9", true)), Path::new("/elsewhere"), true),
            install
        );
        assert_eq!(mod_state(Some(&plus("0.0.9", true)), ours, false), install);
        let unfolded = Plugin {
            folder: None,
            ..plus("0.0.9", true)
        };
        assert_eq!(mod_state(Some(&unfolded), ours, true), install);
        assert_eq!(
            mod_state(Some(&plus("999.0.0", true)), ours, true),
            format!("plus v999.0.0 is newer than this claude-plus v{this}, upgrade claude-plus")
        );
    }

    #[test]
    fn an_old_schema_is_left_to_the_next_session_only_when_one_would_upgrade_it() {
        assert_eq!(
            schema_line(18, true),
            "schema v18, the next Claude Code session or claude-plus daemon --ensure upgrades it"
        );
        assert_eq!(
            schema_line(18, false),
            "schema v18, run claude-plus import to upgrade"
        );
    }

    #[test]
    fn only_a_daemon_that_can_start_upgrades_the_store() {
        let enabled = Some(plus(bundle::VERSION, true));
        let disabled = Some(plus(bundle::VERSION, false));
        let not_running = Status::NotRunning;
        let stuck = Status::Unresponsive { pid: Some(7) };
        assert!(sessions_upgrade(
            enabled.as_ref(),
            &running(false),
            true,
            true
        ));
        assert!(sessions_upgrade(enabled.as_ref(), &not_running, true, true));
        assert!(!sessions_upgrade(enabled.as_ref(), &stuck, true, true));
        assert!(!sessions_upgrade(
            enabled.as_ref(),
            &not_running,
            false,
            true
        ));
        assert!(!sessions_upgrade(
            enabled.as_ref(),
            &not_running,
            true,
            false
        ));
        assert!(!sessions_upgrade(
            disabled.as_ref(),
            &not_running,
            true,
            true
        ));
        assert!(!sessions_upgrade(None, &not_running, true, true));
    }

    #[test]
    fn prices_and_login_lines_say_what_is_used() {
        assert_eq!(
            prices_line("2026-10-10", &UserPrices::Absent),
            "Anthropic's 2026-10-10"
        );
        assert_eq!(
            prices_line("2026-10-10", &UserPrices::Loaded),
            "Anthropic's 2026-10-10, with your prices.toml"
        );
        assert_eq!(
            prices_line("2026-10-10", &UserPrices::Invalid("bad".into())),
            "Anthropic's 2026-10-10; bad"
        );
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".claude.json");
        assert_eq!(login_line(&file), "no .claude.json yet (never signed in)");
        std::fs::write(&file, "{}").unwrap();
        assert_eq!(login_line(&file), "signed out");
        std::fs::write(&file, r#"{"oauthAccount":{"accountUuid":"w"}}"#).unwrap();
        assert_eq!(login_line(&file), "signed in");
        std::fs::write(&file, "{broken").unwrap();
        assert!(login_line(&file).starts_with("cannot read"));
        assert!(login_line(dir.path()).starts_with("cannot read"));
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
