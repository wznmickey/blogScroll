-- What readers do on the site, counted, with no reader in it (stats.rs):
-- per day, what (`metric`: visit, dwell, open, read, like, save, follow,
-- search, setting) and of what (`key`: a post's address, a blog's
-- `site_key`, a search term, a setting). Addresses, not database ids, so the
-- counts survive a database built anew. `source`: 'local' for this server's
-- own visitors, 'site' for the static site's (the Worker's hourly copy in
-- the backup repository, stats/).
CREATE TABLE stats (
    source TEXT NOT NULL,
    day TEXT NOT NULL,
    metric TEXT NOT NULL,
    key TEXT NOT NULL,
    n REAL NOT NULL,
    PRIMARY KEY (source, day, metric, key)
);

-- Today's visitors (an anonymous browser's random id, hashed), so each counts once.
CREATE TABLE visitors (
    day TEXT NOT NULL,
    who TEXT NOT NULL,
    PRIMARY KEY (day, who)
);

-- Who has each of readers' own feeds, as the same anonymous ids: only to count them.
CREATE TABLE reader_subs (
    url TEXT NOT NULL,
    who TEXT NOT NULL,
    last_at INTEGER NOT NULL,
    PRIMARY KEY (url, who)
);

-- The static site's counts of the same (stats/subscribers.json in the backup).
CREATE TABLE site_subscribers (
    url TEXT PRIMARY KEY,
    n INTEGER NOT NULL
);

-- Derived from the counts (stats::refresh_heat): how much readers took to
-- each post lately (`heat`: its standing among them, 0-1), and to each blog.
CREATE TABLE post_heat (
    post_id INTEGER PRIMARY KEY,
    score REAL NOT NULL,
    heat REAL NOT NULL
);
CREATE TABLE blog_heat (
    blog_id INTEGER PRIMARY KEY,
    followers REAL NOT NULL,
    score REAL NOT NULL
);

CREATE INDEX IF NOT EXISTS posts_url ON posts (url);
