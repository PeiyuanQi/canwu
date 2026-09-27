use crate::knowledge::{
    ADDRESS_KNOWLEDGE_SCHEMA, ATTEMPT_REPORT_KNOWLEDGE_SCHEMA, CONNECTION_KNOWLEDGE_SCHEMA,
    ENDPOINT_KNOWLEDGE_SCHEMA, NetworkKnowledgeSeed, carrier_planning_snapshot,
    correspondence_knowledge_schemas, schema_id,
};
use crate::model::{
    CarrierAuthority, CarrierDelegationRecord, CarrierDelegationRequest, CommunicationOpportunity,
    CommunicationOpportunityRecord, CommunicationOpportunityRequest,
    CommunicationOpportunityStatus, CorrespondenceAttemptOutcome, CorrespondenceAttemptReport,
    CorrespondenceAuthority, CorrespondenceIncident, CorrespondenceIncidentKind,
    CorrespondenceIncidentRequest, CorrespondenceIntent, CorrespondenceOperation,
    CorrespondenceOperationRecord, CorrespondencePlanningEvidence, CorrespondenceRecovery,
    CorrespondenceRecoveryAction, CorrespondenceStatus, InformationSagaStep,
    InitiateCorrespondenceRequest, KnowledgeSeedReceipt, KnowledgeSeedRecord,
    PendingInformationOperation, ProgressAction, ProgressRequest, ResolveCorrespondenceRequest,
    carrier_delegation_ref, correspondence_operation_ref, knowledge_seed_ref, opportunity_ref,
};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    CanwuError, CauseRef, CommandContext, CommandIngress, DecisionOrigin, DomainRecord,
    DomainRecordDraft, DomainRecordMutation, DomainRecordSchema, DomainRecordType,
    DomainRecordVersionRef, DomainReference, DomainReferenceSchema, DomainReferenceTarget,
    DomainReferenceTargetKind, EntityRef, ErrorCode, EvidenceRef, HandoffId, HandoffKind,
    IngressClass, IngressPayload, KnowledgeHolderRef, KnowledgeOrigin, KnowledgeRecordDraft,
    KnowledgeWriteGrant, LegExecutionStatus, PayloadSchema, PluginActionDescriptor,
    PluginIngressDescriptor, PluginIngressTarget, PluginRegistrar, RandomOperationTarget,
    RandomStreamKey, ReconciliationOutcome, RoutingRequest, SimDuration, SimTime, SimulationPlugin,
    SimulationView, StateKey, StateVisibility, SystemCadence, SystemDirective, TransportExecution,
    TransportExecutionState, TypedDomainRecordRef, canonical_hash, plan_route,
};
use canwu_information::{
    Access, AccessPayload, AddressedDeliveryAttemptDraft, Channel, DeliveryAttempt,
    DeliveryAttemptPayload, DeliveryAttemptStatus, Dispatch, DispatchPayload, DispatchStatus,
    DispatchTarget, INFORMATION_INGRESS, InformationOperation, InformationOperationEnvelope,
    InformationOperationId, InformationOperationRecord, InformationOperationStatus,
    InformationOutputKind, InformationOutputSlot, LifecycleRequest, RecordBinding, Representation,
    addressed_attempt_output_slot, derive_operation_record_ref, derive_output_record_ref,
    validate_delegation_claim,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const PLUGIN_NAME: &str = "canwu-correspondence";
pub const CORRESPONDENCE_COMMAND: &str = "initiate_correspondence_v1";
pub const RESOLVE_CORRESPONDENCE_COMMAND: &str = "resolve_correspondence_v1";
pub const START_INGRESS: &str = "start_correspondence_v1";
pub const PROGRESS_INGRESS: &str = "progress_correspondence_v1";
pub const INCIDENT_INGRESS: &str = "correspondence_incident_v1";
pub const OPPORTUNITY_INGRESS: &str = "communication_opportunity_v1";
pub const KNOWLEDGE_INGRESS: &str = "install_correspondence_knowledge_v1";
const RESOLUTION_INGRESS: &str = "resolve_correspondence_v1";
/// Plugin-internal ingress that publishes one attempt report; only the
/// lifecycle system schedules it.
const REPORT_INGRESS: &str = "correspondence_attempt_report_v1";
/// Records an accepted carrier delegation; only the delegation command enqueues it.
const DELEGATION_INGRESS: &str = "record_carrier_delegation_v1";
/// Command by which a carrier accepts carrying correspondence for a sender;
/// see [`CarrierDelegationRequest`].
pub const CARRIER_DELEGATION_COMMAND: &str = "delegate_carrier_v1";
/// The delegation capability a carrier other than the sender must hold: the
/// claim of the cited [`CARRIER_DELEGATION_COMMAND`] must list it.
pub const CARRY_CORRESPONDENCE_CAPABILITY: &str = "carry_correspondence";

const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");
const SEMANTIC_HASH: &str = "a3f74357d75d36feae882c946ae7701277f300d1cd848a9cf8c20a7352c6f380";
const SEIZURE_FAILURE_REASON: &str = "carrier seized";
const REPORT_ORIGIN_METHOD: &str = "correspondence_attempt_report_v1";
const INPUT_HASH_DOMAIN: &str = "canwu.correspondence.input.v1";
const OPPORTUNITY_HASH_DOMAIN: &str = "canwu.correspondence.opportunity.v1";
const INCIDENT_HASH_DOMAIN: &str = "canwu.correspondence.incident.v1";
const RECOVERY_HASH_DOMAIN: &str = "canwu.correspondence.recovery.v1";
const KNOWLEDGE_SEED_HASH_DOMAIN: &str = "canwu.correspondence.knowledge-seed.v1";
const INFORMATION_NAMESPACE: &str = "canwu.correspondence";
const MAX_OPPORTUNITY_CANDIDATES: usize = 64;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct AdmittedStart {
    request: InitiateCorrespondenceRequest,
    authority: CorrespondenceAuthority,
    accepted_command: canwu_api::CommandId,
    /// The carrier delegation resolved at command admission; absent for a
    /// sender-owned carrier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    carrier_authority: Option<CarrierAuthority>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct AdmittedRecovery {
    request: ResolveCorrespondenceRequest,
    accepted_command: canwu_api::CommandId,
}

/// Payload of [`REPORT_INGRESS`]: the seizure incident whose confirmed
/// closure is reported. Phase 13 derives the whole report from persisted
/// records, so the payload asserts nothing it could get wrong.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct AttemptReportRequest {
    operation_key: String,
    incident_key: String,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CorrespondencePlugin;

impl SimulationPlugin for CorrespondencePlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }

    fn version(&self) -> &'static str {
        PLUGIN_VERSION
    }

    fn semantic_hash(&self) -> &'static str {
        SEMANTIC_HASH
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(operation_schema())?;
        registrar.register_record_schema(opportunity_schema())?;
        registrar.register_record_schema(knowledge_seed_schema())?;
        registrar.register_record_schema(carrier_delegation_schema())?;
        let knowledge_schemas = correspondence_knowledge_schemas();
        for schema in &knowledge_schemas {
            registrar.register_knowledge_schema(schema.clone())?;
        }
        registrar.register_command(
            PluginActionDescriptor {
                name: CORRESPONDENCE_COMMAND.to_owned(),
                description: "Initiate one decision-backed addressed correspondence".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: vec![
                    record_state::<CarrierDelegationRecord>(),
                    record_state::<CommunicationOpportunityRecord>(),
                    record_state::<Channel>(),
                    record_state::<Dispatch>(),
                ],
                writes: Vec::new(),
            },
            initiate_correspondence_command,
        )?;
        registrar.register_command(
            PluginActionDescriptor {
                name: CARRIER_DELEGATION_COMMAND.to_owned(),
                description: "Accept carrying correspondence for a sender under a delegation claim"
                    .to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: Vec::new(),
                writes: Vec::new(),
            },
            carrier_delegation_command_handler,
        )?;
        registrar.register_command(
            PluginActionDescriptor {
                name: RESOLVE_CORRESPONDENCE_COMMAND.to_owned(),
                description: "Replan, retry, or finalize one failed correspondence".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: vec![
                    record_state::<CarrierDelegationRecord>(),
                    record_state::<CorrespondenceOperationRecord>(),
                    record_state::<DeliveryAttempt>(),
                    record_state::<Dispatch>(),
                ],
                writes: Vec::new(),
            },
            resolve_correspondence_command_handler,
        )?;
        for (name, description, class) in [
            (
                START_INGRESS,
                "Start one admitted addressed correspondence",
                IngressClass::Communication,
            ),
            (
                PROGRESS_INGRESS,
                "Advance one persisted correspondence operation",
                IngressClass::ScheduledSystem,
            ),
            (
                INCIDENT_INGRESS,
                "Resolve one correspondence disaster or interception",
                IngressClass::Information,
            ),
            (
                OPPORTUNITY_INGRESS,
                "Evaluate one bounded communication opportunity",
                IngressClass::Decision,
            ),
            (
                KNOWLEDGE_INGRESS,
                "Install holder-relative correspondence planning knowledge",
                IngressClass::Information,
            ),
            (
                RESOLUTION_INGRESS,
                "Apply one admitted correspondence recovery action",
                IngressClass::Acknowledgement,
            ),
        ] {
            registrar.register_ingress(PluginIngressDescriptor {
                name: name.to_owned(),
                description: description.to_owned(),
                class,
                payload_schema: PayloadSchema::Any,
            })?;
        }
        // Only the delegation command enqueues delegation records, and only
        // the lifecycle system schedules reports; the host cannot author
        // either, so the permits are not retained.
        registrar.register_internal_ingress(PluginIngressDescriptor {
            name: DELEGATION_INGRESS.to_owned(),
            description: "Record one accepted carrier delegation".to_owned(),
            class: IngressClass::Acknowledgement,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_internal_ingress(PluginIngressDescriptor {
            name: REPORT_INGRESS.to_owned(),
            description: "Publish one holder-relative correspondence attempt report".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;

        let mut lifecycle = BoundarySystemContract::new(
            "correspondence-lifecycle-v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        lifecycle.reads = vec![
            StateKey::core_ingress(),
            StateKey::core_knowledge(),
            record_state::<CarrierDelegationRecord>(),
            record_state::<CommunicationOpportunityRecord>(),
            record_state::<CorrespondenceOperationRecord>(),
            record_state::<Dispatch>(),
            record_state::<DeliveryAttempt>(),
            record_state::<InformationOperationRecord>(),
        ];
        lifecycle.writes = vec![
            record_state::<CarrierDelegationRecord>(),
            record_state::<CommunicationOpportunityRecord>(),
            record_state::<CorrespondenceOperationRecord>(),
        ];
        lifecycle.random_streams = vec![correspondence_random_stream()];
        lifecycle.plugin_ingress_targets = vec![PluginIngressTarget {
            target_plugin: canwu_information::PLUGIN_NAME.to_owned(),
            packet_type: INFORMATION_INGRESS.to_owned(),
        }];
        lifecycle.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(lifecycle, settle_correspondence_lifecycle)?;

        let mut knowledge = BoundarySystemContract::new(
            "correspondence-knowledge-ingress-v1",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::EventDriven,
        );
        knowledge.reads = vec![
            StateKey::core_ingress(),
            record_state::<CorrespondenceOperationRecord>(),
            record_state::<DeliveryAttempt>(),
            record_state::<InformationOperationRecord>(),
            record_state::<KnowledgeSeedRecord>(),
        ];
        knowledge.writes = vec![record_state::<KnowledgeSeedRecord>()];
        knowledge.knowledge_writes = knowledge_schemas
            .into_iter()
            .map(|schema| KnowledgeWriteGrant {
                schema: schema.id,
                visibilities: vec![StateVisibility::SameBoundary],
            })
            .collect();
        knowledge.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(knowledge, install_planning_knowledge)
    }
}

fn initiate_correspondence_command(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.ingress == CommandIngress::LegacyDirect {
        return Err(invalid_authority(
            "correspondence initiation requires tracked command ingress",
        ));
    }
    let request: InitiateCorrespondenceRequest = decode(payload, "correspondence request")?;
    validate_start_request(&request)?;
    let carrier_authority = current_carrier_authority(
        view,
        &request.sender,
        &request.carrier,
        request.carrier_delegation,
        context.simulation_time,
    )?;
    if request.due_at < context.simulation_time {
        return Err(invalid_record(
            "correspondence deadline cannot precede command admission",
        ));
    }
    if !view.domain_record_version_evidence_exists(&request.prepared_dispatch)? {
        return Err(invalid_record(
            "prepared dispatch version evidence is unavailable",
        ));
    }
    let dispatch_ref = typed::<Dispatch>(request.prepared_dispatch.record.clone())?;
    let dispatch = view
        .typed_domain_record(&dispatch_ref)?
        .ok_or_else(|| invalid_record("prepared dispatch is missing"))?;
    if dispatch.version != request.prepared_dispatch.version {
        return Err(invalid_record("prepared dispatch version is stale"));
    }
    let dispatch_payload = dispatch.decode_payload::<Dispatch>()?;
    if dispatch_payload.status != DispatchStatus::Prepared
        || !matches!(
            &dispatch_payload.target,
            DispatchTarget::Addressed(recipients)
                if recipients.as_slice() == [request.recipient.clone()]
        )
    {
        return Err(invalid_record(
            "correspondence requires a prepared single-recipient dispatch",
        ));
    }
    let sender_references = dispatch
        .references
        .iter()
        .filter(|reference| reference.role == "sender")
        .collect::<Vec<_>>();
    if !matches!(
        sender_references.as_slice(),
        [reference]
            if reference.target == DomainReferenceTarget::Core(request.sender.clone())
    ) {
        return Err(invalid_record(
            "correspondence sender must match the prepared dispatch sender",
        ));
    }
    let channel_references = dispatch
        .references
        .iter()
        .filter_map(|reference| match (&*reference.role, &reference.target) {
            ("channel", DomainReferenceTarget::Domain(reference)) => Some(reference.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [channel_reference] = channel_references.as_slice() else {
        return Err(invalid_record(
            "correspondence dispatch must reference exactly one channel",
        ));
    };
    let channel_reference = typed::<Channel>(channel_reference.clone())?;
    let channel = view
        .typed_domain_record(&channel_reference)?
        .ok_or_else(|| invalid_record("correspondence channel is missing"))?
        .decode_payload::<Channel>()?;
    if channel.profile != request.channel_profile {
        return Err(invalid_record(
            "correspondence channel profile does not match the prepared dispatch",
        ));
    }

    let authority = if let Some(controller_id) = &context.decision_controller_id {
        if request.automatic_opportunity.is_some()
            || !origin_controls_sender(&context.authority.decision_origin, &request.sender)
        {
            return Err(invalid_authority(
                "decision-backed correspondence sender does not match command authority",
            ));
        }
        CorrespondenceAuthority::Decision {
            controller_id: controller_id.clone(),
        }
    } else {
        let opportunity = request.automatic_opportunity.clone().ok_or_else(|| {
            invalid_authority(
                "non-decision correspondence requires a selected automatic opportunity",
            )
        })?;
        if !matches!(
            context.authority.decision_origin,
            DecisionOrigin::NoResponsibleActor { .. }
        ) {
            return Err(invalid_authority(
                "automatic correspondence requires system authority",
            ));
        }
        let record = view
            .typed_domain_record(&opportunity)?
            .ok_or_else(|| invalid_record("automatic opportunity is missing"))?;
        let payload = record.decode_payload::<CommunicationOpportunityRecord>()?;
        if payload.status != CommunicationOpportunityStatus::SelectedAutomatic
            || payload.operation_key != request.operation_key
            || payload.sender != request.sender
            || payload.selected_recipient.as_ref() != Some(&request.recipient)
        {
            return Err(invalid_authority(
                "automatic opportunity does not authorize this correspondence",
            ));
        }
        CorrespondenceAuthority::Automatic { opportunity }
    };
    let admitted = AdmittedStart {
        request,
        authority,
        accepted_command: context.command_id,
        carrier_authority,
    };
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: SimDuration::ZERO,
        packet_type: START_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(admitted).map_err(encode_error)?,
        affected: Vec::new(),
    }])
}

/// Admits a carrier's acceptance of carrying correspondence for a sender.
///
/// Only the carrier's own command authority can issue it: the decision origin
/// must control the claim's `performed_by`. The claim must be well formed,
/// list [`CARRY_CORRESPONDENCE_CAPABILITY`], delegate to another party than
/// the carrier itself, and not already have expired. The admitted command's
/// ID is what a later [`InitiateCorrespondenceRequest::carrier_delegation`]
/// cites. At the next boundary the lifecycle persists the claim as the
/// carrier's current [`CarrierDelegationRecord`] for that principal (one
/// record per carrier and principal, replaced by a newer delegation), which
/// admission reads instead of command evidence. The delegation is citable
/// once that record exists.
fn carrier_delegation_command_handler(
    _view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.ingress == CommandIngress::LegacyDirect {
        return Err(invalid_authority(
            "carrier delegation requires tracked command ingress",
        ));
    }
    let request: CarrierDelegationRequest = decode(payload, "carrier delegation")?;
    let claim = &request.claim;
    if !origin_controls_sender(&context.authority.decision_origin, &claim.performed_by) {
        return Err(invalid_authority(
            "carrier delegation requires the carrier's command authority",
        ));
    }
    if holder_from_entity(&claim.performed_by) == claim.performed_for {
        return Err(invalid_authority(
            "a carrier cannot delegate carrying to itself",
        ));
    }
    let earliest = claim.not_before.map_or(context.simulation_time, |start| {
        start.max(context.simulation_time)
    });
    validate_delegation_claim(
        claim,
        &claim.performed_by,
        &claim.performed_for,
        CARRY_CORRESPONDENCE_CAPABILITY,
        earliest,
    )
    .map_err(|error| invalid_authority(format!("carrier delegation claim is invalid: {error}")))?;
    let accepted = CarrierAuthority {
        delegation: context.command_id,
        claim: request.claim,
    };
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: SimDuration::ZERO,
        packet_type: DELEGATION_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(accepted).map_err(encode_error)?,
        affected: Vec::new(),
    }])
}

