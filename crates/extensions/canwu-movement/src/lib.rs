//! Movement lifecycle domain extension for Canwu.
//!
//! `canwu-transport` defines period-neutral movement orders, transport
//! executions, itinerary revisions, handoffs, bookings, and capacity pools.
//! This extension is the lifecycle owner for them: a [`MovementPlugin`] keeps
//! every admitted order and execution in one [`MovementState`] domain record,
//! drives execution transitions only through the transport crate's transition
//! methods, settles each leg's departure and arrival at its due time, allocates
//! capacity pools deterministically, and publishes holder-relative movement
//! reports.
//!
//! Incidents, hazards, and hostility remain application systems. The plugin
//! never draws randomness: an application reports a leg failure, reroute, or
//! seizure handoff through [`MOVEMENT_COMMAND`] or, from its own systems,
//! through [`MOVEMENT_INCIDENT_INGRESS`] citing an exact evidence record.

#![allow(
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::too_many_lines
)]

mod lifecycle;
mod model;
mod plugin;
mod report;

pub use lifecycle::{MOVEMENT_GRANTEE_ROLE, MOVEMENT_SUBJECT_ROLE};
pub use model::{
    MOVEMENT_RUNTIME_ID, MovementBookingBindingV1, MovementBookingRequestV1, MovementCommandV1,
    MovementError, MovementErrorCode, MovementIncidentV1, MovementLegDueKind, MovementLegDueV1,
    MovementLimitsV1, MovementObservationHeadV1, MovementObserverGrant, MovementObserverRole,
    MovementOperation, MovementOperationDisposition, MovementOperationOutcomeV1,
    MovementOperationScopeV1, MovementOperationSourceV1, MovementOrderRecordV1,
    MovementOrderRequestV1, MovementPoolOfferV1, MovementRemoteObserverV1, MovementRuntimeRecord,
    MovementState, movement_runtime_reference,
};
pub use plugin::{
    MOVEMENT_COMMAND, MOVEMENT_INCIDENT_INGRESS, MOVEMENT_LEG_DUE_INGRESS,
    MOVEMENT_OPERATION_DROPPED_EVENT, MOVEMENT_OPERATION_INGRESS, MOVEMENT_REPORT_WAKE_INGRESS,
    MOVEMENT_REPORT_WITHHELD_EVENT, MOVEMENT_SEMANTIC_HASH, MovementPlugin, movement_command,
    movement_incident_ingress, movement_incident_target, movement_state,
};
pub use report::{
    MOVEMENT_REPORT_KNOWLEDGE, MovementLegFailureV1, MovementLegObservationV1, MovementPhaseV1,
    MovementReportDetailV1, MovementReportV1, MovementRevisionObservationV1, movement_report,
    movement_report_knowledge_schema_id,
};

/// Persisted plugin identity.
pub const PLUGIN_NAME: &str = "canwu-movement";
/// Namespace of the movement runtime record and report knowledge.
pub const PLUGIN_NAMESPACE: &str = "canwu.movement";
