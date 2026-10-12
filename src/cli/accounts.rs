use anyhow::{Context, Result, anyhow, bail};
use chrono::Local;
use comfy_table::{Cell, CellAlignment, Table, presets::UTF8_FULL_CONDENSED};

use super::format::{column_decimals, count, plural, usd, usd_with};
use super::{AccountsAction, BudgetAction, SelectionArgs};
use crate::accounts::{Auth, auth_account};
use crate::assign::{Filter, Selection, expand_home, select};
use crate::limits::{SPEND_LIMIT, month_start_utc, monthly_pace, next_month_start_utc};
use crate::paths::Paths;
use crate::pricing::PriceTable;
use crate::scope::Scope;
use crate::store::{AccountInfo, AccountRow, AttributionSource, Store};

const ALL_ACCOUNTS: &str = "All accounts";
const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;
const DAY_MS: i64 = 24 * HOUR_MS;
const BUDGET_HINT: &str = "claude-plus budget set <usd> shows dollars";

pub fn accounts(paths: &Paths, prices: &PriceTable, action: Option<AccountsAction>) -> Result<()> {
    match action {
        Some(AccountsAction::Rename { account, label }) => rename(paths, &account, &label),
        Some(AccountsAction::Assign { account, selection }) => {
            assign(paths, prices, &account, &selection)
        }
        Some(AccountsAction::Unassign { selection }) => unassign(paths, prices, &selection),
        None => list(paths, prices),
    }
}

pub fn budget(paths: &Paths, prices: &PriceTable, action: BudgetAction) -> Result<()> {
    match action {
        BudgetAction::Set { usd, account } => set(paths, prices, &usd, account.as_deref()),
        BudgetAction::Show { account } => show(paths, prices, account.as_deref()),
        BudgetAction::Clear { account } => clear(paths, prices, account.as_deref()),
    }
}

pub fn current(store: &Store) -> Result<Option<String>> {
    store.current_account()
}

pub fn resolve(store: &Store, given: Option<&str>) -> Result<String> {
    let Some(given) = given else {
        return current(store)?.ok_or_else(|| {
            anyhow!("no account known yet; pass --account, see claude-plus accounts")
        });
    };
    let accounts = store.accounts()?;
    if let Some(exact) = accounts.iter().find(|a| a.id == given) {
        return Ok(exact.id.clone());
    }
    let wanted = given.to_lowercase();
    let matches: Vec<&AccountInfo> = accounts
        .iter()
        .filter(|a| a.label.to_lowercase() == wanted)
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
    let id = match given {
        Some("all") => None,
        Some(given) => Some(resolve(store, Some(given))?),
        None => current(store)?,
    };
    match id {
        Some(id) => {
            let label = label_of(store, &id)?;
            Ok((Scope::Account(id), label))
        }
        None => Ok((Scope::All, ALL_ACCOUNTS.to_string())),
    }
}

pub fn note_left_out(
    store: &Store,
    given: Option<&str>,
    scope: &Scope,
    label: &str,
    (since, until): (i64, i64),
) -> Result<()> {
    if given.is_some() || scope.account().is_none() {
        return Ok(());
    }
    let kept = scope.sessions(store)?;
    let left_out = store
        .sessions_spending_between(since, until)?
        .iter()
        .any(|session| !scope.keeps(&kept, &session.id));
    if left_out {
        eprintln!("Showing {label}; --account all for every account");
    }
    Ok(())
}

pub fn described(store: &Store, id: &str) -> Result<String> {
    Ok(format!("{} ({id})", label_of(store, id)?))
}

fn now_ms() -> Result<i64> {
    Ok(super::stats::now()?.timestamp_millis())
}

fn imported(paths: &Paths, prices: &PriceTable) -> Result<Store> {
    let (store, report, reconciled) = super::import::incremental(paths, prices)?;
    for line in super::import::issue_lines(&report) {
        eprintln!("{line}");
    }
    reconciled?;
    Ok(store)
}

