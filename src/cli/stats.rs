use anyhow::Result;
use chrono::{DateTime, Days, Local, Offset, TimeZone};
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
    let now = now();
    let offset_seconds = now.offset().fix().local_minus_utc();
    let rows = store.daily_costs(window_start(now, days), offset_seconds)?;

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

fn now() -> DateTime<Local> {
    std::env::var("CLAUDE_PLUS_NOW")
        .ok()
        .and_then(|text| DateTime::parse_from_rfc3339(&text).ok())
        .map_or_else(Local::now, |fixed| fixed.with_timezone(&Local))
}

fn window_start(now: DateTime<Local>, days: u32) -> i64 {
    let first_day = now
        .date_naive()
        .checked_sub_days(Days::new(u64::from(days.saturating_sub(1))))
        .unwrap_or(now.date_naive());
    Local
        .from_local_datetime(&first_day.and_time(chrono::NaiveTime::MIN))
        .earliest()
        .map_or(0, |start| start.timestamp())
}
