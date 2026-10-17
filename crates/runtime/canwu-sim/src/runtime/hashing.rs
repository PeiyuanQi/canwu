use super::records::RecordChangeEncodings;
use super::{
    ArtifactManifest, BOUNDARY_STATE_HASH_V1_PREFIX, BoundaryChange, BoundaryEmission, BoundaryId,
    BoundaryIngressGeneration, BoundaryRecord, BoundaryStateHashFormat, COMMITMENT_FORMAT_VERSION,
    CanwuError, CommandAttemptId, CommandAttemptRecord, CommandId, CommandRecord, DecisionState,
    DomainRecord, DomainRecordChange, EntityRef, ErrorCode, EventId, GENESIS_BOUNDARY_HASH,
    IngressId, IngressRecord, JournalCommitmentRoots, KnowledgeSnapshot, PluginComponentRecord,
    PluginDescriptor, RandomDrawId, RandomDrawRecord, RandomStreamState, ReservationAllocation,
    ReservationOfferRecord, ReservationRequestRecord, RunConfigurationSnapshot, RunManifest,
    RuntimeDomainCommitmentRoots, STATE_REVISION_FORMAT_VERSION, Scenario, ScheduledRecord,
    SchemaRegistry, SimEvent, SimTime, SimulationSnapshot, SystemCadence, WorldSnapshot,
    boundary_state_hash_format, command_attempt_id_slice_is_empty, command_attempt_slice_is_empty,
    component_key, domain_record_slice_is_empty, ingress_record_slice_is_empty, invalid_snapshot,
    invalid_snapshot_error, is_one_u64, is_zero_u64, maintenance_change_slice_is_empty, manifest,
    policy,
};
use canwu_knowledge::KnowledgeBodyDigest;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
/// Canonical roots for independent authoritative state and evidence domains.
pub struct CommitmentRoots {
    pub world: String,
    pub knowledge: String,
    pub plugin_components: String,
    pub domain_records: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub decisions: String,
    pub scheduler: String,
    pub commands: String,
    pub events: String,
    pub ingress: String,
    pub random: String,
    pub boundary_chain: String,
    pub identity: String,
    pub control: String,
}

#[derive(Serialize)]
pub(super) struct StateHashMaterial<'a> {
    pub(super) engine_version: &'a str,
    pub(super) snapshot_format_version: u32,
    pub(super) run_manifest: &'a RunManifest,
    pub(super) run_manifest_hash: &'a str,
    pub(super) initial_time: SimTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) initial_scenario: Option<&'a Scenario>,
    pub(super) now: SimTime,
    pub(super) plugin_registration_closed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) entities: Option<&'a [EntityRef]>,
    pub(super) world: &'a WorldSnapshot,
    #[serde(skip_serializing_if = "AvailabilityTable::is_empty")]
    pub(super) person_availability: AvailabilityTable<'a>,
    #[serde(skip_serializing_if = "CreatedPersonList::is_empty")]
    pub(super) created_persons: CreatedPersonList<'a>,
    pub(super) knowledge: &'a KnowledgeSnapshot,
    pub(super) events: &'a [SimEvent],
    pub(super) commands: &'a [CommandRecord],
    #[serde(skip_serializing_if = "command_attempt_slice_is_empty")]
    pub(super) command_attempts: &'a [CommandAttemptRecord],
    #[serde(skip_serializing_if = "ingress_record_slice_is_empty")]
    pub(super) ingress: &'a [IngressRecord],
    pub(super) plugin_components: &'a [&'a PluginComponentRecord],
    #[serde(skip_serializing_if = "domain_record_slice_is_empty")]
    pub(super) domain_records: &'a [DomainRecord],
    #[serde(skip_serializing_if = "DecisionState::is_empty")]
    pub(super) decisions: &'a DecisionState,
    pub(super) plugin_descriptors: &'a [PluginDescriptor],
    pub(super) schema: &'a SchemaRegistry,
    pub(super) scheduled: &'a [ScheduledRecord],
    #[serde(skip_serializing_if = "transition_manifest_slice_is_empty")]
    pub(super) transition_manifests: &'a [&'a super::PendingTransitionManifest],
    pub(super) root_seed: u64,
    pub(super) authority_root_seed: u64,
    pub(super) random_streams: &'a [RandomStreamState],
    pub(super) random_draws: &'a [RandomDrawRecord],
    pub(super) next_event_id: u64,
    pub(super) next_command_id: u64,
    #[serde(skip_serializing_if = "is_one_u64")]
    pub(super) next_command_attempt_id: u64,
    #[serde(skip_serializing_if = "is_one_u64")]
    pub(super) next_ingress_id: u64,
    pub(super) next_boundary_id: u64,
    pub(super) next_random_draw_id: u64,
    #[serde(skip_serializing_if = "is_one_u64")]
    pub(super) next_knowledge_record_id: u64,
    pub(super) next_schedule_sequence: u64,
    pub(super) next_correlation_id: u64,
    #[serde(skip_serializing_if = "is_one_u64")]
    pub(super) next_decision_trace_id: u64,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub(super) next_person_id: u64,
}

/// The person-availability table of a hash input: a snapshot's ordered map
/// or the runtime's persistent one. Both serialize as the same ordered map,
/// so the bytes do not depend on where the table came from.
#[derive(Clone, Copy)]
pub(super) enum AvailabilityTable<'a> {
    Snapshot(&'a BTreeMap<super::PersonId, super::PersonAvailability>),
    Runtime(&'a im::OrdMap<super::PersonId, super::PersonAvailability>),
}

impl AvailabilityTable<'_> {
    fn is_empty(&self) -> bool {
        match self {
            Self::Snapshot(table) => table.is_empty(),
            Self::Runtime(table) => table.is_empty(),
        }
    }
}

impl Serialize for AvailabilityTable<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Snapshot(table) => table.serialize(serializer),
            Self::Runtime(table) => serializer.collect_map(table.iter()),
        }
    }
}

/// The created-person registry of a hash input: a snapshot's list or the
/// runtime's persistent vector, serialized as the same sequence.
#[derive(Clone, Copy)]
pub(super) enum CreatedPersonList<'a> {
    Snapshot(&'a [super::CreatedPerson]),
    Runtime(&'a im::Vector<super::CreatedPerson>),
}

impl CreatedPersonList<'_> {
    fn is_empty(&self) -> bool {
        match self {
            Self::Snapshot(list) => list.is_empty(),
            Self::Runtime(list) => list.is_empty(),
        }
    }
}

