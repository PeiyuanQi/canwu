use super::event_payloads::{
    ARMY_ARRIVED, ArmyArrived, DEBUG_FIELD_CHANGED, DebugFieldChanged, KNOWLEDGE_PUBLISHED,
    KNOWLEDGE_UPDATED, KnowledgePublished, KnowledgeUpdated, LETTER_DELIVERED, LetterDelivered,
    MOVE_ORDERED, MoveOrdered, PERSON_ARRIVED, PERSON_MOVE_ORDERED, PLUGIN, PersonArrived,
    PersonMoveOrdered, REPORT_DISPATCHED, ReportDispatched, RuntimeEventPayload,
};
use super::{
    ADMISSION_CURSOR_FORMAT_VERSION, ArmyId, BoundaryDirective, BoundaryDomainEntityCuts,
    BoundaryEmissionKind, BoundaryId, BoundaryPhase, BoundaryProposal, BoundaryRecord,
    BoundarySystemContract, COMMITMENT_FORMAT_VERSION, CanwuError, CauseRef, Command,
    CommandAttemptId, CommandAttemptOutcome, CommandAttemptRecord, CommandEnvelope, CommandId,
    CommandIngress, CommandRecord, DECISION_REQUEST_COMMITMENT_DOMAIN, DecisionAction,
    DecisionAttemptErrorCode, DecisionAttemptOutcome, DecisionAttemptRecord, DecisionAuthority,
    DecisionMutation, DecisionOutcome, DecisionPolicyKind, DecisionRandomEvidence, DecisionState,
    DecisionTraceId, DeterministicRng, DomainHistoryCut, DomainRecord, DomainRecordChange,
    DomainRecordClass, DomainRecordCommitStage, DomainRecordHistory, DomainRecordMutation,
    DomainRecordRef, DomainRecordVersionRef, DomainRecordVersionSource, EntityRef, ErrorCode,
    EventId, EvidenceRef, GENESIS_BOUNDARY_HASH, IngressClass, IngressId, IngressPayload,
    IngressQueueKey, IngressRecord, InteractionPolicy, Issuer, KnowledgeHolderPolicy,
    KnowledgeHolderRef, KnowledgeRecord, KnowledgeRecordId, LetterStatus,
    PersistedAdmissionCursors, PersonId, PluginComponentKey, PluginComponentRecord,
    PluginIngressTarget, PluginRegistry, RandomAlgorithm, RandomDrawAddress, RandomDrawId,
    RandomDrawOutcome, RandomDrawProducer, RandomDrawRecord, RandomOperationTarget,
    ReservationAllocation, ReservationDisposition, ReservationPoolKey, ReservationRequestRecord,
    RunConfigurationSnapshot, RunManifest, RuntimeCurrentState, RuntimeState,
    STATE_REVISION_FORMAT_VERSION, ScheduleKey, ScheduledAction, SimDuration, SimEvent,
    SimulationSnapshot, StateVisibility, SystemCadence, SystemDirective, WorldSnapshot,
    authoritative_revision_count, base_schema, boundaries_before_attempts,
    boundary_has_event_ingress, boundary_state_hash_format, boundary_system_due,
    boundary_write_stage, canonical_hash, canonical_text, canonicalize_scenario,
    commitment_roots_are_canonical, component_key, compute_boundary_hash,
    domain_record_commit_stage, invalid_snapshot, invalid_snapshot_error, is_canonical_hash,
    is_domain_record_state, is_expected_command_rejection, manifest, plugins, random,
    record_change_affected_entities, records, snapshot_boundary_head_state_hash,
    snapshot_checkpoint_hash, snapshot_command_attempt_preflight_error, snapshot_commitment_roots,
    snapshot_is_at_boundary_head, validate_command_authority, validate_directives,
    validate_scenario, validate_strict_id_order, validate_type_schema,
};
use std::collections::{BTreeMap, BTreeSet};

/// Describes how an evidence reference can be resolved by a validation backend.
///
/// Runtime state can retain only a suffix of the evidence journals. An
/// archived reference is still valid evidence, but its record is unavailable
/// to the live process until an archive adapter is supplied. Snapshot state,
/// by contrast, normally resolves every record as retained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EvidenceLookup<'a, T> {
    Missing,
    Archived,
    Retained(&'a T),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EvidenceAvailability {
    Missing,
    Archived,
    Retained,
}

/// Shared semantic view used by runtime and snapshot validation.
///
/// The validation rules operate on this interface instead of knowing whether
/// evidence came from the retained runtime tail or a complete snapshot. The
/// two backends therefore differ only in evidence availability, not in the
/// meaning of a cause or directive.
pub(super) trait ValidationContext {
    fn event(&self, id: EventId) -> EvidenceLookup<'_, SimEvent>;
    fn command(&self, id: CommandId) -> EvidenceLookup<'_, CommandRecord>;
    fn command_attempt(&self, id: CommandAttemptId) -> EvidenceLookup<'_, CommandAttemptRecord>;
    fn ingress(&self, id: IngressId) -> EvidenceLookup<'_, IngressRecord>;
    fn boundary(&self, id: BoundaryId) -> EvidenceLookup<'_, BoundaryRecord>;
    fn random_draw(&self, id: RandomDrawId) -> EvidenceLookup<'_, RandomDrawRecord>;
    fn domain_record_version(&self, reference: &DomainRecordVersionRef) -> EvidenceAvailability;
    fn entity_exists(&self, entity: &EntityRef) -> bool;
}

pub(super) struct RuntimeValidationContext<'a> {
    state: &'a RuntimeState,
}

impl<'a> RuntimeValidationContext<'a> {
    pub(super) const fn new(state: &'a RuntimeState) -> Self {
        Self { state }
    }

    fn runtime_lookup<T>(
        id: u64,
        next_id: u64,
        archived_count: u64,
        retained: Option<&T>,
        archived_receipt: bool,
    ) -> EvidenceLookup<'_, T> {
        if id == 0 || id >= next_id {
            EvidenceLookup::Missing
        } else if id <= archived_count {
            if archived_receipt {
                EvidenceLookup::Archived
            } else {
                EvidenceLookup::Missing
            }
        } else if let Some(record) = retained {
            EvidenceLookup::Retained(record)
        } else {
            EvidenceLookup::Missing
        }
    }
}

impl ValidationContext for RuntimeValidationContext<'_> {
    fn event(&self, id: EventId) -> EvidenceLookup<'_, SimEvent> {
        Self::runtime_lookup(
            id.get(),
            self.state.counters.next_event_id,
            self.state.evidence.archived.event_count,
            self.state.evidence.retained_event(id),
            self.state
                .evidence
                .archived_evidence_receipts
                .contains_key(&EvidenceRef::Event(id)),
        )
    }

    fn command(&self, id: CommandId) -> EvidenceLookup<'_, CommandRecord> {
        Self::runtime_lookup(
            id.get(),
            self.state.counters.next_command_id,
            self.state.evidence.archived.command_count,
            self.state.evidence.retained_command(id),
            self.state
                .evidence
                .archived_evidence_receipts
                .contains_key(&EvidenceRef::Command(id)),
        )
    }

    fn command_attempt(&self, id: CommandAttemptId) -> EvidenceLookup<'_, CommandAttemptRecord> {
        Self::runtime_lookup(
            id.get(),
            self.state.counters.next_command_attempt_id,
            self.state.evidence.archived.command_attempt_count,
            self.state.evidence.retained_command_attempt(id),
            self.state
                .evidence
                .archived_evidence_receipts
                .contains_key(&EvidenceRef::CommandAttempt(id)),
        )
    }

    fn ingress(&self, id: IngressId) -> EvidenceLookup<'_, IngressRecord> {
        Self::runtime_lookup(
            id.get(),
            self.state.counters.next_ingress_id,
            self.state.evidence.archived.ingress_count,
            self.state.evidence.retained_ingress(id),
            self.state
                .evidence
                .archived_evidence_receipts
                .contains_key(&EvidenceRef::Ingress(id)),
        )
    }

    fn boundary(&self, id: BoundaryId) -> EvidenceLookup<'_, BoundaryRecord> {
        Self::runtime_lookup(
            id.get(),
            self.state.counters.next_boundary_id,
            self.state.evidence.archived.boundary_count,
            self.state.evidence.retained_boundary(id),
            self.state
                .evidence
                .archived_evidence_receipts
                .contains_key(&EvidenceRef::Boundary(id)),
        )
    }

    fn random_draw(&self, id: RandomDrawId) -> EvidenceLookup<'_, RandomDrawRecord> {
        Self::runtime_lookup(
            id.get(),
            self.state.counters.next_random_draw_id,
            self.state.evidence.archived.random_draw_count,
            self.state.evidence.retained_random_draw(id),
            self.state
                .evidence
                .archived_evidence_receipts
                .contains_key(&EvidenceRef::RandomDraw(id)),
        )
    }

    fn domain_record_version(&self, reference: &DomainRecordVersionRef) -> EvidenceAvailability {
        resolve_runtime_domain_record_version(self.state, reference)
    }

    fn entity_exists(&self, entity: &EntityRef) -> bool {
        runtime_entity_exists(self.state, entity)
    }
}

pub(super) struct SnapshotValidationContext<'a> {
    snapshot: &'a SimulationSnapshot,
}

impl<'a> SnapshotValidationContext<'a> {
    pub(super) const fn new(snapshot: &'a SimulationSnapshot) -> Self {
        Self { snapshot }
    }
}

