//! Poster overlays: quality badges rendered over title posters from local
//! probe data. The template contract is documented in
//! `docs/poster-overlays.md`.

mod collection_order;
mod collections;
mod fields;
mod plex;
mod ports;
mod runtime;
mod service;
#[cfg(test)]
mod tests;

pub use collection_order::{
    MAX_LISTS_PER_PROVIDER, PROVIDER_SLOTS, sort_number, sort_title as collection_sort_title,
};
pub use collections::{
    LogoChoice, MAX_COLLECTION_POSTER_TEXT, PlexCollectionCounts, PlexCollectionListStatus,
    PlexCollectionSource, parse_poster_colour, provider_poster_colour, provider_poster_logo,
};
pub use fields::{
    CHANNEL_LAYOUTS, EDITION_LABEL_SEPARATOR, EDITION_TOKEN_SEPARATOR, MAX_SAMPLE_EDITION_CHARS,
    OverlayAudio, OverlayFields, OverlayHdr, OverlayMediaFacts, OverlayRatingSource,
    OverlayResolution, OverlaySampleValues, OverlaySeriesStatus, OverlaySource, OverlayVideoCodec,
    RENDERER_REVISION, TEMPLATE_FIELDS, TEMPLATE_SPEC_VERSION, blake3_hex, condition_value_set,
    condition_values, edition_from_file_name, edition_token, input_hash,
    is_possible_condition_value, template_version,
};
pub use plex::{PLEX_POSTER_CHANGED_IN_PLEX, PlexSyncCounts, maintenance_remaining};
pub use ports::{
    CollectionPosterRequest, DEFAULT_OVERLAY_PARALLELISM,
    DEFAULT_OVERLAY_RECONCILE_INTERVAL_SECONDS, MAX_OVERLAY_PARALLELISM,
    MIN_OVERLAY_RECONCILE_INTERVAL_SECONDS, PlexCollectionList, PlexCollectionMember,
    PlexCollectionRef, PlexCollectionTitle, PlexMaintenanceWindow, PlexMovieVersion,
    PlexPosterItem, PosterOverlayEngine, PosterOverlayInputs, PosterOverlayLibraryConfig,
    PosterOverlayPlexClient, PosterOverlayPlexState, PosterOverlayPreviewPoster,
    PosterOverlayPreviewRequest, PosterOverlayRenderRequest, PosterOverlayRendered,
    PosterOverlayRepository, PosterOverlaySettings, PosterOverlayState, PosterOverlayStatusCounts,
    PosterOverlayTemplate, PosterOverlayVariant,
};
pub use runtime::{drain_overlay_events, start_poster_overlay_worker};
pub use service::{
    AppPosterOverlayServices, PosterOverlayImage, PosterOverlayOutcome, PosterOverlayOverview,
    PosterOverlayPassProgress, PosterOverlayPassSummary, PosterOverlayPreview,
    PosterOverlayPreviewChoice, overlay_title_for_proxy_source,
};
