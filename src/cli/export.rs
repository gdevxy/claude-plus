use std::io::Write;

use anyhow::{Context, Result};
use chrono::{Days, Local};
use clap::ValueEnum;

use crate::history::calendar::local_midnight;
use crate::history::export;
use crate::paths::Paths;
use crate::pricing::PriceTable;

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    #[value(help = "One object {\"v\":1,\"rows\":[...]}, costs to 6 decimals")]
    Json,
    #[value(help = "A header row, RFC 4180 quoting, LF line ends and costs to 6 decimals")]
    Csv,
}

pub fn run(paths: &Paths, prices: &PriceTable, format: Format, days: Option<u32>) -> Result<()> {
    let (since, until) = window(days)?;
    if let Some(notice) = super::missing_transcripts(paths) {
        eprintln!("{notice}");
        return print(format, &[]);
    }
    let (store, report, reconciled) = super::import::incremental(paths, prices)?;
    for line in super::import::issue_lines(&report) {
        eprintln!("{line}");
    }
    reconciled?;
    let rows = export::rows(
        &store.requests_between(since, until)?,
        &store.adjustments_between(since, until)?,
        &store.sessions_spending_between(since, until)?,
        &Local,
    );
    print(format, &rows)
}

fn print(format: Format, rows: &[export::ExportRow]) -> Result<()> {
    let text = match format {
        Format::Json => format!("{}\n", export::json(rows)?),
        Format::Csv => export::csv(rows),
    };
    std::io::stdout().lock().write_all(text.as_bytes())?;
    Ok(())
}

fn window(days: Option<u32>) -> Result<(i64, i64)> {
    let today = super::stats::now()?.date_naive();
    let tomorrow = today
        .checked_add_days(Days::new(1))
        .context("today is the last supported day")?;
    let until = local_midnight(tomorrow, &Local)?;
    let Some(days) = days else {
        return Ok((i64::MIN, until));
    };
    let first_day = today
        .checked_sub_days(Days::new(u64::from(days - 1)))
        .context("--days reaches before the calendar start")?;
    Ok((local_midnight(first_day, &Local)?, until))
}
