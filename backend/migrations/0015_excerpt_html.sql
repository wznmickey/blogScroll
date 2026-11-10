-- The excerpt with its formatting (paragraphs, lists, headings, emphasis),
-- written from a short list of tags without attributes; NULL when it has
-- none beyond one paragraph. Filled as feeds are read again (PARSER_VERSION).
ALTER TABLE posts ADD COLUMN excerpt_html TEXT;
