//! Loss-aware Markdown parser: converts `markdown-rs` mdast into positioned
//! semantic nodes with exact byte-span preservation.

use markdown::mdast::Node;
use markdown::{Constructs, ParseOptions, to_mdast};
use std::sync::Arc;

use crate::ast::{SemanticKind, SemanticNode};
use crate::source::{LineEnding, SourceSpan};

/// A parsed Markdown document that retains the complete original source.
///
/// The source is stored as [`Arc<str>`] for cheap zero-copy cloning. Every
/// semantic node carries a [`SourceSpan`] that is a byte range into the
/// original source, enabling lossless reconstruction.
#[derive(Debug, Clone)]
pub struct SourceDocument {
    /// The complete original source text.
    source: Arc<str>,
    /// Root semantic node (container for all top-level nodes).
    root: SemanticNode,
    /// Detected line ending style.
    line_ending: LineEnding,
}

/// Error returned when parsing fails.
///
/// `markdown-rs` only errors on MDX syntax; with our options (no MDX) parsing
/// is infallible in practice, but we surface the error type for completeness.
#[derive(Debug)]
pub struct ParseError(markdown::message::Message);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "markdown parse error: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

impl From<markdown::message::Message> for ParseError {
    fn from(msg: markdown::message::Message) -> Self {
        Self(msg)
    }
}

impl SourceDocument {
    /// Parse a Markdown source string into a positioned source document.
    ///
    /// Enables GFM (tables, strikethrough, task lists, autolinks, footnotes),
    /// frontmatter, and math constructs.
    ///
    /// # Errors
    ///
    /// Only fails for MDX syntax errors, which are not enabled by default.
    pub fn parse(source: &str) -> Result<Self, ParseError> {
        let constructs = Constructs {
            frontmatter: true,
            math_flow: true,
            math_text: true,
            ..Constructs::gfm()
        };
        let options = ParseOptions {
            constructs,
            ..ParseOptions::default()
        };

        let mdast = to_mdast(source, &options)?;
        let line_ending = LineEnding::detect(source);
        let source_arc: Arc<str> = Arc::from(source);
        let root = convert_node(&mdast, source);

        Ok(Self {
            source: source_arc,
            root,
            line_ending,
        })
    }

    /// The complete original source text.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The root semantic node.
    #[must_use]
    pub fn root(&self) -> &SemanticNode {
        &self.root
    }

    /// Flat list of all semantic nodes (depth-first traversal, excluding root).
    #[must_use]
    pub fn nodes(&self) -> Vec<&SemanticNode> {
        let mut result = Vec::new();
        collect_nodes(&self.root, &mut result);
        result
    }

    /// Detected line ending style.
    #[must_use]
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    /// Reconstruct the original source from semantic node spans.
    ///
    /// Walks the source byte-by-byte: every byte covered by a semantic node
    /// is taken from that node's span; gaps between nodes are filled from the
    /// source as opaque `Raw` slices. The result is byte-identical to the
    /// original source.
    #[must_use]
    pub fn reconstruct(&self) -> String {
        // Collect all leaf spans in source order.
        let mut spans: Vec<SourceSpan> = Vec::new();
        collect_leaf_spans(&self.root, &mut spans);
        spans.sort_by_key(|s| s.start);

        let source_bytes = self.source.as_bytes();
        let mut result = Vec::with_capacity(source_bytes.len());
        let mut cursor = 0usize;

        for span in &spans {
            // Fill any gap before this span from the source.
            if span.start > cursor {
                result.extend_from_slice(&source_bytes[cursor..span.start]);
            }
            // The span itself (only if it starts at/after cursor to avoid
            // double-counting overlapping container spans — we use leaf-only).
            if span.end > cursor {
                let take_from = cursor.max(span.start);
                result.extend_from_slice(&source_bytes[take_from..span.end]);
                cursor = span.end;
            }
        }

        // Trailing bytes after the last span.
        if cursor < source_bytes.len() {
            result.extend_from_slice(&source_bytes[cursor..]);
        }

        // Safety: the original source is valid UTF-8, and we only slice at
        // byte offsets that come from markdown-rs positions (which are
        // char-boundary-aligned) or at gap boundaries between spans (also
        // char-boundary-aligned because spans are).
        String::from_utf8(result).expect("reconstructed bytes must be valid UTF-8")
    }
}

