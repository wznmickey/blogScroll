-- Feeds used to be cut to their first 20 entries. Read in full, they showed
-- the older ones below; those without a usable date were taken for posts
-- that had just appeared. They are backlog of unknown age: below the blog's
-- other undated posts, in feed order. (New posts come first in a feed, so
-- these are the ones placed at 20 or lower.)
UPDATE posts SET undated = 1, sort_at = fix.sort_at
FROM (
    SELECT p.id,
           (SELECT MIN(o.sort_at) FROM posts o WHERE o.blog_id = p.blog_id AND o.undated = 1)
             - ROW_NUMBER() OVER (PARTITION BY p.blog_id ORDER BY p.sort_at DESC, p.id) AS sort_at
    FROM posts p
    WHERE p.published_at IS NULL AND p.undated = 0
      AND p.sort_at <= p.first_seen_at - 20
      AND EXISTS (SELECT 1 FROM posts o WHERE o.blog_id = p.blog_id AND o.undated = 1
                  AND o.first_seen_at < p.first_seen_at)
) AS fix
WHERE posts.id = fix.id;
