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
use crate::revision::{
    PRESSURE_REVISION_AUTHORITY_REJECTION, PRESSURE_REVISION_COMMAND,
    PRESSURE_REVISION_ORIGIN_REJECTION, PRESSURE_REVISION_SCOPE_REJECTION,
    PRESSURE_REVISION_STALE_REJECTION, PRESSURE_REVISION_SUPERSEDED_REJECTION,
    PressureRevisionDecision, PressureRevisionOutcome, RevisionScopeError,
    SOCIETY_PRESSURE_REVISED_EVENT, SOCIETY_PRESSURE_REVISION_INGRESS,
    SOCIETY_PRESSURE_REVISION_REJECTED_EVENT, SocietyPressureRevisionV1,
    pressure_revision_admission_rejection,
};
use crate::settle_transitions;
use crate::solver::{compute_aggregates, compute_mobilization_candidates, compute_projections};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    CanwuError, CommandId, DecisionOrigin, DomainRecord, DomainRecordDraft, DomainRecordMutation,
    DomainRecordSchema, DomainRecordType, ErrorCode, IngressClass, IngressPayload, Issuer,
    PayloadProperty, PayloadSchema, PayloadValueType, PluginActionDescriptor,
    PluginIngressDescriptor, PluginRegistrar, SimTime, SimulationPlugin, SimulationView, StateKey,
    StateVisibility, SystemCadence, SystemDirective,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const PLUGIN_NAME: &str = "canwu-society";

/// The society plugin: the only writer of `canwu.society:state`.
///
/// # Settlement cadence
///
/// Society settles in the boundaries that carry the plugin's settlement
/// cadence, a [`SystemCadence`] category: `Daily` for [`Self::DAILY`] and
/// [`Self::default`], or the category given to
/// [`Self::with_settlement_cadence`]. Canwu keeps no calendar, so the host
/// decides which boundaries carry the category: a `Monthly` cadence settles
/// society whenever the host settles a `Monthly` boundary, whatever the
/// length of its months, and a host that wants a week or another declared
/// period schedules a category it does not otherwise use, for example
/// `EraScheduled`, at that period.
///
/// The four settlement systems (`settle-social-transitions` in phase 7,
/// `evaluate-mobilization-candidates` in phase 10, `aggregate-social-state`
/// in phase 12, and `materialize-society-projections` in phase 13) run on
/// that cadence. The two event-driven intakes do not: they queue admitted
/// work at any boundary, and the next settlement applies it.
///
/// A transition rate is **per settlement**, not per unit of simulated time
/// (see [`crate::settle_transitions`]), so an application authors each rate
/// for the cadence it registers. The cadence is recorded in the plugin
/// descriptor that every snapshot persists: a run is restored and replayed
/// only with a plugin of the same cadence, and any other is refused with
/// `PluginManifestMismatch`.
///
/// Everything the phase-7 system applies waits for the cadence: due cohort
/// transfers, queued rebases, lifecycle deltas and pressure revisions,
/// institutional decisions, and transitions. Under a coarse cadence that
/// has three costs. A due transfer is checked against the state the earlier
/// transfers of the same settlement left, so of the due transfers that share
/// a cohort, as source or destination, one completes per settlement and the
/// others settle as `stale_source` and must be reissued.
/// The queue record is rewritten whole by every boundary that admits a
/// packet, so the evidence of a queue that waits a period grows with the
/// square of the packets it collects. And the intake drops a packet that
/// arrives while [`crate::MAX_SOCIETY_INGRESS_QUEUE`] wait, with only a
/// [`SOCIETY_INGRESS_REJECTED_EVENT`]: a dropped rebase leaves its cohort
/// behind its stock until the provider sends another, and a dropped pressure
/// revision never settles and must be decided again.
///
/// A settlement that changes nothing writes nothing. The first settlement
/// after a change materializes each derived view that the change cleared
/// (see the `last_*_at` fields of [`crate::SocietyState`]).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SocietyPlugin {
    settlement_cadence: SystemCadence,
}

