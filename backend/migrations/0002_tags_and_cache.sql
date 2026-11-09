-- Hash of the last feed body we parsed. Lets us skip work when a server
-- ignores conditional requests but sends identical bytes.
ALTER TABLE blogs ADD COLUMN content_hash TEXT;

-- Directory tags per blog (开往: blog / tech / life / hybrid ...), for filtering.
CREATE TABLE blog_tags (
    blog_id   INTEGER NOT NULL REFERENCES blogs (id) ON DELETE CASCADE,
    directory TEXT    NOT NULL,
    tag       TEXT    NOT NULL,
    PRIMARY KEY (blog_id, directory, tag)
);
CREATE INDEX blog_tags_tag ON blog_tags (tag, blog_id);

INSERT OR IGNORE INTO blog_tags (blog_id, directory, tag)
SELECT d.blog_id, d.directory, j.value
FROM directory_entries d, json_each(d.tags) j;
