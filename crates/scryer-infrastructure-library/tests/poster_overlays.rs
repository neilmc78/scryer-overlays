//! Poster overlays end to end against a real SQLite datastore and the real
//! renderer: render, skip when unchanged, rebuild on an upgrade, adopt new
//! upstream artwork, refuse an overlaid "original", and revert.

#![cfg(feature = "image-processing")]

use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use image::{ImageFormat, Rgb, RgbImage};
use scryer_application::overlays::{
    AppPosterOverlayServices, PosterOverlayOutcome, PosterOverlayRepository,
    overlay_title_for_proxy_source,
};
use scryer_application::{AppResult, ImageProxyKind, ImageProxyRegistration, ImageProxyRepository};
use scryer_infrastructure_datastore::postgres::PostgresServices;
use scryer_infrastructure_datastore::{MigrationMode, SqliteServices};
use scryer_infrastructure_library::images::ImageProxyStore;
use scryer_infrastructure_library::overlays::{
    OverlayEngine, OverlaySourceFetch, PosterOverlayStore, add_marker, has_marker,
};
use scryer_infrastructure_sql::runtime::{SqlArg, SqlRuntime, StoreDatastore};

const TITLE: &str = "title-1";
const LIBRARY: &str = "library-1";
const ORIGINAL_POSTER_PATH: &str = "/images/titles/title-1/poster/w250?v=0123456789abcdef";

/// Serves whatever poster the test last set, counting downloads.
#[derive(Default)]
struct ScriptedFetch {
    poster: Mutex<Vec<u8>>,
    fetches: AtomicUsize,
}

impl ScriptedFetch {
    fn serve(&self, bytes: Vec<u8>) {
        *self.poster.lock().unwrap() = bytes;
    }

    fn fetches(&self) -> usize {
        self.fetches.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl OverlaySourceFetch for ScriptedFetch {
    async fn fetch(&self, _source_url: &str) -> AppResult<Vec<u8>> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        Ok(self.poster.lock().unwrap().clone())
    }
}

fn poster(shade: u8) -> Vec<u8> {
    let image = RgbImage::from_fn(400, 600, |x, y| {
        Rgb([shade, (x % 256) as u8, (y % 256) as u8])
    });
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, ImageFormat::Jpeg).unwrap();
    out.into_inner()
}

async fn exec(datastore: &StoreDatastore, sql: &str, args: Vec<SqlArg>) {
    SqlRuntime::execute_write(datastore, "overlay_fixture", sql, args)
        .await
        .unwrap_or_else(|error| panic!("fixture statement failed: {error}\n{sql}"));
}

async fn text(datastore: &StoreDatastore, sql: &str) -> Option<String> {
    SqlRuntime::fetch_optional(datastore.read_exec(), sql, &[])
        .await
        .unwrap()
        .and_then(|row| row.opt_text("value").unwrap())
}

async fn seed(datastore: &StoreDatastore) {
    let now = Utc::now();
    exec(
        datastore,
        "INSERT INTO libraries (id, facet, name, slug, created_at, updated_at)
         VALUES ({}, 'movie', 'Overlay Fixture', 'overlay-fixture', {}, {})",
        vec![
            SqlArg::Text(LIBRARY.into()),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
        ],
    )
    .await;
    exec(
        datastore,
        "INSERT INTO library_roots (id, library_id, path, normalized_path, created_at, updated_at)
         VALUES ('root-1', {}, '/overlay-fixture', '/overlay-fixture', {}, {})",
        vec![
            SqlArg::Text(LIBRARY.into()),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
        ],
    )
    .await;
    exec(
        datastore,
        "INSERT INTO titles (id, name, facet, created_at, library_id, root_folder_id, poster_local_path)
         VALUES ({}, 'Fixture', 'movie', {}, {}, 'root-1', {})",
        vec![
            SqlArg::Text(TITLE.into()),
            SqlArg::Timestamp(now),
            SqlArg::Text(LIBRARY.into()),
            SqlArg::Text(ORIGINAL_POSTER_PATH.into()),
        ],
    )
    .await;
    exec(
        datastore,
        "INSERT INTO title_images (id, title_id, provider, kind, source_url, source_etag,
                                   source_format, source_width, source_height, created_at, updated_at)
         VALUES ('image-1', {}, 'tmdb', 'poster', 'https://image.tmdb.org/t/p/w500/a.jpg', 'etag-1',
                 'jpeg', 500, 750, {}, {})",
        vec![SqlArg::Text(TITLE.into()), SqlArg::Timestamp(now), SqlArg::Timestamp(now)],
    )
    .await;
    exec(
        datastore,
        "INSERT INTO title_image_blobs (digest, format, width, height, bytes, created_at, updated_at)
         VALUES ('blake3:0123456789abcdef0000', 'avif', 250, 375, {}, {}, {})",
        vec![SqlArg::OptBytes(Some(vec![0])), SqlArg::Timestamp(now), SqlArg::Timestamp(now)],
    )
    .await;
    exec(
        datastore,
        "INSERT INTO title_image_variants (id, title_image_id, variant_key, blob_digest, created_at, updated_at)
         VALUES ('variant-1', 'image-1', 'w250', 'blake3:0123456789abcdef0000', {}, {})",
        vec![SqlArg::Timestamp(now), SqlArg::Timestamp(now)],
    )
    .await;
    exec(
        datastore,
        "INSERT INTO media_files (id, title_id, file_path, size_bytes, created_at,
                                  video_width, video_height, audio_codec, audio_channels)
         VALUES ('file-1', {}, '/movies/Fixture/Fixture.mkv', 1, {}, 1920, 1080, 'ac3', 6)",
        vec![SqlArg::Text(TITLE.into()), SqlArg::Timestamp(now)],
    )
    .await;
}

