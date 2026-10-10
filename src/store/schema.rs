pub const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE files (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    offset INTEGER NOT NULL DEFAULT 0,
    lines INTEGER NOT NULL DEFAULT 0,
    size INTEGER NOT NULL DEFAULT 0,
    mtime INTEGER NOT NULL DEFAULT 0,
    parse_errors INTEGER NOT NULL DEFAULT 0
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
    total_cost_usd REAL NOT NULL,
    json TEXT NOT NULL,
    PRIMARY KEY (file_id, line_no)
);
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#,
    r#"
ALTER TABLE files ADD COLUMN head_len INTEGER NOT NULL DEFAULT 0;
ALTER TABLE files ADD COLUMN head_hash INTEGER NOT NULL DEFAULT 0;
"#,
];

pub const DERIVED_TABLES: &[&str] = &[
    "requests",
    "tool_uses",
    "tool_results",
    "attachments",
    "compactions",
    "cost_states",
    "sessions",
];
