# claude-plus Session Attribution Backfill Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** claude-plus attributes every Claude desktop app session to the account the desktop app ran it under, corrects desktop sessions filed under the CLI's OAuth login, and lets the user attribute Unknown sessions by project and date range with `claude-plus accounts assign`, preview it with `--dry-run` and undo it with `accounts unassign`.

**Architecture:** A new unit `src/desktop.rs` reads the desktop app's `claude-code-sessions/<accountUuid>/<orgUuid>/*.json` files read-only (only `cliSessionId` and `priorCliSessionIds`), with a per-process modification time and size cache, and writes `session_accounts` rows with the new source `desktop`.
`Writer::attribute` gains the precedence table of the spec in one SQL upsert, and a new `Writer::insert_account_if_missing` creates desktop accounts without ever overwriting a CLI label.
The desktop scan runs in local imports (`import`, and the imports `stats`, `export` and `accounts` do first) and in the daemon writer's `Scan` job (startup, every rescan, and imports through the socket); `doctor` reports what it finds.
A new unit `src/assign.rs` holds the session filter (a `*` glob on `sessions.cwd`, inclusive local days) and the selection summary; `accounts assign` and `accounts unassign` in `src/cli/accounts.rs` import first, select, print, and write through the store writer unless `--dry-run`.

**Tech Stack:** Rust (edition 2024, MSRV 1.88), no new crates (rusqlite, serde, serde_json, chrono, clap, walkdir).

**Spec:** `docs/superpowers/specs/2026-10-12-session-attribution-backfill-design.md`

This plan starts from `main` at `fb13847`.
If `main` has moved, apply the hunks by the code they quote, not by line number.

## Global Constraints

- claude-plus makes no network requests.
- claude-plus never writes Claude Code's files or the desktop app's files; `.claude.json` and every desktop session file are only read.
- From a desktop session file only `cliSessionId` and `priorCliSessionIds` are read; every other key is ignored by deserialization.
- No schema migration: the schema version stays at 18, and `desktop` and `assign` are new values of the free-text `session_accounts.source`.
- `desktop` never replaces a row whose account kind is `cloud`, `gateway` or `api`.
- Never edit `~/.claude/settings.json`.
- Never push; commit on `main` only.
- No em dash character anywhere in code, strings, tests, commit messages or docs; use `-`.
- Minimal code comments; prefer self-explanatory names.
- Every Markdown file written or edited puts each full sentence on its own line.
- Any lint, test failure or flaky test met along the way is fixed, even when it is not caused by the task.
- The Rust gate after every task: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`.
- Commit messages follow the repository's `type(scope): summary` style and end with the single trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`; no other co-author lines.

## Decisions

The spec is silent or does not match the code on these points, so this plan decides them.

1. The test harness (`tests/common/mod.rs`) sets `HOME` to a fresh `<temp>/home` for every command it runs.
   Without it every existing CLI and daemon test would read the developer's real desktop folder.
2. The desktop data folder is `<home>/Library/Application Support/Claude` on macOS and `<home>/.config/Claude` everywhere else.
   The Windows `%APPDATA%` branch of the spec is not built, because the crate is Unix-only today (`tokio::signal::unix`, `rustix`, an unguarded `std::os::unix::fs::OpenOptionsExt` in `src/paths.rs`) and CI builds macOS and Linux only.
3. Desktop accounts are created with `insert_account_if_missing` (`ON CONFLICT DO NOTHING`), never with `upsert_account`.
   `upsert_account` replaces any label that is not custom, so a rescan would otherwise put `Desktop account <uuid8>` back over `<organizationName> · <plan>` every 60 seconds.
4. An attribution write that would leave the same account and source in place reports no change and writes nothing, so a rescan of unchanged data is free.
5. `accounts` counts only sessions that were imported (`account_sessions_count` joins `sessions`), since desktop ids without a transcript are now stored.
6. A skipped desktop file is reported once per process: the stamp cache means it is read again only after it changes.
   An unreadable file is not stamped, so it is read again on every scan, but the daemon logs each path and reason only once per run.
   The CLI prints `Skipped the desktop session file <path>, it <reason>` on stderr; the daemon logs `skipped the desktop session file <path>, it <reason>` as a warning.
   An unreadable desktop folder is printed by the CLI as `Cannot read desktop sessions: <error>` and logged by the daemon once per run.
   The CLI's messages only appear for local imports: when the daemon runs, `import` goes through its socket and the messages only reach `daemon.log`, so `doctor` is the place to see skipped files.
   A file that cannot be read is not remembered, so it is tried again on the next scan, and files that are gone are forgotten.
7. If the daemon's desktop write fails, the daemon forgets its stamp cache so the next rescan reads every file again.
8. `assign` and `unassign` import first, like `accounts` and `budget show`, so desktop and timeline attribution run before Unknown sessions are selected.
   The filter and dates are validated before that import.
9. `assign` requires an account that is in `accounts`; an unknown one fails with `no account called "<given>"; the accounts are <label> (<id>), ...`, or `no account called "<given>"; no account is known yet, see claude-plus accounts`.
   An ambiguous label fails with the existing `"<given>" matches ...; pass the id` message from `resolve`.
10. A leading `~/` in `--project` is replaced with `HOME`, because a quoted pattern is not expanded by the shell.
    Otherwise `--project` is a whole-string match where only `*` is special and matches any characters, `/` included.
11. A session without a start time matches when only `--project` is given, and never matches when `--since` or `--until` is given.
12. Spend is the sum of `requests.cost_usd` and `adjustments.cost_usd` of the selected sessions, the same sum `stats` totals; subagent requests carry their parent's `session_id`, so they are included.
13. Output wording: `Assigned <n> session(s) to <label>`, `Would assign ...`, `Unassigned <n> session(s)`, `Would unassign ...`, then `  <first day> to <last day> · <spend>` (one day when they are equal, the spend alone when no selected session has a start time).
    Nothing selected prints `No Unknown sessions match` or `No assigned sessions match` and exits 0.
14. `doctor` prints, after `Prices:`, `Desktop:     none found in <dir>`, or `Desktop:     <dir> · <n> session files[ · <m> skipped][ · cannot read <path>: <error>]`, or `Desktop:     cannot read <dir>: <error>`.
    When part of the folder cannot be read, the counts of what was read stay and the first path that could not be read is appended.
    When the folder itself cannot be read, only the error is printed.
15. When two desktop files claim the same CLI session id under different accounts, the last one read wins (files are read in path order).
    In the daemon, once the stamp cache is warm only changed files are read, so the most recently changed file wins until the daemon restarts.

## Review Focus

1. The daemon rescans every 60 seconds: re-reading unchanged desktop data must write nothing (Task 1 test `attributing_the_same_account_and_source_again_changes_nothing`, Task 2 test `a_file_is_read_again_only_after_it_changes`).
2. A desktop file read while the desktop app is writing it (truncated JSON) is skipped and read again once it changes (Task 2 test `a_file_is_read_again_only_after_it_changes`).
3. A desktop account the CLI later signs in to keeps the CLI's `<organizationName> · <plan>` label on every later scan (Task 2 test `a_desktop_account_is_labelled_until_the_cli_signs_in_to_it`).
4. Day bounds in a time zone other than UTC: a session at 00:30 local on `--since` day is in, one at 23:59 local the day before is out, and `--until` includes its whole local day (Task 4 test `days_are_local_and_both_bounds_are_inclusive`).
5. A user who types `--project '~/private/*'` in quotes expects it to match their home folder (Task 4 test `a_leading_tilde_is_the_home_folder`).

---

### Task 1: Attribution precedence, new sources and desktop-safe account insertion

