mod doctor;
pub mod format;
mod import;
mod reindex;
mod stats;

use anyhow::Result;
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};

use crate::paths::Paths;
use crate::pricing::PriceTable;
use crate::store::{Store, Upgrade};

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
        #[arg(long, default_value_t = 14, value_parser = clap::value_parser!(u32).range(1..=36500))]
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

fn progress_bar(verb: &str) -> Result<ProgressBar> {
    Ok(
        ProgressBar::new(0).with_style(ProgressStyle::with_template(&format!(
            "✻ {verb} {{human_pos}}/{{human_len}} transcripts {{wide_bar}}"
        ))?),
    )
}

const LABEL_WIDTH: usize = 13;
const SPINNER_TICK: std::time::Duration = std::time::Duration::from_millis(120);

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
    let mut shown: Option<ProgressBar> = None;
    let store = Store::open_with_progress(&paths.database(), &mut |event| match event {
        Upgrade::Records { done, total } => {
            let bar = shown.get_or_insert_with(|| {
                ProgressBar::new(total).with_style(
                    ProgressStyle::with_template(
                        "✻ Upgrading stored records {human_pos}/{human_len} {wide_bar}",
                    )
                    .expect("valid template"),
                )
            });
            bar.set_position(done);
        }
        Upgrade::Compacting => {
            if let Some(bar) = shown.take() {
                bar.finish_and_clear();
            }
            let spinner = ProgressBar::new_spinner()
                .with_style(
                    ProgressStyle::with_template("✻ {msg} {spinner}").expect("valid template"),
                )
                .with_message("Compacting the database, this runs once");
            spinner.enable_steady_tick(SPINNER_TICK);
            shown = Some(spinner);
        }
    });
    if let Some(bar) = shown {
        bar.finish_and_clear();
    }
    store
}