impl ValidationContext for SnapshotValidationContext<'_> {
    fn event(&self, id: EventId) -> EvidenceLookup<'_, SimEvent> {
        snapshot_event_by_id(self.snapshot, id)
            .map_or(EvidenceLookup::Missing, EvidenceLookup::Retained)
    }

    fn command(&self, id: CommandId) -> EvidenceLookup<'_, CommandRecord> {
        snapshot_command_by_id(self.snapshot, id)
            .map_or(EvidenceLookup::Missing, EvidenceLookup::Retained)
    }

    fn command_attempt(&self, id: CommandAttemptId) -> EvidenceLookup<'_, CommandAttemptRecord> {
        snapshot_command_attempt_by_id(self.snapshot, id)
            .map_or(EvidenceLookup::Missing, EvidenceLookup::Retained)
    }

    fn ingress(&self, id: IngressId) -> EvidenceLookup<'_, IngressRecord> {
        snapshot_ingress_by_id(self.snapshot, id)
            .map_or(EvidenceLookup::Missing, EvidenceLookup::Retained)
    }

    fn boundary(&self, id: BoundaryId) -> EvidenceLookup<'_, BoundaryRecord> {
        snapshot_boundary_by_id(self.snapshot, id)
            .map_or(EvidenceLookup::Missing, EvidenceLookup::Retained)
    }

    fn random_draw(&self, id: RandomDrawId) -> EvidenceLookup<'_, RandomDrawRecord> {
        snapshot_random_draw_by_id(self.snapshot, id)
            .map_or(EvidenceLookup::Missing, EvidenceLookup::Retained)
    }

    fn domain_record_version(&self, reference: &DomainRecordVersionRef) -> EvidenceAvailability {
        resolve_snapshot_domain_record_version(self.snapshot, reference)
    }

    fn entity_exists(&self, entity: &EntityRef) -> bool {
        snapshot_entity_exists(self.snapshot, entity)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CauseValidationError {
    MissingEvidence,
    NonCanonicalSystem,
}

pub(super) fn validate_cause_reference<C: ValidationContext>(
    context: &C,
    cause: &CauseRef,
) -> Result<(), CauseValidationError> {
    let available = match cause {
        CauseRef::Boundary(id) => !matches!(context.boundary(*id), EvidenceLookup::Missing),
        CauseRef::Command(id) => !matches!(context.command(*id), EvidenceLookup::Missing),
        CauseRef::Event(id) => !matches!(context.event(*id), EvidenceLookup::Missing),
        CauseRef::System(name) => canonical_text(name),
    };
    match cause {
        CauseRef::System(_) if !available => Err(CauseValidationError::NonCanonicalSystem),
        _ if !available => Err(CauseValidationError::MissingEvidence),
        _ => Ok(()),
    }
}

pub(super) fn resolve_evidence_reference<C: ValidationContext>(
    context: &C,
    reference: &EvidenceRef,
) -> EvidenceAvailability {
    fn availability<T>(lookup: &EvidenceLookup<'_, T>) -> EvidenceAvailability {
        match lookup {
            EvidenceLookup::Missing => EvidenceAvailability::Missing,
            EvidenceLookup::Archived => EvidenceAvailability::Archived,
            EvidenceLookup::Retained(_) => EvidenceAvailability::Retained,
        }
    }

    match reference {
        EvidenceRef::Command(id) => availability(&context.command(*id)),
        EvidenceRef::CommandAttempt(id) => availability(&context.command_attempt(*id)),
        EvidenceRef::Event(id) => availability(&context.event(*id)),
        EvidenceRef::Ingress(id) => availability(&context.ingress(*id)),
        EvidenceRef::Boundary(id) => availability(&context.boundary(*id)),
        EvidenceRef::RandomDraw(id) => availability(&context.random_draw(*id)),
        EvidenceRef::DomainRecordVersion(reference) => context.domain_record_version(reference),
    }
}

pub(super) fn validate_directives_with_context<C: ValidationContext>(
    context: &C,
    plugin: &str,
    allowed_writes: &[super::StateKey],
    state_owners: &BTreeMap<super::StateKey, String>,
    record_schemas: &records::DomainRecordSchemas,
    directives: &[SystemDirective],
) -> Result<(), CanwuError> {
    validate_directives(
        plugin,
        allowed_writes,
        state_owners,
        record_schemas,
        &|entity| context.entity_exists(entity),
        directives,
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EventCorrelationRoot {
    Command(CommandId),
    Boundary(BoundaryId),
    System(String),
    Event(EventId),
}

pub(super) fn validate_snapshot(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
) -> Result<(), CanwuError> {
    if snapshot.engine_version.trim().is_empty() {
        return invalid_snapshot("snapshot engine version cannot be empty");
    }
    if snapshot.revision_format_version != STATE_REVISION_FORMAT_VERSION {
        return invalid_snapshot("snapshot state revision format is not current");
    }
    if snapshot.replay_revision_format_version > STATE_REVISION_FORMAT_VERSION {
        return invalid_snapshot("snapshot exact-replay revision format is unsupported");
    }
    if snapshot.admission_cursor_format_version != ADMISSION_CURSOR_FORMAT_VERSION {
        return invalid_snapshot("snapshot boundary-admission cursor format is not current");
    }
    let Some(run_manifest) = &snapshot.run_manifest else {
        return Err(CanwuError::new(
            ErrorCode::InvalidRunManifest,
            "snapshot is missing its run manifest",
        ));
    };
    let Some(initial_scenario) = snapshot.initial_scenario.as_ref() else {
        return invalid_snapshot(
            "format 8 snapshots require their manifest-bound initial scenario",
        );
    };
    let mut canonical = initial_scenario.clone();
    canonicalize_scenario(&mut canonical);
    if &canonical != initial_scenario || initial_scenario.start_time != snapshot.initial_time {
        return invalid_snapshot("snapshot initial scenario is not canonical or time-aligned");
    }
    validate_scenario(initial_scenario).map_err(|error| {
        invalid_snapshot_error(format!("snapshot initial scenario is invalid: {error}"))
    })?;
    manifest::validate(run_manifest, Some(initial_scenario))?;
    let initial_domain_records = Some(
        initial_scenario
            .domain_records
            .iter()
            .map(|record| (record.reference.clone(), record.clone()))
            .collect::<BTreeMap<_, _>>(),
    );
    if snapshot
        .initial_scenario
        .as_ref()
        .is_some_and(|scenario| scenario.entities != snapshot.entities)
    {
        return invalid_snapshot(
            "snapshot entity registry does not match its manifest-bound initial scenario",
        );
    }
    if !is_canonical_hash(&snapshot.run_manifest_hash)
        || manifest::hash(run_manifest)? != snapshot.run_manifest_hash
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidRunManifest,
            "snapshot run manifest hash is inconsistent",
        ));
    }
    let Some(run_configuration) = &snapshot.run_configuration else {
        return Err(CanwuError::new(
            ErrorCode::InvalidRunConfiguration,
            "snapshot is missing its run configuration",
        ));
    };
    manifest::validate_run_configuration(run_manifest, run_configuration)?;
    let (configuration_entities, configuration_world, configuration_records) =
        snapshot.initial_scenario.as_ref().map_or(
            (
                snapshot.entities.as_slice(),
                &snapshot.world,
                snapshot.domain_records.as_slice(),
            ),
            |scenario| {
                (
                    scenario.entities.as_slice(),
                    &scenario.world,
                    scenario.domain_records.as_slice(),
                )
            },
        );
    validate_run_configuration_entities(
        run_configuration,
        configuration_entities,
        configuration_world,
        configuration_records,
    )?;
    validate_run_configuration_entities(
        run_configuration,
        &snapshot.entities,
        &snapshot.world,
        &snapshot.domain_records,
    )?;
    if matches!(run_configuration, RunConfigurationSnapshot::Declared(_))
        && !snapshot.commands.is_empty()
        && snapshot.command_attempts.is_empty()
    {
        return invalid_snapshot(
            "declared runs cannot contain accepted commands without tracked attempt evidence",
        );
    }
    if !snapshot.ingress.is_empty()
        && has_unqueued_command_history(
            &snapshot.commands,
            &snapshot.command_attempts,
            &snapshot.ingress,
        )
    {
        return invalid_snapshot("canonical ingress cannot coexist with direct command history");
    }
    if snapshot.initial_time > snapshot.now {
        return invalid_snapshot("snapshot initial time cannot follow its current time");
    }
    let has_execution_evidence = snapshot.now != snapshot.initial_time
        || !snapshot.commands.is_empty()
        || !snapshot.command_attempts.is_empty()
        || !snapshot.ingress.is_empty()
        || !snapshot.events.is_empty()
        || !snapshot.boundaries.is_empty()
        || !snapshot.plugin_components.is_empty()
        || !snapshot.random_draws.is_empty()
        || snapshot
            .random_streams
            .iter()
            .any(|stream| stream.position != 0)
        || !snapshot.scheduled.is_empty()
        || snapshot.next_event_id != 1
        || snapshot.next_command_id != 1
        || snapshot.next_command_attempt_id != 1
        || snapshot.next_ingress_id != 1
        || snapshot.next_boundary_id != 1
        || snapshot.next_random_draw_id != 1
        || snapshot.next_knowledge_record_id != 1
        || !snapshot.knowledge.records.is_empty()
        || snapshot.next_schedule_sequence != 1
        || snapshot.next_correlation_id != 1
        || !snapshot.decisions.is_empty()
        || snapshot.next_decision_trace_id != 1;
    if has_execution_evidence && !snapshot.plugin_registration_closed {
        return invalid_snapshot(
            "snapshot execution evidence requires plugin registration to remain closed",
        );
    }
    validate_strict_id_order(&snapshot.world.people, |value| value.id, "people")?;
    validate_strict_id_order(&snapshot.world.governments, |value| value.id, "governments")?;
    validate_strict_id_order(&snapshot.world.territories, |value| value.id, "territories")?;
    validate_strict_id_order(&snapshot.world.routes, |value| value.id, "routes")?;
    validate_strict_id_order(&snapshot.world.armies, |value| value.id, "armies")?;
    validate_strict_id_order(&snapshot.world.letters, |value| value.id, "letters")?;
    let domain_records = validate_snapshot_domain_records(snapshot, plugins)?;
    let max_knowledge_record_id = super::knowledge::validate_snapshot_records(
        snapshot,
        &plugins.knowledge_schemas,
        &plugins.record_schemas,
    )
    .map_err(|error| {
        invalid_snapshot_error(format!("invalid generic knowledge ledger: {error}"))
    })?;
    let evidence_context = SnapshotValidationContext::new(snapshot);
    for records in snapshot.knowledge.records.values() {
        for record in records.values() {
            for reference in &record.origin.evidence {
                if resolve_evidence_reference(&evidence_context, reference)
                    != EvidenceAvailability::Retained
                {
                    return invalid_snapshot(
                        "generic knowledge origin references missing or wrong-version evidence",
                    );
                }
            }
        }
    }
    let (max_boundary_id, max_boundary_correlation, domain_history, admission_cursors) =
        validate_boundary_records(
            snapshot,
            plugins,
            &domain_records,
            initial_domain_records.as_ref(),
        )?;
    if snapshot.admitted_attempt_count != admission_cursors.attempts
        || snapshot.admitted_command_count != admission_cursors.commands
        || snapshot.admitted_event_count != admission_cursors.events
    {
        return invalid_snapshot(
            "persisted admission cursors do not match the globally admitted journal prefixes",
        );
    }
    let max_ingress_id = validate_ingress_records(snapshot, plugins, &domain_history)?;
    validate_decision_state(snapshot)?;
    let boundaries_before_attempt =
        boundaries_before_attempts(snapshot.command_attempts.len(), &snapshot.boundaries)?;
    let mut request_ids = BTreeSet::new();
    let mut accepted_attempts = BTreeMap::new();
    let mut command_boundary_counts = BTreeMap::new();
    let mut accepted_command_count = 0_u64;
    let mut previous_attempt = None;
    for (index, attempt) in snapshot.command_attempts.iter().enumerate() {
        let expected_id = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| {
                invalid_snapshot_error("command attempt index exceeds identifier space")
            })?;
        let expected_revision = u64::try_from(index)
            .map_err(|_| invalid_snapshot_error("command attempt index exceeds revision space"))?
            .checked_add(boundaries_before_attempt[index])
            .ok_or_else(|| invalid_snapshot_error("command revision space is exhausted"))?;
        if attempt.id.get() != expected_id
            || attempt.at < snapshot.initial_time
            || attempt.at > snapshot.now
            || attempt.revision_before != expected_revision
            || attempt.request_id.is_some() != attempt.expected_revision.is_some()
            || previous_attempt.is_some_and(|(at, id)| (attempt.at, attempt.id) <= (at, id))
            || attempt.request_id.is_some_and(|id| !request_ids.insert(id))
        {
            return invalid_snapshot("command attempt journal is not canonical");
        }
        let boundaries_before =
            usize::try_from(boundaries_before_attempt[index]).map_err(|_| {
                invalid_snapshot_error("command attempt boundary cut exceeds platform index space")
            })?;
        let attempt_cut = DomainHistoryCut::after_boundaries(boundaries_before);
        let preflight_error = snapshot_command_attempt_preflight_error(
            snapshot,
            attempt,
            &domain_history,
            attempt_cut,
        );
        match &attempt.outcome {
            CommandAttemptOutcome::Accepted { command_id } => {
                if preflight_error.is_some() {
                    return invalid_snapshot(
                        "accepted command attempt violates its recorded ingress policy",
                    );
                }
                let Some(next_command_count) = accepted_command_count.checked_add(1) else {
                    return invalid_snapshot("command identifier space is exhausted");
                };
                if command_id.get() != next_command_count
                    || attempt
                        .expected_revision
                        .is_some_and(|expected| expected != expected_revision)
                    || accepted_attempts.insert(*command_id, attempt).is_some()
                    || command_boundary_counts
                        .insert(*command_id, boundaries_before)
                        .is_some()
                {
                    return invalid_snapshot(
                        "accepted command attempt does not match command revision order",
                    );
                }
                accepted_command_count = next_command_count;
            }
            CommandAttemptOutcome::Rejected { error } => {
                if !is_expected_command_rejection(&error.code) {
                    return invalid_snapshot(
                        "command attempt journal contains a non-rejection engine failure",
                    );
                }
                if preflight_error
                    .as_ref()
                    .is_some_and(|expected| expected != error)
                {
                    return invalid_snapshot(
                        "rejected command attempt disagrees with deterministic ingress validation",
                    );
                }
            }
        }
        previous_attempt = Some((attempt.at, attempt.id));
    }
    if !snapshot.command_attempts.is_empty()
        && accepted_command_count
            != u64::try_from(snapshot.commands.len())
                .map_err(|_| invalid_snapshot_error("command count exceeds the revision range"))?
    {
        return invalid_snapshot("accepted command attempts do not cover the command journal");
    }
    let mut command_ids = BTreeSet::new();
    let mut previous_command = None;
    for (index, record) in snapshot.commands.iter().enumerate() {
        let expected_id = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| invalid_snapshot_error("command index exceeds identifier space"))?;
        if record.id.get() != expected_id || !command_ids.insert(record.id) {
            return invalid_snapshot("command IDs must be contiguous, unique, and nonzero");
        }
        if record.accepted_at < snapshot.initial_time
            || record.accepted_at > snapshot.now
            || record
                .envelope
                .expected_time
                .is_some_and(|expected| expected != record.accepted_at)
        {
            return invalid_snapshot("command timestamps are invalid");
        }
        if snapshot.command_attempts.is_empty() {
            if record.attempt_id.is_some() || !record.emitted_events.is_empty() {
                return invalid_snapshot(
                    "legacy commands cannot contain partial command-attempt evidence",
                );
            }
        } else {
            let Some(attempt) = accepted_attempts.get(&record.id) else {
                return invalid_snapshot("command is missing its accepted attempt evidence");
            };
            if record.attempt_id != Some(attempt.id)
                || record.accepted_at != attempt.at
                || record.envelope != attempt.envelope
                || record
                    .emitted_events
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
            {
                return invalid_snapshot("command and attempt evidence disagree");
            }
        }
        if previous_command.is_some_and(|(time, id)| (record.accepted_at, record.id) <= (time, id))
        {
            return invalid_snapshot("command records are not in canonical order");
        }
        let boundaries_before = command_boundary_counts
            .get(&record.id)
            .copied()
            .unwrap_or_else(|| boundaries_before_legacy_command(snapshot, record));
        let command_cut = DomainHistoryCut::after_boundaries(boundaries_before);
        validate_snapshot_command(
            snapshot,
            plugins,
            &record.envelope,
            &domain_history,
            command_cut,
        )?;
        previous_command = Some((record.accepted_at, record.id));
    }

    let mut boundary_event_owners = vec![None; snapshot.events.len()];
    for boundary in &snapshot.boundaries {
        for emission in &boundary.emissions {
            if let Some(owner) =
                event_index(emission.event).and_then(|index| boundary_event_owners.get_mut(index))
            {
                owner.get_or_insert(boundary.id);
            }
        }
    }
    let mut event_command_roots = Vec::<Option<CommandId>>::with_capacity(snapshot.events.len());
    let mut event_correlation_roots =
        Vec::<EventCorrelationRoot>::with_capacity(snapshot.events.len());
    let mut correlation_roots = BTreeMap::<u64, EventCorrelationRoot>::new();
    let mut command_emitted_events = BTreeMap::<CommandId, Vec<EventId>>::new();
    let snapshot_context = SnapshotValidationContext::new(snapshot);
    let mut previous_event = None;
    for (index, event) in snapshot.events.iter().enumerate() {
        let expected_id = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| invalid_snapshot_error("event index exceeds identifier space"))?;
        if event.id.get() != expected_id || event.correlation_id == 0 {
            return invalid_snapshot("event IDs must be contiguous, unique, and nonzero");
        }
        if event.timestamp < snapshot.initial_time
            || event.timestamp > snapshot.now
            || previous_event.is_some_and(|(time, id)| (event.timestamp, event.id) <= (time, id))
        {
            return invalid_snapshot("events are not in canonical timestamp and ID order");
        }
        if let Some(cause) = &event.cause {
            validate_cause_reference(&snapshot_context, cause).map_err(|error| {
                invalid_snapshot_error(match error {
                    CauseValidationError::MissingEvidence => {
                        "event references unavailable evidence"
                    }
                    CauseValidationError::NonCanonicalSystem => {
                        "event system cause is not canonical"
                    }
                })
            })?;
        }

        let (command_root, correlation_root) = match &event.cause {
            Some(CauseRef::Command(command_id)) => {
                let Some(command) =
                    journal_record_by_id(&snapshot.commands, command_id.get(), |record| {
                        record.id.get()
                    })
                else {
                    return invalid_snapshot("event references an unknown command cause");
                };
                if command.accepted_at > event.timestamp {
                    return invalid_snapshot("event references a future command cause");
                }
                (
                    Some(*command_id),
                    EventCorrelationRoot::Command(*command_id),
                )
            }
            Some(CauseRef::Event(parent_id)) => {
                let parent_index = event_index(*parent_id).ok_or_else(|| {
                    invalid_snapshot_error("event cause chain references an invalid event ID")
                })?;
                if parent_index >= index {
                    return invalid_snapshot("event references an invalid parent event");
                }
                let parent = &snapshot.events[parent_index];
                if parent.correlation_id != event.correlation_id {
                    return invalid_snapshot("event parent and child must share a correlation ID");
                }
                (
                    event_command_roots[parent_index],
                    event_correlation_roots[parent_index].clone(),
                )
            }
            Some(CauseRef::Boundary(boundary_id)) => {
                let Some(boundary) =
                    journal_record_by_id(&snapshot.boundaries, boundary_id.get(), |record| {
                        record.id.get()
                    })
                else {
                    return invalid_snapshot("event references an unknown boundary cause");
                };
                if boundary.at != event.timestamp
                    || boundary.correlation_id != event.correlation_id
                    || boundary_event_owners[index] != Some(*boundary_id)
                {
                    return invalid_snapshot(
                        "event boundary cause does not own the event or correlation",
                    );
                }
                (None, EventCorrelationRoot::Boundary(*boundary_id))
            }
            Some(CauseRef::System(name)) => {
                if !canonical_text(name) {
                    return invalid_snapshot("event system cause is not canonical");
                }
                (None, EventCorrelationRoot::System(name.clone()))
            }
            None => (None, EventCorrelationRoot::Event(event.id)),
        };
        if let Some(existing) = correlation_roots.get(&event.correlation_id) {
            if existing != &correlation_root {
                return invalid_snapshot("correlation ID is shared by unrelated causal roots");
            }
        } else {
            correlation_roots.insert(event.correlation_id, correlation_root.clone());
        }
        if let Some(command_id) = command_root {
            let command = journal_record_by_id(&snapshot.commands, command_id.get(), |record| {
                record.id.get()
            })
            .ok_or_else(|| invalid_snapshot_error("event references an unknown command root"))?;
            if event.timestamp == command.accepted_at {
                command_emitted_events
                    .entry(command_id)
                    .or_default()
                    .push(event.id);
            }
        }
        let event_entities = if matches!(event.cause, Some(CauseRef::Boundary(_))) {
            None
        } else if let Some(command_id) = command_root {
            let command = journal_record_by_id(&snapshot.commands, command_id.get(), |record| {
                record.id.get()
            })
            .ok_or_else(|| invalid_snapshot_error("event references an unknown command root"))?;
            if event.timestamp == command.accepted_at {
                let boundaries_before =
                    if let Some(count) = command_boundary_counts.get(&command_id) {
                        *count
                    } else {
                        boundaries_before_legacy_command(snapshot, command)
                    };
                Some(DomainHistoryCut::after_boundaries(boundaries_before))
            } else {
                Some(DomainRecordHistory::before_time(snapshot, event.timestamp))
            }
        } else {
            Some(DomainRecordHistory::before_time(snapshot, event.timestamp))
        };
        let entity_exists = |entity: &EntityRef| {
            event_entities.map_or_else(
                || snapshot_entity_identity_exists(snapshot, entity),
                |cut| snapshot_entity_exists_in_history(snapshot, &domain_history, cut, entity),
            )
        };
        if event
            .affected_entities
            .iter()
            .any(|entity| !entity_exists(entity))
        {
            return invalid_snapshot("event references an unknown entity");
        }
        validate_event_kind(snapshot, plugins, event, &entity_exists)?;
        event_command_roots.push(command_root);
        event_correlation_roots.push(correlation_root);
        previous_event = Some((event.timestamp, event.id));
    }
    for command in &snapshot.commands {
        if snapshot.command_attempts.is_empty() {
            continue;
        }
        let expected_events = command_emitted_events
            .get(&command.id)
            .map_or(&[][..], Vec::as_slice);
        if command.emitted_events.as_slice() != expected_events {
            return invalid_snapshot(
                "command receipt events do not match their synchronous causal evidence",
            );
        }
    }
    let (max_random_draw_id, max_random_correlation) = validate_random_evidence(snapshot, plugins)?;
    if snapshot.commitment_format_version == COMMITMENT_FORMAT_VERSION {
        let persisted_roots = snapshot.commitment_roots.as_ref().ok_or_else(|| {
            invalid_snapshot_error("current commitment snapshot is missing its domain roots")
        })?;
        let calculated_roots = snapshot_commitment_roots(snapshot)?;
        if !commitment_roots_are_canonical(persisted_roots) || calculated_roots != *persisted_roots
        {
            return invalid_snapshot(
                "persisted commitment roots do not match the canonical snapshot domains",
            );
        }
    } else if snapshot.commitment_format_version != 0 || snapshot.commitment_roots.is_some() {
        return invalid_snapshot("snapshot commitment metadata is inconsistent");
    }
    let expected_checkpoint_hash = snapshot_checkpoint_hash(snapshot)?;
    if !is_canonical_hash(&snapshot.checkpoint_hash)
        || expected_checkpoint_hash != snapshot.checkpoint_hash
    {
        return invalid_snapshot(
            "checkpoint hash does not bind the persisted state to its boundary head",
        );
    }
    if snapshot_is_at_boundary_head(snapshot)
        && snapshot
            .boundaries
            .last()
            .is_some_and(|record| record.state_hash.is_some())
    {
        let expected_state_hash = snapshot_boundary_head_state_hash(snapshot)?;
        if snapshot
            .boundaries
            .last()
            .and_then(|record| record.state_hash.as_deref())
            != Some(expected_state_hash.as_str())
        {
            return invalid_snapshot(
                "boundary-head state commitment does not match persisted state",
            );
        }
    }
    let mut component_keys = BTreeSet::new();
    let mut previous_component = None;
    for record in &snapshot.plugin_components {
        if !canonical_text(&record.plugin)
            || !canonical_text(&record.component)
            || !plugins.descriptors.contains_key(&record.plugin)
            || !snapshot_entity_exists(snapshot, &record.entity)
            || plugins.state_owners.get(&record.state) != Some(&record.plugin)
            || is_domain_record_state(&plugins.record_schemas, &record.state)
            || (!plugins.immediate_write_states.contains_key(&record.state)
                && !plugins
                    .boundary_writers
                    .keys()
                    .any(|(_, state)| state == &record.state))
        {
            return invalid_snapshot("plugin component record is not owned or well formed");
        }
        let key = component_key(
            &record.plugin,
            &record.state,
            &record.entity,
            &record.component,
        );
        if previous_component
            .as_ref()
            .is_some_and(|previous| previous >= &key)
            || !component_keys.insert(key.clone())
        {
            return invalid_snapshot("snapshot contains duplicate plugin component records");
        }
        previous_component = Some(key);
    }

    let core_schema = base_schema();
    for required in core_schema.iter() {
        if snapshot.schema.get(&required.type_name) != Some(required) {
            return invalid_snapshot("snapshot is missing an exact core schema definition");
        }
    }
    let mut declared_plugin_schema = BTreeSet::new();
    for descriptor in plugins.descriptors.values() {
        for type_name in &descriptor.schema_types {
            if snapshot.schema.get(type_name).is_none() {
                return invalid_snapshot("plugin descriptor references a missing schema type");
            }
            declared_plugin_schema.insert(type_name.as_str());
        }
    }
    for schema in snapshot.schema.iter() {
        validate_type_schema(schema).map_err(|error| {
            invalid_snapshot_error(format!("snapshot schema is invalid: {error}"))
        })?;
    }
    if snapshot.schema.iter().any(|schema| {
        core_schema.get(&schema.type_name).is_none()
            && !declared_plugin_schema.contains(schema.type_name.as_str())
    }) {
        return invalid_snapshot("snapshot contains an unclaimed schema definition");
    }

    let mut schedule_keys = BTreeSet::new();
    let mut previous_schedule = None;
    let mut pending_arrivals = BTreeMap::<ArmyId, usize>::new();
    let mut pending_person_arrivals = BTreeMap::<PersonId, usize>::new();
    let mut pending_reports = BTreeSet::new();
    let mut max_schedule_sequence = 0;
    let mut max_correlation_id = snapshot
        .events
        .iter()
        .map(|event| event.correlation_id)
        .max()
        .unwrap_or(0)
        .max(max_boundary_correlation)
        .max(max_random_correlation);
    for record in &snapshot.scheduled {
        if record.key.at <= snapshot.now
            || record.key.sequence == 0
            || previous_schedule
                .as_ref()
                .is_some_and(|previous| previous >= &record.key)
            || !schedule_keys.insert(record.key.clone())
        {
            return invalid_snapshot("scheduled work is not future-dated or has a duplicate key");
        }
        previous_schedule = Some(record.key.clone());
        max_schedule_sequence = max_schedule_sequence.max(record.key.sequence);
        let correlation_id = scheduled_correlation_id(&record.action);
        if correlation_id == 0 {
            return invalid_snapshot("scheduled work correlation IDs must be nonzero");
        }
        max_correlation_id = max_correlation_id.max(correlation_id);
        match &record.action {
            ScheduledAction::ArmyArrival { army, .. } => {
                *pending_arrivals.entry(*army).or_default() += 1;
            }
            ScheduledAction::PersonArrival { person, .. } => {
                *pending_person_arrivals.entry(*person).or_default() += 1;
            }
            ScheduledAction::KnowledgeReport { dispatch_event, .. } => {
                if !pending_reports.insert(*dispatch_event) {
                    return invalid_snapshot(
                        "multiple pending reports reference the same dispatch event",
                    );
                }
            }
            ScheduledAction::PluginDirective { .. } => {}
        }
        validate_scheduled_action(snapshot, plugins, &record.key, &record.action)?;
    }
    for army in &snapshot.world.armies {
        let pending = pending_arrivals.get(&army.id).copied().unwrap_or(0);
        if (army.transit.is_some() && pending != 1) || (army.transit.is_none() && pending != 0) {
            return invalid_snapshot(
                "army transit state must have exactly one matching pending arrival",
            );
        }
    }
    for person in &snapshot.world.people {
        let pending = pending_person_arrivals
            .get(&person.id)
            .copied()
            .unwrap_or(0);
        if (person.transit.is_some() && pending != 1) || (person.transit.is_none() && pending != 0)
        {
            return invalid_snapshot(
                "person transit state must have exactly one matching pending arrival",
            );
        }
    }
    for dispatch in snapshot
        .events
        .iter()
        .filter(|event| event.kind.is_type(REPORT_DISPATCHED))
    {
        let ReportDispatched {
            recipient,
            army,
            arrives_at,
        } = ReportDispatched::decode(&dispatch.kind)
            .map_err(|_| invalid_snapshot_error("report dispatch payload is malformed"))?;
        let Some(CauseRef::Event(arrival_id)) = dispatch.cause else {
            return invalid_snapshot("report dispatch must be caused by an army arrival");
        };
        let Some(arrival) = snapshot_event_by_id(snapshot, arrival_id) else {
            return invalid_snapshot("report dispatch references a missing army arrival");
        };
        let ArmyArrived {
            army: arrived_army,
            territory: arrived_location,
        } = ArmyArrived::decode(&arrival.kind).map_err(|_| {
            invalid_snapshot_error("report dispatch cause is not an army arrival event")
        })?;
        if arrived_army != army
            || arrival.timestamp != dispatch.timestamp
            || arrival.correlation_id != dispatch.correlation_id
        {
            return invalid_snapshot("report dispatch disagrees with its army arrival cause");
        }
        let delivery_events: Vec<_> = snapshot
            .events
            .iter()
            .filter(|event| {
                event.cause == Some(CauseRef::Event(dispatch.id))
                    && event.kind.is_type(KNOWLEDGE_UPDATED)
            })
            .collect();
        if delivery_events.iter().any(|event| {
            let Ok(KnowledgeUpdated {
                recipient: delivered_recipient,
                army: delivered_army,
                known_location,
            }) = KnowledgeUpdated::decode(&event.kind)
            else {
                return true;
            };
            delivered_recipient != recipient
                || delivered_army != army
                || known_location != arrived_location
                || event.timestamp != arrives_at
                || event.correlation_id != dispatch.correlation_id
        }) {
            return invalid_snapshot("report delivery disagrees with its dispatch event");
        }
        let deliveries = delivery_events.len();
        let pending = pending_reports.contains(&dispatch.id);
        let coherent = match arrives_at.cmp(&snapshot.now) {
            std::cmp::Ordering::Greater => pending && deliveries == 0,
            std::cmp::Ordering::Less => !pending && deliveries == 1,
            std::cmp::Ordering::Equal => usize::from(pending) + deliveries == 1,
        };
        if !coherent {
            return invalid_snapshot(
                "report dispatch must have exactly one pending or completed delivery",
            );
        }
    }

    validate_contiguous_or_exhausted_next_counter(
        snapshot.next_event_id,
        snapshot
            .events
            .iter()
            .map(|event| event.id.get())
            .max()
            .unwrap_or(0),
        "event",
    )?;
    validate_contiguous_or_exhausted_next_counter(
        snapshot.next_command_id,
        snapshot
            .commands
            .iter()
            .map(|command| command.id.get())
            .max()
            .unwrap_or(0),
        "command",
    )?;
    validate_contiguous_or_exhausted_next_counter(
        snapshot.next_command_attempt_id,
        snapshot
            .command_attempts
            .last()
            .map_or(0, |attempt| attempt.id.get()),
        "command attempt",
    )?;
    validate_contiguous_or_exhausted_next_counter(
        snapshot.next_ingress_id,
        max_ingress_id,
        "ingress",
    )?;
    validate_contiguous_next_counter(snapshot.next_boundary_id, max_boundary_id, "boundary")?;
    validate_contiguous_or_exhausted_next_counter(
        snapshot.next_random_draw_id,
        max_random_draw_id,
        "random draw",
    )?;
    validate_contiguous_or_exhausted_next_counter(
        snapshot.next_knowledge_record_id,
        max_knowledge_record_id,
        "knowledge record",
    )?;
    validate_next_counter(
        snapshot.next_schedule_sequence,
        max_schedule_sequence,
        "schedule sequence",
    )?;
    let authoritative_commit_count = u64::try_from(snapshot.commands.len())
        .ok()
        .and_then(|commands| {
            u64::try_from(snapshot.boundaries.len())
                .ok()
                .and_then(|boundaries| commands.checked_add(boundaries))
        })
        .ok_or_else(|| {
            invalid_snapshot_error("authoritative commit count exceeds revision space")
        })?;
    validate_contiguous_next_counter(
        snapshot.next_correlation_id,
        authoritative_commit_count,
        "correlation",
    )?;
    if max_correlation_id > authoritative_commit_count {
        return invalid_snapshot("causal evidence references an uncommitted correlation");
    }
    let expected_state_revision = authoritative_revision_count(
        snapshot.commands.len(),
        snapshot.command_attempts.len(),
        snapshot.boundaries.len(),
    )?;
    if snapshot.state_revision != expected_state_revision {
        return invalid_snapshot(
            "persisted state revision does not match committed command, rejection, and boundary evidence",
        );
    }
    Ok(())
}

