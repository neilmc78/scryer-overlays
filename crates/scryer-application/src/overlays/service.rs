use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use futures_util::StreamExt;
use scryer_domain::{AppPermission, Id, User};

use super::fields::{OverlayFields, OverlaySampleValues, blake3_hex, input_hash, template_version};
use super::plex::{PlexPush, PlexSyncCounts};
use super::ports::{
    MAX_OVERLAY_PARALLELISM, MIN_OVERLAY_RECONCILE_INTERVAL_SECONDS, PosterOverlayEngine,
    PosterOverlayLibraryConfig, PosterOverlayPreviewPoster, PosterOverlayPreviewRequest,
    PosterOverlayRenderRequest, PosterOverlayRepository, PosterOverlaySettings, PosterOverlayState,
    PosterOverlayStatusCounts, PosterOverlayTemplate, PosterOverlayVariant,
};
use crate::{
    AppError, AppResult, AppUseCase, ImageProxyKind, ImageProxySourceRecord,
    MediaServerConnectionRepository,
};

/// Keyset page size for library-wide passes.
pub(crate) const OVERLAY_PASS_PAGE: usize = 200;
/// Length of the `?v=` version token, matching the stored-image route.
const PRESENTED_VERSION_LEN: usize = 16;

/// Poster overlays: the store, the render engine, and the wake handle the
/// worker listens on. Absent in assemblies without an overlay data root.
#[derive(Clone)]
pub struct AppPosterOverlayServices {
    pub(crate) repository: Arc<dyn PosterOverlayRepository>,
    pub(crate) engine: Arc<dyn PosterOverlayEngine>,
    pub(crate) wake: Arc<tokio::sync::Notify>,
    /// The library-wide pass in progress, if any. Kept in memory so the
    /// settings page can follow a rebuild without querying the database.
    pass: Arc<Mutex<PosterOverlayPassProgress>>,
    /// Pushes rendered posters to Plex; absent where no client is wired.
    pub(crate) plex: Option<PlexPush>,
    /// Whether any library has overlays enabled. Lets the image route skip
    /// the overlay lookup entirely on installs that do not use overlays.
    /// Starts `true` so nothing is missed before the first refresh.
    any_library_enabled: Arc<AtomicBool>,
}