fn resolve_correspondence_command_handler(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.ingress == CommandIngress::LegacyDirect {
        return Err(invalid_authority(
            "correspondence recovery requires tracked command ingress",
        ));
    }
    let request: ResolveCorrespondenceRequest = decode(payload, "correspondence recovery")?;
    validate_key(&request.operation_key, "correspondence operation key")?;
    let operation = view
        .typed_domain_record(&correspondence_operation_ref(&request.operation_key))?
        .ok_or_else(|| invalid_record("correspondence recovery names a missing operation"))?
        .decode_payload::<CorrespondenceOperationRecord>()?;
    if context.decision_controller_id.is_none()
        || !origin_controls_sender(&context.authority.decision_origin, &operation.intent.sender)
    {
        return Err(invalid_authority(
            "correspondence recovery requires sender decision authority",
        ));
    }
    validate_recovery_request(&operation, &request, context.simulation_time)?;
    if matches!(
        &request.action,
        CorrespondenceRecoveryAction::RetryDelivery { .. }
    ) {
        // A retry dispatches again, so it must cite the carrier's current
        // delegation; when the retry settles only the admitted claim's
        // window is checked again.
        intent_carrier_authority(view, &operation.intent, context.simulation_time)?;
    }
    if !matches!(
        &request.action,
        CorrespondenceRecoveryAction::ReplanCurrentAttempt
    ) {
        let attempt = operation
            .execution
            .delivery_attempt
            .as_ref()
            .ok_or_else(|| invalid_record("failed correspondence lacks attempt evidence"))?;
        let attempt_ref = typed::<DeliveryAttempt>(attempt.record.clone())?;
        let attempt_record = view
            .typed_domain_record(&attempt_ref)?
            .ok_or_else(|| invalid_record("failed correspondence attempt is missing"))?;
        if attempt_record.version != attempt.version
            || !attempt_record
                .decode_payload::<DeliveryAttempt>()?
                .status
                .is_terminal()
        {
            return Err(invalid_record(
                "correspondence retry/finalize requires the exact terminal attempt",
            ));
        }
        let dispatch_ref = typed::<Dispatch>(operation.dispatch.record.clone())?;
        let dispatch_record = view
            .typed_domain_record(&dispatch_ref)?
            .ok_or_else(|| invalid_record("failed correspondence dispatch is missing"))?;
        if dispatch_record.version != operation.dispatch.version
            || dispatch_record.decode_payload::<Dispatch>()?.status != DispatchStatus::Active
        {
            return Err(invalid_record(
                "correspondence retry/finalize requires the exact active dispatch",
            ));
        }
    }
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: SimDuration::ZERO,
        packet_type: RESOLUTION_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(AdmittedRecovery {
            request,
            accepted_command: context.command_id,
        })
        .map_err(encode_error)?,
        affected: Vec::new(),
    }])
}

fn settle_correspondence_lifecycle(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    let mut seen = BTreeSet::new();
    let mut seen_operations = BTreeSet::new();
    let mut delegations = BTreeMap::new();
    for progress_pass in [false, true] {
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
            if plugin != PLUGIN_NAME
                || packet_type == KNOWLEDGE_INGRESS
                || packet_type == REPORT_INGRESS
            {
                continue;
            }
            if (packet_type == PROGRESS_INGRESS) != progress_pass {
                continue;
            }
            let dedup = canonical_hash(INPUT_HASH_DOMAIN, &(packet_type, payload))?;
            if !seen.insert(dedup) {
                continue;
            }
            match packet_type.as_str() {
                OPPORTUNITY_INGRESS => {
                    settle_opportunity(view, context, *ingress_id, payload, &mut directives)?;
                }
                START_INGRESS => settle_start(
                    view,
                    context,
                    *ingress_id,
                    ingress.cause.as_ref(),
                    payload,
                    &mut seen_operations,
                    &mut directives,
                )?,
                PROGRESS_INGRESS => settle_progress(
                    view,
                    context,
                    ingress.cause.as_ref(),
                    payload,
                    &mut seen_operations,
                    &mut directives,
                )?,
                INCIDENT_INGRESS => {
                    settle_incident(
                        view,
                        context,
                        *ingress_id,
                        payload,
                        &mut seen_operations,
                        &mut directives,
                    )?;
                }
                RESOLUTION_INGRESS => settle_recovery(
                    view,
                    context,
                    *ingress_id,
                    ingress.cause.as_ref(),
                    payload,
                    &mut seen_operations,
                    &mut directives,
                )?,
                DELEGATION_INGRESS => {
                    collect_delegation(ingress.cause.as_ref(), payload, &mut delegations)?;
                }
                _ => {}
            }
        }
    }
    record_delegations(view, delegations, &mut directives)?;
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Collects one accepted delegation for [`record_delegations`], keeping the
/// newest delegation command per (carrier, principal) pair in this boundary.
fn collect_delegation(
    cause: Option<&CauseRef>,
    payload: &Value,
    delegations: &mut BTreeMap<canwu_api::DomainRecordRef, CarrierAuthority>,
) -> Result<(), CanwuError> {
    let accepted: CarrierAuthority = decode(payload, "accepted carrier delegation")?;
    if cause != Some(&CauseRef::Command(accepted.delegation)) {
        return Err(invalid_authority(
            "carrier delegation ingress is not caused by its delegation command",
        ));
    }
    let reference =
        carrier_delegation_ref(&accepted.claim.performed_by, &accepted.claim.performed_for)?
            .into_untyped();
    if delegations
        .get(&reference)
        .is_none_or(|kept| kept.delegation < accepted.delegation)
    {
        delegations.insert(reference, accepted);
    }
    Ok(())
}

/// Persists each pair's newest accepted delegation as its current
/// [`CarrierDelegationRecord`], replacing an older one. An older or repeated
/// delegation than the recorded one changes nothing.
fn record_delegations(
    view: &SimulationView<'_>,
    delegations: BTreeMap<canwu_api::DomainRecordRef, CarrierAuthority>,
    directives: &mut Vec<BoundaryDirective>,
) -> Result<(), CanwuError> {
    for (reference, accepted) in delegations {
        let reference = typed::<CarrierDelegationRecord>(reference)?;
        let mutation = match view.typed_domain_record(&reference)? {
            None => create_typed(&reference, &accepted, Vec::new())?,
            Some(existing) => {
                let current = existing.decode_payload::<CarrierDelegationRecord>()?;
                if current.delegation >= accepted.delegation {
                    continue;
                }
                update_typed(&reference, &accepted, Vec::new(), existing.version)?
            }
        };
        directives.push(mutate(mutation, "Record the carrier's current delegation"));
    }
    Ok(())
}

fn settle_opportunity(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    ingress_id: canwu_api::IngressId,
    payload: &Value,
    directives: &mut Vec<BoundaryDirective>,
) -> Result<(), CanwuError> {
    let mut request: CommunicationOpportunityRequest = decode(payload, "opportunity")?;
    validate_key(&request.operation_key, "opportunity operation key")?;
    validate_key(&request.reason, "opportunity reason")?;
    if request.probability_per_mille > 1_000
        || request.candidates.is_empty()
        || request.candidates.len() > MAX_OPPORTUNITY_CANDIDATES
    {
        return Err(invalid_record(
            "opportunity probability or candidate count is outside limits",
        ));
    }
    request.candidates.sort();
    request.candidates.dedup();
    let input_hash = canonical_hash(OPPORTUNITY_HASH_DOMAIN, &request)?;
    let reference = opportunity_ref(request.operation_key.clone());
    if let Some(existing) = view.typed_domain_record(&reference)? {
        let existing = existing.decode_payload::<CommunicationOpportunityRecord>()?;
        if existing.canonical_input_hash != input_hash {
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                "opportunity key was reused with different input",
            ));
        }
        return Ok(());
    }
    let roll = u16::try_from(view.random_range_for_operation(
        &correspondence_random_stream(),
        EvidenceRef::Ingress(ingress_id),
        "communication_opportunity",
        &request.operation_key,
        RandomOperationTarget::CanonicalKey(input_hash.clone()),
        0,
        1_000,
        "bounded communication opportunity",
    )?)
    .map_err(|_| invalid_record("opportunity roll is not representable"))?;
    let selected = roll < request.probability_per_mille;
    let selected_recipient = if selected {
        let upper = u64::try_from(request.candidates.len())
            .map_err(|_| invalid_record("opportunity candidate count is not representable"))?;
        let index = usize::try_from(view.random_range_for_operation(
            &correspondence_random_stream(),
            EvidenceRef::Ingress(ingress_id),
            "communication_recipient",
            &request.operation_key,
            RandomOperationTarget::CanonicalKey(input_hash.clone()),
            0,
            upper,
            "bounded communication recipient selection",
        )?)
        .map_err(|_| invalid_record("opportunity recipient roll is not representable"))?;
        request.candidates.get(index).cloned()
    } else {
        None
    };
    let status = match (selected, request.automatic) {
        (true, true) => CommunicationOpportunityStatus::SelectedAutomatic,
        (true, false) => CommunicationOpportunityStatus::Offered,
        (false, _) => CommunicationOpportunityStatus::Suppressed,
    };
    let opportunity = CommunicationOpportunity {
        operation_key: request.operation_key,
        canonical_input_hash: input_hash,
        sender: request.sender,
        candidate_digest: canonical_hash(OPPORTUNITY_HASH_DOMAIN, &request.candidates)?,
        candidates: request.candidates,
        reason: request.reason,
        probability_per_mille: request.probability_per_mille,
        roll_per_mille: roll,
        automatic: request.automatic,
        selected_recipient,
        status,
        evaluated_at: context.at,
        evidence: vec![EvidenceRef::Ingress(ingress_id)],
    };
    directives.push(mutate(
        create_typed(
            &reference,
            &opportunity,
            opportunity_references(&opportunity),
        )?,
        "Persist bounded communication opportunity",
    ));
    Ok(())
}