impl SocietyPlugin {
    /// The society plugin on the default `Daily` settlement cadence.
    pub const DAILY: Self = Self {
        settlement_cadence: SystemCadence::Daily,
    };

    /// Returns the society plugin whose settlement systems run on `cadence`.
    ///
    /// Registration fails with `InvalidPluginRegistration` for
    /// `SystemCadence::EventDriven`, which is not a category a host
    /// schedules.
    #[must_use]
    pub const fn with_settlement_cadence(cadence: SystemCadence) -> Self {
        Self {
            settlement_cadence: cadence,
        }
    }

    /// The cadence category whose boundaries settle society.
    #[must_use]
    pub fn settlement_cadence(&self) -> SystemCadence {
        self.settlement_cadence.clone()
    }
}

impl Default for SocietyPlugin {
    fn default() -> Self {
        Self::DAILY
    }
}

impl SimulationPlugin for SocietyPlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn semantic_hash(&self) -> &'static str {
        // The handlers are the same for every settlement cadence; the cadence
        // itself is part of the descriptor's boundary-system contracts.
        "be25498655752ecfa41d681679cda37021792b273840f6c68b933835f3086521"
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
        let cadence = || self.settlement_cadence();
        if cadence() == SystemCadence::EventDriven {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "the society settlement cadence must be a scheduled cadence category",
            ));
        }
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
        // Only the revision command queues an admissible pressure revision;
        // the host cannot author one.
        let _revision_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
            name: SOCIETY_PRESSURE_REVISION_INGRESS.to_owned(),
            description: "Apply one authority-checked pressure revision decision".to_owned(),
            class: IngressClass::Decision,
            payload_schema: PayloadSchema::Object {
                properties: BTreeMap::from([
                    ("decision".to_owned(), required(PayloadValueType::Object)),
                    ("issuer".to_owned(), required(PayloadValueType::Object)),
                    ("command".to_owned(), required(PayloadValueType::Integer)),
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
                reads: vec![
                    society_state_key(),
                    cohort_exchange_ledger_key(),
                    pending_transfer_key(),
                ],
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
        ];
        transfer_intake.writes = vec![pending_transfer_key()];
        transfer_intake.emits = vec![SOCIETY_INGRESS_REJECTED_EVENT.to_owned()];
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
        registrar.register_command(
            PluginActionDescriptor {
                name: PRESSURE_REVISION_COMMAND.to_owned(),
                description: "Queue a versioned institutional revision of policy pressure and \
                              of influence edge and organization activity and reach"
                    .to_owned(),
                payload_schema: pressure_revision_payload_schema(),
                reads: vec![society_state_key(), cohort_exchange_ledger_key()],
                writes: Vec::new(),
            },
            revise_society_pressure,
        )?;

        let mut transition = BoundarySystemContract::new(
            "settle-social-transitions",
            BoundaryPhase::DomainDeltaProposal,
            cadence(),
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
        transition.emits = vec![
            SOCIETY_PRESSURE_REVISED_EVENT.to_owned(),
            SOCIETY_PRESSURE_REVISION_REJECTED_EVENT.to_owned(),
        ];
        transition.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(transition, settle_social_transitions)?;

        let mut mobilization = BoundarySystemContract::new(
            "evaluate-mobilization-candidates",
            BoundaryPhase::HistoricalCandidateEvaluation,
            cadence(),
        );
        mobilization.reads = vec![society_state_key()];
        mobilization.writes = vec![society_state_key()];
        mobilization.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(mobilization, evaluate_mobilization)?;

        let mut aggregate = BoundarySystemContract::new(
            "aggregate-social-state",
            BoundaryPhase::StrategicAggregation,
            cadence(),
        );
        aggregate.reads = vec![society_state_key()];
        aggregate.writes = vec![society_state_key()];
        aggregate.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(aggregate, aggregate_social_state)?;

        let mut project = BoundarySystemContract::new(
            "materialize-society-projections",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            cadence(),
        );
        project.reads = vec![society_state_key()];
        project.writes = vec![society_state_key()];
        project.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(project, materialize_projections)
    }
}

/// Admits one cohort transfer and queues it for its due time.
///
/// Every expected refusal uses an error code the engine records as a
/// rejected command attempt, so a refused transfer never fails its boundary
/// or blocks the commands queued behind it: `InvalidPayload` for a malformed
/// intent, `InvalidAuthority` for a missing or mismatched authority (a run
/// without a society root has no alignment and so no authority),
/// `EntityNotFound` for an unknown source or destination cohort,
/// `InvalidDuration` for a due time in the past, `ValueOutOfRange` for an
/// expected source version that is not the root's current version, and
/// `IdempotencyConflict` for an operation ID bound to another settled or
/// pending transfer. A resend of a settled or pending transfer is accepted
/// and queues nothing. An invalid society record is not a refusal and still
/// fails the boundary.
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
            ErrorCode::InvalidAuthority,
            "the society state record is not configured, so no alignment grants authority",
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
    for cohort_id in [&intent.source_cohort_id, &intent.destination_cohort_id] {
        if !state.cohorts.contains_key(cohort_id) {
            return Err(CanwuError::new(
                ErrorCode::EntityNotFound,
                format!("cohort transfer names unknown cohort {cohort_id}"),
            ));
        }
    }
    if intent.due_time < view.time() {
        return Err(CanwuError::new(
            ErrorCode::InvalidDuration,
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
    // A transfer that was admitted and has not settled yet holds its
    // operation ID as well. One whose packet is still in flight is not
    // visible here; the intake settles that case.
    if let Some(existing) = load_pending(view)?.0.transfers.get(&intent.operation_id) {
        if existing.intent == intent {
            return Ok(Vec::new());
        }
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "operation id is already bound to a different pending transfer",
        ));
    }
    if record.version != intent.expected_source_version {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
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
/// Event a society intake emits for an admitted packet it does not queue:
/// a rebase, lifecycle delta, or pressure revision that arrives while the
/// ingress queue is full, and a cohort transfer packet that is malformed,
/// was not produced by the transfer command, names an unknown cohort, has
/// lost its authority, or reuses an operation ID with other data. The
/// summary names the ingress and the reason; the boundary commits.
pub const SOCIETY_INGRESS_REJECTED_EVENT: &str = "society_ingress_rejected_v1";
const TRANSFER_UNKNOWN_COHORT: &str = "unknown_cohort";
const TRANSFER_OPERATION_CONFLICT: &str = "operation_id_conflict";
const TRANSFER_COMPLETED: &str = "completed";
const TRANSFER_STALE_SOURCE: &str = "stale_source";
const TRANSFER_REJECTED: &str = "rejected";

fn pending_transfer_key() -> StateKey {
    StateKey::new(
        SocietyCohortTransferPendingRecord::NAMESPACE,
        SocietyCohortTransferPendingRecord::NAME,
    )
}

/// Queues every admitted cohort transfer packet in the pending record.
///
/// The system is total over what a packet can carry: a defective packet is
/// skipped with a [`SOCIETY_INGRESS_REJECTED_EVENT`] and the boundary
/// commits. Only an invalid society root or pending record is an error.
fn intake_cohort_transfer(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    // This system runs in every boundary that admits an event or ingress of
    // any plugin; it decodes the society root only when one of them is a
    // cohort transfer.
    let mut packets = Vec::new();
    for ingress_id in &context.admitted_ingress {
        if let Some(ingress) = view.ingress(*ingress_id)?
            && matches!(
                &ingress.payload,
                IngressPayload::Plugin { plugin, packet_type, .. }
                    if plugin == PLUGIN_NAME && packet_type == COHORT_TRANSFER_INGRESS
            )
        {
            packets.push(ingress);
        }
    }
    if packets.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    let state = load_state(view)?.map(|(_, state)| state);
    let (mut pending, pending_record) = load_pending(view)?;
    let mut directives = Vec::new();
    let mut changed = false;
    for ingress in packets {
        match admitted_cohort_transfer(state.as_ref(), &pending, ingress) {
            Ok(Some(transfer)) => {
                pending
                    .transfers
                    .insert(transfer.intent.operation_id.clone(), transfer);
                changed = true;
            }
            // A second delivery of a transfer that is already pending.
            Ok(None) => {}
            Err(reason) => directives.push(BoundaryDirective::Emit {
                event_type: SOCIETY_INGRESS_REJECTED_EVENT.to_owned(),
                summary: format!(
                    "Rejected cohort transfer ingress {}: {reason}",
                    ingress.id.get()
                ),
                affected: Vec::new(),
            }),
        }
    }
    if changed {
        let pending_ref = society_cohort_transfer_pending_reference();
        let mutation = match pending_record {
            Some(record) => DomainRecordMutation::Update {
                record: DomainRecordDraft::from_typed(pending_ref, &pending)?,
                expected_version: record.version,
            },
            None => DomainRecordMutation::Create {
                record: DomainRecordDraft::from_typed(pending_ref, &pending)?,
            },
        };
        directives.push(BoundaryDirective::MutateRecord {
            mutation,
            summary: "Persist admitted society cohort transfer".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Decides one admitted cohort transfer packet: the pending transfer to
/// queue, `None` for a second delivery of a pending one, or the reason the
/// packet is rejected.
///
/// A packet is authentic exactly when its ingress record names, as its
/// cause, the command its payload carries. Only the transfer command's
/// handler produces that, after checking the issuer's authority; a host
/// cannot give a packet a command cause. The check reads the ingress record
/// in the boundary that admits it and no retained command evidence, so a
/// sealed run and its exact replay decide alike. Authority is then checked
/// again against the current root: the alignment must still authorize the
/// actor for the institution the packet was queued for.
fn admitted_cohort_transfer(
    state: Option<&SocietyState>,
    pending: &SocietyCohortTransferPending,
    ingress: &canwu_api::IngressRecord,
) -> Result<Option<PendingCohortTransfer>, &'static str> {
    let IngressPayload::Plugin {
        payload,
        affected_entities,
        ..
    } = &ingress.payload
    else {
        return Err(SOCIETY_INGRESS_MALFORMED_REJECTION);
    };
    let (Some(intent), Some(actor), Some(command_id)) = (
        payload
            .get("intent")
            .and_then(|value| serde_json::from_value::<CohortTransferIntent>(value.clone()).ok())
            .filter(|intent| validate_transfer_intent(intent).is_ok()),
        payload.get("actor").and_then(Value::as_u64),
        payload.get("command_id").and_then(Value::as_u64),
    ) else {
        return Err(SOCIETY_INGRESS_MALFORMED_REJECTION);
    };
    if ingress.cause != Some(canwu_api::CauseRef::Command(CommandId::new(command_id))) {
        return Err(PRESSURE_REVISION_ORIGIN_REJECTION);
    }
    let actor = canwu_api::PersonId::new(actor);
    let Some(state) = state else {
        return Err(PRESSURE_REVISION_AUTHORITY_REJECTION);
    };
    if !state
        .institutional_alignments
        .get(&intent.authority_alignment_id)
        .is_some_and(|alignment| {
            alignment.authorized_actor == Some(actor)
                && affected_entities.contains(&alignment.institution)
        })
    {
        return Err(PRESSURE_REVISION_AUTHORITY_REJECTION);
    }
    let Ok(source_digest) = cohort_transfer_digest(state, &intent) else {
        return Err(TRANSFER_UNKNOWN_COHORT);
    };
    let transfer = PendingCohortTransfer {
        intent,
        actor,
        command_id,
        source_digest,
    };
    match pending.transfers.get(&transfer.intent.operation_id) {
        None => Ok(Some(transfer)),
        // The same transfer delivered again, by the same or a resent command.
        Some(existing) if existing.intent == transfer.intent && existing.actor == actor => Ok(None),
        Some(_) => Err(TRANSFER_OPERATION_CONFLICT),
    }
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
            ErrorCode::InvalidPayload,
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
    let destination = state
        .cohorts
        .get_mut(&intent.destination_cohort_id)
        .expect("destination exists");
    destination.headcount = destination
        .headcount
        .checked_add(intent.quantity)
        .ok_or_else(|| invalid("cohort transfer would overflow the destination cohort"))?;
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

/// Admits one institutional policy decision as the alignment's pending
/// decision.
///
/// Every expected refusal uses an error code the engine records as a
/// rejected command attempt, so a refused decision never fails its boundary
/// or blocks the commands queued behind it: `InvalidPayload` for a malformed
/// decision, `InvalidAuthority` for a missing or mismatched authority (an
/// unknown alignment, or a run without a society root, grants none), and
/// `IdempotencyConflict` for a version that is not newer than the applied or
/// the pending one, which includes a resend. An invalid society record or
/// stored decision is not a refusal and still fails the boundary.
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
    validate_policy_decision(&decision)
        .map_err(|error| CanwuError::new(ErrorCode::InvalidPayload, error.message))?;
    let state = load_state(view)?.map(|(_, state)| state);
    let alignment = state
        .as_ref()
        .and_then(|state| state.institutional_alignments.get(&decision.alignment_id))
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                format!(
                    "unknown institutional policy authority alignment {}",
                    decision.alignment_id
                ),
            )
        })?;
    validate_policy_authority(context, alignment)?;
    if decision.decision_version <= alignment.last_decision_version {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
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
                ErrorCode::IdempotencyConflict,
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

/// Admits one pressure revision decision and queues it for the next society
/// settlement.
///
/// The decision is authorized exactly as `set_institutional_policy` is: a
/// validated `DecisionTicket` controller, the alignment's institution as
/// command subject, and a decision origin that matches the alignment's
/// authorized actor. Every named row must exist and lie within the
/// alignment's authority now, and is checked again when the decision settles.
/// A resend of the alignment's applied decision is accepted and queues
/// nothing; the same version with other content is an `IdempotencyConflict`.
/// The decision's place in the alignment's version sequence is settled by
/// the owner in admission order, because an earlier decision may still be
/// queued.
///
/// Every refusal uses an error code the engine records as a rejected command
/// attempt (`InvalidPayload`, `InvalidAuthority`, `EntityNotFound`, or
/// `IdempotencyConflict`), so a refused decision never fails its boundary.
fn revise_society_pressure(
    view: &SimulationView<'_>,
    context: &canwu_api::CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let decision: PressureRevisionDecision =
        serde_json::from_value(payload.clone()).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!("pressure revision payload could not be decoded: {error}"),
            )
        })?;
    decision
        .validate()
        .map_err(|error| CanwuError::new(ErrorCode::InvalidPayload, error.message))?;
    // Without a society root there is no alignment and so no authority.
    let state = load_state(view)?.map(|(_, state)| state);
    let alignment = state
        .as_ref()
        .and_then(|state| state.institutional_alignments.get(&decision.alignment_id))
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                format!(
                    "unknown pressure revision authority alignment {}",
                    decision.alignment_id
                ),
            )
        })?;
    validate_policy_authority(context, alignment)?;
    state
        .as_ref()
        .map_or(Ok(()), |state| {
            state.pressure_revision_scope(&decision, alignment)
        })
        .map_err(RevisionScopeError::into_error)?;

    let (ledger, _) = load_ledger(view)?;
    if let Some(applied) = ledger
        .pressure_revisions
        .get(&alignment.id)
        .and_then(|outcomes| outcomes.applied.as_ref())
        && applied.decision_version == decision.decision_version
    {
        if applied.digest == decision.digest()? {
            return Ok(Vec::new());
        }
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            format!(
                "pressure revision version {} of alignment {} is already applied with other \
                 content",
                decision.decision_version, alignment.id
            ),
        ));
    }
    let issuer = alignment.institution.clone();
    let packet = SocietyPressureRevisionV1 {
        decision,
        issuer: issuer.clone(),
        command: context.command_id,
    };
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: canwu_api::SimDuration::ZERO,
        packet_type: SOCIETY_PRESSURE_REVISION_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(&packet).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!("pressure revision could not be encoded: {error}"),
            )
        })?,
        affected: vec![issuer],
    }])
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
        // terminal rejection instead of failing every later settlement. The
        // digest is compared with the in-progress state, so an earlier
        // transfer of this settlement that moved either cohort makes this
        // one stale: of the due transfers that share a cohort, one settles
        // per settlement, and the others must be reissued against the new
        // state.
        let result = match cohort_transfer_digest(&state, &transfer.intent) {
            Ok(digest) if digest == transfer.source_digest => {
                // A transfer that does not fit, or whose result is not a
                // valid society state, is a terminal rejection.
                let mut draft = state.clone();
                if apply_cohort_transfer(&mut draft, &transfer.intent).is_ok()
                    && draft.canonicalize().is_ok()
                    && draft.validate().is_ok()
                {
                    state = draft;
                    TRANSFER_COMPLETED
                } else {
                    TRANSFER_REJECTED
                }
            }
            Ok(_) => TRANSFER_STALE_SOURCE,
            // A cohort the pending transfer names no longer exists.
            Err(_) => TRANSFER_REJECTED,
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
    // Queued rebases, lifecycle deltas, and pressure revisions apply in
    // admission order, after transfers and before institutional decisions
    // and transitions.
    let (queue, queue_record) = load_ingress_queue(view)?;
    let queue_consumed = !queue.entries.is_empty();
    let mut ledger_changed = transfer_changed;
    let mut events = Vec::new();
    for entry in &queue.entries {
        ledger_changed |= settle_queued_ingress(
            view,
            &mut state,
            &mut ledger,
            entry,
            context.at,
            &mut events,
        )?;
    }
    apply_pending_policies(view, &mut state)?;
    // A settlement that changes nothing leaves the state, and so the root
    // record, untouched.
    settle_transitions(&mut state, context.at)?;
    carry_unchanged_projections(&before, &mut state, context.at);
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
    proposal.directives.extend(events);
    if ledger_changed {
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

/// Applies one queued packet to the in-progress state, records its terminal
/// outcome, and returns whether the ledger changed. A rejected packet leaves
/// the state unchanged.
fn settle_queued_ingress(
    view: &SimulationView<'_>,
    state: &mut SocietyState,
    ledger: &mut SocietyCohortExchangeLedger,
    entry: &QueuedSocietyIngress,
    at: SimTime,
    events: &mut Vec<BoundaryDirective>,
) -> Result<bool, CanwuError> {
    let key = entry.ingress.get().to_string();
    match entry.packet_type.as_str() {
        SOCIETY_PRESSURE_REVISION_INGRESS => {
            return settle_pressure_revision(state, ledger, entry, at, events);
        }
        COHORT_REBASE_INGRESS => {
            if ledger.rebases.contains_key(&key) {
                return Ok(false);
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
                return Ok(false);
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
    Ok(true)
}

/// Settles one queued pressure revision and returns whether the ledger
/// changed.
///
/// The decision applies when its alignment is still bound to the deciding
/// institution, its version is newer than the alignment's applied revision
/// version, its expected version is that applied version, and every row it
/// names still lies within the alignment's authority. A resend of the
/// applied decision changes nothing and records nothing. Every other packet
/// is rejected without changing society state. An applied or rejected
/// decision replaces the alignment's ledger outcome of that kind and is
/// emitted as an event naming the decision.
fn settle_pressure_revision(
    state: &mut SocietyState,
    ledger: &mut SocietyCohortExchangeLedger,
    entry: &QueuedSocietyIngress,
    at: SimTime,
    events: &mut Vec<BoundaryDirective>,
) -> Result<bool, CanwuError> {
    let packet = match &entry.admission_rejection {
        Some(_) => None,
        None => serde_json::from_value::<SocietyPressureRevisionV1>(entry.payload.clone()).ok(),
    };
    let Some(packet) = packet else {
        // The packet did not come from the revision command, so it names no
        // decision an alignment's ledger entry could record.
        let reason = entry
            .admission_rejection
            .as_deref()
            .unwrap_or(SOCIETY_INGRESS_MALFORMED_REJECTION);
        events.push(BoundaryDirective::Emit {
            event_type: SOCIETY_PRESSURE_REVISION_REJECTED_EVENT.to_owned(),
            summary: format!(
                "Rejected pressure revision ingress {}: {reason}",
                entry.ingress.get()
            ),
            affected: Vec::new(),
        });
        return Ok(false);
    };
    let decision = &packet.decision;
    let digest = decision.digest()?;
    let applied = ledger
        .pressure_revisions
        .get(&decision.alignment_id)
        .and_then(|outcomes| outcomes.applied.as_ref());
    if applied.is_some_and(|outcome| {
        outcome.decision_version == decision.decision_version && outcome.digest == digest
    }) {
        return Ok(false);
    }
    let applied_version = applied.map_or(0, |outcome| outcome.decision_version);
    let authority_holds = state
        .institutional_alignments
        .get(&decision.alignment_id)
        .is_some_and(|alignment| alignment.institution == packet.issuer);
    let rejection = if !authority_holds {
        Some(PRESSURE_REVISION_AUTHORITY_REJECTION)
    } else if decision.decision_version <= applied_version {
        Some(PRESSURE_REVISION_SUPERSEDED_REJECTION)
    } else if decision.expected_version != applied_version {
        Some(PRESSURE_REVISION_STALE_REJECTION)
    } else if state.apply_pressure_revision(decision).is_err() {
        Some(PRESSURE_REVISION_SCOPE_REJECTION)
    } else {
        None
    };
    let outcome = PressureRevisionOutcome {
        decision_version: decision.decision_version,
        digest,
        issuer: packet.issuer.clone(),
        command: packet.command,
        ingress: entry.ingress,
        settled_at: at,
        rejection: rejection.map(str::to_owned),
    };
    let outcomes = ledger
        .pressure_revisions
        .entry(decision.alignment_id.clone())
        .or_default();
    let identity = format!(
        "pressure revision {} of alignment {} (command {}, ingress {})",
        decision.decision_version,
        decision.alignment_id,
        packet.command.get(),
        entry.ingress.get()
    );
    let (event_type, summary) = match rejection {
        None => {
            outcomes.applied = Some(outcome);
            (
                SOCIETY_PRESSURE_REVISED_EVENT,
                format!("Applied {identity}"),
            )
        }
        Some(reason) => {
            outcomes.rejected = Some(outcome);
            (
                SOCIETY_PRESSURE_REVISION_REJECTED_EVENT,
                format!("Rejected {identity}: {reason}"),
            )
        }
    };
    events.push(BoundaryDirective::Emit {
        event_type: event_type.to_owned(),
        summary,
        affected: Vec::new(),
    });
    Ok(true)
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

/// Appends every admitted rebase, lifecycle delta, and pressure revision to
/// the owner-side queue.
///
/// Runs in phase 12 of any boundary that admits ingress, so the rebase stock
/// check sees the state committed through phase 11 of the admitting boundary.
/// A pressure revision is admissible only when its ingress record names the
/// revision command it carries as its cause. The next phase-7 settlement
/// consumes the queue.
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
        } else if packet_type == SOCIETY_PRESSURE_REVISION_INGRESS {
            pressure_revision_admission_rejection(payload, ingress.cause.as_ref())
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

/// Returns whether the visible society root carries the named
/// materialization timestamp, without decoding the root.
///
/// A set timestamp means the materialized view is current: every change a
/// view depends on clears its timestamp in the same write, and a root whose
/// timestamped view does not match its authoritative state fails validation
/// before it is written and whenever it is loaded. The phase-7 settlement
/// loads and validates the root in every boundary these systems run in.
fn materialization_is_current(
    view: &SimulationView<'_>,
    timestamp: &str,
) -> Result<bool, CanwuError> {
    Ok(view
        .typed_domain_record(&society_state_reference())?
        .is_some_and(|record| {
            record
                .payload
                .get(timestamp)
                .is_some_and(|value| !value.is_null())
        }))
}

/// Materializes the mobilization candidates once after a change they depend
/// on cleared their timestamp, and sets it, also when the state yields no
/// candidate. A boundary in which they are current writes nothing.
fn evaluate_mobilization(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if materialization_is_current(view, "last_mobilization_at")? {
        return Ok(BoundaryProposal::default());
    }
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    state.mobilization_candidates = compute_mobilization_candidates(&state, context.at);
    state.last_mobilization_at = Some(context.at);
    update_state(&record, state, "Evaluated social mobilization candidates")
}

/// Materializes the aggregates under the rule of [`evaluate_mobilization`].
fn aggregate_social_state(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if materialization_is_current(view, "last_aggregation_at")? {
        return Ok(BoundaryProposal::default());
    }
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    state.aggregates = compute_aggregates(&state);
    state.last_aggregation_at = Some(context.at);
    update_state(&record, state, "Aggregated social state")
}

/// Completes the projections under the rule of [`evaluate_mobilization`].
///
/// An observer whose projection the phase-7 settlement kept, because its
/// estimate did not change, keeps that projection and its `observed_at`;
/// every other observer receives a projection observed at this boundary.
fn materialize_projections(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if materialization_is_current(view, "last_projection_at")? {
        return Ok(BoundaryProposal::default());
    }
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let mut projections = compute_projections(&state, context.at);
    for (key, projection) in &mut projections {
        if let Some(kept) = state.projections.get(key)
            && kept.actor == projection.actor
            && kept.entries == projection.entries
        {
            projection.observed_at = kept.observed_at;
        }
    }
    state.projections = projections;
    state.last_projection_at = Some(context.at);
    update_state(
        &record,
        state,
        "Materialized actor-relative society projections",
    )
}

/// Restores, after the phase-7 changes, the projection of every observer
/// whose estimate those changes left as it was, and leaves the projections
/// of the others cleared for the phase-13 system.
///
/// An observer's `observed_at` therefore moves only in a settlement that
/// changes that observer's own entries. Clearing every projection whenever
/// any distribution moved would stamp every observer in that settlement and
/// so tell an observer that something outside its scope, or beyond what it
/// can detect, had changed.
fn carry_unchanged_projections(before: &SocietyState, state: &mut SocietyState, at: SimTime) {
    if before.projections.is_empty() {
        return;
    }
    // An estimate reads the distributions, the organizations, and the
    // observer profiles, and nothing else.
    if state.distributions == before.distributions
        && state.organizations == before.organizations
        && state.observer_profiles == before.observer_profiles
    {
        state.projections.clone_from(&before.projections);
        state.last_projection_at = before.last_projection_at;
        return;
    }
    let estimates = compute_projections(state, at);
    state.projections = before
        .projections
        .iter()
        .filter(|(key, kept)| {
            estimates.get(*key).is_some_and(|estimate| {
                estimate.actor == kept.actor && estimate.entries == kept.entries
            })
        })
        .map(|(key, kept)| (key.clone(), kept.clone()))
        .collect();
    state.last_projection_at = before
        .last_projection_at
        .filter(|_| state.projections.len() == estimates.len());
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
/// society plugin applies at its next settlement. A reader (for example
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
            (
                "pressure_revisions".to_owned(),
                optional(PayloadValueType::Object),
            ),
        ]),
        allow_additional: false,
    }
}

fn pressure_revision_payload_schema() -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([
            (
                "alignment_id".to_owned(),
                required(PayloadValueType::String),
            ),
            (
                "decision_version".to_owned(),
                required(PayloadValueType::Integer),
            ),
            (
                "expected_version".to_owned(),
                required(PayloadValueType::Integer),
            ),
            ("policies".to_owned(), optional(PayloadValueType::Array)),
            (
                "influence_edges".to_owned(),
                optional(PayloadValueType::Array),
            ),
            (
                "organizations".to_owned(),
                optional(PayloadValueType::Array),
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
