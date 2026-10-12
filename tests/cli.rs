mod common;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use common::{
    DEMO_SESSION, DESKTOP_ACCOUNT, DESKTOP_ORG, Env, STATS_TABLE, cmd, env, stdout_of,
    stdout_of_with,
};
use predicates::prelude::*;

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
        .stdout(predicate::str::contains("Anthropic's 2026-10-10"))
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
        .env("HOME", temp.path())
        .env("CLAUDE_CONFIG_DIR", temp.path().join("nope"))
        .env("CLAUDE_PLUS_HOME", temp.path().join("plus"))
        .assert()
        .success()
        .stdout(predicate::str::contains("No Claude Code transcripts found"));
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
  Daemon:      not running, start it with claude-plus daemon --ensure
  Claude Code: 2.1.296
  Mod:         not installed, run claude-plus install
  Account:     none seen yet
  Login:       no .claude.json yet (never signed in)
  Prices:      Anthropic's 2026-10-10
  Desktop:     none found in {desktop}
  Imported:    5 requests in 4 files · 1 unparsable line
  Cross-check: 1 session · transcripts $0.0012 · Claude Code $0.0123 · reconciled $0.0123
  Uncounted:   1 session with subagents has no Claude Code counter, its subagent output may be understated
",
        claude = env.claude.display(),
        plus = env.plus.display(),
        desktop = env.desktop_sessions().display()
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
        .env("HOME", temp.path())
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
  Daemon:      not running, start it with claude-plus daemon --ensure
  Claude Code: 2.1.296
  Mod:         not installed, run claude-plus install
  Account:     none seen yet
  Login:       no .claude.json yet (never signed in)
  Prices:      Anthropic's 2026-10-10
  Desktop:     none found in {desktop}
  Imported:    {imported}
",
        claude = env.claude.display(),
        plus = env.plus.display(),
        desktop = env.desktop_sessions().display()
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
    bytes: Vec<u8>,
}

fn file_state(database: &Path) -> FileState {
    let user_version =
        rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
    FileState {
        user_version,
        bytes: std::fs::read(database).unwrap(),
    }
}