fn rendered_hash(outcome: PosterOverlayOutcome) -> String {
    match outcome {
        PosterOverlayOutcome::Rendered { output_hash } => output_hash,
        other => panic!("expected a render, got {other:?}"),
    }
}

#[tokio::test]
async fn render_then_rebuild_only_when_inputs_change_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let services = SqliteServices::new(dir.path().join("scryer.db").to_string_lossy())
        .await
        .expect("sqlite services");
    render_then_rebuild_only_when_inputs_change(services.datastore(), dir.path()).await;
}

/// Same scenario on PostgreSQL. Opt-in like the repository's other engine
/// parity tests: `SCRYER_TEST_POSTGRES_URL` must name a server where the
/// user may create databases; the test migrates a fresh one and drops it.
#[tokio::test]
async fn render_then_rebuild_only_when_inputs_change_postgres() {
    let Some(admin_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        eprintln!("skipping PostgreSQL poster overlay test; SCRYER_TEST_POSTGRES_URL is not set");
        return;
    };
    let database = format!("scryer_overlay_{}", uuid::Uuid::new_v4().simple());
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
        render_then_rebuild_only_when_inputs_change(services.datastore(), dir.path()).await;
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE {database} WITH (FORCE)"
    )))
    .execute(&admin)
    .await
    .expect("drop test database");
}

async fn render_then_rebuild_only_when_inputs_change(
    datastore: StoreDatastore,
    data_dir: &std::path::Path,
) {
    seed(&datastore).await;

    let fetch = Arc::new(ScriptedFetch::default());
    fetch.serve(poster(10));
    let store = Arc::new(PosterOverlayStore::new(datastore.clone()));
    let overlays = AppPosterOverlayServices::new(
        store.clone(),
        Arc::new(OverlayEngine::new(data_dir, fetch.clone()).unwrap()),
    );
    let poster_path = || text(&datastore, "SELECT poster_local_path AS value FROM titles");

    // Library not enabled: nothing happens and the stored poster is served.
    assert_eq!(
        overlays.process_title(TITLE).await,
        PosterOverlayOutcome::Skipped
    );
    assert!(overlays.image(TITLE, "w250").await.unwrap().is_none());
    assert_eq!(fetch.fetches(), 0);

    // First render.
    store.set_library_config(LIBRARY, true, None).await.unwrap();
    let first = rendered_hash(overlays.process_title(TITLE).await);
    let served = overlays
        .image(TITLE, "w250")
        .await
        .unwrap()
        .expect("overlay served");
    assert!(has_marker(&served.bytes));
    assert_eq!(served.etag, format!("overlay:{first}"));
    assert_eq!(
        poster_path().await.as_deref(),
        Some(format!("/images/titles/{TITLE}/poster/w250?v={}", &first[..16]).as_str())
    );
    assert_eq!(fetch.fetches(), 1);
    let state = store.get_state(TITLE).await.unwrap().unwrap();
    let first_original = state.original_hash.clone().unwrap();
    assert!(
        state
            .fields_json
            .unwrap()
            .contains("\"resolution\":\"1080p\"")
    );

    // Same inputs: no render, no download.
    assert_eq!(
        overlays.process_title(TITLE).await,
        PosterOverlayOutcome::Unchanged
    );
    assert_eq!(fetch.fetches(), 1);

    // Upgrade to a 4K Dolby Vision file: rebuilt from the stored original.
    exec(
        &datastore,
        "UPDATE media_files SET video_width = 3840, video_height = 2160,
                video_hdr_format = 'Dolby Vision' WHERE id = 'file-1'",
        vec![],
    )
    .await;
    let upgraded = rendered_hash(overlays.process_title(TITLE).await);
    assert_ne!(upgraded, first);
    assert_eq!(fetch.fetches(), 1, "the original is reused, not refetched");

    // New upstream artwork: the reconcile pass adopts it as the new original.
    fetch.serve(poster(200));
    exec(
        &datastore,
        "UPDATE title_images SET source_etag = 'etag-2'",
        vec![],
    )
    .await;
    let summary = overlays.reconcile().await.unwrap();
    assert_eq!(summary.rendered, 1);
    assert_eq!(fetch.fetches(), 2);
    let state = store.get_state(TITLE).await.unwrap().unwrap();
    assert_ne!(
        state.original_hash.as_deref(),
        Some(first_original.as_str())
    );
    let current = state.output_hash.clone().unwrap();

    // An upstream poster carrying our marker is never taken as an original;
    // the last good overlay keeps being served.
    fetch.serve(add_marker(poster(90), "someone-elses-render").unwrap());
    exec(
        &datastore,
        "UPDATE title_images SET source_etag = 'etag-3'",
        vec![],
    )
    .await;
    assert!(matches!(
        overlays.process_title(TITLE).await,
        PosterOverlayOutcome::Failed(_)
    ));
    let served = overlays
        .image(TITLE, "original")
        .await
        .unwrap()
        .expect("still served");
    assert_eq!(served.etag, format!("overlay:{current}"));
    assert!(
        store
            .get_state(TITLE)
            .await
            .unwrap()
            .unwrap()
            .last_error
            .is_some()
    );

    // Revert: overlays off, stored poster URL restored, outputs gone,
    // original kept.
    assert_eq!(overlays.revert_all().await.unwrap(), 1);
    assert!(overlays.image(TITLE, "w250").await.unwrap().is_none());
    assert_eq!(poster_path().await.as_deref(), Some(ORIGINAL_POSTER_PATH));
    assert!(store.get_state(TITLE).await.unwrap().is_none());
    assert!(!data_dir.join("overlays/output").join(TITLE).exists());
    assert!(data_dir.join("overlays/originals").join(TITLE).exists());
    assert!(
        store
            .list_library_configs()
            .await
            .unwrap()
            .iter()
            .all(|library| !library.enabled)
    );
}

