-- Plex collection order (a local addition in the 9001+ block).
--
-- Each list provider holds a block of sort numbers (slot 1 is !1000_, slot 9
-- is !9000_) and each mirrored list a position inside its provider's block,
-- so its collection sorts as !1010_, !1015_, ... in Plex. Titles record the
-- sort title last pushed to each collection, so Plex is only asked again
-- when Scryer's order or the list's name changes.

ALTER TABLE plex_collection_lists ADD COLUMN position bigint;

CREATE TABLE plex_collection_providers (
    provider text PRIMARY KEY NOT NULL,
    slot bigint NOT NULL UNIQUE,
    updated_at timestamptz NOT NULL
);

CREATE TABLE plex_collection_titles (
    connection_id text NOT NULL REFERENCES media_server_connections(id) ON DELETE CASCADE,
    subscription_id text NOT NULL,
    section_id text NOT NULL,
    collection_key text NOT NULL,
    sort_title text NOT NULL,
    updated_at timestamptz NOT NULL,
    PRIMARY KEY (connection_id, subscription_id, section_id)
);
