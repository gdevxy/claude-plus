use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::daemon::{ensured, stopped};
use crate::daemon::control::{self, is_older};
use crate::paths::{MARKER, Paths};
use crate::setup::bundle::{self, MARKETPLACE, PLUGIN_ID};
use crate::setup::claude::{self, ClaudeCode, Outcome, Step};
use crate::setup::{Found, install_steps, uninstall_steps};

const BINARY: &str = "claude-plus";

macro_rules! say {
    ($($arg:tt)*) => {{
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

pub fn install(paths: &Paths, yes: bool) -> Result<()> {
    let claude = ClaudeCode::default();
    let Some(version) = claude.version()? else {
        bail!(
            "Claude Code was not found on PATH; install it first, then run claude-plus install again"
        );
    };
    if is_older(&version, claude::MIN_VERSION) {
        bail!(
            "Claude Code {version} is older than {}, which the plus mod needs; run claude update, then claude-plus install again",
            claude::MIN_VERSION
        );
    }
    let dir = paths.marketplace_dir();
    let found = found(&claude)?;
    let newer = newer_installed(&dir, &found);
    let target = newer.as_deref().unwrap_or(bundle::VERSION);
    let steps = install_steps(&found, &dir, target);
    confirm_settings(paths, "install", &steps, &[], yes)?;
    paths.ensure_data_dir()?;
    if newer.is_none() {
        bundle::write(&dir)?;
    }
    for step in &steps {
        let outcome = claude.run(step)?;
        say!("{}", installed(step, &found, &outcome, target));
    }
    if let Some(newer) = &newer {
        say!(
            "✻ Kept the plus mod v{newer}, newer than this claude-plus v{}",
            bundle::VERSION
        );
    } else if steps
        .iter()
        .all(|step| matches!(step, Step::AddMarketplace(_)))
    {
        say!("✻ The plus mod v{} is installed", bundle::VERSION);
    }
    for line in ensured(&control::ensure(paths)?) {
        say!("{line}");
    }
    if let Some(warning) = path_warning(&std::env::current_exe()?, std::env::var_os("PATH")) {
        say!("{warning}");
    }
    say!(
        "✻ Start a new Claude Code session to see the pane: it docks beside the conversation from 144 columns, and /plus opens it at any width"
    );
    Ok(())
}

pub fn uninstall(paths: &Paths, purge: bool, yes: bool) -> Result<()> {
    if purge && let Some(refusal) = purge_refusal(paths) {
        bail!(refusal);
    }
    let claude = ClaudeCode::default();
    let present = claude.version()?.is_some();
    let found = if present {
        found(&claude)
            .inspect_err(|error| eprintln!("claude-plus: {error:#}"))
            .ok()
    } else {
        None
    };
    let steps = found.as_ref().map(uninstall_steps).unwrap_or_default();
    let deleted = if purge {
        vec![format!(
            "delete {} with the history claude-plus keeps",
            paths.data_dir.display()
        )]
    } else {
        Vec::new()
    };
    confirm_settings(paths, "uninstall", &steps, &deleted, yes)?;
    for step in &steps {
        let outcome = claude.run(step)?;
        say!("{}", uninstalled(step, &outcome));
    }
    if !present {
        say!(
            "Claude Code was not found on PATH, so the plus mod may still be installed: run claude plugin uninstall {PLUGIN_ID} where it is"
        );
    } else if found.is_none() {
        say!(
            "Claude Code could not say what it has installed, so the plus mod may still be installed: run claude plugin uninstall {PLUGIN_ID} and claude plugin marketplace remove {MARKETPLACE}"
        );
    }
    say!("{}", stopped(control::stop(paths)?));
    bundle::remove(&paths.marketplace_dir())?;
    if purge && let Some(left) = purge_data_dir(paths)? {
        say!("{}", purged(&paths.data_dir, &left));
    }
    say!(
        "✻ claude-plus is uninstalled · close open Claude Code sessions, or their pane starts the daemon again"
    );
    Ok(())
}

fn newer_installed(dir: &Path, found: &Found) -> Option<String> {
    [
        bundle::version_in(dir),
        found.plugin().map(|plugin| plugin.version.clone()),
    ]
    .into_iter()
    .flatten()
    .filter(|installed| is_older(bundle::VERSION, installed))
    .reduce(|a, b| if is_older(&a, &b) { b } else { a })
}

fn found(claude: &ClaudeCode) -> Result<Found> {
    Ok(Found {
        marketplaces: claude.marketplaces()?,
        plugins: claude.plugins()?,
    })
}

fn confirm_settings(
    paths: &Paths,
    command: &str,
    steps: &[Step],
    also: &[String],
    yes: bool,
) -> Result<()> {
    let edits = steps.iter().any(Step::edits_settings);
    if yes || (!edits && also.is_empty()) {
        return Ok(());
    }
    let mut lines = vec![format!("claude-plus {command} will:")];
    for step in steps {
        lines.push(format!("  run {}", step.command_line()));
    }
    if edits {
        lines.push(format!(
            "  which Claude Code records in {}",
            paths.settings_file().display()
        ));
    }
    lines.extend(also.iter().map(|line| format!("  {line}")));
    let stdin = std::io::stdin();
    let typed = stdin.is_terminal();
    if !confirm(&lines, &mut stdin.lock(), &mut std::io::stderr(), typed)? {
        bail!(
            "{command} cancelled, claude-plus changed nothing; claude-plus {command} --yes agrees without asking"
        );
    }
    Ok(())
}

pub fn confirm(
    lines: &[String],
    input: &mut impl BufRead,
    output: &mut impl Write,
    typed: bool,
) -> Result<bool> {
    for line in lines {
        writeln!(output, "{line}")?;
    }
    write!(output, "Continue? [y/N] ")?;
    output.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    if !typed {
        writeln!(output, "{}", answer.trim())?;
    }
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

fn installed(step: &Step, found: &Found, outcome: &Outcome, version: &str) -> String {
    match step {
        Step::AddMarketplace(dir) => match found.marketplace() {
            Some(previous) => format!(
                "✻ Pointed the claude-plus marketplace at {} (was {})",
                dir.display(),
                previous.location()
            ),
            None => format!("✻ Added the claude-plus marketplace · {}", dir.display()),
        },
        Step::Install => format!("✻ Installed the plus mod v{version}"),
        Step::Update { from } => format!("✻ Updated the plus mod from v{from} to v{version}"),
        Step::Enable => "✻ Enabled the plus mod".into(),
        Step::Uninstall | Step::RemoveMarketplace => said(outcome),
    }
}

fn uninstalled(step: &Step, outcome: &Outcome) -> String {
    match (step, outcome) {
        (Step::Uninstall, Outcome::Done(_)) => "✻ Uninstalled the plus mod".into(),
        (Step::RemoveMarketplace, Outcome::Done(_)) => {
            "✻ Removed the claude-plus marketplace".into()
        }
        _ => said(outcome),
    }
}

fn said(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Done(message) | Outcome::AlreadyDone(message) => format!("  {message}"),
    }
}

const DATA_FILES: &[&str] = &[
    "data.db",
    "data.db-wal",
    "data.db-shm",
    "data.db-journal",
    "daemon.sock",
    "daemon.lock",
    "daemon.log",
    "daemon.log.1",
];

fn purge_data_dir(paths: &Paths) -> Result<Option<Vec<String>>> {
    if !paths.marker().exists() {
        return Ok(None);
    }
    for name in DATA_FILES {
        let path = paths.data_dir.join(name);
        match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(error).with_context(|| format!("cannot delete {}", path.display()));
            }
            _ => {}
        }
    }
    let mut left: Vec<String> = std::fs::read_dir(&paths.data_dir)
        .with_context(|| format!("cannot read {}", paths.data_dir.display()))?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<_>>()?;
    left.retain(|name| name != MARKER);
    left.sort();
    if left.is_empty() {
        std::fs::remove_file(paths.marker())
            .and_then(|()| std::fs::remove_dir(&paths.data_dir))
            .with_context(|| format!("cannot delete {}", paths.data_dir.display()))?;
    }
    Ok(Some(left))
}

fn purged(data_dir: &Path, left: &[String]) -> String {
    if left.is_empty() {
        format!("✻ Deleted {}", data_dir.display())
    } else {
        format!(
            "✻ Deleted what claude-plus keeps in {} and left {}, which it did not write",
            data_dir.display(),
            left.join(", ")
        )
    }
}

fn purge_refusal(paths: &Paths) -> Option<String> {
    let data = &paths.data_dir;
    let refuse = |why: String| Some(format!("refusing to purge {}: {why}", data.display()));
    let meta = match std::fs::symlink_metadata(data) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => return refuse(format!("cannot read it: {error}")),
    };
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(data).unwrap_or_default();
        return refuse(format!(
            "it is a symlink to {}; set CLAUDE_PLUS_HOME to that directory to purge it",
            target.display()
        ));
    }
    if !meta.is_dir() {
        return refuse("it is not a directory".into());
    }
    let canonical = |dir: &Path| dir.canonicalize().unwrap_or(dir.to_path_buf());
    let real = canonical(data);
    if canonical(&paths.home).starts_with(&real) {
        return refuse(format!(
            "it holds your home directory {}",
            paths.home.display()
        ));
    }
    for claude_dir in [paths.claude_dir.clone(), paths.home.join(".claude")] {
        let guarded = canonical(&claude_dir);
        if guarded.starts_with(&real) {
            return refuse(format!(
                "it holds {}, which Claude Code keeps",
                claude_dir.display()
            ));
        }
        if real.starts_with(&guarded) {
            return refuse(format!(
                "it is inside {}, which Claude Code keeps",
                claude_dir.display()
            ));
        }
    }
    if !paths.marker().is_file() {
        return refuse(format!(
            "claude-plus did not create it, as it has no {MARKER} file"
        ));
    }
    None
}

