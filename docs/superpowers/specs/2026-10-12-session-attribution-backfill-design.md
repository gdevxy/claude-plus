# claude-plus - Session Attribution Backfill Design Spec

Date: 2026-10-12
Status: Approved in brainstorming, pending written-spec review

## 1. Purpose

The accounts upgrade attributes a session to an account only from a live `session.start` event or from the OAuth login timeline.
Sessions recorded before the upgrade have neither, so they stay "Unknown".
On a real machine that is almost all history: about $3,200 this month sits under Unknown while the current account shows 3 sessions.
History, `stats` and the budget view default to the current account, so the history looks empty.

A second problem showed up while investigating.
The Claude desktop app signs in on its own, separately from the CLI's `.claude.json`.
A desktop session's `session.start` is resolved against the CLI login, so a desktop session run on an Enterprise seat is filed under the CLI's private subscription.

Transcripts do not record the account.
The desktop app does: it stores each session's metadata under `claude-code-sessions/<accountUuid>/<orgUuid>/`, with the CLI session ids it ran.
Everything else can only be attributed by the user, who knows which repositories and periods belong to which account.

### Success criteria

- Every desktop session, past or live, is attributed to the account the desktop app ran it under.
- A desktop session wrongly attributed to the CLI's OAuth account is corrected.
- A user can attribute Unknown sessions by project and date range in one command, preview the result, and undo it.
- An explicit attribution from the environment (cloud, gateway, API key) is never overridden.
- claude-plus still makes no network requests and never edits Claude Code's or the desktop app's files.

### Non-goals

- Guessing an account from models, tools, time of day or rate-limit errors.
  These reflect a user's habits, not the account, and a confident wrong account is worse than Unknown.
- Overriding a `start` or `timeline` attribution from the CLI with `assign`.
- Stored assignment rules that apply to transcripts appearing later.
  `session_accounts` already survives `reindex`, and late old transcripts are rare.

## 2. Terms

- Desktop session file: one JSON file under the desktop app's `claude-code-sessions` folder, describing one desktop session.
- Desktop attribution: an attribution read from a desktop session file, with source `desktop`.
- Assignment: an attribution the user made with `accounts assign`, with source `assign`.

## 3. Desktop attribution

### Location

The desktop app's data folder is derived from `Paths::home`, or from `%APPDATA%` on Windows:

- macOS: `<home>/Library/Application Support/Claude`
- Windows: `%APPDATA%\Claude`
- Linux: `<home>/.config/Claude`

Session files are `claude-code-sessions/<accountUuid>/<orgUuid>/*.json`.
A missing folder means there is nothing to read and is not logged.

### Reading

From each file claude-plus reads `cliSessionId` and `priorCliSessionIds`, and nothing else.
Every listed id is attributed to `oauth:<accountUuid>`, where `<accountUuid>` is the file's grandparent folder name.
A folder name that is not a UUID, a file that is not valid JSON, or a file without `cliSessionId` is skipped and logged once per run.
Ids that match no imported session are kept anyway, so a transcript imported later is already attributed.

The scan runs:

- in `import`, after transcripts are imported;
- in the daemon, at startup and on every periodic rescan (60 seconds).

A file is re-read only when its modification time or size changed since the last scan in that process.

### Unknown desktop accounts

When `oauth:<accountUuid>` is not in `accounts`, it is created with kind `oauth`.
Its label is `Desktop account <first 8 characters of the uuid>`, since the desktop app does not store the organization name.
When the CLI later signs in to the same account, the existing upsert replaces the automatic label with the usual `<organizationName> · <plan>`.
`accounts rename` overrides it either way.

### Precedence

`session_accounts.source` gains two values, `desktop` and `assign`.
Writes follow these rules:

| Incoming | Replaces |
|---|---|
| `desktop` | no row, or a row whose account kind is `oauth` (any source) |
| `start` | no row, `timeline`, `assign` |
| `timeline` | no row |
| `assign` | no row |