fn settle_start(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    ingress_id: canwu_api::IngressId,
    cause: Option<&CauseRef>,
    payload: &Value,
    seen_operations: &mut BTreeSet<String>,
    directives: &mut Vec<BoundaryDirective>,
) -> Result<(), CanwuError> {
    let admitted: AdmittedStart = decode(payload, "admitted correspondence")?;
    if !claim_operation_update(seen_operations, &admitted.request.operation_key) {
        return Err(invalid_record(
            "one correspondence cannot receive multiple state updates in the same boundary",
        ));
    }
    if cause != Some(&CauseRef::Command(admitted.accepted_command)) {
        return Err(invalid_authority(
            "correspondence start ingress is not caused by its accepted command",
        ));
    }
    let input_hash = canonical_hash(INPUT_HASH_DOMAIN, &(&admitted.request, &admitted.authority))?;
    let reference = correspondence_operation_ref(admitted.request.operation_key.clone());
    if let Some(existing) = view.typed_domain_record(&reference)? {
        let existing = existing.decode_payload::<CorrespondenceOperationRecord>()?;
        if existing.canonical_input_hash != input_hash {
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                "correspondence operation key was reused with different input",
            ));
        }
        return Ok(());
    }
    // The dispatch happens now, so the delegation admitted with the command
    // must still cover this boundary. The record is not read again: a newer
    // delegation recorded since admission does not revoke an admitted start.
    admitted_authority_covers(
        &admitted.request.sender,
        &admitted.request.carrier,
        admitted.carrier_authority.as_ref(),
        context.at,
    )?;
    let carrier_authority = admitted.carrier_authority.clone();
    let (snapshot, address) = carrier_planning_snapshot(
        view,
        &admitted.request.carrier,
        &admitted.request.recipient,
        context.at,
    )?;
    let route_plan = plan_route(
        &snapshot,
        &RoutingRequest {
            origin: admitted.request.origin.clone(),
            destination: address.destination.clone(),
            departure_at: context.at,
            policy: admitted.request.routing_policy.clone(),
        },
    )
    .map_err(|error| invalid_record(error.to_string()))?;
    if route_plan.legs.is_empty() {
        return Err(invalid_record(
            "addressed correspondence requires an explicit final-mile leg",
        ));
    }
    let dispatch = typed::<Dispatch>(admitted.request.prepared_dispatch.record.clone())?;
    let (activation, attempt_ref) =
        activation_operation(&admitted.request, dispatch.clone(), context.at)?;
    let execution = TransportExecution::new(admitted.request.execution_id, None);
    let intent = CorrespondenceIntent {
        sender: admitted.request.sender.clone(),
        recipient: admitted.request.recipient.clone(),
        carrier: admitted.request.carrier.clone(),
        channel_profile: admitted.request.channel_profile.clone(),
        origin: admitted.request.origin.clone(),
        accepted_at: context.at,
        due_at: admitted.request.due_at,
        routing_policy: admitted.request.routing_policy.clone(),
        capacity_admission: admitted.request.capacity_admission,
        prepared_dispatch: admitted.request.prepared_dispatch.clone(),
        authority: admitted.authority.clone(),
        accepted_command: admitted.accepted_command,
        carrier_authority,
    };
    let planning_snapshot_digest = snapshot.digest();
    let planning_evidence = CorrespondencePlanningEvidence {
        transport_execution: admitted.request.execution_id,
        itinerary_revision: canwu_api::ItineraryRevisionId(1),
        planned_at: context.at,
        read_cut: address.read_cut.clone(),
        address_source_record: address.source_record,
        planning_snapshot_digest: planning_snapshot_digest.clone(),
        excluded_connections: Vec::new(),
        evidence: vec![EvidenceRef::Ingress(ingress_id)],
    };
    let operation = CorrespondenceOperation {
        operation_key: admitted.request.operation_key,
        canonical_input_hash: input_hash,
        intent,
        address,
        planning_snapshot_digest,
        planning_history: vec![planning_evidence],
        route_plan,
        execution,
        dispatch: admitted.request.prepared_dispatch.clone(),
        current_attempt_number: 1,
        current_attempt_prepared_at: context.at,
        current_due_at: admitted.request.due_at,
        status: CorrespondenceStatus::AwaitingInformationActivation,
        pending_information: Some(PendingInformationOperation {
            step: InformationSagaStep::ActivateDispatch,
            id: activation.id.clone(),
            expected_status: InformationOperationStatus::Completed,
        }),
        delivery_attempt_operation: admitted.request.delivery_attempt_operation,
        recovery_history: Vec::new(),
        next_sequence: 0,
        incidents: BTreeMap::new(),
        last_error: None,
    };
    let mut references = operation_references(&operation, Some(dispatch), None);
    if let CorrespondenceAuthority::Automatic { opportunity } = &operation.intent.authority {
        references.push(DomainReference::from_typed(
            "opportunity",
            opportunity.clone(),
        ));
    }
    references.sort();
    if let CorrespondenceAuthority::Automatic { opportunity } = &operation.intent.authority {
        let opportunity_record = view
            .typed_domain_record(opportunity)?
            .ok_or_else(|| invalid_authority("automatic opportunity is missing at settlement"))?;
        let mut opportunity_payload =
            opportunity_record.decode_payload::<CommunicationOpportunityRecord>()?;
        if opportunity_payload.status != CommunicationOpportunityStatus::SelectedAutomatic
            || opportunity_payload.operation_key != operation.operation_key
            || opportunity_payload.selected_recipient.as_ref() != Some(&operation.intent.recipient)
        {
            return Err(invalid_authority(
                "automatic opportunity was consumed or no longer authorizes this correspondence",
            ));
        }
        opportunity_payload.status = CommunicationOpportunityStatus::Consumed;
        directives.push(mutate(
            update_typed(
                opportunity,
                &opportunity_payload,
                opportunity_references(&opportunity_payload),
                opportunity_record.version,
            )?,
            "Consume automatic communication opportunity",
        ));
    }
    directives.push(mutate(
        create_typed(&reference, &operation, references)?,
        "Persist correspondence intent and route plan",
    ));
    directives.push(schedule_information(&activation)?);
    directives.push(schedule_progress(
        &operation.operation_key,
        0,
        ProgressAction::ReconcileInformation,
        SimDuration::ZERO,
    )?);
    let _ = attempt_ref;
    let _ = ingress_id;
    Ok(())
}

fn settle_progress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    cause: Option<&CauseRef>,
    payload: &Value,
    seen_operations: &mut BTreeSet<String>,
    directives: &mut Vec<BoundaryDirective>,
) -> Result<(), CanwuError> {
    if !matches!(cause, Some(CauseRef::Boundary(_))) {
        return Err(invalid_authority(
            "correspondence progress requires boundary-generated ingress",
        ));
    }
    let request: ProgressRequest = decode(payload, "correspondence progress")?;
    let reference = correspondence_operation_ref(request.operation_key.clone());
    let record = view
        .typed_domain_record(&reference)?
        .ok_or_else(|| invalid_record("progress names a missing correspondence"))?;
    let mut operation = record.decode_payload::<CorrespondenceOperationRecord>()?;
    if request.sequence < operation.next_sequence || operation.status.is_terminal() {
        return Ok(());
    }
    if request.sequence != operation.next_sequence {
        return Err(invalid_record("correspondence progress sequence has a gap"));
    }
    if !claim_operation_update(seen_operations, &request.operation_key) {
        return Ok(());
    }
    match request.action {
        ProgressAction::ReconcileInformation => {
            if operation.pending_information.is_none() {
                return Err(invalid_record(
                    "information reconciliation requires a pending operation",
                ));
            }
            reconcile_information(view, context, record, &mut operation, directives)
        }
        ProgressAction::StartLeg => {
            if operation.status != CorrespondenceStatus::Scheduled {
                return Err(invalid_record(
                    "leg start requires scheduled correspondence",
                ));
            }
            let planned = current_route_leg(&operation)?.planned_departure_at;
            if context.at < planned {
                return Err(invalid_record("leg cannot start before planned departure"));
            }
            start_leg(context, record, &mut operation, directives)
        }
        ProgressAction::CompleteLeg => {
            if operation.status != CorrespondenceStatus::InTransit {
                return Err(invalid_record(
                    "leg completion requires in-transit correspondence",
                ));
            }
            let planned = current_route_leg(&operation)?.planned_arrival_at;
            if context.at < planned {
                return Err(invalid_record("leg cannot complete before planned arrival"));
            }
            complete_leg(context, record, &mut operation, directives)
        }
    }
}

