mod accounts;
mod breakdowns;
mod context;
mod events;
mod history;
mod insights;
mod limits;
mod reconcile;
mod schema;
mod session;
mod timeline;
mod writer;

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDate, TimeZone};
use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};

use crate::transcript::location::Location;
pub use accounts::{AccountInfo, AccountKind, AccountRow, AttributionSource, Budget};
pub use breakdowns::StoredBreakdown;
pub use context::{AttachmentPoint, ToolResultPoint};
pub use events::{EventRow, StoredEvent};
pub use history::{CompactionPoint, SessionInfo, SpentAdjustment, SpentRequest};
pub use insights::{LargeResult, PrefixChange, PromptPoint};
pub use limits::{LimitSample, RESETS_AT_TOLERANCE_MS, SpendPoint};
pub use reconcile::Reconciliation;
pub use session::{ModelTotal, SessionTotals};
pub use timeline::{PREFIX_KINDS, RequestPoint};
pub use writer::*;

const GENERATION_MARK: &str = "#gen-";
const REINDEX_PENDING: &str = "reindex_pending";
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const READ_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(200);
const STATEMENT_CACHE: usize = 64;
const WAL_RETRY: std::time::Duration = std::time::Duration::from_millis(10);

#[derive(Debug, thiserror::Error)]
#[error("another claude-plus is writing, try again in a moment")]
pub struct Busy;

#[derive(Debug, thiserror::Error)]
#[error("database was created by a pre-release claude-plus")]
pub struct PreRelease;

#[derive(Debug, thiserror::Error)]
#[error("not a claude-plus store")]
pub struct NotOurs;

pub const APPLICATION_ID: i32 = 0x436c_2b70;
const HEADER_MAGIC: &[u8; 16] = b"SQLite format 3\0";

pub fn is_claude_plus_database(file: &mut impl std::io::Read) -> bool {
    let mut header = [0u8; 72];
    file.read_exact(&mut header).is_ok()
        && &header[..16] == HEADER_MAGIC
        && i32::from_be_bytes([header[68], header[69], header[70], header[71]]) == APPLICATION_ID
}

#[derive(Debug, thiserror::Error)]
#[error("schema migration step {step} (v{version}) failed: {message}")]
pub struct MigrationFailed {
    step: usize,
    version: usize,
    message: String,
}