`desktop` never replaces a `cloud`, `gateway` or `api` attribution, since the environment named that account explicitly.
`start` never replaces `desktop`, so a resumed desktop session keeps its desktop account.
`start` replaces `assign` because a live event is evidence and an assignment is the user's recollection.

The timeline's pending list (`unattributed_sessions`) already excludes any session with a row, so desktop and assigned sessions are never re-attributed by it.

## 4. `accounts assign` and `accounts unassign`

```
claude-plus accounts assign <account> [--project <glob>] [--since <date>] [--until <date>] [--dry-run]
claude-plus accounts unassign [--project <glob>] [--since <date>] [--until <date>] [--dry-run]
```

- `<account>` resolves like every other account argument: an exact id or a case-insensitive label, an ambiguous label is an error listing the matches.
- At least one of `--project`, `--since` and `--until` is required, so a bare command cannot sweep everything.
- `--project` is a glob matched against the session's working directory (`sessions.cwd`), where `*` matches any characters including `/`.
  A session without a working directory never matches `--project`.
- `--since` and `--until` take `YYYY-MM-DD` in local time, both inclusive, compared with the session's start time.
- `assign` selects only sessions with no `session_accounts` row and writes `source = 'assign'`.
- `unassign` selects only rows with `source = 'assign'` and deletes them, which makes those sessions Unknown again.
- Both print what they selected, then write unless `--dry-run` is given:

```
Assigned 126 sessions to gdevxy@gmail.com's Organization · Max
  2026-06-03 to 2026-10-11 · $2,941.20
```

With `--dry-run` the first word becomes `Would assign` or `Would unassign`.
When nothing matches, the command says so and exits successfully.
Spend is the API-priced cost of the selected sessions' requests, subagents included, formatted like `stats`.

Writes go through the store's writer and are visible to a running daemon without a restart, like `accounts rename`.

## 5. Data model

No migration is needed.
`session_accounts.source` is free text, and `desktop` and `assign` are new values.
The schema version stays at 18, so a released build can still open the database.

## 6. Pane, stats and doctor

- No pane change: the Unknown note and the History account cycle already reflect the new attributions.
- `doctor` reports whether the desktop folder was found, how many session files were read, and how many were skipped.

## 7. Error handling

- An unreadable desktop folder: logged once per run, attribution continues from the other sources.
- A desktop session id that also has a `start` attribution to a `gateway`, `cloud` or `api` account: the existing row stays.
- `assign` to an account that does not exist: an error naming the accounts that do.
- An invalid date or an `--until` before `--since`: an error before any write.

## 8. Documentation

- The accounts spec (`2026-10-11-accounts-and-budgets-design.md`) is updated: the non-goal about pre-upgrade sessions points here, and the attribution order lists desktop and assign.
- The README section on accounts explains Unknown, desktop attribution and `accounts assign` with an example.

## 9. Testing

- Store unit tests for the precedence table, one test per row, plus `desktop` keeping a `gateway` row.
- Desktop reader unit tests: grandparent uuid extraction, `priorCliSessionIds`, skipped files, the automatic label and its replacement on CLI sign-in.
- Glob and date filter unit tests, including `*` across `/`, a missing `cwd`, and inclusive day bounds in local time.
- CLI end-to-end tests with a fixture desktop folder under a temporary `HOME`:
  - a session with a `start` row on the CLI account is moved to the desktop account by `import`, which reproduces the real misattribution;
  - `assign --dry-run` writes nothing, `assign` writes, a second `assign` selects nothing, `unassign` restores Unknown;
  - `stats --account <label>` shows the assigned sessions.
- Daemon test: a desktop session file added while the daemon runs is attributed on the next rescan.
- A manual check `docs/manual-checks/m9-attribution.md` on a real machine: run `import`, confirm desktop sessions moved, assign the private projects to the private account, then confirm History and `stats`.
