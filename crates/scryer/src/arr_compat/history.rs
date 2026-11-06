//! `GET /api/v3/history` for the Sonarr/Radarr compatibility API.
//!
//! Tools that follow an *arr's library through its history, such as Silo's
//! Sonarr/Radarr autoscan, read this page by page, newest first, and act on
//! three record kinds:
//!
//! | arr `eventType` | `data` | Scryer events |
//! |---|---|---|
//! | `downloadFolderImported` | `importedPath` | a file imported, upgraded or restored |
//! | `movieFileRenamed` / `episodeFileRenamed` | `path`, `sourcePath` | a file renamed, a title moved |
//! | `movieFileDeleted` / `episodeFileDeleted` | `path`, `reason` | a file deleted, or replaced by an upgrade |
//!
//! Records come from the domain event log, which already carries every path a
//! change touched (`media_updates`). One event can yield several records, one
//! per file, so a record's `id` is the event's sequence times
//! [`RECORDS_PER_EVENT`] plus its index: unique, stable across reads, and in
//! the same order as the log. Each flavor reads only its own facets (movies
//! for Radarr, series and anime for Sonarr), and a library-scoped endpoint
//! only that library's titles.
//!
//! Only `sortKey=date`, newest first, is served, which is how *arr clients
//! page history; other sort and filter parameters are ignored. `totalRecords`
//! is exact once the log is exhausted, and otherwise counts one record past
//! the page so a client knows to read on.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use scryer_domain::{
    DomainEvent, DomainEventFilter, DomainEventPayload, DomainEventType, MediaFileDeletedReason,
    MediaPathUpdate, MediaUpdateType, TitleContextSnapshot,
};
use serde::{Deserialize, Serialize};

use super::{CompatState, Flavor, LibraryPath, actor, library_scope};
use crate::middleware::map_app_error;

/// Records one event may yield; an event touching more files is cut off here.
pub(super) const RECORDS_PER_EVENT: i64 = 10_000;
/// Records per page when the client does not say.
const DEFAULT_PAGE_SIZE: usize = 10;
const MAX_PAGE_SIZE: usize = 1_000;
/// Events read from the log per request at most, so a deep page cannot make
/// one request walk the whole log.
const MAX_EVENTS_READ: usize = 20_000;
/// Events per log read.
const EVENT_BATCH: usize = 500;