impl Serialize for CreatedPersonList<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Snapshot(list) => list.serialize(serializer),
            Self::Runtime(list) => serializer.collect_seq(list.iter()),
        }
    }
}

fn transition_manifest_slice_is_empty(value: &&[&super::PendingTransitionManifest]) -> bool {
    value.is_empty()
}

#[derive(Serialize)]
struct WorldCommitmentMaterial<'a> {
    people: String,
    governments: String,
    territories: String,
    routes: String,
    armies: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    entities: Option<&'a [EntityRef]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    person_availability: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_persons: Option<String>,
}

#[derive(Serialize)]
struct SchedulerCommitmentMaterial {
    now: SimTime,
    scheduled: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    transition_manifests: Option<String>,
}

#[derive(Serialize)]
struct CommandCommitmentMaterial {
    commands: String,
    attempts: String,
}

#[derive(Serialize)]
struct RandomCommitmentMaterial {
    root_seed: u64,
    streams: String,
    draws: String,
}

#[derive(Serialize)]
struct IdentityCommitmentMaterial<'a> {
    engine_version: &'a str,
    snapshot_format_version: u32,
    run_manifest: &'a RunManifest,
    run_manifest_hash: &'a str,
    initial_time: SimTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    initial_scenario: Option<&'a Scenario>,
    authority_root_seed: u64,
    plugin_descriptors: String,
    schema: &'a SchemaRegistry,
}

#[derive(Serialize)]
pub(super) struct ControlCommitmentMaterial {
    pub(super) plugin_registration_closed: bool,
    pub(super) next_event_id: u64,
    pub(super) next_command_id: u64,
    pub(super) next_command_attempt_id: u64,
    pub(super) next_ingress_id: u64,
    pub(super) next_boundary_id: u64,
    pub(super) next_random_draw_id: u64,
    #[serde(skip_serializing_if = "is_one_u64")]
    pub(super) next_knowledge_record_id: u64,
    pub(super) next_schedule_sequence: u64,
    pub(super) next_correlation_id: u64,
    #[serde(skip_serializing_if = "is_one_u64")]
    pub(super) next_decision_trace_id: u64,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub(super) next_person_id: u64,
}

#[derive(Serialize)]
struct CheckpointHashMaterialV1<'a> {
    state_hash: &'a str,
    boundary_head: Option<&'a str>,
}

#[derive(Serialize)]
struct CheckpointHashMaterialV2<'a> {
    state_hash: &'a str,
    boundary_head: Option<&'a str>,
    run_manifest_hash: &'a str,
}

#[derive(Serialize)]
struct CheckpointHashMaterialV3<'a> {
    state_hash: &'a str,
    boundary_head: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_manifest_hash: Option<&'a str>,
    revision_format_version: u32,
    state_revision: u64,
    replay_revision_format_version: u32,
}

#[derive(Serialize)]
struct CheckpointHashMaterialV4<'a> {
    commitments: &'a CommitmentRoots,
    run_manifest_hash: &'a str,
    commitment_format_version: u32,
    revision_format_version: u32,
    state_revision: u64,
    replay_revision_format_version: u32,
}

pub(super) fn state_hash(material: &StateHashMaterial<'_>) -> Result<String, CanwuError> {
    canonical_hash("canwu.boundary-state.v1", material)
}

fn canonical_sorted_hash_by<T, K, F>(
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

pub(super) fn committed_entities<'a>(
    entities: &'a [EntityRef],
    world: &WorldSnapshot,
) -> Option<&'a [EntityRef]> {
    (!entities.is_empty() && entities != super::scenario::legacy_entities(world))
        .then_some(entities)
}

pub(super) fn committed_initial_scenario(scenario: Option<&Scenario>) -> Option<Scenario> {
    scenario.cloned().map(|mut scenario| {
        if scenario.entities == super::scenario::legacy_entities(&scenario.world) {
            scenario.entities.clear();
        }
        scenario
    })
}

pub(super) fn world_commitment_root(
    world: &WorldSnapshot,
    entities: &[EntityRef],
    person_availability: AvailabilityTable<'_>,
    created_persons: CreatedPersonList<'_>,
) -> Result<String, CanwuError> {
    canonical_hash(
        "canwu.commitment.world.v1",
        &WorldCommitmentMaterial {
            people: canonical_sorted_hash_by(
                "canwu.commitment.world.people.v1",
                &world.people,
                |value| value.id,
            )?,
            governments: canonical_sorted_hash_by(
                "canwu.commitment.world.governments.v1",
                &world.governments,
                |value| value.id,
            )?,
            territories: canonical_sorted_hash_by(
                "canwu.commitment.world.territories.v1",
                &world.territories,
                |value| value.id,
            )?,
            routes: canonical_sorted_hash_by(
                "canwu.commitment.world.routes.v1",
                &world.routes,
                |value| value.id,
            )?,
            armies: canonical_sorted_hash_by(
                "canwu.commitment.world.armies.v1",
                &world.armies,
                |value| value.id,
            )?,
            entities: committed_entities(entities, world),
            person_availability: (!person_availability.is_empty())
                .then(|| {
                    canonical_hash(
                        "canwu.commitment.world.person-availability.v1",
                        &person_availability,
                    )
                })
                .transpose()?,
            created_persons: (!created_persons.is_empty())
                .then(|| {
                    canonical_hash(
                        "canwu.commitment.world.created-persons.v1",
                        &created_persons,
                    )
                })
                .transpose()?,
        },
    )
}

#[derive(Serialize)]
struct KnowledgeCommitmentMaterial<'a> {
    actors: &'a str,
    ledger: &'a str,
}

/// The knowledge root of commitment format 6 from the actor sub-root and the
/// generic ledger's chunked root.
///
/// Format 5 kept a running hash of every record in global ID order, which
/// assumed records never change or leave; retention retires entries, so
/// format 6 commits to the ledger through cached leaf, chunk, and holder
/// roots instead, and a publication or retirement rehashes what it touched.
pub(super) fn knowledge_commitment_root_from_parts(
    actors: &str,
    ledger: &super::HolderLedgers,
) -> Result<String, CanwuError> {
    let ledger = blake3::Hash::from_bytes(ledger.commitment_root()).to_hex();
    canonical_hash(
        "canwu.commitment.knowledge.v3",
        &KnowledgeCommitmentMaterial {
            actors,
            ledger: ledger.as_str(),
        },
    )
}

