use anyhow::Result;
use chrono::{Local, Offset};
use comfy_table::{Cell, CellAlignment, Table, presets::UTF8_FULL_CONDENSED};

use super::format::{count, usd};
use crate::paths::Paths;
use crate::pricing::PriceTable;

pub fn run(paths: &Paths, prices: &PriceTable, days: u32) -> Result<()> {
    if !paths.projects_dir().exists() {
        println!(
            "No Claude Code transcripts found in {}. Run Claude Code once, then try again ✻",
            paths.projects_dir().display()
        );
        return Ok(());
    }
    let (store, _) = super::import::incremental(paths, prices)?;
    let offset_seconds = Local::now().offset().fix().local_minus_utc();
    let since = Local::now().timestamp() - i64::from(days) * 86_400;
    let rows = store.daily_costs(since, offset_seconds)?;

    let mut table = Table::new();
    table.load_style(UTF8_FULL_CONDENSED).set_header(vec![
        Cell::new("Day"),
        Cell::new("Model"),
        Cell::new("Requests").set_alignment(CellAlignment::Right),
        Cell::new("Cost").set_alignment(CellAlignment::Right),
    ]);
    let (mut total_requests, mut total_cost) = (0, 0.0);
    for row in &rows {
        total_requests += row.requests;
        total_cost += row.cost_usd;
        let cost = if row.unpriced > 0 {
            format!("{} (+{} unpriced)", usd(row.cost_usd), row.unpriced)
        } else {
            usd(row.cost_usd)
        };
        table.add_row(vec![
            Cell::new(row.day.to_string()),
            Cell::new(&row.model),
            Cell::new(count(row.requests)).set_alignment(CellAlignment::Right),
            Cell::new(cost).set_alignment(CellAlignment::Right),
        ]);
    }
    table.add_row(vec![
        Cell::new("Total"),
        Cell::new(""),
        Cell::new(count(total_requests)).set_alignment(CellAlignment::Right),
        Cell::new(usd(total_cost)).set_alignment(CellAlignment::Right),
    ]);
    println!("{table}");
    Ok(())
}
