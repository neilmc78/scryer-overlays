-- Poster push server kind (a local addition in the 9001+ block).
--
-- The Silo push target can be Silo's native API or a Jellyfin server; both
-- keep their key on a Jellyfin connection and share the push records.

ALTER TABLE poster_overlay_settings ADD COLUMN silo_server_kind TEXT NOT NULL DEFAULT 'silo';
