use super::{
    BoundaryRecord, BoundaryRequest, COMMITMENT_FORMAT_VERSION, CanwuError, CauseRef,
    CommandAttemptOutcome, CommandAttemptRecord, CommandIngress, CommandOutcome, CommandRecord,
    ENGINE_VERSION, ErrorCode, IngressPayload, IngressRecord, PluginArchiveObjectProvider,
    PluginIngressRequest, PluginRegistry, ReplayJournal, SNAPSHOT_FORMAT_VERSION,
    STATE_REVISION_FORMAT_VERSION, SimDuration, SimTime, Simulation, SimulationPlugin,
    authoritative_revision_count, authoritative_run_identity, boundary_state_hash_format,
    is_canonical_hash, manifest,
};
#[cfg(test)]
use super::{RunConfiguration, RunManifest, Scenario};
use std::{collections::BTreeSet, rc::Rc};

impl Simulation {
    /// Reconstructs caller-supplied core commands without proving a recorded
    /// package environment. Use [`Self::replay_from_journal`] for exact replay.
    #[cfg(test)]
    pub(crate) fn replay(
        seed: u64,
        scenario: Scenario,
        commands: &[CommandRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        Self::replay_with_plugins(seed, scenario, &[], commands, final_time)
    }

    /// Reconstructs caller-supplied inputs under caller-supplied plugins.
    /// This is not an exact replay identity check.
    #[cfg(test)]
    pub(crate) fn replay_with_plugins(
        seed: u64,
        scenario: Scenario,
        plugins: &[&dyn SimulationPlugin],
        commands: &[CommandRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        Self::replay_with_boundaries(seed, scenario, plugins, commands, &[], final_time)
    }

    /// Reconstructs caller-supplied inputs and compares supplied boundaries.
    /// Use [`Self::replay_from_journal`] when command-only runs must also bind
    /// their recorded run and plugin identities.
    #[cfg(test)]
    pub(crate) fn replay_with_boundaries(
        seed: u64,
        scenario: Scenario,
        plugins: &[&dyn SimulationPlugin],
        commands: &[CommandRecord],
        boundaries: &[BoundaryRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        let run_manifest = RunManifest::for_scenario("canwu.inline", "scenario", "1", &scenario)?;
        Self::replay_with_run_manifest(
            seed,
            scenario,
            run_manifest,
            plugins,
            commands,
            boundaries,
            final_time,
        )
    }

    /// Reconstructs caller-supplied inputs under a caller-supplied run manifest.
    /// This is useful for fixtures; it does not establish recorded identity.
    #[cfg(test)]
    pub(crate) fn replay_with_run_manifest(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        plugins: &[&dyn SimulationPlugin],
        commands: &[CommandRecord],
        boundaries: &[BoundaryRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        let simulation =
            Self::new_with_manifest_and_plugins(seed, scenario, run_manifest, plugins)?;
        Self::replay_records(simulation, commands, &[], &[], boundaries, final_time)
    }

    /// Reconstructs caller-supplied inputs under a caller-supplied declared run
    /// configuration. This does not establish recorded-environment identity.
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub(crate) fn replay_with_run_configuration(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        run_configuration: RunConfiguration,
        plugins: &[&dyn SimulationPlugin],
        commands: &[CommandRecord],
        command_attempts: &[CommandAttemptRecord],
        boundaries: &[BoundaryRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        if command_attempts
            .iter()
            .any(|attempt| attempt.ingress == CommandIngress::FrozenReplay)
        {
            return Err(CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                "frozen replay attempts require an environment-bound replay journal",
            ));
        }
        let simulation = Self::new_with_run_configuration_and_plugins(
            seed,
            scenario,
            run_manifest,
            run_configuration,
            plugins,
        )?;
        Self::replay_records(
            simulation,
            commands,
            command_attempts,
            &[],
            boundaries,
            final_time,
        )
    }

    /// Replays only after the recorded engine, run, seed, and plugin manifests
    /// match, then verifies the final checkpoint commitment.
    pub fn replay_from_journal(
        plugins: &[&dyn SimulationPlugin],
        journal: &ReplayJournal,
    ) -> Result<Self, CanwuError> {
        Self::replay_from_journal_with_archive_provider(plugins, journal, Rc::new(()))
    }

    /// Replays with caller-owned package archive storage attached before any
    /// recorded boundary is evaluated. Cold idempotency and continuation
    /// checks therefore use the same authenticated provider during replay as
    /// they do during a live run.
    pub fn replay_from_journal_with_archive_provider(
        plugins: &[&dyn SimulationPlugin],
        journal: &ReplayJournal,
        archive_provider: Rc<dyn PluginArchiveObjectProvider>,
    ) -> Result<Self, CanwuError> {
        if journal.commitment_format_version != COMMITMENT_FORMAT_VERSION {
            return Err(CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                format!(
                    "replay journal commitment format {} is unsupported; this engine reads format {COMMITMENT_FORMAT_VERSION}",
                    journal.commitment_format_version
                ),
            ));
        }
        if journal.revision_format_version != STATE_REVISION_FORMAT_VERSION {
            return Err(CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                format!(
                    "replay journal revision format {} is unsupported; this engine reads format {STATE_REVISION_FORMAT_VERSION}",
                    journal.revision_format_version
                ),
            ));
        }
        if journal.authority_root_seed == 0 {
            return Err(CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                "replay journal is missing its persisted authority root",
            ));
        }
        let expected_final_revision = authoritative_revision_count(
            journal.commands.len(),
            journal.command_attempts.len(),
            journal.boundaries.len(),
        )?;
        if journal.final_revision != expected_final_revision {
            return Err(CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                "replay journal final revision is inconsistent with its committed evidence",
            ));
        }
        let normalized = journal;
        let scenario = normalized.initial_scenario.clone();
        manifest::validate(&normalized.run_manifest, Some(&scenario))?;
        let expected_manifest_hash = manifest::hash(&normalized.run_manifest)?;
        if normalized.run_manifest_hash != expected_manifest_hash {
            return Err(CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                "replay journal run manifest hash is inconsistent",
            ));
        }
        manifest::validate_run_configuration(
            &normalized.run_manifest,
            &normalized.run_configuration,
        )?;
        if normalized.engine_version != ENGINE_VERSION
            || normalized.snapshot_format_version != SNAPSHOT_FORMAT_VERSION
            || !is_canonical_hash(&normalized.run_manifest_hash)
            || manifest::hash(&normalized.run_manifest)? != normalized.run_manifest_hash
            || !is_canonical_hash(&normalized.checkpoint_hash)
        {
            return Err(CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                "replay journal engine, format, or run identity does not match this runtime",
            ));
        }
        let (_, authority_manifest_hash) = authoritative_run_identity(
            &normalized.run_manifest,
            &normalized.run_manifest_hash,
            &normalized.run_configuration,
        )?;
        if normalized.authority_root_seed
            != super::fresh_authority_root_seed(normalized.root_seed, &authority_manifest_hash)?
        {
            return Err(CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                "replay journal authority root is not bound to its run identity",
            ));
        }
        PluginRegistry::from_descriptors(normalized.plugin_descriptors.clone()).map_err(
            |error| {
                CanwuError::new(
                    ErrorCode::ReplayEnvironmentMismatch,
                    format!("replay journal plugin manifest is invalid: {error}"),
                )
            },
        )?;

        let mut simulation = Self::new_with_configuration_snapshot(
            normalized.root_seed,
            scenario,
            normalized.run_manifest.clone(),
            normalized.run_configuration.clone(),
        )?;
        simulation.set_plugin_archive_object_provider(archive_provider);
        simulation.state.current.authority_root_seed = normalized.authority_root_seed;
        let simulation = Self::activate_initial_plugins(simulation, plugins)?;
        let actual_descriptors: Vec<_> = simulation.plugin_descriptors().cloned().collect();
        if actual_descriptors != normalized.plugin_descriptors {
            return Err(CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                "active plugin identities and contracts do not match the replay journal",
            ));
        }

        let mut simulation = Self::replay_records(
            simulation,
            &normalized.commands,
            &normalized.command_attempts,
            &normalized.ingress,
            &normalized.boundaries,
            normalized.final_time,
        )?;
        if normalized.plugin_registration_closed
            && !simulation.state.metadata.plugin_registration_closed
        {
            simulation.advance(SimDuration::ZERO)?;
        }
        if simulation.state.metadata.plugin_registration_closed
            != normalized.plugin_registration_closed
        {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "replayed plugin-registration lifecycle does not match the recorded journal",
            ));
        }
        if simulation.revision() != normalized.final_revision {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "replayed final state revision does not match the recorded journal",
            ));
        }
        if simulation.checkpoint_hash() != normalized.checkpoint_hash {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "replayed final checkpoint does not match the recorded journal",
            ));
        }
        Ok(simulation)
    }

    /// Deserializes a Format 8 replay journal with recursive unknown-field
    /// rejection, then performs the exact environment-bound replay.
    pub fn replay_from_journal_json(
        plugins: &[&dyn SimulationPlugin],
        json: &str,
    ) -> Result<Self, CanwuError> {
        let journal: ReplayJournal = super::deserialize_current_json(json, "replay journal")?;
        Self::replay_from_journal(plugins, &journal)
    }

    #[cfg(test)]
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn replay_from_journal_with_scenario(
        scenario: Scenario,
        plugins: &[&dyn SimulationPlugin],
        journal: &ReplayJournal,
    ) -> Result<Self, CanwuError> {
        if scenario != journal.initial_scenario {
            return Err(CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                "test replay scenario disagrees with the self-contained journal scenario",
            ));
        }
        Self::replay_from_journal(plugins, journal)
    }

    fn replay_records(
        mut simulation: Self,
        commands: &[CommandRecord],
        attempts: &[CommandAttemptRecord],
        ingress: &[IngressRecord],
        boundaries: &[BoundaryRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        simulation.ensure_runtime_ready()?;
        if !attempts.is_empty() {
            return Self::replay_attempt_records(
                simulation, commands, attempts, ingress, boundaries, final_time,
            );
        }
        let mut next_ingress = 0;
        let mut next_command = 0;
        for (boundary_index, expected_boundary) in boundaries.iter().enumerate() {
            enqueue_replay_ingress_cut(
                &mut simulation,
                ingress,
                &mut next_ingress,
                boundary_index,
            )?;
            for admitted in &expected_boundary.admitted_commands {
                let Some(record) = commands.get(next_command) else {
                    return Err(CanwuError::new(
                        ErrorCode::ReplayMismatch,
                        "boundary replay admits a command absent from the journal",
                    ));
                };
                if record.id != *admitted {
                    return Err(CanwuError::new(
                        ErrorCode::ReplayMismatch,
                        "boundary replay command admission does not match journal order",
                    ));
                }
                replay_command_record(&mut simulation, record, expected_boundary.at)?;
                next_command += 1;
            }
            let receipt = simulation.settle_boundary_with_state_hash_format(
                BoundaryRequest {
                    at: expected_boundary.at,
                    cadences: expected_boundary.cadences.clone(),
                },
                boundary_state_hash_format(expected_boundary.state_hash.as_deref())?,
            )?;
            let Some(actual_boundary) = simulation.boundaries().last() else {
                return Err(CanwuError::new(
                    ErrorCode::ReplayMismatch,
                    "boundary replay did not append settlement evidence",
                ));
            };
            if receipt.boundary_id != expected_boundary.id || actual_boundary != expected_boundary {
                return Err(CanwuError::new(
                    ErrorCode::ReplayMismatch,
                    format!(
                        "regenerated boundary {} did not match its journal evidence",
                        expected_boundary.id
                    ),
                ));
            }
        }
        for record in &commands[next_command..] {
            replay_command_record(&mut simulation, record, final_time)?;
        }
        enqueue_replay_ingress_cut(
            &mut simulation,
            ingress,
            &mut next_ingress,
            boundaries.len(),
        )?;
        if next_ingress != ingress.len() {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "ingress journal contains an impossible future boundary issue cut",
            ));
        }
        if final_time < simulation.time() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "replay final time cannot precede the last command",
            ));
        }
        if final_time > simulation.time() {
            simulation.ensure_legacy_advance_does_not_cross_ingress(final_time)?;
            simulation.advance_to(final_time)?;
        }
        Ok(simulation)
    }

    fn replay_attempt_records(
        mut simulation: Self,
        commands: &[CommandRecord],
        attempts: &[CommandAttemptRecord],
        ingress: &[IngressRecord],
        boundaries: &[BoundaryRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        let command_ingress_requests: BTreeSet<_> = ingress
            .iter()
            .filter_map(|record| match &record.payload {
                IngressPayload::Command { request } => Some(request.request_id),
                IngressPayload::Decision { request } => {
                    request.command.as_ref().map(|command| command.request_id)
                }
                IngressPayload::Plugin { .. }
                | IngressPayload::Calendar { .. }
                | IngressPayload::Maintenance { .. } => None,
            })
            .collect();
        let mut next_ingress = 0;
        let mut next_attempt = 0;
        for (boundary_index, expected_boundary) in boundaries.iter().enumerate() {
            enqueue_replay_ingress_cut(
                &mut simulation,
                ingress,
                &mut next_ingress,
                boundary_index,
            )?;
            let mut admitted_commands = Vec::new();
            for admitted in &expected_boundary.admitted_attempts {
                let Some(record) = attempts.get(next_attempt) else {
                    return Err(CanwuError::new(
                        ErrorCode::ReplayMismatch,
                        "boundary replay admits a command attempt absent from the journal",
                    ));
                };
                if record.id != *admitted {
                    return Err(CanwuError::new(
                        ErrorCode::ReplayMismatch,
                        "boundary replay attempt admission does not match journal order",
                    ));
                }
                let queued = record
                    .request_id
                    .is_some_and(|request| command_ingress_requests.contains(&request));
                if !queued {
                    replay_attempt_record(&mut simulation, record, commands, expected_boundary.at)?;
                }
                if let CommandAttemptOutcome::Accepted { command_id } = record.outcome {
                    admitted_commands.push(command_id);
                }
                next_attempt += 1;
            }
            if admitted_commands != expected_boundary.admitted_commands {
                return Err(CanwuError::new(
                    ErrorCode::ReplayMismatch,
                    "boundary replay accepted-command cut disagrees with admitted attempts",
                ));
            }
            let receipt = simulation.settle_boundary_with_state_hash_format(
                BoundaryRequest {
                    at: expected_boundary.at,
                    cadences: expected_boundary.cadences.clone(),
                },
                boundary_state_hash_format(expected_boundary.state_hash.as_deref())?,
            )?;
            let Some(actual_boundary) = simulation.boundaries().last() else {
                return Err(CanwuError::new(
                    ErrorCode::ReplayMismatch,
                    "boundary replay did not append settlement evidence",
                ));
            };
            if receipt.boundary_id != expected_boundary.id || actual_boundary != expected_boundary {
                return Err(CanwuError::new(
                    ErrorCode::ReplayMismatch,
                    format!(
                        "regenerated boundary {} did not match its journal evidence",
                        expected_boundary.id
                    ),
                ));
            }
        }
        for record in &attempts[next_attempt..] {
            replay_attempt_record(&mut simulation, record, commands, final_time)?;
        }
        enqueue_replay_ingress_cut(
            &mut simulation,
            ingress,
            &mut next_ingress,
            boundaries.len(),
        )?;
        if next_ingress != ingress.len() {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "ingress journal contains an impossible future boundary issue cut",
            ));
        }
        if simulation.command_log() != commands {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "replayed accepted command journal does not match its recorded evidence",
            ));
        }
        if final_time < simulation.time() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "replay final time cannot precede the last command attempt",
            ));
        }
        if final_time > simulation.time() {
            simulation.ensure_legacy_advance_does_not_cross_ingress(final_time)?;
            simulation.advance_to(final_time)?;
        }
        Ok(simulation)
    }
}