**Files:**
- Modify: `src/store/accounts.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `pub enum AttributionSource { Start, Timeline, Desktop, Assign }`
  - `Writer::attribute(&mut self, session_id: &str, account_id: &str, source: AttributionSource) -> Result<bool>` following the spec's precedence table; `true` only when the row was inserted or changed.
  - `Writer::insert_account_if_missing(&mut self, row: &AccountRow, now_ms: i64) -> Result<bool>`
  - `Store::account_sessions_count(&self, account_id: &str) -> Result<u64>` now counts only sessions present in `sessions`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module of `src/store/accounts.rs` (it already has `oauth()` and `session()` helpers):

```rust
    fn typed(id: &str, kind: AccountKind) -> AccountRow {
        AccountRow {
            id: id.into(),
            kind,
            label: id.into(),
            organization_type: None,
            billing_type: None,
        }
    }

    fn row_of(store: &Store, session: &str) -> Option<(String, String)> {
        store
            .conn
            .query_row(
                "SELECT account_id, source FROM session_accounts WHERE session_id = ?1",
                [session],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .unwrap()
    }

    fn desktop_row() -> Option<(String, String)> {
        Some(("oauth:desk".into(), "desktop".into()))
    }

    #[test]
    fn a_desktop_attribution_replaces_no_row_or_any_row_on_an_oauth_account() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("cli", "CLI"), 0).unwrap();
        w.upsert_account(&oauth("desk", "Desk"), 0).unwrap();
        w.upsert_account(&oauth("other", "Other"), 0).unwrap();
        let earlier = [
            ("start", AttributionSource::Start),
            ("timeline", AttributionSource::Timeline),
            ("assign", AttributionSource::Assign),
            ("desktop", AttributionSource::Desktop),
        ];
        for (session, source) in earlier {
            let account = if session == "desktop" { "oauth:other" } else { "oauth:cli" };
            assert!(w.attribute(session, account, source).unwrap(), "{session}");
            assert!(
                w.attribute(session, "oauth:desk", AttributionSource::Desktop)
                    .unwrap(),
                "{session}"
            );
        }
        assert!(
            w.attribute("fresh", "oauth:desk", AttributionSource::Desktop)
                .unwrap()
        );
        w.commit().unwrap();
        for session in ["start", "timeline", "assign", "desktop", "fresh"] {
            assert_eq!(row_of(&store, session), desktop_row(), "{session}");
        }
    }

    #[test]
    fn a_desktop_attribution_keeps_a_cloud_gateway_or_api_row() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("desk", "Desk"), 0).unwrap();
        for (id, kind) in [
            ("cloud:bedrock", AccountKind::Cloud),
            ("gateway:https://gw:443", AccountKind::Gateway),
            ("api", AccountKind::Api),
        ] {
            w.upsert_account(&typed(id, kind), 0).unwrap();
            w.attribute(id, id, AttributionSource::Start).unwrap();
            assert!(
                !w.attribute(id, "oauth:desk", AttributionSource::Desktop)
                    .unwrap(),
                "{id}"
            );
        }
        w.commit().unwrap();
        for id in ["cloud:bedrock", "gateway:https://gw:443", "api"] {
            assert_eq!(row_of(&store, id), Some((id.into(), "start".into())));
        }
    }

    #[test]
    fn a_start_attribution_replaces_timeline_and_assign_but_not_desktop_or_start() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("cli", "CLI"), 0).unwrap();
        w.upsert_account(&oauth("desk", "Desk"), 0).unwrap();
        w.attribute("t", "oauth:desk", AttributionSource::Timeline)
            .unwrap();
        w.attribute("a", "oauth:desk", AttributionSource::Assign)
            .unwrap();
        w.attribute("d", "oauth:desk", AttributionSource::Desktop)
            .unwrap();
        w.attribute("s", "oauth:desk", AttributionSource::Start)
            .unwrap();
        assert!(w.attribute("t", "oauth:cli", AttributionSource::Start).unwrap());
        assert!(w.attribute("a", "oauth:cli", AttributionSource::Start).unwrap());
        assert!(!w.attribute("d", "oauth:cli", AttributionSource::Start).unwrap());
        assert!(!w.attribute("s", "oauth:cli", AttributionSource::Start).unwrap());
        assert!(w.attribute("new", "oauth:cli", AttributionSource::Start).unwrap());
        w.commit().unwrap();
        let cli_start = Some(("oauth:cli".to_string(), "start".to_string()));
        assert_eq!(row_of(&store, "t"), cli_start);
        assert_eq!(row_of(&store, "a"), cli_start);
        assert_eq!(row_of(&store, "new"), cli_start);
        assert_eq!(row_of(&store, "d"), desktop_row());
        assert_eq!(
            row_of(&store, "s"),
            Some(("oauth:desk".into(), "start".into()))
        );
    }

    #[test]
    fn timeline_and_assign_attributions_only_fill_an_empty_row() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("cli", "CLI"), 0).unwrap();
        w.upsert_account(&oauth("desk", "Desk"), 0).unwrap();
        for source in [AttributionSource::Timeline, AttributionSource::Assign] {
            for earlier in [
                AttributionSource::Start,
                AttributionSource::Timeline,
                AttributionSource::Desktop,
                AttributionSource::Assign,
            ] {
                let session = format!("{source:?}-{earlier:?}");
                w.attribute(&session, "oauth:desk", earlier).unwrap();
                assert!(!w.attribute(&session, "oauth:cli", source).unwrap(), "{session}");
            }
            let empty = format!("{source:?}-empty");
            assert!(w.attribute(&empty, "oauth:cli", source).unwrap());
        }
        w.commit().unwrap();
        assert_eq!(
            row_of(&store, "Assign-empty"),
            Some(("oauth:cli".into(), "assign".into()))
        );
        assert_eq!(
            row_of(&store, "Timeline-Desktop"),
            desktop_row()
        );
    }

    #[test]
    fn attributing_the_same_account_and_source_again_changes_nothing() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("desk", "Desk"), 0).unwrap();
        assert!(
            w.attribute("s1", "oauth:desk", AttributionSource::Desktop)
                .unwrap()
        );
        w.commit().unwrap();
        let before = store.total_changes();
        let mut w = store.writer().unwrap();
        assert!(
            !w.attribute("s1", "oauth:desk", AttributionSource::Desktop)
                .unwrap()
        );
        w.commit().unwrap();
        assert_eq!(store.total_changes(), before);
    }

    #[test]
    fn inserting_a_missing_account_never_touches_a_known_one() {
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("a", "Acme · Enterprise"), 1).unwrap();
        assert!(!w.insert_account_if_missing(&oauth("a", "Desktop account a"), 2).unwrap());
        assert!(w.insert_account_if_missing(&oauth("b", "Desktop account b"), 3).unwrap());
        w.commit().unwrap();
        let a = store.account("oauth:a").unwrap().unwrap();
        assert_eq!((a.label.as_str(), a.first_seen_ms), ("Acme · Enterprise", 1));
        let b = store.account("oauth:b").unwrap().unwrap();
        assert_eq!((b.label.as_str(), b.first_seen_ms), ("Desktop account b", 3));
    }

    #[test]
    fn an_account_counts_only_sessions_that_were_imported() {
        let mut store = Store::open_in_memory().unwrap();
        session(&mut store, "s1", 10);
        let mut w = store.writer().unwrap();
        w.upsert_account(&oauth("a", "A"), 0).unwrap();
        w.attribute("s1", "oauth:a", AttributionSource::Desktop)
            .unwrap();
        w.attribute("not-imported", "oauth:a", AttributionSource::Desktop)
            .unwrap();
        w.commit().unwrap();
        assert_eq!(store.account_sessions_count("oauth:a").unwrap(), 1);
        assert_eq!(
            store.session_account("not-imported").unwrap().as_deref(),
            Some("oauth:a")
        );
    }
```

`AttributionSource` needs `Debug` for the `{source:?}` names; it already derives it.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test --lib store::accounts`
Expected: compile errors for `AttributionSource::Desktop`, `AttributionSource::Assign` and `insert_account_if_missing`.

- [ ] **Step 3: Implement**

In `src/store/accounts.rs`, replace the `AttributionSource` enum and its `impl`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributionSource {
    Start,
    Timeline,
    Desktop,
    Assign,
}

impl AttributionSource {
    fn as_str(self) -> &'static str {
        match self {
            AttributionSource::Start => "start",
            AttributionSource::Timeline => "timeline",
            AttributionSource::Desktop => "desktop",
            AttributionSource::Assign => "assign",
        }
    }
}
```

Replace the body of `Writer::attribute`:

```rust
    pub fn attribute(
        &mut self,
        session_id: &str,
        account_id: &str,
        source: AttributionSource,
    ) -> Result<bool> {
        let changed = self
            .tx
            .prepare_cached(
                "INSERT INTO session_accounts (session_id, account_id, source) VALUES (?1, ?2, ?3)
                 ON CONFLICT (session_id) DO UPDATE SET account_id = excluded.account_id, source = excluded.source
                 WHERE (session_accounts.account_id <> excluded.account_id OR session_accounts.source <> excluded.source)
                   AND ((excluded.source = 'start' AND session_accounts.source IN ('timeline', 'assign'))
                     OR (excluded.source = 'desktop' AND EXISTS (
                           SELECT 1 FROM accounts
                           WHERE accounts.id = session_accounts.account_id AND accounts.kind = 'oauth')))",
            )?
            .execute(params![session_id, account_id, source.as_str()])?;
        Ok(changed > 0)
    }
```

Add after `upsert_account`:

```rust
    pub fn insert_account_if_missing(&mut self, row: &AccountRow, now_ms: i64) -> Result<bool> {
        let inserted = self
            .tx
            .prepare_cached(
                "INSERT INTO accounts (id, kind, label, organization_type, billing_type, first_seen_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (id) DO NOTHING",
            )?
            .execute(params![
                row.id,
                row.kind.as_str(),
                row.label,
                row.organization_type,
                row.billing_type,
                now_ms
            ])?;
        Ok(inserted > 0)
    }
```

Replace the query of `Store::account_sessions_count`:

```rust
    pub fn account_sessions_count(&self, account_id: &str) -> Result<u64> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT COUNT(*) FROM session_accounts a JOIN sessions s ON s.id = a.session_id
                 WHERE a.account_id = ?1",
            )?
            .query_row([account_id], |r| r.get::<_, i64>(0))? as u64)
    }
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test --lib store::accounts && cargo test --lib accounts`
Expected: all pass, including the existing `a_start_attribution_replaces_a_timeline_one_but_not_the_reverse`.

- [ ] **Step 5: Run the gate**

Run: `cargo fmt && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Expected: clean; `accounts_output_is_exact` in `tests/cli.rs` still shows 1 session per account.

- [ ] **Step 6: Commit**

