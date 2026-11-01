//! Pushing rendered overlays to Silo.
//!
//! Silo keys titles by deterministic content IDs built from provider IDs
//! (`movie-tmdb-603`, `series-tvdb-121361`), so a title is found by building
//! those IDs from its own TMDB, TVDB and IMDb IDs in Silo's order and asking
//! for each until one exists. No catalog scan is needed.
//!
//! The poster is uploaded through Silo's admin API, which stores it as the
//! title's artwork and locks it so Silo's metadata refreshes keep it. A poster
//! is uploaded again only when the rendered output changes.
//!
//! The API key is the one stored on the Jellyfin connection whose host matches
//! Silo's API address, so no second copy of the secret is kept. Push records
//! share the Plex push table, keyed by that connection.
//!
//! Silo is never asked to delete anything. A poster someone changed in Silo
//! after Scryer pushed one is left alone and reported. When push is turned
//! off, or a title's library no longer has overlays, the original poster is
//! uploaded back.

use std::sync::Arc;

use chrono::Utc;
use scryer_domain::MediaServerProvider;

use super::ports::{
    PosterOverlayInputs, PosterOverlayPlexState, PosterOverlaySettings, PosterOverlaySiloClient,
    PosterOverlayVariant, SiloServer,
};
use super::service::{AppPosterOverlayServices, OVERLAY_PASS_PAGE};
use crate::{AppError, AppResult, MediaServerConnectionRepository};

/// Recorded on a title whose poster was replaced in Silo after Scryer pushed
/// one; Scryer leaves that poster alone.
pub const SILO_POSTER_CHANGED_IN_SILO: &str = "the poster was changed in Silo; left as it is";

#[derive(Clone)]
pub(crate) struct SiloPush {
    connections: Arc<dyn MediaServerConnectionRepository>,
    client: Arc<dyn PosterOverlaySiloClient>,
}

impl SiloPush {
    pub(crate) fn new(
        connections: Arc<dyn MediaServerConnectionRepository>,
        client: Arc<dyn PosterOverlaySiloClient>,
    ) -> Self {
        Self {
            connections,
            client,
        }
    }

    /// Silo's address and key, or `None` when no API address is set. Errors
    /// when the address is set but no enabled Jellyfin connection with a key
    /// points at the same host.
    pub(crate) async fn server(
        &self,
        settings: &PosterOverlaySettings,
    ) -> AppResult<Option<SiloServer>> {
        let Some(api_url) = settings
            .silo_api_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
        else {
            return Ok(None);
        };
        let host = host_of(api_url)
            .ok_or_else(|| AppError::Validation("Silo's API address is not a valid URL".into()))?;
        let connection = self
            .connections
            .list(Some(MediaServerProvider::Jellyfin))
            .await?
            .into_iter()
            .find(|connection| {
                connection.enabled
                    && connection
                        .api_key
                        .as_deref()
                        .is_some_and(|key| !key.trim().is_empty())
                    && host_of(&connection.base_url).as_deref() == Some(host.as_str())
            })
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "no enabled Jellyfin connection with an API key points at {host}; add Silo as a Jellyfin media server first"
                ))
            })?;
        Ok(Some(SiloServer {
            api_key: connection.api_key.clone().unwrap_or_default(),
            connection_id: connection.id,
            api_url: api_url.trim_end_matches('/').to_string(),
        }))
    }
}

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
}

/// Silo content IDs the title can have, most likely first: TMDB, IMDb, TVDB
/// for a movie; TVDB, TMDB, IMDb for a series. This mirrors Silo's own anchor
/// order, and the rest cover a title Silo matched through another provider.
pub fn silo_content_ids(inputs: &PosterOverlayInputs) -> Vec<String> {
    let movie = inputs.facet.as_deref() == Some("movie");
    let tmdb = digits(inputs.tmdb_id.as_deref()).map(|id| ("tmdb", id));
    let tvdb = digits(inputs.tvdb_id.as_deref()).map(|id| ("tvdb", id));
    let imdb = inputs
        .imdb_id
        .as_deref()
        .map(|id| id.trim().to_ascii_lowercase())
        .filter(|id| {
            id.len() > 2 && id.starts_with("tt") && id[2..].bytes().all(|b| b.is_ascii_digit())
        })
        .map(|id| ("imdb", id));
    let (kind, order) = if movie {
        ("movie", [tmdb, imdb, tvdb])
    } else {
        ("series", [tvdb, tmdb, imdb])
    };
    order
        .into_iter()
        .flatten()
        .map(|(provider, id)| format!("{kind}-{provider}-{id}"))
        .collect()
}