fn validate_random_evidence(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
) -> Result<(u64, u64), CanwuError> {
    if snapshot.legacy_rng.is_some() {
        return invalid_snapshot("current snapshots cannot retain the legacy global RNG");
    }
    if snapshot
        .random_streams
        .windows(2)
        .any(|pair| pair[0].key >= pair[1].key)
    {
        return invalid_snapshot("random streams are not in canonical order");
    }
    let expected_streams: BTreeSet<_> = std::iter::once(random::core_report_delay_stream())
        .chain(plugins.random_stream_owners.keys().cloned())
        .collect();
    let actual_streams: BTreeSet<_> = snapshot
        .random_streams
        .iter()
        .map(|state| state.key.clone())
        .collect();
    if actual_streams != expected_streams
        || snapshot
            .random_streams
            .iter()
            .any(|state| !state.is_coherent(snapshot.root_seed))
    {
        return invalid_snapshot("random stream state or ownership is inconsistent");
    }

    let mut boundary_draws = BTreeMap::new();
    for boundary in &snapshot.boundaries {
        if boundary
            .random_draws
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return invalid_snapshot("boundary random draw IDs are not canonical");
        }
        for id in &boundary.random_draws {
            if boundary_draws.insert(*id, boundary.id).is_some()
                || snapshot
                    .random_draws
                    .get(usize::try_from(id.get().saturating_sub(1)).unwrap_or(usize::MAX))
                    .is_none_or(|draw| draw.id != *id)
            {
                return invalid_snapshot("boundary references an unknown or duplicate random draw");
            }
        }
    }

    let mut replayed: BTreeMap<_, _> = snapshot
        .random_streams
        .iter()
        .map(|state| (state.key.clone(), (0_u64, state.seed, state.algorithm)))
        .collect();
    let mut previous_draw = None;
    let mut max_correlation_id = 0;
    let core_stream = random::core_report_delay_stream();
    let mut report_draws = BTreeMap::new();
    random::retained_keyed_draws(&snapshot.random_draws).map_err(|error| {
        invalid_snapshot_error(format!("invalid operation-keyed random index: {error}"))
    })?;
    let evidence_context = SnapshotValidationContext::new(snapshot);
    for (index, draw) in snapshot.random_draws.iter().enumerate() {
        let expected_id = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| invalid_snapshot_error("random draw index exceeds identifier space"))?;
        if draw.id.get() != expected_id
            || draw.at < snapshot.initial_time
            || draw.at > snapshot.now
            || draw.correlation_id == 0
            || draw.upper_exclusive == 0
            || draw.value >= draw.upper_exclusive
            || draw.purpose.trim().is_empty()
            || draw.purpose != draw.purpose.trim()
            || previous_draw.is_some_and(|(at, id)| (draw.at, draw.id) <= (at, id))
        {
            return invalid_snapshot("random draw journal is not canonical");
        }
        let Some((position, generator_state, algorithm)) = replayed.get_mut(&draw.stream) else {
            return invalid_snapshot("random draw references an unknown stream");
        };
        match &draw.address {
            RandomDrawAddress::Sequential {
                position: draw_position,
            } => {
                if draw.operation_evidence.is_some() || *draw_position != *position {
                    return invalid_snapshot("random draw positions are not contiguous per stream");
                }
                let mut generator = DeterministicRng::from_seed(*generator_state);
                let value = match algorithm {
                    RandomAlgorithm::SplitMix64V1 => generator.range_modulo(draw.upper_exclusive),
                    RandomAlgorithm::SplitMix64V2 => generator.range(draw.upper_exclusive),
                };
                if value != draw.value {
                    return invalid_snapshot("random draw value does not match its stream state");
                }
                *position = position.checked_add(1).ok_or_else(|| {
                    invalid_snapshot_error("random stream position exceeds identifier space")
                })?;
                *generator_state = generator.state();
            }
            RandomDrawAddress::OperationV1(address) => {
                random::validate_operation_draw(snapshot.root_seed, draw).map_err(|error| {
                    invalid_snapshot_error(format!("invalid operation-keyed random draw: {error}"))
                })?;
                let Some(reference) = &draw.operation_evidence else {
                    return invalid_snapshot("operation-keyed draw lacks evidence");
                };
                if resolve_evidence_reference(&evidence_context, reference)
                    != EvidenceAvailability::Retained
                {
                    return invalid_snapshot(
                        "operation-keyed draw references missing or wrong-version evidence",
                    );
                }
                let RandomDrawProducer::BoundarySystem { plugin, .. } = &draw.producer else {
                    return invalid_snapshot(
                        "operation-keyed draws must be produced by a declared plugin system",
                    );
                };
                if address.producer_plugin != *plugin {
                    return invalid_snapshot(
                        "operation-keyed draw address disagrees with its producer plugin",
                    );
                }
            }
        }

        match &draw.producer {
            RandomDrawProducer::BoundarySystem {
                boundary,
                plugin,
                system,
            } => {
                let Some(record) = snapshot_boundary_by_id(snapshot, *boundary) else {
                    return invalid_snapshot("random draw references an unknown boundary");
                };
                let Some(contract) = snapshot_boundary_contract(plugins, plugin, system) else {
                    return invalid_snapshot("random draw references an unknown boundary system");
                };
                let outcome_is_valid = match &draw.outcome {
                    Some(RandomDrawOutcome::BoundarySystemDecision) => true,
                    Some(RandomDrawOutcome::DecisionSelection {
                        ticket_id,
                        ticket_version,
                        option_id,
                    }) => record.generated_ingress.iter().any(|generation| {
                        if generation.plugin != *plugin || generation.system != *system {
                            return false;
                        }
                        let index = usize::try_from(generation.ingress.get().saturating_sub(1))
                            .unwrap_or(usize::MAX);
                        let Some(IngressPayload::Decision { request }) =
                            snapshot.ingress.get(index).map(|ingress| &ingress.payload)
                        else {
                            return false;
                        };
                        matches!(
                            &request.mutation,
                            DecisionMutation::Resolve {
                                ticket_id: request_ticket,
                                expected_version,
                                decision,
                                ..
                            } if request_ticket == ticket_id
                                && expected_version == ticket_version
                                && matches!(
                                    &decision.outcome,
                                    DecisionOutcome::Selected {
                                        option_id: request_option,
                                    } if request_option == option_id
                                )
                                && decision.random.as_ref().is_some_and(|evidence| {
                                    evidence.draw_id == draw.id
                                        && evidence.value == draw.value
                                        && evidence.upper_exclusive == draw.upper_exclusive
                                })
                        )
                    }),
                    Some(RandomDrawOutcome::KnowledgeReportDelivery { .. }) | None => false,
                };
                if boundary_draws.get(&draw.id) != Some(boundary)
                    || draw.at != record.at
                    || draw.correlation_id != record.correlation_id
                    || draw.cause != CauseRef::Boundary(*boundary)
                    || !outcome_is_valid
                    || !contract.random_streams.contains(&draw.stream)
                    || !boundary_system_due(
                        contract,
                        &record.cadences,
                        boundary_has_event_ingress(record),
                    )
                    || plugins.random_stream_owners.get(&draw.stream)
                        != Some(&(plugin.clone(), system.clone()))
                {
                    return invalid_snapshot("boundary random draw provenance is inconsistent");
                }
            }
            RandomDrawProducer::CoreSystem { system } => {
                let CauseRef::Event(cause) = draw.cause else {
                    return invalid_snapshot("core random draw lacks an event cause");
                };
                let Some(event) = snapshot_event_by_id(snapshot, cause) else {
                    return invalid_snapshot("core random draw references an unknown event");
                };
                let ArmyArrived {
                    army: arrived_army, ..
                } = ArmyArrived::decode(&event.kind).map_err(|_| {
                    invalid_snapshot_error("core random draw cause is not an army arrival")
                })?;
                let Some(RandomDrawOutcome::KnowledgeReportDelivery {
                    recipient,
                    army,
                    dispatch_event,
                    arrives_at,
                }) = &draw.outcome
                else {
                    return invalid_snapshot("core random draw lacks report-delivery evidence");
                };
                let Some(dispatch) = snapshot_event_by_id(snapshot, *dispatch_event) else {
                    return invalid_snapshot("core random draw outcome references a missing event");
                };
                let expected_arrives_at = draw
                    .at
                    .checked_add(SimDuration::hours(36))
                    .and_then(|time| {
                        i64::try_from(draw.value)
                            .ok()
                            .and_then(|value| time.checked_add(SimDuration::minutes(value)))
                    })
                    .ok_or_else(|| {
                        invalid_snapshot_error("core random draw value exceeds time range")
                    })?;
                if boundary_draws.contains_key(&draw.id)
                    || system != "canwu.core.knowledge-report-delay"
                    || draw.stream != core_stream
                    || draw.upper_exclusive != 12 * 60
                    || draw.purpose != "knowledge report delivery jitter"
                    || draw.at != event.timestamp
                    || draw.correlation_id != event.correlation_id
                    || *army != arrived_army
                    || *arrives_at != expected_arrives_at
                    || dispatch.timestamp != draw.at
                    || dispatch.correlation_id != draw.correlation_id
                    || dispatch.cause != Some(CauseRef::Event(cause))
                    || ReportDispatched::decode(&dispatch.kind).map_or(true, |payload| {
                        payload.recipient != *recipient
                            || payload.army != *army
                            || payload.arrives_at != *arrives_at
                    })
                {
                    return invalid_snapshot("core random draw provenance is inconsistent");
                }
                if report_draws.insert(*dispatch_event, draw.id).is_some() {
                    return invalid_snapshot(
                        "report dispatch is backed by more than one core random draw",
                    );
                }
            }
        }
        max_correlation_id = max_correlation_id.max(draw.correlation_id);
        previous_draw = Some((draw.at, draw.id));
    }

    for state in &snapshot.random_streams {
        if replayed.get(&state.key)
            != Some(&(state.position, state.generator_state, state.algorithm))
        {
            return invalid_snapshot("random draw journal does not reproduce stream state");
        }
    }
    for event in &snapshot.events {
        if event.kind.is_type(REPORT_DISPATCHED) && !report_draws.contains_key(&event.id) {
            return invalid_snapshot(
                "report dispatch must be backed by exactly one core random draw",
            );
        }
    }
    Ok((
        snapshot.random_draws.last().map_or(0, |draw| draw.id.get()),
        max_correlation_id,
    ))
}

fn validate_snapshot_domain_records(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
) -> Result<BTreeMap<DomainRecordRef, DomainRecord>, CanwuError> {
    if snapshot
        .domain_records
        .windows(2)
        .any(|pair| pair[0].reference >= pair[1].reference)
    {
        return invalid_snapshot("domain records are not in canonical stable-reference order");
    }
    let records: BTreeMap<_, _> = snapshot
        .domain_records
        .iter()
        .map(|record| (record.reference.clone(), record.clone()))
        .collect();
    if records.len() != snapshot.domain_records.len() {
        return invalid_snapshot("snapshot contains duplicate domain record references");
    }
    records::validate_record_store(&records, &plugins.record_schemas, snapshot.now, &|entity| {
        snapshot.entities.binary_search(entity).is_ok()
            || core_world_entity_exists(&snapshot.world, entity)
    })
    .map_err(|error| {
        invalid_snapshot_error(format!("snapshot domain-record state is invalid: {error}"))
    })?;
    Ok(records)
}

fn validate_decision_state(snapshot: &SimulationSnapshot) -> Result<(), CanwuError> {
    snapshot.decisions.validate().map_err(|error| {
        invalid_snapshot_error(format!("snapshot decision state is invalid: {error}"))
    })?;
    for controller in snapshot.decisions.controllers.values() {
        let authority_exists = match &controller.authority {
            DecisionAuthority::Actor { actor } => {
                snapshot_entity_exists(snapshot, &EntityRef::Person(*actor))
            }
            DecisionAuthority::Institution {
                institution,
                responsible_actor,
            } => {
                snapshot_entity_identity_exists(snapshot, institution)
                    && responsible_actor.is_none_or(|actor| {
                        snapshot_entity_exists(snapshot, &EntityRef::Person(actor))
                    })
            }
            DecisionAuthority::Council { .. } | DecisionAuthority::NoResponsibleActor { .. } => {
                true
            }
        };
        if !authority_exists
            || controller
                .command_subject
                .as_ref()
                .is_some_and(|entity| !snapshot_entity_identity_exists(snapshot, entity))
        {
            return invalid_snapshot(
                "decision controller authority or subject references an unknown entity",
            );
        }
    }
    if snapshot
        .decisions
        .tickets
        .values()
        .any(|ticket| !snapshot_entity_identity_exists(snapshot, &ticket.decision_maker))
    {
        return invalid_snapshot("decision ticket references an unknown decision maker");
    }
    if snapshot.ingress.is_empty() && snapshot.boundaries.is_empty() {
        let next_after_hot_trace = snapshot
            .decisions
            .traces
            .iter()
            .map(|trace| trace.id.get())
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| {
                invalid_snapshot_error("decision trace journal exceeds identifier space")
            })?;
        if snapshot.next_decision_trace_id < next_after_hot_trace {
            return invalid_snapshot("decision trace counter precedes retained hot history");
        }
        // Checkpoint snapshots deliberately omit replay evidence. Their
        // decision roots and counters are authenticated by the checkpoint
        // commitment, while paged checkpoints may keep only the archive
        // directory rather than hydrating historical receipt buckets.
        return Ok(());
    }
    let expected_next_trace_id = snapshot
        .decisions
        .traces
        .iter()
        .map(|trace| trace.id.get())
        .chain(
            snapshot
                .decisions
                .archived_history_keys()
                .filter_map(|key| match key {
                    super::DecisionHistoryKey::Trace(id) => Some(id.get()),
                    super::DecisionHistoryKey::Ticket(_)
                    | super::DecisionHistoryKey::Attempt(_) => None,
                }),
        )
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| invalid_snapshot_error("decision trace journal exceeds identifier space"))?;
    if snapshot.next_decision_trace_id != expected_next_trace_id {
        return invalid_snapshot("decision trace counter does not follow its journal");
    }

    let command_attempts_by_request: BTreeMap<_, _> = snapshot
        .command_attempts
        .iter()
        .filter_map(|attempt| attempt.request_id.map(|request| (request, attempt)))
        .collect();
    let mut reconstructed = DecisionState::default();
    let mut next_trace_id = 1_u64;
    let mut current_revision = 0_u64;
    let mut reconstructed_archive_commits = 0_u64;
    let mut rejected_archive_commits = 0_u64;
    for boundary in &snapshot.boundaries {
        let mut maintenance_change_index = 0_usize;
        for ingress_id in &boundary.admitted_ingress {
            let index = usize::try_from(ingress_id.get().saturating_sub(1)).map_err(|_| {
                invalid_snapshot_error("decision ingress ID exceeds platform range")
            })?;
            let Some(record) = snapshot.ingress.get(index) else {
                return invalid_snapshot("boundary admits an unknown ingress record");
            };
            match &record.payload {
                IngressPayload::Command { request } => {
                    let attempt = command_attempts_by_request
                        .get(&request.request_id)
                        .ok_or_else(|| {
                            invalid_snapshot_error(
                                "admitted command ingress lacks its command attempt",
                            )
                        })?;
                    if attempt.revision_before != current_revision {
                        return invalid_snapshot(
                            "command admission and decision revision chronology disagree",
                        );
                    }
                    current_revision = current_revision.checked_add(1).ok_or_else(|| {
                        invalid_snapshot_error("authoritative revision range is exhausted")
                    })?;
                    continue;
                }
                IngressPayload::Decision { .. }
                | IngressPayload::Plugin { .. }
                | IngressPayload::Calendar { .. } => {}
                IngressPayload::Maintenance { request } => {
                    let change = boundary
                        .maintenance_changes
                        .get(maintenance_change_index)
                        .ok_or_else(|| {
                            invalid_snapshot_error(
                                "maintenance ingress lacks its terminal change record",
                            )
                        })?;
                    maintenance_change_index += 1;
                    match request.as_ref() {
                        super::MaintenanceIngressRequest::DecisionArchive { commit } => {
                            if change.kind != "decision_archive" || change.token != commit.token() {
                                return invalid_snapshot(
                                    "decision archive ingress and terminal change disagree",
                                );
                            }
                            if change.disposition == super::MaintenanceDisposition::Applied {
                                reconstructed = reconstructed
                                    .commit_verified_decision_archive(commit)
                                    .map_err(super::decision::decision_error)?;
                                reconstructed_archive_commits += 1;
                            } else {
                                rejected_archive_commits += 1;
                            }
                        }
                        super::MaintenanceIngressRequest::OwnerAuthorized { commit } => {
                            if change.kind != "owner_authorized" || change.token != commit.token() {
                                return invalid_snapshot(
                                    "owner-authorized ingress and terminal change disagree",
                                );
                            }
                        }
                    }
                }
            }
            let IngressPayload::Decision { request } = &record.payload else {
                continue;
            };
            reconstruct_decision_ingress(
                snapshot,
                request,
                boundary.at,
                &command_attempts_by_request,
                &mut reconstructed,
                &mut next_trace_id,
                &mut current_revision,
            )?;
        }
        if maintenance_change_index != boundary.maintenance_changes.len() {
            return invalid_snapshot(
                "boundary contains a terminal maintenance record without ingress",
            );
        }
        reconstructed.advance_time(boundary.at).map_err(|error| {
            invalid_snapshot_error(format!("decision deadline state is invalid: {error}"))
        })?;
        validate_generated_random_decisions_at_boundary(
            boundary,
            snapshot,
            &reconstructed,
            current_revision,
        )?;
        current_revision = current_revision
            .checked_add(1)
            .ok_or_else(|| invalid_snapshot_error("authoritative revision range is exhausted"))?;
    }
    let controllers_match = reconstructed.controllers == snapshot.decisions.controllers;
    let reconstructed_hot_root = reconstructed
        .hot_history_commitment()
        .map_err(super::decision::decision_error)?;
    let persisted_hot_root = snapshot
        .decisions
        .hot_history_commitment()
        .map_err(super::decision::decision_error)?;
    let state_matches = controllers_match && reconstructed_hot_root == persisted_hot_root;
    let reconstructed_archive_root = reconstructed
        .archive_receipt_commitment()
        .map_err(super::decision::decision_error)?;
    let persisted_archive_root = snapshot
        .decisions
        .archive_receipt_commitment()
        .map_err(super::decision::decision_error)?;
    let trace_counter_matches = next_trace_id == snapshot.next_decision_trace_id;
    if !state_matches
        || reconstructed_archive_root != persisted_archive_root
        || !trace_counter_matches
    {
        return invalid_snapshot(format!(
            "decision ingress history does not reconstruct the persisted decision state \
             (state={state_matches}, archive_root={}, trace_counter={trace_counter_matches}, \
             reconstructed_archived={}, persisted_archived={}, reconstructed_root={}, \
             persisted_root={}, controllers={}, reconstructed_hot={}, persisted_hot={}, \
             applied_archive_commits={}, rejected_archive_commits={})",
            reconstructed_archive_root == persisted_archive_root,
            reconstructed.archived_history_count(),
            snapshot.decisions.archived_history_count(),
            reconstructed_archive_root,
            persisted_archive_root,
            controllers_match,
            reconstructed_hot_root,
            persisted_hot_root,
            reconstructed_archive_commits,
            rejected_archive_commits,
        ));
    }
    Ok(())
}

