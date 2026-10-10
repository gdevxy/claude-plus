use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};

use super::format::{count, plural, usd};
use crate::ingest::{IngestReport, Ingestor};
use crate::paths::Paths;
use crate::pricing::PriceTable;
use crate::store::Store;

pub fn run(paths: &Paths, prices: &PriceTable) -> Result<()> {
    let mut store = super::open_store(paths)?;
    let bar = ProgressBar::new(0).with_style(ProgressStyle::with_template(
        "✻ Importing {pos}/{len} transcripts {wide_bar}",
    )?);
    let report = Ingestor::new(&mut store, prices).ingest_all(
        &paths.projects_dir(),
        &mut |done, total| {
            bar.set_length(total as u64);
            bar.set_position(done as u64);
        },
    )?;
    bar.finish_and_clear();
    print_summary(&store, &report)
}

pub fn incremental(paths: &Paths, prices: &PriceTable) -> Result<(Store, IngestReport)> {
    let mut store = super::open_store(paths)?;
    let report =
        Ingestor::new(&mut store, prices).ingest_all(&paths.projects_dir(), &mut |_, _| {})?;
    Ok((store, report))
}

fn print_summary(store: &Store, report: &IngestReport) -> Result<()> {
    let totals = store.totals()?;
    println!(
        "✻ Imported {} from {} · {} new requests · {}",
        plural(report.lines, "line", "lines"),
        plural(report.files_changed, "changed file", "changed files"),
        count(report.requests_inserted),
        plural(report.parse_errors, "unparsable line", "unparsable lines"),
    );
    println!(
        "  History: {} across {} · {} priced",
        plural(totals.requests, "request", "requests"),
        plural(totals.sessions, "session", "sessions"),
        usd(totals.priced_cost_usd),
    );
    if totals.unpriced_requests > 0 {
        println!(
            "  {} without a known price (see `claude-plus doctor`)",
            plural(totals.unpriced_requests, "request", "requests")
        );
    }
    if report.files_vanished > 0 {
        println!(
            "  {} during import",
            plural(report.files_vanished, "file vanished", "files vanished")
        );
    }
    for (path, message) in &report.failures {
        println!("  Could not read {path}: {message}");
    }
    Ok(())
}
