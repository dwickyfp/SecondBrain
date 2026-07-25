#![forbid(unsafe_code)]

//! SecondBrain Markdown: loss-aware source model.
//!
//! Parses Markdown into positioned semantic nodes while retaining exact source
//! slices for unchanged regions. The original source is kept as an [`Arc<str>`]
//! for zero-copy slicing.

pub mod ast;
pub mod extract;
pub mod frontmatter;
pub mod parse;
pub mod source;

pub use ast::{SemanticKind, SemanticNode};
pub use frontmatter::{MetadataPatch, NoteMetadata, ensure_note_id, parse_metadata};
pub use parse::{Fingerprint, SourceDocument};
pub use source::{LineEnding, SourceSpan};
