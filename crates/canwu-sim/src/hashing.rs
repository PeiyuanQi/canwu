use super::{
    ArtifactManifest, BOUNDARY_STATE_HASH_V1_PREFIX, BoundaryChange, BoundaryEmission, BoundaryId,
    BoundaryIngressGeneration, BoundaryRecord, BoundaryStateHashFormat, COMMITMENT_FORMAT_VERSION,
    CanwuError, CommandAttemptId, CommandAttemptRecord, CommandId, CommandRecord, CommitmentRoots,
    DecisionState, DomainRecord, DomainRecordChange, ErrorCode, EventId, GENESIS_BOUNDARY_HASH,
    IngressId, IngressRecord, JournalCommitmentRoots, KnowledgeSnapshot, PluginComponentRecord,
    PluginDescriptor, RandomDrawId, RandomDrawRecord, RandomStreamState, ReservationAllocation,
    ReservationOfferRecord, ReservationRequestRecord, RunConfigurationSnapshot, RunManifest,
    RuntimeDomainCommitmentRoots, STATE_REVISION_FORMAT_VERSION, Scenario, ScheduledRecord,
    SchemaRegistry, SimEvent, SimTime, SimulationSnapshot, SystemCadence, WorldSnapshot,
    boundary_state_hash_format, command_attempt_id_slice_is_empty, command_attempt_slice_is_empty,
    component_key, domain_record_change_slice_is_empty, domain_record_slice_is_empty,
    ingress_record_slice_is_empty, invalid_snapshot, invalid_snapshot_error, is_one_u64, manifest,
    policy,
};
use serde::Serialize;
use std::collections::BTreeSet;

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
    pub(super) world: &'a WorldSnapshot,
    pub(super) knowledge: &'a KnowledgeSnapshot,
    pub(super) events: &'a [SimEvent],
    pub(super) commands: &'a [CommandRecord],
    #[serde(skip_serializing_if = "command_attempt_slice_is_empty")]
    pub(super) command_attempts: &'a [CommandAttemptRecord],
    #[serde(skip_serializing_if = "ingress_record_slice_is_empty")]
    pub(super) ingress: &'a [IngressRecord],
    pub(super) plugin_components: &'a [PluginComponentRecord],
    #[serde(skip_serializing_if = "domain_record_slice_is_empty")]
    pub(super) domain_records: &'a [DomainRecord],
    #[serde(skip_serializing_if = "DecisionState::is_empty")]
    pub(super) decisions: &'a DecisionState,
    pub(super) plugin_descriptors: &'a [PluginDescriptor],
    pub(super) schema: &'a SchemaRegistry,
    pub(super) scheduled: &'a [ScheduledRecord],
    pub(super) root_seed: u64,
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
    pub(super) next_schedule_sequence: u64,
    pub(super) next_correlation_id: u64,
    #[serde(skip_serializing_if = "is_one_u64")]
    pub(super) next_decision_trace_id: u64,
}

#[derive(Serialize)]
struct WorldCommitmentMaterial {
    people: String,
    governments: String,
    territories: String,
    routes: String,
    armies: String,
}

#[derive(Serialize)]
struct SchedulerCommitmentMaterial {
    now: SimTime,
    scheduled: String,
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
    pub(super) next_schedule_sequence: u64,
    pub(super) next_correlation_id: u64,
    #[serde(skip_serializing_if = "is_one_u64")]
    pub(super) next_decision_trace_id: u64,
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

pub(super) fn world_commitment_root(world: &WorldSnapshot) -> Result<String, CanwuError> {
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
        },
    )
}

pub(super) fn knowledge_commitment_root(
    knowledge: &KnowledgeSnapshot,
) -> Result<String, CanwuError> {
    canonical_hash("canwu.commitment.knowledge.v1", knowledge)
}

pub(super) fn plugin_component_commitment_root(
    components: &[PluginComponentRecord],
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
    canonical_sorted_hash_by("canwu.commitment.domain-records.v1", records, |record| {
        record.reference.clone()
    })
}

pub(super) fn decision_commitment_root(decisions: &DecisionState) -> Result<String, CanwuError> {
    if decisions.is_empty() {
        return Ok(String::new());
    }
    canonical_hash("canwu.commitment.decisions.v1", decisions)
}

