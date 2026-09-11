-- Frozen legacy OAuth schema and synthetic rows; see python-free-fixtures.md.
CREATE TABLE IF NOT EXISTS oauth_clients (
    client_id TEXT PRIMARY KEY,
    payload TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS oauth_tokens (
    token_hash TEXT PRIMARY KEY,
    token_kind TEXT NOT NULL,
    family_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    scopes_json TEXT NOT NULL,
    expires_at INTEGER,
    resource TEXT,
    subject TEXT
);
CREATE INDEX IF NOT EXISTS oauth_tokens_family
    ON oauth_tokens(family_id);
CREATE TABLE IF NOT EXISTS oauth_refresh_history (
    token_hash TEXT PRIMARY KEY,
    family_id TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    used_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS oauth_refresh_history_family
    ON oauth_refresh_history(family_id);
CREATE INDEX IF NOT EXISTS oauth_refresh_history_expiry
    ON oauth_refresh_history(expires_at);
INSERT INTO oauth_tokens VALUES
('9e4bcf71eb0125968d6f7197b44b77882b5e1f83bd68e6bfe859ab44b6fbce37','access','python-family','python-client','["files:read"]',4000000000,'https://example.test/mcp','owner'),
('95af3b7bd265f8599d12e1cf427b25d561e83977693db7db627e75d8644b033b','refresh','python-family','python-client','["files:read"]',4000000000,NULL,'owner');
