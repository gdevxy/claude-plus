use std::cmp::Ordering;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use rustix::process::{Pid, Signal, kill_process};

use super::lock::{Holder, probe};
use crate::api::check_socket_path;
use crate::api::client::Client;
use crate::api::wire::{API_VERSION, Health};
use crate::paths::Paths;
use crate::store::Store;

pub const PROBE_TIMEOUT: Duration = Duration::from_millis(500);
pub const START_TIMEOUT: Duration = Duration::from_secs(3);
pub const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(25);
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Running(Health),
    NotRunning,
    Unresponsive { pid: Option<u32> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Ensured {
    Running(Health),
    KeptNewer(Health),
    Started(Health),
    Restarted { previous: Health, health: Health },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    Stopped { pid: Option<u32> },
    NotRunning,
}

#[derive(Debug, Clone, PartialEq)]
enum Next {
    Keep(Health),
    KeepNewer(Health),
    Start,
    Replace(Health),
    Wait(Option<u32>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Build<'a> {
    pub version: &'a str,
    pub schema: usize,
    pub api: u32,
}

impl Build<'static> {
    pub fn this() -> Self {
        Self {
            version: VERSION,
            schema: Store::latest_schema_version(),
            api: API_VERSION,
        }
    }
}

impl Build<'_> {
    fn is_older_than(&self, other: &Build) -> bool {
        self.order(other).is_lt()
    }

    fn order(&self, other: &Build) -> Ordering {
        self.api
            .cmp(&other.api)
            .then(self.schema.cmp(&other.schema))
            .then_with(|| {
                if is_older(self.version, other.version) {
                    Ordering::Less
                } else if is_older(other.version, self.version) {
                    Ordering::Greater
                } else {
                    Ordering::Equal
                }
            })
    }

    fn of(health: &Health) -> Build<'_> {
        Build {
            version: &health.version,
            schema: health.schema,
            api: health.v,
        }
    }
}

fn next_step(status: Status, ours: &Build) -> Next {
    match status {
        Status::Running(health) if Build::of(&health).is_older_than(ours) => Next::Replace(health),
        Status::Running(health) if ours.is_older_than(&Build::of(&health)) => {
            Next::KeepNewer(health)
        }
        Status::Running(health) => Next::Keep(health),
        Status::NotRunning => Next::Start,
        Status::Unresponsive { pid } => Next::Wait(pid),
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Identifier<'a> {
    Numeric(u64),
    Text(&'a str),
}

#[derive(Debug, PartialEq, Eq)]
struct Version<'a> {
    core: (u64, u64, u64),
    pre: Vec<Identifier<'a>>,
}

impl<'a> Version<'a> {
    fn parse(text: &'a str) -> Option<Self> {
        let text = text.split_once('+').map_or(text, |(version, _)| version);
        let (core, pre) = text
            .split_once('-')
            .map_or((text, None), |(c, p)| (c, Some(p)));
        let mut numbers = core.split('.').map(|n| n.parse::<u64>().ok());
        let core = (numbers.next()??, numbers.next()??, numbers.next()??);
        if numbers.next().is_some() {
            return None;
        }
        let pre = match pre {
            None => Vec::new(),
            Some(pre) => pre
                .split('.')
                .map(|id| match id.parse() {
                    Ok(number) => Some(Identifier::Numeric(number)),
                    Err(_) if !id.is_empty() => Some(Identifier::Text(id)),
                    Err(_) => None,
                })
                .collect::<Option<_>>()?,
        };
        Some(Self { core, pre })
    }

    fn precedes(&self, other: &Self) -> bool {
        match self.core.cmp(&other.core) {
            Ordering::Equal => match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, _) => false,
                (false, true) => true,
                (false, false) => self.pre < other.pre,
            },
            order => order.is_lt(),
        }
    }
}

pub fn is_older(version: &str, than: &str) -> bool {
    match (Version::parse(version), Version::parse(than)) {
        (Some(version), Some(than)) => version.precedes(&than),
        (None, Some(_)) => true,
        _ => false,
    }
}

pub fn status(paths: &Paths) -> Result<Status> {
    match Client::new(&paths.socket())
        .with_timeout(PROBE_TIMEOUT)
        .health()
    {
        Ok(health) => Ok(Status::Running(health)),
        Err(_) => Ok(match probe(&paths.daemon_lock())? {
            Holder::Held { pid } => Status::Unresponsive { pid },
            Holder::Free => Status::NotRunning,
        }),
    }
}

pub fn ensure(paths: &Paths) -> Result<Ensured> {
    check_socket_path(&paths.socket())?;
    match next_step(status(paths)?, &Build::this()) {
        Next::Keep(health) => Ok(Ensured::Running(health)),
        Next::KeepNewer(health) => Ok(Ensured::KeptNewer(health)),
        Next::Start => start(paths),
        Next::Replace(old) => {
            stop(paths)?;
            Ok(match start(paths)? {
                Ensured::Started(health) | Ensured::Running(health) => Ensured::Restarted {
                    previous: old,
                    health,
                },
                restarted => restarted,
            })
        }
        Next::Wait(pid) => wait_for_daemon(paths, pid),
    }
}

