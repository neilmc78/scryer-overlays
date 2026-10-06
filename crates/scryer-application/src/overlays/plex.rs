//! Pushing rendered overlays to Plex.
//!
//! Each title in an overlay-enabled library is uploaded as the poster of the
//! Plex item a catalog scan matched it to (`media_server_playback_items`),
//! and the poster field is locked so Plex's metadata refreshes keep it.
//! A poster is uploaded again only when the rendered output changes.
//!
//! Nothing is changed in Plex during its scheduled maintenance hours: the
//! work is deferred and a pass runs again when the window ends.
//!
//! Plex is never asked to delete anything. A poster someone changed in Plex
//! after Scryer pushed one is left alone and reported. When push is turned
//! off, or a title's library no longer has overlays, the stored original is
//! uploaded back and the poster field unlocked.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{Local, NaiveTime, Timelike, Utc};
use scryer_domain::{MediaServerConnection, MediaServerPlaybackEntityKind, MediaServerProvider};

use super::fields::{
    OverlayFields, OverlayMediaFacts, blake3_hex, edition_token, input_hash, template_version,
};
use super::ports::{
    PlexMaintenanceWindow, PlexPosterItem, PosterOverlayPlexClient, PosterOverlayPlexState,
    PosterOverlayRenderRequest, PosterOverlayVariant,
};
use super::service::{AppPosterOverlayServices, OVERLAY_PASS_PAGE, template_svg_for};
use crate::{AppError, AppResult, MediaServerConnectionRepository};

/// Recorded on a title whose poster was replaced in Plex after Scryer
/// pushed one; Scryer leaves that poster alone.
pub const PLEX_POSTER_CHANGED_IN_PLEX: &str = "the poster was changed in Plex; left as it is";

/// How long the list of Plex connections is reused within a pass.
const CONNECTION_CACHE_TTL: Duration = Duration::from_secs(60);
/// How long a server's maintenance hours are reused before asking again.
const MAINTENANCE_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
/// Margin after the maintenance window before work resumes.
const MAINTENANCE_RESUME_MARGIN: Duration = Duration::from_secs(60);

type MaintenanceCache = HashMap<String, (Instant, Option<PlexMaintenanceWindow>)>;

/// Plex's account service. A connection still pointing here has no server
/// selected, so there is nothing to push to.
const PLEX_SERVICE_HOST: &str = "plex.tv";

#[derive(Clone)]
pub(crate) struct PlexPush {
    connections: Arc<dyn MediaServerConnectionRepository>,
    client: Arc<dyn PosterOverlayPlexClient>,
    cache: Arc<Mutex<Option<(Instant, Vec<MediaServerConnection>)>>>,
    maintenance: Arc<Mutex<MaintenanceCache>>,
    /// When deferred work should run: the end of the earliest maintenance
    /// window that held work back.
    resume_at: Arc<Mutex<Option<Instant>>>,
}

impl PlexPush {
    pub(crate) fn new(
        connections: Arc<dyn MediaServerConnectionRepository>,
        client: Arc<dyn PosterOverlayPlexClient>,
    ) -> Self {
        Self {
            connections,
            client,
            cache: Arc::new(Mutex::new(None)),
            maintenance: Arc::new(Mutex::new(HashMap::new())),
            resume_at: Arc::new(Mutex::new(None)),
        }
    }