/// The actor sub-root of the knowledge root.
pub(super) fn knowledge_actor_commitment_root(
    actors: &BTreeMap<super::PersonId, super::ActorKnowledge>,
) -> Result<String, CanwuError> {
    canonical_hash("canwu.commitment.knowledge.actors.v1", actors)
}

/// The knowledge root of a whole snapshot. Load validation uses it; a decoded
/// ledger computed every root from its entries, and the runtime, which keeps
/// them cached, must reach the same value.
pub(super) fn knowledge_commitment_root(
    knowledge: &KnowledgeSnapshot,
) -> Result<String, CanwuError> {
    knowledge_commitment_root_from_parts(
        &knowledge_actor_commitment_root(&knowledge.actors)?,
        &knowledge.records,
    )
}

pub(super) fn plugin_component_commitment_root(
    components: &[&PluginComponentRecord],
) -> Result<String, CanwuError> {
    canonical_sorted_hash_by(
        "canwu.commitment.plugin-components.v1",
        components,
        |record| {
            component_key(
                &record.plugin,
                &record.state,
                &record.entity,
                &record.component,
            )
        },
    )
}

pub(super) fn domain_record_commitment_root(
    records: &[DomainRecord],
) -> Result<String, CanwuError> {
    let records = records
        .iter()
        .cloned()
        .map(|record| (record.reference.clone(), record))
        .collect();
    super::PersistentDomainRecordStore::from_records(records)?.commitment_root()
}

pub(super) fn decision_commitment_root(decisions: &DecisionState) -> Result<String, CanwuError> {
    if decisions.is_empty() {
        return Ok(String::new());
    }
    decisions
        .authoritative_commitment()
        .map_err(super::decision::decision_error)
}

/// Pending transition manifests are committed in manifest-ID order under an
/// optional sub-root, so runs without manifests keep their scheduler root.
pub(super) fn scheduler_commitment_root(
    now: SimTime,
    scheduled: &[ScheduledRecord],
    transition_manifests: &[&super::PendingTransitionManifest],
) -> Result<String, CanwuError> {
    canonical_hash(
        "canwu.commitment.scheduler.v1",
        &SchedulerCommitmentMaterial {
            now,
            scheduled: canonical_sorted_hash_by(
                "canwu.commitment.scheduler.entries.v1",
                scheduled,
                |record| record.key.clone(),
            )?,
            transition_manifests: (!transition_manifests.is_empty())
                .then(|| {
                    canonical_hash(
                        "canwu.commitment.scheduler.transition-manifests.v1",
                        transition_manifests,
                    )
                })
                .transpose()?,
        },
    )
}

