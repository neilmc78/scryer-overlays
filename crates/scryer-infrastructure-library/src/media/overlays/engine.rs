//! Overlay engine: owns `<data_dir>/overlays/`, the render pool and the
//! source fetch.
//!
//! File layout (every path is derived from a validated title id, never read
//! back from the database):
//!
//! ```text
//! overlays/originals/{title_id}          pristine source poster
//! overlays/output/{title_id}/full.jpg    rendered, full size
//! overlays/output/{title_id}/w250.jpg
//! overlays/output/{title_id}/w70.jpg
//! ```
//!
//! Deletion is limited to the three fixed output file names, their temp
//! siblings, and the then-empty output directory of one title. Originals are
//! only ever replaced by an atomic rename, never deleted.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use scryer_application::overlays::{
    DEFAULT_OVERLAY_PARALLELISM, MAX_OVERLAY_PARALLELISM, PosterOverlayEngine,
    PosterOverlayRenderRequest, PosterOverlayRendered, PosterOverlayVariant,
};
use scryer_application::{AppError, AppResult};

use super::render::{self, BUILTIN_TEMPLATE, OverlayRenderer};
use crate::media::images::processor::HttpTitleImageProcessor;

const MAX_TITLE_ID_LEN: usize = 128;
const TMDB_OVERLAY_SOURCE_SIZE: &str = "w780";

/// Where originals come from. Production fetches over HTTP; tests inject
/// bytes.
#[async_trait]
pub trait OverlaySourceFetch: Send + Sync {
    async fn fetch(&self, source_url: &str) -> AppResult<Vec<u8>>;
}

/// Fetches through the title-image transport policy: DNS-pinned public
/// targets, no redirects, bounded size and retries.
pub struct HttpOverlaySourceFetch {
    processor: HttpTitleImageProcessor,
}

impl Default for HttpOverlaySourceFetch {
    fn default() -> Self {
        Self {
            processor: HttpTitleImageProcessor::new(),
        }
    }
}

#[async_trait]
impl OverlaySourceFetch for HttpOverlaySourceFetch {
    async fn fetch(&self, source_url: &str) -> AppResult<Vec<u8>> {
        let url = overlay_source_url(source_url);
        let (_, bytes, _, _) = self.processor.fetch_source(&url).await?;
        Ok(bytes)
    }
}

/// TMDB serves sized renditions; overlays render on a 780px-wide source so
/// badges stay legible in every served size.
pub fn overlay_source_url(source_url: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(source_url) else {
        return source_url.to_string();
    };
    if !parsed
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case("image.tmdb.org"))
    {
        return source_url.to_string();
    }
    let Some(rest) = parsed.path().strip_prefix("/t/p/") else {
        return source_url.to_string();
    };
    let Some((_, asset)) = rest.split_once('/') else {
        return source_url.to_string();
    };
    let path = format!("/t/p/{TMDB_OVERLAY_SOURCE_SIZE}/{asset}");
    parsed.set_path(&path);
    parsed.to_string()
}

pub struct OverlayEngine {
    root: PathBuf,
    renderer: OverlayRenderer,
    fetcher: Arc<dyn OverlaySourceFetch>,
    pool: RwLock<(usize, Arc<rayon::ThreadPool>)>,
}

impl OverlayEngine {
    /// `data_dir` is Scryer's data directory; the engine owns
    /// `data_dir/overlays`.
    pub fn new(data_dir: &Path, fetcher: Arc<dyn OverlaySourceFetch>) -> AppResult<Self> {
        Ok(Self {
            root: data_dir.join("overlays"),
            renderer: OverlayRenderer::new(),
            fetcher,
            pool: RwLock::new((
                DEFAULT_OVERLAY_PARALLELISM,
                Arc::new(build_pool(DEFAULT_OVERLAY_PARALLELISM)?),
            )),
        })
    }

    pub fn with_http_fetch(data_dir: &Path) -> AppResult<Self> {
        Self::new(data_dir, Arc::new(HttpOverlaySourceFetch::default()))
    }

    fn originals_dir(&self) -> PathBuf {
        self.root.join("originals")
    }

    fn original_path(&self, title_id: &str) -> AppResult<PathBuf> {
        Ok(self.originals_dir().join(checked_title_id(title_id)?))
    }

    fn output_dir(&self, title_id: &str) -> AppResult<PathBuf> {
        Ok(self.root.join("output").join(checked_title_id(title_id)?))
    }

    fn pool(&self) -> Arc<rayon::ThreadPool> {
        match self.pool.read() {
            Ok(guard) => guard.1.clone(),
            Err(poisoned) => poisoned.into_inner().1.clone(),
        }
    }
}

fn build_pool(threads: usize) -> AppResult<rayon::ThreadPool> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|index| format!("scryer-overlay-{index}"))
        .start_handler(|_| {
            let _ = scryer_application::background_worker_priority();
        })
        .build()
        .map_err(|error| AppError::Repository(format!("overlay pool creation failed: {error}")))
}