    /// Whether `connection` is inside its maintenance hours now. If it is,
    /// the end of the window is remembered so the worker can run a pass
    /// then. A server whose hours cannot be read is treated as available.
    async fn in_maintenance(&self, connection: &MediaServerConnection) -> bool {
        let cached = self
            .maintenance
            .lock()
            .ok()
            .and_then(|cache| cache.get(&connection.id).copied())
            .filter(|(at, _)| at.elapsed() < MAINTENANCE_CACHE_TTL)
            .map(|(_, window)| window);
        let window = match cached {
            Some(window) => window,
            None => {
                let window = match self.client.maintenance_window(connection).await {
                    Ok(window) => window,
                    Err(error) => {
                        tracing::debug!(
                            connection_id = %connection.id,
                            %error,
                            "could not read Plex maintenance hours"
                        );
                        None
                    }
                };
                if let Ok(mut cache) = self.maintenance.lock() {
                    cache.insert(connection.id.clone(), (Instant::now(), window));
                }
                window
            }
        };
        let Some(remaining) =
            window.and_then(|window| maintenance_remaining(window, Local::now().time()))
        else {
            return false;
        };
        let resume = Instant::now() + remaining + MAINTENANCE_RESUME_MARGIN;
        if let Ok(mut resume_at) = self.resume_at.lock() {
            *resume_at = Some(resume_at.map_or(resume, |current| current.min(resume)));
        }
        true
    }
}

/// Time left in `window` at local time `now`, or `None` outside it. Plex
/// keeps its maintenance hours in the server's local time; this uses
/// Scryer's, so both should run in the same time zone.
pub fn maintenance_remaining(window: PlexMaintenanceWindow, now: NaiveTime) -> Option<Duration> {
    let start = window.start_hour % 24;
    let end = window.end_hour % 24;
    if start == end {
        return None;
    }
    let hour = now.hour();
    let inside = if start < end {
        (start..end).contains(&hour)
    } else {
        hour >= start || hour < end
    };
    if !inside {
        return None;
    }
    let now_seconds = i64::from(now.num_seconds_from_midnight());
    let end_seconds = i64::from(end) * 3600;
    let left = (end_seconds - now_seconds).rem_euclid(24 * 3600);
    Some(Duration::from_secs(left.unsigned_abs()))
}

impl PlexPush {
    /// Take the time deferred work should run, if any.
    pub(crate) fn take_resume_at(&self) -> Option<Instant> {
        self.resume_at
            .lock()
            .ok()
            .and_then(|mut resume| resume.take())
    }

    /// The time deferred work should run, without taking it.
    pub(crate) fn resume_at(&self) -> Option<Instant> {
        self.resume_at.lock().ok().and_then(|resume| *resume)
    }

    /// Enabled Plex connections with a selected server and a token.
    async fn connections(&self) -> AppResult<Vec<MediaServerConnection>> {
        if let Ok(cache) = self.cache.lock()
            && let Some((at, connections)) = cache.as_ref()
            && at.elapsed() < CONNECTION_CACHE_TTL
        {
            return Ok(connections.clone());
        }
        let connections = self
            .connections
            .list(Some(MediaServerProvider::Plex))
            .await?
            .into_iter()
            .filter(is_pushable)
            .collect::<Vec<_>>();
        if let Ok(mut cache) = self.cache.lock() {
            *cache = Some((Instant::now(), connections.clone()));
        }
        Ok(connections)
    }
}

fn is_pushable(connection: &MediaServerConnection) -> bool {
    connection.enabled
        && connection.provider == MediaServerProvider::Plex
        && connection
            .api_key
            .as_deref()
            .is_some_and(|key| !key.trim().is_empty())
        && url::Url::parse(&connection.base_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
            .is_some_and(|host| host != PLEX_SERVICE_HOST && !host.ends_with(".plex.tv"))
}

/// What Plex pushes did for one title, or across a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlexSyncCounts {
    pub pushed: usize,
    pub unchanged: usize,
    /// Posters changed in Plex after a push, left alone.
    pub changed_in_plex: usize,
    /// Originals put back because push is off or overlays were disabled.
    pub restored: usize,
    /// Changes held back because the server was in its maintenance hours.
    pub deferred: usize,
    pub failed: usize,
}

impl PlexSyncCounts {
    pub fn merge(&mut self, other: Self) {
        self.pushed += other.pushed;
        self.unchanged += other.unchanged;
        self.changed_in_plex += other.changed_in_plex;
        self.restored += other.restored;
        self.deferred += other.deferred;
        self.failed += other.failed;
    }
}

