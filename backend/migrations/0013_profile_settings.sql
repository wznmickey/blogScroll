-- [[policy.profile]]: what a blogger asked to be shown instead of what was
-- found on their site. Read in place of the found values; the found ones
-- stay, so removing a setting brings them back.
ALTER TABLE blogs ADD COLUMN avatar_custom TEXT;
ALTER TABLE blogs ADD COLUMN description_custom TEXT;
-- JSON, like social_links.
ALTER TABLE blogs ADD COLUMN social_custom TEXT;
-- JSON array of tag names.
ALTER TABLE blogs ADD COLUMN tags_custom TEXT;
