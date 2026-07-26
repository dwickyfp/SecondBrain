//! YAML frontmatter parsing and surgical metadata patching.
//!
//! This module provides the ability to parse YAML frontmatter from Markdown
//! sources and surgically insert a `NoteId` while preserving all body bytes
//! and unrelated frontmatter formatting.
//!
//! # Design
//!
//! - Frontmatter is delimited by `---` lines at the very start of the file
//!   (optionally preceded by a UTF-8 BOM).
//! - The body (everything after the closing `---` delimiter) is never
//!   modified — only the frontmatter region is edited.
//! - When inserting an `id`, we append `id: <ULID>` as a new line within
//!   the existing frontmatter block. We do NOT alphabetize or rewrite
//!   unrelated YAML keys.
//! - If no frontmatter exists, we prepend a minimal `---` block before the
//!   original source, preserving every original byte.

use std::str::FromStr;

use secondbrain_core::id::NoteId;
use secondbrain_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_yaml::Mapping;

/// Parsed note metadata extracted from YAML frontmatter.
///
/// The `id` and `title` fields are convenience accessors for commonly
/// used keys. All other frontmatter keys are available in `properties`.
#[derive(Debug, Clone)]
pub struct NoteMetadata {
    /// The note's stable identifier, if present in frontmatter.
    pub id: Option<NoteId>,
    /// The note's title, if present in frontmatter.
    pub title: Option<String>,
    /// All frontmatter key-value pairs as a YAML mapping.
    pub properties: Mapping,
}

/// The result of a surgical metadata patch operation.
///
/// When `changed` is `false`, `source` is byte-identical to the input
/// and no write is needed.
#[derive(Debug, Clone)]
pub struct MetadataPatch {
    /// Whether the source was modified.
    pub changed: bool,
    /// The patched source string (identical to input if `changed` is false).
    pub source: String,
    /// The note ID that is or was set in the frontmatter.
    pub note_id: NoteId,
}

/// A frontmatter value that can be exchanged losslessly through JSON APIs.
pub type PropertyValue = serde_json::Value;

/// One typed top-level property change. `id` is reserved and cannot be changed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum PropertyEdit {
    Set { key: String, value: PropertyValue },
    Remove { key: String },
}

impl PropertyEdit {
    /// The top-level key affected by this edit.
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::Set { key, .. } | Self::Remove { key } => key,
        }
    }
}

/// Result of a surgical property edit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropertyPatch {
    pub changed: bool,
    pub source: String,
}

/// The UTF-8 BOM as a byte sequence.
const BOM: &[u8; 3] = &[0xEF, 0xBB, 0xBF];

/// Parse note metadata from a Markdown source string.
///
/// If the source begins with a YAML frontmatter block (optionally preceded
/// by a UTF-8 BOM), the frontmatter is parsed into [`NoteMetadata`].
/// If no frontmatter is present, an empty `NoteMetadata` is returned.
///
/// # Errors
///
/// Returns [`Error::InvalidMarkdown`] if the YAML is malformed or if the
/// `id` key appears more than once.
pub fn parse_metadata(source: &str) -> Result<NoteMetadata> {
    let extract = extract_frontmatter(source);

    let properties = match &extract {
        FrontmatterExtract::Present {
            yaml_content_end,
            yaml_content_start,
            ..
        }
        | FrontmatterExtract::BomPresent {
            yaml_content_end,
            yaml_content_start,
            ..
        } => {
            let yaml = &source[*yaml_content_start..*yaml_content_end];
            parse_yaml_mapping(yaml)?
        }
        FrontmatterExtract::Absent => Mapping::new(),
    };

    let id = extract_note_id(&properties)?;
    let title = extract_title(&properties);

    Ok(NoteMetadata {
        id,
        title,
        properties,
    })
}

/// Read JSON-compatible top-level properties, excluding the reserved note ID.
pub fn read_properties(source: &str) -> Result<std::collections::BTreeMap<String, PropertyValue>> {
    let metadata = parse_metadata(source)?;
    metadata
        .properties
        .into_iter()
        .filter(|(key, _)| key.as_str() != Some("id"))
        .map(|(key, value)| {
            let key = key
                .as_str()
                .ok_or_else(|| invalid_frontmatter("property keys must be strings"))?;
            let value = serde_json::to_value(value).map_err(|error| {
                invalid_frontmatter(&format!("property is not JSON-compatible: {error}"))
            })?;
            Ok((key.to_owned(), value))
        })
        .collect()
}

