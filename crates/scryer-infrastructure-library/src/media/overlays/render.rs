//! CPU side of poster overlays: SVG rendering, compositing, JPEG encoding and
//! the output metadata marker. Nothing here touches the filesystem or the
//! async runtime.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::Arc;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageReader, Limits, RgbaImage};
use img_parts::Bytes;
use img_parts::jpeg::{Jpeg, JpegSegment, markers};
use resvg::{tiny_skia, usvg};
use scryer_application::overlays::{PosterOverlayRendered, PosterOverlayVariant, blake3_hex};
use scryer_application::{AppError, AppResult};

use super::template;

/// Inter Bold, SIL Open Font License 1.1: `assets/overlays/Inter-OFL.txt`.
static EMBEDDED_FONT: &[u8] = include_bytes!("../../../assets/overlays/Inter-Bold.ttf");
pub const EMBEDDED_FONT_FAMILY: &str = "Inter";
pub static BUILTIN_TEMPLATE: &str = include_str!("../../../assets/overlays/default.svg");

/// Written as a JPEG comment segment into every output. Its presence is how
/// an overlaid poster is recognised and never treated as an original.
pub const MARKER_PREFIX: &[u8] = b"scryer-overlay:";
const MARKER_VERSION: &str = "v1";

const JPEG_QUALITY: u8 = 90;
/// Editor previews: 2:3 like the template viewBox, so element positions in
/// the editor line up with the image exactly.
pub const PREVIEW_WIDTH: u32 = 500;
pub const PREVIEW_HEIGHT: u32 = 750;
/// Decoding bound for originals; a poster far larger than this is refused
/// rather than allowed to exhaust memory during a library-wide rebuild.
const MAX_DECODE_ALLOC: u64 = 256 * 1024 * 1024;
const MAX_ORIGINAL_DIMENSION: u32 = 8000;

#[derive(Clone)]
pub struct OverlayRenderer {
    fontdb: Arc<usvg::fontdb::Database>,
}

impl Default for OverlayRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl OverlayRenderer {
    /// Builds the font database from the embedded font only. Called once at
    /// startup; system fonts are never consulted.
    pub fn new() -> Self {
        let mut fontdb = usvg::fontdb::Database::new();
        fontdb.load_font_data(EMBEDDED_FONT.to_vec());
        fontdb.set_sans_serif_family(EMBEDDED_FONT_FAMILY);
        fontdb.set_serif_family(EMBEDDED_FONT_FAMILY);
        fontdb.set_monospace_family(EMBEDDED_FONT_FAMILY);
        fontdb.set_cursive_family(EMBEDDED_FONT_FAMILY);
        fontdb.set_fantasy_family(EMBEDDED_FONT_FAMILY);
        Self {
            fontdb: Arc::new(fontdb),
        }
    }

    fn options(&self) -> usvg::Options<'static> {
        usvg::Options {
            resources_dir: None,
            font_family: EMBEDDED_FONT_FAMILY.to_string(),
            fontdb: self.fontdb.clone(),
            // Templates may not reference images; refuse any that slip past
            // validation instead of resolving them.
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_data: Box::new(|_, _, _| None),
                resolve_string: Box::new(|_, _| None),
            },
            ..usvg::Options::default()
        }
    }

    /// Validate a template end to end: contract rules, then that resvg can
    /// parse the result.
    pub fn validate(&self, svg: &str) -> AppResult<()> {
        template::validate(svg)?;
        let values = template::TEMPLATE_FIELDS
            .iter()
            .map(|field| (*field, String::new()))
            .collect::<BTreeMap<_, _>>();
        let options = self.options();
        let resolved = template::preprocess_measured(svg, &values, &FontMeasure::new(&options))?;
        usvg::Tree::from_str(&resolved, &options)
            .map(|_| ())
            .map_err(|error| AppError::Validation(format!("overlay template: {error}")))
    }

    pub fn render(
        &self,
        original: &[u8],
        template_svg: &str,
        values: &BTreeMap<&'static str, String>,
        input_hash: &str,
    ) -> AppResult<PosterOverlayRendered> {
        let mut base = decode_original(original)?;
        self.draw(&mut base, template_svg, values)?;

        let full = DynamicImage::ImageRgba8(base).to_rgb8();
        let mut variants = Vec::with_capacity(PosterOverlayVariant::ALL.len());
        for variant in PosterOverlayVariant::ALL {
            let image = match variant.width() {
                Some(target) if target < full.width() => {
                    let target_height = ((u64::from(full.height()) * u64::from(target))
                        / u64::from(full.width()))
                    .max(1) as u32;
                    image::imageops::resize(&full, target, target_height, FilterType::Lanczos3)
                }
                _ => full.clone(),
            };
            let jpeg = encode_jpeg(&DynamicImage::ImageRgb8(image))?;
            variants.push((variant, add_marker(jpeg, input_hash)?));
        }
        let output_hash = variants
            .iter()
            .find(|(variant, _)| *variant == PosterOverlayVariant::Full)
            .map(|(_, bytes)| blake3_hex(bytes))
            .unwrap_or_default();
        Ok(PosterOverlayRendered {
            variants,
            output_hash,
        })
    }
}

