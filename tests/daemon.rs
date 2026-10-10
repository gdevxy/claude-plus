mod common;

use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Stdio};

use claude_plus::api::client::Client;
use claude_plus::api::wire::{Health, Snapshot};
use claude_plus::daemon::lock::InstanceLock;
use common::{Env, STATS_TABLE, cmd, env, eventually, process, stdout_of};
use rustix::process::{Pid, Signal, kill_process};
use serde_json::json;

const SESSION_1: &str = "11111111-1111-1111-1111-111111111111";

fn client(env: &Env) -> Client {
    Client::new(&env.plus.join("daemon.sock"))
}

fn health(env: &Env) -> Option<Health> {
    client(env).health().ok()
}

fn idle_health(env: &Env) -> Health {
    eventually("the daemon to finish ingesting", || {
        health(env).filter(|h| h.ingest.last_ingest_ms.is_some() && h.ingest.backlog == 0)
    })
}

struct Foreground {
    child: Child,
}

impl Foreground {
    fn start(env: &Env, extra: &[&str]) -> Self {
        let child = process(env, &[&["daemon"], extra].concat())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let daemon = Self { child };
        eventually("the daemon to answer", || health(env));
        daemon
    }

    fn signal(&self, signal: Signal) {
        let pid = Pid::from_raw(self.child.id() as i32).unwrap();
        kill_process(pid, signal).unwrap();
    }
}

impl Drop for Foreground {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            self.signal(Signal::CONT);
            self.signal(Signal::TERM);
            let _ = self.child.wait();
        }
    }
}

struct Detached<'a> {
    env: &'a Env,
}

impl Drop for Detached<'_> {
    fn drop(&mut self) {
        let _ = cmd(self.env, &["daemon", "stop"]).output();
    }
}

