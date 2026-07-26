use std::str::FromStr;

use rusqlite::params;
use secondbrain_core::id::NoteId;
use serde::Serialize;

use crate::{Error, IndexDatabase, QueryValidationError, Result};

/// A full-text query with optional exact path and tag filters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchQuery {
    pub text: String,
    pub path: Option<String>,
    pub tag: Option<String>,
}

impl SearchQuery {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            path: None,
            tag: None,
        }
    }

    #[must_use]
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchHit {
    pub note_id: NoteId,
    pub path: String,
    pub title: Option<String>,
    pub snippet: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkHit {
    pub note_id: Option<NoteId>,
    pub path: Option<String>,
    pub title: Option<String>,
    pub target: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrokenLink {
    pub source_note_id: NoteId,
    pub source_path: String,
    pub target: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NoteSummary {
    pub note_id: NoteId,
    pub path: String,
    pub title: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub line: i64,
}

pub const WORKSPACE_GRAPH_FORMAT: &str = "sb-workspace-graph-v1";

/// Versioned, read-only graph derived exclusively from the workspace index.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkspaceGraph {
    pub format: &'static str,
    pub nodes: Vec<WorkspaceGraphNode>,
    pub edges: Vec<WorkspaceGraphEdge>,
    pub broken_links: Vec<WorkspaceGraphBrokenLink>,
    pub ambiguous_links: Vec<WorkspaceGraphAmbiguousLink>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkspaceGraphNode {
    pub note_id: NoteId,
    pub path: String,
    pub title: Option<String>,
    pub incoming_occurrences: u64,
    pub outgoing_occurrences: u64,
    pub orphan: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkspaceGraphEdge {
    pub source_note_id: NoteId,
    pub target_note_id: NoteId,
    pub occurrences: u64,
    pub self_link: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkspaceGraphBrokenLink {
    pub source_note_id: NoteId,
    pub source_path: String,
    pub target: String,
    pub occurrences: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkspaceGraphAmbiguousLink {
    pub source_note_id: NoteId,
    pub source_path: String,
    pub target: String,
    pub occurrences: u64,
    pub candidates: Vec<NoteSummary>,
}

impl IndexDatabase {
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>> {
        let fts = build_fts_query(&query.text)?;
        if fts.is_empty() {
            return Ok(Vec::new());
        }
        let mut statement = self.connection().prepare(
            "SELECT n.note_id,p.path,n.title,
                    snippet(notes_fts,2,'[',']',' … ',16)
             FROM notes_fts
             JOIN notes n ON n.note_id=notes_fts.note_id
             JOIN paths p ON p.note_id=n.note_id AND p.is_current=1
             WHERE notes_fts MATCH ?1
               AND (?2 IS NULL OR p.path=?2)
               AND (?3 IS NULL OR EXISTS (
                    SELECT 1 FROM tags t WHERE t.note_id=n.note_id AND t.tag=?3))
             ORDER BY bm25(notes_fts),p.path,n.note_id",
        )?;
        let rows = statement.query_map(params![fts, query.path, query.tag], |row| {
            let id: String = row.get(0)?;
            Ok((
                id,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (id, path, title, snippet) = row?;
            Ok(SearchHit {
                note_id: parse_id(&id)?,
                path,
                title,
                snippet: terminal_safe(&snippet),
            })
        })
        .collect()
    }

    /// The note currently living at `path`, or `None` when the index holds no
    /// such note.
    ///
    /// This is how a caller turns the path an operator typed into the note id
    /// every other query takes. It deliberately answers from the index rather
    /// than from the file's frontmatter: the index is where the rule about
    /// which identity a file carries — declared id first, identity map
    /// otherwise — was already applied, and asking the file again would restate
    /// that rule somewhere it could drift.
    pub fn note_by_path(&self, path: &str) -> Result<Option<NoteSummary>> {
        let mut statement = self.connection().prepare(
            "SELECT n.note_id,p.path,n.title FROM notes n
             JOIN paths p ON p.note_id=n.note_id AND p.is_current=1
             WHERE p.path=?1",
        )?;
        let mut rows = statement.query([path])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let id: String = row.get(0)?;
        Ok(Some(NoteSummary {
            note_id: parse_id(&id)?,
            path: row.get(1)?,
            title: row.get(2)?,
        }))
    }

    pub fn note_by_id(&self, note: NoteId) -> Result<Option<NoteSummary>> {
        let mut statement = self.connection().prepare(
            "SELECT n.note_id,p.path,n.title FROM notes n
             JOIN paths p ON p.note_id=n.note_id AND p.is_current=1
             WHERE n.note_id=?1",
        )?;
        let mut rows = statement.query([note.to_string()])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let id: String = row.get(0)?;
        Ok(Some(NoteSummary {
            note_id: parse_id(&id)?,
            path: row.get(1)?,
            title: row.get(2)?,
        }))
    }

    /// Headings for `note` in source order.
    pub fn headings(&self, note: NoteId) -> Result<Vec<Heading>> {
        let mut statement = self.connection().prepare(
            "SELECT level,text,line FROM headings
             WHERE note_id=?1 ORDER BY line,id",
        )?;
        let rows = statement.query_map([note.to_string()], |row| {
            Ok(Heading {
                level: row.get(0)?,
                text: row.get(1)?,
                line: row.get(2)?,
            })
        })?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(Into::into)
    }

    pub fn backlinks(&self, note: NoteId) -> Result<Vec<LinkHit>> {
        self.links_query(
            "SELECT n.note_id,p.path,n.title,l.target FROM links l
             JOIN notes n ON n.note_id=l.note_id
             JOIN paths p ON p.note_id=n.note_id AND p.is_current=1
             WHERE l.label=?1 ORDER BY p.path,n.note_id,l.target",
            note,
        )
    }

    pub fn outgoing_links(&self, note: NoteId) -> Result<Vec<LinkHit>> {
        self.links_query(
            "SELECT n.note_id,p.path,n.title,l.target FROM links l
             LEFT JOIN notes n ON n.note_id=l.label
             LEFT JOIN paths p ON p.note_id=n.note_id AND p.is_current=1
             WHERE l.note_id=?1
             ORDER BY n.note_id IS NULL,p.path,n.note_id,l.target",
            note,
        )
    }

    fn links_query(&self, sql: &str, note: NoteId) -> Result<Vec<LinkHit>> {
        let mut statement = self.connection().prepare(sql)?;
        let rows = statement.query_map([note.to_string()], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (id, path, title, target) = row?;
            Ok(LinkHit {
                note_id: id.as_deref().map(parse_id).transpose()?,
                path,
                title,
                target,
            })
        })
        .collect()
    }

    pub fn broken_links(&self) -> Result<Vec<BrokenLink>> {
        let mut statement = self.connection().prepare(
            "SELECT l.note_id,p.path,l.target FROM links l
             JOIN paths p ON p.note_id=l.note_id AND p.is_current=1
             WHERE l.label IS NULL
               AND NOT EXISTS (SELECT 1 FROM link_candidates c WHERE c.link_id=l.id)
             ORDER BY p.path,l.note_id,l.target",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (id, source_path, target) = row?;
            Ok(BrokenLink {
                source_note_id: parse_id(&id)?,
                source_path,
                target,
            })
        })
        .collect()
    }

    pub fn orphans(&self) -> Result<Vec<NoteSummary>> {
        let mut statement = self.connection().prepare(
            "SELECT n.note_id,p.path,n.title FROM notes n
             JOIN paths p ON p.note_id=n.note_id AND p.is_current=1
             WHERE NOT EXISTS (SELECT 1 FROM links l WHERE l.note_id=n.note_id AND l.label IS NOT NULL)
               AND NOT EXISTS (SELECT 1 FROM links l WHERE l.label=n.note_id)
             ORDER BY p.path,n.note_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (id, path, title) = row?;
            Ok(NoteSummary {
                note_id: parse_id(&id)?,
                path,
                title,
            })
        })
        .collect()
    }

    /// Returns one deterministic graph snapshot using indexed data only.
    pub fn workspace_graph(&self) -> Result<WorkspaceGraph> {
        let mut node_statement = self.connection().prepare(
            "SELECT n.note_id,p.path,n.title,
                    (SELECT count(*) FROM links l WHERE l.label=n.note_id),
                    (SELECT count(*) FROM links l WHERE l.note_id=n.note_id AND l.label IS NOT NULL)
             FROM notes n JOIN paths p ON p.note_id=n.note_id AND p.is_current=1
             ORDER BY p.path,n.note_id",
        )?;
        let nodes = node_statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get::<_, i64>(3)? as u64,
                    row.get::<_, i64>(4)? as u64,
                ))
            })?
            .map(|row| {
                let (id, path, title, incoming, outgoing) = row?;
                Ok(WorkspaceGraphNode {
                    note_id: parse_id(&id)?,
                    path,
                    title,
                    incoming_occurrences: incoming,
                    outgoing_occurrences: outgoing,
                    orphan: incoming == 0 && outgoing == 0,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let mut edge_statement = self.connection().prepare(
            "SELECT note_id,label,count(*) FROM links WHERE label IS NOT NULL
             GROUP BY note_id,label ORDER BY note_id,label",
        )?;
        let edges = edge_statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as u64,
                ))
            })?
            .map(|row| {
                let (source, target, occurrences) = row?;
                let source_note_id = parse_id(&source)?;
                let target_note_id = parse_id(&target)?;
                Ok(WorkspaceGraphEdge {
                    self_link: source_note_id == target_note_id,
                    source_note_id,
                    target_note_id,
                    occurrences,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let mut broken_statement = self.connection().prepare(
            "SELECT l.note_id,p.path,l.target,count(*) FROM links l
             JOIN paths p ON p.note_id=l.note_id AND p.is_current=1
             WHERE l.label IS NULL AND NOT EXISTS (SELECT 1 FROM link_candidates c WHERE c.link_id=l.id)
             GROUP BY l.note_id,p.path,l.target ORDER BY p.path,l.note_id,l.target",
        )?;
        let broken_links = broken_statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get::<_, i64>(3)? as u64,
                ))
            })?
            .map(|row| {
                let (id, source_path, target, occurrences) = row?;
                Ok(WorkspaceGraphBrokenLink {
                    source_note_id: parse_id(&id)?,
                    source_path,
                    target,
                    occurrences,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let mut ambiguous_statement = self.connection().prepare(
            "SELECT l.note_id,p.path,l.target,count(DISTINCT l.id) FROM links l
             JOIN paths p ON p.note_id=l.note_id AND p.is_current=1
             WHERE l.label IS NULL AND EXISTS (SELECT 1 FROM link_candidates c WHERE c.link_id=l.id)
             GROUP BY l.note_id,p.path,l.target ORDER BY p.path,l.note_id,l.target",
        )?;
        let ambiguous_rows = ambiguous_statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)? as u64,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut candidate_statement = self.connection().prepare(
            "SELECT DISTINCT n.note_id,p.path,n.title FROM links l
             JOIN link_candidates c ON c.link_id=l.id JOIN notes n ON n.note_id=c.note_id
             JOIN paths p ON p.note_id=n.note_id AND p.is_current=1
             WHERE l.note_id=?1 AND l.target=?2 ORDER BY p.path,n.note_id",
        )?;
        let mut ambiguous_links = Vec::with_capacity(ambiguous_rows.len());
        for (id, source_path, target, occurrences) in ambiguous_rows {
            let candidates = candidate_statement
                .query_map(params![id, target], |row| {
                    Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?))
                })?
                .map(|row| {
                    let (id, path, title) = row?;
                    Ok(NoteSummary {
                        note_id: parse_id(&id)?,
                        path,
                        title,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            ambiguous_links.push(WorkspaceGraphAmbiguousLink {
                source_note_id: parse_id(&id)?,
                source_path,
                target,
                occurrences,
                candidates,
            });
        }
        Ok(WorkspaceGraph {
            format: WORKSPACE_GRAPH_FORMAT,
            nodes,
            edges,
            broken_links,
            ambiguous_links,
        })
    }
}

fn parse_id(value: &str) -> Result<NoteId> {
    NoteId::from_str(value).map_err(|_| Error::InvalidStoredNoteId {
        value: value.to_owned(),
    })
}

fn build_fts_query(input: &str) -> Result<String> {
    if input.chars().any(|c| c.is_control() && !c.is_whitespace()) {
        return Err(Error::InvalidQuery(QueryValidationError::DisallowedControl));
    }
    let mut terms = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for character in input.chars() {
        match character {
            '"' => {
                if quoted && !current.trim().is_empty() {
                    terms.push(current.trim().to_owned());
                    current.clear();
                }
                quoted = !quoted;
            }
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    terms.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if quoted {
        return Err(Error::InvalidQuery(QueryValidationError::UnmatchedQuote));
    }
    if !current.trim().is_empty() {
        terms.push(current.trim().to_owned());
    }
    Ok(terms
        .into_iter()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND "))
}

fn terminal_safe(value: &str) -> String {
    value.chars().fold(String::new(), |mut safe, character| {
        if character.is_control() {
            safe.extend(character.escape_default());
        } else {
            safe.push(character);
        }
        safe
    })
}