fn enqueue_replay_ingress_cut(
    simulation: &mut Simulation,
    ingress: &[IngressRecord],
    next_ingress: &mut usize,
    boundary_count: usize,
) -> Result<(), CanwuError> {
    let expected_boundary_count = u64::try_from(boundary_count).map_err(|_| {
        CanwuError::new(
            ErrorCode::ReplayMismatch,
            "boundary count exceeds ingress range",
        )
    })?;
    while let Some(record) = ingress.get(*next_ingress) {
        if record.eligible_boundary_count < expected_boundary_count {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "ingress journal skipped its recorded issue boundary",
            ));
        }
        if record.eligible_boundary_count > expected_boundary_count {
            break;
        }
        if record.issued_at < simulation.time() {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "ingress journal issue time precedes replay state",
            ));
        }
        if record.issued_at > simulation.time() {
            simulation.ensure_legacy_advance_does_not_cross_ingress(record.issued_at)?;
            simulation.advance_to(record.issued_at)?;
        }
        if let Some(actual) = simulation.state.evidence.ingress.get(*next_ingress) {
            if actual != record {
                return Err(CanwuError::new(
                    ErrorCode::ReplayMismatch,
                    "plugin-generated ingress does not match journal evidence",
                ));
            }
            *next_ingress += 1;
            continue;
        }
        if matches!(record.cause, Some(CauseRef::Boundary(_))) {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "recorded boundary-generated ingress was not reproduced by its plugin system",
            ));
        }
        let receipt = match &record.payload {
            IngressPayload::Command { request } => simulation.enqueue_command(
                record.due_at,
                record.priority,
                request.as_ref().clone(),
            )?,
            IngressPayload::Plugin {
                plugin,
                packet_type,
                payload,
                affected_entities,
                archive_retention,
            } => {
                let mut request = PluginIngressRequest::new(
                    plugin.clone(),
                    packet_type.clone(),
                    record.due_at,
                    payload.clone(),
                )
                .with_priority(record.priority);
                request.affected_entities.clone_from(affected_entities);
                request.cause.clone_from(&record.cause);
                request.archive_retention.clone_from(archive_retention);
                simulation.replay_plugin_ingress(request)?
            }
            IngressPayload::Calendar { cadences } => {
                simulation.schedule_calendar_boundary(record.due_at, cadences.clone())?
            }
            IngressPayload::Decision { request } => simulation.enqueue_decision(
                record.due_at,
                record.priority,
                request.as_ref().clone(),
            )?,
            IngressPayload::Maintenance { request } => match request.as_ref() {
                super::MaintenanceIngressRequest::DecisionArchive { commit } => simulation
                    .enqueue_decision_archive_commit(
                        record.due_at,
                        record.priority,
                        commit.clone(),
                    )?,
                super::MaintenanceIngressRequest::OwnerAuthorized { commit } => simulation
                    .enqueue_owner_authorized_maintenance(
                        record.due_at,
                        record.priority,
                        commit.clone(),
                    )?,
            },
        };
        if receipt.ingress_id != record.id
            || simulation.state.evidence.ingress.last() != Some(record)
        {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "regenerated ingress record does not match journal evidence",
            ));
        }
        *next_ingress += 1;
    }
    Ok(())
}

