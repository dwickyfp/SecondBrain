//! Semantic AST types for the loss-aware source model.

use crate::source::SourceSpan;

/// Classification of a source region.
///
/// `Raw` covers any byte range not represented by a supported semantic node,
/// ensuring every byte of the original source is accounted for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SemanticKind {
    /// Document root container.
    Root,
    /// ATX or Setext heading.
    Heading,
    /// Paragraph block.
    Paragraph,
    /// Fenced or indented code block.
    Code,
    /// Math block (`$$...$$`).
    Math,
    /// Block quote.
    Blockquote,
    /// List container.
    List,
    /// List item.
    ListItem,
    /// Thematic break (`---`, `***`, `___`).
    ThematicBreak,
    /// GFM table.
    Table,
    /// Table row.
    TableRow,
    /// Table cell.
    TableCell,
    /// YAML or TOML frontmatter.
    Frontmatter,
    /// HTML block or inline — preserved as opaque source.
    Html,
    /// Inline code span.
    InlineCode,
    /// Inline math span.
    InlineMath,
    /// Strong emphasis (`**bold**`).
    Strong,
    /// Emphasis (`*italic*`).
    Emphasis,
    /// Strikethrough (`~~text~~`).
    Delete,
    /// Link.
    Link,
    /// Image.
    Image,
    /// Footnote definition.
    FootnoteDefinition,
    /// Footnote reference.
    FootnoteReference,
    /// Definition (link reference target).
    Definition,
    /// Text content leaf.
    Text,
    /// Line break.
    Break,
    /// Any byte range not covered by a semantic node above.
    ///
    /// This includes unrecognized constructs, inter-node whitespace, and
    /// line-ending bytes that fall between semantic nodes.
    Raw,
}

/// A positioned semantic node: a classified byte span into the original source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticNode {
    /// Semantic classification of this region.
    pub kind: SemanticKind,
    /// Byte range `[start, end)` into the original source string.
    pub span: SourceSpan,
    /// Child nodes (for container nodes like `Root`, `List`, `Blockquote`).
    pub children: Vec<SemanticNode>,
}

impl SemanticNode {
    /// Create a leaf node with no children.
    #[must_use]
    pub fn leaf(kind: SemanticKind, span: SourceSpan) -> Self {
        Self {
            kind,
            span,
            children: Vec::new(),
        }
    }

    /// Create a container node with children.
    #[must_use]
    pub fn container(kind: SemanticKind, span: SourceSpan, children: Vec<SemanticNode>) -> Self {
        Self {
            kind,
            span,
            children,
        }
    }

    /// Extract the exact source slice for this node.
    ///
    /// Zero-copy: returns a `&str` slice of the original source.
    #[must_use]
    pub fn raw<'a>(&self, source: &'a str) -> &'a str {
        // Byte offsets from markdown-rs positions are valid UTF-8 boundaries
        // because the parser operates on the same `&str`.
        &source[self.span.start..self.span.end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_node_has_no_children() {
        let node = SemanticNode::leaf(SemanticKind::Heading, SourceSpan::new(0, 7));
        assert!(node.children.is_empty());
    }

    #[test]
    fn raw_exacts_source_slice() {
        let source = "# Héllo";
        // "# Héllo" = 8 bytes (é is 2 bytes in UTF-8)
        let node = SemanticNode::leaf(SemanticKind::Heading, SourceSpan::new(0, 8));
        assert_eq!(node.raw(source), "# Héllo");
    }
}