/// Recursively collect all nodes depth-first (excluding root itself).
fn collect_nodes<'a>(node: &'a SemanticNode, out: &mut Vec<&'a SemanticNode>) {
    for child in &node.children {
        out.push(child);
        collect_nodes(child, out);
    }
}

/// Collect only leaf spans (nodes with no children) in source order.
///
/// Leaf nodes have non-overlapping spans that tile the source. Container
/// spans are skipped to avoid double-counting.
fn collect_leaf_spans(node: &SemanticNode, out: &mut Vec<SourceSpan>) {
    if node.children.is_empty() {
        if !node.span.is_empty() {
            out.push(node.span);
        }
    } else {
        for child in &node.children {
            collect_leaf_spans(child, out);
        }
    }
}

/// Convert a `markdown-rs` mdast node into a [`SemanticNode`].
fn convert_node(node: &Node, source: &str) -> SemanticNode {
    let kind = classify(node);
    let span = span_of(node, source);

    let children: Vec<SemanticNode> = match node {
        Node::Root(root) => convert_children(&root.children, source),
        Node::Blockquote(bq) => convert_children(&bq.children, source),
        Node::List(list) => convert_children(&list.children, source),
        Node::ListItem(item) => convert_children(&item.children, source),
        Node::Paragraph(para) => convert_children(&para.children, source),
        Node::Heading(h) => convert_children(&h.children, source),
        Node::Table(table) => convert_children(&table.children, source),
        Node::TableRow(row) => convert_children(&row.children, source),
        Node::TableCell(cell) => convert_children(&cell.children, source),
        Node::Emphasis(em) => convert_children(&em.children, source),
        Node::Strong(s) => convert_children(&s.children, source),
        Node::Delete(d) => convert_children(&d.children, source),
        Node::Link(l) => convert_children(&l.children, source),
        Node::FootnoteDefinition(fd) => convert_children(&fd.children, source),
        _ => Vec::new(),
    };

    // If a container node has no position, fall back to covering its children.
    let final_span = if span.is_empty() && !children.is_empty() {
        let start = children.first().map(|c| c.span.start).unwrap_or(0);
        let end = children.last().map(|c| c.span.end).unwrap_or(0);
        SourceSpan::new(start, end)
    } else {
        span
    };

    if children.is_empty() {
        SemanticNode::leaf(kind, final_span)
    } else {
        SemanticNode::container(kind, final_span, children)
    }
}

/// Convert a slice of mdast child nodes.
fn convert_children(children: &[Node], source: &str) -> Vec<SemanticNode> {
    children.iter().map(|c| convert_node(c, source)).collect()
}

