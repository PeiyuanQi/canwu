use super::event_payloads::{KnowledgePublished, RuntimeEventPayload};
use super::validation::{
    EvidenceAvailability, RuntimeValidationContext, resolve_evidence_reference,
};
use super::{
    AssertUnwindSafe, BTreeMap, BTreeSet, BoundaryChange, BoundaryContext, BoundaryDirective,
    BoundaryEmission, BoundaryEmissionKind, BoundaryId, BoundaryIngressGeneration,
    BoundaryKnowledgeChange, BoundaryPhase, BoundaryProposal, BoundaryReceipt, BoundaryRecord,
    BoundaryRequest, BoundaryStateHashFormat, BoundarySystemContract,
    BoundaryTransactionCheckpoint, CanwuError, CauseRef, Command, CommandEnvelope, CommandIngress,
    CommandRequest, CommitmentDomains, DecisionAction, DecisionIngressRequest, DecisionMutation,
    DecisionOutcome, DecisionPolicyKind, DecisionRandomEvidence, DomainRecord, DomainRecordChange,
    DomainRecordRef, DomainRecordVersionSource, EntityRef, ErrorCode, EventKind, EvidenceRef,
    GENESIS_BOUNDARY_HASH, HashSet, IngressPayload, KnowledgeHolderRef, KnowledgeRecord,
    KnowledgeRecordId, PluginComponentKey, PluginComponentRecord, PluginRegistry, PolicyDecision,
    RandomDrawAddress, RandomDrawOutcome, RandomOperationTarget, RefCell, ReservationAllocation,
    ReservationDisposition, ReservationOffer, ReservationOfferRecord, ReservationPoolKey,
    ReservationRef, ReservationRequest, ReservationRequestRecord, RunConfigurationSnapshot,
    RuntimeCurrentState, RuntimeState, ScheduleKey, ScheduledAction, SimTime, Simulation,
    SimulationView, SimulationViewState, StateKey, StateVisibility, SystemCadence, SystemDirective,
    canonical_hash, canonical_text, catch_unwind, claim_counter, component_key,
    compute_boundary_hash, invalid_snapshot_error, is_domain_record_state, proposal_entity_exists,
    proposal_entity_identity_exists, random, record_change_affected_entities, records,
    runtime_current_entity_exists, runtime_entity_exists,
    runtime_entity_exists_with_record_overlay, runtime_entity_identity_exists,
    validate_domain_dependents_with_records, validate_runtime_domain_dependents,
};

impl Simulation {
    pub fn settle_boundary(
        &mut self,
        request: BoundaryRequest,
    ) -> Result<BoundaryReceipt, CanwuError> {
        self.settle_boundary_with_state_hash_format(request, BoundaryStateHashFormat::CommitmentsV1)
    }

