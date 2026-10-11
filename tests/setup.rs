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

#[test]
fn uninstall_removes_the_mod_and_its_marketplace_and_stops_the_daemon() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["install", "--yes"]).assert().success();
    let before = env.claude_calls().len();
    cmd(&env, &["uninstall", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::is_match(
            "^✻ Uninstalled the plus mod\n✻ Removed the claude-plus marketplace\n✻ Stopped the claude-plus daemon · pid [0-9]+\n✻ claude-plus is uninstalled · close open Claude Code sessions, or their pane starts the daemon again\n$",
        ).unwrap());
    assert_eq!(
        env.claude_calls()[before..],
        [
            "--version",
            "plugin marketplace list --json",
            "plugin list --json",
            "plugin uninstall plus@claude-plus --json",
            "plugin marketplace remove claude-plus --json",
        ]
    );
    assert!(!marketplace(&env).exists());
    assert!(env.plus.join("data.db").exists());
}

#[test]
fn uninstall_asks_first_and_purge_deletes_the_data_dir() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["install", "--yes"]).assert().success();
    cmd(&env, &["uninstall", "--purge"])
        .write_stdin("")
        .assert()
        .failure()
        .stdout(predicate::str::contains(format!(
            "  delete {} with the history claude-plus keeps\nContinue? [y/N] ",
            env.plus.display()
        )));
    assert!(env.plus.join("data.db").exists());
    cmd(&env, &["uninstall", "--purge"])
        .write_stdin("y\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "✻ Deleted {}\n",
            env.plus.display()
        )));
    assert!(!env.plus.exists());
}

#[test]
fn purge_refuses_a_data_dir_that_holds_the_claude_dir_before_changing_anything() {
    let env = env();
    let parent = env.claude.parent().unwrap();
    cmd(&env, &["uninstall", "--purge", "--yes"])
        .env("CLAUDE_PLUS_HOME", parent)
        .assert()
        .failure()
        .stderr(format!(
            "claude-plus: refusing to purge {}: it holds {}, which Claude Code keeps\n",
            parent.display(),
            env.claude.display()
        ));
    assert_eq!(env.claude_calls(), Vec::<String>::new());
    assert!(env.claude.join("projects").exists());
}

#[test]
fn uninstall_without_claude_code_still_stops_the_daemon() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["daemon", "--ensure"]).assert().success();
    cmd(&env, &["uninstall"])
        .env("PATH", "/usr/bin:/bin")
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "Claude Code was not found on PATH, so the plus mod may still be installed: run claude plugin uninstall plus@claude-plus where it is\n✻ Stopped the claude-plus daemon · pid ",
        ));
}

#[test]
fn uninstalling_twice_finds_nothing_left() {
    let env = env();
    cmd(&env, &["uninstall"])
        .assert()
        .success()
        .stdout("No claude-plus daemon is running\n✻ claude-plus is uninstalled · close open Claude Code sessions, or their pane starts the daemon again\n");
}

#[test]
fn daemon_ensure_brings_an_older_installed_mod_up_to_this_binary() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["install", "--yes"]).assert().success();
    let manifest = marketplace(&env).join("mod/.claude-plugin/plugin.json");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace(
            &format!("\"version\": \"{VERSION}\""),
            "\"version\": \"0.0.1\"",
        ),
    )
    .unwrap();
    let before = env.claude_calls();
    cmd(&env, &["daemon", "--ensure"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(format!(
            "✻ Updated the plus mod from v0.0.1 to v{VERSION} · new Claude Code sessions load it\n✻ claude-plus daemon is running · pid "
        )));
    assert!(
        std::fs::read_to_string(&manifest)
            .unwrap()
            .contains(&format!("\"version\": \"{VERSION}\""))
    );
    assert_eq!(env.claude_calls(), before);
    cmd(&env, &["daemon", "--ensure"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "✻ claude-plus daemon is running · pid ",
        ));
}