fn validate_generated_random_decisions_at_boundary(
    boundary: &BoundaryRecord,
    snapshot: &SimulationSnapshot,
    decisions: &DecisionState,
    revision_before_boundary: u64,
) -> Result<(), CanwuError> {
    let expected_revision = revision_before_boundary
        .checked_add(1)
        .ok_or_else(|| invalid_snapshot_error("authoritative revision range is exhausted"))?;
    for generation in &boundary.generated_ingress {
        let index = usize::try_from(generation.ingress.get().saturating_sub(1)).map_err(|_| {
            invalid_snapshot_error("generated decision ingress ID exceeds platform range")
        })?;
        let Some(IngressRecord {
            payload: IngressPayload::Decision { request },
            ..
        }) = snapshot.ingress.get(index)
        else {
            continue;
        };
        let DecisionMutation::Resolve {
            ticket_id,
            expected_version,
            controller_id,
            policy,
            decision,
            command_request_id,
        } = &request.mutation
        else {
            return invalid_snapshot(
                "boundary-generated decision ingress must resolve an existing ticket",
            );
        };
        let ticket = decisions.ticket(*ticket_id).ok_or_else(|| {
            invalid_snapshot_error(
                "boundary-generated random decision references an unknown ticket at its source boundary",
            )
        })?;
        let controller = decisions.controller(controller_id).ok_or_else(|| {
            invalid_snapshot_error(
                "boundary-generated random decision references an unknown controller at its source boundary",
            )
        })?;
        let Some(random) = &decision.random else {
            return invalid_snapshot("boundary-generated decision ingress lacks random evidence");
        };
        let DecisionOutcome::Selected { option_id } = &decision.outcome else {
            return invalid_snapshot(
                "boundary-generated random decision must select an available option",
            );
        };
        let selected =
            DecisionRandomEvidence::selected_option(ticket, &random.option_weights, random.value)
                .map_err(|error| {
                invalid_snapshot_error(format!(
                    "boundary-generated random decision weights are invalid: {error}"
                ))
            })?;
        let option = ticket.option(option_id).ok_or_else(|| {
            invalid_snapshot_error(
                "boundary-generated random decision selected an unknown ticket option",
            )
        })?;
        let command_matches = match (&option.action, &request.command) {
            (DecisionAction::None, None) => command_request_id.is_none(),
            (DecisionAction::Command { command }, Some(command_request)) => {
                let expected_command = serde_json::from_value::<Command>(command.clone())
                    .map_err(|error| {
                        invalid_snapshot_error(format!(
                            "boundary-generated random decision option contains an invalid command: {error}"
                        ))
                    })?;
                *command_request_id == Some(command_request.request_id)
                    && command_request.expected_revision == expected_revision
                    && command_request.envelope.command == expected_command
                    && command_request.envelope.issuer
                        == super::decision::controller_issuer(controller)
                    && command_request.envelope.authority.as_ref()
                        == Some(&super::decision::controller_authority(controller))
                    && command_request.envelope.expected_time == Some(boundary.at)
            }
            (DecisionAction::None, Some(_)) | (DecisionAction::Command { .. }, None) => false,
        };
        if !ticket.is_open()
            || ticket.version != *expected_version
            || ticket.assigned_controller != *controller_id
            || controller.policy != *policy
            || policy.kind != DecisionPolicyKind::Random
            || request.expected_revision != expected_revision
            || selected != *option_id
            || decision.summary != format!("random policy selected {option_id}")
            || !decision.evaluations.is_empty()
            || decision.external.is_some()
            || !command_matches
        {
            return invalid_snapshot(
                "boundary-generated random decision disagrees with its source-boundary ticket or controller",
            );
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn reconstruct_decision_ingress(
    snapshot: &SimulationSnapshot,
    request: &super::DecisionIngressRequest,
    at: super::SimTime,
    command_attempts_by_request: &BTreeMap<canwu_core::CommandRequestId, &CommandAttemptRecord>,
    reconstructed: &mut DecisionState,
    next_trace_id: &mut u64,
    current_revision: &mut u64,
) -> Result<(), CanwuError> {
    let request_commitment = canonical_hash(DECISION_REQUEST_COMMITMENT_DOMAIN, request)?;
    let base_attempt = |outcome| DecisionAttemptRecord {
        request_id: request.request_id,
        request_commitment: request_commitment.clone(),
        at,
        revision_before: *current_revision,
        expected_revision: request.expected_revision,
        outcome,
    };
    let expected_attempt = if request.expected_revision != *current_revision {
        base_attempt(DecisionAttemptOutcome::Rejected {
            code: DecisionAttemptErrorCode::SimulationRevisionConflict,
            message: format!(
                "decision request {} expected revision {}, current revision is {}",
                request.request_id, request.expected_revision, current_revision
            ),
        })
    } else if let Some(message) = snapshot_decision_entity_error(snapshot, &request.mutation) {
        base_attempt(DecisionAttemptOutcome::Rejected {
            code: DecisionAttemptErrorCode::EntityUnavailable,
            message,
        })
    } else {
        let trace_id = matches!(request.mutation, DecisionMutation::Resolve { .. })
            .then(|| DecisionTraceId::new(*next_trace_id));
        let mut candidate = reconstructed.clone();
        match candidate.apply(request.mutation.clone(), at, trace_id) {
            Err(error) => base_attempt(DecisionAttemptOutcome::Rejected {
                code: error.code.into(),
                message: error.message,
            }),
            Ok(prepared) => {
                let invalid_command = match (&prepared.action, &request.command) {
                    (Some(DecisionAction::Command { command }), Some(command_request)) => {
                        match serde_json::from_value::<Command>(command.clone()) {
                            Err(error) => Some(format!(
                                "decision option contains an invalid command: {error}"
                            )),
                            Ok(expected) => {
                                let controller = prepared
                                    .trace
                                    .as_ref()
                                    .and_then(|trace| candidate.controller(&trace.controller_id))
                                    .ok_or_else(|| {
                                        invalid_snapshot_error(
                                            "decision command trace lacks its controller binding",
                                        )
                                    })?;
                                (command_request.envelope.command != expected
                                    || command_request.expected_revision != *current_revision
                                    || prepared
                                        .trace
                                        .as_ref()
                                        .and_then(|trace| trace.command_request_id)
                                        != Some(command_request.request_id))
                                .then_some(
                                    "nested command does not match the selected decision option"
                                        .to_owned(),
                                )
                                .or_else(|| {
                                    (command_request.envelope.issuer
                                        != super::decision::controller_issuer(controller)
                                        || command_request.envelope.authority.as_ref()
                                            != Some(&super::decision::controller_authority(
                                                controller,
                                            ))
                                        || command_request.envelope.expected_time != Some(at))
                                    .then_some("nested command issuer, authority, or time guard was not derived from the decision controller".to_owned())
                                })
                            }
                        }
                    }
                    (Some(DecisionAction::None) | None, None) => None,
                    _ => Some("decision action and nested command disagree".to_owned()),
                };
                if let Some(message) = invalid_command {
                    base_attempt(DecisionAttemptOutcome::Rejected {
                        code: DecisionAttemptErrorCode::InvalidDecision,
                        message,
                    })
                } else {
                    if trace_id.is_some() {
                        *next_trace_id = next_trace_id.checked_add(1).ok_or_else(|| {
                            invalid_snapshot_error("decision trace identifier space is exhausted")
                        })?;
                    }
                    let command_request_id =
                        request.command.as_ref().map(|request| request.request_id);
                    let accepted = base_attempt(DecisionAttemptOutcome::Accepted {
                        trace_id,
                        command_request_id,
                    });
                    candidate
                        .append_attempt(accepted.clone())
                        .map_err(|error| {
                            invalid_snapshot_error(format!(
                                "reconstructed decision attempt is invalid: {error}"
                            ))
                        })?;
                    *reconstructed = candidate;
                    if let Some(command_request_id) = command_request_id {
                        let command_attempt = command_attempts_by_request
                            .get(&command_request_id)
                            .ok_or_else(|| {
                                invalid_snapshot_error(
                                    "accepted decision command lacks its command attempt",
                                )
                            })?;
                        if command_attempt.revision_before != *current_revision {
                            return invalid_snapshot(
                                "decision command attempt and revision chronology disagree",
                            );
                        }
                        *current_revision = current_revision.checked_add(1).ok_or_else(|| {
                            invalid_snapshot_error("authoritative revision range is exhausted")
                        })?;
                    }
                    accepted
                }
            }
        }
    };
    if matches!(
        expected_attempt.outcome,
        DecisionAttemptOutcome::Rejected { .. }
    ) {
        reconstructed
            .append_attempt(expected_attempt)
            .map_err(|error| {
                invalid_snapshot_error(format!(
                    "reconstructed decision attempt is invalid: {error}"
                ))
            })?;
    }
    Ok(())
}

fn snapshot_decision_entity_error(
    snapshot: &SimulationSnapshot,
    mutation: &DecisionMutation,
) -> Option<String> {
    let entity_exists = |entity: &EntityRef| snapshot_entity_identity_exists(snapshot, entity);
    match mutation {
        DecisionMutation::RegisterController { controller } => {
            let authority_exists = match &controller.authority {
                DecisionAuthority::Actor { actor } => entity_exists(&EntityRef::Person(*actor)),
                DecisionAuthority::Institution {
                    institution,
                    responsible_actor,
                } => {
                    entity_exists(institution)
                        && responsible_actor
                            .is_none_or(|actor| entity_exists(&EntityRef::Person(actor)))
                }
                DecisionAuthority::Council { .. }
                | DecisionAuthority::NoResponsibleActor { .. } => true,
            };
            if !authority_exists {
                Some("decision controller authority references an unknown entity".to_owned())
            } else if controller
                .command_subject
                .as_ref()
                .is_some_and(|entity| !entity_exists(entity))
            {
                Some("decision controller command subject references an unknown entity".to_owned())
            } else {
                None
            }
        }
        DecisionMutation::Open { ticket } if !entity_exists(&ticket.decision_maker) => {
            Some("decision maker references an unknown entity".to_owned())
        }
        DecisionMutation::Open { .. }
        | DecisionMutation::ReplaceOptions { .. }
        | DecisionMutation::Resolve { .. }
        | DecisionMutation::Cancel { .. } => None,
    }
}

fn validate_ingress_records(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    history: &DomainRecordHistory,
) -> Result<u64, CanwuError> {
    let boundary_count = u64::try_from(snapshot.boundaries.len())
        .map_err(|_| invalid_snapshot_error("boundary count exceeds the ingress journal range"))?;
    let mut generated_by_boundary = BTreeMap::new();
    for boundary in &snapshot.boundaries {
        if boundary
            .generated_ingress
            .windows(2)
            .any(|pair| pair[0].ingress >= pair[1].ingress)
        {
            return invalid_snapshot(
                "boundary-generated ingress evidence is not in canonical identifier order",
            );
        }
        for generation in &boundary.generated_ingress {
            if generated_by_boundary
                .insert(generation.ingress, boundary.id)
                .is_some()
            {
                return invalid_snapshot(
                    "ingress is claimed as generated by more than one boundary",
                );
            }
            let index =
                usize::try_from(generation.ingress.get().saturating_sub(1)).map_err(|_| {
                    invalid_snapshot_error(
                        "boundary-generated ingress ID exceeds the platform index range",
                    )
                })?;
            let Some(record) = snapshot.ingress.get(index) else {
                return invalid_snapshot(
                    "boundary-generated ingress evidence references an unknown record",
                );
            };
            if record.id != generation.ingress
                || record.issued_at != boundary.at
                || record.eligible_boundary_count != boundary.id.get()
                || record.cause != Some(CauseRef::Boundary(boundary.id))
                || !matches!(
                    &record.payload,
                    IngressPayload::Plugin { .. } | IngressPayload::Decision { .. }
                )
            {
                return invalid_snapshot(
                    "boundary-generated ingress evidence disagrees with its record",
                );
            }
        }
    }
    let mut previous_issue = None;
    let mut command_request_ids = BTreeSet::new();
    let mut decision_request_ids = BTreeSet::new();
    for (index, record) in snapshot.ingress.iter().enumerate() {
        let expected_id = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| invalid_snapshot_error("ingress index exceeds identifier space"))?;
        if record.id.get() != expected_id
            || record.issued_at < snapshot.initial_time
            || record.issued_at > snapshot.now
            || record.due_at < record.issued_at
            || record.eligible_boundary_count > boundary_count
            || previous_issue.is_some_and(|previous| {
                (record.eligible_boundary_count, record.issued_at, record.id) <= previous
            })
        {
            return invalid_snapshot("ingress journal identity, time, or issue cut is invalid");
        }
        validate_snapshot_ingress_cause(snapshot, record)?;
        let issue_boundary_count =
            usize::try_from(record.eligible_boundary_count).map_err(|_| {
                invalid_snapshot_error("ingress issue cut exceeds the platform index range")
            })?;
        if issue_boundary_count > 0
            && record.issued_at < snapshot.boundaries[issue_boundary_count - 1].at
        {
            return invalid_snapshot("ingress predates its declared eligibility boundary");
        }
        let issue_cut = DomainHistoryCut::after_boundaries(issue_boundary_count);
        match &record.payload {
            IngressPayload::Command { request } => {
                if record.class != IngressClass::Command
                    || request.request_id.get() == 0
                    || !command_request_ids.insert(request.request_id)
                    || request
                        .envelope
                        .expected_time
                        .is_some_and(|expected| expected != record.due_at)
                    || record.cause.is_some()
                {
                    return invalid_snapshot("queued command ingress is not canonical");
                }
            }
            IngressPayload::Plugin {
                plugin,
                packet_type,
                payload,
                affected_entities,
                archive_retention,
            } => {
                let Some(descriptor) = plugins.ingress.get(&(plugin.clone(), packet_type.clone()))
                else {
                    return invalid_snapshot("plugin ingress references an undeclared packet type");
                };
                if record.class != descriptor.class
                    || affected_entities.windows(2).any(|pair| pair[0] >= pair[1])
                    || affected_entities.iter().any(|entity| {
                        !snapshot_entity_identity_exists_in_history(
                            snapshot, history, issue_cut, entity,
                        )
                    })
                {
                    return invalid_snapshot("plugin ingress class or entity evidence is invalid");
                }
                if archive_retention.len() > 32_768
                    || archive_retention.windows(2).any(|pair| pair[0] >= pair[1])
                    || archive_retention.iter().any(|retention| {
                        retention.namespace.is_empty()
                            || retention.namespace.len() > 128
                            || retention.object_id.is_empty()
                            || retention.object_id.len() > 256
                            || !retention.namespace.bytes().all(|byte| {
                                byte.is_ascii_lowercase()
                                    || byte.is_ascii_digit()
                                    || matches!(byte, b'.' | b'-' | b'_')
                            })
                            || !retention.object_id.is_ascii()
                    })
                {
                    return invalid_snapshot("plugin ingress archive retention is malformed");
                }
                if !archive_retention.is_empty()
                    && !plugins
                        .internal_ingress
                        .contains(&(plugin.clone(), packet_type.clone()))
                {
                    return invalid_snapshot(
                        "plugin ingress archive retention lacks internal ownership",
                    );
                }
                plugins
                    .validate_archive_retention(plugin, packet_type, payload, archive_retention)
                    .map_err(|_| {
                        invalid_snapshot_error(
                            "plugin ingress archive retention is not bound to its payload roots",
                        )
                    })?;
                match &record.cause {
                    Some(CauseRef::Boundary(boundary))
                        if generated_by_boundary.get(&record.id) == Some(boundary) => {}
                    Some(CauseRef::Boundary(_)) => {
                        return invalid_snapshot(
                            "boundary-caused ingress lacks matching generation evidence",
                        );
                    }
                    Some(CauseRef::Command(command_id)) => {
                        let Some(command) = snapshot_command_by_id(snapshot, *command_id) else {
                            return invalid_snapshot(
                                "command-generated ingress references an unknown command",
                            );
                        };
                        let Command::Plugin {
                            plugin: producer, ..
                        } = &command.envelope.command
                        else {
                            return invalid_snapshot(
                                "only plugin commands may generate plugin ingress",
                            );
                        };
                        if producer != plugin
                            || command.accepted_at != record.issued_at
                            || generated_by_boundary.contains_key(&record.id)
                        {
                            return invalid_snapshot(
                                "command-generated ingress disagrees with its producer or issue cut",
                            );
                        }
                    }
                    Some(CauseRef::Event(_)) => {
                        return invalid_snapshot(
                            "event-generated ingress is not supported by this snapshot format",
                        );
                    }
                    Some(CauseRef::System(_)) | None
                        if !generated_by_boundary.contains_key(&record.id) => {}
                    Some(CauseRef::System(_)) | None => {
                        return invalid_snapshot(
                            "external ingress is incorrectly claimed as boundary-generated",
                        );
                    }
                }
                if snapshot
                    .run_configuration
                    .as_ref()
                    .and_then(RunConfigurationSnapshot::declared)
                    .is_some_and(|configuration| {
                        configuration.interaction == InteractionPolicy::ReadOnly
                    })
                    && !matches!(&record.cause, Some(CauseRef::Boundary(_)))
                {
                    return invalid_snapshot(
                        "declared read-only runs cannot contain newly authored plugin ingress",
                    );
                }
                descriptor
                    .payload_schema
                    .validate(payload)
                    .map_err(|error| {
                        invalid_snapshot_error(format!(
                            "plugin ingress payload is invalid: {error}"
                        ))
                    })?;
            }
            IngressPayload::Calendar { cadences } => {
                if record.class != IngressClass::ScheduledSystem
                    || record.priority != 0
                    || cadences.is_empty()
                    || cadences.contains(&SystemCadence::EventDriven)
                    || cadences.windows(2).any(|pair| pair[0] >= pair[1])
                    || record.cause != Some(CauseRef::System("canwu.core.calendar".to_owned()))
                {
                    return invalid_snapshot("calendar ingress is not canonical");
                }
            }
            IngressPayload::Decision { request } => {
                if record.class != IngressClass::Decision
                    || request.request_id.get() == 0
                    || !decision_request_ids.insert(request.request_id)
                {
                    return invalid_snapshot("queued decision ingress is not canonical");
                }
                match &record.cause {
                    Some(CauseRef::Boundary(boundary))
                        if generated_by_boundary.get(&record.id) == Some(boundary) => {}
                    None if !generated_by_boundary.contains_key(&record.id) => {}
                    _ => {
                        return invalid_snapshot(
                            "decision ingress cause disagrees with boundary-generation evidence",
                        );
                    }
                }
                if snapshot
                    .run_configuration
                    .as_ref()
                    .and_then(RunConfigurationSnapshot::declared)
                    .is_some_and(|configuration| {
                        configuration.interaction == InteractionPolicy::ReadOnly
                    })
                    && !matches!(&record.cause, Some(CauseRef::Boundary(_)))
                {
                    return invalid_snapshot(
                        "declared read-only runs cannot contain newly authored decision ingress",
                    );
                }
                if let Some(command) = &request.command
                    && (command.request_id.get() == 0
                        || !command_request_ids.insert(command.request_id)
                        || command.expected_revision != request.expected_revision
                        || command.envelope.expected_time != Some(record.due_at))
                {
                    return invalid_snapshot("queued decision command request is not canonical");
                }
            }
            IngressPayload::Maintenance { request } => {
                if record.class != IngressClass::ScheduledSystem {
                    return invalid_snapshot("maintenance ingress is not canonical");
                }
                match request.as_ref() {
                    super::MaintenanceIngressRequest::DecisionArchive { commit } => {
                        if record.cause
                            != Some(CauseRef::System("canwu.core.decision-archive".to_owned()))
                        {
                            return invalid_snapshot("decision archive ingress cause is invalid");
                        }
                        if !commit.has_current_nonempty_shape() {
                            return invalid_snapshot(
                                "decision archive maintenance payload is invalid",
                            );
                        }
                    }
                    super::MaintenanceIngressRequest::OwnerAuthorized { commit } => {
                        if record.cause
                            != Some(CauseRef::System(
                                "canwu.core.owner-authorized-maintenance".to_owned(),
                            ))
                        {
                            return invalid_snapshot(
                                "owner-authorized maintenance ingress cause is invalid",
                            );
                        }
                        super::maintenance::validate_verified_commit_shape(commit)?;
                    }
                }
            }
        }
        previous_issue = Some((record.eligible_boundary_count, record.issued_at, record.id));
    }

    let attempts_by_request: BTreeMap<_, _> = snapshot
        .command_attempts
        .iter()
        .filter_map(|attempt| attempt.request_id.map(|request| (request, attempt)))
        .collect();
    let decision_attempts_by_request: BTreeMap<_, _> = snapshot
        .decisions
        .attempts()
        .iter()
        .map(|attempt| (attempt.request_id, attempt))
        .collect();
    let mut pending = BTreeSet::new();
    let mut cursor = 0;
    for (boundary_index, boundary) in snapshot.boundaries.iter().enumerate() {
        let available_after = u64::try_from(boundary_index)
            .map_err(|_| invalid_snapshot_error("boundary index exceeds ingress range"))?;
        while let Some(record) = snapshot.ingress.get(cursor)
            && record.eligible_boundary_count <= available_after
        {
            if record.issued_at > boundary.at {
                return invalid_snapshot("ingress is assigned to a boundary before it was issued");
            }
            pending.insert(IngressQueueKey::from_record(record));
            cursor += 1;
        }
        if pending.first().is_some_and(|key| key.due_at < boundary.at) {
            return invalid_snapshot("a boundary steps past earlier canonical ingress");
        }
        let expected: Vec<_> = pending
            .iter()
            .take_while(|key| key.due_at <= boundary.at)
            .map(|key| key.id)
            .collect();
        if boundary.admitted_ingress != expected {
            return invalid_snapshot(
                "boundary ingress admission does not match the canonical due queue",
            );
        }
        let mut expected_attempts = Vec::new();
        let mut expected_commands = Vec::new();
        let mut expected_cadences = BTreeSet::new();
        for ingress_id in expected {
            let index = usize::try_from(ingress_id.get().saturating_sub(1)).map_err(|_| {
                invalid_snapshot_error("admitted ingress ID exceeds the platform index range")
            })?;
            let record = &snapshot.ingress[index];
            if let IngressPayload::Command { request } = &record.payload {
                let Some(attempt) = attempts_by_request.get(&request.request_id) else {
                    return invalid_snapshot(
                        "admitted command ingress is missing its deterministic attempt outcome",
                    );
                };
                if attempt.at != boundary.at
                    || attempt.envelope != request.envelope
                    || attempt.expected_revision != Some(request.expected_revision)
                    || attempt.ingress != CommandIngress::LiveRequest
                {
                    return invalid_snapshot(
                        "command ingress, attempt outcome, and boundary admission disagree",
                    );
                }
                expected_attempts.push(attempt.id);
                if let CommandAttemptOutcome::Accepted { command_id } = attempt.outcome {
                    expected_commands.push(command_id);
                }
            } else {
                match &record.payload {
                    IngressPayload::Decision { request } => {
                        if let Some(request) = &request.command {
                            let decision_request_id = match &record.payload {
                                IngressPayload::Decision { request } => request.request_id,
                                _ => unreachable!(),
                            };
                            let accepted_command = decision_attempts_by_request
                                .get(&decision_request_id)
                                .is_some_and(|attempt| {
                                    matches!(
                                        attempt.outcome,
                                        DecisionAttemptOutcome::Accepted {
                                            command_request_id: Some(command_request_id),
                                            ..
                                        } if command_request_id == request.request_id
                                    )
                                });
                            if !accepted_command {
                                continue;
                            }
                            let Some(attempt) = attempts_by_request.get(&request.request_id) else {
                                return invalid_snapshot(
                                    "admitted decision command is missing its deterministic attempt outcome",
                                );
                            };
                            if attempt.at != boundary.at
                                || attempt.envelope != request.envelope
                                || attempt.expected_revision != Some(request.expected_revision)
                                || attempt.ingress != CommandIngress::LiveRequest
                            {
                                return invalid_snapshot(
                                    "decision command, attempt outcome, and boundary admission disagree",
                                );
                            }
                            expected_attempts.push(attempt.id);
                            if let CommandAttemptOutcome::Accepted { command_id } = attempt.outcome
                            {
                                expected_commands.push(command_id);
                            }
                        }
                    }
                    IngressPayload::Calendar { cadences } => {
                        expected_cadences.extend(cadences.iter().cloned());
                    }
                    IngressPayload::Plugin { .. }
                    | IngressPayload::Command { .. }
                    | IngressPayload::Maintenance { .. } => {}
                }
            }
            pending.remove(&IngressQueueKey::from_record(record));
        }
        if !snapshot.ingress.is_empty()
            && (boundary.admitted_attempts != expected_attempts
                || boundary.admitted_commands != expected_commands
                || expected_cadences
                    .iter()
                    .any(|cadence| !boundary.cadences.contains(cadence)))
        {
            return invalid_snapshot(
                "boundary command or calendar effects do not match admitted ingress order",
            );
        }
    }
    for record in &snapshot.ingress[cursor..] {
        if record.eligible_boundary_count != boundary_count {
            return invalid_snapshot("ingress issue cuts skip a completed boundary");
        }
        pending.insert(IngressQueueKey::from_record(record));
    }
    if pending.iter().any(|key| key.due_at < snapshot.now) {
        return invalid_snapshot("snapshot retains ingress overdue before committed time");
    }
    for key in &pending {
        let index = usize::try_from(key.id.get().saturating_sub(1)).map_err(|_| {
            invalid_snapshot_error("pending ingress ID exceeds the platform index range")
        })?;
        if let IngressPayload::Command { request } = &snapshot.ingress[index].payload
            && attempts_by_request.contains_key(&request.request_id)
        {
            return invalid_snapshot("pending command ingress already has an attempt outcome");
        }
    }
    Ok(snapshot.ingress.last().map_or(0, |record| record.id.get()))
}

fn validate_snapshot_ingress_cause(
    snapshot: &SimulationSnapshot,
    record: &IngressRecord,
) -> Result<(), CanwuError> {
    let context = SnapshotValidationContext::new(snapshot);
    if let Some(cause) = &record.cause {
        validate_cause_reference(&context, cause).map_err(|error| {
            invalid_snapshot_error(match error {
                CauseValidationError::MissingEvidence => {
                    "ingress cause references unavailable or future evidence"
                }
                CauseValidationError::NonCanonicalSystem => "ingress system cause is not canonical",
            })
        })?;
    }
    let valid = match &record.cause {
        None => true,
        Some(CauseRef::Boundary(id)) => {
            matches!(context.boundary(*id), EvidenceLookup::Retained(boundary) if {
                    boundary.id == *id
                        && boundary.at <= record.issued_at
                        && id.get() <= record.eligible_boundary_count
            })
        }
        Some(CauseRef::Command(id)) => {
            matches!(context.command(*id), EvidenceLookup::Retained(command) if command.accepted_at <= record.issued_at)
        }
        Some(CauseRef::Event(id)) => {
            matches!(context.event(*id), EvidenceLookup::Retained(event) if event.timestamp <= record.issued_at)
        }
        Some(CauseRef::System(name)) => canonical_text(name),
    };
    if valid {
        Ok(())
    } else {
        invalid_snapshot("ingress cause references unavailable or future evidence")
    }
}

fn journal_record_by_id<T>(
    records: &[T],
    id: u64,
    record_id: impl FnOnce(&T) -> u64,
) -> Option<&T> {
    let index = usize::try_from(id.checked_sub(1)?).ok()?;
    let record = records.get(index)?;
    (record_id(record) == id).then_some(record)
}

fn snapshot_event_by_id(snapshot: &SimulationSnapshot, id: EventId) -> Option<&SimEvent> {
    journal_record_by_id(&snapshot.events, id.get(), |event| event.id.get())
}

fn snapshot_command_by_id(snapshot: &SimulationSnapshot, id: CommandId) -> Option<&CommandRecord> {
    journal_record_by_id(&snapshot.commands, id.get(), |command| command.id.get())
}

fn snapshot_command_attempt_by_id(
    snapshot: &SimulationSnapshot,
    id: CommandAttemptId,
) -> Option<&CommandAttemptRecord> {
    journal_record_by_id(&snapshot.command_attempts, id.get(), |attempt| {
        attempt.id.get()
    })
}

fn snapshot_ingress_by_id(snapshot: &SimulationSnapshot, id: IngressId) -> Option<&IngressRecord> {
    journal_record_by_id(&snapshot.ingress, id.get(), |record| record.id.get())
}

fn snapshot_boundary_by_id(
    snapshot: &SimulationSnapshot,
    id: BoundaryId,
) -> Option<&BoundaryRecord> {
    journal_record_by_id(&snapshot.boundaries, id.get(), |boundary| boundary.id.get())
}

fn snapshot_random_draw_by_id(
    snapshot: &SimulationSnapshot,
    id: RandomDrawId,
) -> Option<&RandomDrawRecord> {
    journal_record_by_id(&snapshot.random_draws, id.get(), |draw| draw.id.get())
}

fn initial_domain_record_matches(
    scenario: Option<&super::Scenario>,
    reference: &DomainRecordVersionRef,
) -> bool {
    reference.version != 0
        && scenario.is_some_and(|scenario| {
            scenario.domain_records.iter().any(|record| {
                record.reference == reference.record && record.version == reference.version
            })
        })
}

fn boundary_domain_record_matches(
    boundary: &BoundaryRecord,
    change_index: u64,
    reference: &DomainRecordVersionRef,
) -> bool {
    usize::try_from(change_index)
        .ok()
        .and_then(|index| boundary.record_changes.get(index))
        .is_some_and(|change| {
            change.current.reference == reference.record
                && change.current.version == reference.version
        })
}

fn resolve_runtime_domain_record_version(
    state: &RuntimeState,
    reference: &DomainRecordVersionRef,
) -> EvidenceAvailability {
    if reference.version == 0 {
        return EvidenceAvailability::Missing;
    }
    match reference.established_by {
        DomainRecordVersionSource::InitialScenario => {
            if initial_domain_record_matches(state.metadata.initial_scenario.as_ref(), reference) {
                EvidenceAvailability::Retained
            } else {
                EvidenceAvailability::Missing
            }
        }
        DomainRecordVersionSource::BoundaryChange {
            boundary,
            change_index,
        } => match RuntimeValidationContext::new(state).boundary(boundary) {
            EvidenceLookup::Retained(record)
                if boundary_domain_record_matches(record, change_index, reference) =>
            {
                EvidenceAvailability::Retained
            }
            EvidenceLookup::Archived
                if state
                    .evidence
                    .archived_evidence_receipts
                    .contains_key(&EvidenceRef::DomainRecordVersion(reference.clone())) =>
            {
                EvidenceAvailability::Archived
            }
            EvidenceLookup::Archived | EvidenceLookup::Retained(_) | EvidenceLookup::Missing => {
                EvidenceAvailability::Missing
            }
        },
    }
}

fn resolve_snapshot_domain_record_version(
    snapshot: &SimulationSnapshot,
    reference: &DomainRecordVersionRef,
) -> EvidenceAvailability {
    if reference.version == 0 {
        return EvidenceAvailability::Missing;
    }
    match reference.established_by {
        DomainRecordVersionSource::InitialScenario => {
            if initial_domain_record_matches(snapshot.initial_scenario.as_ref(), reference) {
                EvidenceAvailability::Retained
            } else {
                EvidenceAvailability::Missing
            }
        }
        DomainRecordVersionSource::BoundaryChange {
            boundary,
            change_index,
        } => snapshot_boundary_by_id(snapshot, boundary).map_or(
            EvidenceAvailability::Missing,
            |record| {
                if boundary_domain_record_matches(record, change_index, reference) {
                    EvidenceAvailability::Retained
                } else {
                    EvidenceAvailability::Missing
                }
            },
        ),
    }
}

fn validate_boundary_records(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    final_domain_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    initial_domain_records: Option<&BTreeMap<DomainRecordRef, DomainRecord>>,
) -> Result<(u64, u64, DomainRecordHistory, PersistedAdmissionCursors), CanwuError> {
    let mut boundary_ids = BTreeSet::new();
    let mut emitted_events = BTreeSet::new();
    let mut boundary_correlations = BTreeSet::new();
    let mut boundary_values = BTreeMap::new();
    let mut knowledge_values = BTreeMap::new();
    let mut next_knowledge_id = 1_u64;
    let mut domain_record_values = final_domain_records.clone();
    for boundary in snapshot.boundaries.iter().rev() {
        for change in boundary.record_changes.iter().rev() {
            let reference = &change.current.reference;
            if domain_record_values.get(reference) != Some(&change.current) {
                return invalid_snapshot(
                    "boundary domain-record history does not match its persisted successor",
                );
            }
            if let Some(previous) = &change.previous {
                domain_record_values.insert(reference.clone(), previous.clone());
            } else {
                domain_record_values.remove(reference);
            }
        }
    }
    let empty_initial_records = BTreeMap::new();
    let expected_initial_records = initial_domain_records.unwrap_or(&empty_initial_records);
    if &domain_record_values != expected_initial_records {
        return invalid_snapshot(
            "boundary domain-record history does not match the manifest-bound initial scenario",
        );
    }
    let initial_world = snapshot
        .initial_scenario
        .as_ref()
        .map_or(&snapshot.world, |scenario| &scenario.world);
    records::validate_record_store(
        &domain_record_values,
        &plugins.record_schemas,
        snapshot.initial_time,
        &|entity| core_world_entity_exists(initial_world, entity),
    )
    .map_err(|error| {
        invalid_snapshot_error(format!(
            "initial domain-record state reconstructed from boundary evidence is invalid: {error}"
        ))
    })?;
    let mut next_attempt = 0;
    let mut next_command = 0;
    let mut next_event = 0;
    let mut previous_boundary = None;
    let mut previous_hash = GENESIS_BOUNDARY_HASH.to_owned();
    let mut previous_maintenance_root: Option<String> = None;
    let mut max_boundary_id = 0;
    let mut max_correlation_id = 0;
    let mut history = DomainRecordHistory::from_initial_records(&domain_record_values);
    let requires_state_hash = matches!(snapshot.run_manifest, Some(RunManifest::Declared { .. }));

    for (index, record) in snapshot.boundaries.iter().enumerate() {
        let expected_id = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| invalid_snapshot_error("boundary index exceeds identifier space"))?;
        if record.id.get() != expected_id || !boundary_ids.insert(record.id) {
            return invalid_snapshot("boundary IDs must be contiguous, unique, and nonzero");
        }
        if record.at < snapshot.initial_time
            || record.at > snapshot.now
            || previous_boundary.is_some_and(|(at, id)| (record.at, record.id) <= (at, id))
            || record.correlation_id == 0
            || !boundary_correlations.insert(record.correlation_id)
        {
            return invalid_snapshot("boundary time, order, or correlation is invalid");
        }
        if record.cadences.contains(&SystemCadence::EventDriven)
            || record.cadences.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return invalid_snapshot("boundary cadences are not canonical");
        }
        validate_boundary_admission(
            record,
            snapshot,
            &mut next_attempt,
            &mut next_command,
            &mut next_event,
        )?;
        let cuts =
            validate_boundary_record_changes(record, snapshot, plugins, &mut domain_record_values)?;
        validate_boundary_ingress_generation(
            record,
            snapshot,
            plugins,
            &domain_record_values,
            &cuts,
        )?;
        validate_boundary_reservations(record, snapshot, plugins, &domain_record_values, &cuts)?;
        validate_boundary_changes(record, snapshot, plugins, &domain_record_values, &cuts)?;
        validate_boundary_knowledge_changes(
            record,
            snapshot,
            plugins,
            &domain_record_values,
            &cuts,
            &mut knowledge_values,
            &mut next_knowledge_id,
        )?;
        for change in &record.changes {
            let key = component_key(
                &change.plugin,
                &change.state,
                &change.entity,
                &change.component,
            );
            if change.previous.as_ref() != boundary_values.get(&key) {
                return invalid_snapshot("boundary change previous-value evidence is inconsistent");
            }
            boundary_values.insert(key, change.value.clone());
        }
        validate_boundary_emissions(
            record,
            snapshot,
            plugins,
            &domain_record_values,
            &cuts,
            &mut emitted_events,
        )?;
        if let Some(state_hash) = record.state_hash.as_deref() {
            boundary_state_hash_format(Some(state_hash))?;
        }
        for change in &record.maintenance_changes {
            if !canonical_text(&change.kind)
                || !is_canonical_hash(&change.token)
                || !is_canonical_hash(&change.source_root)
                || !is_canonical_hash(&change.target_root)
                || matches!(change.disposition, super::MaintenanceDisposition::Applied)
                    != change.rejection.is_none()
            {
                return invalid_snapshot("boundary maintenance evidence is malformed");
            }
            if let Some(rejection) = &change.rejection
                && (rejection.token != change.token
                    || !is_canonical_hash(&rejection.expected_source_root)
                    || rejection.observed_source_root != change.source_root
                    || change.target_root != change.source_root
                    || !canonical_text(&rejection.reason))
            {
                return invalid_snapshot("boundary stale-maintenance receipt is malformed");
            }
        }
        let expected_maintenance_root =
            if previous_maintenance_root.is_some() || !record.maintenance_changes.is_empty() {
                Some(canonical_hash(
                    "canwu.maintenance.terminal-root.v1",
                    &(
                        previous_maintenance_root
                            .as_deref()
                            .unwrap_or(GENESIS_BOUNDARY_HASH),
                        &record.maintenance_changes,
                    ),
                )?)
            } else {
                None
            };
        if record.maintenance_terminal_root != expected_maintenance_root {
            return invalid_snapshot("boundary maintenance terminal root is inconsistent");
        }
        if record.previous_hash != previous_hash
            || !is_canonical_hash(&record.hash)
            || (requires_state_hash && record.state_hash.is_none())
            || compute_boundary_hash(record).map_err(|error| {
                invalid_snapshot_error(format!("could not verify boundary hash: {error}"))
            })? != record.hash
        {
            return invalid_snapshot("boundary hash chain is inconsistent");
        }

        max_boundary_id = record.id.get();
        max_correlation_id = max_correlation_id.max(record.correlation_id);
        previous_boundary = Some((record.at, record.id));
        previous_hash.clone_from(&record.hash);
        previous_maintenance_root.clone_from(&record.maintenance_terminal_root);
        history.apply_boundary(index + 1, &cuts)?;
    }
    let boundary_states: BTreeSet<_> = plugins
        .boundary_writers
        .keys()
        .map(|(_, state)| state.clone())
        .collect();
    let persisted_boundary_values: BTreeMap<_, _> = snapshot
        .plugin_components
        .iter()
        .filter(|record| boundary_states.contains(&record.state))
        .map(|record| {
            (
                component_key(
                    &record.plugin,
                    &record.state,
                    &record.entity,
                    &record.component,
                ),
                record.value.clone(),
            )
        })
        .collect();
    if persisted_boundary_values != boundary_values {
        return invalid_snapshot(
            "boundary changes do not materialize the persisted component state",
        );
    }
    if &domain_record_values != final_domain_records {
        return invalid_snapshot(
            "boundary domain-record changes do not materialize the persisted record state",
        );
    }
    if knowledge_values != snapshot.knowledge.records {
        return invalid_snapshot(
            "boundary knowledge changes do not reconstruct the persisted generic ledger",
        );
    }
    Ok((
        max_boundary_id,
        max_correlation_id,
        history,
        PersistedAdmissionCursors {
            attempts: u64::try_from(next_attempt).map_err(|_| {
                invalid_snapshot_error("admitted attempt cursor exceeds persisted range")
            })?,
            commands: u64::try_from(next_command).map_err(|_| {
                invalid_snapshot_error("admitted command cursor exceeds persisted range")
            })?,
            events: u64::try_from(next_event).map_err(|_| {
                invalid_snapshot_error("admitted event cursor exceeds persisted range")
            })?,
        },
    ))
}

