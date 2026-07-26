use std::fs;
use std::path::Path;

use secondbrain_index::{
    Error, IndexConfig, IndexDatabase, QueryValidationError, SearchQuery, rebuild,
};
use tempfile::tempdir;

const ALPHA_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const BETA_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAW";
const GAMMA_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAX";

fn note(id: &str, title: &str, body: &str) -> String {
    format!("---\nid: {id}\ntitle: {title}\n---\n# {title}\n\n{body}\n")
}

fn build(root: &Path, notes: &[(&str, &str)]) -> IndexDatabase {
    for (path, source) in notes {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    rebuild(root, &IndexConfig::default()).unwrap();
    IndexDatabase::open(root.join(".secondbrain/index.sqlite")).unwrap()
}

#[test]
fn term_search_returns_typed_hit() {
    let dir = tempdir().unwrap();
    let database = build(
        dir.path(),
        &[(
            "alpha.md",
            &note(ALPHA_ID, "Alpha", "A telescope sees nebulae."),
        )],
    );

    let hits = database.search(&SearchQuery::new("telescope")).unwrap();

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].note_id.to_string(), ALPHA_ID);
    assert_eq!(hits[0].path, "alpha.md");
    assert_eq!(hits[0].title.as_deref(), Some("Alpha"));
    assert!(hits[0].snippet.contains("telescope"));
}

#[test]
fn phrase_and_unicode_search_match_exact_text() {
    let dir = tempdir().unwrap();
    let database = build(
        dir.path(),
        &[
            (
                "alpha.md",
                &note(ALPHA_ID, "Alpha", "red giant star and café résumé"),
            ),
            ("beta.md", &note(BETA_ID, "Beta", "red bright giant star")),
        ],
    );

    let phrase = database.search(&SearchQuery::new("\"red giant\"")).unwrap();
    let unicode = database.search(&SearchQuery::new("café résumé")).unwrap();

    assert_eq!(
        phrase
            .iter()
            .map(|hit| hit.path.as_str())
            .collect::<Vec<_>>(),
        ["alpha.md"]
    );
    assert_eq!(
        unicode
            .iter()
            .map(|hit| hit.path.as_str())
            .collect::<Vec<_>>(),
        ["alpha.md"]
    );
}

#[test]
fn path_and_tag_filters_narrow_results() {
    let dir = tempdir().unwrap();
    let database = build(
        dir.path(),
        &[
            (
                "notes/alpha.md",
                &note(ALPHA_ID, "Alpha", "shared term #rust"),
            ),
            ("beta.md", &note(BETA_ID, "Beta", "shared term #other")),
        ],
    );

    let by_path = database
        .search(&SearchQuery::new("shared").with_path("notes/alpha.md"))
        .unwrap();
    let by_tag = database
        .search(&SearchQuery::new("shared").with_tag("rust"))
        .unwrap();

    assert_eq!(
        by_path
            .iter()
            .map(|hit| hit.path.as_str())
            .collect::<Vec<_>>(),
        ["notes/alpha.md"]
    );
    assert_eq!(
        by_tag
            .iter()
            .map(|hit| hit.path.as_str())
            .collect::<Vec<_>>(),
        ["notes/alpha.md"]
    );
}

#[test]
fn backlinks_and_outgoing_links_are_typed_and_ordered() {
    let dir = tempdir().unwrap();
    let database = build(
        dir.path(),
        &[
            ("z.md", &note(ALPHA_ID, "Zed", "See [[Target]].")),
            ("a.md", &note(BETA_ID, "Target", "Destination.")),
            ("b.md", &note(GAMMA_ID, "Bee", "Also [[Target]].")),
        ],
    );
    let target = BETA_ID.parse().unwrap();
    let source = ALPHA_ID.parse().unwrap();

    let backlinks = database.backlinks(target).unwrap();
    let outgoing = database.outgoing_links(source).unwrap();

    assert_eq!(
        backlinks
            .iter()
            .map(|hit| hit.path.as_deref())
            .collect::<Vec<_>>(),
        [Some("b.md"), Some("z.md")]
    );
    assert_eq!(
        outgoing
            .iter()
            .map(|hit| hit.path.as_deref())
            .collect::<Vec<_>>(),
        [Some("a.md")]
    );
    assert_eq!(outgoing[0].note_id, Some(target));
}

