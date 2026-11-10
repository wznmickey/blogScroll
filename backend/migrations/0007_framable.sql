-- Whether a blog's pages may be shown in a frame on this site (the reader
-- pane): from the homepage's X-Frame-Options / CSP frame-ancestors when the
-- profile is refreshed, or an early "no" from the same headers on the feed.
-- NULL = not known yet.
ALTER TABLE blogs ADD COLUMN framable INTEGER;
