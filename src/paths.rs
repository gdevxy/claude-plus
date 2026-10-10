use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub claude_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl Paths {
    pub fn from_env() -> Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME is not set")?;
        Ok(Self::resolve(
            &home,
            std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from),
            std::env::var_os("CLAUDE_PLUS_HOME").map(PathBuf::from),
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
        std::fs::create_dir_all(&self.data_dir)
            .with_context(|| format!("cannot create {}", self.data_dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.data_dir, std::fs::Permissions::from_mode(0o700))?;
        }
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
