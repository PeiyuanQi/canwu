use super::event_payloads::{self, RuntimeEventPayload};
use super::{
    ADMISSION_CURSOR_FORMAT_VERSION, BoundaryRecord, BoundarySystemContract,
    CHECKPOINT_JOURNAL_FORMAT_VERSION, COMMITMENT_FORMAT_VERSION, CanwuError, CheckpointJournal,
    CommandAttemptRecord, CommandRecord, CommitmentRoots, DecisionState, DeterministicRng,
    DomainRecord, DomainRecordClass, DomainRecordSchema, DomainReferenceSchema, ENGINE_VERSION,
    ErrorCode, EventKind, EvidenceCursor, EvidenceJournalSegment, IngressRecord, KnowledgeSnapshot,
    PayloadSchema, PluginActionDescriptor, PluginComponentRecord, PluginDescriptor,
    PluginIngressDescriptor, RandomDrawAddress, RandomDrawOutcome, RandomDrawProducer,
    RandomDrawRecord, RandomStreamKey, RandomStreamState, ReservationRef, RunConfigurationSnapshot,
    RunManifest, SNAPSHOT_FORMAT_VERSION, STATE_REVISION_FORMAT_VERSION, Scenario, ScheduledRecord,
    SchemaRegistry, SimEvent, SimTime, Simulation, SimulationCheckpoint, SimulationSnapshot,
    StateKey, StateVisibility, SystemCadence, SystemContract, WorldSnapshot,
    boundary_state_hash_for_commitments, canonical_hash, checkpoint_hash_for_commitments,
    commitment_roots_are_canonical, compute_boundary_hash, invalid_snapshot,
    invalid_snapshot_error, is_canonical_hash, manifest, random_stream_commitment_root,
    snapshot_checkpoint_hash, snapshot_commitment_roots,
};
use canwu_core::{
    ArmyId, DomainRecordKind, EntityRef, EventId, PersonId, RandomDrawId, TerritoryId,
};
use canwu_event::{CauseRef, EventAudience};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) const LEGACY_V4_ENGINE_VERSION: &str = "0.4.0";