fn reconcile_information(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    record: &DomainRecord,
    operation: &mut CorrespondenceOperation,
    directives: &mut Vec<BoundaryDirective>,
) -> Result<(), CanwuError> {
    let pending = operation
        .pending_information
        .clone()
        .ok_or_else(|| invalid_record("reconciliation has no pending information operation"))?;
    let info_ref = derive_operation_record_ref(&pending.id);
    let Some(info_record) = view.typed_domain_record(&info_ref)? else {
        directives.push(schedule_progress(
            &operation.operation_key,
            operation.next_sequence,
            ProgressAction::ReconcileInformation,
            SimDuration::ZERO,
        )?);
        return Ok(());
    };
    let info = info_record.decode_payload::<InformationOperationRecord>()?;
    if !info.status.is_terminal() {
        directives.push(schedule_progress(
            &operation.operation_key,
            operation.next_sequence,
            ProgressAction::ReconcileInformation,
            SimDuration::ZERO,
        )?);
        return Ok(());
    }
    if info.status == InformationOperationStatus::Rejected {
        operation.status = CorrespondenceStatus::CompensationPending;
        operation.last_error = info.rejection_code;
        operation.pending_information = None;
        directives.push(update_operation(record, operation)?);
        return Ok(());
    }
    if info.status != pending.expected_status {
        return Err(invalid_record(
            "information operation reached an unexpected terminal status",
        ));
    }
    let attempt_ref = delivery_attempt_ref(operation)?;
    match pending.step {
        InformationSagaStep::ActivateDispatch | InformationSagaStep::BeginRetry => {
            let attempt_evidence = info
                .domain_result_evidence
                .iter()
                .find(|evidence| evidence.record == *attempt_ref.as_untyped())
                .cloned()
                .ok_or_else(|| {
                    invalid_record("information result lacks delivery-attempt evidence")
                })?;
            if pending.step == InformationSagaStep::ActivateDispatch {
                operation.dispatch = info
                    .domain_result_evidence
                    .iter()
                    .find(|evidence| evidence.record == operation.dispatch.record)
                    .cloned()
                    .ok_or_else(|| invalid_record("activation result lacks dispatch evidence"))?;
            }
            let mut execution =
                TransportExecution::new(operation.execution.id, Some(attempt_evidence.clone()));
            execution
                .install_initial_itinerary(canwu_api::ItineraryRevision {
                    id: canwu_api::ItineraryRevisionId(1),
                    predecessor: None,
                    plan: operation.route_plan.clone(),
                    planned_at: context.at,
                    valid_from: context.at,
                    reason: canwu_api::ItineraryRevisionReason::Initial,
                    superseded_at: None,
                    evidence: info
                        .domain_result_evidence
                        .iter()
                        .cloned()
                        .map(EvidenceRef::DomainRecordVersion)
                        .collect(),
                })
                .map_err(transport_error)?;
            operation.execution = execution;
            let envelope = attempt_transition_operation(
                operation,
                &attempt_evidence,
                DeliveryAttemptStatus::InTransit,
                context.at,
                "mark-in-transit",
            )?;
            operation.pending_information = Some(PendingInformationOperation {
                step: InformationSagaStep::MarkInTransit,
                id: envelope.id.clone(),
                expected_status: InformationOperationStatus::Completed,
            });
            operation.status = CorrespondenceStatus::AwaitingDispatch;
            advance_sequence(operation)?;
            directives.push(update_operation(record, operation)?);
            directives.push(schedule_information(&envelope)?);
            directives.push(schedule_progress(
                &operation.operation_key,
                operation.next_sequence,
                ProgressAction::ReconcileInformation,
                SimDuration::ZERO,
            )?);
        }
        InformationSagaStep::MarkInTransit => {
            let attempt_evidence = info
                .domain_result_evidence
                .iter()
                .find(|evidence| evidence.record == *attempt_ref.as_untyped())
                .cloned()
                .ok_or_else(|| {
                    invalid_record("information result lacks delivery-attempt evidence")
                })?;
            operation.execution.delivery_attempt = Some(attempt_evidence.clone());
            operation
                .execution
                .begin_saga(
                    attempt_evidence,
                    canwu_api::delivery_completion_operation_key(
                        operation.execution.id,
                        canwu_api::ItineraryRevisionId(1),
                        info.domain_result_evidence
                            .iter()
                            .find(|evidence| evidence.record == *attempt_ref.as_untyped())
                            .map_or(1, |evidence| evidence.version),
                    ),
                )
                .map_err(transport_error)?;
            operation.pending_information = None;
            operation.status = CorrespondenceStatus::Scheduled;
            advance_sequence(operation)?;
            let delay = delay_until(
                context.at,
                current_route_leg(operation)?.planned_departure_at,
            )?;
            directives.push(update_operation(record, operation)?);
            directives.push(schedule_progress(
                &operation.operation_key,
                operation.next_sequence,
                ProgressAction::StartLeg,
                delay,
            )?);
        }
        InformationSagaStep::CompleteDelivery => {
            let attempt_evidence = info
                .domain_result_evidence
                .iter()
                .find(|evidence| evidence.record == *attempt_ref.as_untyped())
                .cloned()
                .ok_or_else(|| {
                    invalid_record("information result lacks delivery-attempt evidence")
                })?;
            let attempt_record = view
                .typed_domain_record(&attempt_ref)?
                .ok_or_else(|| invalid_record("completed attempt record is missing"))?;
            let attempt = attempt_record.decode_payload::<DeliveryAttempt>()?;
            if !attempt.status.is_terminal() {
                return Err(invalid_record(
                    "completed delivery operation left the attempt non-terminal",
                ));
            }
            operation.execution.delivery_attempt = Some(attempt_evidence.clone());
            if let Some(saga) = operation.execution.saga.as_mut() {
                saga.delivery_attempt = attempt_evidence.clone();
                saga.expected_attempt_version = attempt_evidence.version;
            }
            let delivered = attempt.status == DeliveryAttemptStatus::Delivered;
            operation
                .execution
                .reconcile_information(if delivered {
                    ReconciliationOutcome::Success
                } else {
                    ReconciliationOutcome::Failure {
                        error: "delivery attempt failed".to_owned(),
                    }
                })
                .map_err(transport_error)?;
            if !delivered {
                operation.pending_information = None;
                operation.status = if attempt.status == DeliveryAttemptStatus::Failed
                    && attempt.completed_at.is_some_and(|at| at > attempt.due_at)
                {
                    CorrespondenceStatus::DeadlineMissed
                } else {
                    CorrespondenceStatus::Failed
                };
                advance_sequence(operation)?;
                if let Some(report) = seizure_report_directive(operation, &pending.id)? {
                    directives.push(report);
                }
                directives.push(update_operation(record, operation)?);
                return Ok(());
            }
            let dispatch_ref = typed::<Dispatch>(operation.dispatch.record.clone())?;
            let dispatch_record = view
                .typed_domain_record(&dispatch_ref)?
                .ok_or_else(|| invalid_record("active dispatch is missing"))?;
            if dispatch_record.version != operation.dispatch.version {
                return Err(invalid_record("active dispatch evidence is stale"));
            }
            let dispatch = dispatch_record.decode_payload::<Dispatch>()?;
            let envelope = dispatch_completion_operation(operation, &dispatch, context.at)?;
            operation.pending_information = Some(PendingInformationOperation {
                step: InformationSagaStep::CompleteDispatch,
                id: envelope.id.clone(),
                expected_status: InformationOperationStatus::Completed,
            });
            advance_sequence(operation)?;
            directives.push(update_operation(record, operation)?);
            directives.push(schedule_information(&envelope)?);
            directives.push(schedule_progress(
                &operation.operation_key,
                operation.next_sequence,
                ProgressAction::ReconcileInformation,
                SimDuration::ZERO,
            )?);
        }
        InformationSagaStep::CompleteDispatch => {
            operation.dispatch = info
                .domain_result_evidence
                .iter()
                .find(|evidence| evidence.record == operation.dispatch.record)
                .cloned()
                .ok_or_else(|| invalid_record("completion result lacks dispatch evidence"))?;
            let attempt_record = view
                .typed_domain_record(&attempt_ref)?
                .ok_or_else(|| invalid_record("completed attempt record is missing"))?;
            let attempt = attempt_record.decode_payload::<DeliveryAttempt>()?;
            operation.pending_information = None;
            operation.status = if attempt.status == DeliveryAttemptStatus::Delivered {
                CorrespondenceStatus::Settled
            } else if attempt.status == DeliveryAttemptStatus::Failed
                && attempt.completed_at.is_some_and(|at| at > attempt.due_at)
            {
                CorrespondenceStatus::DeadlineMissed
            } else {
                CorrespondenceStatus::Failed
            };
            advance_sequence(operation)?;
            directives.push(update_operation(record, operation)?);
        }
    }
    Ok(())
}

fn settle_recovery(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    ingress_id: canwu_api::IngressId,
    cause: Option<&CauseRef>,
    payload: &Value,
    seen_operations: &mut BTreeSet<String>,
    directives: &mut Vec<BoundaryDirective>,
) -> Result<(), CanwuError> {
    let admitted: AdmittedRecovery = decode(payload, "admitted correspondence recovery")?;
    if !claim_operation_update(seen_operations, &admitted.request.operation_key) {
        return Err(invalid_record(
            "one correspondence cannot receive multiple state updates in the same boundary",
        ));
    }
    if cause != Some(&CauseRef::Command(admitted.accepted_command)) {
        return Err(invalid_authority(
            "correspondence recovery ingress is not caused by its accepted command",
        ));
    }
    let reference = correspondence_operation_ref(&admitted.request.operation_key);
    let record = view
        .typed_domain_record(&reference)?
        .ok_or_else(|| invalid_record("correspondence recovery names a missing operation"))?;
    let mut operation = record.decode_payload::<CorrespondenceOperationRecord>()?;
    let recovery_hash = canonical_hash(RECOVERY_HASH_DOMAIN, &admitted.request)?;
    if let Some(existing) = operation
        .recovery_history
        .iter()
        .find(|recovery| recovery.accepted_command == admitted.accepted_command)
    {
        if existing.canonical_input_hash != recovery_hash {
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                "recovery command was reused with different input",
            ));
        }
        return Ok(());
    }
    validate_recovery_request(&operation, &admitted.request, context.at)?;
    operation.recovery_history.push(CorrespondenceRecovery {
        accepted_command: admitted.accepted_command,
        accepted_at: context.at,
        canonical_input_hash: recovery_hash,
        action: admitted.request.action.clone(),
    });
    match &admitted.request.action {
        CorrespondenceRecoveryAction::ReplanCurrentAttempt => {
            let excluded = active_disaster_connections(&operation);
            install_replanned_route(
                view,
                context,
                &mut operation,
                &excluded,
                canwu_api::ItineraryRevisionReason::KnowledgeUpdate {
                    explanation: "explicit correspondence recovery".to_owned(),
                },
                vec![EvidenceRef::Ingress(ingress_id)],
                directives,
            )?;
        }
        action @ CorrespondenceRecoveryAction::RetryDelivery { .. } => {
            begin_retry(
                view,
                context,
                ingress_id,
                &mut operation,
                action,
                directives,
            )?;
        }
        CorrespondenceRecoveryAction::FinalizeDispatch => {
            let dispatch_ref = typed::<Dispatch>(operation.dispatch.record.clone())?;
            let dispatch_record = view
                .typed_domain_record(&dispatch_ref)?
                .ok_or_else(|| invalid_record("failed correspondence dispatch is missing"))?;
            if dispatch_record.version != operation.dispatch.version {
                return Err(invalid_record(
                    "failed correspondence dispatch evidence is stale",
                ));
            }
            let dispatch = dispatch_record.decode_payload::<Dispatch>()?;
            let envelope = dispatch_completion_operation(&operation, &dispatch, context.at)?;
            operation.pending_information = Some(PendingInformationOperation {
                step: InformationSagaStep::CompleteDispatch,
                id: envelope.id.clone(),
                expected_status: InformationOperationStatus::Completed,
            });
            operation.status = CorrespondenceStatus::AwaitingInformationCompletion;
            advance_sequence(&mut operation)?;
            directives.push(schedule_information(&envelope)?);
            directives.push(schedule_progress(
                &operation.operation_key,
                operation.next_sequence,
                ProgressAction::ReconcileInformation,
                SimDuration::ZERO,
            )?);
        }
    }
    directives.push(update_operation(record, &operation)?);
    Ok(())
}

fn start_leg(
    context: &BoundaryContext,
    record: &DomainRecord,
    operation: &mut CorrespondenceOperation,
    directives: &mut Vec<BoundaryDirective>,
) -> Result<(), CanwuError> {
    let leg = current_route_leg(operation)?.clone();
    operation
        .execution
        .start_current_leg(context.at)
        .map_err(transport_error)?;
    operation.status = CorrespondenceStatus::InTransit;
    advance_sequence(operation)?;
    directives.push(update_operation(record, operation)?);
    directives.push(schedule_progress(
        &operation.operation_key,
        operation.next_sequence,
        ProgressAction::CompleteLeg,
        delay_until(context.at, leg.planned_arrival_at)?,
    )?);
    Ok(())
}

fn complete_leg(
    context: &BoundaryContext,
    record: &DomainRecord,
    operation: &mut CorrespondenceOperation,
    directives: &mut Vec<BoundaryDirective>,
) -> Result<(), CanwuError> {
    let leg = current_route_leg(operation)?.clone();
    let completed_leg_index = operation.execution.current_leg_index;
    let final_leg = operation
        .execution
        .complete_current_leg(context.at, leg.to.as_str().to_owned())
        .map_err(transport_error)?;
    advance_sequence(operation)?;
    if final_leg {
        let attempt = operation
            .execution
            .delivery_attempt
            .clone()
            .ok_or_else(|| invalid_record("arrival lacks a delivery attempt"))?;
        let status = if context.at <= operation.current_due_at {
            DeliveryAttemptStatus::Delivered
        } else {
            DeliveryAttemptStatus::Failed
        };
        let envelope = attempt_transition_operation(
            operation,
            &attempt,
            status,
            context.at,
            "complete-delivery",
        )?;
        operation.pending_information = Some(PendingInformationOperation {
            step: InformationSagaStep::CompleteDelivery,
            id: envelope.id.clone(),
            expected_status: InformationOperationStatus::Completed,
        });
        operation.status = CorrespondenceStatus::AwaitingInformationCompletion;
        directives.push(update_operation(record, operation)?);
        directives.push(schedule_information(&envelope)?);
        directives.push(schedule_progress(
            &operation.operation_key,
            operation.next_sequence,
            ProgressAction::ReconcileInformation,
            SimDuration::ZERO,
        )?);
    } else {
        let next_leg = current_route_leg(operation)?.clone();
        let active_revision = operation
            .execution
            .active_itinerary_revision
            .ok_or_else(|| invalid_record("handoff has no active itinerary"))?;
        let from_leg = operation
            .execution
            .legs
            .iter()
            .find(|candidate| {
                candidate.itinerary_revision == active_revision
                    && candidate.leg_index == completed_leg_index
            })
            .ok_or_else(|| invalid_record("completed transport leg is missing"))?
            .id;
        let to_leg = operation
            .execution
            .legs
            .iter()
            .find(|candidate| {
                candidate.itinerary_revision == active_revision
                    && candidate.leg_index == operation.execution.current_leg_index
            })
            .ok_or_else(|| invalid_record("next transport leg is missing"))?
            .id;
        let handoff_id = u64::try_from(operation.execution.handoffs.len())
            .map_err(|_| invalid_record("handoff count is not representable"))?
            .checked_add(1)
            .ok_or_else(|| invalid_record("handoff identity overflow"))?;
        let attempt = operation
            .execution
            .delivery_attempt
            .clone()
            .ok_or_else(|| invalid_record("handoff lacks delivery-attempt evidence"))?;
        operation
            .execution
            .record_handoff(canwu_api::Handoff {
                id: canwu_api::HandoffId(handoff_id),
                from_leg,
                to_leg,
                from_custodian: format!("connection:{}", leg.connection.as_str()),
                to_custodian: format!("connection:{}", next_leg.connection.as_str()),
                at: context.at,
                location: leg.to.as_str().to_owned(),
                evidence: vec![EvidenceRef::DomainRecordVersion(attempt)],
                kind: canwu_api::HandoffKind::Planned,
            })
            .map_err(transport_error)?;
        operation.status = CorrespondenceStatus::Scheduled;
        let delay = delay_until(context.at, next_leg.planned_departure_at)?;
        directives.push(update_operation(record, operation)?);
        directives.push(schedule_progress(
            &operation.operation_key,
            operation.next_sequence,
            ProgressAction::StartLeg,
            delay,
        )?);
    }
    Ok(())
}

