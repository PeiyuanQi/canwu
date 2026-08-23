use super::event_payloads::{
    REPORT_DISPATCHED, ReportDispatched, RuntimeEventPayload, canonicalize_event_kind,
};
use super::{
    ADMISSION_CURSOR_FORMAT_VERSION, BoundaryRecord, COMMITMENT_FORMAT_VERSION, CanwuError,
    CauseRef, CommandAttemptOutcome, CommandAttemptRecord, DeterministicRng, ENGINE_VERSION,
    ErrorCode, GENESIS_BOUNDARY_HASH, IngressPayload, IngressRecord, RandomDrawAddress,
    RandomDrawId, RandomDrawOutcome, RandomDrawProducer, RandomDrawRecord, RandomStreamState,
    RunConfigurationSnapshot, RunManifest, SNAPSHOT_FORMAT_VERSION, STATE_REVISION_FORMAT_VERSION,
    SimDuration, SimulationSnapshot, canonical_hash, compute_boundary_hash, invalid_snapshot,
    invalid_snapshot_error, is_canonical_hash, manifest, policy, random, snapshot_checkpoint_hash,
    snapshot_commitment_roots, snapshot_is_at_boundary_head, snapshot_state_hash,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn migrate_snapshot(
    mut snapshot: SimulationSnapshot,
) -> Result<SimulationSnapshot, CanwuError> {
    for event in &mut snapshot.events {
        canonicalize_event_kind(&mut event.kind).map_err(|error| {
            invalid_snapshot_error(format!("event payload is not canonical: {error}"))
        })?;
    }
    match snapshot.snapshot_format_version {
        SNAPSHOT_FORMAT_VERSION => {
            if snapshot.engine_version != ENGINE_VERSION {
                return Err(CanwuError::new(
                    ErrorCode::UnsupportedSnapshotVersion,
                    format!(
                        "snapshot format {} from engine {} requires an explicit migration to engine {}",
                        snapshot.snapshot_format_version, snapshot.engine_version, ENGINE_VERSION
                    ),
                ));
            }
            if snapshot.legacy_rng.is_some() {
                return invalid_snapshot("format 5 snapshots cannot contain the legacy global RNG");
            }
            hydrate_snapshot_run_configuration(&mut snapshot)?;
            migrate_snapshot_revision(&mut snapshot)?;
            migrate_snapshot_admission_cursors(&mut snapshot)?;
            migrate_snapshot_commitments(&mut snapshot)?;
            Ok(snapshot)
        }
        2 => {
            if snapshot.run_manifest.is_some()
                || !snapshot.run_manifest_hash.is_empty()
                || !snapshot.checkpoint_hash.is_empty()
                || snapshot.commitment_format_version != 0
                || snapshot.commitment_roots.is_some()
            {
                return invalid_snapshot(
                    "format 2 snapshots cannot contain current manifest or commitment data",
                );
            }
            validate_legacy_ingress_shape(&snapshot)?;
            let checkpoint_hash = canonical_hash("canwu.legacy-checkpoint.v1", &snapshot)?;
            if !snapshot.boundaries.is_empty() || !matches!(snapshot.next_boundary_id, 0 | 1) {
                return invalid_snapshot("format 2 snapshots cannot contain phased-boundary state");
            }
            snapshot.boundaries.clear();
            snapshot.next_boundary_id = 1;
            migrate_format_3_snapshot(snapshot, 2, checkpoint_hash)
        }
        3 => {
            if snapshot.run_manifest.is_some()
                || !snapshot.run_manifest_hash.is_empty()
                || !snapshot.checkpoint_hash.is_empty()
                || snapshot.commitment_format_version != 0
                || snapshot.commitment_roots.is_some()
            {
                return invalid_snapshot(
                    "format 3 snapshots cannot contain current manifest or commitment data",
                );
            }
            validate_legacy_ingress_shape(&snapshot)?;
            let checkpoint_hash = canonical_hash("canwu.legacy-checkpoint.v1", &snapshot)?;
            migrate_format_3_snapshot(snapshot, 3, checkpoint_hash)
        }
        _ => Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "snapshot format {} from engine {} is unsupported; this engine reads formats 2, 3, and {}",
                snapshot.snapshot_format_version, snapshot.engine_version, SNAPSHOT_FORMAT_VERSION
            ),
        )),
    }
}