    pub(super) fn settle_boundary_with_state_hash_format(
        &mut self,
        mut request: BoundaryRequest,
        state_hash_format: BoundaryStateHashFormat,
    ) -> Result<BoundaryReceipt, CanwuError> {
        self.ensure_runtime_ready()?;
        if request.at < self.state.scheduler.now {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "a settlement boundary cannot precede committed simulation time",
            ));
        }
        if self
            .state
            .scheduler
            .pending_ingress
            .first()
            .is_some_and(|key| key.due_at < request.at)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "a settlement boundary cannot step past earlier canonical ingress",
            ));
        }
        if request.cadences.contains(&SystemCadence::EventDriven) {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "event-driven cadence is derived from admitted events, not caller supplied",
            ));
        }
        request.cadences.sort();
        request.cadences.dedup();

        let transaction = BoundaryTransactionCheckpoint::capture(&self.state);
        match self.settle_boundary_inner(request, state_hash_format) {
            Ok(receipt) => Ok(receipt),
            Err(error) => {
                transaction.restore(&mut self.state);
                Err(error)
            }
        }
    }

    fn settle_boundary_inner(
        &mut self,
        mut request: BoundaryRequest,
        state_hash_format: BoundaryStateHashFormat,
    ) -> Result<BoundaryReceipt, CanwuError> {
        self.advance_to_before_boundary(request.at)?;

        let admitted_ingress = self.take_due_ingress(request.at);
        let admitted_ingress_index: HashSet<_> = admitted_ingress.iter().copied().collect();
        let mut maintenance_changes = Vec::new();
        for ingress_id in &admitted_ingress {
            let record = self
                .state
                .evidence
                .retained_ingress(*ingress_id)
                .cloned()
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidSnapshot,
                        "pending ingress references an unknown record",
                    )
                })?;
            match record.payload {
                IngressPayload::Command { request: command } => {
                    let CommandRequest {
                        request_id,
                        expected_revision,
                        envelope,
                    } = *command;
                    self.admit_command(
                        Some(request_id),
                        Some(expected_revision),
                        envelope,
                        CommandIngress::LiveRequest,
                        None,
                        true,
                    )?;
                }
                IngressPayload::Calendar { cadences } => request.cadences.extend(cadences),
                IngressPayload::Plugin { .. } => {}
                IngressPayload::Decision { request } => {
                    self.apply_decision_request(*request)?;
                }
                IngressPayload::Maintenance { request } => {
                    maintenance_changes.push(self.apply_maintenance_request(*request)?);
                }
            }
        }
        self.state
            .current
            .decisions
            .advance_time(request.at)
            .map_err(super::decision::decision_error)?;
        self.invalidate_commitments(CommitmentDomains::DECISIONS);
        self.execute_scheduled_at(request.at)?;
        request.cadences.sort();
        request.cadences.dedup();

        let admitted_attempt_count = self
            .state
            .evidence
            .archived
            .command_attempt_count
            .checked_add(
                u64::try_from(self.state.evidence.command_attempts.len()).map_err(|_| {
                    invalid_snapshot_error("attempt journal exceeds admission cursor range")
                })?,
            )
            .ok_or_else(|| invalid_snapshot_error("attempt journal cursor is exhausted"))?;
        let admitted_command_count = self
            .state
            .evidence
            .archived
            .command_count
            .checked_add(
                u64::try_from(self.state.evidence.commands.len()).map_err(|_| {
                    invalid_snapshot_error("command journal exceeds admission cursor range")
                })?,
            )
            .ok_or_else(|| invalid_snapshot_error("command journal cursor is exhausted"))?;
        let admitted_event_count = self
            .state
            .evidence
            .archived
            .event_count
            .checked_add(
                u64::try_from(self.state.evidence.events.len()).map_err(|_| {
                    invalid_snapshot_error("event journal exceeds admission cursor range")
                })?,
            )
            .ok_or_else(|| invalid_snapshot_error("event journal cursor is exhausted"))?;
        let admitted_attempt_start = self
            .state
            .counters
            .admitted_attempt_count
            .checked_sub(self.state.evidence.archived.command_attempt_count)
            .ok_or_else(|| {
                invalid_snapshot_error("runtime attempt admission cursor precedes live evidence")
            })?;
        let admitted_attempt_start = usize::try_from(admitted_attempt_start).map_err(|_| {
            invalid_snapshot_error("runtime attempt admission cursor exceeds platform range")
        })?;
        let admitted_command_start = self
            .state
            .counters
            .admitted_command_count
            .checked_sub(self.state.evidence.archived.command_count)
            .ok_or_else(|| {
                invalid_snapshot_error("runtime command admission cursor precedes live evidence")
            })?;
        let admitted_command_start = usize::try_from(admitted_command_start).map_err(|_| {
            invalid_snapshot_error("runtime command admission cursor exceeds platform range")
        })?;
        let admitted_event_start = self
            .state
            .counters
            .admitted_event_count
            .checked_sub(self.state.evidence.archived.event_count)
            .ok_or_else(|| {
                invalid_snapshot_error("runtime event admission cursor precedes live evidence")
            })?;
        let admitted_event_start = usize::try_from(admitted_event_start).map_err(|_| {
            invalid_snapshot_error("runtime event admission cursor exceeds platform range")
        })?;
        let admitted_attempts: Vec<_> = self
            .state
            .evidence
            .command_attempts
            .get(admitted_attempt_start..)
            .ok_or_else(|| {
                invalid_snapshot_error("runtime attempt admission cursor exceeds its journal")
            })?
            .iter()
            .map(|record| record.id)
            .collect();
        let admitted_commands: Vec<_> = self
            .state
            .evidence
            .commands
            .get(admitted_command_start..)
            .ok_or_else(|| {
                invalid_snapshot_error("runtime command admission cursor exceeds its journal")
            })?
            .iter()
            .map(|record| record.id)
            .collect();
        let admitted_events: Vec<_> = self
            .state
            .evidence
            .events
            .get(admitted_event_start..)
            .ok_or_else(|| {
                invalid_snapshot_error("runtime event admission cursor exceeds its journal")
            })?
            .iter()
            .map(|event| event.id)
            .collect();

        let (boundary_id_value, next_boundary_id) =
            claim_counter(self.state.counters.next_boundary_id, "boundary ID")?;
        let (correlation_id, next_correlation_id) = claim_counter(
            self.state.counters.next_correlation_id,
            "boundary correlation ID",
        )?;
        self.state.counters.next_boundary_id = next_boundary_id;
        self.state.counters.next_correlation_id = next_correlation_id;
        let boundary_id = BoundaryId::new(boundary_id_value);

        let boundary_snapshot = self.state.current.clone();
        let boundary_time = self.state.scheduler.now;
        let systems = self.plugins.boundary_systems.clone();
        let state_owners = self.plugins.state_owners.clone();
        let record_schemas = self.plugins.record_schemas.clone();
        let mut allocations = BTreeMap::new();
        let mut allocation_records = Vec::new();
        let mut reservation_offer_records = Vec::new();
        let mut reservation_request_records = Vec::new();
        let mut offers = Vec::new();
        let mut requests = Vec::new();
        let mut random_overlay = boundary_snapshot.random_streams.clone();
        let mut pending_random_draws = Vec::new();
        let mut keyed_random_draws = random::keyed_draws_with_reservations(
            &self.state.evidence.random_draws,
            &self.state.evidence.keyed_draw_reservations,
        )?;
        let mut visible_overlay = BTreeMap::new();
        let mut candidate_overlay = BTreeMap::new();
        let mut visible_record_overlay = BTreeMap::new();
        let mut candidate_record_overlay = BTreeMap::new();
        let mut visible_knowledge_overlay = BTreeMap::new();
        let mut pending_knowledge_changes = Vec::new();
        let mut knowledge_correlations = BTreeSet::new();
        let mut ordinary = Vec::new();
        let mut transitions = Vec::new();
        let mut deferred = Vec::new();
        let mut evidence = PendingBoundaryEvidence::default();
        for phase in BoundaryPhase::ALL {
            match phase {
                BoundaryPhase::AtomicDomainCommit => {
                    let (same_boundary, next_boundary) =
                        partition_boundary_visibility(std::mem::take(&mut ordinary));
                    self.apply_boundary_stage(
                        boundary_id,
                        correlation_id,
                        same_boundary,
                        &mut evidence,
                    )?;
                    deferred.extend(next_boundary);
                    visible_overlay.clear();
                    candidate_overlay.clear();
                    visible_record_overlay.clear();
                    candidate_record_overlay.clear();
                }
                BoundaryPhase::ConditionalTransitionCommit => {
                    let (same_boundary, next_boundary) =
                        partition_boundary_visibility(std::mem::take(&mut transitions));
                    self.apply_boundary_stage(
                        boundary_id,
                        correlation_id,
                        same_boundary,
                        &mut evidence,
                    )?;
                    deferred.extend(next_boundary);
                    visible_overlay.clear();
                    visible_record_overlay.clear();
                }
                _ => {}
            }

            let mut phase_directives = Vec::new();
            for registered in systems.iter().filter(|registered| {
                registered.contract.phase == phase
                    && boundary_system_due(
                        &registered.contract,
                        &request.cadences,
                        !admitted_events.is_empty() || !admitted_ingress.is_empty(),
                    )
            }) {
                let reader = format!("{}.{}", registered.plugin, registered.contract.name);
                let (view_current, view_now) = if phase <= BoundaryPhase::InvariantValidation {
                    (&boundary_snapshot, boundary_time)
                } else {
                    (&self.state.current, self.state.scheduler.now)
                };
                let random_session = random::RandomSession::new(
                    &random_overlay,
                    &registered.contract.random_streams,
                    boundary_snapshot.root_seed,
                    &registered.plugin,
                    &keyed_random_draws,
                )?;
                let proposal_evidence = proposal_evidence_refs(boundary_id, &evidence);
                let view = SimulationView {
                    state: SimulationViewState::Boundary {
                        current: view_current,
                        now: view_now,
                        runtime: &self.state,
                    },
                    state_owners: &state_owners,
                    reader: Some(&reader),
                    allowed_reads: Some(&registered.contract.reads),
                    allowed_ingress: Some(&admitted_ingress_index),
                    ingress_plugin: Some(&registered.plugin),
                    component_overlay: Some(&visible_overlay),
                    proposed_components: (phase == BoundaryPhase::InvariantValidation)
                        .then_some(&candidate_overlay),
                    record_overlay: Some(&visible_record_overlay),
                    proposed_records: (phase == BoundaryPhase::InvariantValidation)
                        .then_some(&candidate_record_overlay),
                    boundary_id: Some(boundary_id),
                    proposal_evidence: Some(&proposal_evidence),
                    knowledge_overlay: Some(&visible_knowledge_overlay),
                    allocations: Some(&allocations),
                    allowed_reservations: Some(&registered.contract.reservation_reads),
                    random_session: Some(RefCell::new(random_session)),
                };
                let context = BoundaryContext {
                    boundary_id,
                    at: request.at,
                    phase,
                    plugin: registered.plugin.clone(),
                    system: registered.contract.name.clone(),
                    admitted_attempts: admitted_attempts.clone(),
                    admitted_commands: admitted_commands.clone(),
                    admitted_ingress: admitted_ingress.clone(),
                    admitted_events: admitted_events.clone(),
                    emitted_events: evidence
                        .emissions
                        .iter()
                        .map(|emission| emission.event)
                        .collect(),
                };
                let proposal =
                    catch_unwind(AssertUnwindSafe(|| (registered.handler)(&view, &context)))
                        .map_err(|_| {
                            CanwuError::new(
                                ErrorCode::PluginPanicked,
                                format!(
                                    "boundary system {}.{} panicked",
                                    registered.plugin, registered.contract.name
                                ),
                            )
                        })??;
                let random_execution = view
                    .finish_random_session()
                    .expect("boundary views always have a random session");
                validate_boundary_proposal(
                    &registered.plugin,
                    &registered.contract,
                    view_current,
                    view_now,
                    &self.state,
                    boundary_id,
                    &evidence,
                    &self.plugins,
                    &visible_record_overlay,
                    &visible_knowledge_overlay,
                    &proposal,
                    &random_execution.draws,
                )?;
                random::extend_keyed_draws(&mut keyed_random_draws, &random_execution.draws)?;
                random_overlay.extend(random_execution.states);
                pending_random_draws.extend(random_execution.draws.into_iter().map(|draw| {
                    PendingBoundaryRandomDraw {
                        plugin: registered.plugin.clone(),
                        system: registered.contract.name.clone(),
                        draw,
                    }
                }));
                offers.extend(
                    proposal
                        .offers
                        .into_iter()
                        .map(|offer| PendingReservationOffer {
                            plugin: registered.plugin.clone(),
                            system: registered.contract.name.clone(),
                            offer,
                        }),
                );
                requests.extend(proposal.requests.into_iter().map(|request| {
                    PendingReservationRequest {
                        reservation: ReservationRef::new(
                            &registered.plugin,
                            &registered.contract.name,
                            &request.request,
                        ),
                        request,
                    }
                }));
                phase_directives.extend(proposal.directives.into_iter().map(|directive| {
                    StagedBoundaryDirective {
                        plugin: registered.plugin.clone(),
                        system: registered.contract.name.clone(),
                        phase,
                        visibility: registered.contract.visibility,
                        directive,
                    }
                }));
            }

            let (knowledge_directives, phase_directives) =
                partition_knowledge_directives(phase_directives);
            if !knowledge_directives.is_empty()
                && !matches!(
                    phase,
                    BoundaryPhase::PerceptionAndAttentionRefresh
                        | BoundaryPhase::PerspectiveAndReportMaterialization
                )
            {
                return Err(CanwuError::new(
                    ErrorCode::UndeclaredKnowledgeWrite,
                    "knowledge publication is allowed only in phases 4 and 13",
                ));
            }

            match phase {
                BoundaryPhase::PerceptionAndAttentionRefresh => {
                    if !phase_directives.is_empty() {
                        return Err(CanwuError::new(
                            ErrorCode::InvalidBoundary,
                            "phase 4 accepts knowledge publications but no ordinary directives",
                        ));
                    }
                    self.stage_knowledge_publications(
                        phase,
                        knowledge_directives,
                        &mut visible_knowledge_overlay,
                        &mut pending_knowledge_changes,
                        &mut knowledge_correlations,
                    )?;
                }
                BoundaryPhase::ReservationAndAllocation => {
                    let result = allocate_reservations(
                        std::mem::take(&mut offers),
                        std::mem::take(&mut requests),
                    )?;
                    allocations = result.by_reservation;
                    allocation_records = result.records;
                    reservation_offer_records = result.offers;
                    reservation_request_records = result.requests;
                }
                BoundaryPhase::DomainDeltaProposal => {
                    let record_context = BoundaryRecordOverlayContext {
                        current: &boundary_snapshot,
                        now: boundary_time,
                        scheduled_actions: &self.state.scheduler.actions,
                        run_configuration: &self.state.metadata.run_configuration,
                        schemas: &record_schemas,
                    };
                    extend_boundary_record_candidate_overlay(
                        &record_context,
                        &mut candidate_record_overlay,
                        &phase_directives,
                    )?;
                    extend_boundary_candidate_overlay(
                        &boundary_snapshot,
                        &candidate_record_overlay,
                        &mut candidate_overlay,
                        &phase_directives,
                    )?;
                    extend_boundary_record_overlay(
                        &record_context,
                        &mut visible_record_overlay,
                        &phase_directives,
                    )?;
                    extend_boundary_overlay(
                        &boundary_snapshot,
                        &visible_record_overlay,
                        &mut visible_overlay,
                        &phase_directives,
                    )?;
                    ordinary.extend(phase_directives);
                }
                BoundaryPhase::HistoricalCandidateEvaluation => {
                    let record_context = BoundaryRecordOverlayContext {
                        current: &self.state.current,
                        now: self.state.scheduler.now,
                        scheduled_actions: &self.state.scheduler.actions,
                        run_configuration: &self.state.metadata.run_configuration,
                        schemas: &record_schemas,
                    };
                    extend_boundary_record_overlay(
                        &record_context,
                        &mut visible_record_overlay,
                        &phase_directives,
                    )?;
                    extend_boundary_overlay(
                        &self.state.current,
                        &visible_record_overlay,
                        &mut visible_overlay,
                        &phase_directives,
                    )?;
                    transitions.extend(phase_directives);
                }
                BoundaryPhase::StrategicAggregation
                | BoundaryPhase::PerspectiveAndReportMaterialization => {
                    if phase == BoundaryPhase::PerspectiveAndReportMaterialization {
                        self.stage_knowledge_publications(
                            phase,
                            knowledge_directives,
                            &mut visible_knowledge_overlay,
                            &mut pending_knowledge_changes,
                            &mut knowledge_correlations,
                        )?;
                    }
                    let (same_boundary, next_boundary) =
                        partition_boundary_visibility(phase_directives);
                    self.apply_boundary_stage(
                        boundary_id,
                        correlation_id,
                        same_boundary,
                        &mut evidence,
                    )?;
                    deferred.extend(next_boundary);
                }
                _ if !phase_directives.is_empty() => {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidBoundary,
                        format!("boundary phase {phase:?} cannot produce state directives"),
                    ));
                }
                _ => {}
            }
        }

        self.apply_boundary_stage(boundary_id, correlation_id, deferred, &mut evidence)?;
        self.commit_knowledge_publications(
            boundary_id,
            correlation_id,
            &pending_knowledge_changes,
            &mut evidence.emissions,
        )?;
        let PendingBoundaryEvidence {
            changes,
            record_changes,
            emissions,
            mut generated_ingress,
            random_decisions,
        } = evidence;
        self.invalidate_commitments(CommitmentDomains::RANDOM_STREAMS);
        self.state.current.random_streams = random_overlay;
        let mut random_outcomes = BTreeMap::new();
        for pending in &random_decisions {
            let ticket = self
                .state
                .current
                .decisions
                .ticket(pending.resolution.ticket_id)
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidDecision,
                        "random decision ticket disappeared before boundary commit",
                    )
                })?;
            let option_id = DecisionRandomEvidence::selected_option(
                ticket,
                &pending.resolution.option_weights,
                pending.resolution.sample.value,
            )
            .map_err(|error| CanwuError::new(ErrorCode::InvalidDecision, error.to_string()))?;
            let previous = random_outcomes.insert(
                (
                    pending.resolution.sample.stream.clone(),
                    pending.resolution.sample.address.clone(),
                ),
                RandomDrawOutcome::DecisionSelection {
                    ticket_id: ticket.id,
                    ticket_version: ticket.version,
                    option_id,
                },
            );
            if previous.is_some() {
                return Err(CanwuError::new(
                    ErrorCode::InvalidRandomDraw,
                    "one random draw cannot resolve multiple decisions",
                ));
            }
        }
        let committed_random_draws = self.append_boundary_random_draws(
            boundary_id,
            correlation_id,
            pending_random_draws,
            random_outcomes,
        )?;
        self.materialize_boundary_random_decisions(
            boundary_id,
            &random_decisions,
            &committed_random_draws,
            &mut generated_ingress,
        )?;
        let random_draws = committed_random_draws
            .iter()
            .map(|draw| draw.id)
            .collect::<Vec<_>>();
        self.state.metadata.plugin_registration_closed = true;
        let state_hash = self.compute_boundary_state_hash_for(state_hash_format)?;
        let previous_hash = self
            .state
            .evidence
            .boundary_head_hash()
            .map_or_else(|| GENESIS_BOUNDARY_HASH.to_owned(), str::to_owned);
        let previous_maintenance_root = self
            .state
            .evidence
            .boundaries
            .last()
            .and_then(|boundary| boundary.maintenance_terminal_root.as_deref());
        let maintenance_terminal_root =
            if previous_maintenance_root.is_some() || !maintenance_changes.is_empty() {
                Some(canonical_hash(
                    "canwu.maintenance.terminal-root.v1",
                    &(
                        previous_maintenance_root.unwrap_or(GENESIS_BOUNDARY_HASH),
                        &maintenance_changes,
                    ),
                )?)
            } else {
                None
            };
        let mut record = BoundaryRecord {
            id: boundary_id,
            at: request.at,
            correlation_id,
            cadences: request.cadences,
            admitted_attempts,
            admitted_commands,
            admitted_ingress,
            generated_ingress: generated_ingress.clone(),
            admitted_events,
            reservation_offers: reservation_offer_records,
            reservation_requests: reservation_request_records,
            allocations: allocation_records.clone(),
            random_draws: random_draws.clone(),
            changes: changes.clone(),
            record_changes: record_changes.clone(),
            knowledge_changes: pending_knowledge_changes.clone(),
            maintenance_changes,
            maintenance_terminal_root,
            emissions: emissions.clone(),
            state_hash: Some(state_hash),
            previous_hash,
            hash: String::new(),
        };
        record.hash = compute_boundary_hash(&record)?;
        let boundary_hash = record.hash.clone();
        self.state.evidence.boundaries.push(record);
        self.state.counters.admitted_attempt_count = admitted_attempt_count;
        self.state.counters.admitted_command_count = admitted_command_count;
        self.state.counters.admitted_event_count = admitted_event_count;
        self.advance_state_revision()?;
        self.refresh_checkpoint_hash()?;
        Ok(BoundaryReceipt {
            boundary_id,
            settled_at: request.at,
            emitted_events: emissions
                .into_iter()
                .map(|emission| emission.event)
                .collect(),
            generated_ingress: generated_ingress
                .into_iter()
                .map(|generation| generation.ingress)
                .collect(),
            random_draws,
            boundary_hash,
            change_count: changes.len(),
            record_change_count: record_changes.len(),
            knowledge_batch_count: pending_knowledge_changes.len(),
            knowledge_record_count: pending_knowledge_changes
                .iter()
                .map(|change| change.records.len())
                .sum(),
            allocations: allocation_records,
        })
    }

    fn apply_boundary_stage(
        &mut self,
        boundary_id: BoundaryId,
        correlation_id: u64,
        directives: Vec<StagedBoundaryDirective>,
        evidence: &mut PendingBoundaryEvidence,
    ) -> Result<(), CanwuError> {
        let random_decision_count = directives
            .iter()
            .filter(|staged| {
                matches!(
                    &staged.directive,
                    BoundaryDirective::ResolveDecisionRandomly { .. }
                )
            })
            .count();
        if evidence
            .random_decisions
            .len()
            .checked_add(random_decision_count)
            .is_none_or(|count| count > 1)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidDecision,
                "one boundary may generate at most one random decision resolution",
            ));
        }
        let changes = &mut evidence.changes;
        let record_changes = &mut evidence.record_changes;
        let emissions = &mut evidence.emissions;
        let generated_ingress = &mut evidence.generated_ingress;
        let mutation_requests: Vec<_> = directives
            .iter()
            .filter_map(|staged| match &staged.directive {
                BoundaryDirective::MutateRecord { mutation, summary } => {
                    Some(records::DomainMutationRequest {
                        plugin: &staged.plugin,
                        system: &staged.system,
                        visibility: staged.visibility,
                        mutation,
                        summary,
                    })
                }
                BoundaryDirective::SetComponent { .. }
                | BoundaryDirective::Emit { .. }
                | BoundaryDirective::ScheduleIngress { .. }
                | BoundaryDirective::SchedulePluginIngress { .. }
                | BoundaryDirective::ResolveDecisionRandomly { .. }
                | BoundaryDirective::PublishKnowledge { .. } => None,
            })
            .collect();
        let mut stage_record_changes = BTreeMap::new();
        if !mutation_requests.is_empty() {
            let (next_records, applied) = records::apply_mutation_bundle_cow(
                &self.state.current.domain_records,
                &self.plugins.record_schemas,
                self.state.scheduler.now,
                &|entity| runtime_entity_exists(&self.state, entity),
                mutation_requests,
            )?;
            let first_index = record_changes.len();
            for (offset, change) in applied.iter().enumerate() {
                let index = first_index.checked_add(offset).ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::IdentifierExhausted,
                        "boundary record-change index exceeds the persistent identifier space",
                    )
                })?;
                let index = u64::try_from(index).map_err(|_| {
                    CanwuError::new(
                        ErrorCode::IdentifierExhausted,
                        "boundary record-change index exceeds the persistent identifier space",
                    )
                })?;
                stage_record_changes
                    .insert(change.current.reference.clone(), (index, change.clone()));
            }
            self.invalidate_commitments(CommitmentDomains::DOMAIN_RECORDS);
            self.state.current.domain_records = next_records;
            record_changes.extend(applied);
        }

        for staged in &directives {
            let unavailable = match &staged.directive {
                BoundaryDirective::SetComponent { entity, .. } => {
                    (!runtime_entity_exists(&self.state, entity)).then_some(entity)
                }
                BoundaryDirective::Emit { affected, .. } => affected
                    .iter()
                    .find(|entity| !runtime_entity_exists(&self.state, entity)),
                BoundaryDirective::ScheduleIngress { affected, .. } => affected
                    .iter()
                    .find(|entity| !runtime_entity_identity_exists(&self.state, entity)),
                BoundaryDirective::SchedulePluginIngress { affected, .. } => affected
                    .iter()
                    .find(|entity| !runtime_entity_identity_exists(&self.state, entity)),
                BoundaryDirective::MutateRecord { .. }
                | BoundaryDirective::ResolveDecisionRandomly { .. }
                | BoundaryDirective::PublishKnowledge { .. } => None,
            };
            if let Some(entity) = unavailable {
                return Err(CanwuError::new(
                    ErrorCode::EntityNotFound,
                    format!(
                        "boundary stage {}.{} references unavailable entity {entity}",
                        staged.plugin, staged.system
                    ),
                )
                .with_entity(entity.clone()));
            }
        }

        for staged in directives {
            match staged.directive {
                BoundaryDirective::SetComponent {
                    state,
                    entity,
                    component,
                    value,
                    summary,
                } => {
                    let key = component_key(&staged.plugin, &state, &entity, &component);
                    self.invalidate_commitments(CommitmentDomains::PLUGIN_COMPONENTS);
                    let previous = self
                        .state
                        .current
                        .plugin_components
                        .get(&key)
                        .map(|record| record.value.clone());
                    self.state.current.plugin_components.insert(
                        key,
                        PluginComponentRecord {
                            plugin: staged.plugin.clone(),
                            state: state.clone(),
                            entity: entity.clone(),
                            component: component.clone(),
                            value: value.clone(),
                        },
                    );
                    let change_index = u64::try_from(changes.len()).map_err(|_| {
                        CanwuError::new(
                            ErrorCode::IdentifierExhausted,
                            "boundary change index exceeds the persistent identifier space",
                        )
                    })?;
                    changes.push(BoundaryChange {
                        plugin: staged.plugin.clone(),
                        system: staged.system.clone(),
                        state,
                        entity: entity.clone(),
                        component: component.clone(),
                        previous,
                        value,
                        visibility: staged.visibility,
                        summary: summary.clone(),
                    });
                    let event = self.append_event(
                        EventKind::plugin(staged.plugin.clone(), format!("{component}_changed")),
                        vec![entity],
                        summary,
                        Some(CauseRef::Boundary(boundary_id)),
                        correlation_id,
                    )?;
                    emissions.push(BoundaryEmission {
                        plugin: staged.plugin,
                        system: staged.system,
                        event: event.id,
                        kind: BoundaryEmissionKind::Change { change_index },
                    });
                }
                BoundaryDirective::MutateRecord { mutation, .. } => {
                    let Some((change_index, change)) = stage_record_changes.get(mutation.target())
                    else {
                        return Err(CanwuError::new(
                            ErrorCode::InvalidBoundary,
                            "record mutation is missing its committed change evidence",
                        ));
                    };
                    let event = self.append_event(
                        EventKind::plugin(staged.plugin.clone(), change.operation.event_type()),
                        record_change_affected_entities(change),
                        change.summary.clone(),
                        Some(CauseRef::Boundary(boundary_id)),
                        correlation_id,
                    )?;
                    emissions.push(BoundaryEmission {
                        plugin: staged.plugin,
                        system: staged.system,
                        event: event.id,
                        kind: BoundaryEmissionKind::RecordChange {
                            change_index: *change_index,
                        },
                    });
                }
                BoundaryDirective::Emit {
                    event_type,
                    summary,
                    affected,
                } => {
                    let event = self.append_event(
                        EventKind::plugin(staged.plugin.clone(), event_type),
                        affected,
                        summary,
                        Some(CauseRef::Boundary(boundary_id)),
                        correlation_id,
                    )?;
                    emissions.push(BoundaryEmission {
                        plugin: staged.plugin,
                        system: staged.system,
                        event: event.id,
                        kind: BoundaryEmissionKind::Explicit,
                    });
                }
                BoundaryDirective::PublishKnowledge { .. } => {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidBoundary,
                        "knowledge publication execution is not enabled in this runtime slice",
                    ));
                }
                BoundaryDirective::ScheduleIngress {
                    after,
                    packet_type,
                    priority,
                    payload,
                    mut affected,
                } => {
                    self.ensure_canonical_ingress_can_start()?;
                    let descriptor = self
                        .plugins
                        .ingress
                        .get(&(staged.plugin.clone(), packet_type.clone()))
                        .ok_or_else(|| {
                            CanwuError::new(
                                ErrorCode::InvalidPayload,
                                format!(
                                    "boundary system {}.{} scheduled undeclared ingress type {packet_type}",
                                    staged.plugin, staged.system
                                ),
                            )
                        })?
                        .clone();
                    descriptor.payload_schema.validate(&payload)?;
                    affected.sort();
                    affected.dedup();
                    let due_at = self.state.scheduler.now.checked_add(after).ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidDuration,
                            "boundary-generated ingress exceeds the supported time range",
                        )
                    })?;
                    let receipt = self.append_ingress(
                        due_at,
                        descriptor.class,
                        priority,
                        IngressPayload::Plugin {
                            plugin: staged.plugin.clone(),
                            packet_type,
                            payload,
                            affected_entities: affected,
                            archive_retention: Vec::new(),
                        },
                        Some(CauseRef::Boundary(boundary_id)),
                        true,
                    )?;
                    generated_ingress.push(BoundaryIngressGeneration {
                        ingress: receipt.ingress_id,
                        plugin: staged.plugin,
                        system: staged.system,
                        phase: staged.phase,
                        visibility: staged.visibility,
                    });
                }
                BoundaryDirective::SchedulePluginIngress {
                    target_plugin,
                    after,
                    packet_type,
                    priority,
                    payload,
                    mut affected,
                } => {
                    self.ensure_canonical_ingress_can_start()?;
                    let descriptor = self
                        .plugins
                        .ingress
                        .get(&(target_plugin.clone(), packet_type.clone()))
                        .ok_or_else(|| {
                            CanwuError::new(
                                ErrorCode::InvalidPayload,
                                format!(
                                    "boundary system {}.{} scheduled undeclared target ingress {}.{packet_type}",
                                    staged.plugin, staged.system, target_plugin
                                ),
                            )
                        })?
                        .clone();
                    descriptor.payload_schema.validate(&payload)?;
                    affected.sort();
                    affected.dedup();
                    let due_at = self.state.scheduler.now.checked_add(after).ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidDuration,
                            "boundary-generated cross-plugin ingress exceeds the supported time range",
                        )
                    })?;
                    let receipt = self.append_ingress(
                        due_at,
                        descriptor.class,
                        priority,
                        IngressPayload::Plugin {
                            plugin: target_plugin,
                            packet_type,
                            payload,
                            affected_entities: affected,
                            archive_retention: Vec::new(),
                        },
                        Some(CauseRef::Boundary(boundary_id)),
                        true,
                    )?;
                    generated_ingress.push(BoundaryIngressGeneration {
                        ingress: receipt.ingress_id,
                        plugin: staged.plugin,
                        system: staged.system,
                        phase: staged.phase,
                        visibility: staged.visibility,
                    });
                }
                BoundaryDirective::ResolveDecisionRandomly { resolution } => {
                    evidence
                        .random_decisions
                        .push(PendingRandomDecisionResolution {
                            plugin: staged.plugin,
                            system: staged.system,
                            phase: staged.phase,
                            visibility: staged.visibility,
                            resolution,
                        });
                }
            }
        }
        validate_runtime_domain_dependents(&self.state)?;
        Ok(())
    }

    fn materialize_boundary_random_decisions(
        &mut self,
        boundary_id: BoundaryId,
        pending: &[PendingRandomDecisionResolution],
        committed_draws: &[CommittedBoundaryRandomDraw],
        generated_ingress: &mut Vec<BoundaryIngressGeneration>,
    ) -> Result<(), CanwuError> {
        let expected_revision = self.revision().checked_add(1).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::IdentifierExhausted,
                "random decision target revision is exhausted",
            )
        })?;
        let draws = committed_draws
            .iter()
            .map(|draw| ((draw.stream.clone(), draw.address.clone()), draw.id))
            .collect::<BTreeMap<_, _>>();
        for pending in pending {
            let resolution = &pending.resolution;
            let draw_id = draws
                .get(&(
                    resolution.sample.stream.clone(),
                    resolution.sample.address.clone(),
                ))
                .copied()
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidRandomDraw,
                        "random decision draw was not committed by its source boundary",
                    )
                })?;
            let ticket = self
                .state
                .current
                .decisions
                .ticket(resolution.ticket_id)
                .cloned()
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidDecision,
                        "random decision ticket disappeared before ingress generation",
                    )
                })?;
            let controller = self
                .state
                .current
                .decisions
                .controller(&resolution.controller_id)
                .cloned()
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidDecision,
                        "random decision controller disappeared before ingress generation",
                    )
                })?;
            let option_id = DecisionRandomEvidence::selected_option(
                &ticket,
                &resolution.option_weights,
                resolution.sample.value,
            )
            .map_err(|error| CanwuError::new(ErrorCode::InvalidDecision, error.to_string()))?;
            let option = ticket.option(&option_id).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidDecision,
                    "random decision selected an unavailable option",
                )
            })?;
            let due_at = self.state.scheduler.now;
            let command = match &option.action {
                DecisionAction::Command { command } => {
                    let request_id = resolution.command_request_id.ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidDecision,
                            "random decision command option lacks a command request ID",
                        )
                    })?;
                    let command: Command =
                        serde_json::from_value(command.clone()).map_err(|error| {
                            CanwuError::new(
                                ErrorCode::InvalidDecision,
                                format!("decision option contains an invalid command: {error}"),
                            )
                        })?;
                    Some(CommandRequest::new(
                        request_id,
                        expected_revision,
                        CommandEnvelope::new(
                            super::decision::controller_issuer(&controller),
                            command,
                        )
                        .with_authority(super::decision::controller_authority(&controller))
                        .at_time(due_at),
                    ))
                }
                DecisionAction::None => None,
            };
            let decision = PolicyDecision {
                outcome: DecisionOutcome::Selected {
                    option_id: option_id.clone(),
                },
                summary: format!("random policy selected {option_id}"),
                evaluations: Vec::new(),
                external: None,
                random: Some(DecisionRandomEvidence {
                    draw_id,
                    value: resolution.sample.value,
                    upper_exclusive: resolution.sample.upper_exclusive,
                    option_weights: resolution.option_weights.clone(),
                }),
            };
            let mutation = DecisionMutation::Resolve {
                ticket_id: ticket.id,
                expected_version: ticket.version,
                controller_id: controller.id.clone(),
                policy: controller.policy.clone(),
                decision,
                command_request_id: resolution.command_request_id,
            };
            let mut request = DecisionIngressRequest::new(
                resolution.decision_request_id,
                expected_revision,
                mutation,
            );
            if let Some(command) = command {
                request = request.with_command(command);
            }
            let receipt = self.append_boundary_decision_ingress(
                boundary_id,
                due_at,
                resolution.priority,
                request,
            )?;
            generated_ingress.push(BoundaryIngressGeneration {
                ingress: receipt.ingress_id,
                plugin: pending.plugin.clone(),
                system: pending.system.clone(),
                phase: pending.phase,
                visibility: pending.visibility,
            });
        }
        Ok(())
    }

    fn stage_knowledge_publications(
        &mut self,
        phase: BoundaryPhase,
        directives: Vec<StagedBoundaryDirective>,
        visible_overlay: &mut BTreeMap<
            KnowledgeHolderRef,
            BTreeMap<KnowledgeRecordId, KnowledgeRecord>,
        >,
        pending: &mut Vec<BoundaryKnowledgeChange>,
        correlations: &mut BTreeSet<(String, String, String)>,
    ) -> Result<(), CanwuError> {
        let new_record_count = directives
            .iter()
            .map(|staged| match &staged.directive {
                BoundaryDirective::PublishKnowledge { records, .. } => records.len(),
                _ => 0,
            })
            .sum::<usize>();
        let total_records = pending
            .iter()
            .map(|change| change.records.len())
            .sum::<usize>()
            .checked_add(new_record_count)
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::KnowledgeLimitExceeded,
                    "boundary knowledge record count exceeds platform range",
                )
            })?;
        if total_records > crate::KnowledgeLimitsV1::CURRENT.records_per_boundary {
            return Err(CanwuError::new(
                ErrorCode::KnowledgeLimitExceeded,
                "boundary knowledge record limit exceeded",
            ));
        }
        for staged in directives {
            let BoundaryDirective::PublishKnowledge {
                holder,
                visibility,
                producer_correlation,
                records: drafts,
                summary,
            } = staged.directive
            else {
                return Err(CanwuError::new(
                    ErrorCode::InvalidBoundary,
                    "knowledge stage received an ordinary directive",
                ));
            };
            if let Some(value) = &producer_correlation
                && !correlations.insert((
                    staged.plugin.clone(),
                    staged.system.clone(),
                    value.clone(),
                ))
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidKnowledgeRecord,
                    "producer correlation is duplicated within one system and boundary",
                ));
            }
            let mut records = Vec::with_capacity(drafts.len());
            for draft in drafts {
                let (id, next_id) = claim_counter(
                    self.state.counters.next_knowledge_record_id,
                    "knowledge record ID",
                )?;
                self.state.counters.next_knowledge_record_id = next_id;
                let record = KnowledgeRecord {
                    id: KnowledgeRecordId::new(id),
                    holder: holder.clone(),
                    schema: draft.schema,
                    subjects: draft.subjects,
                    payload: draft.payload,
                    as_of: draft.as_of,
                    learned_at: self.state.scheduler.now,
                    confidence_per_mille: draft.confidence_per_mille,
                    origin: draft.origin,
                    supersedes: draft.supersedes,
                    contradicts: draft.contradicts,
                };
                if visibility == StateVisibility::SameBoundary {
                    visible_overlay
                        .entry(holder.clone())
                        .or_default()
                        .insert(record.id, record.clone());
                }
                records.push(record);
            }
            pending.push(BoundaryKnowledgeChange {
                plugin: staged.plugin,
                system: staged.system,
                phase,
                holder,
                producer_correlation,
                records,
                visibility,
                summary,
            });
        }
        Ok(())
    }

    fn commit_knowledge_publications(
        &mut self,
        boundary_id: BoundaryId,
        correlation_id: u64,
        changes: &[BoundaryKnowledgeChange],
        emissions: &mut Vec<BoundaryEmission>,
    ) -> Result<(), CanwuError> {
        if changes.is_empty() {
            return Ok(());
        }
        let mut ledger = self.state.current.knowledge.records.clone();
        for change in changes {
            let holder = ledger.entry(change.holder.clone()).or_default();
            for record in &change.records {
                if holder.insert(record.id, record.clone()).is_some() {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidKnowledgeRecord,
                        "knowledge publication attempted to reuse a global record ID",
                    ));
                }
            }
        }
        self.state.current.knowledge.records = ledger;
        self.invalidate_commitments(CommitmentDomains::KNOWLEDGE);
        for (index, change) in changes.iter().enumerate() {
            let record_count = u32::try_from(change.records.len()).map_err(|_| {
                CanwuError::new(
                    ErrorCode::KnowledgeLimitExceeded,
                    "knowledge publication event count exceeds u32",
                )
            })?;
            let affected = match &change.holder {
                KnowledgeHolderRef::Person(person) => vec![EntityRef::Person(*person)],
                KnowledgeHolderRef::Entity(entity) => vec![entity.clone()],
            };
            let event = self.append_event(
                KnowledgePublished {
                    holder: change.holder.clone(),
                    record_count,
                }
                .into_kind(),
                affected,
                change.summary.clone(),
                Some(CauseRef::Boundary(boundary_id)),
                correlation_id,
            )?;
            emissions.push(BoundaryEmission {
                plugin: change.plugin.clone(),
                system: change.system.clone(),
                event: event.id,
                kind: BoundaryEmissionKind::KnowledgeChange {
                    change_index: u64::try_from(index).map_err(|_| {
                        CanwuError::new(
                            ErrorCode::IdentifierExhausted,
                            "knowledge change index exceeds identifier space",
                        )
                    })?,
                },
            });
        }
        Ok(())
    }

    pub(super) fn apply_directives(
        &mut self,
        plugin: &str,
        directives: Vec<SystemDirective>,
        allowed_writes: &[StateKey],
        cause: &CauseRef,
        correlation_id: u64,
    ) -> Result<(), CanwuError> {
        for directive in directives {
            match directive {
                SystemDirective::SetComponent {
                    state,
                    entity,
                    component,
                    value,
                    summary,
                } => {
                    let key = component_key(plugin, &state, &entity, &component);
                    self.invalidate_commitments(CommitmentDomains::PLUGIN_COMPONENTS);
                    self.state.current.plugin_components.insert(
                        key,
                        PluginComponentRecord {
                            plugin: plugin.to_owned(),
                            state,
                            entity: entity.clone(),
                            component: component.clone(),
                            value,
                        },
                    );
                    self.emit(
                        EventKind::plugin(plugin, format!("{component}_changed")),
                        vec![entity],
                        summary,
                        Some(cause.clone()),
                        correlation_id,
                    )?;
                }
                SystemDirective::Emit {
                    event_type,
                    summary,
                    affected,
                } => {
                    self.emit(
                        EventKind::plugin(plugin, event_type),
                        affected,
                        summary,
                        Some(cause.clone()),
                        correlation_id,
                    )?;
                }
                SystemDirective::Schedule { after, directive } => {
                    let at = self.state.scheduler.now.checked_add(after).ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidDuration,
                            "plugin scheduled time exceeds the supported range",
                        )
                    })?;
                    self.schedule_at(
                        at,
                        ScheduledAction::PluginDirective {
                            plugin: plugin.to_owned(),
                            directive,
                            allowed_writes: allowed_writes.to_vec(),
                            cause: cause.clone(),
                            correlation_id,
                        },
                    )?;
                }
                SystemDirective::EnqueuePluginIngress {
                    after,
                    packet_type,
                    priority,
                    payload,
                    mut affected,
                } => {
                    self.ensure_canonical_ingress_can_start()?;
                    let descriptor = self
                        .plugins
                        .ingress
                        .get(&(plugin.to_owned(), packet_type.clone()))
                        .ok_or_else(|| {
                            CanwuError::new(
                                ErrorCode::InvalidPayload,
                                format!(
                                    "plugin command scheduled unregistered ingress type {plugin}.{packet_type}"
                                ),
                            )
                        })?
                        .clone();
                    descriptor.payload_schema.validate(&payload)?;
                    affected.sort();
                    affected.dedup();
                    if affected
                        .iter()
                        .any(|entity| !runtime_entity_identity_exists(&self.state, entity))
                    {
                        return Err(CanwuError::new(
                            ErrorCode::EntityNotFound,
                            "plugin command ingress references an unknown entity identity",
                        ));
                    }
                    let due_at = self.state.scheduler.now.checked_add(after).ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidDuration,
                            "plugin command ingress exceeds the supported time range",
                        )
                    })?;
                    self.append_ingress(
                        due_at,
                        descriptor.class,
                        priority,
                        IngressPayload::Plugin {
                            plugin: plugin.to_owned(),
                            packet_type,
                            payload,
                            affected_entities: affected,
                            archive_retention: Vec::new(),
                        },
                        Some(cause.clone()),
                        true,
                    )?;
                }
            }
        }
        Ok(())
    }
}

