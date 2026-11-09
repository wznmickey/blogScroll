-- One row per blog, deduplicated across directories by `site_key`
-- (normalised host + path, see `util::site_key`).
CREATE TABLE blogs (
    id               INTEGER PRIMARY KEY,
    site_key         TEXT    NOT NULL UNIQUE,
    name             TEXT    NOT NULL,
    homepage_url     TEXT    NOT NULL,
    icon_url         TEXT,
    feed_url         TEXT,
    -- 'directory' | 'discovered' | 'manual'
    feed_url_origin  TEXT,
    -- At least one directory lists the blog as alive.
    active           INTEGER NOT NULL DEFAULT 1,
    -- The author asked not to be included (config: policy.exclude).
    excluded         INTEGER NOT NULL DEFAULT 0,
    -- 'summary' (default, RSS excerpt only) | 'full_text' (author granted permission)
    content_policy   TEXT    NOT NULL DEFAULT 'summary',
    etag             TEXT,
    last_modified    TEXT,
    refresh_secs     INTEGER NOT NULL,
    next_check_at    INTEGER NOT NULL DEFAULT 0,
    last_checked_at  INTEGER,
    last_success_at  INTEGER,
    last_new_post_at INTEGER,
    discovery_at     INTEGER,
    failures         INTEGER NOT NULL DEFAULT 0,
    last_error       TEXT,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);
CREATE INDEX blogs_due ON blogs (next_check_at) WHERE active = 1 AND excluded = 0;

-- Membership of a blog in a directory (travellings, foreverblog, ...).
CREATE TABLE directory_entries (
    directory    TEXT    NOT NULL,
    external_id  TEXT    NOT NULL,
    blog_id      INTEGER NOT NULL REFERENCES blogs (id) ON DELETE CASCADE,
    status       TEXT,
    tags         TEXT    NOT NULL DEFAULT '[]',
    active       INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    PRIMARY KEY (directory, external_id)
);
CREATE INDEX directory_entries_blog ON directory_entries (blog_id);

CREATE TABLE posts (
    id             INTEGER PRIMARY KEY,
    blog_id        INTEGER NOT NULL REFERENCES blogs (id) ON DELETE CASCADE,
    guid           TEXT    NOT NULL,
    url            TEXT    NOT NULL,
    title          TEXT    NOT NULL,
    excerpt        TEXT    NOT NULL DEFAULT '',
    cover_url      TEXT,
    author         TEXT,
    tags           TEXT    NOT NULL DEFAULT '[]',
    published_at   INTEGER,
    first_seen_at  INTEGER NOT NULL,
    -- min(published_at, first_seen_at): future-dated posts cannot pin themselves to the top.
    sort_at        INTEGER NOT NULL,
    -- Only ever filled for blogs whose content_policy = 'full_text'.
    content_html   TEXT,
    -- 'feed' | 'page'
    content_source TEXT,
    UNIQUE (blog_id, guid)
);
CREATE INDEX posts_sort ON posts (sort_at DESC, id DESC);
CREATE INDEX posts_blog ON posts (blog_id, sort_at DESC);

CREATE TABLE kv (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
