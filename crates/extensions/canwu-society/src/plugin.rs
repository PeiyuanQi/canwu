use crate::ingress::{
    COHORT_REBASE_INGRESS, CohortHeadcountRebaseV1, MAX_SOCIETY_INGRESS_QUEUE,
    QueuedSocietyIngress, SOCIETY_INGRESS_MALFORMED_REJECTION, SocietyIngressQueue,
    SocietyIngressQueueRecord, is_queued_packet, rebase_admission_rejection,
    society_ingress_queue_reference,
};
use crate::lifecycle::{SOCIETY_LIFECYCLE_DELTA_INGRESS, SocietyLifecycleDeltaV1};
use crate::model::{
    CohortHeadcountRebaseOutcome, CohortTransferIntent, CohortTransferOutcome, DispositionBucket,
    InstitutionalAlignment, PendingCohortTransfer, PolicyDecision, SocietyCohortExchangeLedger,
    SocietyCohortExchangeLedgerRecord, SocietyCohortTransferPending,
    SocietyCohortTransferPendingRecord, SocietyIngressStatus, SocietyLifecycleDeltaOutcome,
    SocietyState, SocietyStateRecord, core_reference_schemas, invalid,
    society_cohort_exchange_ledger_reference, society_cohort_transfer_pending_reference,
    society_state_reference,
};
use crate::settle_transitions;
use crate::solver::{compute_aggregates, compute_mobilization_candidates, compute_projections};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    CanwuError, Command, CommandId, DecisionOrigin, DomainRecord, DomainRecordDraft,
    DomainRecordMutation, DomainRecordSchema, DomainRecordType, ErrorCode, IngressClass,
    IngressPayload, Issuer, PayloadProperty, PayloadSchema, PayloadValueType,
    PluginActionDescriptor, PluginIngressDescriptor, PluginRegistrar, SimTime, SimulationPlugin,
    SimulationView, StateKey, StateVisibility, SystemCadence, SystemDirective,
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
        "23cf74dbfc68789336bbab5c8fbb04227d765673b887f1d7dc8bc6734d6bba38"
    }

    fn validate_activation(&self, records: &[DomainRecord]) -> Result<(), CanwuError> {
        for record in records {
            if record.reference == society_ingress_queue_reference().into_untyped() {
                record
                    .decode_payload::<SocietyIngressQueueRecord>()?
                    .validate()?;
            } else if record.reference == society_cohort_exchange_ledger_reference().into_untyped()
            {
                record
                    .decode_payload::<SocietyCohortExchangeLedgerRecord>()?
                    .validate()?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_record::<SocietyStateRecord>();
        schema.payload_schema = society_payload_schema();
        schema.references = core_reference_schemas();
        registrar.register_record_schema(schema)?;

        let mut ledger_schema =
            DomainRecordSchema::for_record::<SocietyCohortExchangeLedgerRecord>();
        ledger_schema.payload_schema = exchange_ledger_payload_schema();
        registrar.register_record_schema(ledger_schema)?;
        registrar.register_record_schema(DomainRecordSchema::for_record::<
            SocietyCohortTransferPendingRecord,
        >())?;
        let mut queue_schema = DomainRecordSchema::for_record::<SocietyIngressQueueRecord>();
        queue_schema.payload_schema = PayloadSchema::Object {
            properties: BTreeMap::from([
                (
                    "schema_version".to_owned(),
                    required(PayloadValueType::Integer),
                ),
                ("entries".to_owned(), required(PayloadValueType::Array)),
            ]),
            allow_additional: false,
        };
        registrar.register_record_schema(queue_schema)?;

        registrar.register_ingress(PluginIngressDescriptor {
            name: COHORT_REBASE_INGRESS.to_owned(),
            description: "Rebase one cohort headcount to an exact external stock version"
                .to_owned(),
            class: IngressClass::Information,
            payload_schema: rebase_payload_schema(),
        })?;
        // Only boundary systems that declare this target may schedule a
        // lifecycle delta; the host cannot author one.
        let _lifecycle_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
            name: SOCIETY_LIFECYCLE_DELTA_INGRESS.to_owned(),
            description: "Apply one provider-computed target lifecycle delta".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Object {
                properties: BTreeMap::from([
                    ("installs".to_owned(), optional(PayloadValueType::Array)),
                    (
                        "deactivations".to_owned(),
                        optional(PayloadValueType::Array),
                    ),
                    ("releases".to_owned(), optional(PayloadValueType::Array)),
                ]),
                allow_additional: false,
            },
        })?;

        let mut ingress_intake = BoundarySystemContract::new(
            "intake-society-ingress",
            BoundaryPhase::StrategicAggregation,
            SystemCadence::EventDriven,
        );
        ingress_intake.reads = vec![
            ingress_queue_key(),
            StateKey::core_domain_records(),
            StateKey::core_ingress(),
        ];
        ingress_intake.writes = vec![ingress_queue_key()];
        ingress_intake.emits = vec![SOCIETY_INGRESS_REJECTED_EVENT.to_owned()];
        ingress_intake.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(ingress_intake, intake_society_ingress)?;

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

        let mut transfer_intake = BoundarySystemContract::new(
            "intake-cohort-transfers",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        transfer_intake.reads = vec![
            society_state_key(),
            pending_transfer_key(),
            StateKey::core_ingress(),
            StateKey::core_commands(),
        ];
        transfer_intake.writes = vec![pending_transfer_key()];
        transfer_intake.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(transfer_intake, intake_cohort_transfer)?;

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
            pending_transfer_key(),
            ingress_queue_key(),
            StateKey::core_ingress(),
        ];
        transition.writes = vec![
            society_state_key(),
            cohort_exchange_ledger_key(),
            ingress_queue_key(),
        ];
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
        payload: serde_json::json!({"intent": intent, "actor": actor.get(), "source_record_version": record.version, "command_id": context.command_id.get()}),
        affected: vec![alignment.institution.clone()],
    }])
}

