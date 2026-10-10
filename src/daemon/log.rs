use std::fmt::Display;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

pub const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

pub struct Log {
    file: Mutex<LogFile>,
    echo: bool,
}

struct LogFile {
    path: PathBuf,
    file: File,
    len: u64,
}

impl LogFile {
    fn open(path: &Path) -> Result<Self> {
        if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
            std::fs::rename(path, rotated(path))
                .with_context(|| format!("cannot rotate {}", path.display()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("cannot open {}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            len: file.metadata().map_or(0, |m| m.len()),
            file,
        })
    }

    fn append(&mut self, line: &str) {
        if writeln!(self.file, "{line}").is_ok() {
            self.len += line.len() as u64 + 1;
        }
        if self.len > MAX_LOG_BYTES
            && let Ok(reopened) = Self::open(&self.path)
        {
            *self = reopened;
        }
    }
}

fn rotated(path: &Path) -> PathBuf {
    let mut rotated = path.as_os_str().to_owned();
    rotated.push(".1");
    PathBuf::from(rotated)
}

impl Log {
    pub fn open(path: &Path, echo: bool) -> Result<Self> {
        Ok(Self {
            file: Mutex::new(LogFile::open(path)?),
            echo,
        })
    }

    pub fn info(&self, message: impl Display) {
        self.write("info", message);
    }

    pub fn warn(&self, message: impl Display) {
        self.write("warn", message);
    }

    pub fn error(&self, message: impl Display) {
        self.write("error", message);
    }

    fn write(&self, level: &str, message: impl Display) {
        let line = line(Utc::now(), level, message);
        if let Ok(mut file) = self.file.lock() {
            file.append(&line);
        }
        if self.echo {
            eprintln!("{line}");
        }
    }
}

pub fn log_panics(log: Arc<Log>) {
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        let location = info
            .location()
            .map(|at| format!(" at {}:{}:{}", at.file(), at.line(), at.column()))
            .unwrap_or_default();
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("Box<dyn Any>");
        log.error(format!("thread {name} panicked{location}: {message}"));
    }));
}

fn line(at: DateTime<Utc>, level: &str, message: impl Display) -> String {
    format!(
        "{} {level:<5} {message}",
        at.format("%Y-%m-%dT%H:%M:%S%.3fZ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_carry_a_utc_timestamp_and_an_aligned_level() {
        let at = DateTime::parse_from_rfc3339("2026-10-11T08:00:00.123Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            line(at, "info", "listening"),
            "2026-10-11T08:00:00.123Z info  listening"
        );
        assert_eq!(
            line(at, "error", "boom"),
            "2026-10-11T08:00:00.123Z error boom"
        );
    }

    #[test]
    fn appends_privately_and_rotates_a_large_log_once() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("daemon.log");
        std::fs::write(&path, vec![b'x'; MAX_LOG_BYTES as usize + 1]).unwrap();
        let log = Log::open(&path, false).unwrap();
        log.info("first");
        log.warn("second");
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with(" info  first"), "{}", lines[0]);
        assert!(lines[1].ends_with(" warn  second"), "{}", lines[1]);
        let rotated = temp.path().join("daemon.log.1");
        assert_eq!(
            std::fs::metadata(&rotated).unwrap().len(),
            MAX_LOG_BYTES + 1
        );
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        drop(log);
        Log::open(&path, false).unwrap().info("third");
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 3);
    }

    #[test]
    fn a_log_that_grows_too_large_is_rotated_while_open() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("daemon.log");
        let log = Log::open(&path, false).unwrap();
        log.warn("x".repeat(MAX_LOG_BYTES as usize));
        log.info("after");
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 1, "{}", text.len());
        assert!(text.ends_with(" info  after\n"), "{text}");
        let rotated = std::fs::read_to_string(temp.path().join("daemon.log.1")).unwrap();
        assert!(rotated.len() as u64 > MAX_LOG_BYTES);
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn panics_are_written_to_the_log() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("daemon.log");
        let log = std::sync::Arc::new(Log::open(&path, false).unwrap());
        log_panics(log);
        let panicked = std::thread::Builder::new()
            .name("doomed".into())
            .spawn(|| panic!("the store vanished"))
            .unwrap()
            .join();
        let _ = std::panic::take_hook();
        assert!(panicked.is_err());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains(" error thread doomed panicked at src/daemon/log.rs:"),
            "{text}"
        );
        assert!(text.trim_end().ends_with(": the store vanished"), "{text}");
    }
}
