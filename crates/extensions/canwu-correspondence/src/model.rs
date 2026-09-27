use canwu_api::{
    CanwuError, CommandId, DomainRecordType, DomainRecordVersionRef, DomainValueKindClass,
    EntityRef, EvidenceRef, HandoffId, HolderKnowledgeRecordId, ItineraryRevisionId,
    KnowledgeHolderRef, KnowledgeReadCut, RoutePlan, RoutingConnectionRef, RoutingNodeRef,
    RoutingPolicy, SimTime, TransportExecution, TransportExecutionId, TypedDomainRecordRef,
    canonical_hash,
};
use canwu_information::{DelegationClaimV1, InformationOperationId, InformationOperationStatus};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const CORRESPONDENCE_NAMESPACE: &str = "canwu.correspondence";
const CARRIER_DELEGATION_KEY_DOMAIN: &str = "canwu.correspondence.carrier-delegation-pair.v1";

pub struct CommunicationOpportunityRecord;

impl DomainRecordType for CommunicationOpportunityRecord {
    type Payload = CommunicationOpportunity;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = CORRESPONDENCE_NAMESPACE;
    const NAME: &'static str = "opportunity";
}

pub struct CorrespondenceOperationRecord;

impl DomainRecordType for CorrespondenceOperationRecord {
    type Payload = CorrespondenceOperation;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = CORRESPONDENCE_NAMESPACE;
    const NAME: &'static str = "operation";
}

pub struct KnowledgeSeedRecord;

impl DomainRecordType for KnowledgeSeedRecord {
    type Payload = KnowledgeSeedReceipt;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = CORRESPONDENCE_NAMESPACE;
    const NAME: &'static str = "knowledge_seed";
}

/// The carrier's current delegation for one principal: the persisted fact of
/// the newest accepted delegation command for that (carrier, principal) pair
/// (see [`carrier_delegation_ref`]).
///
/// Correspondence admission resolves a cited delegation from this record, not
/// from command evidence, so a run whose evidence was sealed into an archive
/// decides exactly as its replay does. A newer delegation for the same pair
/// replaces the record, so an older delegation command stops being citable
/// and the records stay bounded by the number of pairs.
pub struct CarrierDelegationRecord;