/// Surgically set or remove one safe top-level property.
///
/// Existing frontmatter must be a valid mapping whose top-level layout can be
/// located without interpreting YAML presentation details. Unsupported layouts
/// fail closed rather than risking a nested or unrelated edit.
pub fn edit_property(source: &str, edit: &PropertyEdit) -> Result<PropertyPatch> {
    validate_property_key(edit.key())?;
    let current = read_properties(source)?;
    if matches!(edit, PropertyEdit::Remove { .. }) && !current.contains_key(edit.key()) {
        return Ok(PropertyPatch {
            changed: false,
            source: source.to_owned(),
        });
    }
    if let PropertyEdit::Set { key, value } = edit
        && current.get(key) == Some(value)
    {
        return Ok(PropertyPatch {
            changed: false,
            source: source.to_owned(),
        });
    }

    let line_ending = detect_line_ending(source);
    let rendered = match edit {
        PropertyEdit::Set { key, value } => Some(render_property(key, value, line_ending)?),
        PropertyEdit::Remove { .. } => None,
    };
    let extract = extract_frontmatter(source);
    match extract {
        FrontmatterExtract::Present {
            yaml_content_start,
            yaml_content_end,
        }
        | FrontmatterExtract::BomPresent {
            yaml_content_start,
            yaml_content_end,
        } => {
            let yaml = &source[yaml_content_start..yaml_content_end];
            let spans = top_level_spans(yaml)?;
            let target = spans.iter().find(|span| span.key == edit.key());
            let (start, end) = target.map_or((yaml_content_end, yaml_content_end), |span| {
                (
                    yaml_content_start + span.start,
                    yaml_content_start + span.end,
                )
            });
            let replacement = rendered.unwrap_or_default();
            let mut patched = String::with_capacity(source.len() + replacement.len());
            patched.push_str(&source[..start]);
            patched.push_str(&replacement);
            patched.push_str(&source[end..]);
            Ok(PropertyPatch {
                changed: true,
                source: patched,
            })
        }
        FrontmatterExtract::Absent => {
            let after_bom = source.strip_prefix('\u{feff}').unwrap_or(source);
            if after_bom.starts_with("---\n") || after_bom.starts_with("---\r") {
                return Err(invalid_frontmatter(
                    "frontmatter has no closing --- delimiter",
                ));
            }
            let Some(rendered) = rendered else {
                return Ok(PropertyPatch {
                    changed: false,
                    source: source.to_owned(),
                });
            };
            let bom_len = source.len() - after_bom.len();
            let block = format!("---{line_ending}{rendered}---{line_ending}{line_ending}");
            let mut patched = String::with_capacity(source.len() + block.len());
            patched.push_str(&source[..bom_len]);
            patched.push_str(&block);
            patched.push_str(after_bom);
            Ok(PropertyPatch {
                changed: true,
                source: patched,
            })
        }
    }
}

#[derive(Debug)]
struct PropertySpan {
    key: String,
    start: usize,
    end: usize,
}

fn top_level_spans(yaml: &str) -> Result<Vec<PropertySpan>> {
    let mut starts = Vec::new();
    let mut offset = 0;
    for line in yaml.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        if !content.is_empty() && !content.starts_with([' ', '\t', '#']) {
            let Some((key, _)) = content.split_once(':') else {
                return Err(invalid_frontmatter("unsafe top-level frontmatter layout"));
            };
            validate_layout_key(key)?;
            if starts
                .iter()
                .any(|(_, existing): &(usize, String)| existing == key)
            {
                return Err(invalid_frontmatter("duplicate top-level property key"));
            }
            starts.push((offset, key.to_owned()));
        }
        offset += line.len();
    }
    Ok(starts
        .iter()
        .enumerate()
        .map(|(index, (start, key))| PropertySpan {
            key: key.clone(),
            start: *start,
            end: property_span_end(
                yaml,
                *start,
                starts
                    .get(index + 1)
                    .map_or(yaml.len(), |(start, _)| *start),
            ),
        })
        .collect())
}

fn property_span_end(yaml: &str, start: usize, candidate_end: usize) -> usize {
    let region = &yaml[start..candidate_end];
    let mut offset = 0;
    let mut trailing_comment = None;
    for line in region.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        if content.starts_with('#') || content.is_empty() {
            trailing_comment.get_or_insert(start + offset);
        } else {
            trailing_comment = None;
        }
        offset += line.len();
    }
    trailing_comment.unwrap_or(candidate_end)
}

fn validate_property_key(key: &str) -> Result<()> {
    if key == "id" {
        return Err(invalid_frontmatter("id is reserved and immutable"));
    }
    validate_layout_key(key)
}

fn validate_layout_key(key: &str) -> Result<()> {
    let mut chars = key.chars();
    if !matches!(chars.next(), Some('A'..='Z' | 'a'..='z' | '_'))
        || !chars
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(invalid_frontmatter(
            "property key is not a safe plain YAML key",
        ));
    }
    Ok(())
}

