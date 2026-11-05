//! Jellyfin poster reads and uploads for poster overlays (a local addition).
//!
//! The Silo push names titles by content IDs such as `movie-tmdb-603`.
//! Jellyfin has no lookup by provider ID that every server version supports,
//! so the client reads each library's movies and series once with their
//! `ProviderIds` and keeps an index from content ID to Jellyfin item ID. The
//! index is rebuilt when it is old, or on a miss once it is a little old, so a
//! title added to Jellyfin is found on a later pass without one library read
//! per title.
//!
//! A poster is stored with `POST /Items/{id}/Images/Primary` (a base64 body),
//! and the item's primary image tag stands in for Silo's thumbhash: Jellyfin
//! gives every stored image a new tag, so a tag other than the one recorded
//! at upload means the poster was changed in Jellyfin.
//!
//! The API key travels in Jellyfin's credential headers, never in a URL, and
//! errors carry statuses only. Nothing here deletes anything in Jellyfin.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine as _;
use reqwest::StatusCode;
use scryer_application::overlays::{SiloPosterItem, SiloServer};
use scryer_application::{AppError, AppResult};
use serde_json::Value;
use url::Url;

use crate::jellyfin::JellyfinAuth;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// An upload carries a few MB of base64.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// Items per library page.
pub const JELLYFIN_POSTER_PAGE_SIZE: usize = 500;
/// The most items indexed; a larger read fails rather than half-indexing.
pub const JELLYFIN_POSTER_MAX_ITEMS: usize = 100_000;
/// An index older than this is rebuilt before use.
const INDEX_MAX_AGE: Duration = Duration::from_secs(30 * 60);
/// A miss rebuilds an index at least this old, so a pass with many titles
/// Jellyfin lacks reads the library at most once per interval.
const INDEX_MISS_REFRESH_AGE: Duration = Duration::from_secs(5 * 60);

struct PosterIndex {
    built: Instant,
    /// Content ID (`movie-tmdb-603`) to Jellyfin item ID.
    items: HashMap<String, String>,
}

pub struct JellyfinPosterClient {
    client: reqwest::Client,
    indexes: Mutex<HashMap<(String, String), PosterIndex>>,
}