fn settle_incident(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    ingress_id: canwu_api::IngressId,
    payload: &Value,
    seen_operations: &mut BTreeSet<String>,
    directives: &mut Vec<BoundaryDirective>,
) -> Result<(), CanwuError> {
    let mut request: CorrespondenceIncidentRequest = decode(payload, "correspondence incident")?;
    if !claim_operation_update(seen_operations, &request.operation_key) {
        return Err(invalid_record(
            "one correspondence cannot receive multiple state updates in the same boundary",
        ));
    }
    validate_key(&request.operation_key, "correspondence operation key")?;
    validate_key(&request.incident_key, "incident key")?;
    if request.probability_per_mille > 1_000 {
        return Err(invalid_record(
            "incident probability exceeds one thousand per mille",
        ));
    }
    if let CorrespondenceIncidentKind::Disaster {
        blocked_connections,
        ..
    } = &mut request.kind
    {
        blocked_connections.sort();
        blocked_connections.dedup();
    }
    let input_hash = canonical_hash(INCIDENT_HASH_DOMAIN, &request)?;
    let reference = correspondence_operation_ref(request.operation_key.clone());
    let record = view
        .typed_domain_record(&reference)?
        .ok_or_else(|| invalid_record("incident names a missing correspondence"))?;
    let mut operation = record.decode_payload::<CorrespondenceOperationRecord>()?;
    if operation.status.is_terminal() {
        return Err(invalid_record(
            "terminal correspondence cannot accept another incident",
        ));
    }
    if let Some(existing) = operation.incidents.get(&request.incident_key) {
        if existing.kind != request.kind
            || existing.probability_per_mille != request.probability_per_mille
        {
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                "incident key was reused with different input",
            ));
        }
        return Ok(());
    }
    validate_seizure_citation(&operation, &request.kind)?;
    let suppressed_reason = incident_suppression_reason(&operation, &request.kind);
    let roll = u16::try_from(view.random_range_for_operation(
        &correspondence_random_stream(),
        EvidenceRef::Ingress(ingress_id),
        "correspondence_incident",
        &format!("{}::{}", request.operation_key, request.incident_key),
        RandomOperationTarget::CanonicalKey(input_hash),
        0,
        1_000,
        "correspondence incident resolution",
    )?)
    .map_err(|_| invalid_record("incident roll is not representable"))?;
    let triggered = suppressed_reason.is_none() && roll < request.probability_per_mille;
    let mut information_operation = None;
    if triggered {
        match &request.kind {
            CorrespondenceIncidentKind::Interception {
                intercepted_by,
                extent_per_mille,
            } => {
                if *extent_per_mille > 1_000 {
                    return Err(invalid_record(
                        "interception extent exceeds one thousand per mille",
                    ));
                }
                let envelope = interception_operation(
                    view,
                    &operation,
                    &request.incident_key,
                    intercepted_by,
                    *extent_per_mille,
                    context.at,
                )?;
                information_operation = Some(envelope.id.clone());
                directives.push(schedule_information(&envelope)?);
            }
            CorrespondenceIncidentKind::Disaster {
                blocked_connections,
                explanation,
            } => {
                operation
                    .execution
                    .fail_current_leg(explanation.clone(), context.at)
                    .map_err(transport_error)?;
                let mut excluded = active_disaster_connections(&operation);
                excluded.extend(blocked_connections.iter().cloned());
                excluded.sort();
                excluded.dedup();
                install_replanned_route(
                    view,
                    context,
                    &mut operation,
                    &excluded,
                    canwu_api::ItineraryRevisionReason::Disaster {
                        explanation: explanation.clone(),
                    },
                    vec![EvidenceRef::Ingress(ingress_id)],
                    directives,
                )?;
            }
            CorrespondenceIncidentKind::CarrierSeized {
                seized_by,
                custody_handoff,
            } => {
                let envelope = seize_carrier(
                    context,
                    ingress_id,
                    &mut operation,
                    seized_by,
                    *custody_handoff,
                )?;
                information_operation = Some(envelope.id.clone());
                directives.push(schedule_information(&envelope)?);
                directives.push(schedule_progress(
                    &operation.operation_key,
                    operation.next_sequence,
                    ProgressAction::ReconcileInformation,
                    SimDuration::ZERO,
                )?);
            }
        }
    }
    operation.incidents.insert(
        request.incident_key.clone(),
        CorrespondenceIncident {
            incident_key: request.incident_key,
            at: context.at,
            probability_per_mille: request.probability_per_mille,
            roll_per_mille: roll,
            triggered,
            suppressed_reason,
            kind: request.kind,
            evidence: vec![EvidenceRef::Ingress(ingress_id)],
            information_operation,
        },
    );
    directives.push(update_operation(record, &operation)?);
    Ok(())
}

fn incident_suppression_reason(
    operation: &CorrespondenceOperation,
    kind: &CorrespondenceIncidentKind,
) -> Option<String> {
    // A carrier waiting for a route still holds the packet and can be seized;
    // other incidents need a moving or scheduled attempt.
    let waiting_seizure = operation.status == CorrespondenceStatus::WaitingForRoute
        && matches!(kind, CorrespondenceIncidentKind::CarrierSeized { .. });
    if !waiting_seizure
        && !matches!(
            operation.status,
            CorrespondenceStatus::Scheduled | CorrespondenceStatus::InTransit
        )
    {
        return Some("incident is not applicable outside an active transport attempt".to_owned());
    }
    if operation.pending_information.is_some() {
        return Some(
            "incident is suppressed while an information transition is pending".to_owned(),
        );
    }
    if current_route_leg(operation).is_err() {
        return Some("incident is suppressed because no current transport leg exists".to_owned());
    }
    if matches!(kind, CorrespondenceIncidentKind::Disaster { .. })
        && !matches!(
            operation.execution.state,
            TransportExecutionState::Planning
                | TransportExecutionState::Ready
                | TransportExecutionState::Executing
                | TransportExecutionState::ReplanPending
        )
    {
        return Some("disaster is suppressed outside a replan-capable transport state".to_owned());
    }
    seizure_citation_mismatch(operation, kind)
}

fn active_disaster_connections(
    operation: &CorrespondenceOperation,
) -> Vec<canwu_api::RoutingConnectionRef> {
    let mut excluded = operation
        .incidents
        .values()
        .filter(|incident| incident.triggered)
        .filter_map(|incident| match &incident.kind {
            CorrespondenceIncidentKind::Disaster {
                blocked_connections,
                ..
            } => Some(blocked_connections.as_slice()),
            CorrespondenceIncidentKind::Interception { .. }
            | CorrespondenceIncidentKind::CarrierSeized { .. } => None,
        })
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    excluded.sort();
    excluded.dedup();
    excluded
}

/// Rejects a malformed seizure: a zero custody handoff identity, a malformed
/// seizing identity, or the carrier seizing itself. A citation that collides
/// with a recorded handoff depends on execution progress, so it suppresses
/// the incident instead (see [`incident_suppression_reason`]).
fn validate_seizure_citation(
    operation: &CorrespondenceOperation,
    kind: &CorrespondenceIncidentKind,
) -> Result<(), CanwuError> {
    let CorrespondenceIncidentKind::CarrierSeized {
        seized_by,
        custody_handoff,
    } = kind
    else {
        return Ok(());
    };
    if custody_handoff.0 == 0 {
        return Err(invalid_record("custody handoff identity must be nonzero"));
    }
    if let EntityRef::Domain(record) = seized_by
        && (record.kind.namespace.trim().is_empty()
            || record.kind.name.trim().is_empty()
            || record.id.trim().is_empty())
    {
        return Err(invalid_record(
            "carrier seizure requires a well-formed seizing identity",
        ));
    }
    if *seized_by == holder_entity(&operation.intent.carrier) {
        return Err(invalid_record("a carrier cannot seize its own custody"));
    }
    Ok(())
}

/// The suppression reason of a seizure whose cited custody handoff identity
/// is already recorded in the current execution. Every recorded handoff is
/// a different custody transfer, so the cited one is never this seizure's.
fn seizure_citation_mismatch(
    operation: &CorrespondenceOperation,
    kind: &CorrespondenceIncidentKind,
) -> Option<String> {
    let CorrespondenceIncidentKind::CarrierSeized {
        custody_handoff, ..
    } = kind
    else {
        return None;
    };
    operation
        .execution
        .handoffs
        .iter()
        .any(|handoff| handoff.id == *custody_handoff)
        .then(|| "cited custody handoff identity is already recorded".to_owned())
}

/// Applies a triggered carrier seizure: fails the current leg unless a
/// disaster already failed it, records the terminal seizure handoff under
/// the cited identity, and closes the delivery attempt as failed through the
/// information lifecycle. The report is scheduled once that closure is
/// confirmed.
fn seize_carrier(
    context: &BoundaryContext,
    ingress_id: canwu_api::IngressId,
    operation: &mut CorrespondenceOperation,
    seized_by: &EntityRef,
    custody_handoff: HandoffId,
) -> Result<InformationOperationEnvelope, CanwuError> {
    let leg_plan = current_route_leg(operation)?.clone();
    let active = operation
        .execution
        .active_itinerary_revision
        .ok_or_else(|| invalid_record("seizure has no active itinerary"))?;
    let current_leg_index = operation.execution.current_leg_index;
    let leg = operation
        .execution
        .legs
        .iter()
        .find(|leg| leg.itinerary_revision == active && leg.leg_index == current_leg_index)
        .cloned()
        .ok_or_else(|| invalid_record("seized transport leg is missing"))?;
    let location = if leg.status == LegExecutionStatus::Departed {
        leg_plan.connection.as_str().to_owned()
    } else {
        leg_plan.from.as_str().to_owned()
    };
    if leg.status != LegExecutionStatus::Failed {
        operation
            .execution
            .fail_current_leg(SEIZURE_FAILURE_REASON.to_owned(), context.at)
            .map_err(transport_error)?;
    }
    let attempt = operation
        .execution
        .delivery_attempt
        .clone()
        .ok_or_else(|| invalid_record("seizure lacks delivery-attempt evidence"))?;
    let mut evidence = vec![
        EvidenceRef::Ingress(ingress_id),
        EvidenceRef::DomainRecordVersion(attempt.clone()),
    ];
    evidence.sort();
    operation
        .execution
        .record_handoff(canwu_api::Handoff {
            id: custody_handoff,
            from_leg: leg.id,
            to_leg: leg.id,
            from_custodian: format!("connection:{}", leg_plan.connection.as_str()),
            to_custodian: format!("seizure:{seized_by}"),
            at: context.at,
            location,
            evidence,
            kind: HandoffKind::Seizure {
                by: seized_by.clone(),
            },
        })
        .map_err(transport_error)?;
    let envelope = attempt_transition_operation(
        operation,
        &attempt,
        DeliveryAttemptStatus::Failed,
        context.at,
        "carrier-seized",
    )?;
    operation.pending_information = Some(PendingInformationOperation {
        step: InformationSagaStep::CompleteDelivery,
        id: envelope.id.clone(),
        expected_status: InformationOperationStatus::Completed,
    });
    operation.status = CorrespondenceStatus::AwaitingInformationCompletion;
    operation.last_error = None;
    advance_sequence(operation)?;
    Ok(envelope)
}

/// Schedules the attempt report when `closing` is the information operation
/// through which a triggered carrier seizure closed the current attempt.
fn seizure_report_directive(
    operation: &CorrespondenceOperation,
    closing: &InformationOperationId,
) -> Result<Option<BoundaryDirective>, CanwuError> {
    let Some(request) = operation.incidents.values().find_map(|incident| {
        (matches!(
            incident.kind,
            CorrespondenceIncidentKind::CarrierSeized { .. }
        ) && incident.triggered
            && incident.information_operation.as_ref() == Some(closing))
        .then(|| AttemptReportRequest {
            operation_key: operation.operation_key.clone(),
            incident_key: incident.incident_key.clone(),
        })
    }) else {
        return Ok(None);
    };
    Ok(Some(BoundaryDirective::ScheduleIngress {
        after: SimDuration::ZERO,
        packet_type: REPORT_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(request).map_err(encode_error)?,
        affected: Vec::new(),
    }))
}

