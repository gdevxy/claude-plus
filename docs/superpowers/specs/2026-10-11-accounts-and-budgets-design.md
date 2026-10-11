# claude-plus - Accounts and Budgets Design Spec

Date: 2026-10-11
Status: Approved in brainstorming, pending written-spec review

## 1. Purpose

claude-plus assumes one Claude subscription per machine.
Two real setups break that assumption.

First, enterprise seats.
A Claude for Enterprise seat signs in with OAuth like a subscription, but its limit is a monthly spend limit (for example $700 per person, resetting on the 1st at 00:00 UTC) shared by Chat, Cowork and Claude Code.
Claude Code reports it to mods as a `spend_limit` rate-limit window with a percent and a reset time, without dollar amounts.

Second, account switching.
A user logs out of a company account and into a private subscription on the same machine, and back.
Today every limit sample, fit, history row and insight is pooled across accounts, which produces false resets, a skewed percent-per-dollar fit, stale limit bars from the other account, and mixed spend totals.

This change makes the account a first-class dimension, adds a per-account monthly budget set from the CLI, adapts the wording for setups without a subscription, and prepares for a company LLM gateway (LiteLLM or similar) that the user's company plans to adopt.

### Success criteria

- Every session started after the upgrade is attributed to the account it ran under, including sessions that only exist as transcripts.
- Switching accounts never registers as a limit reset, never feeds the other account's spend into the fit, and never shows the other account's limit bars.
- An enterprise session shows its spend limit in dollars when a budget is set: `$27.89 of $700 · 4% · resets in 20d · on pace for 31%`.
- A budget without a reported limit (gateway or API key) shows a bar computed from local spend, labelled as covering this machine only.
- History, Insights, `stats` and `export` can be filtered per account and default to the current one.
- Bedrock, Vertex, Foundry and gateway-prefixed model ids are priced.
- A user price file can override the bundled prices for negotiated rates.
- claude-plus still makes no network requests and never edits Claude Code's files.

### Non-goals

- Fetching real billed amounts from any API.
- Per-product breakdown (Chat, Cowork), since that data never reaches the machine.
- Attributing sessions recorded before the upgrade to an account; they stay "Unknown".
- Budget periods other than the calendar month.

## 2. Terms

- Account: who a session bills to.
  Kinds are `oauth` (a claude.ai login, subscription or enterprise), `api` (an Anthropic API key), `cloud` (Bedrock, Vertex or Foundry) and `gateway` (a custom `ANTHROPIC_BASE_URL`).
- Account period: a time span during which one OAuth account was signed in on this machine.
- Reported limit: a rate-limit window Claude Code reports (`five_hour`, `seven_day`, `spend_limit`).
- Budget: a monthly amount in USD the user sets for an account.
- Local spend: API-priced cost of Claude Code requests read from this machine's transcripts.

## 3. Account resolution

### OAuth account timeline

The daemon reads the `oauthAccount` block of `.claude.json` in the Claude Code config location (`~/.claude.json`, or `$CLAUDE_CONFIG_DIR/.claude.json`), read-only.
It reads it at startup, on every `session.start` event before attributing that session, and on every periodic rescan (60 seconds).
The existing watcher only covers `projects/`, and an account switch only matters to sessions that start after it, so these three moments are enough.
It keeps `accountUuid`, `organizationName`, `organizationType` and `billingType`, and nothing else.
The email address is not stored.

When the uuid differs from the open period's, the open period is closed at the change time and a new one is opened.
When the block disappears (logout), the open period is closed and none is opened.
A missing, unreadable or malformed file closes nothing and is logged once per daemon run.

### Session attribution

The mod adds an `auth` object to the `session.start` event it already forwards:

- `provider`: `bedrock`, `vertex` or `foundry` when the matching `CLAUDE_CODE_USE_*` variable is set, else `anthropic`.
- `base_url`: the value of `ANTHROPIC_BASE_URL`, or null.
- `api_key`: true when `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN` is set; only presence is sent, never the value.
  An `apiKeyHelper` lives in the settings files, not the environment, so it is not detected; such a session falls back to the timeline.

