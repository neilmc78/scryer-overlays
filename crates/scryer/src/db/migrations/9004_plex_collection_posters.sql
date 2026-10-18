-- Plex collection posters (a local addition in the 9001+ block).
--
-- A mirrored list may give its collection a poster Scryer draws: the
-- provider's logo, a background colour and a label. Titles record which
-- design was last uploaded and the poster Plex showed right after, so an
-- unchanged design is not uploaded again and a poster changed in Plex is
-- left alone.

ALTER TABLE plex_collection_lists ADD COLUMN poster_enabled INTEGER NOT NULL DEFAULT 0;
ALTER TABLE plex_collection_lists ADD COLUMN poster_background TEXT;
ALTER TABLE plex_collection_lists ADD COLUMN poster_text TEXT;
ALTER TABLE plex_collection_lists ADD COLUMN poster_logo INTEGER NOT NULL DEFAULT 1;
ALTER TABLE plex_collection_titles ADD COLUMN poster_hash TEXT;
ALTER TABLE plex_collection_titles ADD COLUMN poster_thumb TEXT;
