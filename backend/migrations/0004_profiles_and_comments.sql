-- Blog profile from its homepage and about page (refreshed about monthly).
ALTER TABLE blogs ADD COLUMN description TEXT;
ALTER TABLE blogs ADD COLUMN avatar_url TEXT;
ALTER TABLE blogs ADD COLUMN about_url TEXT;
ALTER TABLE blogs ADD COLUMN about_text TEXT;
ALTER TABLE blogs ADD COLUMN social_links TEXT NOT NULL DEFAULT '[]';
ALTER TABLE blogs ADD COLUMN generator TEXT;
ALTER TABLE blogs ADD COLUMN profile_at INTEGER;

-- Detected comment system, as JSON (see comments::CommentSystem).
ALTER TABLE blogs ADD COLUMN comment_system TEXT;
-- The author allowed embedding their comment widget here (config: policy.embed_comments).
ALTER TABLE blogs ADD COLUMN embed_comments INTEGER NOT NULL DEFAULT 0;
-- Earliest time to ask the comment service for counts again.
ALTER TABLE blogs ADD COLUMN comments_next_at INTEGER NOT NULL DEFAULT 0;

ALTER TABLE posts ADD COLUMN comments_url TEXT;
ALTER TABLE posts ADD COLUMN comment_count INTEGER;