fn install_replanned_route(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    operation: &mut CorrespondenceOperation,
    excluded_connections: &[canwu_api::RoutingConnectionRef],
    reason: canwu_api::ItineraryRevisionReason,
    evidence: Vec<EvidenceRef>,
    directives: &mut Vec<BoundaryDirective>,
) -> Result<(), CanwuError> {
    let previous_execution_leg = operation
        .execution
        .legs
        .iter()
        .find(|leg| {
            Some(leg.itinerary_revision) == operation.execution.active_itinerary_revision
                && leg.leg_index == operation.execution.current_leg_index
        })
        .cloned()
        .ok_or_else(|| invalid_record("reroute source execution leg is missing"))?;
    let previous_leg_plan = operation
        .route_plan
        .legs
        .get(previous_execution_leg.leg_index)
        .ok_or_else(|| invalid_record("reroute source leg plan is missing"))?
        .clone();
    let previous_attempt = operation
        .execution
        .delivery_attempt
        .clone()
        .ok_or_else(|| invalid_record("reroute lacks delivery-attempt evidence"))?;
    let (mut snapshot, address) = carrier_planning_snapshot(
        view,
        &operation.intent.carrier,
        &operation.intent.recipient,
        context.at,
    )?;
    snapshot
        .network
        .connections
        .retain(|connection| !excluded_connections.contains(&connection.id));
    let origin = operation
        .execution
        .current_endpoint
        .clone()
        .ok_or_else(|| invalid_record("reroute has no current endpoint"))?;
    let planned = plan_route(
        &snapshot,
        &RoutingRequest {
            origin: canwu_api::RoutingNodeRef::new(origin.clone()),
            destination: address.destination.clone(),
            departure_at: context.at,
            policy: operation.intent.routing_policy.clone(),
        },
    );
    let Ok(plan) = planned else {
        operation.status = CorrespondenceStatus::WaitingForRoute;
        operation.last_error = Some("no known route after disruption".to_owned());
        advance_sequence(operation)?;
        return Ok(());
    };
    if plan.legs.is_empty() {
        operation.status = CorrespondenceStatus::WaitingForRoute;
        operation.last_error = Some("replanning requires an explicit final-mile leg".to_owned());
        advance_sequence(operation)?;
        return Ok(());
    }
    let active = operation
        .execution
        .active_itinerary_revision
        .ok_or_else(|| invalid_record("reroute has no active itinerary"))?;
    let next = canwu_api::ItineraryRevisionId(
        active
            .0
            .checked_add(1)
            .ok_or_else(|| invalid_record("itinerary revision overflow"))?,
    );
    operation
        .execution
        .reroute(
            canwu_api::ItineraryRevision {
                id: next,
                predecessor: Some(active),
                plan: plan.clone(),
                planned_at: context.at,
                valid_from: context.at,
                reason,
                superseded_at: None,
                evidence: evidence.clone(),
            },
            context.at,
        )
        .map_err(transport_error)?;
    let next_leg = current_route_leg(operation)?.clone();
    let next_execution_leg = operation
        .execution
        .legs
        .iter()
        .find(|leg| {
            Some(leg.itinerary_revision) == operation.execution.active_itinerary_revision
                && leg.leg_index == operation.execution.current_leg_index
        })
        .cloned()
        .ok_or_else(|| invalid_record("reroute destination execution leg is missing"))?;
    let handoff_id = u64::try_from(operation.execution.handoffs.len())
        .map_err(|_| invalid_record("handoff count is not representable"))?
        .checked_add(1)
        .ok_or_else(|| invalid_record("handoff identity overflow"))?;
    operation
        .execution
        .record_handoff(canwu_api::Handoff {
            id: canwu_api::HandoffId(handoff_id),
            from_leg: previous_execution_leg.id,
            to_leg: next_execution_leg.id,
            from_custodian: format!("connection:{}", previous_leg_plan.connection.as_str()),
            to_custodian: format!("connection:{}", next_leg.connection.as_str()),
            at: context.at,
            location: origin,
            evidence: evidence
                .iter()
                .cloned()
                .chain(std::iter::once(EvidenceRef::DomainRecordVersion(
                    previous_attempt,
                )))
                .collect(),
            kind: canwu_api::HandoffKind::Planned,
        })
        .map_err(transport_error)?;
    operation.route_plan = plan;
    operation.planning_snapshot_digest = operation.route_plan.planning_snapshot_digest.clone();
    operation.address = address.clone();
    operation
        .planning_history
        .push(CorrespondencePlanningEvidence {
            transport_execution: operation.execution.id,
            itinerary_revision: next,
            planned_at: context.at,
            read_cut: address.read_cut,
            address_source_record: address.source_record,
            planning_snapshot_digest: operation.planning_snapshot_digest.clone(),
            excluded_connections: excluded_connections.to_vec(),
            evidence,
        });
    operation.status = CorrespondenceStatus::Scheduled;
    operation.last_error = None;
    advance_sequence(operation)?;
    let delay = delay_until(
        context.at,
        current_route_leg(operation)?.planned_departure_at,
    )?;
    directives.push(schedule_progress(
        &operation.operation_key,
        operation.next_sequence,
        ProgressAction::StartLeg,
        delay,
    )?);
    Ok(())
}

