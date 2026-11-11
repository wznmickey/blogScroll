-- What readers sync between their devices (readers.rs): one blob per sync
-- code, encrypted in the browser with a key the server never sees, and known
-- here by a hash of a token derived from the same code. `version` goes up by
-- one with every write. A deleted one stays as a marker (`deleted`, no data),
-- so devices still syncing with that code stop instead of filling it again.
CREATE TABLE sync_docs (
    id TEXT PRIMARY KEY,
    version INTEGER NOT NULL,
    data BLOB NOT NULL,
    deleted INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL
);
