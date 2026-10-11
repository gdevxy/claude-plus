use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rustix::fs::{FlockOperation, flock};

use crate::daemon::control::is_older;

include!(concat!(env!("OUT_DIR"), "/bundle.rs"));

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MARKETPLACE: &str = "claude-plus";
pub const PLUGIN_ID: &str = "plus@claude-plus";
const MANIFEST: &str = "mod/.claude-plugin/plugin.json";

pub fn write(dir: &Path) -> Result<()> {
    let _lock = lock(dir)?;
    replace(dir)
}

pub fn refresh(dir: &Path) -> Result<Option<String>> {
    let outdated = |dir: &Path| version_in(dir).filter(|installed| is_older(installed, VERSION));
    if outdated(dir).is_none() {
        return Ok(None);
    }
    let _lock = lock(dir)?;
    let Some(installed) = outdated(dir) else {
        return Ok(None);
    };
    replace(dir)?;
    Ok(Some(installed))
}

fn lock(dir: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = sibling(dir, "lock");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    flock(&file, FlockOperation::LockExclusive)
        .with_context(|| format!("cannot lock {}", path.display()))?;
    Ok(file)
}

pub fn remove(dir: &Path) -> Result<()> {
    remove_if_present(dir)?;
    match std::fs::remove_file(sibling(dir, "lock")) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

fn replace(dir: &Path) -> Result<()> {
    let staging = sibling(dir, "new");
    let previous = sibling(dir, "old");
    for leftover in [&staging, &previous] {
        remove_if_present(leftover)?;
    }
    for (name, bytes) in FILES {
        let path = staging.join(name);
        create_private_dirs(path.parent().expect("a bundled file has a parent"))?;
        write_private(&path, bytes)?;
    }
    if dir.exists() {
        std::fs::rename(dir, &previous)
            .with_context(|| format!("cannot move {} aside", dir.display()))?;
    }
    std::fs::rename(&staging, dir)
        .with_context(|| format!("cannot put the mod in {}", dir.display()))?;
    remove_if_present(&previous)
}

pub fn version_in(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(MANIFEST)).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(manifest.get("version")?.as_str()?.to_string())
}

fn sibling(dir: &Path, suffix: &str) -> PathBuf {
    let mut name = dir.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{suffix}"));
    dir.with_file_name(name)
}

fn remove_if_present(dir: &Path) -> Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("cannot remove {}", dir.display()))
        }
        _ => Ok(()),
    }
}

