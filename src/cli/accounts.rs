use anyhow::{Result, bail};
use comfy_table::{Cell, CellAlignment, Table, presets::UTF8_FULL_CONDENSED};

use super::format::{count, usd, usd_with};
use super::{AccountsAction, BudgetAction};
use crate::accounts::{Auth, auth_account};
use crate::limits::{SPEND_LIMIT, month_start_utc, monthly_pace, next_month_start_utc};
use crate::paths::Paths;
use crate::pricing::PriceTable;
use crate::scope::Scope;
use crate::store::{AccountInfo, AccountRow, Store};

const ALL_ACCOUNTS: &str = "All accounts";
const HOUR_SECONDS: i64 = 3600;
const DAY_SECONDS: i64 = 24 * HOUR_SECONDS;

pub fn accounts(paths: &Paths, prices: &PriceTable, action: Option<AccountsAction>) -> Result<()> {
    match action {
        Some(AccountsAction::Rename { account, label }) => rename(paths, &account, &label),
        None => list(paths, prices),
    }
}

pub fn budget(paths: &Paths, prices: &PriceTable, action: BudgetAction) -> Result<()> {
    match action {
        BudgetAction::Set { usd, account } => set(paths, &usd, account.as_deref()),
        BudgetAction::Show { account } => show(paths, prices, account.as_deref()),
        BudgetAction::Clear { account } => clear(paths, account.as_deref()),
    }
}

