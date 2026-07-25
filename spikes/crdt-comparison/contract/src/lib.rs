#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    time::Instant,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROTOCOL: &str = "secondbrain-crdt-spike-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub protocol: String,
    pub scenario: String,
    pub seed: u64,
    pub commands: Vec<ScenarioCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScenarioCommand {
    CreateReplica {
        replica: String,
    },
    ApplyLocal {
        replica: String,
        actor: String,
        operation: Operation,
    },
    ApplyWorkload {
        replica: String,
        actor: String,
        kind: WorkloadKind,
        count: u64,
        seed: u64,
    },
    ProbeRelativePosition {
        replica: String,
        index: u32,
        insert_at: u32,
        text: String,
        expected_index: u32,
    },
    ExportUpdates {
        replica: String,
        update: String,
    },
    ExportIncremental {
        replica: String,
        since_update: String,
        update: String,
    },
    ImportUpdates {
        replica: String,
        update: String,
        repeat: u32,
        truncate_bytes: usize,
    },
    Materialize {
        replica: String,
        observation: String,
    },
    UndoActor {
        replica: String,
        actor: String,
    },
    Snapshot {
        replica: String,
        snapshot: String,
    },
    CompactedSnapshot {
        replica: String,
        snapshot: String,
    },
    Restore {
        replica: String,
        snapshot: String,
    },
    TruncateRestore {
        replica: String,
        snapshot: String,
        truncate_bytes: usize,
    },
    Metrics {
        replica: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadKind {
    Text,
    ListMove,
    Properties,
    Mixed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    TextInsert {
        index: u32,
        text: String,
    },
    TextDelete {
        index: u32,
        len: u32,
    },
    TextMark {
        index: u32,
        len: u32,
        key: String,
        value: String,
    },
    BlockInsert {
        index: u32,
        id: String,
        text: String,
    },
    BlockMove {
        from: u32,
        to: u32,
    },
    PropertySet {
        key: String,
        value: String,
    },
    ExternalReplace {
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Materialized {
    pub text: String,
    pub blocks: Vec<Block>,
    pub properties: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Block {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub protocol: String,
    pub candidate: String,
    pub candidate_version: String,
    pub scenario: String,
    pub status: Status,
    pub commands_executed: usize,
    pub observations: BTreeMap<String, Materialized>,
    pub state_hashes: BTreeMap<String, String>,
    pub metrics: Metrics,
    pub unsupported: Vec<Unsupported>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Passed,
    Failed,
    Unsupported,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metrics {
    pub operation_count: u64,
    pub update_bytes: u64,
    pub snapshot_bytes: u64,
    pub materialized_bytes: u64,
    pub native_features: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unsupported {
    pub command_index: usize,
    pub feature: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub mandatory: bool,
    pub request: Request,
    pub expected_equal: Vec<Vec<String>>,
    pub expected: BTreeMap<String, Materialized>,
    pub require_no_errors: bool,
    pub require_no_unsupported: bool,
    #[serde(default)]
    pub minimum_operation_count: u64,
}

#[derive(Debug, Serialize)]
pub struct Invocation {
    pub response: Response,
    pub wall_time_ns: u128,
    pub stdout_bytes: usize,
}

pub fn state_hash(value: &Materialized) -> String {
    blake3::hash(&serde_json::to_vec(value).expect("materialized state is serializable"))
        .to_hex()
        .to_string()
}

#[derive(Debug, Error)]
pub enum ContractError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("candidate exited {0}: {1}")]
    Exit(String, String),
    #[error("contract violation: {0}")]
    Violation(String),
}

pub fn load_fixture(path: &Path) -> Result<Fixture, ContractError> {
    let fixture: Fixture = serde_json::from_slice(&std::fs::read(path)?)?;
    if fixture.request.protocol != PROTOCOL {
        return Err(ContractError::Violation("wrong request protocol".into()));
    }
    if fixture.request.commands.is_empty() {
        return Err(ContractError::Violation("empty command stream".into()));
    }
    Ok(fixture)
}

pub fn invoke(binary: &Path, request: &Request) -> Result<Invocation, ContractError> {
    let started = Instant::now();
    let mut child = Command::new(binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(&serde_json::to_vec(request)?)?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(ContractError::Exit(
            output.status.to_string(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    let response: Response = serde_json::from_slice(&output.stdout)?;
    if response.protocol != PROTOCOL || response.scenario != request.scenario {
        return Err(ContractError::Violation(
            "response identity does not match request".into(),
        ));
    }
    if response.commands_executed != request.commands.len() {
        return Err(ContractError::Violation(format!(
            "executed {} of {} commands",
            response.commands_executed,
            request.commands.len()
        )));
    }
    Ok(Invocation {
        response,
        wall_time_ns: started.elapsed().as_nanos(),
        stdout_bytes: output.stdout.len(),
    })
}

pub fn assert_fixture(fixture: &Fixture, response: &Response) -> Result<(), ContractError> {
    if fixture.require_no_errors && !response.errors.is_empty() {
        return Err(ContractError::Violation(format!(
            "candidate errors: {:?}",
            response.errors
        )));
    }
    if fixture.require_no_unsupported && !response.unsupported.is_empty() {
        return Err(ContractError::Violation(format!(
            "unsupported: {:?}",
            response
                .unsupported
                .iter()
                .map(|u| &u.feature)
                .collect::<Vec<_>>()
        )));
    }
    if response.metrics.operation_count < fixture.minimum_operation_count {
        return Err(ContractError::Violation(format!(
            "operation count {} is below {}",
            response.metrics.operation_count, fixture.minimum_operation_count
        )));
    }
    for group in &fixture.expected_equal {
        let Some(first) = group
            .first()
            .and_then(|name| response.observations.get(name))
        else {
            return Err(ContractError::Violation(
                "missing convergence observation".into(),
            ));
        };
        for name in &group[1..] {
            if response.observations.get(name) != Some(first) {
                return Err(ContractError::Violation(format!(
                    "observations did not converge: {group:?}"
                )));
            }
        }
    }
    for (name, expected) in &fixture.expected {
        if response.observations.get(name) != Some(expected) {
            return Err(ContractError::Violation(format!(
                "unexpected observation {name}"
            )));
        }
    }
    if response.status != Status::Passed {
        return Err(ContractError::Violation(format!(
            "candidate status is {:?}",
            response.status
        )));
    }
    Ok(())
}

pub fn fixture_paths() -> Vec<std::path::PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures");
    let mut paths = std::fs::read_dir(root)
        .expect("fixture directory")
        .map(|e| e.expect("fixture entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}
