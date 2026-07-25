use std::collections::{BTreeMap, HashMap};

use crdt_spike_contract::{
    Block, Materialized, Metrics, Operation, PROTOCOL, Request, Response, ScenarioCommand, Status,
    Unsupported, WorkloadKind, state_hash,
};
use loro::{ExportMode, LoroDoc, ToJson, UndoManager, VersionVector};

struct Replica {
    doc: LoroDoc,
    undo: UndoManager,
    actor: Option<String>,
}

pub fn run(request: Request) -> Response {
    let mut replicas: HashMap<String, Replica> = HashMap::new();
    let mut updates: HashMap<String, Vec<u8>> = HashMap::new();
    let mut update_versions: HashMap<String, VersionVector> = HashMap::new();
    let mut snapshots: HashMap<String, Vec<u8>> = HashMap::new();
    let mut observations = BTreeMap::new();
    let mut state_hashes = BTreeMap::new();
    let mut metrics = Metrics::default();
    let mut unsupported = Vec::new();
    let mut errors = Vec::new();
    let mut executed = 0;
    metrics.native_features.extend([
        ("rich_text_marks".into(), true),
        ("relative_positions".into(), true),
        ("identity_preserving_move".into(), true),
        ("per_peer_undo".into(), true),
        ("shallow_snapshot".into(), true),
    ]);

    for (index, command) in request.commands.iter().enumerate() {
        executed += 1;
        let result: Result<(), String> = (|| {
            match command {
                ScenarioCommand::CreateReplica { replica } => {
                    let doc = LoroDoc::new();
                    doc.set_peer_id(peer_id(replica)).map_err(err)?;
                    let mut undo = UndoManager::new(&doc);
                    undo.set_max_undo_steps(1_000_000);
                    replicas.insert(
                        replica.clone(),
                        Replica {
                            doc,
                            undo,
                            actor: None,
                        },
                    );
                }
                ScenarioCommand::ApplyLocal {
                    replica,
                    actor,
                    operation,
                } => {
                    let replica = get_mut(&mut replicas, replica)?;
                    if replica
                        .actor
                        .as_ref()
                        .is_some_and(|existing| existing != actor)
                    {
                        unsupported.push(Unsupported {
                            command_index: index,
                            feature: "multiple_actors_on_one_replica".into(),
                            reason: "Loro UndoManager is bound to one peer identity".into(),
                        });
                        return Ok(());
                    }
                    replica.actor = Some(actor.clone());
                    apply(&replica.doc, operation)?;
                    replica.doc.commit();
                    replica.undo.record_new_checkpoint().map_err(err)?;
                    metrics.operation_count += 1;
                }
                ScenarioCommand::ApplyWorkload {
                    replica,
                    actor,
                    kind,
                    count,
                    seed,
                } => {
                    let replica = get_mut(&mut replicas, replica)?;
                    replica.actor = Some(actor.clone());
                    apply_workload(&replica.doc, *kind, *count, *seed)?;
                    replica.doc.commit();
                    metrics.operation_count += count;
                }
                ScenarioCommand::ProbeRelativePosition {
                    replica,
                    index,
                    insert_at,
                    text,
                    expected_index,
                } => {
                    let doc = &get(&replicas, replica)?.doc;
                    let native = doc.get_text("text");
                    let cursor = native
                        .get_cursor(*index as usize, Default::default())
                        .ok_or("could not create cursor")?;
                    native.insert(*insert_at as usize, text).map_err(err)?;
                    let actual = doc.get_cursor_pos(&cursor).map_err(err)?.current.pos;
                    if actual != *expected_index as usize {
                        return Err(format!(
                            "relative position mapped to {actual}, expected {expected_index}"
                        ));
                    }
                    metrics.operation_count += 1;
                }
                ScenarioCommand::ExportUpdates { replica, update } => {
                    let bytes = get(&replicas, replica)?
                        .doc
                        .export(ExportMode::all_updates())
                        .map_err(err)?;
                    metrics.update_bytes += bytes.len() as u64;
                    updates.insert(update.clone(), bytes);
                    update_versions.insert(update.clone(), get(&replicas, replica)?.doc.oplog_vv());
                }
                ScenarioCommand::ExportIncremental {
                    replica,
                    since_update,
                    update,
                } => {
                    let since = update_versions
                        .get(since_update)
                        .ok_or_else(|| format!("unknown baseline update {since_update}"))?;
                    let bytes = get(&replicas, replica)?
                        .doc
                        .export(ExportMode::updates(since))
                        .map_err(err)?;
                    metrics.update_bytes += bytes.len() as u64;
                    updates.insert(update.clone(), bytes);
                    update_versions.insert(update.clone(), get(&replicas, replica)?.doc.oplog_vv());
                }
                ScenarioCommand::ImportUpdates {
                    replica,
                    update,
                    repeat,
                    truncate_bytes,
                } => {
                    let source = updates
                        .get(update)
                        .ok_or_else(|| format!("unknown update {update}"))?;
                    let len = source.len().saturating_sub(*truncate_bytes);
                    for _ in 0..*repeat {
                        let result = get_mut(&mut replicas, replica)?.doc.import(&source[..len]);
                        if *truncate_bytes == 0 {
                            result.map_err(err)?;
                        } else if result.is_ok() {
                            return Err("truncated update was accepted".into());
                        }
                    }
                }
                ScenarioCommand::Materialize {
                    replica,
                    observation,
                } => {
                    let value = materialize(&get(&replicas, replica)?.doc);
                    metrics.materialized_bytes +=
                        serde_json::to_vec(&value).map_err(err)?.len() as u64;
                    state_hashes.insert(observation.clone(), state_hash(&value));
                    observations.insert(observation.clone(), value);
                }
                ScenarioCommand::UndoActor { replica, actor } => {
                    let replica = get_mut(&mut replicas, replica)?;
                    if replica.actor.as_deref() != Some(actor) {
                        return Err(format!("actor {actor} is not local peer"));
                    }
                    if !replica.undo.undo().map_err(err)? {
                        return Err("undo stack empty".into());
                    }
                }
                ScenarioCommand::Snapshot { replica, snapshot } => {
                    let bytes = get(&replicas, replica)?
                        .doc
                        .export(ExportMode::Snapshot)
                        .map_err(err)?;
                    metrics.snapshot_bytes += bytes.len() as u64;
                    snapshots.insert(snapshot.clone(), bytes);
                }
                ScenarioCommand::CompactedSnapshot { replica, snapshot } => {
                    let doc = &get(&replicas, replica)?.doc;
                    let frontiers = doc.oplog_frontiers();
                    let bytes = doc
                        .export(ExportMode::shallow_snapshot(&frontiers))
                        .map_err(err)?;
                    metrics.snapshot_bytes += bytes.len() as u64;
                    snapshots.insert(snapshot.clone(), bytes);
                }
                ScenarioCommand::Restore { replica, snapshot } => {
                    let bytes = snapshots
                        .get(snapshot)
                        .ok_or_else(|| format!("unknown snapshot {snapshot}"))?;
                    let doc = LoroDoc::from_snapshot(bytes).map_err(err)?;
                    let undo = UndoManager::new(&doc);
                    replicas.insert(
                        replica.clone(),
                        Replica {
                            doc,
                            undo,
                            actor: None,
                        },
                    );
                }
                ScenarioCommand::TruncateRestore {
                    replica: _,
                    snapshot,
                    truncate_bytes,
                } => {
                    let bytes = snapshots
                        .get(snapshot)
                        .ok_or_else(|| format!("unknown snapshot {snapshot}"))?;
                    let len = bytes.len().saturating_sub(*truncate_bytes);
                    if LoroDoc::from_snapshot(&bytes[..len]).is_ok() {
                        return Err("truncated snapshot was accepted".into());
                    }
                }
                ScenarioCommand::Metrics { replica } => {
                    let _ = get(&replicas, replica)?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            errors.push(format!("command {index}: {error}"));
        }
    }
    let status = if !errors.is_empty() {
        Status::Failed
    } else if !unsupported.is_empty() {
        Status::Unsupported
    } else {
        Status::Passed
    };
    Response {
        protocol: PROTOCOL.into(),
        candidate: "loro".into(),
        candidate_version: "1.13.7".into(),
        scenario: request.scenario,
        status,
        commands_executed: executed,
        observations,
        state_hashes,
        metrics,
        unsupported,
        errors,
    }
}

fn apply(doc: &LoroDoc, operation: &Operation) -> Result<(), String> {
    match operation {
        Operation::TextInsert { index, text } => doc
            .get_text("text")
            .insert(*index as usize, text)
            .map_err(err)?,
        Operation::TextDelete { index, len } => doc
            .get_text("text")
            .delete(*index as usize, *len as usize)
            .map_err(err)?,
        Operation::TextMark {
            index,
            len,
            key,
            value,
        } => doc
            .get_text("text")
            .mark(
                *index as usize..(*index + *len) as usize,
                key,
                value.as_str(),
            )
            .map_err(err)?,
        Operation::BlockInsert { index, id, text } => doc
            .get_movable_list("blocks")
            .insert(*index as usize, format!("{id}\0{text}"))
            .map_err(err)?,
        Operation::BlockMove { from, to } => doc
            .get_movable_list("blocks")
            .mov(*from as usize, *to as usize)
            .map_err(err)?,
        Operation::PropertySet { key, value } => doc
            .get_map("properties")
            .insert(key, value.clone())
            .map_err(err)?,
        Operation::ExternalReplace { text } => {
            let native = doc.get_text("text");
            let len = native.len_unicode();
            if len > 0 {
                native.delete(0, len).map_err(err)?;
            }
            native.insert(0, text).map_err(err)?;
        }
    }
    Ok(())
}

fn apply_workload(doc: &LoroDoc, kind: WorkloadKind, count: u64, seed: u64) -> Result<(), String> {
    for i in 0..count {
        match kind {
            WorkloadKind::Text => doc
                .get_text("text")
                .insert(doc.get_text("text").len_unicode(), "x")
                .map_err(err)?,
            WorkloadKind::ListMove => {
                let list = doc.get_movable_list("blocks");
                if i < 100 {
                    list.push(format!("b{i}\0block {i}")).map_err(err)?;
                } else {
                    list.mov(
                        (i as usize + seed as usize) % 100,
                        ((i * 17) as usize + seed as usize) % 100,
                    )
                    .map_err(err)?;
                }
            }
            WorkloadKind::Properties => doc
                .get_map("properties")
                .insert(&format!("k{}", (i + seed) % 1000), i.to_string())
                .map_err(err)?,
            WorkloadKind::Mixed => match i % 3 {
                0 => doc
                    .get_text("text")
                    .insert(doc.get_text("text").len_unicode(), "m")
                    .map_err(err)?,
                1 => doc
                    .get_map("properties")
                    .insert(&format!("k{}", (i + seed) % 1000), i.to_string())
                    .map_err(err)?,
                _ => {
                    let list = doc.get_movable_list("blocks");
                    list.push(format!("b{i}\0block {i}")).map_err(err)?;
                }
            },
        }
    }
    Ok(())
}

fn materialize(doc: &LoroDoc) -> Materialized {
    let blocks = doc
        .get_movable_list("blocks")
        .get_deep_value()
        .to_json_value()
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|value| {
            let raw = value.as_str().unwrap_or_default();
            let (id, text) = raw.split_once('\0').unwrap_or((raw, ""));
            Block {
                id: id.into(),
                text: text.into(),
            }
        })
        .collect();
    let properties = doc
        .get_map("properties")
        .get_deep_value()
        .to_json_value()
        .as_object()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|(key, value)| (key, value.as_str().unwrap_or_default().into()))
        .collect();
    Materialized {
        text: doc.get_text("text").to_string(),
        blocks,
        properties,
    }
}

fn peer_id(name: &str) -> u64 {
    name.bytes().fold(1469598103934665603, |hash, byte| {
        hash.wrapping_mul(1099511628211) ^ u64::from(byte)
    })
}
fn get<'a>(replicas: &'a HashMap<String, Replica>, name: &str) -> Result<&'a Replica, String> {
    replicas
        .get(name)
        .ok_or_else(|| format!("unknown replica {name}"))
}
fn get_mut<'a>(
    replicas: &'a mut HashMap<String, Replica>,
    name: &str,
) -> Result<&'a mut Replica, String> {
    replicas
        .get_mut(name)
        .ok_or_else(|| format!("unknown replica {name}"))
}
fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
