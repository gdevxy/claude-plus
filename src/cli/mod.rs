mod doctor;
pub mod format;
mod import;
mod reindex;
mod stats;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::paths::Paths;
use crate::pricing::PriceTable;
use crate::store::Store;

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
    Import,
    Stats {
        #[arg(long, default_value_t = 14)]
        days: u32,
    },
    Doctor,
    Reindex,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::from_env()?;
    let prices = PriceTable::bundled();
    match cli.command {
        Command::Import => import::run(&paths, &prices),
        Command::Stats { days } => stats::run(&paths, &prices, days),
        Command::Doctor => doctor::run(&paths, &prices),
        Command::Reindex => reindex::run(&paths, &prices),
    }
}

const LABEL_WIDTH: usize = 13;

fn labeled(label: &str, value: &str) -> String {
    format!("  {label:<LABEL_WIDTH$}{value}")
}

fn transcripts_missing(paths: &Paths) -> bool {
    let missing = !paths.projects_dir().exists();
    if missing {
        println!(
            "No Claude Code transcripts found in {}. Run Claude Code once, then try again ✻",
            paths.projects_dir().display()
        );
    }
    missing
}

fn open_store(paths: &Paths) -> Result<Store> {
    paths.ensure_data_dir()?;
    Store::open(&paths.database())
}