fn migrate_snapshot_revision(snapshot: &mut SimulationSnapshot) -> Result<(), CanwuError> {
    match snapshot.revision_format_version {
        STATE_REVISION_FORMAT_VERSION => Ok(()),
        0 => {
            if snapshot.state_revision != 0 || snapshot.replay_revision_format_version != 0 {
                return invalid_snapshot(
                    "legacy revision snapshots cannot contain current revision or replay provenance",
                );
            }
            let legacy_checkpoint = snapshot_checkpoint_hash(snapshot)?;
            if !is_canonical_hash(&snapshot.checkpoint_hash)
                || legacy_checkpoint != snapshot.checkpoint_hash
            {
                return invalid_snapshot(
                    "legacy checkpoint hash does not bind the pre-migration state",
                );
            }
            validate_legacy_boundary_hash_chain(&snapshot.boundaries)?;
            migrate_command_attempt_revisions(
                &mut snapshot.command_attempts,
                &snapshot.boundaries,
            )?;
            migrate_ingress_command_revisions(
                &mut snapshot.ingress,
                &snapshot.command_attempts,
                &snapshot.boundaries,
            )?;
            if snapshot_is_at_boundary_head(snapshot) {
                let migrated_state_hash = snapshot_state_hash(snapshot)?;
                snapshot
                    .boundaries
                    .last_mut()
                    .expect("boundary-head snapshots have a final boundary")
                    .state_hash = Some(migrated_state_hash);
            }
            rehash_snapshot_boundaries(snapshot)?;
            snapshot.state_revision = authoritative_revision_count(
                snapshot.commands.len(),
                snapshot.command_attempts.len(),
                snapshot.boundaries.len(),
            )?;
            snapshot.revision_format_version = STATE_REVISION_FORMAT_VERSION;
            snapshot.checkpoint_hash = snapshot_checkpoint_hash(snapshot)?;
            Ok(())
        }
        version => Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "state revision format {version} is unsupported; this engine reads legacy format 0 and current format {STATE_REVISION_FORMAT_VERSION}"
            ),
        )),
    }
}

fn migrate_snapshot_admission_cursors(snapshot: &mut SimulationSnapshot) -> Result<(), CanwuError> {
    match snapshot.admission_cursor_format_version {
        ADMISSION_CURSOR_FORMAT_VERSION => Ok(()),
        0 => {
            if snapshot.admitted_attempt_count != 0
                || snapshot.admitted_command_count != 0
                || snapshot.admitted_event_count != 0
            {
                return invalid_snapshot(
                    "legacy admission-cursor snapshots cannot contain current cursor values",
                );
            }
            let cursors = admission_cursors_from_boundaries(&snapshot.boundaries)?;
            snapshot.admitted_attempt_count = cursors.attempts;
            snapshot.admitted_command_count = cursors.commands;
            snapshot.admitted_event_count = cursors.events;
            snapshot.admission_cursor_format_version = ADMISSION_CURSOR_FORMAT_VERSION;
            Ok(())
        }
        version => Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "admission cursor format {version} is unsupported; this engine reads legacy format 0 and current format {ADMISSION_CURSOR_FORMAT_VERSION}"
            ),
        )),
    }
}