#[tokio::test]
async fn renders_from_the_title_poster_url_when_the_image_cache_is_empty_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let services = SqliteServices::new(dir.path().join("scryer.db").to_string_lossy())
        .await
        .expect("sqlite services");
    renders_from_the_title_poster_url_when_the_image_cache_is_empty(
        services.datastore(),
        dir.path(),
    )
    .await;
}

#[tokio::test]
async fn renders_from_the_title_poster_url_when_the_image_cache_is_empty_postgres() {
    let Some(admin_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        eprintln!("skipping PostgreSQL poster overlay test; SCRYER_TEST_POSTGRES_URL is not set");
        return;
    };
    let database = format!("scryer_overlay_{}", uuid::Uuid::new_v4().simple());
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
        renders_from_the_title_poster_url_when_the_image_cache_is_empty(
            services.datastore(),
            dir.path(),
        )
        .await;
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE {database} WITH (FORCE)"
    )))
    .execute(&admin)
    .await
    .expect("drop test database");
}

/// After a backup restore the image cache (`title_images`) is empty until
/// metadata refreshes it; the title row still names its upstream poster.
async fn renders_from_the_title_poster_url_when_the_image_cache_is_empty(
    datastore: StoreDatastore,
    data_dir: &std::path::Path,
) {
    seed(&datastore).await;
    exec(&datastore, "DELETE FROM title_image_variants", vec![]).await;
    exec(&datastore, "DELETE FROM title_images", vec![]).await;
    exec(
        &datastore,
        "UPDATE titles SET poster_url = 'https://image.tmdb.org/t/p/w500/b.jpg', poster_local_path = NULL",
        vec![],
    )
    .await;

    let fetch = Arc::new(ScriptedFetch::default());
    fetch.serve(poster(40));
    let store = Arc::new(PosterOverlayStore::new(datastore.clone()));
    let overlays = AppPosterOverlayServices::new(
        store.clone(),
        Arc::new(OverlayEngine::new(data_dir, fetch.clone()).unwrap()),
    );
    store.set_library_config(LIBRARY, true, None).await.unwrap();

    // A second cut beside the primary file: its edition comes only from the
    // `{edition-...}` tag in its name.
    exec(
        &datastore,
        "INSERT INTO media_files (id, title_id, file_path, size_bytes, created_at,
                                  video_width, video_height, audio_codec, role)
         VALUES ('file-2', {}, '/overlay-fixture/Fixture {edition-Theatrical}.mkv', 10, {},
                 3840, 2160, 'aac', 'additional')",
        vec![SqlArg::Text(TITLE.into()), SqlArg::Timestamp(Utc::now())],
    )
    .await;

    let rendered = rendered_hash(overlays.process_title(TITLE).await);
    assert_eq!(fetch.fetches(), 1);
    let fields = store
        .get_state(TITLE)
        .await
        .unwrap()
        .unwrap()
        .fields_json
        .unwrap();
    assert!(fields.contains("\"edition\":\"theatrical\""), "{fields}");
    assert!(
        fields.contains("\"resolution\":\"1080p\""),
        "the additional 4K cut must not raise the primary's resolution: {fields}"
    );
    assert!(
        overlays
            .image(TITLE, "w250")
            .await
            .unwrap()
            .is_some_and(|image| has_marker(&image.bytes))
    );
    let counts = store.status_counts().await.unwrap();
    assert_eq!((counts.rendered, counts.no_artwork), (1, 0));

    // Template previews draw on stored originals from the chosen kind of
    // library, keeping a pinned title while it is usable.
    let pick = |facet: Option<&'static str>, title_id: Option<&'static str>| {
        let store = store.clone();
        async move {
            store
                .preview_posters(facet, title_id, 8)
                .await
                .unwrap()
                .into_iter()
                .map(|poster| (poster.title_id, poster.name))
                .collect::<Vec<_>>()
        }
    };
    let fixture = vec![(TITLE.to_string(), "Fixture".to_string())];
    assert_eq!(pick(Some("movie"), None).await, fixture);
    assert_eq!(pick(None, None).await, fixture);
    assert!(pick(Some("series"), None).await.is_empty());
    assert_eq!(
        pick(Some("series"), Some(TITLE)).await,
        fixture,
        "a pinned title is kept"
    );
    assert_eq!(
        pick(Some("movie"), Some("no-such-title")).await,
        fixture,
        "an unusable pin falls back to a random poster"
    );

    // The catalog reaches posters through the image proxy: register the
    // title's poster exactly as the GraphQL mapper does, then follow its
    // token back to the overlay the media route serves.
    let proxy = ImageProxyStore::new(datastore.clone());
    let register = |owner_type: &str, kind: ImageProxyKind| {
        proxy.register_image_source(ImageProxyRegistration {
            upstream_url: Some("https://image.tmdb.org/t/p/w500/b.jpg".into()),
            owner_type: Some(owner_type.into()),
            owner_id: Some(TITLE.into()),
            image_kind: kind,
            fallback_class: "portrait".into(),
            default_variant: "w250".into(),
        })
    };
    let token_of = |url: String| url.split('/').rev().nth(1).unwrap().to_string();
    let poster_token = token_of(register("title", ImageProxyKind::Poster));
    let fanart_token = token_of(register("title", ImageProxyKind::Fanart));
    let movie_token = token_of(register("movie", ImageProxyKind::Poster));
    proxy.flush_image_proxy_sources().await.unwrap();
    proxy.clear_image_proxy_memory();
    let source = proxy
        .get_image_proxy_source(&poster_token)
        .await
        .unwrap()
        .expect("registered source persists");
    let title_id = overlay_title_for_proxy_source(&source).expect("title poster token");
    assert_eq!(title_id, TITLE);
    assert!(
        overlays
            .image(title_id, "w250")
            .await
            .unwrap()
            .is_some_and(|image| has_marker(&image.bytes))
    );
    for other in [fanart_token, movie_token] {
        let source = proxy.get_image_proxy_source(&other).await.unwrap().unwrap();
        assert_eq!(overlay_title_for_proxy_source(&source), None);
    }

    // No artwork anywhere: reported as such, not silently skipped, and the
    // last good overlay keeps being served.
    exec(&datastore, "UPDATE titles SET poster_url = NULL", vec![]).await;
    assert_eq!(
        overlays.process_title(TITLE).await,
        PosterOverlayOutcome::NoArtwork
    );
    assert_eq!(store.status_counts().await.unwrap().no_artwork, 1);
    assert!(overlays.image(TITLE, "w250").await.unwrap().is_some());

    // A local path is not a download source.
    exec(
        &datastore,
        "UPDATE titles SET poster_url = '/images/titles/title-1/poster/w250'",
        vec![],
    )
    .await;
    assert_eq!(
        overlays.process_title(TITLE).await,
        PosterOverlayOutcome::NoArtwork
    );

    // Revert with no cached image to point back at: the overlay path is
    // cleared so the catalog falls back to the upstream poster.
    assert_eq!(overlays.revert_all().await.unwrap(), 1);
    let local_path = text(&datastore, "SELECT poster_local_path AS value FROM titles").await;
    assert_eq!(
        local_path, None,
        "overlay path {rendered} must not survive revert"
    );
}
