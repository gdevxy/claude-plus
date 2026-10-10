use std::time::Duration;

use anyhow::Result;

use super::format::duration;
use crate::api::wire::Health;
use crate::daemon::control::{self, Ensured, Stopped};
use crate::daemon::{self as service, Mode};
use crate::paths::Paths;
use crate::pricing::PriceTable;

pub fn run(paths: &Paths, prices: PriceTable, detached: bool, rescan_secs: u64) -> Result<()> {
    let mode = if detached {
        Mode::Detached
    } else {
        Mode::Foreground
    };
    service::run(paths, prices, mode, Duration::from_secs(rescan_secs))
}

pub fn ensure(paths: &Paths) -> Result<()> {
    match control::ensure(paths)? {
        Ensured::Running(health) => println!("{}", running(&health)),
        Ensured::KeptNewer(health) => {
            println!("{}", running(&health));
            println!(
                "  It runs v{}, newer than this claude-plus v{}",
                health.version,
                control::VERSION
            );
        }
        Ensured::Started(health) => {
            println!("✻ Started the claude-plus daemon · pid {}", health.pid)
        }
        Ensured::Restarted { previous, health } => println!(
            "✻ Restarted the claude-plus daemon on v{} (was v{previous}) · pid {}",
            health.version, health.pid
        ),
    }
    Ok(())
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

fn stopped(stopped: Stopped) -> String {
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