const COHORT_TRANSFER_INGRESS: &str = "cohort-transfer";
/// Event the society intake emits for an admitted packet it cannot queue.
pub const SOCIETY_INGRESS_REJECTED_EVENT: &str = "society_ingress_rejected_v1";
const TRANSFER_COMPLETED: &str = "completed";
const TRANSFER_STALE_SOURCE: &str = "stale_source";
const TRANSFER_REJECTED: &str = "rejected";

fn pending_transfer_key() -> StateKey {
    StateKey::new(
        SocietyCohortTransferPendingRecord::NAMESPACE,
        SocietyCohortTransferPendingRecord::NAME,
    )
}

#[allow(clippy::too_many_lines)]
fn intake_cohort_transfer(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((_, state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let pending_ref = society_cohort_transfer_pending_reference();
    let (mut pending, pending_record) = load_pending(view)?;
    let mut changed = false;
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
            .map_err(|e| invalid(format!("admitted cohort transfer malformed: {e}")))?;
        validate_transfer_intent(&intent)?;
        let actor = canwu_api::PersonId::new(
            payload["actor"]
                .as_u64()
                .ok_or_else(|| invalid("admitted transfer actor missing"))?,
        );
        let command_id = CommandId::new(
            payload["command_id"]
                .as_u64()
                .ok_or_else(|| invalid("admitted transfer command evidence missing"))?,
        );
        let command = view.command(command_id)?.ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                "cohort transfer command evidence is unavailable",
            )
        })?;
        let Command::Plugin {
            plugin: command_plugin,
            command: command_name,
            payload: command_payload,
        } = &command.envelope.command
        else {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "cohort transfer evidence is not a society command",
            ));
        };
        if command_plugin != PLUGIN_NAME
            || command_name != "transfer_cohort_population"
            || command_payload["operation_id"] != serde_json::json!(intent.operation_id)
            || command
                .envelope
                .authority
                .as_ref()
                .and_then(|a| a.command_subject.as_ref())
                != state
                    .institutional_alignments
                    .get(&intent.authority_alignment_id)
                    .map(|a| &a.institution)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "cohort transfer ingress is not backed by the original authorized command",
            ));
        }
        let source_digest = cohort_transfer_digest(&state, &intent)?;
        if let Some(existing) = pending.transfers.get(&intent.operation_id) {
            if existing.intent == intent
                && existing.actor == actor
                && existing.command_id == command_id.get()
                && existing.source_digest == source_digest
            {
                continue;
            }
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                "operation id was reused with different transfer data",
            ));
        }
        pending.transfers.insert(
            intent.operation_id.clone(),
            PendingCohortTransfer {
                intent,
                actor,
                command_id: command_id.get(),
                source_digest,
            },
        );
        changed = true;
    }
    if !changed {
        return Ok(BoundaryProposal::default());
    }
    let mutation = match pending_record {
        Some(record) => DomainRecordMutation::Update {
            record: DomainRecordDraft::from_typed(pending_ref, &pending)?,
            expected_version: record.version,
        },
        None => DomainRecordMutation::Create {
            record: DomainRecordDraft::from_typed(pending_ref, &pending)?,
        },
    };
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation,
            summary: "Persist admitted society cohort transfer".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn load_pending(
    view: &SimulationView<'_>,
) -> Result<(SocietyCohortTransferPending, Option<DomainRecord>), CanwuError> {
    let Some(record) = view.typed_domain_record(&society_cohort_transfer_pending_reference())?
    else {
        return Ok((
            SocietyCohortTransferPending {
                schema_version: 1,
                transfers: BTreeMap::new(),
            },
            None,
        ));
    };
    let pending = record.decode_payload::<SocietyCohortTransferPendingRecord>()?;
    if pending.schema_version != 1 {
        return Err(invalid("unsupported pending cohort transfer schema"));
    }
    Ok((pending, Some(record.clone())))
}

