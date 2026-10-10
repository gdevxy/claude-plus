use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Days, Local, NaiveDate, NaiveTime, TimeZone};
use comfy_table::{Cell, CellAlignment, Table, presets::UTF8_FULL_CONDENSED};

use super::format::{count, usd};
use crate::paths::Paths;
use crate::pricing::PriceTable;

pub fn run(paths: &Paths, prices: &PriceTable, days: u32) -> Result<()> {
    if super::transcripts_missing(paths) {
        return Ok(());
    }
    let (store, report) = super::import::incremental(paths, prices)?;
    for line in super::import::issue_lines(&report) {
        eprintln!("{line}");
    }
    let today = now()?.date_naive();
    let first_day = today
        .checked_sub_days(Days::new(u64::from(days - 1)))
        .context("--days reaches before the calendar start")?;
    let tomorrow = today
        .checked_add_days(Days::new(1))
        .context("today is the last supported day")?;
    let rows = store.daily_costs(local_midnight(first_day), local_midnight(tomorrow), &Local)?;

    let mut table = Table::new();
    table.load_style(UTF8_FULL_CONDENSED).set_header(vec![
        Cell::new("Day"),
        Cell::new("Model"),
        Cell::new("Requests").set_alignment(CellAlignment::Right),
        Cell::new("Cost").set_alignment(CellAlignment::Right),
    ]);
    let (mut total_requests, mut total_cost, mut total_unpriced) = (0, 0.0, 0);
    for row in &rows {
        total_requests += row.requests;
        total_cost += row.cost_usd;
        total_unpriced += row.unpriced;
        table.add_row(vec![
            Cell::new(row.day.to_string()),
            Cell::new(&row.model),
            Cell::new(count(row.requests)).set_alignment(CellAlignment::Right),
            Cell::new(cost_text(row.cost_usd, row.unpriced)).set_alignment(CellAlignment::Right),
        ]);
    }
    table.add_row(vec![
        Cell::new("Total"),
        Cell::new(""),
        Cell::new(count(total_requests)).set_alignment(CellAlignment::Right),
        Cell::new(cost_text(total_cost, total_unpriced)).set_alignment(CellAlignment::Right),
    ]);
    println!("{table}");
    Ok(())
}

fn cost_text(cost_usd: f64, unpriced: u64) -> String {
    if unpriced > 0 {
        format!("{} (+{} unpriced)", usd(cost_usd), count(unpriced))
    } else {
        usd(cost_usd)
    }
}

fn now() -> Result<DateTime<Local>> {
    match std::env::var("CLAUDE_PLUS_NOW") {
        Ok(text) => DateTime::parse_from_rfc3339(&text)
            .map(|fixed| fixed.with_timezone(&Local))
            .map_err(|_| anyhow!("CLAUDE_PLUS_NOW must be an RFC 3339 timestamp, got \"{text}\"")),
        Err(_) => Ok(Local::now()),
    }
}

fn local_midnight(day: NaiveDate) -> i64 {
    Local
        .from_local_datetime(&day.and_time(NaiveTime::MIN))
        .earliest()
        .map_or_else(
            || day.and_time(NaiveTime::MIN).and_utc().timestamp(),
            |start| start.timestamp(),
        )
}