impl AppPosterOverlayServices {
    pub fn new(
        repository: Arc<dyn PosterOverlayRepository>,
        engine: Arc<dyn PosterOverlayEngine>,
    ) -> Self {
        Self {
            repository,
            engine,
            wake: Arc::new(tokio::sync::Notify::new()),
            pass: Arc::new(Mutex::new(PosterOverlayPassProgress::default())),
            plex: None,
            any_library_enabled: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Push rendered posters to the Plex servers in `connections` through
    /// `client`, when the overlay settings turn push on.
    pub fn with_plex(
        mut self,
        connections: Arc<dyn MediaServerConnectionRepository>,
        client: Arc<dyn super::ports::PosterOverlayPlexClient>,
    ) -> Self {
        self.plex = Some(PlexPush::new(connections, client));
        self
    }

    /// When Plex work held back by a maintenance window should run.
    pub(crate) fn plex_resume_at(&self) -> Option<std::time::Instant> {
        self.plex.as_ref().and_then(PlexPush::resume_at)
    }

    /// Clear the pending resume once a pass picks the deferred work up.
    pub(crate) fn take_plex_resume_at(&self) -> Option<std::time::Instant> {
        self.plex.as_ref().and_then(PlexPush::take_resume_at)
    }

    async fn plex_push_enabled(&self) -> bool {
        self.plex.is_some()
            && self
                .repository
                .get_settings()
                .await
                .is_ok_and(|settings| settings.plex_push_enabled)
    }

    /// Re-read whether any library has overlays enabled.
    pub async fn refresh_enabled_flag(&self) -> AppResult<()> {
        let any = self
            .repository
            .list_library_configs()
            .await?
            .iter()
            .any(|library| library.enabled);
        self.any_library_enabled.store(any, Ordering::Relaxed);
        Ok(())
    }

    /// Ask the worker for a library-wide pass.
    pub fn request_pass(&self) {
        self.update_pass(|pass| pass.queued = true);
        self.wake.notify_one();
    }

    /// The library-wide pass in progress or queued, if any.
    pub fn pass_progress(&self) -> PosterOverlayPassProgress {
        self.pass
            .lock()
            .map(|pass| pass.clone())
            .unwrap_or_default()
    }

    fn update_pass(&self, change: impl FnOnce(&mut PosterOverlayPassProgress)) {
        if let Ok(mut pass) = self.pass.lock() {
            change(&mut pass);
        }
    }

    /// The overlaid poster for an image-route variant, or `None` to serve
    /// the stored original.
    pub async fn image(
        &self,
        title_id: &str,
        variant: &str,
    ) -> AppResult<Option<PosterOverlayImage>> {
        if !self.any_library_enabled.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let Some(variant) = PosterOverlayVariant::from_route(variant) else {
            return Ok(None);
        };
        let Some(output_hash) = self.repository.active_output_hash(title_id).await? else {
            return Ok(None);
        };
        Ok(self
            .engine
            .read_output(title_id, variant)
            .await?
            .map(|bytes| PosterOverlayImage {
                bytes,
                etag: format!("overlay:{output_hash}"),
            }))
    }

    /// Bring one title's overlay up to date. Idempotent: a title whose
    /// `input_hash` is unchanged is not re-rendered. Failures are recorded
    /// on the title's state and the previous output keeps being served.
    /// When Plex push is on, the poster is then brought up to date on every
    /// Plex server the title is matched on.
    pub async fn process_title(&self, title_id: &str) -> PosterOverlayOutcome {
        let push = self.plex_push_enabled().await;
        self.process_title_and_push(title_id, push).await.0
    }

    async fn process_title_and_push(
        &self,
        title_id: &str,
        push: bool,
    ) -> (PosterOverlayOutcome, PlexSyncCounts) {
        let outcome = self.render_title(title_id).await;
        let plex = match outcome {
            PosterOverlayOutcome::Rendered { .. } | PosterOverlayOutcome::Unchanged if push => {
                self.sync_plex(title_id).await
            }
            _ => PlexSyncCounts::default(),
        };
        (outcome, plex)
    }

    async fn render_title(&self, title_id: &str) -> PosterOverlayOutcome {
        match process_title(self, title_id).await {
            Ok(outcome) => outcome,
            Err(error) => {
                let message = error.to_string();
                tracing::warn!(title_id, error = %message, "poster overlay render failed");
                let mut state = self
                    .repository
                    .get_state(title_id)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| PosterOverlayState {
                        title_id: title_id.to_string(),
                        ..PosterOverlayState::default()
                    });
                state.last_error = Some(message.clone());
                // The title may have been deleted between the failure and
                // this write; the foreign key then rejects it, which is fine.
                let _ = self.repository.save_state(&state).await;
                PosterOverlayOutcome::Failed(message)
            }
        }
    }

    /// Process titles with at most `parallelism` in flight.
    pub async fn process_titles(&self, title_ids: Vec<String>) -> PosterOverlayPassSummary {
        self.process_titles_tracked(title_ids, false).await
    }

    /// As `process_titles`; `track` counts each outcome into the running
    /// library-wide pass.
    async fn process_titles_tracked(
        &self,
        title_ids: Vec<String>,
        track: bool,
    ) -> PosterOverlayPassSummary {
        let settings = self.repository.get_settings().await.unwrap_or_default();
        let parallelism = settings.parallelism.clamp(1, MAX_OVERLAY_PARALLELISM);
        let push = self.plex.is_some() && settings.plex_push_enabled;
        let mut summary = PosterOverlayPassSummary::default();
        let mut outcomes = futures_util::stream::iter(title_ids)
            .map(|title_id| async move { self.process_title_and_push(&title_id, push).await })
            .buffer_unordered(parallelism);
        while let Some((outcome, plex)) = outcomes.next().await {
            summary.record(&outcome);
            summary.plex.merge(plex);
            if track {
                self.update_pass(|pass| {
                    pass.processed += 1;
                    match outcome {
                        PosterOverlayOutcome::Rendered { .. } => pass.rendered += 1,
                        PosterOverlayOutcome::Failed(_) => pass.failed += 1,
                        _ => {}
                    }
                });
            }
        }
        summary
    }

    /// Library-wide pass: every title in an enabled library is brought up to
    /// date, and titles whose library was disabled get their original poster
    /// URL back.
    pub async fn reconcile(&self) -> AppResult<PosterOverlayPassSummary> {
        let repository = &self.repository;
        // A request made from here on asks for another pass after this one.
        self.update_pass(|pass| pass.queued = false);
        let mut title_ids = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let page = repository
                .list_enabled_title_ids(after.as_deref(), OVERLAY_PASS_PAGE)
                .await?;
            let Some(last) = page.last().cloned() else {
                break;
            };
            title_ids.extend(page);
            after = Some(last);
        }

        let _running = RunningPass::start(self, title_ids.len());
        if !title_ids.is_empty() {
            tracing::info!(titles = title_ids.len(), "poster overlay reconcile started");
        }
        let mut summary = PosterOverlayPassSummary::default();
        for page in title_ids.chunks(OVERLAY_PASS_PAGE) {
            summary.merge(self.process_titles_tracked(page.to_vec(), true).await);
        }

        let mut after: Option<String> = None;
        loop {
            let page = repository
                .list_state_title_ids(after.as_deref(), OVERLAY_PASS_PAGE)
                .await?;
            let Some(last) = page.last().cloned() else {
                break;
            };
            for title_id in &page {
                if repository.active_output_hash(title_id).await?.is_some() {
                    continue;
                }
                if let Some(output_hash) = repository
                    .get_state(title_id)
                    .await?
                    .and_then(|state| state.output_hash)
                {
                    repository
                        .restore_presented_poster_version(title_id, presented_version(&output_hash))
                        .await?;
                }
            }
            after = Some(last);
        }

        let push = self.plex_push_enabled().await;
        summary
            .plex
            .merge(self.restore_inactive_plex_posters(push).await);

        if !title_ids.is_empty() {
            tracing::info!(
                rendered = summary.rendered,
                unchanged = summary.unchanged,
                failed = summary.failed,
                "poster overlay reconcile finished"
            );
        }
        let plex = summary.plex;
        if plex.pushed + plex.restored + plex.changed_in_plex + plex.deferred + plex.failed > 0 {
            tracing::info!(
                pushed = plex.pushed,
                restored = plex.restored,
                changed_in_plex = plex.changed_in_plex,
                deferred_for_maintenance = plex.deferred,
                failed = plex.failed,
                "poster overlay Plex sync finished"
            );
        }
        if summary.no_artwork > 0 {
            tracing::warn!(
                titles = summary.no_artwork,
                "poster overlays skipped titles with no poster artwork to draw on"
            );
        }
        Ok(summary)
    }

