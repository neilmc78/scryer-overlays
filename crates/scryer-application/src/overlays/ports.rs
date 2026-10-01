use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::fields::OverlayMediaFacts;
use crate::AppResult;

pub const DEFAULT_OVERLAY_PARALLELISM: usize = 3;
pub const MAX_OVERLAY_PARALLELISM: usize = 16;
pub const DEFAULT_OVERLAY_RECONCILE_INTERVAL_SECONDS: u64 = 6 * 60 * 60;
pub const MIN_OVERLAY_RECONCILE_INTERVAL_SECONDS: u64 = 15 * 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PosterOverlaySettings {
    /// Upper bound on concurrent renders, and the size of the render pool.
    pub parallelism: usize,
    pub reconcile_interval_seconds: u64,
    /// Phase 2: push rendered posters to Plex. Stored now so the setting has
    /// a home; nothing reads it yet.
    pub plex_push_enabled: bool,
}

impl Default for PosterOverlaySettings {
    fn default() -> Self {
        Self {
            parallelism: DEFAULT_OVERLAY_PARALLELISM,
            reconcile_interval_seconds: DEFAULT_OVERLAY_RECONCILE_INTERVAL_SECONDS,
            plex_push_enabled: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PosterOverlayLibraryConfig {
    pub library_id: String,
    pub library_name: String,
    pub facet: String,
    pub enabled: bool,
    /// `None` selects the built-in template.
    pub template_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PosterOverlayTemplate {
    pub id: String,
    pub name: String,
    pub svg: String,
    pub content_hash: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PosterOverlayState {
    pub title_id: String,
    pub original_path: Option<String>,
    pub original_hash: Option<String>,
    pub source_identity: Option<String>,
    pub input_hash: Option<String>,
    pub output_hash: Option<String>,
    pub template_version: Option<String>,
    pub fields_json: Option<String>,
    pub rendered_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

/// Everything the pipeline reads about one title, in one round trip.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PosterOverlayInputs {
    pub title_id: String,
    pub library_id: Option<String>,
    pub overlay_enabled: bool,
    pub template_id: Option<String>,
    /// The poster Scryer currently presents, from `title_images`.
    pub poster_source_url: Option<String>,
    pub poster_source_etag: Option<String>,
    pub files: Vec<OverlayMediaFacts>,
}

impl PosterOverlayInputs {
    /// Identity of the upstream artwork: a change here means new artwork.
    pub fn source_identity(&self) -> Option<String> {
        let url = self.poster_source_url.as_deref()?;
        Some(match self.poster_source_etag.as_deref() {
            Some(etag) if !etag.is_empty() => format!("{url}#{etag}"),
            _ => url.to_string(),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PosterOverlayStatusCounts {
    pub enabled_titles: i64,
    pub rendered: i64,
    pub failed: i64,
}

#[async_trait]
pub trait PosterOverlayRepository: Send + Sync {
    async fn get_settings(&self) -> AppResult<PosterOverlaySettings>;
    async fn save_settings(&self, settings: &PosterOverlaySettings) -> AppResult<()>;

    async fn list_library_configs(&self) -> AppResult<Vec<PosterOverlayLibraryConfig>>;
    async fn set_library_config(
        &self,
        library_id: &str,
        enabled: bool,
        template_id: Option<&str>,
    ) -> AppResult<()>;

    async fn list_templates(&self) -> AppResult<Vec<PosterOverlayTemplate>>;
    async fn get_template(&self, template_id: &str) -> AppResult<Option<PosterOverlayTemplate>>;
    async fn save_template(&self, template: &PosterOverlayTemplate) -> AppResult<()>;
    async fn delete_template(&self, template_id: &str) -> AppResult<bool>;

    async fn load_inputs(&self, title_id: &str) -> AppResult<Option<PosterOverlayInputs>>;
    /// Titles in overlay-enabled libraries, keyset-paged by id.
    async fn list_enabled_title_ids(
        &self,
        after_title_id: Option<&str>,
        limit: usize,
    ) -> AppResult<Vec<String>>;
    /// Titles that have overlay state, keyset-paged by id.
    async fn list_state_title_ids(
        &self,
        after_title_id: Option<&str>,
        limit: usize,
    ) -> AppResult<Vec<String>>;

    async fn get_state(&self, title_id: &str) -> AppResult<Option<PosterOverlayState>>;
    async fn save_state(&self, state: &PosterOverlayState) -> AppResult<()>;
    async fn delete_state(&self, title_id: &str) -> AppResult<()>;
    async fn status_counts(&self) -> AppResult<PosterOverlayStatusCounts>;

    /// The output hash to serve for a title, only when its library has
    /// overlays enabled and a render exists.
    async fn active_output_hash(&self, title_id: &str) -> AppResult<Option<String>>;

    /// Point the title's presented poster URL at the overlay version so
    /// browsers holding the original under an immutable URL refetch.
    async fn set_presented_poster_version(&self, title_id: &str, version: &str) -> AppResult<()>;
    /// Point the title's presented poster URL back at the stored original
    /// variant, but only while it still carries `overlay_version`: an image
    /// refresh that already rewrote it is left alone.
    async fn restore_presented_poster_version(
        &self,
        title_id: &str,
        overlay_version: &str,
    ) -> AppResult<()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PosterOverlayVariant {
    Full,
    W250,
    W70,
}

impl PosterOverlayVariant {
    /// Maps the `/images/titles/{id}/poster/{variant}` route variants.
    pub fn from_route(variant: &str) -> Option<Self> {
        match variant {
            "original" | "w500" => Some(Self::Full),
            "w250" => Some(Self::W250),
            "w70" => Some(Self::W70),
            _ => None,
        }
    }

    pub fn file_stem(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::W250 => "w250",
            Self::W70 => "w70",
        }
    }

    pub fn width(self) -> Option<u32> {
        match self {
            Self::Full => None,
            Self::W250 => Some(250),
            Self::W70 => Some(70),
        }
    }

    pub const ALL: [Self; 3] = [Self::Full, Self::W250, Self::W70];
}

#[derive(Clone, Debug)]
pub struct PosterOverlayRenderRequest {
    pub original: Vec<u8>,
    pub template_svg: String,
    pub values: std::collections::BTreeMap<&'static str, String>,
    /// Written into the output's metadata marker.
    pub input_hash: String,
}

#[derive(Clone, Debug)]
pub struct PosterOverlayRendered {
    /// JPEG bytes for every variant, each carrying the marker.
    pub variants: Vec<(PosterOverlayVariant, Vec<u8>)>,
    /// blake3 of the full-size output.
    pub output_hash: String,
}

/// A template rendered on a sample poster for the editor. Nothing is
/// written: the result goes straight back to the caller.
#[derive(Clone, Debug)]
pub struct PosterOverlayPreviewRequest {
    /// Poster to draw on; a neutral placeholder when absent.
    pub background: Option<Vec<u8>>,
    pub template_svg: String,
    pub values: std::collections::BTreeMap<&'static str, String>,
}

/// Rendering, marker detection and overlay file ownership. Everything this
/// port writes or removes lives under the overlay data root.
#[async_trait]
pub trait PosterOverlayEngine: Send + Sync {
    /// Download the poster to use as a pristine original.
    async fn fetch_source(&self, source_url: &str) -> AppResult<Vec<u8>>;
    async fn read_original(&self, title_id: &str) -> AppResult<Option<Vec<u8>>>;
    /// Atomically replaces any previous original. Returns its path.
    async fn store_original(&self, title_id: &str, bytes: Vec<u8>) -> AppResult<String>;
    /// CPU work: runs on the render pool, never on the async runtime.
    async fn render(&self, request: PosterOverlayRenderRequest)
    -> AppResult<PosterOverlayRendered>;
    async fn write_outputs(
        &self,
        title_id: &str,
        rendered: &PosterOverlayRendered,
    ) -> AppResult<()>;
    async fn read_output(
        &self,
        title_id: &str,
        variant: PosterOverlayVariant,
    ) -> AppResult<Option<Vec<u8>>>;
    /// Removes the title's rendered outputs. Originals are kept.
    async fn remove_outputs(&self, title_id: &str) -> AppResult<()>;
    /// True when the bytes are a poster this engine produced.
    fn has_marker(&self, bytes: &[u8]) -> bool;
    /// CPU work on the render pool. Returns a JPEG without the overlay
    /// marker, sized for the editor; it is never stored.
    async fn render_preview(&self, request: PosterOverlayPreviewRequest) -> AppResult<Vec<u8>>;
    fn validate_template(&self, svg: &str) -> AppResult<()>;
    fn builtin_template(&self) -> &'static str;
    fn set_parallelism(&self, parallelism: usize);
}
