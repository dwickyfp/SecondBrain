//! `secondbrain search` — full-text search over the derived index.

use std::path::Path;

use secondbrain_core::id::NoteId;
use secondbrain_index::SearchQuery;
use serde::Serialize;

use crate::exit::{CliError, OK};
use crate::output::{Format, Report, emit, or_none, plural};
use crate::workspace::Workspace;

/// One matching note.
#[derive(Serialize)]
struct Hit {
    note_id: NoteId,
    path: String,
    title: Option<String>,
    snippet: String,
}

/// What a search found.
#[derive(Serialize)]
struct SearchReport {
    query: String,
    hits: Vec<Hit>,
}

impl Report for SearchReport {
    fn render(&self) -> String {
        let mut text = format!(
            "{} for {:?}",
            plural(self.hits.len(), "hit", "hits"),
            self.query
        );
        for hit in &self.hits {
            text.push_str(&format!(
                "\n  {}\n    {}  {}\n    {}",
                hit.path,
                hit.note_id,
                or_none(hit.title.as_ref()),
                hit.snippet
            ));
        }
        text
    }
}

/// Searches `workspace` for `query`.
pub fn run(format: Format, workspace: &Path, query: &str) -> Result<u8, CliError> {
    let workspace = Workspace::open(workspace)?;
    let database = workspace.open_index()?;
    let hits = database
        .search(&SearchQuery::new(query))?
        .into_iter()
        .map(|hit| Hit {
            note_id: hit.note_id,
            path: hit.path,
            title: hit.title,
            snippet: hit.snippet,
        })
        .collect();
    emit(
        format,
        &SearchReport {
            query: query.to_owned(),
            hits,
        },
    )?;
    Ok(OK)
}