fn validate_boundary_admission(
    record: &BoundaryRecord,
    snapshot: &SimulationSnapshot,
    next_attempt: &mut usize,
    next_command: &mut usize,
    next_event: &mut usize,
) -> Result<(), CanwuError> {
    if record
        .admitted_attempts
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
        || record
            .admitted_commands
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || record
            .admitted_events
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return invalid_snapshot("boundary admission lists are not canonical");
    }
    let mut accepted_attempt_commands = Vec::new();
    for id in &record.admitted_attempts {
        let Some(attempt) = snapshot.command_attempts.get(*next_attempt) else {
            return invalid_snapshot("boundary admits a command attempt beyond the journal prefix");
        };
        if attempt.id != *id || attempt.at > record.at {
            return invalid_snapshot(
                "boundary command-attempt admission is out of order or premature",
            );
        }
        if let CommandAttemptOutcome::Accepted { command_id } = attempt.outcome {
            accepted_attempt_commands.push(command_id);
        }
        *next_attempt += 1;
    }
    if snapshot
        .command_attempts
        .get(*next_attempt)
        .is_some_and(|attempt| attempt.at < record.at)
    {
        return invalid_snapshot(
            "boundary omitted an earlier command attempt from its admission cut",
        );
    }
    if !snapshot.command_attempts.is_empty()
        && accepted_attempt_commands != record.admitted_commands
    {
        return invalid_snapshot(
            "boundary command admission does not match its accepted attempt evidence",
        );
    }
    for id in &record.admitted_commands {
        let Some(command) = snapshot.commands.get(*next_command) else {
            return invalid_snapshot("boundary admits a command beyond the journal prefix");
        };
        if command.id != *id || command.accepted_at > record.at {
            return invalid_snapshot("boundary command admission is out of order or premature");
        }
        *next_command += 1;
    }
    if snapshot
        .commands
        .get(*next_command)
        .is_some_and(|command| command.accepted_at < record.at)
    {
        return invalid_snapshot("boundary omitted an earlier command from its admission cut");
    }

    for id in &record.admitted_events {
        let Some(event) = snapshot.events.get(*next_event) else {
            return invalid_snapshot("boundary admits an event beyond the journal prefix");
        };
        if event.id != *id || event.timestamp > record.at {
            return invalid_snapshot("boundary event admission is out of order or premature");
        }
        match &event.cause {
            Some(CauseRef::Boundary(boundary)) if *boundary >= record.id => {
                return invalid_snapshot("boundary admitted an event from its own or a later cut");
            }
            Some(CauseRef::Command(command))
                if usize::try_from(command.get())
                    .map_or(true, |command_number| command_number > *next_command) =>
            {
                return invalid_snapshot("boundary admitted an event before its command cause");
            }
            Some(CauseRef::Event(parent))
                if usize::try_from(parent.get())
                    .map_or(true, |event_number| event_number > *next_event) =>
            {
                return invalid_snapshot("boundary admitted an event before its parent cause");
            }
            Some(
                CauseRef::Boundary(_)
                | CauseRef::Command(_)
                | CauseRef::Event(_)
                | CauseRef::System(_),
            )
            | None => {}
        }
        *next_event += 1;
    }
    if let Some(event) = snapshot.events.get(*next_event) {
        let precedes_current_emission = record
            .emissions
            .first()
            .is_some_and(|emission| event.id < emission.event);
        let comes_from_earlier_boundary = matches!(
            &event.cause,
            Some(CauseRef::Boundary(boundary)) if *boundary < record.id
        );
        let comes_from_admitted_command = matches!(
            &event.cause,
            Some(CauseRef::Command(command))
                if usize::try_from(command.get())
                    .is_ok_and(|command_number| command_number <= *next_command)
        );
        let comes_from_admitted_parent = matches!(
            &event.cause,
            Some(CauseRef::Event(parent))
                if usize::try_from(parent.get())
                    .is_ok_and(|event_number| event_number <= *next_event)
        );
        if event.timestamp < record.at
            || (event.timestamp == record.at
                && (precedes_current_emission
                    || comes_from_earlier_boundary
                    || comes_from_admitted_command
                    || comes_from_admitted_parent))
        {
            return invalid_snapshot("boundary omitted an existing event from its admission cut");
        }
    }
    Ok(())
}