struct PendingReservationOffer {
    plugin: String,
    system: String,
    offer: ReservationOffer,
}

struct PendingReservationRequest {
    reservation: ReservationRef,
    request: ReservationRequest,
}

struct ReservationAllocationResult {
    by_reservation: BTreeMap<ReservationRef, ReservationAllocation>,
    offers: Vec<ReservationOfferRecord>,
    requests: Vec<ReservationRequestRecord>,
    records: Vec<ReservationAllocation>,
}

struct StagedBoundaryDirective {
    plugin: String,
    system: String,
    phase: BoundaryPhase,
    visibility: StateVisibility,
    directive: BoundaryDirective,
}

#[derive(Default)]
struct PendingBoundaryEvidence {
    changes: Vec<BoundaryChange>,
    record_changes: Vec<DomainRecordChange>,
    emissions: Vec<BoundaryEmission>,
    generated_ingress: Vec<BoundaryIngressGeneration>,
    random_decisions: Vec<PendingRandomDecisionResolution>,
}

struct PendingRandomDecisionResolution {
    plugin: String,
    system: String,
    phase: BoundaryPhase,
    visibility: StateVisibility,
    resolution: super::RandomDecisionResolution,
}

fn proposal_evidence_refs(
    boundary: BoundaryId,
    pending: &PendingBoundaryEvidence,
) -> BTreeSet<EvidenceRef> {
    let mut values = BTreeSet::new();
    for (index, change) in pending.record_changes.iter().enumerate() {
        if let Ok(change_index) = u64::try_from(index) {
            values.insert(EvidenceRef::DomainRecordVersion(
                super::DomainRecordVersionRef {
                    record: change.current.reference.clone(),
                    version: change.current.version,
                    established_by: DomainRecordVersionSource::BoundaryChange {
                        boundary,
                        change_index,
                    },
                },
            ));
        }
    }
    values.extend(
        pending
            .emissions
            .iter()
            .map(|emission| EvidenceRef::Event(emission.event)),
    );
    values
}

