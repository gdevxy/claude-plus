use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub claude_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl Paths {
    pub fn from_env() -> Result<Self> {
        Self::from_vars(|name| std::env::var_os(name))
    }

    pub fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Result<Self> {
        let path = |name| {
            var(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        let home = path("HOME").context("HOME is not set")?;
        Ok(Self::resolve(
            &home,
            path("CLAUDE_CONFIG_DIR"),
            path("CLAUDE_PLUS_HOME"),
        ))
    }

    pub fn resolve(
        home: &Path,
        claude_config_dir: Option<PathBuf>,
        plus_home: Option<PathBuf>,
    ) -> Self {
        Self {
            claude_dir: claude_config_dir.unwrap_or_else(|| home.join(".claude")),
            data_dir: plus_home.unwrap_or_else(|| home.join(".claude-plus")),
        }
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.claude_dir.join("projects")
    }

    pub fn database(&self) -> PathBuf {
        self.data_dir.join("data.db")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.claude_dir.join("settings.json")
    }

    pub fn ensure_data_dir(&self) -> Result<()> {
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_live_under_home() {
        let paths = Paths::resolve(Path::new("/home/ada"), None, None);
        assert_eq!(paths.claude_dir, PathBuf::from("/home/ada/.claude"));
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
    }

    #[test]
    fn overrides_win() {
        let paths = Paths::resolve(
            Path::new("/home/ada"),
            Some(PathBuf::from("/tmp/claude")),
            Some(PathBuf::from("/tmp/plus")),
        );
        assert_eq!(paths.claude_dir, PathBuf::from("/tmp/claude"));
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
        let paths = Paths::from_vars(vars(&[
            ("HOME", "/home/ada"),
            ("CLAUDE_CONFIG_DIR", "/cfg"),
            ("CLAUDE_PLUS_HOME", "/plus"),
        ]))
        .unwrap();
        assert_eq!(
            (paths.claude_dir, paths.data_dir),
            (PathBuf::from("/cfg"), PathBuf::from("/plus"))
        );
    }

    #[test]
    fn empty_overrides_count_as_unset() {
        let paths = Paths::from_vars(vars(&[
            ("HOME", "/home/ada"),
            ("CLAUDE_CONFIG_DIR", ""),
            ("CLAUDE_PLUS_HOME", ""),
        ]))
        .unwrap();
        assert_eq!(paths, Paths::resolve(Path::new("/home/ada"), None, None));
    }

    #[test]
    fn missing_or_empty_home_is_an_error() {
        for pairs in [&[][..], &[("HOME", "")][..]] {
            let error = Paths::from_vars(vars(pairs)).unwrap_err();
            assert_eq!(error.to_string(), "HOME is not set");
        }
    }

    #[test]
    fn from_env_uses_the_process_environment() {
        let expected = Paths::from_vars(|name| std::env::var_os(name)).unwrap();
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
