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