impl JellyfinPosterClient {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            indexes: Mutex::new(HashMap::new()),
        }
    }

    fn cache_key(server: &SiloServer) -> (String, String) {
        (server.api_url.clone(), server.api_key.clone())
    }

    /// The cached item ID and the index's age, if there is an index.
    fn cached(&self, server: &SiloServer, content_id: &str) -> Option<(Option<String>, Duration)> {
        let indexes = self.indexes.lock().ok()?;
        let index = indexes.get(&Self::cache_key(server))?;
        Some((index.items.get(content_id).cloned(), index.built.elapsed()))
    }

    /// The Jellyfin item ID for a content ID, or `None` when Jellyfin has no
    /// such title.
    async fn resolve(&self, server: &SiloServer, content_id: &str) -> AppResult<Option<String>> {
        match self.cached(server, content_id) {
            Some((_, age)) if age >= INDEX_MAX_AGE => {}
            Some((Some(item_id), _)) => return Ok(Some(item_id)),
            Some((None, age)) if age < INDEX_MISS_REFRESH_AGE => return Ok(None),
            _ => {}
        }
        let items = self.read_index(server).await?;
        let found = items.get(content_id).cloned();
        if let Ok(mut indexes) = self.indexes.lock() {
            indexes.insert(
                Self::cache_key(server),
                PosterIndex {
                    built: Instant::now(),
                    items,
                },
            );
        }
        Ok(found)
    }

    fn forget(&self, server: &SiloServer, content_id: &str) {
        if let Ok(mut indexes) = self.indexes.lock()
            && let Some(index) = indexes.get_mut(&Self::cache_key(server))
        {
            index.items.remove(content_id);
        }
    }

    /// Every movie and series with the content IDs its provider IDs give.
    async fn read_index(&self, server: &SiloServer) -> AppResult<HashMap<String, String>> {
        let base = jellyfin_base(&server.api_url)?;
        let mut items = HashMap::new();
        let mut start = 0;
        loop {
            let page = self
                .get_json(&server.api_key, index_url(&base, start)?)
                .await?;
            let rows = page
                .get("Items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for row in &rows {
                index_item(row, &mut items);
            }
            start += rows.len();
            if rows.len() < JELLYFIN_POSTER_PAGE_SIZE {
                return Ok(items);
            }
            if start >= JELLYFIN_POSTER_MAX_ITEMS {
                return Err(AppError::Repository(format!(
                    "Jellyfin has more than {JELLYFIN_POSTER_MAX_ITEMS} movies and series; the poster index was not built"
                )));
            }
        }
    }

    /// The item's current primary image tag, or `None` when the item is gone.
    async fn primary_tag(
        &self,
        server: &SiloServer,
        item_id: &str,
    ) -> AppResult<Option<Option<String>>> {
        let base = jellyfin_base(&server.api_url)?;
        let page = self
            .get_json(&server.api_key, item_url(&base, item_id)?)
            .await?;
        let Some(item) = page
            .get("Items")
            .and_then(Value::as_array)
            .and_then(|rows| {
                rows.iter()
                    .find(|row| row.get("Id").and_then(Value::as_str) == Some(item_id))
            })
        else {
            return Ok(None);
        };
        Ok(Some(
            item.get("ImageTags")
                .and_then(|tags| tags.get("Primary"))
                .and_then(Value::as_str)
                .filter(|tag| !tag.is_empty())
                .map(str::to_string),
        ))
    }

    async fn get_json(&self, api_key: &str, url: Url) -> AppResult<Value> {
        let response = self
            .client
            .get(url)
            .header("Accept", "application/json")
            .jellyfin_auth(api_key, None)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_error("read its library", status));
        }
        response
            .json::<Value>()
            .await
            .map_err(|_| AppError::Repository("Jellyfin sent an unreadable answer".into()))
    }

    pub async fn item(
        &self,
        server: &SiloServer,
        content_id: &str,
    ) -> AppResult<Option<SiloPosterItem>> {
        let Some(item_id) = self.resolve(server, content_id).await? else {
            return Ok(None);
        };
        match self.primary_tag(server, &item_id).await? {
            Some(tag) => Ok(Some(SiloPosterItem {
                poster_thumbhash: tag,
            })),
            None => {
                // The item left Jellyfin since the index was read.
                self.forget(server, content_id);
                Ok(None)
            }
        }
    }

    pub async fn upload_poster(
        &self,
        server: &SiloServer,
        content_id: &str,
        poster: Vec<u8>,
    ) -> AppResult<Option<String>> {
        let item_id = self
            .resolve(server, content_id)
            .await?
            .ok_or_else(|| AppError::Repository("Jellyfin no longer has this title".into()))?;
        let base = jellyfin_base(&server.api_url)?;
        let response = self
            .client
            .post(upload_url(&base, &item_id)?)
            .header("Content-Type", image_content_type(&poster))
            .jellyfin_auth(&server.api_key, None)
            .timeout(UPLOAD_TIMEOUT)
            .body(base64::engine::general_purpose::STANDARD.encode(&poster))
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_error("store a poster", status));
        }
        // Jellyfin's answer carries no tag; read the one it gave the image.
        Ok(self.primary_tag(server, &item_id).await?.flatten())
    }
}