```bash
git add src/store/accounts.rs
git commit -m "feat(store): rank desktop and assign attributions and insert accounts without relabelling

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Desktop session reader and desktop attribution

**Files:**
- Modify: `src/paths.rs` (add `desktop_sessions_dir` and `desktop_dir_in`)
- Create: `src/desktop.rs`
- Modify: `src/lib.rs` (add `pub mod desktop;` in alphabetical order, after `pub mod daemon;`)

**Interfaces:**
- Consumes: `Writer::attribute(.., AttributionSource::Desktop)`, `Writer::insert_account_if_missing` (Task 1).
- Produces:
  - `Paths::desktop_sessions_dir(&self) -> PathBuf`
  - `pub struct DesktopSession { pub account_uuid: String, pub cli_session_ids: Vec<String> }`
  - `pub struct Skipped { pub path: PathBuf, pub reason: &'static str }`
  - `pub struct DesktopScan { pub found: bool, pub files_read: usize, pub sessions: Vec<DesktopSession>, pub skipped: Vec<Skipped>, pub problem: Option<String> }` (derives `Default`)
  - `pub struct DesktopReader` (derives `Default`) with `pub fn scan(&mut self, root: &Path) -> DesktopScan`
  - `pub fn desktop::attribute(store: &mut Store, sessions: &[DesktopSession], now_ms: i64) -> Result<usize>`
  - `pub fn desktop::account_row(account_uuid: &str) -> AccountRow`

- [ ] **Step 1: Write the failing path test**

Add to the `tests` module of `src/paths.rs`:

```rust
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
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test --lib paths::tests::the_desktop_app_keeps_its_sessions_in_its_own_data_folder`
Expected: compile error, `desktop_dir_in` not found.

- [ ] **Step 3: Implement the path**

In `src/paths.rs`, add to `impl Paths` after `projects_dir`:

```rust
    pub fn desktop_sessions_dir(&self) -> PathBuf {
        desktop_dir_in(&self.home, std::env::consts::OS).join("claude-code-sessions")
    }
```

Add next to `retention_days_in`:

```rust
fn desktop_dir_in(home: &Path, os: &str) -> PathBuf {
    match os {
        "macos" => home.join("Library/Application Support/Claude"),
        _ => home.join(".config/Claude"),
    }
}
```

Run: `cargo test --lib paths`
Expected: PASS.

- [ ] **Step 4: Write the failing reader tests**

Create `src/desktop.rs` with only the test module first:

```rust
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
        write(temp.path(), ACCOUNT, "local_2.json", r#"{"cliSessionId":"cli-only"}"#);
        let scan = DesktopReader::default().scan(temp.path());
        assert_eq!(scan.sessions[0].cli_session_ids, vec!["cli-only".to_string()]);
    }

    #[test]
    fn files_outside_an_account_folder_or_broken_are_skipped_and_a_desktop_only_session_is_ignored() {
        let temp = tempfile::tempdir().unwrap();
        write(temp.path(), "not-a-uuid", "a.json", r#"{"cliSessionId":"x"}"#);
        write(temp.path(), ACCOUNT, "b.json", r#"{"cliSession"#);
        write(temp.path(), ACCOUNT, "c.json", r#"{"sessionId":"local_c","cliSessionId":null}"#);
        write(temp.path(), ACCOUNT, "d.json", r#"{"cliSessionId":"x","priorCliSessionIds":"oops"}"#);
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
        let path = write(temp.path(), ACCOUNT, "local_3.json", r#"{"cliSessionId":"cli-"#);
        let mut reader = DesktopReader::default();
        let first = reader.scan(temp.path());
        assert_eq!(reasons(&first), vec![("local_3.json".to_string(), "is not valid JSON")]);
        let unchanged = reader.scan(temp.path());
        assert_eq!((unchanged.files_read, unchanged.skipped.len()), (0, 0));
        std::fs::write(&path, r#"{"cliSessionId":"cli-3","priorCliSessionIds":[]}"#).unwrap();
        let changed = reader.scan(temp.path());
        assert_eq!(changed.files_read, 1);
        assert_eq!(changed.sessions[0].cli_session_ids, vec!["cli-3".to_string()]);
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
        assert_eq!(store.account(&id).unwrap().unwrap().label, "Acme · Enterprise");
        assert_eq!(attribute(&mut store, &sessions, 20).unwrap(), 0);
        assert_eq!(store.account(&id).unwrap().unwrap().label, "Acme · Enterprise");
        assert_eq!(store.session_account("cli-1").unwrap(), Some(id));
    }

    #[test]
    fn a_desktop_session_leaves_the_cli_login_for_the_desktop_account() {
        let mut store = Store::open_in_memory().unwrap();
        let mut writer = store.writer().unwrap();
        writer.upsert_account(&account_row("c478b5d6-1111-4222-8333-444455556666"), 0).unwrap();
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
```

Add `pub mod desktop;` to `src/lib.rs` after `pub mod daemon;`.

- [ ] **Step 5: Run the tests to see them fail**

Run: `cargo test --lib desktop`
Expected: compile errors, `DesktopReader`, `DesktopScan`, `DesktopSession`, `attribute` and `account_row` not found.

- [ ] **Step 6: Implement the reader**

Put this above the test module in `src/desktop.rs`:

```rust
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
            changed += usize::from(writer.attribute(id, &account.id, AttributionSource::Desktop)?);
        }
    }
    writer.commit()?;
    Ok(changed)
}
```

`account_row` slices `[..8]` safely because `read_session` only yields uuids, which are ASCII and 36 characters long.

- [ ] **Step 7: Run the tests to see them pass**

Run: `cargo test --lib desktop && cargo test --lib paths`
Expected: PASS.
If `an_unreadable_account_folder_is_a_problem_and_the_rest_is_still_read` fails only because the tests run as root, report it instead of deleting the test.

- [ ] **Step 8: Run the gate**

Run: `cargo fmt && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Expected: clean.

- [ ] **Step 9: Commit**

```bash
git add src/paths.rs src/desktop.rs src/lib.rs
git commit -m "feat(desktop): read the desktop app's session files and attribute their CLI sessions

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Desktop attribution in import, the daemon and doctor

**Files:**
- Modify: `tests/common/mod.rs` (temp `HOME`, desktop helpers, `attribution`, `seed_start`)
- Modify: `src/cli/import.rs` (`locally`)
- Modify: `src/daemon/writer.rs` (`spawn`, `Worker`, `Job::Scan`, test `Fixture`)
- Modify: `src/daemon/mod.rs` (the `writer::spawn` call)
- Modify: `src/cli/doctor.rs` (the `Desktop:` line)
- Test: `tests/cli.rs`, `tests/daemon.rs`

**Interfaces:**
- Consumes: `Paths::desktop_sessions_dir`, `DesktopReader::scan`, `desktop::attribute`, `DesktopScan`, `Skipped` (Task 2).
- Produces:
  - `writer::spawn(store: Store, prices: PriceTable, projects_dir: PathBuf, claude_json: PathBuf, desktop_sessions: PathBuf, log: Arc<Log>) -> Result<(WriterHandle, JoinHandle<()>)>`
  - Test helpers on `Env`: `pub home: PathBuf`, `desktop_sessions(&self) -> PathBuf`, `write_desktop_session(&self, account: &str, org: &str, name: &str, body: &str) -> PathBuf`, `attribution(&self, session: &str) -> Option<(String, String)>`, `seed_start(&self, session: &str, account: &str)`; constants `DESKTOP_ACCOUNT`, `DESKTOP_ORG`, `DEMO_SESSION`.

- [ ] **Step 1: Give every test command its own HOME and add the desktop helpers**

In `tests/common/mod.rs`, add the field and constants:

```rust
pub struct Env {
    pub _temp: tempfile::TempDir,
    pub claude: PathBuf,
    pub plus: PathBuf,
    pub home: PathBuf,
}

pub const DESKTOP_ACCOUNT: &str = "e08c8984-5a1e-4c1b-9d2f-0123456789ab";
pub const DESKTOP_ORG: &str = "7d3f0c2e-8b4a-4e6f-a1c9-0123456789ab";
pub const DEMO_SESSION: &str = "11111111-1111-1111-1111-111111111111";
```

In `env()`, create the home folder and set the field:

```rust
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    Env {
        _temp: temp,
        claude,
        plus,
        home,
    }
```

Add to `impl Env`:

```rust
    pub fn desktop_sessions(&self) -> PathBuf {
        let app = if cfg!(target_os = "macos") {
            "Library/Application Support/Claude"
        } else {
            ".config/Claude"
        };
        self.home.join(app).join("claude-code-sessions")
    }

    pub fn write_desktop_session(&self, account: &str, org: &str, name: &str, body: &str) -> PathBuf {
        let dir = self.desktop_sessions().join(account).join(org);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    pub fn attribution(&self, session: &str) -> Option<(String, String)> {
        use rusqlite::OptionalExtension;
        let connection = rusqlite::Connection::open_with_flags(
            self.plus.join("data.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .ok()?;
        connection
            .query_row(
                "SELECT account_id, source FROM session_accounts WHERE session_id = ?1",
                [session],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .unwrap()
    }

    pub fn seed_start(&self, session: &str, account: &str) {
        let mut store = claude_plus::store::Store::open(&self.plus.join("data.db")).unwrap();
        let mut writer = store.writer().unwrap();
        writer
            .attribute(session, account, claude_plus::store::AttributionSource::Start)
            .unwrap();
        writer.commit().unwrap();
    }
```

In both `cmd` and `process`, add `.env("HOME", &env.home)` right after `.args(args)`.
Tests that set their own `HOME` afterwards (`empty_plus_home_falls_back_to_home`, the purge tests in `tests/setup.rs`) still win, because a later `.env` replaces an earlier one.

Run: `cargo test --test cli --test daemon --test setup`
Expected: PASS; if a test depended on the real `HOME`, fix the test so it creates what it needs under `env.home`.

- [ ] **Step 2: Write the failing CLI and daemon tests**

In `tests/cli.rs`, change the `use common::...` line to:

```rust
use common::{
    DEMO_SESSION, DESKTOP_ACCOUNT, DESKTOP_ORG, Env, STATS_TABLE, cmd, env, stdout_of,
    stdout_of_with,
};
```

Add:

```rust
const CLI_ACCOUNT: &str = "c478b5d6-2b7e-4f0a-8c3d-0123456789ab";
const DESKTOP_SESSION: &str = "33333333-3333-3333-3333-333333333333";

fn desktop_file(cli_session: &str) -> String {
    format!(
        r#"{{"sessionId":"local_9f1c","cliSessionId":"{cli_session}","priorCliSessionIds":[],"cwd":"/work/demo","createdAt":1791021000000,"title":"kept private"}}"#
    )
}

#[test]
fn import_moves_a_desktop_session_off_the_cli_login_to_the_desktop_account() {
    let env = env();
    env.write_claude_json(&format!(
        r#"{{"oauthAccount":{{"accountUuid":"{CLI_ACCOUNT}","organizationName":"Me","organizationType":"claude_max"}}}}"#
    ));
    env.write_transcript_at(DESKTOP_SESSION, "claude-opus-5-5", "2026-10-03T10:00:00Z", 1_000);
    cmd(&env, &["import"]).assert().success();
    env.seed_start(DESKTOP_SESSION, &format!("oauth:{CLI_ACCOUNT}"));
    assert_eq!(
        env.attribution(DESKTOP_SESSION),
        Some((format!("oauth:{CLI_ACCOUNT}"), "start".into()))
    );
    let file = env.write_desktop_session(
        DESKTOP_ACCOUNT,
        DESKTOP_ORG,
        "local_9f1c.json",
        &desktop_file(DESKTOP_SESSION),
    );
    let before = std::fs::read(&file).unwrap();
    cmd(&env, &["import"]).assert().success().stderr("");
    assert_eq!(
        env.attribution(DESKTOP_SESSION),
        Some((format!("oauth:{DESKTOP_ACCOUNT}"), "desktop".into()))
    );
    assert_eq!(std::fs::read(&file).unwrap(), before);
    let accounts = stdout_of(&env, &["accounts"]);
    assert!(accounts.contains("Desktop account e08c8984"), "{accounts}");
    let stats = stdout_of(
        &env,
        &["stats", "--days", "1", "--account", "desktop account E08C8984"],
    );
    assert!(stats.starts_with("Account: Desktop account e08c8984\n"), "{stats}");
    assert!(stats.contains("claude-opus-5-5"), "{stats}");
}

#[test]
fn a_skipped_desktop_file_is_named_on_stderr_and_counted_by_doctor() {
    let env = env();
    env.write_desktop_session(DESKTOP_ACCOUNT, DESKTOP_ORG, "local_a.json", &desktop_file(DEMO_SESSION));
    let broken = env.write_desktop_session(DESKTOP_ACCOUNT, DESKTOP_ORG, "local_b.json", "{\"cliSess");
    cmd(&env, &["import"]).assert().success().stderr(format!(
        "Skipped the desktop session file {}, it is not valid JSON\n",
        broken.display()
    ));
    assert_eq!(
        env.attribution(DEMO_SESSION),
        Some((format!("oauth:{DESKTOP_ACCOUNT}"), "desktop".into()))
    );
    let doctor = stdout_of(&env, &["doctor"]);
    assert!(
        doctor.contains(&format!(
            "  Prices:      Anthropic's 2026-10-10\n  Desktop:     {} · 2 session files · 1 skipped\n",
            env.desktop_sessions().display()
        )),
        "{doctor}"
    );
}
```

In `tests/cli.rs`, add the `Desktop:` line to the three exact doctor expectations (`doctor_output_is_exact`, `doctor_header`, `doctor_on_a_fresh_home_creates_nothing`), right after the `Prices:` line:

```
  Prices:      Anthropic's 2026-10-10
  Desktop:     none found in {desktop}
```

and add `desktop = env.desktop_sessions().display()` to each of those `format!` argument lists.

In `tests/daemon.rs`, change the `use common::...` line to:

```rust
use common::{
    DESKTOP_ACCOUNT, DESKTOP_ORG, Env, STATS_TABLE, cmd, env, eventually, process, stdout_of,
};
```

Add:

```rust
#[test]
fn a_desktop_session_file_added_while_running_is_attributed_on_the_next_rescan() {
    let env = env();
    let _daemon = Foreground::start(&env, &["--rescan-secs", "1"]);
    idle_health(&env);
    assert_eq!(env.attribution(SESSION_1), None);
    env.write_desktop_session(
        DESKTOP_ACCOUNT,
        DESKTOP_ORG,
        "local_live.json",
        &format!(r#"{{"cliSessionId":"{SESSION_1}","priorCliSessionIds":[]}}"#),
    );
    let attributed = eventually("the rescan to attribute the desktop session", || {
        env.attribution(SESSION_1)
    });
    assert_eq!(
        attributed,
        (format!("oauth:{DESKTOP_ACCOUNT}"), "desktop".to_string())
    );
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `cargo test --test cli import_moves_a_desktop_session && cargo test --test cli a_skipped_desktop_file && cargo test --test cli doctor && cargo test --test daemon a_desktop_session_file_added`
Expected: FAIL, the attribution stays on the CLI account or is `None`, and doctor has no `Desktop:` line.

- [ ] **Step 4: Run the desktop scan in a local import**

In `src/cli/import.rs`, add `use crate::desktop::{self, DesktopReader};`, and in `locally`, replace

```rust
    accounts::attribute_pending(&mut store)?;
```

with

```rust
    attribute_desktop(&mut store, paths, now_ms)?;
    accounts::attribute_pending(&mut store)?;
```

and add:

```rust
fn attribute_desktop(store: &mut Store, paths: &Paths, now_ms: i64) -> Result<()> {
    let scan = DesktopReader::default().scan(&paths.desktop_sessions_dir());
    if let Some(problem) = &scan.problem {
        eprintln!("Cannot read desktop sessions: {problem}");
    }
    for skipped in &scan.skipped {
        eprintln!(
            "Skipped the desktop session file {}, it {}",
            skipped.path.display(),
            skipped.reason
        );
    }
    desktop::attribute(store, &scan.sessions, now_ms)?;
    Ok(())
}
```

- [ ] **Step 5: Run the desktop scan in the daemon's Scan job**

In `src/daemon/writer.rs`, add `use crate::desktop::{self, DesktopReader};`.
Add the parameter to `spawn` after `claude_json: PathBuf,`:

```rust
    desktop_sessions: PathBuf,
```

and the fields to the `Worker { .. }` literal in `spawn` after `claude_json,`:

```rust
        desktop_sessions,
        desktop: DesktopReader::default(),
```

Add the fields to `struct Worker` after `claude_json: PathBuf,`:

```rust
    desktop_sessions: PathBuf,
    desktop: DesktopReader,
```

In `Worker::run`, change the `Job::Scan` arm to:

```rust
                Job::Scan(reply) => {
                    self.observe_login();
                    let outcome = self.scan(&mut drain);
                    self.attribute_desktop();
                    self.publish_rebuilding(None);
                    self.finish_ingest(outcome, reply, true);
                }
```

Add to `impl Worker`, after `observe_login`:

```rust
    fn attribute_desktop(&mut self) {
        if self.is_cancelled() {
            return;
        }
        let scan = self.desktop.scan(&self.desktop_sessions);
        if let Some(problem) = &scan.problem {
            self.log
                .warn_once("desktop", format!("cannot read desktop sessions: {problem}"));
        }
        for skipped in &scan.skipped {
            self.log.warn(format!(
                "skipped the desktop session file {}, it {}",
                skipped.path.display(),
                skipped.reason
            ));
        }
        if let Err(error) =
            desktop::attribute(&mut self.store, &scan.sessions, epoch_ms(SystemTime::now()))
        {
            self.desktop = DesktopReader::default();
            self.log
                .warn(format!("cannot attribute desktop sessions: {error:#}"));
        }
    }
```

In the test `Fixture::new` of the same file, pass the new argument after `temp.path().join(".claude.json"),`:

```rust
                temp.path().join("desktop/claude-code-sessions"),
```

In `src/daemon/mod.rs`, pass it in the `writer::spawn(..)` call after `paths.claude_json.clone(),`:

```rust
        paths.desktop_sessions_dir(),
```

- [ ] **Step 6: Report the desktop folder in doctor**

In `src/cli/doctor.rs`, change the format import to `use super::format::{count, duration, plural, usd};` and add `use crate::desktop::DesktopReader;`.
After the `Prices:` `println!` in `run`, add:

```rust
    println!(
        "{}",
        labeled("Desktop:", &desktop_line(&paths.desktop_sessions_dir()))
    );
```

Add:

```rust
fn desktop_line(dir: &Path) -> String {
    let scan = DesktopReader::default().scan(dir);
    if let Some(problem) = scan.problem {
        return format!("cannot read {}: {problem}", dir.display());
    }
    if !scan.found {
        return format!("none found in {}", dir.display());
    }
    let mut line = format!(
        "{} · {}",
        dir.display(),
        plural(scan.files_read as u64, "session file", "session files")
    );
    if !scan.skipped.is_empty() {
        line.push_str(&format!(" · {} skipped", count(scan.skipped.len() as u64)));
    }
    line
}
```

- [ ] **Step 7: Run the tests to see them pass**

Run: `cargo test --test cli && cargo test --test daemon && cargo test --lib daemon`
Expected: PASS, including `doctor_on_a_fresh_home_creates_nothing` (doctor only reads the desktop folder).

- [ ] **Step 8: Run the gate**

Run: `cargo fmt && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Expected: clean.

- [ ] **Step 9: Commit**

```bash
git add tests/common/mod.rs tests/cli.rs tests/daemon.rs src/cli/import.rs src/cli/doctor.rs src/daemon/writer.rs src/daemon/mod.rs
git commit -m "feat(ingest): attribute desktop sessions on import and every daemon rescan, and report them in doctor

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Session filter, selection and the store queries behind assign

**Files:**
- Create: `src/assign.rs`
- Modify: `src/lib.rs` (add `pub mod assign;` after `pub mod api;`)
- Modify: `src/store/accounts.rs` (`Candidate`, `unknown_sessions`, `assigned_sessions`, `Writer::unassign`)
- Modify: `src/store/mod.rs` (re-export `Candidate`)

**Interfaces:**
- Consumes: `AttributionSource::Assign` (Task 1), `crate::history::calendar::local_midnight<Tz: TimeZone>(NaiveDate, &Tz) -> Result<i64>`.
- Produces:
  - `pub struct Candidate { pub id: String, pub cwd: Option<String>, pub started_at: Option<i64>, pub spend_usd: f64 }` (re-exported from `crate::store`)
  - `Store::unknown_sessions(&self) -> Result<Vec<Candidate>>`, `Store::assigned_sessions(&self) -> Result<Vec<Candidate>>`
  - `Writer::unassign(&mut self, session_id: &str) -> Result<bool>`
  - `pub struct Filter` with `pub fn new<Tz: TimeZone>(project: Option<String>, since: Option<&str>, until: Option<&str>, tz: &Tz) -> Result<Filter>` and `pub fn keeps(&self, candidate: &Candidate) -> bool`
  - `pub struct Selection { pub sessions: Vec<String>, pub first_ts: Option<i64>, pub last_ts: Option<i64>, pub spend_usd: f64 }` with `pub fn days<Tz: TimeZone>(&self, tz: &Tz) -> Option<(NaiveDate, NaiveDate)>`
  - `pub fn select(candidates: Vec<Candidate>, filter: &Filter) -> Selection`
  - `pub fn glob_matches(pattern: &str, text: &str) -> bool`
  - `pub fn expand_home(pattern: &str, home: &Path) -> String`

- [ ] **Step 1: Write the failing store test**

Add to the `tests` module of `src/store/accounts.rs`:

```rust
    #[test]
    fn unknown_and_assigned_sessions_carry_their_folder_start_and_spend() {
        use crate::store::RequestRow;
        use crate::transcript::TokenCounts;
        let mut store = Store::open_in_memory().unwrap();
        let mut w = store.writer().unwrap();
        for (id, cwd, ts) in [
            ("s1", Some("/w/a"), Some(10)),
            ("s2", None, None),
            ("s3", Some("/w/b"), Some(30)),
            ("s4", Some("/w/c"), Some(40)),
        ] {
            w.session(&SessionRow {
                id: id.into(),
                project: "p".into(),
                cwd: cwd.map(String::from),
                ts,
            })
            .unwrap();
        }
        for (key, session, cost, sidechain) in [
            ("r1", "s1", 1.5, false),
            ("r2", "s1", 0.25, true),
            ("r3", "s3", 2.0, false),
        ] {
            w.request(&RequestRow {
                key: key.into(),
                session_id: session.into(),
                ts: 10,
                sent_ts: None,
                model: "m".into(),
                tokens: TokenCounts::default(),
                cost_usd: Some(cost),
                is_sidechain: sidechain,
                agent_id: None,
                skill: None,
                mcp_server: None,
                effort: None,
            })
            .unwrap();
        }
        w.upsert_account(&oauth("a", "A"), 0).unwrap();
        w.attribute("s3", "oauth:a", AttributionSource::Assign).unwrap();
        w.attribute("s4", "oauth:a", AttributionSource::Desktop).unwrap();
        w.commit().unwrap();
        store
            .conn
            .execute(
                "INSERT INTO adjustments (session_id, model, ts, input, output, cache_read, cache_write, cost_usd)
                 VALUES ('s1', 'm', 12, 0, 0, 0, 0, 0.5)",
                [],
            )
            .unwrap();
        let candidate = |id: &str, cwd: Option<&str>, started_at, spend_usd| Candidate {
            id: id.into(),
            cwd: cwd.map(String::from),
            started_at,
            spend_usd,
        };
        assert_eq!(
            store.unknown_sessions().unwrap(),
            vec![
                candidate("s2", None, None, 0.0),
                candidate("s1", Some("/w/a"), Some(10), 2.25),
            ]
        );
        assert_eq!(
            store.assigned_sessions().unwrap(),
            vec![candidate("s3", Some("/w/b"), Some(30), 2.0)]
        );
        let mut w = store.writer().unwrap();
        assert!(!w.unassign("s4").unwrap());
        assert!(w.unassign("s3").unwrap());
        assert!(!w.unassign("s3").unwrap());
        w.commit().unwrap();
        assert_eq!(store.session_account("s3").unwrap(), None);
        assert_eq!(store.session_account("s4").unwrap().as_deref(), Some("oauth:a"));
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test --lib store::accounts::tests::unknown_and_assigned_sessions`
Expected: compile errors, `Candidate`, `unknown_sessions`, `assigned_sessions` and `unassign` not found.

- [ ] **Step 3: Implement the store side**

In `src/store/accounts.rs`, add after the `Budget` struct:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub cwd: Option<String>,
    pub started_at: Option<i64>,
    pub spend_usd: f64,
}

const CANDIDATES: &str = "SELECT s.id, s.cwd, s.started_at,
        COALESCE((SELECT SUM(r.cost_usd) FROM requests r WHERE r.session_id = s.id), 0.0)
      + COALESCE((SELECT SUM(j.cost_usd) FROM adjustments j WHERE j.session_id = s.id), 0.0)
     FROM sessions s LEFT JOIN session_accounts a ON a.session_id = s.id";
```

Add to `impl Writer<'_>` after `attribute`:

```rust
    pub fn unassign(&mut self, session_id: &str) -> Result<bool> {
        let removed = self
            .tx
            .prepare_cached(
                "DELETE FROM session_accounts WHERE session_id = ?1 AND source = 'assign'",
            )?
            .execute([session_id])?;
        Ok(removed > 0)
    }
```

Add to `impl Store` after `unattributed_sessions`:

```rust
    pub fn unknown_sessions(&self) -> Result<Vec<Candidate>> {
        self.candidates("a.session_id IS NULL")
    }

    pub fn assigned_sessions(&self) -> Result<Vec<Candidate>> {
        self.candidates("a.source = 'assign'")
    }

    fn candidates(&self, condition: &str) -> Result<Vec<Candidate>> {
        let mut statement = self.conn.prepare_cached(&format!(
            "{CANDIDATES} WHERE {condition} ORDER BY s.started_at, s.id"
        ))?;
        let rows = statement.query_map([], |r| {
            Ok(Candidate {
                id: r.get(0)?,
                cwd: r.get(1)?,
                started_at: r.get(2)?,
                spend_usd: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
```

In `src/store/mod.rs`, change the re-export to:

```rust
pub use accounts::{AccountInfo, AccountKind, AccountRow, AttributionSource, Budget, Candidate};
```

Run: `cargo test --lib store::accounts`
Expected: PASS.

- [ ] **Step 4: Write the failing filter tests**

Create `src/assign.rs` with only the test module, and add `pub mod assign;` to `src/lib.rs` after `pub mod api;`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, Utc};

    fn at(text: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(text).unwrap().timestamp()
    }

    fn session(id: &str, cwd: Option<&str>, started: Option<&str>, spend_usd: f64) -> Candidate {
        Candidate {
            id: id.into(),
            cwd: cwd.map(String::from),
            started_at: started.map(at),
            spend_usd,
        }
    }

    fn project(pattern: &str) -> Filter {
        Filter::new(Some(pattern.into()), None, None, &Utc).unwrap()
    }

    #[test]
    fn a_star_matches_any_characters_slashes_included() {
        assert!(glob_matches("*/private/*", "/Users/me/private/notes/app"));
        assert!(glob_matches("/Users/me/*", "/Users/me/a/b/c"));
        assert!(glob_matches("*", ""));
        assert!(glob_matches("/w/*-api", "/w/x/billing-api"));
        assert!(glob_matches("/w/demo", "/w/demo"));
        assert!(!glob_matches("/w/demo", "/w/demo/sub"));
        assert!(!glob_matches("/w/*-api", "/w/billing-api/src"));
        assert!(!glob_matches("*/private/*", "/Users/me/privateer"));
        assert!(glob_matches("/w/a?b[c]", "/w/a?b[c]"));
    }

    #[test]
    fn a_session_without_a_working_directory_never_matches_a_project() {
        assert!(!project("*").keeps(&session("s", None, Some("2026-10-01T00:00:00Z"), 0.0)));
        assert!(project("*").keeps(&session("s", Some("/x"), None, 0.0)));
    }

    #[test]
    fn days_are_local_and_both_bounds_are_inclusive() {
        let paris = FixedOffset::east_opt(2 * 3600).unwrap();
        let filter = Filter::new(None, Some("2026-10-02"), Some("2026-10-03"), &paris).unwrap();
        let keeps = |started: &str| filter.keeps(&session("s", Some("/x"), Some(started), 0.0));
        assert!(!keeps("2026-10-01T21:59:59Z"));
        assert!(keeps("2026-10-01T22:30:00Z"));
        assert!(keeps("2026-10-03T21:59:59Z"));
        assert!(!keeps("2026-10-03T22:00:00Z"));
        assert!(!filter.keeps(&session("s", Some("/x"), None, 0.0)));
        let one_day = Filter::new(None, Some("2026-10-02"), Some("2026-10-02"), &paris).unwrap();
        assert!(one_day.keeps(&session("s", None, Some("2026-10-02T21:00:00Z"), 0.0)));
    }

    #[test]
    fn a_filter_needs_a_project_or_a_day_and_valid_ordered_dates() {
        let error = |project: Option<&str>, since: Option<&str>, until: Option<&str>| {
            Filter::new(project.map(String::from), since, until, &Utc)
                .unwrap_err()
                .to_string()
        };
        assert_eq!(
            error(None, None, None),
            "pass --project, --since or --until to choose the sessions"
        );
        assert_eq!(
            error(None, Some("2026-13-01"), None),
            "--since takes a day as YYYY-MM-DD, not 2026-13-01"
        );
        assert_eq!(
            error(None, None, Some("yesterday")),
            "--until takes a day as YYYY-MM-DD, not yesterday"
        );
        assert_eq!(
            error(None, Some("2026-10-05"), Some("2026-10-04")),
            "--until 2026-10-04 is before --since 2026-10-05"
        );
        assert!(Filter::new(None, None, Some("2026-10-04"), &Utc).is_ok());
    }

    #[test]
    fn a_selection_counts_its_sessions_days_and_spend() {
        let candidates = vec![
            session("a", Some("/w/one"), Some("2026-06-03T10:00:00Z"), 1.25),
            session("b", Some("/elsewhere"), Some("2026-07-01T10:00:00Z"), 9.0),
            session("c", Some("/w/two"), None, 0.5),
            session("d", Some("/w/three"), Some("2026-10-11T23:30:00Z"), 2.0),
        ];
        let selection = select(candidates, &project("/w/*"));
        assert_eq!(selection.sessions, vec!["a", "c", "d"]);
        assert_eq!(selection.spend_usd, 3.75);
        let day = |text: &str| NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap();
        assert_eq!(selection.days(&Utc), Some((day("2026-06-03"), day("2026-10-11"))));
        let paris = FixedOffset::east_opt(2 * 3600).unwrap();
        assert_eq!(selection.days(&paris), Some((day("2026-06-03"), day("2026-10-12"))));
        let undated = select(vec![session("c", Some("/w/two"), None, 0.5)], &project("/w/*"));
        assert_eq!(undated.days(&Utc), None);
    }

    #[test]
    fn a_leading_tilde_is_the_home_folder() {
        let home = Path::new("/Users/me");
        assert_eq!(expand_home("~/private/*", home), "/Users/me/private/*");
        assert_eq!(expand_home("~", home), "/Users/me");
        assert_eq!(expand_home("/w/~/x", home), "/w/~/x");
        assert_eq!(expand_home("~other/x", home), "~other/x");
    }
}
```

- [ ] **Step 5: Run the tests to see them fail**

Run: `cargo test --lib assign`
Expected: compile errors, `Filter`, `select`, `glob_matches` and `expand_home` not found.

- [ ] **Step 6: Implement the filter and the selection**

Put this above the test module in `src/assign.rs`:

```rust
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Days, NaiveDate, TimeZone};

