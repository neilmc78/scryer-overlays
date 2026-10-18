//! Mirroring public lists into Plex as collections.
//!
//! A list switched on here becomes a Plex collection named after the list.
//! Its items are the Plex items of the titles still on the list: the item a
//! catalog scan matched, or every version of a movie Plex splits by edition.
//!
//! Only collections Scryer manages are changed. Each item Scryer adds is
//! recorded, and an item is taken out of a collection only by removing that
//! one collection name from it, so collections made in Plex are never
//! touched. Nothing is deleted on the server; Plex drops a collection once
//! its last item leaves.
//!
//! Personal lists are never mirrored: a Plex collection would show what a
//! member follows to everyone who uses the server.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::Utc;
use scryer_domain::{
    AppPermission, ListMembership, ListScope, ListSubscription, MediaServerConnection,
    MediaServerPlaybackEntityKind, User,
};

use super::collection_order::{
    MAX_LISTS_PER_PROVIDER, assign_slot, move_list, move_provider, ordered, sort_title,
};
use super::fields::blake3_hex;
use super::plex::PlexPush;
use super::ports::{
    CollectionPosterRequest, PlexCollectionMember, PlexCollectionRef, PlexCollectionTitle,
    PlexPosterItem,
};
use super::service::AppPosterOverlayServices;
use crate::lists::ListSubscriptionQuery;
use crate::{AppError, AppResult, AppUseCase};

/// One list to mirror: its collection name and the titles on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlexCollectionSource {
    pub subscription_id: String,
    pub name: String,
    pub title_ids: Vec<String>,
    /// The sort title Plex should sort the collection by, such as
    /// `!1010_IMDB Top 250 Movies`; `None` leaves it to Plex.
    pub sort_title: Option<String>,
    /// The poster Scryer draws for the collection; `None` leaves it to Plex.
    pub poster: Option<CollectionPosterRequest>,
}

#[derive(Default)]
struct LookCounts {
    sorted: usize,
    posters: usize,
    failed: usize,
}

/// Bumped when the poster layout changes, so every poster is drawn again.
const COLLECTION_POSTER_REVISION: &str = "1";

/// Identity of a poster design: what is drawn, and how.
fn collection_poster_hash(poster: &CollectionPosterRequest) -> String {
    blake3_hex(
        format!(
            "{COLLECTION_POSTER_REVISION}|{:02x}{:02x}{:02x}|{}|{}",
            poster.background[0],
            poster.background[1],
            poster.background[2],
            poster.logo.as_deref().unwrap_or(""),
            poster.text
        )
        .as_bytes(),
    )
}

/// The provider's bundled logo, when there is one.
pub fn provider_poster_logo(provider: &str) -> Option<&'static str> {
    [
        "imdb",
        "tmdb",
        "tvdb",
        "anilist",
        "trakt",
        "mal",
        "simkl",
        "letterboxd",
        "mdblist",
    ]
    .into_iter()
    .find(|logo| *logo == provider)
}

/// The provider's poster colour: a dark brand tone that white text reads on.
pub fn provider_poster_colour(provider: &str) -> &'static str {
    match provider {
        "imdb" => "#7A6200",
        "tmdb" => "#0D253F",
        "tvdb" => "#1B3A2A",
        "anilist" => "#1F2A44",
        "trakt" => "#6B1414",
        "mal" => "#2E51A2",
        "simkl" => "#1A1A1A",
        "letterboxd" => "#202830",
        "mdblist" => "#1D2D44",
        _ => "#333A45",
    }
}

/// `#rrggbb` as bytes.
pub fn parse_poster_colour(value: &str) -> Option<[u8; 3]> {
    let hex = value.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// Labels longer than this are refused.
pub const MAX_COLLECTION_POSTER_TEXT: usize = 60;

/// The poster a list's settings describe.
fn poster_request(
    provider: &str,
    name: &str,
    background: Option<&str>,
    text: Option<&str>,
    logo: bool,
) -> CollectionPosterRequest {
    CollectionPosterRequest {
        background: background
            .and_then(parse_poster_colour)
            .or_else(|| parse_poster_colour(provider_poster_colour(provider)))
            .unwrap_or([0x33, 0x3a, 0x45]),
        logo: logo
            .then(|| provider_poster_logo(provider).map(str::to_string))
            .flatten(),
        text: text
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .unwrap_or(name)
            .to_string(),
    }
}

/// What a collection sync did across every Plex server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlexCollectionCounts {
    /// Items put in a collection.
    pub added: usize,
    /// Items taken out of a collection.
    pub removed: usize,
    pub unchanged: usize,
    /// Collections given a new sort title.
    pub sorted: usize,
    /// Collection posters uploaded or unlocked.
    pub posters: usize,
    /// Servers skipped because they were in their maintenance hours.
    pub deferred: usize,
    pub failed: usize,
}

