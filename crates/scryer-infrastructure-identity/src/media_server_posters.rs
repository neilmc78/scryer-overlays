//! Plex poster reads and uploads for poster overlays.
//!
//! Talks to the selected server at the connection's `base_url` with its
//! stored token, sent as a header so it never appears in a URL or an error.
//! Four calls: read an item, read its selected poster, upload a poster, and
//! lock or unlock the poster field. Nothing here deletes anything on the
//! server.

use async_trait::async_trait;
use reqwest::StatusCode;
use scryer_application::overlays::{
    PlexMaintenanceWindow, PlexPosterItem, PosterOverlayPlexClient,
};
use scryer_application::{AppError, AppResult};
use scryer_domain::MediaServerConnection;
use scryer_outbound_http::generic_reqwest_client;
use serde_json::Value;
use url::Url;

pub struct HttpPlexPosterClient {
    client: reqwest::Client,
}

impl HttpPlexPosterClient {
    pub fn new() -> Self {
        Self {
            client: generic_reqwest_client(),
        }
    }
}

impl Default for HttpPlexPosterClient {
    fn default() -> Self {
        Self::new()
    }
}

fn token(connection: &MediaServerConnection) -> AppResult<&str> {
    connection
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| AppError::Validation("the Plex connection has no stored token".into()))
}

/// `path` resolved against the server URL, keeping any base path.
fn server_url(connection: &MediaServerConnection, path: &str) -> AppResult<Url> {
    let mut base = connection.base_url.trim().to_string();
    if !base.ends_with('/') {
        base.push('/');
    }
    Url::parse(&base)
        .and_then(|base| base.join(path.trim_start_matches('/')))
        .map_err(|_| AppError::Validation("the Plex connection has an invalid server URL".into()))
}

fn request_failed(action: &str, error: reqwest::Error) -> AppError {
    let reason = if error.is_timeout() {
        "timed out"
    } else if error.is_connect() {
        "could not connect"
    } else {
        "request failed"
    };
    AppError::Repository(format!("Plex {action}: {reason}"))
}

fn bad_status(action: &str, status: StatusCode) -> AppError {
    AppError::Repository(format!(
        "Plex {action} failed with status {}",
        status.as_u16()
    ))
}

fn json_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

#[async_trait]
impl PosterOverlayPlexClient for HttpPlexPosterClient {
    async fn item(
        &self,
        connection: &MediaServerConnection,
        rating_key: &str,
    ) -> AppResult<Option<PlexPosterItem>> {
        let url = server_url(connection, &format!("library/metadata/{rating_key}"))?;
        let response = self
            .client
            .get(url)
            .header("Accept", "application/json")
            .header("X-Plex-Token", token(connection)?)
            .send()
            .await
            .map_err(|error| request_failed("item read", error))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(bad_status("item read", response.status()));
        }
        let body = response
            .json::<Value>()
            .await
            .map_err(|_| AppError::Repository("Plex item read returned unreadable JSON".into()))?;
        let Some(metadata) = body
            .get("MediaContainer")
            .and_then(|container| container.get("Metadata"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
        else {
            return Ok(None);
        };
        let item_type = json_string(metadata.get("type")).unwrap_or_default();
        let section_id = json_string(metadata.get("librarySectionID")).unwrap_or_default();
        Ok(Some(PlexPosterItem {
            rating_key: json_string(metadata.get("ratingKey"))
                .unwrap_or_else(|| rating_key.to_string()),
            item_type,
            section_id,
            thumb: json_string(metadata.get("thumb")),
        }))
    }

    async fn current_poster(
        &self,
        connection: &MediaServerConnection,
        item: &PlexPosterItem,
    ) -> AppResult<Option<Vec<u8>>> {
        let Some(thumb) = item.thumb.as_deref() else {
            return Ok(None);
        };
        // Only the server's own image paths are followed.
        if !thumb.starts_with("/library/") {
            return Ok(None);
        }
        let response = self
            .client
            .get(server_url(connection, thumb)?)
            .header("X-Plex-Token", token(connection)?)
            .send()
            .await
            .map_err(|error| request_failed("poster read", error))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(bad_status("poster read", response.status()));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| request_failed("poster read", error))?;
        Ok(Some(bytes.to_vec()))
    }

