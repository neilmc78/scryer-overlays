-- A movie with several versions in Plex has one Plex item per version, and
-- each gets its own poster. Push records are therefore kept per Plex item:
-- the table is rebuilt with the item in its key, keeping every existing row
-- so nothing is uploaded again. No other table references it.

CREATE TABLE poster_overlay_plex_state_0270 (
    connection_id TEXT NOT NULL REFERENCES media_server_connections(id) ON DELETE CASCADE,
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    -- The Plex item (ratingKey) the poster was uploaded to.
    provider_item_id TEXT NOT NULL,
    -- Identity of what was uploaded: the title's output hash, or for one
    -- version of a movie, that version's input hash.
    pushed_output_hash TEXT,
    -- The item's `thumb` right after the upload; a different value later
    -- means the poster was changed in Plex.
    pushed_thumb TEXT,
    pushed_at TEXT,
    last_error TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (connection_id, title_id, provider_item_id)
);

INSERT INTO poster_overlay_plex_state_0270
    (connection_id, title_id, provider_item_id, pushed_output_hash, pushed_thumb,
     pushed_at, last_error, updated_at)
SELECT connection_id, title_id, provider_item_id, pushed_output_hash, pushed_thumb,
       pushed_at, last_error, updated_at
  FROM poster_overlay_plex_state;

DROP TABLE poster_overlay_plex_state;

ALTER TABLE poster_overlay_plex_state_0270 RENAME TO poster_overlay_plex_state;

CREATE INDEX idx_poster_overlay_plex_state_title ON poster_overlay_plex_state(title_id);
