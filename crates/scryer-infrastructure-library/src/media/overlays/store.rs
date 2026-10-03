//! SQL for poster overlays. Writes go only to the `poster_overlay_*` tables,
//! plus the value of `titles.poster_local_path` so browsers refetch a poster
//! whose presented bytes changed. No existing schema is altered.

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::overlays::{
    MAX_OVERLAY_PARALLELISM, OverlayMediaFacts, PosterOverlayInputs, PosterOverlayLibraryConfig,
    PosterOverlayRepository, PosterOverlaySettings, PosterOverlayState, PosterOverlayStatusCounts,
    PosterOverlayTemplate,
};
use scryer_application::{AppResult, TitleImageKind};

use crate::media::images::synthesize_local_title_image_url;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

const SETTINGS_ID: &str = "default";
const PRESENTED_VARIANT: &str = "w250";

#[derive(Clone)]
pub struct PosterOverlayStore {
    datastore: StoreDatastore,
}

impl PosterOverlayStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }

    fn read(&self) -> SqlExec<'_, '_> {
        self.datastore.read_exec()
    }
}

fn is_remote_url(url: &str) -> bool {
    let url = url.trim();
    url.starts_with("https://") || url.starts_with("http://")
}

fn overlay_poster_path(title_id: &str, version: &str) -> String {
    format!("/images/titles/{title_id}/poster/{PRESENTED_VARIANT}?v={version}")
}