#[test]
fn pre_release_databases_are_refused_untouched() {
    let env = env();
    cmd(&env, &["import"]).assert().success();
    let database = env.plus.join("data.db");
    rusqlite::Connection::open(&database)
        .unwrap()
        .pragma_update(None, "user_version", 5)
        .unwrap();
    let message = format!(
        "database at {} was created by a pre-release claude-plus; delete it and run claude-plus import",
        database.display()
    );
    let before = file_state(&database);
    let doctor = stdout_of(&env, &["doctor"]);
    assert!(
        doctor.ends_with(&format!("  Database:    {message}\n")),
        "{doctor}"
    );
    cmd(&env, &["import"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(format!("claude-plus: {message}\n"));
    let after = file_state(&database);
    assert_eq!(after.user_version, 5);
    assert!(after.bytes == before.bytes, "data.db changed");
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
  Daemon:      not running, start it with claude-plus daemon --ensure
  Claude Code: 2.1.296
  Mod:         not installed, run claude-plus install
  Account:     none seen yet
  Login:       no .claude.json yet (never signed in)
  Prices:      Anthropic's 2026-10-10
  Desktop:     none found in {desktop}
  Database:    not created yet, run claude-plus import
",
        claude = env.claude.display(),
        plus = env.plus.display(),
        desktop = env.desktop_sessions().display()
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
    let transcript = env
        .claude
        .join("projects/-work-demo/22222222-2222-2222-2222-222222222222.jsonl");
    let content = std::fs::read_to_string(&transcript).unwrap();
    let counter = content.lines().last().unwrap().to_string();
    std::fs::write(&transcript, format!("{content}{counter}\n")).unwrap();
    cmd(&env, &["import"])
        .assert()
        .failure()
        .stdout(
            "✻ Imported 1 line from 1 changed file · 0 new requests · 0 unparsable lines\n  History: 5 requests across 2 sessions · $0.0812 priced\n",
        )
        .stderr(predicate::str::starts_with(
            "claude-plus: reconciling with Claude Code counters: adjustments are locked",
        ));
}

fn foreign_database(env: &Env) -> PathBuf {
    std::fs::create_dir_all(&env.plus).unwrap();
    let database = env.plus.join("data.db");
    std::fs::write(&database, "this is not an SQLite database, just text").unwrap();
    database
}

#[test]
fn import_refuses_a_data_db_claude_plus_did_not_make() {
    let env = env();
    let database = foreign_database(&env);
    cmd(&env, &["import"]).assert().failure().stderr(format!(
        "claude-plus: {} is not a claude-plus store, so claude-plus leaves it as it is; move it aside, or set CLAUDE_PLUS_HOME to a directory of its own\n",
        database.display()
    ));
}

#[test]
fn doctor_reports_a_data_db_claude_plus_did_not_make() {
    let env = env();
    let database = foreign_database(&env);
    let doctor = stdout_of(&env, &["doctor"]);
    assert!(
        doctor.ends_with(&format!(
            "  Database:    {} is not a claude-plus store, so claude-plus leaves it as it is; move it aside, or set CLAUDE_PLUS_HOME to a directory of its own\n",
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
Account: All accounts
┌────────────┬───────────────────────────┬──────────┬──────────────────────┐
│ Day        ┆ Model                     ┆ Requests ┆                 Cost │
╞════════════╪═══════════════════════════╪══════════╪══════════════════════╡
│ 2026-10-03 ┆ claude-haiku-4-5-20251001 ┆        - ┆ <$0.01               │
│ 2026-10-03 ┆ claude-mystery-9          ┆        1 ┆  $0.00 (+1 unpriced) │
│ 2026-10-03 ┆ claude-opus-5-5           ┆        1 ┆  $2.00               │
│ 2026-10-03 ┆ claude-sonnet-5-5         ┆        2 ┆  $0.01               │
├────────────┼───────────────────────────┼──────────┼──────────────────────┤
│ Total      ┆                           ┆        4 ┆  $2.01 (+1 unpriced) │
└────────────┴───────────────────────────┴──────────┴──────────────────────┘
  includes $0.0111 reconciled from Claude Code session counters
"
    );
}

#[test]
fn doctor_counts_retired_generations_apart() {
    let env = env();
    let session = "88888888-8888-8888-8888-888888888888";
    let dir = env.claude.join("projects/-work-extra");
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = dir.join(format!("{session}.jsonl"));
    let lines = |requests: &[&str]| -> String {
        requests
            .iter()
            .map(|request| assistant_line(session, request, "2026-10-03T10:00:00Z", false) + "\n")
            .collect()
    };
    std::fs::write(&transcript, lines(&["req_g1", "req_g2"])).unwrap();
    cmd(&env, &["import"]).assert().success();
    std::fs::write(&transcript, lines(&["req_g3"])).unwrap();
    cmd(&env, &["import"]).assert().success();
    assert!(
        stdout_of(&env, &["doctor"])
            .contains("  Imported:    8 requests in 5 files (1 retired) · 1 unparsable line\n")
    );
}

#[test]
fn help_describes_every_command() {
    let env = env();
    let help = stdout_of(&env, &["--help"]);
    for line in [
        "  install    Add the claude-plus pane to Claude Code and start the daemon\n",
        "  uninstall  Remove the claude-plus pane from Claude Code and stop the daemon\n",
        "  import     Read new Claude Code transcripts into the local store\n",
        "  stats      Show daily cost per model, importing first\n",
        "  export     Print stored usage per day, project, account and model, importing first\n",
        "  doctor     Check the setup and compare totals with Claude Code; claude-plus only reads\n",
        "  reindex    Rebuild every derived table from the stored records\n",
        "  daemon     Run the daemon that keeps the store live, in the foreground\n",
    ] {
        assert!(help.contains(line), "missing {line:?} in\n{help}");
    }
}

#[test]
fn an_interrupted_reindex_is_reported_and_finished_by_the_next_import() {
    let env = env();
    cmd(&env, &["import"]).assert().success();
    rusqlite::Connection::open(env.plus.join("data.db"))
        .unwrap()
        .execute_batch(
            "DELETE FROM requests; INSERT INTO meta (key, value) VALUES ('reindex_pending', '1');",
        )
        .unwrap();
    let doctor = stdout_of(&env, &["doctor"]);
    assert!(
        doctor.contains(
            "  Imported:    0 requests in 4 files · 1 unparsable line\n  Reindex:     interrupted, the next import finishes it\n"
        ),
        "{doctor}"
    );
    assert_eq!(
        stdout_of(&env, &["import"]),
        "✻ Imported 0 lines from 0 changed files · 0 new requests · 0 unparsable lines\n  History: 5 requests across 2 sessions · $0.0812 priced\n"
    );
    assert!(!stdout_of(&env, &["doctor"]).contains("Reindex:"));
}

const EXPORT_CSV: &str = "\
day,project,account,model,requests,unpriced,input,output,cache_read,cache_write_5m,cache_write_1h,cost_usd,reconciled_usd
2026-10-01,demo,Unknown,claude-opus-5-5,1,0,10,1000,20000,0,5000,0.064040,0.000000
2026-10-02,demo,Unknown,claude-haiku-4-5-20251001,1,0,100,62,0,0,0,0.000410,0.000000
2026-10-02,demo,Unknown,claude-sonnet-5-5,1,0,5,200,0,1000,0,0.004510,0.000000
2026-10-03,demo,Unknown,claude-haiku-4-5-20251001,0,0,1000,20,0,0,0,0.001100,0.001100
2026-10-03,demo,Unknown,claude-sonnet-5-5,2,0,10,1105,1000,0,0,0.011170,0.010000
";

#[test]
fn export_csv_is_exact_and_matches_stats() {
    let env = env();
    let export = stdout_of(&env, &["export", "--format", "csv"]);
    assert_eq!(export, EXPORT_CSV);
    let mut exported: std::collections::BTreeMap<(String, String), (u64, f64)> =
        std::collections::BTreeMap::new();
    for line in export.lines().skip(1) {
        let fields: Vec<&str> = line.split(',').collect();
        let entry = exported
            .entry((fields[0].to_string(), fields[3].to_string()))
            .or_default();
        entry.0 += fields[4].parse::<u64>().unwrap();
        entry.1 += fields[11].parse::<f64>().unwrap();
    }
    let exported: Vec<(String, String, String, String)> = exported
        .into_iter()
        .map(|((day, model), (requests, usd))| {
            let requests = if requests == 0 {
                "-".to_string()
            } else {
                requests.to_string()
            };
            (day, model, requests, format!("${usd:.4}"))
        })
        .collect();
    let stats = stdout_of(&env, &["stats"]);
    let shown: Vec<(String, String, String, String)> = stats
        .lines()
        .filter(|line| line.starts_with("│ 2026-"))
        .map(|line| {
            let cells: Vec<String> = line
                .trim_matches('│')
                .split('┆')
                .map(|cell| cell.trim().to_string())
                .collect();
            (
                cells[0].clone(),
                cells[1].clone(),
                cells[2].clone(),
                cells[3].clone(),
            )
        })
        .collect();
    assert_eq!(exported, shown, "{stats}");
}

#[test]
fn export_json_carries_the_same_rows() {
    let env = env();
    let json: serde_json::Value =
        serde_json::from_str(&stdout_of(&env, &["export", "--format", "json"])).unwrap();
    assert_eq!(json["v"], 2);
    let rows = json["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 5);
    let total: f64 = rows
        .iter()
        .map(|row| row["cost_usd"].as_f64().unwrap())
        .sum();
    assert!((total - 0.08123).abs() < 1e-9, "{total}");
    assert_eq!(rows[3]["requests"], 0);
    assert_eq!(rows[3]["reconciled_usd"], 0.0011);
}

#[test]
fn export_days_keeps_the_last_calendar_days() {
    let env = env();
    let today: String = EXPORT_CSV
        .lines()
        .filter(|line| !line.starts_with("2026-10-0") || line.starts_with("2026-10-03"))
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(
        stdout_of(&env, &["export", "--format", "csv", "--days", "1"]),
        today
    );
    cmd(&env, &["export", "--format", "csv", "--days", "0"])
        .assert()
        .failure();
}

#[test]
fn export_without_transcripts_prints_an_empty_export_and_the_notice_on_stderr() {
    let temp = tempfile::tempdir().unwrap();
    for (format, empty) in [
        (
            "csv",
            format!("{}\n", claude_plus::history::export::CSV_HEADER),
        ),
        ("json", "{\"v\":2,\"rows\":[]}\n".to_string()),
    ] {
        Command::cargo_bin("claude-plus")
            .unwrap()
            .args(["export", "--format", format])
            .env("HOME", temp.path())
            .env("CLAUDE_CONFIG_DIR", temp.path().join("nope"))
            .env("CLAUDE_PLUS_HOME", temp.path().join("plus"))
            .assert()
            .success()
            .stdout(empty)
            .stderr(predicate::str::contains("No Claude Code transcripts found"));
    }
}

#[test]
fn export_help_names_what_each_format_prints() {
    let env = env();
    let help = stdout_of(&env, &["export", "--help"]);
    assert!(
        help.contains("- json: One object {\"v\":2,\"rows\":[...]}, costs to 6 decimals\n"),
        "{help}"
    );
    assert!(
        help.contains(
            "- csv:  A header row, RFC 4180 quoting, LF line ends and costs to 6 decimals\n"
        ),
        "{help}"
    );
}

#[test]
fn export_rejects_an_unknown_format() {
    let env = env();
    cmd(&env, &["export", "--format", "xml"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("[possible values: json, csv]"));
    cmd(&env, &["export"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--format <FORMAT>"));
}

#[test]
fn a_reader_that_goes_away_ends_the_output_without_a_panic() {
    use std::os::unix::process::ExitStatusExt;
    let env = env();
    let ended_by_a_closed_pipe = |args: &[&str]| {
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let output = common::process(&env, args)
            .stdout(writer)
            .stderr(std::process::Stdio::piped())
            .output()
            .unwrap();
        assert_eq!(output.status.signal(), Some(libc::SIGPIPE), "{args:?}");
        assert_eq!(String::from_utf8(output.stderr).unwrap(), "", "{args:?}");
    };
    struct StopOnDrop<'a>(&'a Env);
    impl Drop for StopOnDrop<'_> {
        fn drop(&mut self) {
            let _ = cmd(self.0, &["daemon", "stop"]).output();
        }
    }
    let _daemon = StopOnDrop(&env);
    ended_by_a_closed_pipe(&["doctor"]);
    ended_by_a_closed_pipe(&["daemon", "--ensure"]);
    let health = claude_plus::api::client::Client::new(&env.plus.join("daemon.sock")).health();
    ended_by_a_closed_pipe(&["daemon", "stop"]);
    assert!(health.is_ok(), "daemon --ensure started no daemon");
}

#[test]
fn a_local_import_records_the_claude_code_login_without_touching_it() {
    let env = env();
    let login = env.claude.join(".claude.json");
    let body = r#"{"oauthAccount":{"accountUuid":"u-1","organizationName":"Acme","organizationType":"claude_max"}}"#;
    std::fs::write(&login, body).unwrap();
    cmd(&env, &["import"]).assert().success();
    let connection = rusqlite::Connection::open(env.plus.join("data.db")).unwrap();
    let (id, label): (String, String) = connection
        .query_row("SELECT id, label FROM accounts", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((id.as_str(), label.as_str()), ("oauth:u-1", "Acme · Max"));
    let open: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM account_periods WHERE to_ms IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(open, 1);
    assert_eq!(std::fs::read_to_string(&login).unwrap(), body);
}

const ME: &str = r#"{"oauthAccount":{"accountUuid":"me","organizationName":"Me","organizationType":"claude_max"}}"#;
const WORK: &str = r#"{"oauthAccount":{"accountUuid":"work","organizationName":"Acme","organizationType":"claude_enterprise"}}"#;
const MID_MONTH: &str = "2026-10-21T12:00:00Z";

fn at(env: &Env, args: &[&str], now: &str) -> String {
    stdout_of_with(env, args, &[("CLAUDE_PLUS_NOW", now)])
}

fn signed_in_with_spend(env: &Env) {
    env.write_claude_json(WORK);
    at(env, &["import"], "2026-10-21T10:00:00Z");
    env.write_transcript_at("s1", "claude-opus-5-5", "2026-10-21T11:00:00Z", 1_155_000);
}

fn two_logins(env: &Env) {
    env.write_claude_json(ME);
    at(env, &["import"], "2026-10-21T08:00:00Z");
    env.write_transcript_at("mine", "claude-sonnet-5-5", "2026-10-21T09:00:00Z", 45_000);
    env.write_claude_json(WORK);
    at(env, &["import"], "2026-10-21T10:00:00Z");
    env.write_transcript_at("work", "claude-opus-5-5", "2026-10-21T11:00:00Z", 1_155_000);
}

#[test]
fn a_budget_is_set_shown_and_cleared_for_the_signed_in_account() {
    let env = env();
    signed_in_with_spend(&env);
    assert_eq!(
        at(&env, &["budget", "set", "700"], MID_MONTH),
        "Budget for Acme · Enterprise set to $700.00 a month, reset on the 1st at 00:00 UTC\n"
    );
    assert_eq!(
        at(&env, &["budget", "show"], MID_MONTH),
        "  Account:     Acme · Enterprise (oauth:work)
  Budget:      $700.00 a month, resets in 10d
  Local:       $23.10 spent this month on this machine
  Pace:        on pace for 5.0% by the reset
"
    );
    assert_eq!(
        at(&env, &["budget", "clear"], MID_MONTH),
        "Budget for Acme · Enterprise cleared\n"
    );
    assert_eq!(
        at(&env, &["budget", "clear"], MID_MONTH),
        "Acme · Enterprise has no budget\n"
    );
}

#[test]
fn budget_show_derives_dollars_from_the_reported_percent() {
    let env = env();
    signed_in_with_spend(&env);
    at(&env, &["budget", "set", "700"], MID_MONTH);
    env.seed_spend_limit("oauth:work", 4.0, "2026-10-21T11:30:00Z");
    assert_eq!(
        at(&env, &["budget", "show"], MID_MONTH),
        "  Account:     Acme · Enterprise (oauth:work)
  Budget:      $700.00 a month, resets in 10d
  Reported:    $28.00 used, 4.0% of the spend limit
  Local:       $23.10 spent this month on this machine
  Pace:        on pace for 6.0% by the reset
"
    );
}

#[test]
fn budget_show_says_when_the_pace_goes_over_the_budget() {
    let env = env();
    signed_in_with_spend(&env);
    at(&env, &["budget", "set", "30"], MID_MONTH);
    assert_eq!(
        at(&env, &["budget", "show"], MID_MONTH),
        "  Account:     Acme · Enterprise (oauth:work)
  Budget:      $30.00 a month, resets in 10d
  Local:       $23.10 spent this month on this machine
  Pace:        on pace for 116% by the reset, over the budget
"
    );
}

#[test]
fn budget_show_without_a_budget_still_shows_the_spend() {
    let env = env();
    signed_in_with_spend(&env);
    assert_eq!(
        at(&env, &["budget", "show"], MID_MONTH),
        "  Account:     Acme · Enterprise (oauth:work)
  Budget:      none
  Local:       $23.10 spent this month on this machine
               claude-plus budget set <usd> shows dollars
"
    );
    env.seed_spend_limit("oauth:work", 4.0, "2026-10-21T11:30:00Z");
    assert_eq!(
        at(&env, &["budget", "show"], MID_MONTH),
        "  Account:     Acme · Enterprise (oauth:work)
  Budget:      none
  Reported:    4.0% of the spend limit used
  Local:       $23.10 spent this month on this machine
               claude-plus budget set <usd> shows dollars
"
    );
}

#[test]
fn budget_set_and_clear_follow_a_login_switch() {
    let env = env();
    env.write_claude_json(ME);
    at(&env, &["import"], "2026-10-21T08:00:00Z");
    env.write_claude_json(WORK);
    assert_eq!(
        at(&env, &["budget", "set", "10"], MID_MONTH),
        "Budget for Acme · Enterprise set to $10.00 a month, reset on the 1st at 00:00 UTC\n"
    );
    env.write_claude_json(ME);
    assert_eq!(
        at(&env, &["budget", "clear"], "2026-10-21T13:00:00Z"),
        "Me · Max has no budget\n"
    );
}

#[test]
fn a_label_matches_regardless_of_case_beyond_ascii() {
    let env = env();
    env.seed_accounts(&[("oauth:a", "équipe")]);
    cmd(&env, &["budget", "set", "10", "--account", "ÉQUIPE"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Budget for équipe set"));
}

#[test]
fn accounts_output_is_exact() {
    let env = env();
    two_logins(&env);
    at(&env, &["budget", "set", "700"], MID_MONTH);
    assert_eq!(
        at(&env, &["accounts"], MID_MONTH),
        "\
┌───────────────────┬────────────┬───────┬──────────┬────────────┬─────────┐
│ Account           ┆ Id         ┆ Kind  ┆ Sessions ┆ This month ┆  Budget │
╞═══════════════════╪════════════╪═══════╪══════════╪════════════╪═════════╡
│ Me · Max          ┆ oauth:me   ┆ oauth ┆        1 ┆      $0.45 ┆       - │
│ Acme · Enterprise ┆ oauth:work ┆ oauth ┆        1 ┆     $23.10 ┆ $700.00 │
└───────────────────┴────────────┴───────┴──────────┴────────────┴─────────┘
"
    );
}

#[test]
fn a_budget_must_be_a_positive_amount() {
    let env = env();
    for amount in ["0", "-5", "nan", "inf", "abc"] {
        cmd(&env, &["budget", "set", amount, "--account", "api"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "a budget is a number of dollars above zero",
            ));
    }
}

#[test]
fn an_ambiguous_label_lists_the_matches() {
    let env = env();
    env.seed_accounts(&[("oauth:a", "Work"), ("oauth:b", "work")]);
    cmd(&env, &["budget", "set", "10", "--account", "WORK"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "\"WORK\" matches oauth:a (Work), oauth:b (work)",
        ));
}

#[test]
fn accounts_lists_and_renames() {
    let env = env();
    env.seed_accounts(&[("oauth:a", "Acme · Enterprise")]);
    cmd(&env, &["accounts", "rename", "oauth:a", "Work"])
        .assert()
        .success();
    cmd(&env, &["accounts"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Work"))
        .stdout(predicate::str::contains("oauth:a"));
}

#[test]
fn a_budget_can_be_set_for_an_account_not_seen_yet() {
    let env = env();
    cmd(&env, &["budget", "set", "50", "--account", "api"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Budget for API key set to $50.00 a month",
        ));
    cmd(&env, &["budget", "set", "50", "--account", "nobody"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no account called \"nobody\""));
}

#[test]
fn without_any_account_the_budget_asks_for_one() {
    let env = env();
    cmd(&env, &["budget", "set", "50"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no account known yet"));
}

#[test]
fn stats_count_only_the_current_account_unless_all_is_asked() {
    let env = env();
    two_logins(&env);
    let current = at(&env, &["stats", "--days", "1"], MID_MONTH);
    assert!(
        current.starts_with("Account: Acme · Enterprise\n"),
        "{current}"
    );
    assert!(current.contains("claude-opus-5-5"), "{current}");
    assert!(!current.contains("claude-sonnet-5-5"), "{current}");
    assert!(!current.contains("$0.45"), "{current}");
    let all = at(
        &env,
        &["stats", "--days", "1", "--account", "all"],
        MID_MONTH,
    );
    assert!(all.starts_with("Account: All accounts\n"), "{all}");
    assert!(
        all.contains("claude-opus-5-5") && all.contains("claude-sonnet-5-5"),
        "{all}"
    );
    let mine = at(
        &env,
        &["stats", "--days", "1", "--account", "oauth:me"],
        MID_MONTH,
    );
    assert!(mine.starts_with("Account: Me · Max\n"), "{mine}");
    assert!(mine.contains("claude-sonnet-5-5"), "{mine}");
    assert!(!mine.contains("claude-opus-5-5"), "{mine}");
}

#[test]
fn doctor_reports_the_account_login_and_prices() {
    let env = env();
    signed_in_with_spend(&env);
    at(&env, &["import"], MID_MONTH);
    let out = stdout_of(&env, &["doctor"]);
    assert!(
        out.contains("  Account:     Acme · Enterprise (oauth:work)\n  Login:       signed in\n  Prices:      Anthropic's 2026-10-10\n"),
        "{out}"
    );
    std::fs::write(
        env.plus.join("prices.toml"),
        "[models.claude-opus-5-5]\ninput = 1\noutput = 1\ncache_read = 0.1\n",
    )
    .unwrap();
    let out = stdout_of(&env, &["doctor"]);
    assert!(
        out.contains("  Prices:      Anthropic's 2026-10-10, with your prices.toml\n"),
        "{out}"
    );
    env.write_claude_json("{broken");
    let out = stdout_of(&env, &["doctor"]);
    assert!(out.contains("  Login:       cannot read "), "{out}");
}

#[test]
fn export_keeps_only_the_asked_account() {
    let env = env();
    two_logins(&env);
    let export = |extra: &[&str]| {
        let mut args = vec!["export", "--format", "csv"];
        args.extend_from_slice(extra);
        at(&env, &args, MID_MONTH)
    };
    let current = export(&[]);
    assert!(
        current.contains("Acme") && !current.contains("Me · Max"),
        "{current}"
    );
    let all = export(&["--account", "all"]);
    assert!(all.contains("Acme") && all.contains("Me · Max"), "{all}");
}

#[test]
fn the_current_account_follows_a_login_switch_before_its_first_session() {
    let env = env();
    env.write_claude_json(ME);
    at(&env, &["import"], "2026-10-21T08:00:00Z");
    env.write_transcript_at("mine", "claude-sonnet-5-5", "2026-10-21T09:00:00Z", 45_000);
    at(&env, &["import"], "2026-10-21T09:30:00Z");
    env.write_claude_json(WORK);
    let shown = at(&env, &["budget", "show"], MID_MONTH);
    assert!(
        shown.starts_with("  Account:     Acme · Enterprise (oauth:work)\n"),
        "{shown}"
    );
}

#[test]
fn an_unreadable_login_is_warned_about_once_and_a_missing_one_not_at_all() {
    let env = env();
    cmd(&env, &["import"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    env.write_claude_json("{broken");
    let output = cmd(&env, &["import"])
        .assert()
        .success()
        .get_output()
        .clone();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(
        stderr.starts_with("Cannot read the Claude Code login: ")
            && stderr.contains(".claude.json is not valid JSON"),
        "{stderr}"
    );
}

#[test]
fn stats_and_export_say_on_stderr_when_other_accounts_are_left_out() {
    let env = env();
    two_logins(&env);
    let note = "Showing Acme · Enterprise; --account all for every account\n";
    let stderr_of = |args: &[&str]| {
        let output = cmd(&env, args)
            .env("CLAUDE_PLUS_NOW", MID_MONTH)
            .assert()
            .success()
            .get_output()
            .clone();
        (
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    };
    let (stats, stats_err) = stderr_of(&["stats", "--days", "1"]);
    assert_eq!(stats_err, note);
    assert!(!stats.contains("--account all"), "{stats}");
    let (export, export_err) = stderr_of(&["export", "--format", "csv", "--days", "1"]);
    assert_eq!(export_err, note);
    assert!(!export.contains("--account all"), "{export}");
    for args in [
        ["stats", "--days", "1", "--account", "all"].as_slice(),
        ["stats", "--days", "1", "--account", "oauth:work"].as_slice(),
        [
            "export",
            "--format",
            "csv",
            "--days",
            "1",
            "--account",
            "all",
        ]
        .as_slice(),
    ] {
        assert_eq!(stderr_of(args).1, "", "{args:?}");
    }
}

#[test]
fn stats_say_nothing_extra_when_only_the_current_account_spent() {
    let env = env();
    signed_in_with_spend(&env);
    let output = cmd(&env, &["stats", "--days", "1"])
        .env("CLAUDE_PLUS_NOW", MID_MONTH)
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "");
}

const CLI_ACCOUNT: &str = "c478b5d6-2b7e-4f0a-8c3d-0123456789ab";
const DESKTOP_SESSION: &str = "33333333-3333-3333-3333-333333333333";

fn desktop_file(cli_session: &str) -> String {
    format!(
        r#"{{"sessionId":"local_9f1c","cliSessionId":"{cli_session}","priorCliSessionIds":[],"cwd":"/work/demo","createdAt":1791021000000,"title":"kept private"}}"#
    )
}

#[test]
fn import_moves_a_desktop_session_off_the_cli_login_to_the_desktop_account() {
    let env = env();
    env.write_claude_json(&format!(
        r#"{{"oauthAccount":{{"accountUuid":"{CLI_ACCOUNT}","organizationName":"Me","organizationType":"claude_max"}}}}"#
    ));
    env.write_transcript_at(
        DESKTOP_SESSION,
        "claude-opus-5-5",
        "2026-10-03T10:00:00Z",
        1_000,
    );
    cmd(&env, &["import"]).assert().success();
    env.seed_start(DESKTOP_SESSION, &format!("oauth:{CLI_ACCOUNT}"));
    assert_eq!(
        env.attribution(DESKTOP_SESSION),
        Some((format!("oauth:{CLI_ACCOUNT}"), "start".into()))
    );
    let file = env.write_desktop_session(
        DESKTOP_ACCOUNT,
        DESKTOP_ORG,
        "local_9f1c.json",
        &desktop_file(DESKTOP_SESSION),
    );
    let before = std::fs::read(&file).unwrap();
    cmd(&env, &["import"]).assert().success().stderr("");
    assert_eq!(
        env.attribution(DESKTOP_SESSION),
        Some((format!("oauth:{DESKTOP_ACCOUNT}"), "desktop".into()))
    );
    assert_eq!(std::fs::read(&file).unwrap(), before);
    let accounts = stdout_of(&env, &["accounts"]);
    assert!(accounts.contains("Desktop account e08c8984"), "{accounts}");
    let stats = stdout_of(
        &env,
        &[
            "stats",
            "--days",
            "1",
            "--account",
            "desktop account E08C8984",
        ],
    );
    assert!(
        stats.starts_with("Account: Desktop account e08c8984\n"),
        "{stats}"
    );
    assert!(stats.contains("claude-opus-5-5"), "{stats}");
}

#[test]
fn a_skipped_desktop_file_is_named_on_stderr_and_counted_by_doctor() {
    let env = env();
    env.write_desktop_session(
        DESKTOP_ACCOUNT,
        DESKTOP_ORG,
        "local_a.json",
        &desktop_file(DEMO_SESSION),
    );
    let broken =
        env.write_desktop_session(DESKTOP_ACCOUNT, DESKTOP_ORG, "local_b.json", "{\"cliSess");
    cmd(&env, &["import"]).assert().success().stderr(format!(
        "Skipped the desktop session file {}, it is not valid JSON\n",
        broken.display()
    ));
    assert_eq!(
        env.attribution(DEMO_SESSION),
        Some((format!("oauth:{DESKTOP_ACCOUNT}"), "desktop".into()))
    );
    let doctor = stdout_of(&env, &["doctor"]);
    assert!(
        doctor.contains(&format!(
            "  Prices:      Anthropic's 2026-10-10\n  Desktop:     {} · 2 session files · 1 skipped\n",
            env.desktop_sessions().display()
        )),
        "{doctor}"
    );
}
