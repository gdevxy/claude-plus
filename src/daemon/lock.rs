use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Seek, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use rustix::fs::{FlockOperation, flock};

const ATTEMPTS: u32 = 5;
const RETRY: Duration = Duration::from_millis(20);

#[derive(Debug, thiserror::Error)]
#[error("a claude-plus daemon is already running{}", pid_suffix(*.pid))]
pub struct AlreadyRunning {
    pub pid: Option<u32>,
}

fn pid_suffix(pid: Option<u32>) -> String {
    pid.map(|pid| format!(" (pid {pid})")).unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holder {
    Free,
    Held { pid: Option<u32> },
}

pub struct InstanceLock {
    file: File,
}

impl InstanceLock {
    pub fn acquire(path: &Path) -> Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("cannot open {}", path.display()))?;
        for attempt in 1..=ATTEMPTS {
            match flock(&file, FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => {
                    file.set_len(0)?;
                    file.rewind()?;
                    write!(file, "{}", std::process::id())?;
                    file.flush()?;
                    return Ok(Self { file });
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock && attempt < ATTEMPTS => {
                    std::thread::sleep(RETRY);
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) => {
                    return Err(error).with_context(|| format!("cannot lock {}", path.display()));
                }
            }
        }
        Err(AlreadyRunning {
            pid: read_pid(&mut file),
        }
        .into())
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = self.file.set_len(0);
    }
}

pub fn probe(path: &Path) -> Result<Holder> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Holder::Free),
        Err(error) => return Err(error).with_context(|| format!("cannot open {}", path.display())),
    };
    match flock(&file, FlockOperation::NonBlockingLockShared) {
        Ok(()) => {
            flock(&file, FlockOperation::Unlock)?;
            Ok(Holder::Free)
        }
        Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(Holder::Held {
            pid: read_pid(&mut file),
        }),
        Err(error) => Err(error).with_context(|| format!("cannot probe {}", path.display())),
    }
}

fn read_pid(file: &mut File) -> Option<u32> {
    let mut text = String::new();
    file.rewind().ok()?;
    file.read_to_string(&mut text).ok()?;
    text.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_holder_at_a_time_and_its_pid_is_readable() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("daemon.lock");
        assert_eq!(probe(&path).unwrap(), Holder::Free);
        assert!(!path.exists());
        let held = InstanceLock::acquire(&path).unwrap();
        let pid = Some(std::process::id());
        assert_eq!(probe(&path).unwrap(), Holder::Held { pid });
        let error = InstanceLock::acquire(&path).err().expect("must be held");
        assert_eq!(
            error.to_string(),
            format!(
                "a claude-plus daemon is already running (pid {})",
                std::process::id()
            )
        );
        assert_eq!(error.downcast_ref::<AlreadyRunning>().unwrap().pid, pid);
        drop(held);
        assert_eq!(probe(&path).unwrap(), Holder::Free);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        InstanceLock::acquire(&path).unwrap();
    }

    #[test]
    fn a_probe_never_makes_the_daemon_fail_to_start() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("daemon.lock");
        std::fs::write(&path, "").unwrap();
        let prober = File::open(&path).unwrap();
        flock(&prober, FlockOperation::NonBlockingLockShared).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(RETRY);
            flock(&prober, FlockOperation::Unlock).unwrap();
        });
        assert!(InstanceLock::acquire(&path).is_ok());
        release.join().unwrap();
    }

    #[test]
    fn the_lock_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("daemon.lock");
        let _held = InstanceLock::acquire(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