    /// Disable overlays in every library, point every poster back at its
    /// stored original, and remove rendered outputs and their state.
    /// Stored originals are kept.
    pub async fn revert_all(&self) -> AppResult<usize> {
        let repository = &self.repository;
        // Disable first so a concurrent pass cannot render behind the revert.
        for library in repository.list_library_configs().await? {
            if library.enabled {
                repository
                    .set_library_config(&library.library_id, false, library.template_id.as_deref())
                    .await?;
            }
        }
        self.refresh_enabled_flag().await?;
        let mut reverted = 0;
        let mut after: Option<String> = None;
        loop {
            let page = repository
                .list_state_title_ids(after.as_deref(), OVERLAY_PASS_PAGE)
                .await?;
            let Some(last) = page.last().cloned() else {
                break;
            };
            for title_id in &page {
                if let Some(state) = repository.get_state(title_id).await?
                    && let Some(output_hash) = state.output_hash.as_deref()
                {
                    repository
                        .restore_presented_poster_version(title_id, presented_version(output_hash))
                        .await?;
                }
                self.engine.remove_outputs(title_id).await?;
                repository.delete_state(title_id).await?;
                reverted += 1;
            }
            after = Some(last);
        }
        if reverted > 0 {
            tracing::info!(reverted, "reverted poster overlays to originals");
        }
        Ok(reverted)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PosterOverlayOutcome {
    Rendered {
        output_hash: String,
    },
    Unchanged,
    /// The title's library does not have overlays enabled.
    Skipped,
    /// The title has no poster artwork to draw on yet.
    NoArtwork,
    /// The title no longer exists; its outputs were removed.
    Removed,
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct PosterOverlayImage {
    pub bytes: Vec<u8>,
    pub etag: String,
}

#[derive(Clone, Debug)]
pub struct PosterOverlayOverview {
    pub settings: PosterOverlaySettings,
    pub libraries: Vec<PosterOverlayLibraryConfig>,
    pub templates: Vec<PosterOverlayTemplate>,
    pub counts: PosterOverlayStatusCounts,
    pub builtin_template: &'static str,
    pub pass: PosterOverlayPassProgress,
}

/// A template rendered for the editor.
#[derive(Clone, Debug)]
pub struct PosterOverlayPreview {
    pub jpeg: Vec<u8>,
    /// The library title drawn on; `None` for the neutral placeholder.
    pub poster: Option<PosterOverlayPreviewPoster>,
}

/// Which library poster a preview is drawn on.
#[derive(Clone, Debug, Default)]
pub struct PosterOverlayPreviewChoice {
    /// `movie`, `series` or `anime`; any title when absent.
    pub facet: Option<String>,
    /// Keep drawing on this title, so the poster does not change between
    /// edits. A random one from `facet` is used when absent or unusable.
    pub title_id: Option<String>,
}

/// The title whose poster an image-proxy source stands for, if any. Only
/// title posters carry overlays: movie entities, requests, people and
/// backgrounds are proxied unchanged.
pub fn overlay_title_for_proxy_source(source: &ImageProxySourceRecord) -> Option<&str> {
    (source.owner_type.as_deref() == Some("title")
        && source.image_kind == ImageProxyKind::Poster.as_str())
    .then_some(source.owner_id.as_deref())
    .flatten()
    .filter(|title_id| !title_id.is_empty())
}

/// A library-wide pass as the settings page shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PosterOverlayPassProgress {
    /// A pass was requested and has not started yet.
    pub queued: bool,
    pub running: bool,
    /// Titles the running pass covers.
    pub total: usize,
    /// Of those, titles checked so far.
    pub processed: usize,
    /// Of those, titles re-rendered so far.
    pub rendered: usize,
    pub failed: usize,
}

/// Marks a pass as running for as long as it lives, so a pass that ends
/// early on an error is not reported as still running.
struct RunningPass<'a>(&'a AppPosterOverlayServices);

impl<'a> RunningPass<'a> {
    fn start(services: &'a AppPosterOverlayServices, total: usize) -> Self {
        services.update_pass(|pass| {
            *pass = PosterOverlayPassProgress {
                queued: pass.queued,
                running: true,
                total,
                ..PosterOverlayPassProgress::default()
            };
        });
        Self(services)
    }
}

impl Drop for RunningPass<'_> {
    fn drop(&mut self) {
        self.0.update_pass(|pass| pass.running = false);
    }
}

