-- What kind of source a feed is: blog | news | podcast | newsletter | forum | video | other.
-- `kind` is set by a directory or the config (wins); `detected_kind` comes from the feed itself.
ALTER TABLE blogs ADD COLUMN kind TEXT;
ALTER TABLE blogs ADD COLUMN detected_kind TEXT;
-- Content areas computed from the blog's posts, as JSON [{"topic": "dev", "share": 0.6}, ...].
ALTER TABLE blogs ADD COLUMN topics TEXT NOT NULL DEFAULT '[]';
-- Hidden without deleting anything, e.g. 'spam' for a domain taken over by a gambling site.
ALTER TABLE blogs ADD COLUMN flagged TEXT;
-- Added by feed URL without a name: take the name and site link from the feed itself.
ALTER TABLE blogs ADD COLUMN from_feed_meta INTEGER NOT NULL DEFAULT 0;

-- Keywords from the title, feed categories, excerpt and the feed's own body
-- text (never the article page), as JSON [["词", weight], ...].
ALTER TABLE posts ADD COLUMN keywords TEXT NOT NULL DEFAULT '[]';
-- Tags derived from the keywords (e.g. "Docker"), in addition to the feed's own.
ALTER TABLE posts ADD COLUMN auto_tags TEXT NOT NULL DEFAULT '[]';
-- Content areas, as JSON ["dev", "ops"].
ALTER TABLE posts ADD COLUMN topics TEXT NOT NULL DEFAULT '[]';
-- The feed gave no usable date and the post came with the blog's first fetch,
-- so its age is unknown; it must not be shown as new.
ALTER TABLE posts ADD COLUMN undated INTEGER NOT NULL DEFAULT 0;

-- Unit-length term vectors per post: tags (own + auto, normalised) and keywords.
-- Powers tag filters, related posts and personalised discover.
CREATE TABLE post_terms (
    post_id INTEGER NOT NULL REFERENCES posts (id) ON DELETE CASCADE,
    term    TEXT    NOT NULL,
    weight  REAL    NOT NULL,
    is_tag  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (post_id, term)
) WITHOUT ROWID;
CREATE INDEX post_terms_term ON post_terms (term, post_id);
-- How a tag term (stored lowercased) is displayed, e.g. docker → Docker.
CREATE TABLE tag_labels (
    term  TEXT PRIMARY KEY,
    label TEXT NOT NULL
) WITHOUT ROWID;

-- Undated posts from a blog's first fetch were sorted as if new (first_seen - position).
-- Move them below everything dated, keeping their feed order. Posts that only lost
-- their date under the new parser rules are fixed when their feed is re-parsed.
UPDATE posts SET undated = 1, sort_at = sort_at - first_seen_at
WHERE published_at IS NULL
  AND first_seen_at = (SELECT MIN(p2.first_seen_at) FROM posts p2 WHERE p2.blog_id = posts.blog_id);

-- Discourse feeds stored every reply of a topic as its own post (`/t/topic/2851#post_3`).
-- Keep the oldest row per topic. Other fragments stay: microblog pages give
-- each entry an anchor (`/moments/#moment-1a2b`) and those are distinct posts.
DELETE FROM posts
WHERE substr(url, instr(url, '#') + 1) GLOB 'post_[0-9]*'
  AND EXISTS (
      SELECT 1 FROM posts p2
      WHERE p2.blog_id = posts.blog_id AND p2.id < posts.id
        AND substr(p2.url, 1, instr(p2.url || '#', '#') - 1) = substr(posts.url, 1, instr(posts.url, '#') - 1));
