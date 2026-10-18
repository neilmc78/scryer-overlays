-- Plex collections from lists (a local addition in the 9001+ block).
--
-- A public list switched on here is mirrored into Plex as a collection named
-- after the list. Members record which Plex items Scryer added to which
-- collection, so only collections Scryer manages are ever changed. Members
-- keep no foreign key to lists or titles: when either goes away, the row is
-- what lets the next pass take the item back out of the collection.

CREATE TABLE plex_collection_lists (
    subscription_id text PRIMARY KEY NOT NULL REFERENCES list_subscriptions(id) ON DELETE CASCADE,
    enabled boolean NOT NULL DEFAULT false,
    updated_at timestamptz NOT NULL
);

CREATE TABLE plex_collection_members (
    connection_id text NOT NULL REFERENCES media_server_connections(id) ON DELETE CASCADE,
    subscription_id text NOT NULL,
    provider_item_id text NOT NULL,
    title_id text NOT NULL,
    collection_name text NOT NULL,
    pushed_at timestamptz,
    last_error text,
    updated_at timestamptz NOT NULL,
    PRIMARY KEY (connection_id, subscription_id, provider_item_id)
);

CREATE INDEX idx_plex_collection_members_subscription ON plex_collection_members(subscription_id);
