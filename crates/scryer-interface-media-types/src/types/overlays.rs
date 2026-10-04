//! GraphQL types for poster overlays: per-library enablement, custom SVG
//! templates, render settings and progress counts.

use async_graphql::{ID, InputObject, SimpleObject};
use chrono::{DateTime, Utc};
use scryer_application::overlays::{
    CHANNEL_LAYOUTS, OverlayAudio, OverlayHdr, OverlayRatingSource, OverlayResolution,
    OverlaySampleValues, OverlaySeriesStatus, OverlaySource, OverlayVideoCodec,
    PosterOverlayLibraryConfig, PosterOverlayOverview, PosterOverlayPassProgress,
    PosterOverlayPreviewChoice, PosterOverlaySettings, PosterOverlayStatusCounts,
    PosterOverlayTemplate, TEMPLATE_FIELDS, TEMPLATE_SPEC_VERSION,
};

/// Render settings shared by every library.
#[derive(SimpleObject, Clone)]
#[graphql(name = "PosterOverlaySettings")]
pub struct PosterOverlaySettingsPayload {
    /// Posters rendered at once; also bounds memory during library-wide
    /// rebuilds.
    pub parallelism: i32,
    /// Seconds between safety-net passes that pick up missed changes and new
    /// upstream artwork.
    pub reconcile_interval_seconds: i64,
}

impl From<PosterOverlaySettings> for PosterOverlaySettingsPayload {
    fn from(value: PosterOverlaySettings) -> Self {
        Self {
            parallelism: i32::try_from(value.parallelism).unwrap_or(i32::MAX),
            reconcile_interval_seconds: i64::try_from(value.reconcile_interval_seconds)
                .unwrap_or(i64::MAX),
        }
    }
}

/// Whether a library's posters carry overlays, and with which template.
#[derive(SimpleObject, Clone)]
#[graphql(name = "PosterOverlayLibrary")]
pub struct PosterOverlayLibraryPayload {
    /// The library's identifier.
    pub library_id: ID,
    /// The library's display name.
    pub library_name: String,
    /// The library's media facet: `movie`, `series` or `anime`.
    pub facet: String,
    /// Whether posters in this library carry overlays.
    pub enabled: bool,
    /// Null when the library uses the built-in template.
    pub template_id: Option<ID>,
}

impl From<PosterOverlayLibraryConfig> for PosterOverlayLibraryPayload {
    fn from(value: PosterOverlayLibraryConfig) -> Self {
        Self {
            library_id: ID(value.library_id),
            library_name: value.library_name,
            facet: value.facet,
            enabled: value.enabled,
            template_id: value.template_id.map(ID),
        }
    }
}

/// A custom overlay template. The format is specified in
/// `docs/poster-overlays.md`.
#[derive(SimpleObject, Clone)]
#[graphql(name = "PosterOverlayTemplate")]
pub struct PosterOverlayTemplatePayload {
    /// The template's identifier.
    pub id: ID,
    /// The template's display name.
    pub name: String,
    /// The template SVG.
    pub svg: String,
    /// blake3 of the SVG; posters rebuild when it changes.
    pub content_hash: String,
    /// When the template was created.
    pub created_at: DateTime<Utc>,
    /// When the template was last changed.
    pub updated_at: DateTime<Utc>,
}