/// A numeric provider ID as Silo keeps it: trimmed, otherwise unchanged.
fn digits(id: Option<&str>) -> Option<String> {
    let id = id?.trim();
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_string())
}

/// What Silo pushes did for one title, or across a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SiloSyncCounts {
    pub pushed: usize,
    pub unchanged: usize,
    /// Titles Silo does not have.
    pub not_in_silo: usize,
    /// Posters changed in Silo after a push, left alone.
    pub changed_in_silo: usize,
    /// Originals put back because push is off or overlays were disabled.
    pub restored: usize,
    pub failed: usize,
}

impl SiloSyncCounts {
    pub fn merge(&mut self, other: Self) {
        self.pushed += other.pushed;
        self.unchanged += other.unchanged;
        self.not_in_silo += other.not_in_silo;
        self.changed_in_silo += other.changed_in_silo;
        self.restored += other.restored;
        self.failed += other.failed;
    }

    pub fn is_empty(&self) -> bool {
        self.pushed + self.not_in_silo + self.changed_in_silo + self.restored + self.failed == 0
    }
}

impl AppPosterOverlayServices {
    /// The Silo server to push to, when Silo push is on and configured.
    pub(crate) async fn silo_push_server(&self) -> Option<SiloServer> {
        let silo = self.silo.as_ref()?;
        let settings = self.repository.get_settings().await.ok()?;
        if !settings.silo_push_enabled {
            return None;
        }
        match silo.server(&settings).await {
            Ok(server) => server,
            Err(error) => {
                tracing::warn!(%error, "Silo poster push is on but Silo cannot be reached");
                None
            }
        }
    }

    /// Bring the title's poster in Silo up to date with its rendered overlay.
    /// Failures are recorded on the push record and never fail the render.
    pub(crate) async fn sync_silo(&self, title_id: &str, server: &SiloServer) -> SiloSyncCounts {
        let mut counts = SiloSyncCounts::default();
        let Some(silo) = self.silo.as_ref() else {
            return counts;
        };
        match self.push_silo(silo, server, title_id).await {
            Ok(PushOutcome::Pushed) => counts.pushed += 1,
            Ok(PushOutcome::Unchanged) => counts.unchanged += 1,
            Ok(PushOutcome::NotInSilo) => counts.not_in_silo += 1,
            Ok(PushOutcome::ChangedInSilo) => counts.changed_in_silo += 1,
            Err((content_id, error)) => {
                let message = error.to_string();
                tracing::warn!(title_id, error = %message, "poster overlay push to Silo failed");
                if let Some(content_id) = content_id {
                    let previous = self
                        .repository
                        .get_plex_state(&server.connection_id, title_id, &content_id)
                        .await
                        .ok()
                        .flatten()
                        .unwrap_or_default();
                    let _ = self
                        .repository
                        .save_plex_state(&PosterOverlayPlexState {
                            connection_id: server.connection_id.clone(),
                            title_id: title_id.to_string(),
                            provider_item_id: content_id,
                            last_error: Some(message),
                            ..previous
                        })
                        .await;
                }
                counts.failed += 1;
            }
        }
        counts
    }