/// Add an item's content IDs to the index: TMDB, TVDB and IMDb, under the
/// kind Silo-style content IDs use (`movie` or `series`). The first item to
/// claim an ID keeps it.
fn index_item(row: &Value, items: &mut HashMap<String, String>) {
    let Some(item_id) = row.get("Id").and_then(Value::as_str) else {
        return;
    };
    let kind = match row.get("Type").and_then(Value::as_str) {
        Some(kind) if kind.eq_ignore_ascii_case("movie") => "movie",
        Some(kind) if kind.eq_ignore_ascii_case("series") => "series",
        _ => return,
    };
    let Some(ids) = row.get("ProviderIds").and_then(Value::as_object) else {
        return;
    };
    for (provider, value) in ids {
        let provider = provider.to_ascii_lowercase();
        let value = match value {
            Value::String(value) => value.trim().to_ascii_lowercase(),
            Value::Number(value) => value.to_string(),
            _ => continue,
        };
        let valid = match provider.as_str() {
            "tmdb" | "tvdb" => !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
            "imdb" => {
                value.len() > 2
                    && value.starts_with("tt")
                    && value[2..].bytes().all(|b| b.is_ascii_digit())
            }
            _ => false,
        };
        if valid {
            items
                .entry(format!("{kind}-{provider}-{value}"))
                .or_insert_with(|| item_id.to_string());
        }
    }
}

/// `image/png` for a PNG, else `image/jpeg` (what the engine renders).
fn image_content_type(image: &[u8]) -> &'static str {
    if image.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else {
        "image/jpeg"
    }
}