pub(super) fn scheduler_commitment_root(
    now: SimTime,
    scheduled: &[ScheduledRecord],
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
    let world = world_commitment_root(material.world)?;
    let knowledge = knowledge_commitment_root(material.knowledge)?;
    let plugin_components = plugin_component_commitment_root(material.plugin_components)?;
    let domain_records = domain_record_commitment_root(material.domain_records)?;
    let decisions = decision_commitment_root(material.decisions)?;
    let scheduler = scheduler_commitment_root(material.now, material.scheduled)?;
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
            next_schedule_sequence: material.next_schedule_sequence,
            next_correlation_id: material.next_correlation_id,
            next_decision_trace_id: material.next_decision_trace_id,
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
    } = &mut authoritative_manifest
    else {
        return Err(CanwuError::new(
            ErrorCode::InvalidRunManifest,
            "declared run policy requires a declared run manifest",
        ));
    };
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
    state_hash(&StateHashMaterial {
        engine_version: &snapshot.engine_version,
        snapshot_format_version: snapshot.snapshot_format_version,
        run_manifest: &authoritative_manifest,
        run_manifest_hash: &authoritative_manifest_hash,
        initial_time: snapshot.initial_time,
        initial_scenario: snapshot.initial_scenario.as_ref(),
        now: snapshot.now,
        plugin_registration_closed: snapshot.plugin_registration_closed,
        world: &snapshot.world,
        knowledge: &snapshot.knowledge,
        events: &snapshot.events,
        commands: &snapshot.commands,
        command_attempts: &snapshot.command_attempts,
        ingress: &snapshot.ingress,
        plugin_components: &snapshot.plugin_components,
        domain_records: &snapshot.domain_records,
        decisions: &snapshot.decisions,
        plugin_descriptors: &snapshot.plugin_descriptors,
        schema: &snapshot.schema,
        scheduled: &snapshot.scheduled,
        root_seed: snapshot.root_seed,
        random_streams: &snapshot.random_streams,
        random_draws: &snapshot.random_draws,
        next_event_id: snapshot.next_event_id,
        next_command_id: snapshot.next_command_id,
        next_command_attempt_id: snapshot.next_command_attempt_id,
        next_ingress_id: snapshot.next_ingress_id,
        next_boundary_id: snapshot.next_boundary_id,
        next_random_draw_id: snapshot.next_random_draw_id,
        next_schedule_sequence: snapshot.next_schedule_sequence,
        next_correlation_id: snapshot.next_correlation_id,
        next_decision_trace_id: snapshot.next_decision_trace_id,
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
    commitment_roots(
        &StateHashMaterial {
            engine_version: &snapshot.engine_version,
            snapshot_format_version: snapshot.snapshot_format_version,
            run_manifest: &authoritative_manifest,
            run_manifest_hash: &authoritative_manifest_hash,
            initial_time: snapshot.initial_time,
            initial_scenario: snapshot.initial_scenario.as_ref(),
            now: snapshot.now,
            plugin_registration_closed: snapshot.plugin_registration_closed,
            world: &snapshot.world,
            knowledge: &snapshot.knowledge,
            events: &snapshot.events,
            commands: &snapshot.commands,
            command_attempts: &snapshot.command_attempts,
            ingress: &snapshot.ingress,
            plugin_components: &snapshot.plugin_components,
            domain_records: &snapshot.domain_records,
            decisions: &snapshot.decisions,
            plugin_descriptors: &snapshot.plugin_descriptors,
            schema: &snapshot.schema,
            scheduled: &snapshot.scheduled,
            root_seed: snapshot.root_seed,
            random_streams: &snapshot.random_streams,
            random_draws: &snapshot.random_draws,
            next_event_id: snapshot.next_event_id,
            next_command_id: snapshot.next_command_id,
            next_command_attempt_id: snapshot.next_command_attempt_id,
            next_ingress_id: snapshot.next_ingress_id,
            next_boundary_id: snapshot.next_boundary_id,
            next_random_draw_id: snapshot.next_random_draw_id,
            next_schedule_sequence: snapshot.next_schedule_sequence,
            next_correlation_id: snapshot.next_correlation_id,
            next_decision_trace_id: snapshot.next_decision_trace_id,
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
    let admitted_ingress: BTreeSet<_> = snapshot
        .boundaries
        .iter()
        .flat_map(|record| record.admitted_ingress.iter().copied())
        .collect();
    if admitted_ingress.len() != snapshot.ingress.len() {
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

pub(super) fn compute_boundary_hash(record: &BoundaryRecord) -> Result<String, CanwuError> {
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
        #[serde(skip_serializing_if = "domain_record_change_slice_is_empty")]
        record_changes: &'a [DomainRecordChange],
        emissions: &'a [BoundaryEmission],
        state_hash: &'a Option<String>,
        previous_hash: &'a str,
    }

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
            record_changes: &record.record_changes,
            emissions: &record.emissions,
            state_hash: &record.state_hash,
            previous_hash: &record.previous_hash,
        },
    )
}

pub(super) fn canonical_hash<T: Serialize + ?Sized>(
    domain: &str,
    value: &T,
) -> Result<String, CanwuError> {
    let encoded = serde_json::to_vec(value).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidSnapshot,
            format!("could not encode deterministic hash material: {error}"),
        )
    })?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain.as_bytes());
    hasher.update(&[0]);
    hasher.update(&encoded);
    Ok(hasher.finalize().to_hex().to_string())
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