impl DomainRecordType for CarrierDelegationRecord {
    type Payload = CarrierAuthority;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = CORRESPONDENCE_NAMESPACE;
    const NAME: &'static str = "carrier_delegation";
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunicationOpportunityStatus {
    Offered,
    SelectedAutomatic,
    Suppressed,
    Consumed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommunicationOpportunity {
    pub operation_key: String,
    pub canonical_input_hash: String,
    pub sender: EntityRef,
    pub candidates: Vec<KnowledgeHolderRef>,
    pub candidate_digest: String,
    pub reason: String,
    pub probability_per_mille: u16,
    pub roll_per_mille: u16,
    pub automatic: bool,
    pub selected_recipient: Option<KnowledgeHolderRef>,
    pub status: CommunicationOpportunityStatus,
    pub evaluated_at: SimTime,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommunicationOpportunityRequest {
    pub operation_key: String,
    pub sender: EntityRef,
    pub candidates: Vec<KnowledgeHolderRef>,
    pub reason: String,
    pub probability_per_mille: u16,
    pub automatic: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CorrespondenceAuthority {
    Decision {
        controller_id: String,
    },
    Automatic {
        opportunity: TypedDomainRecordRef<CommunicationOpportunityRecord>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CorrespondenceIntent {
    pub sender: EntityRef,
    pub recipient: KnowledgeHolderRef,
    pub carrier: KnowledgeHolderRef,
    pub channel_profile: String,
    pub origin: RoutingNodeRef,
    pub accepted_at: SimTime,
    pub due_at: SimTime,
    pub routing_policy: RoutingPolicy,
    pub capacity_admission: CorrespondenceCapacityAdmission,
    pub prepared_dispatch: DomainRecordVersionRef,
    pub authority: CorrespondenceAuthority,
    pub accepted_command: CommandId,
    /// The delegation under which a carrier other than the sender carries
    /// this correspondence, resolved from
    /// [`InitiateCorrespondenceRequest::carrier_delegation`]; `None` for a
    /// sender-owned carrier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier_authority: Option<CarrierAuthority>,
}

/// Payload of the carrier delegation command
/// ([`crate::CARRIER_DELEGATION_COMMAND`]).
///
/// The command is admitted only under the command authority of the carrier
/// the claim names as `performed_by`, so it records the carrier's own
/// acceptance of carrying correspondence for `performed_for`. The claim must
/// list [`crate::CARRY_CORRESPONDENCE_CAPABILITY`] and must not already have
/// expired.
///
/// An accepted delegation is recorded at the boundary after its command and
/// is citable from then on. Only the newest accepted delegation for a
/// (carrier, principal) pair is citable; issuing a new one replaces the
/// previous one, which is how a carrier narrows or ends its delegation early.
/// A correspondence that already started keeps its admitted delegation for
/// the current attempt, but its retries must cite the carrier's current
/// delegation, so a replaced delegation also stops them. Otherwise a
/// delegation stays citable until its `expires_at`, and one without
/// `expires_at` never lapses.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CarrierDelegationRequest {
    pub claim: DelegationClaimV1,
}

/// A resolved carrier delegation: the admitted delegation command and the
/// claim it carries.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CarrierAuthority {
    pub delegation: CommandId,
    pub claim: DelegationClaimV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrespondenceCapacityAdmission {
    Unconstrained,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AddressResolution {
    pub recipient: KnowledgeHolderRef,
    pub destination: RoutingNodeRef,
    pub resolved_at: SimTime,
    pub read_cut: KnowledgeReadCut,
    pub source_record: HolderKnowledgeRecordId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CorrespondencePlanningEvidence {
    pub transport_execution: TransportExecutionId,
    pub itinerary_revision: ItineraryRevisionId,
    pub planned_at: SimTime,
    pub read_cut: KnowledgeReadCut,
    pub address_source_record: HolderKnowledgeRecordId,
    pub planning_snapshot_digest: String,
    pub excluded_connections: Vec<RoutingConnectionRef>,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrespondenceStatus {
    AwaitingInformationActivation,
    AwaitingDispatch,
    Scheduled,
    InTransit,
    AwaitingInformationCompletion,
    Settled,
    DeadlineMissed,
    WaitingForRoute,
    CompensationPending,
    Failed,
}

impl CorrespondenceStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Settled | Self::DeadlineMissed | Self::CompensationPending | Self::Failed
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InformationSagaStep {
    ActivateDispatch,
    BeginRetry,
    MarkInTransit,
    CompleteDelivery,
    CompleteDispatch,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PendingInformationOperation {
    pub step: InformationSagaStep,
    pub id: InformationOperationId,
    pub expected_status: InformationOperationStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CorrespondenceIncidentKind {
    Disaster {
        blocked_connections: Vec<RoutingConnectionRef>,
        explanation: String,
    },
    /// Records access for the interceptor; the delivery attempt continues.
    Interception {
        intercepted_by: KnowledgeHolderRef,
        extent_per_mille: u16,
    },
    /// The carrier and the packet were taken by `seized_by`. Unlike
    /// [`Self::Interception`], a triggered seizure terminates the delivery
    /// attempt: the current leg fails, a terminal
    /// [`canwu_api::HandoffKind::Seizure`] custody handoff is recorded in the
    /// correspondence's transport execution under `custody_handoff`, the
    /// attempt closes as failed, and the carrier holder receives a
    /// [`CorrespondenceAttemptReport`]. The dispatch stays active for the
    /// sender's explicit retry or finalization.
    ///
    /// A zero `custody_handoff`, a malformed seizing identity, or the carrier
    /// itself as `seized_by` rejects the incident. A `custody_handoff` that
    /// already names a recorded handoff of the execution (for example a
    /// planned one) does not apply: the incident is retained as suppressed
    /// evidence and the attempt continues.
    CarrierSeized {
        seized_by: EntityRef,
        custody_handoff: HandoffId,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CorrespondenceIncidentRequest {
    pub operation_key: String,
    pub incident_key: String,
    pub probability_per_mille: u16,
    pub kind: CorrespondenceIncidentKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CorrespondenceIncident {
    pub incident_key: String,
    pub at: SimTime,
    pub probability_per_mille: u16,
    pub roll_per_mille: u16,
    pub triggered: bool,
    pub suppressed_reason: Option<String>,
    pub kind: CorrespondenceIncidentKind,
    pub evidence: Vec<EvidenceRef>,
    pub information_operation: Option<InformationOperationId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CorrespondenceOperation {
    pub operation_key: String,
    pub canonical_input_hash: String,
    pub intent: CorrespondenceIntent,
    pub address: AddressResolution,
    pub planning_snapshot_digest: String,
    pub planning_history: Vec<CorrespondencePlanningEvidence>,
    pub route_plan: RoutePlan,
    pub execution: TransportExecution,
    pub dispatch: DomainRecordVersionRef,
    pub current_attempt_number: u32,
    pub current_attempt_prepared_at: SimTime,
    pub current_due_at: SimTime,
    pub status: CorrespondenceStatus,
    pub pending_information: Option<PendingInformationOperation>,
    pub delivery_attempt_operation: InformationOperationId,
    pub recovery_history: Vec<CorrespondenceRecovery>,
    pub next_sequence: u64,
    pub incidents: BTreeMap<String, CorrespondenceIncident>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CorrespondenceRecovery {
    pub accepted_command: CommandId,
    pub accepted_at: SimTime,
    pub canonical_input_hash: String,
    pub action: CorrespondenceRecoveryAction,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CorrespondenceRecoveryAction {
    ReplanCurrentAttempt,
    RetryDelivery {
        due_at: SimTime,
        delivery_attempt_operation: InformationOperationId,
        execution_id: TransportExecutionId,
    },
    FinalizeDispatch,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResolveCorrespondenceRequest {
    pub operation_key: String,
    pub action: CorrespondenceRecoveryAction,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InitiateCorrespondenceRequest {
    pub operation_key: String,
    pub sender: EntityRef,
    pub recipient: KnowledgeHolderRef,
    pub carrier: KnowledgeHolderRef,
    pub channel_profile: String,
    pub origin: RoutingNodeRef,
    pub due_at: SimTime,
    pub prepared_dispatch: DomainRecordVersionRef,
    pub delivery_attempt_operation: InformationOperationId,
    pub routing_policy: RoutingPolicy,
    pub capacity_admission: CorrespondenceCapacityAdmission,
    pub execution_id: TransportExecutionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automatic_opportunity: Option<TypedDomainRecordRef<CommunicationOpportunityRecord>>,
    /// Required exactly when `carrier` differs from `sender`: an admitted
    /// carrier delegation command ([`crate::CARRIER_DELEGATION_COMMAND`])
    /// whose claim names the carrier as `performed_by` and the sender as
    /// `performed_for`, lists [`crate::CARRY_CORRESPONDENCE_CAPABILITY`], and
    /// whose validity interval covers each dispatch (the initial one and every
    /// retry). With it, route planning and address resolution read the
    /// carrier's knowledge ledger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier_delegation: Option<CommandId>,
}

/// A holder-relative report that one delivery attempt ended.
///
/// Published to the carrier holder, the party that experienced the ending.
/// When the sender is its own carrier that is the sender; a sender that
/// delegated the carrying learns of the ending only through the application.
/// The report names no seizing identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorrespondenceAttemptReport {
    pub operation_key: String,
    pub attempt_number: u32,
    pub ended_at: SimTime,
    pub outcome: CorrespondenceAttemptOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CorrespondenceAttemptOutcome {
    /// The attempt ended because the carrier was seized; `custody_handoff`
    /// is the seizure handoff in the attempt's transport execution.
    CarrierSeized { custody_handoff: HandoffId },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressAction {
    ReconcileInformation,
    StartLeg,
    CompleteLeg,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProgressRequest {
    pub operation_key: String,
    pub sequence: u64,
    pub action: ProgressAction,
}

#[must_use]
pub fn correspondence_operation_ref(
    operation_key: impl Into<String>,
) -> TypedDomainRecordRef<CorrespondenceOperationRecord> {
    TypedDomainRecordRef::new(operation_key)
}

#[must_use]
pub fn opportunity_ref(
    operation_key: impl Into<String>,
) -> TypedDomainRecordRef<CommunicationOpportunityRecord> {
    TypedDomainRecordRef::new(operation_key)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeSeedReceipt {
    pub seed_key: String,
    pub canonical_input_hash: String,
    pub holder: KnowledgeHolderRef,
    pub published_at: SimTime,
}

#[must_use]
pub fn knowledge_seed_ref(
    seed_key: impl Into<String>,
) -> TypedDomainRecordRef<KnowledgeSeedRecord> {
    TypedDomainRecordRef::new(seed_key)
}

/// The record of `carrier`'s current delegation for `principal`.
pub fn carrier_delegation_ref(
    carrier: &EntityRef,
    principal: &KnowledgeHolderRef,
) -> Result<TypedDomainRecordRef<CarrierDelegationRecord>, CanwuError> {
    let pair = canonical_hash(CARRIER_DELEGATION_KEY_DOMAIN, &(carrier, principal))?;
    Ok(TypedDomainRecordRef::new(format!("pair:{pair}")))
}
