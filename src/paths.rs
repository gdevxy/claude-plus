use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub home: PathBuf,
    pub claude_dir: PathBuf,
    pub claude_json: PathBuf,
    pub data_dir: PathBuf,
}

impl Paths {
    pub fn from_env() -> Result<Self> {
        let cwd = std::env::current_dir().ok();
        Self::from_vars(|name| std::env::var_os(name), cwd.as_deref())
    }

    pub fn from_vars(var: impl Fn(&str) -> Option<OsString>, cwd: Option<&Path>) -> Result<Self> {
        let path = |name| -> Result<Option<PathBuf>> {
            let Some(path) = var(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
            else {
                return Ok(None);
            };
            let path = match cwd {
                Some(cwd) if !path.is_absolute() && name == "CLAUDE_CONFIG_DIR" => cwd.join(path),
                _ => path,
            };
            if !path.is_absolute() {
                bail!("{name} must be an absolute path, not {}", path.display());
            }
            Ok(Some(path.components().collect()))
        };
        let home = path("HOME")?.context("HOME is not set")?;
        Ok(Self::resolve(
            &home,
            path("CLAUDE_CONFIG_DIR")?,
            path("CLAUDE_PLUS_HOME")?,
        ))
    }

    pub fn resolve(
        home: &Path,
        claude_config_dir: Option<PathBuf>,
        plus_home: Option<PathBuf>,
    ) -> Self {
        Self {
            home: home.to_path_buf(),
            claude_json: claude_config_dir
                .as_ref()
                .map_or_else(|| home.join(".claude.json"), |dir| dir.join(".claude.json")),
            claude_dir: claude_config_dir.unwrap_or_else(|| home.join(".claude")),
            data_dir: plus_home.unwrap_or_else(|| home.join(".claude-plus")),
        }
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.claude_dir.join("projects")
    }

    pub fn desktop_sessions_dir(&self) -> PathBuf {
        desktop_dir_in(&self.home, std::env::consts::OS).join("claude-code-sessions")
    }

    pub fn user_prices(&self) -> PathBuf {
        self.data_dir.join("prices.toml")
    }

    pub fn database(&self) -> PathBuf {
        self.data_dir.join("data.db")
    }

    pub const DEFAULT_RETENTION_DAYS: u64 = 30;

    pub fn retention_days(&self) -> Option<u64> {
        retention_days_in(&self.claude_dir)
    }

    pub fn settings_file(&self) -> PathBuf {
        self.claude_dir.join("settings.json")
    }

    pub fn socket(&self) -> PathBuf {
        self.data_dir.join("daemon.sock")
    }

    pub fn daemon_lock(&self) -> PathBuf {
        self.data_dir.join("daemon.lock")
    }

    pub fn daemon_stopped(&self) -> PathBuf {
        self.data_dir.join("daemon.stopped")
    }

    pub fn daemon_log(&self) -> PathBuf {
        self.data_dir.join("daemon.log")
    }

    pub fn marketplace_dir(&self) -> PathBuf {
        self.data_dir.join("marketplace")
    }

    pub fn marker(&self) -> PathBuf {
        self.data_dir.join(MARKER)
    }

    pub fn ensure_data_dir(&self) -> Result<()> {
        let existed = std::fs::symlink_metadata(&self.data_dir).is_ok();
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
            builder.mode(0o700);
            builder
                .create(&self.data_dir)
                .with_context(|| format!("cannot create {}", self.data_dir.display()))?;
            std::fs::set_permissions(&self.data_dir, std::fs::Permissions::from_mode(0o700))
                .with_context(|| {
                    format!("cannot restrict {} to its owner", self.data_dir.display())
                })?;
        }
        #[cfg(not(unix))]
        builder
            .create(&self.data_dir)
            .with_context(|| format!("cannot create {}", self.data_dir.display()))?;
        if !existed || self.entries()?.is_empty() {
            self.mark_data_dir()?;
        }
        Ok(())
    }

    fn entries(&self) -> Result<Vec<String>> {
        std::fs::read_dir(&self.data_dir)
            .with_context(|| format!("cannot read {}", self.data_dir.display()))?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<std::io::Result<_>>()
            .with_context(|| format!("cannot read {}", self.data_dir.display()))
    }

