use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::bundle::{MARKETPLACE, PLUGIN_ID};

pub const PROGRAM: &str = "claude";
pub const MIN_VERSION: &str = "2.1.296";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Marketplace {
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

impl Marketplace {
    pub fn location(&self) -> String {
        self.path
            .clone()
            .or_else(|| self.repo.clone())
            .or_else(|| self.url.clone())
            .unwrap_or_else(|| self.source.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Plugin {
    pub id: String,
    pub version: String,
    pub scope: String,
    pub enabled: bool,
    #[serde(default, rename = "readFromFolder")]
    pub folder: Option<String>,
    #[serde(default, rename = "folderVersion")]
    pub folder_version: Option<String>,
}

impl Plugin {
    pub fn loaded_version(&self) -> &str {
        self.folder_version.as_deref().unwrap_or(&self.version)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    AddMarketplace(PathBuf),
    Install,
    Update { from: String },
    Enable,
    Uninstall,
    RemoveMarketplace,
}

impl Step {
    pub fn args(&self) -> Vec<OsString> {
        let words: &[&str] = match self {
            Step::AddMarketplace(_) => &["plugin", "marketplace", "add"],
            Step::Install => &["plugin", "install", PLUGIN_ID],
            Step::Update { .. } => &["plugin", "update", PLUGIN_ID],
            Step::Enable => &["plugin", "enable", PLUGIN_ID],
            Step::Uninstall => &["plugin", "uninstall", PLUGIN_ID],
            Step::RemoveMarketplace => &["plugin", "marketplace", "remove", MARKETPLACE],
        };
        let mut args: Vec<OsString> = words.iter().map(OsString::from).collect();
        if let Step::AddMarketplace(path) = self {
            args.push(path.into());
        }
        args.push("--json".into());
        args
    }

    pub fn command_line(&self) -> String {
        let args = self.args();
        let shown = args[..args.len() - 1]
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned());
        std::iter::once(PROGRAM.to_string())
            .chain(shown)
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn edits_settings(&self) -> bool {
        !matches!(self, Step::Update { .. })
    }

    fn may_find_it_done(&self) -> bool {
        matches!(
            self,
            Step::Uninstall | Step::RemoveMarketplace | Step::Enable
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done(String),
    AlreadyDone(String),
}

#[derive(Deserialize)]
struct Reply {
    outcome: String,
    #[serde(default)]
    message: String,
    #[serde(default, rename = "failureCode")]
    failure_code: Option<String>,
}

const ALREADY_DONE: &[&str] = &["not_installed", "not_configured", "already_in_goal_state"];

pub fn outcome(step: &Step, stdout: &str) -> Result<Outcome> {
    let line = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    let reply: Reply = serde_json::from_str(line)
        .with_context(|| format!("{} answered {line:?}", step.command_line()))?;
    let already = reply
        .failure_code
        .as_deref()
        .is_some_and(|code| ALREADY_DONE.contains(&code));
    match reply.outcome.as_str() {
        "ok" => Ok(Outcome::Done(reply.message)),
        "failed" if already && step.may_find_it_done() => Ok(Outcome::AlreadyDone(reply.message)),
        _ => bail!("{} failed: {}", step.command_line(), reply.message),
    }
}

pub fn version_of(text: &str) -> Option<&str> {
    let version = text.split_whitespace().next()?;
    version
        .split('.')
        .all(|part| !part.is_empty())
        .then_some(version)
        .filter(|version| version.chars().next().is_some_and(|c| c.is_ascii_digit()))
}

#[derive(Debug, Clone)]
pub struct ClaudeCode {
    program: PathBuf,
}

impl Default for ClaudeCode {
    fn default() -> Self {
        Self::new(PROGRAM)
    }
}

impl ClaudeCode {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    pub fn version(&self) -> Result<Option<String>> {
        let output = match self.output(["--version"]) {
            Err(error) if is_not_found(&error) => return Ok(None),
            result => result?,
        };
        let text = String::from_utf8_lossy(&output.stdout);
        version_of(&text)
            .map(|version| Some(version.to_string()))
            .with_context(|| format!("claude --version answered {:?}", text.trim()))
    }

    pub fn marketplaces(&self) -> Result<Vec<Marketplace>> {
        self.list(&["plugin", "marketplace", "list", "--json"])
    }

    pub fn plugins(&self) -> Result<Vec<Plugin>> {
        self.list(&["plugin", "list", "--json"])
    }

    pub fn run(&self, step: &Step) -> Result<Outcome> {
        let output = self.output(step.args())?;
        outcome(step, &String::from_utf8_lossy(&output.stdout))
    }

    fn list<T: for<'de> Deserialize<'de>>(&self, args: &[&str]) -> Result<Vec<T>> {
        let output = self.output(args)?;
        let command = format!("{PROGRAM} {}", args[..args.len() - 1].join(" "));
        if !output.status.success() {
            bail!(
                "{command} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        serde_json::from_slice(&output.stdout)
            .with_context(|| format!("{command} answered something other than a list"))
    }

    fn output<I, S>(&self, args: I) -> std::io::Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        Command::new(&self.program)
            .args(args)
            .stdin(Stdio::null())
            .output()
    }
}

fn is_not_found(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_step_runs_the_claude_plugin_command_it_shows() {
        let add = Step::AddMarketplace(PathBuf::from("/home/ada/.claude-plus/marketplace"));
        assert_eq!(
            add.command_line(),
            "claude plugin marketplace add /home/ada/.claude-plus/marketplace"
        );
        assert_eq!(add.args().last().unwrap(), "--json");
        let lines: Vec<String> = [
            Step::Install,
            Step::Update {
                from: "0.0.9".into(),
            },
            Step::Enable,
            Step::Uninstall,
            Step::RemoveMarketplace,
        ]
        .iter()
        .map(Step::command_line)
        .collect();
        assert_eq!(
            lines,
            [
                "claude plugin install plus@claude-plus",
                "claude plugin update plus@claude-plus",
                "claude plugin enable plus@claude-plus",
                "claude plugin uninstall plus@claude-plus",
                "claude plugin marketplace remove claude-plus",
            ]
        );
    }

    #[test]
    fn only_an_update_leaves_the_settings_alone() {
        assert!(
            !Step::Update {
                from: "0.0.9".into()
            }
            .edits_settings()
        );
        for step in [
            Step::AddMarketplace(PathBuf::from("/m")),
            Step::Install,
            Step::Enable,
            Step::Uninstall,
            Step::RemoveMarketplace,
        ] {
            assert!(step.edits_settings(), "{step:?}");
        }
    }

    #[test]
    fn a_json_reply_is_read_from_the_last_line() {
        let stdout = "Adding marketplace…\n{\"command\":\"marketplace-add\",\"outcome\":\"ok\",\"message\":\"Successfully added marketplace: claude-plus\"}\n";
        assert_eq!(
            outcome(&Step::AddMarketplace(PathBuf::from("/m")), stdout).unwrap(),
            Outcome::Done("Successfully added marketplace: claude-plus".into())
        );
    }

    #[test]
    fn removing_what_is_already_gone_is_no_failure() {
        let gone = r#"{"command":"uninstall","outcome":"failed","message":"Plugin \"plus@claude-plus\" not found in installed plugins","failureCode":"not_installed"}"#;
        assert!(matches!(
            outcome(&Step::Uninstall, gone).unwrap(),
            Outcome::AlreadyDone(_)
        ));
        let missing = r#"{"command":"marketplace-remove","outcome":"failed","message":"Marketplace 'claude-plus' not found","failureCode":"not_configured"}"#;
        assert!(matches!(
            outcome(&Step::RemoveMarketplace, missing).unwrap(),
            Outcome::AlreadyDone(_)
        ));
        let enabled = r#"{"command":"enable","outcome":"failed","message":"Plugin \"plus@claude-plus\" is already enabled","failureCode":"already_in_goal_state"}"#;
        assert!(matches!(
            outcome(&Step::Enable, enabled).unwrap(),
            Outcome::AlreadyDone(_)
        ));
    }

    #[test]
    fn a_failed_step_names_the_command_and_the_reason() {
        let failed = r#"{"command":"install","outcome":"failed","message":"Plugin \"plus\" not found in marketplace \"claude-plus\"","failureCode":"not_found"}"#;
        assert_eq!(
            outcome(&Step::Install, failed).unwrap_err().to_string(),
            "claude plugin install plus@claude-plus failed: Plugin \"plus\" not found in marketplace \"claude-plus\""
        );
        let not_json = outcome(&Step::Install, "Segmentation fault\n").unwrap_err();
        assert_eq!(
            not_json.to_string(),
            "claude plugin install plus@claude-plus answered \"Segmentation fault\""
        );
    }

    #[test]
    fn the_version_is_the_first_word_of_claude_version() {
        assert_eq!(version_of("2.1.296 (Claude Code)\n"), Some("2.1.296"));
        assert_eq!(
            version_of("2.2.0-beta.1 (Claude Code)"),
            Some("2.2.0-beta.1")
        );
        assert_eq!(version_of(""), None);
        assert_eq!(version_of("Claude Code"), None);
    }

    #[test]
    fn lists_read_what_claude_plugin_list_prints() {
        let plugins: Vec<Plugin> = serde_json::from_str(
            r#"[{"id":"plus@claude-plus","version":"0.1.0","scope":"user","enabled":true,"installPath":"/c","readFromFolder":"/m/mod","folderVersion":"0.1.0"}]"#,
        )
        .unwrap();
        assert_eq!(
            plugins,
            [Plugin {
                id: "plus@claude-plus".into(),
                version: "0.1.0".into(),
                scope: "user".into(),
                enabled: true,
                folder: Some("/m/mod".into()),
                folder_version: Some("0.1.0".into()),
            }]
        );
        let marketplaces: Vec<Marketplace> = serde_json::from_str(
            r#"[{"name":"claude-plus","source":"directory","path":"/m","installLocation":"/m"},{"name":"other","source":"github","repo":"ada/other"}]"#,
        )
        .unwrap();
        assert_eq!(marketplaces[0].location(), "/m");
        assert_eq!(marketplaces[1].location(), "ada/other");
    }
}