pub(super) fn random_stream_commitment_root(
    streams: &[RandomStreamState],
) -> Result<String, CanwuError> {
    canonical_sorted_hash_by("canwu.commitment.random.streams.v1", streams, |stream| {
        stream.key.clone()
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn identity_commitment_root(
    engine_version: &str,
    snapshot_format_version: u32,
    run_manifest: &RunManifest,
    run_manifest_hash: &str,
    initial_time: SimTime,
    initial_scenario: Option<&Scenario>,
    authority_root_seed: u64,
    plugin_descriptors: &[PluginDescriptor],
    schema: &SchemaRegistry,
) -> Result<String, CanwuError> {
    canonical_hash(
        "canwu.commitment.identity.v1",
        &IdentityCommitmentMaterial {
            engine_version,
            snapshot_format_version,
            run_manifest,
            run_manifest_hash,
            initial_time,
            initial_scenario,
            authority_root_seed,
            plugin_descriptors: canonical_sorted_hash_by(
                "canwu.commitment.identity.plugins.v1",
                plugin_descriptors,
                |descriptor| descriptor.name.clone(),
            )?,
            schema,
        },
    )
}

fn commitment_roots(
    material: &StateHashMaterial<'_>,
    boundary_head: Option<&str>,
    journal_roots: Option<&JournalCommitmentRoots>,
) -> Result<CommitmentRoots, CanwuError> {
    let world = world_commitment_root(
        material.world,
        material.entities.unwrap_or_default(),
        material.person_availability,
        material.created_persons,
    )?;
    let knowledge = knowledge_commitment_root(material.knowledge)?;
    let plugin_components = plugin_component_commitment_root(material.plugin_components)?;
    let domain_records = domain_record_commitment_root(material.domain_records)?;
    let decisions = decision_commitment_root(material.decisions)?;
    let scheduler = scheduler_commitment_root(
        material.now,
        material.scheduled,
        material.transition_manifests,
    )?;
    let command_root = match journal_roots {
        Some(roots) => roots.commands.clone(),
        None => canonical_sorted_hash_by(
            "canwu.commitment.commands.accepted.v1",
            material.commands,
            |record| record.id,
        )?,
    };
    let attempt_root = match journal_roots {
        Some(roots) => roots.attempts.clone(),
        None => canonical_sorted_hash_by(
            "canwu.commitment.commands.attempts.v1",
            material.command_attempts,
            |record| record.id,
        )?,
    };
    let commands = canonical_hash(
        "canwu.commitment.commands.v1",
        &CommandCommitmentMaterial {
            commands: command_root,
            attempts: attempt_root,
        },
    )?;
    let events = match journal_roots {
        Some(roots) => roots.events.clone(),
        None => canonical_sorted_hash_by("canwu.commitment.events.v1", material.events, |event| {
            event.id
        })?,
    };
    let ingress = match journal_roots {
        Some(roots) => roots.ingress.clone(),
        None => {
            canonical_sorted_hash_by("canwu.commitment.ingress.v1", material.ingress, |record| {
                record.id
            })?
        }
    };
    let random = canonical_hash(
        "canwu.commitment.random.v1",
        &RandomCommitmentMaterial {
            root_seed: material.root_seed,
            streams: random_stream_commitment_root(material.random_streams)?,
            draws: match journal_roots {
                Some(roots) => roots.random_draws.clone(),
                None => canonical_sorted_hash_by(
                    "canwu.commitment.random.draws.v1",
                    material.random_draws,
                    |draw| draw.id,
                )?,
            },
        },
    )?;
    let boundary_chain = canonical_hash(
        "canwu.commitment.boundary-chain.v1",
        boundary_head.unwrap_or(GENESIS_BOUNDARY_HASH),
    )?;
    let identity = identity_commitment_root(
        material.engine_version,
        material.snapshot_format_version,
        material.run_manifest,
        material.run_manifest_hash,
        material.initial_time,
        material.initial_scenario,
        material.authority_root_seed,
        material.plugin_descriptors,
        material.schema,
    )?;
    let control = canonical_hash(
        "canwu.commitment.control.v1",
        &ControlCommitmentMaterial {
            plugin_registration_closed: material.plugin_registration_closed,
            next_event_id: material.next_event_id,
            next_command_id: material.next_command_id,
            next_command_attempt_id: material.next_command_attempt_id,
            next_ingress_id: material.next_ingress_id,
            next_boundary_id: material.next_boundary_id,
            next_random_draw_id: material.next_random_draw_id,
            next_knowledge_record_id: material.next_knowledge_record_id,
            next_schedule_sequence: material.next_schedule_sequence,
            next_correlation_id: material.next_correlation_id,
            next_decision_trace_id: material.next_decision_trace_id,
            next_person_id: material.next_person_id,
        },
    )?;
    Ok(CommitmentRoots {
        world,
        knowledge,
        plugin_components,
        domain_records,
        decisions,
        scheduler,
        commands,
        events,
        ingress,
        random,
        boundary_chain,
        identity,
        control,
    })
}

pub(super) fn runtime_commitment_roots(
    domain: &RuntimeDomainCommitmentRoots,
    journal: &JournalCommitmentRoots,
    root_seed: u64,
    boundary_head: Option<&str>,
    control: &ControlCommitmentMaterial,
) -> Result<CommitmentRoots, CanwuError> {
    let commands = canonical_hash(
        "canwu.commitment.commands.v1",
        &CommandCommitmentMaterial {
            commands: journal.commands.clone(),
            attempts: journal.attempts.clone(),
        },
    )?;
    let random = canonical_hash(
        "canwu.commitment.random.v1",
        &RandomCommitmentMaterial {
            root_seed,
            streams: domain.random_streams.clone(),
            draws: journal.random_draws.clone(),
        },
    )?;
    Ok(CommitmentRoots {
        world: domain.world.clone(),
        knowledge: domain.knowledge.clone(),
        plugin_components: domain.plugin_components.clone(),
        domain_records: domain.domain_records.clone(),
        decisions: domain.decisions.clone(),
        scheduler: domain.scheduler.clone(),
        commands,
        events: journal.events.clone(),
        ingress: journal.ingress.clone(),
        random,
        boundary_chain: canonical_hash(
            "canwu.commitment.boundary-chain.v1",
            boundary_head.unwrap_or(GENESIS_BOUNDARY_HASH),
        )?,
        identity: domain.identity.clone(),
        control: canonical_hash("canwu.commitment.control.v1", control)?,
    })
}

pub(super) fn boundary_state_hash_for_commitments(
    commitments: &CommitmentRoots,
) -> Result<String, CanwuError> {
    Ok(format!(
        "{BOUNDARY_STATE_HASH_V1_PREFIX}{}",
        canonical_hash("canwu.boundary-state.v1", commitments)?
    ))
}

pub(super) fn authoritative_run_identity(
    run_manifest: &RunManifest,
    run_manifest_hash: &str,
    run_configuration: &RunConfigurationSnapshot,
) -> Result<(RunManifest, String), CanwuError> {
    if !matches!(run_configuration, RunConfigurationSnapshot::Declared(_)) {
        return Ok((run_manifest.clone(), run_manifest_hash.to_owned()));
    }
    let mut authoritative_manifest = run_manifest.clone();
    let RunManifest::Declared {
        run_configuration, ..
    } = &mut authoritative_manifest;
    **run_configuration = ArtifactManifest::new(
        "canwu.core",
        "authoritative-policy-excluded",
        "1",
        policy::authoritative_configuration_hash()?,
    )?;
    let authoritative_manifest_hash = manifest::hash(&authoritative_manifest)?;
    Ok((authoritative_manifest, authoritative_manifest_hash))
}

pub(super) fn snapshot_state_hash(snapshot: &SimulationSnapshot) -> Result<String, CanwuError> {
    let Some(run_manifest) = &snapshot.run_manifest else {
        return Err(CanwuError::new(
            ErrorCode::InvalidRunManifest,
            "snapshot is missing its run manifest",
        ));
    };
    let run_configuration = snapshot.run_configuration.as_ref().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidRunConfiguration,
            "snapshot is missing its run configuration",
        )
    })?;
    let (authoritative_manifest, authoritative_manifest_hash) =
        authoritative_run_identity(run_manifest, &snapshot.run_manifest_hash, run_configuration)?;
    let initial_scenario = committed_initial_scenario(snapshot.initial_scenario.as_ref());
    let transition_manifests: Vec<_> = snapshot.pending_transition_manifests.iter().collect();
    let plugin_components: Vec<_> = snapshot.plugin_components.iter().collect();
    state_hash(&StateHashMaterial {
        engine_version: &snapshot.engine_version,
        snapshot_format_version: snapshot.snapshot_format_version,
        run_manifest: &authoritative_manifest,
        run_manifest_hash: &authoritative_manifest_hash,
        initial_time: snapshot.initial_time,
        initial_scenario: initial_scenario.as_ref(),
        now: snapshot.now,
        plugin_registration_closed: snapshot.plugin_registration_closed,
        entities: committed_entities(&snapshot.entities, &snapshot.world),
        world: &snapshot.world,
        person_availability: AvailabilityTable::Snapshot(&snapshot.person_availability),
        created_persons: CreatedPersonList::Snapshot(&snapshot.created_persons),
        knowledge: &snapshot.knowledge,
        events: &snapshot.events,
        commands: &snapshot.commands,
        command_attempts: &snapshot.command_attempts,
        ingress: &snapshot.ingress,
        plugin_components: &plugin_components,
        domain_records: &snapshot.domain_records,
        decisions: &snapshot.decisions,
        plugin_descriptors: &snapshot.plugin_descriptors,
        schema: &snapshot.schema,
        scheduled: &snapshot.scheduled,
        transition_manifests: &transition_manifests,
        root_seed: snapshot.root_seed,
        authority_root_seed: snapshot.authority_root_seed,
        random_streams: &snapshot.random_streams,
        random_draws: &snapshot.random_draws,
        next_event_id: snapshot.next_event_id,
        next_command_id: snapshot.next_command_id,
        next_command_attempt_id: snapshot.next_command_attempt_id,
        next_ingress_id: snapshot.next_ingress_id,
        next_boundary_id: snapshot.next_boundary_id,
        next_random_draw_id: snapshot.next_random_draw_id,
        next_knowledge_record_id: snapshot.next_knowledge_record_id,
        next_schedule_sequence: snapshot.next_schedule_sequence,
        next_correlation_id: snapshot.next_correlation_id,
        next_decision_trace_id: snapshot.next_decision_trace_id,
        next_person_id: snapshot.next_person_id,
    })
}