fn migrate_snapshot_commitments(snapshot: &mut SimulationSnapshot) -> Result<(), CanwuError> {
    match snapshot.commitment_format_version {
        COMMITMENT_FORMAT_VERSION => {
            if snapshot.commitment_roots.is_none() {
                return invalid_snapshot(
                    "current commitment snapshot is missing its persisted domain roots",
                );
            }
            Ok(())
        }
        0 => {
            if snapshot.commitment_roots.is_some() {
                return invalid_snapshot(
                    "legacy commitment snapshot cannot contain current domain roots",
                );
            }
            let legacy_checkpoint = snapshot_checkpoint_hash(snapshot)?;
            if !is_canonical_hash(&snapshot.checkpoint_hash)
                || legacy_checkpoint != snapshot.checkpoint_hash
            {
                return invalid_snapshot(
                    "legacy checkpoint hash does not bind the pre-commitment state",
                );
            }
            snapshot.commitment_roots = Some(snapshot_commitment_roots(snapshot)?);
            snapshot.commitment_format_version = COMMITMENT_FORMAT_VERSION;
            snapshot.checkpoint_hash = snapshot_checkpoint_hash(snapshot)?;
            Ok(())
        }
        version => Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "commitment format {version} is unsupported; this engine reads legacy format 0 and current format {COMMITMENT_FORMAT_VERSION}"
            ),
        )),
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct PersistedAdmissionCursors {
    pub(super) attempts: u64,
    pub(super) commands: u64,
    pub(super) events: u64,
}

fn admission_cursors_from_boundaries(
    boundaries: &[BoundaryRecord],
) -> Result<PersistedAdmissionCursors, CanwuError> {
    let mut cursors = PersistedAdmissionCursors::default();
    for boundary in boundaries {
        cursors.attempts = cursors
            .attempts
            .checked_add(
                u64::try_from(boundary.admitted_attempts.len()).map_err(|_| {
                    invalid_snapshot_error("admitted attempt count exceeds cursor range")
                })?,
            )
            .ok_or_else(|| invalid_snapshot_error("admitted attempt cursor is exhausted"))?;
        cursors.commands = cursors
            .commands
            .checked_add(
                u64::try_from(boundary.admitted_commands.len()).map_err(|_| {
                    invalid_snapshot_error("admitted command count exceeds cursor range")
                })?,
            )
            .ok_or_else(|| invalid_snapshot_error("admitted command cursor is exhausted"))?;
        cursors.events =
            cursors
                .events
                .checked_add(u64::try_from(boundary.admitted_events.len()).map_err(|_| {
                    invalid_snapshot_error("admitted event count exceeds cursor range")
                })?)
                .ok_or_else(|| invalid_snapshot_error("admitted event cursor is exhausted"))?;
    }
    Ok(cursors)
}

pub(super) fn rehash_snapshot_boundaries(
    snapshot: &mut SimulationSnapshot,
) -> Result<(), CanwuError> {
    let mut previous_hash = GENESIS_BOUNDARY_HASH.to_owned();
    for boundary in &mut snapshot.boundaries {
        boundary.previous_hash.clone_from(&previous_hash);
        boundary.hash = compute_boundary_hash(boundary)?;
        previous_hash.clone_from(&boundary.hash);
    }
    Ok(())
}

fn validate_legacy_boundary_hash_chain(boundaries: &[BoundaryRecord]) -> Result<(), CanwuError> {
    let mut previous_hash = GENESIS_BOUNDARY_HASH.to_owned();
    for boundary in boundaries {
        if boundary.previous_hash != previous_hash
            || !is_canonical_hash(&boundary.hash)
            || boundary
                .state_hash
                .as_deref()
                .is_some_and(|hash| !is_canonical_hash(hash))
            || compute_boundary_hash(boundary)? != boundary.hash
        {
            return invalid_snapshot("legacy boundary hash chain is inconsistent");
        }
        previous_hash.clone_from(&boundary.hash);
    }
    Ok(())
}

