//! Addressed correspondence orchestration for Canwu.
//!
//! This published experimental domain extension binds decision-backed communication
//! intents to holder-relative address and route knowledge, the pure routing
//! mechanism, transport execution, and the neutral information lifecycle.

#![allow(clippy::missing_errors_doc, clippy::too_many_lines)]

mod host;
mod knowledge;
mod model;
mod plugin;

pub use host::{
    correspondence_command, correspondence_decision_ticket,
    correspondence_recovery_decision_ticket, resolve_correspondence_command,
};
pub use knowledge::{
    ADDRESS_KNOWLEDGE_SCHEMA, CONNECTION_KNOWLEDGE_SCHEMA, ENDPOINT_KNOWLEDGE_SCHEMA, KnownAddress,
    KnownRoutingConnection, KnownRoutingEndpoint, NetworkKnowledgeSeed,
    correspondence_knowledge_schemas, planning_knowledge_query,
};
pub use model::{
    AddressResolution, CommunicationOpportunity, CommunicationOpportunityRecord,
    CommunicationOpportunityRequest, CommunicationOpportunityStatus, CorrespondenceAuthority,
    CorrespondenceCapacityAdmission, CorrespondenceIncident, CorrespondenceIncidentKind,
    CorrespondenceIncidentRequest, CorrespondenceIntent, CorrespondenceOperation,
    CorrespondenceOperationRecord, CorrespondencePlanningEvidence, CorrespondenceRecovery,
    CorrespondenceRecoveryAction, CorrespondenceStatus, InformationSagaStep,
    InitiateCorrespondenceRequest, KnowledgeSeedReceipt, KnowledgeSeedRecord, ProgressAction,
    ProgressRequest, ResolveCorrespondenceRequest, correspondence_operation_ref,
    knowledge_seed_ref, opportunity_ref,
};
pub use plugin::{
    CORRESPONDENCE_COMMAND, CorrespondencePlugin, INCIDENT_INGRESS, KNOWLEDGE_INGRESS,
    OPPORTUNITY_INGRESS, PLUGIN_NAME, RESOLVE_CORRESPONDENCE_COMMAND, START_INGRESS,
};