use crate::history::calendar::local_midnight;
use crate::store::Candidate;

#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    project: Option<String>,
    since_ts: Option<i64>,
    until_ts: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    pub sessions: Vec<String>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub spend_usd: f64,
}

impl Filter {
    pub fn new<Tz: TimeZone>(
        project: Option<String>,
        since: Option<&str>,
        until: Option<&str>,
        tz: &Tz,
    ) -> Result<Self> {
        if project.is_none() && since.is_none() && until.is_none() {
            bail!("pass --project, --since or --until to choose the sessions");
        }
        let since = since.map(|text| day("--since", text)).transpose()?;
        let until = until.map(|text| day("--until", text)).transpose()?;
        if let (Some(since), Some(until)) = (since, until)
            && until < since
        {
            bail!("--until {until} is before --since {since}");
        }
        let since_ts = since.map(|day| local_midnight(day, tz)).transpose()?;
        let until_ts = until
            .map(|day| {
                let next = day
                    .checked_add_days(Days::new(1))
                    .context("--until is the last supported day")?;
                local_midnight(next, tz)
            })
            .transpose()?;
        Ok(Self {
            project,
            since_ts,
            until_ts,
        })
    }

    pub fn keeps(&self, candidate: &Candidate) -> bool {
        let in_project = self.project.as_deref().is_none_or(|pattern| {
            candidate
                .cwd
                .as_deref()
                .is_some_and(|cwd| glob_matches(pattern, cwd))
        });
        let undated = self.since_ts.is_none() && self.until_ts.is_none();
        let in_days = undated
            || candidate.started_at.is_some_and(|started| {
                self.since_ts.is_none_or(|since| started >= since)
                    && self.until_ts.is_none_or(|until| started < until)
            });
        in_project && in_days
    }
}

