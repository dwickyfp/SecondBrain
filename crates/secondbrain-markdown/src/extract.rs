//! Extraction of structured note data from a parsed Markdown document.
//!
//! This module walks the semantic AST of a [`SourceDocument`] and collects:
//!
//! - **Wikilinks** (`[[target]]`, `[[target|alias]]`, `[[target#heading]]`, `![[embed]]`)
//! - **Tags** (`#tag`, including Unicode tags like `#café` or `#日本語`)
//! - **Headings** (level, text, span)
//! - **Tasks** (GFM task list items with `[ ]` or `[x]` markers)
//! - **Properties** (YAML frontmatter key-value pairs)
//! - **Plain text** (concatenation of all text content)
//!
//! Wikilinks and tags inside code spans and code blocks are ignored.
//! Tags are deduplicated, preserving the first occurrence's span.

use crate::SourceDocument;
use crate::ast::{SemanticKind, SemanticNode};
use crate::frontmatter::parse_metadata;
use crate::source::SourceSpan;

use serde_yaml::Value;

/// A wikilink or embed link extracted from the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedLink {
    /// The link target (note name or file path).
    pub target: String,
    /// Optional display alias (`[[target|alias]]`).
    pub alias: Option<String>,
    /// Optional heading anchor (`[[target#heading]]`).
    pub heading: Option<String>,
    /// Whether this is an embed (`![[target]]`).
    pub embed: bool,
    /// Byte span of the full link syntax in the source.
    pub span: SourceSpan,
}

/// A hashtag extracted from the document text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedTag {
    /// The tag text without the `#` prefix.
    pub text: String,
    /// Byte span of the tag (including `#`) in the source.
    pub span: SourceSpan,
}

/// A heading extracted from the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedHeading {
    /// Heading level (1–6).
    pub level: u8,
    /// Heading text (without the `#` markers).
    pub text: String,
    /// Byte span of the full heading line in the source.
    pub span: SourceSpan,
}

/// A GFM task list item extracted from the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedTask {
    /// Whether the task is checked (`[x]`) or not (`[ ]`).
    pub checked: bool,
    /// The task text (without the `- [ ]` / `- [x]` marker).
    pub text: String,
    /// Byte span of the task list item in the source.
    pub span: SourceSpan,
}

/// A frontmatter property value.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValue {
    /// YAML `null` (`~` or empty).
    Null,
    /// YAML boolean (`true` / `false`).
    Bool(bool),
    /// YAML integer.
    Int(i64),
    /// YAML floating-point number.
    Float(f64),
    /// YAML string.
    Str(String),
    /// YAML sequence (list).
    List(Vec<String>),
}

/// All extracted data from a note.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedNote {
    /// Wikilinks and embeds, in source order.
    pub links: Vec<ExtractedLink>,
    /// Tags, deduplicated, preserving first-occurrence span.
    pub tags: Vec<ExtractedTag>,
    /// Headings, in source order.
    pub headings: Vec<ExtractedHeading>,
    /// GFM task list items, in source order.
    pub tasks: Vec<ExtractedTask>,
    /// Frontmatter properties as `(key, value)` pairs, in YAML order.
    pub properties: Vec<(String, PropertyValue)>,
    /// Concatenated plain text from all text nodes (excluding code).
    pub plain_text: String,
}

/// Extract structured note data from a parsed document.
///
/// Walks the semantic AST, scanning text nodes for wikilinks and tags while
/// skipping code spans and code blocks. Headings and task list items are
/// identified by their semantic kind and raw source text.
///
/// # Panics
///
/// This function does not panic; frontmatter parse errors result in an empty
/// properties list.
#[must_use]
pub fn extract(doc: &SourceDocument) -> ExtractedNote {
    let source = doc.source();
    let root = doc.root();

    let mut links = Vec::new();
    let mut tags = Vec::new();
    let mut headings = Vec::new();
    let mut tasks = Vec::new();
    let mut plain_text = String::new();

    walk(
        root,
        source,
        &mut links,
        &mut tags,
        &mut headings,
        &mut tasks,
        &mut plain_text,
    );

    // Deduplicate tags: preserve first occurrence span.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    tags.retain(|t| seen.insert(t.text.clone()));

    // Sort links by span start to ensure source order.
    links.sort_by_key(|l| l.span.start);

    // Properties from frontmatter.
    let properties = extract_properties(source);

    ExtractedNote {
        links,
        tags,
        headings,
        tasks,
        properties,
        plain_text,
    }
}