#[test]
fn headings_from_real_fixture_are_typed_and_in_source_order() {
    let dir = tempdir().unwrap();
    let source = include_str!("../../../fixtures/markdown/extract/headings.md");
    let database = build(dir.path(), &[("headings.md", source)]);
    let note = database.note_by_path("headings.md").unwrap().unwrap();

    let headings = database.headings(note.note_id).unwrap();

    assert_eq!(
        headings
            .iter()
            .map(|heading| (heading.level, heading.text.as_str(), heading.line))
            .collect::<Vec<_>>(),
        [
            (1, "Headings", 1),
            (2, "First Section", 3),
            (3, "Subsection", 7),
            (2, "Second Section", 11),
        ]
    );
}

#[test]
fn headings_are_empty_for_notes_without_an_outline_and_missing_notes() {
    let dir = tempdir().unwrap();
    let database = build(dir.path(), &[("empty.md", "Plain text only.\n")]);
    let note = database.note_by_path("empty.md").unwrap().unwrap();

    assert!(database.headings(note.note_id).unwrap().is_empty());
    assert!(
        database
            .headings("01ARZ3NDEKTSV4RRFFQ69G5FAY".parse().unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn outgoing_links_order_resolved_by_path_then_unresolved_by_target() {
    let dir = tempdir().unwrap();
    let database = build(
        dir.path(),
        &[
            (
                "source.md",
                &note(
                    ALPHA_ID,
                    "Source",
                    "[[Missing Z]] [[A Target]] [[Missing A]] [[Z Target]]",
                ),
            ),
            ("z.md", &note(BETA_ID, "A Target", "Destination.")),
            ("a.md", &note(GAMMA_ID, "Z Target", "Destination.")),
        ],
    );

    let outgoing = database.outgoing_links(ALPHA_ID.parse().unwrap()).unwrap();

    assert_eq!(
        outgoing
            .iter()
            .map(|hit| (hit.target.as_str(), hit.note_id, hit.path.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("Z Target", Some(GAMMA_ID.parse().unwrap()), Some("a.md")),
            ("A Target", Some(BETA_ID.parse().unwrap()), Some("z.md")),
            ("Missing A", None, None),
            ("Missing Z", None, None),
        ]
    );
}

#[test]
fn broken_links_and_orphans_are_reported_in_stable_order() {
    let dir = tempdir().unwrap();
    let database = build(
        dir.path(),
        &[
            ("z.md", &note(ALPHA_ID, "Zed", "[[Missing Z]]")),
            ("a.md", &note(BETA_ID, "Alpha", "No links.")),
            ("b.md", &note(GAMMA_ID, "Bee", "[[Missing A]]")),
        ],
    );

    let broken = database.broken_links().unwrap();
    let orphans = database.orphans().unwrap();

    assert_eq!(
        broken
            .iter()
            .map(|link| link.source_path.as_str())
            .collect::<Vec<_>>(),
        ["b.md", "z.md"]
    );
    assert_eq!(
        orphans
            .iter()
            .map(|note| note.path.as_str())
            .collect::<Vec<_>>(),
        ["a.md", "b.md", "z.md"]
    );
}

#[test]
fn snippets_preserve_printable_unicode_and_escape_terminal_controls() {
    let dir = tempdir().unwrap();
    let database = build(
        dir.path(),
        &[(
            "alpha.md",
            &note(
                ALPHA_ID,
                "Alpha",
                "findme café \u{1b}[31m red\nnext\tcolumn",
            ),
        )],
    );

    let hits = database.search(&SearchQuery::new("findme")).unwrap();

    assert!(hits[0].snippet.contains("café"));
    assert!(!hits[0].snippet.contains('\u{1b}'));
    assert!(!hits[0].snippet.contains('\n'));
    assert!(hits[0].snippet.contains("\\u{1b}") || hits[0].snippet.contains("\\x1b"));
}

#[test]
fn equal_rank_searches_tie_break_by_path_then_note_id() {
    let dir = tempdir().unwrap();
    let database = build(
        dir.path(),
        &[
            ("z.md", &note(ALPHA_ID, "Same", "equal token")),
            ("a.md", &note(BETA_ID, "Same", "equal token")),
        ],
    );

    let hits = database.search(&SearchQuery::new("equal")).unwrap();

    assert_eq!(
        hits.iter().map(|hit| hit.path.as_str()).collect::<Vec<_>>(),
        ["a.md", "z.md"]
    );
}

#[test]
fn invalid_search_text_returns_typed_validation_errors() {
    let dir = tempdir().unwrap();
    let database = build(dir.path(), &[]);
    for (text, expected) in [
        ("nul\0byte", QueryValidationError::DisallowedControl),
        ("\"unterminated", QueryValidationError::UnmatchedQuote),
        ("\u{1b}\u{7}", QueryValidationError::DisallowedControl),
    ] {
        let error = database.search(&SearchQuery::new(text)).unwrap_err();
        assert!(matches!(error, Error::InvalidQuery(actual) if actual == expected));
    }
}

#[test]
fn corrupt_stored_note_id_has_contextual_error() {
    let dir = tempdir().unwrap();
    let database = build(
        dir.path(),
        &[("alpha.md", &note(ALPHA_ID, "Alpha", "needle"))],
    );
    database
        .connection()
        .execute("PRAGMA foreign_keys=OFF", [])
        .unwrap();
    database
        .connection()
        .execute("UPDATE notes SET note_id='bad-id'", [])
        .unwrap();
    database
        .connection()
        .execute("UPDATE paths SET note_id='bad-id'", [])
        .unwrap();
    database
        .connection()
        .execute("UPDATE notes_fts SET note_id='bad-id'", [])
        .unwrap();
    let error = database.search(&SearchQuery::new("needle")).unwrap_err();
    assert!(matches!(error, Error::InvalidStoredNoteId { value } if value == "bad-id"));
}

#[test]
fn graph_is_deterministic_aggregated_and_reports_every_resolution_state() {
    let dir = tempdir().unwrap();
    let database = build(
        dir.path(),
        &[
            (
                "z/source.md",
                &note(
                    ALPHA_ID,
                    "Source",
                    "[[Target]] [[Target]] [[Source]] [[Missing]] [[Shared]] [[Shared]]",
                ),
            ),
            ("a/target.md", &note(BETA_ID, "Target", "Connected.")),
            (
                "b/shared.md",
                &format!("---\nid: {GAMMA_ID}\ntitle: One\naliases: [Shared]\n---\nOne.\n"),
            ),
            (
                "c/shared.md",
                "---\nid: 01ARZ3NDEKTSV4RRFFQ69G5FAY\ntitle: Two\naliases: [Shared]\n---\nTwo.\n",
            ),
            (
                "orphan.md",
                "---\nid: 01ARZ3NDEKTSV4RRFFQ69G5FAZ\ntitle: Orphan\n---\nAlone.\n",
            ),
        ],
    );

    let first = database.workspace_graph().unwrap();
    let second = database.workspace_graph().unwrap();

    assert_eq!(first, second);
    assert_eq!(first.format, "sb-workspace-graph-v1");
    assert_eq!(
        first
            .nodes
            .iter()
            .map(|node| node.path.as_str())
            .collect::<Vec<_>>(),
        [
            "a/target.md",
            "b/shared.md",
            "c/shared.md",
            "orphan.md",
            "z/source.md"
        ]
    );
    assert_eq!(first.edges.len(), 2);
    assert_eq!(
        first
            .edges
            .iter()
            .map(|edge| (edge.occurrences, edge.self_link))
            .collect::<Vec<_>>(),
        [(1, true), (2, false)]
    );
    assert_eq!(first.broken_links[0].target, "Missing");
    assert_eq!(first.broken_links[0].occurrences, 1);
    assert_eq!(first.ambiguous_links[0].target, "Shared");
    assert_eq!(first.ambiguous_links[0].occurrences, 2);
    assert_eq!(
        first.ambiguous_links[0]
            .candidates
            .iter()
            .map(|note| note.path.as_str())
            .collect::<Vec<_>>(),
        ["b/shared.md", "c/shared.md"]
    );
    assert!(
        first
            .nodes
            .iter()
            .find(|node| node.path == "orphan.md")
            .unwrap()
            .orphan
    );
    assert_eq!(
        database.broken_links().unwrap().len(),
        1,
        "ambiguity is not mislabeled as broken"
    );
    assert!(
        database
            .outgoing_links(ALPHA_ID.parse().unwrap())
            .unwrap()
            .iter()
            .filter(|link| link.target == "Shared")
            .all(|link| link.note_id.is_none())
    );
}
