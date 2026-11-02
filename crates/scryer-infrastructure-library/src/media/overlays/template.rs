//! Template preprocessing: resolves `data-scryer-if` / `data-scryer-unless`
//! conditionals and `{{field}}` placeholders, producing plain SVG for resvg.
//!
//! The contract is specified in `docs/poster-overlays.md`. Everything here is
//! strict: an unknown field, an unsupported version or a malformed condition
//! is an error rather than a silently blank badge.

use std::borrow::Cow;
use std::collections::BTreeMap;

use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesStart, BytesText, Event};
use quick_xml::name::QName;
use quick_xml::{Reader, Writer};
use scryer_application::overlays::TEMPLATE_SPEC_VERSION;
use scryer_application::{AppError, AppResult};

pub const MAX_TEMPLATE_BYTES: usize = 256 * 1024;

const VERSION_ATTR: &str = "data-scryer-version";
const IF_ATTR: &str = "data-scryer-if";
const UNLESS_ATTR: &str = "data-scryer-unless";
const STACK_ATTR: &str = "data-scryer-stack";
/// On `<text>`: `x y width height`, the box the text must fit in. The text
/// shrinks from its own `font-size`, wraps onto up to
/// `data-scryer-fit-lines` lines, and is centred vertically in the box.
const FIT_ATTR: &str = "data-scryer-fit";
const FIT_LINES_ATTR: &str = "data-scryer-fit-lines";
pub const MAX_FIT_LINES: usize = 4;
/// Distance between wrapped lines, as a multiple of the font size.
const FIT_LINE_HEIGHT: f64 = 1.15;
/// Half of Inter's cap height: a baseline this far below a point centres
/// capitals on it.
const FIT_CAP_CENTER: f64 = 0.36;
const MIN_FIT_FONT_SIZE: f64 = 6.0;
/// On `<text>`: `true` draws a format's logo (Dolby Atmos, DTS-HD, HDR10+,
/// IMAX, ...) in place of the label it stands for, where one is bundled.
/// Labels without a logo stay text.
const LOGOS_ATTR: &str = "data-scryer-logos";
/// A format logo's height as a share of the font size: a little over Inter's
/// cap height, so the logo's lettering sits level with the text beside it.
const FORMAT_LOGO_HEIGHT: f64 = 0.8;
/// Room kept clear on each side of a format logo, as a share of the font
/// size: widely spaced wordmarks such as Atmos's would otherwise run into
/// the next word.
const FORMAT_LOGO_MARGIN: f64 = 0.15;

/// Measures text for fitting. Widths are at font size 100 and exclude
/// letter spacing.
pub trait TextMeasure {
    fn width_at_100(&self, text: &str) -> f64;
}

/// A rough measure (0.62em per character) for checking a template without
/// the renderer's font.
pub struct ApproximateMeasure;

impl TextMeasure for ApproximateMeasure {
    fn width_at_100(&self, text: &str) -> f64 {
        text.chars().count() as f64 * 62.0
    }
}
const STEP_ATTR: &str = "data-scryer-step";

/// `<use href="#scryer-logo-imdb"/>` draws a bundled logo. The renderer adds
/// the referenced logos to the document; templates never carry them.
pub const LOGO_ID_PREFIX: &str = "scryer-logo-";

/// Logos a template may reference, as `(name, svg)`. Each is a plain vector
/// SVG whose internal ids already carry its own `scryer-logo-<name>-` prefix.
pub const LOGOS: &[(&str, &str)] = &[
    (
        "anilist",
        include_str!("../../../assets/overlays/logos/anilist.svg"),
    ),
    (
        "imdb",
        include_str!("../../../assets/overlays/logos/imdb.svg"),
    ),
    (
        "letterboxd",
        include_str!("../../../assets/overlays/logos/letterboxd.svg"),
    ),
    (
        "mal",
        include_str!("../../../assets/overlays/logos/mal.svg"),
    ),
    (
        "mdblist",
        include_str!("../../../assets/overlays/logos/mdblist.svg"),
    ),
    (
        "metacritic",
        include_str!("../../../assets/overlays/logos/metacritic.svg"),
    ),
    (
        "popcornmeter",
        include_str!("../../../assets/overlays/logos/popcornmeter.svg"),
    ),
    (
        "rottentomatoes",
        include_str!("../../../assets/overlays/logos/rottentomatoes.svg"),
    ),
    (
        "simkl",
        include_str!("../../../assets/overlays/logos/simkl.svg"),
    ),
    (
        "tmdb",
        include_str!("../../../assets/overlays/logos/tmdb.svg"),
    ),
    (
        "trakt",
        include_str!("../../../assets/overlays/logos/trakt.svg"),
    ),
    (
        "tvdb",
        include_str!("../../../assets/overlays/logos/tvdb.svg"),
    ),
    // Format logos: single-colour paths with no fill of their own, so they
    // take the `fill` of the `<use>` drawing them (black when it has none):
    // with `data-scryer-logos`, the colour of the text they replace. See
    // `logos/formats/NOTICE.txt`.
    (
        "dolby-atmos",
        include_str!("../../../assets/overlays/logos/formats/dolby-atmos.svg"),
    ),
    (
        "dolby-digital-plus",
        include_str!("../../../assets/overlays/logos/formats/dolby-digital-plus.svg"),
    ),
    (
        "dolby-truehd",
        include_str!("../../../assets/overlays/logos/formats/dolby-truehd.svg"),
    ),
    (
        "dolby-vision",
        include_str!("../../../assets/overlays/logos/formats/dolby-vision.svg"),
    ),
    (
        "dts-hd",
        include_str!("../../../assets/overlays/logos/formats/dts-hd.svg"),
    ),
    (
        "hdr",
        include_str!("../../../assets/overlays/logos/formats/hdr.svg"),
    ),
    (
        "hdr10plus",
        include_str!("../../../assets/overlays/logos/formats/hdr10plus.svg"),
    ),
    (
        "imax",
        include_str!("../../../assets/overlays/logos/formats/imax.svg"),
    ),
];

