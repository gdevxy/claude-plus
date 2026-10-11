use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::daemon::{ensured, stopped};
use crate::daemon::control::{self, is_older};
use crate::paths::Paths;
use crate::setup::bundle::{self, PLUGIN_ID};
use crate::setup::claude::{self, ClaudeCode, Outcome, Step};
use crate::setup::{Found, install_steps, uninstall_steps};

const BINARY: &str = "claude-plus";

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
    let steps = install_steps(&found, &dir, bundle::VERSION);
    confirm_settings(paths, "install", &steps, &[], yes)?;
    paths.ensure_data_dir()?;
    bundle::write(&dir)?;
    for step in &steps {
        let outcome = claude.run(step)?;
        println!("{}", installed(step, &found, &outcome));
    }
    if steps
        .iter()
        .all(|step| matches!(step, Step::AddMarketplace(_)))
    {
        println!("✻ The plus mod v{} is installed", bundle::VERSION);
    }
    for line in ensured(&control::ensure(paths)?) {
        println!("{line}");
    }
    if let Some(warning) = path_warning(&std::env::current_exe()?, std::env::var_os("PATH")) {
        println!("{warning}");
    }
    println!(
        "✻ Start a new Claude Code session to see the pane: it docks beside the conversation from 144 columns, and /plus opens it at any width"
    );
    Ok(())
}

pub fn uninstall(paths: &Paths, purge: bool, yes: bool) -> Result<()> {
    if purge && let Some(refusal) = purge_refusal(paths) {
        bail!(refusal);
    }
    let claude = ClaudeCode::default();
    let found = match claude.version()? {
        Some(_) => Some(found(&claude)?),
        None => None,
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
        println!("{}", uninstalled(step, &outcome));
    }
    if found.is_none() {
        println!(
            "Claude Code was not found on PATH, so the plus mod may still be installed: run claude plugin uninstall {PLUGIN_ID} where it is"
        );
    }
    println!("{}", stopped(control::stop(paths)?));
    bundle::remove(&paths.marketplace_dir())?;
    if purge {
        remove_dir(&paths.data_dir)?;
        println!("✻ Deleted {}", paths.data_dir.display());
    }
    println!(
        "✻ claude-plus is uninstalled · close open Claude Code sessions, or their pane starts the daemon again"
    );
    Ok(())
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
    if !confirm(&lines, &mut stdin.lock(), &mut std::io::stdout(), typed)? {
        bail!(
            "{command} cancelled, nothing was changed; claude-plus {command} --yes agrees without asking"
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

fn installed(step: &Step, found: &Found, outcome: &Outcome) -> String {
    match step {
        Step::AddMarketplace(dir) => match found.marketplace() {
            Some(previous) => format!(
                "✻ Pointed the claude-plus marketplace at {} (was {})",
                dir.display(),
                previous.location()
            ),
            None => format!("✻ Added the claude-plus marketplace · {}", dir.display()),
        },
        Step::Install => format!("✻ Installed the plus mod v{}", bundle::VERSION),
        Step::Update { from } => {
            format!(
                "✻ Updated the plus mod from v{from} to v{}",
                bundle::VERSION
            )
        }
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

fn remove_dir(dir: &Path) -> Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("cannot delete {}", dir.display()))
        }
        _ => Ok(()),
    }
}

fn purge_refusal(paths: &Paths) -> Option<String> {
    let data = paths
        .data_dir
        .canonicalize()
        .unwrap_or(paths.data_dir.clone());
    let guarded = [
        paths.home.as_path(),
        paths.claude_dir.as_path(),
        Path::new("/"),
    ];
    guarded
        .iter()
        .map(|dir| dir.canonicalize().unwrap_or(dir.to_path_buf()))
        .any(|dir| dir.starts_with(&data))
        .then(|| {
            format!(
                "refusing to purge {}: it holds {} or your home directory",
                paths.data_dir.display(),
                paths.claude_dir.display()
            )
        })
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
        let refused = |plus: PathBuf| purge_refusal(&Paths::resolve(home, None, Some(plus)));
        assert_eq!(refused(home.join(".claude-plus")), None);
        assert!(refused(home.to_path_buf()).is_some());
        assert!(refused(PathBuf::from("/")).is_some());
        assert!(refused(home.join(".claude")).is_some());
        assert!(
            purge_refusal(&Paths::resolve(
                home,
                Some(home.join("a/claude")),
                Some(home.join("a"))
            ))
            .is_some()
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
