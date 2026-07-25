CREATE TABLE notes (
    note_id TEXT PRIMARY KEY NOT NULL,
    title TEXT,
    content_hash TEXT,
    modified_at TEXT
) STRICT;

CREATE TABLE paths (
    id INTEGER PRIMARY KEY,
    note_id TEXT NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    is_current INTEGER NOT NULL DEFAULT 1 CHECK (is_current IN (0, 1))
) STRICT;
CREATE UNIQUE INDEX paths_unique_current_path ON paths(path) WHERE is_current = 1;
CREATE UNIQUE INDEX paths_unique_current_note ON paths(note_id) WHERE is_current = 1;

CREATE TABLE properties (
    id INTEGER PRIMARY KEY,
    note_id TEXT NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    value TEXT NOT NULL,
    UNIQUE(note_id, name)
) STRICT;

CREATE TABLE links (
    id INTEGER PRIMARY KEY,
    note_id TEXT NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE,
    target TEXT NOT NULL,
    label TEXT
) STRICT;

CREATE TABLE tags (
    id INTEGER PRIMARY KEY,
    note_id TEXT NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    UNIQUE(note_id, tag)
) STRICT;

CREATE TABLE headings (
    id INTEGER PRIMARY KEY,
    note_id TEXT NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE,
    level INTEGER NOT NULL CHECK (level BETWEEN 1 AND 6),
    text TEXT NOT NULL,
    line INTEGER NOT NULL
) STRICT;

CREATE TABLE tasks (
    id INTEGER PRIMARY KEY,
    note_id TEXT NOT NULL REFERENCES notes(note_id) ON DELETE CASCADE,
    text TEXT NOT NULL,
    completed INTEGER NOT NULL CHECK (completed IN (0, 1)),
    line INTEGER NOT NULL
) STRICT;

CREATE TABLE index_state (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;

CREATE VIRTUAL TABLE notes_fts USING fts5(
    note_id UNINDEXED,
    title,
    body,
    tokenize = 'unicode61'
);