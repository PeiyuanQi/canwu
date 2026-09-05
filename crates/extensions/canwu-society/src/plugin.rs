use crate::model::{
    CohortTransferIntent, CohortTransferOutcome, DispositionBucket, InstitutionalAlignment,
    PolicyDecision, SocietyCohortExchangeLedger, SocietyCohortExchangeLedgerRecord, SocietyState,
    SocietyStateRecord, core_reference_schemas, invalid, society_cohort_exchange_ledger_reference,
    society_state_reference,
};
use crate::settle_transitions;
use crate::solver::{compute_aggregates, compute_mobilization_candidates, compute_projections};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    CanwuError, DecisionOrigin, DomainRecord, DomainRecordDraft, DomainRecordMutation,
    DomainRecordSchema, DomainRecordType, ErrorCode, IngressClass, IngressPayload, Issuer,
    PayloadProperty, PayloadSchema, PayloadValueType, PluginActionDescriptor,
    PluginIngressDescriptor, PluginRegistrar, SimulationPlugin, SimulationView, StateKey,
    StateVisibility, SystemCadence, SystemDirective,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const PLUGIN_NAME: &str = "canwu-society";

#[derive(Clone, Copy, Debug, Default)]
pub struct SocietyPlugin;

impl SimulationPlugin for SocietyPlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn semantic_hash(&self) -> &'static str {
        "a4e005ac53d979c74d6fa1d01302df1116fc5322c6461a60edfb1d83c6dddfd1"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_record::<SocietyStateRecord>();
        schema.payload_schema = society_payload_schema();
        schema.references = core_reference_schemas();
        registrar.register_record_schema(schema)?;

        let mut ledger_schema =
            DomainRecordSchema::for_record::<SocietyCohortExchangeLedgerRecord>();
        ledger_schema.payload_schema = exchange_ledger_payload_schema();
        registrar.register_record_schema(ledger_schema)?;

        registrar.register_command(
            PluginActionDescriptor {
                name: "transfer_cohort_population".to_owned(),
                description: "Execute an authority-checked owner-side cohort population transfer"
                    .to_owned(),
                payload_schema: cohort_transfer_payload_schema(),
                reads: vec![society_state_key(), cohort_exchange_ledger_key()],
                writes: Vec::new(),
            },
            transfer_cohort_population,
        )?;

        registrar.register_ingress(PluginIngressDescriptor {
            name: COHORT_TRANSFER_INGRESS.to_owned(),
            description: "Apply an admitted cohort transfer".to_owned(),
            class: IngressClass::Decision,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_command(
            PluginActionDescriptor {
                name: "set_institutional_policy".to_owned(),
                description: "Record a versioned institutional alignment decision".to_owned(),
                payload_schema: policy_payload_schema(),
                reads: vec![society_state_key(), policy_decision_key()],
                writes: vec![policy_decision_key()],
            },
            set_institutional_policy,
        )?;

        let mut transition = BoundarySystemContract::new(
            "settle-social-transitions",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        transition.reads = vec![
            society_state_key(),
            policy_decision_key(),
            cohort_exchange_ledger_key(),
            StateKey::core_ingress(),
        ];
        transition.writes = vec![society_state_key(), cohort_exchange_ledger_key()];
        transition.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(transition, settle_social_transitions)?;

        let mut mobilization = BoundarySystemContract::new(
            "evaluate-mobilization-candidates",
            BoundaryPhase::HistoricalCandidateEvaluation,
            SystemCadence::Daily,
        );
        mobilization.reads = vec![society_state_key()];
        mobilization.writes = vec![society_state_key()];
        mobilization.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(mobilization, evaluate_mobilization)?;

        let mut aggregate = BoundarySystemContract::new(
            "aggregate-social-state",
            BoundaryPhase::StrategicAggregation,
            SystemCadence::Daily,
        );
        aggregate.reads = vec![society_state_key()];
        aggregate.writes = vec![society_state_key()];
        aggregate.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(aggregate, aggregate_social_state)?;

        let mut project = BoundarySystemContract::new(
            "materialize-society-projections",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        project.reads = vec![society_state_key()];
        project.writes = vec![society_state_key()];
        project.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(project, materialize_projections)
    }
}

fn transfer_cohort_population(
    view: &SimulationView<'_>,
    context: &canwu_api::CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let intent: CohortTransferIntent =
        serde_json::from_value(payload.clone()).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!("cohort transfer intent could not be decoded: {error}"),
            )
        })?;
    validate_transfer_intent(&intent)?;
    let Some((record, state)) = load_state(view)? else {
        return Err(CanwuError::new(
            ErrorCode::DomainRecordNotFound,
            "the society state record is not configured",
        ));
    };
    let alignment = state
        .institutional_alignments
        .get(&intent.authority_alignment_id)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                "unknown transfer authority alignment",
            )
        })?;
    let actor = match (&context.issuer, &context.authority.decision_origin) {
        (Issuer::Actor(issuer), DecisionOrigin::Actor { actor })
            if issuer == actor && Some(*actor) == alignment.authorized_actor =>
        {
            *actor
        }
        _ => {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "cohort transfer requires the authorized actor ingress",
            ));
        }
    };
    if context.authority.command_subject.as_ref() != Some(&alignment.institution) {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "transfer command subject does not own the authority alignment",
        ));
    }
    if intent.due_time < view.time() {
        return Err(CanwuError::new(
            ErrorCode::InvalidDecision,
            "cohort transfer due time is stale",
        ));
    }
    let (ledger, _) = load_ledger(view)?;
    if let Some(existing) = ledger.outcomes.get(&intent.operation_id) {
        if existing.source_record_version == intent.expected_source_version
            && existing.source_cohort_id == intent.source_cohort_id
            && existing.destination_cohort_id == intent.destination_cohort_id
            && existing.quantity == intent.quantity
        {
            return Ok(Vec::new());
        }
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "operation id is already bound to a different transfer",
        ));
    }
    if record.version != intent.expected_source_version {
        return Err(CanwuError::new(
            ErrorCode::DomainRecordVersionConflict,
            format!(
                "society record version {} does not match expected {}",
                record.version, intent.expected_source_version
            ),
        ));
    }
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: canwu_api::SimDuration::days(1),
        packet_type: COHORT_TRANSFER_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::json!({"intent": intent, "actor": actor.get(), "source_record_version": record.version}),
        affected: vec![alignment.institution.clone()],
    }])
}

