use anyhow::Result;

use super::format::{plural, usd};
use crate::ingest::{IngestReport, Ingestor};
use crate::paths::Paths;
use crate::pricing::PriceTable;
use crate::store::Reconciliation;
use crate::store::Store;

pub fn run(paths: &Paths, prices: &PriceTable) -> Result<()> {
    if super::transcripts_missing(paths) {
        return Ok(());
    }
    let (store, report, reconciled) = incremental(paths, prices)?;
    print_summary(&store, &report)?;
    reconciled.map(|_| ())
}

pub fn incremental(
    paths: &Paths,
    prices: &PriceTable,
) -> Result<(Store, IngestReport, Result<Reconciliation>)> {
    let mut store = super::open_store(paths)?;
    let mut ingestor = Ingestor::new(&mut store, prices);
    let rebuilding = super::progress_bar("Rebuilding")?;
    ingestor.finish_interrupted_reindex(&mut |done, total| {
        rebuilding.set_length(total as u64);
        rebuilding.set_position(done as u64);
    })?;
    rebuilding.finish_and_clear();
    let bar = super::progress_bar("Importing")?;
    let report = ingestor.ingest_all(&paths.projects_dir(), &mut |done, total| {
        bar.set_length(total as u64);
        bar.set_position(done as u64);
    })?;
    bar.finish_and_clear();
    let reconciled = ingestor.reconcile(&report);
    Ok((store, report, reconciled))
}

pub fn issue_lines(report: &IngestReport) -> Vec<String> {
    let mut lines = Vec::new();
    if report.files_vanished > 0 {
        lines.push(format!(
            "  {} during import",
            plural(report.files_vanished, "file vanished", "files vanished")
        ));
    }
    for (path, message) in &report.failures {
        lines.push(format!("  Could not read {path}: {message}"));
    }
    lines
}

fn print_summary(store: &Store, report: &IngestReport) -> Result<()> {
    let totals = store.totals()?;
    println!(
        "✻ Imported {} from {} · {} · {}",
        plural(report.lines, "line", "lines"),
        plural(report.files_changed, "changed file", "changed files"),
        plural(report.requests_inserted, "new request", "new requests"),
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
    for line in issue_lines(report) {
        println!("{line}");
    }
    Ok(())
}
