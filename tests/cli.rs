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
        .stdout(predicate::str::contains("Claude Code cost cross-check"));
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
