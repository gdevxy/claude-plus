use std::collections::BTreeMap;

pub const SHOWN_TARGET_CHARS: usize = 30;
pub const FILE_READERS: &[&str] = &[
    "cat", "sed", "head", "tail", "awk", "less", "more", "nl", "bat", "jq", "wc", "grep", "rg",
];
pub const COMMAND_TOOLS: &[&str] = &["Bash", "PowerShell"];
const PROGRAM_PREFIXES: &[&str] = &["sudo", "time", "nohup", "env", "command", "exec"];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    File {
        tool: String,
        path: String,
    },
    Command {
        tool: String,
        program: String,
        argument: Option<Argument>,
    },
    Other {
        tool: String,
        target: String,
    },
    Unnamed {
        tool: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Argument {
    Path(String),
    Word(String),
}

impl Source {
    pub fn of(tool: Option<&str>, target: Option<&str>, cwd: Option<&str>) -> Self {
        let tool = tool.map_or_else(|| "A tool".to_string(), tool_name);
        let Some(target) = target.map(str::trim).filter(|t| !t.is_empty()) else {
            return Source::Unnamed { tool };
        };
        if COMMAND_TOOLS.contains(&tool.as_str()) {
            return command(&tool, target, cwd);
        }
        if is_path(target) && !target.contains(char::is_whitespace) {
            return Source::File {
                tool,
                path: relative(target, None, cwd),
            };
        }
        Source::Other {
            tool,
            target: target.lines().next().unwrap_or("").trim().to_string(),
        }
    }

    pub fn tool(&self) -> &str {
        match self {
            Source::File { tool, .. }
            | Source::Command { tool, .. }
            | Source::Other { tool, .. }
            | Source::Unnamed { tool } => tool,
        }
    }

    pub fn file(&self) -> Option<&str> {
        match self {
            Source::File { path, .. } => Some(path),
            Source::Command {
                program,
                argument: Some(Argument::Path(path)),
                ..
            } if FILE_READERS.contains(&program.as_str()) => Some(path),
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Source::File { tool, path } => format!("{tool} {}", tail(path, SHOWN_TARGET_CHARS)),
            Source::Command {
                tool,
                program,
                argument,
            } => {
                let room = SHOWN_TARGET_CHARS.saturating_sub(program.chars().count() + 1);
                match argument {
                    None => format!("{tool} {program}"),
                    Some(Argument::Path(path)) => format!("{tool} {program} {}", tail(path, room)),
                    Some(Argument::Word(word)) => format!("{tool} {program} {}", head(word, room)),
                }
            }
            Source::Other { tool, target } => {
                format!("{tool} {}", head(target, SHOWN_TARGET_CHARS))
            }
            Source::Unnamed { tool } if COMMAND_TOOLS.contains(&tool.as_str()) => {
                format!("{tool} commands too long to name")
            }
            Source::Unnamed { tool } => tool.clone(),
        }
    }
}

pub fn tool_name(tool: &str) -> String {
    let Some((server, name)) = tool
        .strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
    else {
        return tool.to_string();
    };
    let server = server
        .strip_prefix("plugin_")
        .and_then(|rest| rest.split_once('_'))
        .map_or(server, |(_, server)| server);
    match is_uuid(server) {
        true => format!("MCP {name}"),
        false => format!("{server} {name}"),
    }
}

fn is_uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.iter().map(|g| g.len()).eq([8, 4, 4, 4, 12])
        && groups
            .iter()
            .all(|g| g.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn command(tool: &str, command: &str, cwd: Option<&str>) -> Source {
    let mut dir: Option<String> = None;
    let mut vars: BTreeMap<String, String> = BTreeMap::new();
    for statement in statements(command) {
        let mut words = statement.into_iter().peekable();
        while let Some((name, value)) = words.peek().and_then(|w| assignment(w)) {
            vars.insert(name, value);
            words.next();
        }
        let mut words: Vec<String> = words.map(|w| expanded(&w, &vars)).collect();
        while words
            .first()
            .is_some_and(|w| PROGRAM_PREFIXES.contains(&basename(w)) || assignment(w).is_some())
        {
            words.remove(0);
        }
        let Some(program) = words.first() else {
            continue;
        };
        let program = basename(program).to_string();
        match program.as_str() {
            "cd" | "pushd" => {
                if let Some(to) = words.get(1) {
                    dir = Some(joined(to, dir.as_deref()));
                }
                continue;
            }
            "export" | "set" | "source" | "." | "exit" | "true" => continue,
            _ => {}
        }
        let arguments: Vec<&String> = words[1..].iter().filter(|w| !w.starts_with('-')).collect();
        let argument = arguments
            .iter()
            .find(|w| is_path(w))
            .map(|path| Argument::Path(relative(path, dir.as_deref(), cwd)))
            .or_else(|| {
                arguments
                    .iter()
                    .find(|w| !w.contains(char::is_whitespace) && !w.is_empty())
                    .map(|word| Argument::Word(word.to_string()))
            });
        return Source::Command {
            tool: tool.to_string(),
            program,
            argument,
        };
    }
    Source::Unnamed {
        tool: tool.to_string(),
    }
}

fn basename(program: &str) -> &str {
    program.rsplit('/').next().unwrap_or(program)
}

fn statements(command: &str) -> Vec<Vec<String>> {
    let mut statements = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut chars = command.chars().peekable();
    let mut quote: Option<char> = None;
    let end_word = |word: &mut String, quoted: &mut bool, words: &mut Vec<String>| {
        if !word.is_empty() || *quoted {
            words.push(std::mem::take(word));
        }
        *quoted = false;
    };
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some(open), ch) if ch == open => quote = None,
            (Some('"'), '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            }
            (Some(_), ch) => word.push(ch),
            (None, '\'' | '"') => {
                quote = Some(ch);
                quoted = true;
            }
            (None, '\\') => {
                if let Some(next) = chars.next().filter(|c| *c != '\n') {
                    word.push(next);
                }
            }
            (None, ';' | '&' | '|' | '\n' | '(' | ')') => {
                end_word(&mut word, &mut quoted, &mut words);
                if !words.is_empty() {
                    statements.push(std::mem::take(&mut words));
                }
            }
            (None, ch) if ch.is_whitespace() => end_word(&mut word, &mut quoted, &mut words),
            (None, ch) => word.push(ch),
        }
    }
    end_word(&mut word, &mut quoted, &mut words);
    if !words.is_empty() {
        statements.push(words);
    }
    statements
}