enum PushOutcome {
    Pushed,
    Unchanged,
    ChangedInPlex,
}

/// What a push uploads.
enum PosterSource<'a> {
    /// The title's rendered overlay.
    Rendered(&'a [u8]),
    /// One version of a movie, rendered only when it is uploaded.
    Version(PosterOverlayRenderRequest),
}

/// One Plex item a title's poster goes to on a connection.
struct PushTarget<'a> {
    rating_key: String,
    /// Identity of the poster: compared with what was last uploaded.
    change_key: String,
    source: PosterSource<'a>,
}

/// What rendering a movie's versions needs, read once per title.
struct VersionContext {
    files: Vec<OverlayMediaFacts>,
    facet: Option<String>,
    content_status: Option<String>,
    ratings: Vec<crate::TitleExternalRating>,
    title_name: String,
    tmdb_id: String,
    original: Vec<u8>,
    original_hash: String,
    template_svg: String,
    template_version: String,
}

/// The file name in a path, whichever separator it uses.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

impl AppPosterOverlayServices {
    /// Bring the title's poster on every Plex server up to date with its
    /// rendered overlay. A movie Plex shows as several versions gets one
    /// poster per version instead. Failures are recorded on the push records
    /// and never fail the render.
    pub(crate) async fn sync_plex(&self, title_id: &str) -> PlexSyncCounts {
        let mut counts = PlexSyncCounts::default();
        let Some(plex) = self.plex.as_ref() else {
            return counts;
        };
        let output_hash = match self.repository.get_state(title_id).await {
            Ok(Some(state)) => state.output_hash,
            _ => None,
        };
        let Some(output_hash) = output_hash else {
            return counts;
        };
        let poster = match self
            .engine
            .read_output(title_id, PosterOverlayVariant::Full)
            .await
        {
            Ok(Some(bytes)) => bytes,
            _ => return counts,
        };
        let (connections, mappings) = match self.plex_targets(plex, title_id).await {
            Ok(targets) => targets,
            Err(error) => {
                tracing::warn!(title_id, %error, "could not read Plex connections for a poster push");
                counts.failed += 1;
                return counts;
            }
        };
        // Read only for a title Plex has no single item for.
        let mut versions: Option<Option<VersionContext>> = None;
        for connection in &connections {
            if plex.in_maintenance(connection).await {
                counts.deferred += 1;
                continue;
            }
            let mapped = mappings
                .iter()
                .find(|(connection_id, _)| connection_id == &connection.id)
                .map(|(_, key)| key.clone());
            let targets = match mapped {
                Some(rating_key) => vec![PushTarget {
                    rating_key,
                    change_key: output_hash.clone(),
                    source: PosterSource::Rendered(&poster),
                }],
                None => {
                    if versions.is_none() {
                        versions = Some(match self.version_context(title_id).await {
                            Ok(context) => context,
                            Err(error) => {
                                tracing::warn!(title_id, %error, "could not read a title's versions for a Plex push");
                                None
                            }
                        });
                    }
                    let Some(context) = versions.as_ref().and_then(Option::as_ref) else {
                        continue;
                    };
                    match self.version_targets(plex, connection, context).await {
                        Ok(targets) => targets,
                        Err(error) => {
                            tracing::warn!(
                                title_id,
                                connection_id = %connection.id,
                                %error,
                                "could not find a movie's versions in Plex"
                            );
                            counts.failed += 1;
                            continue;
                        }
                    }
                }
            };
            if targets.is_empty() {
                continue;
            }
            let current = targets
                .iter()
                .map(|target| target.rating_key.clone())
                .collect::<HashSet<_>>();
            for target in targets {
                let rating_key = target.rating_key.clone();
                match self.push_one(plex, connection, title_id, target).await {
                    Ok(PushOutcome::Pushed) => counts.pushed += 1,
                    Ok(PushOutcome::Unchanged) => counts.unchanged += 1,
                    Ok(PushOutcome::ChangedInPlex) => counts.changed_in_plex += 1,
                    Err(error) => {
                        let message = error.to_string();
                        tracing::warn!(
                            title_id,
                            connection_id = %connection.id,
                            error = %message,
                            "poster overlay push to Plex failed"
                        );
                        let _ = self
                            .repository
                            .save_plex_state(&PosterOverlayPlexState {
                                connection_id: connection.id.clone(),
                                title_id: title_id.to_string(),
                                provider_item_id: rating_key.clone(),
                                last_error: Some(message),
                                ..self
                                    .repository
                                    .get_plex_state(&connection.id, title_id, &rating_key)
                                    .await
                                    .ok()
                                    .flatten()
                                    .unwrap_or_default()
                            })
                            .await;
                        counts.failed += 1;
                    }
                }
            }
            counts.merge(
                self.restore_superseded(plex, connection, title_id, &current)
                    .await,
            );
        }
        counts
    }