/// Element names a template may not contain. Templates are vector badges;
/// nothing may pull in outside content.
const FORBIDDEN_ELEMENTS: &[&str] = &["image", "foreignObject", "script", "feImage"];

pub use scryer_application::overlays::TEMPLATE_FIELDS;
use scryer_application::overlays::{
    CHANNEL_LAYOUTS, condition_value_set, condition_values, edition_token,
    is_possible_condition_value,
};

fn invalid(message: impl Into<String>) -> AppError {
    AppError::Validation(format!("overlay template: {}", message.into()))
}

/// Resolve conditionals and placeholders against `values`, fitting text
/// with an approximate measure.
pub fn preprocess(svg: &str, values: &BTreeMap<&'static str, String>) -> AppResult<String> {
    preprocess_measured(svg, values, &ApproximateMeasure)
}

/// Resolve conditionals and placeholders against `values`, fitting text
/// with `measure`.
pub fn preprocess_measured(
    svg: &str,
    values: &BTreeMap<&'static str, String>,
    measure: &dyn TextMeasure,
) -> AppResult<String> {
    if svg.len() > MAX_TEMPLATE_BYTES {
        return Err(invalid(format!(
            "template exceeds {MAX_TEMPLATE_BYTES} bytes"
        )));
    }
    let mut reader = Reader::from_str(svg);
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Vec::with_capacity(svg.len()));
    // Depth inside an element whose condition failed; 0 when emitting.
    let mut skip_depth = 0usize;
    let mut saw_root = false;
    // One entry per open emitted element: its stack layout, if it is one.
    let mut open: Vec<Option<Stack>> = Vec::new();
    let mut logos: Vec<&'static str> = Vec::new();

    loop {
        let event = reader
            .read_event()
            .map_err(|error| invalid(format!("malformed XML: {error}")))?;
        match event {
            Event::Eof => break,
            Event::Start(start) => {
                if skip_depth > 0 {
                    skip_depth += 1;
                    continue;
                }
                check_element(&start, &mut saw_root)?;
                let fit = text_fit(&start)?;
                let format_logos = logo_text(&start)?;
                match emit_element(&start, values, &mut open, &mut logos)? {
                    Some(element) if format_logos => {
                        let pieces = text_pieces(&mut reader, values)?;
                        write_logo_text(&mut writer, element, fit, &pieces, measure, &mut logos)?;
                    }
                    Some(element) => match fit {
                        Some(fit) => {
                            let content = fitted_text_content(&mut reader, values)?;
                            write_fitted_text(&mut writer, element, fit, &content, measure)?;
                        }
                        None => {
                            open.push(stack_layout(&start)?);
                            write(&mut writer, Event::Start(element))?;
                        }
                    },
                    None => skip_depth = 1,
                }
            }
            Event::Empty(start) => {
                if skip_depth > 0 {
                    continue;
                }
                check_element(&start, &mut saw_root)?;
                text_fit(&start)?;
                logo_text(&start)?;
                if let Some(element) = emit_element(&start, values, &mut open, &mut logos)? {
                    write(&mut writer, Event::Empty(element))?;
                }
            }
            Event::End(end) => {
                if skip_depth > 0 {
                    skip_depth -= 1;
                    continue;
                }
                open.pop();
                if open.is_empty() && !logos.is_empty() {
                    write_logo_defs(&mut writer, &logos)?;
                }
                write(&mut writer, Event::End(end))?;
            }
            Event::Text(text) => {
                if skip_depth > 0 {
                    continue;
                }
                let substituted = substitute(&text, values)?;
                write(
                    &mut writer,
                    Event::Text(BytesText::from_escaped(substituted)),
                )?;
            }
            Event::DocType(_) => return Err(invalid("DOCTYPE declarations are not allowed")),
            Event::Comment(_) | Event::PI(_) => {}
            other => {
                if skip_depth == 0 {
                    write(&mut writer, other)?;
                }
            }
        }
    }
    if !saw_root {
        return Err(invalid("missing <svg> root element"));
    }
    String::from_utf8(writer.into_inner()).map_err(|_| invalid("output is not UTF-8"))
}

/// Check a template against the contract using empty values, so every
/// condition and placeholder is parsed at least once.
pub fn validate(svg: &str) -> AppResult<()> {
    let values = TEMPLATE_FIELDS
        .iter()
        .map(|field| (*field, String::new()))
        .collect::<BTreeMap<_, _>>();
    preprocess(svg, &values)?;
    // Conditions inside skipped subtrees are not evaluated above; walk them
    // explicitly so a typo in a rarely-true branch is still caught.
    let mut reader = Reader::from_str(svg);
    loop {
        match reader
            .read_event()
            .map_err(|error| invalid(format!("malformed XML: {error}")))?
        {
            Event::Eof => return Ok(()),
            Event::Start(start) | Event::Empty(start) => {
                stack_layout(&start)?;
                text_fit(&start)?;
                logo_text(&start)?;
                referenced_logo(&start)?;
                check_reserved_id(&start)?;
                for attribute in start.attributes() {
                    let attribute =
                        attribute.map_err(|error| invalid(format!("bad attribute: {error}")))?;
                    let key = attribute.key.0;
                    if key == IF_ATTR || key == UNLESS_ATTR {
                        evaluate_condition(&attribute.value, &values)?;
                        check_condition_values(&attribute.value)?;
                    } else if key == STACK_ATTR
                        || key == STEP_ATTR
                        || key == FIT_ATTR
                        || key == FIT_LINES_ATTR
                        || key == LOGOS_ATTR
                    {
                        // Checked together below.
                    } else {
                        substitute(&attribute.value, &values)?;
                    }
                }
            }
            Event::Text(text) => {
                substitute(&text, &values)?;
            }
            _ => {}
        }
    }
}