#[allow(dead_code, clippy::too_many_lines)]
fn apply_cohort_transfer_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let (mut ledger, ledger_record) = load_ledger(view)?;
    let mut changed = false;
    let mut directives = Vec::new();
    for ingress_id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*ingress_id)? else {
            continue;
        };
        let canwu_api::IngressPayload::Plugin {
            plugin,
            packet_type,
            payload,
            ..
        } = &ingress.payload
        else {
            continue;
        };
        if plugin != PLUGIN_NAME || packet_type != COHORT_TRANSFER_INGRESS {
            continue;
        }
        let admitted: Value = payload.clone();
        let intent: CohortTransferIntent = serde_json::from_value(admitted["intent"].clone())
            .map_err(|e| invalid(format!("admitted cohort transfer malformed: {e}")))?;
        let actor = canwu_api::PersonId::new(
            admitted["actor"]
                .as_u64()
                .ok_or_else(|| invalid("admitted transfer actor missing"))?,
        );
        let source_version = admitted["source_record_version"]
            .as_u64()
            .ok_or_else(|| invalid("admitted source version missing"))?;
        if let Some(existing) = ledger.outcomes.get(&intent.operation_id) {
            if existing.source_record_version == source_version
                && existing.quantity == intent.quantity
                && existing.source_cohort_id == intent.source_cohort_id
                && existing.destination_cohort_id == intent.destination_cohort_id
            {
                continue;
            }
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                "operation id was reused with different transfer data",
            ));
        }
        if record.version < source_version {
            return Err(CanwuError::new(
                ErrorCode::DomainRecordVersionConflict,
                "admitted cohort transfer source version is stale",
            ));
        }
        apply_cohort_transfer(&mut state, &intent)?;
        ledger.outcomes.insert(
            intent.operation_id.clone(),
            CohortTransferOutcome {
                operation_id: intent.operation_id.clone(),
                source_record_version: source_version,
                source_cohort_id: intent.source_cohort_id.clone(),
                destination_cohort_id: intent.destination_cohort_id.clone(),
                quantity: intent.quantity,
                actor,
                authority_alignment_id: intent.authority_alignment_id.clone(),
                due_time: intent.due_time,
                completed_at: context.at,
                result: "completed".to_owned(),
            },
        );
        changed = true;
    }
    if !changed {
        return Ok(BoundaryProposal::default());
    }
    state.canonicalize()?;
    state.validate()?;
    ledger.validate()?;
    directives.push(BoundaryDirective::MutateRecord {
        mutation: DomainRecordMutation::Update {
            record: state.record_draft()?,
            expected_version: record.version,
        },
        summary: "Applied society cohort transfer".to_owned(),
    });
    let mutation = match ledger_record {
        Some(r) => DomainRecordMutation::Update {
            record: DomainRecordDraft::from_typed(
                society_cohort_exchange_ledger_reference(),
                &ledger,
            )?,
            expected_version: r.version,
        },
        None => DomainRecordMutation::Create {
            record: DomainRecordDraft::from_typed(
                society_cohort_exchange_ledger_reference(),
                &ledger,
            )?,
        },
    };
    directives.push(BoundaryDirective::MutateRecord {
        mutation,
        summary: "Recorded society cohort transfer outcome".to_owned(),
    });
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