fn render_property(key: &str, value: &PropertyValue, line_ending: &str) -> Result<String> {
    let mut mapping = serde_yaml::Mapping::new();
    let yaml_value = serde_yaml::to_value(value).map_err(|error| {
        invalid_frontmatter(&format!("property cannot be encoded as YAML: {error}"))
    })?;
    mapping.insert(serde_yaml::Value::String(key.to_owned()), yaml_value);
    let rendered = serde_yaml::to_string(&mapping)
        .map_err(|error| {
            invalid_frontmatter(&format!("property cannot be encoded as YAML: {error}"))
        })?
        .replace('\n', line_ending);
    Ok(rendered)
}

fn invalid_frontmatter(summary: &str) -> Error {
    Error::InvalidMarkdown {
        path: std::path::PathBuf::new(),
        summary: summary.to_owned(),
    }
}

/// Ensure that a Markdown source has a `NoteId` in its frontmatter.
///
/// If the source already contains a canonical `id` matching `generated`,
/// no change is made (`changed = false`).
///
/// If the source has frontmatter without an `id`, a new `id:` line is
/// appended within the frontmatter block. All other frontmatter content
/// and the entire body are preserved byte-for-byte.
///
/// If the source has no frontmatter, a minimal block is prepended and
/// the original source follows byte-for-byte.
///
/// # Errors
///
/// Returns an error if the existing frontmatter is malformed YAML or if
/// the `id` key appears more than once.
pub fn ensure_note_id(source: &str, generated: NoteId) -> Result<MetadataPatch> {
    let metadata = parse_metadata(source)?;

    // If the note already has the correct id, no change needed.
    if let Some(existing) = metadata.id
        && existing == generated
    {
        return Ok(MetadataPatch {
            changed: false,
            source: source.to_string(),
            note_id: generated,
        });
    }

    match extract_frontmatter(source) {
        FrontmatterExtract::Present {
            yaml_content_end, ..
        }
        | FrontmatterExtract::BomPresent {
            yaml_content_end, ..
        } => {
            // Insert the id line just before the closing "---" delimiter.
            // We use the source's line ending style for the new line.
            let line_ending = detect_line_ending(source);
            let id_line = format!("id: {generated}{line_ending}");

            // Build the patched source: everything up to (but not including)
            // the closing "---" + id_line + closing "---" + rest of source.
            let mut patched = String::with_capacity(source.len() + id_line.len() + 4);
            patched.push_str(&source[..yaml_content_end]);
            patched.push_str(&id_line);
            patched.push_str(&source[yaml_content_end..]);

            Ok(MetadataPatch {
                changed: true,
                source: patched,
                note_id: generated,
            })
        }
        FrontmatterExtract::Absent => {
            // No frontmatter — prepend a minimal block.
            // Determine the line ending from the source.
            let line_ending = detect_line_ending(source);

            let fm_block = format!("---{le}id: {generated}{le}---{le}", le = line_ending);

            let mut patched = String::with_capacity(fm_block.len() + source.len());
            patched.push_str(&fm_block);
            patched.push_str(source);

            Ok(MetadataPatch {
                changed: true,
                source: patched,
                note_id: generated,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// Result of attempting to extract frontmatter from a source string.
#[derive(Debug)]
enum FrontmatterExtract {
    /// No frontmatter found.
    Absent,
    /// Frontmatter found starting at byte 0.
    Present {
        /// Byte offset where YAML content begins (after the first `---\n`).
        yaml_content_start: usize,
        /// Byte offset where YAML content ends (start of the closing `---` line).
        yaml_content_end: usize,
    },
    /// Frontmatter found after a UTF-8 BOM.
    BomPresent {
        /// Byte offset where YAML content begins.
        yaml_content_start: usize,
        /// Byte offset where YAML content ends.
        yaml_content_end: usize,
    },
}

/// Extract frontmatter from a source string.
///
/// Frontmatter is recognized when:
/// - The source starts with `---\n` or `---\r\n` (possibly preceded by a
///   UTF-8 BOM).
/// - A subsequent line contains exactly `---` (with optional `\r`).
fn extract_frontmatter(source: &str) -> FrontmatterExtract {
    let bytes = source.as_bytes();

    // Check for BOM.
    let (start, bom_present) = if bytes.starts_with(BOM) {
        (BOM.len(), true)
    } else {
        (0, false)
    };

    // Check if the source (after optional BOM) starts with "---\n" or "---\r\n".
    let after_bom = &bytes[start..];
    if !starts_with_delimiter(after_bom) {
        return FrontmatterExtract::Absent;
    }

    // Skip past the opening "---" line (including line ending).
    let content_start = start + skip_line(&bytes[start..]);

    // Find the closing "---" line.
    let content_end = find_closing_delimiter(&bytes[content_start..])
        .map(|offset| content_start + offset)
        .unwrap_or(content_start);

    if content_end == content_start {
        // No closing delimiter found — not valid frontmatter.
        return FrontmatterExtract::Absent;
    }

    // yaml_content_end points to the start of the closing "---" line.
    // Verify there IS a closing delimiter (content_end != content_start
    // already checked above).

    if bom_present {
        FrontmatterExtract::BomPresent {
            yaml_content_start: content_start,
            yaml_content_end: content_end,
        }
    } else {
        FrontmatterExtract::Present {
            yaml_content_start: content_start,
            yaml_content_end: content_end,
        }
    }
}

/// Check if bytes start with "---\n", "---\r\n", or "---\r".
fn starts_with_delimiter(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    &bytes[..3] == b"---" && (bytes[3] == b'\n' || bytes[3] == b'\r')
}

/// Get the length of the first line including its line ending.
fn skip_line(bytes: &[u8]) -> usize {
    for (i, &byte) in bytes.iter().enumerate() {
        if byte == b'\n' {
            return i + 1;
        }
        if byte == b'\r' {
            // Check for \r\n
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                return i + 2;
            }
            return i + 1;
        }
    }
    bytes.len()
}

/// Find the byte offset of a line containing only "---" (with optional \r).
/// Returns the offset relative to the start of the given slice.
fn find_closing_delimiter(bytes: &[u8]) -> Option<usize> {
    let mut pos = 0usize;
    while pos < bytes.len() {
        // Find the next newline.
        let line_end = bytes[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| pos + p)
            .unwrap_or(bytes.len());

        // Extract the current line (without line ending).
        let line = &bytes[pos..line_end];
        let line_trimmed = line.strip_suffix(b"\r").unwrap_or(line);

        if line_trimmed == b"---" {
            return Some(pos);
        }

        pos = if line_end < bytes.len() {
            line_end + 1
        } else {
            bytes.len()
        };
    }
    None
}

/// Detect the dominant line ending style in the source.
fn detect_line_ending(source: &str) -> &'static str {
    let bytes = source.as_bytes();
    let mut lf = 0usize;
    let mut crlf = 0usize;
    let mut cr = 0usize;

    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                crlf += 1;
                i += 2;
                continue;
            }
            cr += 1;
        } else if bytes[i] == b'\n' {
            lf += 1;
        }
        i += 1;
    }

    if crlf >= lf && crlf >= cr && crlf > 0 {
        "\r\n"
    } else if cr > lf && cr > 0 {
        "\r"
    } else {
        "\n"
    }
}