    /// The content ID Silo has the title under, trying the one pushed to
    /// before, then each candidate in turn.
    async fn find_silo_item(
        &self,
        silo: &SiloPush,
        server: &SiloServer,
        title_id: &str,
        inputs: &PosterOverlayInputs,
    ) -> AppResult<Option<(String, super::ports::SiloPosterItem)>> {
        let mut candidates = self
            .repository
            .list_plex_states_for_title(title_id)
            .await?
            .into_iter()
            .filter(|state| state.connection_id == server.connection_id)
            .map(|state| state.provider_item_id)
            .collect::<Vec<_>>();
        for id in silo_content_ids(inputs) {
            if !candidates.contains(&id) {
                candidates.push(id);
            }
        }
        for content_id in candidates {
            if let Some(item) = silo.client.item(server, &content_id).await? {
                return Ok(Some((content_id, item)));
            }
        }
        Ok(None)
    }

    async fn push_silo(
        &self,
        silo: &SiloPush,
        server: &SiloServer,
        title_id: &str,
    ) -> Result<PushOutcome, (Option<String>, AppError)> {
        let fail = |error: AppError| (None, error);
        let Some(output_hash) = self
            .repository
            .get_state(title_id)
            .await
            .map_err(fail)?
            .and_then(|state| state.output_hash)
        else {
            return Ok(PushOutcome::Unchanged);
        };
        let Some(inputs) = self.repository.load_inputs(title_id).await.map_err(fail)? else {
            return Ok(PushOutcome::Unchanged);
        };
        let Some((content_id, item)) = self
            .find_silo_item(silo, server, title_id, &inputs)
            .await
            .map_err(fail)?
        else {
            return Ok(PushOutcome::NotInSilo);
        };
        let fail = |error: AppError| (Some(content_id.clone()), error);
        let state = self
            .repository
            .get_plex_state(&server.connection_id, title_id, &content_id)
            .await
            .map_err(fail)?;
        if let Some(state) = state
            .as_ref()
            .filter(|state| state.pushed_output_hash.is_some())
        {
            // Without a thumbhash from the upload, a change in Silo cannot be
            // told apart, so the poster is only kept current.
            if state.pushed_thumb.is_some() && item.poster_thumbhash != state.pushed_thumb {
                // Someone chose another poster in Silo after our upload.
                if state.last_error.as_deref() != Some(SILO_POSTER_CHANGED_IN_SILO) {
                    self.repository
                        .save_plex_state(&PosterOverlayPlexState {
                            last_error: Some(SILO_POSTER_CHANGED_IN_SILO.to_string()),
                            ..state.clone()
                        })
                        .await
                        .map_err(fail)?;
                }
                return Ok(PushOutcome::ChangedInSilo);
            }
            if state.pushed_output_hash.as_deref() == Some(output_hash.as_str()) {
                if state.last_error.is_some() {
                    self.repository
                        .save_plex_state(&PosterOverlayPlexState {
                            last_error: None,
                            ..state.clone()
                        })
                        .await
                        .map_err(fail)?;
                }
                return Ok(PushOutcome::Unchanged);
            }
        }
        let poster = self
            .engine
            .read_output(title_id, PosterOverlayVariant::Full)
            .await
            .map_err(fail)?
            .ok_or_else(|| {
                fail(AppError::Repository(
                    "the rendered poster is missing".into(),
                ))
            })?;
        let thumbhash = silo
            .client
            .upload_poster(server, &content_id, poster)
            .await
            .map_err(fail)?;
        self.repository
            .save_plex_state(&PosterOverlayPlexState {
                connection_id: server.connection_id.clone(),
                title_id: title_id.to_string(),
                provider_item_id: content_id.clone(),
                pushed_output_hash: Some(output_hash),
                pushed_thumb: thumbhash,
                pushed_at: Some(Utc::now()),
                last_error: None,
            })
            .await
            .map_err(fail)?;
        Ok(PushOutcome::Pushed)
    }

