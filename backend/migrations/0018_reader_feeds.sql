-- Feeds readers added themselves (设置 → 来源 → 我添加的来源; readers.rs):
-- crawled like every other blog (sources/readers.rs, directory `readers`),
-- but a blog listed by no other directory is `private`: left out of every
-- list, search and export, and shown only to readers who ask for it by id.
-- Registrations carry no reader: only the feed, and when it was last asked
-- for (the list drops what nobody asked for in a while).
CREATE TABLE reader_feeds (
    url TEXT PRIMARY KEY,
    first_at INTEGER NOT NULL,
    last_at INTEGER NOT NULL
);

ALTER TABLE blogs ADD COLUMN private INTEGER NOT NULL DEFAULT 0;