const COHORT_TRANSFER_INGRESS: &str = "cohort-transfer";

fn validate_transfer_intent(intent: &CohortTransferIntent) -> Result<(), CanwuError> {
    if intent.operation_id.is_empty()
        || intent.authority_alignment_id.is_empty()
        || intent.source_cohort_id.is_empty()
        || intent.destination_cohort_id.is_empty()
        || intent.quantity == 0
        || intent.expected_source_version == 0
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            "cohort transfer intent has empty or zero required fields",
        ));
    }
    if intent.source_cohort_id == intent.destination_cohort_id {
        return Err(CanwuError::new(
            ErrorCode::InvalidDecision,
            "source and destination cohorts must differ",
        ));
    }
    Ok(())
}

fn apply_cohort_transfer(
    state: &mut SocietyState,
    intent: &CohortTransferIntent,
) -> Result<(), CanwuError> {
    let source_count = state
        .cohorts
        .get(&intent.source_cohort_id)
        .ok_or_else(|| invalid("unknown source cohort"))?
        .headcount;
    let destination_exists = state.cohorts.contains_key(&intent.destination_cohort_id);
    if !destination_exists || intent.quantity >= source_count {
        return Err(invalid(
            "destination cohort is missing or transfer would leave an invalid source cohort",
        ));
    }
    let source_targets: BTreeSet<_> = state
        .distributions
        .values()
        .filter(|d| d.cohort_id == intent.source_cohort_id)
        .map(|d| d.target_id.clone())
        .collect();
    let destination_targets: BTreeSet<_> = state
        .distributions
        .values()
        .filter(|d| d.cohort_id == intent.destination_cohort_id)
        .map(|d| d.target_id.clone())
        .collect();
    if source_targets != destination_targets || source_targets.is_empty() {
        return Err(invalid(
            "source and destination target sets must be equal and non-empty",
        ));
    }
    for target_id in source_targets {
        let source_id = crate::distribution_id(&intent.source_cohort_id, &target_id);
        let destination_id = crate::distribution_id(&intent.destination_cohort_id, &target_id);
        let source = state
            .distributions
            .get(&source_id)
            .cloned()
            .ok_or_else(|| invalid("missing source distribution"))?;
        let allocations = proportional_allocations(&source.buckets, intent.quantity, source_count)?;
        let destination = state
            .distributions
            .get_mut(&destination_id)
            .ok_or_else(|| invalid("missing destination distribution"))?;
        for (index, moved) in allocations.into_iter().enumerate() {
            destination.buckets.push(DispositionBucket {
                profile: source.buckets[index].profile,
                headcount: moved,
            });
        }
        let source_mut = state
            .distributions
            .get_mut(&source_id)
            .expect("source distribution exists");
        for (bucket, moved) in source_mut.buckets.iter_mut().zip(proportional_allocations(
            &source.buckets,
            intent.quantity,
            source_count,
        )?) {
            bucket.headcount -= moved;
        }
    }
    state
        .cohorts
        .get_mut(&intent.source_cohort_id)
        .expect("source exists")
        .headcount -= intent.quantity;
    state
        .cohorts
        .get_mut(&intent.destination_cohort_id)
        .expect("destination exists")
        .headcount += intent.quantity;
    Ok(())
}

