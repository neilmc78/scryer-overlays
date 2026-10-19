-- Plex collection order (a local addition in the 9001+ block).
--
-- Each list provider holds a block of sort numbers (slot 1 is !1000_, slot 9
-- is !9000_) and each mirrored list a position inside its provider's block,
-- so its collection sorts as !1010_, !1015_, ... in Plex. Titles record the
-- sort title last pushed to each collection, so Plex is only asked again
-- when Scryer's order or the list's name changes.

ALTER TABLE plex_collection_lists ADD COLUMN position INTEGER;

CREATE TABLE plex_collection_providers (
    provider TEXT PRIMARY KEY NOT NULL,
    slot INTEGER NOT NULL UNIQUE,
    updated_at TEXT NOT NULL
);

CREATE TABLE plex_collection_titles (
    connection_id TEXT NOT NULL REFERENCES media_server_connections(id) ON DELETE CASCADE,
    subscription_id TEXT NOT NULL,
    section_id TEXT NOT NULL,
    collection_key TEXT NOT NULL,
    sort_title TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (connection_id, subscription_id, section_id)
);
