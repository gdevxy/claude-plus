use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::Value;

use crate::transcript::redact::redact;

pub type Step = fn(&mut Connection, &mut dyn FnMut(Upgrade)) -> Result<()>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upgrade {
    Records { done: u64, total: u64 },
    Compacting,
}

const BATCH_ROWS: i64 = 2000;
const VACUUM_PENDING: &str = "vacuum_pending";

pub(super) fn redact_stored_records(
    conn: &mut Connection,
    on_upgrade: &mut dyn FnMut(Upgrade),
) -> Result<()> {
    let total = conn.query_row("SELECT COUNT(*) FROM raw_records", [], |r| {
        r.get::<_, i64>(0)
    })? as u64;
    if total == 0 {
        return Ok(());
    }
    on_upgrade(Upgrade::Records { done: 0, total });
    let mut done = 0;
    let mut after = (i64::MIN, i64::MIN);
    loop {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let batch = raw_batch(&tx, after)?;
        let Some(&(file_id, line_no, _)) = batch.last() else {
            break;
        };
        after = (file_id, line_no);
        rewrite(
            &tx,
            "UPDATE raw_records SET json = ?3 WHERE file_id = ?1 AND line_no = ?2",
            &batch,
        )?;
        tx.commit()?;
        done += batch.len() as u64;
        on_upgrade(Upgrade::Records { done, total });
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let cost_states = tx
        .prepare("SELECT file_id, line_no, json FROM cost_states")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rewrite(
        &tx,
        "UPDATE cost_states SET json = ?3 WHERE file_id = ?1 AND line_no = ?2",
        &cost_states,
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, 'yes')",
        [VACUUM_PENDING],
    )?;
    tx.commit()?;
    Ok(())
}

pub(super) fn compact_if_pending(
    conn: &Connection,
    on_upgrade: &mut dyn FnMut(Upgrade),
) -> Result<()> {
    let pending = conn
        .query_row(
            "SELECT 1 FROM meta WHERE key = ?1",
            [VACUUM_PENDING],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if pending {
        on_upgrade(Upgrade::Compacting);
        conn.execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")?;
        conn.execute("DELETE FROM meta WHERE key = ?1", [VACUUM_PENDING])?;
    }
    Ok(())
}

fn raw_batch(tx: &Transaction, after: (i64, i64)) -> Result<Vec<(i64, i64, String)>> {
    let mut statement = tx.prepare_cached(
        "SELECT file_id, line_no, json FROM raw_records
         WHERE (file_id, line_no) > (?1, ?2) ORDER BY file_id, line_no LIMIT ?3",
    )?;
    let rows = statement.query_map(params![after.0, after.1, BATCH_ROWS], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn rewrite(tx: &Transaction, sql: &str, rows: &[(i64, i64, String)]) -> Result<()> {
    let mut update = tx.prepare_cached(sql)?;
    for (file_id, line_no, json) in rows {
        let Ok(value) = serde_json::from_str::<Value>(json) else {
            continue;
        };
        let redacted = redact(&value).to_string();
        if redacted != *json {
            update.execute(params![file_id, line_no, redacted])?;
        }
    }
    Ok(())
}
