-- Poster overlays (a local addition kept outside upstream numbering).
--
-- Numbered 9001, far above upstream's migrations, so the dev's new
-- migrations keep their own numbers and apply alongside it on every update.
-- The runner applies every migration a database has not run yet, so
-- upstream's later migrations still apply on a database that already ran
-- this one.

CREATE TABLE poster_overlay_templates (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    svg TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE poster_overlay_libraries (
    library_id TEXT PRIMARY KEY NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    enabled INTEGER NOT NULL DEFAULT 0,
    -- NULL selects the built-in default template.
    template_id TEXT REFERENCES poster_overlay_templates(id) ON DELETE SET NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE poster_overlay_settings (
    id TEXT PRIMARY KEY NOT NULL,
    parallelism INTEGER NOT NULL DEFAULT 3,
    reconcile_interval_seconds INTEGER NOT NULL DEFAULT 21600,
    plex_push_enabled INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);

CREATE TABLE poster_overlay_state (
    title_id TEXT PRIMARY KEY NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    original_path TEXT,
    original_hash TEXT,
    -- Identity of the upstream artwork the original was taken from.
    source_identity TEXT,
    input_hash TEXT,
    output_hash TEXT,
    template_version TEXT,
    fields_json TEXT,
    rendered_at TEXT,
    last_error TEXT,
    updated_at TEXT NOT NULL
);

-- Posters pushed to Plex: per connection, title and Plex item (a movie Plex
-- shows as several versions has one item per version), what was uploaded and
-- the item's `thumb` right after, so a pass uploads again only when the poster
-- changed or someone else replaced it in Plex.
CREATE TABLE poster_overlay_plex_state (
    connection_id TEXT NOT NULL REFERENCES media_server_connections(id) ON DELETE CASCADE,
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    provider_item_id TEXT NOT NULL,
    pushed_output_hash TEXT,
    pushed_thumb TEXT,
    pushed_at TEXT,
    last_error TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (connection_id, title_id, provider_item_id)
);

CREATE INDEX idx_poster_overlay_plex_state_title ON poster_overlay_plex_state(title_id);
