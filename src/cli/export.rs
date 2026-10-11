use std::collections::HashMap;
use std::io::Write;

use anyhow::{Context, Result};
use chrono::{Days, Local};
use clap::ValueEnum;

use crate::history::calendar::local_midnight;
use crate::history::export::{self, SessionAccount};
use crate::paths::Paths;
use crate::pricing::PriceTable;
use crate::store::Store;

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    #[value(help = "One object {\"v\":2,\"rows\":[...]}, costs to 6 decimals")]
    Json,
    #[value(help = "A header row, RFC 4180 quoting, LF line ends and costs to 6 decimals")]
    Csv,
}

pub fn run(
    paths: &Paths,
    prices: &PriceTable,
    format: Format,
    days: Option<u32>,
    account: Option<&str>,
) -> Result<()> {
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
    let (scope, label) = super::accounts::scope(&store, account)?;
    super::accounts::note_left_out(&store, account, &scope, &label, (since, until))?;
    let kept = scope.sessions(&store)?;
    let mut requests = store.requests_between(since, until)?;
    requests.retain(|request| scope.keeps(&kept, &request.session_id));
    let mut adjustments = store.adjustments_between(since, until)?;
    adjustments.retain(|adjustment| scope.keeps(&kept, &adjustment.session_id));
    let mut sessions = store.sessions_spending_between(since, until)?;
    sessions.retain(|session| scope.keeps(&kept, &session.id));
    let rows = export::rows(
        &requests,
        &adjustments,
        &sessions,
        &account_labels(&store)?,
        &Local,
    );
    print(format, &rows)
}

fn account_labels(store: &Store) -> Result<HashMap<String, SessionAccount>> {
    let labels: HashMap<String, String> = store
        .accounts()?
        .into_iter()
        .map(|account| (account.id, account.label))
        .collect();
    Ok(store
        .session_accounts()?
        .into_iter()
        .filter_map(|(session, id)| {
            let label = labels.get(&id)?.clone();
            Some((session, SessionAccount { id, label }))
        })
        .collect())
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
