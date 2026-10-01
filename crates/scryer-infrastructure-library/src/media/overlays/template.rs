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

/// Element names a template may not contain. Templates are vector badges;
/// nothing may pull in outside content.
const FORBIDDEN_ELEMENTS: &[&str] = &["image", "foreignObject", "script", "feImage"];

pub use scryer_application::overlays::TEMPLATE_FIELDS;

fn invalid(message: impl Into<String>) -> AppError {
    AppError::Validation(format!("overlay template: {}", message.into()))
}

/// Resolve conditionals and placeholders against `values`.
pub fn preprocess(svg: &str, values: &BTreeMap<&'static str, String>) -> AppResult<String> {
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
                match rewrite_element(&start, values)? {
                    Some(element) => write(&mut writer, Event::Start(element))?,
                    None => skip_depth = 1,
                }
            }
            Event::Empty(start) => {
                if skip_depth > 0 {
                    continue;
                }
                check_element(&start, &mut saw_root)?;
                if let Some(element) = rewrite_element(&start, values)? {
                    write(&mut writer, Event::Empty(element))?;
                }
            }
            Event::End(end) => {
                if skip_depth > 0 {
                    skip_depth -= 1;
                    continue;
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
                for attribute in start.attributes() {
                    let attribute =
                        attribute.map_err(|error| invalid(format!("bad attribute: {error}")))?;
                    let key = attribute.key.0;
                    if key == IF_ATTR || key == UNLESS_ATTR {
                        evaluate_condition(&attribute.value, &values)?;
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

/// Returns the element with contract attributes stripped and placeholders
/// substituted, or `None` when its conditions exclude it.
fn rewrite_element<'a>(
    start: &BytesStart<'a>,
    values: &BTreeMap<&'static str, String>,
) -> AppResult<Option<BytesStart<'static>>> {
    let name = String::from_utf8_lossy(start.name().as_ref().as_bytes()).into_owned();
    let mut rewritten = BytesStart::new(name);
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
            VERSION_ATTR => {}
            _ => {
                let value = substitute(&attribute.value, values)?;
                rewritten.push_attribute(Attribute {
                    key: QName(key),
                    value: Cow::Owned(value),
                });
            }
        }
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
        let holds = if let Some((field, options)) = clause.split_once("!=") {
            let value = field_value(field.trim(), values)?;
            !split_options(options)?.any(|option| option == value)
        } else if let Some((field, options)) = clause.split_once('=') {
            let value = field_value(field.trim(), values)?;
            split_options(options)?.any(|option| option == value)
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
