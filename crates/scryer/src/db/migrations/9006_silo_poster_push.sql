-- Silo poster push (a local addition in the 9001+ block).
--
-- Push records share poster_overlay_plex_state, keyed by the Jellyfin
-- connection that holds Silo's API key, so only the switch and Silo's native
-- API address are new.

ALTER TABLE poster_overlay_settings ADD COLUMN silo_push_enabled INTEGER NOT NULL DEFAULT 0;
ALTER TABLE poster_overlay_settings ADD COLUMN silo_api_url TEXT;
