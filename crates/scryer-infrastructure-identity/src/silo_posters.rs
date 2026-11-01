//! Silo poster reads and uploads for poster overlays.
//!
//! Talks to Silo's native API (`/api/v2`), not its Jellyfin-compatible one,
//! which cannot receive images. The admin API key is sent as a bearer token,
//! so it never appears in a URL or an error. Nothing here deletes anything in
//! Silo.

use async_trait::async_trait;
use reqwest::StatusCode;
use scryer_application::overlays::{PosterOverlaySiloClient, SiloPosterItem, SiloServer};
use scryer_application::{AppError, AppResult};
use scryer_outbound_http::generic_reqwest_client;
use serde::Deserialize;
use url::Url;

pub struct HttpSiloPosterClient {
    client: reqwest::Client,
}

impl HttpSiloPosterClient {
    pub fn new() -> Self {
        Self {
            client: generic_reqwest_client(),
        }
    }
}

impl Default for HttpSiloPosterClient {
    fn default() -> Self {
        Self::new()
    }
}

/// `{api_url}/api/v2/<segments...>`, each segment escaped.
fn endpoint(server: &SiloServer, segments: &[&str]) -> AppResult<Url> {
    let mut url = Url::parse(&server.api_url)
        .map_err(|_| AppError::Validation("Silo's API address is not a valid URL".into()))?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| AppError::Validation("Silo's API address cannot take a path".into()))?;
        path.pop_if_empty().push("api").push("v2");
        for segment in segments {
            path.push(segment);
        }
    }
    Ok(url)
}

fn request_error(error: reqwest::Error) -> AppError {
    // reqwest errors carry the URL, never the bearer token.
    AppError::Repository(format!("Silo request failed: {error}"))
}

fn status_error(action: &str, status: StatusCode) -> AppError {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => AppError::Repository(format!(
            "Silo refused to {action}: the Jellyfin connection's API key must be a Silo admin key"
        )),
        _ => AppError::Repository(format!("Silo could not {action}: HTTP {status}")),
    }
}

#[derive(Deserialize)]
struct ItemDetail {
    #[serde(default)]
    poster_thumbhash: Option<String>,
}

#[derive(Deserialize)]
struct Applied {
    #[serde(default)]
    thumbhash: String,
}

/// A multipart body with the poster as `image` and its slot as `type`.
fn poster_form(poster: &[u8]) -> (String, Vec<u8>) {
    let boundary = format!("scryer-{:032x}", rand_suffix());
    let mut body = Vec::with_capacity(poster.len() + 512);
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"type\"\r\n\r\nposter\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"poster.jpg\"\r\n\
             Content-Type: image/jpeg\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(poster);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// Enough randomness to keep a boundary out of any JPEG's bytes.
fn rand_suffix() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    nanos ^ (u128::from(std::process::id()) << 64)
}

#[async_trait]
impl PosterOverlaySiloClient for HttpSiloPosterClient {
    async fn item(
        &self,
        server: &SiloServer,
        content_id: &str,
    ) -> AppResult<Option<SiloPosterItem>> {
        let response = self
            .client
            .get(endpoint(server, &["catalog", "items", content_id])?)
            .bearer_auth(&server.api_key)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(request_error)?;
        match response.status() {
            StatusCode::NOT_FOUND => return Ok(None),
            status if !status.is_success() => return Err(status_error("read a title", status)),
            _ => {}
        }
        let detail: ItemDetail = response.json().await.map_err(request_error)?;
        Ok(Some(SiloPosterItem {
            poster_thumbhash: detail.poster_thumbhash.filter(|hash| !hash.is_empty()),
        }))
    }

    async fn upload_poster(
        &self,
        server: &SiloServer,
        content_id: &str,
        poster: Vec<u8>,
    ) -> AppResult<String> {
        let (content_type, body) = poster_form(&poster);
        let response = self
            .client
            .post(endpoint(
                server,
                &["admin", "items", content_id, "images", "upload"],
            )?)
            .bearer_auth(&server.api_key)
            .header("Accept", "application/json")
            .header("Content-Type", content_type)
            .body(body)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_error("store a poster", status));
        }
        let applied: Applied = response.json().await.map_err(request_error)?;
        Ok(applied.thumbhash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(api_url: &str) -> SiloServer {
        SiloServer {
            connection_id: "c".into(),
            api_url: api_url.into(),
            api_key: "k".into(),
        }
    }

    #[test]
    fn endpoints_keep_a_base_path_and_escape_ids() {
        assert_eq!(
            endpoint(
                &server("http://silo:8090"),
                &["catalog", "items", "movie-tmdb-603"]
            )
            .unwrap()
            .as_str(),
            "http://silo:8090/api/v2/catalog/items/movie-tmdb-603"
        );
        assert_eq!(
            endpoint(
                &server("https://example.org/silo/"),
                &["admin", "items", "a/b"]
            )
            .unwrap()
            .as_str(),
            "https://example.org/silo/api/v2/admin/items/a%2Fb"
        );
    }

    #[test]
    fn the_form_carries_the_slot_and_the_image() {
        let (content_type, body) = poster_form(b"JPEGBYTES");
        let boundary = content_type.split("boundary=").nth(1).unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("name=\"type\"\r\n\r\nposter\r\n"));
        assert!(text.contains("name=\"image\"; filename=\"poster.jpg\""));
        assert!(text.contains("JPEGBYTES"));
        assert!(text.ends_with(&format!("--{boundary}--\r\n")));
    }
}
