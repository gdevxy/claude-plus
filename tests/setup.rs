mod common;

use claude_plus::api::client::Client;
use common::{Env, cmd, env};
use predicates::prelude::*;

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Daemon<'a>(&'a Env);

impl Drop for Daemon<'_> {
    fn drop(&mut self) {
        let _ = cmd(self.0, &["daemon", "stop"]).output();
    }
}

fn marketplace(env: &Env) -> std::path::PathBuf {
    env.plus.join("marketplace")
}

fn listing_calls() -> Vec<String> {
    [
        "--version",
        "plugin marketplace list --json",
        "plugin list --json",
    ]
    .map(String::from)
    .to_vec()
}

fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

#[test]
fn install_adds_the_marketplace_installs_the_mod_and_starts_the_daemon() {
    let env = env();
    let _daemon = Daemon(&env);
    let assert = cmd(&env, &["install", "--yes"]).assert().success();
    let out = stdout(&assert);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[..2],
        [
            format!(
                "✻ Added the claude-plus marketplace · {}",
                marketplace(&env).display()
            )
            .as_str(),
            format!("✻ Installed the plus mod v{VERSION}").as_str(),
        ]
    );
    assert!(
        lines[2].starts_with("✻ Started the claude-plus daemon · pid "),
        "{out}"
    );
    assert!(
        lines[3].starts_with("claude-plus is not on PATH, where the pane looks for it"),
        "{out}"
    );
    assert_eq!(
        lines[4],
        "✻ Start a new Claude Code session to see the pane: it docks beside the conversation from 144 columns, and /plus opens it at any width"
    );
    let mut expected = listing_calls();
    expected.push(format!(
        "plugin marketplace add {} --json",
        marketplace(&env).display()
    ));
    expected.push("plugin install plus@claude-plus --json".into());
    assert_eq!(env.claude_calls(), expected);
    assert!(marketplace(&env).join("mod/hooks/register.tsx").is_file());
    assert!(!marketplace(&env).join("mod/tests").exists());
    assert!(Client::new(&env.plus.join("daemon.sock")).health().is_ok());
}

#[test]
fn install_asks_before_claude_code_records_anything_in_its_settings() {
    let env = env();
    let _daemon = Daemon(&env);
    let settings = env.claude.join("settings.json");
    cmd(&env, &["install"])
        .write_stdin("n\n")
        .assert()
        .failure()
        .stdout(predicate::str::contains(format!(
            "claude-plus install will:\n  run claude plugin marketplace add {}\n  run claude plugin install plus@claude-plus\n  which Claude Code records in {}\nContinue? [y/N] ",
            marketplace(&env).display(),
            settings.display()
        ) + "n\n"))
        .stderr("claude-plus: install cancelled, nothing was changed; claude-plus install --yes agrees without asking\n");
    assert_eq!(env.claude_calls(), listing_calls());
    assert!(!env.plus.exists());
    cmd(&env, &["install"])
        .write_stdin("y\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "✻ Installed the plus mod v{VERSION}"
        )));
}

#[test]
fn a_marketplace_of_that_name_elsewhere_is_pointed_here_after_asking() {
    let env = env();
    let _daemon = Daemon(&env);
    std::fs::create_dir_all(env.claude_state()).unwrap();
    std::fs::write(env.claude_state().join("marketplace"), "/elsewhere").unwrap();
    std::fs::write(env.claude_state().join("plugin"), VERSION).unwrap();
    cmd(&env, &["install"])
        .write_stdin("y\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "  run claude plugin marketplace add {0}\n  which Claude Code records in",
            marketplace(&env).display()
        )))
        .stdout(predicate::str::contains(format!(
            "✻ Pointed the claude-plus marketplace at {} (was /elsewhere)\n✻ The plus mod v{VERSION} is installed\n",
            marketplace(&env).display()
        )));
}

#[test]
fn installing_again_asks_nothing_and_changes_nothing() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["install", "--yes"]).assert().success();
    let before = env.claude_calls().len();
    cmd(&env, &["install"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(format!(
            "✻ The plus mod v{VERSION} is installed\n✻ claude-plus daemon is running · pid "
        )));
    assert_eq!(env.claude_calls()[before..], listing_calls());
}

#[test]
fn an_older_mod_is_updated_without_asking() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["install", "--yes"]).assert().success();
    std::fs::write(env.claude_state().join("plugin"), "0.0.9").unwrap();
    std::fs::write(env.claude_state().join("enabled"), "true").unwrap();
    cmd(&env, &["install"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(format!(
            "✻ Updated the plus mod from v0.0.9 to v{VERSION}\n"
        )));
    assert_eq!(
        env.claude_calls().last().unwrap(),
        "plugin update plus@claude-plus --json"
    );
}

#[test]
fn install_needs_claude_code_of_the_mod_api_version() {
    let env = env();
    cmd(&env, &["install", "--yes"])
        .env("PATH", "/usr/bin:/bin")
        .assert()
        .failure()
        .stderr("claude-plus: Claude Code was not found on PATH; install it first, then run claude-plus install again\n");
    cmd(&env, &["install", "--yes"])
        .env("FAKE_CLAUDE_VERSION", "2.1.200")
        .assert()
        .failure()
        .stderr("claude-plus: Claude Code 2.1.200 is older than 2.1.296, which the plus mod needs; run claude update, then claude-plus install again\n");
    assert!(!env.plus.exists());
}

#[test]
fn a_failing_claude_plugin_command_stops_the_install_with_its_reason() {
    let env = env();
    std::fs::create_dir_all(env.claude_state()).unwrap();
    std::fs::write(
        env.claude_state().join("fail"),
        "plugin install plus@claude-plus --json",
    )
    .unwrap();
    cmd(&env, &["install", "--yes"]).assert().failure().stderr(
        "claude-plus: claude plugin install plus@claude-plus failed: the fake was told to fail\n",
    );
    assert!(!env.plus.join("daemon.sock").exists());
}
