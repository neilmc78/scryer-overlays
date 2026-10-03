use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::Arc;

use async_trait::async_trait;
use image::{ImageFormat, Rgb, RgbImage};
use scryer_application::AppResult;
use scryer_application::overlays::{
    OverlayFields, OverlayMediaFacts, PosterOverlayEngine, PosterOverlayRenderRequest,
    PosterOverlayVariant,
};

use super::*;

fn empty_values() -> BTreeMap<&'static str, String> {
    TEMPLATE_FIELDS
        .iter()
        .map(|field| (*field, String::new()))
        .collect()
}

fn values(pairs: &[(&'static str, &str)]) -> BTreeMap<&'static str, String> {
    let mut values = empty_values();
    for (key, value) in pairs {
        values.insert(key, value.to_string());
    }
    values
}

fn poster(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
    let image = RgbImage::from_fn(width, height, |x, y| {
        Rgb([(x % 256) as u8, (y % 256) as u8, 96])
    });
    let mut out = Cursor::new(Vec::new());
    image
        .write_to(&mut out, format)
        .expect("encode poster fixture");
    out.into_inner()
}

// ── Template substitution and conditionals ────────────────────────────────

const HEADER: &str =
    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 150" data-scryer-version="1">"#;

fn template(body: &str) -> String {
    format!("{HEADER}{body}</svg>")
}

#[test]
fn placeholders_are_substituted_in_text_and_attributes() {
    let svg = template(r#"<text class="{{hdr}}">{{resolution_label}} / {{audio_label}}</text>"#);
    let out = preprocess(
        &svg,
        &values(&[
            ("hdr", "dv"),
            ("resolution_label", "4K"),
            ("audio_label", "DD+"),
        ]),
    )
    .expect("preprocess");
    assert!(out.contains(r#"class="dv""#), "{out}");
    assert!(out.contains(">4K / DD+</text>"), "{out}");
    assert!(
        !out.contains("data-scryer-version"),
        "contract attributes are stripped"
    );
}

#[test]
fn substituted_values_are_xml_escaped() {
    let svg = template("<text>{{edition_label}}</text>");
    let out =
        preprocess(&svg, &values(&[("edition_label", "<CUT & \"FINAL\">")])).expect("preprocess");
    assert!(out.contains("&lt;CUT &amp; &quot;FINAL&quot;&gt;"), "{out}");
}

#[test]
fn existing_entities_in_templates_are_left_alone() {
    let svg = template("<text>R&amp;D {{resolution}}</text>");
    let out = preprocess(&svg, &values(&[("resolution", "1080p")])).expect("preprocess");
    assert!(out.contains("R&amp;D 1080p"), "{out}");
}

#[test]
fn conditional_elements_render_only_when_the_field_matches() {
    let svg = template(concat!(
        r#"<g data-scryer-if="hdr=dv|hdr10plus"><text>HDR</text></g>"#,
        r#"<g data-scryer-if="hdr"><text>ANY</text></g>"#,
        r#"<g data-scryer-unless="hdr=dv"><text>NOT-DV</text></g>"#,
        r#"<rect data-scryer-if="resolution=2160p;hdr!=sdr" width="1" height="1"/>"#,
    ));

    let dv = preprocess(&svg, &values(&[("hdr", "dv"), ("resolution", "2160p")])).unwrap();
    assert!(dv.contains("HDR") && dv.contains("ANY") && !dv.contains("NOT-DV"));
    assert!(dv.contains("<rect"), "all clauses hold");

    let sdr = preprocess(&svg, &values(&[("hdr", "sdr"), ("resolution", "2160p")])).unwrap();
    assert!(!sdr.contains(">HDR<") && sdr.contains("ANY") && sdr.contains("NOT-DV"));
    assert!(
        !sdr.contains("<rect"),
        "one failing clause excludes the element"
    );

    let none = preprocess(&svg, &empty_values()).unwrap();
    assert!(!none.contains("ANY"), "a bare field tests for presence");
}

#[test]
fn excluded_subtrees_are_dropped_with_their_descendants() {
    let svg = template(
        r#"<g data-scryer-if="hdr=dv"><g><text>{{hdr_label}}</text></g></g><text>after</text>"#,
    );
    let out = preprocess(&svg, &empty_values()).unwrap();
    assert!(!out.contains("<g>"), "{out}");
    assert!(out.contains("after"), "{out}");
}

#[test]
fn contract_violations_are_rejected() {
    let cases = [
        ("unknown placeholder", template("<text>{{rating}}</text>")),
        ("unknown condition field", template(r#"<g data-scryer-if="rating=pg"/>"#)),
        ("empty condition", template(r#"<g data-scryer-if=" "/>"#)),
        ("unterminated placeholder", template("<text>{{hdr</text>")),
        ("image element", template(r#"<image href="file:///etc/passwd"/>"#)),
        (
            "missing version",
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"/>"#.to_string(),
        ),
        (
            "future version",
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1" data-scryer-version="99"/>"#
                .to_string(),
        ),
        ("not svg", "<html data-scryer-version=\"1\"/>".to_string()),
    ];
    for (name, svg) in cases {
        assert!(validate(&svg).is_err(), "{name} should be rejected");
    }
}

#[test]
fn validation_checks_conditions_inside_branches_that_are_not_taken() {
    let svg = template(r#"<g data-scryer-if="hdr=dv"><g data-scryer-if="ratng"/></g>"#);
    assert!(validate(&svg).is_err());
}

#[test]
fn builtin_template_satisfies_the_contract() {
    OverlayRenderer::new()
        .validate(BUILTIN_TEMPLATE)
        .expect("built-in template must validate");
}

// ── Marker detection ──────────────────────────────────────────────────────

#[test]
fn marker_round_trips_without_reencoding() {
    let jpeg = poster(ImageFormat::Jpeg, 40, 60);
    assert!(!has_marker(&jpeg));
    let marked = add_marker(jpeg.clone(), "abc123").expect("add marker");
    assert!(has_marker(&marked));
    assert_eq!(marker_input_hash(&marked).as_deref(), Some("abc123"));

    let original_pixels = image::load_from_memory(&jpeg).unwrap().to_rgb8();
    let marked_pixels = image::load_from_memory(&marked).unwrap().to_rgb8();
    assert_eq!(original_pixels, marked_pixels, "entropy data is untouched");
}

#[test]
fn non_jpeg_and_unrelated_comments_are_not_markers() {
    assert!(!has_marker(&poster(ImageFormat::Png, 10, 10)));
    assert!(!has_marker(b"not an image"));
    assert!(!has_marker(&[]));

    let jpeg = poster(ImageFormat::Jpeg, 10, 10);
    let mut parsed = img_parts::jpeg::Jpeg::from_bytes(img_parts::Bytes::from(jpeg)).unwrap();
    parsed.segments_mut().insert(
        1,
        img_parts::jpeg::JpegSegment::new_with_contents(
            img_parts::jpeg::markers::COM,
            img_parts::Bytes::from_static(b"made with some editor"),
        ),
    );
    let mut commented = Vec::new();
    parsed.encoder().write_to(&mut commented).unwrap();
    assert!(!has_marker(&commented));
}

// ── Rendering ─────────────────────────────────────────────────────────────

#[test]
fn render_composites_badges_and_marks_every_variant() {
    let original = poster(ImageFormat::Png, 500, 750);
    let fields = OverlayFields::aggregate(&[OverlayMediaFacts {
        video_width: Some(3840),
        video_height: Some(2160),
        video_hdr_format: Some("Dolby Vision".into()),
        audio_codec: Some("eac3".into()),
        audio_profile: Some("Dolby Digital Plus + Dolby Atmos".into()),
        audio_channels: Some(6),
        ..OverlayMediaFacts::default()
    }]);
    let rendered = OverlayRenderer::new()
        .render(
            &original,
            BUILTIN_TEMPLATE,
            &fields.template_values(),
            "input-1",
        )
        .expect("render");

    assert_eq!(rendered.variants.len(), 3);
    for (variant, bytes) in &rendered.variants {
        assert!(has_marker(bytes), "{variant:?} carries the marker");
        assert_eq!(marker_input_hash(bytes).as_deref(), Some("input-1"));
        let decoded = image::load_from_memory(bytes).unwrap();
        match variant.width() {
            Some(width) => assert_eq!(decoded.width(), width),
            None => assert_eq!((decoded.width(), decoded.height()), (500, 750)),
        }
    }

    // The resolution badge sits over the top-left corner, which is black in
    // the fixture at (0,0) and must now be the badge background instead.
    let (_, full) = &rendered.variants[0];
    let full = image::load_from_memory(full).unwrap().to_rgb8();
    let source = image::load_from_memory(&original).unwrap().to_rgb8();
    assert_ne!(
        full.get_pixel(60, 40),
        source.get_pixel(60, 40),
        "badge drawn"
    );
    assert_eq!(
        rendered.output_hash,
        scryer_application::overlays::blake3_hex(&rendered.variants[0].1)
    );
}

#[test]
fn text_renders_from_the_embedded_font() {
    // A template that is only text: if no font resolved, nothing is drawn
    // and the poster comes back unchanged.
    let svg = template(
        r##"<rect width="100" height="150" fill="#000"/><text x="5" y="80" font-size="40" fill="#fff">4K</text>"##,
    );
    let original = poster(ImageFormat::Png, 100, 150);
    let rendered = OverlayRenderer::new()
        .render(&original, &svg, &empty_values(), "h")
        .expect("render");
    let full = image::load_from_memory(&rendered.variants[0].1)
        .unwrap()
        .to_luma8();
    assert!(full.pixels().any(|pixel| pixel.0[0] > 200), "glyphs drawn");
}

// ── Engine file ownership (synthetic fixtures only) ───────────────────────

struct FixedFetch(Vec<u8>);

#[async_trait]
impl OverlaySourceFetch for FixedFetch {
    async fn fetch(&self, _source_url: &str) -> AppResult<Vec<u8>> {
        Ok(self.0.clone())
    }
}

fn engine(dir: &std::path::Path) -> OverlayEngine {
    OverlayEngine::new(dir, Arc::new(FixedFetch(Vec::new()))).expect("engine")
}

async fn render_and_write(engine: &OverlayEngine, title_id: &str) {
    let rendered = engine
        .render(PosterOverlayRenderRequest {
            original: poster(ImageFormat::Jpeg, 300, 450),
            template_svg: BUILTIN_TEMPLATE.to_string(),
            values: empty_values(),
            input_hash: "h".into(),
        })
        .await
        .expect("render");
    engine
        .write_outputs(title_id, &rendered)
        .await
        .expect("write");
}

#[tokio::test]
async fn remove_outputs_deletes_only_that_titles_rendered_files() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    engine
        .store_original("title-a", poster(ImageFormat::Jpeg, 30, 45))
        .await
        .unwrap();
    render_and_write(&engine, "title-a").await;
    render_and_write(&engine, "title-b").await;
    let output_a = dir.path().join("overlays/output/title-a");
    let stray = output_a.join("keep-me.txt");
    std::fs::write(&stray, b"not ours").unwrap();
    let unrelated = dir.path().join("library.db");
    std::fs::write(&unrelated, b"user data").unwrap();

    engine.remove_outputs("title-a").await.unwrap();

    for variant in PosterOverlayVariant::ALL {
        assert!(
            engine
                .read_output("title-a", variant)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            engine
                .read_output("title-b", variant)
                .await
                .unwrap()
                .is_some()
        );
    }
    assert!(
        stray.exists(),
        "unknown files survive; the directory is kept"
    );
    assert!(unrelated.exists());
    assert!(
        engine.read_original("title-a").await.unwrap().is_some(),
        "originals are kept"
    );
}

#[tokio::test]
async fn remove_outputs_removes_the_empty_directory_and_tolerates_absence() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    render_and_write(&engine, "title-a").await;
    engine.remove_outputs("title-a").await.unwrap();
    assert!(!dir.path().join("overlays/output/title-a").exists());
    engine.remove_outputs("title-a").await.unwrap();
    engine.remove_outputs("never-rendered").await.unwrap();
}

#[tokio::test]
async fn store_original_replaces_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    engine
        .store_original("title-a", vec![1, 2, 3])
        .await
        .unwrap();
    engine.store_original("title-a", vec![4, 5]).await.unwrap();
    assert_eq!(
        engine.read_original("title-a").await.unwrap(),
        Some(vec![4, 5])
    );
    let leftovers = std::fs::read_dir(dir.path().join("overlays/originals"))
        .unwrap()
        .count();
    assert_eq!(leftovers, 1, "no temp file left behind");
}

#[tokio::test]
async fn unsafe_title_ids_never_reach_the_filesystem() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let victim = dir.path().join("victim.txt");
    std::fs::write(&victim, b"keep").unwrap();
    for title_id in ["", "..", "../victim.txt", "a/b", "a\\b", "title\0id"] {
        assert!(
            engine.remove_outputs(title_id).await.is_err(),
            "{title_id:?}"
        );
        assert!(engine.store_original(title_id, vec![1]).await.is_err());
        assert!(
            engine
                .read_output(title_id, PosterOverlayVariant::Full)
                .await
                .is_err()
        );
    }
    assert!(victim.exists());
}

#[test]
fn tmdb_sources_are_upsized_and_others_left_alone() {
    assert_eq!(
        overlay_source_url("https://image.tmdb.org/t/p/w500/abc.jpg"),
        "https://image.tmdb.org/t/p/w780/abc.jpg"
    );
    assert_eq!(
        overlay_source_url("https://artworks.thetvdb.com/banners/x.jpg"),
        "https://artworks.thetvdb.com/banners/x.jpg"
    );
}

fn preview_template() -> String {
    // A white block over the top-left quarter of the built-in template's
    // 1000x1500 viewBox, shown only when a resolution is set.
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1500" data-scryer-version="1"><rect data-scryer-if="resolution" x="0" y="0" width="500" height="750" fill="#ffffff"/></svg>"##.to_string()
}

#[test]
fn preview_draws_on_the_placeholder_without_a_marker() {
    let renderer = OverlayRenderer::new();
    let jpeg = renderer
        .render_preview(
            None,
            &preview_template(),
            &values(&[("resolution", "2160p")]),
        )
        .expect("preview");

    assert!(
        !has_marker(&jpeg),
        "previews are never stored, so carry no marker"
    );
    let image = image::load_from_memory(&jpeg).unwrap().to_rgb8();
    assert_eq!(image.dimensions(), (PREVIEW_WIDTH, PREVIEW_HEIGHT));
    // viewBox (500, 750) is preview pixel (250, 375): the block covers the
    // top-left quarter and nothing else.
    assert!(
        image
            .get_pixel(100, 100)
            .0
            .iter()
            .all(|channel| *channel > 240)
    );
    assert!(
        image
            .get_pixel(400, 600)
            .0
            .iter()
            .all(|channel| *channel < 120)
    );
}

#[test]
fn preview_skips_elements_whose_sample_value_is_unset() {
    let jpeg = OverlayRenderer::new()
        .render_preview(None, &preview_template(), &empty_values())
        .expect("preview");
    let image = image::load_from_memory(&jpeg).unwrap().to_rgb8();
    assert!(
        image
            .get_pixel(100, 100)
            .0
            .iter()
            .all(|channel| *channel < 120)
    );
}

#[test]
fn preview_crops_a_library_poster_to_the_template_shape() {
    let square = poster(ImageFormat::Png, 900, 900);
    let jpeg = OverlayRenderer::new()
        .render_preview(Some(&square), &preview_template(), &empty_values())
        .expect("preview");
    let image = image::load_from_memory(&jpeg).unwrap();
    assert_eq!(
        (image.width(), image.height()),
        (PREVIEW_WIDTH, PREVIEW_HEIGHT)
    );
}

#[test]
fn preview_falls_back_to_the_placeholder_for_an_unreadable_poster() {
    let jpeg = OverlayRenderer::new()
        .render_preview(Some(b"not an image"), &preview_template(), &empty_values())
        .expect("an unreadable background must not fail the preview");
    let image = image::load_from_memory(&jpeg).unwrap();
    assert_eq!(
        (image.width(), image.height()),
        (PREVIEW_WIDTH, PREVIEW_HEIGHT)
    );
}

#[test]
fn preview_rejects_templates_that_break_the_contract() {
    let error = OverlayRenderer::new()
        .render_preview(None, &template("<text>{{rating}}</text>"), &empty_values())
        .expect_err("unknown field");
    assert!(error.to_string().contains("unknown field"), "{error}");
}

#[test]
fn conditions_on_values_a_field_never_takes_are_rejected() {
    for (condition, message) in [
        ("resolution=4K", "use one of 2160p, 1080p, 720p, sd"),
        ("resolution!=4k", "resolution is never \"4k\""),
        ("hdr=dv|dolby", "hdr is never \"dolby\""),
        ("audio_codec=atmos", "truehd_atmos"),
        ("resolution=2160p;hdr=HDR10", "hdr is never \"HDR10\""),
        ("audio_channels=9.2", "or a count such as 10ch"),
        ("edition=Director's Cut", "\"directors_cut\""),
    ] {
        let svg = template(&format!(
            r#"<g data-scryer-if="{condition}"><text>x</text></g>"#
        ));
        let error = validate(&svg).expect_err(condition).to_string();
        assert!(error.contains(message), "{condition}: {error}");
    }
}

#[test]
fn conditions_on_real_values_tokens_and_labels_are_accepted() {
    for condition in [
        "resolution",
        "resolution=2160p",
        "resolution=2160p|1080p",
        "resolution_label=4K",
        "hdr=hdr10plus|hdr10|hlg",
        "hdr_label=DOLBY VISION",
        "audio_codec!=other",
        "audio_label=DD+ ATMOS",
        "audio_channels=7.1|10ch",
        "edition=directors_cut",
    ] {
        let svg = template(&format!(
            r#"<g data-scryer-if="{condition}" data-scryer-unless="resolution=sd"><text>x</text></g>"#
        ));
        validate(&svg).unwrap_or_else(|error| panic!("{condition}: {error}"));
    }
}

#[test]
fn series_status_conditions_use_tokens_or_labels() {
    for condition in [
        "series_status=ended|canceled",
        "series_status_label=CONTINUING",
    ] {
        validate(&template(&format!(
            r#"<g data-scryer-if="{condition}"><text>{{{{series_status_label}}}}</text></g>"#
        )))
        .unwrap_or_else(|error| panic!("{condition}: {error}"));
    }
    let error = validate(&template(
        r#"<g data-scryer-if="series_status=Ended"><text>x</text></g>"#,
    ))
    .expect_err("label used as token");
    assert!(
        error
            .to_string()
            .contains("use one of continuing, upcoming, ended, canceled"),
        "{error}"
    );
}

#[test]
fn edition_conditions_match_any_of_a_titles_editions() {
    let both = values(&[
        ("edition", "directors_cut,theatrical"),
        ("edition_label", "DIRECTOR'S CUT / THEATRICAL"),
    ]);
    for (condition, expected) in [
        ("edition", true),
        ("edition=theatrical", true),
        ("edition=extended|directors_cut", true),
        ("edition=extended", false),
        ("edition!=theatrical", false),
        ("edition!=extended", true),
        ("edition_label=THEATRICAL", true),
    ] {
        assert_eq!(
            evaluate_condition(condition, &both).unwrap(),
            expected,
            "{condition}"
        );
    }
    let none = empty_values();
    assert!(!evaluate_condition("edition", &none).unwrap());
    assert!(!evaluate_condition("edition=theatrical", &none).unwrap());
    assert!(evaluate_condition("edition!=theatrical", &none).unwrap());
}