fn migrate_ingress_command_revisions(
    ingress: &mut [IngressRecord],
    attempts: &[CommandAttemptRecord],
    boundaries: &[BoundaryRecord],
) -> Result<(), CanwuError> {
    let mut attempts_by_request = BTreeMap::new();
    for attempt in attempts {
        let Some(request_id) = attempt.request_id else {
            continue;
        };
        let expected_revision = attempt.expected_revision.ok_or_else(|| {
            invalid_snapshot_error("tracked command attempt is missing its revision guard")
        })?;
        if attempts_by_request
            .insert(request_id, expected_revision)
            .is_some()
        {
            return invalid_snapshot("command request ID is reused across attempt evidence");
        }
    }
    let (legacy_revision_by_cut, migrated_revision_by_cut) =
        revision_values_after_boundary_cuts(attempts, boundaries)?;
    let admitted_ingress: BTreeSet<_> = boundaries
        .iter()
        .flat_map(|boundary| boundary.admitted_ingress.iter().copied())
        .collect();
    for record in ingress {
        let IngressPayload::Command { request } = &mut record.payload else {
            continue;
        };
        if admitted_ingress.contains(&record.id) {
            request.expected_revision =
                *attempts_by_request
                    .get(&request.request_id)
                    .ok_or_else(|| {
                        invalid_snapshot_error(
                            "admitted command ingress is missing its deterministic attempt",
                        )
                    })?;
            continue;
        }
        let issue_cut = usize::try_from(record.eligible_boundary_count).map_err(|_| {
            invalid_snapshot_error("command ingress issue cut exceeds the platform index range")
        })?;
        let legacy_revision = *legacy_revision_by_cut.get(issue_cut).ok_or_else(|| {
            invalid_snapshot_error("command ingress issue cut exceeds boundary history")
        })?;
        let migrated_revision = migrated_revision_by_cut[issue_cut];
        request.expected_revision = translate_revision_guard(
            request.expected_revision,
            legacy_revision,
            migrated_revision,
        )?;
    }
    Ok(())
}