fn proportional_allocations(
    buckets: &[DispositionBucket],
    quantity: u64,
    total: u64,
) -> Result<Vec<u64>, CanwuError> {
    let mut allocations = Vec::with_capacity(buckets.len());
    let mut remainders = Vec::new();
    let mut assigned = 0_u64;
    for (index, bucket) in buckets.iter().enumerate() {
        let product = bucket
            .headcount
            .checked_mul(quantity)
            .ok_or_else(|| invalid("cohort transfer allocation overflow"))?;
        let base = product / total;
        assigned += base;
        allocations.push(base);
        remainders.push((product % total, index));
    }
    remainders.sort_by(|a, b| b.cmp(a));
    for (_, index) in remainders.into_iter().take(
        usize::try_from(quantity - assigned)
            .map_err(|_| invalid("cohort transfer remainder overflowed usize"))?,
    ) {
        allocations[index] += 1;
    }
    Ok(allocations)
}

fn load_ledger(
    view: &SimulationView<'_>,
) -> Result<(SocietyCohortExchangeLedger, Option<DomainRecord>), CanwuError> {
    let Some(record) = view.typed_domain_record(&society_cohort_exchange_ledger_reference())?
    else {
        return Ok((
            SocietyCohortExchangeLedger {
                schema_version: SocietyCohortExchangeLedger::SCHEMA_VERSION,
                outcomes: BTreeMap::new(),
            },
            None,
        ));
    };
    let ledger = record.decode_payload::<SocietyCohortExchangeLedgerRecord>()?;
    ledger.validate()?;
    Ok((ledger, Some(record.clone())))
}

fn set_institutional_policy(
    view: &SimulationView<'_>,
    context: &canwu_api::CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let decision: PolicyDecision = serde_json::from_value(payload.clone()).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("institutional policy payload could not be decoded: {error}"),
        )
    })?;
    validate_policy_decision(&decision)?;
    let Some((_, state)) = load_state(view)? else {
        return Err(CanwuError::new(
            ErrorCode::DomainRecordNotFound,
            "the society state record is not configured",
        ));
    };
    let alignment = state
        .institutional_alignments
        .get(&decision.alignment_id)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDecision,
                format!("unknown institutional alignment {}", decision.alignment_id),
            )
        })?;
    validate_policy_authority(context, alignment)?;
    if decision.decision_version <= alignment.last_decision_version {
        return Err(CanwuError::new(
            ErrorCode::InvalidDecision,
            format!(
                "institutional decision version {} is not newer than applied version {}",
                decision.decision_version, alignment.last_decision_version
            ),
        ));
    }
    if let Some(pending) = view.component(
        &policy_decision_key(),
        &alignment.institution,
        &alignment.id,
    )? {
        let pending: PolicyDecision = serde_json::from_value(pending.clone()).map_err(|error| {
            invalid(format!(
                "stored institutional decision is malformed: {error}"
            ))
        })?;
        if decision.decision_version <= pending.decision_version {
            return Err(CanwuError::new(
                ErrorCode::InvalidDecision,
                format!(
                    "institutional decision version {} is not newer than pending version {}",
                    decision.decision_version, pending.decision_version
                ),
            ));
        }
    }

    Ok(vec![SystemDirective::SetComponent {
        state: policy_decision_key(),
        entity: alignment.institution.clone(),
        component: alignment.id.clone(),
        value: serde_json::to_value(&decision).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!("institutional decision could not be encoded: {error}"),
            )
        })?,
        summary: format!(
            "Institutional alignment {} received a new policy",
            alignment.id
        ),
    }])
}