fn validate_boundary_reservations(
    record: &BoundaryRecord,
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    cuts: &BoundaryDomainEntityCuts,
) -> Result<(), CanwuError> {
    if record.reservation_offers.windows(2).any(|pair| {
        (&pair[0].offer.pool, &pair[0].plugin, &pair[0].system)
            >= (&pair[1].offer.pool, &pair[1].plugin, &pair[1].system)
    }) {
        return invalid_snapshot("boundary reservation offers are not canonical");
    }
    let mut remaining = BTreeMap::new();
    for offered in &record.reservation_offers {
        validate_snapshot_reservation_pool(&offered.offer.pool, snapshot, final_records, cuts)?;
        let Some(contract) = snapshot_boundary_contract(plugins, &offered.plugin, &offered.system)
        else {
            return invalid_snapshot("reservation offer references an unknown boundary system");
        };
        if contract.phase != BoundaryPhase::ReservationAndAllocation
            || !boundary_system_due(
                contract,
                &record.cadences,
                boundary_has_event_ingress(record),
            )
            || !contract
                .reservation_offers
                .contains(&offered.offer.pool.state)
            || plugins.reservation_offerers.get(&offered.offer.pool.state)
                != Some(&(offered.plugin.clone(), offered.system.clone()))
            || remaining
                .insert(offered.offer.pool.clone(), offered.offer.capacity)
                .is_some()
        {
            return invalid_snapshot("boundary reservation offer is unauthorized or duplicated");
        }
    }

    if record.reservation_requests.windows(2).any(|pair| {
        compare_reservation_request_records(&pair[0], &pair[1]) != std::cmp::Ordering::Less
    }) || record.allocations.len() != record.reservation_requests.len()
    {
        return invalid_snapshot("boundary reservation requests or allocations are not canonical");
    }
    let mut request_refs = BTreeSet::new();
    for (requested, allocation) in record.reservation_requests.iter().zip(&record.allocations) {
        validate_snapshot_reservation_pool(&requested.request.pool, snapshot, final_records, cuts)?;
        let Some(contract) = snapshot_boundary_contract(
            plugins,
            &requested.reservation.plugin,
            &requested.reservation.system,
        ) else {
            return invalid_snapshot("reservation request references an unknown boundary system");
        };
        if requested.reservation.request != requested.request.request
            || requested.request.request.trim().is_empty()
            || requested.request.request != requested.request.request.trim()
            || requested.request.tie_break.trim().is_empty()
            || requested.request.tie_break != requested.request.tie_break.trim()
            || requested.request.quantity == 0
            || contract.phase != BoundaryPhase::ReservationAndAllocation
            || !boundary_system_due(
                contract,
                &record.cadences,
                boundary_has_event_ingress(record),
            )
            || !contract
                .reservation_requests
                .contains(&requested.request.pool.state)
            || !request_refs.insert(requested.reservation.clone())
        {
            return invalid_snapshot("boundary reservation request is invalid");
        }
        let available = remaining.entry(requested.request.pool.clone()).or_default();
        let granted = requested.request.quantity.min(*available);
        *available -= granted;
        let disposition = if granted == requested.request.quantity {
            ReservationDisposition::Fulfilled
        } else if granted == 0 {
            ReservationDisposition::Rejected
        } else {
            ReservationDisposition::Partial
        };
        let expected = ReservationAllocation {
            reservation: requested.reservation.clone(),
            pool: requested.request.pool.clone(),
            requested: requested.request.quantity,
            granted,
            remaining_after: *available,
            disposition,
        };
        if allocation != &expected {
            return invalid_snapshot("boundary reservation allocation evidence is inconsistent");
        }
    }
    Ok(())
}

fn compare_reservation_request_records(
    left: &ReservationRequestRecord,
    right: &ReservationRequestRecord,
) -> std::cmp::Ordering {
    left.request
        .pool
        .cmp(&right.request.pool)
        .then_with(|| right.request.priority.cmp(&left.request.priority))
        .then_with(|| left.request.tie_break.cmp(&right.request.tie_break))
        .then_with(|| left.reservation.cmp(&right.reservation))
}

fn validate_snapshot_reservation_pool(
    pool: &ReservationPoolKey,
    snapshot: &SimulationSnapshot,
    final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    cuts: &BoundaryDomainEntityCuts,
) -> Result<(), CanwuError> {
    if pool.resource.trim().is_empty()
        || pool.resource != pool.resource.trim()
        || !snapshot_entity_exists_at_boundary(snapshot, final_records, cuts, None, &pool.entity)
    {
        return invalid_snapshot("snapshot contains an invalid reservation pool");
    }
    Ok(())
}

fn snapshot_boundary_contract<'a>(
    plugins: &'a PluginRegistry,
    plugin: &str,
    system: &str,
) -> Option<&'a BoundarySystemContract> {
    plugins
        .descriptors
        .get(plugin)?
        .boundary_systems
        .iter()
        .find(|contract| contract.name == system)
}

fn validate_boundary_ingress_generation(
    record: &BoundaryRecord,
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    cuts: &BoundaryDomainEntityCuts,
) -> Result<(), CanwuError> {
    for generation in &record.generated_ingress {
        let Some(contract) =
            snapshot_boundary_contract(plugins, &generation.plugin, &generation.system)
        else {
            return invalid_snapshot("generated ingress references an unknown boundary system");
        };
        let Some(commit_stage) = domain_record_commit_stage(contract.phase, contract.visibility)
        else {
            return invalid_snapshot("generated ingress has no deterministic commit stage");
        };
        let index = usize::try_from(generation.ingress.get().saturating_sub(1)).map_err(|_| {
            invalid_snapshot_error("generated ingress ID exceeds the journal index range")
        })?;
        let Some(ingress) = snapshot.ingress.get(index) else {
            return invalid_snapshot("generated ingress references an unknown ingress record");
        };
        let generated_delay = ingress.due_at.checked_sub(ingress.issued_at);
        let payload_is_valid = match &ingress.payload {
            IngressPayload::Plugin {
                plugin,
                packet_type,
                affected_entities,
                ..
            } => {
                (plugin == &generation.plugin
                    || contract
                        .plugin_ingress_targets
                        .contains(&PluginIngressTarget {
                            target_plugin: plugin.clone(),
                            packet_type: packet_type.clone(),
                        }))
                    && affected_entities.iter().all(|entity| match entity {
                        EntityRef::Domain(reference) => cuts.identity_exists_for_proposal(
                            final_records,
                            reference,
                            contract.phase,
                            commit_stage,
                            &generation.plugin,
                            &generation.system,
                        ),
                        _ => core_world_entity_exists(&snapshot.world, entity),
                    })
            }
            IngressPayload::Decision { request } => {
                let DecisionMutation::Resolve {
                    ticket_id,
                    expected_version,
                    policy,
                    decision,
                    command_request_id,
                    ..
                } = &request.mutation
                else {
                    return invalid_snapshot(
                        "boundary-generated decision ingress must resolve an existing ticket",
                    );
                };
                let Some(random) = &decision.random else {
                    return invalid_snapshot(
                        "boundary-generated decision ingress lacks random evidence",
                    );
                };
                let Some(draw) = snapshot.random_draws.get(
                    usize::try_from(random.draw_id.get().saturating_sub(1)).unwrap_or(usize::MAX),
                ) else {
                    return invalid_snapshot(
                        "boundary-generated decision references an unknown random draw",
                    );
                };
                let DecisionOutcome::Selected { option_id } = &decision.outcome else {
                    return invalid_snapshot(
                        "boundary-generated random decision must select an option",
                    );
                };
                let producer_matches = matches!(
                    &draw.producer,
                    RandomDrawProducer::BoundarySystem {
                        boundary,
                        plugin,
                        system,
                    } if *boundary == record.id
                        && plugin == &generation.plugin
                        && system == &generation.system
                );
                let outcome_matches = matches!(
                    &draw.outcome,
                    Some(RandomDrawOutcome::DecisionSelection {
                        ticket_id: outcome_ticket,
                        ticket_version,
                        option_id: outcome_option,
                    }) if outcome_ticket == ticket_id
                        && ticket_version == expected_version
                        && outcome_option == option_id
                );
                let target_matches = matches!(
                    &draw.address,
                    RandomDrawAddress::OperationV1(address)
                        if address.target
                            == (RandomOperationTarget::DecisionTicket {
                                ticket_id: *ticket_id,
                                ticket_version: *expected_version,
                            })
                );
                let weights_match = DecisionRandomEvidence::selected_option_from_weights(
                    &random.option_weights,
                    random.value,
                    random.upper_exclusive,
                )
                .is_ok_and(|selected| selected == *option_id);
                policy.kind == DecisionPolicyKind::Random
                    && producer_matches
                    && outcome_matches
                    && target_matches
                    && weights_match
                    && random.value == draw.value
                    && random.upper_exclusive == draw.upper_exclusive
                    && decision.external.is_none()
                    && *command_request_id
                        == request.command.as_ref().map(|command| command.request_id)
            }
            IngressPayload::Command { .. }
            | IngressPayload::Calendar { .. }
            | IngressPayload::Maintenance { .. } => false,
        };
        if generation.phase != contract.phase
            || generation.visibility != contract.visibility
            || !payload_is_valid
            || ingress.id != generation.ingress
            || ingress.issued_at != record.at
            || ingress.eligible_boundary_count != record.id.get()
            || ingress.cause != Some(CauseRef::Boundary(record.id))
            || generated_delay.is_none_or(SimDuration::is_negative)
            || generated_delay.and_then(|delay| ingress.issued_at.checked_add(delay))
                != Some(ingress.due_at)
            || !boundary_system_due(
                contract,
                &record.cadences,
                boundary_has_event_ingress(record),
            )
        {
            return invalid_snapshot(
                "generated ingress producer, commit stage, or entity provenance is inconsistent",
            );
        }
    }
    Ok(())
}

fn validate_boundary_changes(
    record: &BoundaryRecord,
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    cuts: &BoundaryDomainEntityCuts,
) -> Result<(), CanwuError> {
    let mut change_keys = BTreeSet::new();
    for change in &record.changes {
        let Some(contract) = snapshot_boundary_contract(plugins, &change.plugin, &change.system)
        else {
            return invalid_snapshot("boundary change references an unknown system");
        };
        let Some(stage) = boundary_write_stage(contract.phase) else {
            return invalid_snapshot("boundary change references a non-writing phase");
        };
        let Some(commit_stage) = domain_record_commit_stage(contract.phase, change.visibility)
        else {
            return invalid_snapshot("boundary change has no deterministic commit stage");
        };
        if change.component.trim().is_empty()
            || change.component != change.component.trim()
            || !snapshot_entity_exists_for_boundary_proposal(
                snapshot,
                final_records,
                cuts,
                contract,
                commit_stage,
                (&change.plugin, &change.system),
                &change.entity,
            )
            || !contract.writes.contains(&change.state)
            || !boundary_system_due(
                contract,
                &record.cadences,
                boundary_has_event_ingress(record),
            )
            || contract.visibility != change.visibility
            || plugins.state_owners.get(&change.state) != Some(&change.plugin)
            || plugins.boundary_writers.get(&(stage, change.state.clone()))
                != Some(&(change.plugin.clone(), change.system.clone()))
            || !change_keys.insert((
                change.plugin.clone(),
                change.system.clone(),
                change.state.clone(),
                change.entity.clone(),
                change.component.clone(),
            ))
        {
            return invalid_snapshot("boundary change is unauthorized or duplicated");
        }
    }
    Ok(())
}

fn validate_boundary_record_changes(
    record: &BoundaryRecord,
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    values: &mut BTreeMap<DomainRecordRef, DomainRecord>,
) -> Result<BoundaryDomainEntityCuts, CanwuError> {
    let mut by_stage = BTreeMap::<DomainRecordCommitStage, Vec<&DomainRecordChange>>::new();
    let mut previous_order = None;
    for change in &record.record_changes {
        let Some(contract) = snapshot_boundary_contract(plugins, &change.plugin, &change.system)
        else {
            return invalid_snapshot("domain-record change references an unknown boundary system");
        };
        let Some(write_stage) = boundary_write_stage(contract.phase) else {
            return invalid_snapshot("domain-record change references a non-writing phase");
        };
        let Some(commit_stage) = domain_record_commit_stage(contract.phase, change.visibility)
        else {
            return invalid_snapshot("domain-record change has no deterministic commit stage");
        };
        let reference = &change.current.reference;
        let state = records::record_state_key(&reference.kind);
        let order = (commit_stage, reference.clone());
        if !canonical_text(&change.summary)
            || previous_order
                .as_ref()
                .is_some_and(|previous| previous >= &order)
            || change
                .previous
                .as_ref()
                .is_some_and(|previous| previous.reference != *reference)
            || !contract.writes.contains(&state)
            || !boundary_system_due(
                contract,
                &record.cadences,
                boundary_has_event_ingress(record),
            )
            || contract.visibility != change.visibility
            || plugins.state_owners.get(&state) != Some(&change.plugin)
            || plugins.boundary_writers.get(&(write_stage, state.clone()))
                != Some(&(change.plugin.clone(), change.system.clone()))
            || plugins
                .record_schemas
                .get(&reference.kind)
                .is_none_or(|(owner, _)| owner != &change.plugin)
        {
            return invalid_snapshot(
                "boundary domain-record change is unauthorized, duplicated, or noncanonical",
            );
        }
        previous_order = Some(order);
        by_stage.entry(commit_stage).or_default().push(change);
    }

    let mut cuts = BoundaryDomainEntityCuts::default();
    for stage in DomainRecordCommitStage::ALL {
        if let Some(changes) = by_stage.get(&stage) {
            let mutations: Vec<_> = changes
                .iter()
                .map(|change| records::mutation_from_change(change))
                .collect();
            let requests: Vec<_> = changes
                .iter()
                .zip(&mutations)
                .map(|(change, mutation)| records::DomainMutationRequest {
                    plugin: &change.plugin,
                    system: &change.system,
                    visibility: change.visibility,
                    mutation,
                    summary: &change.summary,
                })
                .collect();
            let (next, applied) = records::apply_mutation_bundle(
                values,
                &plugins.record_schemas,
                record.at,
                &|entity| core_world_entity_exists(&snapshot.world, entity),
                requests,
            )
            .map_err(|error| {
                invalid_snapshot_error(format!(
                    "boundary domain-record transition is invalid: {error}"
                ))
            })?;
            let recorded: Vec<_> = changes.iter().map(|change| (*change).clone()).collect();
            if applied != recorded {
                return invalid_snapshot(
                    "boundary domain-record transition evidence disagrees with deterministic replay",
                );
            }
            for change in &applied {
                cuts.record(stage, change);
            }
            *values = next;
        }
    }
    Ok(cuts)
}

