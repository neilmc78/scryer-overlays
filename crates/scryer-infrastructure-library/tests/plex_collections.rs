//! Plex collections from lists end to end against a real datastore: a list
//! becomes a collection named after it, a rename moves its items to the new
//! name, titles that leave the list leave the collection, and collections
//! made in Plex are never touched.

#![cfg(feature = "image-processing")]

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::MediaServerConnectionRepository;
use scryer_application::overlays::{
    AppPosterOverlayServices, CollectionPosterRequest, PlexCollectionRef, PlexCollectionSource,
    PlexMaintenanceWindow, PlexMovieVersion, PlexPosterItem, PosterOverlayPlexClient,
    PosterOverlayRepository,
};
use scryer_application::{AppError, AppResult};
use scryer_domain::{
    AppPermissionMask, MediaServerConnection, MediaServerPlaybackEntityKind,
    MediaServerPlaybackItem, MediaServerProvider,
};
use scryer_infrastructure_datastore::postgres::PostgresServices;
use scryer_infrastructure_datastore::{MigrationMode, SqliteServices};
use scryer_infrastructure_library::overlays::{
    OverlayEngine, OverlaySourceFetch, PosterOverlayStore,
};
use scryer_infrastructure_sql::runtime::{SqlArg, SqlRuntime, StoreDatastore};

const CONNECTION: &str = "plex-1";
const LIST: &str = "list-1";
const MAPPED_TITLE: &str = "title-mapped";
const VERSIONED_TITLE: &str = "title-versions";
const MAPPED_ITEM: &str = "100";
const VERSION_ITEMS: [&str; 2] = ["200", "201"];
const OWN_COLLECTION: &str = "My Favourites";

struct NoFetch;

#[async_trait]
impl OverlaySourceFetch for NoFetch {
    async fn fetch(&self, _source_url: &str) -> AppResult<Vec<u8>> {
        Err(AppError::NotFound("no artwork in this test".into()))
    }
}

/// Plex items and the collections each carries.
#[derive(Default)]
struct FakePlex {
    collections: Mutex<HashMap<String, BTreeSet<String>>>,
    edits: AtomicUsize,
    fail: AtomicBool,
    /// Collection key per name while any item carries it; Plex drops an
    /// emptied collection and makes a new one when it is used again.
    keys: Mutex<HashMap<String, String>>,
    next_key: AtomicUsize,
    sort_titles: Mutex<HashMap<String, String>>,
    sort_calls: AtomicUsize,
    poster_uploads: Mutex<HashMap<String, usize>>,
    poster_locks: Mutex<HashMap<String, bool>>,
}

impl FakePlex {
    fn with_items() -> Self {
        let plex = Self::default();
        let mut collections = plex.collections.lock().unwrap();
        collections.insert(
            MAPPED_ITEM.into(),
            BTreeSet::from([OWN_COLLECTION.to_string()]),
        );
        for item in VERSION_ITEMS {
            collections.insert(item.into(), BTreeSet::new());
        }
        drop(collections);
        plex
    }

    fn of(&self, item: &str) -> Vec<String> {
        self.collections.lock().unwrap()[item]
            .iter()
            .cloned()
            .collect()
    }
}

#[async_trait]
impl PosterOverlayPlexClient for FakePlex {
    async fn item(
        &self,
        _connection: &MediaServerConnection,
        rating_key: &str,
    ) -> AppResult<Option<PlexPosterItem>> {
        Ok(self
            .collections
            .lock()
            .unwrap()
            .contains_key(rating_key)
            .then(|| PlexPosterItem {
                rating_key: rating_key.into(),
                item_type: "movie".into(),
                section_id: "1".into(),
                thumb: None,
            }))
    }

    async fn find_movie_versions(
        &self,
        _connection: &MediaServerConnection,
        title: &str,
        tmdb_id: &str,
    ) -> AppResult<Vec<PlexMovieVersion>> {
        if title != "Versions" || tmdb_id != "679" {
            return Ok(Vec::new());
        }
        Ok(VERSION_ITEMS
            .iter()
            .map(|key| PlexMovieVersion {
                item: PlexPosterItem {
                    rating_key: (*key).into(),
                    item_type: "movie".into(),
                    section_id: "1".into(),
                    thumb: None,
                },
                edition_title: None,
                files: Vec::new(),
            })
            .collect())
    }