    /// Pushable connections, and the Plex item each one maps the title to.
    async fn plex_targets(
        &self,
        plex: &PlexPush,
        title_id: &str,
    ) -> AppResult<(Vec<MediaServerConnection>, Vec<(String, String)>)> {
        let connections = plex.connections().await?;
        if connections.is_empty() {
            return Ok((connections, Vec::new()));
        }
        let mappings = plex
            .connections
            .list_playback_items_for_entity(MediaServerPlaybackEntityKind::Title, title_id)
            .await?
            .into_iter()
            .map(|item| (item.connection_id, item.provider_item_id))
            .collect();
        Ok((connections, mappings))
    }

    /// What rendering the title's versions needs, or `None` when the title
    /// is not a movie with files in two or more editions.
    async fn version_context(&self, title_id: &str) -> AppResult<Option<VersionContext>> {
        let Some(inputs) = self.repository.load_inputs(title_id).await? else {
            return Ok(None);
        };
        let editions = inputs
            .files
            .iter()
            .filter_map(OverlayMediaFacts::edition_name)
            .map(|edition| edition_token(&edition))
            .filter(|token| !token.is_empty())
            .collect::<HashSet<_>>();
        let (Some(title_name), Some(tmdb_id)) = (inputs.title_name.clone(), inputs.tmdb_id.clone())
        else {
            return Ok(None);
        };
        if editions.len() < 2 || inputs.facet.as_deref() != Some("movie") {
            return Ok(None);
        }
        let Some(original) = self
            .engine
            .read_original(title_id)
            .await?
            .filter(|bytes| !self.engine.has_marker(bytes))
        else {
            return Ok(None);
        };
        let template_svg = template_svg_for(self, inputs.template_id.as_deref()).await?;
        Ok(Some(VersionContext {
            original_hash: blake3_hex(&original),
            template_version: template_version(&template_svg),
            template_svg,
            original,
            files: inputs.files,
            facet: inputs.facet,
            content_status: inputs.content_status,
            ratings: inputs.ratings,
            title_name,
            tmdb_id,
        }))
    }