impl From<PosterOverlayTemplate> for PosterOverlayTemplatePayload {
    fn from(value: PosterOverlayTemplate) -> Self {
        Self {
            id: ID(value.id),
            name: value.name,
            svg: value.svg,
            content_hash: value.content_hash,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

/// Progress across overlay-enabled libraries.
#[derive(SimpleObject, Clone)]
#[graphql(name = "PosterOverlayCounts")]
pub struct PosterOverlayCountsPayload {
    /// Titles in libraries with overlays enabled.
    pub enabled_titles: i64,
    /// Of those, titles with a rendered poster.
    pub rendered: i64,
    /// Of those, titles whose last render failed.
    pub failed: i64,
    /// Of those, titles with no poster artwork to draw on yet.
    pub no_artwork: i64,
}

impl From<PosterOverlayStatusCounts> for PosterOverlayCountsPayload {
    fn from(value: PosterOverlayStatusCounts) -> Self {
        Self {
            enabled_titles: value.enabled_titles,
            rendered: value.rendered,
            failed: value.failed,
            no_artwork: value.no_artwork,
        }
    }
}

/// A library-wide overlay pass, such as a requested rebuild.
#[derive(SimpleObject, Clone)]
#[graphql(name = "PosterOverlayPass")]
pub struct PosterOverlayPassPayload {
    /// A pass was requested and has not started yet.
    pub queued: bool,
    /// A pass is running now.
    pub running: bool,
    /// Titles the running pass covers.
    pub total: i64,
    /// Of those, titles checked so far.
    pub processed: i64,
    /// Of those, titles re-rendered so far; unchanged titles are skipped.
    pub rendered: i64,
    /// Of those, titles whose render failed.
    pub failed: i64,
}

impl From<PosterOverlayPassProgress> for PosterOverlayPassPayload {
    fn from(value: PosterOverlayPassProgress) -> Self {
        let count = |value: usize| i64::try_from(value).unwrap_or(i64::MAX);
        Self {
            queued: value.queued,
            running: value.running,
            total: count(value.total),
            processed: count(value.processed),
            rendered: count(value.rendered),
            failed: count(value.failed),
        }
    }
}

/// Everything the poster overlay settings page shows.
#[derive(SimpleObject, Clone)]
#[graphql(name = "PosterOverlayOverview")]
pub struct PosterOverlayOverviewPayload {
    /// Render settings shared by every library.
    pub settings: PosterOverlaySettingsPayload,
    /// Overlay enablement and template for every library.
    pub libraries: Vec<PosterOverlayLibraryPayload>,
    /// Custom templates, by name.
    pub templates: Vec<PosterOverlayTemplatePayload>,
    /// Render progress across enabled libraries.
    pub counts: PosterOverlayCountsPayload,
    /// The built-in template, a working example of the format.
    pub builtin_template: String,
    /// Highest `data-scryer-version` this server accepts.
    pub template_spec_version: i32,
    /// Every field a template may reference.
    pub template_fields: Vec<String>,
    /// Values the template preview can be set to.
    pub sample_options: PosterOverlaySampleOptionsPayload,
    /// The library-wide pass in progress or queued.
    pub pass: PosterOverlayPassPayload,
}

/// One value of a badge field: the token conditions match and the label
/// placeholders show.
#[derive(SimpleObject, Clone)]
#[graphql(name = "PosterOverlaySampleOption")]
pub struct PosterOverlaySampleOptionPayload {
    /// The value conditions such as `data-scryer-if="hdr=dv"` match.
    pub token: String,
    /// The text the matching `_label` placeholder shows.
    pub label: String,
}

/// Every value each badge field can take, best first.
#[derive(SimpleObject, Clone)]
#[graphql(name = "PosterOverlaySampleOptions")]
pub struct PosterOverlaySampleOptionsPayload {
    /// Values of `resolution` and `resolution_label`.
    pub resolutions: Vec<PosterOverlaySampleOptionPayload>,
    /// Values of `hdr` and `hdr_label`.
    pub hdr: Vec<PosterOverlaySampleOptionPayload>,
    /// Values of `audio_codec` and `audio_label`.
    pub audio: Vec<PosterOverlaySampleOptionPayload>,
    /// Common values of `audio_channels`.
    pub audio_channels: Vec<String>,
    /// Values of `series_status` and `series_status_label`.
    pub series_status: Vec<PosterOverlaySampleOptionPayload>,
    /// Values of `video_codec` and `video_codec_label`.
    pub video_codec: Vec<PosterOverlaySampleOptionPayload>,
    /// Values of `source` and `source_label`.
    pub source: Vec<PosterOverlaySampleOptionPayload>,
}

impl PosterOverlaySampleOptionsPayload {
    fn current() -> Self {
        fn option(token: &str, label: &str) -> PosterOverlaySampleOptionPayload {
            PosterOverlaySampleOptionPayload {
                token: token.to_string(),
                label: label.to_string(),
            }
        }
        Self {
            resolutions: OverlayResolution::ALL
                .iter()
                .map(|value| option(value.token(), value.label()))
                .collect(),
            hdr: OverlayHdr::ALL
                .iter()
                .map(|value| option(value.token(), value.label()))
                .collect(),
            audio: OverlayAudio::ALL
                .iter()
                .map(|value| option(value.token(), value.label()))
                .collect(),
            source: OverlaySource::ALL
                .iter()
                .map(|value| option(value.token(), value.label()))
                .collect(),
            video_codec: OverlayVideoCodec::ALL
                .iter()
                .map(|value| option(value.token(), value.label()))
                .collect(),
            series_status: OverlaySeriesStatus::ALL
                .iter()
                .map(|value| option(value.token(), value.label()))
                .collect(),
            audio_channels: CHANNEL_LAYOUTS
                .iter()
                .map(|layout| layout.to_string())
                .collect(),
        }
    }
}

impl From<PosterOverlayOverview> for PosterOverlayOverviewPayload {
    fn from(value: PosterOverlayOverview) -> Self {
        Self {
            settings: value.settings.into(),
            libraries: value.libraries.into_iter().map(Into::into).collect(),
            templates: value.templates.into_iter().map(Into::into).collect(),
            counts: value.counts.into(),
            builtin_template: value.builtin_template.to_string(),
            template_spec_version: i32::try_from(TEMPLATE_SPEC_VERSION).unwrap_or(i32::MAX),
            template_fields: TEMPLATE_FIELDS
                .iter()
                .map(|field| field.to_string())
                .collect(),
            sample_options: PosterOverlaySampleOptionsPayload::current(),
            pass: value.pass.into(),
        }
    }
}

/// Result of checking a template without saving it.
#[derive(SimpleObject, Clone)]
#[graphql(name = "PosterOverlayTemplateValidation")]
pub struct PosterOverlayTemplateValidationPayload {
    /// Whether the template can be saved.
    pub valid: bool,
    /// Why the template was rejected; null when valid.
    pub error: Option<String>,
}

/// Overlay enablement and template for one library.
#[derive(InputObject)]
#[graphql(name = "SetPosterOverlayLibraryInput")]
pub struct SetPosterOverlayLibraryInput {
    /// The library to change.
    pub library_id: ID,
    /// Whether posters in the library carry overlays.
    pub enabled: bool,
    /// Null selects the built-in template.
    pub template_id: Option<ID>,
}

/// New render settings, shared by every library.
#[derive(InputObject)]
#[graphql(name = "UpdatePosterOverlaySettingsInput")]
pub struct UpdatePosterOverlaySettingsInput {
    /// Posters rendered at once.
    pub parallelism: i32,
    /// Seconds between safety-net passes.
    pub reconcile_interval_seconds: i64,
}

/// A custom template to create or replace.
#[derive(InputObject)]
#[graphql(name = "SavePosterOverlayTemplateInput")]
pub struct SavePosterOverlayTemplateInput {
    /// Null creates a new template.
    pub id: Option<ID>,
    /// The template's display name.
    pub name: String,
    /// The template SVG; it must pass validation.
    pub svg: String,
}

/// A draft template and the sample values to preview it with. Empty or
/// absent values leave that field unset, as for a title without the data.
#[derive(InputObject)]
#[graphql(name = "PreviewPosterOverlayTemplateInput")]
pub struct PreviewPosterOverlayTemplateInput {
    /// The draft template SVG.
    pub svg: String,
    /// A resolution token from `sampleOptions.resolutions`.
    pub resolution: Option<String>,
    /// An HDR token from `sampleOptions.hdr`.
    pub hdr: Option<String>,
    /// An audio token from `sampleOptions.audio`.
    pub audio: Option<String>,
    /// A layout from `sampleOptions.audioChannels`.
    pub audio_channels: Option<String>,
    /// Free text, as an edition would appear on a release.
    pub edition: Option<String>,
    /// A status token from `sampleOptions.seriesStatus`.
    pub series_status: Option<String>,
    /// A codec token from `sampleOptions.videoCodec`.
    pub video_codec: Option<String>,
    /// A source token from `sampleOptions.source`.
    pub source: Option<String>,
    /// Draw on a poster from this kind of library: `movie`, `series` or
    /// `anime`. Any library when null.
    pub poster_facet: Option<String>,
    /// Keep drawing on this title's poster (from a previous preview's
    /// `posterTitleId`); a random one is picked when null.
    pub poster_title_id: Option<ID>,
}

impl PreviewPosterOverlayTemplateInput {
    pub fn poster_choice(&self) -> PosterOverlayPreviewChoice {
        PosterOverlayPreviewChoice {
            facet: self.poster_facet.clone(),
            title_id: self.poster_title_id.as_ref().map(|id| id.to_string()),
        }
    }

    pub fn sample(&self) -> OverlaySampleValues {
        OverlaySampleValues {
            resolution: self.resolution.clone(),
            hdr: self.hdr.clone(),
            audio: self.audio.clone(),
            audio_channels: self.audio_channels.clone(),
            edition: self.edition.clone(),
            series_status: self.series_status.clone(),
            video_codec: self.video_codec.clone(),
            source: self.source.clone(),
        }
    }
}

/// A rendered template preview, or why it could not be rendered.
#[derive(SimpleObject, Clone)]
#[graphql(name = "PosterOverlayTemplatePreview")]
pub struct PosterOverlayTemplatePreviewPayload {
    /// `data:image/jpeg;base64,...`; null when the template or sample was
    /// rejected.
    pub image: Option<String>,
    /// True when drawn on a poster from the library rather than a neutral
    /// placeholder.
    pub library_poster: bool,
    /// The title whose poster the preview is drawn on; pass it back as
    /// `posterTitleId` to keep the same poster while editing.
    pub poster_title_id: Option<ID>,
    /// That title's name.
    pub poster_title_name: Option<String>,
    /// Why the template or sample was rejected; null on success.
    pub error: Option<String>,
}