    async fn current_poster(
        &self,
        _connection: &MediaServerConnection,
        _item: &PlexPosterItem,
    ) -> AppResult<Option<Vec<u8>>> {
        Ok(None)
    }

    async fn upload_poster(
        &self,
        _connection: &MediaServerConnection,
        item: &PlexPosterItem,
        jpeg: Vec<u8>,
    ) -> AppResult<()> {
        assert_eq!(item.item_type, "collection", "only collections get posters");
        assert!(image::load_from_memory(&jpeg).is_ok(), "a real image");
        *self
            .poster_uploads
            .lock()
            .unwrap()
            .entry(item.rating_key.clone())
            .or_default() += 1;
        Ok(())
    }

    async fn maintenance_window(
        &self,
        _connection: &MediaServerConnection,
    ) -> AppResult<Option<PlexMaintenanceWindow>> {
        Ok(None)
    }

    async fn set_poster_locked(
        &self,
        _connection: &MediaServerConnection,
        item: &PlexPosterItem,
        locked: bool,
    ) -> AppResult<()> {
        assert_eq!(item.item_type, "collection", "only collections get posters");
        self.poster_locks
            .lock()
            .unwrap()
            .insert(item.rating_key.clone(), locked);
        Ok(())
    }

    async fn add_item_collections(
        &self,
        _connection: &MediaServerConnection,
        item: &PlexPosterItem,
        names: &[String],
    ) -> AppResult<()> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(AppError::Repository(
                "Plex collection update: could not connect".into(),
            ));
        }
        self.edits.fetch_add(1, Ordering::SeqCst);
        self.collections
            .lock()
            .unwrap()
            .get_mut(&item.rating_key)
            .unwrap()
            .extend(names.iter().cloned());
        Ok(())
    }

    async fn remove_item_collections(
        &self,
        _connection: &MediaServerConnection,
        item: &PlexPosterItem,
        names: &[String],
    ) -> AppResult<()> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(AppError::Repository(
                "Plex collection update: could not connect".into(),
            ));
        }
        self.edits.fetch_add(1, Ordering::SeqCst);
        let mut collections = self.collections.lock().unwrap();
        let carried = collections.get_mut(&item.rating_key).unwrap();
        for name in names {
            carried.remove(name);
        }
        Ok(())
    }

    async fn find_collections(
        &self,
        _connection: &MediaServerConnection,
        name: &str,
    ) -> AppResult<Vec<PlexCollectionRef>> {
        let used = self
            .collections
            .lock()
            .unwrap()
            .values()
            .any(|carried| carried.contains(name));
        let mut keys = self.keys.lock().unwrap();
        if !used {
            keys.remove(name);
            return Ok(Vec::new());
        }
        let key = keys
            .entry(name.to_string())
            .or_insert_with(|| format!("c-{}", self.next_key.fetch_add(1, Ordering::SeqCst)))
            .clone();
        Ok(vec![PlexCollectionRef {
            section_id: "1".into(),
            rating_key: key,
        }])
    }

    async fn set_collection_sort_title(
        &self,
        _connection: &MediaServerConnection,
        collection: &PlexCollectionRef,
        sort_title: &str,
    ) -> AppResult<()> {
        self.sort_calls.fetch_add(1, Ordering::SeqCst);
        self.sort_titles
            .lock()
            .unwrap()
            .insert(collection.rating_key.clone(), sort_title.to_string());
        Ok(())
    }
}

impl FakePlex {
    /// The sort title of the collection currently named `name`.
    /// Poster uploads and the lock of the collection named `name`.
    fn poster_of(&self, name: &str) -> (usize, Option<bool>) {
        let Some(key) = self.keys.lock().unwrap().get(name).cloned() else {
            return (0, None);
        };
        (
            self.poster_uploads
                .lock()
                .unwrap()
                .get(&key)
                .copied()
                .unwrap_or(0),
            self.poster_locks.lock().unwrap().get(&key).copied(),
        )
    }