fn validate_policy_authority(
    context: &canwu_api::CommandContext,
    alignment: &InstitutionalAlignment,
) -> Result<(), CanwuError> {
    let Some(controller_id) = context.decision_controller_id.as_deref() else {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "institutional policy changes require a validated DecisionTicket controller",
        ));
    };
    if !matches!(
        &context.issuer,
        Issuer::Ai(issuer) | Issuer::Human(issuer) if issuer == controller_id
    ) {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "institutional policy issuer does not match its validated DecisionTicket controller",
        ));
    }
    if context.authority.command_subject.as_ref() != Some(&alignment.institution) {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            format!(
                "command subject {:?} cannot change institutional alignment {} owned by {:?}",
                context.authority.command_subject, alignment.id, alignment.institution
            ),
        ));
    }
    match (
        &context.authority.decision_origin,
        alignment.authorized_actor,
    ) {
        (DecisionOrigin::Actor { actor }, Some(authorized)) if *actor == authorized => {}
        (
            DecisionOrigin::Institution {
                institution,
                responsible_actor: Some(actor),
            },
            Some(authorized),
        ) if institution == &alignment.institution && *actor == authorized => {}
        (
            DecisionOrigin::Institution {
                institution,
                responsible_actor: _,
            },
            None,
        ) if institution == &alignment.institution => {}
        _ => {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                format!(
                    "decision origin {:?} cannot change institutional alignment {}",
                    context.authority.decision_origin, alignment.id
                ),
            ));
        }
    }
    Ok(())
}

fn settle_social_transitions(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let before = state.clone();
    let (mut ledger, ledger_record) = load_ledger(view)?;
    let mut transfer_changed = false;
    for ingress_id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*ingress_id)? else {
            continue;
        };
        let IngressPayload::Plugin {
            plugin,
            packet_type,
            payload,
            ..
        } = &ingress.payload
        else {
            continue;
        };
        if plugin != PLUGIN_NAME || packet_type != COHORT_TRANSFER_INGRESS {
            continue;
        }
        let intent: CohortTransferIntent = serde_json::from_value(payload["intent"].clone())
            .map_err(|error| invalid(format!("admitted cohort transfer malformed: {error}")))?;
        let source_version = payload["source_record_version"]
            .as_u64()
            .ok_or_else(|| invalid("admitted source version missing"))?;
        if let Some(existing) = ledger.outcomes.get(&intent.operation_id) {
            if existing.source_record_version == source_version
                && existing.quantity == intent.quantity
            {
                continue;
            }
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                "operation id was reused with different transfer data",
            ));
        }
        if record.version < source_version {
            return Err(CanwuError::new(
                ErrorCode::DomainRecordVersionConflict,
                "admitted cohort transfer source version is stale",
            ));
        }
        apply_cohort_transfer(&mut state, &intent)?;
        ledger.outcomes.insert(
            intent.operation_id.clone(),
            CohortTransferOutcome {
                operation_id: intent.operation_id.clone(),
                source_record_version: source_version,
                source_cohort_id: intent.source_cohort_id.clone(),
                destination_cohort_id: intent.destination_cohort_id.clone(),
                quantity: intent.quantity,
                actor: canwu_api::PersonId::new(
                    payload["actor"]
                        .as_u64()
                        .ok_or_else(|| invalid("admitted transfer actor missing"))?,
                ),
                authority_alignment_id: intent.authority_alignment_id.clone(),
                due_time: intent.due_time,
                completed_at: context.at,
                result: "completed".to_owned(),
            },
        );
        transfer_changed = true;
    }
    apply_pending_policies(view, &mut state)?;
    settle_transitions(&mut state, context.at)?;
    if state == before && !transfer_changed {
        return Ok(BoundaryProposal::default());
    }
    let mut proposal = update_state(&record, state, "Settled aggregate social transitions")?;
    if transfer_changed {
        ledger.validate()?;
        let mutation = match ledger_record {
            Some(record) => DomainRecordMutation::Update {
                record: DomainRecordDraft::from_typed(
                    society_cohort_exchange_ledger_reference(),
                    &ledger,
                )?,
                expected_version: record.version,
            },
            None => DomainRecordMutation::Create {
                record: DomainRecordDraft::from_typed(
                    society_cohort_exchange_ledger_reference(),
                    &ledger,
                )?,
            },
        };
        proposal.directives.push(BoundaryDirective::MutateRecord {
            mutation,
            summary: "Recorded society cohort transfer outcome".to_owned(),
        });
    }
    Ok(proposal)
}

fn evaluate_mobilization(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let candidates = compute_mobilization_candidates(&state, context.at);
    if state.mobilization_candidates == candidates && state.last_mobilization_at == Some(context.at)
    {
        return Ok(BoundaryProposal::default());
    }
    state.mobilization_candidates = candidates;
    state.last_mobilization_at = Some(context.at);
    update_state(&record, state, "Evaluated social mobilization candidates")
}

