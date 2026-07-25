use crdt_spike_contract::{PROTOCOL, fixture_paths, load_fixture};

#[test]
fn mandatory_fixtures_are_complete_and_valid() {
    let fixtures = fixture_paths()
        .into_iter()
        .map(|path| load_fixture(&path).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(fixtures.len(), 10);
    assert!(fixtures.iter().all(|fixture| fixture.mandatory));
    assert!(
        fixtures
            .iter()
            .all(|fixture| fixture.request.protocol == PROTOCOL)
    );
    assert!(
        fixtures
            .iter()
            .any(|fixture| fixture.minimum_operation_count == 100_000)
    );
}

#[test]
fn strict_schema_rejects_unknown_fields() {
    let json = r#"{"mandatory":true,"unexpected":1,"request":{"protocol":"secondbrain-crdt-spike-v1","scenario":"x","seed":1,"commands":[]},"expected_equal":[],"expected":{},"require_no_errors":true,"require_no_unsupported":true,"minimum_operation_count":0}"#;
    assert!(serde_json::from_str::<crdt_spike_contract::Fixture>(json).is_err());
}