/// Titles checked for a stored original to use as the preview background.
const PREVIEW_BACKGROUND_CANDIDATES: usize = 8;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PosterOverlayPassSummary {
    pub rendered: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub no_artwork: usize,
    pub failed: usize,
    /// What the pass did on Plex servers.
    pub plex: PlexSyncCounts,
}

impl PosterOverlayPassSummary {
    fn merge(&mut self, other: Self) {
        self.rendered += other.rendered;
        self.unchanged += other.unchanged;
        self.skipped += other.skipped;
        self.no_artwork += other.no_artwork;
        self.failed += other.failed;
        self.plex.merge(other.plex);
    }

    fn record(&mut self, outcome: &PosterOverlayOutcome) {
        match outcome {
            PosterOverlayOutcome::Rendered { .. } => self.rendered += 1,
            PosterOverlayOutcome::Unchanged => self.unchanged += 1,
            PosterOverlayOutcome::Skipped | PosterOverlayOutcome::Removed => self.skipped += 1,
            PosterOverlayOutcome::NoArtwork => self.no_artwork += 1,
            PosterOverlayOutcome::Failed(_) => self.failed += 1,
        }
    }
}

impl AppUseCase {
    pub(crate) fn poster_overlays(&self) -> Option<&AppPosterOverlayServices> {
        self.services.poster_overlays.as_ref()
    }