pub struct Store {
    conn: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCheckpoint {
    pub id: i64,
    pub offset: u64,
    pub lines: u64,
    pub size: u64,
    pub mtime: i64,
    pub head_len: u64,
    pub head_hash: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Totals {
    pub sessions: u64,
    pub requests: u64,
    pub priced_cost_usd: f64,
    pub reconciled_usd: f64,
    pub unpriced_requests: u64,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DailyCost {
    pub day: NaiveDate,
    pub model: String,
    pub requests: u64,
    pub cost_usd: f64,
    pub reconciled_usd: f64,
    pub unpriced: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CostStateDelta {
    pub session_id: String,
    pub ours_usd: f64,
    pub reconciled_usd: f64,
    pub claude_usd: Option<f64>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FileCounts {
    pub live: u64,
    pub retired: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawRow {
    pub file_id: i64,
    pub line_no: u64,
    pub kind: String,
    pub json: String,
    pub location: Location,
    pub retired: bool,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_writable(path).map_err(|error| opening_error(path, error))
    }

    fn open_writable(path: &Path) -> Result<Self> {
        create_stamped(path)?;
        refuse_unstamped(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("cannot restrict {} to its owner", path.display()))?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        refuse_unstamped_connection(&conn)?;
        use_wal(&conn)?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::migrated(conn)
    }

    pub fn open_read_only(path: &Path) -> Result<Self> {
        let open = || -> Result<Self> {
            refuse_unstamped(path)?;
            let conn = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            conn.busy_timeout(READ_BUSY_TIMEOUT)?;
            refuse_unstamped_connection(&conn)?;
            let store = Self { conn };
            store.refuse_unsupported_schema()?;
            Ok(store)
        };
        open().map_err(|error| opening_error(path, error))
    }

    pub fn reading<T>(&self, read: impl FnOnce(&Store) -> Result<T>) -> Result<T> {
        let snapshot = self.conn.unchecked_transaction()?;
        let result = read(self)?;
        snapshot.commit()?;
        Ok(result)
    }

    pub fn schema_version(&self) -> Result<usize> {
        user_version(&self.conn)
    }

    pub fn latest_schema_version() -> usize {
        schema::version_of(schema::MIGRATIONS.len() - 1)
    }

    fn refuse_unsupported_schema(&self) -> Result<()> {
        let current = self.schema_version()?;
        let latest = Self::latest_schema_version();
        if schema::PRE_RELEASE_VERSIONS.contains(&current) {
            return Err(PreRelease.into());
        }
        if current > latest {
            bail!(
                "database schema v{current} is newer than this claude-plus supports (v{latest}); upgrade claude-plus"
            );
        }
        Ok(())
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::migrated(Connection::open_in_memory()?)
    }

    fn migrated(conn: Connection) -> Result<Self> {
        Self::migrated_with(conn, schema::MIGRATIONS)
    }

    fn migrated_with(conn: Connection, migrations: &[&str]) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE);
        let mut store = Self { conn };
        store.refuse_unsupported_schema()?;
        let latest = schema::version_of(migrations.len() - 1);
        while store.schema_version()? < latest {
            let tx = begin_write(&mut store.conn)?;
            let Some((version, sql)) = schema::next_migration(migrations, user_version(&tx)?)
            else {
                break;
            };
            tx.execute_batch(sql)
                .map_err(|error| migration_failed(version, error))?;
            set_user_version(&tx, version)?;
            tx.commit().map_err(friendly)?;
        }
        Ok(store)
    }

    pub fn writer(&mut self) -> Result<Writer<'_>> {
        Ok(Writer {
            tx: begin_write(&mut self.conn)?,
        })
    }

    #[cfg(test)]
    pub(crate) fn set_busy_timeout(&self, timeout: std::time::Duration) {
        self.conn.busy_timeout(timeout).unwrap();
    }

    pub fn checkpoint(&self, path: &str) -> Result<Option<FileCheckpoint>> {
        checkpoint(&self.conn, path)
    }

    pub fn totals(&self) -> Result<Totals> {
        Ok(self.conn.query_row(
            "SELECT (SELECT COUNT(DISTINCT session_id) FROM requests), COUNT(*), COALESCE(SUM(cost_usd), 0),
                    SUM(CASE WHEN cost_usd IS NULL THEN 1 ELSE 0 END), MIN(ts), MAX(ts),
                    (SELECT COALESCE(SUM(cost_usd), 0) FROM adjustments) FROM requests",
            [],
            |r| {
                let reconciled_usd: f64 = r.get(6)?;
                Ok(Totals {
                    sessions: r.get::<_, i64>(0)? as u64,
                    requests: r.get::<_, i64>(1)? as u64,
                    priced_cost_usd: r.get::<_, f64>(2)? + reconciled_usd,
                    reconciled_usd,
                    unpriced_requests: r.get::<_, Option<i64>>(3)?.unwrap_or(0) as u64,
                    first_ts: r.get(4)?,
                    last_ts: r.get(5)?,
                })
            },
        )?)
    }

    pub fn daily_costs<Tz: TimeZone>(
        &self,
        since_ts: i64,
        until_ts: i64,
        tz: &Tz,
    ) -> Result<Vec<DailyCost>> {
        let mut grouped = BTreeMap::new();
        for (ts, model, cost) in self.costs_between("requests", since_ts, until_ts)? {
            let day = daily_entry(&mut grouped, ts, model, tz)?;
            day.requests += 1;
            match cost {
                Some(usd) => day.cost_usd += usd,
                None => day.unpriced += 1,
            }
        }
        for (ts, model, cost) in self.costs_between("adjustments", since_ts, until_ts)? {
            if let Some(usd) = cost {
                let day = daily_entry(&mut grouped, ts, model, tz)?;
                day.cost_usd += usd;
                day.reconciled_usd += usd;
            }
        }
        Ok(grouped.into_values().collect())
    }

    fn costs_between(
        &self,
        table: &str,
        since_ts: i64,
        until_ts: i64,
    ) -> Result<Vec<(i64, String, Option<f64>)>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT ts, model, cost_usd FROM {table} WHERE ts >= ?1 AND ts < ?2"
        ))?;
        let rows = statement.query_map([since_ts, until_ts], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn unpriced_models(&self) -> Result<Vec<(String, u64)>> {
        let mut statement = self.conn.prepare(
            "SELECT model, COUNT(*) FROM requests WHERE cost_usd IS NULL GROUP BY model ORDER BY COUNT(*) DESC",
        )?;
        let rows = statement.query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn cost_state_deltas(&self) -> Result<Vec<CostStateDelta>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT c.session_id, (SELECT COALESCE(SUM(cost_usd), 0) FROM requests r WHERE r.session_id = c.session_id),
                    (SELECT COALESCE(SUM(cost_usd), 0) FROM adjustments a WHERE a.session_id = c.session_id), c.total_cost_usd
             FROM cost_states c
             WHERE {} ORDER BY c.session_id",
            reconcile::LATEST_COST_STATE
        ))?;
        let rows = statement.query_map([], |r| {
            Ok(CostStateDelta {
                session_id: r.get(0)?,
                ours_usd: r.get(1)?,
                reconciled_usd: r.get(2)?,
                claude_usd: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn raw_files(&self) -> Result<Vec<(i64, String)>> {
        raw_files(&self.conn)
    }

    pub fn raw_records(&self, file_id: i64, kinds: &[&str]) -> Result<Vec<RawRow>> {
        raw_records(&self.conn, file_id, kinds)
    }

    pub fn begin_reindex(&mut self) -> Result<()> {
        self.conn.pragma_update(None, "foreign_keys", false)?;
        let cleared = self.writer().and_then(|mut writer| {
            writer.clear_derived()?;
            writer.mark_reindex_pending(true)?;
            writer.commit()
        });
        self.conn.pragma_update(None, "foreign_keys", true)?;
        cleared
    }

    pub fn reindex_pending(&self) -> Result<bool> {
        Ok(self
            .conn
            .prepare_cached("SELECT 1 FROM meta WHERE key = ?1")?
            .exists([REINDEX_PENDING])?)
    }

    pub fn parse_error_total(&self) -> Result<u64> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(SUM(parse_errors), 0) FROM files",
            [],
            |r| r.get::<_, i64>(0),
        )? as u64)
    }

    #[cfg(test)]
    pub(crate) fn dump(&self, table: &str) -> Vec<String> {
        let mut statement = self
            .conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1, 2"))
            .unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |r| {
                (0..columns)
                    .map(|i| {
                        r.get::<_, rusqlite::types::Value>(i)
                            .map(|v| format!("{v:?}"))
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map(|cells| cells.join("|"))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    pub fn file_counts(&self) -> Result<FileCounts> {
        let mut statement = self.conn.prepare("SELECT path FROM files")?;
        let mut counts = FileCounts::default();
        for path in statement.query_map([], |r| r.get::<_, String>(0))? {
            let path = path?;
            if live_path(&path) == path {
                counts.live += 1;
            } else {
                counts.retired += 1;
            }
        }
        Ok(counts)
    }
}

fn migration_failed(version: usize, error: rusqlite::Error) -> anyhow::Error {
    let message = match error {
        rusqlite::Error::SqlInputError { msg, .. } => msg,
        rusqlite::Error::SqliteFailure(_, Some(msg)) => msg,
        other => other.to_string(),
    };
    MigrationFailed {
        step: schema::step_of(version),
        version,
        message,
    }
    .into()
}

fn create_stamped(path: &Path) -> Result<()> {
    if std::fs::symlink_metadata(path).is_ok() {
        return Ok(());
    }
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut fresh = path.as_os_str().to_owned();
    fresh.push(format!(
        ".new-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let fresh = std::path::PathBuf::from(fresh);
    let created = stamp_new_file(&fresh).and_then(|()| match std::fs::hard_link(&fresh, path) {
        Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => {
            Err(error).with_context(|| format!("cannot create {}", path.display()))
        }
        _ => Ok(()),
    });
    let _ = std::fs::remove_file(&fresh);
    created
}

fn stamp_new_file(fresh: &Path) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
        .open(fresh)
        .with_context(|| format!("cannot create {}", fresh.display()))?;
    Connection::open(fresh)?.pragma_update(None, "application_id", APPLICATION_ID)?;
    Ok(())
}

fn refuse_unstamped(path: &Path) -> Result<()> {
    use rustix::fs::{Mode, OFlags};
    let flags = OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let fd = rustix::fs::open(path, flags, Mode::empty())
        .map_err(std::io::Error::from)
        .with_context(|| format!("cannot open {}", path.display()))?;
    if !is_claude_plus_store(fd) {
        return Err(NotOurs.into());
    }
    Ok(())
}

fn refuse_unstamped_connection(conn: &Connection) -> Result<()> {
    let id: i32 = conn
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(friendly)?;
    if id != APPLICATION_ID {
        return Err(NotOurs.into());
    }
    Ok(())
}

pub fn is_claude_plus_store(fd: std::os::fd::OwnedFd) -> bool {
    use rustix::fs::FileType;
    let regular = rustix::fs::fstat(&fd)
        .is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile);
    regular && is_claude_plus_database(&mut std::fs::File::from(fd))
}

fn opening_error(path: &Path, error: anyhow::Error) -> anyhow::Error {
    if error.is::<NotOurs>() {
        return anyhow::anyhow!(
            "{} is not a claude-plus store, so claude-plus leaves it as it is; move it aside, or set CLAUDE_PLUS_HOME to a directory of its own",
            path.display()
        );
    }
    if error.is::<PreRelease>() {
        return anyhow::anyhow!(
            "database at {} was created by a pre-release claude-plus; delete it and run claude-plus import",
            path.display()
        );
    }
    if let Some(failed) = error.downcast_ref::<MigrationFailed>() {
        return anyhow::anyhow!("cannot upgrade {}: {failed}", path.display());
    }
    match error.downcast_ref::<rusqlite::Error>() {
        Some(sqlite) if !error.is::<Busy>() => {
            anyhow::anyhow!("cannot open {}: {sqlite}", path.display())
        }
        _ => error,
    }
}

pub(crate) fn begin_write(conn: &mut Connection) -> Result<Transaction<'_>> {
    conn.transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(friendly)
}

fn use_wal(conn: &Connection) -> Result<()> {
    let deadline = std::time::Instant::now() + BUSY_TIMEOUT;
    loop {
        match conn.pragma_update(None, "journal_mode", "WAL") {
            Err(error)
                if error.sqlite_error_code() == Some(ErrorCode::DatabaseBusy)
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(WAL_RETRY);
            }
            result => return result.map_err(friendly),
        }
    }
}

fn user_version(conn: &Connection) -> Result<usize> {
    let version = conn
        .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
        .map_err(friendly)?;
    Ok(usize::try_from(version).unwrap_or(0))
}

fn set_user_version(conn: &Connection, version: usize) -> Result<()> {
    conn.pragma_update(None, "user_version", version as i64)
        .map_err(friendly)
}

pub(crate) fn friendly(error: rusqlite::Error) -> anyhow::Error {
    if error.sqlite_error_code() == Some(ErrorCode::DatabaseBusy) {
        Busy.into()
    } else {
        error.into()
    }
}

pub(crate) fn checkpoint(conn: &Connection, path: &str) -> Result<Option<FileCheckpoint>> {
    Ok(conn
        .prepare_cached(
            "SELECT id, offset, lines, size, mtime, head_len, head_hash FROM files WHERE path = ?1",
        )?
        .query_row([path], |r| {
            Ok(FileCheckpoint {
                id: r.get(0)?,
                offset: r.get::<_, i64>(1)? as u64,
                lines: r.get::<_, i64>(2)? as u64,
                size: r.get::<_, i64>(3)? as u64,
                mtime: r.get(4)?,
                head_len: r.get::<_, i64>(5)? as u64,
                head_hash: r.get::<_, i64>(6)? as u64,
            })
        })
        .optional()?)
}

fn daily_entry<'a, Tz: TimeZone>(
    grouped: &'a mut BTreeMap<(NaiveDate, String), DailyCost>,
    ts: i64,
    model: String,
    tz: &Tz,
) -> Result<&'a mut DailyCost> {
    let day = DateTime::from_timestamp(ts, 0)
        .context("timestamp out of range")?
        .with_timezone(tz)
        .date_naive();
    Ok(grouped.entry((day, model.clone())).or_insert(DailyCost {
        day,
        model,
        requests: 0,
        cost_usd: 0.0,
        reconciled_usd: 0.0,
        unpriced: 0,
    }))
}

fn live_path(path: &str) -> &str {
    match path.rsplit_once(GENERATION_MARK) {
        Some((live, generation))
            if !generation.is_empty() && generation.bytes().all(|b| b.is_ascii_digit()) =>
        {
            live
        }
        _ => path,
    }
}

pub(crate) fn raw_files(conn: &Connection) -> Result<Vec<(i64, String)>> {
    let mut statement = conn.prepare(
        "SELECT id, path FROM files WHERE id IN (SELECT DISTINCT file_id FROM raw_records) ORDER BY id",
    )?;
    let rows = statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub(crate) fn raw_records(conn: &Connection, file_id: i64, kinds: &[&str]) -> Result<Vec<RawRow>> {
    let path: String = conn
        .query_row("SELECT path FROM files WHERE id = ?1", [file_id], |r| {
            r.get(0)
        })
        .optional()?
        .with_context(|| format!("no transcript file with id {file_id}"))?;
    let live = live_path(&path);
    let retired = live != path;
    let location = Location::from_path(Path::new(live));
    let mut sql = String::from("SELECT line_no, kind, json FROM raw_records WHERE file_id = ?1");
    if !kinds.is_empty() {
        let placeholders = vec!["?"; kinds.len()].join(",");
        sql.push_str(&format!(" AND kind IN ({placeholders})"));
    }
    sql.push_str(" ORDER BY line_no");
    let mut statement = conn.prepare(&sql)?;
    let params = std::iter::once(&file_id as &dyn rusqlite::ToSql)
        .chain(kinds.iter().map(|kind| kind as &dyn rusqlite::ToSql));
    let rows = statement.query_map(rusqlite::params_from_iter(params), |r| {
        Ok(RawRow {
            file_id,
            line_no: r.get::<_, i64>(0)? as u64,
            kind: r.get(1)?,
            json: r.get(2)?,
            location: location.clone(),
            retired,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::TokenCounts;

    fn stamped(path: &Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "application_id", APPLICATION_ID)
            .unwrap();
        conn
    }

    fn request(key: &str, ts: i64, model: &str, cost: Option<f64>) -> RequestRow {
        RequestRow {
            key: key.into(),
            session_id: "s1".into(),
            ts,
            sent_ts: None,
            model: model.into(),
            tokens: TokenCounts {
                input: 1,
                output: 2,
                ..Default::default()
            },
            cost_usd: cost,
            is_sidechain: false,
            agent_id: None,
            skill: None,
            mcp_server: None,
            effort: None,
        }
    }

    #[test]
    fn requests_are_deduplicated_by_key() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        assert_eq!(
            w.request(&request("r1", 100, "m", Some(1.0))).unwrap(),
            RequestWrite::Inserted
        );
        assert_eq!(
            w.request(&request("r1", 100, "m", Some(1.0))).unwrap(),
            RequestWrite::Unchanged
        );
        w.commit().unwrap();
        assert_eq!(store.totals().unwrap().requests, 1);
    }

    #[test]
    fn streamed_records_keep_the_highest_output() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let mut partial = request("r1", 100, "m", Some(1.0));
        partial.tokens.output = 10;
        let mut last = request("r1", 100, "m", Some(3.0));
        last.tokens.output = 900;
        assert_eq!(w.request(&partial).unwrap(), RequestWrite::Inserted);
        assert_eq!(w.request(&last).unwrap(), RequestWrite::Updated);
        assert_eq!(w.request(&partial).unwrap(), RequestWrite::Unchanged);
        w.commit().unwrap();
        let totals = store.totals().unwrap();
        assert_eq!((totals.requests, totals.priced_cost_usd), (1, 3.0));
    }

    #[test]
    fn a_longer_stream_replaces_every_token_field() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.request(&request("r1", 100, "m", Some(1.0))).unwrap();
        let mut last = request("r1", 100, "m", Some(2.0));
        last.tokens = TokenCounts {
            input: 11,
            output: 12,
            thinking: 13,
            cache_read: 14,
            cache_write_5m: 15,
            cache_write_1h: 16,
        };
        assert_eq!(w.request(&last).unwrap(), RequestWrite::Updated);
        w.commit().unwrap();
        assert_eq!(
            store.dump("requests"),
            vec![
                r#"Text("r1")|Text("s1")|Integer(100)|Text("m")|Integer(11)|Integer(12)|Integer(13)|Integer(14)|Integer(15)|Integer(16)|Real(2.0)|Integer(0)|Null|Null|Null|Null|Null"#
            ]
        );
    }

    #[test]
    fn writer_stores_tools_attachments_and_compactions() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let file = w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        let tool_use = ToolUseRow {
            id: "t1".into(),
            session_id: "s1".into(),
            ts: 5,
            name: "Read".into(),
            input_chars: 7,
            target: Some("/w/a.rs".into()),
        };
        w.tool_use(&tool_use).unwrap();
        w.tool_use(&ToolUseRow {
            name: "Other".into(),
            ..tool_use
        })
        .unwrap();
        let result = ToolResultRow {
            tool_use_id: "t1".into(),
            session_id: "s1".into(),
            ts: 6,
            chars: 40,
            images: 2,
            is_error: true,
            is_sidechain: false,
        };
        w.tool_result(&result).unwrap();
        w.tool_result(&ToolResultRow { chars: 1, ..result })
            .unwrap();
        w.attachment(&AttachmentRow {
            file_id: file,
            line_no: 3,
            session_id: "s1".into(),
            ts: Some(7),
            kind: "instructions".into(),
            chars: 9,
            names: vec!["/h/CLAUDE.md".into(), "a".into()],
            removed: vec![],
        })
        .unwrap();
        w.compaction(&CompactionRow {
            file_id: file,
            line_no: 4,
            session_id: "s1".into(),
            ts: None,
            trigger: Some("auto".into()),
            pre_tokens: Some(100),
            post_tokens: None,
        })
        .unwrap();
        w.commit().unwrap();
        assert_eq!(
            store.dump("tool_uses"),
            vec![r#"Text("t1")|Text("s1")|Integer(5)|Text("Read")|Integer(7)|Text("/w/a.rs")"#]
        );
        assert_eq!(
            store.dump("tool_results"),
            vec![
                r#"Text("t1")|Text("s1")|Integer(6)|Integer(40)|Integer(2)|Integer(1)|Integer(0)"#
            ]
        );
        assert_eq!(
            store.dump("attachments"),
            vec![format!(
                r#"Integer({file})|Integer(3)|Text("s1")|Integer(7)|Text("instructions")|Integer(9)|Text("[\"/h/CLAUDE.md\",\"a\"]")|Text("[]")"#
            )]
        );
        assert_eq!(
            store.dump("compactions"),
            vec![format!(
                r#"Integer({file})|Integer(4)|Text("s1")|Null|Text("auto")|Integer(100)|Null"#
            )]
        );
    }

    #[test]
    fn cost_state_deltas_are_per_session() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let file = w.file("/p/proj/s.jsonl", 0, 0).unwrap();
        for (key, session, cost) in [("a", "s1", 0.25), ("b", "s2", 0.5), ("c", "s2", 0.25)] {
            w.request(&RequestRow {
                session_id: session.into(),
                ..request(key, 1, "m", Some(cost))
            })
            .unwrap();
        }
        for (line_no, session, total) in [(1, "s2", 0.8), (2, "s1", 0.3)] {
            w.cost_state(&CostStateRow {
                file_id: file,
                line_no,
                session_id: session.into(),
                total_cost_usd: Some(total),
                model_count: 1,
                json: "{}".into(),
            })
            .unwrap();
        }
        w.commit().unwrap();
        let deltas: Vec<(String, f64, Option<f64>)> = store
            .cost_state_deltas()
            .unwrap()
            .into_iter()
            .map(|d| (d.session_id, d.ours_usd, d.claude_usd))
            .collect();
        assert_eq!(
            deltas,
            vec![
                ("s1".to_string(), 0.25, Some(0.3)),
                ("s2".to_string(), 0.75, Some(0.8))
            ]
        );
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let store = Store::open_in_memory().unwrap();
        let enabled: bool = store
            .conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert!(enabled);
    }

    #[test]
    fn raw_rows_need_a_known_file() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        assert!(w.raw(42, 1, "user", "{}").is_err());
        drop(w);
        let error = store.raw_records(42, &[]).unwrap_err();
        assert_eq!(error.to_string(), "no transcript file with id 42");
    }

