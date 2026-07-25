use std::collections::{BTreeMap, HashMap};

use crdt_spike_contract::{
    Block, Materialized, Metrics, Operation, PROTOCOL, Request, Response, ScenarioCommand, Status,
    Unsupported, WorkloadKind, state_hash,
};
use yrs::{
    Array, Assoc, Doc, GetString, IndexedSequence, Map, Out, ReadTxn, StateVector, Text, Transact,
    Update, WriteTxn, undo::UndoManager, updates::decoder::Decode,
};

struct Replica {
    doc: Doc,
    undo: UndoManager<()>,
    actor: Option<String>,
}

pub fn run(request: Request) -> Response {
    let mut replicas: HashMap<String, Replica> = HashMap::new();
    let mut updates: HashMap<String, Vec<u8>> = HashMap::new();
    let mut update_versions: HashMap<String, StateVector> = HashMap::new();
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
        ("identity_preserving_move".into(), false),
        ("per_peer_undo".into(), true),
        ("shallow_snapshot".into(), false),
    ]);

    for (index, command) in request.commands.iter().enumerate() {
        executed += 1;
        let result: Result<(), String> = (|| {
            match command {
                ScenarioCommand::CreateReplica { replica } => {
                    replicas.insert(replica.clone(), new_replica(replica));
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
                        unsupported.push(Unsupported { command_index: index, feature: "multiple_actors_on_one_replica".into(), reason: "Yrs UndoManager tracks configured transaction origins; this spike binds one actor to each native document".into() });
                        return Ok(());
                    }
                    replica.actor = Some(actor.clone());
                    if matches!(operation, Operation::BlockMove { .. }) {
                        unsupported.push(Unsupported { command_index: index, feature: "identity_preserving_move".into(), reason: "Yrs 0.27.3 Array exposes insert/remove but no public native move; delete+insert would change CRDT identity".into() });
                        return Ok(());
                    }
                    apply(&replica.doc, operation)?;
                    replica.undo.reset();
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
                    if matches!(kind, WorkloadKind::ListMove) {
                        unsupported.push(Unsupported {
                            command_index: index,
                            feature: "identity_preserving_move".into(),
                            reason: "Yrs 0.27.3 has no native Array move".into(),
                        });
                        return Ok(());
                    }
                    apply_workload(&replica.doc, *kind, *count, *seed);
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
                    let native = doc.get_or_insert_text("text");
                    let mut txn = doc.transact_mut();
                    let position = native
                        .sticky_index(&txn, *index, Assoc::After)
                        .ok_or("could not create sticky index")?;
                    native.insert(&mut txn, *insert_at, text);
                    let actual = position
                        .get_offset(&txn)
                        .ok_or("could not resolve sticky index")?
                        .index;
                    if actual != *expected_index {
                        return Err(format!(
                            "relative position mapped to {actual}, expected {expected_index}"
                        ));
                    }
                    metrics.operation_count += 1;
                }
                ScenarioCommand::ExportUpdates { replica, update } => {
                    let bytes = get(&replicas, replica)?
                        .doc
                        .transact()
                        .encode_state_as_update_v1(&StateVector::default());
                    metrics.update_bytes += bytes.len() as u64;
                    updates.insert(update.clone(), bytes);
                    update_versions.insert(
                        update.clone(),
                        get(&replicas, replica)?.doc.transact().state_vector(),
                    );
                }
                ScenarioCommand::ExportIncremental {
                    replica,
                    since_update,
                    update,
                } => {
                    let since = update_versions
                        .get(since_update)
                        .ok_or_else(|| format!("unknown baseline update {since_update}"))?;
                    let transaction = get(&replicas, replica)?.doc.transact();
                    let bytes = transaction.encode_state_as_update_v1(since);
                    let version = transaction.state_vector();
                    metrics.update_bytes += bytes.len() as u64;
                    updates.insert(update.clone(), bytes);
                    update_versions.insert(update.clone(), version);
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
                        let decoded = Update::decode_v1(&source[..len]);
                        if *truncate_bytes == 0 {
                            get_mut(&mut replicas, replica)?
                                .doc
                                .transact_mut()
                                .apply_update(decoded.map_err(err)?)
                                .map_err(err)?;
                        } else if decoded.is_ok() {
                            return Err("truncated update decoded successfully".into());
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
                    if !replica.undo.undo_blocking() {
                        return Err("undo stack empty".into());
                    }
                }
                ScenarioCommand::Snapshot { replica, snapshot } => {
                    let bytes = get(&replicas, replica)?
                        .doc
                        .transact()
                        .encode_state_as_update_v1(&StateVector::default());
                    metrics.snapshot_bytes += bytes.len() as u64;
                    snapshots.insert(snapshot.clone(), bytes);
                }
                ScenarioCommand::CompactedSnapshot { replica, snapshot } => {
                    let bytes = get(&replicas, replica)?
                        .doc
                        .transact()
                        .encode_state_as_update_v1(&StateVector::default());
                    unsupported.push(Unsupported { command_index: index, feature: "compacted_snapshot".into(), reason: "Yrs 0.27.3 exposes update snapshots but no native shallow/compacted snapshot API".into() });
                    metrics.snapshot_bytes += bytes.len() as u64;
                    snapshots.insert(snapshot.clone(), bytes);
                }
                ScenarioCommand::Restore { replica, snapshot } => {
                    let bytes = snapshots
                        .get(snapshot)
                        .ok_or_else(|| format!("unknown snapshot {snapshot}"))?;
                    let restored = new_replica(replica);
                    restored
                        .doc
                        .transact_mut()
                        .apply_update(Update::decode_v1(bytes).map_err(err)?)
                        .map_err(err)?;
                    replicas.insert(replica.clone(), restored);
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
                    if Update::decode_v1(&bytes[..len]).is_ok() {
                        return Err("truncated snapshot decoded successfully".into());
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
        candidate: "yrs".into(),
        candidate_version: "0.27.3".into(),
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

fn new_replica(name: &str) -> Replica {
    let doc = Doc::with_client_id(client_id(name));
    let text = doc.get_or_insert_text("text");
    let blocks = doc.get_or_insert_array("blocks");
    let properties = doc.get_or_insert_map("properties");
    let mut undo = UndoManager::new();
    undo.expand_scope(&doc, &text);
    undo.expand_scope(&doc, &blocks);
    undo.expand_scope(&doc, &properties);
    undo.include_origin(doc.client_id());
    Replica {
        doc,
        undo,
        actor: None,
    }
}

fn apply(doc: &Doc, operation: &Operation) -> Result<(), String> {
    let mut txn = doc.transact_mut_with(doc.client_id());
    match operation {
        Operation::TextInsert { index, text } => txn
            .get_or_insert_text("text")
            .insert(&mut txn, *index, text),
        Operation::TextDelete { index, len } => txn
            .get_or_insert_text("text")
            .remove_range(&mut txn, *index, *len),
        Operation::TextMark {
            index,
            len,
            key,
            value,
        } => txn.get_or_insert_text("text").format(
            &mut txn,
            *index,
            *len,
            [(key.clone().into(), value.clone().into())]
                .into_iter()
                .collect(),
        ),
        Operation::BlockInsert { index, id, text } => {
            txn.get_or_insert_array("blocks")
                .insert(&mut txn, *index, format!("{id}\0{text}"));
        }
        Operation::BlockMove { .. } => unreachable!("reported unsupported before apply"),
        Operation::PropertySet { key, value } => {
            txn.get_or_insert_map("properties")
                .insert(&mut txn, key.clone(), value.clone());
        }
        Operation::ExternalReplace { text } => {
            let native = txn.get_or_insert_text("text");
            let len = native.len(&txn);
            if len > 0 {
                native.remove_range(&mut txn, 0, len);
            }
            native.insert(&mut txn, 0, text);
        }
    }
    Ok(())
}

fn apply_workload(doc: &Doc, kind: WorkloadKind, count: u64, seed: u64) {
    for i in 0..count {
        let mut txn = doc.transact_mut_with(doc.client_id());
        match kind {
            WorkloadKind::Text => {
                let text = txn.get_or_insert_text("text");
                let len = text.len(&txn);
                text.insert(&mut txn, len, "x");
            }
            WorkloadKind::Properties => {
                txn.get_or_insert_map("properties").insert(
                    &mut txn,
                    format!("k{}", (i + seed) % 1000),
                    i.to_string(),
                );
            }
            WorkloadKind::Mixed => match i % 3 {
                0 => {
                    let text = txn.get_or_insert_text("text");
                    let len = text.len(&txn);
                    text.insert(&mut txn, len, "m");
                }
                1 => {
                    txn.get_or_insert_map("properties").insert(
                        &mut txn,
                        format!("k{}", (i + seed) % 1000),
                        i.to_string(),
                    );
                }
                _ => {
                    let list = txn.get_or_insert_array("blocks");
                    let len = list.len(&txn);
                    list.insert(&mut txn, len, format!("b{i}\0block {i}"));
                }
            },
            WorkloadKind::ListMove => unreachable!("reported unsupported"),
        }
    }
}

fn materialize(doc: &Doc) -> Materialized {
    let txn = doc.transact();
    let text = txn
        .get_text("text")
        .map(|value| value.get_string(&txn))
        .unwrap_or_default();
    let blocks = txn
        .get_array("blocks")
        .map(|array| {
            array
                .iter(&txn)
                .map(|value| {
                    let raw = out_string(value, &txn);
                    let (id, text) = raw.split_once('\0').unwrap_or((&raw, ""));
                    Block {
                        id: id.into(),
                        text: text.into(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let properties = txn
        .get_map("properties")
        .map(|map| {
            map.iter(&txn)
                .map(|(key, value)| (key.to_string(), out_string(value, &txn)))
                .collect()
        })
        .unwrap_or_default();
    Materialized {
        text,
        blocks,
        properties,
    }
}

fn out_string(value: Out, txn: &impl ReadTxn) -> String {
    match value {
        Out::Any(yrs::Any::String(value)) => value.to_string(),
        other => other.to_string(txn),
    }
}
fn client_id(name: &str) -> u64 {
    name.bytes().fold(1469598103934665603_u64, |hash, byte| {
        hash.wrapping_mul(1099511628211) ^ u64::from(byte)
    }) & ((1_u64 << 53) - 1)
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