    async fn upload_poster(
        &self,
        connection: &MediaServerConnection,
        item: &PlexPosterItem,
        jpeg: Vec<u8>,
    ) -> AppResult<()> {
        let url = server_url(
            connection,
            &format!("library/metadata/{}/posters", item.rating_key),
        )?;
        let response = self
            .client
            .post(url)
            .header("X-Plex-Token", token(connection)?)
            .header("Content-Type", "image/jpeg")
            .body(jpeg)
            .send()
            .await
            .map_err(|error| request_failed("poster upload", error))?;
        if !response.status().is_success() {
            return Err(bad_status("poster upload", response.status()));
        }
        Ok(())
    }

    async fn maintenance_window(
        &self,
        connection: &MediaServerConnection,
    ) -> AppResult<Option<PlexMaintenanceWindow>> {
        let response = self
            .client
            .get(server_url(connection, ":/prefs")?)
            .header("Accept", "application/json")
            .header("X-Plex-Token", token(connection)?)
            .send()
            .await
            .map_err(|error| request_failed("settings read", error))?;
        if !response.status().is_success() {
            return Err(bad_status("settings read", response.status()));
        }
        let body = response.json::<Value>().await.map_err(|_| {
            AppError::Repository("Plex settings read returned unreadable JSON".into())
        })?;
        let settings = body
            .get("MediaContainer")
            .and_then(|container| container.get("Setting"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let hour = |id: &str| -> Option<u32> {
            settings
                .iter()
                .find(|setting| setting.get("id").and_then(Value::as_str) == Some(id))
                .and_then(|setting| json_string(setting.get("value")))
                .and_then(|value| value.trim().parse::<u32>().ok())
                .filter(|hour| *hour < 24)
        };
        Ok(match (hour("ButlerStartHour"), hour("ButlerEndHour")) {
            (Some(start_hour), Some(end_hour)) => Some(PlexMaintenanceWindow {
                start_hour,
                end_hour,
            }),
            _ => None,
        })
    }

    async fn set_poster_locked(
        &self,
        connection: &MediaServerConnection,
        item: &PlexPosterItem,
        locked: bool,
    ) -> AppResult<()> {
        let item_type = match item.item_type.as_str() {
            "movie" => "1",
            "show" => "2",
            other => {
                return Err(AppError::Validation(format!(
                    "Plex items of type \"{other}\" have no lockable poster here"
                )));
            }
        };
        if item.section_id.is_empty() {
            return Err(AppError::Validation(
                "the Plex item has no library section".into(),
            ));
        }
        let mut url = server_url(
            connection,
            &format!("library/sections/{}/all", item.section_id),
        )?;
        url.query_pairs_mut()
            .append_pair("type", item_type)
            .append_pair("id", &item.rating_key)
            .append_pair("thumb.locked", if locked { "1" } else { "0" });
        let response = self
            .client
            .put(url)
            .header("X-Plex-Token", token(connection)?)
            .send()
            .await
            .map_err(|error| request_failed("poster lock", error))?;
        if !response.status().is_success() {
            return Err(bad_status("poster lock", response.status()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use scryer_domain::{AppPermissionMask, MediaServerProvider};
    use serde_json::json;
    use wiremock::matchers::{body_bytes, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn connection(base_url: &str) -> MediaServerConnection {
        MediaServerConnection {
            id: "plex-1".into(),
            provider: MediaServerProvider::Plex,
            display_name: "Plex".into(),
            base_url: base_url.into(),
            external_url: None,
            enabled: true,
            login_enabled: false,
            linking_enabled: false,
            auto_add_enabled: false,
            default_app_permissions: AppPermissionMask::default(),
            default_library_grants: Vec::new(),
            machine_id: Some("machine".into()),
            api_key: Some("plex-token".into()),
            emby_server_id: None,
            emby_connect_enabled: false,
            path_mappings: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn movie() -> PlexPosterItem {
        PlexPosterItem {
            rating_key: "42".into(),
            item_type: "movie".into(),
            section_id: "3".into(),
            thumb: Some("/library/metadata/42/thumb/1700".into()),
        }
    }

    #[tokio::test]
    async fn reads_an_item_with_its_section_and_poster() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/42"))
            .and(header("x-plex-token", "plex-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": {"Metadata": [{
                    "ratingKey": "42", "type": "movie", "librarySectionID": 3,
                    "thumb": "/library/metadata/42/thumb/1700"
                }]}
            })))
            .mount(&server)
            .await;
        let item = HttpPlexPosterClient::new()
            .item(&connection(&server.uri()), "42")
            .await
            .unwrap();
        assert_eq!(item, Some(movie()));
    }

    #[tokio::test]
    async fn a_missing_item_is_none_and_other_failures_are_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/404"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/401"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let client = HttpPlexPosterClient::new();
        let connection = connection(&server.uri());
        assert_eq!(client.item(&connection, "404").await.unwrap(), None);
        let error = client.item(&connection, "401").await.unwrap_err();
        assert!(error.to_string().contains("401"), "{error}");
        assert!(
            !error.to_string().contains("plex-token"),
            "errors never carry the token"
        );
    }

    #[tokio::test]
    async fn uploads_the_poster_then_locks_and_unlocks_it() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/library/metadata/42/posters"))
            .and(header("x-plex-token", "plex-token"))
            .and(header("content-type", "image/jpeg"))
            .and(body_bytes(vec![1, 2, 3]))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        for locked in ["1", "0"] {
            Mock::given(method("PUT"))
                .and(path("/library/sections/3/all"))
                .and(query_param("type", "1"))
                .and(query_param("id", "42"))
                .and(query_param("thumb.locked", locked))
                .respond_with(ResponseTemplate::new(200))
                .expect(1)
                .mount(&server)
                .await;
        }
        let client = HttpPlexPosterClient::new();
        let connection = connection(&server.uri());
        client
            .upload_poster(&connection, &movie(), vec![1, 2, 3])
            .await
            .unwrap();
        client
            .set_poster_locked(&connection, &movie(), true)
            .await
            .unwrap();
        client
            .set_poster_locked(&connection, &movie(), false)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn reads_the_selected_poster_only_from_server_paths() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/42/thumb/1700"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![9, 9]))
            .mount(&server)
            .await;
        let client = HttpPlexPosterClient::new();
        let connection = connection(&server.uri());
        assert_eq!(
            client.current_poster(&connection, &movie()).await.unwrap(),
            Some(vec![9, 9])
        );
        let elsewhere = PlexPosterItem {
            thumb: Some("https://example.com/poster.jpg".into()),
            ..movie()
        };
        assert_eq!(
            client
                .current_poster(&connection, &elsewhere)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn reads_the_maintenance_hours_from_the_server_settings() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/:/prefs"))
            .and(header("x-plex-token", "plex-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "MediaContainer": {"Setting": [
                    {"id": "FriendlyName", "value": "Plex"},
                    {"id": "ButlerStartHour", "value": 3},
                    {"id": "ButlerEndHour", "value": "5"}
                ]}
            })))
            .mount(&server)
            .await;
        assert_eq!(
            HttpPlexPosterClient::new()
                .maintenance_window(&connection(&server.uri()))
                .await
                .unwrap(),
            Some(PlexMaintenanceWindow {
                start_hour: 3,
                end_hour: 5
            })
        );
    }

    #[tokio::test]
    async fn a_server_under_a_base_path_keeps_it() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/plex/library/metadata/42"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;
        let connection = connection(&format!("{}/plex", server.uri()));
        assert_eq!(
            HttpPlexPosterClient::new()
                .item(&connection, "42")
                .await
                .unwrap(),
            None
        );
    }
}