fn cohort_transfer_digest(
    state: &SocietyState,
    intent: &CohortTransferIntent,
) -> Result<String, CanwuError> {
    let source = state
        .cohorts
        .get(&intent.source_cohort_id)
        .ok_or_else(|| invalid("unknown source cohort"))?;
    let destination = state
        .cohorts
        .get(&intent.destination_cohort_id)
        .ok_or_else(|| invalid("unknown destination cohort"))?;
    // Bind only the cohorts the transfer moves between. Their dispositions
    // may keep changing through ordinary transitions; the transfer moves the
    // proportional share current when it settles.
    canwu_api::canonical_hash(
        "canwu.society.cohort-transfer-source.v2",
        &(source, destination),
    )
}

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
    state.invalidate_derived_state();
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
        return Ok((SocietyCohortExchangeLedger::empty(), None));
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

#[allow(clippy::too_many_lines)]
fn settle_social_transitions(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let before = state.clone();
    let (mut ledger, ledger_record) = load_ledger(view)?;
    let (mut pending, pending_record) = load_pending(view)?;
    let mut transfer_changed = false;
    let operation_ids: Vec<_> = pending.transfers.keys().cloned().collect();
    for operation_id in operation_ids {
        let transfer = pending
            .transfers
            .get(&operation_id)
            .cloned()
            .ok_or_else(|| invalid("pending cohort transfer disappeared"))?;
        if transfer.intent.due_time > context.at {
            continue;
        }
        // An applied transfer stays in the intake-owned pending record; its
        // ledger outcome, not its (now changed) source digest, settles it.
        if ledger.outcomes.contains_key(&operation_id) {
            pending.transfers.remove(&operation_id);
            continue;
        }
        // A source that changed after intake (for example through a rebase
        // or lifecycle delta) or a transfer that no longer fits settles as a
        // terminal rejection instead of failing every later Daily boundary.
        let result = if cohort_transfer_digest(&state, &transfer.intent)? == transfer.source_digest
        {
            let mut draft = state.clone();
            if apply_cohort_transfer(&mut draft, &transfer.intent).is_ok() {
                state = draft;
                TRANSFER_COMPLETED
            } else {
                TRANSFER_REJECTED
            }
        } else {
            TRANSFER_STALE_SOURCE
        };
        ledger.outcomes.insert(
            operation_id.clone(),
            CohortTransferOutcome {
                operation_id: operation_id.clone(),
                source_record_version: transfer.intent.expected_source_version,
                source_cohort_id: transfer.intent.source_cohort_id.clone(),
                destination_cohort_id: transfer.intent.destination_cohort_id.clone(),
                quantity: transfer.intent.quantity,
                actor: transfer.actor,
                authority_alignment_id: transfer.intent.authority_alignment_id.clone(),
                due_time: transfer.intent.due_time,
                completed_at: context.at,
                result: result.to_owned(),
            },
        );
        pending.transfers.remove(&operation_id);
        transfer_changed = true;
    }
    // Queued rebases and lifecycle deltas apply in admission order, after
    // transfers and before institutional decisions and transitions.
    let (queue, queue_record) = load_ingress_queue(view)?;
    let queue_consumed = !queue.entries.is_empty();
    for entry in &queue.entries {
        settle_queued_ingress(view, &mut state, &mut ledger, entry, context.at)?;
    }
    apply_pending_policies(view, &mut state)?;
    settle_transitions(&mut state, context.at)?;
    if state == before
        && !transfer_changed
        && !queue_consumed
        && pending_record.is_some()
        && pending.transfers == load_pending(view)?.0.transfers
    {
        return Ok(BoundaryProposal::default());
    }
    let mut proposal = if state == before {
        BoundaryProposal::default()
    } else {
        update_state(&record, state, "Settled aggregate social transitions")?
    };
    if let (true, Some(queue_record)) = (queue_consumed, queue_record) {
        proposal.directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: DomainRecordDraft::from_typed(
                    society_ingress_queue_reference(),
                    &SocietyIngressQueue {
                        schema_version: SocietyIngressQueue::SCHEMA_VERSION,
                        entries: Vec::new(),
                    },
                )?,
                expected_version: queue_record.version,
            },
            summary: "Consumed queued society ingress".to_owned(),
        });
    }
    if transfer_changed || queue_consumed {
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
            summary: "Recorded society cohort exchange outcomes".to_owned(),
        });
    }

    Ok(proposal)
}

