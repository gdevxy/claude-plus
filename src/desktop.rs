use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use serde::Deserialize;

use crate::store::{AccountKind, AccountRow, AttributionSource, Store};

const NOT_IN_ACCOUNT: &str = "is not in an account folder";
const UNREADABLE: &str = "cannot be read";
const NOT_JSON: &str = "is not valid JSON";
const NOT_SESSION_FILE: &str = "is not a desktop session file";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopSession {
    pub account_uuid: String,
    pub cli_session_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub path: PathBuf,
    pub reason: &'static str,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct DesktopScan {
    pub found: bool,
    pub files_read: usize,
    pub sessions: Vec<DesktopSession>,
    pub skipped: Vec<Skipped>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
}

#[derive(Debug, Default)]
pub struct DesktopReader {
    stamps: HashMap<PathBuf, Stamp>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionFile {
    cli_session_id: Option<String>,
    #[serde(default)]
    prior_cli_session_ids: Vec<String>,
}

impl DesktopReader {
    pub fn scan(&mut self, root: &Path) -> DesktopScan {
        let mut scan = DesktopScan::default();
        match std::fs::metadata(root) {
            Err(error) if error.kind() == ErrorKind::NotFound => return scan,
            Err(error) => {
                scan.problem = Some(error.to_string());
                return scan;
            }
            Ok(_) => scan.found = true,
        }
        let entries = walkdir::WalkDir::new(root)
            .min_depth(3)
            .max_depth(3)
            .sort_by_file_name();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    scan.problem.get_or_insert_with(|| error.to_string());
                    continue;
                }
            };
            let path = entry.path();
            let is_json = path.extension().is_some_and(|ext| ext == "json");
            if !entry.file_type().is_file() || !is_json {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let stamp = Stamp {
                modified: metadata.modified().ok(),
                len: metadata.len(),
            };
            if self.stamps.insert(path.to_path_buf(), stamp) == Some(stamp) {
                continue;
            }
            scan.files_read += 1;
            match read_session(path) {
                Ok(session) if session.cli_session_ids.is_empty() => {}
                Ok(session) => scan.sessions.push(session),
                Err(reason) => scan.skipped.push(Skipped {
                    path: path.to_path_buf(),
                    reason,
                }),
            }
        }
        scan
    }
}

fn read_session(path: &Path) -> Result<DesktopSession, &'static str> {
    let account_uuid = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| is_uuid(name))
        .ok_or(NOT_IN_ACCOUNT)?
        .to_string();
    let text = std::fs::read_to_string(path).map_err(|_| UNREADABLE)?;
    let file: SessionFile = serde_json::from_str(&text).map_err(|error| {
        if error.is_data() {
            NOT_SESSION_FILE
        } else {
            NOT_JSON
        }
    })?;
    let cli_session_ids = file
        .cli_session_id
        .into_iter()
        .chain(file.prior_cli_session_ids)
        .filter(|id| !id.is_empty())
        .collect();
    Ok(DesktopSession {
        account_uuid,
        cli_session_ids,
    })
}

fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.char_indices().all(|(index, ch)| match index {
            8 | 13 | 18 | 23 => ch == '-',
            _ => ch.is_ascii_hexdigit(),
        })
}

pub fn account_row(account_uuid: &str) -> AccountRow {
    AccountRow {
        id: format!("oauth:{account_uuid}"),
        kind: AccountKind::Oauth,
        label: format!("Desktop account {}", &account_uuid[..8]),
        organization_type: None,
        billing_type: None,
    }
}