impl OverlayRenderer {
    /// Draw a template onto `base` in place.
    fn draw(
        &self,
        base: &mut RgbaImage,
        template_svg: &str,
        values: &BTreeMap<&'static str, String>,
    ) -> AppResult<()> {
        let options = self.options();
        let resolved =
            template::preprocess_measured(template_svg, values, &FontMeasure::new(&options))?;
        let tree = usvg::Tree::from_str(&resolved, &options)
            .map_err(|error| AppError::Validation(format!("overlay template: {error}")))?;
        let (width, height) = base.dimensions();
        let mut pixmap = tiny_skia::Pixmap::new(width, height)
            .ok_or_else(|| AppError::Validation("poster has invalid dimensions".into()))?;
        resvg::render(
            &tree,
            fit_transform(tree.size(), width, height),
            &mut pixmap.as_mut(),
        );
        composite(base, &pixmap);
        Ok(())
    }

    /// Render a template for the editor at `PREVIEW_WIDTH` x
    /// `PREVIEW_HEIGHT`. The background is cropped to fill; an unreadable or
    /// absent one falls back to a neutral placeholder. The JPEG carries no
    /// marker because it is never stored.
    pub fn render_preview(
        &self,
        background: Option<&[u8]>,
        template_svg: &str,
        values: &BTreeMap<&'static str, String>,
    ) -> AppResult<Vec<u8>> {
        template::validate(template_svg)?;
        let mut base = background
            .and_then(|bytes| decode_original(bytes).ok())
            .map(|image| {
                DynamicImage::ImageRgba8(image)
                    .resize_to_fill(PREVIEW_WIDTH, PREVIEW_HEIGHT, FilterType::Triangle)
                    .to_rgba8()
            })
            .unwrap_or_else(placeholder_poster);
        self.draw(&mut base, template_svg, values)?;
        encode_jpeg(&DynamicImage::ImageRgba8(base).to_rgb8().into())
    }
}

/// Neutral slate gradient standing in for a poster.
/// Measures text with the embedded font, by laying it out the way the
/// renderer will. Each word is measured once per render.
struct FontMeasure<'a> {
    options: &'a usvg::Options<'static>,
    cache: std::cell::RefCell<std::collections::HashMap<String, f64>>,
}

impl<'a> FontMeasure<'a> {
    fn new(options: &'a usvg::Options<'static>) -> Self {
        Self {
            options,
            cache: std::cell::RefCell::new(std::collections::HashMap::new()),
        }
    }
}

impl template::TextMeasure for FontMeasure<'_> {
    fn width_at_100(&self, text: &str) -> f64 {
        if let Some(width) = self.cache.borrow().get(text) {
            return *width;
        }
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="400"><text x="0" y="200" font-family="{EMBEDDED_FONT_FAMILY}" font-weight="700" font-size="100">{}</text></svg>"#,
            quick_xml::escape::escape(text)
        );
        let width = usvg::Tree::from_str(&svg, self.options)
            .map(|tree| f64::from(tree.root().abs_bounding_box().width()))
            .unwrap_or_else(|_| template::ApproximateMeasure.width_at_100(text));
        self.cache.borrow_mut().insert(text.to_string(), width);
        width
    }
}

fn placeholder_poster() -> RgbaImage {
    const TOP: [f32; 3] = [58.0, 68.0, 92.0];
    const BOTTOM: [f32; 3] = [16.0, 19.0, 28.0];
    RgbaImage::from_fn(PREVIEW_WIDTH, PREVIEW_HEIGHT, |_, y| {
        let t = y as f32 / (PREVIEW_HEIGHT - 1) as f32;
        let channel = |index: usize| (TOP[index] + (BOTTOM[index] - TOP[index]) * t).round() as u8;
        image::Rgba([channel(0), channel(1), channel(2), 255])
    })
}

