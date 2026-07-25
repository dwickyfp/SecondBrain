#![cfg(unix)]

use std::{fs, path::Path};

use crdt_spike_contract::{ContractError, Request, ScenarioCommand, invoke};
use tempfile::tempdir;

#[test]
fn process_harness_rejects_incomplete_candidate_response() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempdir().unwrap();
    let fake = directory.path().join("fake");
    fs::write(&fake, "#!/bin/sh\nprintf '{\"protocol\":\"secondbrain-crdt-spike-v1\",\"candidate\":\"fake\",\"candidate_version\":\"0\",\"scenario\":\"self-test\",\"status\":\"passed\",\"commands_executed\":0,\"observations\":{},\"state_hashes\":{},\"metrics\":{\"operation_count\":0,\"update_bytes\":0,\"snapshot_bytes\":0,\"materialized_bytes\":0,\"native_features\":{}},\"unsupported\":[],\"errors\":[]}'\n").unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let request = Request {
        protocol: "secondbrain-crdt-spike-v1".into(),
        scenario: "self-test".into(),
        seed: 1,
        commands: vec![ScenarioCommand::CreateReplica {
            replica: "a".into(),
        }],
    };
    assert!(matches!(
        invoke(Path::new(&fake), &request),
        Err(ContractError::Violation(_))
    ));
}
