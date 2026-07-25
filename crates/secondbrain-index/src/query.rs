use std::str::FromStr;

use rusqlite::params;
use secondbrain_core::id::NoteId;

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteSummary {
    pub note_id: NoteId,
    pub path: String,
    pub title: Option<String>,
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
             WHERE l.label IS NULL ORDER BY p.path,l.note_id,l.target",
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
