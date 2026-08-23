//! Neutral information lifecycle records and validation for Canwu.
//!
//! This unpublished extension models content, representations, carrier
//! instances, dispatches, access, interpretation, audiences, releases, and the
//! idempotent operations that create or advance them. It intentionally contains
//! no application-specific subject matter, consequences, probabilities, or
//! policy choices.

#![allow(clippy::missing_errors_doc, clippy::too_many_lines)]

mod lifecycle;
mod model;
mod operation;
mod plugin;
mod query;
mod schema;

pub use lifecycle::{
    AddressedDeliveryAttemptDraft, GenericInformationPublicationDraft, InformationLifecycle,
    InformationMutationPlan, LifecycleRequest, MAX_ATOMIC_ADDRESSED_ATTEMPTS, RecordBinding,
    audience_membership_root_v1, validate_access_context, validate_content_lineage,
    validate_delegation_claim, validate_delegation_grant, validate_delivery_attempt_transition,
    validate_dispatch_transition, validate_instance_transition, validate_interpretation,
    validate_release_transition, validate_representation_lineage,
    verify_audience_membership_proof_v1,
};
pub use model::{
    AccessPayload, AudienceAccessEvidence, AudienceMembership, AudienceMembershipLeafV1,
    AudienceMembershipProofV1, AudiencePayload, AuthorityAssignmentPayload, ChannelCapability,
    ChannelPayload, ClaimedSourceRef, ContentDerivation, ContentDigest, ContentPayload,
    ContentRelation, ContentSourceEdge, ContentSourceRole, DelegationAuthorityGrant,
    DelegationClaimV1, DelegationEvidenceSelector, DeliveryAttemptPayload, DeliveryAttemptStatus,
    DigestAlgorithm, DispatchPayload, DispatchStatus, DispatchTarget, InformationBody,
    InformationLimitsV1, InstancePayload, InstanceStatus, InterpretationAuthority,
    InterpretationPayload, InterpretationStatus, ReleasePayload, ReleaseScope, ReleaseStatus,
    RepresentationPayload, RepresentationSourceEdge,
};
pub use operation::{
    InformationAdmissionRef, InformationContinuation, InformationOperation,
    InformationOperationEnvelope, InformationOperationId, InformationOperationPayload,
    InformationOperationStatus, InformationOutputKind, InformationOutputSlot,
    InformationOutputSlotRef, InformationRetryDisposition, LineageParent, OperationLineageNode,
    addressed_attempt_output_slot, canonical_input_bytes, classify_operation_retry,
    derive_operation_record_ref, derive_output_record_ref, validate_operation_envelope,
    validate_operation_lineage, validate_operation_transition,
};
pub use plugin::{
    AUTHORITY_COMMAND_PRODUCER, AUTHORITY_COMMAND_TYPE, DELEGATED_AUTHORITY_GRANT,
    INFORMATION_COMMAND, INFORMATION_INGRESS, INSTITUTIONAL_AUTHORITY_GRANT, InformationPlugin,
    PLUGIN_NAME, PLUGIN_NAMESPACE, information_authority_grants,
};
pub use query::{InformationQuery, InformationRecordSet};
pub use schema::{
    Access, Audience, AuthorityAssignment, Channel, Content, DeliveryAttempt, Dispatch,
    InformationOperationRecord, Instance, Interpretation, NeutralKnowledgeSchema, Release,
    Representation, information_knowledge_schemas, information_record_schemas,
    information_semantic_identity, neutral_knowledge_schemas,
};