pub(super) fn snapshot_boundary_head_state_hash(
    snapshot: &SimulationSnapshot,
) -> Result<String, CanwuError> {
    let boundary = snapshot
        .boundaries
        .last()
        .ok_or_else(|| invalid_snapshot_error("snapshot has no boundary head commitment"))?;
    match boundary_state_hash_format(boundary.state_hash.as_deref())? {
        BoundaryStateHashFormat::LegacyV0 => snapshot_state_hash(snapshot),
        BoundaryStateHashFormat::CommitmentsV1 => {
            if snapshot.commitment_format_version != COMMITMENT_FORMAT_VERSION {
                return invalid_snapshot(
                    "boundary state commitment v1 requires current domain commitments",
                );
            }
            let mut roots = snapshot.commitment_roots.clone().ok_or_else(|| {
                invalid_snapshot_error(
                    "boundary state commitment v1 is missing current domain commitments",
                )
            })?;
            roots.boundary_chain = canonical_hash(
                "canwu.commitment.boundary-chain.v1",
                boundary.previous_hash.as_str(),
            )?;
            boundary_state_hash_for_commitments(&roots)
        }
    }
}

pub(super) fn snapshot_commitment_roots(
    snapshot: &SimulationSnapshot,
) -> Result<CommitmentRoots, CanwuError> {
    let Some(run_manifest) = &snapshot.run_manifest else {
        return Err(CanwuError::new(
            ErrorCode::InvalidRunManifest,
            "snapshot is missing its run manifest",
        ));
    };
    let run_configuration = snapshot.run_configuration.as_ref().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidRunConfiguration,
            "snapshot is missing its run configuration",
        )
    })?;
    let (authoritative_manifest, authoritative_manifest_hash) =
        authoritative_run_identity(run_manifest, &snapshot.run_manifest_hash, run_configuration)?;
    let initial_scenario = committed_initial_scenario(snapshot.initial_scenario.as_ref());
    let transition_manifests: Vec<_> = snapshot.pending_transition_manifests.iter().collect();
    let plugin_components: Vec<_> = snapshot.plugin_components.iter().collect();
    commitment_roots(
        &StateHashMaterial {
            engine_version: &snapshot.engine_version,
            snapshot_format_version: snapshot.snapshot_format_version,
            run_manifest: &authoritative_manifest,
            run_manifest_hash: &authoritative_manifest_hash,
            initial_time: snapshot.initial_time,
            initial_scenario: initial_scenario.as_ref(),
            now: snapshot.now,
            plugin_registration_closed: snapshot.plugin_registration_closed,
            entities: committed_entities(&snapshot.entities, &snapshot.world),
            world: &snapshot.world,
            person_availability: AvailabilityTable::Snapshot(&snapshot.person_availability),
            created_persons: CreatedPersonList::Snapshot(&snapshot.created_persons),
            knowledge: &snapshot.knowledge,
            events: &snapshot.events,
            commands: &snapshot.commands,
            command_attempts: &snapshot.command_attempts,
            ingress: &snapshot.ingress,
            plugin_components: &plugin_components,
            domain_records: &snapshot.domain_records,
            decisions: &snapshot.decisions,
            plugin_descriptors: &snapshot.plugin_descriptors,
            schema: &snapshot.schema,
            scheduled: &snapshot.scheduled,
            transition_manifests: &transition_manifests,
            root_seed: snapshot.root_seed,
            authority_root_seed: snapshot.authority_root_seed,
            random_streams: &snapshot.random_streams,
            random_draws: &snapshot.random_draws,
            next_event_id: snapshot.next_event_id,
            next_command_id: snapshot.next_command_id,
            next_command_attempt_id: snapshot.next_command_attempt_id,
            next_ingress_id: snapshot.next_ingress_id,
            next_boundary_id: snapshot.next_boundary_id,
            next_random_draw_id: snapshot.next_random_draw_id,
            next_knowledge_record_id: snapshot.next_knowledge_record_id,
            next_schedule_sequence: snapshot.next_schedule_sequence,
            next_correlation_id: snapshot.next_correlation_id,
            next_decision_trace_id: snapshot.next_decision_trace_id,
            next_person_id: snapshot.next_person_id,
        },
        snapshot
            .boundaries
            .last()
            .map(|record| record.hash.as_str()),
        None,
    )
}

fn checkpoint_hash(state_hash: &str, boundary_head: Option<&str>) -> Result<String, CanwuError> {
    canonical_hash(
        "canwu.checkpoint.v1",
        &CheckpointHashMaterialV1 {
            state_hash,
            boundary_head,
        },
    )
}

