use std::time::Duration;

use anyhow::Result;

use super::format::duration;
use crate::api::wire::Health;
use crate::daemon::control::{self, Ensured, Stopped};
use crate::daemon::{self as service, Mode};
use crate::paths::Paths;
use crate::pricing::PriceTable;
use crate::setup::bundle;

pub fn run(paths: &Paths, prices: PriceTable, detached: bool, rescan_secs: u64) -> Result<()> {
    let mode = if detached {
        Mode::Detached
    } else {
        Mode::Foreground
    };
    service::run(paths, prices, mode, Duration::from_secs(rescan_secs))
}

pub fn ensure(paths: &Paths) -> Result<()> {
    match bundle::refresh(&paths.marketplace_dir()) {
        Ok(Some(from)) => println!("{}", refreshed(&from)),
        Ok(None) => {}
        Err(error) => eprintln!("claude-plus: could not update the plus mod: {error:#}"),
    }
    for line in ensured(&control::ensure(paths)?) {
        println!("{line}");
    }
    Ok(())
}

fn refreshed(from: &str) -> String {
    let what = if from == bundle::VERSION {
        format!("v{from} to this build")
    } else {
        format!("from v{from} to v{}", bundle::VERSION)
    };
    format!("✻ Updated the plus mod {what} · new Claude Code sessions load it")
}

pub fn ensured(ensured: &Ensured) -> Vec<String> {
    match ensured {
        Ensured::Running(health) => vec![running(health)],
        Ensured::KeptNewer(health) => vec![
            running(health),
            format!(
                "  It runs v{}, newer than this claude-plus v{}",
                health.version,
                control::VERSION
            ),
        ],
        Ensured::Started(health) => {
            vec![format!(
                "✻ Started the claude-plus daemon · pid {}",
                health.pid
            )]
        }
        Ensured::Restarted { previous, health } => vec![restarted(previous, health)],
    }
}

fn restarted(previous: &Health, health: &Health) -> String {
    let was = if previous.version == health.version {
        "an older build".to_string()
    } else {
        format!("v{}", previous.version)
    };
    format!(
        "✻ Restarted the claude-plus daemon on v{} (was {was}) · pid {}",
        health.version, health.pid
    )
}

fn running(health: &Health) -> String {
    format!(
        "✻ claude-plus daemon is running · pid {} · up {}",
        health.pid,
        duration(health.uptime_s)
    )
}

pub fn stop(paths: &Paths) -> Result<()> {
    println!("{}", stopped(control::stop(paths)?));
    Ok(())
}

pub fn stopped(stopped: Stopped) -> String {
    match stopped {
        Stopped::Stopped { pid: Some(pid) } => {
            format!("✻ Stopped the claude-plus daemon · pid {pid}")
        }
        Stopped::Stopped { pid: None } => "✻ Stopped the claude-plus daemon".into(),
        Stopped::NotRunning => "No claude-plus daemon is running".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::wire::IngestHealth;

    fn health(version: &str, schema: usize, pid: u32) -> Health {
        Health {
            v: 1,
            version: version.into(),
            schema,
            pid,
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
                rebuilding: false,
            },
        }
    }

    #[test]
    fn a_restart_names_the_version_it_replaced() {
        assert_eq!(
            restarted(&health("0.1.0", 10, 7), &health("0.2.0", 10, 42)),
            "✻ Restarted the claude-plus daemon on v0.2.0 (was v0.1.0) · pid 42"
        );
        assert_eq!(
            restarted(&health("0.1.0", 9, 7), &health("0.1.0", 10, 42)),
            "✻ Restarted the claude-plus daemon on v0.1.0 (was an older build) · pid 42"
        );
    }

    #[test]
    fn a_refresh_names_the_version_or_the_build_it_replaced() {
        assert_eq!(
            refreshed("0.0.1"),
            format!(
                "✻ Updated the plus mod from v0.0.1 to v{} · new Claude Code sessions load it",
                bundle::VERSION
            )
        );
        assert_eq!(
            refreshed(bundle::VERSION),
            format!(
                "✻ Updated the plus mod v{} to this build · new Claude Code sessions load it",
                bundle::VERSION
            )
        );
    }

    #[test]
    fn a_stopped_daemon_is_named_by_its_pid_when_it_is_known() {
        assert_eq!(
            stopped(Stopped::Stopped { pid: Some(42) }),
            "✻ Stopped the claude-plus daemon · pid 42"
        );
        assert_eq!(
            stopped(Stopped::Stopped { pid: None }),
            "✻ Stopped the claude-plus daemon"
        );
        assert_eq!(
            stopped(Stopped::NotRunning),
            "No claude-plus daemon is running"
        );
    }
}
