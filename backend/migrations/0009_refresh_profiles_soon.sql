-- Profiles now also read social links from the about page, from theme
-- settings in scripts, from NexT's <span data-url> links and from redirect
-- wrappers, and know more sites. Bring the next refresh forward for every
-- blog, spread over the coming week (refreshes are due 30 days after
-- profile_at), so the new links appear without a burst of requests.
UPDATE blogs
SET profile_at = MIN(profile_at, CAST(strftime('%s', 'now') AS INTEGER) - 30 * 86400 + (id % 7) * 86400)
WHERE profile_at IS NOT NULL;
