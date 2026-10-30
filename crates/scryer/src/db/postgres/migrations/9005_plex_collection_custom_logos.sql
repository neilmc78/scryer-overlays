-- Custom logos on collection posters (a local addition in the 9001+ block).
--
-- `custom` draws the image the user uploaded for the list instead of the
-- provider's logo. NULL keeps the earlier meaning: `poster_logo` chooses
-- between the provider's logo and none. The image itself is a file under
-- the overlay data root, not a column.

ALTER TABLE plex_collection_lists ADD COLUMN poster_logo_source text;