/// Recursively walk semantic nodes, collecting links, tags, headings, tasks,
/// and plain text.
///
/// Code spans (`InlineCode`, `Code`, `Math`, `InlineMath`) are skipped —
/// their content is not scanned for links/tags and not included in plain text.
#[allow(clippy::too_many_arguments)]
fn walk(
    node: &SemanticNode,
    source: &str,
    links: &mut Vec<ExtractedLink>,
    tags: &mut Vec<ExtractedTag>,
    headings: &mut Vec<ExtractedHeading>,
    tasks: &mut Vec<ExtractedTask>,
    plain_text: &mut String,
) {
    // Skip code-like nodes entirely.
    if matches!(
        node.kind,
        SemanticKind::Code
            | SemanticKind::InlineCode
            | SemanticKind::Math
            | SemanticKind::InlineMath
    ) {
        return;
    }

    match node.kind {
        SemanticKind::Text => {
            let text = node.raw(source);
            let span = node.span;
            scan_text(text, span, links, tags);
            if !plain_text.is_empty() {
                plain_text.push(' ');
            }
            plain_text.push_str(text);
        }
        SemanticKind::Heading => {
            if let Some(h) = parse_heading(node, source) {
                headings.push(h);
            }
        }
        SemanticKind::ListItem => {
            if let Some(t) = parse_task(node, source) {
                tasks.push(t);
            }
        }
        _ => {}
    }

    // Recurse into children.
    for child in &node.children {
        walk(child, source, links, tags, headings, tasks, plain_text);
    }
}