fn create_private_dirs(dir: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("cannot create {}", dir.display()))
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut file| file.write_all(bytes))
        .with_context(|| format!("cannot write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn names() -> Vec<&'static str> {
        FILES.iter().map(|(name, _)| *name).collect()
    }

    #[test]
    fn the_bundle_holds_the_marketplace_and_the_mod_without_its_tests() {
        let names = names();
        for expected in [
            ".claude-plugin/marketplace.json",
            "mod/.claude-plugin/plugin.json",
            "mod/hooks/hooks.json",
            "mod/hooks/register.tsx",
            "mod/types/index.d.ts",
        ] {
            assert!(names.contains(&expected), "{expected} is missing");
        }
        assert!(
            names.iter().all(|name| !name.starts_with("mod/tests/")
                && !name.starts_with("mod/.claude-plugin/types/")),
            "{names:?}"
        );
    }

    #[test]
    fn every_bundled_file_is_the_one_in_the_working_tree() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        for (name, bytes) in FILES {
            assert_eq!(std::fs::read(root.join(name)).unwrap(), *bytes, "{name}");
        }
    }

    #[test]
    fn the_mod_and_the_marketplace_name_this_release() {
        let manifest = |name: &str| -> serde_json::Value {
            let (_, bytes) = FILES.iter().find(|(file, _)| *file == name).unwrap();
            serde_json::from_slice(bytes).unwrap()
        };
        assert_eq!(manifest(MANIFEST)["version"], VERSION);
        let marketplace = manifest(".claude-plugin/marketplace.json");
        assert_eq!(marketplace["name"], MARKETPLACE);
        assert_eq!(
            marketplace["plugins"][0]["name"],
            manifest(MANIFEST)["name"]
        );
        assert_eq!(marketplace["plugins"][0]["source"], "./mod");
        assert_eq!(
            PLUGIN_ID,
            format!(
                "{}@{MARKETPLACE}",
                manifest(MANIFEST)["name"].as_str().unwrap()
            )
        );
    }

    #[test]
    fn writing_the_bundle_replaces_the_folder_and_keeps_it_private() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("marketplace");
        std::fs::create_dir_all(dir.join("mod/hooks")).unwrap();
        std::fs::write(dir.join("mod/hooks/stale.ts"), "old").unwrap();
        std::fs::create_dir_all(temp.path().join("marketplace.new")).unwrap();
        write(&dir).unwrap();
        assert!(!dir.join("mod/hooks/stale.ts").exists());
        assert!(!temp.path().join("marketplace.new").exists());
        assert!(!temp.path().join("marketplace.old").exists());
        assert_eq!(version_in(&dir).as_deref(), Some(VERSION));
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.join("mod/hooks")), 0o700);
        assert_eq!(mode(&dir.join("mod/hooks/register.tsx")), 0o600);
        write(&dir).unwrap();
        assert_eq!(version_in(&dir).as_deref(), Some(VERSION));
    }

    fn with_version(dir: &Path, version: &str) {
        let manifest = dir.join(MANIFEST);
        let text = std::fs::read_to_string(&manifest).unwrap();
        let old = text.replace(
            &format!("\"version\": \"{VERSION}\""),
            &format!("\"version\": \"{version}\""),
        );
        std::fs::write(&manifest, old).unwrap();
    }

    #[test]
    fn an_older_mod_folder_is_refreshed_and_a_newer_or_missing_one_left_alone() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("marketplace");
        assert_eq!(refresh(&dir).unwrap(), None);
        assert!(!dir.exists());
        write(&dir).unwrap();
        with_version(&dir, "0.0.1");
        std::fs::write(dir.join("mod/hooks/stale.ts"), "old").unwrap();
        assert_eq!(refresh(&dir).unwrap().as_deref(), Some("0.0.1"));
        assert_eq!(version_in(&dir).as_deref(), Some(VERSION));
        assert!(!dir.join("mod/hooks/stale.ts").exists());
        assert_eq!(refresh(&dir).unwrap(), None);
        with_version(&dir, "999.0.0");
        assert_eq!(refresh(&dir).unwrap(), None);
        assert_eq!(version_in(&dir).as_deref(), Some("999.0.0"));
    }

    #[test]
    fn sessions_starting_together_refresh_the_folder_once() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("marketplace");
        write(&dir).unwrap();
        with_version(&dir, "0.0.1");
        let refreshed: Vec<Option<String>> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| refresh(&dir).unwrap()))
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect()
        });
        assert_eq!(
            refreshed.iter().filter(|r| r.is_some()).count(),
            1,
            "{refreshed:?}"
        );
        assert_eq!(version_in(&dir).as_deref(), Some(VERSION));
        assert!(dir.join("mod/hooks/register.tsx").is_file());
    }

    #[test]
    fn removing_the_folder_takes_its_lock_too() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("marketplace");
        write(&dir).unwrap();
        remove(&dir).unwrap();
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
        remove(&dir).unwrap();
    }

    #[test]
    fn a_missing_or_unreadable_manifest_has_no_version() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(version_in(temp.path()), None);
        std::fs::create_dir_all(temp.path().join("mod/.claude-plugin")).unwrap();
        std::fs::write(temp.path().join(MANIFEST), "{ not json").unwrap();
        assert_eq!(version_in(temp.path()), None);
    }
}
