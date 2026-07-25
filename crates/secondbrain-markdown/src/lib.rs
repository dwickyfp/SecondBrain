#![forbid(unsafe_code)]

//! SecondBrain Markdown: loss-aware source model.
//!
//! Parses Markdown into positioned semantic nodes while retaining exact source
//! slices for unchanged regions. The original source is kept as an [`Arc<str>`]
//! for zero-copy slicing.

pub mod ast;
pub mod parse;
pub mod source;

pub use ast::{SemanticKind, SemanticNode};
pub use parse::SourceDocument;
pub use source::{LineEnding, SourceSpan};