#[test]
fn daemon_ensure_without_an_installed_mod_writes_no_folder() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["daemon", "--ensure"]).assert().success();
    assert!(!marketplace(&env).exists());
    assert!(!env.plus.join("marketplace.lock").exists());
}

#[test]
fn doctor_names_the_installed_mod_and_the_folder_it_loads_from() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["install", "--yes"]).assert().success();
    cmd(&env, &["doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "  Claude Code: 2.1.296\n  Mod:         plus v{VERSION} · {}/mod\n",
            marketplace(&env).display()
        )));
}

#[test]
fn purge_refuses_a_relative_data_dir_and_deletes_nothing() {
    let env = env();
    let project = env.claude.parent().unwrap().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("precious"), "keep").unwrap();
    cmd(&env, &["uninstall", "--purge", "--yes"])
        .current_dir(&project)
        .env("HOME", env.claude.parent().unwrap())
        .env("CLAUDE_PLUS_HOME", ".")
        .assert()
        .failure()
        .stderr("claude-plus: CLAUDE_PLUS_HOME must be an absolute path, not .\n");
    assert!(project.join("precious").is_file());
    assert_eq!(env.claude_calls(), Vec::<String>::new());
}

#[test]
fn purge_refuses_a_directory_claude_plus_did_not_make() {
    let env = env();
    let root = env.claude.parent().unwrap();
    let foreign = root.join("documents");
    std::fs::create_dir_all(&foreign).unwrap();
    std::fs::write(foreign.join("data.db"), "not ours").unwrap();
    let decoy = env.claude.join("decoy");
    std::fs::create_dir_all(&decoy).unwrap();
    std::fs::write(decoy.join("precious"), "keep").unwrap();
    cmd(&env, &["uninstall", "--purge", "--yes"])
        .env("CLAUDE_PLUS_HOME", &foreign)
        .assert()
        .failure()
        .stderr(format!(
            "claude-plus: refusing to purge {}: claude-plus did not create it, as it has no .claude-plus-data file\n",
            foreign.display()
        ));
    cmd(&env, &["uninstall", "--purge", "--yes"])
        .env("CLAUDE_PLUS_HOME", &decoy)
        .assert()
        .failure()
        .stderr(format!(
            "claude-plus: refusing to purge {}: it is inside {}, which Claude Code keeps\n",
            decoy.display(),
            env.claude.display()
        ));
    assert!(foreign.join("data.db").is_file());
    assert!(decoy.join("precious").is_file());
    assert_eq!(env.claude_calls(), Vec::<String>::new());
}

#[test]
fn purge_refuses_a_symlinked_data_dir() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["install", "--yes"]).assert().success();
    let link = env.claude.parent().unwrap().join("link");
    std::os::unix::fs::symlink(&env.plus, &link).unwrap();
    let before = env.claude_calls().len();
    cmd(&env, &["uninstall", "--purge", "--yes"])
        .env("CLAUDE_PLUS_HOME", &link)
        .assert()
        .failure()
        .stderr(format!(
            "claude-plus: refusing to purge {}: it is a symlink to {}; set CLAUDE_PLUS_HOME to that directory to purge it\n",
            link.display(),
            env.plus.display()
        ));
    assert_eq!(env.claude_calls().len(), before);
    assert!(env.plus.join("data.db").exists());
}

#[test]
fn purge_keeps_files_claude_plus_did_not_write() {
    let env = env();
    let _daemon = Daemon(&env);
    cmd(&env, &["install", "--yes"]).assert().success();
    std::fs::write(env.plus.join("notes.txt"), "mine").unwrap();
    cmd(&env, &["uninstall", "--purge", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "✻ Deleted what claude-plus keeps in {} and left notes.txt, which it did not write\n",
            env.plus.display()
        )));
    let mut left: Vec<_> = std::fs::read_dir(&env.plus)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left, [".claude-plus-data", "notes.txt"]);
}
