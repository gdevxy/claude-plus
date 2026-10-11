use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Days, Local};
use comfy_table::{Cell, CellAlignment, Table, presets::UTF8_FULL_CONDENSED};

use super::format::{column_decimals, count, usd, usd_with};
use crate::history::calendar::local_midnight;
use crate::paths::Paths;
use crate::pricing::PriceTable;
use crate::store::DailyCost;

pub fn run(paths: &Paths, prices: &PriceTable, days: u32) -> Result<()> {
    let today = now()?.date_naive();
    let first_day = today
        .checked_sub_days(Days::new(u64::from(days - 1)))
        .context("--days reaches before the calendar start")?;
    let tomorrow = today
        .checked_add_days(Days::new(1))
        .context("today is the last supported day")?;
    let (since, until) = (
        local_midnight(first_day, &Local)?,
        local_midnight(tomorrow, &Local)?,
    );
    if super::transcripts_missing(paths) {
        return Ok(());
    }
    let (store, report, reconciled) = super::import::incremental(paths, prices)?;
    for line in super::import::issue_lines(&report) {
        eprintln!("{line}");
    }
    reconciled?;
    let rows = store.daily_costs(since, until, &Local)?;
    let total = Total::of(&rows);
    let decimals = column_decimals(
        &rows
            .iter()
            .map(|row| row.cost_usd)
            .chain([total.cost_usd])
            .collect::<Vec<_>>(),
    );
    let costs = CostColumn::new(
        rows.iter()
            .map(|row| (row.cost_usd, row.unpriced))
            .chain([(total.cost_usd, total.unpriced)]),
        decimals,
    );

    let mut table = Table::new();
    table.load_style(UTF8_FULL_CONDENSED).set_header(vec![
        Cell::new("Day"),
        Cell::new("Model"),
        Cell::new("Requests").set_alignment(CellAlignment::Right),
        Cell::new("Cost").set_alignment(CellAlignment::Right),
    ]);
    for (row, cost) in rows.iter().zip(&costs.cells) {
        table.add_row(vec![
            Cell::new(row.day.to_string()),
            Cell::new(&row.model),
            Cell::new(requests_text(row)).set_alignment(CellAlignment::Right),
            Cell::new(cost).set_alignment(CellAlignment::Right),
        ]);
    }
    table.add_row(vec![
        Cell::new("Total"),
        Cell::new(""),
        Cell::new(count(total.requests)).set_alignment(CellAlignment::Right),
        Cell::new(costs.cells.last().map_or("", String::as_str))
            .set_alignment(CellAlignment::Right),
    ]);
    println!("{}", with_total_separator(&table.to_string()));
    if total.reconciled_usd > 0.0 {
        println!(
            "  includes {} reconciled from Claude Code session counters",
            usd(total.reconciled_usd)
        );
    }
    Ok(())
}

#[derive(Default)]
struct Total {
    requests: u64,
    cost_usd: f64,
    reconciled_usd: f64,
    unpriced: u64,
}

impl Total {
    fn of(rows: &[DailyCost]) -> Self {
        rows.iter().fold(Self::default(), |total, row| Self {
            requests: total.requests + row.requests,
            cost_usd: total.cost_usd + row.cost_usd,
            reconciled_usd: total.reconciled_usd + row.reconciled_usd,
            unpriced: total.unpriced + row.unpriced,
        })
    }
}

struct CostColumn {
    cells: Vec<String>,
}

impl CostColumn {
    fn new(values: impl Iterator<Item = (f64, u64)>, decimals: usize) -> Self {
        let parts: Vec<(String, String)> = values
            .map(|(cost_usd, unpriced)| {
                let suffix = if unpriced > 0 {
                    format!(" (+{} unpriced)", count(unpriced))
                } else {
                    String::new()
                };
                (usd_with(cost_usd, decimals), suffix)
            })
            .collect();
        let money_width = parts
            .iter()
            .map(|(money, _)| money.len())
            .max()
            .unwrap_or(0);
        let suffix_width = parts
            .iter()
            .map(|(_, suffix)| suffix.len())
            .max()
            .unwrap_or(0);
        let cells = parts
            .iter()
            .map(|(money, suffix)| format!("{money:>money_width$}{suffix:<suffix_width$}"))
            .collect();
        Self { cells }
    }
}

fn with_total_separator(table: &str) -> String {
    let lines: Vec<&str> = table.lines().collect();
    let Some((top, _)) = lines.split_first() else {
        return table.to_string();
    };
    let separator: String = top
        .chars()
        .map(|ch| match ch {
            '┌' => '├',
            '┬' => '┼',
            '┐' => '┤',
            other => other,
        })
        .collect();
    let total_row = lines.len().saturating_sub(2);
    let mut out: Vec<&str> = lines[..total_row].to_vec();
    out.push(&separator);
    out.extend(&lines[total_row..]);
    out.join("\n")
}

fn requests_text(row: &DailyCost) -> String {
    if row.requests == 0 && row.reconciled_usd > 0.0 {
        "-".to_string()
    } else {
        count(row.requests)
    }
}

pub(super) fn now() -> Result<DateTime<Local>> {
    match std::env::var("CLAUDE_PLUS_NOW") {
        Ok(text) => DateTime::parse_from_rfc3339(&text)
            .map(|fixed| fixed.with_timezone(&Local))
            .map_err(|_| anyhow!("CLAUDE_PLUS_NOW must be an RFC 3339 timestamp, got \"{text}\"")),
        Err(_) => Ok(Local::now()),
    }
}
