use crdt_spike_contract::{Operation, PROTOCOL, Request, ScenarioCommand, WorkloadKind};

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() != 6 {
        eprintln!("usage: crdt-comparison-bench <name> <workload> <operations> <replicas> <seed>");
        std::process::exit(2);
    }
    let name = &args[1];
    let workload = &args[2];
    let operations: u64 = args[3].parse().expect("operations");
    let replicas: usize = args[4].parse().expect("replicas");
    let seed: u64 = args[5].parse().expect("seed");
    let mut commands = Vec::new();
    for replica in 0..replicas {
        commands.push(ScenarioCommand::CreateReplica {
            replica: format!("r{replica}"),
        });
    }
    match workload.as_str() {
        "text" | "list_move" | "properties" | "mixed" => {
            let kind = match workload.as_str() {
                "text" => WorkloadKind::Text,
                "list_move" => WorkloadKind::ListMove,
                "properties" => WorkloadKind::Properties,
                _ => WorkloadKind::Mixed,
            };
            commands.push(ScenarioCommand::ApplyWorkload {
                replica: "r0".into(),
                actor: "actor-0".into(),
                kind,
                count: operations,
                seed,
            });
        }
        "offline_merge" => {
            let each = operations / replicas as u64;
            for replica in 0..replicas {
                commands.push(ScenarioCommand::ApplyWorkload {
                    replica: format!("r{replica}"),
                    actor: format!("actor-{replica}"),
                    kind: WorkloadKind::Text,
                    count: each,
                    seed: seed + replica as u64,
                });
                commands.push(ScenarioCommand::ExportUpdates {
                    replica: format!("r{replica}"),
                    update: format!("u{replica}"),
                });
            }
            for source in (1..replicas).rev() {
                commands.push(ScenarioCommand::ImportUpdates {
                    replica: "r0".into(),
                    update: format!("u{source}"),
                    repeat: 2,
                    truncate_bytes: 0,
                });
            }
        }
        "snapshot_restore" | "compacted_restore" => {
            commands.push(ScenarioCommand::ApplyWorkload {
                replica: "r0".into(),
                actor: "actor-0".into(),
                kind: WorkloadKind::Mixed,
                count: operations,
                seed,
            });
            if workload == "compacted_restore" {
                commands.push(ScenarioCommand::CompactedSnapshot {
                    replica: "r0".into(),
                    snapshot: "snapshot".into(),
                });
            } else {
                commands.push(ScenarioCommand::Snapshot {
                    replica: "r0".into(),
                    snapshot: "snapshot".into(),
                });
            }
            commands.push(ScenarioCommand::Restore {
                replica: "restored".into(),
                snapshot: "snapshot".into(),
            });
            if workload == "compacted_restore" {
                commands.push(ScenarioCommand::ApplyLocal {
                    replica: "restored".into(),
                    actor: "actor-restored".into(),
                    operation: Operation::PropertySet {
                        key: "restore_kind".into(),
                        value: "compacted_requested".into(),
                    },
                });
            }
            commands.push(ScenarioCommand::Materialize {
                replica: "restored".into(),
                observation: "restored".into(),
            });
        }
        "incremental_update" => {
            let first = operations / 2;
            commands.push(ScenarioCommand::ApplyWorkload {
                replica: "r0".into(),
                actor: "actor-0".into(),
                kind: WorkloadKind::Text,
                count: first,
                seed,
            });
            commands.push(ScenarioCommand::ExportUpdates {
                replica: "r0".into(),
                update: "base".into(),
            });
            commands.push(ScenarioCommand::ImportUpdates {
                replica: "r1".into(),
                update: "base".into(),
                repeat: 1,
                truncate_bytes: 0,
            });
            commands.push(ScenarioCommand::ApplyWorkload {
                replica: "r0".into(),
                actor: "actor-0".into(),
                kind: WorkloadKind::Text,
                count: operations - first,
                seed: seed + 1,
            });
            commands.push(ScenarioCommand::ExportIncremental {
                replica: "r0".into(),
                since_update: "base".into(),
                update: "incremental".into(),
            });
            commands.push(ScenarioCommand::ImportUpdates {
                replica: "r1".into(),
                update: "incremental".into(),
                repeat: 1,
                truncate_bytes: 0,
            });
        }
        _ => panic!("unknown workload"),
    }
    commands.push(ScenarioCommand::Materialize {
        replica: "r0".into(),
        observation: "final".into(),
    });
    commands.push(ScenarioCommand::Metrics {
        replica: "r0".into(),
    });
    let request = Request {
        protocol: PROTOCOL.into(),
        scenario: name.clone(),
        seed,
        commands,
    };
    serde_json::to_writer(std::io::stdout(), &request).expect("write request");
}
