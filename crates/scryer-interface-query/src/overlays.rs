//! Poster overlay reads: settings, per-library enablement, templates and
//! progress, plus template validation and previews.

use async_graphql::{Context, ID, Object, Result as GqlResult};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use scryer_application::AppError;
use scryer_interface_core::{actor_from_ctx, app_from_ctx, to_gql_error};
use scryer_interface_media::types::{
    PosterOverlayOverviewPayload, PosterOverlayTemplatePreviewPayload,
    PosterOverlayTemplateValidationPayload, PreviewPosterOverlayTemplateInput,
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
        #[graphql(desc = "The template SVG to check.")] svg: String,
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

    /// Render a draft template with sample values for the editor. Nothing
    /// is saved. Requires catalog settings management.
    async fn preview_poster_overlay_template(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The draft template and the sample values to show.")]
        input: PreviewPosterOverlayTemplateInput,
    ) -> GqlResult<PosterOverlayTemplatePreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        match app
            .preview_poster_overlay_template(
                &actor,
                &input.svg,
                &input.sample(),
                &input.poster_choice(),
            )
            .await
        {
            Ok(preview) => Ok(PosterOverlayTemplatePreviewPayload {
                image: Some(format!(
                    "data:image/jpeg;base64,{}",
                    BASE64.encode(&preview.jpeg)
                )),
                library_poster: preview.poster.is_some(),
                poster_title_id: preview
                    .poster
                    .as_ref()
                    .map(|poster| ID(poster.title_id.clone())),
                poster_title_name: preview.poster.map(|poster| poster.name),
                error: None,
            }),
            Err(AppError::Validation(message)) => Ok(PosterOverlayTemplatePreviewPayload {
                image: None,
                library_poster: false,
                poster_title_id: None,
                poster_title_name: None,
                error: Some(message),
            }),
            Err(error) => Err(to_gql_error(error)),
        }
    }
}
