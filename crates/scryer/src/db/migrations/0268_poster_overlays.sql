-- Poster overlays: per-library enablement, custom SVG templates, global
-- runtime settings, and per-title render state.
--
-- These tables only reference existing product tables. Overlay output is a
-- derived presentation cache: pristine originals and rendered posters live on
-- disk under `<data_dir>/overlays/`, and `poster_overlay_state` records what
-- was rendered from what, so a rebuild happens only when `input_hash` changes.

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