fn replay_command_record(
    simulation: &mut Simulation,
    record: &CommandRecord,
    latest_time: SimTime,
) -> Result<(), CanwuError> {
    if record.accepted_at < simulation.time() || record.accepted_at > latest_time {
        return Err(CanwuError::new(
            ErrorCode::ReplayMismatch,
            "replay command timestamps do not match authoritative operation order",
        ));
    }
    simulation.ensure_legacy_advance_does_not_cross_ingress(record.accepted_at)?;
    simulation.advance_to(record.accepted_at)?;
    let CommandOutcome::Accepted { receipt } = simulation.admit_command(
        None,
        None,
        record.envelope.clone(),
        CommandIngress::LegacyDirect,
        None,
        false,
    )?
    else {
        return Err(CanwuError::new(
            ErrorCode::ReplayMismatch,
            "legacy replay command was rejected",
        ));
    };
    if receipt.command_id != record.id {
        return Err(CanwuError::new(
            ErrorCode::ReplayMismatch,
            "replay command IDs did not match the journal",
        ));
    }
    Ok(())
}

fn replay_attempt_record(
    simulation: &mut Simulation,
    record: &CommandAttemptRecord,
    commands: &[CommandRecord],
    latest_time: SimTime,
) -> Result<(), CanwuError> {
    if record.at < simulation.time() || record.at > latest_time {
        return Err(CanwuError::new(
            ErrorCode::ReplayMismatch,
            "replay command-attempt timestamps do not match authoritative operation order",
        ));
    }
    simulation.ensure_legacy_advance_does_not_cross_ingress(record.at)?;
    simulation.advance_to(record.at)?;
    let outcome = simulation.admit_command(
        record.request_id,
        record.expected_revision,
        record.envelope.clone(),
        record.ingress,
        None,
        true,
    )?;
    if simulation.command_attempts().last() != Some(record) {
        return Err(CanwuError::new(
            ErrorCode::ReplayMismatch,
            format!(
                "regenerated command attempt {} did not match its journal evidence",
                record.id
            ),
        ));
    }
    match (&record.outcome, outcome) {
        (CommandAttemptOutcome::Accepted { command_id }, CommandOutcome::Accepted { receipt })
            if receipt.command_id == *command_id =>
        {
            let index = usize::try_from(command_id.get().saturating_sub(1)).map_err(|_| {
                CanwuError::new(
                    ErrorCode::ReplayMismatch,
                    "replayed command ID exceeds the journal index range",
                )
            })?;
            if simulation.command_log().last() != commands.get(index) {
                return Err(CanwuError::new(
                    ErrorCode::ReplayMismatch,
                    "regenerated command record did not match its journal evidence",
                ));
            }
        }
        (
            CommandAttemptOutcome::Rejected { error: expected },
            CommandOutcome::Rejected { rejection },
        ) if rejection.error == *expected => {}
        _ => {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "replayed command-attempt outcome differs from its journal evidence",
            ));
        }
    }
    Ok(())
}
