//! Addressed correspondence orchestration for Canwu.
//!
//! This published domain extension binds decision-backed communication
//! intents to holder-relative address and route knowledge, the pure routing
//! mechanism, transport execution, and the neutral information lifecycle.
//!
//! # Carriers and disclosure
//!
//! Route planning and address resolution read the carrier holder's knowledge
//! ledger through the same bounded query and admission rule as
//! [`planning_snapshot_from_holder_knowledge`]. A carrier other than the
//! sender must first accept the carrying: under its own command authority it
//! issues [`CARRIER_DELEGATION_COMMAND`] (see [`carrier_delegation_command`])
//! with a delegation claim naming itself as `performed_by`, the sender as
//! `performed_for`, and [`CARRY_CORRESPONDENCE_CAPABILITY`]. The accepted
//! claim is persisted as the carrier's current [`CarrierDelegationRecord`]
//! for that sender; a newer delegation for the same sender replaces it. The
//! sender's correspondence then cites the current delegation command in
//! [`InitiateCorrespondenceRequest::carrier_delegation`], and the claim's
//! validity interval must cover every dispatch. The record is checked when
//! the initial or a retry command is admitted; when that dispatch settles,
//! only the admitted claim's validity interval is checked again, and
//! replanning the same attempt checks nothing. Because admission reads
//! the persisted record rather than command evidence, a run whose evidence
//! was sealed decides exactly as its replay. Naming another holder as carrier
//! without its current delegation is not read authority.
//!
//! Disclosure stays holder-relative. Dispatch publishes nothing to the sender
//! or the carrier: the accepted route and read cuts are system evidence on
//! the operation record, which restricted viewers cannot read, so the engine
//! publishes none of the carrier's knowledge to the sender. The sender still
//! observes consequences of the carrier's plan, such as when a delivery
//! arrives or whether the start succeeds. The carrier learns the outcome it
//! experiences: when a
//! [`CorrespondenceIncidentKind::CarrierSeized`] incident ends the attempt,
//! the carrier holder receives an [`ATTEMPT_REPORT_KNOWLEDGE_SCHEMA`] record
//! saying that the attempt ended by seizure, without naming the seizing
//! identity. A sender that delegated the carrying is not told by the engine;
//! the application relays the news when it reaches the sender. A carrier that
//! accepts no knowledge when the report settles (it died after the seizure,
//! or was retired) receives nothing; an [`ATTEMPT_REPORT_WITHHELD_EVENT`]
//! records the withheld report and the boundary stands.
//!
//! # Parties
//!
//! A sender, recipient, carrier, opportunity candidate, or seed holder may be
//! any knowledge holder, a domain entity included: the plugin's records name
//! a domain-entity party by its domain record and any other party by its core
//! identity, and the prepared dispatch may name its sender either way. A
//! domain-entity sender or carrier issues its commands under
//! `DecisionOrigin::Institution`. A retired party stays referenced, so this
//! plugin's history does not keep the party's owner from retiring it
//! (`canwu-information`'s dispatch and attempt records keep their own rule),
//! but it takes no new correspondence: a carrier the engine refuses as a
//! knowledge holder (retired, deleted, dead, or ineligible) is refused at
//! admission of a correspondence, a retry, or a carrier delegation, and a
//! start or retry whose carrier became so after admission is skipped
//! ([`START_SKIPPED_EVENT`] for a start; the retry's `last_error`).
//!
//! # Knowledge seeds
//!
//! A [`NetworkKnowledgeSeed`] settles once per `seed_key` with a
//! [`KnowledgeSeedReceipt`]. A seed whose holder accepts no knowledge, that
//! one publication directive cannot carry, or that states a connection no
//! planning network admits publishes nothing, and its receipt records the
//! [`KnowledgeSeedRefusal`].

#![allow(clippy::missing_errors_doc, clippy::too_many_lines)]

mod host;
mod knowledge;
mod model;
mod plugin;

pub use host::{
    carrier_delegation_command, correspondence_command, correspondence_decision_ticket,
    correspondence_recovery_decision_ticket, resolve_correspondence_command,
};
pub use knowledge::{
    ADDRESS_KNOWLEDGE_SCHEMA, ATTEMPT_REPORT_KNOWLEDGE_SCHEMA, CONNECTION_KNOWLEDGE_SCHEMA,
    ENDPOINT_KNOWLEDGE_SCHEMA, KnowledgeReadCutDigest, KnownAddress, KnownRoutingConnection,
    KnownRoutingEndpoint, NetworkKnowledgeSeed, correspondence_knowledge_schemas,
    planning_knowledge_query, planning_snapshot_from_holder_knowledge,
    planning_snapshot_from_knowledge_result,
};
pub use model::{
    AddressResolution, CarrierAuthority, CarrierDelegationRecord, CarrierDelegationRequest,
    CommunicationOpportunity, CommunicationOpportunityRecord, CommunicationOpportunityRequest,
    CommunicationOpportunityStatus, CorrespondenceAttemptOutcome, CorrespondenceAttemptReport,
    CorrespondenceAuthority, CorrespondenceCapacityAdmission, CorrespondenceIncident,
    CorrespondenceIncidentKind, CorrespondenceIncidentRequest, CorrespondenceIntent,
    CorrespondenceOperation, CorrespondenceOperationRecord, CorrespondencePlanningEvidence,
    CorrespondenceRecovery, CorrespondenceRecoveryAction, CorrespondenceStatus,
    InformationSagaStep, InitiateCorrespondenceRequest, KnowledgeSeedReceipt, KnowledgeSeedRecord,
    KnowledgeSeedRefusal, ProgressAction, ProgressRequest, ResolveCorrespondenceRequest,
    carrier_delegation_ref, correspondence_operation_ref, knowledge_seed_ref, opportunity_ref,
};
pub use plugin::{
    ATTEMPT_REPORT_WITHHELD_EVENT, CARRIER_DELEGATION_COMMAND, CARRY_CORRESPONDENCE_CAPABILITY,
    CORRESPONDENCE_COMMAND, CorrespondencePlugin, INCIDENT_INGRESS, KNOWLEDGE_INGRESS,
    OPPORTUNITY_INGRESS, PLUGIN_NAME, RESOLVE_CORRESPONDENCE_COMMAND, START_INGRESS,
    START_SKIPPED_EVENT,
};