fn template_from_row(row: &SqlRow) -> AppResult<PosterOverlayTemplate> {
    Ok(PosterOverlayTemplate {
        id: row.text("id")?,
        name: row.text("name")?,
        svg: row.text("svg")?,
        content_hash: row.text("content_hash")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn non_negative(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

#[async_trait]
impl PosterOverlayRepository for PosterOverlayStore {
    async fn get_settings(&self) -> AppResult<PosterOverlaySettings> {
        let row = SqlRuntime::fetch_optional(
            self.read(),
            "SELECT parallelism, reconcile_interval_seconds, plex_push_enabled
               FROM poster_overlay_settings WHERE id = {}",
            &[SqlArg::Text(SETTINGS_ID.into())],
        )
        .await?;
        let Some(row) = row else {
            return Ok(PosterOverlaySettings::default());
        };
        let defaults = PosterOverlaySettings::default();
        Ok(PosterOverlaySettings {
            parallelism: usize::try_from(row.i64("parallelism")?)
                .ok()
                .filter(|value| (1..=MAX_OVERLAY_PARALLELISM).contains(value))
                .unwrap_or(defaults.parallelism),
            reconcile_interval_seconds: match non_negative(row.i64("reconcile_interval_seconds")?) {
                0 => defaults.reconcile_interval_seconds,
                seconds => seconds,
            },
            plex_push_enabled: row.bool("plex_push_enabled")?,
        })
    }

    async fn save_settings(&self, settings: &PosterOverlaySettings) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "save_poster_overlay_settings",
            "INSERT INTO poster_overlay_settings
                (id, parallelism, reconcile_interval_seconds, plex_push_enabled, updated_at)
             VALUES ({}, {}, {}, {}, {})
             ON CONFLICT (id) DO UPDATE SET
                parallelism = excluded.parallelism,
                reconcile_interval_seconds = excluded.reconcile_interval_seconds,
                plex_push_enabled = excluded.plex_push_enabled,
                updated_at = excluded.updated_at",
            vec![
                SqlArg::Text(SETTINGS_ID.into()),
                SqlArg::I64(settings.parallelism as i64),
                SqlArg::I64(i64::try_from(settings.reconcile_interval_seconds).unwrap_or(i64::MAX)),
                SqlArg::Bool(settings.plex_push_enabled),
                SqlArg::Timestamp(Utc::now()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn list_library_configs(&self) -> AppResult<Vec<PosterOverlayLibraryConfig>> {
        let rows = SqlRuntime::fetch_all(
            self.read(),
            "SELECT l.id, l.name, l.facet, pol.enabled, pol.template_id
               FROM libraries l
               LEFT JOIN poster_overlay_libraries pol ON pol.library_id = l.id
              ORDER BY l.facet, l.name, l.id",
            &[],
        )
        .await?;
        rows.iter()
            .map(|row| {
                Ok(PosterOverlayLibraryConfig {
                    library_id: row.text("id")?,
                    library_name: row.text("name")?,
                    facet: row.text("facet")?,
                    enabled: row.opt_bool("enabled")?.unwrap_or(false),
                    template_id: row.opt_text("template_id")?,
                })
            })
            .collect()
    }

    async fn set_library_config(
        &self,
        library_id: &str,
        enabled: bool,
        template_id: Option<&str>,
    ) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "set_poster_overlay_library",
            "INSERT INTO poster_overlay_libraries (library_id, enabled, template_id, updated_at)
             VALUES ({}, {}, {}, {})
             ON CONFLICT (library_id) DO UPDATE SET
                enabled = excluded.enabled,
                template_id = excluded.template_id,
                updated_at = excluded.updated_at",
            vec![
                SqlArg::Text(library_id.to_string()),
                SqlArg::Bool(enabled),
                SqlArg::OptText(template_id.map(str::to_string)),
                SqlArg::Timestamp(Utc::now()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn list_templates(&self) -> AppResult<Vec<PosterOverlayTemplate>> {
        let rows = SqlRuntime::fetch_all(
            self.read(),
            "SELECT id, name, svg, content_hash, created_at, updated_at
               FROM poster_overlay_templates ORDER BY name, id",
            &[],
        )
        .await?;
        rows.iter().map(template_from_row).collect()
    }

    async fn get_template(&self, template_id: &str) -> AppResult<Option<PosterOverlayTemplate>> {
        SqlRuntime::fetch_optional(
            self.read(),
            "SELECT id, name, svg, content_hash, created_at, updated_at
               FROM poster_overlay_templates WHERE id = {}",
            &[SqlArg::Text(template_id.to_string())],
        )
        .await?
        .as_ref()
        .map(template_from_row)
        .transpose()
    }

    async fn save_template(&self, template: &PosterOverlayTemplate) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "save_poster_overlay_template",
            "INSERT INTO poster_overlay_templates
                (id, name, svg, content_hash, created_at, updated_at)
             VALUES ({}, {}, {}, {}, {}, {})
             ON CONFLICT (id) DO UPDATE SET
                name = excluded.name,
                svg = excluded.svg,
                content_hash = excluded.content_hash,
                updated_at = excluded.updated_at",
            vec![
                SqlArg::Text(template.id.clone()),
                SqlArg::Text(template.name.clone()),
                SqlArg::Text(template.svg.clone()),
                SqlArg::Text(template.content_hash.clone()),
                SqlArg::Timestamp(template.created_at),
                SqlArg::Timestamp(template.updated_at),
            ],
        )
        .await?;
        Ok(())
    }

    async fn delete_template(&self, template_id: &str) -> AppResult<bool> {
        // Libraries using it fall back to the built-in template by foreign
        // key (`ON DELETE SET NULL`).
        let rows = SqlRuntime::execute_write(
            &self.datastore,
            "delete_poster_overlay_template",
            "DELETE FROM poster_overlay_templates WHERE id = {}",
            vec![SqlArg::Text(template_id.to_string())],
        )
        .await?;
        Ok(rows > 0)
    }

    async fn load_inputs(&self, title_id: &str) -> AppResult<Option<PosterOverlayInputs>> {
        let Some(title) = SqlRuntime::fetch_optional(
            self.read(),
            "SELECT t.id, t.library_id, t.facet, t.content_status, t.poster_url,
                    pol.enabled, pol.template_id, ti.source_url, ti.source_etag
               FROM titles t
               LEFT JOIN poster_overlay_libraries pol ON pol.library_id = t.library_id
               LEFT JOIN title_images ti ON ti.title_id = t.id AND ti.kind = 'poster'
              WHERE t.id = {}",
            &[SqlArg::Text(title_id.to_string())],
        )
        .await?
        else {
            return Ok(None);
        };
        let files = SqlRuntime::fetch_all(
            self.read(),
            "SELECT video_width, video_height, resolution, video_hdr_format,
                    audio_codec, audio_profile, audio_channels, edition, file_path, role
               FROM media_files
              WHERE title_id = {} AND role IN ('primary', 'additional')
              ORDER BY id",
            &[SqlArg::Text(title_id.to_string())],
        )
        .await?
        .iter()
        .map(|row| {
            Ok(OverlayMediaFacts {
                video_width: row.opt_i64("video_width")?,
                video_height: row.opt_i64("video_height")?,
                parsed_resolution: row.opt_text("resolution")?,
                video_hdr_format: row.opt_text("video_hdr_format")?,
                audio_codec: row.opt_text("audio_codec")?,
                audio_profile: row.opt_text("audio_profile")?,
                audio_channels: row.opt_i64("audio_channels")?,
                edition: row.opt_text("edition")?,
                file_path: row.opt_text("file_path")?,
                additional: row.opt_text("role")?.as_deref() == Some("additional"),
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
        Ok(Some(PosterOverlayInputs {
            title_id: title.text("id")?,
            library_id: title.opt_text("library_id")?,
            overlay_enabled: title.opt_bool("enabled")?.unwrap_or(false),
            template_id: title.opt_text("template_id")?,
            poster_source_url: match title.opt_text("source_url")? {
                Some(url) => Some(url),
                // The image cache is empty after a restore until metadata
                // refreshes it, and for titles whose artwork has not been
                // cached yet; the title row still carries the upstream URL.
                None => title
                    .opt_text("poster_url")?
                    .filter(|url| is_remote_url(url)),
            },
            poster_source_etag: title.opt_text("source_etag")?,
            facet: title.opt_text("facet")?,
            content_status: title.opt_text("content_status")?,
            files,
        }))
    }

    async fn list_enabled_title_ids(
        &self,
        after_title_id: Option<&str>,
        limit: usize,
    ) -> AppResult<Vec<String>> {
        SqlRuntime::fetch_all(
            self.read(),
            "SELECT t.id
               FROM titles t
               JOIN poster_overlay_libraries pol ON pol.library_id = t.library_id
              WHERE pol.enabled = {} AND t.id > {}
              ORDER BY t.id
              LIMIT {}",
            &[
                SqlArg::Bool(true),
                SqlArg::Text(after_title_id.unwrap_or("").to_string()),
                SqlArg::I64(limit as i64),
            ],
        )
        .await?
        .iter()
        .map(|row| row.text("id"))
        .collect()
    }

    async fn list_state_title_ids(
        &self,
        after_title_id: Option<&str>,
        limit: usize,
    ) -> AppResult<Vec<String>> {
        SqlRuntime::fetch_all(
            self.read(),
            "SELECT title_id FROM poster_overlay_state
              WHERE title_id > {} ORDER BY title_id LIMIT {}",
            &[
                SqlArg::Text(after_title_id.unwrap_or("").to_string()),
                SqlArg::I64(limit as i64),
            ],
        )
        .await?
        .iter()
        .map(|row| row.text("title_id"))
        .collect()
    }

    async fn get_state(&self, title_id: &str) -> AppResult<Option<PosterOverlayState>> {
        SqlRuntime::fetch_optional(
            self.read(),
            "SELECT title_id, original_path, original_hash, source_identity, input_hash,
                    output_hash, template_version, fields_json, rendered_at, last_error
               FROM poster_overlay_state WHERE title_id = {}",
            &[SqlArg::Text(title_id.to_string())],
        )
        .await?
        .map(|row| {
            Ok(PosterOverlayState {
                title_id: row.text("title_id")?,
                original_path: row.opt_text("original_path")?,
                original_hash: row.opt_text("original_hash")?,
                source_identity: row.opt_text("source_identity")?,
                input_hash: row.opt_text("input_hash")?,
                output_hash: row.opt_text("output_hash")?,
                template_version: row.opt_text("template_version")?,
                fields_json: row.opt_text("fields_json")?,
                rendered_at: row.opt_timestamp("rendered_at")?,
                last_error: row.opt_text("last_error")?,
            })
        })
        .transpose()
    }

    async fn save_state(&self, state: &PosterOverlayState) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "save_poster_overlay_state",
            "INSERT INTO poster_overlay_state
                (title_id, original_path, original_hash, source_identity, input_hash,
                 output_hash, template_version, fields_json, rendered_at, last_error, updated_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
             ON CONFLICT (title_id) DO UPDATE SET
                original_path = excluded.original_path,
                original_hash = excluded.original_hash,
                source_identity = excluded.source_identity,
                input_hash = excluded.input_hash,
                output_hash = excluded.output_hash,
                template_version = excluded.template_version,
                fields_json = excluded.fields_json,
                rendered_at = excluded.rendered_at,
                last_error = excluded.last_error,
                updated_at = excluded.updated_at",
            vec![
                SqlArg::Text(state.title_id.clone()),
                SqlArg::OptText(state.original_path.clone()),
                SqlArg::OptText(state.original_hash.clone()),
                SqlArg::OptText(state.source_identity.clone()),
                SqlArg::OptText(state.input_hash.clone()),
                SqlArg::OptText(state.output_hash.clone()),
                SqlArg::OptText(state.template_version.clone()),
                SqlArg::OptText(state.fields_json.clone()),
                SqlArg::OptTimestamp(state.rendered_at),
                SqlArg::OptText(state.last_error.clone()),
                SqlArg::Timestamp(Utc::now()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn delete_state(&self, title_id: &str) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "delete_poster_overlay_state",
            "DELETE FROM poster_overlay_state WHERE title_id = {}",
            vec![SqlArg::Text(title_id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn status_counts(&self) -> AppResult<PosterOverlayStatusCounts> {
        let row = SqlRuntime::fetch_optional(
            self.read(),
            "SELECT COUNT(t.id) AS enabled_titles,
                    COUNT(s.output_hash) AS rendered,
                    COUNT(s.last_error) AS failed,
                    COALESCE(SUM(CASE
                        WHEN NOT EXISTS (
                            SELECT 1 FROM title_images ti
                             WHERE ti.title_id = t.id AND ti.kind = 'poster'
                               AND ti.source_url IS NOT NULL
                        )
                        AND (t.poster_url IS NULL
                             OR (t.poster_url NOT LIKE 'https://%' AND t.poster_url NOT LIKE 'http://%'))
                        THEN 1 ELSE 0 END), 0) AS no_artwork
               FROM titles t
               JOIN poster_overlay_libraries pol ON pol.library_id = t.library_id
               LEFT JOIN poster_overlay_state s ON s.title_id = t.id
              WHERE pol.enabled = {}",
            &[SqlArg::Bool(true)],
        )
        .await?;
        let Some(row) = row else {
            return Ok(PosterOverlayStatusCounts::default());
        };
        Ok(PosterOverlayStatusCounts {
            enabled_titles: row.i64("enabled_titles")?,
            rendered: row.i64("rendered")?,
            failed: row.i64("failed")?,
            no_artwork: row.i64("no_artwork")?,
        })
    }

    async fn active_output_hash(&self, title_id: &str) -> AppResult<Option<String>> {
        Ok(SqlRuntime::fetch_optional(
            self.read(),
            "SELECT s.output_hash
               FROM poster_overlay_state s
               JOIN titles t ON t.id = s.title_id
               JOIN poster_overlay_libraries pol ON pol.library_id = t.library_id
              WHERE s.title_id = {} AND pol.enabled = {} AND s.output_hash IS NOT NULL",
            &[SqlArg::Text(title_id.to_string()), SqlArg::Bool(true)],
        )
        .await?
        .map(|row| row.text("output_hash"))
        .transpose()?)
    }

    async fn set_presented_poster_version(&self, title_id: &str, version: &str) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "set_poster_overlay_presented_version",
            "UPDATE titles SET poster_local_path = {} WHERE id = {}",
            vec![
                SqlArg::Text(overlay_poster_path(title_id, version)),
                SqlArg::Text(title_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn restore_presented_poster_version(
        &self,
        title_id: &str,
        overlay_version: &str,
    ) -> AppResult<()> {
        let digest = SqlRuntime::fetch_optional(
            self.read(),
            "SELECT tiv.blob_digest
               FROM title_image_variants tiv
               JOIN title_images ti ON ti.id = tiv.title_image_id
              WHERE ti.title_id = {} AND ti.kind = 'poster' AND tiv.variant_key = {}",
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(PRESENTED_VARIANT.into()),
            ],
        )
        .await?
        .map(|row| row.text("blob_digest"))
        .transpose()?;
        let restored = digest.map(|digest| {
            synthesize_local_title_image_url(
                "",
                title_id,
                TitleImageKind::Poster,
                PRESENTED_VARIANT,
                &digest,
            )
        });
        SqlRuntime::execute_write(
            &self.datastore,
            "restore_poster_overlay_presented_version",
            "UPDATE titles SET poster_local_path = {} WHERE id = {} AND poster_local_path = {}",
            vec![
                SqlArg::OptText(restored),
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(overlay_poster_path(title_id, overlay_version)),
            ],
        )
        .await?;
        Ok(())
    }
}