    fn sort_title_of(&self, name: &str) -> Option<String> {
        let used = self
            .collections
            .lock()
            .unwrap()
            .values()
            .any(|carried| carried.contains(name));
        if !used {
            // Plex drops a collection as soon as its last item leaves.
            self.keys.lock().unwrap().remove(name);
            return None;
        }
        let key = self.keys.lock().unwrap().get(name).cloned()?;
        self.sort_titles.lock().unwrap().get(&key).cloned()
    }
}

/// One Plex connection; only `MAPPED_TITLE` has a catalog match.
struct FakeConnections;

fn plex_connection() -> MediaServerConnection {
    MediaServerConnection {
        id: CONNECTION.into(),
        provider: MediaServerProvider::Plex,
        display_name: "Plex".into(),
        base_url: "http://plex.local:32400".into(),
        external_url: None,
        enabled: true,
        login_enabled: false,
        linking_enabled: false,
        auto_add_enabled: false,
        default_app_permissions: AppPermissionMask::default(),
        default_library_grants: Vec::new(),
        machine_id: Some("machine".into()),
        api_key: Some("token".into()),
        emby_server_id: None,
        emby_connect_enabled: false,
        path_mappings: Vec::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[async_trait]
impl MediaServerConnectionRepository for FakeConnections {
    async fn list(
        &self,
        _provider: Option<MediaServerProvider>,
    ) -> AppResult<Vec<MediaServerConnection>> {
        Ok(vec![plex_connection()])
    }
    async fn get_by_id(&self, _id: &str) -> AppResult<Option<MediaServerConnection>> {
        Ok(Some(plex_connection()))
    }
    async fn create(&self, connection: MediaServerConnection) -> AppResult<MediaServerConnection> {
        Ok(connection)
    }
    async fn update(&self, connection: MediaServerConnection) -> AppResult<MediaServerConnection> {
        Ok(connection)
    }
    async fn list_playback_items_for_entity(
        &self,
        entity_kind: MediaServerPlaybackEntityKind,
        entity_id: &str,
    ) -> AppResult<Vec<MediaServerPlaybackItem>> {
        Ok(
            (entity_kind == MediaServerPlaybackEntityKind::Title && entity_id == MAPPED_TITLE)
                .then(|| MediaServerPlaybackItem {
                    connection_id: CONNECTION.into(),
                    entity_kind,
                    entity_id: entity_id.into(),
                    provider_item_id: MAPPED_ITEM.into(),
                    last_seen_at: Utc::now(),
                })
                .into_iter()
                .collect(),
        )
    }
    async fn replace_playback_items_for_connection(
        &self,
        _connection_id: &str,
        _items: Vec<MediaServerPlaybackItem>,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn delete(&self, _id: &str) -> AppResult<()> {
        Ok(())
    }
    async fn has_external_accounts(&self, _id: &str) -> AppResult<bool> {
        Ok(false)
    }
    async fn has_notification_channels(&self, _id: &str) -> AppResult<bool> {
        Ok(false)
    }
}

async fn exec(datastore: &StoreDatastore, sql: &str, args: Vec<SqlArg>) {
    SqlRuntime::execute_write(datastore, "plex_collections_fixture", sql, args)
        .await
        .expect("fixture write");
}

async fn seed(datastore: &StoreDatastore) {
    let now = Utc::now();
    let stamp = || SqlArg::Timestamp(now);
    exec(
        datastore,
        "INSERT INTO libraries (id, facet, name, slug, created_at, updated_at)
         VALUES ('library-1', 'movie', 'Collection Fixture', 'collection-fixture', {}, {})",
        vec![stamp(), stamp()],
    )
    .await;
    exec(
        datastore,
        "INSERT INTO library_roots (id, library_id, path, normalized_path, created_at, updated_at)
         VALUES ('root-1', 'library-1', '/movies', '/movies', {}, {})",
        vec![stamp(), stamp()],
    )
    .await;
    for (id, name) in [(MAPPED_TITLE, "Mapped"), (VERSIONED_TITLE, "Versions")] {
        exec(
            datastore,
            "INSERT INTO titles (id, name, facet, created_at, library_id, root_folder_id)
             VALUES ({}, {}, 'movie', {}, 'library-1', 'root-1')",
            vec![SqlArg::Text(id.into()), SqlArg::Text(name.into()), stamp()],
        )
        .await;
    }
    exec(
        datastore,
        "INSERT INTO title_external_ids (id, title_id, source, external_id, created_at)
         VALUES ('ext-1', {}, 'tmdb', '679', {})",
        vec![SqlArg::Text(VERSIONED_TITLE.into()), stamp()],
    )
    .await;
    exec(
        datastore,
        "INSERT INTO media_server_connections (id, provider, display_name, base_url, created_at, updated_at)
         VALUES ({}, 'plex', 'Plex', 'http://plex.local:32400', {}, {})",
        vec![SqlArg::Text(CONNECTION.into()), stamp(), stamp()],
    )
    .await;
    exec(
        datastore,
        "INSERT INTO users (id, username, created_at, updated_at)
         VALUES ('owner-1', 'owner', {}, {})",
        vec![stamp(), stamp()],
    )
    .await;
    exec(
        datastore,
        "INSERT INTO list_subscriptions (id, scope, owner_user_id, provider, source_type,
                                         source_origin, name, mode, interval_seconds,
                                         created_at, updated_at)
         VALUES ({}, 'public', 'owner-1', 'imdb', 'chart', 'smg_chart',
                 'IMDB Top 250 Movies', 'add', 86400, {}, {})",
        vec![SqlArg::Text(LIST.into()), stamp(), stamp()],
    )
    .await;
}

fn source(name: &str, titles: &[&str]) -> PlexCollectionSource {
    PlexCollectionSource {
        subscription_id: LIST.into(),
        name: name.into(),
        title_ids: titles.iter().map(|title| (*title).to_string()).collect(),
        sort_title: None,
        poster: None,
    }
}

fn sorted_source(name: &str, sort_title: &str, titles: &[&str]) -> PlexCollectionSource {
    PlexCollectionSource {
        sort_title: Some(sort_title.into()),
        ..source(name, titles)
    }
}

#[tokio::test]
async fn lists_become_plex_collections_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let services = SqliteServices::new(dir.path().join("scryer.db").to_string_lossy())
        .await
        .expect("sqlite services");
    lists_become_plex_collections(services.datastore(), dir.path()).await;
}

#[tokio::test]
async fn lists_become_plex_collections_postgres() {
    let Some(admin_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        eprintln!("skipping PostgreSQL Plex collections test; SCRYER_TEST_POSTGRES_URL is not set");
        return;
    };
    let database = format!("scryer_collections_{}", uuid::Uuid::new_v4().simple());
    let admin = sqlx::PgPool::connect(&admin_url)
        .await
        .expect("postgres admin");
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {database}")))
        .execute(&admin)
        .await
        .expect("create test database");
    let mut url = url::Url::parse(&admin_url).expect("postgres url");
    url.set_path(&format!("/{database}"));
    let dir = tempfile::tempdir().unwrap();
    {
        let services = PostgresServices::new_with_mode(url.as_str(), MigrationMode::Apply)
            .await
            .expect("postgres services");
        lists_become_plex_collections(services.datastore(), dir.path()).await;
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE {database} WITH (FORCE)"
    )))
    .execute(&admin)
    .await
    .expect("drop test database");
}