pub(super) fn checkpoint_hash_for_configuration(
    state_hash: &str,
    boundary_head: Option<&str>,
    run_manifest_hash: &str,
    run_configuration: &RunConfigurationSnapshot,
    revision_format_version: u32,
    state_revision: u64,
    replay_revision_format_version: u32,
) -> Result<String, CanwuError> {
    if revision_format_version == STATE_REVISION_FORMAT_VERSION {
        return canonical_hash(
            "canwu.checkpoint.v3",
            &CheckpointHashMaterialV3 {
                state_hash,
                boundary_head,
                run_manifest_hash: matches!(
                    run_configuration,
                    RunConfigurationSnapshot::Declared(_)
                )
                .then_some(run_manifest_hash),
                revision_format_version,
                state_revision,
                replay_revision_format_version,
            },
        );
    }
    if revision_format_version != 0 || state_revision != 0 || replay_revision_format_version != 0 {
        return Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "state revision format {revision_format_version} is unsupported; this engine writes format {STATE_REVISION_FORMAT_VERSION}"
            ),
        ));
    }
    if !matches!(run_configuration, RunConfigurationSnapshot::Declared(_)) {
        return checkpoint_hash(state_hash, boundary_head);
    }
    canonical_hash(
        "canwu.checkpoint.v2",
        &CheckpointHashMaterialV2 {
            state_hash,
            boundary_head,
            run_manifest_hash,
        },
    )
}

pub(super) fn checkpoint_hash_for_commitments(
    commitments: &CommitmentRoots,
    run_manifest_hash: &str,
    commitment_format_version: u32,
    revision_format_version: u32,
    state_revision: u64,
    replay_revision_format_version: u32,
) -> Result<String, CanwuError> {
    if commitment_format_version != COMMITMENT_FORMAT_VERSION {
        return Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "commitment format {commitment_format_version} is unsupported; this engine writes format {COMMITMENT_FORMAT_VERSION}"
            ),
        ));
    }
    if revision_format_version != STATE_REVISION_FORMAT_VERSION {
        return Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "commitment format {commitment_format_version} requires state revision format {STATE_REVISION_FORMAT_VERSION}"
            ),
        ));
    }
    canonical_hash(
        "canwu.checkpoint.v4",
        &CheckpointHashMaterialV4 {
            commitments,
            run_manifest_hash,
            commitment_format_version,
            revision_format_version,
            state_revision,
            replay_revision_format_version,
        },
    )
}

pub(super) fn snapshot_checkpoint_hash(
    snapshot: &SimulationSnapshot,
) -> Result<String, CanwuError> {
    match snapshot.commitment_format_version {
        COMMITMENT_FORMAT_VERSION => checkpoint_hash_for_commitments(
            snapshot.commitment_roots.as_ref().ok_or_else(|| {
                invalid_snapshot_error("current commitment snapshot is missing its domain roots")
            })?,
            &snapshot.run_manifest_hash,
            snapshot.commitment_format_version,
            snapshot.revision_format_version,
            snapshot.state_revision,
            snapshot.replay_revision_format_version,
        ),
        0 => {
            if snapshot.commitment_roots.is_some() {
                return invalid_snapshot(
                    "legacy commitment snapshot cannot contain current domain roots",
                );
            }
            let state_hash = snapshot_state_hash(snapshot)?;
            checkpoint_hash_for_configuration(
                &state_hash,
                snapshot
                    .boundaries
                    .last()
                    .map(|record| record.hash.as_str()),
                &snapshot.run_manifest_hash,
                snapshot.run_configuration.as_ref().ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidRunConfiguration,
                        "snapshot is missing its run configuration",
                    )
                })?,
                snapshot.revision_format_version,
                snapshot.state_revision,
                snapshot.replay_revision_format_version,
            )
        }
        version => Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "commitment format {version} is unsupported; this engine reads legacy format 0 and current format {COMMITMENT_FORMAT_VERSION}"
            ),
        )),
    }
}

pub(super) fn snapshot_is_at_boundary_head(snapshot: &SimulationSnapshot) -> bool {
    let Some(last) = snapshot.boundaries.last() else {
        return false;
    };
    if last.at != snapshot.now {
        return false;
    }
    let admitted_attempts: BTreeSet<_> = snapshot
        .boundaries
        .iter()
        .flat_map(|record| record.admitted_attempts.iter().copied())
        .collect();
    if admitted_attempts.len() != snapshot.command_attempts.len() {
        return false;
    }
    let admitted_commands: BTreeSet<_> = snapshot
        .boundaries
        .iter()
        .flat_map(|record| record.admitted_commands.iter().copied())
        .collect();
    if admitted_commands.len() != snapshot.commands.len() {
        return false;
    }
    let mut settled_ingress: BTreeSet<_> = snapshot
        .boundaries
        .iter()
        .flat_map(|record| record.admitted_ingress.iter().copied())
        .collect();
    let boundary_count = u64::try_from(snapshot.boundaries.len()).unwrap_or(u64::MAX);
    for record in &snapshot.ingress {
        if let super::IngressPayload::PluginCancellation { cancelled, .. } = &record.payload {
            // A host cancellation issued after the head boundary changes the
            // journal past that boundary's recorded state hash.
            if record.eligible_boundary_count >= boundary_count
                && !matches!(record.cause, Some(super::CauseRef::Boundary(_)))
            {
                return false;
            }
            settled_ingress.insert(record.id);
            settled_ingress.insert(*cancelled);
        }
    }
    if settled_ingress.len() != snapshot.ingress.len() {
        return false;
    }
    let accounted_events: BTreeSet<_> = snapshot
        .boundaries
        .iter()
        .flat_map(|record| {
            record
                .admitted_events
                .iter()
                .copied()
                .chain(record.emissions.iter().map(|emission| emission.event))
        })
        .collect();
    accounted_events.len() == snapshot.events.len()
}

/// The boundary hash. A record change enters it through the encoding digests
/// of the two versions it names (see [`RecordChangeCommitments`]);
/// `record_change_encodings`, aligned with `record.record_changes`, supplies
/// the encodings the record store already computed, and `None` encodes the
/// versions afresh. Both give the same hash.
#[cfg(test)]
pub(super) fn compute_boundary_hash(
    record: &BoundaryRecord,
    record_change_encodings: Option<&[RecordChangeEncodings]>,
) -> Result<String, CanwuError> {
    compute_boundary_hash_with(record, record_change_encodings, None)
}