/// Parse a YAML string into a `serde_yaml::Mapping`.
fn parse_yaml_mapping(yaml: &str) -> Result<Mapping> {
    let value: serde_yaml::Value =
        serde_yaml::from_str(yaml).map_err(|e| Error::InvalidMarkdown {
            path: std::path::PathBuf::new(),
            summary: format!("YAML frontmatter parse error: {e}"),
        })?;

    match value {
        serde_yaml::Value::Mapping(mapping) => Ok(mapping),
        serde_yaml::Value::Null => Ok(Mapping::new()),
        _ => Err(Error::InvalidMarkdown {
            path: std::path::PathBuf::new(),
            summary: "YAML frontmatter must be a mapping".to_string(),
        }),
    }
}

/// Extract the `NoteId` from the `id` key in the mapping.
///
/// # Errors
///
/// Returns an error if the `id` key appears more than once, or if the
/// value is not a valid canonical ULID.
fn extract_note_id(properties: &Mapping) -> Result<Option<NoteId>> {
    let id_key = serde_yaml::Value::String("id".to_string());

    // Check for duplicate id keys by counting occurrences.
    let count = properties
        .iter()
        .filter(|(k, _)| k.as_str() == Some("id"))
        .count();

    if count > 1 {
        return Err(Error::InvalidMarkdown {
            path: std::path::PathBuf::new(),
            summary: "duplicate id keys in frontmatter".to_string(),
        });
    }

    let value = properties.get(&id_key);
    match value {
        Some(serde_yaml::Value::String(s)) => {
            let id = NoteId::from_str(s)?;
            Ok(Some(id))
        }
        Some(serde_yaml::Value::Null) | None => Ok(None),
        Some(_) => Err(Error::InvalidMarkdown {
            path: std::path::PathBuf::new(),
            summary: "id must be a string".to_string(),
        }),
    }
}

/// Extract the `title` from the `title` key in the mapping.
fn extract_title(properties: &Mapping) -> Option<String> {
    let title_key = serde_yaml::Value::String("title".to_string());
    match properties.get(&title_key) {
        Some(serde_yaml::Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}