impl Selection {
    pub fn days<Tz: TimeZone>(&self, tz: &Tz) -> Option<(NaiveDate, NaiveDate)> {
        let local = |ts: i64| {
            DateTime::from_timestamp(ts, 0).map(|utc| utc.with_timezone(tz).date_naive())
        };
        Some((local(self.first_ts?)?, local(self.last_ts?)?))
    }
}

pub fn select(candidates: Vec<Candidate>, filter: &Filter) -> Selection {
    let kept: Vec<Candidate> = candidates
        .into_iter()
        .filter(|candidate| filter.keeps(candidate))
        .collect();
    Selection {
        first_ts: kept.iter().filter_map(|c| c.started_at).min(),
        last_ts: kept.iter().filter_map(|c| c.started_at).max(),
        spend_usd: kept.iter().map(|c| c.spend_usd).sum(),
        sessions: kept.into_iter().map(|c| c.id).collect(),
    }
}

pub fn glob_matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if pattern.get(p) == Some(&'*') {
            star = Some((p, t));
            p += 1;
        } else if pattern.get(p) == Some(&text[t]) {
            p += 1;
            t += 1;
        } else if let Some((star_p, star_t)) = star {
            p = star_p + 1;
            t = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|ch| *ch == '*')
}

pub fn expand_home(pattern: &str, home: &Path) -> String {
    match pattern.strip_prefix('~') {
        Some("") => home.display().to_string(),
        Some(rest) if rest.starts_with('/') => format!("{}{rest}", home.display()),
        _ => pattern.to_string(),
    }
}

