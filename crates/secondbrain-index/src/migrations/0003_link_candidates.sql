CREATE TABLE link_candidates (
    link_id INTEGER NOT NULL REFERENCES links(id) ON DELETE CASCADE,
    note_id TEXT NOT NULL,
    PRIMARY KEY(link_id, note_id)
) STRICT;
CREATE INDEX link_candidates_note_id_idx ON link_candidates(note_id);
