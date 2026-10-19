-- Plex collections from lists (a local addition in the 9001+ block).
--
-- A public list switched on here is mirrored into Plex as a collection named
-- after the list. Members record which Plex items Scryer added to which
-- collection, so only collections Scryer manages are ever changed. Members
-- keep no foreign key to lists or titles: when either goes away, the row is
-- what lets the next pass take the item back out of the collection.

CREATE TABLE plex_collection_lists (
    subscription_id TEXT PRIMARY KEY NOT NULL REFERENCES list_subscriptions(id) ON DELETE CASCADE,
    enabled INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);

CREATE TABLE plex_collection_members (
    connection_id TEXT NOT NULL REFERENCES media_server_connections(id) ON DELETE CASCADE,
    subscription_id TEXT NOT NULL,
    provider_item_id TEXT NOT NULL,
    title_id TEXT NOT NULL,
    collection_name TEXT NOT NULL,
    pushed_at TEXT,
    last_error TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (connection_id, subscription_id, provider_item_id)
);

CREATE INDEX idx_plex_collection_members_subscription ON plex_collection_members(subscription_id);