fn revision_values_after_boundary_cuts(
    attempts: &[CommandAttemptRecord],
    boundaries: &[BoundaryRecord],
) -> Result<(Vec<u64>, Vec<u64>), CanwuError> {
    let mut legacy = vec![0];
    let mut migrated = vec![0];
    let mut accepted_attempts = 0_u64;
    let mut admitted_attempts = 0_u64;
    for (boundary_index, boundary) in boundaries.iter().enumerate() {
        for attempt_id in &boundary.admitted_attempts {
            let attempt_index =
                usize::try_from(attempt_id.get().saturating_sub(1)).map_err(|_| {
                    invalid_snapshot_error("boundary attempt ID exceeds the journal index range")
                })?;
            let attempt = attempts.get(attempt_index).ok_or_else(|| {
                invalid_snapshot_error("boundary admits an unknown command attempt")
            })?;
            admitted_attempts = admitted_attempts.checked_add(1).ok_or_else(|| {
                invalid_snapshot_error("admitted attempt count exceeds revision space")
            })?;
            if matches!(attempt.outcome, CommandAttemptOutcome::Accepted { .. }) {
                accepted_attempts = accepted_attempts.checked_add(1).ok_or_else(|| {
                    invalid_snapshot_error("accepted attempt count exceeds revision space")
                })?;
            }
        }
        let completed_boundaries = u64::try_from(boundary_index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or_else(|| invalid_snapshot_error("boundary count exceeds revision space"))?;
        legacy.push(
            accepted_attempts
                .checked_add(completed_boundaries)
                .ok_or_else(|| invalid_snapshot_error("legacy revision space is exhausted"))?,
        );
        migrated.push(
            admitted_attempts
                .checked_add(completed_boundaries)
                .ok_or_else(|| invalid_snapshot_error("migrated revision space is exhausted"))?,
        );
    }
    Ok((legacy, migrated))
}

fn migrate_command_attempt_revisions(
    attempts: &mut [CommandAttemptRecord],
    boundaries: &[BoundaryRecord],
) -> Result<(), CanwuError> {
    let boundaries_before = boundaries_before_attempts(attempts.len(), boundaries)?;
    let mut accepted_commands = 0_u64;
    for (index, attempt) in attempts.iter_mut().enumerate() {
        let attempt_index = u64::try_from(index)
            .map_err(|_| invalid_snapshot_error("command attempt index exceeds revision space"))?;
        let legacy_revision = accepted_commands
            .checked_add(boundaries_before[index])
            .ok_or_else(|| invalid_snapshot_error("legacy command revision space is exhausted"))?;
        let migrated_revision = attempt_index
            .checked_add(boundaries_before[index])
            .ok_or_else(|| invalid_snapshot_error("command revision space is exhausted"))?;
        if attempt.revision_before != legacy_revision {
            return invalid_snapshot("legacy command attempt revision is inconsistent");
        }
        if let Some(expected) = attempt.expected_revision {
            attempt.expected_revision = Some(translate_revision_guard(
                expected,
                legacy_revision,
                migrated_revision,
            )?);
        }
        attempt.revision_before = migrated_revision;
        match &mut attempt.outcome {
            CommandAttemptOutcome::Accepted { .. } => {
                if attempt
                    .expected_revision
                    .is_some_and(|expected| expected != migrated_revision)
                {
                    return invalid_snapshot(
                        "legacy accepted attempt has a stale expected revision",
                    );
                }
                accepted_commands = accepted_commands.checked_add(1).ok_or_else(|| {
                    invalid_snapshot_error("accepted command count exceeds revision space")
                })?;
            }
            CommandAttemptOutcome::Rejected { error }
                if error.code == ErrorCode::SimulationRevisionConflict =>
            {
                let expected = attempt.expected_revision.ok_or_else(|| {
                    invalid_snapshot_error("revision conflict is missing its expected revision")
                })?;
                error.message = format!(
                    "command expected revision {expected}, but simulation is at revision {migrated_revision}"
                );
            }
            CommandAttemptOutcome::Rejected { .. } => {}
        }
    }
    Ok(())
}

fn translate_revision_guard(
    expected: u64,
    legacy_revision: u64,
    migrated_revision: u64,
) -> Result<u64, CanwuError> {
    if expected >= legacy_revision {
        Ok(migrated_revision
            .checked_add(expected - legacy_revision)
            .unwrap_or(if migrated_revision == u64::MAX {
                0
            } else {
                u64::MAX
            }))
    } else {
        migrated_revision
            .checked_sub(legacy_revision - expected)
            .ok_or_else(|| invalid_snapshot_error("migrated expected revision underflowed"))
    }
}

pub(super) fn boundaries_before_attempts(
    attempt_count: usize,
    boundaries: &[BoundaryRecord],
) -> Result<Vec<u64>, CanwuError> {
    let boundary_count = u64::try_from(boundaries.len())
        .map_err(|_| invalid_snapshot_error("boundary count exceeds revision space"))?;
    let mut values = vec![boundary_count; attempt_count];
    for (boundary_index, boundary) in boundaries.iter().enumerate() {
        let prior_boundaries = u64::try_from(boundary_index)
            .map_err(|_| invalid_snapshot_error("boundary index exceeds revision space"))?;
        for attempt_id in &boundary.admitted_attempts {
            let attempt_index =
                usize::try_from(attempt_id.get().saturating_sub(1)).map_err(|_| {
                    invalid_snapshot_error("boundary attempt ID exceeds the journal index range")
                })?;
            let Some(value) = values.get_mut(attempt_index) else {
                return invalid_snapshot("boundary admits an unknown command attempt");
            };
            *value = prior_boundaries;
        }
    }
    Ok(values)
}

pub(super) fn authoritative_revision_count(
    command_count: usize,
    attempt_count: usize,
    boundary_count: usize,
) -> Result<u64, CanwuError> {
    let command_transactions = if attempt_count == 0 {
        command_count
    } else {
        attempt_count
    };
    u64::try_from(command_transactions)
        .ok()
        .and_then(|commands| {
            u64::try_from(boundary_count)
                .ok()
                .and_then(|boundaries| commands.checked_add(boundaries))
        })
        .ok_or_else(|| invalid_snapshot_error("authoritative revision space is exhausted"))
}

fn validate_legacy_ingress_shape(snapshot: &SimulationSnapshot) -> Result<(), CanwuError> {
    if snapshot.revision_format_version != 0
        || snapshot.state_revision != 0
        || snapshot.replay_revision_format_version != 0
        || snapshot.admission_cursor_format_version != 0
        || snapshot.admitted_attempt_count != 0
        || snapshot.admitted_command_count != 0
        || snapshot.admitted_event_count != 0
        || snapshot.run_configuration.is_some()
        || snapshot.initial_scenario.is_some()
        || !snapshot.command_attempts.is_empty()
        || !snapshot.ingress.is_empty()
        || !snapshot.domain_records.is_empty()
        || snapshot.next_command_attempt_id != 1
        || snapshot.next_ingress_id != 1
        || snapshot
            .commands
            .iter()
            .any(|record| record.attempt_id.is_some() || !record.emitted_events.is_empty())
        || snapshot.boundaries.iter().any(|record| {
            !record.admitted_attempts.is_empty()
                || !record.admitted_ingress.is_empty()
                || !record.generated_ingress.is_empty()
                || !record.record_changes.is_empty()
        })
    {
        return invalid_snapshot(
            "legacy snapshots cannot contain current run-policy or command-attempt evidence",
        );
    }
    Ok(())
}

fn migrate_format_3_snapshot(
    mut snapshot: SimulationSnapshot,
    source_snapshot_format: u32,
    checkpoint_hash: String,
) -> Result<SimulationSnapshot, CanwuError> {
    if !snapshot.plugin_descriptors.is_empty() {
        return Err(CanwuError::new(
            ErrorCode::PluginManifestMismatch,
            "legacy plugin snapshots lack executable semantic identities and cannot be safely migrated",
        ));
    }
    if !snapshot.random_streams.is_empty()
        || !snapshot.random_draws.is_empty()
        || snapshot.next_random_draw_id != 0
    {
        return invalid_snapshot("legacy snapshots cannot contain scoped random state");
    }
    let legacy_rng = snapshot
        .legacy_rng
        .take()
        .ok_or_else(|| invalid_snapshot_error("legacy snapshot is missing its global RNG state"))?;
    let dispatch_count = u64::try_from(
        snapshot
            .events
            .iter()
            .filter(|event| event.kind.is_type(REPORT_DISPATCHED))
            .count(),
    )
    .map_err(|_| invalid_snapshot_error("legacy random draw count exceeds identifier space"))?;
    let root_seed = DeterministicRng::seed_before(legacy_rng.state(), dispatch_count);
    let core_key = random::core_report_delay_stream();
    let mut core_state = RandomStreamState::initial(root_seed, core_key.clone());
    core_state.position = dispatch_count;
    core_state.generator_state = legacy_rng.state();

    let mut random_draws = Vec::new();
    for event in &snapshot.events {
        if !event.kind.is_type(REPORT_DISPATCHED) {
            continue;
        }
        let ReportDispatched {
            recipient,
            army,
            arrives_at,
        } = ReportDispatched::decode(&event.kind)
            .map_err(|_| invalid_snapshot_error("legacy report dispatch payload is malformed"))?;
        let jitter = arrives_at
            .as_minutes()
            .checked_sub(event.timestamp.as_minutes())
            .and_then(|duration| duration.checked_sub(SimDuration::hours(36).as_minutes()))
            .ok_or_else(|| {
                invalid_snapshot_error("legacy report timing exceeds the supported range")
            })?;
        let Ok(value) = u64::try_from(jitter) else {
            return invalid_snapshot("legacy report jitter is outside the scoped RNG contract");
        };
        let Some(CauseRef::Event(cause)) = &event.cause else {
            return invalid_snapshot("legacy report dispatch lacks its arrival-event cause");
        };
        if value >= 12 * 60 {
            return invalid_snapshot("legacy report jitter is outside the scoped RNG contract");
        }
        let id_value = u64::try_from(random_draws.len())
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| invalid_snapshot_error("legacy random draw IDs are exhausted"))?;
        random_draws.push(RandomDrawRecord {
            id: RandomDrawId::new(id_value),
            at: event.timestamp,
            stream: core_key.clone(),
            address: RandomDrawAddress::Sequential {
                position: id_value - 1,
            },
            operation_evidence: None,
            upper_exclusive: 12 * 60,
            value,
            purpose: "knowledge report delivery jitter".to_owned(),
            producer: RandomDrawProducer::CoreSystem {
                system: "canwu.core.knowledge-report-delay".to_owned(),
            },
            outcome: Some(RandomDrawOutcome::KnowledgeReportDelivery {
                recipient,
                army,
                dispatch_event: event.id,
                arrives_at,
            }),
            cause: CauseRef::Event(*cause),
            correlation_id: event.correlation_id,
        });
    }

    let mut streams = BTreeMap::from([(core_key, core_state)]);
    for key in snapshot
        .plugin_descriptors
        .iter()
        .flat_map(|descriptor| &descriptor.boundary_systems)
        .flat_map(|contract| &contract.random_streams)
    {
        streams
            .entry(key.clone())
            .or_insert_with(|| RandomStreamState::initial(root_seed, key.clone()));
    }
    snapshot.root_seed = root_seed;
    snapshot.random_streams = streams.into_values().collect();
    snapshot.random_draws = random_draws;
    snapshot.next_random_draw_id = dispatch_count.checked_add(1).ok_or_else(|| {
        invalid_snapshot_error("legacy random draw counter exceeds identifier space")
    })?;

    let mut previous_hash = GENESIS_BOUNDARY_HASH.to_owned();
    for boundary in &mut snapshot.boundaries {
        boundary.random_draws.clear();
        boundary.state_hash = None;
        boundary.previous_hash.clone_from(&previous_hash);
        boundary.hash = compute_boundary_hash(boundary)?;
        previous_hash.clone_from(&boundary.hash);
    }
    let source_engine_version = snapshot.engine_version.clone();
    let run_manifest = RunManifest::migrated_legacy(
        source_engine_version,
        source_snapshot_format,
        checkpoint_hash,
    );
    snapshot.run_manifest_hash = manifest::hash(&run_manifest)?;
    snapshot.run_manifest = Some(run_manifest);
    snapshot.run_configuration = Some(RunConfigurationSnapshot::LegacyUnspecified);
    snapshot.state_revision = authoritative_revision_count(
        snapshot.commands.len(),
        snapshot.command_attempts.len(),
        snapshot.boundaries.len(),
    )?;
    snapshot.revision_format_version = STATE_REVISION_FORMAT_VERSION;
    let admission_cursors = admission_cursors_from_boundaries(&snapshot.boundaries)?;
    snapshot.admitted_attempt_count = admission_cursors.attempts;
    snapshot.admitted_command_count = admission_cursors.commands;
    snapshot.admitted_event_count = admission_cursors.events;
    snapshot.admission_cursor_format_version = ADMISSION_CURSOR_FORMAT_VERSION;
    ENGINE_VERSION.clone_into(&mut snapshot.engine_version);
    snapshot.snapshot_format_version = SNAPSHOT_FORMAT_VERSION;
    snapshot.checkpoint_hash = snapshot_checkpoint_hash(&snapshot)?;
    migrate_snapshot_commitments(&mut snapshot)?;
    Ok(snapshot)
}

fn hydrate_snapshot_run_configuration(snapshot: &mut SimulationSnapshot) -> Result<(), CanwuError> {
    if snapshot.run_configuration.is_some() {
        return Ok(());
    }
    let Some(run_manifest) = &snapshot.run_manifest else {
        return Err(CanwuError::new(
            ErrorCode::InvalidRunManifest,
            "snapshot is missing its run manifest",
        ));
    };
    snapshot.run_configuration = Some(inferred_run_configuration(run_manifest)?);
    Ok(())
}

pub(super) fn inferred_run_configuration(
    run_manifest: &RunManifest,
) -> Result<RunConfigurationSnapshot, CanwuError> {
    Ok(match run_manifest {
        RunManifest::Declared {
            run_configuration, ..
        } if run_configuration.semantic_hash == policy::compatibility_configuration_hash()? => {
            RunConfigurationSnapshot::CompatibilityV1
        }
        RunManifest::Declared { .. } => RunConfigurationSnapshot::ManifestOnlyV1,
        RunManifest::MigratedLegacy { .. } => RunConfigurationSnapshot::LegacyUnspecified,
    })
}