fn list(paths: &Paths, prices: &PriceTable) -> Result<()> {
    let store = imported(paths, prices)?;
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
    let spends = accounts
        .iter()
        .map(|account| store.account_spend_since(&account.id, month_start))
        .collect::<Result<Vec<f64>>>()?;
    let decimals = column_decimals(&spends);
    for (account, spend) in accounts.iter().zip(spends) {
        let budget = store
            .budget(&account.id)?
            .map_or("-".to_string(), |b| usd_with(b.amount_usd, 2));
        table.add_row(vec![
            Cell::new(&account.label),
            Cell::new(&account.id),
            Cell::new(account.kind.as_str()),
            Cell::new(count(store.account_sessions_count(&account.id)?))
                .set_alignment(CellAlignment::Right),
            Cell::new(usd_with(spend, decimals)).set_alignment(CellAlignment::Right),
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

fn filter_of(paths: &Paths, selection: &SelectionArgs) -> Result<Filter> {
    Filter::new(
        selection
            .project
            .as_deref()
            .map(|pattern| expand_home(pattern, &paths.home)),
        selection.since.as_deref(),
        selection.until.as_deref(),
        &Local,
    )
}

fn assign(
    paths: &Paths,
    prices: &PriceTable,
    given: &str,
    selection: &SelectionArgs,
) -> Result<()> {
    let filter = filter_of(paths, selection)?;
    let mut store = imported(paths, prices)?;
    let account = assignable(&store, given)?;
    let chosen = select(store.unknown_sessions()?, &filter);
    if chosen.sessions.is_empty() {
        println!("No Unknown sessions match");
        return Ok(());
    }
    if !selection.dry_run {
        let mut writer = store.writer()?;
        for session in &chosen.sessions {
            writer.attribute(session, &account.id, AttributionSource::Assign)?;
        }
        writer.commit()?;
    }
    let verb = if selection.dry_run {
        "Would assign"
    } else {
        "Assigned"
    };
    println!(
        "{verb} {} to {}",
        plural(chosen.sessions.len() as u64, "session", "sessions"),
        account.label
    );
    println!("{}", selection_line(&chosen));
    Ok(())
}

fn unassign(paths: &Paths, prices: &PriceTable, selection: &SelectionArgs) -> Result<()> {
    let filter = filter_of(paths, selection)?;
    let mut store = imported(paths, prices)?;
    let chosen = select(store.assigned_sessions()?, &filter);
    if chosen.sessions.is_empty() {
        println!("No assigned sessions match");
        return Ok(());
    }
    if !selection.dry_run {
        let mut writer = store.writer()?;
        for session in &chosen.sessions {
            writer.unassign(session)?;
        }
        writer.commit()?;
    }
    let verb = if selection.dry_run {
        "Would unassign"
    } else {
        "Unassigned"
    };
    println!(
        "{verb} {}",
        plural(chosen.sessions.len() as u64, "session", "sessions")
    );
    println!("{}", selection_line(&chosen));
    Ok(())
}

fn assignable(store: &Store, given: &str) -> Result<AccountInfo> {
    let accounts = store.accounts()?;
    let wanted = given.to_lowercase();
    let known = accounts
        .iter()
        .any(|a| a.id == given || a.label.to_lowercase() == wanted);
    if !known && accounts.is_empty() {
        bail!("no account called \"{given}\"; no account is known yet, see claude-plus accounts");
    }
    if !known {
        bail!(
            "no account called \"{given}\"; the accounts are {}",
            accounts
                .iter()
                .map(|a| format!("{} ({})", a.label, a.id))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let id = resolve(store, Some(given))?;
    accounts
        .into_iter()
        .find(|a| a.id == id)
        .with_context(|| format!("no account called \"{given}\"; see claude-plus accounts"))
}

fn selection_line(chosen: &Selection) -> String {
    let spend = usd(chosen.spend_usd);
    match chosen.days(&Local) {
        Some((first, last)) if first == last => format!("  {first} · {spend}"),
        Some((first, last)) => format!("  {first} to {last} · {spend}"),
        None => format!("  {spend}"),
    }
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

fn writable(paths: &Paths, prices: &PriceTable, given: Option<&str>) -> Result<Store> {
    if given.is_none() {
        imported(paths, prices)?;
    }
    super::open_store(paths)
}

fn set(paths: &Paths, prices: &PriceTable, amount: &str, given: Option<&str>) -> Result<()> {
    let amount = parse_budget(amount)?;
    let mut store = writable(paths, prices, given)?;
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

fn clear(paths: &Paths, prices: &PriceTable, given: Option<&str>) -> Result<()> {
    let mut store = writable(paths, prices, given)?;
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
    let store = imported(paths, prices)?;
    let id = resolve(&store, given)?;
    let now_ms = now_ms()?;
    let month_start = month_start_utc(now_ms);
    let reported_pct = store
        .limit_samples_since(month_start)?
        .into_iter()
        .filter(|s| s.window == SPEND_LIMIT && s.account_id.as_deref() == Some(id.as_str()))
        .max_by_key(|s| s.ts_ms)
        .map(|s| s.used_pct);
    let facts = BudgetFacts {
        account: described(&store, &id)?,
        budget_usd: store.budget(&id)?.map(|budget| budget.amount_usd),
        reported_pct,
        local_usd: store.account_spend_since(&id, month_start / 1000)?,
        now_ms,
    };
    for line in facts.lines() {
        println!("{line}");
    }
    Ok(())
}

struct BudgetFacts {
    account: String,
    budget_usd: Option<f64>,
    reported_pct: Option<f64>,
    local_usd: f64,
    now_ms: i64,
}

impl BudgetFacts {
    fn lines(&self) -> Vec<String> {
        let mut lines = vec![super::labeled("Account:", &self.account)];
        match self.budget_usd {
            Some(amount) => self.with_budget(amount, &mut lines),
            None => self.without_budget(&mut lines),
        }
        lines
    }

    fn with_budget(&self, amount: f64, lines: &mut Vec<String>) {
        let reported_usd = self.reported_pct.map(|pct| pct / 100.0 * amount);
        let values: Vec<f64> = [amount, self.local_usd]
            .into_iter()
            .chain(reported_usd)
            .collect();
        let decimals = column_decimals(&values);
        let money = |value: f64| usd_with(value, decimals);
        let resets_in = until(next_month_start_utc(self.now_ms) - self.now_ms);
        lines.push(super::labeled(
            "Budget:",
            &format!("{} a month, resets in {resets_in}", money(amount)),
        ));
        if let (Some(pct), Some(usd)) = (self.reported_pct, reported_usd) {
            lines.push(super::labeled(
                "Reported:",
                &format!("{} used, {} of the spend limit", money(usd), percent(pct)),
            ));
        }
        lines.push(super::labeled(
            "Local:",
            &format!("{} spent this month on this machine", money(self.local_usd)),
        ));
        let used_pct = self.reported_pct.unwrap_or(self.local_usd / amount * 100.0);
        lines.push(super::labeled("Pace:", &self.pace(used_pct)));
    }

    fn without_budget(&self, lines: &mut Vec<String>) {
        lines.push(super::labeled("Budget:", "none"));
        if let Some(pct) = self.reported_pct {
            lines.push(super::labeled(
                "Reported:",
                &format!("{} of the spend limit used", percent(pct)),
            ));
        }
        lines.push(super::labeled(
            "Local:",
            &format!("{} spent this month on this machine", usd(self.local_usd)),
        ));
        lines.push(super::labeled("", BUDGET_HINT));
    }

    fn pace(&self, used_pct: f64) -> String {
        let pace = monthly_pace(
            used_pct,
            month_start_utc(self.now_ms),
            next_month_start_utc(self.now_ms),
            self.now_ms,
        );
        let Some(pace) = pace else {
            return "shown after the first full day of the month".to_string();
        };
        let projected = pace.projected_pct.unwrap_or(used_pct);
        if projected > 100.0 {
            format!(
                "on pace for {} by the reset, over the budget",
                percent(projected)
            )
        } else {
            format!("on pace for {} by the reset", percent(projected))
        }
    }
}

fn percent(value: f64) -> String {
    if (value * 10.0).round() < 100.0 {
        format!("{value:.1}%")
    } else {
        format!("{value:.0}%")
    }
}

fn until(ms: i64) -> String {
    match ms.max(0) {
        ms if ms >= DAY_MS => format!("{}d", ms / DAY_MS),
        ms if ms >= HOUR_MS => format!("{}h", ms / HOUR_MS),
        ms if ms >= MINUTE_MS => format!("{}m", ms / MINUTE_MS),
        _ => "under a minute".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(text)
            .unwrap()
            .timestamp_millis()
    }

    fn facts(budget_usd: Option<f64>, reported_pct: Option<f64>, now: &str) -> BudgetFacts {
        BudgetFacts {
            account: "Acme · Enterprise (oauth:work)".into(),
            budget_usd,
            reported_pct,
            local_usd: 23.1,
            now_ms: at(now),
        }
    }

    const MID_MONTH: &str = "2026-10-21T12:00:00Z";

    #[test]
    fn the_reported_percent_is_turned_into_dollars_of_the_budget() {
        assert_eq!(
            facts(Some(700.0), Some(4.0), MID_MONTH).lines(),
            [
                "  Account:     Acme · Enterprise (oauth:work)",
                "  Budget:      $700.00 a month, resets in 10d",
                "  Reported:    $28.00 used, 4.0% of the spend limit",
                "  Local:       $23.10 spent this month on this machine",
                "  Pace:        on pace for 6.0% by the reset",
            ]
        );
    }

    #[test]
    fn without_a_reported_percent_the_pace_follows_local_spend() {
        let lines = facts(Some(700.0), None, MID_MONTH).lines();
        assert_eq!(
            lines[2],
            "  Local:       $23.10 spent this month on this machine"
        );
        assert_eq!(lines[3], "  Pace:        on pace for 5.0% by the reset");
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn a_pace_over_the_budget_says_so() {
        assert_eq!(
            facts(Some(30.0), None, MID_MONTH).lines()[3],
            "  Pace:        on pace for 116% by the reset, over the budget"
        );
    }

    #[test]
    fn the_pace_waits_for_a_full_day() {
        let lines = facts(Some(700.0), None, "2026-10-01T12:00:00Z").lines();
        assert_eq!(lines[1], "  Budget:      $700.00 a month, resets in 30d");
        assert_eq!(
            lines[3],
            "  Pace:        shown after the first full day of the month"
        );
    }

    #[test]
    fn amounts_below_a_dollar_share_one_precision() {
        let mut facts = facts(Some(0.5), None, MID_MONTH);
        facts.local_usd = 0.1234;
        let lines = facts.lines();
        assert_eq!(lines[1], "  Budget:      $0.5000 a month, resets in 10d");
        assert_eq!(
            lines[2],
            "  Local:       $0.1234 spent this month on this machine"
        );
    }

    #[test]
    fn without_a_budget_the_spend_and_a_hint_are_shown() {
        assert_eq!(
            facts(None, Some(0.4), MID_MONTH).lines(),
            [
                "  Account:     Acme · Enterprise (oauth:work)",
                "  Budget:      none",
                "  Reported:    0.4% of the spend limit used",
                "  Local:       $23.10 spent this month on this machine",
                "               claude-plus budget set <usd> shows dollars",
            ]
        );
        assert_eq!(facts(None, None, MID_MONTH).lines().len(), 4);
    }

    #[test]
    fn percents_keep_one_decimal_below_ten() {
        assert_eq!(percent(0.4), "0.4%");
        assert_eq!(percent(9.94), "9.9%");
        assert_eq!(percent(9.96), "10%");
        assert_eq!(percent(116.4), "116%");
    }

    #[test]
    fn the_reset_is_counted_in_minutes_below_an_hour() {
        assert_eq!(until(3 * DAY_MS + 5 * HOUR_MS), "3d");
        assert_eq!(until(2 * HOUR_MS + 59 * MINUTE_MS), "2h");
        assert_eq!(until(35 * MINUTE_MS + 30_000), "35m");
        assert_eq!(until(59_000), "under a minute");
        assert_eq!(until(-5), "under a minute");
        assert_eq!(
            facts(Some(700.0), None, "2026-10-31T23:25:00Z").lines()[1],
            "  Budget:      $700.00 a month, resets in 35m"
        );
    }
}