    fn require_poster_overlays(&self) -> AppResult<&AppPosterOverlayServices> {
        self.poster_overlays()
            .ok_or_else(|| AppError::Validation("poster overlays are not available".into()))
    }

    // ── Queries and mutations ──────────────────────────────────────────────

    pub async fn poster_overlay_overview(&self, actor: &User) -> AppResult<PosterOverlayOverview> {
        self.require_library_settings_read_permission(actor).await?;
        let overlays = self.require_poster_overlays()?;
        let repository = &overlays.repository;
        Ok(PosterOverlayOverview {
            settings: repository.get_settings().await?,
            libraries: repository.list_library_configs().await?,
            templates: repository.list_templates().await?,
            counts: repository.status_counts().await?,
            builtin_template: overlays.engine.builtin_template(),
            pass: overlays.pass_progress(),
        })
    }

    /// The library-wide pass in progress or queued. Reads no database, so
    /// the settings page can poll it during a rebuild.
    pub async fn poster_overlay_pass(&self, actor: &User) -> AppResult<PosterOverlayPassProgress> {
        self.require_library_settings_read_permission(actor).await?;
        Ok(self.require_poster_overlays()?.pass_progress())
    }

    pub async fn set_poster_overlay_library(
        &self,
        actor: &User,
        library_id: &str,
        enabled: bool,
        template_id: Option<&str>,
    ) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let overlays = self.require_poster_overlays()?;
        let libraries = overlays.repository.list_library_configs().await?;
        if !libraries
            .iter()
            .any(|library| library.library_id == library_id)
        {
            return Err(AppError::NotFound(format!("library {library_id}")));
        }
        if let Some(template_id) = template_id
            && overlays
                .repository
                .get_template(template_id)
                .await?
                .is_none()
        {
            return Err(AppError::NotFound(format!(
                "overlay template {template_id}"
            )));
        }
        overlays
            .repository
            .set_library_config(library_id, enabled, template_id)
            .await?;
        overlays.refresh_enabled_flag().await?;
        overlays.request_pass();
        Ok(())
    }

    /// Update render parallelism and the reconcile interval. The Phase 2
    /// Plex flag is not settable here and keeps its stored value.
    pub async fn update_poster_overlay_settings(
        &self,
        actor: &User,
        parallelism: usize,
        reconcile_interval_seconds: u64,
        plex_push_enabled: Option<bool>,
    ) -> AppResult<PosterOverlaySettings> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let overlays = self.require_poster_overlays()?;
        let previous = overlays.repository.get_settings().await?;
        let settings = PosterOverlaySettings {
            parallelism,
            reconcile_interval_seconds,
            plex_push_enabled: plex_push_enabled.unwrap_or(previous.plex_push_enabled),
        };
        if settings.parallelism == 0 || settings.parallelism > MAX_OVERLAY_PARALLELISM {
            return Err(AppError::Validation(format!(
                "overlay parallelism must be between 1 and {MAX_OVERLAY_PARALLELISM}"
            )));
        }
        if settings.reconcile_interval_seconds < MIN_OVERLAY_RECONCILE_INTERVAL_SECONDS {
            return Err(AppError::Validation(format!(
                "overlay reconcile interval must be at least {MIN_OVERLAY_RECONCILE_INTERVAL_SECONDS} seconds"
            )));
        }
        overlays.repository.save_settings(&settings).await?;
        overlays.engine.set_parallelism(settings.parallelism);
        // Turning push on uploads every poster; turning it off puts the
        // originals back. Either way a pass does the work.
        if previous.plex_push_enabled != settings.plex_push_enabled {
            overlays.request_pass();
        }
        Ok(settings)
    }

    pub async fn save_poster_overlay_template(
        &self,
        actor: &User,
        template_id: Option<&str>,
        name: &str,
        svg: &str,
    ) -> AppResult<PosterOverlayTemplate> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let overlays = self.require_poster_overlays()?;
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::Validation(
                "template name must not be empty".into(),
            ));
        }
        overlays.engine.validate_template(svg)?;
        let now = Utc::now();
        let existing = match template_id {
            Some(id) => Some(
                overlays
                    .repository
                    .get_template(id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("overlay template {id}")))?,
            ),
            None => None,
        };
        let template = PosterOverlayTemplate {
            id: existing
                .as_ref()
                .map(|template| template.id.clone())
                .unwrap_or_else(|| Id::new().0),
            name: name.to_string(),
            svg: svg.to_string(),
            content_hash: blake3_hex(svg.as_bytes()),
            created_at: existing
                .as_ref()
                .map(|template| template.created_at)
                .unwrap_or(now),
            updated_at: now,
        };
        overlays.repository.save_template(&template).await?;
        overlays.request_pass();
        Ok(template)
    }

    pub async fn delete_poster_overlay_template(
        &self,
        actor: &User,
        template_id: &str,
    ) -> AppResult<bool> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let overlays = self.require_poster_overlays()?;
        let deleted = overlays.repository.delete_template(template_id).await?;
        if deleted {
            overlays.request_pass();
        }
        Ok(deleted)
    }

    /// Checks a template without saving it.
    pub async fn validate_poster_overlay_template(&self, actor: &User, svg: &str) -> AppResult<()> {
        self.require_library_settings_read_permission(actor).await?;
        self.require_poster_overlays()?
            .engine
            .validate_template(svg)
    }

    /// Render a draft template with sample values, for the editor. Draws on
    /// a stored original from the library when one exists so contrast can
    /// be judged; nothing is written.
    pub async fn preview_poster_overlay_template(
        &self,
        actor: &User,
        svg: &str,
        sample: &OverlaySampleValues,
        choice: &PosterOverlayPreviewChoice,
    ) -> AppResult<PosterOverlayPreview> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let overlays = self.require_poster_overlays()?;
        overlays.engine.validate_template(svg)?;
        let fields = OverlayFields::from_sample(sample)
            .map_err(|message| AppError::Validation(format!("preview sample: {message}")))?;

        let facet = choice
            .facet
            .as_deref()
            .map(str::trim)
            .filter(|facet| !facet.is_empty());
        if let Some(facet) = facet
            && !matches!(facet, "movie" | "series" | "anime")
        {
            return Err(AppError::Validation(format!(
                "preview facet must be movie, series or anime, not \"{facet}\""
            )));
        }
        let mut background = None;
        let mut poster = None;
        for candidate in overlays
            .repository
            .preview_posters(
                facet,
                choice.title_id.as_deref(),
                PREVIEW_BACKGROUND_CANDIDATES,
            )
            .await?
        {
            if let Some(original) = overlays.engine.read_original(&candidate.title_id).await? {
                background = Some(original);
                poster = Some(candidate);
                break;
            }
        }
        // Ratings come from the title behind the preview, so a ratings badge
        // shows real scores; the placeholder poster gets sample ones.
        let ratings = match &poster {
            Some(poster) => overlays
                .repository
                .load_inputs(&poster.title_id)
                .await?
                .map(|inputs| inputs.ratings)
                .unwrap_or_default(),
            None => sample_ratings(),
        };
        let values = fields.with_ratings(&ratings).template_values();
        let jpeg = overlays
            .engine
            .render_preview(PosterOverlayPreviewRequest {
                background,
                template_svg: svg.to_string(),
                values,
            })
            .await?;
        Ok(PosterOverlayPreview { jpeg, poster })
    }

    /// Queue a library-wide pass. Unchanged posters are skipped by hash.
    pub async fn request_poster_overlay_rebuild(&self, actor: &User) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        self.require_poster_overlays()?.wake.notify_one();
        Ok(())
    }

    /// Disable overlays everywhere and serve every original again. Rendered
    /// outputs and their state are removed; stored originals are kept.
    pub async fn revert_all_poster_overlays(&self, actor: &User) -> AppResult<usize> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let overlays = self.require_poster_overlays()?;
        let reverted = overlays.revert_all().await?;
        // Every library is disabled now, so every poster pushed to Plex is
        // inactive and gets its original back.
        overlays.restore_inactive_plex_posters(false).await;
        Ok(reverted)
    }

    /// The overlaid poster for the image route, or `None` to serve the stored
    /// original.
    pub async fn poster_overlay_image(
        &self,
        title_id: &str,
        variant: &str,
    ) -> AppResult<Option<PosterOverlayImage>> {
        match self.poster_overlays() {
            Some(overlays) => overlays.image(title_id, variant).await,
            None => Ok(None),
        }
    }

    /// The overlaid poster behind a `/images/media/{token}` URL. Title posters
    /// reach the client through the image proxy, keyed by a token that
    /// records the owning title; when that title has an overlay it is served
    /// in place of the proxied original.
    pub async fn poster_overlay_image_for_media_token(
        &self,
        token: &str,
        variant: &str,
    ) -> AppResult<Option<PosterOverlayImage>> {
        let Some(overlays) = self.poster_overlays() else {
            return Ok(None);
        };
        // Installs without overlays never pay for the token lookup.
        if !overlays.any_library_enabled.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let Some(source) = self
            .image_proxy_repository()
            .get_image_proxy_source(token)
            .await?
        else {
            return Ok(None);
        };
        match overlay_title_for_proxy_source(&source) {
            Some(title_id) => overlays.image(title_id, variant).await,
            None => Ok(None),
        }
    }

    pub async fn reconcile_poster_overlays(&self) -> AppResult<PosterOverlayPassSummary> {
        self.require_poster_overlays()?.reconcile().await
    }
}