pub(super) struct PendingBoundaryRandomDraw {
    pub(super) plugin: String,
    pub(super) system: String,
    pub(super) draw: random::PendingRandomDraw,
}

pub(super) struct CommittedBoundaryRandomDraw {
    pub(super) id: super::RandomDrawId,
    pub(super) stream: super::RandomStreamKey,
    pub(super) address: RandomDrawAddress,
}

pub(super) fn boundary_system_due(
    contract: &BoundarySystemContract,
    cadences: &[SystemCadence],
    has_admitted_events: bool,
) -> bool {
    match contract.cadence {
        SystemCadence::EventDriven => has_admitted_events,
        _ => cadences.contains(&contract.cadence),
    }
}

pub(super) fn boundary_has_event_ingress(record: &BoundaryRecord) -> bool {
    !record.admitted_events.is_empty() || !record.admitted_ingress.is_empty()
}

#[allow(clippy::too_many_arguments)]
fn validate_boundary_proposal(
    plugin: &str,
    contract: &BoundarySystemContract,
    current: &RuntimeCurrentState,
    now: SimTime,
    runtime: &RuntimeState,
    boundary_id: BoundaryId,
    pending_evidence: &PendingBoundaryEvidence,
    plugins: &PluginRegistry,
    record_overlay: &BTreeMap<DomainRecordRef, DomainRecord>,
    knowledge_overlay: &BTreeMap<KnowledgeHolderRef, BTreeMap<KnowledgeRecordId, KnowledgeRecord>>,
    proposal: &BoundaryProposal,
    pending_random_draws: &[random::PendingRandomDraw],
) -> Result<(), CanwuError> {
    if contract.phase != BoundaryPhase::ReservationAndAllocation
        && (!proposal.offers.is_empty() || !proposal.requests.is_empty())
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            format!(
                "boundary system {plugin}.{} proposed reservations in phase {:?}",
                contract.name, contract.phase
            ),
        ));
    }

    let entity_exists = |entity: &EntityRef| {
        proposal_entity_exists(
            current,
            &plugins.record_schemas,
            record_overlay,
            proposal,
            entity,
        )
    };
    let mut offered_pools = BTreeSet::new();
    for offer in &proposal.offers {
        validate_reservation_pool(&offer.pool, &entity_exists)?;
        if !contract.reservation_offers.contains(&offer.pool.state)
            || plugins
                .state_owners
                .get(&offer.pool.state)
                .is_none_or(|owner| owner != plugin)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                format!(
                    "boundary system {plugin}.{} offered undeclared state {}.{}",
                    contract.name, offer.pool.state.namespace, offer.pool.state.name
                ),
            ));
        }
        if !offered_pools.insert(&offer.pool) {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                format!(
                    "boundary system {plugin}.{} offered the same reservation pool twice",
                    contract.name
                ),
            ));
        }
    }

    let mut request_names = BTreeSet::new();
    for request in &proposal.requests {
        validate_reservation_pool(&request.pool, &entity_exists)?;
        if request.request.trim().is_empty()
            || request.request != request.request.trim()
            || request.tie_break.trim().is_empty()
            || request.tie_break != request.tie_break.trim()
            || request.quantity == 0
            || !request_names.insert(&request.request)
            || !contract.reservation_requests.contains(&request.pool.state)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                format!(
                    "boundary system {plugin}.{} produced an invalid reservation request",
                    contract.name
                ),
            ));
        }
    }

    let publication_count = proposal
        .directives
        .iter()
        .filter(|directive| matches!(directive, BoundaryDirective::PublishKnowledge { .. }))
        .count();
    if publication_count > crate::KnowledgeLimitsV1::CURRENT.batches_per_system_boundary {
        return Err(CanwuError::new(
            ErrorCode::KnowledgeLimitExceeded,
            "system knowledge publication batch limit exceeded",
        ));
    }
    let mut component_keys = BTreeSet::new();
    let mut record_targets = BTreeSet::new();
    let mut producer_correlations = BTreeSet::new();
    let mut canonical_drafts = BTreeSet::new();
    let mut random_decision_samples = BTreeSet::new();
    for directive in &proposal.directives {
        match directive {
            BoundaryDirective::SetComponent {
                state: state_key,
                entity,
                component,
                ..
            } => {
                if component.trim().is_empty()
                    || component != component.trim()
                    || !contract.writes.contains(state_key)
                    || plugins
                        .state_owners
                        .get(state_key)
                        .is_none_or(|owner| owner != plugin)
                    || is_domain_record_state(&plugins.record_schemas, state_key)
                {
                    return Err(CanwuError::new(
                        ErrorCode::UndeclaredStateWrite,
                        format!(
                            "boundary system {plugin}.{} produced an undeclared component write",
                            contract.name
                        ),
                    ));
                }
                if !entity_exists(entity) {
                    return Err(CanwuError::new(
                        ErrorCode::EntityNotFound,
                        format!(
                            "boundary system {plugin}.{} targeted missing entity {entity}",
                            contract.name
                        ),
                    )
                    .with_entity(entity.clone()));
                }
                let key = component_key(plugin, state_key, entity, component);
                if !component_keys.insert(key) {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidBoundary,
                        format!(
                            "boundary system {plugin}.{} wrote the same component twice",
                            contract.name
                        ),
                    ));
                }
            }
            BoundaryDirective::MutateRecord { mutation, summary } => {
                let target = mutation.target();
                let state_key = records::record_state_key(&target.kind);
                if !canonical_text(summary)
                    || !contract.writes.contains(&state_key)
                    || plugins
                        .state_owners
                        .get(&state_key)
                        .is_none_or(|owner| owner != plugin)
                    || plugins
                        .record_schemas
                        .get(&target.kind)
                        .is_none_or(|(owner, _)| owner != plugin)
                {
                    return Err(CanwuError::new(
                        ErrorCode::UndeclaredStateWrite,
                        format!(
                            "boundary system {plugin}.{} produced an undeclared record mutation",
                            contract.name
                        ),
                    ));
                }
                if !record_targets.insert(target.clone()) {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidBoundary,
                        format!(
                            "boundary system {plugin}.{} mutated the same record twice",
                            contract.name
                        ),
                    ));
                }
            }
            BoundaryDirective::Emit {
                event_type,
                affected,
                ..
            } => {
                if event_type.trim().is_empty()
                    || event_type != event_type.trim()
                    || !contract.emits.contains(event_type)
                {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidBoundary,
                        format!(
                            "boundary system {plugin}.{} emitted an undeclared event type",
                            contract.name
                        ),
                    ));
                }
                if affected.iter().any(|entity| !entity_exists(entity)) {
                    return Err(CanwuError::new(
                        ErrorCode::EntityNotFound,
                        format!(
                            "boundary system {plugin}.{} emitted an event for a missing entity",
                            contract.name
                        ),
                    ));
                }
            }
            BoundaryDirective::PublishKnowledge {
                holder,
                visibility,
                producer_correlation,
                records,
                summary,
            } => {
                if !matches!(
                    contract.phase,
                    BoundaryPhase::PerceptionAndAttentionRefresh
                        | BoundaryPhase::PerspectiveAndReportMaterialization
                ) {
                    return Err(CanwuError::new(
                        ErrorCode::UndeclaredKnowledgeWrite,
                        "knowledge publication is allowed only in phases 4 and 13",
                    ));
                }
                if records.is_empty()
                    || records.len() > crate::KnowledgeLimitsV1::CURRENT.records_per_batch
                {
                    return Err(CanwuError::new(
                        ErrorCode::KnowledgeLimitExceeded,
                        "knowledge publication batch is empty or exceeds its record limit",
                    ));
                }
                if !canonical_text(summary)
                    || summary.len() > crate::KnowledgeLimitsV1::CURRENT.text_bytes
                {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidKnowledgeRecord,
                        "knowledge publication summary is not canonical or exceeds its limit",
                    ));
                }
                if let Some(value) = producer_correlation
                    && (!canonical_text(value)
                        || value.len() > 256
                        || !producer_correlations.insert(value))
                {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidKnowledgeRecord,
                        "producer correlation is invalid or duplicated",
                    ));
                }
                for draft in records {
                    let Some(grant) = contract
                        .knowledge_writes
                        .iter()
                        .find(|grant| grant.schema == draft.schema)
                    else {
                        return Err(CanwuError::new(
                            ErrorCode::UndeclaredKnowledgeWrite,
                            format!(
                                "boundary system {plugin}.{} did not declare the knowledge schema",
                                contract.name
                            ),
                        ));
                    };
                    if !grant.visibilities.contains(visibility) {
                        return Err(CanwuError::new(
                            ErrorCode::UndeclaredKnowledgeWrite,
                            "knowledge publication visibility is not granted",
                        ));
                    }
                    let Some((owner, schema)) = plugins.knowledge_schemas.get(&draft.schema) else {
                        return Err(CanwuError::new(
                            ErrorCode::InvalidKnowledgeSchema,
                            "knowledge publication uses an unregistered schema",
                        ));
                    };
                    if owner != plugin || !schema.writable {
                        return Err(CanwuError::new(
                            ErrorCode::UndeclaredKnowledgeWrite,
                            "knowledge publication uses a foreign or read-only schema",
                        ));
                    }
                    super::knowledge::validate_draft(
                        draft,
                        schema,
                        holder,
                        current,
                        &plugins.record_schemas,
                    )?;
                    for reference in &draft.origin.evidence {
                        validate_proposal_evidence_reference(
                            runtime,
                            boundary_id,
                            pending_evidence,
                            reference,
                        )?;
                    }
                    let existing = current
                        .knowledge
                        .records
                        .get(holder)
                        .into_iter()
                        .flat_map(|records| records.iter())
                        .chain(
                            knowledge_overlay
                                .get(holder)
                                .into_iter()
                                .flat_map(|records| records.iter()),
                        )
                        .collect::<BTreeMap<_, _>>();
                    for related in draft.supersedes.iter().chain(&draft.contradicts) {
                        let Some(related_record) = existing.get(related) else {
                            return Err(CanwuError::new(
                                ErrorCode::KnowledgeRecordNotFound,
                                "knowledge relation does not resolve for the same holder at this cut",
                            ));
                        };
                        if related_record.schema.kind != draft.schema.kind {
                            return Err(CanwuError::new(
                                ErrorCode::InvalidKnowledgeRecord,
                                "knowledge supersession and contradiction cannot cross schema kinds",
                            ));
                        }
                    }
                    let encoded = serde_json::to_vec(&(holder, draft)).map_err(|error| {
                        CanwuError::new(
                            ErrorCode::InvalidKnowledgeRecord,
                            format!("holder-scoped knowledge draft could not be encoded: {error}"),
                        )
                    })?;
                    if !canonical_drafts.insert(encoded) {
                        return Err(CanwuError::new(
                            ErrorCode::InvalidKnowledgeRecord,
                            "one system proposal contains a duplicate canonical knowledge draft",
                        ));
                    }
                }
            }
            BoundaryDirective::ScheduleIngress {
                after,
                packet_type,
                payload,
                affected,
                ..
            } => {
                let descriptor = plugins
                    .ingress
                    .get(&(plugin.to_owned(), packet_type.clone()))
                    .ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidPayload,
                            format!(
                                "boundary system {plugin}.{} scheduled undeclared ingress type {packet_type}",
                                contract.name
                            ),
                        )
                    })?;
                if after.is_negative() || now.checked_add(*after).is_none() {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidDuration,
                        "boundary-generated ingress requires a nonnegative supported delay",
                    ));
                }
                descriptor.payload_schema.validate(payload)?;
                if affected.iter().any(|entity| {
                    !proposal_entity_identity_exists(
                        current,
                        &plugins.record_schemas,
                        proposal,
                        entity,
                    )
                }) {
                    return Err(CanwuError::new(
                        ErrorCode::EntityNotFound,
                        format!(
                            "boundary system {plugin}.{} scheduled ingress for an unknown entity identity",
                            contract.name
                        ),
                    ));
                }
            }
            BoundaryDirective::SchedulePluginIngress {
                target_plugin,
                after,
                packet_type,
                payload,
                affected,
                ..
            } => {
                let grant = super::PluginIngressTarget {
                    target_plugin: target_plugin.clone(),
                    packet_type: packet_type.clone(),
                };
                if !contract.plugin_ingress_targets.contains(&grant) {
                    return Err(CanwuError::new(
                        ErrorCode::UndeclaredStateWrite,
                        format!(
                            "boundary system {plugin}.{} did not declare target ingress {target_plugin}.{packet_type}",
                            contract.name
                        ),
                    ));
                }
                let descriptor = plugins
                    .ingress
                    .get(&(target_plugin.clone(), packet_type.clone()))
                    .ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidPayload,
                            format!(
                                "boundary system {plugin}.{} scheduled undeclared target ingress {target_plugin}.{packet_type}",
                                contract.name
                            ),
                        )
                    })?;
                if after.is_negative() || now.checked_add(*after).is_none() {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidDuration,
                        "boundary-generated cross-plugin ingress requires a nonnegative supported delay",
                    ));
                }
                descriptor.payload_schema.validate(payload)?;
                if affected.iter().any(|entity| {
                    !proposal_entity_identity_exists(
                        current,
                        &plugins.record_schemas,
                        proposal,
                        entity,
                    )
                }) {
                    return Err(CanwuError::new(
                        ErrorCode::EntityNotFound,
                        format!(
                            "boundary system {plugin}.{} scheduled cross-plugin ingress for an unknown entity identity",
                            contract.name
                        ),
                    ));
                }
            }
            BoundaryDirective::ResolveDecisionRandomly { resolution } => {
                validate_random_decision_resolution(
                    plugin,
                    contract,
                    current,
                    pending_random_draws,
                    &mut random_decision_samples,
                    resolution,
                )?;
            }
        }
    }
    Ok(())
}

