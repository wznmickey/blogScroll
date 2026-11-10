-- Profiles now keep up to three accounts per platform ("X" and "X (…En)").
-- Blogs with social links may have more: bring their next refresh forward,
-- spread over the coming week as in 0009.
UPDATE blogs
SET profile_at = MIN(profile_at, CAST(strftime('%s', 'now') AS INTEGER) - 30 * 86400 + (id % 7) * 86400)
WHERE profile_at IS NOT NULL AND social_links <> '[]';