/// The event types history is built from.
const HISTORY_EVENT_TYPES: [DomainEventType; 6] = [
    DomainEventType::MediaFileImported,
    DomainEventType::MediaFileUpgraded,
    DomainEventType::MediaFileRestored,
    DomainEventType::MediaFileRenamed,
    DomainEventType::MediaFileDeleted,
    DomainEventType::TitleMoved,
];

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HistoryQuery {
    page: Option<usize>,
    page_size: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HistoryPage {
    page: usize,
    page_size: usize,
    sort_key: &'static str,
    sort_direction: &'static str,
    total_records: usize,
    records: Vec<HistoryRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HistoryRecord {
    pub id: i64,
    pub event_type: &'static str,
    /// RFC 3339, UTC.
    pub date: String,
    pub source_title: String,
    pub data: HistoryData,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HistoryData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imported_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub(super) async fn history(
    State(state): State<CompatState>,
    path: LibraryPath,
    Query(query): Query<HistoryQuery>,
    headers: HeaderMap,
) -> Response {
    let actor = match actor(&state, &headers).await {
        Ok(actor) => actor,
        Err(error) => return *error,
    };
    let library_id =
        match library_scope(&state, &actor, path, scryer_domain::LibraryPermission::View).await {
            Ok(library_id) => library_id,
            Err(error) => return *error,
        };
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query
        .page_size
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    let wanted = page.saturating_mul(page_size);

    let mut records = Vec::new();
    let mut title_libraries: HashMap<String, Option<String>> = HashMap::new();
    let mut before_sequence = None;
    let mut read = 0;
    let mut exhausted = false;
    // One record past the page tells the client there is more to read.
    while records.len() <= wanted {
        if read >= MAX_EVENTS_READ {
            break;
        }
        let events = match state
            .auth
            .app
            .list_domain_events(
                &actor,
                &DomainEventFilter {
                    event_types: Some(HISTORY_EVENT_TYPES.to_vec()),
                    before_sequence,
                    limit: EVENT_BATCH,
                    ..DomainEventFilter::default()
                },
            )
            .await
        {
            Ok(events) => events,
            Err(error) => return map_app_error(error),
        };
        let Some(last) = events.last() else {
            exhausted = true;
            break;
        };
        before_sequence = Some(last.sequence);
        read += events.len();
        let short = events.len() < EVENT_BATCH;
        for event in &events {
            if !event
                .facet
                .as_ref()
                .is_some_and(|facet| state.flavor.facets().contains(facet))
            {
                continue;
            }
            if let Some(library_id) = library_id.as_deref() {
                let Some(title_id) = event.title_id.as_deref() else {
                    continue;
                };
                let title_library = match title_libraries.get(title_id) {
                    Some(library) => library.clone(),
                    None => {
                        let library = state
                            .auth
                            .app
                            .get_title(&actor, title_id)
                            .await
                            .ok()
                            .flatten()
                            .map(|title| title.library_id);
                        title_libraries.insert(title_id.to_string(), library.clone());
                        library
                    }
                };
                if title_library.as_deref() != Some(library_id) {
                    continue;
                }
            }
            records.extend(history_records(event, state.flavor));
        }
        if short {
            exhausted = true;
            break;
        }
    }

    let total_records = if exhausted {
        records.len()
    } else {
        records.len().max(wanted + 1)
    };
    let start = (page - 1).saturating_mul(page_size).min(records.len());
    let end = wanted.min(records.len());
    Json(HistoryPage {
        page,
        page_size,
        sort_key: "date",
        sort_direction: "descending",
        total_records,
        records: records[start..end].to_vec(),
    })
    .into_response()
}

/// The *arr history records one event yields, in the order a newest-first
/// reader should see them.
pub(super) fn history_records(event: &DomainEvent, flavor: Flavor) -> Vec<HistoryRecord> {
    let renamed = match flavor {
        Flavor::Radarr => "movieFileRenamed",
        Flavor::Sonarr => "episodeFileRenamed",
    };
    let deleted = match flavor {
        Flavor::Radarr => "movieFileDeleted",
        Flavor::Sonarr => "episodeFileDeleted",
    };
    let mut data = Vec::new();
    let source_title;
    match &event.payload {
        DomainEventPayload::MediaFileImported(payload) => {
            source_title = source_title_of(&payload.title);
            imports(&payload.media_updates, &mut data);
        }
        DomainEventPayload::MediaFileUpgraded(payload) => {
            source_title = source_title_of(&payload.title);
            imports(&payload.media_updates, &mut data);
        }
        DomainEventPayload::MediaFileRestored(payload) => {
            source_title = source_title_of(&payload.title);
            imports(&payload.media_updates, &mut data);
        }
        DomainEventPayload::MediaFileRenamed(payload) => {
            source_title = source_title_of(&payload.title);
            renames(&payload.media_updates, renamed, &mut data);
        }
        DomainEventPayload::MediaFileDeleted(payload) => {
            source_title = source_title_of(&payload.title);
            // Purging the recycle bin removes a copy the library already lost;
            // only a file leaving its library folder is history.
            let reason = match payload.reason {
                MediaFileDeletedReason::Deleted => "Manual",
                MediaFileDeletedReason::UpgradeCleanup => "Upgrade",
                MediaFileDeletedReason::MissingOnDisk => "MissingFromDisk",
                MediaFileDeletedReason::RecycleBinPurged
                | MediaFileDeletedReason::RecycleBinPurgedAfterUpgrade => return Vec::new(),
            };
            for update in &payload.media_updates {
                if let Some(path) = non_empty(&update.path) {
                    data.push((
                        deleted,
                        HistoryData {
                            path: Some(path),
                            reason: Some(reason.to_string()),
                            ..HistoryData::default()
                        },
                    ));
                }
            }
        }
        DomainEventPayload::TitleMoved(payload) => {
            source_title = source_title_of(&payload.title);
            let path = payload.destination_path.as_deref().and_then(non_empty);
            let source_path = payload.source_path.as_deref().and_then(non_empty);
            if path.is_some() || source_path.is_some() {
                data.push((
                    renamed,
                    HistoryData {
                        path,
                        source_path,
                        ..HistoryData::default()
                    },
                ));
            }
        }
        _ => return Vec::new(),
    }
    let date = event
        .occurred_at
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let base = event.sequence.saturating_mul(RECORDS_PER_EVENT);
    data.into_iter()
        .take(RECORDS_PER_EVENT as usize)
        .enumerate()
        // Newest first: within one event, the last file listed comes first.
        .rev()
        .map(|(index, (event_type, data))| HistoryRecord {
            id: base + index as i64,
            event_type,
            date: date.clone(),
            source_title: source_title.clone(),
            data,
        })
        .collect()
}

/// An import record for each path a file now occupies.
fn imports(updates: &[MediaPathUpdate], data: &mut Vec<(&'static str, HistoryData)>) {
    for update in updates {
        if update.update_type == MediaUpdateType::Deleted {
            continue;
        }
        if let Some(path) = non_empty(&update.path) {
            data.push((
                "downloadFolderImported",
                HistoryData {
                    imported_path: Some(path),
                    ..HistoryData::default()
                },
            ));
        }
    }
}

/// Rename records from a rename's paths: the old paths (deleted) paired in
/// order with the new ones (created). An unpaired path still gets a record
/// with the side it has.
fn renames(
    updates: &[MediaPathUpdate],
    event_type: &'static str,
    data: &mut Vec<(&'static str, HistoryData)>,
) {
    let mut old = updates
        .iter()
        .filter(|update| update.update_type == MediaUpdateType::Deleted)
        .filter_map(|update| non_empty(&update.path));
    let mut new = updates
        .iter()
        .filter(|update| update.update_type != MediaUpdateType::Deleted)
        .filter_map(|update| non_empty(&update.path));
    loop {
        let (source_path, path) = (old.next(), new.next());
        if source_path.is_none() && path.is_none() {
            break;
        }
        data.push((
            event_type,
            HistoryData {
                path,
                source_path,
                ..HistoryData::default()
            },
        ));
    }
}

fn non_empty(path: &str) -> Option<String> {
    let path = path.trim();
    (!path.is_empty()).then(|| path.to_string())
}

fn source_title_of(title: &TitleContextSnapshot) -> String {
    match title.year {
        Some(year) => format!("{} ({year})", title.title_name),
        None => title.title_name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use scryer_domain::{
        DomainEventActorKind, DomainEventStream, DomainExternalIds, MediaFacet,
        MediaFileDeletedEventData, MediaFileImportedEventData, MediaFileRenamedEventData,
    };

    fn title() -> TitleContextSnapshot {
        TitleContextSnapshot {
            title_name: "Harbor Lights".into(),
            facet: MediaFacet::Movie,
            external_ids: DomainExternalIds::default(),
            poster_url: None,
            year: Some(2021),
        }
    }

    fn update(path: &str, update_type: MediaUpdateType) -> MediaPathUpdate {
        MediaPathUpdate {
            path: path.into(),
            update_type,
        }
    }

    fn event(sequence: i64, payload: DomainEventPayload) -> DomainEvent {
        DomainEvent {
            sequence,
            event_id: format!("e{sequence}"),
            occurred_at: Utc.with_ymd_and_hms(2026, 11, 6, 12, 0, 0).unwrap(),
            actor_kind: DomainEventActorKind::System,
            actor_user_id: None,
            actor_display_name: "System".into(),
            title_id: Some("t1".into()),
            facet: Some(MediaFacet::Movie),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: "t1".into(),
            },
            payload,
        }
    }

    #[test]
    fn an_import_lists_the_new_files_and_not_the_ones_they_replaced() {
        let records = history_records(
            &event(
                7,
                DomainEventPayload::MediaFileImported(MediaFileImportedEventData {
                    title: title(),
                    media_updates: vec![
                        update(
                            "/movies/Harbor Lights (2021)/old.mkv",
                            MediaUpdateType::Deleted,
                        ),
                        update(
                            "/movies/Harbor Lights (2021)/new.mkv",
                            MediaUpdateType::Created,
                        ),
                    ],
                }),
            ),
            Flavor::Radarr,
        );
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.event_type, "downloadFolderImported");
        assert_eq!(record.id, 7 * RECORDS_PER_EVENT);
        assert_eq!(record.date, "2026-11-06T12:00:00Z");
        assert_eq!(record.source_title, "Harbor Lights (2021)");
        assert_eq!(
            record.data.imported_path.as_deref(),
            Some("/movies/Harbor Lights (2021)/new.mkv")
        );
    }

    #[test]
    fn renames_pair_old_and_new_paths_newest_first_with_distinct_ids() {
        let records = history_records(
            &event(
                3,
                DomainEventPayload::MediaFileRenamed(MediaFileRenamedEventData {
                    title: title(),
                    media_updates: vec![
                        update("/tv/Show/a-old.mkv", MediaUpdateType::Deleted),
                        update("/tv/Show/a.mkv", MediaUpdateType::Created),
                        update("/tv/Show/b-old.mkv", MediaUpdateType::Deleted),
                        update("/tv/Show/b.mkv", MediaUpdateType::Created),
                    ],
                    renamed_count: 2,
                    episode_ids: Vec::new(),
                }),
            ),
            Flavor::Sonarr,
        );
        let pairs = records
            .iter()
            .map(|record| {
                (
                    record.id,
                    record.event_type,
                    record.data.path.as_deref(),
                    record.data.source_path.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            pairs,
            [
                (
                    3 * RECORDS_PER_EVENT + 1,
                    "episodeFileRenamed",
                    Some("/tv/Show/b.mkv"),
                    Some("/tv/Show/b-old.mkv")
                ),
                (
                    3 * RECORDS_PER_EVENT,
                    "episodeFileRenamed",
                    Some("/tv/Show/a.mkv"),
                    Some("/tv/Show/a-old.mkv")
                ),
            ]
        );
    }

    #[test]
    fn deletes_carry_a_reason_and_recycle_bin_purges_are_not_history() {
        let deleted = |reason| {
            history_records(
                &event(
                    9,
                    DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
                        title: title(),
                        media_updates: vec![update("/movies/x.mkv", MediaUpdateType::Deleted)],
                        file_id: None,
                        reason,
                        episode_ids: Vec::new(),
                    }),
                ),
                Flavor::Radarr,
            )
        };
        let upgrade = deleted(MediaFileDeletedReason::UpgradeCleanup);
        assert_eq!(upgrade[0].event_type, "movieFileDeleted");
        assert_eq!(upgrade[0].data.reason.as_deref(), Some("Upgrade"));
        assert_eq!(upgrade[0].data.path.as_deref(), Some("/movies/x.mkv"));
        assert!(deleted(MediaFileDeletedReason::RecycleBinPurged).is_empty());
    }

    #[test]
    fn the_wire_shape_matches_arr_history() {
        let record = HistoryRecord {
            id: 1,
            event_type: "downloadFolderImported",
            date: "2026-11-06T12:00:00Z".into(),
            source_title: "Harbor Lights (2021)".into(),
            data: HistoryData {
                imported_path: Some("/movies/new.mkv".into()),
                ..HistoryData::default()
            },
        };
        assert_eq!(
            serde_json::to_value(&record).unwrap(),
            serde_json::json!({
                "id": 1,
                "eventType": "downloadFolderImported",
                "date": "2026-11-06T12:00:00Z",
                "sourceTitle": "Harbor Lights (2021)",
                "data": { "importedPath": "/movies/new.mkv" }
            })
        );
    }

    mod endpoint {
        use super::*;
        use crate::arr_compat::router;
        use crate::middleware::{
            AuthState, AuthlessWebClientProofState, WebSocketOriginPolicy,
            integration_test_common::TestContext,
        };
        use crate::rate_limit::ScryerRateLimiter;
        use axum::{
            body::{Body, to_bytes},
            http::{Request, StatusCode},
        };
        use scryer_domain::{NewDomainEvent, NewTitle, Title, User};
        use serde_json::Value;
        use tower::ServiceExt;

        fn import(actor: &User, title: &Title, path: &str) -> NewDomainEvent {
            NewDomainEvent {
                event_id: scryer_domain::Id::new().0,
                occurred_at: Utc::now(),
                actor_kind: DomainEventActorKind::User,
                actor_user_id: Some(actor.id.clone()),
                actor_display_name: actor.username.clone(),
                title_id: Some(title.id.clone()),
                facet: Some(title.facet.clone()),
                correlation_id: None,
                causation_id: None,
                schema_version: 1,
                stream: DomainEventStream::Title {
                    title_id: title.id.clone(),
                },
                payload: DomainEventPayload::MediaFileImported(MediaFileImportedEventData {
                    title: TitleContextSnapshot {
                        title_name: title.name.clone(),
                        facet: title.facet.clone(),
                        external_ids: DomainExternalIds::default(),
                        poster_url: None,
                        year: None,
                    },
                    media_updates: vec![update(path, MediaUpdateType::Created)],
                }),
            }
        }

        async fn read(router: &axum::Router, uri: &str, key: Option<&str>) -> (StatusCode, Value) {
            let mut request = Request::builder().uri(uri);
            if let Some(key) = key {
                request = request.header("X-Api-Key", key);
            }
            let response = router
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let body = to_bytes(response.into_body(), 1_000_000).await.unwrap();
            (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
        }

        #[tokio::test]
        async fn history_serves_each_flavor_its_own_imports_newest_first_by_page() {
            tokio::time::timeout(std::time::Duration::from_secs(120), async {
                let context = TestContext::new().await;
                let router = router(AuthState {
                    app: context.app.clone(),
                    schema: context.schema.clone(),
                    auth_runtime: context.auth_runtime.clone(),
                    rate_limiter: ScryerRateLimiter::from_env(Default::default()),
                    ws_origin_policy: WebSocketOriginPolicy::default(),
                    list_account_origin_policy: Default::default(),
                    authless_web_client_proof: AuthlessWebClientProofState::new(),
                });
                let admin = context.app.find_or_create_default_user().await.unwrap();
                let key = context
                    .app
                    .create_api_key(
                        &admin,
                        scryer_application::CreateApiKey {
                            label: "Silo".into(),
                            expiry: scryer_application::ApiKeyExpiryPreset::Never,
                        },
                    )
                    .await
                    .unwrap();
                let add = |name: &str, facet| NewTitle {
                    name: name.into(),
                    facet,
                    monitored: true,
                    ..Default::default()
                };
                let movie = context
                    .app
                    .add_title(&admin, add("Harbor Lights", MediaFacet::Movie))
                    .await
                    .unwrap();
                let series = context
                    .app
                    .add_title(&admin, add("Paper Lantern Bay", MediaFacet::Series))
                    .await
                    .unwrap();
                for event in [
                    import(&admin, &movie, "/movies/Harbor Lights/first.mkv"),
                    import(&admin, &series, "/tv/Paper Lantern Bay/s01e01.mkv"),
                    import(&admin, &movie, "/movies/Harbor Lights/second.mkv"),
                ] {
                    context.app.append_domain_event(event).await.unwrap();
                }

                let uri = "/compat/radarr/api/v3/history?page=1&pageSize=200&sortKey=date&sortDirection=descending";
                assert_eq!(read(&router, uri, None).await.0, StatusCode::UNAUTHORIZED);
                let (status, page) = read(&router, uri, Some(&key.raw_key)).await;
                assert_eq!(status, StatusCode::OK);
                let imported = page["records"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|record| record["data"]["importedPath"].as_str().unwrap().to_string())
                    .collect::<Vec<_>>();
                assert_eq!(
                    imported,
                    ["/movies/Harbor Lights/second.mkv", "/movies/Harbor Lights/first.mkv"]
                );
                assert_eq!(page["totalRecords"], 2);

                let (_, sonarr) = read(
                    &router,
                    "/compat/sonarr/api/v3/history?page=1&pageSize=200",
                    Some(&key.raw_key),
                )
                .await;
                assert_eq!(sonarr["records"].as_array().unwrap().len(), 1);
                assert_eq!(
                    sonarr["records"][0]["data"]["importedPath"],
                    "/tv/Paper Lantern Bay/s01e01.mkv"
                );

                // One record per page, and the total tells the client to read on.
                let (_, first) = read(
                    &router,
                    "/compat/radarr/api/v3/history?page=1&pageSize=1",
                    Some(&key.raw_key),
                )
                .await;
                let (_, second) = read(
                    &router,
                    "/compat/radarr/api/v3/history?page=2&pageSize=1",
                    Some(&key.raw_key),
                )
                .await;
                assert!(first["totalRecords"].as_u64().unwrap() >= 2);
                assert_eq!(
                    first["records"][0]["data"]["importedPath"],
                    "/movies/Harbor Lights/second.mkv"
                );
                assert_eq!(
                    second["records"][0]["data"]["importedPath"],
                    "/movies/Harbor Lights/first.mkv"
                );
                assert!(
                    first["records"][0]["id"].as_i64() > second["records"][0]["id"].as_i64()
                );
            })
            .await
            .expect("bounded history fixture");
        }
    }
}