fn write(writer: &mut Writer<Vec<u8>>, event: Event<'_>) -> AppResult<()> {
    writer
        .write_event(event)
        .map_err(|error| invalid(format!("failed to write SVG: {error}")))
}

fn check_element(start: &BytesStart<'_>, saw_root: &mut bool) -> AppResult<()> {
    let name = start.local_name();
    let name = name.as_ref();
    if FORBIDDEN_ELEMENTS
        .iter()
        .any(|forbidden| forbidden.eq_ignore_ascii_case(name))
    {
        return Err(invalid(format!("<{name}> elements are not allowed")));
    }
    if *saw_root {
        return Ok(());
    }
    if name != "svg" {
        return Err(invalid("the root element must be <svg>"));
    }
    *saw_root = true;
    let version = start
        .try_get_attribute(VERSION_ATTR)
        .map_err(|error| invalid(format!("bad attribute: {error}")))?
        .ok_or_else(|| invalid(format!("the root <svg> must declare {VERSION_ATTR}")))?;
    match version.value.trim().parse::<u32>() {
        Ok(version) if version >= 1 && version <= TEMPLATE_SPEC_VERSION => Ok(()),
        _ => Err(invalid(format!(
            "unsupported {VERSION_ATTR} \"{}\"; this build supports 1 to {TEMPLATE_SPEC_VERSION}",
            version.value
        ))),
    }
}

/// The box and line limit of a fitted `<text>`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TextFit {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    lines: usize,
}

/// The fit an element declares, checked even where it never renders.
fn text_fit(start: &BytesStart<'_>) -> AppResult<Option<TextFit>> {
    let attribute = |name: &str| -> AppResult<Option<String>> {
        Ok(start
            .try_get_attribute(name)
            .map_err(|error| invalid(format!("bad attribute: {error}")))?
            .map(|attribute| attribute.value.trim().to_string()))
    };
    let fit = attribute(FIT_ATTR)?;
    let lines = attribute(FIT_LINES_ATTR)?;
    let Some(fit) = fit else {
        if lines.is_some() {
            return Err(invalid(format!("{FIT_LINES_ATTR} needs {FIT_ATTR}")));
        }
        return Ok(None);
    };
    if start.local_name().as_ref() != "text" {
        return Err(invalid(format!("{FIT_ATTR} is only allowed on <text>")));
    }
    let numbers = fit
        .split([' ', ','])
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<f64>().ok().filter(|value| value.is_finite()))
        .collect::<Option<Vec<_>>>()
        .filter(|numbers| numbers.len() == 4 && numbers[2] > 0.0 && numbers[3] > 0.0)
        .ok_or_else(|| {
            invalid(format!(
                "{FIT_ATTR} must be \"x y width height\" with a positive size, not \"{fit}\""
            ))
        })?;
    let lines = match lines {
        None => 1,
        Some(lines) => lines
            .parse::<usize>()
            .ok()
            .filter(|lines| (1..=MAX_FIT_LINES).contains(lines))
            .ok_or_else(|| {
                invalid(format!(
                    "{FIT_LINES_ATTR} must be 1 to {MAX_FIT_LINES}, not \"{lines}\""
                ))
            })?,
    };
    Ok(Some(TextFit {
        x: numbers[0],
        y: numbers[1],
        width: numbers[2],
        height: numbers[3],
        lines,
    }))
}

/// The text inside a fitted `<text>`, placeholders resolved and unescaped.
/// It may hold only text: no child elements.
fn fitted_text_content(
    reader: &mut Reader<&[u8]>,
    values: &BTreeMap<&'static str, String>,
) -> AppResult<String> {
    let mut raw = String::new();
    loop {
        match reader
            .read_event()
            .map_err(|error| invalid(format!("malformed XML: {error}")))?
        {
            Event::Text(text) => raw.push_str(&substitute(&text, values)?),
            Event::GeneralRef(reference) => {
                raw.push('&');
                raw.push_str(reference.as_ref());
                raw.push(';');
            }
            Event::Comment(_) => {}
            Event::End(_) => break,
            Event::Eof => return Err(invalid("unterminated <text>")),
            _ => {
                return Err(invalid(format!(
                    "a <text> with {FIT_ATTR} may contain only text and placeholders"
                )));
            }
        }
    }
    quick_xml::escape::unescape(&raw)
        .map(|text| text.into_owned())
        .map_err(|error| invalid(format!("bad text: {error}")))
}

fn attribute_number(element: &BytesStart<'_>, name: &str) -> AppResult<Option<f64>> {
    Ok(element
        .try_get_attribute(name)
        .map_err(|error| invalid(format!("bad attribute: {error}")))?
        .and_then(|attribute| {
            attribute
                .value
                .trim()
                .trim_end_matches("px")
                .parse::<f64>()
                .ok()
        })
        .filter(|value| value.is_finite()))
}