/// The boundary hash, with the body digests of the boundary's knowledge
/// changes when the caller has them (aligned with `record.knowledge_changes`
/// and their bodies); `None` encodes the bodies afresh. Both give the same
/// hash.
pub(super) fn compute_boundary_hash_with(
    record: &BoundaryRecord,
    record_change_encodings: Option<&[RecordChangeEncodings]>,
    knowledge_digests: Option<&[Vec<KnowledgeBodyDigest>]>,
) -> Result<String, CanwuError> {
    #[derive(Serialize)]
    struct BoundaryHashMaterial<'a> {
        id: BoundaryId,
        at: SimTime,
        correlation_id: u64,
        cadences: &'a [SystemCadence],
        #[serde(skip_serializing_if = "command_attempt_id_slice_is_empty")]
        admitted_attempts: &'a [CommandAttemptId],
        admitted_commands: &'a [CommandId],
        #[serde(skip_serializing_if = "Option::is_none")]
        admitted_ingress: Option<&'a [IngressId]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        generated_ingress: Option<&'a [BoundaryIngressGeneration]>,
        admitted_events: &'a [EventId],
        reservation_offers: &'a [ReservationOfferRecord],
        reservation_requests: &'a [ReservationRequestRecord],
        allocations: &'a [ReservationAllocation],
        random_draws: &'a [RandomDrawId],
        changes: &'a [BoundaryChange],
        #[serde(skip_serializing_if = "RecordChangeCommitments::is_empty")]
        record_changes: RecordChangeCommitments<'a>,
        // Every field of every knowledge change, including the visibility,
        // producer correlation, and supersession mode that load validation
        // and the holder views read, with each body as its ID and digest.
        // Omitted while empty, so the material of a boundary without
        // knowledge changes is unchanged.
        #[serde(skip_serializing_if = "Option::is_none")]
        knowledge_changes: Option<KnowledgeChangeCommitments<'a>>,
        #[serde(skip_serializing_if = "maintenance_change_slice_is_empty")]
        maintenance_changes: &'a [super::MaintenanceChangeRecord],
        #[serde(skip_serializing_if = "Option::is_none")]
        maintenance_terminal_root: &'a Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        person_availability_changes: Option<&'a [super::BoundaryPersonAvailabilityChange]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        created_persons: Option<&'a [super::BoundaryPersonCreation]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        evaluation_traces: Option<&'a [super::BoundaryEvaluationTrace]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        transition_manifests: Option<&'a [super::PendingTransitionManifest]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        transition_audits: Option<&'a [super::TransitionAuditRecord]>,
        emissions: &'a [BoundaryEmission],
        state_hash: &'a Option<String>,
        previous_hash: &'a str,
    }

    let computed;
    let knowledge_digests = match knowledge_digests {
        Some(digests)
            if digests.len() == record.knowledge_changes.len()
                && digests
                    .iter()
                    .zip(&record.knowledge_changes)
                    .all(|(digests, change)| digests.len() == change.records.len()) =>
        {
            digests
        }
        _ => {
            computed = knowledge_change_digests(&record.knowledge_changes)?;
            computed.as_slice()
        }
    };
    let encoded;
    let record_change_encodings = match record_change_encodings {
        Some(encodings) if encodings.len() == record.record_changes.len() => encodings,
        _ => {
            encoded = record
                .record_changes
                .iter()
                .map(RecordChangeEncodings::of)
                .collect::<Result<Vec<_>, _>>()?;
            encoded.as_slice()
        }
    };
    canonical_hash(
        "canwu.boundary-record.v1",
        &BoundaryHashMaterial {
            id: record.id,
            at: record.at,
            correlation_id: record.correlation_id,
            cadences: &record.cadences,
            admitted_attempts: &record.admitted_attempts,
            admitted_commands: &record.admitted_commands,
            admitted_ingress: (!record.admitted_ingress.is_empty())
                .then_some(record.admitted_ingress.as_slice()),
            generated_ingress: (!record.generated_ingress.is_empty())
                .then_some(record.generated_ingress.as_slice()),
            admitted_events: &record.admitted_events,
            reservation_offers: &record.reservation_offers,
            reservation_requests: &record.reservation_requests,
            allocations: &record.allocations,
            random_draws: &record.random_draws,
            changes: &record.changes,
            record_changes: RecordChangeCommitments {
                changes: &record.record_changes,
                encodings: record_change_encodings,
            },
            knowledge_changes: (!record.knowledge_changes.is_empty()).then_some(
                KnowledgeChangeCommitments {
                    changes: &record.knowledge_changes,
                    digests: knowledge_digests,
                },
            ),
            maintenance_changes: &record.maintenance_changes,
            maintenance_terminal_root: &record.maintenance_terminal_root,
            person_availability_changes: (!record.person_availability_changes.is_empty())
                .then_some(record.person_availability_changes.as_slice()),
            created_persons: (!record.created_persons.is_empty())
                .then_some(record.created_persons.as_slice()),
            evaluation_traces: (!record.evaluation_traces.is_empty())
                .then_some(record.evaluation_traces.as_slice()),
            transition_manifests: (!record.transition_manifests.is_empty())
                .then_some(record.transition_manifests.as_slice()),
            transition_audits: (!record.transition_audits.is_empty())
                .then_some(record.transition_audits.as_slice()),
            emissions: &record.emissions,
            state_hash: &record.state_hash,
            previous_hash: &record.previous_hash,
        },
    )
}

/// The body digests of a boundary's knowledge changes, encoding each body.
pub(super) fn knowledge_change_digests(
    changes: &[super::BoundaryKnowledgeChange],
) -> Result<Vec<Vec<KnowledgeBodyDigest>>, CanwuError> {
    changes
        .iter()
        .map(|change| {
            change
                .records
                .iter()
                .map(|body| KnowledgeBodyDigest::of(body).map_err(CanwuError::from))
                .collect()
        })
        .collect()
}

/// The knowledge changes of a boundary as its hash commits to them: every
/// field of each change, each body replaced by its ID and digest.
struct KnowledgeChangeCommitments<'a> {
    changes: &'a [super::BoundaryKnowledgeChange],
    digests: &'a [Vec<KnowledgeBodyDigest>],
}

