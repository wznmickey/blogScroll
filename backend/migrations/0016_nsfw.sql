-- Adult content (R18 / NSFW), from words in the title or the author's tags
-- (analyze::is_nsfw); hidden unless a reader asks to see it. Set by the
-- next analysis (at start and every few hours).
ALTER TABLE posts ADD COLUMN nsfw INTEGER NOT NULL DEFAULT 0;