fn wait_for_daemon(paths: &Paths, pid: Option<u32>) -> Result<Ensured> {
    let client = Client::new(&paths.socket()).with_timeout(PROBE_TIMEOUT);
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if let Ok(health) = client.health() {
            return Ok(Ensured::Running(health));
        }
        if probe(&paths.daemon_lock())? == Holder::Free {
            return start(paths);
        }
        if Instant::now() >= deadline {
            bail!(unresponsive(paths, pid));
        }
        std::thread::sleep(POLL);
    }
}

pub fn stop(paths: &Paths) -> Result<Stopped> {
    let (known, asked) = match status(paths)? {
        Status::NotRunning => return Ok(Stopped::NotRunning),
        Status::Running(health) => {
            let _ = Client::new(&paths.socket())
                .with_timeout(PROBE_TIMEOUT)
                .shutdown();
            (Some(health.pid), true)
        }
        Status::Unresponsive { pid } => (pid, false),
    };
    if asked && wait_until_free(paths, STOP_TIMEOUT)? {
        return Ok(Stopped::Stopped { pid: known });
    }
    let pid = match probe(&paths.daemon_lock())? {
        Holder::Free => return Ok(Stopped::Stopped { pid: known }),
        Holder::Held { pid: Some(pid) } => pid,
        Holder::Held { pid: None } => bail!(unresponsive(paths, known)),
    };
    let target = i32::try_from(pid)
        .ok()
        .and_then(Pid::from_raw)
        .with_context(|| format!("invalid daemon pid {pid}"))?;
    let _ = kill_process(target, Signal::TERM);
    if wait_until_free(paths, STOP_TIMEOUT)? {
        return Ok(Stopped::Stopped { pid: Some(pid) });
    }
    bail!(
        "the claude-plus daemon (pid {pid}) did not stop within {} s, see {}",
        STOP_TIMEOUT.as_secs(),
        paths.daemon_log().display()
    )
}

fn start(paths: &Paths) -> Result<Ensured> {
    paths.ensure_data_dir()?;
    let mut command = Command::new(std::env::current_exe()?);
    // SAFETY: setsid is async-signal-safe, so it may run between fork and exec.
    unsafe {
        command.pre_exec(|| {
            rustix::process::setsid()?;
            Ok(())
        });
    }
    let mut child = command
        .args(["daemon", "--detached"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("cannot start the claude-plus daemon")?;
    let client = Client::new(&paths.socket()).with_timeout(PROBE_TIMEOUT);
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        let exited = child.try_wait()?;
        match (exited, client.health()) {
            (None, Ok(health)) if health.pid == child.id() => return Ok(Ensured::Started(health)),
            (Some(exit), Ok(health)) if exit.success() => return Ok(Ensured::Running(health)),
            (Some(exit), _) if !exit.success() => bail!(
                "the claude-plus daemon exited at start ({exit}), see {}",
                paths.daemon_log().display()
            ),
            _ => {}
        }
        if Instant::now() >= deadline {
            bail!(
                "the claude-plus daemon did not answer within {} s, see {}",
                START_TIMEOUT.as_secs(),
                paths.daemon_log().display()
            );
        }
        std::thread::sleep(POLL);
    }
}

fn wait_until_free(paths: &Paths, timeout: Duration) -> Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        if probe(&paths.daemon_lock())? == Holder::Free {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(POLL);
    }
}

