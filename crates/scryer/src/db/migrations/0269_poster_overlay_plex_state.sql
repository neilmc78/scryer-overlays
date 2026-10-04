-- Poster overlays pushed to Plex: per connection and title, which rendered
-- output was uploaded and the poster URL Plex reported right after, so a
-- pass uploads again only when the output changed or someone else replaced
-- the poster in Plex.

CREATE TABLE poster_overlay_plex_state (
    connection_id TEXT NOT NULL REFERENCES media_server_connections(id) ON DELETE CASCADE,
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    -- The Plex item (ratingKey) the poster was uploaded to.
    provider_item_id TEXT NOT NULL,
    pushed_output_hash TEXT,
    -- The item's `thumb` right after the upload; a different value later
    -- means the poster was changed in Plex.
    pushed_thumb TEXT,
    pushed_at TEXT,
    last_error TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (connection_id, title_id)
);

CREATE INDEX idx_poster_overlay_plex_state_title ON poster_overlay_plex_state(title_id);
