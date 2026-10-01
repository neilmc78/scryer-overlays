use async_graphql::{Context, ID, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::types::{
    PosterOverlaySettingsPayload, PosterOverlayTemplatePayload, SavePosterOverlayTemplateInput,
    SetPosterOverlayLibraryInput, UpdatePosterOverlaySettingsInput,
};

/// Poster overlay changes. Every one requires catalog settings permission.
#[derive(Default)]
pub struct PosterOverlayMutations;

#[Object]
impl PosterOverlayMutations {
    /// Turn overlays on or off for a library and choose its template.
    /// Enabling starts a render pass; disabling serves originals at once.
    async fn set_poster_overlay_library(
        &self,
        ctx: &Context<'_>,
        input: SetPosterOverlayLibraryInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.set_poster_overlay_library(
            &actor,
            input.library_id.as_str(),
            input.enabled,
            input.template_id.as_ref().map(|id| id.as_str()),
        )
        .await
        .map_err(to_gql_error)?;
        Ok(true)
    }

    async fn update_poster_overlay_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdatePosterOverlaySettingsInput,
    ) -> GqlResult<PosterOverlaySettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let parallelism = usize::try_from(input.parallelism).unwrap_or(0);
        let interval = u64::try_from(input.reconcile_interval_seconds).unwrap_or(0);
        let settings = app
            .update_poster_overlay_settings(&actor, parallelism, interval)
            .await
            .map_err(to_gql_error)?;
        Ok(settings.into())
    }

    /// Create a template, or replace one when `id` is given. Posters using
    /// it are rebuilt.
    async fn save_poster_overlay_template(
        &self,
        ctx: &Context<'_>,
        input: SavePosterOverlayTemplateInput,
    ) -> GqlResult<PosterOverlayTemplatePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let template = app
            .save_poster_overlay_template(
                &actor,
                input.id.as_ref().map(|id| id.as_str()),
                &input.name,
                &input.svg,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(template.into())
    }

    /// Delete a template. Libraries using it fall back to the built-in one.
    async fn delete_poster_overlay_template(&self, ctx: &Context<'_>, id: ID) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_poster_overlay_template(&actor, id.as_str())
            .await
            .map_err(to_gql_error)
    }

    /// Start a pass over every enabled library. Posters whose inputs are
    /// unchanged are skipped.
    async fn rebuild_poster_overlays(&self, ctx: &Context<'_>) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.request_poster_overlay_rebuild(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }

    /// Disable overlays in every library and serve every original poster
    /// again. Returns how many posters were reverted.
    async fn revert_all_poster_overlays(&self, ctx: &Context<'_>) -> GqlResult<i32> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let reverted = app
            .revert_all_poster_overlays(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(i32::try_from(reverted).unwrap_or(i32::MAX))
    }
}