fn jellyfin_base(api_url: &str) -> AppResult<Url> {
    let mut url = Url::parse(api_url.trim())
        .map_err(|_| AppError::Validation("Jellyfin's address is not a valid URL".into()))?;
    if url.cannot_be_a_base() {
        return Err(AppError::Validation(
            "Jellyfin's address cannot take a path".into(),
        ));
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

fn join(base: &Url, path: &str) -> AppResult<Url> {
    base.join(path)
        .map_err(|_| AppError::Validation("Jellyfin's address cannot take a path".into()))
}

fn index_url(base: &Url, start: usize) -> AppResult<Url> {
    let mut url = join(base, "Items")?;
    url.query_pairs_mut()
        .append_pair("Recursive", "true")
        .append_pair("IncludeItemTypes", "Movie,Series")
        .append_pair("Fields", "ProviderIds")
        .append_pair("EnableImages", "false")
        .append_pair("StartIndex", &start.to_string())
        .append_pair("Limit", &JELLYFIN_POSTER_PAGE_SIZE.to_string());
    Ok(url)
}

fn item_url(base: &Url, item_id: &str) -> AppResult<Url> {
    let mut url = join(base, "Items")?;
    url.query_pairs_mut()
        .append_pair("Ids", item_id)
        .append_pair("Recursive", "true")
        .append_pair("Limit", "1");
    Ok(url)
}

fn upload_url(base: &Url, item_id: &str) -> AppResult<Url> {
    let mut url = join(base, "Items/")?;
    url.path_segments_mut()
        .map_err(|_| AppError::Validation("Jellyfin's address cannot take a path".into()))?
        .pop_if_empty()
        .push(item_id)
        .push("Images")
        .push("Primary");
    Ok(url)
}

fn request_error(error: reqwest::Error) -> AppError {
    // The key travels in headers; drop the URL anyway to keep item IDs and
    // query strings out of stored errors.
    let reason = if error.is_timeout() {
        "timed out"
    } else if error.is_connect() {
        "could not connect"
    } else {
        "request failed"
    };
    AppError::Repository(format!("Jellyfin {reason}"))
}

fn status_error(action: &str, status: StatusCode) -> AppError {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => AppError::Repository(format!(
            "Jellyfin refused to {action}: the Jellyfin connection's API key must be an admin API key"
        )),
        _ => AppError::Repository(format!("Jellyfin could not {action}: HTTP {status}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_application::overlays::PosterPushServerKind;
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn server(api_url: &str) -> SiloServer {
        SiloServer {
            connection_id: "c".into(),
            api_url: api_url.into(),
            api_key: "k".into(),
            kind: PosterPushServerKind::Jellyfin,
        }
    }

    fn client() -> JellyfinPosterClient {
        JellyfinPosterClient::new(scryer_outbound_http::generic_reqwest_client())
    }

    fn library() -> Value {
        json!({ "Items": [
            { "Id": "m1", "Type": "Movie",
              "ProviderIds": { "Tmdb": "603", "Imdb": "tt0133093" } },
            { "Id": "s1", "Type": "Series", "ProviderIds": { "TVDB": 121361 } },
            { "Id": "dup", "Type": "Movie", "ProviderIds": { "Tmdb": "603" } },
            { "Id": "e1", "Type": "Episode", "ProviderIds": { "Tvdb": "5" } }
        ]})
    }

    #[test]
    fn the_index_maps_provider_ids_to_content_ids() {
        let mut items = HashMap::new();
        for row in library()["Items"].as_array().unwrap() {
            index_item(row, &mut items);
        }
        assert_eq!(items["movie-tmdb-603"], "m1");
        assert_eq!(items["movie-imdb-tt0133093"], "m1");
        assert_eq!(items["series-tvdb-121361"], "s1");
        assert_eq!(items.len(), 3, "episodes are not indexed: {items:?}");
    }

    #[test]
    fn urls_keep_a_base_path_and_escape_the_item_id() {
        let base = jellyfin_base("https://example.org/jellyfin").unwrap();
        assert_eq!(
            upload_url(&base, "a/b").unwrap().as_str(),
            "https://example.org/jellyfin/Items/a%2Fb/Images/Primary"
        );
        assert!(
            index_url(&base, 500)
                .unwrap()
                .as_str()
                .starts_with("https://example.org/jellyfin/Items?Recursive=true")
        );
    }

    #[tokio::test]
    async fn reads_a_titles_tag_and_uploads_a_base64_poster() {
        let jellyfin = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Items"))
            .and(query_param("IncludeItemTypes", "Movie,Series"))
            .and(header("X-Emby-Token", "k"))
            .respond_with(ResponseTemplate::new(200).set_body_json(library()))
            .expect(1)
            .mount(&jellyfin)
            .await;
        Mock::given(method("GET"))
            .and(path("/Items"))
            .and(query_param("Ids", "m1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": [{ "Id": "m1", "ImageTags": { "Primary": "tag-1" } }]
            })))
            .mount(&jellyfin)
            .await;
        let poster = b"\xff\xd8\xff poster".to_vec();
        Mock::given(method("POST"))
            .and(path("/Items/m1/Images/Primary"))
            .and(header("Content-Type", "image/jpeg"))
            .and(header("X-Emby-Token", "k"))
            .and(wiremock::matchers::body_string(
                base64::engine::general_purpose::STANDARD.encode(&poster),
            ))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&jellyfin)
            .await;

        let client = client();
        let server = server(&jellyfin.uri());
        let item = client.item(&server, "movie-tmdb-603").await.unwrap();
        assert_eq!(item.unwrap().poster_thumbhash.as_deref(), Some("tag-1"));
        let tag = client
            .upload_poster(&server, "movie-imdb-tt0133093", poster)
            .await
            .unwrap();
        assert_eq!(tag.as_deref(), Some("tag-1"));
        // A title Jellyfin lacks is answered from the fresh index.
        assert!(
            client
                .item(&server, "movie-tmdb-1")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_refused_key_is_an_error_without_the_key() {
        let jellyfin = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&jellyfin)
            .await;
        let error = client()
            .item(&server(&jellyfin.uri()), "movie-tmdb-603")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("admin API key"), "{error}");
        assert!(!error.contains("k\""), "{error}");
    }

    #[tokio::test]
    async fn an_item_gone_from_jellyfin_is_not_found() {
        let jellyfin = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Items"))
            .and(query_param("IncludeItemTypes", "Movie,Series"))
            .respond_with(ResponseTemplate::new(200).set_body_json(library()))
            .mount(&jellyfin)
            .await;
        Mock::given(method("GET"))
            .and(path("/Items"))
            .and(query_param("Ids", "m1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "Items": [] })))
            .mount(&jellyfin)
            .await;
        let client = client();
        let server = server(&jellyfin.uri());
        assert!(
            client
                .item(&server, "movie-tmdb-603")
                .await
                .unwrap()
                .is_none()
        );
    }
}
