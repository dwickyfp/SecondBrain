//! Semantic operations for translating external edits.
//!
//! A [`SemanticOperation`] represents a single deterministic transformation
//! that, when applied to the base source, produces (or contributes to) the
//! incoming source. Operations use [`NodeAnchor`]s — structural paths plus
//! content hashes — to unambiguously identify target nodes.

use crate::ast::SemanticKind;
use crate::source::SourceSpan;
use secondbrain_core::hash::ContentHash;

// ---------------------------------------------------------------------------
// NodeAnchor
// ---------------------------------------------------------------------------

/// A structural path that locates a node within the document tree.
///
/// The path is a sequence of child indices from the root. For example,
/// `[0]` is the first top-level node, `[1, 0]` is the first child of the
/// second top-level node.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct StructuralPath {
    /// Child indices from root downward.
    pub indices: Vec<usize>,
}

impl StructuralPath {
    /// Create a new structural path.
    #[must_use]
    pub fn new(indices: Vec<usize>) -> Self {
        Self { indices }
    }

    /// Create a root-level path (a top-level node).
    #[must_use]
    pub fn root(index: usize) -> Self {
        Self {
            indices: vec![index],
        }
    }

    /// Whether this is a root-level path (depth 1).
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.indices.len() == 1
    }
}

/// An anchor that identifies a node by structural path and content hash.
///
/// The structural path locates the node's position in the tree; the content
/// hash verifies the node's identity (its raw source bytes). Together they
/// provide unambiguous identification even when the document has duplicate
/// paragraphs or similar nodes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct NodeAnchor {
    /// Structural path from root to the node.
    pub path: StructuralPath,
    /// Semantic kind of the node at this path.
    pub kind: SemanticKind,
    /// BLAKE3 hash of the node's raw source bytes.
    pub content_hash: ContentHash,
}

impl NodeAnchor {
    /// Create a new node anchor.
    #[must_use]
    pub fn new(path: StructuralPath, kind: SemanticKind, content_hash: ContentHash) -> Self {
        Self {
            path,
            kind,
            content_hash,
        }
    }
}

// ---------------------------------------------------------------------------
// SemanticOperation
// ---------------------------------------------------------------------------

/// A single semantic transformation derived from a document diff.
///
/// Each variant carries the information needed to apply the operation
/// deterministically to the base source. Operations are designed so that
/// [`crate::apply::apply_operations`] can produce the incoming source.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SemanticOperation {
    /// Insert a new node after the anchor node.
    ///
    /// The `anchor` identifies an existing node in the base document. The
    /// `content` is the raw source text of the new node to insert immediately
    /// after the anchor (including any necessary blank-line separators).
    InsertNode {
        /// The node after which to insert (use a zero-path anchor for insert
        /// at the very beginning).
        anchor: Option<NodeAnchor>,
        /// Raw source text of the new node to insert.
        content: String,
    },

    /// Delete the node identified by the anchor.
    DeleteNode {
        /// The node to delete.
        anchor: NodeAnchor,
    },

    /// Replace the node identified by the anchor with new content.
    ///
    /// Used for text updates, heading renames, task-state toggles, and raw
    /// block replacements.
    ReplaceNode {
        /// The node to replace.
        anchor: NodeAnchor,
        /// Raw source text of the replacement node.
        content: String,
    },

    /// Move a node to a new position in the document.
    ///
    /// The `anchor` identifies the node to move; `target_index` is the new
    /// root-level index where the node should appear after the move.
    /// For list-item reordering, `content` carries the fully reordered list
    /// source text (since the apply layer needs it to reconstruct the list).
    MoveNode {
        /// The node to move (for list reordering, this is the list container).
        anchor: NodeAnchor,
        /// New root-level index for the node.
        target_index: usize,
        /// For list-item moves: the fully reordered list source text.
        /// For simple node moves: empty (the apply layer reorders by index).
        content: String,
    },

    /// Set or update a frontmatter property.
    SetProperty {
        /// Property key.
        key: String,
        /// New property value (as a YAML scalar string).
        value: String,
    },

    /// Remove a frontmatter property.
    RemoveProperty {
        /// Property key to remove.
        key: String,
    },

    /// The diff is ambiguous and requires human review.
    ///
    /// Emitted when the diff cannot unambiguously determine which node
    /// changed (e.g., duplicate paragraphs). The `reason` explains the
    /// ambiguity. When this operation is present, `apply_operations` should
    /// return the incoming source as-is (no destructive automatic edits).
    NeedsReview {
        /// Human-readable explanation of the ambiguity.
        reason: String,
        /// The byte span in the base document where ambiguity exists (if
        /// known).
        span: Option<SourceSpan>,
    },
}

impl SemanticOperation {
    /// Returns a short kind name for this operation (for assertions/debug).
    #[must_use]
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::InsertNode { .. } => "InsertNode",
            Self::DeleteNode { .. } => "DeleteNode",
            Self::ReplaceNode { .. } => "ReplaceNode",
            Self::MoveNode { .. } => "MoveNode",
            Self::SetProperty { .. } => "SetProperty",
            Self::RemoveProperty { .. } => "RemoveProperty",
            Self::NeedsReview { .. } => "NeedsReview",
        }
    }
}
