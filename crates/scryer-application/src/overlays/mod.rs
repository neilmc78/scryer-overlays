//! Poster overlays: quality badges rendered over title posters from local
//! probe data. The template contract is documented in
//! `docs/poster-overlays.md`.

mod fields;
mod ports;
mod runtime;
mod service;
#[cfg(test)]
mod tests;

pub use fields::{
    CHANNEL_LAYOUTS, MAX_SAMPLE_EDITION_CHARS, OverlayAudio, OverlayFields, OverlayHdr,
    OverlayMediaFacts, OverlayResolution, OverlaySampleValues, RENDERER_REVISION, TEMPLATE_FIELDS,
    TEMPLATE_SPEC_VERSION, blake3_hex, condition_values, edition_token, input_hash,
    is_possible_condition_value, template_version,
};
pub use ports::{
    DEFAULT_OVERLAY_PARALLELISM, DEFAULT_OVERLAY_RECONCILE_INTERVAL_SECONDS,
    MAX_OVERLAY_PARALLELISM, MIN_OVERLAY_RECONCILE_INTERVAL_SECONDS, PosterOverlayEngine,
    PosterOverlayInputs, PosterOverlayLibraryConfig, PosterOverlayPreviewRequest,
    PosterOverlayRenderRequest, PosterOverlayRendered, PosterOverlayRepository,
    PosterOverlaySettings, PosterOverlayState, PosterOverlayStatusCounts, PosterOverlayTemplate,
    PosterOverlayVariant,
};
pub use runtime::{drain_overlay_events, start_poster_overlay_worker};
pub use service::{
    AppPosterOverlayServices, PosterOverlayImage, PosterOverlayOutcome, PosterOverlayOverview,
    PosterOverlayPassSummary, PosterOverlayPreview,
};
