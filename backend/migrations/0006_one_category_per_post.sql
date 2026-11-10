-- One category per post instead of a list of topics: a top-level category
-- (`travel`), one of its sub-categories (`tech/ai`), or `other`. Tags (the
-- author's own and those derived from keywords) stay separate, in `tags`,
-- `auto_tags` and `post_terms`. Filled in by the analysis that runs at startup.
ALTER TABLE posts ADD COLUMN category TEXT NOT NULL DEFAULT 'other';
ALTER TABLE posts DROP COLUMN topics;
CREATE INDEX posts_category ON posts (category, sort_at);

-- A blog's main top-level categories, [{"id": "tech", "share": 0.6}].
ALTER TABLE blogs RENAME COLUMN topics TO categories;
UPDATE blogs SET categories = '[]';