pub fn attribute(store: &mut Store, sessions: &[DesktopSession], now_ms: i64) -> Result<usize> {
    if sessions.is_empty() {
        return Ok(0);
    }
    let mut writer = store.writer()?;
    let mut changed = 0;
    for session in sessions {
        let account = account_row(&session.account_uuid);
        writer.insert_account_if_missing(&account, now_ms)?;
        for id in &session.cli_session_ids {
            changed +=
                usize::from(writer.attribute(id, &account.id, AttributionSource::Desktop)?);
        }
    }
    writer.commit()?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::AccountKind;

    const ACCOUNT: &str = "e08c8984-1111-4222-8333-444455556666";
    const ORG: &str = "0a0b0c0d-1111-4222-8333-444455556666";

    fn write(root: &Path, account: &str, name: &str, body: &str) -> PathBuf {
        let dir = root.join(account).join(ORG);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    fn reasons(scan: &DesktopScan) -> Vec<(String, &'static str)> {
        scan.skipped
            .iter()
            .map(|skipped| {
                let name = skipped.path.file_name().unwrap().to_string_lossy();
                (name.into_owned(), skipped.reason)
            })
            .collect()
    }

    #[test]
    fn every_cli_session_of_a_file_belongs_to_its_grandparent_account() {
        let temp = tempfile::tempdir().unwrap();
        write(
            temp.path(),
            ACCOUNT,
            "local_1.json",
            r#"{"sessionId":"local_1","cliSessionId":"cli-now","priorCliSessionIds":["cli-a","cli-b"],"cwd":"/w","createdAt":1791021000000}"#,
        );
        let scan = DesktopReader::default().scan(temp.path());
        assert!(scan.found);
        assert_eq!(scan.files_read, 1);
        assert_eq!(reasons(&scan), vec![]);
        assert_eq!(
            scan.sessions,
            vec![DesktopSession {
                account_uuid: ACCOUNT.into(),
                cli_session_ids: vec!["cli-now".into(), "cli-a".into(), "cli-b".into()],
            }]
        );
    }

    #[test]
    fn a_file_without_prior_ids_or_created_at_still_counts() {
        let temp = tempfile::tempdir().unwrap();
        write(
            temp.path(),
            ACCOUNT,
            "local_2.json",
            r#"{"cliSessionId":"cli-only"}"#,
        );
        let scan = DesktopReader::default().scan(temp.path());
        assert_eq!(
            scan.sessions[0].cli_session_ids,
            vec!["cli-only".to_string()]
        );
    }

    #[test]
    fn files_outside_an_account_folder_or_broken_are_skipped_and_a_desktop_only_session_is_ignored()
    {
        let temp = tempfile::tempdir().unwrap();
        write(
            temp.path(),
            "not-a-uuid",
            "a.json",
            r#"{"cliSessionId":"x"}"#,
        );
        write(temp.path(), ACCOUNT, "b.json", r#"{"cliSession"#);
        write(
            temp.path(),
            ACCOUNT,
            "c.json",
            r#"{"sessionId":"local_c","cliSessionId":null}"#,
        );
        write(
            temp.path(),
            ACCOUNT,
            "d.json",
            r#"{"cliSessionId":"x","priorCliSessionIds":"oops"}"#,
        );
        write(temp.path(), ACCOUNT, "notes.txt", "not a session file");
        let scan = DesktopReader::default().scan(temp.path());
        assert_eq!(scan.files_read, 4);
        assert_eq!(scan.sessions, vec![]);
        assert_eq!(
            reasons(&scan),
            vec![
                ("b.json".to_string(), "is not valid JSON"),
                ("d.json".to_string(), "is not a desktop session file"),
                ("a.json".to_string(), "is not in an account folder"),
            ]
        );
    }

    #[test]
    fn a_missing_folder_is_simply_not_found() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            DesktopReader::default().scan(&temp.path().join("absent")),
            DesktopScan::default()
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_account_folder_is_a_problem_and_the_rest_is_still_read() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let other = "f00dbabe-1111-4222-8333-444455556666";
        write(temp.path(), ACCOUNT, "a.json", r#"{"cliSessionId":"a"}"#);
        write(temp.path(), other, "b.json", r#"{"cliSessionId":"b"}"#);
        let locked = temp.path().join(other);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let scan = DesktopReader::default().scan(temp.path());
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(scan.problem.is_some(), "{scan:?}");
        assert_eq!(scan.sessions.len(), 1);
    }

    #[test]
    fn a_file_is_read_again_only_after_it_changes() {
        let temp = tempfile::tempdir().unwrap();
        let path = write(
            temp.path(),
            ACCOUNT,
            "local_3.json",
            r#"{"cliSessionId":"cli-"#,
        );
        let mut reader = DesktopReader::default();
        let first = reader.scan(temp.path());
        assert_eq!(
            reasons(&first),
            vec![("local_3.json".to_string(), "is not valid JSON")]
        );
        let unchanged = reader.scan(temp.path());
        assert_eq!((unchanged.files_read, unchanged.skipped.len()), (0, 0));
        std::fs::write(&path, r#"{"cliSessionId":"cli-3","priorCliSessionIds":[]}"#).unwrap();
        let changed = reader.scan(temp.path());
        assert_eq!(changed.files_read, 1);
        assert_eq!(
            changed.sessions[0].cli_session_ids,
            vec!["cli-3".to_string()]
        );
    }

    #[test]
    fn a_desktop_account_is_labelled_until_the_cli_signs_in_to_it() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open_in_memory().unwrap();
        let sessions = vec![DesktopSession {
            account_uuid: ACCOUNT.into(),
            cli_session_ids: vec!["cli-1".into()],
        }];
        let id = format!("oauth:{ACCOUNT}");
        assert_eq!(attribute(&mut store, &sessions, 5).unwrap(), 1);
        let info = store.account(&id).unwrap().unwrap();
        assert_eq!(
            (info.kind, info.label.as_str(), info.first_seen_ms),
            (AccountKind::Oauth, "Desktop account e08c8984", 5)
        );
        let login = temp.path().join(".claude.json");
        std::fs::write(
            &login,
            format!(r#"{{"oauthAccount":{{"accountUuid":"{ACCOUNT}","organizationName":"Acme","organizationType":"claude_enterprise"}}}}"#),
        )
        .unwrap();
        crate::accounts::observe(&mut store, &login, 10).unwrap();
        assert_eq!(
            store.account(&id).unwrap().unwrap().label,
            "Acme · Enterprise"
        );
        assert_eq!(attribute(&mut store, &sessions, 20).unwrap(), 0);
        assert_eq!(
            store.account(&id).unwrap().unwrap().label,
            "Acme · Enterprise"
        );
        assert_eq!(store.session_account("cli-1").unwrap(), Some(id));
    }

    #[test]
    fn a_desktop_session_leaves_the_cli_login_for_the_desktop_account() {
        let mut store = Store::open_in_memory().unwrap();
        let mut writer = store.writer().unwrap();
        writer
            .upsert_account(&account_row("c478b5d6-1111-4222-8333-444455556666"), 0)
            .unwrap();
        writer
            .attribute(
                "cli-1",
                "oauth:c478b5d6-1111-4222-8333-444455556666",
                crate::store::AttributionSource::Start,
            )
            .unwrap();
        writer.commit().unwrap();
        let sessions = vec![DesktopSession {
            account_uuid: ACCOUNT.into(),
            cli_session_ids: vec!["cli-1".into()],
        }];
        assert_eq!(attribute(&mut store, &sessions, 5).unwrap(), 1);
        assert_eq!(
            store.session_account("cli-1").unwrap(),
            Some(format!("oauth:{ACCOUNT}"))
        );
    }
}