fn validate_boundary_knowledge_changes(
    record: &BoundaryRecord,
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    cuts: &BoundaryDomainEntityCuts,
    values: &mut BTreeMap<KnowledgeHolderRef, BTreeMap<KnowledgeRecordId, KnowledgeRecord>>,
    next_id: &mut u64,
) -> Result<(), CanwuError> {
    let mut previous_order = None;
    let mut correlations = BTreeSet::new();
    let mut visible_current_boundary = BTreeSet::new();
    for change in &record.knowledge_changes {
        let Some(contract) = snapshot_boundary_contract(plugins, &change.plugin, &change.system)
        else {
            return invalid_snapshot("knowledge change references an unknown boundary system");
        };
        let order = (change.phase, change.plugin.clone(), change.system.clone());
        if previous_order
            .as_ref()
            .is_some_and(|previous| previous > &order)
            || change.phase != contract.phase
            || !matches!(
                change.phase,
                BoundaryPhase::PerceptionAndAttentionRefresh
                    | BoundaryPhase::PerspectiveAndReportMaterialization
            )
            || !boundary_system_due(
                contract,
                &record.cadences,
                boundary_has_event_ingress(record),
            )
            || change.records.is_empty()
            || change.records.len() > crate::KnowledgeLimitsV1::CURRENT.records_per_batch
            || !canonical_text(&change.summary)
            || change.summary.len() > crate::KnowledgeLimitsV1::CURRENT.text_bytes
        {
            return invalid_snapshot(
                "boundary knowledge change is unauthorized, empty, or noncanonical",
            );
        }
        previous_order = Some(order);
        if let Some(correlation) = &change.producer_correlation
            && (!canonical_text(correlation)
                || correlation.len() > 256
                || !correlations.insert((
                    change.plugin.clone(),
                    change.system.clone(),
                    correlation.clone(),
                )))
        {
            return invalid_snapshot("boundary knowledge producer correlation is invalid");
        }
        if !snapshot_knowledge_holder_exists_for_change(
            snapshot,
            plugins,
            final_records,
            cuts,
            change.phase,
            &change.holder,
        ) {
            return invalid_snapshot(
                "boundary knowledge holder was unavailable at its proposal cut",
            );
        }
        for stored in &change.records {
            if stored.id.get() != *next_id
                || stored.holder != change.holder
                || stored.learned_at != record.at
            {
                return invalid_snapshot(
                    "boundary knowledge IDs, holder, or learned time are inconsistent",
                );
            }
            let Some(grant) = contract
                .knowledge_writes
                .iter()
                .find(|grant| grant.schema == stored.schema)
            else {
                return invalid_snapshot("boundary knowledge schema was not granted");
            };
            if !grant.visibilities.contains(&change.visibility)
                || plugins
                    .knowledge_schemas
                    .get(&stored.schema)
                    .is_none_or(|(owner, schema)| owner != &change.plugin || !schema.writable)
            {
                return invalid_snapshot("boundary knowledge schema ownership is inconsistent");
            }
            let holder_records = values.entry(change.holder.clone()).or_default();
            for related in stored.supersedes.iter().chain(&stored.contradicts) {
                let Some(related_record) = holder_records.get(related) else {
                    return invalid_snapshot(
                        "boundary knowledge relation targets unavailable history",
                    );
                };
                if related.get() >= stored.id.get()
                    || related_record.schema.kind != stored.schema.kind
                    || (related_record.learned_at == record.at
                        && !visible_current_boundary.contains(related))
                {
                    return invalid_snapshot(
                        "boundary knowledge relation is forward, cross-kind, or hidden at its cut",
                    );
                }
            }
            if holder_records.insert(stored.id, stored.clone()).is_some() {
                return invalid_snapshot("boundary knowledge record ID is duplicated");
            }
            if change.phase == BoundaryPhase::PerceptionAndAttentionRefresh
                && change.visibility == StateVisibility::SameBoundary
            {
                visible_current_boundary.insert(stored.id);
            }
            *next_id = next_id.checked_add(1).ok_or_else(|| {
                invalid_snapshot_error("knowledge record identifier space is exhausted")
            })?;
        }
    }
    Ok(())
}

fn snapshot_knowledge_holder_exists_for_change(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    cuts: &BoundaryDomainEntityCuts,
    phase: BoundaryPhase,
    holder: &KnowledgeHolderRef,
) -> bool {
    match holder {
        KnowledgeHolderRef::Person(person) => {
            snapshot_entity_exists(snapshot, &EntityRef::Person(*person))
        }
        KnowledgeHolderRef::Entity(EntityRef::Domain(reference)) => {
            let stage = (phase == BoundaryPhase::PerspectiveAndReportMaterialization)
                .then_some(DomainRecordCommitStage::Aggregation);
            cuts.is_live(final_records, reference, stage)
                && plugins
                    .record_schemas
                    .get(&reference.kind)
                    .is_some_and(|(_, schema)| {
                        schema.class == DomainRecordClass::Entity
                            && schema.holder_policy == KnowledgeHolderPolicy::Allowed
                    })
        }
        KnowledgeHolderRef::Entity(entity) => snapshot_entity_exists(snapshot, entity),
    }
}

fn validate_boundary_emissions(
    record: &BoundaryRecord,
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    cuts: &BoundaryDomainEntityCuts,
    emitted_events: &mut BTreeSet<EventId>,
) -> Result<(), CanwuError> {
    if record
        .emissions
        .windows(2)
        .any(|pair| pair[0].event >= pair[1].event)
    {
        return invalid_snapshot("boundary emitted event IDs are not canonical");
    }
    let mut matched_changes = BTreeSet::new();
    let mut matched_record_changes = BTreeSet::new();
    let mut matched_knowledge_changes = BTreeSet::new();
    for emission in &record.emissions {
        let Some(event) = snapshot_event_by_id(snapshot, emission.event) else {
            return invalid_snapshot("boundary references an unknown emitted event");
        };
        if event.timestamp != record.at
            || event.correlation_id != record.correlation_id
            || event.cause != Some(CauseRef::Boundary(record.id))
            || !emitted_events.insert(emission.event)
        {
            return invalid_snapshot("boundary emitted event evidence is inconsistent");
        }
        let Some(contract) =
            snapshot_boundary_contract(plugins, &emission.plugin, &emission.system)
        else {
            return invalid_snapshot("boundary emission references an unknown system");
        };
        if !boundary_system_due(
            contract,
            &record.cadences,
            boundary_has_event_ingress(record),
        ) {
            return invalid_snapshot("boundary emission source system was not due");
        }
        match emission.kind {
            BoundaryEmissionKind::Change { change_index } => {
                let Some((plugin, event_type)) = event.kind.plugin_identity() else {
                    return invalid_snapshot("boundary change emitted a non-plugin event");
                };
                if plugin != emission.plugin.as_str() {
                    return invalid_snapshot("boundary emission plugin provenance is inconsistent");
                }
                let Some(commit_stage) =
                    domain_record_commit_stage(contract.phase, contract.visibility)
                else {
                    return invalid_snapshot("boundary emission has no deterministic commit stage");
                };
                let index = usize::try_from(change_index).map_err(|_| {
                    invalid_snapshot_error("boundary change evidence index is out of range")
                })?;
                let Some(change) = record.changes.get(index) else {
                    return invalid_snapshot("boundary emission references an unknown change");
                };
                if !matched_changes.insert(change_index)
                    || emission.plugin != change.plugin
                    || emission.system != change.system
                    || event.summary != change.summary
                    || event.affected_entities != vec![change.entity.clone()]
                    || event_type != format!("{}_changed", change.component)
                    || !snapshot_entity_exists_for_boundary_proposal(
                        snapshot,
                        final_records,
                        cuts,
                        contract,
                        commit_stage,
                        (&emission.plugin, &emission.system),
                        &change.entity,
                    )
                {
                    return invalid_snapshot("boundary change evidence provenance is inconsistent");
                }
            }
            BoundaryEmissionKind::KnowledgeChange { change_index } => {
                let index = usize::try_from(change_index).map_err(|_| {
                    invalid_snapshot_error("knowledge change evidence index is out of range")
                })?;
                let Some(change) = record.knowledge_changes.get(index) else {
                    return invalid_snapshot(
                        "boundary emission references an unknown knowledge change",
                    );
                };
                let KnowledgePublished {
                    holder,
                    record_count,
                } = KnowledgePublished::decode(&event.kind).map_err(|_| {
                    invalid_snapshot_error("knowledge change emitted the wrong event kind")
                })?;
                let expected_affected = match &change.holder {
                    KnowledgeHolderRef::Person(person) => vec![EntityRef::Person(*person)],
                    KnowledgeHolderRef::Entity(entity) => vec![entity.clone()],
                };
                if !matched_knowledge_changes.insert(change_index)
                    || emission.plugin != change.plugin
                    || emission.system != change.system
                    || holder != change.holder
                    || usize::try_from(record_count).ok() != Some(change.records.len())
                    || event.summary != change.summary
                    || event.affected_entities != expected_affected
                {
                    return invalid_snapshot(
                        "boundary knowledge evidence provenance is inconsistent",
                    );
                }
            }
            BoundaryEmissionKind::RecordChange { change_index } => {
                let Some((plugin, event_type)) = event.kind.plugin_identity() else {
                    return invalid_snapshot("domain-record change emitted a non-plugin event");
                };
                if plugin != emission.plugin.as_str() {
                    return invalid_snapshot("boundary emission plugin provenance is inconsistent");
                }
                let index = usize::try_from(change_index).map_err(|_| {
                    invalid_snapshot_error("boundary record-change evidence index is out of range")
                })?;
                let Some(change) = record.record_changes.get(index) else {
                    return invalid_snapshot(
                        "boundary emission references an unknown domain record change",
                    );
                };
                if !matched_record_changes.insert(change_index)
                    || emission.plugin != change.plugin
                    || emission.system != change.system
                    || event.summary != change.summary
                    || event.affected_entities != record_change_affected_entities(change)
                    || event_type != change.operation.event_type()
                {
                    return invalid_snapshot(
                        "boundary domain-record evidence provenance is inconsistent",
                    );
                }
            }
            BoundaryEmissionKind::Explicit => {
                let Some((plugin, event_type)) = event.kind.plugin_identity() else {
                    return invalid_snapshot("explicit boundary emission is not a plugin event");
                };
                if plugin != emission.plugin.as_str() {
                    return invalid_snapshot("boundary emission plugin provenance is inconsistent");
                }
                let Some(commit_stage) =
                    domain_record_commit_stage(contract.phase, contract.visibility)
                else {
                    return invalid_snapshot("boundary emission has no deterministic commit stage");
                };
                if !contract.emits.iter().any(|emitted| emitted == event_type)
                    || event.affected_entities.iter().any(|entity| {
                        !snapshot_entity_exists_for_boundary_proposal(
                            snapshot,
                            final_records,
                            cuts,
                            contract,
                            commit_stage,
                            (&emission.plugin, &emission.system),
                            entity,
                        )
                    })
                {
                    return invalid_snapshot(
                        "boundary explicit event is unauthorized or references unavailable state",
                    );
                }
            }
        }
    }
    if matched_changes.len() != record.changes.len() {
        return invalid_snapshot("boundary change is missing its emitted evidence event");
    }
    if matched_record_changes.len() != record.record_changes.len() {
        return invalid_snapshot(
            "boundary domain-record change is missing its emitted evidence event",
        );
    }
    if matched_knowledge_changes.len() != record.knowledge_changes.len() {
        return invalid_snapshot("boundary knowledge change is missing its emitted evidence event");
    }
    Ok(())
}

fn boundaries_before_legacy_command(
    snapshot: &SimulationSnapshot,
    command: &CommandRecord,
) -> usize {
    snapshot
        .boundaries
        .iter()
        .position(|boundary| boundary.admitted_commands.contains(&command.id))
        .unwrap_or_else(|| {
            snapshot
                .boundaries
                .iter()
                .take_while(|boundary| boundary.at <= command.accepted_at)
                .count()
        })
}

fn validate_snapshot_command(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    envelope: &CommandEnvelope,
    history: &DomainRecordHistory,
    cut: DomainHistoryCut,
) -> Result<(), CanwuError> {
    match &envelope.issuer {
        Issuer::Actor(actor)
            if !snapshot_entity_exists_in_history(
                snapshot,
                history,
                cut,
                &EntityRef::Person(*actor),
            ) =>
        {
            return invalid_snapshot("command issuer actor is missing");
        }
        Issuer::System(name) if !canonical_text(name) => {
            return invalid_snapshot("system command issuer ID is not canonical");
        }
        Issuer::Human(name)
        | Issuer::Ai(name)
        | Issuer::Institution(name)
        | Issuer::Replay(name)
        | Issuer::Experiment(name)
            if !canonical_text(name) =>
        {
            return invalid_snapshot("typed command issuer ID is not canonical");
        }
        Issuer::Actor(_)
        | Issuer::Human(_)
        | Issuer::Ai(_)
        | Issuer::Institution(_)
        | Issuer::Replay(_)
        | Issuer::Experiment(_)
        | Issuer::Debug
        | Issuer::System(_) => {}
    }
    if let Some(authority) = &envelope.authority {
        validate_command_authority(authority, &|entity| {
            snapshot_entity_exists_in_history(snapshot, history, cut, entity)
        })
        .map_err(|error| {
            invalid_snapshot_error(format!("command authority is invalid: {error}"))
        })?;
    }
    match &envelope.command {
        Command::OrderMovement {
            subject,
            destination,
            cargo,
        } => {
            if !snapshot_entity_exists_in_history(snapshot, history, cut, subject)
                || snapshot.world.territory(*destination).is_none()
                || cargo.windows(2).any(|pair| pair[0] >= pair[1])
                || cargo
                    .iter()
                    .any(|letter| snapshot.world.letter(*letter).is_none())
            {
                return invalid_snapshot("movement command references invalid entities or cargo");
            }
        }
        Command::DebugSetArmyMorale { army, morale } => {
            if snapshot.world.army(*army).is_none() || *morale > 100 {
                return invalid_snapshot("debug morale command is invalid");
            }
        }
        Command::Plugin {
            plugin,
            command,
            payload,
        } => {
            let Some(descriptor) = plugins.descriptors.get(plugin) else {
                return invalid_snapshot("plugin command references an unknown plugin");
            };
            let Some(action) = descriptor
                .commands
                .iter()
                .find(|candidate| candidate.name == *command)
            else {
                return invalid_snapshot("plugin command is absent from its manifest");
            };
            action.payload_schema.validate(payload).map_err(|error| {
                invalid_snapshot_error(format!("plugin command payload is invalid: {error}"))
            })?;
        }
    }
    Ok(())
}

fn event_index(event_id: EventId) -> Option<usize> {
    event_id
        .get()
        .checked_sub(1)
        .and_then(|value| usize::try_from(value).ok())
}

