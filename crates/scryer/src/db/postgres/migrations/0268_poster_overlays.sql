-- Poster overlays: per-library enablement, custom SVG templates, global
-- runtime settings, and per-title render state.
--
-- These tables only reference existing product tables. Overlay output is a
-- derived presentation cache: pristine originals and rendered posters live on
-- disk under `<data_dir>/overlays/`, and `poster_overlay_state` records what
-- was rendered from what, so a rebuild happens only when `input_hash` changes.

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