async fn lists_become_plex_collections(datastore: StoreDatastore, data_dir: &std::path::Path) {
    seed(&datastore).await;
    let plex = Arc::new(FakePlex::with_items());
    let store = Arc::new(PosterOverlayStore::new(datastore.clone()));
    let overlays = AppPosterOverlayServices::new(
        store.clone(),
        Arc::new(OverlayEngine::new(data_dir, Arc::new(NoFetch)).unwrap()),
    )
    .with_plex(Arc::new(FakeConnections), plex.clone());

    // The switch is stored per list.
    store.set_plex_collection_list(LIST, true).await.unwrap();
    let lists = store.list_plex_collection_lists().await.unwrap();
    assert_eq!(lists.len(), 1);
    assert!(lists[0].enabled);

    // Both titles join: the matched item, and each version of the other.
    let name = "IMDB Top 250 Movies";
    let counts = overlays
        .sync_plex_collections(&[source(name, &[MAPPED_TITLE, VERSIONED_TITLE])])
        .await;
    assert_eq!((counts.added, counts.failed), (3, 0), "{counts:?}");
    assert_eq!(
        plex.of(MAPPED_ITEM),
        vec![name.to_string(), OWN_COLLECTION.to_string()]
    );
    for item in VERSION_ITEMS {
        assert_eq!(plex.of(item), vec![name.to_string()]);
    }
    assert_eq!(store.list_plex_collection_members().await.unwrap().len(), 3);

    // Nothing changed: Plex is not asked again.
    let edits = plex.edits.load(Ordering::SeqCst);
    let counts = overlays
        .sync_plex_collections(&[source(name, &[MAPPED_TITLE, VERSIONED_TITLE])])
        .await;
    assert_eq!((counts.added, counts.removed, counts.unchanged), (0, 0, 3));
    assert_eq!(plex.edits.load(Ordering::SeqCst), edits);

    // A rename moves every item to the new name.
    let renamed = "IMDb Top 250";
    overlays
        .sync_plex_collections(&[source(renamed, &[MAPPED_TITLE, VERSIONED_TITLE])])
        .await;
    assert_eq!(
        plex.of(MAPPED_ITEM),
        vec![renamed.to_string(), OWN_COLLECTION.to_string()]
    );
    assert_eq!(plex.of(VERSION_ITEMS[0]), vec![renamed.to_string()]);

    // A failed change is recorded and retried on the next sync.
    plex.fail.store(true, Ordering::SeqCst);
    let counts = overlays
        .sync_plex_collections(&[source(renamed, &[MAPPED_TITLE])])
        .await;
    assert!(counts.failed > 0, "{counts:?}");
    let members = store.list_plex_collection_members().await.unwrap();
    assert!(members.iter().any(|member| member.last_error.is_some()));
    plex.fail.store(false, Ordering::SeqCst);

    // The versioned title left the list: its items leave the collection.
    let counts = overlays
        .sync_plex_collections(&[source(renamed, &[MAPPED_TITLE])])
        .await;
    assert_eq!(counts.failed, 0, "{counts:?}");
    for item in VERSION_ITEMS {
        assert!(plex.of(item).is_empty(), "{item}: {:?}", plex.of(item));
    }
    let members = store.list_plex_collection_members().await.unwrap();
    assert_eq!(members.len(), 1);
    assert!(members[0].last_error.is_none());

    // Switched off: the last item leaves; the collection made in Plex stays.
    overlays.sync_plex_collections(&[]).await;
    assert_eq!(plex.of(MAPPED_ITEM), vec![OWN_COLLECTION.to_string()]);
    assert!(
        store
            .list_plex_collection_members()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn collections_sort_where_scryer_orders_them_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let services = SqliteServices::new(dir.path().join("scryer.db").to_string_lossy())
        .await
        .expect("sqlite services");
    collections_sort_where_scryer_orders_them(services.datastore(), dir.path()).await;
}

#[tokio::test]
async fn collections_sort_where_scryer_orders_them_postgres() {
    let Some(admin_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        eprintln!(
            "skipping PostgreSQL Plex collection order test; SCRYER_TEST_POSTGRES_URL is not set"
        );
        return;
    };
    let database = format!("scryer_collections_{}", uuid::Uuid::new_v4().simple());
    let admin = sqlx::PgPool::connect(&admin_url)
        .await
        .expect("postgres admin");
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {database}")))
        .execute(&admin)
        .await
        .expect("create test database");
    let mut url = url::Url::parse(&admin_url).expect("postgres url");
    url.set_path(&format!("/{database}"));
    let dir = tempfile::tempdir().unwrap();
    {
        let services = PostgresServices::new_with_mode(url.as_str(), MigrationMode::Apply)
            .await
            .expect("postgres services");
        collections_sort_where_scryer_orders_them(services.datastore(), dir.path()).await;
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE {database} WITH (FORCE)"
    )))
    .execute(&admin)
    .await
    .expect("drop test database");
}

