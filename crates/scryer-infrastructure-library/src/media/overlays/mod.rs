//! Poster overlays: the SQL store, the render engine and its file layout.
//! The template contract is specified in `docs/poster-overlays.md`.

#[cfg(feature = "image-processing")]
mod engine;
#[cfg(feature = "image-processing")]
mod render;
mod store;
#[cfg(feature = "image-processing")]
mod template;
#[cfg(all(test, feature = "image-processing"))]
mod tests;

#[cfg(feature = "image-processing")]
pub use engine::{HttpOverlaySourceFetch, OverlayEngine, OverlaySourceFetch, overlay_source_url};
#[cfg(feature = "image-processing")]
pub use render::{
    BUILTIN_TEMPLATE, EMBEDDED_FONT_FAMILY, MARKER_PREFIX, OverlayRenderer, PREVIEW_HEIGHT,
    PREVIEW_WIDTH, add_marker, has_marker, marker_input_hash,
};
pub use store::PosterOverlayStore;
#[cfg(feature = "image-processing")]
pub use template::{
    LOGO_ID_PREFIX, LOGOS, TEMPLATE_FIELDS, evaluate_condition, preprocess, substitute, validate,
};