fn day(flag: &str, text: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .map_err(|_| anyhow!("{flag} takes a day as YYYY-MM-DD, not {text}"))
}
```

- [ ] **Step 7: Run the tests to see them pass**

Run: `cargo test --lib assign && cargo test --lib store::accounts`
Expected: PASS.

- [ ] **Step 8: Run the gate**

Run: `cargo fmt && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Expected: clean.

- [ ] **Step 9: Commit**

```bash
git add src/assign.rs src/lib.rs src/store/accounts.rs src/store/mod.rs
git commit -m "feat(store): select Unknown and assigned sessions by project glob and local days

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `accounts assign` and `accounts unassign`

**Files:**
- Modify: `src/cli/mod.rs` (`AccountsAction::Assign`, `AccountsAction::Unassign`, `SelectionArgs`, the `Accounts` about)
- Modify: `src/cli/accounts.rs` (dispatch, `assign`, `unassign`, `assignable`, `selection_lines`)
- Test: `tests/cli.rs`

**Interfaces:**
- Consumes: `Filter::new`, `select`, `Selection::days`, `expand_home` (Task 4); `Store::unknown_sessions`, `Store::assigned_sessions`, `Writer::unassign` (Task 4); `Writer::attribute(.., AttributionSource::Assign)` (Task 1); `resolve`, `imported` and `super::open_store` already in `src/cli/accounts.rs` and `src/cli/mod.rs`.
- Produces: the commands `claude-plus accounts assign <account> [--project <glob>] [--since <date>] [--until <date>] [--dry-run]` and `claude-plus accounts unassign [--project <glob>] [--since <date>] [--until <date>] [--dry-run]`.

- [ ] **Step 1: Write the failing end-to-end tests**

Add to `tests/cli.rs`:

```rust
const DEMO_SESSION_2: &str = "22222222-2222-2222-2222-222222222222";

#[test]
fn assign_previews_writes_then_selects_nothing_and_unassign_restores_unknown() {
    let env = env();
    env.seed_accounts(&[("oauth:me", "Me · Max")]);
    let assign = |extra: &[&str]| {
        stdout_of(&env, &[&["accounts", "assign", "me · max", "--project", "/work/*"][..], extra].concat())
    };
    assert_eq!(
        assign(&["--dry-run"]),
        "Would assign 2 sessions to Me · Max\n  2026-10-01 to 2026-10-03 · $0.0812\n"
    );
    assert_eq!(env.attribution(DEMO_SESSION), None);
    assert_eq!(
        assign(&[]),
        "Assigned 2 sessions to Me · Max\n  2026-10-01 to 2026-10-03 · $0.0812\n"
    );
    for session in [DEMO_SESSION, DEMO_SESSION_2] {
        assert_eq!(
            env.attribution(session),
            Some(("oauth:me".into(), "assign".into()))
        );
    }
    assert_eq!(assign(&[]), "No Unknown sessions match\n");
    assert_eq!(
        stdout_of(&env, &["stats", "--account", "Me · Max"]),
        STATS_TABLE.replacen("Account: All accounts", "Account: Me · Max", 1)
    );
    assert_eq!(
        stdout_of(&env, &["accounts", "unassign", "--project", "/work/*", "--dry-run"]),
        "Would unassign 2 sessions\n  2026-10-01 to 2026-10-03 · $0.0812\n"
    );
    assert_eq!(env.attribution(DEMO_SESSION), Some(("oauth:me".into(), "assign".into())));
    assert_eq!(
        stdout_of(&env, &["accounts", "unassign", "--project", "/work/*"]),
        "Unassigned 2 sessions\n  2026-10-01 to 2026-10-03 · $0.0812\n"
    );
    assert_eq!(env.attribution(DEMO_SESSION), None);
    assert_eq!(
        stdout_of(&env, &["accounts", "unassign", "--project", "/work/*"]),
        "No assigned sessions match\n"
    );
}