    /// Put the original poster back in Silo for every pushed title whose
    /// overlay is no longer active there: all of them when push is off, else
    /// those whose library no longer has overlays.
    pub(crate) async fn restore_inactive_silo_posters(&self) -> SiloSyncCounts {
        let mut counts = SiloSyncCounts::default();
        let Some(silo) = self.silo.as_ref() else {
            return counts;
        };
        let settings = match self.repository.get_settings().await {
            Ok(settings) => settings,
            Err(_) => return counts,
        };
        let server = match silo.server(&settings).await {
            Ok(Some(server)) => server,
            Ok(None) => return counts,
            Err(error) => {
                tracing::warn!(%error, "could not reach Silo to restore posters");
                return counts;
            }
        };
        let mut after: Option<(String, String, String)> = None;
        loop {
            let page = match self
                .repository
                .list_plex_states(
                    after.as_ref().map(|(title, connection, item)| {
                        (title.as_str(), connection.as_str(), item.as_str())
                    }),
                    OVERLAY_PASS_PAGE,
                )
                .await
            {
                Ok(page) => page,
                Err(error) => {
                    tracing::warn!(%error, "could not list Silo poster pushes");
                    return counts;
                }
            };
            let Some(last) = page.last() else {
                break;
            };
            after = Some((
                last.title_id.clone(),
                last.connection_id.clone(),
                last.provider_item_id.clone(),
            ));
            for state in page
                .iter()
                .filter(|state| state.connection_id == server.connection_id)
            {
                let active = settings.silo_push_enabled
                    && matches!(
                        self.repository.active_output_hash(&state.title_id).await,
                        Ok(Some(_))
                    );
                if active {
                    continue;
                }
                match self.restore_silo_one(silo, &server, state).await {
                    Ok(true) => counts.restored += 1,
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(title_id = %state.title_id, %error, "could not restore a Silo poster");
                        counts.failed += 1;
                    }
                }
            }
        }
        counts
    }

    async fn restore_silo_one(
        &self,
        silo: &SiloPush,
        server: &SiloServer,
        state: &PosterOverlayPlexState,
    ) -> AppResult<bool> {
        let mut changed = false;
        if let Some(item) = silo.client.item(server, &state.provider_item_id).await?
            && state.pushed_thumb.is_some()
            && item.poster_thumbhash == state.pushed_thumb
            && let Some(original) = self.engine.read_original(&state.title_id).await?
            && !self.engine.has_marker(&original)
        {
            silo.client
                .upload_poster(server, &state.provider_item_id, original)
                .await?;
            changed = true;
        }
        self.repository
            .delete_plex_state(
                &state.connection_id,
                &state.title_id,
                &state.provider_item_id,
            )
            .await?;
        Ok(changed)
    }
}

enum PushOutcome {
    Pushed,
    Unchanged,
    NotInSilo,
    ChangedInSilo,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(
        facet: &str,
        tmdb: Option<&str>,
        tvdb: Option<&str>,
        imdb: Option<&str>,
    ) -> PosterOverlayInputs {
        PosterOverlayInputs {
            facet: Some(facet.to_string()),
            tmdb_id: tmdb.map(str::to_string),
            tvdb_id: tvdb.map(str::to_string),
            imdb_id: imdb.map(str::to_string),
            ..PosterOverlayInputs::default()
        }
    }

    #[test]
    fn content_ids_follow_silos_anchor_order() {
        assert_eq!(
            silo_content_ids(&inputs(
                "movie",
                Some("603"),
                Some("169"),
                Some("tt0133093")
            )),
            ["movie-tmdb-603", "movie-imdb-tt0133093", "movie-tvdb-169"]
        );
        assert_eq!(
            silo_content_ids(&inputs("series", Some("1399"), Some("121361"), None)),
            ["series-tvdb-121361", "series-tmdb-1399"]
        );
        // Anime is a series to Silo.
        assert_eq!(
            silo_content_ids(&inputs("anime", None, Some("79824"), None)),
            ["series-tvdb-79824"]
        );
    }

    #[test]
    fn malformed_ids_are_skipped() {
        assert!(
            silo_content_ids(&inputs("movie", Some("abc"), Some(" "), Some("nm123"))).is_empty()
        );
        assert_eq!(
            silo_content_ids(&inputs("movie", None, None, Some(" TT0111161 "))),
            ["movie-imdb-tt0111161"]
        );
    }
}
