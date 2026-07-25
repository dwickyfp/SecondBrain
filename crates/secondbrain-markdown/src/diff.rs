//! Semantic document diffing.
//!
//! [`diff_documents`] compares a base [`SourceDocument`] against an incoming
//! [`SourceDocument`] and produces a [`Vec<SemanticOperation>`] that, when
//! applied, transforms the base into the incoming.
//!
//! # Strategy
//!
//! 1. **Frontmatter diff**: Compare properties from both documents. Changed
//!    keys produce `SetProperty`; removed keys produce `RemoveProperty`.
//! 2. **Top-level node diff**: Walk root-level children of both documents.
//!    - Nodes present in both with matching content hash → unchanged, skip.
//!    - Nodes present in base only → `DeleteNode`.
//!    - Nodes present in incoming only → `InsertNode`.
//!    - Nodes present in both with different content hash → `ReplaceNode`
//!      (when the structural position is unambiguous) or `NeedsReview`
//!      (when duplicate nodes create ambiguity).
//! 3. **Move detection**: If a node's content hash matches a node at a
//!    different position, emit `MoveNode` (only when identity is
//!    unambiguous — i.e., the content hash appears exactly once in both
//!    documents).
//!
//! The diff is deterministic: repeated calls on the same inputs always
//! produce identical operations.

use crate::SourceDocument;
use crate::ast::{SemanticKind, SemanticNode};
use crate::frontmatter::parse_metadata;
use crate::operation::{NodeAnchor, SemanticOperation, StructuralPath};

use secondbrain_core::hash::ContentHash;
use serde_yaml::Value;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Diff a base document against an incoming document.
///
/// Returns a list of [`SemanticOperation`]s that transform the base into
/// the incoming. The operations are deterministic and ordered frontmatter-
/// first, then node operations in source order.
///
/// # Panics
///
/// This function does not panic.
#[must_use]
#[allow(clippy::collapsible_if)]
pub fn diff_documents(base: &SourceDocument, incoming: &SourceDocument) -> Vec<SemanticOperation> {
    let mut operations = Vec::new();

    // 1. Diff frontmatter properties.
    operations.extend(diff_frontmatter(base.source(), incoming.source()));

    // 2. Diff top-level nodes.
    operations.extend(diff_top_level_nodes(base, incoming));

    operations
}

// ---------------------------------------------------------------------------
// Frontmatter diffing
// ---------------------------------------------------------------------------

