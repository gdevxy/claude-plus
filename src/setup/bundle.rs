use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rustix::fs::{FlockOperation, flock};

use crate::daemon::control::is_older;

include!(concat!(env!("OUT_DIR"), "/bundle.rs"));

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MARKETPLACE: &str = "claude-plus";
pub const PLUGIN_ID: &str = "plus@claude-plus";
const MANIFEST: &str = "mod/.claude-plugin/plugin.json";
const MARKETPLACE_MANIFEST: &str = ".claude-plugin/marketplace.json";
const HASH_FILE: &str = ".bundle-hash";

pub fn write(dir: &Path) -> Result<()> {
    let _lock = lock(dir)?;
    replace(dir)
}

pub fn refresh(dir: &Path) -> Result<Option<String>> {
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

fn outdated(dir: &Path) -> Option<String> {
    match version_in(dir) {
        Some(installed) if installed == VERSION => {
            let hash = std::fs::read_to_string(dir.join(HASH_FILE)).ok();
            (hash.as_deref() != Some(bundle_hash().as_str())).then_some(installed)
        }
        Some(installed) => is_older(&installed, VERSION).then_some(installed),
        None => (!dir.exists())
            .then(|| version_in(&sibling(dir, "old")))
            .flatten(),
    }
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
    if !dir.exists() && !sibling(dir, "lock").exists() {
        return Ok(());
    }
    let _lock = lock(dir)?;
    check_ours(dir)?;
    for path in [dir.to_path_buf(), sibling(dir, "new"), sibling(dir, "old")] {
        remove_if_present(&path)?;
    }
    match std::fs::remove_file(sibling(dir, "lock")) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

fn check_ours(dir: &Path) -> Result<()> {
    let named_ours = || -> Option<bool> {
        let text = std::fs::read_to_string(dir.join(MARKETPLACE_MANIFEST)).ok()?;
        let manifest: serde_json::Value = serde_json::from_str(&text).ok()?;
        Some(manifest.get("name")?.as_str()? == MARKETPLACE)
    };
    if std::fs::symlink_metadata(dir).is_err() || named_ours() == Some(true) {
        return Ok(());
    }
    bail!(
        "{} holds something that is not the claude-plus marketplace; move it away, then try again",
        dir.display()
    )
}

fn replace(dir: &Path) -> Result<()> {
    check_ours(dir)?;
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
    write_private(&staging.join(HASH_FILE), bundle_hash().as_bytes())?;
    match swap(&staging, dir) {
        Ok(()) => remove_if_present(&staging),
        Err(error) if error == rustix::io::Errno::NOENT => std::fs::rename(&staging, dir)
            .with_context(|| format!("cannot put the mod in {}", dir.display())),
        Err(error) if error == rustix::io::Errno::INVAL || error == rustix::io::Errno::NOTSUP => {
            std::fs::rename(dir, &previous)
                .with_context(|| format!("cannot move {} aside", dir.display()))?;
            std::fs::rename(&staging, dir)
                .with_context(|| format!("cannot put the mod in {}", dir.display()))?;
            remove_if_present(&previous)
        }
        Err(error) => {
            Err(error).with_context(|| format!("cannot put the mod in {}", dir.display()))
        }
    }
}

fn swap(staging: &Path, dir: &Path) -> rustix::io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    renameat_with(CWD, staging, CWD, dir, RenameFlags::EXCHANGE)
}

fn bundle_hash() -> String {
    hash_of(FILES)
}

fn hash_of(files: &[(&str, &[u8])]) -> String {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for (name, bytes) in files {
        for part in [name.as_bytes(), &(bytes.len() as u64).to_le_bytes(), bytes] {
            for byte in part {
                hash = (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME);
            }
        }
        hash = (hash ^ 0xff).wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
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
        write(&dir).unwrap();
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
    fn a_folder_of_this_version_from_another_build_is_refreshed() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("marketplace");
        write(&dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(HASH_FILE)).unwrap(),
            bundle_hash()
        );
        std::fs::write(dir.join(HASH_FILE), "another build").unwrap();
        assert_eq!(refresh(&dir).unwrap().as_deref(), Some(VERSION));
        assert_eq!(refresh(&dir).unwrap(), None);
        std::fs::remove_file(dir.join(HASH_FILE)).unwrap();
        assert_eq!(refresh(&dir).unwrap().as_deref(), Some(VERSION));
        with_version(&dir, "999.0.0");
        std::fs::write(dir.join(HASH_FILE), "another build").unwrap();
        assert_eq!(refresh(&dir).unwrap(), None);
    }

    #[test]
    fn the_bundle_hash_names_every_file_and_its_bytes() {
        assert_eq!(bundle_hash().len(), 16);
        assert_eq!(bundle_hash(), bundle_hash());
        assert_ne!(hash_of(&[("a", b"bc")]), hash_of(&[("ab", b"c")]));
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
    fn a_reader_never_finds_the_folder_missing_while_it_is_rewritten() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("marketplace");
        write(&dir).unwrap();
        let probed = dir.join(".claude-plugin/marketplace.json");
        let done = std::sync::atomic::AtomicBool::new(false);
        let (probes, misses) = std::thread::scope(|scope| {
            let reader = scope.spawn(|| {
                let (mut probes, mut misses) = (0u64, 0u64);
                while !done.load(std::sync::atomic::Ordering::Relaxed) {
                    probes += 1;
                    if !probed.is_file() {
                        misses += 1;
                    }
                }
                (probes, misses)
            });
            for _ in 0..40 {
                write(&dir).unwrap();
            }
            done.store(true, std::sync::atomic::Ordering::Relaxed);
            reader.join().unwrap()
        });
        assert!(probes > 0);
        assert_eq!(misses, 0, "missing in {misses} of {probes} probes");
        assert_eq!(version_in(&dir).as_deref(), Some(VERSION));
    }

    #[test]
    fn a_folder_lost_between_two_renames_is_rebuilt() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("marketplace");
        write(&dir).unwrap();
        with_version(&dir, "0.0.1");
        std::fs::rename(&dir, temp.path().join("marketplace.old")).unwrap();
        assert_eq!(refresh(&dir).unwrap().as_deref(), Some("0.0.1"));
        assert_eq!(version_in(&dir).as_deref(), Some(VERSION));
        assert!(!temp.path().join("marketplace.old").exists());
    }

    #[test]
    fn a_folder_of_that_name_that_is_not_ours_is_never_replaced_or_removed() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("marketplace");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("precious"), "keep").unwrap();
        let refused = format!(
            "{} holds something that is not the claude-plus marketplace; move it away, then try again",
            dir.display()
        );
        assert_eq!(write(&dir).unwrap_err().to_string(), refused);
        assert_eq!(remove(&dir).unwrap_err().to_string(), refused);
        assert!(dir.join("precious").is_file());
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
