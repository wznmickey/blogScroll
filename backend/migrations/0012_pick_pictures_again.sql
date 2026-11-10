-- Pictures are now chosen by scoring every image on the page (and the about
-- page), skipping lazy-load placeholders and theme defaults, with the
-- author's GitHub avatar and the favicon as fallbacks. Read every profile
-- again over the coming week, as in 0009.
UPDATE blogs
SET profile_at = MIN(profile_at, CAST(strftime('%s', 'now') AS INTEGER) - 30 * 86400 + (id % 7) * 86400)
WHERE profile_at IS NOT NULL;