fn validate_event_kind(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    event: &SimEvent,
    entity_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(), CanwuError> {
    let valid = match event.kind.event_type() {
        MOVE_ORDERED => MoveOrdered::decode(&event.kind).is_ok_and(|payload| {
            snapshot.world.army(payload.army).is_some()
                && snapshot.world.territory(payload.from).is_some()
                && snapshot.world.territory(payload.to).is_some()
                && payload.arrival_at >= event.timestamp
        }),
        ARMY_ARRIVED => ArmyArrived::decode(&event.kind).is_ok_and(|payload| {
            snapshot.world.army(payload.army).is_some()
                && snapshot.world.territory(payload.territory).is_some()
        }),
        PERSON_MOVE_ORDERED => PersonMoveOrdered::decode(&event.kind).is_ok_and(|payload| {
            snapshot.world.person(payload.person).is_some()
                && snapshot.world.territory(payload.from).is_some()
                && snapshot.world.territory(payload.to).is_some()
                && payload.arrival_at > event.timestamp
        }),
        PERSON_ARRIVED => PersonArrived::decode(&event.kind).is_ok_and(|payload| {
            snapshot.world.person(payload.person).is_some()
                && snapshot.world.territory(payload.territory).is_some()
        }),
        LETTER_DELIVERED => LetterDelivered::decode(&event.kind).is_ok_and(|payload| {
            snapshot.world.letter(payload.letter).is_some()
                && snapshot.world.person(payload.carrier).is_some()
                && snapshot.world.person(payload.recipient).is_some()
                && snapshot.world.territory(payload.territory).is_some()
        }),
        REPORT_DISPATCHED => ReportDispatched::decode(&event.kind).is_ok_and(|payload| {
            snapshot.world.person(payload.recipient).is_some()
                && snapshot.world.army(payload.army).is_some()
                && payload.arrives_at >= event.timestamp
        }),
        KNOWLEDGE_UPDATED => KnowledgeUpdated::decode(&event.kind).is_ok_and(|payload| {
            snapshot.world.person(payload.recipient).is_some()
                && snapshot.world.army(payload.army).is_some()
                && snapshot.world.territory(payload.known_location).is_some()
        }),
        KNOWLEDGE_PUBLISHED => {
            KnowledgePublished::decode(&event.kind).is_ok_and(|payload| payload.record_count > 0)
        }
        DEBUG_FIELD_CHANGED => DebugFieldChanged::decode(&event.kind)
            .is_ok_and(|payload| entity_exists(&payload.entity)),
        PLUGIN => event
            .kind
            .plugin_identity()
            .is_some_and(|(plugin, event_type)| {
                plugins.descriptors.contains_key(plugin)
                    && canonical_text(plugin)
                    && canonical_text(event_type)
            }),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        invalid_snapshot("event kind references invalid state")
    }
}

fn validate_scheduled_action(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    key: &ScheduleKey,
    action: &ScheduledAction,
) -> Result<(), CanwuError> {
    match action {
        ScheduledAction::ArmyArrival {
            army,
            destination,
            order_event,
            correlation_id,
        } => {
            let Some(army_state) = snapshot.world.army(*army) else {
                return invalid_snapshot("scheduled army arrival is invalid");
            };
            let Some(transit) = &army_state.transit else {
                return invalid_snapshot("scheduled arrival has no matching army transit");
            };
            let Some(order) = snapshot_event_by_id(snapshot, *order_event) else {
                return invalid_snapshot("scheduled arrival references an unknown order event");
            };
            if !order.kind.is_type(MOVE_ORDERED) {
                return invalid_snapshot("scheduled arrival does not reference a move order event");
            }
            let MoveOrdered {
                army: ordered_army,
                from,
                to,
                arrival_at,
            } = MoveOrdered::decode(&order.kind).map_err(|_| {
                invalid_snapshot_error("scheduled move order event payload is invalid")
            })?;
            let Some(CauseRef::Command(command_id)) = order.cause else {
                return invalid_snapshot("move order event does not reference its command");
            };
            let command_matches = snapshot.commands.iter().any(|record| {
                record.id == command_id
                    && record.accepted_at == order.timestamp
                    && matches!(
                        &record.envelope.command,
                        Command::OrderMovement {
                            subject: EntityRef::Army(commanded_army),
                            destination: commanded_destination,
                            cargo,
                        } if *commanded_army == *army
                            && *commanded_destination == *destination
                            && cargo.is_empty()
                    )
            });
            if !command_matches
                || ordered_army != *army
                || from != transit.from
                || to != *destination
                || transit.to != *destination
                || arrival_at != key.at
                || transit.arrives_at != key.at
                || order.timestamp != transit.departed_at
                || order.correlation_id != *correlation_id
            {
                return invalid_snapshot(
                    "scheduled arrival, transit, move command, and order event disagree",
                );
            }
        }
        ScheduledAction::PersonArrival {
            person,
            destination,
            order_event,
            cargo,
            correlation_id,
        } => {
            let Some(person_state) = snapshot.world.person(*person) else {
                return invalid_snapshot("scheduled person arrival is invalid");
            };
            let Some(transit) = &person_state.transit else {
                return invalid_snapshot("scheduled person arrival has no matching transit");
            };
            let Some(order) = snapshot_event_by_id(snapshot, *order_event) else {
                return invalid_snapshot("scheduled person arrival references an unknown event");
            };
            if !order.kind.is_type(PERSON_MOVE_ORDERED) {
                return invalid_snapshot(
                    "scheduled person arrival does not reference a person order",
                );
            }
            let PersonMoveOrdered {
                person: ordered_person,
                from,
                to,
                arrival_at,
            } = PersonMoveOrdered::decode(&order.kind).map_err(|_| {
                invalid_snapshot_error("scheduled person move order event payload is invalid")
            })?;
            let Some(CauseRef::Command(command_id)) = order.cause else {
                return invalid_snapshot("person movement order does not reference its command");
            };
            let command_matches = snapshot.commands.iter().any(|record| {
                record.id == command_id
                    && record.accepted_at == order.timestamp
                    && matches!(
                        &record.envelope.command,
                        Command::OrderMovement {
                            subject: EntityRef::Person(commanded_person),
                            destination: commanded_destination,
                            cargo: commanded_cargo,
                        } if commanded_person == person
                            && commanded_destination == destination
                            && commanded_cargo == cargo
                    )
            });
            if !command_matches
                || ordered_person != *person
                || from != transit.from
                || to != *destination
                || transit.to != *destination
                || arrival_at != key.at
                || transit.arrives_at != key.at
                || order.timestamp != transit.departed_at
                || order.correlation_id != *correlation_id
            {
                return invalid_snapshot(
                    "scheduled person arrival, transit, command, and order event disagree",
                );
            }
            if cargo.windows(2).any(|pair| pair[0] >= pair[1]) {
                return invalid_snapshot("scheduled person arrival cargo is not canonical");
            }
            for letter_id in cargo {
                let Some(letter) = snapshot.world.letter(*letter_id) else {
                    return invalid_snapshot(
                        "scheduled person arrival references an unknown letter",
                    );
                };
                if letter.status != LetterStatus::InTransit || letter.carrier != Some(*person) {
                    return invalid_snapshot("scheduled person arrival has invalid letter custody");
                }
            }
        }
        ScheduledAction::KnowledgeReport {
            recipient,
            army,
            location,
            observed_at,
            dispatch_event,
            correlation_id,
        } => {
            if snapshot.world.person(*recipient).is_none()
                || snapshot.world.army(*army).is_none()
                || snapshot.world.territory(*location).is_none()
            {
                return invalid_snapshot("scheduled knowledge report is invalid");
            }
            let Some(dispatch) = snapshot_event_by_id(snapshot, *dispatch_event) else {
                return invalid_snapshot("scheduled report references an unknown dispatch event");
            };
            if !dispatch.kind.is_type(REPORT_DISPATCHED) {
                return invalid_snapshot(
                    "scheduled report does not reference a report dispatch event",
                );
            }
            let ReportDispatched {
                recipient: dispatched_recipient,
                army: dispatched_army,
                arrives_at,
            } = ReportDispatched::decode(&dispatch.kind).map_err(|_| {
                invalid_snapshot_error("scheduled report dispatch event payload is invalid")
            })?;
            let Some(CauseRef::Event(arrival_event_id)) = dispatch.cause else {
                return invalid_snapshot("report dispatch does not reference an arrival event");
            };
            let Some(arrival) = snapshot_event_by_id(snapshot, arrival_event_id) else {
                return invalid_snapshot("report dispatch references an unknown arrival event");
            };
            if !arrival.kind.is_type(ARMY_ARRIVED) {
                return invalid_snapshot("report dispatch cause is not an army arrival");
            }
            let ArmyArrived {
                army: arrived_army,
                territory,
            } = ArmyArrived::decode(&arrival.kind)
                .map_err(|_| invalid_snapshot_error("army arrival event payload is invalid"))?;
            if dispatched_recipient != *recipient
                || dispatched_army != *army
                || arrived_army != *army
                || territory != *location
                || arrives_at != key.at
                || dispatch.timestamp != arrival.timestamp
                || *observed_at != arrival.timestamp
                || dispatch.correlation_id != *correlation_id
                || arrival.correlation_id != *correlation_id
            {
                return invalid_snapshot(
                    "scheduled report, dispatch event, and arrival event disagree",
                );
            }
        }
        ScheduledAction::PluginDirective {
            plugin,
            directive,
            allowed_writes,
            cause,
            correlation_id,
        } => {
            let context = SnapshotValidationContext::new(snapshot);
            validate_cause_reference(&context, cause).map_err(|error| {
                invalid_snapshot_error(match error {
                    CauseValidationError::MissingEvidence => {
                        "scheduled directive has an unavailable cause"
                    }
                    CauseValidationError::NonCanonicalSystem => {
                        "scheduled directive has a non-canonical system cause"
                    }
                })
            })?;
            let Some(descriptor) = plugins.descriptors.get(plugin) else {
                return invalid_snapshot("scheduled directive references an unknown plugin");
            };
            let mut canonical_writes = allowed_writes.clone();
            plugins::validate_state_keys(&mut canonical_writes).map_err(|error| {
                invalid_snapshot_error(format!(
                    "scheduled directive has invalid write declarations: {error}"
                ))
            })?;
            if canonical_writes != *allowed_writes
                || !descriptor
                    .commands
                    .iter()
                    .map(|action| &action.writes)
                    .chain(descriptor.systems.iter().map(|system| &system.writes))
                    .any(|writes| writes == allowed_writes)
            {
                return invalid_snapshot(
                    "scheduled directive write access does not match a plugin contract",
                );
            }
            match cause {
                CauseRef::Boundary(id) => {
                    let Some(boundary) = snapshot_boundary_by_id(snapshot, *id) else {
                        return invalid_snapshot(
                            "scheduled directive has an unknown boundary cause",
                        );
                    };
                    if boundary.at > key.at || boundary.correlation_id != *correlation_id {
                        return invalid_snapshot(
                            "scheduled directive disagrees with its boundary correlation",
                        );
                    }
                }
                CauseRef::Command(id) => {
                    let Some(command) = snapshot_command_by_id(snapshot, *id) else {
                        return invalid_snapshot(
                            "scheduled directive has an unknown command cause",
                        );
                    };
                    if command.accepted_at > key.at {
                        return invalid_snapshot(
                            "scheduled directive references a future command cause",
                        );
                    }
                    if command.emitted_events.iter().any(|event_id| {
                        snapshot_event_by_id(snapshot, *event_id)
                            .is_none_or(|event| event.correlation_id != *correlation_id)
                    }) {
                        return invalid_snapshot(
                            "scheduled directive disagrees with its command correlation",
                        );
                    }
                }
                CauseRef::Event(id) => {
                    let Some(event) = snapshot_event_by_id(snapshot, *id) else {
                        return invalid_snapshot("scheduled directive has an unknown event cause");
                    };
                    if event.timestamp > key.at || event.correlation_id != *correlation_id {
                        return invalid_snapshot(
                            "scheduled directive disagrees with its event correlation",
                        );
                    }
                }
                CauseRef::System(name) if !canonical_text(name) => {
                    return invalid_snapshot(
                        "scheduled directive has a non-canonical system cause",
                    );
                }
                CauseRef::System(_) => {}
            }
            validate_directives_with_context(
                &context,
                plugin,
                allowed_writes,
                &plugins.state_owners,
                &plugins.record_schemas,
                std::slice::from_ref(directive.as_ref()),
            )
            .map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    format!("scheduled plugin directive is invalid: {error}"),
                )
            })?;
        }
    }
    Ok(())
}

const fn scheduled_correlation_id(action: &ScheduledAction) -> u64 {
    match action {
        ScheduledAction::ArmyArrival { correlation_id, .. }
        | ScheduledAction::PersonArrival { correlation_id, .. }
        | ScheduledAction::KnowledgeReport { correlation_id, .. }
        | ScheduledAction::PluginDirective { correlation_id, .. } => *correlation_id,
    }
}

fn validate_next_counter(next: u64, maximum_existing: u64, label: &str) -> Result<(), CanwuError> {
    if next == 0 || next <= maximum_existing {
        return invalid_snapshot(format!("next {label} counter is invalid"));
    }
    Ok(())
}

fn validate_contiguous_next_counter(
    next: u64,
    maximum_existing: u64,
    label: &str,
) -> Result<(), CanwuError> {
    let Some(expected) = maximum_existing.checked_add(1) else {
        return invalid_snapshot(format!("{label} identifier space is exhausted"));
    };
    if next != expected {
        return invalid_snapshot(format!("next {label} counter is not contiguous"));
    }
    Ok(())
}

fn validate_contiguous_or_exhausted_next_counter(
    next: u64,
    maximum_existing: u64,
    label: &str,
) -> Result<(), CanwuError> {
    if next == u64::MAX {
        return Ok(());
    }
    validate_contiguous_next_counter(next, maximum_existing, label)
}

pub(super) fn claim_counter(current: u64, label: &str) -> Result<(u64, u64), CanwuError> {
    let Some(next) = current.checked_add(1) else {
        return Err(CanwuError::new(
            ErrorCode::IdentifierExhausted,
            format!("{label} space is exhausted"),
        ));
    };
    if current == 0 {
        return Err(CanwuError::new(
            ErrorCode::InvalidSnapshot,
            format!("next {label} counter cannot be zero"),
        ));
    }
    Ok((current, next))
}

pub(super) fn validate_run_configuration_entities(
    run_configuration: &RunConfigurationSnapshot,
    entities: &[EntityRef],
    world: &WorldSnapshot,
    domain_records: &[DomainRecord],
) -> Result<(), CanwuError> {
    let Some(binding) = run_configuration
        .declared()
        .and_then(|configuration| configuration.seat_binding.as_ref())
    else {
        return Ok(());
    };
    if binding.actor.is_some_and(|actor| {
        entities.binary_search(&EntityRef::Person(actor)).is_err() && world.person(actor).is_none()
    }) || binding.institution.as_ref().is_some_and(|institution| {
        !entity_exists_in_parts(entities, world, domain_records, institution)
    }) {
        return Err(CanwuError::new(
            ErrorCode::InvalidRunConfiguration,
            "run seat binding references an entity absent from the scenario or snapshot",
        ));
    }
    Ok(())
}

pub(super) fn core_world_entity_exists(world: &WorldSnapshot, entity: &EntityRef) -> bool {
    match entity {
        EntityRef::Army(id) => world.army(*id).is_some(),
        EntityRef::Government(id) => world.government(*id).is_some(),
        EntityRef::Person(id) => world.person(*id).is_some(),
        EntityRef::Route(id) => world.route(*id).is_some(),
        EntityRef::Territory(id) => world.territory(*id).is_some(),
        EntityRef::Domain(_) | EntityRef::Organization(_) => false,
        EntityRef::Resource(id) => world.letter(super::LetterId::new(id.get())).is_some(),
    }
}

fn entity_exists_in_parts(
    entities: &[EntityRef],
    world: &WorldSnapshot,
    domain_records: &[DomainRecord],
    entity: &EntityRef,
) -> bool {
    match entity {
        EntityRef::Domain(reference) => domain_records.iter().any(|record| {
            &record.reference == reference
                && record.class == DomainRecordClass::Entity
                && !record.is_deleted()
        }),
        _ => entities.binary_search(entity).is_ok() || core_world_entity_exists(world, entity),
    }
}

fn snapshot_entity_exists(snapshot: &SimulationSnapshot, entity: &EntityRef) -> bool {
    entity_exists_in_parts(
        &snapshot.entities,
        &snapshot.world,
        &snapshot.domain_records,
        entity,
    )
}

pub(super) fn snapshot_entity_exists_in_history(
    snapshot: &SimulationSnapshot,
    history: &DomainRecordHistory,
    cut: DomainHistoryCut,
    entity: &EntityRef,
) -> bool {
    match entity {
        EntityRef::Domain(reference) => history.is_live(reference, cut),
        _ => {
            snapshot.entities.binary_search(entity).is_ok()
                || core_world_entity_exists(&snapshot.world, entity)
        }
    }
}

fn snapshot_entity_identity_exists_in_history(
    snapshot: &SimulationSnapshot,
    history: &DomainRecordHistory,
    cut: DomainHistoryCut,
    entity: &EntityRef,
) -> bool {
    match entity {
        EntityRef::Domain(reference) => history.exists(reference, cut),
        _ => {
            snapshot.entities.binary_search(entity).is_ok()
                || core_world_entity_exists(&snapshot.world, entity)
        }
    }
}

fn snapshot_entity_exists_at_boundary(
    snapshot: &SimulationSnapshot,
    final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    cuts: &BoundaryDomainEntityCuts,
    stage: Option<DomainRecordCommitStage>,
    entity: &EntityRef,
) -> bool {
    match entity {
        EntityRef::Domain(reference) => cuts.is_live(final_records, reference, stage),
        _ => {
            snapshot.entities.binary_search(entity).is_ok()
                || core_world_entity_exists(&snapshot.world, entity)
        }
    }
}

fn snapshot_entity_exists_for_boundary_proposal(
    snapshot: &SimulationSnapshot,
    final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    cuts: &BoundaryDomainEntityCuts,
    contract: &BoundarySystemContract,
    commit_stage: DomainRecordCommitStage,
    source: (&str, &str),
    entity: &EntityRef,
) -> bool {
    match entity {
        EntityRef::Domain(reference) => {
            cuts.is_live(final_records, reference, Some(commit_stage))
                && cuts.is_live_for_proposal(
                    final_records,
                    reference,
                    contract.phase,
                    commit_stage,
                    source.0,
                    source.1,
                )
        }
        _ => {
            snapshot.entities.binary_search(entity).is_ok()
                || core_world_entity_exists(&snapshot.world, entity)
        }
    }
}

fn snapshot_entity_identity_exists(snapshot: &SimulationSnapshot, entity: &EntityRef) -> bool {
    match entity {
        EntityRef::Domain(reference) => snapshot.domain_records.iter().any(|record| {
            &record.reference == reference && record.class == DomainRecordClass::Entity
        }),
        _ => {
            snapshot.entities.binary_search(entity).is_ok()
                || core_world_entity_exists(&snapshot.world, entity)
        }
    }
}

pub(super) fn runtime_entity_exists(state: &RuntimeState, entity: &EntityRef) -> bool {
    runtime_current_entity_exists(&state.current, entity)
}

pub(super) fn runtime_current_entity_exists(
    current: &RuntimeCurrentState,
    entity: &EntityRef,
) -> bool {
    match entity {
        EntityRef::Domain(reference) => {
            records::domain_entity_exists(&current.domain_records, reference)
        }
        _ => current.entities.contains(entity),
    }
}

pub(super) fn runtime_entity_identity_exists(state: &RuntimeState, entity: &EntityRef) -> bool {
    runtime_current_entity_identity_exists(&state.current, entity)
}

fn runtime_current_entity_identity_exists(
    current: &RuntimeCurrentState,
    entity: &EntityRef,
) -> bool {
    match entity {
        EntityRef::Domain(reference) => current
            .domain_records
            .get(reference)
            .is_some_and(|record| record.class == DomainRecordClass::Entity),
        _ => runtime_current_entity_exists(current, entity),
    }
}

pub(super) fn runtime_has_unqueued_command_history(state: &RuntimeState) -> bool {
    state.evidence.archived_unqueued_command_history
        || has_unqueued_command_history(
            &state.evidence.commands,
            &state.evidence.command_attempts,
            &state.evidence.ingress,
        )
}

pub(super) fn has_unqueued_command_history(
    commands: &[CommandRecord],
    attempts: &[CommandAttemptRecord],
    ingress: &[IngressRecord],
) -> bool {
    let queued_requests: BTreeSet<_> = ingress
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
    commands.iter().any(|command| command.attempt_id.is_none())
        || attempts.iter().any(|attempt| {
            attempt
                .request_id
                .is_none_or(|request| !queued_requests.contains(&request))
        })
}

pub(super) fn validate_runtime_cause(
    state: &RuntimeState,
    cause: &CauseRef,
) -> Result<(), CanwuError> {
    validate_cause_reference(&RuntimeValidationContext::new(state), cause).map_err(|_| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            "ingress cause does not reference canonical committed evidence",
        )
    })
}

pub(super) fn runtime_entity_exists_with_record_overlay(
    current: &RuntimeCurrentState,
    record_overlay: &BTreeMap<DomainRecordRef, DomainRecord>,
    entity: &EntityRef,
) -> bool {
    match entity {
        EntityRef::Domain(reference) => record_overlay
            .get(reference)
            .or_else(|| current.domain_records.get(reference))
            .is_some_and(|record| {
                record.class == DomainRecordClass::Entity && !record.is_deleted()
            }),
        _ => runtime_current_entity_exists(current, entity),
    }
}

pub(super) fn proposal_entity_exists(
    current: &RuntimeCurrentState,
    schemas: &records::DomainRecordSchemas,
    record_overlay: &BTreeMap<DomainRecordRef, DomainRecord>,
    proposal: &BoundaryProposal,
    entity: &EntityRef,
) -> bool {
    let EntityRef::Domain(reference) = entity else {
        return runtime_current_entity_exists(current, entity);
    };
    if let Some(mutation) = proposal.directives.iter().rev().find_map(|directive| {
        let BoundaryDirective::MutateRecord { mutation, .. } = directive else {
            return None;
        };
        (mutation.target() == reference).then_some(mutation)
    }) {
        return match mutation {
            DomainRecordMutation::Delete { .. } => false,
            DomainRecordMutation::Create { .. }
            | DomainRecordMutation::Update { .. }
            | DomainRecordMutation::Retire { .. } => schemas
                .get(&reference.kind)
                .is_some_and(|(_, schema)| schema.class == DomainRecordClass::Entity),
        };
    }
    runtime_entity_exists_with_record_overlay(current, record_overlay, entity)
}

pub(super) fn proposal_entity_identity_exists(
    current: &RuntimeCurrentState,
    schemas: &records::DomainRecordSchemas,
    proposal: &BoundaryProposal,
    entity: &EntityRef,
) -> bool {
    let EntityRef::Domain(reference) = entity else {
        return runtime_current_entity_identity_exists(current, entity);
    };
    if current
        .domain_records
        .get(reference)
        .is_some_and(|record| record.class == DomainRecordClass::Entity)
    {
        return true;
    }
    proposal.directives.iter().any(|directive| {
        let BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Create { record },
            ..
        } = directive
        else {
            return false;
        };
        &record.reference == reference
            && schemas
                .get(&reference.kind)
                .is_some_and(|(_, schema)| schema.class == DomainRecordClass::Entity)
    })
}

pub(super) fn validate_runtime_domain_dependents(state: &RuntimeState) -> Result<(), CanwuError> {
    validate_domain_dependents_with_records(
        &state.current.plugin_components,
        &state.scheduler.actions,
        &state.metadata.run_configuration,
        &state.current.domain_records,
    )
}

pub(super) fn validate_domain_dependents_with_records(
    plugin_components: &BTreeMap<PluginComponentKey, PluginComponentRecord>,
    scheduled_actions: &BTreeMap<ScheduleKey, ScheduledAction>,
    run_configuration: &RunConfigurationSnapshot,
    domain_records: &impl records::DomainRecordRead,
) -> Result<(), CanwuError> {
    let unavailable = |entity: &EntityRef| matches!(entity, EntityRef::Domain(reference) if !records::domain_entity_exists(domain_records, reference));
    if plugin_components
        .values()
        .any(|component| unavailable(&component.entity))
    {
        return Err(CanwuError::new(
            ErrorCode::DomainRecordReferenced,
            "a domain entity with persisted plugin components cannot be deleted",
        ));
    }
    if scheduled_actions.values().any(|action| match action {
        ScheduledAction::PluginDirective { directive, .. } => {
            system_directive_has_entity(directive, &unavailable)
        }
        ScheduledAction::ArmyArrival { .. }
        | ScheduledAction::PersonArrival { .. }
        | ScheduledAction::KnowledgeReport { .. } => false,
    }) {
        return Err(CanwuError::new(
            ErrorCode::DomainRecordReferenced,
            "a domain entity referenced by future scheduled work cannot be deleted",
        ));
    }
    if run_configuration
        .declared()
        .and_then(|configuration| configuration.seat_binding.as_ref())
        .and_then(|binding| binding.institution.as_ref())
        .is_some_and(unavailable)
    {
        return Err(CanwuError::new(
            ErrorCode::DomainRecordReferenced,
            "the institution bound to the active run seat cannot be deleted",
        ));
    }
    Ok(())
}

fn system_directive_has_entity(
    directive: &SystemDirective,
    predicate: &dyn Fn(&EntityRef) -> bool,
) -> bool {
    match directive {
        SystemDirective::SetComponent { entity, .. } => predicate(entity),
        SystemDirective::Emit { affected, .. }
        | SystemDirective::EnqueuePluginIngress { affected, .. } => affected.iter().any(predicate),
        SystemDirective::Schedule { directive, .. } => {
            system_directive_has_entity(directive, predicate)
        }
    }
}
