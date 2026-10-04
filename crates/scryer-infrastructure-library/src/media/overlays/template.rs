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
        "imdb",
        include_str!("../../../assets/overlays/logos/imdb.svg"),
    ),
    (
        "letterboxd",
        include_str!("../../../assets/overlays/logos/letterboxd.svg"),
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
        "tmdb",
        include_str!("../../../assets/overlays/logos/tmdb.svg"),
    ),
    (
        "trakt",
        include_str!("../../../assets/overlays/logos/trakt.svg"),
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
                match emit_element(&start, values, &mut open, &mut logos)? {
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
    let words = content.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        return (max_size, Vec::new());
    }
    let widths = words
        .iter()
        .map(|word| measure.width_at_100(word))
        .collect::<Vec<_>>();
    let space = (measure.width_at_100("x x") - 2.0 * measure.width_at_100("x")).max(0.0);
    let chars = words
        .iter()
        .map(|word| word.chars().count() as f64)
        .collect::<Vec<_>>();
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
    let (lines, _) = wrap(size);
    let mut lines = lines
        .into_iter()
        .map(|line| {
            line.into_iter()
                .map(|index| words[index])
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>();
    // Text that cannot fit even at the smallest size keeps its first lines.
    if lines.len() > fit.lines {
        let rest = lines.split_off(fit.lines - 1).join(" ");
        lines.push(rest);
    }
    (size, lines)
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
            VERSION_ATTR | STACK_ATTR | STEP_ATTR | FIT_ATTR | FIT_LINES_ATTR => {}
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