fn decode_original(bytes: &[u8]) -> AppResult<RgbaImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| AppError::Validation(format!("unreadable poster: {error}")))?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    limits.max_image_width = Some(MAX_ORIGINAL_DIMENSION);
    limits.max_image_height = Some(MAX_ORIGINAL_DIMENSION);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| AppError::Validation(format!("failed to decode poster: {error}")))?;
    if image.width() == 0 || image.height() == 0 {
        return Err(AppError::Validation("poster has no pixels".into()));
    }
    Ok(image.to_rgba8())
}

/// Uniform scale that fits the template's viewBox inside the poster,
/// centred (SVG `xMidYMid meet`).
fn fit_transform(size: usvg::Size, width: u32, height: u32) -> tiny_skia::Transform {
    let scale = (width as f32 / size.width()).min(height as f32 / size.height());
    let tx = (width as f32 - size.width() * scale) / 2.0;
    let ty = (height as f32 - size.height() * scale) / 2.0;
    tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, tx, ty)
}

/// Source-over blend of the rendered overlay onto the poster.
fn composite(base: &mut RgbaImage, overlay: &tiny_skia::Pixmap) {
    for (pixel, over) in base.pixels_mut().zip(overlay.pixels()) {
        let alpha = u32::from(over.alpha());
        if alpha == 0 {
            continue;
        }
        let inverse = 255 - alpha;
        // Overlay channels are premultiplied, so only the base is scaled.
        let blend = |under: u8, over: u8| -> u8 {
            ((u32::from(over) * 255 + u32::from(under) * inverse + 127) / 255).min(255) as u8
        };
        pixel.0[0] = blend(pixel.0[0], over.red());
        pixel.0[1] = blend(pixel.0[1], over.green());
        pixel.0[2] = blend(pixel.0[2], over.blue());
        pixel.0[3] = 255;
    }
}

fn encode_jpeg(image: &DynamicImage) -> AppResult<Vec<u8>> {
    let mut out = Vec::new();
    image
        .write_with_encoder(JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY))
        .map_err(|error| AppError::Repository(format!("failed to encode poster: {error}")))?;
    Ok(out)
}

/// Insert the marker comment segment without re-encoding the image data.
pub fn add_marker(jpeg: Vec<u8>, input_hash: &str) -> AppResult<Vec<u8>> {
    let mut parsed = Jpeg::from_bytes(Bytes::from(jpeg)).map_err(|error| {
        AppError::Repository(format!("failed to parse encoded poster: {error}"))
    })?;
    let mut contents = MARKER_PREFIX.to_vec();
    contents.extend_from_slice(MARKER_VERSION.as_bytes());
    contents.push(b':');
    contents.extend_from_slice(input_hash.as_bytes());
    let segment = JpegSegment::new_with_contents(markers::COM, Bytes::from(contents));
    // Keep a leading JFIF/EXIF application segment first, as decoders expect.
    let position = parsed
        .segments()
        .iter()
        .position(|segment| !(markers::APP0..=markers::APP15).contains(&segment.marker()))
        .unwrap_or(0);
    parsed.segments_mut().insert(position, segment);
    let mut out = Vec::new();
    parsed
        .encoder()
        .write_to(&mut out)
        .map_err(|error| AppError::Repository(format!("failed to write poster marker: {error}")))?;
    Ok(out)
}

/// True when `bytes` is a JPEG carrying the overlay marker. Other formats
/// are never produced by the renderer, so they cannot carry it.
pub fn has_marker(bytes: &[u8]) -> bool {
    let Ok(parsed) = Jpeg::from_bytes(Bytes::copy_from_slice(bytes)) else {
        return false;
    };
    parsed
        .segments_by_marker(markers::COM)
        .any(|segment| segment.contents().starts_with(MARKER_PREFIX))
}

/// The input hash recorded in an output's marker, if any.
pub fn marker_input_hash(bytes: &[u8]) -> Option<String> {
    let parsed = Jpeg::from_bytes(Bytes::copy_from_slice(bytes)).ok()?;
    parsed
        .segments_by_marker(markers::COM)
        .find_map(|segment| {
            segment
                .contents()
                .strip_prefix(MARKER_PREFIX)
                .map(<[u8]>::to_vec)
        })
        .and_then(|rest| String::from_utf8(rest).ok())
        .and_then(|rest| {
            rest.strip_prefix(MARKER_VERSION)
                .and_then(|rest| rest.strip_prefix(':'))
                .map(str::to_string)
        })
}