pub fn resolve(store: &Store, given: Option<&str>) -> Result<String> {
    let Some(given) = given else {
        return match store.latest_session_account()?.or(store.open_account()?) {
            Some(id) => Ok(id),
            None => bail!("no account known yet; pass --account, see claude-plus accounts"),
        };
    };
    let accounts = store.accounts()?;
    if let Some(exact) = accounts.iter().find(|a| a.id == given) {
        return Ok(exact.id.clone());
    }
    let matches: Vec<&AccountInfo> = accounts
        .iter()
        .filter(|a| a.label.eq_ignore_ascii_case(given))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.id.clone()),
        [] if given == "api" || given.starts_with("gateway:") || given.starts_with("cloud:") => {
            Ok(given.to_string())
        }
        [] => bail!("no account called \"{given}\"; see claude-plus accounts"),
        many => bail!(
            "\"{given}\" matches {}; pass the id",
            many.iter()
                .map(|a| format!("{} ({})", a.id, a.label))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

pub fn scope(store: &Store, given: Option<&str>) -> Result<(Scope, String)> {
    if given == Some("all") {
        return Ok((Scope::All, ALL_ACCOUNTS.to_string()));
    }
    let id = match resolve(store, given) {
        Ok(id) => id,
        Err(_) if given.is_none() => return Ok((Scope::All, ALL_ACCOUNTS.to_string())),
        Err(error) => return Err(error),
    };
    let label = store
        .accounts()?
        .into_iter()
        .find(|account| account.id == id)
        .map_or_else(|| id.clone(), |account| account.label);
    Ok((Scope::Account(id), label))
}

fn now_ms() -> Result<i64> {
    Ok(super::stats::now()?.timestamp_millis())
}

fn list(paths: &Paths, prices: &PriceTable) -> Result<()> {
    let (store, report, reconciled) = super::import::incremental(paths, prices)?;
    for line in super::import::issue_lines(&report) {
        eprintln!("{line}");
    }
    reconciled?;
    let month_start = month_start_utc(now_ms()?) / 1000;
    let accounts = store.accounts()?;
    if accounts.is_empty() {
        println!("No accounts seen yet. Sign in to Claude Code and use it once ✻");
        return Ok(());
    }
    let mut table = Table::new();
    table.load_style(UTF8_FULL_CONDENSED).set_header(vec![
        Cell::new("Account"),
        Cell::new("Id"),
        Cell::new("Kind"),
        Cell::new("Sessions").set_alignment(CellAlignment::Right),
        Cell::new("This month").set_alignment(CellAlignment::Right),
        Cell::new("Budget").set_alignment(CellAlignment::Right),
    ]);
    for account in &accounts {
        let budget = store
            .budget(&account.id)?
            .map_or("-".to_string(), |b| usd_with(b.amount_usd, 2));
        table.add_row(vec![
            Cell::new(&account.label),
            Cell::new(&account.id),
            Cell::new(account.kind.as_str()),
            Cell::new(count(store.account_sessions_count(&account.id)?))
                .set_alignment(CellAlignment::Right),
            Cell::new(usd(store.account_spend_since(&account.id, month_start)?))
                .set_alignment(CellAlignment::Right),
            Cell::new(budget).set_alignment(CellAlignment::Right),
        ]);
    }
    println!("{table}");
    Ok(())
}

fn rename(paths: &Paths, given: &str, label: &str) -> Result<()> {
    let label = label.trim();
    if label.is_empty() {
        bail!("a label cannot be empty");
    }
    let mut store = super::open_store(paths)?;
    let id = resolve(&store, Some(given))?;
    let mut writer = store.writer()?;
    if !writer.rename_account(&id, label)? {
        bail!("no account called \"{given}\"; see claude-plus accounts");
    }
    writer.commit()?;
    println!("{id} is now called {label}");
    Ok(())
}

fn parse_budget(usd: &str) -> Result<f64> {
    match usd.trim().parse::<f64>() {
        Ok(amount) if amount.is_finite() && amount > 0.0 => Ok(amount),
        _ => bail!("a budget is a number of dollars above zero, not {usd}"),
    }
}

fn typed_account(id: &str) -> Option<AccountRow> {
    let auth = if id == "api" {
        Auth {
            provider: "anthropic".into(),
            base_url: None,
            api_key: true,
        }
    } else if let Some(origin) = id.strip_prefix("gateway:") {
        Auth {
            provider: "anthropic".into(),
            base_url: Some(origin.into()),
            api_key: false,
        }
    } else {
        Auth {
            provider: id.strip_prefix("cloud:")?.into(),
            base_url: None,
            api_key: false,
        }
    };
    auth_account(&auth).filter(|row| row.id == id)
}

fn label_of(store: &Store, id: &str) -> Result<String> {
    Ok(store
        .account(id)?
        .map(|a| a.label)
        .or_else(|| typed_account(id).map(|row| row.label))
        .unwrap_or_else(|| id.to_string()))
}

fn set(paths: &Paths, amount: &str, given: Option<&str>) -> Result<()> {
    let amount = parse_budget(amount)?;
    let mut store = super::open_store(paths)?;
    let id = resolve(&store, given)?;
    let known = store.account(&id)?;
    let now = now_ms()?;
    let mut writer = store.writer()?;
    let label = match known {
        Some(account) => account.label,
        None => {
            let Some(row) = typed_account(&id) else {
                bail!("no account called \"{id}\"; see claude-plus accounts");
            };
            writer.upsert_account(&row, now)?;
            row.label
        }
    };
    writer.set_budget(&id, amount, now)?;
    writer.commit()?;
    println!(
        "Budget for {label} set to {} a month, reset on the 1st at 00:00 UTC",
        usd_with(amount, 2)
    );
    Ok(())
}

fn clear(paths: &Paths, given: Option<&str>) -> Result<()> {
    let mut store = super::open_store(paths)?;
    let id = resolve(&store, given)?;
    let label = label_of(&store, &id)?;
    let mut writer = store.writer()?;
    let removed = writer.clear_budget(&id)?;
    writer.commit()?;
    if removed {
        println!("Budget for {label} cleared");
    } else {
        println!("{label} has no budget");
    }
    Ok(())
}

fn show(paths: &Paths, prices: &PriceTable, given: Option<&str>) -> Result<()> {
    let (store, report, reconciled) = super::import::incremental(paths, prices)?;
    for line in super::import::issue_lines(&report) {
        eprintln!("{line}");
    }
    reconciled?;
    let id = resolve(&store, given)?;
    let label = label_of(&store, &id)?;
    println!("{}", super::labeled("Account:", &format!("{label} ({id})")));
    let Some(budget) = store.budget(&id)? else {
        println!("{}", super::labeled("Budget:", "none"));
        return Ok(());
    };
    let now = now_ms()?;
    let (month_start, reset) = (month_start_utc(now), next_month_start_utc(now));
    let local = store.account_spend_since(&id, month_start / 1000)?;
    println!(
        "{}",
        super::labeled(
            "Budget:",
            &format!(
                "{} a month, resets in {}",
                usd_with(budget.amount_usd, 2),
                until(reset - now)
            )
        )
    );
    let reported = store
        .limit_samples_since(month_start)?
        .into_iter()
        .filter(|s| s.window == SPEND_LIMIT && s.account_id.as_deref() == Some(id.as_str()))
        .max_by_key(|s| s.ts_ms)
        .map(|s| s.used_pct);
    if let Some(pct) = reported {
        println!(
            "{}",
            super::labeled(
                "Reported:",
                &format!(
                    "{pct:.0}% used, about {}",
                    usd_with(pct / 100.0 * budget.amount_usd, 2)
                )
            )
        );
    }
    println!("{}", super::labeled("Claude Code here:", &usd(local)));
    let used_pct = reported.unwrap_or(local / budget.amount_usd * 100.0);
    let pace = match monthly_pace(used_pct, month_start, reset, now) {
        Some(pace) => format!(
            "on pace for {:.0}% by the reset",
            pace.projected_pct.unwrap_or(used_pct)
        ),
        None => "needs a full day of this month".to_string(),
    };
    println!("{}", super::labeled("Pace:", &pace));
    Ok(())
}

fn until(ms: i64) -> String {
    let seconds = ms.max(0) / 1000;
    if seconds >= DAY_SECONDS {
        format!("{}d", seconds / DAY_SECONDS)
    } else {
        format!("{}h", seconds / HOUR_SECONDS)
    }
}