/// Title ids become path components, so only a conservative character set is
/// accepted. Anything else is refused before a path is built.
fn checked_title_id(title_id: &str) -> AppResult<&str> {
    let valid = !title_id.is_empty()
        && title_id.len() <= MAX_TITLE_ID_LEN
        && title_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if valid {
        Ok(title_id)
    } else {
        Err(AppError::Validation(format!(
            "title id is not usable as an overlay path: {title_id:?}"
        )))
    }
}

fn output_file_name(variant: PosterOverlayVariant) -> String {
    format!("{}.jpg", variant.file_stem())
}

fn temp_name(name: &str) -> String {
    format!(".{name}.tmp")
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> AppError {
    AppError::Repository(format!("failed to {action} {}: {error}", path.display()))
}

/// Write `bytes` beside `target` and rename over it, so readers never see a
/// partial file.
async fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> AppResult<()> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|error| io_error("create", dir, error))?;
    let temp = dir.join(temp_name(name));
    let target = dir.join(name);
    tokio::fs::write(&temp, bytes)
        .await
        .map_err(|error| io_error("write", &temp, error))?;
    tokio::fs::rename(&temp, &target)
        .await
        .map_err(|error| io_error("replace", &target, error))
}

async fn read_optional(path: &Path) -> AppResult<Option<Vec<u8>>> {
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("read", path, error)),
    }
}

async fn remove_if_present(path: &Path) -> AppResult<()> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("remove", path, error)),
    }
}

#[async_trait]
impl PosterOverlayEngine for OverlayEngine {
    async fn fetch_source(&self, source_url: &str) -> AppResult<Vec<u8>> {
        self.fetcher.fetch(source_url).await
    }

    async fn read_original(&self, title_id: &str) -> AppResult<Option<Vec<u8>>> {
        read_optional(&self.original_path(title_id)?).await
    }

    async fn store_original(&self, title_id: &str, bytes: Vec<u8>) -> AppResult<String> {
        let name = checked_title_id(title_id)?;
        let dir = self.originals_dir();
        write_atomic(&dir, name, &bytes).await?;
        Ok(dir.join(name).to_string_lossy().into_owned())
    }

    async fn render(
        &self,
        request: PosterOverlayRenderRequest,
    ) -> AppResult<PosterOverlayRendered> {
        let renderer = self.renderer.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        self.pool().spawn(move || {
            if send.is_closed() {
                return;
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                renderer.render(
                    &request.original,
                    &request.template_svg,
                    &request.values,
                    &request.input_hash,
                )
            }))
            .unwrap_or_else(|_| Err(AppError::Repository("overlay render panicked".into())));
            let _ = send.send(result);
        });
        receive
            .await
            .map_err(|error| AppError::Repository(format!("overlay render was dropped: {error}")))?
    }

    async fn write_outputs(
        &self,
        title_id: &str,
        rendered: &PosterOverlayRendered,
    ) -> AppResult<()> {
        let dir = self.output_dir(title_id)?;
        // Smaller variants first: `full.jpg` landing last means a complete
        // set exists whenever the full-size output does.
        for variant in [
            PosterOverlayVariant::W70,
            PosterOverlayVariant::W250,
            PosterOverlayVariant::Full,
        ] {
            let bytes = rendered
                .variants
                .iter()
                .find(|(candidate, _)| *candidate == variant)
                .map(|(_, bytes)| bytes)
                .ok_or_else(|| {
                    AppError::Repository(format!(
                        "overlay render is missing the {} variant",
                        variant.file_stem()
                    ))
                })?;
            write_atomic(&dir, &output_file_name(variant), bytes).await?;
        }
        Ok(())
    }

    async fn read_output(
        &self,
        title_id: &str,
        variant: PosterOverlayVariant,
    ) -> AppResult<Option<Vec<u8>>> {
        read_optional(&self.output_dir(title_id)?.join(output_file_name(variant))).await
    }

    async fn remove_outputs(&self, title_id: &str) -> AppResult<()> {
        let dir = self.output_dir(title_id)?;
        for variant in PosterOverlayVariant::ALL {
            let name = output_file_name(variant);
            remove_if_present(&dir.join(&name)).await?;
            remove_if_present(&dir.join(temp_name(&name))).await?;
        }
        // Non-recursive: fails harmlessly if anything unexpected remains.
        match tokio::fs::remove_dir(&dir).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Ok(()),
        }
    }

    fn has_marker(&self, bytes: &[u8]) -> bool {
        render::has_marker(bytes)
    }

    fn validate_template(&self, svg: &str) -> AppResult<()> {
        self.renderer.validate(svg)
    }

    fn builtin_template(&self) -> &'static str {
        BUILTIN_TEMPLATE
    }

    fn set_parallelism(&self, parallelism: usize) {
        let parallelism = parallelism.clamp(1, MAX_OVERLAY_PARALLELISM);
        let Ok(mut guard) = self.pool.write() else {
            return;
        };
        if guard.0 == parallelism {
            return;
        }
        match build_pool(parallelism) {
            // Renders already queued on the old pool finish there; it is
            // dropped once the last of them releases it.
            Ok(pool) => *guard = (parallelism, Arc::new(pool)),
            Err(error) => tracing::warn!(%error, "keeping the existing overlay render pool"),
        }
    }
}