    #[test]
    fn checkpoints_round_trip() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let id = w.file("/p/a.jsonl", 10, 5).unwrap();
        assert_eq!(w.file("/p/a.jsonl", 10, 5).unwrap(), id);
        w.advance_file(
            id,
            &FileProgress {
                offset: 120,
                lines: 3,
                size: 200,
                mtime: 7,
                head_len: 64,
                head_hash: 0xdead_beef_dead_beef,
            },
            1,
        )
        .unwrap();
        w.commit().unwrap();
        assert_eq!(
            store.checkpoint("/p/a.jsonl").unwrap(),
            Some(FileCheckpoint {
                id,
                offset: 120,
                lines: 3,
                size: 200,
                mtime: 7,
                head_len: 64,
                head_hash: 0xdead_beef_dead_beef
            })
        );
        assert_eq!(store.parse_error_total().unwrap(), 1);
        assert_eq!(store.checkpoint("/p/missing.jsonl").unwrap(), None);
    }

    #[test]
    fn daily_costs_group_by_local_day_and_model() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let day1 = DateTime::parse_from_rfc3339("2026-10-01T23:30:00Z")
            .unwrap()
            .timestamp();
        w.request(&request("a", day1, "claude-opus-5-5", Some(1.5)))
            .unwrap();
        w.request(&request("b", day1 + 60, "claude-opus-5-5", Some(0.5)))
            .unwrap();
        w.request(&request("c", day1 + 120, "mystery", None))
            .unwrap();
        w.commit().unwrap();

        let utc = store.daily_costs(0, i64::MAX, &chrono::Utc).unwrap();
        assert_eq!(utc.len(), 2);
        assert_eq!(utc[0].day, NaiveDate::from_ymd_opt(2026, 10, 1).unwrap());
        assert_eq!(
            (utc[0].model.as_str(), utc[0].requests, utc[0].cost_usd),
            ("claude-opus-5-5", 2, 2.0)
        );
        assert_eq!((utc[1].model.as_str(), utc[1].unpriced), ("mystery", 1));

        let zurich_summer = store
            .daily_costs(
                0,
                i64::MAX,
                &chrono::FixedOffset::east_opt(2 * 3600).unwrap(),
            )
            .unwrap();
        assert_eq!(
            zurich_summer[0].day,
            NaiveDate::from_ymd_opt(2026, 10, 2).unwrap()
        );

        let totals = store.totals().unwrap();
        assert_eq!(
            (
                totals.requests,
                totals.unpriced_requests,
                totals.priced_cost_usd
            ),
            (3, 1, 2.0)
        );
        assert_eq!(
            store.unpriced_models().unwrap(),
            vec![("mystery".to_string(), 1)]
        );
    }

    #[test]
    fn daily_costs_exclude_requests_at_or_after_until() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.request(&request("a", 100, "m", Some(1.0))).unwrap();
        w.request(&request("b", 200, "m", Some(1.0))).unwrap();
        w.commit().unwrap();
        let rows = store.daily_costs(0, 200, &chrono::Utc).unwrap();
        assert_eq!(rows[0].requests, 1);
    }

    #[test]
    fn sessions_track_first_and_last_seen_and_titles() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        for ts in [Some(50), Some(10), None, Some(90)] {
            w.session(&SessionRow {
                id: "s1".into(),
                project: "p".into(),
                cwd: Some("/w".into()),
                ts,
            })
            .unwrap();
        }
        w.title("s1", "AI title", false).unwrap();
        w.title("s1", "My title", true).unwrap();
        w.title("s1", "Later AI title", false).unwrap();
        w.commit().unwrap();
        let (started, last, title): (i64, i64, String) = store
            .conn
            .query_row(
                "SELECT started_at, last_seen_at, title FROM sessions WHERE id = 's1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((started, last, title.as_str()), (10, 90, "My title"));
    }

    #[test]
    fn cost_state_deltas_compare_latest_claude_total_with_ours() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let file = w.file("/p/s1.jsonl", 0, 0).unwrap();
        w.request(&request("a", 1, "claude-opus-5-5", Some(0.25)))
            .unwrap();
        w.request(&request("b", 2, "claude-opus-5-5", Some(0.50)))
            .unwrap();
        w.cost_state(&CostStateRow {
            file_id: file,
            line_no: 3,
            session_id: "s1".into(),
            total_cost_usd: Some(0.20),
            model_count: 1,
            json: "{}".into(),
        })
        .unwrap();
        w.cost_state(&CostStateRow {
            file_id: file,
            line_no: 9,
            session_id: "s1".into(),
            total_cost_usd: Some(0.75),
            model_count: 1,
            json: "{}".into(),
        })
        .unwrap();
        w.commit().unwrap();
        assert_eq!(
            store.cost_state_deltas().unwrap(),
            vec![CostStateDelta {
                session_id: "s1".into(),
                ours_usd: 0.75,
                reconciled_usd: 0.0,
                claude_usd: Some(0.75)
            }]
        );
    }

    #[test]
    fn clear_derived_keeps_raw_and_files() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let file = w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        w.raw(file, 1, "assistant", "{}").unwrap();
        w.request(&request("a", 1, "m", Some(1.0))).unwrap();
        w.commit().unwrap();
        let mut w = store.writer().unwrap();
        w.clear_derived().unwrap();
        assert_eq!(
            w.raw_files().unwrap(),
            vec![(file, "/p/proj/s1.jsonl".to_string())]
        );
        let raw = w.raw_records(file, &["assistant"]).unwrap();
        assert_eq!(raw.len(), 1);
        assert_eq!(raw[0].location.project, "proj");
        w.commit().unwrap();
        assert_eq!(store.totals().unwrap().requests, 0);
    }

    #[test]
    fn beginning_a_reindex_clears_derived_rows_and_keeps_foreign_keys_on() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let file = w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        w.raw(file, 1, "assistant", "{}").unwrap();
        w.request(&request("a", 1, "m", Some(1.0))).unwrap();
        w.commit().unwrap();
        store.begin_reindex().unwrap();
        assert_eq!(store.totals().unwrap().requests, 0);
        assert_eq!(store.raw_records(file, &[]).unwrap().len(), 1);
        assert!(store.reindex_pending().unwrap());
        let enabled: bool = store
            .conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert!(enabled);
    }

    #[test]
    fn the_reindex_marker_round_trips() {
        let mut store = Store::open_in_memory().unwrap();
        assert!(!store.reindex_pending().unwrap());
        let mut w = store.writer().unwrap();
        w.mark_reindex_pending(true).unwrap();
        w.mark_reindex_pending(true).unwrap();
        w.commit().unwrap();
        assert!(store.reindex_pending().unwrap());
        let mut w = store.writer().unwrap();
        w.mark_reindex_pending(false).unwrap();
        w.commit().unwrap();
        assert!(!store.reindex_pending().unwrap());
    }

    #[test]
    fn abandoned_clear_derived_leaves_previous_data_intact() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        w.request(&request("a", 1, "m", Some(1.0))).unwrap();
        w.commit().unwrap();
        let mut w = store.writer().unwrap();
        w.clear_derived().unwrap();
        drop(w);
        assert_eq!(store.totals().unwrap().requests, 1);
    }

    #[test]
    fn raw_records_filter_by_kind_in_line_order() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let file = w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        w.raw(file, 3, "user", "{}").unwrap();
        w.raw(file, 2, "assistant", "{}").unwrap();
        w.raw(file, 1, "system", "{}").unwrap();
        w.commit().unwrap();
        let kinds = |wanted: &[&str]| -> Vec<(u64, String)> {
            store
                .raw_records(file, wanted)
                .unwrap()
                .into_iter()
                .map(|r| (r.line_no, r.kind))
                .collect()
        };
        assert_eq!(
            kinds(&["assistant", "user"]),
            vec![(2, "assistant".into()), (3, "user".into())]
        );
        assert_eq!(kinds(&[]).len(), 3);
    }

    #[test]
    fn retiring_a_file_keeps_its_rows_under_a_free_generation_path() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let first = w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        w.raw(first, 1, "attachment", "{}").unwrap();
        w.cost_state(&CostStateRow {
            file_id: first,
            line_no: 2,
            session_id: "s".into(),
            total_cost_usd: Some(1.0),
            model_count: 1,
            json: "{}".into(),
        })
        .unwrap();
        assert_eq!(w.retire_file(first).unwrap(), "/p/proj/s1.jsonl#gen-1");
        let second = w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        assert_ne!(second, first);
        w.raw(second, 1, "user", "{}").unwrap();
        assert_eq!(w.retire_file(second).unwrap(), "/p/proj/s1.jsonl#gen-2");
        w.commit().unwrap();

        assert_eq!(store.checkpoint("/p/proj/s1.jsonl").unwrap(), None);
        assert_eq!(
            store.raw_files().unwrap(),
            vec![
                (first, "/p/proj/s1.jsonl#gen-1".to_string()),
                (second, "/p/proj/s1.jsonl#gen-2".to_string())
            ]
        );
        let retired = store.raw_records(first, &[]).unwrap();
        assert_eq!(
            (
                retired[0].location.project.as_str(),
                retired[0].location.session_hint.as_str()
            ),
            ("proj", "s1")
        );
        let cost_states: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM cost_states", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cost_states, 1);
    }

    #[test]
    fn generation_numbers_ignore_similar_paths() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.file("/p/proj/s1.jsonl.bak#gen-7", 0, 0).unwrap();
        w.file("/p/proj/s10.jsonl#gen-3", 0, 0).unwrap();
        let live = w.file("/p/proj/s1.jsonl", 0, 0).unwrap();
        assert_eq!(w.retire_file(live).unwrap(), "/p/proj/s1.jsonl#gen-1");
    }

    #[test]
    fn reopening_a_file_store_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        {
            let mut store = Store::open(&path).unwrap();
            let mut w = store.writer().unwrap();
            w.request(&request("r1", 1, "m", Some(1.0))).unwrap();
            w.commit().unwrap();
        }
        assert_eq!(Store::open(&path).unwrap().totals().unwrap().requests, 1);
    }

    #[test]
    fn a_second_writer_is_asked_to_retry() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        let mut first = Store::open(&path).unwrap();
        let mut second = Store::open(&path).unwrap();
        second.set_busy_timeout(std::time::Duration::from_millis(10));
        let _held = first.writer().unwrap();
        let error = second.writer().err().expect("must be busy");
        assert_eq!(
            error.to_string(),
            "another claude-plus is writing, try again in a moment"
        );
        assert!(error.is::<Busy>());
        let error = second
            .reconcile(&crate::pricing::PriceTable::bundled())
            .unwrap_err();
        assert!(error.is::<Busy>());
    }

    #[test]
    fn writer_reads_checkpoints_inside_its_transaction() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        let id = w.file("/p/a.jsonl", 3, 4).unwrap();
        let checkpoint = w.checkpoint("/p/a.jsonl").unwrap().unwrap();
        assert_eq!(
            (checkpoint.id, checkpoint.size, checkpoint.mtime),
            (id, 3, 4)
        );
        assert_eq!(w.checkpoint("/p/b.jsonl").unwrap(), None);
    }

    #[test]
    fn concurrent_first_opens_create_the_schema_once() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        let start = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            let opens: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        start.wait();
                        Store::open(&path).map(|store| store.schema_version().unwrap())
                    })
                })
                .collect();
            for open in opens {
                assert_eq!(
                    open.join().unwrap().unwrap(),
                    Store::latest_schema_version()
                );
            }
        });
    }

    #[test]
    fn pre_release_stores_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        let expected = format!(
            "database at {} was created by a pre-release claude-plus; delete it and run claude-plus import",
            path.display()
        );
        for version in 1..=6 {
            stamped(&path)
                .pragma_update(None, "user_version", version)
                .unwrap();
            let error = Store::open(&path).err().expect("must refuse");
            assert_eq!(error.to_string(), expected);
            let error = Store::open_read_only(&path).err().expect("must refuse");
            assert_eq!(format!("{error:#}"), expected);
            let version_after: i64 = Connection::open(&path)
                .unwrap()
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
            assert_eq!(version_after, version);
        }
    }

    #[test]
    fn a_new_store_is_at_the_latest_version() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.schema_version().unwrap(), 17);
        assert_eq!(Store::latest_schema_version(), 17);
        assert!(!store.reindex_pending().unwrap());
    }

    #[test]
    fn a_first_release_store_is_upgraded_in_place() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        {
            let store = Store::migrated_with(stamped(&path), &schema::MIGRATIONS[..1]).unwrap();
            assert_eq!(store.schema_version().unwrap(), 7);
        }
        Connection::open(&path)
            .unwrap()
            .execute(
                "INSERT INTO requests (key, session_id, ts, model, input, output, thinking, cache_read,
                   cache_write_5m, cache_write_1h, cost_usd, is_sidechain)
                 VALUES ('r1', 's1', 1, 'm', 0, 0, 0, 0, 0, 0, 1.0, 0)",
                [],
            )
            .unwrap();
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 17);
        assert_eq!(store.totals().unwrap().requests, 1);
        assert!(store.reindex_pending().unwrap());
        assert_eq!(store.dump("events"), Vec::<String>::new());
        assert_eq!(store.dump("limit_samples"), Vec::<String>::new());
        assert_eq!(store.dump("live_breakdowns"), Vec::<String>::new());
    }

    #[test]
    fn stored_tool_results_are_rebuilt_to_learn_which_thread_they_belong_to() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        let before = 7;
        {
            let store =
                Store::migrated_with(stamped(&path), &schema::MIGRATIONS[..before]).unwrap();
            assert_eq!(store.schema_version().unwrap(), 13);
            store
                .conn
                .execute(
                    "INSERT INTO tool_results (tool_use_id, session_id, ts, chars, images, is_error)
                     VALUES ('t1', 's1', 1, 10, 0, 0)",
                    [],
                )
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 17);
        assert!(store.reindex_pending().unwrap());
    }

    #[test]
    fn stored_attachments_are_rebuilt_to_learn_what_they_removed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        {
            let store = Store::migrated_with(stamped(&path), &schema::MIGRATIONS[..9]).unwrap();
            assert_eq!(store.schema_version().unwrap(), 15);
            assert!(!store.reindex_pending().unwrap());
            store
                .conn
                .execute(
                    "INSERT INTO attachments (file_id, line_no, session_id, ts, kind, chars, names)
                     VALUES (1, 1, 's1', 1, 'deferred_tools_delta', 0, '[]')",
                    [],
                )
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 17);
        assert!(store.reindex_pending().unwrap());
    }

    #[test]
    fn a_failing_migration_names_its_step_without_repeating_the_sql() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        let broken = [
            schema::MIGRATIONS[0],
            schema::MIGRATIONS[1],
            "CREATE TABLE files (id INTEGER PRIMARY KEY);",
        ];
        let error = Store::migrated_with(Connection::open(&path).unwrap(), &broken)
            .err()
            .expect("must fail");
        assert_eq!(
            error.to_string(),
            "schema migration step 3 (v9) failed: table files already exists"
        );
        assert_eq!(
            opening_error(&path, error).to_string(),
            format!(
                "cannot upgrade {}: schema migration step 3 (v9) failed: table files already exists",
                path.display()
            )
        );
        let version: i64 = Connection::open(&path)
            .unwrap()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 8);
    }

    #[test]
    fn a_new_store_carries_the_claude_plus_application_id_in_its_header() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        drop(Store::open(&path).unwrap());
        assert!(is_claude_plus_database(
            &mut std::fs::File::open(&path).unwrap()
        ));
        assert!(!is_claude_plus_database(&mut &b"SQLite format 3\0"[..]));
        let other = temp.path().join("other.db");
        Connection::open(&other)
            .unwrap()
            .execute_batch("CREATE TABLE notes (x)")
            .unwrap();
        assert!(!is_claude_plus_database(
            &mut std::fs::File::open(&other).unwrap()
        ));
    }

    const REFUSAL: &str = "is not a claude-plus store, so claude-plus leaves it as it is; move it aside, or set CLAUDE_PLUS_HOME to a directory of its own";

    fn assert_refused(path: &Path) {
        let before = std::fs::read(path).unwrap();
        let error = Store::open(path).err().expect("must refuse").to_string();
        assert_eq!(error, format!("{} {REFUSAL}", path.display()));
        let error = Store::open_read_only(path)
            .err()
            .expect("must refuse")
            .to_string();
        assert_eq!(error, format!("{} {REFUSAL}", path.display()));
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[test]
    fn a_store_without_the_claude_plus_stamp_is_refused_and_left_as_it_was() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        drop(Store::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "application_id", 0).unwrap();
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
        drop(conn);
        assert_refused(&path);
        assert!(!is_claude_plus_database(
            &mut std::fs::File::open(&path).unwrap()
        ));
    }

    #[test]
    fn a_foreign_database_with_claude_plus_table_names_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        for version in [9, 16] {
            let path = temp.path().join(format!("v{version}.db"));
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE sessions (id TEXT); INSERT INTO sessions VALUES ('precious');
                 CREATE TABLE meta (key TEXT, value TEXT);",
            )
            .unwrap();
            conn.pragma_update(None, "user_version", version).unwrap();
            drop(conn);
            assert_refused(&path);
        }
        let other = temp.path().join("other.db");
        Connection::open(&other)
            .unwrap()
            .pragma_update(None, "application_id", 7)
            .unwrap();
        assert_refused(&other);
    }

    #[test]
    fn an_existing_file_that_is_no_store_is_refused_and_left_as_it_was() {
        let temp = tempfile::tempdir().unwrap();
        for (name, content) in [
            ("empty", &b""[..]),
            ("text", b"mine"),
            ("header", b"SQLite format 3\0"),
        ] {
            let path = temp.path().join(name);
            std::fs::write(&path, content).unwrap();
            assert_refused(&path);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_refused_file_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        std::fs::write(&path, "mine").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Store::open(&path).is_err());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_symlink_is_not_followed_to_create_a_store() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        let target = temp.path().join("elsewhere.db");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(Store::open(&path).is_err());
        assert!(!target.exists());
    }

    #[test]
    fn a_new_store_leaves_nothing_but_its_own_files() {
        let temp = tempfile::tempdir().unwrap();
        drop(Store::open(&temp.path().join("data.db")).unwrap());
        let mut names: Vec<_> = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| !name.starts_with("data.db-"))
            .collect();
        names.sort();
        assert_eq!(names, ["data.db"]);
    }

    #[test]
    fn newer_schema_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        stamped(&path)
            .pragma_update(None, "user_version", 99)
            .unwrap();
        let expected = format!(
            "database schema v99 is newer than this claude-plus supports (v{}); upgrade claude-plus",
            Store::latest_schema_version()
        );
        let error = Store::open(&path).err().expect("must refuse").to_string();
        assert_eq!(error, expected);
        let error = Store::open_read_only(&path)
            .err()
            .expect("must refuse")
            .to_string();
        assert_eq!(error, expected);
    }

    #[test]
    fn read_only_open_neither_creates_nor_migrates() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        assert!(Store::open_read_only(&path).is_err());
        assert!(!path.exists());
        drop(Store::migrated_with(stamped(&path), &schema::MIGRATIONS[..1]).unwrap());
        let store = Store::open_read_only(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 7);
    }

    #[test]
    fn reads_made_together_ignore_commits_made_between_them() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        let mut store = Store::open(&path).unwrap();
        let mut w = store.writer().unwrap();
        w.request(&request("r1", 100, "m", Some(1.0))).unwrap();
        w.commit().unwrap();
        let reader = Store::open_read_only(&path).unwrap();
        let (before, after) = reader
            .reading(|r| {
                let before = r.session_totals("s1")?.requests;
                let mut w = store.writer()?;
                w.request(&request("r2", 200, "m", Some(1.0)))?;
                w.commit()?;
                Ok((before, r.session_totals("s1")?.requests))
            })
            .unwrap();
        assert_eq!((before, after), (1, 1));
        assert_eq!(reader.session_totals("s1").unwrap().requests, 2);
    }

    #[test]
    fn readers_wait_briefly_for_a_busy_database() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        Store::open(&path).unwrap();
        let reader = Store::open_read_only(&path).unwrap();
        let timeout: i64 = reader
            .conn
            .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
            .unwrap();
        assert_eq!(timeout, READ_BUSY_TIMEOUT.as_millis() as i64);
        assert_eq!(READ_BUSY_TIMEOUT, std::time::Duration::from_millis(200));
    }

    #[cfg(unix)]
    #[test]
    fn database_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.db");
        Store::open(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
