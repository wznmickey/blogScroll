-- A feed is a video channel when its entries link to video pages (YouTube,
-- Bilibili, PeerTube, ...), no longer when they merely attach clips: a family
-- blog with videos is a blog. Earlier detections are cleared; the next read
-- of each feed decides again.
UPDATE blogs SET detected_kind = NULL WHERE detected_kind = 'video';
