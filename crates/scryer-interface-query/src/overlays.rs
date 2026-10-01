//! Poster overlay reads: settings, per-library enablement, templates and
//! progress, plus template validation.

use async_graphql::{Context, Object, Result as GqlResult};
use scryer_application::AppError;
use scryer_interface_core::{actor_from_ctx, app_from_ctx, to_gql_error};
use scryer_interface_media::types::{
    PosterOverlayOverviewPayload, PosterOverlayTemplateValidationPayload,
};

#[derive(Default)]
pub(crate) struct PosterOverlayQueries;

#[Object]
impl PosterOverlayQueries {
    /// Overlay settings, libraries, templates and progress. Requires
    /// catalog or library settings access.
    async fn poster_overlays(&self, ctx: &Context<'_>) -> GqlResult<PosterOverlayOverviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let overview = app
            .poster_overlay_overview(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(overview.into())
    }

    /// Check a template against the format without saving it.
    async fn validate_poster_overlay_template(
        &self,
        ctx: &Context<'_>,
        svg: String,
    ) -> GqlResult<PosterOverlayTemplateValidationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        match app.validate_poster_overlay_template(&actor, &svg).await {
            Ok(()) => Ok(PosterOverlayTemplateValidationPayload {
                valid: true,
                error: None,
            }),
            Err(AppError::Validation(message)) => Ok(PosterOverlayTemplateValidationPayload {
                valid: false,
                error: Some(message),
            }),
            Err(error) => Err(to_gql_error(error)),
        }
    }
}