fn reject_unknown_fields(input: &Value, encoded: &Value, path: &str) -> Result<(), CanwuError> {
    match (input, encoded) {
        (Value::Object(input), Value::Object(encoded)) => {
            for (key, value) in input {
                let next = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                let Some(expected) = encoded.get(key) else {
                    return invalid_snapshot(format!(
                        "strict legacy wire contains unknown field `{next}`"
                    ));
                };
                reject_unknown_fields(value, expected, &next)?;
            }
        }
        (Value::Array(input), Value::Array(encoded)) => {
            if input.len() != encoded.len() {
                return invalid_snapshot(format!(
                    "strict legacy wire array `{path}` changed shape during decoding"
                ));
            }
            for (index, (value, expected)) in input.iter().zip(encoded).enumerate() {
                reject_unknown_fields(value, expected, &format!("{path}[{index}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn deserialize_strict<T>(value: &Value, label: &str) -> Result<T, CanwuError>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let decoded: T = serde_json::from_value(value.clone()).map_err(|error| {
        invalid_snapshot_error(format!("could not deserialize strict {label}: {error}"))
    })?;
    let encoded = serde_json::to_value(&decoded).map_err(|error| {
        invalid_snapshot_error(format!("could not re-encode strict {label}: {error}"))
    })?;
    reject_unknown_fields(value, &encoded, "")?;
    Ok(decoded)
}

fn deserialize_strict_json<T>(json: &str, input: &Value, label: &str) -> Result<T, CanwuError>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let decoded: T = serde_json::from_str(json).map_err(|error| {
        invalid_snapshot_error(format!("could not deserialize strict {label}: {error}"))
    })?;
    let encoded = serde_json::to_value(&decoded).map_err(|error| {
        invalid_snapshot_error(format!("could not re-encode strict {label}: {error}"))
    })?;
    reject_unknown_fields(input, &encoded, "")?;
    Ok(decoded)
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LegacyV4RandomDrawRecord {
    pub id: RandomDrawId,
    pub at: SimTime,
    pub stream: RandomStreamKey,
    pub position: u64,
    pub upper_exclusive: u64,
    pub value: u64,
    pub purpose: String,
    pub producer: RandomDrawProducer,
    #[serde(default)]
    pub outcome: Option<RandomDrawOutcome>,
    pub cause: CauseRef,
    pub correlation_id: u64,
}

impl From<LegacyV4RandomDrawRecord> for RandomDrawRecord {
    fn from(value: LegacyV4RandomDrawRecord) -> Self {
        Self {
            id: value.id,
            at: value.at,
            stream: value.stream,
            address: RandomDrawAddress::Sequential {
                position: value.position,
            },
            operation_evidence: None,
            upper_exclusive: value.upper_exclusive,
            value: value.value,
            purpose: value.purpose,
            producer: value.producer,
            outcome: value.outcome,
            cause: value.cause,
            correlation_id: value.correlation_id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LegacyV4DomainRecordSchema {
    kind: DomainRecordKind,
    class: DomainRecordClass,
    payload_schema: PayloadSchema,
    references: Vec<DomainReferenceSchema>,
}

impl From<LegacyV4DomainRecordSchema> for DomainRecordSchema {
    fn from(value: LegacyV4DomainRecordSchema) -> Self {
        let mut current = Self::new(value.kind, value.class);
        current.payload_schema = value.payload_schema;
        current.references = value.references;
        current
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LegacyV4BoundarySystemContract {
    name: String,
    phase: super::BoundaryPhase,
    cadence: SystemCadence,
    reads: Vec<StateKey>,
    writes: Vec<StateKey>,
    emits: Vec<String>,
    reservation_offers: Vec<StateKey>,
    reservation_requests: Vec<StateKey>,
    reservation_reads: Vec<ReservationRef>,
    #[serde(default)]
    random_streams: Vec<RandomStreamKey>,
    visibility: StateVisibility,
}

impl From<LegacyV4BoundarySystemContract> for BoundarySystemContract {
    fn from(value: LegacyV4BoundarySystemContract) -> Self {
        Self {
            name: value.name,
            phase: value.phase,
            cadence: value.cadence,
            reads: value.reads,
            writes: value.writes,
            emits: value.emits,
            reservation_offers: value.reservation_offers,
            reservation_requests: value.reservation_requests,
            reservation_reads: value.reservation_reads,
            random_streams: value.random_streams,
            knowledge_writes: Vec::new(),
            plugin_ingress_targets: Vec::new(),
            visibility: value.visibility,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LegacyV4PluginDescriptor {
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    semantic_hash: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    event_audiences: BTreeMap<String, EventAudience>,
    systems: Vec<SystemContract>,
    #[serde(default)]
    boundary_systems: Vec<LegacyV4BoundarySystemContract>,
    commands: Vec<PluginActionDescriptor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    ingress: Vec<PluginIngressDescriptor>,
    schema_types: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    record_schemas: Vec<LegacyV4DomainRecordSchema>,
}

impl From<LegacyV4PluginDescriptor> for PluginDescriptor {
    fn from(value: LegacyV4PluginDescriptor) -> Self {
        Self {
            name: value.name,
            version: value.version,
            semantic_hash: value.semantic_hash,
            event_audiences: value.event_audiences,
            systems: value.systems,
            boundary_systems: value.boundary_systems.into_iter().map(Into::into).collect(),
            commands: value.commands,
            ingress: value.ingress,
            schema_types: value.schema_types,
            record_schemas: value.record_schemas.into_iter().map(Into::into).collect(),
            knowledge_schemas: Vec::new(),
        }
    }
}

fn legacy_sorted_hash_by<T, K, F>(
    domain: &str,
    values: &[T],
    mut key: F,
) -> Result<String, CanwuError>
where
    T: Serialize,
    K: Ord,
    F: FnMut(&T) -> K,
{
    let mut ordered: Vec<_> = values.iter().collect();
    ordered.sort_by_key(|value| key(value));
    canonical_hash(domain, &ordered)
}

#[derive(Serialize)]
struct LegacyV4IdentityCommitmentMaterial<'a> {
    engine_version: &'a str,
    snapshot_format_version: u32,
    run_manifest: &'a RunManifest,
    run_manifest_hash: &'a str,
    initial_time: SimTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    initial_scenario: Option<&'a Scenario>,
    plugin_descriptors: String,
    schema: &'a SchemaRegistry,
}

fn legacy_identity_commitment_root(
    run_manifest: &RunManifest,
    run_manifest_hash: &str,
    initial_time: SimTime,
    initial_scenario: Option<&Scenario>,
    plugin_descriptors: &[LegacyV4PluginDescriptor],
    schema: &SchemaRegistry,
) -> Result<String, CanwuError> {
    canonical_hash(
        "canwu.commitment.identity.v1",
        &LegacyV4IdentityCommitmentMaterial {
            engine_version: LEGACY_V4_ENGINE_VERSION,
            snapshot_format_version: 4,
            run_manifest,
            run_manifest_hash,
            initial_time,
            initial_scenario,
            plugin_descriptors: legacy_sorted_hash_by(
                "canwu.commitment.identity.plugins.v1",
                plugin_descriptors,
                |descriptor| descriptor.name.clone(),
            )?,
            schema,
        },
    )
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LegacyV4EventKind {
    MoveOrdered {
        army: ArmyId,
        from: TerritoryId,
        to: TerritoryId,
        arrival_at: SimTime,
    },
    ArmyArrived {
        army: ArmyId,
        territory: TerritoryId,
    },
    ReportDispatched {
        recipient: PersonId,
        army: ArmyId,
        arrives_at: SimTime,
    },
    KnowledgeUpdated {
        recipient: PersonId,
        army: ArmyId,
        known_location: TerritoryId,
    },
    DebugFieldChanged {
        entity: EntityRef,
        field: String,
        old_value: String,
        new_value: String,
    },
    Plugin {
        plugin: String,
        event_type: String,
    },
}

impl From<LegacyV4EventKind> for EventKind {
    fn from(value: LegacyV4EventKind) -> Self {
        match value {
            LegacyV4EventKind::MoveOrdered {
                army,
                from,
                to,
                arrival_at,
            } => event_payloads::MoveOrdered {
                army,
                from,
                to,
                arrival_at,
            }
            .into_kind(),
            LegacyV4EventKind::ArmyArrived { army, territory } => {
                event_payloads::ArmyArrived { army, territory }.into_kind()
            }
            LegacyV4EventKind::ReportDispatched {
                recipient,
                army,
                arrives_at,
            } => event_payloads::ReportDispatched {
                recipient,
                army,
                arrives_at,
            }
            .into_kind(),
            LegacyV4EventKind::KnowledgeUpdated {
                recipient,
                army,
                known_location,
            } => event_payloads::KnowledgeUpdated {
                recipient,
                army,
                known_location,
            }
            .into_kind(),
            LegacyV4EventKind::DebugFieldChanged {
                entity,
                field,
                old_value,
                new_value,
            } => event_payloads::DebugFieldChanged {
                entity,
                field,
                old_value,
                new_value,
            }
            .into_kind(),
            LegacyV4EventKind::Plugin { plugin, event_type } => {
                EventKind::plugin(plugin, event_type)
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyV4SimEvent {
    id: EventId,
    timestamp: SimTime,
    kind: LegacyV4EventKind,
    affected_entities: Vec<EntityRef>,
    summary: String,
    cause: Option<CauseRef>,
    correlation_id: u64,
}

impl From<LegacyV4SimEvent> for SimEvent {
    fn from(value: LegacyV4SimEvent) -> Self {
        Self {
            id: value.id,
            timestamp: value.timestamp,
            kind: value.kind.into(),
            affected_entities: value.affected_entities,
            summary: value.summary,
            cause: value.cause,
            correlation_id: value.correlation_id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LegacyV4SimulationSnapshot {
    pub engine_version: String,
    pub snapshot_format_version: u32,
    #[serde(default)]
    pub run_manifest: Option<RunManifest>,
    #[serde(default)]
    pub run_manifest_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_configuration: Option<RunConfigurationSnapshot>,
    #[serde(default)]
    pub checkpoint_hash: String,
    #[serde(default, skip_serializing_if = "super::is_zero_u32")]
    pub commitment_format_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commitment_roots: Option<CommitmentRoots>,
    #[serde(default)]
    pub revision_format_version: u32,
    #[serde(default, skip_serializing_if = "super::is_zero_u64")]
    pub state_revision: u64,
    #[serde(default, skip_serializing_if = "super::is_zero_u32")]
    pub replay_revision_format_version: u32,
    #[serde(default, skip_serializing_if = "super::is_zero_u32")]
    pub admission_cursor_format_version: u32,
    #[serde(default, skip_serializing_if = "super::is_zero_u64")]
    pub admitted_attempt_count: u64,
    #[serde(default, skip_serializing_if = "super::is_zero_u64")]
    pub admitted_command_count: u64,
    #[serde(default, skip_serializing_if = "super::is_zero_u64")]
    pub admitted_event_count: u64,
    pub initial_time: SimTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_scenario: Option<Scenario>,
    pub now: SimTime,
    pub plugin_registration_closed: bool,
    pub world: WorldSnapshot,
    pub knowledge: KnowledgeSnapshot,
    events: Vec<LegacyV4SimEvent>,
    pub commands: Vec<CommandRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command_attempts: Vec<CommandAttemptRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ingress: Vec<IngressRecord>,
    #[serde(default)]
    pub boundaries: Vec<BoundaryRecord>,
    pub plugin_components: Vec<PluginComponentRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domain_records: Vec<DomainRecord>,
    #[serde(default, skip_serializing_if = "DecisionState::is_empty")]
    pub decisions: DecisionState,
    pub plugin_descriptors: Vec<LegacyV4PluginDescriptor>,
    pub schema: SchemaRegistry,
    #[serde(default)]
    pub root_seed: u64,
    #[serde(default)]
    pub random_streams: Vec<RandomStreamState>,
    #[serde(default)]
    pub random_draws: Vec<LegacyV4RandomDrawRecord>,
    pub scheduled: Vec<ScheduledRecord>,
    #[serde(default, rename = "rng", skip_serializing_if = "Option::is_none")]
    pub legacy_rng: Option<DeterministicRng>,
    pub next_event_id: u64,
    pub next_command_id: u64,
    #[serde(default = "super::one_u64", skip_serializing_if = "super::is_one_u64")]
    pub next_command_attempt_id: u64,
    #[serde(default = "super::one_u64", skip_serializing_if = "super::is_one_u64")]
    pub next_ingress_id: u64,
    #[serde(default)]
    pub next_boundary_id: u64,
    #[serde(default)]
    pub next_random_draw_id: u64,
    pub next_schedule_sequence: u64,
    pub next_correlation_id: u64,
    #[serde(default = "super::one_u64", skip_serializing_if = "super::is_one_u64")]
    pub next_decision_trace_id: u64,
}

impl LegacyV4SimulationSnapshot {
    fn into_current(self) -> SimulationSnapshot {
        SimulationSnapshot {
            engine_version: self.engine_version,
            snapshot_format_version: self.snapshot_format_version,
            run_manifest: self.run_manifest,
            run_manifest_hash: self.run_manifest_hash,
            run_configuration: self.run_configuration,
            checkpoint_hash: self.checkpoint_hash,
            commitment_format_version: self.commitment_format_version,
            commitment_roots: self.commitment_roots,
            revision_format_version: self.revision_format_version,
            state_revision: self.state_revision,
            replay_revision_format_version: self.replay_revision_format_version,
            admission_cursor_format_version: self.admission_cursor_format_version,
            admitted_attempt_count: self.admitted_attempt_count,
            admitted_command_count: self.admitted_command_count,
            admitted_event_count: self.admitted_event_count,
            initial_time: self.initial_time,
            initial_scenario: self.initial_scenario,
            now: self.now,
            plugin_registration_closed: self.plugin_registration_closed,
            entities: super::scenario::legacy_entities(&self.world),
            world: self.world,
            knowledge: self.knowledge,
            events: self.events.into_iter().map(Into::into).collect(),
            commands: self.commands,
            command_attempts: self.command_attempts,
            ingress: self.ingress,
            boundaries: self.boundaries,
            plugin_components: self.plugin_components,
            domain_records: self.domain_records,
            decisions: self.decisions,
            plugin_descriptors: self
                .plugin_descriptors
                .into_iter()
                .map(Into::into)
                .collect(),
            schema: self.schema,
            root_seed: self.root_seed,
            random_streams: self.random_streams,
            random_draws: self.random_draws.into_iter().map(Into::into).collect(),
            scheduled: self.scheduled,
            legacy_rng: self.legacy_rng,
            next_event_id: self.next_event_id,
            next_command_id: self.next_command_id,
            next_command_attempt_id: self.next_command_attempt_id,
            next_ingress_id: self.next_ingress_id,
            next_boundary_id: self.next_boundary_id,
            next_random_draw_id: self.next_random_draw_id,
            next_knowledge_record_id: 1,
            next_schedule_sequence: self.next_schedule_sequence,
            next_correlation_id: self.next_correlation_id,
            next_decision_trace_id: self.next_decision_trace_id,
        }
    }
}

#[derive(Serialize)]
struct LegacyRandomCommitmentMaterial {
    root_seed: u64,
    streams: String,
    draws: String,
}

fn legacy_random_commitment_root(
    root_seed: u64,
    streams: &[RandomStreamState],
    draws: &[LegacyV4RandomDrawRecord],
) -> Result<String, CanwuError> {
    let mut ordered: Vec<_> = draws.iter().collect();
    ordered.sort_by_key(|draw| draw.id);
    canonical_hash(
        "canwu.commitment.random.v1",
        &LegacyRandomCommitmentMaterial {
            root_seed,
            streams: random_stream_commitment_root(streams)?,
            draws: canonical_hash("canwu.commitment.random.draws.v1", &ordered)?,
        },
    )
}

fn validate_boundary_chain(boundaries: &[BoundaryRecord]) -> Result<(), CanwuError> {
    let mut previous = super::GENESIS_BOUNDARY_HASH;
    for boundary in boundaries {
        if boundary.previous_hash != previous || compute_boundary_hash(boundary)? != boundary.hash {
            return invalid_snapshot("legacy format-4 boundary hash chain is inconsistent");
        }
        previous = &boundary.hash;
    }
    Ok(())
}

fn validate_legacy_commitments(
    legacy: &LegacyV4SimulationSnapshot,
    shadow: &SimulationSnapshot,
) -> Result<(), CanwuError> {
    if legacy.commitment_format_version != COMMITMENT_FORMAT_VERSION {
        return invalid_snapshot("legacy format-4 snapshot must use commitment format 1");
    }
    let stored = legacy.commitment_roots.as_ref().ok_or_else(|| {
        invalid_snapshot_error("legacy format-4 snapshot is missing commitment roots")
    })?;
    if !commitment_roots_are_canonical(stored) {
        return invalid_snapshot("legacy format-4 commitment roots are not canonical");
    }
    let mut expected = snapshot_commitment_roots(shadow)?;
    expected.random = legacy_random_commitment_root(
        legacy.root_seed,
        &legacy.random_streams,
        &legacy.random_draws,
    )?;
    expected.identity = legacy_identity_commitment_root(
        legacy.run_manifest.as_ref().ok_or_else(|| {
            invalid_snapshot_error("legacy format-4 snapshot is missing its run manifest")
        })?,
        &legacy.run_manifest_hash,
        legacy.initial_time,
        legacy.initial_scenario.as_ref(),
        &legacy.plugin_descriptors,
        &legacy.schema,
    )?;
    if &expected != stored {
        return invalid_snapshot(
            "legacy format-4 commitment roots do not match the persisted state",
        );
    }
    let expected_checkpoint = checkpoint_hash_for_commitments(
        stored,
        &legacy.run_manifest_hash,
        legacy.commitment_format_version,
        legacy.revision_format_version,
        legacy.state_revision,
        legacy.replay_revision_format_version,
    )?;
    if !is_canonical_hash(&legacy.checkpoint_hash) || expected_checkpoint != legacy.checkpoint_hash
    {
        return invalid_snapshot("legacy format-4 checkpoint hash is inconsistent");
    }
    if let Some(boundary) = legacy.boundaries.last()
        && let Some(state_hash) = boundary.state_hash.as_deref()
    {
        let Some(hash) = state_hash.strip_prefix(super::BOUNDARY_STATE_HASH_V1_PREFIX) else {
            return invalid_snapshot(
                "legacy format-4 migration requires the final boundary state hash to use v1 commitments",
            );
        };
        let mut boundary_roots = stored.clone();
        boundary_roots.boundary_chain = canonical_hash(
            "canwu.commitment.boundary-chain.v1",
            boundary.previous_hash.as_str(),
        )?;
        if !is_canonical_hash(hash)
            || boundary_state_hash_for_commitments(&boundary_roots)? != state_hash
        {
            return invalid_snapshot("legacy format-4 final boundary state hash is inconsistent");
        }
    }
    Ok(())
}

fn validate_legacy_v4(
    legacy: &LegacyV4SimulationSnapshot,
) -> Result<SimulationSnapshot, CanwuError> {
    if legacy.engine_version != LEGACY_V4_ENGINE_VERSION || legacy.snapshot_format_version != 4 {
        return Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            "legacy migration accepts only engine 0.4.0 snapshot format 4",
        ));
    }
    if legacy.legacy_rng.is_some() {
        return invalid_snapshot("legacy format-4 snapshots cannot contain the pre-format-4 RNG");
    }
    if legacy.revision_format_version != STATE_REVISION_FORMAT_VERSION
        || legacy.admission_cursor_format_version != ADMISSION_CURSOR_FORMAT_VERSION
        || legacy.replay_revision_format_version > STATE_REVISION_FORMAT_VERSION
    {
        return invalid_snapshot("legacy format-4 revision or admission format is unsupported");
    }
    let mut shadow = legacy.clone().into_current();
    if shadow.run_configuration.is_none() {
        let manifest = shadow.run_manifest.as_ref().ok_or_else(|| {
            invalid_snapshot_error("legacy format-4 snapshot is missing its run manifest")
        })?;
        shadow.run_configuration = Some(super::migration::inferred_run_configuration(manifest)?);
    }
    let run_manifest = shadow.run_manifest.as_ref().ok_or_else(|| {
        invalid_snapshot_error("legacy format-4 snapshot is missing its run manifest")
    })?;
    manifest::validate(run_manifest, shadow.initial_scenario.as_ref(), true)?;
    if manifest::hash(run_manifest)? != shadow.run_manifest_hash {
        return invalid_snapshot("legacy format-4 run manifest hash is inconsistent");
    }
    validate_boundary_chain(&legacy.boundaries)?;
    validate_legacy_commitments(legacy, &shadow)?;
    Ok(shadow)
}

fn rebase_boundary_chain(snapshot: &mut SimulationSnapshot) -> Result<(), CanwuError> {
    let mut previous = super::GENESIS_BOUNDARY_HASH.to_owned();
    for boundary in &mut snapshot.boundaries {
        boundary.previous_hash.clone_from(&previous);
        boundary.hash = compute_boundary_hash(boundary)?;
        previous.clone_from(&boundary.hash);
    }
    Ok(())
}

fn refresh_migrated_boundary_head_state_hash(
    snapshot: &mut SimulationSnapshot,
) -> Result<(), CanwuError> {
    let Some(previous_hash) = snapshot
        .boundaries
        .last()
        .map(|boundary| boundary.previous_hash.clone())
    else {
        return Ok(());
    };
    let mut roots = snapshot_commitment_roots(snapshot)?;
    roots.boundary_chain =
        canonical_hash("canwu.commitment.boundary-chain.v1", previous_hash.as_str())?;
    let state_hash = boundary_state_hash_for_commitments(&roots)?;
    let boundary = snapshot
        .boundaries
        .last_mut()
        .expect("a captured boundary head must still exist");
    boundary.state_hash = Some(state_hash);
    boundary.hash = compute_boundary_hash(boundary)?;
    Ok(())
}

pub(super) fn migrate_legacy_v4(
    legacy: &LegacyV4SimulationSnapshot,
) -> Result<SimulationSnapshot, CanwuError> {
    let mut snapshot = validate_legacy_v4(legacy)?;
    ENGINE_VERSION.clone_into(&mut snapshot.engine_version);
    snapshot.snapshot_format_version = SNAPSHOT_FORMAT_VERSION;
    snapshot.replay_revision_format_version = 0;
    snapshot.commitment_roots = None;
    snapshot.checkpoint_hash.clear();
    rebase_boundary_chain(&mut snapshot)?;
    refresh_migrated_boundary_head_state_hash(&mut snapshot)?;
    snapshot.commitment_roots = Some(snapshot_commitment_roots(&snapshot)?);
    snapshot.checkpoint_hash = snapshot_checkpoint_hash(&snapshot)?;
    Ok(snapshot)
}

pub(super) fn deserialize_snapshot_json(json: &str) -> Result<SimulationSnapshot, CanwuError> {
    let value: Value = serde_json::from_str(json).map_err(|error| {
        invalid_snapshot_error(format!("could not deserialize snapshot envelope: {error}"))
    })?;
    let object = value
        .as_object()
        .ok_or_else(|| invalid_snapshot_error("snapshot envelope must be an object"))?;
    let format = object
        .get("snapshot_format_version")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| invalid_snapshot_error("snapshot format selector is missing or invalid"))?;
    let engine = object
        .get("engine_version")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_snapshot_error("snapshot engine selector is missing or invalid"))?;
    match format {
        SNAPSHOT_FORMAT_VERSION if engine == ENGINE_VERSION => {
            deserialize_strict_json(json, &value, "format-5 snapshot")
        }
        4 if engine == LEGACY_V4_ENGINE_VERSION => {
            let legacy: LegacyV4SimulationSnapshot =
                deserialize_strict_json(json, &value, "legacy format-4 snapshot")?;
            migrate_legacy_v4(&legacy)
        }
        _ => Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "snapshot format {format} from engine {engine} is unsupported; this engine reads its own format {SNAPSHOT_FORMAT_VERSION} and engine {LEGACY_V4_ENGINE_VERSION} format 4"
            ),
        )),
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyV4SimulationCheckpoint {
    format_version: u32,
    journal_end: EvidenceCursor,
    state: LegacyV4SimulationSnapshot,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyV4EvidenceJournalSegment {
    format_version: u32,
    start: EvidenceCursor,
    end: EvidenceCursor,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    events: Vec<LegacyV4SimEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    commands: Vec<CommandRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    command_attempts: Vec<CommandAttemptRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    ingress: Vec<IngressRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    boundaries: Vec<BoundaryRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    random_draws: Vec<LegacyV4RandomDrawRecord>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyV4CheckpointJournal {
    checkpoint: LegacyV4SimulationCheckpoint,
    segments: Vec<LegacyV4EvidenceJournalSegment>,
}

fn advance_cursor(
    cursor: EvidenceCursor,
    segment: &LegacyV4EvidenceJournalSegment,
) -> Result<EvidenceCursor, CanwuError> {
    let add = |value: u64, length: usize, label: &str| {
        value
            .checked_add(u64::try_from(length).map_err(|_| {
                invalid_snapshot_error(format!("legacy {label} segment is too large"))
            })?)
            .ok_or_else(|| invalid_snapshot_error(format!("legacy {label} cursor overflow")))
    };
    Ok(EvidenceCursor {
        event_count: add(cursor.event_count, segment.events.len(), "event")?,
        command_count: add(cursor.command_count, segment.commands.len(), "command")?,
        command_attempt_count: add(
            cursor.command_attempt_count,
            segment.command_attempts.len(),
            "command-attempt",
        )?,
        ingress_count: add(cursor.ingress_count, segment.ingress.len(), "ingress")?,
        boundary_count: add(cursor.boundary_count, segment.boundaries.len(), "boundary")?,
        random_draw_count: add(
            cursor.random_draw_count,
            segment.random_draws.len(),
            "random-draw",
        )?,
    })
}

fn clear_snapshot_evidence(snapshot: &mut SimulationSnapshot) {
    snapshot.events.clear();
    snapshot.commands.clear();
    snapshot.command_attempts.clear();
    snapshot.ingress.clear();
    snapshot.boundaries.clear();
    snapshot.random_draws.clear();
}

fn migrate_legacy_checkpoint_journal(
    legacy: LegacyV4CheckpointJournal,
) -> Result<CheckpointJournal, CanwuError> {
    if legacy.checkpoint.format_version != CHECKPOINT_JOURNAL_FORMAT_VERSION {
        return invalid_snapshot("legacy checkpoint-journal format is unsupported");
    }
    let mut full = legacy.checkpoint.state.clone();
    if !full.events.is_empty()
        || !full.commands.is_empty()
        || !full.command_attempts.is_empty()
        || !full.ingress.is_empty()
        || !full.boundaries.is_empty()
        || !full.random_draws.is_empty()
    {
        return invalid_snapshot("legacy checkpoint state duplicates append-only evidence");
    }
    let mut cursor = EvidenceCursor::default();
    for segment in &legacy.segments {
        if segment.format_version != CHECKPOINT_JOURNAL_FORMAT_VERSION || segment.start != cursor {
            return invalid_snapshot("legacy checkpoint-journal segments are not contiguous");
        }
        let end = advance_cursor(cursor, segment)?;
        if end == cursor || segment.end != end {
            return invalid_snapshot("legacy checkpoint-journal segment end is invalid");
        }
        full.events.extend(segment.events.iter().cloned());
        full.commands.extend(segment.commands.iter().cloned());
        full.command_attempts
            .extend(segment.command_attempts.iter().cloned());
        full.ingress.extend(segment.ingress.iter().cloned());
        full.boundaries.extend(segment.boundaries.iter().cloned());
        full.random_draws
            .extend(segment.random_draws.iter().cloned());
        cursor = end;
    }
    if cursor != legacy.checkpoint.journal_end {
        return invalid_snapshot("legacy checkpoint-journal does not reach its declared cut");
    }

    let migrated = migrate_legacy_v4(&full)?;
    let continuation_checkpoint = Simulation::from_snapshot(migrated.clone())?.checkpoint()?;
    let mut checkpoint_state = migrated.clone();
    clear_snapshot_evidence(&mut checkpoint_state);
    let mut segments = Vec::with_capacity(legacy.segments.len());
    let mut event_at = 0usize;
    let mut command_at = 0usize;
    let mut attempt_at = 0usize;
    let mut ingress_at = 0usize;
    let mut boundary_at = 0usize;
    let mut draw_at = 0usize;
    for segment in legacy.segments {
        let event_end = event_at + segment.events.len();
        let command_end = command_at + segment.commands.len();
        let attempt_end = attempt_at + segment.command_attempts.len();
        let ingress_end = ingress_at + segment.ingress.len();
        let boundary_end = boundary_at + segment.boundaries.len();
        let draw_end = draw_at + segment.random_draws.len();
        segments.push(EvidenceJournalSegment {
            format_version: CHECKPOINT_JOURNAL_FORMAT_VERSION,
            start: segment.start,
            end: segment.end,
            events: migrated.events[event_at..event_end].to_vec(),
            commands: migrated.commands[command_at..command_end].to_vec(),
            command_attempts: migrated.command_attempts[attempt_at..attempt_end].to_vec(),
            ingress: migrated.ingress[ingress_at..ingress_end].to_vec(),
            boundaries: migrated.boundaries[boundary_at..boundary_end].to_vec(),
            random_draws: migrated.random_draws[draw_at..draw_end].to_vec(),
            archive: None,
        });
        event_at = event_end;
        command_at = command_end;
        attempt_at = attempt_end;
        ingress_at = ingress_end;
        boundary_at = boundary_end;
        draw_at = draw_end;
    }
    Ok(CheckpointJournal {
        checkpoint: SimulationCheckpoint {
            format_version: CHECKPOINT_JOURNAL_FORMAT_VERSION,
            journal_end: legacy.checkpoint.journal_end,
            state: checkpoint_state,
            archived_segment_headers: Vec::new(),
            archived_segment_manifest_root: None,
            archived_evidence_receipts: Vec::new(),
            archived_receipt_root: None,
            evidence_dependencies: continuation_checkpoint.evidence_dependencies,
            evidence_dependency_root: continuation_checkpoint.evidence_dependency_root,
            keyed_draw_reservations: Vec::new(),
            keyed_reservation_root: None,
        },
        segments,
    })
}

pub(super) fn deserialize_checkpoint_journal_json(
    json: &str,
) -> Result<CheckpointJournal, CanwuError> {
    let value: Value = serde_json::from_str(json).map_err(|error| {
        invalid_snapshot_error(format!("could not deserialize checkpoint journal: {error}"))
    })?;
    let state = value
        .get("checkpoint")
        .and_then(|value| value.get("state"))
        .and_then(Value::as_object)
        .ok_or_else(|| invalid_snapshot_error("checkpoint journal state selector is missing"))?;
    let format = state
        .get("snapshot_format_version")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| invalid_snapshot_error("checkpoint journal format selector is invalid"))?;
    let engine = state
        .get("engine_version")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_snapshot_error("checkpoint journal engine selector is invalid"))?;
    match format {
        SNAPSHOT_FORMAT_VERSION if engine == ENGINE_VERSION => {
            deserialize_strict_json(json, &value, "format-5 checkpoint journal")
        }
        4 if engine == LEGACY_V4_ENGINE_VERSION => {
            let legacy: LegacyV4CheckpointJournal =
                deserialize_strict_json(json, &value, "legacy format-4 checkpoint journal")?;
            migrate_legacy_checkpoint_journal(legacy)
        }
        _ => Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            "checkpoint journal engine or snapshot format is unsupported",
        )),
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LegacyV4ReplayJournalWire {
    pub engine_version: String,
    pub snapshot_format_version: u32,
    pub root_seed: u64,
    pub run_manifest: RunManifest,
    pub run_manifest_hash: String,
    #[serde(default)]
    pub run_configuration: Option<RunConfigurationSnapshot>,
    pub plugin_descriptors: Vec<LegacyV4PluginDescriptor>,
    pub plugin_registration_closed: bool,
    pub commands: Vec<CommandRecord>,
    #[serde(default)]
    pub command_attempts: Vec<CommandAttemptRecord>,
    #[serde(default)]
    pub ingress: Vec<IngressRecord>,
    pub boundaries: Vec<BoundaryRecord>,
    pub final_time: SimTime,
    pub checkpoint_hash: String,
    #[serde(default)]
    pub commitment_format_version: u32,
    #[serde(default)]
    pub revision_format_version: u32,
    #[serde(default)]
    pub final_revision: u64,
}

pub(super) fn validate_legacy_replay_wire(wire: &LegacyV4ReplayJournalWire) -> Result<(), String> {
    if wire.engine_version != LEGACY_V4_ENGINE_VERSION || wire.snapshot_format_version != 4 {
        return Err("legacy replay accepts only engine 0.4.0 format 4".to_owned());
    }
    if manifest::hash(&wire.run_manifest).map_err(|error| error.to_string())?
        != wire.run_manifest_hash
    {
        return Err("legacy replay manifest hash is inconsistent".to_owned());
    }
    let expected_revision = super::authoritative_revision_count(
        wire.commands.len(),
        wire.command_attempts.len(),
        wire.boundaries.len(),
    )
    .map_err(|error| error.to_string())?;
    if wire.final_revision != expected_revision {
        return Err("legacy replay final revision is inconsistent".to_owned());
    }
    validate_boundary_chain(&wire.boundaries).map_err(|error| error.to_string())
}
pub(super) fn deserialize_replay_value(value: &Value) -> Result<super::ReplayJournal, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "replay journal envelope must be an object".to_owned())?;
    let format = object
        .get("snapshot_format_version")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| "replay journal format selector is missing or invalid".to_owned())?;
    let engine = object
        .get("engine_version")
        .and_then(Value::as_str)
        .ok_or_else(|| "replay journal engine selector is missing or invalid".to_owned())?;
    match format {
        SNAPSHOT_FORMAT_VERSION if engine == ENGINE_VERSION => {
            let wire: super::persistence::ReplayJournalWire =
                deserialize_strict(value, "format-5 replay journal")
                    .map_err(|error| error.to_string())?;
            let run_configuration = wire
                .run_configuration
                .map_or_else(|| super::inferred_run_configuration(&wire.run_manifest), Ok)
                .map_err(|error| error.to_string())?;
            Ok(super::ReplayJournal {
                engine_version: wire.engine_version,
                snapshot_format_version: wire.snapshot_format_version,
                root_seed: wire.root_seed,
                run_manifest: wire.run_manifest,
                run_manifest_hash: wire.run_manifest_hash,
                run_configuration,
                plugin_descriptors: wire.plugin_descriptors,
                plugin_registration_closed: wire.plugin_registration_closed,
                commands: wire.commands,
                command_attempts: wire.command_attempts,
                ingress: wire.ingress,
                boundaries: wire.boundaries,
                final_time: wire.final_time,
                checkpoint_hash: wire.checkpoint_hash,
                commitment_format_version: wire.commitment_format_version,
                revision_format_version: wire.revision_format_version,
                final_revision: wire.final_revision,
            })
        }
        4 if engine == LEGACY_V4_ENGINE_VERSION => {
            let wire: LegacyV4ReplayJournalWire =
                deserialize_strict(value, "legacy format-4 replay journal")
                    .map_err(|error| error.to_string())?;
            validate_legacy_replay_wire(&wire)?;
            let run_configuration = wire
                .run_configuration
                .clone()
                .map_or_else(|| super::inferred_run_configuration(&wire.run_manifest), Ok)
                .map_err(|error| error.to_string())?;
            Ok(super::ReplayJournal {
                engine_version: wire.engine_version,
                snapshot_format_version: wire.snapshot_format_version,
                root_seed: wire.root_seed,
                run_manifest: wire.run_manifest,
                run_manifest_hash: wire.run_manifest_hash,
                run_configuration,
                plugin_descriptors: wire
                    .plugin_descriptors
                    .into_iter()
                    .map(Into::into)
                    .collect(),
                plugin_registration_closed: wire.plugin_registration_closed,
                commands: wire.commands,
                command_attempts: wire.command_attempts,
                ingress: wire.ingress,
                boundaries: wire.boundaries,
                final_time: wire.final_time,
                checkpoint_hash: wire.checkpoint_hash,
                commitment_format_version: wire.commitment_format_version,
                revision_format_version: 0,
                final_revision: wire.final_revision,
            })
        }
        _ => Err(format!(
            "replay journal format {format} from engine {engine} is unsupported"
        )),
    }
}
#[cfg(test)]
mod tests {
    use super::super::{Simulation, demo_scenario};
    use super::*;

    fn empty_legacy_value() -> Value {
        let (scenario, _) = demo_scenario();
        let simulation = Simulation::new(401, scenario).expect("fixture simulation should build");
        let mut value =
            serde_json::to_value(simulation.snapshot()).expect("current snapshot should serialize");
        let object = value.as_object_mut().expect("snapshot should be an object");
        object.remove("entities");
        if let Some(initial_scenario) = object
            .get_mut("initial_scenario")
            .and_then(Value::as_object_mut)
        {
            initial_scenario.remove("entities");
        }
        object.insert(
            "engine_version".to_owned(),
            Value::String(LEGACY_V4_ENGINE_VERSION.to_owned()),
        );
        object.insert("snapshot_format_version".to_owned(), Value::from(4));
        let mut legacy: LegacyV4SimulationSnapshot =
            serde_json::from_value(value).expect("empty current wire should fit legacy V4");
        let shadow = legacy.clone().into_current();
        let mut roots = snapshot_commitment_roots(&shadow).expect("roots should compute");
        roots.random = legacy_random_commitment_root(
            legacy.root_seed,
            &legacy.random_streams,
            &legacy.random_draws,
        )
        .expect("legacy random root should compute");
        roots.identity = legacy_identity_commitment_root(
            legacy.run_manifest.as_ref().expect("manifest should exist"),
            &legacy.run_manifest_hash,
            legacy.initial_time,
            legacy.initial_scenario.as_ref(),
            &legacy.plugin_descriptors,
            &legacy.schema,
        )
        .expect("legacy identity root should compute");
        legacy.commitment_roots = Some(roots.clone());
        legacy.checkpoint_hash = checkpoint_hash_for_commitments(
            &roots,
            &legacy.run_manifest_hash,
            legacy.commitment_format_version,
            legacy.revision_format_version,
            legacy.state_revision,
            legacy.replay_revision_format_version,
        )
        .expect("legacy checkpoint should compute");
        serde_json::to_value(legacy).expect("legacy snapshot should serialize")
    }

    #[test]
    fn strict_v4_snapshot_validates_before_migration_and_rejects_unknown_nested_fields() {
        let value = empty_legacy_value();
        let json = serde_json::to_string(&value).expect("fixture should serialize");
        let migrated = deserialize_snapshot_json(&json).expect("valid V4 should migrate");
        assert_eq!(migrated.engine_version, ENGINE_VERSION);
        assert_eq!(migrated.snapshot_format_version, SNAPSHOT_FORMAT_VERSION);
        Simulation::from_snapshot(migrated).expect("migrated snapshot should become live");

        let mut tampered = value;
        tampered["world"]["format_5_only"] = Value::Bool(true);
        let error = deserialize_snapshot_json(
            &serde_json::to_string(&tampered).expect("tamper should serialize"),
        )
        .expect_err("unknown nested fields must fail before migration");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert!(error.message.contains("world.format_5_only"));
    }

    #[test]
    fn legacy_replay_and_checkpoint_journal_are_validated_then_marked_historical() {
        let snapshot = empty_legacy_value();
        let snapshot_object = snapshot.as_object().expect("snapshot object");
        let replay = serde_json::json!({
            "engine_version": LEGACY_V4_ENGINE_VERSION,
            "snapshot_format_version": 4,
            "root_seed": snapshot_object["root_seed"],
            "run_manifest": snapshot_object["run_manifest"],
            "run_manifest_hash": snapshot_object["run_manifest_hash"],
            "run_configuration": snapshot_object["run_configuration"],
            "plugin_descriptors": snapshot_object["plugin_descriptors"],
            "plugin_registration_closed": snapshot_object["plugin_registration_closed"],
            "commands": [],
            "command_attempts": [],
            "ingress": [],
            "boundaries": [],
            "final_time": snapshot_object["now"],
            "checkpoint_hash": snapshot_object["checkpoint_hash"],
            "commitment_format_version": snapshot_object["commitment_format_version"],
            "revision_format_version": snapshot_object["revision_format_version"],
            "final_revision": 0
        });
        let journal: crate::ReplayJournal = serde_json::from_value(replay)
            .expect("valid legacy replay envelope should deserialize");
        assert_eq!(journal.revision_format_version, 0);

        let bundle = serde_json::json!({
            "checkpoint": {
                "format_version": CHECKPOINT_JOURNAL_FORMAT_VERSION,
                "journal_end": EvidenceCursor::default(),
                "state": snapshot
            },
            "segments": []
        });
        let migrated = deserialize_checkpoint_journal_json(
            &serde_json::to_string(&bundle).expect("bundle should serialize"),
        )
        .expect("valid legacy checkpoint journal should migrate");
        assert_eq!(
            migrated.checkpoint.state.snapshot_format_version,
            SNAPSHOT_FORMAT_VERSION
        );
    }
}