/// Applies one queued packet to the in-progress state and records its
/// terminal outcome. A rejected packet leaves the state unchanged.
fn settle_queued_ingress(
    view: &SimulationView<'_>,
    state: &mut SocietyState,
    ledger: &mut SocietyCohortExchangeLedger,
    entry: &QueuedSocietyIngress,
    at: SimTime,
) -> Result<(), CanwuError> {
    let key = entry.ingress.get().to_string();
    match entry.packet_type.as_str() {
        COHORT_REBASE_INGRESS => {
            if ledger.rebases.contains_key(&key) {
                return Ok(());
            }
            let rebase =
                serde_json::from_value::<CohortHeadcountRebaseV1>(entry.payload.clone()).ok();
            let result = match (&entry.admission_rejection, &rebase) {
                (Some(reason), _) => Err(reason.clone()),
                (None, None) => Err(SOCIETY_INGRESS_MALFORMED_REJECTION.to_owned()),
                (None, Some(rebase)) => state
                    .rebase_cohort(&rebase.cohort_id, rebase.new_headcount)
                    .map_err(|error| error.message),
            };
            let (status, rejection, previous_headcount) = match result {
                Ok(previous) => (SocietyIngressStatus::Applied, None, Some(previous)),
                Err(reason) => (SocietyIngressStatus::Rejected, Some(reason), None),
            };
            ledger.rebases.insert(
                key,
                CohortHeadcountRebaseOutcome {
                    ingress: entry.ingress,
                    admitted_at: entry.admitted_at,
                    settled_at: at,
                    status,
                    rejection,
                    rebase,
                    previous_headcount,
                },
            );
        }
        SOCIETY_LIFECYCLE_DELTA_INGRESS => {
            if ledger.lifecycle_deltas.contains_key(&key) {
                return Ok(());
            }
            let (status, rejection, blocked_releases) = match &entry.admission_rejection {
                Some(reason) => (
                    SocietyIngressStatus::Rejected,
                    Some(reason.clone()),
                    BTreeSet::new(),
                ),
                None => {
                    match serde_json::from_value::<SocietyLifecycleDeltaV1>(entry.payload.clone()) {
                        Err(_) => (
                            SocietyIngressStatus::Rejected,
                            Some(SOCIETY_INGRESS_MALFORMED_REJECTION.to_owned()),
                            BTreeSet::new(),
                        ),
                        Ok(delta) => apply_queued_lifecycle_delta(view, state, delta)?,
                    }
                }
            };
            ledger.lifecycle_deltas.insert(
                key,
                SocietyLifecycleDeltaOutcome {
                    ingress: entry.ingress,
                    admitted_at: entry.admitted_at,
                    settled_at: at,
                    status,
                    rejection,
                    blocked_releases,
                },
            );
        }
        other => {
            return Err(invalid(format!(
                "society ingress queue contains unsupported packet {other}"
            )));
        }
    }
    Ok(())
}

/// Applies one delivered lifecycle delta. A release is blocked, while the
/// rest of the delta still applies, when a released alignment carries a
/// stored institutional decision (decision components are permanent, so
/// releasing the alignment would orphan them) or when a live society
/// dependency would make the release fail; blocked targets are recorded on
/// the outcome.
fn apply_queued_lifecycle_delta(
    view: &SimulationView<'_>,
    state: &mut SocietyState,
    mut delta: SocietyLifecycleDeltaV1,
) -> Result<(SocietyIngressStatus, Option<String>, BTreeSet<String>), CanwuError> {
    let mut blocked = BTreeSet::new();
    for set in delta
        .deactivations
        .iter()
        .filter(|set| delta.releases.contains(&set.target_id))
    {
        for alignment in &set.alignments {
            if let Some(stored) = state.institutional_alignments.get(&alignment.id)
                && view
                    .component(&policy_decision_key(), &stored.institution, &stored.id)?
                    .is_some()
            {
                blocked.insert(set.target_id.clone());
            }
        }
    }
    delta.releases.retain(|target| !blocked.contains(target));
    let result = match state.apply_lifecycle_delta(&delta) {
        Err(_) if !delta.releases.is_empty() => {
            blocked.append(&mut delta.releases);
            state.apply_lifecycle_delta(&delta)
        }
        result => result,
    };
    Ok(match result {
        Ok(()) => (SocietyIngressStatus::Applied, None, blocked),
        Err(error) => (
            SocietyIngressStatus::Rejected,
            Some(error.message),
            BTreeSet::new(),
        ),
    })
}

