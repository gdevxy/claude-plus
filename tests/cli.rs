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
        .stdout(predicate::str::contains("$0.0811"))
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
        .stdout(predicate::str::contains("$0.0811"));
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
        .stdout(predicate::str::contains("$0.0811"))
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
│ 2026-10-02 ┆ claude-haiku-4-5-20251001 ┆        1 ┆ $0.0003 │
│ 2026-10-02 ┆ claude-sonnet-5-5         ┆        1 ┆ $0.0045 │
│ 2026-10-03 ┆ claude-haiku-4-5-20251001 ┆        0 ┆ $0.0011 │
│ 2026-10-03 ┆ claude-sonnet-5-5         ┆        2 ┆ $0.0112 │
│ Total      ┆                           ┆        5 ┆ $0.0811 │
└────────────┴───────────────────────────┴──────────┴─────────┘
  includes $0.0111 reconciled from Claude Code session counters
";

#[test]
fn import_output_is_exact() {
    let env = env();
    assert_eq!(
        stdout_of(&env, &["import"]),
        "✻ Imported 16 lines from 4 changed files · 5 new requests · 1 unparsable line\n  History: 5 requests across 2 sessions · $0.0811 priced\n"
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
  Cross-check: 2 sessions · transcripts $0.0701 · Claude Code $0.0811 · reconciled $0.0811
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
        .stdout(predicate::str::contains("(default)").not());
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
            "  Cross-check: 3 sessions · transcripts $0.0801 · Claude Code $0.0854 · reconciled $0.0912
  Uncounted:   1 session with subagents has no Claude Code counter, its subagent output may be understated
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
            "│ Total      ┆                           ┆        2 ┆ $0.0123 │
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
