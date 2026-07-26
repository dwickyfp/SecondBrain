#![forbid(unsafe_code)]

//! Rebuildable, derived SQLite index for a SecondBrain workspace.

mod database;
mod import;
mod indexer;
mod query;

pub use database::{Error, IndexDatabase, QueryValidationError, Result};
pub use import::{
    IMPORT_PREVIEW_FORMAT, ImportApplyOutcome, ImportError, ImportInventory, ImportIssue,
    ImportPreview, PlannedWrites, apply_import, preview_import,
};
pub use indexer::{
    DumpLink, DumpNote, INDEX_SCHEMA_VERSION, IndexConfig, IndexError, IndexHealth,
    IndexOpenReport, IndexReport, LogicalDump, ensure_index, index_health, index_path,
    logical_dump, note_paths, rebuild,
};
pub use query::{
    BrokenLink, Heading, LinkHit, NoteSummary, SearchHit, SearchQuery, WORKSPACE_GRAPH_FORMAT,
    WorkspaceGraph, WorkspaceGraphAmbiguousLink, WorkspaceGraphBrokenLink, WorkspaceGraphEdge,
    WorkspaceGraphNode,
};