fn assignment(word: &str) -> Option<(String, String)> {
    let (name, value) = word.split_once('=')?;
    let valid = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    valid.then(|| (name.to_string(), value.to_string()))
}

fn expanded(word: &str, vars: &BTreeMap<String, String>) -> String {
    let mut out = word.to_string();
    for (name, value) in vars.iter().rev() {
        out = out
            .replace(&format!("${{{name}}}"), value)
            .replace(&format!("${name}"), value);
    }
    out
}

fn is_path(word: &str) -> bool {
    if word.contains("://") || word.contains('/') {
        return true;
    }
    word.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && (1..=5).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
            && ext.chars().any(|c| c.is_ascii_alphabetic())
    })
}

fn joined(path: &str, dir: Option<&str>) -> String {
    match dir {
        Some(dir) if !path.starts_with('/') && !path.starts_with('~') && !path.contains("://") => {
            let dir = dir.trim_end_matches('/');
            match path.strip_prefix("./") {
                Some(rest) => format!("{dir}/{rest}"),
                None if path == "." => dir.to_string(),
                None => format!("{dir}/{path}"),
            }
        }
        _ => path.to_string(),
    }
}

fn relative(path: &str, dir: Option<&str>, cwd: Option<&str>) -> String {
    let full = joined(path, dir);
    let Some(cwd) = cwd.map(|c| c.trim_end_matches('/')) else {
        return full;
    };
    match full.strip_prefix(cwd) {
        Some("") => ".".into(),
        Some(rest) if rest.starts_with('/') => rest[1..].to_string(),
        _ => full,
    }
}

fn head(text: &str, room: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= room {
        return text.to_string();
    }
    let kept: String = chars[..room.saturating_sub(1)].iter().collect();
    format!("{}…", kept.trim_end())
}