/// Diff frontmatter properties between base and incoming sources.
fn diff_frontmatter(base_source: &str, incoming_source: &str) -> Vec<SemanticOperation> {
    let base_meta = match parse_metadata(base_source) {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };
    let incoming_meta = match parse_metadata(incoming_source) {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    let mut ops = Vec::new();

    // Check for changed or new properties.
    for (key, incoming_value) in &incoming_meta.properties {
        let key_str = match key {
            Value::String(s) => s.as_str(),
            _ => continue,
        };

        let base_value = base_meta.properties.get(key);
        match base_value {
            None => {
                // New property.
                ops.push(SemanticOperation::SetProperty {
                    key: key_str.to_string(),
                    value: yaml_to_string(incoming_value),
                });
            }
            Some(base_value) if base_value != incoming_value => {
                // Changed property.
                ops.push(SemanticOperation::SetProperty {
                    key: key_str.to_string(),
                    value: yaml_to_string(incoming_value),
                });
            }
            _ => {}
        }
    }

    // Check for removed properties.
    for (key, _) in &base_meta.properties {
        let key_str = match key {
            Value::String(s) => s.as_str(),
            _ => continue,
        };
        if !incoming_meta.properties.contains_key(key) {
            ops.push(SemanticOperation::RemoveProperty {
                key: key_str.to_string(),
            });
        }
    }

    ops
}

/// Convert a YAML value to a YAML scalar string for the SetProperty operation.
fn yaml_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => "~".to_string(),
        _ => {
            // For complex values (sequences, mappings), serialize to YAML.
            serde_yaml::to_string(value)
                .unwrap_or_default()
                .trim()
                .to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level node diffing
// ---------------------------------------------------------------------------

/// A node entry with its anchor, content, and structural path.
#[derive(Debug, Clone)]
struct NodeEntry {
    /// The semantic node.
    node: SemanticNode,
    /// Structural path (root-level index).
    path: StructuralPath,
    /// Content hash of the node's raw source.
    content_hash: ContentHash,
    /// Raw source text of the node.
    raw: String,
}

/// Collect top-level (root-level) nodes from a document as entries.
///
/// Frontmatter nodes are excluded — they are handled by the frontmatter
/// diff layer (`diff_frontmatter`) which produces `SetProperty` /
/// `RemoveProperty` operations.
fn collect_top_level_entries(doc: &SourceDocument) -> Vec<NodeEntry> {
    let source = doc.source();
    let mut index = 0usize;
    doc.root()
        .children
        .iter()
        .filter(|node| {
            // Skip frontmatter nodes — handled by diff_frontmatter.
            node.kind != SemanticKind::Frontmatter
        })
        .map(|node| {
            let raw = node.raw(source).to_string();
            let hash = ContentHash::digest(raw.as_bytes());
            let path = StructuralPath::root(index);
            index += 1;
            NodeEntry {
                node: node.clone(),
                path,
                content_hash: hash,
                raw,
            }
        })
        .collect()
}

/// Diff top-level nodes between base and incoming documents.
#[allow(clippy::collapsible_if)]
fn diff_top_level_nodes(
    base: &SourceDocument,
    incoming: &SourceDocument,
) -> Vec<SemanticOperation> {
    let base_nodes = collect_top_level_entries(base);
    let incoming_nodes = collect_top_level_entries(incoming);

    let mut ops = Vec::new();

    // Build a map of content hash → count for both documents to detect
    // duplicates and enable move detection.
    let base_hash_counts = count_hashes(&base_nodes);
    let incoming_hash_counts = count_hashes(&incoming_nodes);

    // Check for ambiguity: if the same content hash appears multiple times
    // in base AND the incoming has changes to that content, we emit NeedsReview.
    if has_ambiguous_changes(&base_nodes, &incoming_nodes, &base_hash_counts) {
        // Embed the incoming source in the reason so apply_operations can
        // recover it and return it as the result.
        let reason = crate::apply::embed_incoming_in_reason(
            "duplicate nodes with content changes create ambiguous diff",
            incoming.source(),
        );
        ops.push(SemanticOperation::NeedsReview { reason, span: None });
        return ops;
    }

    // Detect moves: a content hash present in both but at different indices.
    // Only when the hash appears exactly once in both (unambiguous identity).
    let mut moved_incoming: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut deleted_base: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for (b_idx, b_entry) in base_nodes.iter().enumerate() {
        if base_hash_counts
            .get(&b_entry.content_hash)
            .copied()
            .unwrap_or(0)
            != 1
        {
            continue;
        }
        // Look for this hash in incoming at a different position.
        if let Some(i_idx) = incoming_nodes
            .iter()
            .position(|e| e.content_hash == b_entry.content_hash)
        {
            if incoming_hash_counts
                .get(&b_entry.content_hash)
                .copied()
                .unwrap_or(0)
                == 1
                && b_idx != i_idx
            {
                // This is a move.
                ops.push(SemanticOperation::MoveNode {
                    anchor: NodeAnchor::new(
                        b_entry.path.clone(),
                        b_entry.node.kind,
                        b_entry.content_hash,
                    ),
                    target_index: i_idx,
                    content: String::new(),
                });
                moved_incoming.insert(i_idx);
                deleted_base.insert(b_idx);
            }
        }
    }

    // Walk both node lists to find deletions, insertions, and replacements.
    // We use a greedy alignment: walk base and incoming in parallel.
    let mut b = 0usize; // base cursor
    let mut i = 0usize; // incoming cursor

    while b < base_nodes.len() || i < incoming_nodes.len() {
        if b < base_nodes.len() && deleted_base.contains(&b) {
            // Already handled as a move source — skip.
            b += 1;
            continue;
        }
        if i < incoming_nodes.len() && moved_incoming.contains(&i) {
            // Already handled as a move target — skip.
            i += 1;
            continue;
        }

        if b < base_nodes.len() && i < incoming_nodes.len() {
            let base_entry = &base_nodes[b];
            let incoming_entry = &incoming_nodes[i];

            if base_entry.content_hash == incoming_entry.content_hash {
                // Unchanged — advance both.
                b += 1;
                i += 1;
                continue;
            }

            // Content differs. Check if base node was deleted (i.e., it
            // doesn't appear anywhere in incoming at all) vs replaced.
            let base_exists_in_incoming = incoming_nodes
                .iter()
                .any(|e| e.content_hash == base_entry.content_hash);

            let incoming_exists_in_base = base_nodes
                .iter()
                .any(|e| e.content_hash == incoming_entry.content_hash);

            if !base_exists_in_incoming && !incoming_exists_in_base {
                // Pure replacement: same position, different content.
                // But check if this is a list whose items were reordered
                // (a move within the list).
                if base_entry.node.kind == SemanticKind::List
                    && incoming_entry.node.kind == SemanticKind::List
                {
                    if let Some(list_ops) = try_diff_list_items(
                        &base_entry.node,
                        &incoming_entry.node,
                        base.source(),
                        incoming.source(),
                        &base_entry.path,
                    ) {
                        ops.extend(list_ops);
                        b += 1;
                        i += 1;
                        continue;
                    }
                }

                ops.push(SemanticOperation::ReplaceNode {
                    anchor: NodeAnchor::new(
                        base_entry.path.clone(),
                        base_entry.node.kind,
                        base_entry.content_hash,
                    ),
                    content: incoming_entry.raw.clone(),
                });
                b += 1;
                i += 1;
                continue;
            }

            if !base_exists_in_incoming && incoming_exists_in_base {
                // Base node was deleted (its content is gone from incoming),
                // but the incoming node at this position exists elsewhere in
                // base — this is a deletion of base node.
                ops.push(SemanticOperation::DeleteNode {
                    anchor: NodeAnchor::new(
                        base_entry.path.clone(),
                        base_entry.node.kind,
                        base_entry.content_hash,
                    ),
                });
                b += 1;
                continue;
            }

            if base_exists_in_incoming && !incoming_exists_in_base {
                // Incoming node is new (inserted); base node still exists
                // elsewhere in incoming — this is an insertion.
                // Find the anchor: the previous base node (or None for
                // insert at beginning).
                let anchor = if b > 0 {
                    let prev = &base_nodes[b - 1];
                    Some(NodeAnchor::new(
                        prev.path.clone(),
                        prev.node.kind,
                        prev.content_hash,
                    ))
                } else {
                    None
                };
                ops.push(SemanticOperation::InsertNode {
                    anchor,
                    content: incoming_entry.raw.clone(),
                });
                i += 1;
                continue;
            }

            // Both exist elsewhere — ambiguous, but we already checked for
            // ambiguity above. Fall through to replacement.
            ops.push(SemanticOperation::ReplaceNode {
                anchor: NodeAnchor::new(
                    base_entry.path.clone(),
                    base_entry.node.kind,
                    base_entry.content_hash,
                ),
                content: incoming_entry.raw.clone(),
            });
            b += 1;
            i += 1;
            continue;
        }

        // Remaining base nodes are deletions.
        if b < base_nodes.len() {
            let base_entry = &base_nodes[b];
            if !deleted_base.contains(&b) {
                ops.push(SemanticOperation::DeleteNode {
                    anchor: NodeAnchor::new(
                        base_entry.path.clone(),
                        base_entry.node.kind,
                        base_entry.content_hash,
                    ),
                });
            }
            b += 1;
            continue;
        }

        // Remaining incoming nodes are insertions.
        if i < incoming_nodes.len() {
            if !moved_incoming.contains(&i) {
                let incoming_entry = &incoming_nodes[i];
                // Anchor: last base node (or None).
                let anchor = if !base_nodes.is_empty() {
                    let last = base_nodes.last().unwrap();
                    Some(NodeAnchor::new(
                        last.path.clone(),
                        last.node.kind,
                        last.content_hash,
                    ))
                } else {
                    None
                };
                ops.push(SemanticOperation::InsertNode {
                    anchor,
                    content: incoming_entry.raw.clone(),
                });
            }
            i += 1;
            continue;
        }
    }

    ops
}

/// Try to diff list items within a List node.
///
/// If both nodes are Lists with the same number of items (same set of
/// content hashes), and the items are just reordered, emit a single
/// `MoveNode` operation for the list.
///
/// Returns `Some(ops)` if a move was detected, or `None` if the list
/// items have actual content changes (fall back to replacement).
fn try_diff_list_items(
    base_list: &SemanticNode,
    incoming_list: &SemanticNode,
    base_source: &str,
    incoming_source: &str,
    base_path: &StructuralPath,
) -> Option<Vec<SemanticOperation>> {
    // Both must be lists with children.
    if base_list.kind != SemanticKind::List || incoming_list.kind != SemanticKind::List {
        return None;
    }

    let base_items: Vec<&SemanticNode> = base_list.children.iter().collect();
    let incoming_items: Vec<&SemanticNode> = incoming_list.children.iter().collect();

    // Must have the same number of items.
    if base_items.len() != incoming_items.len() {
        return None;
    }

    // Compute content hashes for all items.
    let base_hashes: Vec<ContentHash> = base_items
        .iter()
        .map(|item| ContentHash::digest(item.raw(base_source).as_bytes()))
        .collect();
    let incoming_hashes: Vec<ContentHash> = incoming_items
        .iter()
        .map(|item| ContentHash::digest(item.raw(incoming_source).as_bytes()))
        .collect();

    // Check that the set of hashes is identical (same items, just reordered).
    let mut base_sorted = base_hashes.clone();
    base_sorted.sort();
    let mut incoming_sorted = incoming_hashes.clone();
    incoming_sorted.sort();
    if base_sorted != incoming_sorted {
        return None; // Items have actual content changes — not a move.
    }

    // Check for uniqueness: each hash should appear exactly once in base
    // (otherwise the move is ambiguous).
    let base_counts = {
        let mut counts = std::collections::HashMap::new();
        for h in &base_hashes {
            *counts.entry(*h).or_insert(0) += 1;
        }
        counts
    };
    if base_counts.values().any(|&c| c > 1) {
        return None; // Duplicate items — ambiguous move.
    }

    // Find the first item that moved (for the MoveNode operation).
    // We emit a single MoveNode for the entire list.
    let mut moved_from = None;
    let mut moved_to = None;
    for (idx, (bh, ih)) in base_hashes.iter().zip(incoming_hashes.iter()).enumerate() {
        if bh != ih {
            // Found a mismatch — find where the base item at idx went.
            if let Some(target) = incoming_hashes.iter().position(|h| h == bh) {
                moved_from = Some(idx);
                moved_to = Some(target);
                break;
            }
        }
    }

    // If items are just reordered, emit a MoveNode for the list.
    // The content is the incoming list's raw source text so the apply
    // layer can replace the list's content with the reordered version.
    if let (Some(_from), Some(to)) = (moved_from, moved_to) {
        let anchor = NodeAnchor::new(
            base_path.clone(),
            base_list.kind,
            ContentHash::digest(base_list.raw(base_source).as_bytes()),
        );

        // The incoming list's raw source is the reordered content.
        let reordered_content = incoming_list.raw(incoming_source).to_string();

        Some(vec![SemanticOperation::MoveNode {
            anchor,
            target_index: to,
            content: reordered_content,
        }])
    } else {
        // Items are identical in order — no move.
        Some(vec![])
    }
}

/// Count occurrences of each content hash in a list of node entries.
fn count_hashes(entries: &[NodeEntry]) -> std::collections::HashMap<ContentHash, usize> {
    let mut counts = std::collections::HashMap::new();
    for entry in entries {
        *counts.entry(entry.content_hash).or_insert(0) += 1;
    }
    counts
}

/// Check if the diff has ambiguous changes due to duplicate nodes.
///
/// Returns true if there are duplicate nodes in base that have content
/// changes in the incoming document, making it impossible to unambiguously
/// determine which node changed.
fn has_ambiguous_changes(
    _base_nodes: &[NodeEntry],
    incoming_nodes: &[NodeEntry],
    base_hash_counts: &std::collections::HashMap<ContentHash, usize>,
) -> bool {
    // For each content hash that appears multiple times in base, check if
    // the incoming document has a different number of that hash (indicating
    // one of the duplicates was changed).
    for (hash, &count) in base_hash_counts {
        if count < 2 {
            continue;
        }
        let incoming_count = incoming_nodes
            .iter()
            .filter(|e| &e.content_hash == hash)
            .count();
        // If the count changed, some duplicates were modified — ambiguous.
        if incoming_count != count {
            return true;
        }
    }
    false
}