fn unresponsive(paths: &Paths, pid: Option<u32>) -> String {
    let who = pid.map(|pid| format!(" (pid {pid})")).unwrap_or_default();
    format!(
        "the claude-plus daemon{who} holds {} but does not answer, see {}",
        paths.daemon_lock().display(),
        paths.daemon_log().display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::wire::IngestHealth;

    fn health(version: &str) -> Health {
        Health {
            v: 1,
            version: version.into(),
            schema: 9,
            pid: 42,
            uptime_s: 1,
            claude_dir: "/c".into(),
            ingest: IngestHealth {
                lag_ms: 0,
                backlog: 0,
                last_ingest_ms: None,
                progress: None,
                watching: true,
                last_error: None,
                last_error_at_ms: None,
                rebuilding: false,
            },
        }
    }

    fn release(version: &str) -> Build<'_> {
        Build {
            version,
            schema: 9,
            api: 1,
        }
    }

    #[test]
    fn ensure_keeps_a_matching_daemon_and_replaces_another_version() {
        assert_eq!(
            next_step(Status::Running(health("0.1.0")), &release("0.1.0")),
            Next::Keep(health("0.1.0"))
        );
        assert_eq!(
            next_step(Status::Running(health("0.0.9")), &release("0.1.0")),
            Next::Replace(health("0.0.9"))
        );
        assert_eq!(
            next_step(Status::Running(health("0.10.0")), &release("0.9.1")),
            Next::KeepNewer(health("0.10.0"))
        );
        assert_eq!(
            next_step(Status::Running(health("0.1.0")), &release("0.1.0-rc.1")),
            Next::KeepNewer(health("0.1.0"))
        );
        assert_eq!(
            next_step(Status::Running(health("0.1.0-rc.1")), &release("0.1.0")),
            Next::Replace(health("0.1.0-rc.1"))
        );
        assert_eq!(
            next_step(Status::Running(health("garbage")), &release("0.1.0")),
            Next::Replace(health("garbage"))
        );
        assert_eq!(
            next_step(
                Status::Running(health("0.1.0+linux")),
                &release("0.1.0+macos")
            ),
            Next::Keep(health("0.1.0+linux"))
        );
        assert_eq!(
            next_step(Status::NotRunning, &release("0.1.0")),
            Next::Start
        );
        assert_eq!(
            next_step(Status::Unresponsive { pid: Some(7) }, &release("0.1.0")),
            Next::Wait(Some(7))
        );
    }

    fn built(version: &str, schema: usize, v: u32) -> Health {
        Health {
            schema,
            v,
            ..health(version)
        }
    }

    #[test]
    fn ensure_replaces_a_daemon_older_on_its_store_or_api_and_keeps_a_newer_one() {
        let ours = Build {
            version: "0.1.0",
            schema: 10,
            api: 1,
        };
        let step = |health: Health| next_step(Status::Running(health), &ours);
        assert_eq!(
            step(built("0.1.0", 10, 1)),
            Next::Keep(built("0.1.0", 10, 1))
        );
        assert_eq!(
            step(built("0.1.0", 9, 1)),
            Next::Replace(built("0.1.0", 9, 1))
        );
        assert_eq!(
            step(built("0.1.0", 10, 0)),
            Next::Replace(built("0.1.0", 10, 0))
        );
        assert_eq!(
            step(built("0.2.0", 9, 1)),
            Next::Replace(built("0.2.0", 9, 1))
        );
        assert_eq!(
            step(built("0.1.0", 11, 1)),
            Next::KeepNewer(built("0.1.0", 11, 1))
        );
        assert_eq!(
            step(built("0.1.0", 10, 2)),
            Next::KeepNewer(built("0.1.0", 10, 2))
        );
    }

    #[test]
    fn two_builds_never_replace_each_other_back_and_forth() {
        let builds = [
            ("0.1.0", 10, 1),
            ("0.2.0", 9, 1),
            ("0.1.0", 9, 2),
            ("0.3.0", 11, 0),
            ("0.1.0", 10, 1),
            ("not-a-version", 10, 1),
        ];
        for (a_version, a_schema, a_api) in builds {
            for (b_version, b_schema, b_api) in builds {
                let a = Build {
                    version: a_version,
                    schema: a_schema,
                    api: a_api,
                };
                let b = Build {
                    version: b_version,
                    schema: b_schema,
                    api: b_api,
                };
                let a_replaces_b = matches!(
                    next_step(Status::Running(built(b_version, b_schema, b_api)), &a),
                    Next::Replace(_)
                );
                let b_replaces_a = matches!(
                    next_step(Status::Running(built(a_version, a_schema, a_api)), &b),
                    Next::Replace(_)
                );
                assert!(!(a_replaces_b && b_replaces_a), "{a:?} and {b:?} swap");
            }
        }
        let ours = Build {
            version: "0.1.0",
            schema: 10,
            api: 1,
        };
        assert_eq!(
            next_step(Status::Running(built("0.2.0", 9, 1)), &ours),
            Next::Replace(built("0.2.0", 9, 1))
        );
        assert_eq!(
            next_step(Status::Running(built("0.1.0", 9, 2)), &ours),
            Next::KeepNewer(built("0.1.0", 9, 2))
        );
    }

    #[test]
    fn versions_compare_by_semver_precedence() {
        let ordered = [
            "0.9.0",
            "0.10.0-alpha",
            "0.10.0-alpha.1",
            "0.10.0-alpha.beta",
            "0.10.0-beta.2",
            "0.10.0-beta.11",
            "0.10.0-rc.1",
            "0.10.0",
            "1.0.0+build.5",
            "1.2.3",
        ];
        for pair in ordered.windows(2) {
            assert!(is_older(pair[0], pair[1]), "{} < {}", pair[0], pair[1]);
            assert!(!is_older(pair[1], pair[0]), "{} > {}", pair[1], pair[0]);
        }
        assert!(!is_older("1.0.0+a", "1.0.0+b"));
        assert!(is_older("1.0", "1.0.0"));
    }

    #[test]
    fn nothing_running_is_reported_without_creating_files() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(temp.path(), None, Some(temp.path().join("plus")));
        assert_eq!(status(&paths).unwrap(), Status::NotRunning);
        assert_eq!(stop(&paths).unwrap(), Stopped::NotRunning);
        assert!(!paths.data_dir.exists());
    }

    #[test]
    fn a_lock_holder_without_a_socket_is_unresponsive() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(temp.path(), None, None);
        paths.ensure_data_dir().unwrap();
        let _held = super::super::lock::InstanceLock::acquire(&paths.daemon_lock()).unwrap();
        assert_eq!(
            status(&paths).unwrap(),
            Status::Unresponsive {
                pid: Some(std::process::id())
            }
        );
    }
}
