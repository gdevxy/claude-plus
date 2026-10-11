mod accounts;
mod daemon;
mod doctor;
mod export;
pub mod format;
mod import;
mod reindex;
mod setup;
mod stats;

use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};

use crate::api::client::{Client, ClientError};
use crate::api::wire::Health;
use crate::daemon::control::{self, Status, Stop};
use crate::paths::Paths;
use crate::pricing::{PriceTable, UserPrices};
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
        #[arg(
            long,
            help = "Account id or label, or all [default: the current account]"
        )]
        account: Option<String>,
    },
    #[command(about = "Print stored usage per day, project, account and model, importing first")]
    Export {
        #[arg(long, value_enum, help = "Output format")]
        format: export::Format,
        #[arg(
            long,
            value_parser = clap::value_parser!(u32).range(1..=36500),
            help = "Number of calendar days to include, today included [default: all]"
        )]
        days: Option<u32>,
        #[arg(
            long,
            help = "Account id or label, or all [default: the current account]"
        )]
        account: Option<String>,
    },
    #[command(about = "List the accounts claude-plus has seen, or rename one")]
    Accounts {
        #[command(subcommand)]
        action: Option<AccountsAction>,
    },
    #[command(about = "Set, show or clear a monthly budget for an account")]
    Budget {
        #[command(subcommand)]
        action: BudgetAction,
    },
    #[command(
        about = "Check the setup and compare totals with Claude Code; claude-plus only reads"
    )]
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
        #[arg(
            long,
            requires = "ensure",
            help = "With --ensure, leave a daemon stopped by `claude-plus daemon stop` stopped"
        )]
        unless_stopped: bool,
        #[arg(long, hide = true)]
        detached: bool,
        #[arg(long, hide = true, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..))]
        rescan_secs: u64,
        #[command(subcommand)]
        action: Option<DaemonAction>,
    },
}

#[derive(Subcommand)]
enum AccountsAction {
    #[command(about = "Give an account your own label")]
    Rename { account: String, label: String },
}

#[derive(Subcommand)]
enum BudgetAction {
    #[command(about = "Set the monthly budget in USD, reset on the 1st at 00:00 UTC")]
    Set {
        #[arg(allow_hyphen_values = true)]
        usd: String,
        #[arg(long, help = "Account id or label [default: the current account]")]
        account: Option<String>,
    },
    #[command(about = "Show the budget, this month's spend and the pace")]
    Show {
        #[arg(long, help = "Account id or label [default: the current account]")]
        account: Option<String>,
    },
    #[command(about = "Remove the budget")]
    Clear {
        #[arg(long, help = "Account id or label [default: the current account]")]
        account: Option<String>,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    #[command(about = "Stop the running daemon")]
    Stop,
}

pub fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    if !matches!(
        cli.command,
        Command::Install { .. }
            | Command::Uninstall { .. }
            | Command::Daemon {
                action: None,
                ensure: false,
                ..
            }
    ) {
        end_quietly_when_the_reader_goes_away();
    }
    let paths = Paths::from_env()?;
    let (prices, user_prices) = PriceTable::with_user_file(&paths.user_prices());
    if let UserPrices::Invalid(message) = &user_prices
        && !matches!(
            cli.command,
            Command::Install { .. } | Command::Uninstall { .. }
        )
    {
        eprintln!("warning: {message}, using Anthropic's prices");
    }
    match cli.command {
        Command::Daemon {
            ensure: true,
            unless_stopped,
            action: None,
            ..
        } => {
            let policy = if unless_stopped {
                Stop::Respect
            } else {
                Stop::Override
            };
            return daemon::ensure(&paths, policy);
        }
        Command::Import => import::run(&paths, &prices),
        Command::Stats { days, account } => stats::run(&paths, &prices, days, account.as_deref()),
        Command::Export {
            format,
            days,
            account,
        } => export::run(&paths, &prices, format, days, account.as_deref()),
        Command::Accounts { action } => accounts::accounts(&paths, &prices, action),
        Command::Budget { action } => accounts::budget(&paths, &prices, action),
        Command::Doctor => doctor::run(&paths, &prices, &user_prices),
        Command::Reindex => reindex::run(&paths, &prices),
        Command::Install { yes } => setup::install(&paths, yes),
        Command::Uninstall { purge, yes } => setup::uninstall(&paths, purge, yes),
        Command::Daemon {
            action: Some(DaemonAction::Stop),
            ..
        } => daemon::stop(&paths),
        Command::Daemon {
            detached,
            rescan_secs,
            ..
        } => daemon::run(&paths, prices, detached, rescan_secs),
    }
    .map(|()| ExitCode::SUCCESS)
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
