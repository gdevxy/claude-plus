#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use assert_cmd::Command;

pub struct Env {
    pub _temp: tempfile::TempDir,
    pub claude: PathBuf,
    pub plus: PathBuf,
}

pub fn copy_dir(from: &Path, to: &Path) {
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

pub fn env() -> Env {
    let temp = tempfile::Builder::new()
        .prefix("cp-")
        .tempdir_in("/tmp")
        .unwrap();
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

pub const VARS: &[(&str, &str)] = &[
    ("TZ", "UTC"),
    ("CLAUDE_PLUS_NOW", "2026-10-03T12:00:00Z"),
    ("NO_COLOR", "1"),
];

pub fn cmd(env: &Env, args: &[&str]) -> Command {
    let mut command = Command::cargo_bin("claude-plus").unwrap();
    command
        .args(args)
        .env("CLAUDE_CONFIG_DIR", &env.claude)
        .env("CLAUDE_PLUS_HOME", &env.plus)
        .envs(VARS.iter().copied());
    command
}

pub fn process(env: &Env, args: &[&str]) -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_claude-plus"));
    command
        .args(args)
        .env("CLAUDE_CONFIG_DIR", &env.claude)
        .env("CLAUDE_PLUS_HOME", &env.plus)
        .envs(VARS.iter().copied());
    command
}

pub fn stdout_of(env: &Env, args: &[&str]) -> String {
    let output = cmd(env, args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).unwrap()
}

pub fn stdout_of_with(env: &Env, args: &[&str], vars: &[(&str, &str)]) -> String {
    let mut command = cmd(env, args);
    for (key, value) in vars {
        command.env(key, value);
    }
    String::from_utf8(command.assert().success().get_output().stdout.clone()).unwrap()
}

pub fn eventually<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(value) = probe() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub const STATS_TABLE: &str = "\
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