/// A public list as the Lists page shows its Plex collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlexCollectionListStatus {
    pub subscription_id: String,
    /// The list's name, which is also the collection's.
    pub name: String,
    pub enabled: bool,
    /// Titles on the list that are in the library.
    pub titles: usize,
    /// Plex items currently in the collection.
    pub pushed_items: usize,
    /// Plex items whose last change failed.
    pub failed_items: usize,
    pub last_error: Option<String>,
    /// The list's provider, whose block of sort numbers it sits in.
    pub provider: String,
    /// The provider's block (1 is `!1000_`), once it has one.
    pub slot: Option<i64>,
    /// Place inside the provider's block, from 0, while mirrored.
    pub position: Option<i64>,
    /// The collection's sort title in Plex, while mirrored.
    pub sort_title: Option<String>,
    /// Whether Scryer draws the collection's poster.
    pub poster_enabled: bool,
    /// The poster's colour as `#rrggbb`: the chosen one, else the
    /// provider's.
    pub poster_background: String,
    /// The poster's label when shortened; `None` shows the list's name.
    pub poster_text: Option<String>,
    /// Whether the provider's logo is drawn.
    pub poster_logo: bool,
    /// Whether Scryer has a logo for the provider.
    pub poster_logo_available: bool,
}

/// Titles still on the list: in the library and not departed.
fn member_title_ids(rows: &[ListMembership]) -> Vec<String> {
    rows.iter()
        .filter(|row| row.left_at.is_none())
        .filter_map(|row| row.title_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

type MemberKey = (String, String);

impl AppPosterOverlayServices {
    /// Bring every Plex server's list collections in line with `sources`.
    /// A managed item whose list is not among `sources` leaves its
    /// collection. Failures are recorded on the member rows and retried on
    /// the next sync.
    pub async fn sync_plex_collections(
        &self,
        sources: &[PlexCollectionSource],
    ) -> PlexCollectionCounts {
        let mut counts = PlexCollectionCounts::default();
        let Some(plex) = self.plex.as_ref() else {
            return counts;
        };
        let connections = match plex.connections().await {
            Ok(connections) => connections,
            Err(error) => {
                tracing::warn!(%error, "could not read Plex connections for collections");
                counts.failed += 1;
                return counts;
            }
        };
        let members = match self.repository.list_plex_collection_members().await {
            Ok(members) => members,
            Err(error) => {
                tracing::warn!(%error, "could not read Plex collection members");
                counts.failed += 1;
                return counts;
            }
        };
        if connections.is_empty() {
            return counts;
        }
        let titles = self
            .repository
            .list_plex_collection_titles()
            .await
            .unwrap_or_default();
        let mut items_by_title: HashMap<(String, String), Vec<String>> = HashMap::new();
        for connection in &connections {
            let existing = members
                .iter()
                .filter(|member| member.connection_id == connection.id)
                .cloned()
                .collect::<Vec<_>>();
            if sources.is_empty() && existing.is_empty() {
                continue;
            }
            if plex.in_maintenance(connection).await {
                counts.deferred += 1;
                continue;
            }
            let mut desired: BTreeMap<MemberKey, (String, String)> = BTreeMap::new();
            for source in sources {
                for title_id in &source.title_ids {
                    let cache_key = (connection.id.clone(), title_id.clone());
                    if !items_by_title.contains_key(&cache_key) {
                        let keys = match self.collection_items(plex, connection, title_id).await {
                            Ok(keys) => keys,
                            Err(error) => {
                                tracing::debug!(
                                    title_id,
                                    connection_id = %connection.id,
                                    %error,
                                    "could not find a title's Plex items for a collection"
                                );
                                Vec::new()
                            }
                        };
                        items_by_title.insert(cache_key.clone(), keys);
                    }
                    for key in &items_by_title[&cache_key] {
                        desired.insert(
                            (source.subscription_id.clone(), key.clone()),
                            (title_id.clone(), source.name.clone()),
                        );
                    }
                }
            }
            let result = self
                .apply_collections(plex, connection, &desired, &existing)
                .await;
            counts.added += result.added;
            counts.removed += result.removed;
            counts.unchanged += result.unchanged;
            counts.failed += result.failed;
            let looks = self
                .apply_collection_looks(plex, connection, sources, &desired, &titles)
                .await;
            counts.sorted += looks.sorted;
            counts.posters += looks.posters;
            counts.failed += looks.failed;
        }
        if counts.added
            + counts.removed
            + counts.sorted
            + counts.posters
            + counts.failed
            + counts.deferred
            > 0
        {
            tracing::info!(
                added = counts.added,
                removed = counts.removed,
                sorted = counts.sorted,
                posters = counts.posters,
                deferred_for_maintenance = counts.deferred,
                failed = counts.failed,
                "Plex list collections synced"
            );
        }
        counts
    }

    /// Give each list's collection on `connection` its sort title and, when
    /// switched on, its poster, in every section it exists in. Plex is asked
    /// only when Scryer's side changed since the last push or Plex made the
    /// collection anew; edits made in Plex are not read back, so a poster
    /// changed there stays until the design changes here.
    async fn apply_collection_looks(
        &self,
        plex: &PlexPush,
        connection: &MediaServerConnection,
        sources: &[PlexCollectionSource],
        desired: &BTreeMap<MemberKey, (String, String)>,
        titles: &[PlexCollectionTitle],
    ) -> LookCounts {
        let mut counts = LookCounts::default();
        let recorded = titles
            .iter()
            .filter(|title| title.connection_id == connection.id)
            .collect::<Vec<_>>();
        for title in recorded.iter().filter(|title| {
            !sources.iter().any(|source| {
                source.subscription_id == title.subscription_id && source.sort_title.is_some()
            })
        }) {
            let _ = self
                .repository
                .delete_plex_collection_title(
                    &title.connection_id,
                    &title.subscription_id,
                    &title.section_id,
                )
                .await;
        }
        for source in sources {
            let Some(sort_title) = source.sort_title.as_deref() else {
                continue;
            };
            // A collection exists only once an item is in it.
            if !desired
                .keys()
                .any(|(subscription_id, _)| subscription_id == &source.subscription_id)
            {
                continue;
            }
            let collections = match plex.client.find_collections(connection, &source.name).await {
                Ok(collections) => collections,
                Err(error) => {
                    tracing::warn!(
                        connection_id = %connection.id,
                        collection = %source.name,
                        %error,
                        "could not find a Plex collection"
                    );
                    counts.failed += 1;
                    continue;
                }
            };
            for collection in collections {
                let previous = recorded
                    .iter()
                    .find(|title| {
                        title.subscription_id == source.subscription_id
                            && title.section_id == collection.section_id
                    })
                    .copied();
                let same_collection =
                    previous.is_some_and(|title| title.collection_key == collection.rating_key);
                let mut next = PlexCollectionTitle {
                    connection_id: connection.id.clone(),
                    subscription_id: source.subscription_id.clone(),
                    section_id: collection.section_id.clone(),
                    collection_key: collection.rating_key.clone(),
                    sort_title: previous
                        .filter(|_| same_collection)
                        .map(|title| title.sort_title.clone())
                        .unwrap_or_default(),
                    poster_hash: previous
                        .filter(|_| same_collection)
                        .and_then(|title| title.poster_hash.clone()),
                    poster_thumb: previous
                        .filter(|_| same_collection)
                        .and_then(|title| title.poster_thumb.clone()),
                };
                let before = next.clone();
                if next.sort_title != sort_title {
                    match plex
                        .client
                        .set_collection_sort_title(connection, &collection, sort_title)
                        .await
                    {
                        Ok(()) => {
                            counts.sorted += 1;
                            next.sort_title = sort_title.to_string();
                        }
                        Err(error) => {
                            tracing::warn!(
                                connection_id = %connection.id,
                                collection = %source.name,
                                %error,
                                "could not set a Plex collection's sort title"
                            );
                            counts.failed += 1;
                        }
                    }
                }
                match self
                    .apply_collection_poster(plex, connection, &collection, source, &mut next)
                    .await
                {
                    Ok(true) => counts.posters += 1,
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(
                            connection_id = %connection.id,
                            collection = %source.name,
                            %error,
                            "could not update a Plex collection's poster"
                        );
                        counts.failed += 1;
                    }
                }
                if next != before || !same_collection {
                    let _ = self.repository.save_plex_collection_title(&next).await;
                }
            }
        }
        counts
    }

    /// Upload and lock the source's poster when its design differs from the
    /// one last uploaded to this collection; unlock it when the poster was
    /// switched off. Returns whether Plex was changed.
    async fn apply_collection_poster(
        &self,
        plex: &PlexPush,
        connection: &MediaServerConnection,
        collection: &PlexCollectionRef,
        source: &PlexCollectionSource,
        next: &mut PlexCollectionTitle,
    ) -> AppResult<bool> {
        let item = PlexPosterItem {
            rating_key: collection.rating_key.clone(),
            item_type: "collection".into(),
            section_id: collection.section_id.clone(),
            thumb: None,
        };
        let Some(poster) = source.poster.as_ref() else {
            if next.poster_hash.is_none() {
                return Ok(false);
            }
            // Switched off: Plex keeps whatever it shows, unlocked.
            plex.client
                .set_poster_locked(connection, &item, false)
                .await?;
            next.poster_hash = None;
            next.poster_thumb = None;
            return Ok(true);
        };
        let hash = collection_poster_hash(poster);
        if next.poster_hash.as_deref() == Some(hash.as_str()) {
            return Ok(false);
        }
        let jpeg = self.engine.render_collection_poster(poster.clone()).await?;
        plex.client.upload_poster(connection, &item, jpeg).await?;
        plex.client
            .set_poster_locked(connection, &item, true)
            .await?;
        next.poster_thumb = plex
            .client
            .item(connection, &collection.rating_key)
            .await?
            .and_then(|item| item.thumb);
        next.poster_hash = Some(hash);
        Ok(true)
    }

    /// The Plex items a title has on `connection`: the one the catalog scan
    /// matched, else each version of a movie found by title and TMDB id.
    async fn collection_items(
        &self,
        plex: &PlexPush,
        connection: &MediaServerConnection,
        title_id: &str,
    ) -> AppResult<Vec<String>> {
        let mapped = plex
            .connections
            .list_playback_items_for_entity(MediaServerPlaybackEntityKind::Title, title_id)
            .await?
            .into_iter()
            .filter(|item| item.connection_id == connection.id)
            .map(|item| item.provider_item_id)
            .collect::<Vec<_>>();
        if !mapped.is_empty() {
            return Ok(mapped);
        }
        let Some(inputs) = self.repository.load_inputs(title_id).await? else {
            return Ok(Vec::new());
        };
        let (Some(name), Some(tmdb_id)) = (inputs.title_name, inputs.tmdb_id) else {
            return Ok(Vec::new());
        };
        if inputs.facet.as_deref() != Some("movie") {
            return Ok(Vec::new());
        }
        Ok(plex
            .client
            .find_movie_versions(connection, &name, &tmdb_id)
            .await?
            .into_iter()
            .map(|version| version.item.rating_key)
            .collect())
    }

    /// Change each Plex item on `connection` whose managed collections
    /// differ from `desired`, and record what each item now carries.
    async fn apply_collections(
        &self,
        plex: &PlexPush,
        connection: &MediaServerConnection,
        desired: &BTreeMap<MemberKey, (String, String)>,
        existing: &[PlexCollectionMember],
    ) -> PlexCollectionCounts {
        let mut counts = PlexCollectionCounts::default();
        let existing_by_key = existing
            .iter()
            .map(|member| {
                (
                    (
                        member.subscription_id.clone(),
                        member.provider_item_id.clone(),
                    ),
                    member,
                )
            })
            .collect::<HashMap<_, _>>();
        let mut item_keys = desired
            .keys()
            .map(|(_, item)| item.clone())
            .collect::<BTreeSet<_>>();
        item_keys.extend(
            existing
                .iter()
                .map(|member| member.provider_item_id.clone()),
        );

        for item_key in item_keys {
            // Names this item should carry, and the rows that ask for them.
            let wanted = desired
                .iter()
                .filter(|((_, item), _)| item == &item_key)
                .collect::<Vec<_>>();
            let wanted_names = wanted
                .iter()
                .map(|(_, (_, name))| name.clone())
                .collect::<BTreeSet<_>>();
            let rows = existing
                .iter()
                .filter(|member| member.provider_item_id == item_key)
                .collect::<Vec<_>>();
            let in_place = rows
                .iter()
                .filter(|member| member.pushed_at.is_some())
                .map(|member| member.collection_name.clone())
                .collect::<BTreeSet<_>>();
            let to_remove = in_place
                .difference(&wanted_names)
                .cloned()
                .collect::<Vec<_>>();
            let to_add = wanted
                .iter()
                .filter(|((subscription_id, _), (_, name))| {
                    existing_by_key
                        .get(&(subscription_id.clone(), item_key.clone()))
                        .is_none_or(|member| {
                            member.pushed_at.is_none()
                                || member.last_error.is_some()
                                || &member.collection_name != name
                        })
                })
                .map(|(_, (_, name))| name.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            if to_remove.is_empty() && to_add.is_empty() {
                counts.unchanged += wanted.len();
                // Rows of lists no longer mirrored whose name another list
                // still uses: the item keeps the name, the row goes.
                for member in rows.iter().filter(|member| {
                    !desired.contains_key(&(
                        member.subscription_id.clone(),
                        member.provider_item_id.clone(),
                    ))
                }) {
                    let _ = self.forget_member(member).await;
                }
                continue;
            }

            let outcome = self
                .change_item(plex, connection, &item_key, &to_remove, &to_add)
                .await;
            match outcome {
                Ok(true) => {
                    counts.added += to_add.len();
                    counts.removed += to_remove.len();
                    for ((subscription_id, _), (title_id, name)) in &wanted {
                        let _ = self
                            .repository
                            .save_plex_collection_member(&PlexCollectionMember {
                                connection_id: connection.id.clone(),
                                subscription_id: subscription_id.clone(),
                                provider_item_id: item_key.clone(),
                                title_id: title_id.clone(),
                                collection_name: name.clone(),
                                pushed_at: Some(Utc::now()),
                                last_error: None,
                            })
                            .await;
                    }
                    for member in rows.iter().filter(|member| {
                        !desired.contains_key(&(
                            member.subscription_id.clone(),
                            member.provider_item_id.clone(),
                        ))
                    }) {
                        let _ = self.forget_member(member).await;
                    }
                }
                // The item is gone from Plex: nothing left to change there.
                Ok(false) => {
                    for member in &rows {
                        let _ = self.forget_member(member).await;
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    tracing::warn!(
                        connection_id = %connection.id,
                        item = %item_key,
                        error = %message,
                        "could not update a Plex item's collections"
                    );
                    counts.failed += 1;
                    for ((subscription_id, _), (title_id, name)) in &wanted {
                        let previous = existing_by_key
                            .get(&(subscription_id.clone(), item_key.clone()))
                            .copied();
                        let _ = self
                            .repository
                            .save_plex_collection_member(&PlexCollectionMember {
                                connection_id: connection.id.clone(),
                                subscription_id: subscription_id.clone(),
                                provider_item_id: item_key.clone(),
                                title_id: title_id.clone(),
                                // Keep the name the item still carries so a
                                // retried rename removes the right one.
                                collection_name: previous
                                    .filter(|member| member.pushed_at.is_some())
                                    .map_or_else(
                                        || name.clone(),
                                        |member| member.collection_name.clone(),
                                    ),
                                pushed_at: previous.and_then(|member| member.pushed_at),
                                last_error: Some(message.clone()),
                            })
                            .await;
                    }
                    for member in rows.iter().filter(|member| {
                        !desired.contains_key(&(
                            member.subscription_id.clone(),
                            member.provider_item_id.clone(),
                        ))
                    }) {
                        let _ = self
                            .repository
                            .save_plex_collection_member(&PlexCollectionMember {
                                last_error: Some(message.clone()),
                                ..(*member).clone()
                            })
                            .await;
                    }
                }
            }
        }
        counts
    }

    /// Remove `to_remove` and add `to_add` on one item. `Ok(false)` when the
    /// item no longer exists in Plex.
    async fn change_item(
        &self,
        plex: &PlexPush,
        connection: &MediaServerConnection,
        item_key: &str,
        to_remove: &[String],
        to_add: &[String],
    ) -> AppResult<bool> {
        let Some(item): Option<PlexPosterItem> = plex.client.item(connection, item_key).await?
        else {
            return Ok(false);
        };
        if !to_remove.is_empty() {
            plex.client
                .remove_item_collections(connection, &item, to_remove)
                .await?;
        }
        if !to_add.is_empty() {
            plex.client
                .add_item_collections(connection, &item, to_add)
                .await?;
        }
        Ok(true)
    }

    async fn forget_member(&self, member: &PlexCollectionMember) -> AppResult<()> {
        self.repository
            .delete_plex_collection_member(
                &member.connection_id,
                &member.subscription_id,
                &member.provider_item_id,
            )
            .await
    }

    /// Ask the worker to sync list collections now.
    pub fn request_collection_sync(&self) {
        self.collections_wake.notify_one();
    }
}

/// Where each mirrored list sorts: its provider's block and its place in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CollectionPlace {
    slot: i64,
    position: i64,
}

impl AppUseCase {
    fn require_collections(&self) -> AppResult<&AppPosterOverlayServices> {
        self.poster_overlays()
            .ok_or_else(|| AppError::Validation("Plex collections are not available".into()))
    }

    async fn collection_subscriptions(&self) -> AppResult<Vec<ListSubscription>> {
        Ok(self
            .services
            .lists
            .subscriptions
            .list(ListSubscriptionQuery::public())
            .await?
            .into_iter()
            .filter(|subscription| subscription.scope == ListScope::Public)
            .collect())
    }

    /// The block and place of every mirrored public list. Providers without
    /// a block are given one and lists without a place go to the end of
    /// their provider's block; positions are kept gap-free. Anything
    /// changed is stored.
    async fn plex_collection_places(
        &self,
        overlays: &AppPosterOverlayServices,
        subscriptions: &[ListSubscription],
    ) -> AppResult<HashMap<String, CollectionPlace>> {
        let repository = &overlays.repository;
        let lists = repository.list_plex_collection_lists().await?;
        let provider_of = subscriptions
            .iter()
            .map(|subscription| {
                (
                    subscription.id.as_str(),
                    subscription.source.provider.as_str(),
                )
            })
            .collect::<HashMap<_, _>>();
        let mut by_provider: BTreeMap<String, Vec<(String, Option<i64>)>> = BTreeMap::new();
        for list in lists.iter().filter(|list| list.enabled) {
            if let Some(provider) = provider_of.get(list.subscription_id.as_str()) {
                by_provider
                    .entry((*provider).to_string())
                    .or_default()
                    .push((list.subscription_id.clone(), list.position));
            }
        }
        let stored_slots = repository
            .list_plex_collection_slots()
            .await?
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let mut slots = stored_slots.clone();
        let in_use = by_provider.keys().cloned().collect::<BTreeSet<_>>();
        for provider in &in_use {
            match assign_slot(&slots, provider, &in_use) {
                Ok((slot, reclaimed)) => {
                    if let Some(holder) = reclaimed {
                        slots.remove(&holder);
                    }
                    slots.insert(provider.clone(), slot);
                }
                Err(error) => tracing::warn!(provider, %error, "a list provider has no Plex block"),
            }
        }
        if slots != stored_slots {
            let entries = slots.clone().into_iter().collect::<Vec<_>>();
            repository.replace_plex_collection_slots(&entries).await?;
        }
        let mut places = HashMap::new();
        for (provider, provider_lists) in &by_provider {
            let Some(slot) = slots.get(provider).copied() else {
                continue;
            };
            let stored = provider_lists
                .iter()
                .cloned()
                .collect::<HashMap<String, Option<i64>>>();
            for (index, subscription_id) in ordered(provider_lists).into_iter().enumerate() {
                let position = i64::try_from(index).unwrap_or(i64::MAX);
                if position >= MAX_LISTS_PER_PROVIDER {
                    tracing::warn!(provider, "too many lists to number in one Plex block");
                    break;
                }
                if stored.get(&subscription_id).copied().flatten() != Some(position) {
                    repository
                        .set_plex_collection_list_position(&subscription_id, Some(position))
                        .await?;
                }
                places.insert(subscription_id, CollectionPlace { slot, position });
            }
        }
        Ok(places)
    }

    /// The public lists that are switched on, with the titles still on each.
    async fn plex_collection_sources(&self) -> AppResult<Vec<PlexCollectionSource>> {
        let Some(overlays) = self.poster_overlays() else {
            return Ok(Vec::new());
        };
        let subscriptions = self.collection_subscriptions().await?;
        let places = self
            .plex_collection_places(overlays, &subscriptions)
            .await?;
        let enabled = overlays
            .repository
            .list_plex_collection_lists()
            .await?
            .into_iter()
            .filter(|list| list.enabled)
            .map(|list| (list.subscription_id.clone(), list))
            .collect::<HashMap<_, _>>();
        let mut sources = Vec::new();
        for subscription in subscriptions {
            let Some(list) = enabled.get(&subscription.id) else {
                continue;
            };
            let name = subscription.name.trim().to_string();
            if name.is_empty() {
                continue;
            }
            let rows = self
                .services
                .lists
                .memberships
                .list_by_subscription(&subscription.id)
                .await?;
            sources.push(PlexCollectionSource {
                sort_title: places
                    .get(&subscription.id)
                    .map(|place| sort_title(place.slot, place.position, &name)),
                poster: list.poster_enabled.then(|| {
                    poster_request(
                        &subscription.source.provider,
                        &name,
                        list.poster_background.as_deref(),
                        list.poster_text.as_deref(),
                        list.poster_logo,
                    )
                }),
                subscription_id: subscription.id,
                name,
                title_ids: member_title_ids(&rows),
            });
        }
        Ok(sources)
    }

    /// Sync every switched-on public list into its Plex collection.
    pub async fn sync_plex_list_collections(&self) -> AppResult<PlexCollectionCounts> {
        let Some(overlays) = self.poster_overlays() else {
            return Ok(PlexCollectionCounts::default());
        };
        let sources = self.plex_collection_sources().await?;
        Ok(overlays.sync_plex_collections(&sources).await)
    }

    /// Public lists with their Plex collection setting, order and progress.
    pub async fn plex_collection_lists(
        &self,
        actor: &User,
    ) -> AppResult<Vec<PlexCollectionListStatus>> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let Some(overlays) = self.poster_overlays() else {
            return Ok(Vec::new());
        };
        let subscriptions = self.collection_subscriptions().await?;
        let places = self
            .plex_collection_places(overlays, &subscriptions)
            .await?;
        let enabled = overlays
            .repository
            .list_plex_collection_lists()
            .await?
            .into_iter()
            .map(|list| (list.subscription_id.clone(), list))
            .collect::<HashMap<_, _>>();
        let slots = overlays
            .repository
            .list_plex_collection_slots()
            .await?
            .into_iter()
            .collect::<HashMap<_, _>>();
        let members = overlays.repository.list_plex_collection_members().await?;
        let mut statuses = Vec::new();
        for subscription in subscriptions {
            let rows = self
                .services
                .lists
                .memberships
                .list_by_subscription(&subscription.id)
                .await?;
            let own = members
                .iter()
                .filter(|member| member.subscription_id == subscription.id)
                .collect::<Vec<_>>();
            let place = places.get(&subscription.id).copied();
            let list = enabled.get(&subscription.id);
            statuses.push(PlexCollectionListStatus {
                enabled: list.is_some_and(|list| list.enabled),
                poster_enabled: list.is_some_and(|list| list.poster_enabled),
                poster_background: list
                    .and_then(|list| list.poster_background.clone())
                    .unwrap_or_else(|| {
                        provider_poster_colour(&subscription.source.provider).to_string()
                    }),
                poster_text: list.and_then(|list| list.poster_text.clone()),
                poster_logo: list.is_none_or(|list| list.poster_logo),
                poster_logo_available: provider_poster_logo(&subscription.source.provider)
                    .is_some(),
                titles: member_title_ids(&rows).len(),
                pushed_items: own
                    .iter()
                    .filter(|member| member.pushed_at.is_some() && member.last_error.is_none())
                    .count(),
                failed_items: own
                    .iter()
                    .filter(|member| member.last_error.is_some())
                    .count(),
                last_error: own.iter().find_map(|member| member.last_error.clone()),
                slot: place
                    .map(|place| place.slot)
                    .or_else(|| slots.get(&subscription.source.provider).copied()),
                position: place.map(|place| place.position),
                sort_title: place
                    .map(|place| sort_title(place.slot, place.position, subscription.name.trim())),
                provider: subscription.source.provider,
                subscription_id: subscription.id,
                name: subscription.name,
            });
        }
        Ok(statuses)
    }

    async fn collection_subscription(&self, subscription_id: &str) -> AppResult<ListSubscription> {
        let subscription = self
            .services
            .lists
            .subscriptions
            .get_by_id(subscription_id)
            .await?
            .ok_or_else(|| AppError::NotFound("list".into()))?;
        if subscription.scope != ListScope::Public {
            return Err(AppError::Validation(
                "only public lists can be mirrored into Plex".into(),
            ));
        }
        Ok(subscription)
    }

    /// Switch a public list's Plex collection on or off, then sync. A list
    /// switched on goes to the end of its provider's block.
    pub async fn set_plex_collection_list(
        &self,
        actor: &User,
        subscription_id: &str,
        enabled: bool,
    ) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let overlays = self.require_collections()?;
        let subscription = self.collection_subscription(subscription_id).await?;
        let repository = &overlays.repository;
        if enabled {
            // Refuse before switching on when the provider cannot get a block.
            let subscriptions = self.collection_subscriptions().await?;
            let provider_of = subscriptions
                .iter()
                .map(|entry| (entry.id.as_str(), entry.source.provider.as_str()))
                .collect::<HashMap<_, _>>();
            let in_use = repository
                .list_plex_collection_lists()
                .await?
                .into_iter()
                .filter(|list| list.enabled)
                .filter_map(|list| {
                    provider_of
                        .get(list.subscription_id.as_str())
                        .map(|provider| (*provider).to_string())
                })
                .chain(std::iter::once(subscription.source.provider.clone()))
                .collect::<BTreeSet<_>>();
            let slots = repository
                .list_plex_collection_slots()
                .await?
                .into_iter()
                .collect::<BTreeMap<_, _>>();
            assign_slot(&slots, &subscription.source.provider, &in_use)
                .map_err(AppError::Validation)?;
        }
        repository
            .set_plex_collection_list(subscription_id, enabled)
            .await?;
        if !enabled {
            repository
                .set_plex_collection_list_position(subscription_id, None)
                .await?;
        }
        let subscriptions = self.collection_subscriptions().await?;
        self.plex_collection_places(overlays, &subscriptions)
            .await?;
        overlays.request_collection_sync();
        Ok(())
    }

    /// Move a mirrored list one place up or down inside its provider's
    /// block, then sync so Plex sorts it there.
    pub async fn move_plex_collection_list(
        &self,
        actor: &User,
        subscription_id: &str,
        up: bool,
    ) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let overlays = self.require_collections()?;
        let subscription = self.collection_subscription(subscription_id).await?;
        let subscriptions = self.collection_subscriptions().await?;
        let places = self
            .plex_collection_places(overlays, &subscriptions)
            .await?;
        let Some(place) = places.get(subscription_id).copied() else {
            return Err(AppError::Validation(
                "the list is not shown in Plex as a collection".into(),
            ));
        };
        let provider_lists = subscriptions
            .iter()
            .filter(|entry| entry.source.provider == subscription.source.provider)
            .filter_map(|entry| {
                places
                    .get(&entry.id)
                    .filter(|other| other.slot == place.slot)
                    .map(|other| (entry.id.clone(), Some(other.position)))
            })
            .collect::<Vec<_>>();
        let order = move_list(&ordered(&provider_lists), subscription_id, up);
        for (index, id) in order.iter().enumerate() {
            overlays
                .repository
                .set_plex_collection_list_position(id, Some(i64::try_from(index).unwrap_or(0)))
                .await?;
        }
        overlays.request_collection_sync();
        Ok(())
    }

    /// Move a provider's whole block of collections one block up or down,
    /// trading places with the provider there, then sync.
    pub async fn move_plex_collection_provider(
        &self,
        actor: &User,
        provider: &str,
        up: bool,
    ) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let overlays = self.require_collections()?;
        let slots = overlays
            .repository
            .list_plex_collection_slots()
            .await?
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        if !slots.contains_key(provider) {
            return Err(AppError::Validation(
                "the provider has no lists shown in Plex".into(),
            ));
        }
        let moved = move_provider(&slots, provider, up);
        if moved != slots {
            let entries = moved.into_iter().collect::<Vec<_>>();
            overlays
                .repository
                .replace_plex_collection_slots(&entries)
                .await?;
            overlays.request_collection_sync();
        }
        Ok(())
    }

    /// Check a poster design and resolve it into what is drawn.
    fn checked_poster(
        subscription: &ListSubscription,
        background: Option<&str>,
        text: Option<&str>,
        logo: bool,
    ) -> AppResult<(Option<String>, Option<String>, CollectionPosterRequest)> {
        let background = background
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                parse_poster_colour(value)
                    .map(|_| value.to_ascii_uppercase())
                    .ok_or_else(|| {
                        AppError::Validation(format!("\"{value}\" is not a colour; use #rrggbb"))
                    })
            })
            .transpose()?;
        let text = text
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if text
            .as_ref()
            .is_some_and(|text| text.chars().count() > MAX_COLLECTION_POSTER_TEXT)
        {
            return Err(AppError::Validation(format!(
                "the label may be at most {MAX_COLLECTION_POSTER_TEXT} characters"
            )));
        }
        let request = poster_request(
            &subscription.source.provider,
            subscription.name.trim(),
            background.as_deref(),
            text.as_deref(),
            logo,
        );
        Ok((background, text, request))
    }

    /// Choose a mirrored list's poster: whether Scryer draws one, its
    /// colour (`None` for the provider's), its label (`None` for the list's
    /// name) and whether the logo shows. Then sync.
    pub async fn set_plex_collection_poster(
        &self,
        actor: &User,
        subscription_id: &str,
        enabled: bool,
        background: Option<&str>,
        text: Option<&str>,
        logo: bool,
    ) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let overlays = self.require_collections()?;
        let subscription = self.collection_subscription(subscription_id).await?;
        let mirrored = overlays
            .repository
            .list_plex_collection_lists()
            .await?
            .iter()
            .any(|list| list.subscription_id == subscription_id && list.enabled);
        if !mirrored {
            return Err(AppError::Validation(
                "show the list in Plex as a collection first".into(),
            ));
        }
        let (background, text, _) = Self::checked_poster(&subscription, background, text, logo)?;
        overlays
            .repository
            .set_plex_collection_poster(
                subscription_id,
                enabled,
                background.as_deref(),
                text.as_deref(),
                logo,
            )
            .await?;
        overlays.request_collection_sync();
        Ok(())
    }

    /// Draw a poster design for the editor without saving or pushing it.
    pub async fn preview_plex_collection_poster(
        &self,
        actor: &User,
        subscription_id: &str,
        background: Option<&str>,
        text: Option<&str>,
        logo: bool,
    ) -> AppResult<Vec<u8>> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        let overlays = self.require_collections()?;
        let subscription = self.collection_subscription(subscription_id).await?;
        let (_, _, request) = Self::checked_poster(&subscription, background, text, logo)?;
        overlays.engine.render_collection_poster(request).await
    }

    /// Ask for a collection sync now.
    pub async fn request_plex_collection_sync(&self, actor: &User) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageLists)
            .await?;
        self.require_collections()?.request_collection_sync();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_are_titles_still_on_the_list_once_each() {
        let row = |title: Option<&str>, left: bool| ListMembership {
            series_movie: None,
            subscription_id: "list".into(),
            item_key: title.unwrap_or("none").into(),
            rank: None,
            season: None,
            display_title: None,
            year: None,
            external_ids: Vec::new(),
            smg_title_id: None,
            title_id: title.map(str::to_string),
            request_id: None,
            kind: scryer_domain::MediaFacet::Movie,
            state: scryer_domain::ListMembershipState::InLibrary,
            state_reason: None,
            added_by_list: true,
            first_seen_at: Utc::now(),
            last_seen_at: Utc::now(),
            left_at: left.then(Utc::now),
            left_handled: false,
        };
        let rows = [
            row(Some("b"), false),
            row(Some("a"), false),
            row(Some("a"), false),
            row(Some("gone"), true),
            row(None, false),
        ];
        assert_eq!(
            member_title_ids(&rows),
            vec!["a".to_string(), "b".to_string()]
        );
    }
}