fn pid_in(line: &str) -> u32 {
    line.split("pid ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|pid| pid.parse().ok())
        .unwrap_or_else(|| panic!("no pid in {line:?}"))
}

fn snapshot(env: &Env, session: &str) -> Snapshot {
    client(env).snapshot(session).unwrap()
}

#[test]
fn ensure_starts_one_detached_daemon_and_stop_ends_it() {
    let env = env();
    let _cleanup = Detached { env: &env };
    let started = stdout_of(&env, &["daemon", "--ensure"]);
    assert!(
        started.starts_with("✻ Started the claude-plus daemon · pid "),
        "{started}"
    );
    let pid = pid_in(&started);
    assert_eq!(health(&env).unwrap().pid, pid);
    let again = stdout_of(&env, &["daemon", "--ensure"]);
    assert!(
        again.starts_with(&format!(
            "✻ claude-plus daemon is running · pid {pid} · up "
        )),
        "{again}"
    );
    assert_eq!(
        stdout_of(&env, &["daemon", "stop"]),
        format!("✻ Stopped the claude-plus daemon · pid {pid}\n")
    );
    assert!(health(&env).is_none());
    assert!(!env.plus.join("daemon.sock").exists());
    assert_eq!(
        stdout_of(&env, &["daemon", "stop"]),
        "No claude-plus daemon is running\n"
    );
}

#[test]
fn concurrent_ensures_share_one_daemon() {
    let env = env();
    let _cleanup = Detached { env: &env };
    let runs: Vec<_> = (0..4)
        .map(|_| {
            process(&env, &["daemon", "--ensure"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let lines: Vec<String> = runs
        .into_iter()
        .map(|run| {
            let output = run.wait_with_output().unwrap();
            assert!(output.status.success(), "{output:?}");
            String::from_utf8(output.stdout).unwrap()
        })
        .collect();
    let pid = health(&env).unwrap().pid;
    assert!(lines.iter().all(|line| pid_in(line) == pid), "{lines:?}");
    let started = lines
        .iter()
        .filter(|line| line.starts_with("✻ Started the claude-plus daemon"))
        .count();
    assert_eq!(started, 1, "{lines:?}");
    assert_eq!(
        stdout_of(&env, &["daemon", "stop"]),
        format!("✻ Stopped the claude-plus daemon · pid {pid}\n")
    );
}

#[test]
fn a_second_foreground_daemon_is_refused() {
    let env = env();
    let first = Foreground::start(&env, &[]);
    cmd(&env, &["daemon"]).assert().failure().stderr(format!(
        "claude-plus: a claude-plus daemon is already running (pid {})\n",
        first.child.id()
    ));
    assert_eq!(health(&env).unwrap().pid, first.child.id());
}

#[test]
fn sigterm_and_sigint_stop_the_daemon_cleanly() {
    for (signal, name) in [(Signal::TERM, "SIGTERM"), (Signal::INT, "SIGINT")] {
        let env = env();
        let mut daemon = Foreground::start(&env, &[]);
        daemon.signal(signal);
        let status = daemon.child.wait().unwrap();
        assert!(status.success(), "{name}: {status}");
        assert!(!env.plus.join("daemon.sock").exists(), "{name}");
        let log = std::fs::read_to_string(env.plus.join("daemon.log")).unwrap();
        assert!(
            log.contains(&format!("info  {name} received, stopping\n")),
            "{log}"
        );
        assert!(log.ends_with(" info  stopped\n"), "{log}");
        cmd(&env, &["daemon", "--ensure"]).assert().success();
        cmd(&env, &["daemon", "stop"]).assert().success();
    }
}

#[test]
fn the_daemon_keeps_its_files_private() {
    let env = env();
    let _daemon = Foreground::start(&env, &[]);
    let mode = |name: &str| {
        std::fs::metadata(env.plus.join(name))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(mode(""), 0o700);
    for name in ["daemon.sock", "daemon.lock", "daemon.log", "data.db"] {
        assert_eq!(mode(name), 0o600, "{name}");
    }
}

#[test]
fn the_daemon_backfills_then_tails_new_sessions_subagents_and_appends() {
    let env = env();
    let _daemon = Foreground::start(&env, &[]);
    idle_health(&env);
    assert_eq!(snapshot(&env, SESSION_1).totals.requests, 3);

    let session = "33333333-3333-3333-3333-333333333333";
    let dir = env.claude.join("projects/-work-live");
    let line = |id: &str, sidechain: bool| {
        format!(
            r#"{{"type":"assistant","sessionId":"{session}","isSidechain":{sidechain},"timestamp":"2026-10-03T11:00:00Z","requestId":"{id}","message":{{"id":"{id}","model":"claude-sonnet-5-5","content":[],"usage":{{"input_tokens":1,"output_tokens":10}}}}}}"#
        ) + "\n"
    };
    std::fs::create_dir_all(dir.join(session).join("subagents/workflows/wf_1")).unwrap();
    std::fs::write(
        dir.join(format!("{session}.jsonl")),
        line("req_live_1", false),
    )
    .unwrap();
    std::fs::write(
        dir.join(session).join("subagents/agent-s1.jsonl"),
        line("req_live_2", true),
    )
    .unwrap();
    std::fs::write(
        dir.join(session)
            .join("subagents/workflows/wf_1/agent-w9.jsonl"),
        line("req_live_3", true),
    )
    .unwrap();
    let live = eventually("the new session to be tailed", || {
        Some(snapshot(&env, session)).filter(|s| s.totals.requests == 3)
    });
    assert_eq!(live.totals.subagents, 2);

    let main = dir.join(format!("{session}.jsonl"));
    let mut content = std::fs::read_to_string(&main).unwrap();
    content.push_str(&line("req_live_4", false));
    std::fs::write(&main, content).unwrap();
    eventually("the appended line to be tailed", || {
        Some(snapshot(&env, session)).filter(|s| s.totals.requests == 4)
    });
}

#[test]
fn a_projects_dir_created_after_start_is_watched_by_the_next_rescan() {
    let env = env();
    std::fs::rename(env.claude.join("projects"), env.claude.join("later")).unwrap();
    let _daemon = Foreground::start(&env, &["--rescan-secs", "1"]);
    assert!(!health(&env).unwrap().ingest.watching);
    std::fs::rename(env.claude.join("later"), env.claude.join("projects")).unwrap();
    eventually("the late projects dir to be imported", || {
        Some(snapshot(&env, SESSION_1)).filter(|s| s.totals.requests == 3)
    });
    eventually("the watcher to start", || {
        health(&env).filter(|h| h.ingest.watching)
    });
}

#[test]
fn mod_events_round_trip_through_the_daemon() {
    let env = env();
    let _daemon = Foreground::start(&env, &[]);
    let accepted = client(&env)
        .send_events(&json!({"events": [
            {"session": SESSION_1, "kind": "session.start", "ts_ms": 1_791_021_000_000_i64, "data": {}},
            {"session": SESSION_1, "kind": "session.measure", "ts_ms": 1_791_021_500_000_i64, "data": {"contextPercent": 41}}
        ]}))
        .unwrap();
    assert_eq!((accepted.accepted, accepted.rejected.len()), (2, 0));
    let snapshot = snapshot(&env, SESSION_1);
    assert_eq!(
        snapshot.measure.unwrap().data,
        json!({"contextPercent": 41})
    );
}

#[test]
fn a_daemon_that_cannot_open_the_store_explains_where_to_look() {
    let env = env();
    std::fs::create_dir_all(&env.plus).unwrap();
    rusqlite::Connection::open(env.plus.join("data.db"))
        .unwrap()
        .pragma_update(None, "user_version", 3)
        .unwrap();
    let log = env.plus.join("daemon.log");
    cmd(&env, &["daemon", "--ensure"])
        .assert()
        .failure()
        .stderr(format!(
            "claude-plus: the claude-plus daemon exited at start (exit status: 1), see {}\n",
            log.display()
        ));
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(
        text.contains(
            "was created by a pre-release claude-plus; delete it and run claude-plus import"
        ),
        "{text}"
    );
}

#[test]
fn a_data_dir_too_deep_for_a_socket_is_refused_before_starting() {
    let env = env();
    let deep = env._temp.path().join("d".repeat(120));
    let socket = deep.join("daemon.sock");
    let refusal = format!(
        "claude-plus: cannot listen on {}: the socket path is {} bytes and this system allows {}; set CLAUDE_PLUS_HOME to a shorter directory\n",
        socket.display(),
        socket.as_os_str().len(),
        claude_plus::api::MAX_SOCKET_PATH
    );
    for args in [&["daemon", "--ensure"][..], &["daemon"][..]] {
        cmd(&env, args)
            .env("CLAUDE_PLUS_HOME", &deep)
            .assert()
            .failure()
            .stdout("")
            .stderr(refusal.clone());
    }
    assert!(!deep.exists());
}

fn log_of(env: &Env) -> String {
    std::fs::read_to_string(env.plus.join("daemon.log")).unwrap()
}

#[test]
fn import_defers_to_the_running_daemon_and_prints_the_same_report() {
    let env = env();
    let _daemon = Foreground::start(&env, &[]);
    idle_health(&env);
    assert_eq!(
        stdout_of(&env, &["import"]),
        "✻ Imported 0 lines from 0 changed files · 0 new requests · 0 unparsable lines\n  History: 5 requests across 2 sessions · $0.0812 priced\n"
    );
    assert!(log_of(&env).contains(" info  import requested over the socket\n"));
}

#[test]
fn stats_and_reindex_defer_to_the_running_daemon() {
    let env = env();
    let _daemon = Foreground::start(&env, &[]);
    idle_health(&env);
    assert_eq!(stdout_of(&env, &["stats", "--days", "3"]), STATS_TABLE);
    assert_eq!(
        stdout_of(&env, &["reindex"]),
        "✻ Rebuilt 5 requests from 15 stored records\n"
    );
    let log = log_of(&env);
    assert!(
        log.contains(" info  import requested over the socket\n"),
        "{log}"
    );
    assert!(
        log.contains(" info  reindex requested over the socket\n"),
        "{log}"
    );
    assert_eq!(stdout_of(&env, &["stats", "--days", "3"]), STATS_TABLE);
}

#[test]
fn doctor_reports_the_running_daemon() {
    let env = env();
    let daemon = Foreground::start(&env, &[]);
    let doctor = stdout_of(&env, &["doctor"]);
    assert!(
        doctor.contains(&format!(
            "  Daemon:      running · pid {} · up ",
            daemon.child.id()
        )),
        "{doctor}"
    );
}

#[test]
fn a_daemon_reading_another_claude_dir_is_not_used() {
    let env = env();
    let _daemon = Foreground::start(&env, &[]);
    let other = env._temp.path().join("other-claude");
    std::fs::create_dir_all(other.join("projects")).unwrap();
    cmd(&env, &["import"])
        .env("CLAUDE_CONFIG_DIR", &other)
        .assert()
        .failure()
        .stdout("")
        .stderr(format!(
            "claude-plus: the running claude-plus daemon reads {}, not {}; stop it with claude-plus daemon stop\n",
            env.claude.display(),
            other.display()
        ));
}

#[test]
fn a_silent_lock_holder_is_reported_instead_of_racing_it() {
    let env = env();
    std::fs::create_dir_all(&env.plus).unwrap();
    let _held = InstanceLock::acquire(&env.plus.join("daemon.lock")).unwrap();
    let pid = std::process::id();
    cmd(&env, &["import"]).assert().failure().stderr(format!(
        "claude-plus: the claude-plus daemon (pid {pid}) does not answer; stop it with claude-plus daemon stop\n"
    ));
    assert!(stdout_of(&env, &["doctor"]).contains(&format!(
        "  Daemon:      pid {pid} does not answer, see {}\n",
        env.plus.join("daemon.log").display()
    )));
    assert!(!env.plus.join("data.db").exists());
}

#[test]
fn doctor_explains_a_data_dir_too_deep_for_a_socket() {
    let env = env();
    let deep = env._temp.path().join("d".repeat(120));
    let socket = deep.join("daemon.sock");
    let output = cmd(&env, &["doctor"])
        .env("CLAUDE_PLUS_HOME", &deep)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doctor = String::from_utf8(output).unwrap();
    assert!(
        doctor.contains(&format!(
            "  Daemon:      cannot run, the socket path is {} bytes and this system allows {}; set CLAUDE_PLUS_HOME to a shorter directory\n",
            socket.as_os_str().len(),
            claude_plus::api::MAX_SOCKET_PATH
        )),
        "{doctor}"
    );
}

#[test]
fn ensure_starts_a_daemon_once_a_stopping_one_frees_the_lock() {
    let env = env();
    let _cleanup = Detached { env: &env };
    std::fs::create_dir_all(&env.plus).unwrap();
    let held = InstanceLock::acquire(&env.plus.join("daemon.lock")).unwrap();
    let ensure = process(&env, &["daemon", "--ensure"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1000));
    drop(held);
    let output = ensure.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.starts_with("✻ Started the claude-plus daemon · pid "),
        "{stdout}"
    );
    assert_eq!(health(&env).unwrap().pid, pid_in(&stdout));
}

#[test]
fn a_replaced_projects_dir_is_watched_again_without_waiting_for_a_rescan() {
    let env = env();
    let _daemon = Foreground::start(&env, &[]);
    idle_health(&env);
    let replacement = env.claude.join("new-projects");
    std::fs::create_dir_all(replacement.join("-work-moved")).unwrap();
    std::fs::rename(env.claude.join("projects"), env.claude.join("old-projects")).unwrap();
    eventually("the lost watch to be reported", || {
        health(&env).filter(|h| !h.ingest.watching)
    });
    std::fs::rename(&replacement, env.claude.join("projects")).unwrap();
    eventually("the replaced projects dir to be watched", || {
        health(&env).filter(|h| h.ingest.watching && h.ingest.backlog == 0)
    });
    let session = "44444444-4444-4444-4444-444444444444";
    std::fs::write(
        env.claude.join(format!("projects/-work-moved/{session}.jsonl")),
        format!(
            r#"{{"type":"assistant","sessionId":"{session}","timestamp":"2026-10-03T11:00:00Z","requestId":"req_moved","message":{{"id":"req_moved","model":"claude-sonnet-5-5","content":[],"usage":{{"input_tokens":1,"output_tokens":10}}}}}}"#
        ) + "\n",
    )
    .unwrap();
    eventually(
        "a transcript in the replaced projects dir to be tailed",
        || Some(snapshot(&env, session)).filter(|s| s.totals.requests == 1),
    );
}

#[test]
fn a_second_interrupt_stops_the_daemon_without_waiting_for_the_drain() {
    use std::io::Write;
    let env = env();
    let mut daemon = Foreground::start(&env, &[]);
    let mut stalled =
        std::os::unix::net::UnixStream::connect(env.plus.join("daemon.sock")).unwrap();
    stalled.write_all(b"GET /health HTTP/1.1\r\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    daemon.signal(Signal::INT);
    eventually("the first interrupt to be logged", || {
        log_of(&env)
            .contains(" info  SIGINT received, stopping\n")
            .then_some(())
    });
    let second = std::time::Instant::now();
    daemon.signal(Signal::INT);
    let status = daemon.child.wait().unwrap();
    let waited = second.elapsed();
    assert_eq!(status.code(), Some(130), "{status}");
    assert!(waited < std::time::Duration::from_secs(1), "{waited:?}");
    assert!(
        log_of(&env).contains(" warn  SIGINT received again, exiting now\n"),
        "{}",
        log_of(&env)
    );
    assert!(!env.plus.join("daemon.sock").exists());
}

#[test]
fn stop_terminates_a_daemon_that_does_not_answer() {
    let env = env();
    let daemon = Foreground::start(&env, &[]);
    idle_health(&env);
    let pid = daemon.child.id();
    daemon.signal(Signal::STOP);
    let stop = process(&env, &["daemon", "stop"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_secs(2));
    daemon.signal(Signal::CONT);
    let output = stop.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("✻ Stopped the claude-plus daemon · pid {pid}\n")
    );
    let log = log_of(&env);
    assert!(log.contains(" info  SIGTERM received, stopping\n"), "{log}");
    assert!(!log.contains("shutdown requested over the socket"), "{log}");
}