fn tail(text: &str, room: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= room {
        return text.to_string();
    }
    let kept: String = chars[chars.len() - room.saturating_sub(1)..]
        .iter()
        .collect();
    format!("…{kept}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CWD: Option<&str> = Some("/work/demo");

    fn bash(command: &str) -> Source {
        Source::of(Some("Bash"), Some(command), CWD)
    }

    fn named(program: &str, argument: Option<Argument>) -> Source {
        Source::Command {
            tool: "Bash".into(),
            program: program.into(),
            argument,
        }
    }

    fn path(path: &str) -> Option<Argument> {
        Some(Argument::Path(path.into()))
    }

    #[test]
    fn a_command_is_named_by_its_program_and_first_meaningful_argument() {
        assert_eq!(
            bash("cargo test --workspace -- --nocapture"),
            named("cargo", Some(Argument::Word("test".into())))
        );
        assert_eq!(
            bash("sed -n '20,60p' docs/plan.md | cut -c1-600"),
            named("sed", path("docs/plan.md"))
        );
        assert_eq!(
            bash("grep -n \"Ruling\" progress.md"),
            named("grep", path("progress.md"))
        );
        assert_eq!(bash("ls"), named("ls", None));
        assert_eq!(
            bash("/usr/bin/env FOO=1 python3 script.py"),
            named("python3", path("script.py"))
        );
    }

    #[test]
    fn a_leading_cd_is_dropped_and_resolves_the_paths_after_it() {
        let a = bash("cd /work/demo/sdd/open-edition && sed -n 1,80p progress.md");
        let b = bash("cd /work/demo/sdd/open-edition; cat lane-rules.md");
        let c = bash("cd /work/demo/backend || exit 1; cat docs/dev/backend.md");
        assert_eq!(a, named("sed", path("sdd/open-edition/progress.md")));
        assert_eq!(b, named("cat", path("sdd/open-edition/lane-rules.md")));
        assert_eq!(c, named("cat", path("backend/docs/dev/backend.md")));
        assert_eq!(
            bash("cd /work/demo/sdd && sed -n 200,300p ./open-edition/progress.md"),
            a
        );
    }

    #[test]
    fn variables_set_before_the_command_are_expanded() {
        assert_eq!(
            bash("D=sdd/open-edition; sed -n '20,60p' $D/brief.md | cut -c1-600; ls $D"),
            named("sed", path("sdd/open-edition/brief.md"))
        );
        assert_eq!(
            bash("M=\"docs/plan.md\" && sed -n '1543,1680p' ${M}"),
            named("sed", path("docs/plan.md"))
        );
    }

    #[test]
    fn file_tools_are_named_by_their_path_relative_to_the_project() {
        assert_eq!(
            Source::of(Some("Read"), Some("/work/demo/src/big.rs"), CWD),
            Source::File {
                tool: "Read".into(),
                path: "src/big.rs".into()
            }
        );
        assert_eq!(
            Source::of(Some("Grep"), Some("fn main"), CWD),
            Source::Other {
                tool: "Grep".into(),
                target: "fn main".into()
            }
        );
        assert_eq!(
            Source::of(Some("Bash"), None, CWD),
            Source::Unnamed {
                tool: "Bash".into()
            }
        );
        assert_eq!(
            Source::of(None, None, CWD),
            Source::Unnamed {
                tool: "A tool".into()
            }
        );
    }

    #[test]
    fn mcp_tools_are_named_by_server_and_tool() {
        assert_eq!(tool_name("mcp__linear__search"), "linear search");
        assert_eq!(
            tool_name("mcp__plugin_atlassian_atlassian__getJiraIssue"),
            "atlassian getJiraIssue"
        );
        assert_eq!(
            tool_name("mcp__6d6763aa-971c-43fc-9893-9e003ffc424d__updateConfluenceContent"),
            "MCP updateConfluenceContent"
        );
        assert_eq!(tool_name("Read"), "Read");
    }

    #[test]
    fn labels_shorten_only_when_shown_keeping_a_path_tail_and_a_word_head() {
        let labels: Vec<String> = [
            Source::of(
                Some("Read"),
                Some("/work/demo/docs/plans/2026-10-11-m6-insights-and-alerts.md"),
                CWD,
            ),
            Source::of(Some("Read"), Some("/elsewhere/src/main.rs"), CWD),
            Source::of(
                Some("Grep"),
                Some("rg --files-with-matches --hidden 'fn main' /work/demo"),
                CWD,
            ),
            bash("cd /work/demo && sed -n 1,9p docs/superpowers/plans/2026-09-26-self-hosted.md"),
            bash("cargo test --workspace"),
            Source::of(Some("Bash"), None, CWD),
        ]
        .iter()
        .map(Source::label)
        .collect();
        assert_eq!(
            labels,
            [
                "Read …-11-m6-insights-and-alerts.md",
                "Read /elsewhere/src/main.rs",
                "Grep rg --files-with-matches --hid…",
                "Bash sed …2026-09-26-self-hosted.md",
                "Bash cargo test",
                "Bash commands too long to name",
            ]
        );
    }
}
