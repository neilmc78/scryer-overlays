//! GraphQL types for poster overlays: per-library enablement, custom SVG
//! templates, render settings and progress counts.

use async_graphql::{ID, InputObject, SimpleObject};
use chrono::{DateTime, Utc};
use scryer_application::overlays::{
    PosterOverlayLibraryConfig, PosterOverlayOverview, PosterOverlaySettings,
    PosterOverlayStatusCounts, PosterOverlayTemplate, TEMPLATE_FIELDS, TEMPLATE_SPEC_VERSION,
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
    pub library_id: ID,
    pub library_name: String,
    pub facet: String,
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
    pub id: ID,
    pub name: String,
    pub svg: String,
    pub content_hash: String,
    pub created_at: DateTime<Utc>,
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
}

impl From<PosterOverlayStatusCounts> for PosterOverlayCountsPayload {
    fn from(value: PosterOverlayStatusCounts) -> Self {
        Self {
            enabled_titles: value.enabled_titles,
            rendered: value.rendered,
            failed: value.failed,
        }
    }
}

#[derive(SimpleObject, Clone)]
#[graphql(name = "PosterOverlayOverview")]
pub struct PosterOverlayOverviewPayload {
    pub settings: PosterOverlaySettingsPayload,
    pub libraries: Vec<PosterOverlayLibraryPayload>,
    pub templates: Vec<PosterOverlayTemplatePayload>,
    pub counts: PosterOverlayCountsPayload,
    /// The built-in template, a working example of the format.
    pub builtin_template: String,
    /// Highest `data-scryer-version` this server accepts.
    pub template_spec_version: i32,
    /// Every field a template may reference.
    pub template_fields: Vec<String>,
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
        }
    }
}

/// Result of checking a template without saving it.
#[derive(SimpleObject, Clone)]
#[graphql(name = "PosterOverlayTemplateValidation")]
pub struct PosterOverlayTemplateValidationPayload {
    pub valid: bool,
    /// Why the template was rejected; null when valid.
    pub error: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "SetPosterOverlayLibraryInput")]
pub struct SetPosterOverlayLibraryInput {
    pub library_id: ID,
    pub enabled: bool,
    /// Null selects the built-in template.
    pub template_id: Option<ID>,
}

#[derive(InputObject)]
#[graphql(name = "UpdatePosterOverlaySettingsInput")]
pub struct UpdatePosterOverlaySettingsInput {
    pub parallelism: i32,
    pub reconcile_interval_seconds: i64,
}

#[derive(InputObject)]
#[graphql(name = "SavePosterOverlayTemplateInput")]
pub struct SavePosterOverlayTemplateInput {
    /// Null creates a new template.
    pub id: Option<ID>,
    pub name: String,
    pub svg: String,
}