/// Writes a fitted `<text>`: the largest font size up to its own at which
/// the words wrap into the allowed lines inside the box, one `<tspan>` per
/// line, the block centred vertically.
fn write_fitted_text(
    writer: &mut Writer<Vec<u8>>,
    element: BytesStart<'static>,
    fit: TextFit,
    content: &str,
    measure: &dyn TextMeasure,
) -> AppResult<()> {
    let max_size = attribute_number(&element, "font-size")?.unwrap_or(16.0);
    let spacing = attribute_number(&element, "letter-spacing")?.unwrap_or(0.0);
    let x = attribute_number(&element, "x")?.unwrap_or(fit.x);
    let (size, lines) = fit_words(content, max_size, spacing, fit, measure);

    let mut rewritten = BytesStart::new("text");
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|error| invalid(format!("bad attribute: {error}")))?;
        if !matches!(attribute.key.0, "font-size" | "y") {
            rewritten.push_attribute(attribute);
        }
    }
    let line_height = size * FIT_LINE_HEIGHT;
    let center = fit.y + fit.height / 2.0;
    let first =
        center - (lines.len().saturating_sub(1) as f64) * line_height / 2.0 + size * FIT_CAP_CENTER;
    rewritten.push_attribute(("font-size", format_number(size).as_str()));
    rewritten.push_attribute(("y", format_number(first).as_str()));
    write(writer, Event::Start(rewritten))?;
    for (index, line) in lines.iter().enumerate() {
        let mut tspan = BytesStart::new("tspan");
        tspan.push_attribute(("x", format_number(x).as_str()));
        tspan.push_attribute((
            "y",
            format_number(first + index as f64 * line_height).as_str(),
        ));
        write(writer, Event::Start(tspan))?;
        write(writer, Event::Text(BytesText::new(line)))?;
        write(
            writer,
            Event::End(quick_xml::events::BytesEnd::new("tspan")),
        )?;
    }
    write(writer, Event::End(quick_xml::events::BytesEnd::new("text")))
}

fn format_number(value: f64) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    format!("{rounded}")
}

