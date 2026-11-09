-- Entries from feeds that ship unrendered templates (`{{ post.title }}`);
-- the parser skips these from now on.
DELETE FROM posts WHERE title LIKE '%{{%}}%';