/// Classify an mdast node into a [`SemanticKind`].
fn classify(node: &Node) -> SemanticKind {
    match node {
        Node::Root(_) => SemanticKind::Root,
        Node::Heading(_) => SemanticKind::Heading,
        Node::Paragraph(_) => SemanticKind::Paragraph,
        Node::Code(_) => SemanticKind::Code,
        Node::Math(_) => SemanticKind::Math,
        Node::Blockquote(_) => SemanticKind::Blockquote,
        Node::List(_) => SemanticKind::List,
        Node::ListItem(_) => SemanticKind::ListItem,
        Node::ThematicBreak(_) => SemanticKind::ThematicBreak,
        Node::Table(_) => SemanticKind::Table,
        Node::TableRow(_) => SemanticKind::TableRow,
        Node::TableCell(_) => SemanticKind::TableCell,
        Node::Yaml(_) | Node::Toml(_) => SemanticKind::Frontmatter,
        Node::Html(_) => SemanticKind::Html,
        Node::InlineCode(_) => SemanticKind::InlineCode,
        Node::InlineMath(_) => SemanticKind::InlineMath,
        Node::Strong(_) => SemanticKind::Strong,
        Node::Emphasis(_) => SemanticKind::Emphasis,
        Node::Delete(_) => SemanticKind::Delete,
        Node::Link(_) => SemanticKind::Link,
        Node::Image(_) => SemanticKind::Image,
        Node::FootnoteDefinition(_) => SemanticKind::FootnoteDefinition,
        Node::FootnoteReference(_) => SemanticKind::FootnoteReference,
        Node::Definition(_) => SemanticKind::Definition,
        Node::Text(_) => SemanticKind::Text,
        Node::Break(_) => SemanticKind::Break,
        // MDX and other exotic nodes become Raw — preserved as opaque source.
        _ => SemanticKind::Raw,
    }
}

/// Extract the byte span from an mdast node's position.
///
/// `markdown-rs` positions use byte offsets (not char offsets) for the
/// `Point.offset` field, so we can use them directly.
fn span_of(node: &Node, source: &str) -> SourceSpan {
    let position = match node {
        Node::Root(root) => root.position.as_ref(),
        Node::Heading(h) => h.position.as_ref(),
        Node::Paragraph(p) => p.position.as_ref(),
        Node::Code(c) => c.position.as_ref(),
        Node::Math(m) => m.position.as_ref(),
        Node::Blockquote(b) => b.position.as_ref(),
        Node::List(l) => l.position.as_ref(),
        Node::ListItem(li) => li.position.as_ref(),
        Node::ThematicBreak(tb) => tb.position.as_ref(),
        Node::Table(t) => t.position.as_ref(),
        Node::TableRow(tr) => tr.position.as_ref(),
        Node::TableCell(tc) => tc.position.as_ref(),
        Node::Yaml(y) => y.position.as_ref(),
        Node::Toml(t) => t.position.as_ref(),
        Node::Html(h) => h.position.as_ref(),
        Node::InlineCode(ic) => ic.position.as_ref(),
        Node::InlineMath(im) => im.position.as_ref(),
        Node::Strong(s) => s.position.as_ref(),
        Node::Emphasis(e) => e.position.as_ref(),
        Node::Delete(d) => d.position.as_ref(),
        Node::Link(l) => l.position.as_ref(),
        Node::Image(i) => i.position.as_ref(),
        Node::FootnoteDefinition(fd) => fd.position.as_ref(),
        Node::FootnoteReference(fr) => fr.position.as_ref(),
        Node::Definition(d) => d.position.as_ref(),
        Node::Text(t) => t.position.as_ref(),
        Node::Break(b) => b.position.as_ref(),
        _ => None,
    };

    match position {
        Some(pos) => {
            let start = pos.start.offset;
            let end = pos.end.offset;
            // Clamp to source length for safety.
            let max = source.len();
            SourceSpan::new(start.min(max), end.min(max))
        }
        None => SourceSpan::new(0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_heading() {
        let source = "# Title\n";
        let doc = SourceDocument::parse(source).expect("parse");
        assert_eq!(doc.source(), source);
        assert_eq!(doc.line_ending(), LineEnding::Lf);
    }

    #[test]
    fn parse_empty() {
        let doc = SourceDocument::parse("").expect("parse");
        assert_eq!(doc.source(), "");
        assert_eq!(doc.reconstruct(), "");
    }

    #[test]
    fn nodes_returns_flat_list() {
        let source = "# H\n\nText.\n";
        let doc = SourceDocument::parse(source).expect("parse");
        let nodes = doc.nodes();
        assert!(!nodes.is_empty());
    }
}
