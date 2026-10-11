mod daemon;
mod doctor;
mod export;
pub mod format;
mod import;
mod reindex;
mod setup;
mod stats;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};

use crate::api::client::{Client, ClientError};
use crate::api::wire::Health;
use crate::daemon::control::{self, Status};
use crate::paths::Paths;
use crate::pricing::PriceTable;
use crate::store::Store;

const PROGRESS_POLL: Duration = Duration::from_millis(100);

#[derive(Parser)]
#[command(
    name = "claude-plus",
    version,
    about = "Token, cache and cost visibility for Claude Code"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Add the claude-plus pane to Claude Code and start the daemon")]
    Install {
        #[arg(short, long, help = "Agree to the Claude Code changes without asking")]
        yes: bool,
    },
    #[command(about = "Remove the claude-plus pane from Claude Code and stop the daemon")]
    Uninstall {
        #[arg(
            long,
            help = "Also delete the claude-plus data directory and the history it keeps"
        )]
        purge: bool,
        #[arg(short, long, help = "Agree to the changes without asking")]
        yes: bool,
    },
    #[command(about = "Read new Claude Code transcripts into the local store")]
    Import,
    #[command(about = "Show daily cost per model, importing first")]
    Stats {
        #[arg(
            long,
            default_value_t = 14,
            value_parser = clap::value_parser!(u32).range(1..=36500),
            help = "Number of calendar days to show, today included"
        )]
        days: u32,
    },
    #[command(about = "Print stored usage per day, project and model, importing first")]
    Export {
        #[arg(long, value_enum, help = "Output format")]
        format: export::Format,
        #[arg(
            long,
            value_parser = clap::value_parser!(u32).range(1..=36500),
            help = "Number of calendar days to include, today included [default: all]"
        )]
        days: Option<u32>,
    },
    #[command(about = "Check the setup and compare totals with Claude Code, read-only")]
    Doctor,
    #[command(about = "Rebuild every derived table from the stored records")]
    Reindex,
    #[command(
        about = "Run the daemon that keeps the store live, in the foreground",
        args_conflicts_with_subcommands = true
    )]
    Daemon {
        #[arg(long, help = "Start a detached daemon unless one answers, then return")]
        ensure: bool,
        #[arg(long, hide = true)]
        detached: bool,
        #[arg(long, hide = true, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..))]
        rescan_secs: u64,
        #[command(subcommand)]
        action: Option<DaemonAction>,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    #[command(about = "Stop the running daemon")]
    Stop,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    if !matches!(
        cli.command,
        Command::Daemon {
            action: None,
            ensure: false,
            ..
        }
    ) {
        end_quietly_when_the_reader_goes_away();
    }
    let paths = Paths::from_env()?;
    let prices = PriceTable::bundled();
    match cli.command {
        Command::Import => import::run(&paths, &prices),
        Command::Stats { days } => stats::run(&paths, &prices, days),
        Command::Export { format, days } => export::run(&paths, &prices, format, days),
        Command::Doctor => doctor::run(&paths, &prices),
        Command::Reindex => reindex::run(&paths, &prices),
        Command::Install { yes } => setup::install(&paths, yes),
        Command::Uninstall { purge, yes } => setup::uninstall(&paths, purge, yes),
        Command::Daemon {
            action: Some(DaemonAction::Stop),
            ..
        } => daemon::stop(&paths),
        Command::Daemon { ensure: true, .. } => daemon::ensure(&paths),
        Command::Daemon {
            detached,
            rescan_secs,
            ..
        } => daemon::run(&paths, prices, detached, rescan_secs),
    }
}

fn end_quietly_when_the_reader_goes_away() {
    // SAFETY: restoring the default action of SIGPIPE has no preconditions.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

fn progress_bar(verb: &str) -> Result<ProgressBar> {
    Ok(
        ProgressBar::new(0).with_style(ProgressStyle::with_template(&format!(
            "✻ {verb} {{human_pos}}/{{human_len}} transcripts {{wide_bar}}"
        ))?),
    )
}

const LABEL_WIDTH: usize = 13;

fn labeled(label: &str, value: &str) -> String {
    format!("  {label:<LABEL_WIDTH$}{value}")
}

fn transcripts_missing(paths: &Paths) -> bool {
    let notice = missing_transcripts(paths);
    if let Some(notice) = &notice {
        println!("{notice}");
    }
    notice.is_some()
}

fn missing_transcripts(paths: &Paths) -> Option<String> {
    (!paths.projects_dir().exists()).then(|| {
        format!(
            "No Claude Code transcripts found in {}. Run Claude Code once, then try again ✻",
            paths.projects_dir().display()
        )
    })
}

fn running_daemon(paths: &Paths) -> Result<Option<Health>> {
    match control::status(paths)? {
        Status::NotRunning => Ok(None),
        Status::Unresponsive { pid } => {
            let who = pid.map(|pid| format!(" (pid {pid})")).unwrap_or_default();
            bail!(
                "the claude-plus daemon{who} does not answer; stop it with claude-plus daemon stop"
            )
        }
        Status::Running(health) if Path::new(&health.claude_dir) != paths.claude_dir => bail!(
            "the running claude-plus daemon reads {}, not {}; stop it with claude-plus daemon stop",
            health.claude_dir,
            paths.claude_dir.display()
        ),
        Status::Running(health) => Ok(Some(health)),
    }
}

fn through_daemon<T>(
    paths: &Paths,
    verb: &str,
    call: impl FnOnce(&Client) -> Result<T, ClientError>,
) -> Result<T> {
    let bar = progress_bar(verb)?;
    let finished = AtomicBool::new(false);
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            let client = Client::new(&paths.socket());
            while !finished.load(Ordering::Relaxed) {
                if let Ok(Some(progress)) = client.health().map(|h| h.ingest.progress) {
                    bar.set_length(progress.total as u64);
                    bar.set_position(progress.done as u64);
                }
                std::thread::sleep(PROGRESS_POLL);
            }
        });
        let result = call(&Client::new(&paths.socket()));
        finished.store(true, Ordering::Relaxed);
        result
    });
    bar.finish_and_clear();
    Ok(result?)
}

fn open_store(paths: &Paths) -> Result<Store> {
    paths.ensure_data_dir()?;
    Store::open(&paths.database())
}