/// Scores for previewing a ratings badge when no library poster is available.
fn sample_ratings() -> Vec<crate::TitleExternalRating> {
    [
        ("imdb", 7.8),
        ("rottentomatoes", 91.0),
        ("popcornmeter", 86.0),
        ("metacritic", 74.0),
        ("metacriticuser", 7.9),
        ("letterboxd", 3.9),
        ("tmdb", 7.6),
        ("trakt", 80.0),
        ("mdblist", 82.0),
    ]
    .into_iter()
    .map(|(source, value)| crate::TitleExternalRating {
        source: source.to_string(),
        value: Some(value),
        score: None,
        normalized: 0.0,
        votes: None,
        url: String::new(),
    })
    .collect()
}

/// The template a library uses: its own, or the built-in one when it has
/// none or the template was deleted.
pub(crate) async fn template_svg_for(
    overlays: &AppPosterOverlayServices,
    template_id: Option<&str>,
) -> AppResult<String> {
    Ok(match template_id {
        Some(template_id) => match overlays.repository.get_template(template_id).await? {
            Some(template) => template.svg,
            None => overlays.engine.builtin_template().to_string(),
        },
        None => overlays.engine.builtin_template().to_string(),
    })
}

pub(crate) fn presented_version(output_hash: &str) -> &str {
    let end = output_hash
        .char_indices()
        .nth(PRESENTED_VERSION_LEN)
        .map_or(output_hash.len(), |(index, _)| index);
    &output_hash[..end]
}