fn aggregate_social_state(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let aggregates = compute_aggregates(&state);
    if state.aggregates == aggregates && state.last_aggregation_at == Some(context.at) {
        return Ok(BoundaryProposal::default());
    }
    state.aggregates = aggregates;
    state.last_aggregation_at = Some(context.at);
    update_state(&record, state, "Aggregated social state")
}

fn materialize_projections(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let projections = compute_projections(&state, context.at);
    if state.projections == projections && state.last_projection_at == Some(context.at) {
        return Ok(BoundaryProposal::default());
    }
    state.projections = projections;
    state.last_projection_at = Some(context.at);
    update_state(
        &record,
        state,
        "Materialized actor-relative society projections",
    )
}

fn apply_pending_policies(
    view: &SimulationView<'_>,
    state: &mut SocietyState,
) -> Result<(), CanwuError> {
    for alignment in state.institutional_alignments.values_mut() {
        let Some(value) = view.component(
            &policy_decision_key(),
            &alignment.institution,
            &alignment.id,
        )?
        else {
            continue;
        };
        let decision: PolicyDecision = serde_json::from_value(value.clone()).map_err(|error| {
            invalid(format!(
                "stored institutional decision is malformed: {error}"
            ))
        })?;
        validate_policy_decision(&decision)?;
        if decision.alignment_id != alignment.id {
            return Err(invalid(format!(
                "institutional decision {} was stored on alignment {}",
                decision.alignment_id, alignment.id
            )));
        }
        if decision.decision_version <= alignment.last_decision_version {
            continue;
        }
        alignment.support_per_mille = decision.support_per_mille;
        alignment.enforcement_per_mille = decision.enforcement_per_mille;
        alignment.access_grant_per_mille = decision.access_grant_per_mille;
        alignment.last_decision_version = decision.decision_version;
    }
    Ok(())
}

fn update_state(
    record: &DomainRecord,
    mut state: SocietyState,
    summary: &str,
) -> Result<BoundaryProposal, CanwuError> {
    state.canonicalize()?;
    state.validate()?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: state.record_draft()?,
                expected_version: record.version,
            },
            summary: summary.to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn load_state(
    view: &SimulationView<'_>,
) -> Result<Option<(DomainRecord, SocietyState)>, CanwuError> {
    let Some(record) = view.typed_domain_record(&society_state_reference())? else {
        return Ok(None);
    };
    let mut state = record.decode_payload::<SocietyStateRecord>()?;
    state.canonicalize()?;
    state.validate()?;
    state.validate_at(view.time())?;
    state.validate_record_binding(record)?;
    Ok(Some((record.clone(), state)))
}

pub(crate) fn validate_policy_decision(decision: &PolicyDecision) -> Result<(), CanwuError> {
    if decision.decision_version == 0
        || decision.support_per_mille > 1_000
        || decision.enforcement_per_mille > 1_000
        || decision.access_grant_per_mille > 1_000
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidDecision,
            "institutional policy fields must be versioned and within permille bounds",
        ));
    }
    Ok(())
}

fn cohort_exchange_ledger_key() -> StateKey {
    StateKey::new(
        SocietyCohortExchangeLedgerRecord::NAMESPACE,
        SocietyCohortExchangeLedgerRecord::NAME,
    )
}

fn society_state_key() -> StateKey {
    StateKey::new(SocietyStateRecord::NAMESPACE, SocietyStateRecord::NAME)
}

pub(crate) fn policy_decision_key() -> StateKey {
    StateKey::new("canwu.society", "policy-decisions")
}

fn cohort_transfer_payload_schema() -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([
            (
                "operation_id".to_owned(),
                required(PayloadValueType::String),
            ),
            (
                "authority_alignment_id".to_owned(),
                required(PayloadValueType::String),
            ),
            (
                "source_cohort_id".to_owned(),
                required(PayloadValueType::String),
            ),
            (
                "destination_cohort_id".to_owned(),
                required(PayloadValueType::String),
            ),
            ("quantity".to_owned(), required(PayloadValueType::Integer)),
            (
                "expected_source_version".to_owned(),
                required(PayloadValueType::Integer),
            ),
            ("due_time".to_owned(), required(PayloadValueType::Integer)),
        ]),
        allow_additional: false,
    }
}

