use async_graphql::{Context, ID, Object, Result as GqlResult};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use scryer_application::AppError;

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::types::{
    PlexCollectionPosterInput, PosterOverlaySettingsPayload, PosterOverlayTemplatePayload,
    SavePosterOverlayTemplateInput, SetPosterOverlayLibraryInput, UpdatePosterOverlaySettingsInput,
};

/// Poster overlay changes, which require catalog settings permission, and Plex
/// list collections, which require list management permission.
#[derive(Default)]
pub struct PosterOverlayMutations;

#[Object]
impl PosterOverlayMutations {
    /// Turn overlays on or off for a library and choose its template.
    /// Enabling starts a render pass; disabling serves originals at once.
    async fn set_poster_overlay_library(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The library, whether overlays are on, and its template.")]
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

    /// Change render parallelism, the safety-net reconcile interval and
    /// whether posters are pushed to Plex.
    async fn update_poster_overlay_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The complete new settings.")] input: UpdatePosterOverlaySettingsInput,
    ) -> GqlResult<PosterOverlaySettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let parallelism = usize::try_from(input.parallelism).unwrap_or(0);
        let interval = u64::try_from(input.reconcile_interval_seconds).unwrap_or(0);
        let settings = app
            .update_poster_overlay_settings(
                &actor,
                parallelism,
                interval,
                input.plex_push_enabled,
                input.silo_push_enabled,
                input.silo_api_url,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(settings.into())
    }

    /// Create a template, or replace one when `id` is given. Posters using
    /// it are rebuilt.
    async fn save_poster_overlay_template(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The template to create or replace.")]
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
    async fn delete_poster_overlay_template(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The template to delete.")] id: ID,
    ) -> GqlResult<bool> {
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

    /// Mirror a public list into Plex as a collection named after it, or
    /// stop. Stopping takes the list's items back out of the collection.
    /// Requires list management permission.
    async fn set_plex_collection_list(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The public list.")] subscription_id: ID,
        #[graphql(desc = "Whether the list is mirrored into Plex.")] enabled: bool,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.set_plex_collection_list(&actor, subscription_id.as_str(), enabled)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }

    /// Move a mirrored list one place up or down inside its provider's block
    /// of Plex sort numbers. Requires list management permission.
    async fn move_plex_collection_list(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The mirrored public list.")] subscription_id: ID,
        #[graphql(desc = "Up towards the provider's first number, else down.")] up: bool,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.move_plex_collection_list(&actor, subscription_id.as_str(), up)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }

    /// Move a provider's whole block of collections one block up or down,
    /// trading places with the provider there. Requires list management
    /// permission.
    async fn move_plex_collection_provider(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The list provider, such as `imdb`.")] provider: String,
        #[graphql(desc = "Up towards `!1000_`, else down.")] up: bool,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.move_plex_collection_provider(&actor, &provider, up)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }

    /// Choose a mirrored list's collection poster, then sync it to Plex.
    /// Requires list management permission.
    async fn set_plex_collection_poster(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The poster design.")] input: PlexCollectionPosterInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let logo = input
            .logo_choice()
            .map_err(|message| to_gql_error(AppError::Validation(message)))?;
        app.set_plex_collection_poster(
            &actor,
            input.subscription_id.as_str(),
            input.enabled,
            input.background.as_deref(),
            input.text.as_deref(),
            logo,
        )
        .await
        .map_err(to_gql_error)?;
        Ok(true)
    }

    /// Upload a PNG, JPEG or WebP image (at most 2 MB) as a mirrored list's
    /// collection poster logo, replacing any earlier one, then sync. Choose
    /// it with `logoSource: "custom"`. Requires list management permission.
    async fn upload_plex_collection_logo(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The mirrored public list.")] subscription_id: ID,
        #[graphql(desc = "The image, base64-encoded.")] image_base64: String,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        // Base64 of 2 MB plus slack for padding and whitespace.
        if image_base64.len() > 3 * 1024 * 1024 {
            return Err(to_gql_error(AppError::Validation(
                "the image may be at most 2 MB".into(),
            )));
        }
        let bytes = BASE64
            .decode(image_base64.trim())
            .map_err(|_| to_gql_error(AppError::Validation("the image is not base64".into())))?;
        app.upload_plex_collection_logo(&actor, subscription_id.as_str(), bytes)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }

    /// Sync every mirrored list into its Plex collection now. Requires list
    /// management permission.
    async fn sync_plex_collections(&self, ctx: &Context<'_>) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.request_plex_collection_sync(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }
}