async fn process_title(
    overlays: &AppPosterOverlayServices,
    title_id: &str,
) -> AppResult<PosterOverlayOutcome> {
    let repository = &overlays.repository;
    let engine = &overlays.engine;

    let Some(inputs) = repository.load_inputs(title_id).await? else {
        // The title is gone; its state row went with it by cascade. Only the
        // rendered outputs remain to clear.
        engine.remove_outputs(title_id).await?;
        return Ok(PosterOverlayOutcome::Removed);
    };
    if !inputs.overlay_enabled {
        return Ok(PosterOverlayOutcome::Skipped);
    }
    let (Some(source_url), Some(source_identity)) =
        (inputs.poster_source_url.clone(), inputs.source_identity())
    else {
        return Ok(PosterOverlayOutcome::NoArtwork);
    };

    let mut state = repository
        .get_state(title_id)
        .await?
        .unwrap_or_else(|| PosterOverlayState {
            title_id: title_id.to_string(),
            ..PosterOverlayState::default()
        });

    // Resolve the pristine original. A stored original is trusted only while
    // the upstream artwork is the one it came from and it carries no marker.
    let stored = engine
        .read_original(title_id)
        .await?
        .filter(|bytes| !engine.has_marker(bytes));
    let original = match stored {
        Some(bytes) if state.source_identity.as_deref() == Some(source_identity.as_str()) => bytes,
        _ => {
            let fetched = engine.fetch_source(&source_url).await?;
            if engine.has_marker(&fetched) {
                return Err(AppError::Validation(
                    "the upstream poster already carries a Scryer overlay marker and cannot be used as an original"
                        .into(),
                ));
            }
            let path = engine.store_original(title_id, fetched.clone()).await?;
            state.original_path = Some(path);
            state.source_identity = Some(source_identity);
            fetched
        }
    };
    let original_hash = blake3_hex(&original);
    state.original_hash = Some(original_hash.clone());

    let template_svg = template_svg_for(overlays, inputs.template_id.as_deref()).await?;
    let version = template_version(&template_svg);
    let fields = OverlayFields::aggregate(&inputs.files)
        .with_title(inputs.facet.as_deref(), inputs.content_status.as_deref())
        .with_ratings(&inputs.ratings);
    let input = input_hash(&original_hash, &version, &fields);

    let outputs_present = engine
        .read_output(title_id, PosterOverlayVariant::W70)
        .await?
        .is_some();
    if outputs_present
        && state.output_hash.is_some()
        && state.input_hash.as_deref() == Some(input.as_str())
    {
        if state.last_error.is_some() {
            state.last_error = None;
            repository.save_state(&state).await?;
        }
        return Ok(PosterOverlayOutcome::Unchanged);
    }

    let values = fields.template_values();
    let fields_json = serde_json::to_string(&values).ok();
    let rendered = engine
        .render(PosterOverlayRenderRequest {
            original,
            template_svg,
            values,
            input_hash: input.clone(),
        })
        .await?;
    engine.write_outputs(title_id, &rendered).await?;

    state.input_hash = Some(input);
    state.output_hash = Some(rendered.output_hash.clone());
    state.template_version = Some(version);
    state.fields_json = fields_json;
    state.rendered_at = Some(Utc::now());
    state.last_error = None;
    repository.save_state(&state).await?;
    repository
        .set_presented_poster_version(title_id, presented_version(&rendered.output_hash))
        .await?;
    Ok(PosterOverlayOutcome::Rendered {
        output_hash: rendered.output_hash,
    })
}