fn validate_random_decision_resolution(
    plugin: &str,
    contract: &BoundarySystemContract,
    current: &RuntimeCurrentState,
    pending_random_draws: &[random::PendingRandomDraw],
    used_samples: &mut BTreeSet<(super::RandomStreamKey, RandomDrawAddress)>,
    resolution: &super::RandomDecisionResolution,
) -> Result<(), CanwuError> {
    if resolution.decision_request_id.get() == 0
        || resolution
            .command_request_id
            .is_some_and(|request_id| request_id.get() == 0)
        || resolution.expected_version == 0
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidDecision,
            "random decision resolution requires nonzero request IDs and ticket version",
        ));
    }
    let ticket = current
        .decisions
        .ticket(resolution.ticket_id)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDecision,
                "random decision resolution references an unknown ticket",
            )
        })?;
    if !ticket.is_open()
        || ticket.version != resolution.expected_version
        || ticket.assigned_controller != resolution.controller_id
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidDecision,
            "random decision resolution references a closed, stale, or differently controlled ticket",
        ));
    }
    let controller = current
        .decisions
        .controller(&resolution.controller_id)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDecision,
                "random decision resolution references an unknown controller",
            )
        })?;
    if controller.policy.kind != DecisionPolicyKind::Random {
        return Err(CanwuError::new(
            ErrorCode::InvalidDecision,
            "random decision resolution requires a controller with random policy identity",
        ));
    }
    if !contract.random_streams.contains(&resolution.sample.stream) {
        return Err(CanwuError::new(
            ErrorCode::UndeclaredRandomStream,
            format!(
                "boundary system {plugin}.{} did not declare the random decision stream",
                contract.name
            ),
        ));
    }
    let RandomDrawAddress::OperationV1(address) = &resolution.sample.address else {
        return Err(CanwuError::new(
            ErrorCode::InvalidRandomDraw,
            "random decisions require an operation-keyed draw",
        ));
    };
    if address.producer_plugin != plugin
        || address.target
            != (RandomOperationTarget::DecisionTicket {
                ticket_id: ticket.id,
                ticket_version: ticket.version,
            })
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidRandomDraw,
            "random decision draw address does not bind the current ticket version",
        ));
    }
    let sample_key = (
        resolution.sample.stream.clone(),
        resolution.sample.address.clone(),
    );
    if !used_samples.insert(sample_key.clone()) {
        return Err(CanwuError::new(
            ErrorCode::InvalidRandomDraw,
            "one random draw cannot resolve more than one decision",
        ));
    }
    if !pending_random_draws.iter().any(|draw| {
        draw.stream == sample_key.0
            && draw.address == sample_key.1
            && draw.upper_exclusive == resolution.sample.upper_exclusive
            && draw.value == resolution.sample.value
    }) {
        return Err(CanwuError::new(
            ErrorCode::InvalidRandomDraw,
            "random decision resolution does not reference a draw produced by this proposal",
        ));
    }
    let total_weight = resolution
        .option_weights
        .iter()
        .try_fold(0_u64, |total, option| total.checked_add(option.weight))
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDecision,
                "random decision option weights overflow the supported range",
            )
        })?;
    if total_weight != resolution.sample.upper_exclusive {
        return Err(CanwuError::new(
            ErrorCode::InvalidDecision,
            "random decision option weights disagree with the draw bound",
        ));
    }
    let selected = DecisionRandomEvidence::selected_option(
        ticket,
        &resolution.option_weights,
        resolution.sample.value,
    )
    .map_err(|error| CanwuError::new(ErrorCode::InvalidDecision, error.to_string()))?;
    let action = &ticket
        .option(&selected)
        .expect("validated random decision selected an existing option")
        .action;
    if matches!(action, DecisionAction::Command { .. }) != resolution.command_request_id.is_some() {
        return Err(CanwuError::new(
            ErrorCode::InvalidDecision,
            "random decision command options require exactly one command request ID",
        ));
    }
    Ok(())
}