impl Serialize for KnowledgeChangeCommitments<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        #[derive(Serialize)]
        struct BodyCommitment<'a> {
            id: super::KnowledgeRecordId,
            digest: &'a KnowledgeBodyDigest,
        }

        struct Bodies<'a>(
            &'a [Arc<super::KnowledgeRecordBody>],
            &'a [KnowledgeBodyDigest],
        );

        impl Serialize for Bodies<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_seq(self.0.iter().zip(self.1).map(|(body, digest)| {
                    BodyCommitment {
                        id: body.id,
                        digest,
                    }
                }))
            }
        }

        struct Change<'a>(
            &'a super::BoundaryKnowledgeChange,
            &'a [KnowledgeBodyDigest],
        );

        impl Serialize for Change<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let change = self.0;
                let mut state = serializer.serialize_struct("BoundaryKnowledgeChange", 9)?;
                state.serialize_field("plugin", &change.plugin)?;
                state.serialize_field("system", &change.system)?;
                state.serialize_field("phase", &change.phase)?;
                state.serialize_field("producer_correlation", &change.producer_correlation)?;
                state.serialize_field("visibility", &change.visibility)?;
                state.serialize_field("summary", &change.summary)?;
                state.serialize_field("supersession", &change.supersession)?;
                state.serialize_field("records", &Bodies(&change.records, self.1))?;
                state.serialize_field("holders", &change.holders)?;
                state.end()
            }
        }

        serializer.collect_seq(
            self.changes
                .iter()
                .zip(self.digests)
                .map(|(change, digests)| Change(change, digests)),
        )
    }
}

/// The record changes of a boundary as its hash commits to them: each change
/// with its fields in declaration order, except that `previous` and
/// `current` are the lowercase hex encoding digests
/// (`canonical_byte_hash("canwu.domain-record.encoding.v1", canonical JSON)`)
/// of the versions it names instead of the versions themselves. The digests
/// bind every byte of both versions, so the commitment means what the
/// version bodies meant, but a boundary hashes 64 characters per version
/// instead of serializing its whole value twice.
pub(super) struct RecordChangeCommitments<'a> {
    pub(super) changes: &'a [DomainRecordChange],
    pub(super) encodings: &'a [RecordChangeEncodings],
}

impl RecordChangeCommitments<'_> {
    fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

impl Serialize for RecordChangeCommitments<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeSeq, SerializeStruct};

        struct Change<'a> {
            change: &'a DomainRecordChange,
            encodings: &'a RecordChangeEncodings,
        }

        impl Serialize for Change<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut state = serializer.serialize_struct("DomainRecordChange", 7)?;
                state.serialize_field("plugin", &self.change.plugin)?;
                state.serialize_field("system", &self.change.system)?;
                state.serialize_field("operation", &self.change.operation)?;
                state.serialize_field(
                    "previous",
                    &self
                        .encodings
                        .previous
                        .as_ref()
                        .map(|encoding| DigestHex(encoding.digest())),
                )?;
                state.serialize_field("current", &DigestHex(self.encodings.current.digest()))?;
                state.serialize_field("visibility", &self.change.visibility)?;
                state.serialize_field("summary", &self.change.summary)?;
                state.end()
            }
        }

        let mut sequence = serializer.serialize_seq(Some(self.changes.len()))?;
        for (change, encodings) in self.changes.iter().zip(self.encodings) {
            sequence.serialize_element(&Change { change, encodings })?;
        }
        sequence.end()
    }
}

/// A 32-byte digest serialized as lowercase hex without a heap allocation.
struct DigestHex([u8; 32]);

impl Serialize for DigestHex {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut text = [0_u8; 64];
        for (index, byte) in self.0.iter().enumerate() {
            text[index * 2] = DIGITS[usize::from(byte >> 4)];
            text[index * 2 + 1] = DIGITS[usize::from(byte & 0x0f)];
        }
        serializer.serialize_str(std::str::from_utf8(&text).expect("hex digits are ASCII"))
    }
}

/// Computes the engine's canonical JSON commitment for plugin-owned data.
///
/// The caller supplies a stable, versioned domain string. This is the same
/// deterministic encoding and domain separation used by the runtime's own
/// commitments, allowing extension operation identifiers to remain replay
/// compatible with kernel validation.
pub fn canonical_hash<T: Serialize + ?Sized>(
    domain: &str,
    value: &T,
) -> Result<String, CanwuError> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain.as_bytes());
    hasher.update(&[0]);
    // The JSON streams into the hash through a stack buffer, so a commitment
    // over a large value does not first copy the whole encoding into memory.
    // The hashed bytes are exactly those `serde_json::to_vec` would produce.
    let mut writer = HashWriter {
        hasher,
        buffer: [0; HASH_WRITER_BUFFER_BYTES],
        len: 0,
    };
    serde_json::to_writer(&mut writer, value).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidSnapshot,
            format!("could not encode deterministic hash material: {error}"),
        )
    })?;
    Ok(writer.finish().to_hex().to_string())
}

const HASH_WRITER_BUFFER_BYTES: usize = 4_096;

/// Feeds serialized bytes into a BLAKE3 hasher in buffer-sized updates.
struct HashWriter {
    hasher: blake3::Hasher,
    buffer: [u8; HASH_WRITER_BUFFER_BYTES],
    len: usize,
}

impl HashWriter {
    fn finish(mut self) -> blake3::Hash {
        self.hasher.update(&self.buffer[..self.len]);
        self.hasher.finalize()
    }
}

impl std::io::Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.len + bytes.len() > HASH_WRITER_BUFFER_BYTES {
            self.hasher.update(&self.buffer[..self.len]);
            self.len = 0;
            if bytes.len() >= HASH_WRITER_BUFFER_BYTES {
                self.hasher.update(bytes);
                return Ok(bytes.len());
            }
        }
        self.buffer[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Computes a domain-separated BLAKE3 commitment over an already canonical
/// byte payload.
///
/// Extension contracts should prefer [`canonical_hash`] for serde values. This
/// raw form exists for frozen binary contracts, such as Merkle interior nodes,
/// whose byte encoding is defined independently of JSON.
#[must_use]
pub fn canonical_byte_hash(domain: &str, payload: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain.as_bytes());
    hasher.update(&[0]);
    hasher.update(payload);
    hasher.finalize().to_hex().to_string()
}

pub(super) fn is_canonical_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn commitment_roots_are_canonical(roots: &CommitmentRoots) -> bool {
    [
        &roots.world,
        &roots.knowledge,
        &roots.plugin_components,
        &roots.domain_records,
        &roots.scheduler,
        &roots.commands,
        &roots.events,
        &roots.ingress,
        &roots.random,
        &roots.boundary_chain,
        &roots.identity,
        &roots.control,
    ]
    .into_iter()
    .all(|root| is_canonical_hash(root))
        && (roots.decisions.is_empty() || is_canonical_hash(&roots.decisions))
}
