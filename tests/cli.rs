use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;

struct Env {
    _temp: tempfile::TempDir,
    claude: PathBuf,
    plus: PathBuf,
}

fn copy_dir(from: &Path, to: &Path) {
    for entry in walkdir::WalkDir::new(from) {
        let entry = entry.unwrap();
        let target = to.join(entry.path().strip_prefix(from).unwrap());
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn env() -> Env {
    let temp = tempfile::tempdir().unwrap();
    let claude = temp.path().join("claude");
    let plus = temp.path().join("plus");
    copy_dir(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .as_path(),
        &claude,
    );
    Env {
        _temp: temp,
        claude,
        plus,
    }
}

fn cmd(env: &Env, args: &[&str]) -> Command {
    let mut command = Command::cargo_bin("claude-plus").unwrap();
    command
        .args(args)
        .env("CLAUDE_CONFIG_DIR", &env.claude)
        .env("CLAUDE_PLUS_HOME", &env.plus)
        .env("TZ", "UTC")
        .env("CLAUDE_PLUS_NOW", "2026-10-03T12:00:00Z")
        .env("NO_COLOR", "1");
    command
}

#[test]
fn import_reports_and_is_idempotent() {
    let env = env();
    cmd(&env, &["import"])
        .assert()
        .success()
        .stdout(predicate::str::contains("5 new requests"))
        .stdout(predicate::str::contains("1 unparsable line"))
        .stdout(predicate::str::contains("$0.0812"))
        .stdout(predicate::str::contains("vanished").not())
        .stdout(predicate::str::contains("Could not read").not());
    cmd(&env, &["import"])
        .assert()
        .success()
        .stdout(predicate::str::contains("0 new requests"));
}

#[test]
fn stats_shows_daily_costs_per_model() {
    let env = env();
    cmd(&env, &["stats", "--days", "36500"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2026-10-01"))
        .stdout(predicate::str::contains("claude-opus-5-5"))
        .stdout(predicate::str::contains("$0.0640"))
        .stdout(predicate::str::contains("2026-10-02"))
        .stdout(predicate::str::contains("claude-sonnet-5-5"))
        .stdout(predicate::str::contains("$0.0045"))
        .stdout(predicate::str::contains("claude-haiku-4-5-20251001"))
        .stdout(predicate::str::contains("Total"))
        .stdout(predicate::str::contains("$0.0812"));
}

#[test]
fn doctor_checks_setup() {
    let env = env();
    cmd(&env, &["import"]).assert().success();
    cmd(&env, &["doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains("4 transcript files"))
        .stdout(predicate::str::contains("1 unparsable line"))
        .stdout(predicate::str::contains("cleanupPeriodDays: 30 (default)"))
        .stdout(predicate::str::contains("prices 2026-10-10"))
        .stdout(predicate::str::contains("Cross-check:"));
}

#[test]
fn reindex_rebuilds_the_same_totals() {
    let env = env();
    cmd(&env, &["import"]).assert().success();
    cmd(&env, &["reindex"])
        .assert()
        .success()
        .stdout(predicate::str::contains("5 requests"));
    let adjustments: i64 = rusqlite::Connection::open(env.plus.join("data.db"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM adjustments", [], |r| r.get(0))
        .unwrap();
    assert_eq!(adjustments, 2);
    cmd(&env, &["stats", "--days", "36500"])
        .assert()
        .success()
        .stdout(predicate::str::contains("$0.0812"))
        .stdout(predicate::str::contains("includes $0.0111 reconciled"));
}

#[test]
fn missing_claude_dir_is_friendly() {
    let temp = tempfile::tempdir().unwrap();
    Command::cargo_bin("claude-plus")
        .unwrap()
        .args(["stats"])
        .env("CLAUDE_CONFIG_DIR", temp.path().join("nope"))
        .env("CLAUDE_PLUS_HOME", temp.path().join("plus"))
        .assert()
        .success()
        .stdout(predicate::str::contains("No Claude Code transcripts found"));
}

fn stdout_of(env: &Env, args: &[&str]) -> String {
    let output = cmd(env, args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).unwrap()
}

fn write_unpriced_transcript(env: &Env) {
    let dir = env.claude.join("projects/-work-other");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("44444444-4444-4444-4444-444444444444.jsonl"),
        concat!(
            r#"{"type":"assistant","sessionId":"44444444-4444-4444-4444-444444444444","timestamp":"2026-10-03T09:00:00Z","requestId":"req_mystery","message":{"id":"msg_mystery","model":"claude-mystery-9","content":[{"type":"text","text":"x"}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            "\n"
        ),
    )
    .unwrap();
}

fn stdout_of_with(env: &Env, args: &[&str], vars: &[(&str, &str)]) -> String {
    let mut command = cmd(env, args);
    for (key, value) in vars {
        command.env(key, value);
    }
    String::from_utf8(command.assert().success().get_output().stdout.clone()).unwrap()
}

const STATS_TABLE: &str = "\
┌────────────┬───────────────────────────┬──────────┬─────────┐
│ Day        ┆ Model                     ┆ Requests ┆    Cost │
╞════════════╪═══════════════════════════╪══════════╪═════════╡
│ 2026-10-01 ┆ claude-opus-5-5           ┆        1 ┆ $0.0640 │
│ 2026-10-02 ┆ claude-haiku-4-5-20251001 ┆        1 ┆ $0.0004 │
│ 2026-10-02 ┆ claude-sonnet-5-5         ┆        1 ┆ $0.0045 │
│ 2026-10-03 ┆ claude-haiku-4-5-20251001 ┆        - ┆ $0.0011 │
│ 2026-10-03 ┆ claude-sonnet-5-5         ┆        2 ┆ $0.0112 │
├────────────┼───────────────────────────┼──────────┼─────────┤
│ Total      ┆                           ┆        5 ┆ $0.0812 │
└────────────┴───────────────────────────┴──────────┴─────────┘
  includes $0.0111 reconciled from Claude Code session counters
";

#[test]
fn import_output_is_exact() {
    let env = env();
    assert_eq!(
        stdout_of(&env, &["import"]),
        "✻ Imported 16 lines from 4 changed files · 5 new requests · 1 unparsable line\n  History: 5 requests across 2 sessions · $0.0812 priced\n"
    );
}

#[test]
fn stats_output_is_exact() {
    let env = env();
    assert_eq!(stdout_of(&env, &["stats", "--days", "3"]), STATS_TABLE);
}

#[test]
fn stats_window_is_calendar_aligned() {
    let env = env();
    let one_day = stdout_of(&env, &["stats", "--days", "1"]);
    assert!(!one_day.contains("2026-10-02"), "{one_day}");
    let two_days = stdout_of(&env, &["stats", "--days", "2"]);
    assert!(!two_days.contains("2026-10-01"), "{two_days}");
    assert!(two_days.contains("2026-10-02"), "{two_days}");
    let three_days = stdout_of(&env, &["stats", "--days", "3"]);
    assert!(three_days.contains("2026-10-01"), "{three_days}");
}

#[test]
fn doctor_output_is_exact() {
    let env = env();
    cmd(&env, &["import"]).assert().success();
    let expected = format!(
        "✻ claude-plus doctor
  Claude dir:  {claude} · 4 transcript files
  Data dir:    {plus}
  Retention:   cleanupPeriodDays: 30 (default)
               Claude Code deletes older transcripts, claude-plus keeps its own copy
  Price table: bundled prices 2026-10-10
  Imported:    5 requests in 4 files · 1 unparsable line
  Cross-check: 1 session · transcripts $0.0012 · Claude Code $0.0123 · reconciled $0.0123
  Uncounted:   1 session with subagents has no Claude Code counter, its subagent output may be understated
",
        claude = env.claude.display(),
        plus = env.plus.display()
    );
    assert_eq!(stdout_of(&env, &["doctor"]), expected);
}

#[test]
fn doctor_shows_configured_retention() {
    let env = env();
    std::fs::write(
        env.claude.join("settings.json"),
        r#"{"cleanupPeriodDays": 90}"#,
    )
    .unwrap();
    cmd(&env, &["doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Retention:   cleanupPeriodDays: 90\n",
        ))
        .stdout(predicate::str::contains("(default)").not())
        .stdout(predicate::str::contains("deletes older transcripts").not());
}

#[test]
fn doctor_warns_about_short_configured_retention() {
    let env = env();
    for days in [14, 30] {
        std::fs::write(
            env.claude.join("settings.json"),
            format!(r#"{{"cleanupPeriodDays": {days}}}"#),
        )
        .unwrap();
        cmd(&env, &["doctor"])
            .assert()
            .success()
            .stdout(predicate::str::contains(format!(
                "  Retention:   cleanupPeriodDays: {days}\n               Claude Code deletes older transcripts, claude-plus keeps its own copy\n"
            )));
    }
}

#[test]
fn unpriced_models_are_surfaced() {
    let env = env();
    write_unpriced_transcript(&env);
    cmd(&env, &["import"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 request without a known price"));
    cmd(&env, &["doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "  Unpriced:    claude-mystery-9 (1 request)\n",
        ));
    let stats = stdout_of(&env, &["stats", "--days", "3"]);
    assert!(stats.contains("$0.0000 (+1 unpriced)"), "{stats}");
    let total_row = stats.lines().find(|line| line.contains("Total")).unwrap();
    assert!(total_row.contains("(+1 unpriced)"), "{total_row}");
}

#[test]
fn import_without_transcripts_is_friendly() {
    let temp = tempfile::tempdir().unwrap();
    Command::cargo_bin("claude-plus")
        .unwrap()
        .args(["import"])
        .env("CLAUDE_CONFIG_DIR", temp.path().join("nope"))
        .env("CLAUDE_PLUS_HOME", temp.path().join("plus"))
        .assert()
        .success()
        .stdout(predicate::str::contains("No Claude Code transcripts found"));
}

#[test]
fn stats_groups_days_with_the_offset_in_force_at_each_request() {
    let env = env();
    let dir = env.claude.join("projects/-work-dst");
    std::fs::create_dir_all(&dir).unwrap();
    let line = |id: &str, ts: &str| {
        format!(
            r#"{{"type":"assistant","sessionId":"33333333-3333-3333-3333-333333333333","timestamp":"{ts}","requestId":"{id}","message":{{"id":"{id}","model":"claude-sonnet-5-5","content":[{{"type":"text","text":"x"}}],"usage":{{"input_tokens":1,"output_tokens":1}}}}}}"#
        )
    };
    std::fs::write(
        dir.join("33333333-3333-3333-3333-333333333333.jsonl"),
        format!(
            "{}\n{}\n",
            line("req_before", "2026-10-24T22:30:00Z"),
            line("req_after", "2026-10-25T09:00:00Z")
        ),
    )
    .unwrap();
    let output = stdout_of_with(
        &env,
        &["stats", "--days", "14"],
        &[
            ("TZ", "Europe/Zurich"),
            ("CLAUDE_PLUS_NOW", "2026-11-07T12:00:00+01:00"),
        ],
    );
    assert!(!output.contains("2026-10-24"), "{output}");
    assert!(output.contains("2026-10-25"), "{output}");
}

#[test]
fn stats_excludes_requests_after_today() {
    let env = env();
    let output = stdout_of_with(
        &env,
        &["stats", "--days", "3"],
        &[("CLAUDE_PLUS_NOW", "2026-10-01T12:00:00Z")],
    );
    assert!(output.contains("2026-10-01"), "{output}");
    assert!(!output.contains("2026-10-02"), "{output}");
}

#[test]
fn invalid_now_override_is_rejected() {
    let env = env();
    cmd(&env, &["stats"])
        .env("CLAUDE_PLUS_NOW", "yesterday")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "CLAUDE_PLUS_NOW must be an RFC 3339 timestamp, got \"yesterday\"",
        ));
}

#[test]
fn invalid_now_override_fails_before_importing() {
    let env = env();
    cmd(&env, &["stats"])
        .env("CLAUDE_PLUS_NOW", "yesterday")
        .assert()
        .failure()
        .stdout(predicate::str::is_empty());
    assert!(!env.plus.join("data.db").exists());
}

fn assistant_line(session: &str, request: &str, ts: &str, sidechain: bool) -> String {
    format!(
        r#"{{"type":"assistant","sessionId":"{session}","isSidechain":{sidechain},"timestamp":"{ts}","requestId":"{request}","message":{{"id":"{request}","model":"claude-sonnet-5-5","content":[{{"type":"text","text":"x"}}],"usage":{{"input_tokens":1,"output_tokens":1000}}}}}}"#
    )
}

#[test]
fn doctor_reports_reconciliation() {
    let env = env();
    let dir = env.claude.join("projects/-work-extra");
    let uncounted = "55555555-5555-5555-5555-555555555555";
    let over = "66666666-6666-6666-6666-666666666666";
    std::fs::create_dir_all(dir.join(uncounted).join("subagents")).unwrap();
    std::fs::write(
        dir.join(uncounted).join("subagents/agent-u1.jsonl"),
        assistant_line(uncounted, "req_u1", "2026-10-03T10:00:00Z", true) + "\n",
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("{over}.jsonl")),
        format!(
            "{}\n{}\n",
            assistant_line(over, "req_o1", "2026-10-03T10:00:00Z", false),
            format_args!(
                r#"{{"type":"cost-state","sessionId":"{over}","totalCostUSD":0.00425,"modelUsage":{{"claude-sonnet-5-5":{{"inputTokens":1,"outputTokens":400,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"costUSD":0.00425}}}}}}"#
            )
        ),
    )
    .unwrap();
    cmd(&env, &["import"]).assert().success();
    let doctor = stdout_of(&env, &["doctor"]);
    assert!(
        doctor.ends_with(
            "  Cross-check: 2 sessions · transcripts $0.0112 · Claude Code $0.0165 · reconciled $0.0223
  Uncounted:   2 sessions with subagents have no Claude Code counter, their subagent output may be understated
  Over:        1 session where transcripts exceed Claude Code's counter
"
        ),
        "{doctor}"
    );
}

#[test]
fn stats_shows_reconciled_share() {
    let env = env();
    let today = stdout_of(&env, &["stats", "--days", "1"]);
    assert!(
        today.ends_with(
            "├────────────┼───────────────────────────┼──────────┼─────────┤
│ Total      ┆                           ┆        2 ┆ $0.0123 │
└────────────┴───────────────────────────┴──────────┴─────────┘
  includes $0.0111 reconciled from Claude Code session counters
"
        ),
        "{today}"
    );
    let before = stdout_of_with(
        &env,
        &["stats", "--days", "3"],
        &[("CLAUDE_PLUS_NOW", "2026-10-02T12:00:00Z")],
    );
    assert!(!before.contains("reconciled"), "{before}");
}

#[test]
fn zero_days_is_rejected() {
    let env = env();
    cmd(&env, &["stats", "--days", "0"]).assert().failure();
    cmd(&env, &["stats", "--days", "36501"]).assert().failure();
}

fn doctor_header(env: &Env, transcript_files: u64, imported: &str) -> String {
    format!(
        "✻ claude-plus doctor
  Claude dir:  {claude} · {transcript_files} transcript files
  Data dir:    {plus}
  Retention:   cleanupPeriodDays: 30 (default)
               Claude Code deletes older transcripts, claude-plus keeps its own copy
  Price table: bundled prices 2026-10-10
  Imported:    {imported}
",
        claude = env.claude.display(),
        plus = env.plus.display()
    )
}

#[test]
fn doctor_lists_counter_only_unpriced_models() {
    let env = env();
    let session = "77777777-7777-7777-7777-777777777777";
    let dir = env.claude.join("projects/-work-extra");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{session}.jsonl")),
        format!(
            "{}\n{}\n",
            assistant_line(session, "req_m1", "2026-10-03T10:00:00Z", false),
            format_args!(
                r#"{{"type":"cost-state","sessionId":"{session}","totalCostUSD":0.0101,"modelUsage":{{"claude-sonnet-5-5":{{"inputTokens":1,"outputTokens":1000}},"claude-mystery-9":{{"inputTokens":10,"outputTokens":10}}}}}}"#
            )
        ),
    )
    .unwrap();
    cmd(&env, &["import"]).assert().success();
    let expected = doctor_header(&env, 5, "6 requests in 5 files · 1 unparsable line")
        + "  Unpriced:    claude-mystery-9 (Claude Code counter only)
  Cross-check: 2 sessions · transcripts $0.0112 · Claude Code $0.0224 · reconciled $0.0223
  Uncounted:   1 session with subagents has no Claude Code counter, its subagent output may be understated
";
    assert_eq!(stdout_of(&env, &["doctor"]), expected);
}

struct FileState {
    user_version: i64,
    size: u64,
    modified: std::time::SystemTime,
}

fn file_state(database: &Path) -> FileState {
    let metadata = std::fs::metadata(database).unwrap();
    let user_version =
        rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
    FileState {
        user_version,
        size: metadata.len(),
        modified: metadata.modified().unwrap(),
    }
}

#[test]
fn doctor_leaves_an_older_database_untouched() {
    let env = env();
    cmd(&env, &["import"]).assert().success();
    let database = env.plus.join("data.db");
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(
            "DROP TABLE adjustments; DROP INDEX cost_states_by_session; PRAGMA user_version = 2;",
        )
        .unwrap();
    let before = file_state(&database);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let doctor = stdout_of(&env, &["doctor"]);
    assert!(
        doctor.ends_with("  Database:    schema v2, run claude-plus import to upgrade\n"),
        "{doctor}"
    );
    let after = file_state(&database);
    assert_eq!(after.user_version, 2);
    assert_eq!((after.size, after.modified), (before.size, before.modified));
}

#[test]
fn doctor_on_a_fresh_home_creates_nothing() {
    let env = env();
    let expected = format!(
        "✻ claude-plus doctor
  Claude dir:  {claude} · 4 transcript files
  Data dir:    {plus}
  Retention:   cleanupPeriodDays: 30 (default)
               Claude Code deletes older transcripts, claude-plus keeps its own copy
  Price table: bundled prices 2026-10-10
  Database:    not created yet, run claude-plus import
",
        claude = env.claude.display(),
        plus = env.plus.display()
    );
    assert_eq!(stdout_of(&env, &["doctor"]), expected);
    assert!(!env.plus.exists());
}

#[test]
fn doctor_reads_a_database_with_live_wal_files() {
    let env = env();
    cmd(&env, &["import"]).assert().success();
    let database = env.plus.join("data.db");
    let writer = rusqlite::Connection::open(&database).unwrap();
    writer
        .execute(
            "DELETE FROM adjustments WHERE model = 'claude-haiku-4-5-20251001'",
            [],
        )
        .unwrap();
    assert!(env.plus.join("data.db-wal").exists());
    assert!(env.plus.join("data.db-shm").exists());
    let doctor = stdout_of(&env, &["doctor"]);
    assert!(
        doctor.contains(
            "  Cross-check: 1 session · transcripts $0.0012 · Claude Code $0.0123 · reconciled $0.0112\n"
        ),
        "{doctor}"
    );
    let remaining: i64 = writer
        .query_row("SELECT COUNT(*) FROM adjustments", [], |r| r.get(0))
        .unwrap();
    assert_eq!(remaining, 1);
}

#[test]
fn import_upgrades_an_older_database_quietly() {
    let env = env();
    cmd(&env, &["import"]).assert().success();
    let database = env.plus.join("data.db");
    {
        let conn = rusqlite::Connection::open(&database).unwrap();
        conn.execute(
            "UPDATE raw_records SET json = ?1 WHERE kind = 'user' AND json LIKE '%hello%'",
            [r#"{"type":"user","sessionId":"11111111-1111-1111-1111-111111111111","timestamp":"2026-10-01T10:00:00Z","message":{"role":"user","content":"hello my password"}}"#],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 4).unwrap();
    }
    cmd(&env, &["import"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty())
        .stdout(predicate::str::contains("0 new requests"));
    let conn = rusqlite::Connection::open(&database).unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    let leaked: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM raw_records WHERE json LIKE '%password%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!((version, leaked), (5, 0));
}

#[test]
fn empty_plus_home_falls_back_to_home() {
    let env = env();
    let home = env._temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    cmd(&env, &["import"])
        .env("CLAUDE_PLUS_HOME", "")
        .env("HOME", &home)
        .current_dir(env._temp.path())
        .assert()
        .success();
    assert!(!env._temp.path().join("data.db").exists());
    assert!(home.join(".claude-plus/data.db").exists());
    assert!(!env.plus.exists());
}

#[test]
fn reconcile_failure_still_prints_the_import_report() {
    let env = env();
    cmd(&env, &["import"]).assert().success();
    rusqlite::Connection::open(env.plus.join("data.db"))
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER no_adjustments BEFORE INSERT ON adjustments
             BEGIN SELECT RAISE(ABORT, 'adjustments are locked'); END;",
        )
        .unwrap();
    write_unpriced_transcript(&env);
    cmd(&env, &["import"])
        .assert()
        .failure()
        .stdout(
            "✻ Imported 1 line from 2 changed files · 1 new request · 0 unparsable lines\n  History: 6 requests across 3 sessions · $0.0812 priced\n  1 request without a known price (see `claude-plus doctor`)\n",
        )
        .stderr(predicate::str::starts_with(
            "claude-plus: reconciling with Claude Code counters: adjustments are locked",
        ));
}

fn corrupt_database(env: &Env) -> PathBuf {
    std::fs::create_dir_all(&env.plus).unwrap();
    let database = env.plus.join("data.db");
    std::fs::write(&database, "this is not an SQLite database, just text").unwrap();
    database
}

#[test]
fn import_names_a_corrupt_database() {
    let env = env();
    let database = corrupt_database(&env);
    cmd(&env, &["import"]).assert().failure().stderr(format!(
        "claude-plus: cannot open {}: file is not a database\n",
        database.display()
    ));
}

#[test]
fn doctor_reports_a_corrupt_database() {
    let env = env();
    let database = corrupt_database(&env);
    let doctor = stdout_of(&env, &["doctor"]);
    assert!(
        doctor.ends_with(&format!(
            "  Database:    cannot open {}: file is not a database\n",
            database.display()
        )),
        "{doctor}"
    );
}

#[test]
fn stats_columns_share_one_precision_with_aligned_decimals() {
    let env = env();
    write_unpriced_transcript(&env);
    let session = "88888888-8888-8888-8888-888888888888";
    let dir = env.claude.join("projects/-work-big");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{session}.jsonl")),
        format!(
            r#"{{"type":"assistant","sessionId":"{session}","timestamp":"2026-10-03T11:00:00Z","requestId":"req_big","message":{{"id":"req_big","model":"claude-opus-5-5","content":[{{"type":"text","text":"x"}}],"usage":{{"input_tokens":0,"output_tokens":100000}}}}}}"#
        ) + "\n",
    )
    .unwrap();
    assert_eq!(
        stdout_of(&env, &["stats", "--days", "1"]),
        "\
┌────────────┬───────────────────────────┬──────────┬─────────────────────┐
│ Day        ┆ Model                     ┆ Requests ┆                Cost │
╞════════════╪═══════════════════════════╪══════════╪═════════════════════╡
│ 2026-10-03 ┆ claude-haiku-4-5-20251001 ┆        - ┆ $0.00               │
│ 2026-10-03 ┆ claude-mystery-9          ┆        1 ┆ $0.00 (+1 unpriced) │
│ 2026-10-03 ┆ claude-opus-5-5           ┆        1 ┆ $2.00               │
│ 2026-10-03 ┆ claude-sonnet-5-5         ┆        2 ┆ $0.01               │
├────────────┼───────────────────────────┼──────────┼─────────────────────┤
│ Total      ┆                           ┆        4 ┆ $2.01 (+1 unpriced) │
└────────────┴───────────────────────────┴──────────┴─────────────────────┘
  includes $0.0111 reconciled from Claude Code session counters
"
    );
}