#[test]
fn assign_takes_inclusive_days_and_leaves_other_attributions_alone() {
    let env = env();
    env.seed_accounts(&[("oauth:me", "Me · Max"), ("oauth:work", "Acme · Enterprise")]);
    cmd(&env, &["import"]).assert().success();
    env.seed_start(DEMO_SESSION, "oauth:work");
    let day = stdout_of(
        &env,
        &["accounts", "assign", "oauth:me", "--since", "2026-10-01", "--until", "2026-10-03"],
    );
    assert!(
        day.starts_with("Assigned 1 session to Me · Max\n  2026-10-03 · $"),
        "{day}"
    );
    assert_eq!(env.attribution(DEMO_SESSION), Some(("oauth:work".into(), "start".into())));
    assert_eq!(env.attribution(DEMO_SESSION_2), Some(("oauth:me".into(), "assign".into())));
    cmd(&env, &["accounts", "unassign", "--until", "2026-10-03"]).assert().success();
    assert_eq!(env.attribution(DEMO_SESSION), Some(("oauth:work".into(), "start".into())));
    assert_eq!(env.attribution(DEMO_SESSION_2), None);
}

#[test]
fn assign_refuses_a_bare_command_bad_days_and_an_unknown_account_before_writing() {
    let env = env();
    env.seed_accounts(&[("oauth:me", "Me · Max")]);
    let fails = |args: &[&str], stderr: &str| {
        cmd(&env, args).assert().failure().stderr(format!("claude-plus: {stderr}\n"));
    };
    fails(
        &["accounts", "assign", "oauth:me"],
        "pass --project, --since or --until to choose the sessions",
    );
    fails(
        &["accounts", "unassign", "--dry-run"],
        "pass --project, --since or --until to choose the sessions",
    );
    fails(
        &["accounts", "assign", "oauth:me", "--since", "03/10/2026"],
        "--since takes a day as YYYY-MM-DD, not 03/10/2026",
    );
    fails(
        &["accounts", "assign", "oauth:me", "--since", "2026-10-03", "--until", "2026-10-01"],
        "--until 2026-10-01 is before --since 2026-10-03",
    );
    fails(
        &["accounts", "assign", "nobody", "--project", "*"],
        "no account called \"nobody\"; the accounts are Me · Max (oauth:me)",
    );
    assert_eq!(env.attribution(DEMO_SESSION), None);
}

#[test]
fn assign_with_no_account_known_says_so() {
    let env = env();
    cmd(&env, &["accounts", "assign", "api", "--project", "*"])
        .assert()
        .failure()
        .stderr("claude-plus: no account called \"api\"; no account is known yet, see claude-plus accounts\n");
}

#[test]
fn accounts_help_lists_assign_and_unassign() {
    let env = env();
    let help = stdout_of(&env, &["accounts", "--help"]);
    assert!(help.contains("Attribute Unknown sessions to an account by project and days"), "{help}");
    assert!(help.contains("Make sessions given with assign Unknown again"), "{help}");
    let assign = stdout_of(&env, &["accounts", "assign", "--help"]);
    for flag in ["--project <GLOB>", "--since <DAY>", "--until <DAY>", "--dry-run"] {
        assert!(assign.contains(flag), "missing {flag} in\n{assign}");
    }
}
```

The fixture's two sessions both run in `/work/demo`; `DEMO_SESSION` starts on 2026-10-01 and `DEMO_SESSION_2` on 2026-10-03, and their spend adds up to the `$0.0812` total of `STATS_TABLE`.
If the `$0.0812` lines fail with a different figure, the per-session sum disagrees with `stats`; find out why before touching the expectation.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --test cli assign && cargo test --test cli accounts_help`
Expected: FAIL, clap rejects the unknown subcommand `assign`.

- [ ] **Step 3: Add the subcommands**

In `src/cli/mod.rs`, change the `Accounts` about:

```rust
    #[command(about = "List the accounts claude-plus has seen, rename one, or assign Unknown sessions to one")]
    Accounts {
```

Replace `enum AccountsAction` with:

```rust
#[derive(Subcommand)]
enum AccountsAction {
    #[command(about = "Give an account your own label")]
    Rename { account: String, label: String },
    #[command(about = "Attribute Unknown sessions to an account by project and days")]
    Assign {
        account: String,
        #[command(flatten)]
        selection: SelectionArgs,
    },
    #[command(about = "Make sessions given with assign Unknown again")]
    Unassign {
        #[command(flatten)]
        selection: SelectionArgs,
    },
}

#[derive(clap::Args)]
struct SelectionArgs {
    #[arg(
        long,
        value_name = "GLOB",
        help = "Working directory of the session, where * matches anything, / included"
    )]
    project: Option<String>,
    #[arg(long, value_name = "DAY", help = "First day, YYYY-MM-DD in local time")]
    since: Option<String>,
    #[arg(long, value_name = "DAY", help = "Last day, YYYY-MM-DD in local time, included")]
    until: Option<String>,
    #[arg(long, help = "Show what would change and write nothing")]
    dry_run: bool,
}
```

- [ ] **Step 4: Implement assign and unassign**

In `src/cli/accounts.rs`, change the imports to:

```rust
use anyhow::{Context, Result, anyhow, bail};
use chrono::Local;
use comfy_table::{Cell, CellAlignment, Table, presets::UTF8_FULL_CONDENSED};

use super::format::{column_decimals, count, plural, usd, usd_with};
use super::{AccountsAction, BudgetAction, SelectionArgs};
use crate::accounts::{Auth, auth_account};
use crate::assign::{Filter, Selection, expand_home, select};
use crate::limits::{SPEND_LIMIT, month_start_utc, monthly_pace, next_month_start_utc};
use crate::paths::Paths;
use crate::pricing::PriceTable;
use crate::scope::Scope;
use crate::store::{AccountInfo, AccountRow, AttributionSource, Store};
```

Replace the body of `accounts`:

```rust
pub fn accounts(paths: &Paths, prices: &PriceTable, action: Option<AccountsAction>) -> Result<()> {
    match action {
        Some(AccountsAction::Rename { account, label }) => rename(paths, &account, &label),
        Some(AccountsAction::Assign { account, selection }) => {
            assign(paths, prices, &account, &selection)
        }
        Some(AccountsAction::Unassign { selection }) => unassign(paths, prices, &selection),
        None => list(paths, prices),
    }
}
```

Add after `rename`:

```rust
fn filter_of(paths: &Paths, selection: &SelectionArgs) -> Result<Filter> {
    Filter::new(
        selection
            .project
            .as_deref()
            .map(|pattern| expand_home(pattern, &paths.home)),
        selection.since.as_deref(),
        selection.until.as_deref(),
        &Local,
    )
}

fn assign(paths: &Paths, prices: &PriceTable, given: &str, selection: &SelectionArgs) -> Result<()> {
    let filter = filter_of(paths, selection)?;
    imported(paths, prices)?;
    let mut store = super::open_store(paths)?;
    let account = assignable(&store, given)?;
    let chosen = select(store.unknown_sessions()?, &filter);
    if chosen.sessions.is_empty() {
        println!("No Unknown sessions match");
        return Ok(());
    }
    if !selection.dry_run {
        let mut writer = store.writer()?;
        for session in &chosen.sessions {
            writer.attribute(session, &account.id, AttributionSource::Assign)?;
        }
        writer.commit()?;
    }
    let verb = if selection.dry_run { "Would assign" } else { "Assigned" };
    println!(
        "{verb} {} to {}",
        plural(chosen.sessions.len() as u64, "session", "sessions"),
        account.label
    );
    println!("{}", selection_line(&chosen));
    Ok(())
}

fn unassign(paths: &Paths, prices: &PriceTable, selection: &SelectionArgs) -> Result<()> {
    let filter = filter_of(paths, selection)?;
    imported(paths, prices)?;
    let mut store = super::open_store(paths)?;
    let chosen = select(store.assigned_sessions()?, &filter);
    if chosen.sessions.is_empty() {
        println!("No assigned sessions match");
        return Ok(());
    }
    if !selection.dry_run {
        let mut writer = store.writer()?;
        for session in &chosen.sessions {
            writer.unassign(session)?;
        }
        writer.commit()?;
    }
    let verb = if selection.dry_run { "Would unassign" } else { "Unassigned" };
    println!(
        "{verb} {}",
        plural(chosen.sessions.len() as u64, "session", "sessions")
    );
    println!("{}", selection_line(&chosen));
    Ok(())
}

fn assignable(store: &Store, given: &str) -> Result<AccountInfo> {
    let accounts = store.accounts()?;
    let wanted = given.to_lowercase();
    let known = accounts
        .iter()
        .any(|a| a.id == given || a.label.to_lowercase() == wanted);
    if !known && accounts.is_empty() {
        bail!("no account called \"{given}\"; no account is known yet, see claude-plus accounts");
    }
    if !known {
        bail!(
            "no account called \"{given}\"; the accounts are {}",
            accounts
                .iter()
                .map(|a| format!("{} ({})", a.label, a.id))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let id = resolve(store, Some(given))?;
    accounts
        .into_iter()
        .find(|a| a.id == id)
        .with_context(|| format!("no account called \"{given}\"; see claude-plus accounts"))
}

fn selection_line(chosen: &Selection) -> String {
    let spend = usd(chosen.spend_usd);
    match chosen.days(&Local) {
        Some((first, last)) if first == last => format!("  {first} · {spend}"),
        Some((first, last)) => format!("  {first} to {last} · {spend}"),
        None => format!("  {spend}"),
    }
}
```

