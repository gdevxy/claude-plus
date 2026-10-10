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
        .stdout(predicate::str::contains("3 new requests"))
        .stdout(predicate::str::contains("1 unparsable line"))
        .stdout(predicate::str::contains("$0.0689"))
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
        .stdout(predicate::str::contains("$0.0689"));
}

#[test]
fn doctor_checks_setup() {
    let env = env();
    cmd(&env, &["import"]).assert().success();
    cmd(&env, &["doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 transcript files"))
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
        .stdout(predicate::str::contains("3 requests"));
    cmd(&env, &["stats", "--days", "36500"])
        .assert()
        .success()
        .stdout(predicate::str::contains("$0.0689"));
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
        dir.join("22222222-2222-2222-2222-222222222222.jsonl"),
        concat!(
            r#"{"type":"assistant","sessionId":"22222222-2222-2222-2222-222222222222","timestamp":"2026-10-03T09:00:00Z","requestId":"req_mystery","message":{"id":"msg_mystery","model":"claude-mystery-9","content":[{"type":"text","text":"x"}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            "\n"
        ),
    )
    .unwrap();
}

const STATS_TABLE: &str = "\
┌────────────┬───────────────────────────┬──────────┬─────────┐
│ Day        ┆ Model                     ┆ Requests ┆    Cost │
╞════════════╪═══════════════════════════╪══════════╪═════════╡
│ 2026-10-01 ┆ claude-opus-5-5           ┆        1 ┆ $0.0640 │
│ 2026-10-02 ┆ claude-haiku-4-5-20251001 ┆        1 ┆ $0.0003 │
│ 2026-10-02 ┆ claude-sonnet-5-5         ┆        1 ┆ $0.0045 │
│ Total      ┆                           ┆        3 ┆ $0.0689 │
└────────────┴───────────────────────────┴──────────┴─────────┘
";

#[test]
fn import_output_is_exact() {
    let env = env();
    assert_eq!(
        stdout_of(&env, &["import"]),
        "✻ Imported 12 lines from 2 changed files · 3 new requests · 1 unparsable line\n  History: 3 requests across 1 session · $0.0689 priced\n"
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
  Claude dir:  {claude} · 2 transcript files
  Data dir:    {plus}
  Retention:   cleanupPeriodDays: 30 (default)
               Claude Code deletes older transcripts, claude-plus keeps its own copy
  Price table: bundled prices 2026-10-10
  Imported:    3 requests in 2 files · 1 unparsable line
  Cross-check: 1 session · ours $0.0689 · Claude Code $0.0689
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
