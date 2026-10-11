pub const PRE_RELEASE_VERSIONS: std::ops::RangeInclusive<usize> = 1..=6;

pub const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE files (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    offset INTEGER NOT NULL DEFAULT 0,
    lines INTEGER NOT NULL DEFAULT 0,
    size INTEGER NOT NULL DEFAULT 0,
    mtime INTEGER NOT NULL DEFAULT 0,
    parse_errors INTEGER NOT NULL DEFAULT 0,
    head_len INTEGER NOT NULL DEFAULT 0,
    head_hash INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE raw_records (
    file_id INTEGER NOT NULL REFERENCES files(id),
    line_no INTEGER NOT NULL,
    kind TEXT NOT NULL,
    json TEXT NOT NULL,
    PRIMARY KEY (file_id, line_no)
);
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    cwd TEXT,
    started_at INTEGER,
    last_seen_at INTEGER,
    title TEXT,
    title_is_custom INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE requests (
    key TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    ts INTEGER NOT NULL,
    model TEXT NOT NULL,
    input INTEGER NOT NULL,
    output INTEGER NOT NULL,
    thinking INTEGER NOT NULL,
    cache_read INTEGER NOT NULL,
    cache_write_5m INTEGER NOT NULL,
    cache_write_1h INTEGER NOT NULL,
    cost_usd REAL,
    is_sidechain INTEGER NOT NULL,
    agent_id TEXT,
    skill TEXT,
    mcp_server TEXT,
    effort TEXT
);
CREATE INDEX requests_by_ts ON requests(ts);
CREATE INDEX requests_by_session ON requests(session_id, ts);
CREATE TABLE tool_uses (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    ts INTEGER NOT NULL,
    name TEXT NOT NULL,
    input_chars INTEGER NOT NULL
);
CREATE TABLE tool_results (
    tool_use_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    ts INTEGER NOT NULL,
    chars INTEGER NOT NULL,
    images INTEGER NOT NULL,
    is_error INTEGER NOT NULL
);
CREATE TABLE attachments (
    file_id INTEGER NOT NULL,
    line_no INTEGER NOT NULL,
    session_id TEXT NOT NULL,
    ts INTEGER,
    kind TEXT NOT NULL,
    chars INTEGER NOT NULL,
    names TEXT NOT NULL,
    PRIMARY KEY (file_id, line_no)
);
CREATE TABLE compactions (
    file_id INTEGER NOT NULL,
    line_no INTEGER NOT NULL,
    session_id TEXT NOT NULL,
    ts INTEGER,
    trigger TEXT,
    pre_tokens INTEGER,
    post_tokens INTEGER,
    PRIMARY KEY (file_id, line_no)
);
CREATE TABLE cost_states (
    file_id INTEGER NOT NULL,
    line_no INTEGER NOT NULL,
    session_id TEXT NOT NULL,
    total_cost_usd REAL,
    model_count INTEGER NOT NULL,
    json TEXT NOT NULL,
    PRIMARY KEY (file_id, line_no)
);
CREATE INDEX cost_states_by_session ON cost_states(session_id, file_id, line_no);
CREATE TABLE adjustments (
    session_id TEXT NOT NULL,
    model TEXT NOT NULL,
    ts INTEGER NOT NULL,
    input INTEGER NOT NULL,
    output INTEGER NOT NULL,
    cache_read INTEGER NOT NULL,
    cache_write INTEGER NOT NULL,
    cost_usd REAL,
    PRIMARY KEY (session_id, model, ts)
);
CREATE INDEX adjustments_by_ts ON adjustments(ts);
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#,
    r#"
CREATE TABLE events (
    id INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    ts_ms INTEGER NOT NULL,
    received_ms INTEGER NOT NULL,
    json TEXT NOT NULL
);
CREATE INDEX events_by_session_kind ON events(session_id, kind, ts_ms);
"#,
    r#"
CREATE TABLE limit_samples (
    id INTEGER PRIMARY KEY,
    ts_ms INTEGER NOT NULL,
    session_id TEXT NOT NULL,
    model TEXT,
    window TEXT NOT NULL,
    used_pct REAL NOT NULL,
    resets_at_ms INTEGER
);
CREATE INDEX limit_samples_by_window ON limit_samples(window, ts_ms);
"#,
    r#"
ALTER TABLE events ADD COLUMN client_id TEXT;
CREATE UNIQUE INDEX events_by_client_id ON events(session_id, client_id);
"#,
    r#"
ALTER TABLE requests ADD COLUMN sent_ts INTEGER;
INSERT OR REPLACE INTO meta (key, value) SELECT 'reindex_pending', '1' WHERE EXISTS (SELECT 1 FROM requests);
"#,
    r#"
CREATE TABLE live_breakdowns (
    session_id TEXT PRIMARY KEY,
    ts_ms INTEGER NOT NULL,
    received_ms INTEGER NOT NULL,
    json TEXT NOT NULL
);
CREATE INDEX live_breakdowns_by_ts ON live_breakdowns(ts_ms);
"#,
    r#"
CREATE INDEX attachments_by_session ON attachments(session_id, ts);
CREATE INDEX compactions_by_session ON compactions(session_id, ts);
CREATE INDEX tool_results_by_session ON tool_results(session_id, chars);
CREATE INDEX tool_uses_by_session ON tool_uses(session_id, name);
CREATE INDEX tool_uses_mcp_by_ts ON tool_uses(ts, name) WHERE name >= 'mcp__' AND name < 'mcp_`';
CREATE INDEX requests_skill_by_ts ON requests(ts, skill) WHERE skill IS NOT NULL;
"#,
    r#"
ALTER TABLE tool_results ADD COLUMN is_sidechain INTEGER NOT NULL DEFAULT 0;
INSERT OR REPLACE INTO meta (key, value) SELECT 'reindex_pending', '1' WHERE EXISTS (SELECT 1 FROM tool_results);
"#,
    r#"
ALTER TABLE tool_uses ADD COLUMN target TEXT;
CREATE TABLE prompts (
    uuid TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    ts INTEGER NOT NULL
);
CREATE INDEX prompts_by_ts ON prompts(ts);
CREATE INDEX tool_results_by_ts ON tool_results(ts, chars);
INSERT OR REPLACE INTO meta (key, value) SELECT 'reindex_pending', '1' WHERE EXISTS (SELECT 1 FROM requests);
"#,
];

pub fn version_of(index: usize) -> usize {
    PRE_RELEASE_VERSIONS.end() + index + 1
}

pub fn next_migration<'a>(migrations: &[&'a str], version: usize) -> Option<(usize, &'a str)> {
    let index = version.saturating_sub(*PRE_RELEASE_VERSIONS.end());
    migrations.get(index).map(|sql| (version_of(index), *sql))
}

pub fn step_of(version: usize) -> usize {
    version - PRE_RELEASE_VERSIONS.end()
}

pub const DERIVED_TABLES: &[&str] = &[
    "requests",
    "tool_uses",
    "tool_results",
    "attachments",
    "compactions",
    "cost_states",
    "sessions",
    "adjustments",
    "prompts",
];