/// Appends every admitted rebase and lifecycle delta to the owner-side queue.
///
/// Runs in phase 12 of any boundary that admits ingress, so the rebase stock
/// check sees the state committed through phase 11 of the admitting boundary.
/// The next Daily phase-7 settlement consumes the queue.
fn intake_society_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let (mut queue, queue_record) = load_ingress_queue(view)?;
    let mut directives = Vec::new();
    let mut changed = false;
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
        if !is_queued_packet(plugin, packet_type) {
            continue;
        }
        if queue.entries.len() >= MAX_SOCIETY_INGRESS_QUEUE {
            directives.push(BoundaryDirective::Emit {
                event_type: SOCIETY_INGRESS_REJECTED_EVENT.to_owned(),
                summary: format!(
                    "Rejected society ingress {ingress_id}: the ingress queue is full"
                ),
                affected: Vec::new(),
            });
            continue;
        }
        let admission_rejection = if packet_type == COHORT_REBASE_INGRESS {
            match serde_json::from_value::<CohortHeadcountRebaseV1>(payload.clone()) {
                Err(_) => Some(SOCIETY_INGRESS_MALFORMED_REJECTION.to_owned()),
                Ok(rebase) => {
                    let current = rebase.external_stock.record.kind.namespace
                        != crate::model::SOCIETY_NAMESPACE
                        && view.domain_record_version_is_current(&rebase.external_stock)?;
                    rebase_admission_rejection(&rebase, current)
                }
            }
        } else {
            None
        };
        queue.entries.push(QueuedSocietyIngress {
            ingress: *ingress_id,
            admitted_at: context.boundary_id,
            packet_type: packet_type.clone(),
            payload: payload.clone(),
            admission_rejection,
        });
        changed = true;
    }
    if changed {
        queue.validate()?;
        let draft = DomainRecordDraft::from_typed(society_ingress_queue_reference(), &queue)?;
        let mutation = match queue_record {
            Some(record) => DomainRecordMutation::Update {
                record: draft,
                expected_version: record.version,
            },
            None => DomainRecordMutation::Create { record: draft },
        };
        directives.push(BoundaryDirective::MutateRecord {
            mutation,
            summary: "Queued admitted society ingress".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn load_ingress_queue(
    view: &SimulationView<'_>,
) -> Result<(SocietyIngressQueue, Option<DomainRecord>), CanwuError> {
    let Some(record) = view.typed_domain_record(&society_ingress_queue_reference())? else {
        return Ok((
            SocietyIngressQueue {
                schema_version: SocietyIngressQueue::SCHEMA_VERSION,
                entries: Vec::new(),
            },
            None,
        ));
    };
    let queue = record.decode_payload::<SocietyIngressQueueRecord>()?;
    queue.validate()?;
    // Only the intake writes queue entries. A non-empty queue whose current
    // version the initial scenario established was never admitted.
    if !queue.entries.is_empty()
        && view
            .current_domain_record_version(&record.reference)?
            .is_none_or(|version| {
                version.established_by == canwu_api::DomainRecordVersionSource::InitialScenario
            })
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "an initial scenario cannot seed admitted society ingress",
        ));
    }
    Ok((queue, Some(record.clone())))
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

/// Component state holding versioned institutional policy decisions that the
/// society plugin applies at its next Daily settlement. A reader (for example
/// a lifecycle provider) declares this key to observe accepted decisions.
#[must_use]
pub fn society_policy_decision_state_key() -> StateKey {
    policy_decision_key()
}

fn ingress_queue_key() -> StateKey {
    StateKey::new(
        SocietyIngressQueueRecord::NAMESPACE,
        SocietyIngressQueueRecord::NAME,
    )
}

fn rebase_payload_schema() -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([
            ("cohort_id".to_owned(), required(PayloadValueType::String)),
            (
                "new_headcount".to_owned(),
                required(PayloadValueType::Integer),
            ),
            (
                "external_stock".to_owned(),
                required(PayloadValueType::Object),
            ),
            ("reason".to_owned(), required(PayloadValueType::String)),
        ]),
        allow_additional: false,
    }
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
            ("rebases".to_owned(), optional(PayloadValueType::Object)),
            (
                "lifecycle_deltas".to_owned(),
                optional(PayloadValueType::Object),
            ),
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
