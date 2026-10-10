use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Days, Local, NaiveDate, NaiveTime, TimeDelta, TimeZone};
use comfy_table::{Cell, CellAlignment, Table, presets::UTF8_FULL_CONDENSED};

use super::format::{count, usd};
use crate::paths::Paths;
use crate::pricing::PriceTable;

const MINUTES_PER_DAY: i64 = 24 * 60;

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
    let (store, report) = super::import::incremental(paths, prices)?;
    for line in super::import::issue_lines(&report) {
        eprintln!("{line}");
    }
    let rows = store.daily_costs(since, until, &Local)?;

    let mut table = Table::new();
    table.load_style(UTF8_FULL_CONDENSED).set_header(vec![
        Cell::new("Day"),
        Cell::new("Model"),
        Cell::new("Requests").set_alignment(CellAlignment::Right),
        Cell::new("Cost").set_alignment(CellAlignment::Right),
    ]);
    let (mut total_requests, mut total_cost, mut total_reconciled, mut total_unpriced) =
        (0, 0.0, 0.0, 0);
    for row in &rows {
        total_requests += row.requests;
        total_cost += row.cost_usd;
        total_reconciled += row.reconciled_usd;
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
    if total_reconciled > 0.0 {
        println!(
            "  includes {} reconciled from Claude Code session counters",
            usd(total_reconciled)
        );
    }
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

fn local_midnight<Tz: TimeZone>(day: NaiveDate, tz: &Tz) -> Result<i64> {
    let midnight = day.and_time(NaiveTime::MIN);
    (0..MINUTES_PER_DAY)
        .find_map(|minute| {
            tz.from_local_datetime(&(midnight + TimeDelta::minutes(minute)))
                .earliest()
        })
        .map(|start| start.timestamp())
        .with_context(|| format!("{day} has no valid local time"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, MappedLocalTime, NaiveDateTime, Offset};

    #[derive(Debug, Clone, Copy)]
    struct GapAtMidnight;

    const GAP_DAY: (i32, u32, u32) = (2026, 9, 6);

    fn summer() -> FixedOffset {
        FixedOffset::west_opt(3 * 3600).unwrap()
    }

    fn winter() -> FixedOffset {
        FixedOffset::west_opt(4 * 3600).unwrap()
    }

    fn gap_start() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(GAP_DAY.0, GAP_DAY.1, GAP_DAY.2)
            .unwrap()
            .and_time(NaiveTime::MIN)
    }

    impl TimeZone for GapAtMidnight {
        type Offset = FixedOffset;

        fn from_offset(_: &FixedOffset) -> Self {
            GapAtMidnight
        }

        fn offset_from_local_date(&self, local: &NaiveDate) -> MappedLocalTime<FixedOffset> {
            self.offset_from_local_datetime(&local.and_time(NaiveTime::MIN))
        }

        fn offset_from_local_datetime(
            &self,
            local: &NaiveDateTime,
        ) -> MappedLocalTime<FixedOffset> {
            let gap_end = gap_start() + chrono::TimeDelta::hours(1);
            if *local < gap_start() {
                MappedLocalTime::Single(winter())
            } else if *local < gap_end {
                MappedLocalTime::None
            } else {
                MappedLocalTime::Single(summer())
            }
        }

        fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&utc.and_time(NaiveTime::MIN))
        }

        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
            let transition = gap_start() - winter().fix();
            if *utc < transition {
                winter()
            } else {
                summer()
            }
        }
    }

    fn utc(text: &str) -> i64 {
        DateTime::parse_from_rfc3339(text).unwrap().timestamp()
    }

    #[test]
    fn local_midnight_skipped_by_a_gap_starts_the_day_when_the_gap_ends() {
        let day = NaiveDate::from_ymd_opt(GAP_DAY.0, GAP_DAY.1, GAP_DAY.2).unwrap();
        assert_eq!(
            local_midnight(day, &GapAtMidnight).unwrap(),
            utc("2026-09-06T04:00:00Z")
        );
        assert_eq!(
            local_midnight(day.pred_opt().unwrap(), &GapAtMidnight).unwrap(),
            utc("2026-09-05T04:00:00Z")
        );
        assert_eq!(
            local_midnight(day.succ_opt().unwrap(), &GapAtMidnight).unwrap(),
            utc("2026-09-07T03:00:00Z")
        );
    }
}
