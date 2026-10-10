use std::time::Duration;

use anyhow::Result;

use super::format::duration;
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
        Ensured::Running(health) => println!(
            "✻ claude-plus daemon is running · pid {} · up {}",
            health.pid,
            duration(health.uptime_s)
        ),
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

pub fn stop(paths: &Paths) -> Result<()> {
    match control::stop(paths)? {
        Stopped::Stopped { pid } => println!("✻ Stopped the claude-plus daemon · pid {pid}"),
        Stopped::NotRunning => println!("No claude-plus daemon is running"),
    }
    Ok(())
}