    /// One target per version of the movie on this connection, each built
    /// from only that version's files. A version is matched to Scryer's
    /// files by file name, else by its edition name.
    async fn version_targets(
        &self,
        plex: &PlexPush,
        connection: &MediaServerConnection,
        context: &VersionContext,
    ) -> AppResult<Vec<PushTarget<'static>>> {
        let versions = plex
            .client
            .find_movie_versions(connection, &context.title_name, &context.tmdb_id)
            .await?;
        // A single item is the core match's to find.
        if versions.len() < 2 {
            return Ok(Vec::new());
        }
        let mut targets = Vec::new();
        for version in versions {
            let names = version
                .files
                .iter()
                .map(|path| file_name(path))
                .collect::<HashSet<_>>();
            let mut files = context
                .files
                .iter()
                .filter(|facts| {
                    facts
                        .file_path
                        .as_deref()
                        .is_some_and(|path| names.contains(file_name(path)))
                })
                .cloned()
                .collect::<Vec<_>>();
            if files.is_empty()
                && let Some(edition) = version.edition_title.as_deref()
            {
                let wanted = edition_token(edition);
                files = context
                    .files
                    .iter()
                    .filter(|facts| {
                        !wanted.is_empty()
                            && facts.edition_name().map(|name| edition_token(&name))
                                == Some(wanted.clone())
                    })
                    .cloned()
                    .collect();
            }
            if files.is_empty() {
                continue;
            }
            for facts in &mut files {
                facts.additional = false;
            }
            let fields = OverlayFields::aggregate(&files)
                .with_title(context.facet.as_deref(), context.content_status.as_deref())
                .with_ratings(&context.ratings);
            let input = input_hash(&context.original_hash, &context.template_version, &fields);
            targets.push(PushTarget {
                rating_key: version.item.rating_key,
                change_key: input.clone(),
                source: PosterSource::Version(PosterOverlayRenderRequest {
                    original: context.original.clone(),
                    template_svg: context.template_svg.clone(),
                    values: fields.template_values(),
                    input_hash: input,
                }),
            });
        }
        Ok(targets)
    }

    async fn push_one(
        &self,
        plex: &PlexPush,
        connection: &MediaServerConnection,
        title_id: &str,
        target: PushTarget<'_>,
    ) -> AppResult<PushOutcome> {
        let rating_key = target.rating_key.as_str();
        let state = self
            .repository
            .get_plex_state(&connection.id, title_id, rating_key)
            .await?;
        let Some(item) = plex.client.item(connection, rating_key).await? else {
            // The item is gone from Plex; the next catalog scan or version
            // lookup finds its replacement.
            if state.is_some() {
                self.repository
                    .delete_plex_state(&connection.id, title_id, rating_key)
                    .await?;
            }
            return Ok(PushOutcome::Unchanged);
        };

        if let Some(state) = state.as_ref().filter(|state| state.pushed_thumb.is_some()) {
            if item.thumb == state.pushed_thumb {
                if state.pushed_output_hash.as_deref() == Some(target.change_key.as_str()) {
                    if state.last_error.is_some() {
                        self.repository
                            .save_plex_state(&PosterOverlayPlexState {
                                last_error: None,
                                ..state.clone()
                            })
                            .await?;
                    }
                    return Ok(PushOutcome::Unchanged);
                }
            } else if !self.plex_shows_our_poster(plex, connection, &item).await? {
                // Someone chose another poster in Plex after our upload.
                if state.last_error.as_deref() != Some(PLEX_POSTER_CHANGED_IN_PLEX) {
                    self.repository
                        .save_plex_state(&PosterOverlayPlexState {
                            last_error: Some(PLEX_POSTER_CHANGED_IN_PLEX.to_string()),
                            ..state.clone()
                        })
                        .await?;
                }
                return Ok(PushOutcome::ChangedInPlex);
            }
        }

        let poster = match target.source {
            PosterSource::Rendered(bytes) => bytes.to_vec(),
            PosterSource::Version(request) => self
                .engine
                .render(request)
                .await?
                .variants
                .into_iter()
                .find(|(variant, _)| *variant == PosterOverlayVariant::Full)
                .map(|(_, bytes)| bytes)
                .ok_or_else(|| AppError::Repository("the version poster did not render".into()))?,
        };
        plex.client.upload_poster(connection, &item, poster).await?;
        plex.client
            .set_poster_locked(connection, &item, true)
            .await?;
        let thumb = plex
            .client
            .item(connection, rating_key)
            .await?
            .and_then(|item| item.thumb);
        self.repository
            .save_plex_state(&PosterOverlayPlexState {
                connection_id: connection.id.clone(),
                title_id: title_id.to_string(),
                provider_item_id: rating_key.to_string(),
                pushed_output_hash: Some(target.change_key),
                pushed_thumb: thumb,
                pushed_at: Some(Utc::now()),
                last_error: None,
            })
            .await?;
        Ok(PushOutcome::Pushed)
    }

    /// Put the original back on items of this title that are no longer its
    /// targets on `connection`, such as the single item a movie had before
    /// Plex split it into versions.
    async fn restore_superseded(
        &self,
        plex: &PlexPush,
        connection: &MediaServerConnection,
        title_id: &str,
        current: &HashSet<String>,
    ) -> PlexSyncCounts {
        let mut counts = PlexSyncCounts::default();
        let states = match self.repository.list_plex_states_for_title(title_id).await {
            Ok(states) => states,
            Err(error) => {
                tracing::warn!(title_id, %error, "could not list a title's Plex pushes");
                return counts;
            }
        };
        for state in states.iter().filter(|state| {
            state.connection_id == connection.id && !current.contains(&state.provider_item_id)
        }) {
            match self.restore_one(plex, connection, state).await {
                Ok(true) => counts.restored += 1,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(title_id, %error, "could not restore a superseded Plex poster");
                    counts.failed += 1;
                }
            }
        }
        counts
    }

    /// Whether the poster Plex shows now carries Scryer's marker.
    async fn plex_shows_our_poster(
        &self,
        plex: &PlexPush,
        connection: &MediaServerConnection,
        item: &PlexPosterItem,
    ) -> AppResult<bool> {
        Ok(plex
            .client
            .current_poster(connection, item)
            .await?
            .is_some_and(|bytes| self.engine.has_marker(&bytes)))
    }

    /// Put originals back on every Plex item whose title should no longer
    /// carry a pushed overlay: push is off, or the title's library no longer
    /// has overlays. Each record is processed once and then removed.
    pub(crate) async fn restore_inactive_plex_posters(&self, push_enabled: bool) -> PlexSyncCounts {
        let mut counts = PlexSyncCounts::default();
        let Some(plex) = self.plex.as_ref() else {
            return counts;
        };
        let connections = match plex.connections().await {
            Ok(connections) => connections,
            Err(error) => {
                tracing::warn!(%error, "could not read Plex connections to restore posters");
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
                    tracing::warn!(%error, "could not list Plex poster pushes");
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
            for state in &page {
                let active = push_enabled
                    && matches!(
                        self.repository.active_output_hash(&state.title_id).await,
                        Ok(Some(_))
                    );
                if active {
                    continue;
                }
                // A connection that was disabled or removed cannot be reached;
                // its records go with it when it is deleted.
                let Some(connection) = connections
                    .iter()
                    .find(|connection| connection.id == state.connection_id)
                else {
                    continue;
                };
                if plex.in_maintenance(connection).await {
                    counts.deferred += 1;
                    continue;
                }
                match self.restore_one(plex, connection, state).await {
                    Ok(true) => counts.restored += 1,
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(
                            title_id = %state.title_id,
                            connection_id = %state.connection_id,
                            %error,
                            "could not restore a Plex poster"
                        );
                        let _ = self
                            .repository
                            .save_plex_state(&PosterOverlayPlexState {
                                last_error: Some(error.to_string()),
                                ..state.clone()
                            })
                            .await;
                        counts.failed += 1;
                    }
                }
            }
        }
        counts
    }

    /// Upload the stored original over a poster Scryer pushed and unlock the
    /// field. A poster someone changed in Plex is left as it is. Returns
    /// whether Plex was changed.
    async fn restore_one(
        &self,
        plex: &PlexPush,
        connection: &MediaServerConnection,
        state: &PosterOverlayPlexState,
    ) -> AppResult<bool> {
        let mut changed = false;
        if let Some(item) = plex
            .client
            .item(connection, &state.provider_item_id)
            .await?
            && state.pushed_output_hash.is_some()
            && self.plex_shows_our_poster(plex, connection, &item).await?
        {
            if let Some(original) = self.engine.read_original(&state.title_id).await? {
                plex.client
                    .upload_poster(connection, &item, original)
                    .await?;
            }
            plex.client
                .set_poster_locked(connection, &item, false)
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
