use canwu_api::{
    CommandId, DomainRecordType, DomainRecordVersionRef, DomainValueKindClass, EntityRef,
    EvidenceRef, HolderKnowledgeRecordId, ItineraryRevisionId, KnowledgeHolderRef,
    KnowledgeReadCut, RoutePlan, RoutingConnectionRef, RoutingNodeRef, RoutingPolicy, SimTime,
    TransportExecution, TransportExecutionId, TypedDomainRecordRef,
};
use canwu_information::{InformationOperationId, InformationOperationStatus};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const CORRESPONDENCE_NAMESPACE: &str = "canwu.correspondence";

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
    Interception {
        intercepted_by: KnowledgeHolderRef,
        extent_per_mille: u16,
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
