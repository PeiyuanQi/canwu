use super::{
    AssertUnwindSafe, BTreeMap, BTreeSet, BoundaryChange, BoundaryContext, BoundaryDirective,
    BoundaryEmission, BoundaryEmissionKind, BoundaryId, BoundaryIngressGeneration, BoundaryPhase,
    BoundaryProposal, BoundaryReceipt, BoundaryRecord, BoundaryRequest, BoundaryStateHashFormat,
    BoundarySystemContract, BoundaryTransactionCheckpoint, CanwuError, CauseRef, CommandIngress,
    CommandRequest, CommitmentDomains, DomainRecord, DomainRecordChange, DomainRecordRef,
    EntityRef, ErrorCode, EventKind, GENESIS_BOUNDARY_HASH, HashSet, IngressPayload,
    PluginComponentKey, PluginComponentRecord, PluginRegistry, RefCell, ReservationAllocation,
    ReservationDisposition, ReservationOffer, ReservationOfferRecord, ReservationPoolKey,
    ReservationRef, ReservationRequest, ReservationRequestRecord, RunConfigurationSnapshot,
    RuntimeCurrentState, ScheduleKey, ScheduledAction, SimTime, Simulation, SimulationView,
    SimulationViewState, StateKey, StateVisibility, SystemCadence, SystemDirective, canonical_text,
    catch_unwind, claim_counter, component_key, compute_boundary_hash, invalid_snapshot_error,
    is_domain_record_state, proposal_entity_exists, proposal_entity_identity_exists, random,
    record_change_affected_entities, records, runtime_current_entity_exists, runtime_entity_exists,
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
                        true,
                    )?;
                }
                IngressPayload::Calendar { cadences } => request.cadences.extend(cadences),
                IngressPayload::Plugin { .. } => {}
            }
        }
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
        let mut visible_overlay = BTreeMap::new();
        let mut candidate_overlay = BTreeMap::new();
        let mut visible_record_overlay = BTreeMap::new();
        let mut candidate_record_overlay = BTreeMap::new();
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
                )?;
                let view = SimulationView {
                    state: SimulationViewState::Boundary {
                        current: view_current,
                        now: view_now,
                        evidence: &self.state.evidence,
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
                validate_boundary_proposal(
                    &registered.plugin,
                    &registered.contract,
                    view_current,
                    view_now,
                    &self.plugins,
                    &visible_record_overlay,
                    &proposal,
                )?;
                let random_execution = view
                    .finish_random_session()
                    .expect("boundary views always have a random session");
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

            match phase {
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
        let PendingBoundaryEvidence {
            changes,
            record_changes,
            emissions,
            generated_ingress,
        } = evidence;
        self.invalidate_commitments(CommitmentDomains::RANDOM_STREAMS);
        self.state.current.random_streams = random_overlay;
        let random_draws =
            self.append_boundary_random_draws(boundary_id, correlation_id, pending_random_draws)?;
        self.state.metadata.plugin_registration_closed = true;
        let state_hash = self.compute_boundary_state_hash_for(state_hash_format)?;
        let previous_hash = self
            .state
            .evidence
            .boundary_head_hash()
            .map_or_else(|| GENESIS_BOUNDARY_HASH.to_owned(), str::to_owned);
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
                | BoundaryDirective::ScheduleIngress { .. } => None,
            })
            .collect();
        let mut stage_record_changes = BTreeMap::new();
        if !mutation_requests.is_empty() {
            let (next_records, applied) = records::apply_mutation_bundle(
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
                BoundaryDirective::MutateRecord { .. } => None,
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
                        EventKind::Plugin {
                            plugin: staged.plugin.clone(),
                            event_type: format!("{component}_changed"),
                        },
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
                        EventKind::Plugin {
                            plugin: staged.plugin.clone(),
                            event_type: change.operation.event_type().to_owned(),
                        },
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
                        EventKind::Plugin {
                            plugin: staged.plugin.clone(),
                            event_type,
                        },
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
            }
        }
        validate_runtime_domain_dependents(&self.state)?;
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
                        EventKind::Plugin {
                            plugin: plugin.to_owned(),
                            event_type: format!("{component}_changed"),
                        },
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
                        EventKind::Plugin {
                            plugin: plugin.to_owned(),
                            event_type,
                        },
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
}

pub(super) struct PendingBoundaryRandomDraw {
    pub(super) plugin: String,
    pub(super) system: String,
    pub(super) draw: random::PendingRandomDraw,
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

fn validate_boundary_proposal(
    plugin: &str,
    contract: &BoundarySystemContract,
    current: &RuntimeCurrentState,
    now: SimTime,
    plugins: &PluginRegistry,
    record_overlay: &BTreeMap<DomainRecordRef, DomainRecord>,
    proposal: &BoundaryProposal,
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

    let mut component_keys = BTreeSet::new();
    let mut record_targets = BTreeSet::new();
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
        }
    }
    Ok(())
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
    let mut base = context.current.domain_records.clone();
    base.extend(
        overlay
            .iter()
            .map(|(reference, record)| (reference.clone(), record.clone())),
    );
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
            | BoundaryDirective::ScheduleIngress { .. } => None,
        })
        .collect();
    if requests.is_empty() {
        return Ok(());
    }
    let (next, changes) = records::apply_mutation_bundle(
        &base,
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