/// Scan a text slice for wikilinks `[[...]]` and tags `#...`.
///
/// The `base` span is the span of the entire text slice; offsets within the
/// slice are translated to absolute byte offsets by adding `base.start`.
#[allow(clippy::collapsible_if)]
#[allow(clippy::too_many_arguments)]
fn scan_text(
    text: &str,
    base: SourceSpan,
    links: &mut Vec<ExtractedLink>,
    tags: &mut Vec<ExtractedTag>,
) {
    let bytes = text.as_bytes();
    let base_start = base.start;
    let mut i = 0usize;

    while i < bytes.len() {
        // Skip escaped `\[[` — the backslash escapes the `[`.
        if i + 2 < bytes.len() && bytes[i] == b'\\' && bytes[i + 1] == b'[' && bytes[i + 2] == b'['
        {
            // Skip the backslash and both brackets.
            i += 3;
            // Skip the rest until `]]`.
            while i + 1 < bytes.len() {
                if bytes[i] == b']' && bytes[i + 1] == b']' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }
        // Check for embed wikilink: ![[...]]
        if i + 2 < bytes.len() && bytes[i] == b'!' && bytes[i + 1] == b'[' && bytes[i + 2] == b'[' {
            if let Some(link) = parse_wikilink(text, i, base_start, true) {
                links.push(link.0);
                i = link.1;
                continue;
            }
        }
        // Check for wikilink: [[...]]
        if i + 1 < bytes.len() && bytes[i] == b'[' && bytes[i + 1] == b'[' {
            if let Some(link) = parse_wikilink(text, i, base_start, false) {
                links.push(link.0);
                i = link.1;
                continue;
            }
        }
        // Check for tag: #word
        if bytes[i] == b'#' {
            if let Some(tag) = parse_tag(text, i, base_start) {
                tags.push(tag.0);
                i = tag.1;
                continue;
            }
        }
        i += 1;
    }
}

/// Parse a wikilink starting at byte offset `start` in `text`.
///
/// Returns `(link, end_offset)` where `end_offset` is the position after the
/// closing `]]`.
fn parse_wikilink(
    text: &str,
    start: usize,
    base_start: usize,
    embed: bool,
) -> Option<(ExtractedLink, usize)> {
    let bytes = text.as_bytes();

    // Determine where the content starts (after `![[` or `[[`).
    let content_start = if embed { start + 3 } else { start + 2 };

    // Find the closing `]]`.
    let mut depth = 1usize;
    let mut j = content_start;
    while j < bytes.len() {
        if j + 1 < bytes.len() && bytes[j] == b']' && bytes[j + 1] == b']' {
            depth -= 1;
            if depth == 0 {
                break;
            }
            j += 2;
            continue;
        }
        if j + 1 < bytes.len() && bytes[j] == b'[' && bytes[j + 1] == b'[' {
            depth += 1;
            j += 2;
            continue;
        }
        j += 1;
    }

    if depth != 0 || j >= bytes.len() {
        return None;
    }

    // j points at the first `]` of the closing `]]`.
    let content = &text[content_start..j];
    let end = j + 2; // After `]]`.

    // Don't treat empty content as a link.
    if content.trim().is_empty() {
        return None;
    }

    // Parse content: target|alias or target#heading or target#heading|alias
    let (target_part, alias) = match content.split_once('|') {
        Some((t, a)) => (t, Some(a.trim().to_string())),
        None => (content, None),
    };

    let (target, heading) = match target_part.split_once('#') {
        Some((t, h)) => (t.trim().to_string(), Some(h.trim().to_string())),
        None => (target_part.trim().to_string(), None),
    };

    if target.is_empty() {
        return None;
    }

    let span = SourceSpan::new(base_start + start, base_start + end);

    Some((
        ExtractedLink {
            target,
            alias,
            heading,
            embed,
            span,
        },
        end,
    ))
}

/// Parse a tag starting at byte offset `start` (the `#`) in `text`.
///
/// A tag is `#` followed by one or more non-whitespace, non-punctuation
/// characters. Unicode word characters are allowed.
///
/// Returns `(tag, end_offset)` where `end_offset` is the position after the
/// tag text (not including the `#`... actually including).
fn parse_tag(text: &str, start: usize, base_start: usize) -> Option<(ExtractedTag, usize)> {
    let bytes = text.as_bytes();

    // A tag must not be preceded by a word character (e.g., "foo#bar" is not a tag).
    // But we also don't want to match `#` in the middle of a word.
    // markdown-rs doesn't parse tags natively, so we do it here.
    if start > 0 {
        let prev = bytes[start - 1];
        if is_word_byte(prev) {
            return None;
        }
    }

    // Skip the `#`.
    let mut j = start + 1;
    let tag_start = start + 1;

    // Collect tag characters: Unicode word characters and some connectors.
    while j < bytes.len() {
        let c = bytes[j];
        if is_tag_char(c, j, bytes) {
            j += 1;
        } else {
            break;
        }
    }

    if j == tag_start {
        return None; // Empty tag.
    }

    // Need to decode the tag from UTF-8.
    let tag_bytes = &bytes[tag_start..j];
    let tag_text = match std::str::from_utf8(tag_bytes) {
        Ok(s) => s.to_string(),
        Err(_) => return None,
    };

    // Tags must start with a letter or Unicode letter (not a number).
    if tag_text
        .chars()
        .next()
        .map(|c| c.is_alphabetic())
        .unwrap_or(false)
    {
        let span = SourceSpan::new(base_start + start, base_start + j);
        Some((
            ExtractedTag {
                text: tag_text,
                span,
            },
            j,
        ))
    } else {
        None
    }
}

/// Check if a byte is a word character (ASCII alphanumeric or underscore).
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Check if a byte can be part of a tag.
///
/// Tags allow Unicode word characters (handled by checking the UTF-8 sequence)
/// and `_` and `-`. We approximate by allowing any non-ASCII byte (which starts
/// a multibyte UTF-8 sequence) and ASCII word characters plus `-`.
fn is_tag_char(b: u8, _pos: usize, _bytes: &[u8]) -> bool {
    // Non-ASCII bytes are part of multibyte UTF-8 sequences — allow them.
    if b >= 0x80 {
        return true;
    }
    // ASCII word characters and hyphens.
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Strip a Markdown list marker from the start of a line.
///
/// Supports `-`, `*`, `+` (unordered) and `N.` (ordered) followed by
/// whitespace. Returns the remaining text after the marker, or `None` if
/// no list marker is found.
fn strip_list_marker(raw: &str) -> Option<&str> {
    let trimmed = raw.trim_start();
    if let Some(rest) = trimmed.strip_prefix("- ") {
        Some(rest)
    } else if let Some(rest) = trimmed.strip_prefix("* ") {
        Some(rest)
    } else if let Some(rest) = trimmed.strip_prefix("+ ") {
        Some(rest)
    } else {
        // Ordered list: `1. `, `2. `, etc.
        let dot_pos = trimmed.find(". ")?;
        let prefix = &trimmed[..dot_pos];
        if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()) {
            Some(&trimmed[dot_pos + 2..])
        } else {
            None
        }
    }
}

/// Parse a heading node to extract level and text.
fn parse_heading(node: &SemanticNode, source: &str) -> Option<ExtractedHeading> {
    let raw = node.raw(source);
    let span = node.span;

    // Determine the heading level from leading `#` marks.
    let level = raw.chars().take_while(|&c| c == '#').count() as u8;

    if level == 0 || level > 6 {
        return None;
    }

    // Extract heading text: skip `#`s and leading whitespace.
    let text = raw[level as usize..].trim().to_string();

    if text.is_empty() {
        return None;
    }

    Some(ExtractedHeading { level, text, span })
}

/// Parse a list item node to check if it's a GFM task.
///
/// GFM task list items have the form `- [ ] text` or `- [x] text`.
/// We check the raw source for the task marker pattern.
fn parse_task(node: &SemanticNode, source: &str) -> Option<ExtractedTask> {
    let raw = node.raw(source);
    let span = node.span;

    // Strip the list marker: `- `, `* `, `+ `, or `N. ` (ordered list).
    let after_marker = strip_list_marker(raw)?;

    // Look for `[ ]` or `[x]`/`[X]` marker.
    let (checked, after_task) = if let Some(rest) = after_marker.strip_prefix("[ ]") {
        (false, rest)
    } else if let Some(rest) = after_marker.strip_prefix("[x]") {
        (true, rest)
    } else if let Some(rest) = after_marker.strip_prefix("[X]") {
        (true, rest)
    } else {
        return None; // Not a task.
    };

    let text = after_task.trim().to_string();

    Some(ExtractedTask {
        checked,
        text,
        span,
    })
}

/// Extract frontmatter properties as typed key-value pairs.
///
/// Returns an empty vector if no frontmatter is present or if parsing fails.
fn extract_properties(source: &str) -> Vec<(String, PropertyValue)> {
    let metadata = match parse_metadata(source) {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    let mut result = Vec::new();

    for (key, value) in &metadata.properties {
        let key_str = match key {
            Value::String(s) => s.clone(),
            _ => continue,
        };
        let pv = yaml_to_property_value(value);
        result.push((key_str, pv));
    }

    result
}

/// Convert a `serde_yaml::Value` to a [`PropertyValue`].
fn yaml_to_property_value(value: &Value) -> PropertyValue {
    match value {
        Value::Null => PropertyValue::Null,
        Value::Bool(b) => PropertyValue::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                PropertyValue::Int(i)
            } else if let Some(f) = n.as_f64() {
                PropertyValue::Float(f)
            } else {
                PropertyValue::Null
            }
        }
        Value::String(s) => PropertyValue::Str(s.clone()),
        Value::Sequence(seq) => {
            let items: Vec<String> = seq
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Bool(b) => b.to_string(),
                    Value::Number(n) => n.to_string(),
                    Value::Null => String::new(),
                    _ => String::new(),
                })
                .collect();
            PropertyValue::List(items)
        }
        _ => PropertyValue::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_wikilink_inline() {
        let text = "See [[Note]] here.";
        let mut links = Vec::new();
        let mut tags = Vec::new();
        scan_text(text, SourceSpan::new(0, text.len()), &mut links, &mut tags);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "Note");
        assert!(links[0].alias.is_none());
        assert!(links[0].heading.is_none());
        assert!(!links[0].embed);
    }

    #[test]
    fn parse_tag_simple() {
        let text = "a #tag here";
        let mut links = Vec::new();
        let mut tags = Vec::new();
        scan_text(text, SourceSpan::new(0, text.len()), &mut links, &mut tags);
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].text, "tag");
    }

    #[test]
    fn parse_tag_unicode() {
        let text = "#café";
        let mut links = Vec::new();
        let mut tags = Vec::new();
        scan_text(text, SourceSpan::new(0, text.len()), &mut links, &mut tags);
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].text, "café");
    }

    #[test]
    fn parse_tag_not_after_word() {
        let text = "foo#bar";
        let mut links = Vec::new();
        let mut tags = Vec::new();
        scan_text(text, SourceSpan::new(0, text.len()), &mut links, &mut tags);
        assert!(tags.is_empty(), "foo#bar should not produce a tag");
    }

    #[test]
    fn parse_tag_not_number() {
        let text = "#123";
        let mut links = Vec::new();
        let mut tags = Vec::new();
        scan_text(text, SourceSpan::new(0, text.len()), &mut links, &mut tags);
        assert!(tags.is_empty(), "#123 should not produce a tag");
    }

    #[test]
    fn yaml_bool_to_property() {
        let v = Value::Bool(true);
        assert_eq!(yaml_to_property_value(&v), PropertyValue::Bool(true));
    }

    #[test]
    fn yaml_int_to_property() {
        let v = Value::Number(serde_yaml::Number::from(42));
        assert_eq!(yaml_to_property_value(&v), PropertyValue::Int(42));
    }

    #[test]
    fn yaml_string_to_property() {
        let v = Value::String("hello".to_string());
        assert_eq!(
            yaml_to_property_value(&v),
            PropertyValue::Str("hello".to_string())
        );
    }

    #[test]
    fn yaml_list_to_property() {
        let v = Value::Sequence(vec![
            Value::String("a".to_string()),
            Value::String("b".to_string()),
        ]);
        assert_eq!(
            yaml_to_property_value(&v),
            PropertyValue::List(vec!["a".to_string(), "b".to_string()])
        );
    }
}