    fn mark_data_dir(&self) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let opened = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.marker());
        match opened {
            Ok(mut file) => file
                .write_all(MARKER_TEXT.as_bytes())
                .with_context(|| format!("cannot write {}", self.marker().display())),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("cannot write {}", self.marker().display()))
            }
        }
    }
}

pub const MARKER: &str = ".claude-plus-data";

pub const DATABASE_FILES: &[&str] = &["data.db-wal", "data.db-shm", "data.db-journal"];
pub const DAEMON_FILES: &[&str] = &[
    "daemon.sock",
    "daemon.lock",
    "daemon.log",
    "daemon.log.1",
    "daemon.stopped",
];
const MARKER_TEXT: &str =
    "claude-plus keeps its data here; claude-plus uninstall --purge deletes what it wrote\n";

fn desktop_dir_in(home: &Path, os: &str) -> PathBuf {
    match os {
        "macos" => home.join("Library/Application Support/Claude"),
        _ => home.join(".config/Claude"),
    }
}

pub fn retention_days_in(claude_dir: &Path) -> Option<u64> {
    std::fs::read_to_string(claude_dir.join("settings.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|settings| settings.get("cleanupPeriodDays")?.as_u64())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_app_keeps_its_sessions_in_its_own_data_folder() {
        let home = Path::new("/home/ada");
        assert_eq!(
            desktop_dir_in(home, "macos"),
            PathBuf::from("/home/ada/Library/Application Support/Claude")
        );
        assert_eq!(
            desktop_dir_in(home, "linux"),
            PathBuf::from("/home/ada/.config/Claude")
        );
        assert_eq!(
            Paths::resolve(home, Some(PathBuf::from("/cfg")), None).desktop_sessions_dir(),
            desktop_dir_in(home, std::env::consts::OS).join("claude-code-sessions")
        );
    }

    #[test]
    fn defaults_live_under_home() {
        let paths = Paths::resolve(Path::new("/home/ada"), None, None);
        assert_eq!(paths.claude_dir, PathBuf::from("/home/ada/.claude"));
        assert_eq!(paths.claude_json, PathBuf::from("/home/ada/.claude.json"));
        assert_eq!(paths.data_dir, PathBuf::from("/home/ada/.claude-plus"));
        assert_eq!(
            paths.projects_dir(),
            PathBuf::from("/home/ada/.claude/projects")
        );
        assert_eq!(
            paths.database(),
            PathBuf::from("/home/ada/.claude-plus/data.db")
        );
        assert_eq!(
            paths.settings_file(),
            PathBuf::from("/home/ada/.claude/settings.json")
        );
        assert_eq!(
            [paths.socket(), paths.daemon_lock(), paths.daemon_log()],
            [
                PathBuf::from("/home/ada/.claude-plus/daemon.sock"),
                PathBuf::from("/home/ada/.claude-plus/daemon.lock"),
                PathBuf::from("/home/ada/.claude-plus/daemon.log"),
            ]
        );
    }

    #[test]
    fn the_mod_marketplace_lives_in_the_data_dir() {
        let paths = Paths::resolve(Path::new("/home/ada"), None, Some(PathBuf::from("/plus")));
        assert_eq!(paths.marketplace_dir(), PathBuf::from("/plus/marketplace"));
    }

    #[test]
    fn overrides_win() {
        let paths = Paths::resolve(
            Path::new("/home/ada"),
            Some(PathBuf::from("/tmp/claude")),
            Some(PathBuf::from("/tmp/plus")),
        );
        assert_eq!(paths.claude_dir, PathBuf::from("/tmp/claude"));
        assert_eq!(paths.claude_json, PathBuf::from("/tmp/claude/.claude.json"));
        assert_eq!(paths.data_dir, PathBuf::from("/tmp/plus"));
    }

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn from_vars_reads_home_and_overrides() {
        let paths = Paths::from_vars(
            vars(&[
                ("HOME", "/home/ada"),
                ("CLAUDE_CONFIG_DIR", "/cfg"),
                ("CLAUDE_PLUS_HOME", "/plus"),
            ]),
            None,
        )
        .unwrap();
        assert_eq!(
            (paths.claude_dir, paths.data_dir),
            (PathBuf::from("/cfg"), PathBuf::from("/plus"))
        );
    }

    #[test]
    fn empty_overrides_count_as_unset() {
        let paths = Paths::from_vars(
            vars(&[
                ("HOME", "/home/ada"),
                ("CLAUDE_CONFIG_DIR", ""),
                ("CLAUDE_PLUS_HOME", ""),
            ]),
            None,
        )
        .unwrap();
        assert_eq!(paths, Paths::resolve(Path::new("/home/ada"), None, None));
    }

    #[test]
    fn a_relative_home_or_data_dir_is_an_error() {
        for name in ["HOME", "CLAUDE_PLUS_HOME"] {
            let mut pairs = vec![("HOME", "/home/ada")];
            pairs.retain(|(key, _)| *key != name);
            pairs.push((name, "."));
            let error = Paths::from_vars(vars(&pairs), Some(Path::new("/work"))).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("{name} must be an absolute path, not .")
            );
        }
    }

    #[test]
    fn a_relative_claude_config_dir_is_taken_from_the_working_directory_as_claude_code_does() {
        let pairs = [("HOME", "/home/ada"), ("CLAUDE_CONFIG_DIR", "cfg")];
        let paths = Paths::from_vars(vars(&pairs), Some(Path::new("/work"))).unwrap();
        assert_eq!(paths.claude_dir, PathBuf::from("/work/cfg"));
        let error = Paths::from_vars(vars(&pairs), None).unwrap_err();
        assert_eq!(
            error.to_string(),
            "CLAUDE_CONFIG_DIR must be an absolute path, not cfg"
        );
    }

    #[test]
    fn trailing_separators_are_dropped() {
        let paths = Paths::from_vars(
            vars(&[("HOME", "/home/ada/"), ("CLAUDE_PLUS_HOME", "/plus//")]),
            None,
        )
        .unwrap();
        assert_eq!(
            (paths.home, paths.data_dir),
            (PathBuf::from("/home/ada"), PathBuf::from("/plus"))
        );
    }

    #[test]
    fn the_data_dir_carries_a_marker_that_is_never_rewritten() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(temp.path(), None, None);
        paths.ensure_data_dir().unwrap();
        assert_eq!(paths.marker(), paths.data_dir.join(MARKER));
        let text = std::fs::read_to_string(paths.marker()).unwrap();
        assert!(
            text.starts_with("claude-plus keeps its data here"),
            "{text}"
        );
        std::fs::write(paths.marker(), "kept").unwrap();
        paths.ensure_data_dir().unwrap();
        assert_eq!(std::fs::read_to_string(paths.marker()).unwrap(), "kept");
    }

    #[test]
    fn only_a_new_or_empty_data_dir_is_marked() {
        let temp = tempfile::tempdir().unwrap();
        let at = |name: &str| Paths::resolve(temp.path(), None, Some(temp.path().join(name)));
        let new = at("new");
        new.ensure_data_dir().unwrap();
        assert!(new.marker().is_file());

        let empty = at("empty");
        std::fs::create_dir(&empty.data_dir).unwrap();
        empty.ensure_data_dir().unwrap();
        assert!(empty.marker().is_file());

        let foreign = at("foreign");
        std::fs::create_dir(&foreign.data_dir).unwrap();
        std::fs::write(foreign.data_dir.join("daemon.log"), "theirs").unwrap();
        foreign.ensure_data_dir().unwrap();
        drop(crate::store::Store::open(&foreign.database()).unwrap());
        foreign.ensure_data_dir().unwrap();
        assert!(!foreign.marker().exists());
    }

    #[test]
    fn missing_or_empty_home_is_an_error() {
        for pairs in [&[][..], &[("HOME", "")][..]] {
            let error = Paths::from_vars(vars(pairs), None).unwrap_err();
            assert_eq!(error.to_string(), "HOME is not set");
        }
    }

    #[test]
    fn from_env_uses_the_process_environment() {
        let cwd = std::env::current_dir().unwrap();
        let expected = Paths::from_vars(|name| std::env::var_os(name), Some(&cwd)).unwrap();
        assert_eq!(Paths::from_env().unwrap(), expected);
    }

    #[cfg(unix)]
    #[test]
    fn missing_parents_of_the_data_dir_are_private_too() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(temp.path(), None, Some(temp.path().join("a/b")));
        paths.ensure_data_dir().unwrap();
        for dir in [temp.path().join("a"), temp.path().join("a/b")] {
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{}", dir.display());
        }
    }

    #[cfg(unix)]
    #[test]
    fn data_dir_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(temp.path(), None, None);
        paths.ensure_data_dir().unwrap();
        let mode = std::fs::metadata(&paths.data_dir)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
    }
}