fn install_planning_knowledge(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    let mut seeds = BTreeMap::new();
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
        if plugin == PLUGIN_NAME && packet_type == REPORT_INGRESS {
            if !matches!(ingress.cause, Some(CauseRef::Boundary(_))) {
                return Err(invalid_authority(
                    "correspondence attempt reports require boundary-generated ingress",
                ));
            }
            directives.push(attempt_report_publication(view, *ingress_id, payload)?);
            continue;
        }
        if plugin != PLUGIN_NAME || packet_type != KNOWLEDGE_INGRESS {
            continue;
        }
        let seed: NetworkKnowledgeSeed = decode(payload, "correspondence knowledge seed")?;
        validate_key(&seed.seed_key, "knowledge seed key")?;
        let hash = canonical_hash(KNOWLEDGE_SEED_HASH_DOMAIN, &seed)?;
        if let Some((prior, _, _)) = seeds.get(&seed.seed_key) {
            if prior != &hash {
                return Err(CanwuError::new(
                    ErrorCode::IdempotencyConflict,
                    "knowledge seed key was reused with different input",
                ));
            }
            continue;
        }
        seeds.insert(seed.seed_key.clone(), (hash, seed, *ingress_id));
    }
    for (hash, seed, ingress_id) in seeds.values() {
        let reference = knowledge_seed_ref(seed.seed_key.clone());
        if let Some(existing) = view.typed_domain_record(&reference)? {
            let receipt = existing.decode_payload::<KnowledgeSeedRecord>()?;
            if receipt.canonical_input_hash != *hash {
                return Err(CanwuError::new(
                    ErrorCode::IdempotencyConflict,
                    "knowledge seed key conflicts with persisted input",
                ));
            }
            continue;
        }
        let mut records = Vec::new();
        for endpoint in &seed.endpoints {
            records.push(knowledge_draft(
                ENDPOINT_KNOWLEDGE_SCHEMA,
                endpoint,
                *ingress_id,
            )?);
        }
        for connection in &seed.connections {
            records.push(knowledge_draft(
                CONNECTION_KNOWLEDGE_SCHEMA,
                connection,
                *ingress_id,
            )?);
        }
        for address in &seed.addresses {
            records.push(knowledge_draft(
                ADDRESS_KNOWLEDGE_SCHEMA,
                address,
                *ingress_id,
            )?);
        }
        directives.push(BoundaryDirective::PublishKnowledge {
            holder: seed.holder.clone(),
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some(format!("knowledge-seed:{}", seed.seed_key)),
            records,
            summary: "Publish holder-relative correspondence planning knowledge".to_owned(),
        });
        let receipt = KnowledgeSeedReceipt {
            seed_key: seed.seed_key.clone(),
            canonical_input_hash: hash.clone(),
            holder: seed.holder.clone(),
            published_at: context.at,
        };
        directives.push(mutate(
            create_typed(
                &reference,
                &receipt,
                vec![holder_reference("holder", &seed.holder)],
            )?,
            "Persist correspondence knowledge seed receipt",
        ));
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Publishes one attempt report to the carrier holder. The report is derived
/// from persisted records only: the triggered seizure incident, the
/// information operation that closed the attempt, and the closed delivery
/// attempt record.
fn attempt_report_publication(
    view: &SimulationView<'_>,
    ingress_id: canwu_api::IngressId,
    payload: &Value,
) -> Result<BoundaryDirective, CanwuError> {
    let request: AttemptReportRequest = decode(payload, "correspondence attempt report")?;
    let operation = view
        .typed_domain_record(&correspondence_operation_ref(&request.operation_key))?
        .ok_or_else(|| invalid_record("attempt report names a missing correspondence"))?
        .decode_payload::<CorrespondenceOperationRecord>()?;
    let mismatch = || invalid_record("attempt report does not match a closed seizure");
    let incident = operation
        .incidents
        .get(&request.incident_key)
        .filter(|incident| incident.triggered)
        .ok_or_else(mismatch)?;
    let (
        CorrespondenceIncidentKind::CarrierSeized {
            custody_handoff, ..
        },
        Some(closing),
    ) = (&incident.kind, incident.information_operation.as_ref())
    else {
        return Err(mismatch());
    };
    let closing = view
        .typed_domain_record(&derive_operation_record_ref(closing))?
        .ok_or_else(mismatch)?
        .decode_payload::<InformationOperationRecord>()?;
    let attempt_kind = canwu_api::DomainRecordKind::for_type::<DeliveryAttempt>();
    let attempt_version = closing
        .domain_result_evidence
        .iter()
        .find(|evidence| evidence.record.kind == attempt_kind)
        .filter(|_| closing.status == InformationOperationStatus::Completed)
        .cloned()
        .ok_or_else(mismatch)?;
    let attempt_record = view
        .typed_domain_record(&typed::<DeliveryAttempt>(attempt_version.record.clone())?)?
        .ok_or_else(mismatch)?;
    let attempt = attempt_record.decode_payload::<DeliveryAttempt>()?;
    if attempt_record.version != attempt_version.version
        || attempt.status != DeliveryAttemptStatus::Failed
    {
        return Err(mismatch());
    }
    let report = CorrespondenceAttemptReport {
        operation_key: operation.operation_key.clone(),
        attempt_number: attempt.attempt_number,
        ended_at: incident.at,
        outcome: CorrespondenceAttemptOutcome::CarrierSeized {
            custody_handoff: *custody_handoff,
        },
    };
    let mut evidence = vec![
        EvidenceRef::Ingress(ingress_id),
        EvidenceRef::DomainRecordVersion(attempt_version),
    ];
    evidence.sort();
    Ok(BoundaryDirective::PublishKnowledge {
        holder: operation.intent.carrier.clone(),
        visibility: StateVisibility::SameBoundary,
        producer_correlation: Some(format!(
            "attempt-report:{}:{}",
            report.operation_key, report.attempt_number
        )),
        records: vec![KnowledgeRecordDraft {
            schema: schema_id(ATTEMPT_REPORT_KNOWLEDGE_SCHEMA),
            subjects: Vec::new(),
            payload: serde_json::to_value(&report).map_err(encode_error)?,
            as_of: None,
            confidence_per_mille: 1_000,
            origin: KnowledgeOrigin {
                method: REPORT_ORIGIN_METHOD.to_owned(),
                evidence,
            },
            supersedes: Vec::new(),
            contradicts: Vec::new(),
        }],
        summary: "Publish a holder-relative correspondence attempt report".to_owned(),
    })
}

fn activation_operation(
    request: &InitiateCorrespondenceRequest,
    dispatch: TypedDomainRecordRef<Dispatch>,
    at: SimTime,
) -> Result<
    (
        InformationOperationEnvelope,
        TypedDomainRecordRef<DeliveryAttempt>,
    ),
    CanwuError,
> {
    let slot = addressed_attempt_output_slot(0);
    let attempt_untyped = derive_output_record_ref(&request.delivery_attempt_operation, &slot);
    let attempt = typed::<DeliveryAttempt>(attempt_untyped)?;
    let binding = RecordBinding::new(
        attempt.clone(),
        vec![
            DomainReference::from_typed("dispatch", dispatch.clone()),
            holder_reference("recipient", &request.recipient),
        ],
    );
    Ok((
        InformationOperationEnvelope {
            id: request.delivery_attempt_operation.clone(),
            operation_version: 1,
            operation_kind: "activate_addressed_dispatch".to_owned(),
            output_slots: vec![slot],
            lineage: Vec::new(),
            operation: InformationOperation {
                request: LifecycleRequest::ActivateAddressedDispatch {
                    dispatch,
                    expected_version: request.prepared_dispatch.version,
                    dispatched_at: at,
                    attempts: vec![AddressedDeliveryAttemptDraft {
                        binding,
                        payload: DeliveryAttemptPayload {
                            status: DeliveryAttemptStatus::Prepared,
                            attempt_number: 1,
                            prepared_at: at,
                            dispatched_at: None,
                            due_at: request.due_at,
                            completed_at: None,
                        },
                    }],
                },
            },
        },
        attempt,
    ))
}

fn begin_retry(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    ingress_id: canwu_api::IngressId,
    operation: &mut CorrespondenceOperation,
    action: &CorrespondenceRecoveryAction,
    directives: &mut Vec<BoundaryDirective>,
) -> Result<(), CanwuError> {
    let CorrespondenceRecoveryAction::RetryDelivery {
        due_at,
        delivery_attempt_operation,
        execution_id,
    } = action
    else {
        return Err(invalid_record("retry helper requires retry action"));
    };
    // The retry dispatches a new attempt now, so the delegation admitted with
    // the retry command must still cover this boundary.
    admitted_authority_covers(
        &operation.intent.sender,
        &operation.intent.carrier,
        operation.intent.carrier_authority.as_ref(),
        context.at,
    )?;
    let excluded = active_disaster_connections(operation);
    let (mut snapshot, address) = carrier_planning_snapshot(
        view,
        &operation.intent.carrier,
        &operation.intent.recipient,
        context.at,
    )?;
    snapshot
        .network
        .connections
        .retain(|connection| !excluded.contains(&connection.id));
    let planned = plan_route(
        &snapshot,
        &RoutingRequest {
            origin: operation.intent.origin.clone(),
            destination: address.destination.clone(),
            departure_at: context.at,
            policy: operation.intent.routing_policy.clone(),
        },
    );
    let Ok(plan) = planned else {
        operation.last_error = Some("retry has no known route".to_owned());
        advance_sequence(operation)?;
        return Ok(());
    };
    if plan.legs.is_empty() {
        operation.last_error = Some("retry requires an explicit final-mile leg".to_owned());
        advance_sequence(operation)?;
        return Ok(());
    }
    let previous_attempt = operation
        .execution
        .delivery_attempt
        .clone()
        .ok_or_else(|| invalid_record("retry lacks previous delivery-attempt evidence"))?;
    let attempt_number = operation
        .current_attempt_number
        .checked_add(1)
        .ok_or_else(|| invalid_record("delivery-attempt number overflow"))?;
    let envelope = retry_attempt_operation(
        operation,
        &previous_attempt,
        attempt_number,
        context.at,
        *due_at,
        delivery_attempt_operation,
    )?;
    operation.route_plan = plan;
    operation.planning_snapshot_digest = operation.route_plan.planning_snapshot_digest.clone();
    operation.address = address.clone();
    operation
        .planning_history
        .push(CorrespondencePlanningEvidence {
            transport_execution: *execution_id,
            itinerary_revision: canwu_api::ItineraryRevisionId(1),
            planned_at: context.at,
            read_cut: address.read_cut,
            address_source_record: address.source_record,
            planning_snapshot_digest: operation.planning_snapshot_digest.clone(),
            excluded_connections: excluded,
            evidence: vec![EvidenceRef::Ingress(ingress_id)],
        });
    operation.execution = TransportExecution::new(*execution_id, None);
    operation.delivery_attempt_operation = delivery_attempt_operation.clone();
    operation.current_attempt_number = attempt_number;
    operation.current_attempt_prepared_at = context.at;
    operation.current_due_at = *due_at;
    operation.pending_information = Some(PendingInformationOperation {
        step: InformationSagaStep::BeginRetry,
        id: envelope.id.clone(),
        expected_status: InformationOperationStatus::Completed,
    });
    operation.status = CorrespondenceStatus::AwaitingInformationActivation;
    operation.last_error = None;
    advance_sequence(operation)?;
    directives.push(schedule_information(&envelope)?);
    directives.push(schedule_progress(
        &operation.operation_key,
        operation.next_sequence,
        ProgressAction::ReconcileInformation,
        SimDuration::ZERO,
    )?);
    Ok(())
}

fn retry_attempt_operation(
    operation: &CorrespondenceOperation,
    previous_attempt: &DomainRecordVersionRef,
    attempt_number: u32,
    prepared_at: SimTime,
    due_at: SimTime,
    id: &InformationOperationId,
) -> Result<InformationOperationEnvelope, CanwuError> {
    let slot = InformationOutputSlot {
        index: 0,
        name: "result".to_owned(),
        kind: InformationOutputKind::DeliveryAttempt,
    };
    let attempt = typed::<DeliveryAttempt>(derive_output_record_ref(id, &slot))?;
    let dispatch = typed::<Dispatch>(operation.dispatch.record.clone())?;
    let previous = typed::<DeliveryAttempt>(previous_attempt.record.clone())?;
    Ok(InformationOperationEnvelope {
        id: id.clone(),
        operation_version: 1,
        operation_kind: "begin_delivery_attempt".to_owned(),
        output_slots: vec![slot],
        lineage: Vec::new(),
        operation: InformationOperation {
            request: LifecycleRequest::BeginDeliveryAttempt {
                binding: RecordBinding::new(
                    attempt,
                    vec![
                        DomainReference::from_typed("dispatch", dispatch),
                        DomainReference::from_typed("previous_attempt", previous),
                        holder_reference("recipient", &operation.intent.recipient),
                    ],
                ),
                payload: DeliveryAttemptPayload {
                    status: DeliveryAttemptStatus::Prepared,
                    attempt_number,
                    prepared_at,
                    dispatched_at: None,
                    due_at,
                    completed_at: None,
                },
            },
        },
    })
}

fn attempt_transition_operation(
    operation: &CorrespondenceOperation,
    attempt: &DomainRecordVersionRef,
    status: DeliveryAttemptStatus,
    at: SimTime,
    suffix: &str,
) -> Result<InformationOperationEnvelope, CanwuError> {
    let reference = typed::<DeliveryAttempt>(attempt.record.clone())?;
    let dispatched_at = Some(operation.current_attempt_prepared_at);
    Ok(InformationOperationEnvelope {
        id: InformationOperationId::new(
            INFORMATION_NAMESPACE,
            format!(
                "{}::attempt-{}::{suffix}",
                operation.operation_key, operation.current_attempt_number
            ),
        ),
        operation_version: 1,
        operation_kind: "transition_delivery_attempt".to_owned(),
        output_slots: Vec::new(),
        lineage: Vec::new(),
        operation: InformationOperation {
            request: LifecycleRequest::TransitionDeliveryAttempt {
                record: reference,
                expected_version: attempt.version,
                proposed: DeliveryAttemptPayload {
                    status,
                    attempt_number: operation.current_attempt_number,
                    prepared_at: operation.current_attempt_prepared_at,
                    dispatched_at,
                    due_at: operation.current_due_at,
                    completed_at: status.is_terminal().then_some(at),
                },
            },
        },
    })
}

fn dispatch_completion_operation(
    operation: &CorrespondenceOperation,
    dispatch: &DispatchPayload,
    at: SimTime,
) -> Result<InformationOperationEnvelope, CanwuError> {
    let record = typed::<Dispatch>(operation.dispatch.record.clone())?;
    Ok(InformationOperationEnvelope {
        id: InformationOperationId::new(
            INFORMATION_NAMESPACE,
            format!("{}::complete-dispatch", operation.operation_key),
        ),
        operation_version: 1,
        operation_kind: "transition_dispatch".to_owned(),
        output_slots: Vec::new(),
        lineage: Vec::new(),
        operation: InformationOperation {
            request: LifecycleRequest::TransitionDispatch {
                record,
                expected_version: operation.dispatch.version,
                proposed: DispatchPayload {
                    status: DispatchStatus::Completed,
                    target: dispatch.target.clone(),
                    prepared_at: dispatch.prepared_at,
                    dispatched_at: dispatch.dispatched_at,
                    completed_at: Some(at),
                },
            },
        },
    })
}

fn interception_operation(
    view: &SimulationView<'_>,
    operation: &CorrespondenceOperation,
    incident_key: &str,
    holder: &KnowledgeHolderRef,
    extent_per_mille: u16,
    at: SimTime,
) -> Result<InformationOperationEnvelope, CanwuError> {
    let dispatch = typed::<Dispatch>(operation.intent.prepared_dispatch.record.clone())?;
    let dispatch_record = view
        .typed_domain_record(&dispatch)?
        .ok_or_else(|| invalid_record("interception dispatch is missing"))?;
    let representation = dispatch_record
        .references
        .iter()
        .find_map(|reference| match (&*reference.role, &reference.target) {
            ("representation", DomainReferenceTarget::Domain(target)) => Some(target.clone()),
            _ => None,
        })
        .ok_or_else(|| invalid_record("interception dispatch lacks representation"))?;
    let representation = typed::<Representation>(representation)?;
    let attempt = operation
        .execution
        .delivery_attempt
        .as_ref()
        .ok_or_else(|| invalid_record("interception lacks an active attempt"))?;
    let attempt = typed::<DeliveryAttempt>(attempt.record.clone())?;
    let id = InformationOperationId::new(
        INFORMATION_NAMESPACE,
        format!("{}::interception::{incident_key}", operation.operation_key),
    );
    let slot = InformationOutputSlot {
        index: 0,
        name: "result".to_owned(),
        kind: InformationOutputKind::Access,
    };
    let access = typed::<Access>(derive_output_record_ref(&id, &slot))?;
    Ok(InformationOperationEnvelope {
        id,
        operation_version: 1,
        operation_kind: "record_access".to_owned(),
        output_slots: vec![slot],
        lineage: Vec::new(),
        operation: InformationOperation {
            request: LifecycleRequest::RecordAccess {
                binding: RecordBinding::new(
                    access,
                    vec![
                        DomainReference::from_typed("representation", representation),
                        DomainReference::from_typed("dispatch", dispatch),
                        DomainReference::from_typed("delivery_attempt", attempt),
                        holder_reference("holder", holder),
                    ],
                ),
                payload: AccessPayload {
                    accessed_at: at,
                    method: "interception".to_owned(),
                    extent_per_mille,
                },
                audience_evidence: None,
            },
        },
    })
}

fn delivery_attempt_ref(
    operation: &CorrespondenceOperation,
) -> Result<TypedDomainRecordRef<DeliveryAttempt>, CanwuError> {
    let slot = if operation.current_attempt_number == 1 {
        addressed_attempt_output_slot(0)
    } else {
        InformationOutputSlot {
            index: 0,
            name: "result".to_owned(),
            kind: InformationOutputKind::DeliveryAttempt,
        }
    };
    typed::<DeliveryAttempt>(derive_output_record_ref(
        &operation.delivery_attempt_operation,
        &slot,
    ))
}

fn current_route_leg(
    operation: &CorrespondenceOperation,
) -> Result<&canwu_api::RouteLeg, CanwuError> {
    let revision = operation
        .execution
        .active_itinerary_revision
        .ok_or_else(|| invalid_record("transport execution has no active itinerary"))?;
    let plan = operation
        .execution
        .revisions
        .iter()
        .find(|candidate| candidate.id == revision)
        .ok_or_else(|| invalid_record("active itinerary revision is missing"))?;
    plan.plan
        .legs
        .get(operation.execution.current_leg_index)
        .ok_or_else(|| invalid_record("transport execution has no current route leg"))
}

fn schedule_information(
    envelope: &InformationOperationEnvelope,
) -> Result<BoundaryDirective, CanwuError> {
    Ok(BoundaryDirective::SchedulePluginIngress {
        target_plugin: canwu_information::PLUGIN_NAME.to_owned(),
        after: SimDuration::ZERO,
        packet_type: INFORMATION_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(envelope).map_err(encode_error)?,
        affected: Vec::new(),
    })
}

fn schedule_progress(
    operation_key: &str,
    sequence: u64,
    action: ProgressAction,
    after: SimDuration,
) -> Result<BoundaryDirective, CanwuError> {
    Ok(BoundaryDirective::ScheduleIngress {
        after,
        packet_type: PROGRESS_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(ProgressRequest {
            operation_key: operation_key.to_owned(),
            sequence,
            action,
        })
        .map_err(encode_error)?,
        affected: Vec::new(),
    })
}

fn operation_references(
    operation: &CorrespondenceOperation,
    dispatch: Option<TypedDomainRecordRef<Dispatch>>,
    attempt: Option<TypedDomainRecordRef<DeliveryAttempt>>,
) -> Vec<DomainReference> {
    let mut references = vec![
        core_reference("sender", operation.intent.sender.clone()),
        holder_reference("recipient", &operation.intent.recipient),
        holder_reference("carrier", &operation.intent.carrier),
    ];
    if let Some(dispatch) = dispatch {
        references.push(DomainReference::from_typed("dispatch", dispatch));
    }
    if let Some(attempt) = attempt {
        references.push(DomainReference::from_typed("delivery_attempt", attempt));
    }
    references.sort();
    references
}

fn current_operation_references(
    operation: &CorrespondenceOperation,
) -> Result<Vec<DomainReference>, CanwuError> {
    let dispatch = typed::<Dispatch>(operation.dispatch.record.clone())?;
    let attempt = operation
        .execution
        .delivery_attempt
        .as_ref()
        .map(|attempt| typed::<DeliveryAttempt>(attempt.record.clone()))
        .transpose()?;
    let mut references = operation_references(operation, Some(dispatch), attempt);
    if let CorrespondenceAuthority::Automatic { opportunity } = &operation.intent.authority {
        references.push(DomainReference::from_typed(
            "opportunity",
            opportunity.clone(),
        ));
    }
    references.sort();
    Ok(references)
}

fn opportunity_references(opportunity: &CommunicationOpportunity) -> Vec<DomainReference> {
    let mut references = vec![core_reference("sender", opportunity.sender.clone())];
    references.extend(
        opportunity
            .candidates
            .iter()
            .map(|candidate| holder_reference("candidate", candidate)),
    );
    references.sort();
    references.dedup();
    references
}

fn create_typed<T>(
    reference: &TypedDomainRecordRef<T>,
    payload: &T::Payload,
    references: Vec<DomainReference>,
) -> Result<DomainRecordMutation, CanwuError>
where
    T: DomainRecordType,
    T::Payload: Serialize,
{
    let mut draft = DomainRecordDraft::from_typed(reference.clone(), payload)?;
    draft.references = references;
    Ok(DomainRecordMutation::Create { record: draft })
}

fn update_typed<T>(
    reference: &TypedDomainRecordRef<T>,
    payload: &T::Payload,
    references: Vec<DomainReference>,
    expected_version: u64,
) -> Result<DomainRecordMutation, CanwuError>
where
    T: DomainRecordType,
    T::Payload: Serialize,
{
    let mut draft = DomainRecordDraft::from_typed(reference.clone(), payload)?;
    draft.references = references;
    Ok(DomainRecordMutation::Update {
        record: draft,
        expected_version,
    })
}

fn update_operation(
    record: &DomainRecord,
    operation: &CorrespondenceOperation,
) -> Result<BoundaryDirective, CanwuError> {
    let reference = correspondence_operation_ref(operation.operation_key.clone());
    let mut draft = DomainRecordDraft::from_typed(reference, operation)?;
    draft.references = current_operation_references(operation)?;
    Ok(mutate(
        DomainRecordMutation::Update {
            record: draft,
            expected_version: record.version,
        },
        "Advance correspondence operation",
    ))
}

fn knowledge_draft(
    name: &str,
    payload: &impl Serialize,
    ingress_id: canwu_api::IngressId,
) -> Result<KnowledgeRecordDraft, CanwuError> {
    Ok(KnowledgeRecordDraft {
        schema: schema_id(name),
        subjects: Vec::new(),
        payload: serde_json::to_value(payload).map_err(encode_error)?,
        as_of: None,
        confidence_per_mille: 1_000,
        origin: KnowledgeOrigin {
            method: "correspondence_network_observation_v1".to_owned(),
            evidence: vec![EvidenceRef::Ingress(ingress_id)],
        },
        supersedes: Vec::new(),
        contradicts: Vec::new(),
    })
}

fn operation_schema() -> DomainRecordSchema {
    let mut schema = DomainRecordSchema::for_record::<CorrespondenceOperationRecord>();
    schema.references = vec![
        reference_schema(
            "carrier",
            vec![DomainReferenceTargetKind::AnyEntity],
            true,
            false,
        ),
        reference_schema(
            "delivery_attempt",
            vec![DomainReferenceTargetKind::for_domain::<DeliveryAttempt>()],
            false,
            false,
        ),
        reference_schema(
            "dispatch",
            vec![DomainReferenceTargetKind::for_domain::<Dispatch>()],
            true,
            false,
        ),
        reference_schema(
            "opportunity",
            vec![DomainReferenceTargetKind::for_domain::<
                CommunicationOpportunityRecord,
            >()],
            false,
            false,
        ),
        reference_schema(
            "recipient",
            vec![DomainReferenceTargetKind::AnyEntity],
            true,
            false,
        ),
        reference_schema(
            "sender",
            vec![DomainReferenceTargetKind::AnyEntity],
            true,
            false,
        ),
    ];
    schema
}

fn opportunity_schema() -> DomainRecordSchema {
    let mut schema = DomainRecordSchema::for_record::<CommunicationOpportunityRecord>();
    schema.references = vec![
        reference_schema(
            "candidate",
            vec![DomainReferenceTargetKind::AnyEntity],
            true,
            true,
        ),
        reference_schema(
            "sender",
            vec![DomainReferenceTargetKind::AnyEntity],
            true,
            false,
        ),
    ];
    schema
}

fn knowledge_seed_schema() -> DomainRecordSchema {
    let mut schema = DomainRecordSchema::for_record::<KnowledgeSeedRecord>();
    schema.references = vec![reference_schema(
        "holder",
        vec![DomainReferenceTargetKind::AnyEntity],
        true,
        false,
    )];
    schema
}

fn carrier_delegation_schema() -> DomainRecordSchema {
    DomainRecordSchema::for_record::<CarrierDelegationRecord>()
}

fn reference_schema(
    role: &str,
    targets: Vec<DomainReferenceTargetKind>,
    required: bool,
    multiple: bool,
) -> DomainReferenceSchema {
    DomainReferenceSchema {
        role: role.to_owned(),
        targets,
        required,
        multiple,
        allow_retired: false,
    }
}

fn mutate(mutation: DomainRecordMutation, summary: &str) -> BoundaryDirective {
    BoundaryDirective::MutateRecord {
        mutation,
        summary: summary.to_owned(),
    }
}

fn core_reference(role: &str, entity: EntityRef) -> DomainReference {
    DomainReference {
        role: role.to_owned(),
        target: DomainReferenceTarget::Core(entity),
    }
}

fn holder_reference(role: &str, holder: &KnowledgeHolderRef) -> DomainReference {
    let entity = match holder {
        KnowledgeHolderRef::Person(person) => EntityRef::Person(*person),
        KnowledgeHolderRef::Entity(entity) => entity.clone(),
    };
    core_reference(role, entity)
}

fn typed<T: DomainRecordType>(
    reference: canwu_api::DomainRecordRef,
) -> Result<TypedDomainRecordRef<T>, CanwuError> {
    TypedDomainRecordRef::from_untyped(reference).map_err(|reference| {
        invalid_record(format!("domain record has the wrong kind: {reference}"))
    })
}

fn record_state<T: DomainRecordType>() -> StateKey {
    DomainRecordSchema::for_type::<T>().state_key()
}

fn correspondence_random_stream() -> RandomStreamKey {
    RandomStreamKey::new(PLUGIN_NAME, "operation-resolution", 1)
}

fn validate_start_request(request: &InitiateCorrespondenceRequest) -> Result<(), CanwuError> {
    validate_key(&request.operation_key, "correspondence operation key")?;
    validate_key(&request.channel_profile, "channel profile")?;
    if request.execution_id.0 == 0 || request.prepared_dispatch.version == 0 {
        return Err(invalid_record(
            "correspondence execution and dispatch versions must be nonzero",
        ));
    }
    Ok(())
}

/// Resolves the carrier's current delegation for the sender, requires it to
/// be the `cited` delegation command, and checks its claim at `at`.
///
/// A sender-owned carrier cites none and takes none. Any other carrier must
/// cite the accepted [`CARRIER_DELEGATION_COMMAND`] that its
/// [`CarrierDelegationRecord`] for this sender currently holds; naming
/// another holder without it, or citing a delegation the carrier has since
/// replaced, is not read authority. The claim must give
/// [`CARRY_CORRESPONDENCE_CAPABILITY`] to the carrier (`performed_by`) for
/// the sender (`performed_for`) over an interval covering `at`. The record is
/// a persisted fact, so the decision is the same live and in replay even
/// after command evidence is sealed.
fn current_carrier_authority(
    view: &SimulationView<'_>,
    sender: &EntityRef,
    carrier: &KnowledgeHolderRef,
    cited: Option<canwu_api::CommandId>,
    at: SimTime,
) -> Result<Option<CarrierAuthority>, CanwuError> {
    let carrier_entity = holder_entity(carrier);
    if carrier_entity == *sender {
        return if cited.is_some() {
            Err(invalid_authority(
                "a sender-owned carrier takes no carrier delegation",
            ))
        } else {
            Ok(None)
        };
    }
    let Some(cited) = cited else {
        return Err(invalid_authority(
            "a carrier other than the sender requires an admitted carrier delegation",
        ));
    };
    let principal = holder_from_entity(sender);
    let authority = view
        .typed_domain_record(&carrier_delegation_ref(&carrier_entity, &principal)?)?
        .map(DomainRecord::decode_payload::<CarrierDelegationRecord>)
        .transpose()?
        .filter(|authority| authority.delegation == cited)
        .ok_or_else(|| {
            invalid_authority(
                "cited carrier delegation is not the carrier's current delegation for the sender",
            )
        })?;
    validate_delegation_claim(
        &authority.claim,
        &carrier_entity,
        &principal,
        CARRY_CORRESPONDENCE_CAPABILITY,
        at,
    )
    .map_err(|error| invalid_authority(format!("carrier delegation claim is invalid: {error}")))?;
    Ok(Some(authority))
}

/// Checks, at settlement, that the delegation resolved when the command was
/// admitted still covers `at`. Settlement happens at the admission instant,
/// so the admission-time record check stands; only the claim window is
/// re-checked, which never depends on records written after admission.
fn admitted_authority_covers(
    sender: &EntityRef,
    carrier: &KnowledgeHolderRef,
    authority: Option<&CarrierAuthority>,
    at: SimTime,
) -> Result<(), CanwuError> {
    let carrier_entity = holder_entity(carrier);
    let Some(authority) = authority else {
        return if carrier_entity == *sender {
            Ok(())
        } else {
            Err(invalid_authority(
                "a carrier other than the sender requires an admitted carrier delegation",
            ))
        };
    };
    validate_delegation_claim(
        &authority.claim,
        &carrier_entity,
        &holder_from_entity(sender),
        CARRY_CORRESPONDENCE_CAPABILITY,
        at,
    )
    .map_err(|error| invalid_authority(format!("carrier delegation claim is invalid: {error}")))
}

/// Re-resolves an admitted correspondence's carrier delegation at `at`.
fn intent_carrier_authority(
    view: &SimulationView<'_>,
    intent: &CorrespondenceIntent,
    at: SimTime,
) -> Result<Option<CarrierAuthority>, CanwuError> {
    current_carrier_authority(
        view,
        &intent.sender,
        &intent.carrier,
        intent
            .carrier_authority
            .as_ref()
            .map(|authority| authority.delegation),
        at,
    )
}

fn holder_entity(holder: &KnowledgeHolderRef) -> EntityRef {
    match holder {
        KnowledgeHolderRef::Person(person) => EntityRef::Person(*person),
        KnowledgeHolderRef::Entity(entity) => entity.clone(),
    }
}

fn holder_from_entity(entity: &EntityRef) -> KnowledgeHolderRef {
    match entity {
        EntityRef::Person(person) => KnowledgeHolderRef::Person(*person),
        entity => KnowledgeHolderRef::Entity(entity.clone()),
    }
}

fn claim_operation_update(seen_operations: &mut BTreeSet<String>, operation_key: &str) -> bool {
    seen_operations.insert(operation_key.to_owned())
}

fn validate_recovery_request(
    operation: &CorrespondenceOperation,
    request: &ResolveCorrespondenceRequest,
    at: SimTime,
) -> Result<(), CanwuError> {
    if operation.pending_information.is_some() {
        return Err(invalid_record(
            "correspondence recovery cannot overlap an information operation",
        ));
    }
    match &request.action {
        CorrespondenceRecoveryAction::ReplanCurrentAttempt => {
            if operation.status != CorrespondenceStatus::WaitingForRoute
                || operation.execution.state != TransportExecutionState::ReplanPending
            {
                return Err(invalid_record(
                    "route recovery requires a replan-pending correspondence",
                ));
            }
        }
        CorrespondenceRecoveryAction::RetryDelivery {
            due_at,
            delivery_attempt_operation,
            execution_id,
        } => {
            if !matches!(
                operation.status,
                CorrespondenceStatus::Failed | CorrespondenceStatus::DeadlineMissed
            ) || *due_at < at
                || execution_id.0 == 0
            {
                return Err(invalid_record(
                    "delivery retry requires failed correspondence, future deadline, and execution identity",
                ));
            }
            if operation.current_attempt_number
                >= canwu_information::InformationLimitsV1::default()
                    .max_delivery_attempts_per_recipient
            {
                return Err(invalid_record(
                    "delivery retry exceeds the information attempt limit",
                ));
            }
            if operation.delivery_attempt_operation == *delivery_attempt_operation
                || operation.execution.id == *execution_id
                || operation.recovery_history.iter().any(|recovery| {
                    matches!(
                        &recovery.action,
                        CorrespondenceRecoveryAction::RetryDelivery {
                            delivery_attempt_operation: prior_operation,
                            execution_id: prior_execution,
                            ..
                        } if prior_operation == delivery_attempt_operation
                            || prior_execution == execution_id
                    )
                })
            {
                return Err(invalid_record(
                    "delivery retry identities must be new within the correspondence",
                ));
            }
        }
        CorrespondenceRecoveryAction::FinalizeDispatch => {
            if !matches!(
                operation.status,
                CorrespondenceStatus::Failed | CorrespondenceStatus::DeadlineMissed
            ) {
                return Err(invalid_record(
                    "dispatch finalization requires failed correspondence",
                ));
            }
        }
    }
    Ok(())
}

fn validate_key(value: &str, label: &str) -> Result<(), CanwuError> {
    if value.is_empty() || value.trim() != value || value.len() > 256 {
        return Err(invalid_record(format!(
            "{label} must be canonical text no longer than 256 bytes"
        )));
    }
    Ok(())
}

fn origin_controls_sender(origin: &DecisionOrigin, sender: &EntityRef) -> bool {
    match (origin, sender) {
        (DecisionOrigin::Actor { actor }, EntityRef::Person(sender)) => actor == sender,
        (DecisionOrigin::Institution { institution, .. }, sender) => institution == sender,
        _ => false,
    }
}

fn advance_sequence(operation: &mut CorrespondenceOperation) -> Result<(), CanwuError> {
    operation.next_sequence = operation
        .next_sequence
        .checked_add(1)
        .ok_or_else(|| invalid_record("correspondence progress sequence overflow"))?;
    Ok(())
}

fn delay_until(now: SimTime, target: SimTime) -> Result<SimDuration, CanwuError> {
    if target <= now {
        return Ok(SimDuration::ZERO);
    }
    target
        .checked_sub(now)
        .ok_or_else(|| invalid_record("correspondence schedule duration overflow"))
}

fn decode<T: for<'de> Deserialize<'de>>(payload: &Value, label: &str) -> Result<T, CanwuError> {
    serde_json::from_value(payload.clone()).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("{label} could not be decoded: {error}"),
        )
    })
}

fn invalid_record(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}

fn invalid_authority(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidAuthority, message)
}

#[allow(clippy::needless_pass_by_value)]
fn encode_error(error: serde_json::Error) -> CanwuError {
    CanwuError::new(
        ErrorCode::InvalidPayload,
        format!("correspondence payload could not be encoded: {error}"),
    )
}

#[allow(clippy::needless_pass_by_value)]
fn transport_error(error: canwu_api::TransportError) -> CanwuError {
    invalid_record(format!("transport transition failed: {error:?}"))
}