The mod reads these with `$.env.get`.

The daemon resolves a session's account in this order:

1. `provider` other than `anthropic`: the `cloud` account for that provider.
2. `base_url` set: the `gateway` account keyed by its scheme, host and port.
3. `api_key` true: the `api` account.
4. Otherwise the OAuth account whose period contains the session's start time.

A session without a `session.start` event (a `claude -p` run, a session while the daemon was down) uses step 4 only.
A session with no matching period stays unattributed and appears as "Unknown".
Attribution is stored in its own table, `session_accounts`, because `sessions` is a derived table that `reindex` clears.
A `session.start` attribution replaces a timeline one, and neither is recomputed later.

### Labels

A new account is labelled automatically:

- `oauth`: `<organizationName> · <plan>`, where plan comes from `organizationType` (`claude_max` → Max, `claude_pro` → Pro, `claude_enterprise` → Enterprise, `claude_team` → Team, anything else → its raw value).
- `gateway`: the host.
- `cloud`: the provider name.
- `api`: "API key".

`claude-plus accounts rename <account> <label>` overrides it.

## 4. Data model

One schema migration adds:

```sql
CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    label TEXT NOT NULL,
    label_is_custom INTEGER NOT NULL DEFAULT 0,
    organization_type TEXT,
    billing_type TEXT,
    first_seen_ms INTEGER NOT NULL
);
CREATE TABLE account_periods (
    account_id TEXT NOT NULL,
    from_ms INTEGER NOT NULL,
    to_ms INTEGER
);
CREATE INDEX account_periods_by_time ON account_periods(from_ms);
CREATE TABLE budgets (
    account_id TEXT PRIMARY KEY,
    amount_usd REAL NOT NULL,
    period TEXT NOT NULL,
    updated_ms INTEGER NOT NULL
);
CREATE TABLE session_accounts (
    session_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    source TEXT NOT NULL
);
CREATE INDEX session_accounts_by_account ON session_accounts(account_id);
ALTER TABLE limit_samples ADD COLUMN account_id TEXT;
CREATE INDEX limit_samples_by_account ON limit_samples(account_id, window, ts_ms);
```

Account ids are `oauth:<accountUuid>`, `api`, `cloud:<provider>` and `gateway:<scheme://host:port>`.
`period` is always `monthly` in this version.
`session_accounts.source` is `start` or `timeline`.
Sessions without a row and limit samples with a null `account_id` are "Unknown".
`limit_samples.account_id` is copied from the session's account when the sample is stored.

## 5. Limits, fit and pace per account

- `limits::instances` splits on a change of account as well as of window, so another account's lower percent is never read as a reset.
- `limits::fit` pairs each account's samples only with spend from that account's sessions, and stores fits keyed by account, window and model family.
- `limits::windows` and the live snapshot return only the windows of the session's own account.
  The mod's `limitsOf` merge keeps only kept windows of the same account as the live measure, which removes the stale bar.
- Pace for `five_hour` and `seven_day` is unchanged.
- Pace for `spend_limit` and for budgets uses the average daily spend since the period began (the 1st at 00:00 UTC), projected to the reset.
  It needs at least one full day of data; before that, no pace is shown.
  It warns when the projection exceeds 100%.

## 6. Budgets

`claude-plus budget set <usd> [--account <label|id>]` sets a monthly budget, defaulting to the account of the most recent session.
`claude-plus budget show [--account ...]` prints the budget, the reported percent if any, local spend this month and the pace.
`claude-plus budget clear [--account ...]` removes it.
An amount that is not a finite number above zero is rejected.

The budget bar has three forms:

| Reported `spend_limit` | Budget | Bar |
|---|---|---|
| yes | yes | `$27.89 of $700 · 4%`, dollars derived from the reported percent, plus a dim line `Claude Code here: $23.10` from local spend. |
| yes | no | `Spend 4%` as today, plus a dim hint `claude-plus budget set <usd> shows dollars`. |
| no | yes | `Budget (this machine)` with local spend over the budget. |

The reported percent always wins over local spend; the two are never mixed into one figure.
Budget alerts reuse the existing limit toast: one toast when the projection first exceeds 100% in a period, one when usage crosses 90%.

## 7. Pane

- Live view header: the account label after the model, for example `Acme · Enterprise`.
- The section heading "Plan limits" becomes "Limits".
- The cache note says the lifetime is "assumed from your account" instead of "from your plan".
- History: `a` cycles current account, each other account with data, and all accounts (Unknown included).
  The selected account appears in the view title.
  The `pct` metric is offered only when a single account with a calibrated `five_hour` fit is selected.
- Insights: computed for the current account; savings in percent of 5h only when that account has a calibrated fit.
- Every new string is checked at the narrowest supported width and in each theme.

## 8. CLI

- `claude-plus accounts`: lists accounts with id, label, kind, sessions, this month's local spend and budget.
- `claude-plus accounts rename <account> <label>`.
- `claude-plus budget set|show|clear` (section 6).
- `stats` and `export` take `--account <label|id|all>`, defaulting to the current account; `export` gains an `account` column.
- `doctor` reports the current account, whether `.claude.json` was readable, and whether a user price file is loaded.

`<account>` matches an id exactly or a label case-insensitively; an ambiguous label is an error listing the matches.

## 9. Pricing for gateways and cloud providers

- Anthropic's first-party prices bundled in `assets/prices.toml` stay the default.
- A user price file at `$CLAUDE_PLUS_HOME/prices.toml` is optional and never created by claude-plus.
  When present, it is merged over the bundled table with the existing `PriceTable::merged_with`, in every command and in the daemon, so it only needs the models whose prices differ.
  An invalid file is reported by `doctor` and by the command that loaded it, and the bundled table is used alone.
- `normalize_model` also strips:
  - a Bedrock region prefix and `anthropic.` vendor prefix, and a `-v<n>:<n>` suffix (`us.anthropic.claude-opus-5-5-20260901-v1:0` → `claude-opus-5-5`);
  - a Vertex `@<date>` suffix (`claude-opus-5-5@20260901` → `claude-opus-5-5`);
  - a gateway `<provider>/` prefix (`anthropic/claude-opus-5-5` → `claude-opus-5-5`).
- Requests whose model still has no price stay unpriced as today, and `doctor` lists those model ids.

## 10. Error handling

- No readable `.claude.json`: OAuth attribution is skipped, other kinds still resolve, sessions may be Unknown; logged once.
- A `session.start` without `auth` (an older mod): the daemon falls back to the timeline.
- Two OAuth accounts in one session's lifetime: the session keeps its start account.
- Reported percent above 100 on an exceeded spend limit: the bar is drawn full in the warning color with the real percent.
- Budget set for an account with no sessions: allowed, so a budget can be set before first use.

## 11. Testing

- Rust unit tests:
  - the account timeline for login, logout, switch and malformed file;
  - attribution order (cloud, gateway, api, oauth period, unknown);
  - `instances` with a 60% private sample followed by a 5% company sample, which must not be a reset;
  - `fit` that ignores another account's spend;
  - the monthly pace, including the first day and the period boundary in UTC;
  - the new `normalize_model` cases;
  - price file merge and invalid file fallback.
- API tests: a snapshot includes only its own account's windows; budget forms in the snapshot.
- Mod tests: the three budget bar forms, the account label in the header, the History account cycle, at the narrowest width and each theme.
- A manual E2E check `docs/manual-checks/m8-accounts.md`: with the daemon running, sign in to account A, run a session, switch to account B, run a session, then confirm the bars, the History filter, `accounts`, and a budget set on B.