fn validate_proposal_evidence_reference(
    runtime: &RuntimeState,
    boundary_id: BoundaryId,
    pending: &PendingBoundaryEvidence,
    reference: &EvidenceRef,
) -> Result<(), CanwuError> {
    if let EvidenceRef::DomainRecordVersion(version) = reference
        && let DomainRecordVersionSource::BoundaryChange {
            boundary,
            change_index,
        } = version.established_by
        && boundary == boundary_id
    {
        let resolved = usize::try_from(change_index)
            .ok()
            .and_then(|index| pending.record_changes.get(index))
            .is_some_and(|change| {
                change.current.reference == version.record
                    && change.current.version == version.version
            });
        return if resolved {
            Ok(())
        } else {
            Err(CanwuError::new(
                ErrorCode::EvidenceUnavailable,
                "knowledge origin references an unavailable current-boundary record version",
            ))
        };
    }

    if let EvidenceRef::Event(id) = reference
        && runtime
            .evidence
            .retained_event(*id)
            .is_some_and(|event| event.cause == Some(CauseRef::Boundary(boundary_id)))
    {
        if pending
            .emissions
            .iter()
            .any(|emission| emission.event == *id)
        {
            return Ok(());
        }
        return Err(CanwuError::new(
            ErrorCode::EvidenceUnavailable,
            "knowledge origin references an event outside the proposal-visible boundary cut",
        ));
    }

    if let EvidenceRef::Ingress(id) = reference
        && runtime
            .evidence
            .retained_ingress(*id)
            .is_some_and(|record| record.cause == Some(CauseRef::Boundary(boundary_id)))
    {
        return Err(CanwuError::new(
            ErrorCode::EvidenceUnavailable,
            "current-boundary generated ingress is not proposal-visible evidence",
        ));
    }

    match resolve_evidence_reference(&RuntimeValidationContext::new(runtime), reference) {
        EvidenceAvailability::Retained | EvidenceAvailability::Archived => Ok(()),
        EvidenceAvailability::Missing => Err(CanwuError::new(
            ErrorCode::EvidenceUnavailable,
            "knowledge origin references missing or wrong-version evidence",
        )),
    }
}