/// The font size and lines for `content` in `fit`.
fn fit_words(
    content: &str,
    max_size: f64,
    spacing: f64,
    fit: TextFit,
    measure: &dyn TextMeasure,
) -> (f64, Vec<String>) {
    let words = content
        .split_whitespace()
        .map(Word::Text)
        .collect::<Vec<_>>();
    let (size, lines) = fit_layout(&words, max_size, spacing, fit, measure);
    let lines = lines
        .into_iter()
        .map(|line| {
            line.into_iter()
                .map(|index| match words[index] {
                    Word::Text(word) => word,
                    Word::Logo { .. } => "",
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    (size, lines)
}

/// One unit of a laid-out line: a word of text or a format logo, which
/// wraps and shrinks like a word.
#[derive(Clone, Copy, Debug)]
enum Word<'a> {
    Text(&'a str),
    Logo { name: &'static str, aspect: f64 },
}

/// A word's width at font size 100, and the letters letter spacing adds to.
fn word_metrics(word: &Word<'_>, measure: &dyn TextMeasure) -> (f64, f64) {
    match word {
        Word::Text(text) => (measure.width_at_100(text), text.chars().count() as f64),
        Word::Logo { aspect, .. } => (
            (FORMAT_LOGO_HEIGHT * aspect + 2.0 * FORMAT_LOGO_MARGIN) * 100.0,
            0.0,
        ),
    }
}

/// The font size and lines, as word indices, for `words` in `fit`.
fn fit_layout(
    words: &[Word<'_>],
    max_size: f64,
    spacing: f64,
    fit: TextFit,
    measure: &dyn TextMeasure,
) -> (f64, Vec<Vec<usize>>) {
    if words.is_empty() {
        return (max_size, Vec::new());
    }
    let (widths, chars): (Vec<f64>, Vec<f64>) =
        words.iter().map(|word| word_metrics(word, measure)).unzip();
    let space = (measure.width_at_100("x x") - 2.0 * measure.width_at_100("x")).max(0.0);
    let word_width =
        |index: usize, size: f64| widths[index] * size / 100.0 + chars[index] * spacing;
    let space_width = |size: f64| space * size / 100.0 + spacing;

    // Greedy wrap at `size`: each line as word indices, and the widest.
    let wrap = |size: f64| -> (Vec<Vec<usize>>, f64) {
        let mut lines: Vec<Vec<usize>> = Vec::new();
        let mut current = 0.0;
        let mut widest: f64 = 0.0;
        for index in 0..words.len() {
            let width = word_width(index, size);
            match lines.last_mut() {
                Some(line) if current + space_width(size) + width <= fit.width => {
                    line.push(index);
                    current += space_width(size) + width;
                }
                _ => {
                    lines.push(vec![index]);
                    current = width;
                }
            }
            widest = widest.max(current);
        }
        (lines, widest)
    };
    let fits = |size: f64| -> bool {
        let (lines, widest) = wrap(size);
        lines.len() <= fit.lines
            && widest <= fit.width
            && lines.len() as f64 * size * FIT_LINE_HEIGHT <= fit.height * FIT_LINE_HEIGHT
    };

    let floor = MIN_FIT_FONT_SIZE.min(max_size);
    let size = if fits(max_size) {
        max_size
    } else if !fits(floor) {
        floor
    } else {
        let (mut low, mut high) = (floor, max_size);
        for _ in 0..24 {
            let middle = (low + high) / 2.0;
            if fits(middle) {
                low = middle;
            } else {
                high = middle;
            }
        }
        low
    };
    let (mut lines, _) = wrap(size);
    // Text that cannot fit even at the smallest size keeps its first lines.
    if lines.len() > fit.lines {
        let rest = lines.split_off(fit.lines - 1).concat();
        lines.push(rest);
    }
    (size, lines)
}

/// Text inside a `<text>` with format logos: literal text, or a logo drawn
/// in place of a label.
enum Piece {
    /// Escaped XML text, placeholders resolved.
    Text(String),
    Logo(&'static str),
}

/// Whether a `<text>` asks for format logos, checked even where it never
/// renders.
fn logo_text(start: &BytesStart<'_>) -> AppResult<bool> {
    let Some(attribute) = start
        .try_get_attribute(LOGOS_ATTR)
        .map_err(|error| invalid(format!("bad attribute: {error}")))?
    else {
        return Ok(false);
    };
    if start.local_name().as_ref() != "text" {
        return Err(invalid(format!("{LOGOS_ATTR} is only allowed on <text>")));
    }
    match attribute.value.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(invalid(format!(
            "{LOGOS_ATTR} must be true or false, not \"{other}\""
        ))),
    }
}

/// The logo a label placeholder is drawn as, given the title's values.
fn label_logo(field: &str, values: &BTreeMap<&'static str, String>) -> Option<&'static str> {
    let value = |name: &str| values.get(name).map(String::as_str).unwrap_or_default();
    match field {
        "audio_label" => match value("audio_codec") {
            "truehd_atmos" | "ddp_atmos" => Some("dolby-atmos"),
            "truehd" => Some("dolby-truehd"),
            "ddp" => Some("dolby-digital-plus"),
            "dtsx" | "dtshd_ma" | "dtshd_hra" => Some("dts-hd"),
            _ => None,
        },
        "hdr_label" => match value("hdr") {
            "dv" => Some("dolby-vision"),
            "hdr10plus" => Some("hdr10plus"),
            "hdr10" => Some("hdr"),
            _ => None,
        },
        _ => None,
    }
}

/// The content of a `<text>` with format logos: label placeholders with a
/// logo become logos, and an edition named IMAX shows IMAX's. It may hold
/// only text: no child elements.
fn text_pieces(
    reader: &mut Reader<&[u8]>,
    values: &BTreeMap<&'static str, String>,
) -> AppResult<Vec<Piece>> {
    let mut pieces = Vec::new();
    let push_text = |pieces: &mut Vec<Piece>, text: &str| match pieces.last_mut() {
        Some(Piece::Text(last)) => last.push_str(text),
        _ => pieces.push(Piece::Text(text.to_string())),
    };
    loop {
        match reader
            .read_event()
            .map_err(|error| invalid(format!("malformed XML: {error}")))?
        {
            Event::Text(text) => {
                let mut rest: &str = &text;
                while let Some(open) = rest.find("{{") {
                    push_text(&mut pieces, &rest[..open]);
                    let after = &rest[open + 2..];
                    let close = after
                        .find("}}")
                        .ok_or_else(|| invalid("unterminated {{ placeholder"))?;
                    let field = after[..close].trim();
                    let value = field_value(field, values)?;
                    if let Some(logo) = label_logo(field, values) {
                        pieces.push(Piece::Logo(logo));
                    } else if field == "edition_label" {
                        for (index, word) in value.split(' ').enumerate() {
                            if index > 0 {
                                push_text(&mut pieces, " ");
                            }
                            if word.eq_ignore_ascii_case("imax") {
                                pieces.push(Piece::Logo("imax"));
                            } else {
                                push_text(&mut pieces, &quick_xml::escape::escape(word));
                            }
                        }
                    } else {
                        push_text(&mut pieces, &quick_xml::escape::escape(value));
                    }
                    rest = &after[close + 2..];
                }
                push_text(&mut pieces, rest);
            }
            Event::GeneralRef(reference) => {
                push_text(&mut pieces, &format!("&{};", reference.as_ref() as &str));
            }
            Event::Comment(_) => {}
            Event::End(_) => break,
            Event::Eof => return Err(invalid("unterminated <text>")),
            _ => {
                return Err(invalid(format!(
                    "a <text> with {LOGOS_ATTR} may contain only text and placeholders"
                )));
            }
        }
    }
    Ok(pieces)
}

/// A bundled logo's width over its height, from its `viewBox`.
fn logo_aspect(name: &str) -> AppResult<f64> {
    let (_, svg) = LOGOS
        .iter()
        .find(|(logo, _)| *logo == name)
        .ok_or_else(|| invalid(format!("unknown logo \"{name}\"")))?;
    logo_parts(svg)
        .and_then(|(view_box, _)| {
            let numbers = view_box
                .split([' ', ','])
                .filter(|part| !part.is_empty())
                .map(|part| part.parse::<f64>().ok())
                .collect::<Option<Vec<_>>>()?;
            (numbers.len() == 4 && numbers[2] > 0.0 && numbers[3] > 0.0)
                .then(|| numbers[2] / numbers[3])
        })
        .ok_or_else(|| invalid(format!("bundled logo \"{name}\" is malformed")))
}

/// Writes a `<text>` with format logos. Without a logo to draw it is written
/// as it would be without the attribute. With one, each line becomes text
/// runs and logos laid out along the baseline, aligned as the text was.
fn write_logo_text(
    writer: &mut Writer<Vec<u8>>,
    element: BytesStart<'static>,
    fit: Option<TextFit>,
    pieces: &[Piece],
    measure: &dyn TextMeasure,
    logos: &mut Vec<&'static str>,
) -> AppResult<()> {
    let unescape = |text: &str| -> AppResult<String> {
        quick_xml::escape::unescape(text)
            .map(|text| text.into_owned())
            .map_err(|error| invalid(format!("bad text: {error}")))
    };
    if !pieces.iter().any(|piece| matches!(piece, Piece::Logo(_))) {
        let escaped = pieces
            .iter()
            .map(|piece| match piece {
                Piece::Text(text) => text.as_str(),
                Piece::Logo(_) => "",
            })
            .collect::<String>();
        return match fit {
            Some(fit) => write_fitted_text(writer, element, fit, &unescape(&escaped)?, measure),
            None => {
                write(writer, Event::Start(element))?;
                write(writer, Event::Text(BytesText::from_escaped(escaped)))?;
                write(writer, Event::End(quick_xml::events::BytesEnd::new("text")))
            }
        };
    }

    // Words in order: text split on whitespace, each logo a word of its own.
    let mut texts = Vec::new();
    for piece in pieces {
        if let Piece::Text(text) = piece {
            texts.push(unescape(text)?);
        }
    }
    let mut words = Vec::new();
    let mut next_text = texts.iter();
    for piece in pieces {
        match piece {
            Piece::Text(_) => {
                let text = next_text.next().map(String::as_str).unwrap_or_default();
                words.extend(text.split_whitespace().map(Word::Text));
            }
            Piece::Logo(name) => words.push(Word::Logo {
                name,
                aspect: logo_aspect(name)?,
            }),
        }
    }

    let font_size = attribute_number(&element, "font-size")?.unwrap_or(16.0);
    let spacing = attribute_number(&element, "letter-spacing")?.unwrap_or(0.0);
    let (size, lines, baselines, x) = match fit {
        Some(fit) => {
            let (size, lines) = fit_layout(&words, font_size, spacing, fit, measure);
            let line_height = size * FIT_LINE_HEIGHT;
            let first = fit.y + fit.height / 2.0
                - (lines.len().saturating_sub(1) as f64) * line_height / 2.0
                + size * FIT_CAP_CENTER;
            let baselines = (0..lines.len())
                .map(|index| first + index as f64 * line_height)
                .collect::<Vec<_>>();
            let x = attribute_number(&element, "x")?.unwrap_or(fit.x);
            (size, lines, baselines, x)
        }
        None => {
            let y = attribute_number(&element, "y")?.unwrap_or(0.0);
            let x = attribute_number(&element, "x")?.unwrap_or(0.0);
            (font_size, vec![(0..words.len()).collect()], vec![y], x)
        }
    };

    let attribute_text = |name: &str| -> AppResult<Option<String>> {
        Ok(element
            .try_get_attribute(name)
            .map_err(|error| invalid(format!("bad attribute: {error}")))?
            .map(|attribute| attribute.value.into_owned()))
    };
    let anchor = attribute_text("text-anchor")?.unwrap_or_default();
    let fill = attribute_text("fill")?;
    let mut group = BytesStart::new("g");
    if let Some(transform) = attribute_text("transform")? {
        group.push_attribute(("transform", transform.as_str()));
    }
    write(writer, Event::Start(group))?;

    let (widths, chars): (Vec<f64>, Vec<f64>) =
        words.iter().map(|word| word_metrics(word, measure)).unzip();
    let width = |index: usize| widths[index] * size / 100.0 + chars[index] * spacing;
    let space = (measure.width_at_100("x x") - 2.0 * measure.width_at_100("x")).max(0.0) * size
        / 100.0
        + spacing;
    for (line, baseline) in lines.iter().zip(baselines) {
        let total = line.iter().map(|index| width(*index)).sum::<f64>()
            + space * line.len().saturating_sub(1) as f64;
        let mut cursor = match anchor.trim() {
            "middle" => x - total / 2.0,
            "end" => x - total,
            _ => x,
        };
        let mut run: Vec<&str> = Vec::new();
        let mut run_start = cursor;
        for (position, index) in line.iter().enumerate() {
            if position > 0 {
                cursor += space;
            }
            match words[*index] {
                Word::Text(text) => {
                    if run.is_empty() {
                        run_start = cursor;
                    }
                    run.push(text);
                }
                Word::Logo { name, aspect } => {
                    write_text_run(writer, &element, &mut run, run_start, baseline, size)?;
                    let height = size * FORMAT_LOGO_HEIGHT;
                    let mut logo = BytesStart::new("use");
                    logo.push_attribute(("href", format!("#{LOGO_ID_PREFIX}{name}").as_str()));
                    logo.push_attribute((
                        "x",
                        format_number(cursor + size * FORMAT_LOGO_MARGIN).as_str(),
                    ));
                    logo.push_attribute((
                        "y",
                        format_number(baseline - size * FIT_CAP_CENTER - height / 2.0).as_str(),
                    ));
                    logo.push_attribute(("width", format_number(height * aspect).as_str()));
                    logo.push_attribute(("height", format_number(height).as_str()));
                    if let Some(fill) = fill.as_deref() {
                        logo.push_attribute(("fill", fill));
                    }
                    write(writer, Event::Empty(logo))?;
                    if !logos.contains(&name) {
                        logos.push(name);
                    }
                }
            }
            cursor += width(*index);
        }
        write_text_run(writer, &element, &mut run, run_start, baseline, size)?;
    }
    write(writer, Event::End(quick_xml::events::BytesEnd::new("g")))
}

/// Writes the text words gathered since the last logo as one left-aligned
/// `<text>` at `x`, with the original element's styling.
fn write_text_run(
    writer: &mut Writer<Vec<u8>>,
    element: &BytesStart<'static>,
    run: &mut Vec<&str>,
    x: f64,
    baseline: f64,
    size: f64,
) -> AppResult<()> {
    if run.is_empty() {
        return Ok(());
    }
    let mut text = BytesStart::new("text");
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|error| invalid(format!("bad attribute: {error}")))?;
        if !matches!(
            attribute.key.0,
            "x" | "y" | "font-size" | "text-anchor" | "transform"
        ) {
            text.push_attribute(attribute);
        }
    }
    text.push_attribute(("x", format_number(x).as_str()));
    text.push_attribute(("y", format_number(baseline).as_str()));
    text.push_attribute(("font-size", format_number(size).as_str()));
    text.push_attribute(("text-anchor", "start"));
    write(writer, Event::Start(text))?;
    write(writer, Event::Text(BytesText::new(&run.join(" "))))?;
    run.clear();
    write(writer, Event::End(quick_xml::events::BytesEnd::new("text")))
}

/// A `data-scryer-stack` group: each child it keeps is moved one `step`
/// further along, so children hidden by their conditions leave no gap.
#[derive(Clone, Copy, Debug)]
struct Stack {
    dx: f64,
    dy: f64,
    placed: usize,
}

/// The stack layout an element declares, checked even where it never
/// renders.
fn stack_layout(start: &BytesStart<'_>) -> AppResult<Option<Stack>> {
    let attribute = |name: &str| -> AppResult<Option<String>> {
        Ok(start
            .try_get_attribute(name)
            .map_err(|error| invalid(format!("bad attribute: {error}")))?
            .map(|attribute| attribute.value.trim().to_string()))
    };
    let direction = attribute(STACK_ATTR)?;
    let step = attribute(STEP_ATTR)?;
    let (direction, step) = match (direction, step) {
        (None, None) => return Ok(None),
        (Some(direction), Some(step)) => (direction, step),
        _ => {
            return Err(invalid(format!(
                "{STACK_ATTR} and {STEP_ATTR} must be used together"
            )));
        }
    };
    let step = step
        .parse::<f64>()
        .ok()
        .filter(|step| step.is_finite() && *step > 0.0 && *step <= 10_000.0)
        .ok_or_else(|| {
            invalid(format!(
                "{STEP_ATTR} must be a positive number, not \"{step}\""
            ))
        })?;
    let (dx, dy) = match direction.as_str() {
        "down" => (0.0, step),
        "up" => (0.0, -step),
        "right" => (step, 0.0),
        "left" => (-step, 0.0),
        other => {
            return Err(invalid(format!(
                "{STACK_ATTR} must be down, up, right or left, not \"{other}\""
            )));
        }
    };
    Ok(Some(Stack { dx, dy, placed: 0 }))
}

/// The logo a `<use>` element draws, if it references one. Any reference
/// into the reserved prefix must name a bundled logo.
fn referenced_logo(start: &BytesStart<'_>) -> AppResult<Option<&'static str>> {
    if start.local_name().as_ref() != "use" {
        return Ok(None);
    }
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|error| invalid(format!("bad attribute: {error}")))?;
        if attribute.key.local_name().as_ref() != "href" {
            continue;
        }
        let Some(name) = attribute
            .value
            .trim()
            .strip_prefix('#')
            .and_then(|id| id.strip_prefix(LOGO_ID_PREFIX))
        else {
            continue;
        };
        return LOGOS
            .iter()
            .find(|(logo, _)| *logo == name)
            .map(|(logo, _)| Some(*logo))
            .ok_or_else(|| {
                invalid(format!(
                    "unknown logo \"{name}\"; use one of {}",
                    LOGOS
                        .iter()
                        .map(|(logo, _)| *logo)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            });
    }
    Ok(None)
}

/// Ids under the logo prefix belong to the renderer.
fn check_reserved_id(start: &BytesStart<'_>) -> AppResult<()> {
    if let Some(id) = start
        .try_get_attribute("id")
        .map_err(|error| invalid(format!("bad attribute: {error}")))?
        && id.value.trim().starts_with(LOGO_ID_PREFIX)
    {
        return Err(invalid(format!(
            "ids starting with \"{LOGO_ID_PREFIX}\" are reserved for logos"
        )));
    }
    Ok(())
}

/// Rewrites an element that is not inside a skipped subtree: records the
/// logo it draws and, inside a stack, moves it to its slot.
fn emit_element(
    start: &BytesStart<'_>,
    values: &BTreeMap<&'static str, String>,
    open: &mut [Option<Stack>],
    logos: &mut Vec<&'static str>,
) -> AppResult<Option<BytesStart<'static>>> {
    check_reserved_id(start)?;
    let logo = referenced_logo(start)?;
    let parent = open.last_mut().and_then(Option::as_mut);
    let offset = parent.as_ref().map(|stack| {
        (
            stack.dx * stack.placed as f64,
            stack.dy * stack.placed as f64,
        )
    });
    let Some(element) = rewrite_element(start, values, offset)? else {
        return Ok(None);
    };
    if let Some(stack) = parent {
        stack.placed += 1;
    }
    if let Some(logo) = logo
        && !logos.contains(&logo)
    {
        logos.push(logo);
    }
    Ok(Some(element))
}

/// `<defs>` holding a `<symbol>` per referenced logo, appended to the root.
fn write_logo_defs(writer: &mut Writer<Vec<u8>>, logos: &[&'static str]) -> AppResult<()> {
    let mut defs = String::from("<defs>");
    for name in logos {
        let (_, svg) = LOGOS
            .iter()
            .find(|(logo, _)| logo == name)
            .ok_or_else(|| invalid(format!("unknown logo \"{name}\"")))?;
        let (view_box, body) = logo_parts(svg)
            .ok_or_else(|| invalid(format!("bundled logo \"{name}\" is malformed")))?;
        defs.push_str(&format!(
            "<symbol id=\"{LOGO_ID_PREFIX}{name}\" viewBox=\"{view_box}\" preserveAspectRatio=\"xMidYMid meet\">{body}</symbol>"
        ));
    }
    defs.push_str("</defs>");
    write(writer, Event::Text(BytesText::from_escaped(defs)))
}

/// The `viewBox` and inner markup of a bundled logo.
fn logo_parts(svg: &str) -> Option<(&str, &str)> {
    let open_end = svg.find('>')?;
    let root = &svg[..open_end];
    let view_box_start = root.find("viewBox=\"")? + "viewBox=\"".len();
    let view_box_len = root[view_box_start..].find('"')?;
    let body_end = svg.rfind("</svg>")?;
    Some((
        &root[view_box_start..view_box_start + view_box_len],
        svg[open_end + 1..body_end].trim(),
    ))
}

/// Returns the element with contract attributes stripped and placeholders
/// substituted, or `None` when its conditions exclude it. `offset` moves the
/// element, ahead of any transform of its own.
fn rewrite_element<'a>(
    start: &BytesStart<'a>,
    values: &BTreeMap<&'static str, String>,
    offset: Option<(f64, f64)>,
) -> AppResult<Option<BytesStart<'static>>> {
    let name = String::from_utf8_lossy(start.name().as_ref().as_bytes()).into_owned();
    let mut rewritten = BytesStart::new(name);
    let translate = offset.map(|(dx, dy)| format!("translate({dx} {dy})"));
    let mut translated = translate.is_none();
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|error| invalid(format!("bad attribute: {error}")))?;
        let key = attribute.key.0;
        match key {
            IF_ATTR => {
                if !evaluate_condition(&attribute.value, values)? {
                    return Ok(None);
                }
            }
            UNLESS_ATTR => {
                if evaluate_condition(&attribute.value, values)? {
                    return Ok(None);
                }
            }
            VERSION_ATTR | STACK_ATTR | STEP_ATTR | FIT_ATTR | FIT_LINES_ATTR | LOGOS_ATTR => {}
            "transform" if !translated => {
                let own = substitute(&attribute.value, values)?;
                rewritten.push_attribute((
                    "transform",
                    format!("{} {own}", translate.as_deref().unwrap_or_default()).as_str(),
                ));
                translated = true;
            }
            _ => {
                let value = substitute(&attribute.value, values)?;
                rewritten.push_attribute(Attribute {
                    key: QName(key),
                    value: Cow::Owned(value),
                });
            }
        }
    }
    if !translated && let Some(translate) = translate {
        rewritten.push_attribute(("transform", translate.as_str()));
    }
    Ok(Some(rewritten))
}

fn field_value<'v>(field: &str, values: &'v BTreeMap<&'static str, String>) -> AppResult<&'v str> {
    values
        .get(field)
        .map(String::as_str)
        .ok_or_else(|| invalid(format!("unknown field \"{field}\"")))
}

/// Evaluate a condition: clauses separated by `;` must all hold. A clause is
/// `field` (non-empty), `field=a|b` (equals any) or `field!=a|b` (equals
/// none). The raw attribute text is unescaped XML.
pub fn evaluate_condition(raw: &str, values: &BTreeMap<&'static str, String>) -> AppResult<bool> {
    let mut clauses = 0;
    let mut result = true;
    for clause in raw
        .split(';')
        .map(str::trim)
        .filter(|clause| !clause.is_empty())
    {
        clauses += 1;
        // Multi-valued fields (a title's editions) match when any of their
        // values does; every other field holds exactly one value.
        let holds = if let Some((field, options)) = clause.split_once("!=") {
            let field = field.trim();
            let present = condition_value_set(field, field_value(field, values)?);
            !split_options(options)?.any(|option| present.contains(&option))
        } else if let Some((field, options)) = clause.split_once('=') {
            let field = field.trim();
            let present = condition_value_set(field, field_value(field, values)?);
            split_options(options)?.any(|option| present.contains(&option))
        } else {
            !field_value(clause, values)?.is_empty()
        };
        result &= holds;
    }
    if clauses == 0 {
        return Err(invalid("empty condition"));
    }
    Ok(result)
}

/// Reject a condition that compares a field against a value it can never
/// take, such as `resolution=4K` (the token is `2160p`; `4K` is the label).
/// Without this the template saves fine and the badge silently never shows.
fn check_condition_values(raw: &str) -> AppResult<()> {
    for clause in raw
        .split(';')
        .map(str::trim)
        .filter(|clause| !clause.is_empty())
    {
        let Some((field, options)) = clause.split_once("!=").or_else(|| clause.split_once('='))
        else {
            continue;
        };
        let field = field.trim();
        for option in options.split('|').map(str::trim) {
            if is_possible_condition_value(field, option) {
                continue;
            }
            let expected = match (condition_values(field), field) {
                (Some(values), _) => format!("use one of {}", values.join(", ")),
                (None, "audio_channels") => {
                    format!(
                        "use one of {}, or a count such as 10ch",
                        CHANNEL_LAYOUTS.join(", ")
                    )
                }
                (None, "edition") => format!(
                    "editions are lowercase slugs, for example \"{}\"",
                    edition_token(option)
                ),
                _ => String::new(),
            };
            return Err(invalid(format!(
                "{field} is never \"{option}\"; {expected}"
            )));
        }
    }
    Ok(())
}

fn split_options(options: &str) -> AppResult<impl Iterator<Item = &str>> {
    if options.trim().is_empty() {
        return Err(invalid("a condition compares against no values"));
    }
    Ok(options.split('|').map(str::trim))
}

/// Replace `{{field}}` in raw (escaped) XML text; values are escaped on
/// insertion so existing entities in the template are left untouched.
pub fn substitute(raw: &str, values: &BTreeMap<&'static str, String>) -> AppResult<String> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let close = after
            .find("}}")
            .ok_or_else(|| invalid("unterminated {{ placeholder"))?;
        let field = after[..close].trim();
        out.push_str(&quick_xml::escape::escape(field_value(field, values)?));
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    Ok(out)
}
