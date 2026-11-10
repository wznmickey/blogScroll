-- About-page settings an author asked for ([[policy.about]] in the config):
-- which page is the about page, whether to keep all of it (formatted, in
-- about_html) rather than an excerpt, or their own introduction instead.
ALTER TABLE blogs ADD COLUMN about_page TEXT;
ALTER TABLE blogs ADD COLUMN about_full INTEGER NOT NULL DEFAULT 0;
ALTER TABLE blogs ADD COLUMN about_custom TEXT;
ALTER TABLE blogs ADD COLUMN about_html TEXT;