fn validate_reservation_pool(
    pool: &ReservationPoolKey,
    entity_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(), CanwuError> {
    if pool.resource.trim().is_empty()
        || pool.resource != pool.resource.trim()
        || !entity_exists(&pool.entity)
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "reservation pools require a canonical resource and an existing entity",
        ));
    }
    Ok(())
}

fn extend_boundary_overlay(
    current: &RuntimeCurrentState,
    record_overlay: &BTreeMap<DomainRecordRef, DomainRecord>,
    overlay: &mut BTreeMap<PluginComponentKey, PluginComponentRecord>,
    directives: &[StagedBoundaryDirective],
) -> Result<(), CanwuError> {
    extend_boundary_component_overlay(current, record_overlay, overlay, directives, false)
}

fn extend_boundary_candidate_overlay(
    current: &RuntimeCurrentState,
    record_overlay: &BTreeMap<DomainRecordRef, DomainRecord>,
    overlay: &mut BTreeMap<PluginComponentKey, PluginComponentRecord>,
    directives: &[StagedBoundaryDirective],
) -> Result<(), CanwuError> {
    extend_boundary_component_overlay(current, record_overlay, overlay, directives, true)
}

fn extend_boundary_component_overlay(
    current: &RuntimeCurrentState,
    record_overlay: &BTreeMap<DomainRecordRef, DomainRecord>,
    overlay: &mut BTreeMap<PluginComponentKey, PluginComponentRecord>,
    directives: &[StagedBoundaryDirective],
    include_next_boundary: bool,
) -> Result<(), CanwuError> {
    for staged in directives.iter().filter(|staged| {
        include_next_boundary || staged.visibility == StateVisibility::SameBoundary
    }) {
        if let BoundaryDirective::SetComponent {
            state: state_key,
            entity,
            component,
            value,
            ..
        } = &staged.directive
        {
            let key = component_key(&staged.plugin, state_key, entity, component);
            if overlay.contains_key(&key) {
                return Err(CanwuError::new(
                    ErrorCode::InvalidBoundary,
                    "multiple boundary proposals target the same component",
                ));
            }
            if !runtime_entity_exists_with_record_overlay(current, record_overlay, entity) {
                return Err(CanwuError::new(
                    ErrorCode::EntityNotFound,
                    format!("boundary proposal targeted missing entity {entity}"),
                ));
            }
            overlay.insert(
                key,
                PluginComponentRecord {
                    plugin: staged.plugin.clone(),
                    state: state_key.clone(),
                    entity: entity.clone(),
                    component: component.clone(),
                    value: value.clone(),
                },
            );
        }
    }
    Ok(())
}