async fn collections_sort_where_scryer_orders_them(
    datastore: StoreDatastore,
    data_dir: &std::path::Path,
) {
    seed(&datastore).await;
    let plex = Arc::new(FakePlex::with_items());
    let store = Arc::new(PosterOverlayStore::new(datastore.clone()));
    let overlays = AppPosterOverlayServices::new(
        store.clone(),
        Arc::new(OverlayEngine::new(data_dir, Arc::new(NoFetch)).unwrap()),
    )
    .with_plex(Arc::new(FakeConnections), plex.clone());

    // Blocks and places are stored, and two providers can trade blocks.
    store
        .replace_plex_collection_slots(&[("imdb".into(), 1), ("tmdb".into(), 2)])
        .await
        .unwrap();
    store
        .replace_plex_collection_slots(&[("imdb".into(), 2), ("tmdb".into(), 1)])
        .await
        .unwrap();
    assert_eq!(
        store.list_plex_collection_slots().await.unwrap(),
        vec![("tmdb".to_string(), 1), ("imdb".to_string(), 2)]
    );
    store.set_plex_collection_list(LIST, true).await.unwrap();
    store
        .set_plex_collection_list_position(LIST, Some(3))
        .await
        .unwrap();
    assert_eq!(
        store.list_plex_collection_lists().await.unwrap()[0].position,
        Some(3)
    );

    // The collection gets its sort title once, and not again while unchanged.
    let name = "IMDB Top 250 Movies";
    let first = "!1010_IMDB Top 250 Movies";
    let counts = overlays
        .sync_plex_collections(&[sorted_source(name, first, &[MAPPED_TITLE])])
        .await;
    assert_eq!((counts.sorted, counts.failed), (1, 0), "{counts:?}");
    assert_eq!(plex.sort_title_of(name).as_deref(), Some(first));
    let counts = overlays
        .sync_plex_collections(&[sorted_source(name, first, &[MAPPED_TITLE])])
        .await;
    assert_eq!(counts.sorted, 0);
    assert_eq!(plex.sort_calls.load(Ordering::SeqCst), 1);

    // A move in Scryer renumbers it in Plex.
    let moved = "!2010_IMDB Top 250 Movies";
    let counts = overlays
        .sync_plex_collections(&[sorted_source(name, moved, &[MAPPED_TITLE])])
        .await;
    assert_eq!(counts.sorted, 1);
    assert_eq!(plex.sort_title_of(name).as_deref(), Some(moved));

    // The list empties, so Plex drops the collection; when it fills again
    // the new collection gets the sort title too.
    overlays
        .sync_plex_collections(&[sorted_source(name, moved, &[])])
        .await;
    assert_eq!(plex.sort_title_of(name), None);
    let counts = overlays
        .sync_plex_collections(&[sorted_source(name, moved, &[MAPPED_TITLE])])
        .await;
    assert_eq!(counts.sorted, 1);
    assert_eq!(plex.sort_title_of(name).as_deref(), Some(moved));

    // Switched off: what was pushed is forgotten.
    overlays.sync_plex_collections(&[]).await;
    assert!(
        store
            .list_plex_collection_titles()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn collection_posters_upload_once_per_design_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let services = SqliteServices::new(dir.path().join("scryer.db").to_string_lossy())
        .await
        .expect("sqlite services");
    collection_posters_upload_once_per_design(services.datastore(), dir.path()).await;
}

#[tokio::test]
async fn collection_posters_upload_once_per_design_postgres() {
    let Some(admin_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        eprintln!(
            "skipping PostgreSQL collection poster test; SCRYER_TEST_POSTGRES_URL is not set"
        );
        return;
    };
    let database = format!("scryer_collections_{}", uuid::Uuid::new_v4().simple());
    let admin = sqlx::PgPool::connect(&admin_url)
        .await
        .expect("postgres admin");
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {database}")))
        .execute(&admin)
        .await
        .expect("create test database");
    let mut url = url::Url::parse(&admin_url).expect("postgres url");
    url.set_path(&format!("/{database}"));
    let dir = tempfile::tempdir().unwrap();
    {
        let services = PostgresServices::new_with_mode(url.as_str(), MigrationMode::Apply)
            .await
            .expect("postgres services");
        collection_posters_upload_once_per_design(services.datastore(), dir.path()).await;
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE {database} WITH (FORCE)"
    )))
    .execute(&admin)
    .await
    .expect("drop test database");
}