fn exchange_ledger_payload_schema() -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([
            (
                "schema_version".to_owned(),
                required(PayloadValueType::Integer),
            ),
            ("outcomes".to_owned(), required(PayloadValueType::Object)),
        ]),
        allow_additional: false,
    }
}

fn policy_payload_schema() -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([
            (
                "access_grant_per_mille".to_owned(),
                required(PayloadValueType::Integer),
            ),
            (
                "alignment_id".to_owned(),
                required(PayloadValueType::String),
            ),
            (
                "decision_version".to_owned(),
                required(PayloadValueType::Integer),
            ),
            (
                "enforcement_per_mille".to_owned(),
                required(PayloadValueType::Integer),
            ),
            (
                "support_per_mille".to_owned(),
                required(PayloadValueType::Integer),
            ),
        ]),
        allow_additional: false,
    }
}

fn society_payload_schema() -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([
            ("aggregates".to_owned(), required(PayloadValueType::Object)),
            ("cohorts".to_owned(), required(PayloadValueType::Object)),
            (
                "distributions".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "influence_edges".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "institutional_alignments".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "last_aggregation_at".to_owned(),
                optional(PayloadValueType::Integer),
            ),
            (
                "last_mobilization_at".to_owned(),
                optional(PayloadValueType::Integer),
            ),
            (
                "last_projection_at".to_owned(),
                optional(PayloadValueType::Integer),
            ),
            (
                "last_transition_at".to_owned(),
                optional(PayloadValueType::Integer),
            ),
            (
                "mobilization_candidates".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "observer_profiles".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "organization_relations".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "organizations".to_owned(),
                required(PayloadValueType::Object),
            ),
            ("policies".to_owned(), required(PayloadValueType::Object)),
            ("projections".to_owned(), required(PayloadValueType::Object)),
            ("remainders".to_owned(), required(PayloadValueType::Object)),
            (
                "schema_version".to_owned(),
                required(PayloadValueType::Integer),
            ),
            ("targets".to_owned(), required(PayloadValueType::Object)),
            (
                "topology_passes".to_owned(),
                required(PayloadValueType::Integer),
            ),
            (
                "transition_rules".to_owned(),
                required(PayloadValueType::Object),
            ),
        ]),
        allow_additional: false,
    }
}

const fn required(value_type: PayloadValueType) -> PayloadProperty {
    PayloadProperty {
        value_type,
        required: true,
    }
}

const fn optional(value_type: PayloadValueType) -> PayloadProperty {
    PayloadProperty {
        value_type,
        required: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canwu_api::{
        Canwu, CommandAuthority, CommandContext, CommandId, CommandIngress, CommandPolicyContext,
        EntityRef, Issuer, SimTime,
    };

    #[test]
    fn decision_policy_authority_binds_controller_origin_and_subject() {
        let ids = Canwu::demo_ids();
        let alignment = InstitutionalAlignment {
            id: "alignment".to_owned(),
            institution: EntityRef::Government(ids.government),
            target_id: "target".to_owned(),
            affected_cohorts: std::collections::BTreeSet::default(),
            support_per_mille: 0,
            enforcement_per_mille: 0,
            access_grant_per_mille: 0,
            authorized_actor: Some(ids.commander),
            last_decision_version: 0,
        };
        let context = |subject| CommandContext {
            issuer: Issuer::Ai("controller".to_owned()),
            authority: CommandAuthority {
                decision_origin: DecisionOrigin::Actor {
                    actor: ids.commander,
                },
                seat_id: None,
                permission_profile_id: None,
                command_subject: subject,
            },
            decision_controller_id: Some("controller".to_owned()),
            run_policy: CommandPolicyContext::LegacyUnspecified,
            ingress: CommandIngress::LiveRequest,
            attempt_id: None,
            command_id: CommandId::new(1),
            request_id: None,
            revision: 0,
            simulation_time: SimTime::EPOCH,
            expected_revision: None,
            expected_time: None,
        };

        assert!(validate_policy_authority(&context(None), &alignment).is_err());
        assert!(
            validate_policy_authority(&context(Some(EntityRef::Army(ids.army))), &alignment)
                .is_err()
        );
        assert!(
            validate_policy_authority(
                &context(Some(EntityRef::Government(ids.government))),
                &alignment,
            )
            .is_ok()
        );
    }
}