If clippy flags `count` or `anyhow` as unused after the import change, keep only what the file uses.

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test --test cli assign && cargo test --test cli accounts && cargo test --test cli help_describes_every_command`
Expected: PASS.

- [ ] **Step 6: Run the gate**

Run: `cargo fmt && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add src/cli/mod.rs src/cli/accounts.rs tests/cli.rs
git commit -m "feat(cli): assign Unknown sessions to an account by project and days, and unassign them

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: README, accounts spec and the attribution manual check

**Files:**
- Modify: `README.md` (section `## Accounts and budgets`, table under `## Commands`)
- Modify: `docs/superpowers/specs/2026-10-11-accounts-and-budgets-design.md` (non-goal, attribution order)
- Create: `docs/manual-checks/m9-attribution.md`

**Interfaces:**
- Consumes: the command names, flags and output wording of Tasks 3 and 5.
- Produces: user documentation; no code.

- [ ] **Step 1: Update the README**

In `README.md`, replace the line

```
Sessions from before the upgrade show as Unknown under All accounts.
```

with:

````markdown
A session is Unknown when claude-plus cannot tell its account, which is the case for most sessions recorded before the accounts upgrade.
Unknown sessions count under All accounts only, so History, `stats` and the budget of an account leave them out.

Sessions run in the Claude desktop app are attributed to the account the desktop app is signed in to, even when the CLI is signed in to another one.
claude-plus reads the session list the desktop app keeps in `~/Library/Application Support/Claude/claude-code-sessions` (`~/.config/Claude` on Linux) on every import and every minute in the daemon, and only reads it.
An account seen only there is called `Desktop account <first 8 characters of its id>` until the CLI signs in to it or you rename it.
`claude-plus doctor` shows how many desktop session files it read and how many it skipped.

You attribute the remaining Unknown sessions yourself, by project folder and days:

```bash
claude-plus accounts assign "Me · Max" --project '~/private/*' --dry-run
claude-plus accounts assign "Me · Max" --project '~/private/*'
claude-plus accounts assign "Acme · Enterprise" --since 2026-06-01 --until 2026-09-30
```

`--project` matches the session's working directory, where `*` matches anything, `/` included; quote it so the shell leaves it alone.
`--since` and `--until` are days in local time, both included.
`assign` only touches Unknown sessions and never overrides an attribution claude-plus made itself.
`--dry-run` prints what would change, and `claude-plus accounts unassign` with the same options makes assigned sessions Unknown again.
````

In the `## Commands` table, replace the `accounts` row with:

```
| `claude-plus accounts [rename <account> <label>]` | List the accounts claude-plus knows with this month's spend, or rename one. |
| `claude-plus accounts assign <account> [--project <glob>] [--since <day>] [--until <day>] [--dry-run]` | Attribute Unknown sessions to an account. |
| `claude-plus accounts unassign [--project <glob>] [--since <day>] [--until <day>] [--dry-run]` | Make assigned sessions Unknown again. |
```

Check that the README section reads in order (Unknown, desktop, assign, budgets, prices) and that every sentence is on its own line.

- [ ] **Step 2: Update the accounts spec**

In `docs/superpowers/specs/2026-10-11-accounts-and-budgets-design.md`, replace the non-goal

```
- Attributing sessions recorded before the upgrade to an account; they stay "Unknown".
```

with:

```
- Attributing sessions recorded before the upgrade automatically from the timeline; they stay "Unknown" unless the desktop app recorded their account or the user assigns them, see `2026-10-12-session-attribution-backfill-design.md`.
```

Replace

```
A `session.start` attribution replaces a timeline one, and neither is recomputed later.
```

with:

```
Two more sources attribute a session, from `2026-10-12-session-attribution-backfill-design.md`:

- `desktop`: the Claude desktop app's own record of the account it ran the session under.
- `assign`: the user's `claude-plus accounts assign`.

A `desktop` attribution replaces any attribution to an `oauth` account and never a `cloud`, `gateway` or `api` one.
A `session.start` attribution replaces a `timeline` or `assign` one, and never a `desktop` one.
`timeline` and `assign` only attribute a session that has none.
Nothing else is recomputed later.
```

- [ ] **Step 3: Write the manual check**

Create `docs/manual-checks/m9-attribution.md`:

````markdown
# Manual check for session attribution

Run this by hand in a real terminal on a machine that has months of Claude Code history, after Tasks 1 to 5 of the session attribution backfill plan are on `main`.
It needs the Claude desktop app signed in to a company account and the CLI signed in to a private one.

## Run this build

Build the release binary and put it first on `PATH`, so the pane and the CLI both use this build and not the Homebrew one.

```bash
cargo build --release --locked
export PATH="$PWD/target/release:$PATH"
claude-plus daemon stop
claude-plus --version
```

- [ ] Before anything else, copy the database so the check can be repeated: `cp ~/.claude-plus/data.db /tmp/data-before.db`.
- [ ] Note the Unknown total: `claude-plus stats --days 31 --account all` and `claude-plus accounts`.

## Checks

- [ ] Run `claude-plus doctor`: the `Desktop:` line names `~/Library/Application Support/Claude/claude-code-sessions` with a number of session files, and any skipped ones are worth a look in the files themselves.
- [ ] Run `claude-plus import`: it prints the usual report and at most one `Skipped the desktop session file ...` line per broken file.
- [ ] Run `claude-plus accounts`: the desktop app's account appears with its CLI label if the CLI ever signed in to it, else as `Desktop account <8 characters>`, and its Sessions count matches the desktop app's session list.
- [ ] Pick a session you ran in the desktop app while the CLI was signed in to the private account; `sqlite3 ~/.claude-plus/data.db "SELECT account_id, source FROM session_accounts WHERE session_id = '<cliSessionId>'"` shows the desktop account and `desktop`.
- [ ] Run `claude-plus accounts assign "<private label>" --project '~/<private folder>/*' --dry-run`: it says `Would assign <n> sessions to <private label>` with a date range and spend that look right, and `claude-plus accounts` is unchanged.
- [ ] Run the same command without `--dry-run`: it says `Assigned <n> sessions ...`, and running it again says `No Unknown sessions match`.
- [ ] Assign the company period: `claude-plus accounts assign "<company label>" --since <first day> --until <last day>`.
- [ ] Run `claude-plus stats --days 31` and `claude-plus stats --days 31 --account "<company label>"`: the private and company totals now add up to most of what was Unknown.
- [ ] Open the History view in the pane and press `a`: each account shows its own history and the Unknown note shrinks.
- [ ] Run `claude-plus accounts unassign --project '~/<private folder>/*' --dry-run`, then without `--dry-run`: the sessions are Unknown again, and assigning them once more restores the totals.
- [ ] Start a new session in the desktop app and send one prompt: within a minute the pane shows the company account for it, not the CLI's private one.
- [ ] Confirm nothing was written to the desktop app's folder: `ls -lT ~/Library/Application\ Support/Claude/claude-code-sessions/*/*/ | head` shows no modification time from this check other than the desktop app's own.

## Outcome

If every line holds, nothing is committed.
Otherwise fix each defect in its owning task's files with a test that pins it, commit it on its own as `fix(<unit>): ...`, and repeat the check.
````

- [ ] **Step 4: Check the docs**

Run: `grep -n "$(printf '\342\200\224')" README.md docs/manual-checks/m9-attribution.md docs/superpowers/specs/2026-10-11-accounts-and-budgets-design.md`
Expected: no output.
Read each edited paragraph once more and split any line that holds two sentences.

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo +1.88 check --all-targets`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add README.md docs/superpowers/specs/2026-10-11-accounts-and-budgets-design.md docs/manual-checks/m9-attribution.md
git commit -m "docs: explain Unknown, desktop attribution and accounts assign, and add the attribution manual check

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Self-review

Spec coverage, section by section:

- 1 Purpose and success criteria: desktop sessions past and live (Tasks 2, 3), the CLI misattribution corrected (Task 3 `import_moves_a_desktop_session_off_the_cli_login_to_the_desktop_account`), assign with preview and undo (Task 5), environment attributions kept (Task 1 `a_desktop_attribution_keeps_a_cloud_gateway_or_api_row`), no network and no writes to foreign files (Task 3 byte comparison of the desktop file, Global Constraints).
- 3 Location: Task 2 `desktop_dir_in`; Windows is Decision 2. Reading and skips: Task 2. Scan moments: Task 3 (local import, daemon `Scan` job, which runs at startup, on each rescan and for imports over the socket). Stamp cache: Task 2. Unknown desktop accounts and labels: Task 2 and Decision 3. Precedence table: Task 1, one test per row.
- 4 `assign` and `unassign`: Tasks 4 and 5, including resolution, the required filter, the glob, local inclusive days, dry run, empty selection, spend and writes through the store writer.
- 5 Data model: no migration (Global Constraints).
- 6 Doctor: Task 3 and Decision 14.
- 7 Error handling: unreadable folder (Task 2 test, Task 3 wiring), gateway kept (Task 1), unknown account (Task 5), invalid and reversed days (Tasks 4 and 5).
- 8 Documentation: Task 6.
- 9 Testing: store rows (Task 1), reader (Task 2), filters (Task 4), CLI end-to-end (Tasks 3, 5), daemon rescan (Task 3), manual check (Task 6).

Types used across tasks: `AttributionSource::{Desktop, Assign}` (Task 1) in Tasks 2, 4, 5; `DesktopReader`, `DesktopScan.{found, files_read, sessions, skipped, problem}`, `Skipped.{path, reason}` (Task 2) in Task 3; `Candidate` (Task 4) in `assign.rs` and Task 5; `Filter::new(Option<String>, Option<&str>, Option<&str>, &Tz)`, `select`, `Selection::days`, `expand_home` (Task 4) in Task 5; `writer::spawn` gains `desktop_sessions: PathBuf` in both callers (Task 3).