pub fn path_warning(exe: &Path, path_var: Option<std::ffi::OsString>) -> Option<String> {
    let exe = exe.canonicalize().unwrap_or(exe.to_path_buf());
    let found = path_var
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(BINARY))
        .find(|candidate| is_executable(candidate));
    match found {
        Some(found) if found.canonicalize().ok().as_ref() == Some(&exe) => None,
        Some(found) => Some(format!(
            "The pane starts {BINARY} from PATH, which finds {} and not this binary ({}); set binaryPath with /plugin configure {PLUGIN_ID} if that is not the one you want",
            found.display(),
            exe.display()
        )),
        None => Some(format!(
            "{BINARY} is not on PATH, where the pane looks for it: add {} to PATH, or set binaryPath to {} with /plugin configure {PLUGIN_ID}",
            exe.parent()
                .map(PathBuf::from)
                .unwrap_or_default()
                .display(),
            exe.display()
        )),
    }
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_yes_agrees() {
        for (answer, agreed) in [
            ("y\n", true),
            ("YES\n", true),
            ("n\n", false),
            ("\n", false),
            ("", false),
            ("maybe\n", false),
        ] {
            let mut output = Vec::new();
            let lines = ["claude-plus install will:".to_string()];
            assert_eq!(
                confirm(&lines, &mut answer.as_bytes(), &mut output, false).unwrap(),
                agreed,
                "{answer:?}"
            );
            assert_eq!(
                String::from_utf8(output).unwrap(),
                format!(
                    "claude-plus install will:\nContinue? [y/N] {}\n",
                    answer.trim()
                )
            );
        }
    }

    #[test]
    fn purging_never_reaches_home_or_the_claude_dir() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        for dir in [
            ".claude/projects",
            ".claude-plus",
            "a/claude",
            "c/projects",
            "x",
        ] {
            std::fs::create_dir_all(home.join(dir)).unwrap();
        }
        let refused = |plus: PathBuf| purge_refusal(&Paths::resolve(home, None, Some(plus)));
        let ours = Paths::resolve(home, None, None);
        ours.ensure_data_dir().unwrap();
        assert_eq!(purge_refusal(&ours), None);
        assert_eq!(refused(home.join("missing")), None);
        assert!(
            refused(home.to_path_buf())
                .unwrap()
                .contains("home directory")
        );
        assert!(
            refused(home.join("x/.."))
                .unwrap()
                .contains("home directory")
        );
        assert!(
            refused(home.join(".claude/.."))
                .unwrap()
                .contains("home directory")
        );
        assert!(
            refused(PathBuf::from("/"))
                .unwrap()
                .contains("home directory")
        );
        assert!(
            refused(home.parent().unwrap().to_path_buf())
                .unwrap()
                .contains("home directory")
        );
        assert!(refused(home.join(".claude")).unwrap().contains("holds"));
        assert!(
            refused(home.join(".claude/projects"))
                .unwrap()
                .contains("inside")
        );
        let custom = |plus: &str| {
            purge_refusal(&Paths::resolve(
                home,
                Some(home.join("c")),
                Some(home.join(plus)),
            ))
        };
        assert!(custom("c/projects").unwrap().contains("inside"));
        assert!(custom(".claude/projects").unwrap().contains("inside"));
        assert!(
            purge_refusal(&Paths::resolve(
                home,
                Some(home.join("a/claude")),
                Some(home.join("a"))
            ))
            .unwrap()
            .contains("holds")
        );
    }

    #[test]
    fn the_pane_finds_this_binary_on_path_or_says_how() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        let other = temp.path().join("other");
        for dir in [&bin, &other] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(BINARY), "").unwrap();
            std::fs::set_permissions(dir.join(BINARY), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        let exe = bin.join(BINARY);
        let path = |dirs: &[&Path]| Some(std::env::join_paths(dirs).unwrap());
        assert_eq!(path_warning(&exe, path(&[&bin])), None);
        let shadowed = path_warning(&exe, path(&[&other, &bin])).unwrap();
        assert!(
            shadowed.starts_with("The pane starts claude-plus from PATH, which finds"),
            "{shadowed}"
        );
        let missing = path_warning(&exe, path(&[temp.path()])).unwrap();
        assert!(
            missing.starts_with("claude-plus is not on PATH, where the pane looks for it: add"),
            "{missing}"
        );
        assert!(
            missing.ends_with("with /plugin configure plus@claude-plus"),
            "{missing}"
        );
    }
}