fn extend_boundary_record_overlay(
    context: &BoundaryRecordOverlayContext<'_>,
    overlay: &mut BTreeMap<DomainRecordRef, DomainRecord>,
    directives: &[StagedBoundaryDirective],
) -> Result<(), CanwuError> {
    extend_boundary_domain_record_overlay(context, overlay, directives, false)
}

fn extend_boundary_record_candidate_overlay(
    context: &BoundaryRecordOverlayContext<'_>,
    overlay: &mut BTreeMap<DomainRecordRef, DomainRecord>,
    directives: &[StagedBoundaryDirective],
) -> Result<(), CanwuError> {
    extend_boundary_domain_record_overlay(context, overlay, directives, true)
}

struct BoundaryRecordOverlayContext<'a> {
    current: &'a RuntimeCurrentState,
    now: SimTime,
    scheduled_actions: &'a BTreeMap<ScheduleKey, ScheduledAction>,
    run_configuration: &'a RunConfigurationSnapshot,
    schemas: &'a records::DomainRecordSchemas,
}

fn extend_boundary_domain_record_overlay(
    context: &BoundaryRecordOverlayContext<'_>,
    overlay: &mut BTreeMap<DomainRecordRef, DomainRecord>,
    directives: &[StagedBoundaryDirective],
    include_next_boundary: bool,
) -> Result<(), CanwuError> {
    let requests: Vec<_> = directives
        .iter()
        .filter(|staged| {
            include_next_boundary || staged.visibility == StateVisibility::SameBoundary
        })
        .filter_map(|staged| match &staged.directive {
            BoundaryDirective::MutateRecord { mutation, summary } => {
                Some(records::DomainMutationRequest {
                    plugin: &staged.plugin,
                    system: &staged.system,
                    visibility: staged.visibility,
                    mutation,
                    summary,
                })
            }
            BoundaryDirective::SetComponent { .. }
            | BoundaryDirective::Emit { .. }
            | BoundaryDirective::ScheduleIngress { .. }
            | BoundaryDirective::SchedulePluginIngress { .. }
            | BoundaryDirective::ResolveDecisionRandomly { .. }
            | BoundaryDirective::PublishKnowledge { .. } => None,
        })
        .collect();
    if requests.is_empty() {
        return Ok(());
    }
    let (next, changes) = records::apply_mutation_bundle_cow_with_overlay(
        &context.current.domain_records,
        overlay,
        context.schemas,
        context.now,
        &|entity| runtime_current_entity_exists(context.current, entity),
        requests,
    )?;
    validate_domain_dependents_with_records(
        &context.current.plugin_components,
        context.scheduled_actions,
        context.run_configuration,
        &next,
    )?;
    for change in changes {
        overlay.insert(change.current.reference.clone(), change.current);
    }
    Ok(())
}

fn partition_boundary_visibility(
    directives: Vec<StagedBoundaryDirective>,
) -> (Vec<StagedBoundaryDirective>, Vec<StagedBoundaryDirective>) {
    directives
        .into_iter()
        .partition(|staged| staged.visibility == StateVisibility::SameBoundary)
}

fn partition_knowledge_directives(
    directives: Vec<StagedBoundaryDirective>,
) -> (Vec<StagedBoundaryDirective>, Vec<StagedBoundaryDirective>) {
    directives
        .into_iter()
        .partition(|staged| matches!(staged.directive, BoundaryDirective::PublishKnowledge { .. }))
}

fn allocate_reservations(
    mut offers: Vec<PendingReservationOffer>,
    mut requests: Vec<PendingReservationRequest>,
) -> Result<ReservationAllocationResult, CanwuError> {
    offers.sort_by(|left, right| {
        left.offer
            .pool
            .cmp(&right.offer.pool)
            .then_with(|| left.plugin.cmp(&right.plugin))
            .then_with(|| left.system.cmp(&right.system))
    });
    let mut remaining = BTreeMap::new();
    let mut offer_records = Vec::new();
    for pending in offers {
        if remaining
            .insert(pending.offer.pool.clone(), pending.offer.capacity)
            .is_some()
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                format!(
                    "reservation pool was offered more than once, including by {}.{}",
                    pending.plugin, pending.system
                ),
            ));
        }
        offer_records.push(ReservationOfferRecord {
            plugin: pending.plugin,
            system: pending.system,
            offer: pending.offer,
        });
    }
    requests.sort_by(|left, right| {
        left.request
            .pool
            .cmp(&right.request.pool)
            .then_with(|| right.request.priority.cmp(&left.request.priority))
            .then_with(|| left.request.tie_break.cmp(&right.request.tie_break))
            .then_with(|| left.reservation.cmp(&right.reservation))
    });
    let mut seen = BTreeSet::new();
    let mut by_reservation = BTreeMap::new();
    let mut request_records = Vec::new();
    let mut records = Vec::new();
    for pending in requests {
        if !seen.insert(pending.reservation.clone()) {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "reservation request identity is duplicated",
            ));
        }
        request_records.push(ReservationRequestRecord {
            reservation: pending.reservation.clone(),
            request: pending.request.clone(),
        });
        let available = remaining.entry(pending.request.pool.clone()).or_default();
        let granted = pending.request.quantity.min(*available);
        *available -= granted;
        let disposition = if granted == pending.request.quantity {
            ReservationDisposition::Fulfilled
        } else if granted == 0 {
            ReservationDisposition::Rejected
        } else {
            ReservationDisposition::Partial
        };
        let allocation = ReservationAllocation {
            reservation: pending.reservation.clone(),
            pool: pending.request.pool,
            requested: pending.request.quantity,
            granted,
            remaining_after: *available,
            disposition,
        };
        by_reservation.insert(pending.reservation, allocation.clone());
        records.push(allocation);
    }
    Ok(ReservationAllocationResult {
        by_reservation,
        offers: offer_records,
        requests: request_records,
        records,
    })
}