async fn collection_posters_upload_once_per_design(
    datastore: StoreDatastore,
    data_dir: &std::path::Path,
) {
    seed(&datastore).await;
    let plex = Arc::new(FakePlex::with_items());
    let store = Arc::new(PosterOverlayStore::new(datastore.clone()));
    let overlays = AppPosterOverlayServices::new(
        store.clone(),
        Arc::new(OverlayEngine::new(data_dir, Arc::new(NoFetch)).unwrap()),
    )
    .with_plex(Arc::new(FakeConnections), plex.clone());

    // Poster settings are stored on the list.
    store.set_plex_collection_list(LIST, true).await.unwrap();
    store
        .set_plex_collection_poster(LIST, true, Some("#7A6200"), Some("TOP 250"), true, None)
        .await
        .unwrap();
    let list = store.list_plex_collection_lists().await.unwrap().remove(0);
    assert!(list.poster_enabled && list.poster_logo);
    assert_eq!(list.poster_logo_source, None);
    assert_eq!(list.poster_background.as_deref(), Some("#7A6200"));
    assert_eq!(list.poster_text.as_deref(), Some("TOP 250"));

    let name = "IMDB Top 250 Movies";
    let sort = "!1010_IMDB Top 250 Movies";
    let design = |text: &str| PlexCollectionSource {
        poster: Some(CollectionPosterRequest {
            background: [0x7a, 0x62, 0x00],
            logo: Some("imdb".into()),
            custom_logo: None,
            text: text.into(),
        }),
        ..sorted_source(name, sort, &[MAPPED_TITLE])
    };

    // Drawn, uploaded and locked once; not again while the design holds.
    let counts = overlays.sync_plex_collections(&[design("TOP 250")]).await;
    assert_eq!((counts.posters, counts.failed), (1, 0), "{counts:?}");
    assert_eq!(plex.poster_of(name), (1, Some(true)));
    overlays.sync_plex_collections(&[design("TOP 250")]).await;
    assert_eq!(plex.poster_of(name).0, 1);

    // A new label is a new design.
    let counts = overlays
        .sync_plex_collections(&[design("IMDB TOP 250")])
        .await;
    assert_eq!(counts.posters, 1);
    assert_eq!(plex.poster_of(name).0, 2);

    // A custom logo is stored on the list and is part of the design: a new
    // image is uploaded once, the same image again is not.
    store
        .set_plex_collection_poster(LIST, true, None, None, true, Some("custom"))
        .await
        .unwrap();
    let list = store.list_plex_collection_lists().await.unwrap().remove(0);
    assert_eq!(list.poster_logo_source.as_deref(), Some("custom"));
    let custom = |logo: &[u8]| PlexCollectionSource {
        poster: Some(CollectionPosterRequest {
            background: [0x7a, 0x62, 0x00],
            logo: None,
            custom_logo: Some(logo.to_vec()),
            text: "IMDB TOP 250".into(),
        }),
        ..sorted_source(name, sort, &[MAPPED_TITLE])
    };
    let counts = overlays.sync_plex_collections(&[custom(&png(4))]).await;
    assert_eq!((counts.posters, counts.failed), (1, 0), "{counts:?}");
    assert_eq!(plex.poster_of(name).0, 3);
    overlays.sync_plex_collections(&[custom(&png(4))]).await;
    assert_eq!(plex.poster_of(name).0, 3);
    overlays.sync_plex_collections(&[custom(&png(8))]).await;
    assert_eq!(plex.poster_of(name).0, 4);

    // Switched off: unlocked, nothing uploaded, and it stays that way.
    let counts = overlays
        .sync_plex_collections(&[sorted_source(name, sort, &[MAPPED_TITLE])])
        .await;
    assert_eq!(counts.posters, 1);
    assert_eq!(plex.poster_of(name), (4, Some(false)));
    let counts = overlays
        .sync_plex_collections(&[sorted_source(name, sort, &[MAPPED_TITLE])])
        .await;
    assert_eq!(counts.posters, 0);
}

/// A small opaque PNG, `width` pixels wide.
fn png(width: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(width, 4, image::Rgba([255, 0, 0, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("encode logo fixture");
    out.into_inner()
}
