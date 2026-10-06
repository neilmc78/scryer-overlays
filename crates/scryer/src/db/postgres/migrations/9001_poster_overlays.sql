-- Poster overlays (a local addition kept outside upstream numbering).
--
-- Numbered 9001, far above upstream's migrations, so the dev's new
-- migrations keep their own numbers and apply alongside it on every update.
-- The runner applies every migration a database has not run yet, so
-- upstream's later migrations still apply on a database that already ran
-- this one.

CREATE TABLE poster_overlay_templates (
    id text PRIMARY KEY NOT NULL,
    name text NOT NULL,
    svg text NOT NULL,
    content_hash text NOT NULL,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL
);

CREATE TABLE poster_overlay_libraries (
    library_id text PRIMARY KEY NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    enabled boolean NOT NULL DEFAULT false,
    -- NULL selects the built-in default template.
    template_id text REFERENCES poster_overlay_templates(id) ON DELETE SET NULL,
    updated_at timestamptz NOT NULL
);

CREATE TABLE poster_overlay_settings (
    id text PRIMARY KEY NOT NULL,
    parallelism bigint NOT NULL DEFAULT 3,
    reconcile_interval_seconds bigint NOT NULL DEFAULT 21600,
    plex_push_enabled boolean NOT NULL DEFAULT false,
    updated_at timestamptz NOT NULL
);

CREATE TABLE poster_overlay_state (
    title_id text PRIMARY KEY NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    original_path text,
    original_hash text,
    -- Identity of the upstream artwork the original was taken from.
    source_identity text,
    input_hash text,
    output_hash text,
    template_version text,
    fields_json text,
    rendered_at timestamptz,
    last_error text,
    updated_at timestamptz NOT NULL
);

-- Posters pushed to Plex: per connection, title and Plex item (a movie Plex
-- shows as several versions has one item per version), what was uploaded and
-- the item's `thumb` right after, so a pass uploads again only when the poster
-- changed or someone else replaced it in Plex.
CREATE TABLE poster_overlay_plex_state (
    connection_id text NOT NULL REFERENCES media_server_connections(id) ON DELETE CASCADE,
    title_id text NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    provider_item_id text NOT NULL,
    pushed_output_hash text,
    pushed_thumb text,
    pushed_at timestamptz,
    last_error text,
    updated_at timestamptz NOT NULL,
    PRIMARY KEY (connection_id, title_id, provider_item_id)
);

CREATE INDEX idx_poster_overlay_plex_state_title ON poster_overlay_plex_state(title_id);
