//! Deterministic runtime, validated commands, scheduling, plugins, and snapshots.

#![allow(
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::too_many_lines
)]

mod boundary;
mod ingress;
mod manifest;
mod policy;
mod random;
mod records;

pub use boundary::{
    BoundaryChange, BoundaryContext, BoundaryDirective, BoundaryEmission, BoundaryEmissionKind,
    BoundaryIngressGeneration, BoundaryProposal, BoundaryReceipt, BoundaryRecord, BoundaryRequest,
    BoundarySystemContract, BoundarySystemHandler, ReservationAllocation, ReservationDisposition,
    ReservationOffer, ReservationOfferRecord, ReservationPoolKey, ReservationRef,
    ReservationRequest, ReservationRequestRecord,
};
pub use ingress::{
    IngressClass, IngressPayload, IngressReceipt, IngressRecord, PluginIngressDescriptor,
    PluginIngressRequest,
};
pub use manifest::{ArtifactManifest, RUN_MANIFEST_FORMAT_VERSION, RunManifest};
pub use policy::{
    CommandPolicyContext, ControllerPolicy, InteractionPolicy, ObservationPolicy,
    RUN_CONFIGURATION_FORMAT_VERSION, RunConfiguration, RunConfigurationSnapshot, RunPurpose,
    SeatBinding, SeatPolicy, TracePolicy,
};
pub use random::{
    RandomAlgorithm, RandomDrawOutcome, RandomDrawProducer, RandomDrawRecord, RandomStreamKey,
    RandomStreamState,
};
pub use records::{
    DomainRecord, DomainRecordChange, DomainRecordClass, DomainRecordDraft, DomainRecordLifecycle,
    DomainRecordMutation, DomainRecordOperation, DomainRecordSchema, DomainReference,
    DomainReferenceSchema, DomainReferenceTarget, DomainReferenceTargetKind,
};

use canwu_core::{
    ArmyId, BoundaryId, CommandAttemptId, CommandId, CommandRequestId, DeterministicRng,
    DomainRecordKind, DomainRecordRef, EntityRef, EventId, FieldSchema, GovernmentId, IngressId,
    PersonId, RandomDrawId, RouteId, SchemaRegistry, TerritoryId, TypeSchema,
};
use canwu_event::{CauseRef, EventKind, SimEvent};
use canwu_knowledge::{
    ActorKnowledge, ArmyKnowledge, EstimateRange, KnowledgeSnapshot, KnowledgeSource,
};
use canwu_time::{SimDuration, SimTime};
use canwu_world::{
    Army, Government, MapPoint, Person, Route, Territory, TransitState, WorldSnapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::panic::{AssertUnwindSafe, catch_unwind};

use ingress::IngressQueueKey;

pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SNAPSHOT_FORMAT_VERSION: u32 = 4;
/// Version of the independently migrated authoritative revision commitment.
pub const STATE_REVISION_FORMAT_VERSION: u32 = 1;
/// Version of persisted monotonic boundary-admission cursors.
pub const ADMISSION_CURSOR_FORMAT_VERSION: u32 = 1;
/// Version of the domain-separated checkpoint commitment contract.
pub const COMMITMENT_FORMAT_VERSION: u32 = 1;
const CORE_STATE_NAMESPACE: &str = "canwu.core";
const GENESIS_BOUNDARY_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
/// Canonical roots for independent authoritative state and evidence domains.
pub struct CommitmentRoots {
    pub world: String,
    pub knowledge: String,
    pub plugin_components: String,
    pub domain_records: String,
    pub scheduler: String,
    pub commands: String,
    pub events: String,
    pub ingress: String,
    pub random: String,
    pub boundary_chain: String,
    pub identity: String,
    pub control: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    ActorNotFound,
    ArmyNotFound,
    DestinationNotFound,
    DuplicateBoundaryWriter,
    DuplicateDomainRecord,
    DuplicateDomainRecordKind,
    DuplicatePlugin,
    DuplicatePluginCommand,
    DuplicatePluginIngress,
    DuplicatePluginSystem,
    DuplicateStateOwner,
    DuplicateReservationOfferer,
    EntityNotFound,
    DomainRecordNotFound,
    DomainRecordReferenced,
    DomainRecordVersionConflict,
    InvalidAuthority,
    InvalidBoundary,
    InvalidDuration,
    InvalidDomainRecord,
    IdempotencyConflict,
    InteractionReadOnly,
    InvalidPayload,
    InvalidPluginRegistration,
    InvalidRandomDraw,
    InvalidRandomStream,
    InvalidRunConfiguration,
    InvalidRunManifest,
    InvalidSnapshot,
    IdentifierExhausted,
    LegacyReplayUnavailable,
    LateIngress,
    MissingIdempotencyKey,
    MixedCommandIngress,
    NoRoute,
    PluginCommandNotFound,
    PluginManifestMismatch,
    PluginNotActive,
    PluginPanicked,
    PluginRegistrationClosed,
    ReplayMismatch,
    ReplayEnvironmentMismatch,
    SimulationRevisionConflict,
    SimulationTimeConflict,
    UndeclaredRandomStream,
    UndeclaredStateRead,
    UndeclaredStateWrite,
    UnsupportedSnapshotVersion,
    ValueOutOfRange,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CanwuError {
    pub code: ErrorCode,
    pub message: String,
    pub related_entities: Vec<EntityRef>,
}

impl CanwuError {
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            related_entities: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_entity(mut self, entity: EntityRef) -> Self {
        self.related_entities.push(entity);
        self
    }
}

impl Display for CanwuError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}: {}",
            error_code_name(&self.code),
            self.message
        )
    }
}

impl Error for CanwuError {}

const fn error_code_name(code: &ErrorCode) -> &'static str {
    match code {
        ErrorCode::ActorNotFound => "actor_not_found",
        ErrorCode::ArmyNotFound => "army_not_found",
        ErrorCode::DestinationNotFound => "destination_not_found",
        ErrorCode::DuplicateBoundaryWriter => "duplicate_boundary_writer",
        ErrorCode::DuplicateDomainRecord => "duplicate_domain_record",
        ErrorCode::DuplicateDomainRecordKind => "duplicate_domain_record_kind",
        ErrorCode::DuplicatePlugin => "duplicate_plugin",
        ErrorCode::DuplicatePluginCommand => "duplicate_plugin_command",
        ErrorCode::DuplicatePluginIngress => "duplicate_plugin_ingress",
        ErrorCode::DuplicatePluginSystem => "duplicate_plugin_system",
        ErrorCode::DuplicateStateOwner => "duplicate_state_owner",
        ErrorCode::DuplicateReservationOfferer => "duplicate_reservation_offerer",
        ErrorCode::EntityNotFound => "entity_not_found",
        ErrorCode::DomainRecordNotFound => "domain_record_not_found",
        ErrorCode::DomainRecordReferenced => "domain_record_referenced",
        ErrorCode::DomainRecordVersionConflict => "domain_record_version_conflict",
        ErrorCode::InvalidAuthority => "invalid_authority",
        ErrorCode::InvalidBoundary => "invalid_boundary",
        ErrorCode::InvalidDuration => "invalid_duration",
        ErrorCode::InvalidDomainRecord => "invalid_domain_record",
        ErrorCode::IdempotencyConflict => "idempotency_conflict",
        ErrorCode::InteractionReadOnly => "interaction_read_only",
        ErrorCode::InvalidPayload => "invalid_payload",
        ErrorCode::InvalidPluginRegistration => "invalid_plugin_registration",
        ErrorCode::InvalidRandomDraw => "invalid_random_draw",
        ErrorCode::InvalidRandomStream => "invalid_random_stream",
        ErrorCode::InvalidRunConfiguration => "invalid_run_configuration",
        ErrorCode::InvalidRunManifest => "invalid_run_manifest",
        ErrorCode::InvalidSnapshot => "invalid_snapshot",
        ErrorCode::IdentifierExhausted => "identifier_exhausted",
        ErrorCode::LegacyReplayUnavailable => "legacy_replay_unavailable",
        ErrorCode::LateIngress => "late_ingress",
        ErrorCode::MissingIdempotencyKey => "missing_idempotency_key",
        ErrorCode::MixedCommandIngress => "mixed_command_ingress",
        ErrorCode::NoRoute => "no_route",
        ErrorCode::PluginCommandNotFound => "plugin_command_not_found",
        ErrorCode::PluginManifestMismatch => "plugin_manifest_mismatch",
        ErrorCode::PluginNotActive => "plugin_not_active",
        ErrorCode::PluginPanicked => "plugin_panicked",
        ErrorCode::PluginRegistrationClosed => "plugin_registration_closed",
        ErrorCode::ReplayMismatch => "replay_mismatch",
        ErrorCode::ReplayEnvironmentMismatch => "replay_environment_mismatch",
        ErrorCode::SimulationRevisionConflict => "simulation_revision_conflict",
        ErrorCode::SimulationTimeConflict => "simulation_time_conflict",
        ErrorCode::UndeclaredRandomStream => "undeclared_random_stream",
        ErrorCode::UndeclaredStateRead => "undeclared_state_read",
        ErrorCode::UndeclaredStateWrite => "undeclared_state_write",
        ErrorCode::UnsupportedSnapshotVersion => "unsupported_snapshot_version",
        ErrorCode::ValueOutOfRange => "value_out_of_range",
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum Issuer {
    Actor(PersonId),
    Human(String),
    Ai(String),
    Institution(String),
    Replay(String),
    Experiment(String),
    Debug,
    System(String),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandIngress {
    LegacyDirect,
    LiveRequest,
    FrozenReplay,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionOrigin {
    Actor {
        actor: PersonId,
    },
    Institution {
        institution: EntityRef,
        responsible_actor: Option<PersonId>,
    },
    Council {
        council_id: String,
    },
    NoResponsibleActor {
        reason: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandAuthority {
    pub decision_origin: DecisionOrigin,
    pub seat_id: Option<String>,
    pub permission_profile_id: Option<String>,
    pub command_subject: Option<EntityRef>,
}

impl CommandAuthority {
    #[must_use]
    pub const fn for_actor(actor: PersonId) -> Self {
        Self {
            decision_origin: DecisionOrigin::Actor { actor },
            seat_id: None,
            permission_profile_id: None,
            command_subject: None,
        }
    }

    #[must_use]
    pub fn no_responsible_actor(reason: impl Into<String>) -> Self {
        Self {
            decision_origin: DecisionOrigin::NoResponsibleActor {
                reason: reason.into(),
            },
            seat_id: None,
            permission_profile_id: None,
            command_subject: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandContext {
    pub issuer: Issuer,
    pub authority: CommandAuthority,
    pub run_policy: CommandPolicyContext,
    pub ingress: CommandIngress,
    pub attempt_id: Option<CommandAttemptId>,
    pub command_id: CommandId,
    pub request_id: Option<CommandRequestId>,
    pub revision: u64,
    pub simulation_time: SimTime,
    pub expected_revision: Option<u64>,
    pub expected_time: Option<SimTime>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryPhase {
    EventIngress = 1,
    BoundarySnapshot = 2,
    DerivedFieldSolve = 3,
    PerceptionAndAttentionRefresh = 4,
    DecisionAndAcceptedEffectIntake = 5,
    ReservationAndAllocation = 6,
    DomainDeltaProposal = 7,
    InvariantValidation = 8,
    AtomicDomainCommit = 9,
    HistoricalCandidateEvaluation = 10,
    ConditionalTransitionCommit = 11,
    StrategicAggregation = 12,
    PerspectiveAndReportMaterialization = 13,
    SaveReplayAndDiagnosticHashing = 14,
}

impl BoundaryPhase {
    pub const ALL: [Self; 14] = [
        Self::EventIngress,
        Self::BoundarySnapshot,
        Self::DerivedFieldSolve,
        Self::PerceptionAndAttentionRefresh,
        Self::DecisionAndAcceptedEffectIntake,
        Self::ReservationAndAllocation,
        Self::DomainDeltaProposal,
        Self::InvariantValidation,
        Self::AtomicDomainCommit,
        Self::HistoricalCandidateEvaluation,
        Self::ConditionalTransitionCommit,
        Self::StrategicAggregation,
        Self::PerspectiveAndReportMaterialization,
        Self::SaveReplayAndDiagnosticHashing,
    ];
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemCadence {
    EventDriven,
    SubDaily,
    Daily,
    Monthly,
    Seasonal,
    Annual,
    EraScheduled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StateVisibility {
    SameBoundary,
    NextBoundary,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct StateKey {
    pub namespace: String,
    pub name: String,
}

impl StateKey {
    #[must_use]
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
        }
    }

    #[must_use]
    pub fn core_people() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "people")
    }

    #[must_use]
    pub fn core_governments() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "governments")
    }

    #[must_use]
    pub fn core_territories() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "territories")
    }

    #[must_use]
    pub fn core_routes() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "routes")
    }

    #[must_use]
    pub fn core_armies() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "armies")
    }

    #[must_use]
    pub fn core_knowledge() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "knowledge")
    }

    #[must_use]
    pub fn core_commands() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "commands")
    }

    #[must_use]
    pub fn core_events() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "events")
    }

    #[must_use]
    pub fn core_ingress() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "ingress")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SystemContract {
    pub name: String,
    pub phase: BoundaryPhase,
    pub cadence: SystemCadence,
    pub reads: Vec<StateKey>,
    pub writes: Vec<StateKey>,
    pub visibility: StateVisibility,
}

impl SystemContract {
    #[must_use]
    pub fn event_driven(name: impl Into<String>, phase: BoundaryPhase) -> Self {
        Self {
            name: name.into(),
            phase,
            cadence: SystemCadence::EventDriven,
            reads: Vec::new(),
            writes: Vec::new(),
            visibility: StateVisibility::SameBoundary,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    MoveArmy {
        army: ArmyId,
        destination: TerritoryId,
    },
    DebugSetArmyMorale {
        army: ArmyId,
        morale: u16,
    },
    Plugin {
        plugin: String,
        command: String,
        payload: Value,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommandEnvelope {
    pub issuer: Issuer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<CommandAuthority>,
    pub command: Command,
    pub expected_time: Option<SimTime>,
}

impl CommandEnvelope {
    #[must_use]
    pub const fn new(issuer: Issuer, command: Command) -> Self {
        Self {
            issuer,
            authority: None,
            command,
            expected_time: None,
        }
    }

    #[must_use]
    pub const fn at_time(mut self, expected_time: SimTime) -> Self {
        self.expected_time = Some(expected_time);
        self
    }

    #[must_use]
    pub fn with_authority(mut self, authority: CommandAuthority) -> Self {
        self.authority = Some(authority);
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommandRequest {
    pub request_id: CommandRequestId,
    /// Must equal the persisted authoritative revision at command admission.
    ///
    /// Accepted commands, persisted expected rejections, and completed
    /// settlement boundaries advance the revision. Bare clock movement does
    /// not, so declared external commands also carry `envelope.expected_time`.
    pub expected_revision: u64,
    pub envelope: CommandEnvelope,
}

impl CommandRequest {
    #[must_use]
    pub const fn new(
        request_id: CommandRequestId,
        expected_revision: u64,
        envelope: CommandEnvelope,
    ) -> Self {
        Self {
            request_id,
            expected_revision,
            envelope,
        }
    }
}

#[derive(Clone, Copy)]
struct CommandAdmission {
    request_id: Option<CommandRequestId>,
    expected_revision: Option<u64>,
    expected_time: Option<SimTime>,
    revision_before: u64,
    ingress: CommandIngress,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommandRecord {
    pub id: CommandId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<CommandAttemptId>,
    pub accepted_at: SimTime,
    pub envelope: CommandEnvelope,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emitted_events: Vec<EventId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandReceipt {
    pub attempt_id: Option<CommandAttemptId>,
    pub command_id: CommandId,
    pub request_id: Option<CommandRequestId>,
    /// Authoritative revision after the accepted command commits.
    pub revision: u64,
    pub accepted_at: SimTime,
    pub emitted_events: Vec<EventId>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommandRejection {
    pub attempt_id: Option<CommandAttemptId>,
    pub request_id: Option<CommandRequestId>,
    /// Authoritative revision after persisted rejection evidence commits.
    /// Non-persisted conflicts retain the already committed current revision.
    pub retained_revision: u64,
    pub rejected_at: SimTime,
    pub error: CanwuError,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum CommandOutcome {
    Accepted { receipt: CommandReceipt },
    Rejected { rejection: CommandRejection },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum CommandAttemptOutcome {
    Accepted { command_id: CommandId },
    Rejected { error: CanwuError },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommandAttemptRecord {
    pub id: CommandAttemptId,
    pub at: SimTime,
    /// Authoritative revision immediately before this attempt transaction.
    pub revision_before: u64,
    pub ingress: CommandIngress,
    pub request_id: Option<CommandRequestId>,
    pub expected_revision: Option<u64>,
    pub envelope: CommandEnvelope,
    pub outcome: CommandAttemptOutcome,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DemoIds {
    pub commander: PersonId,
    pub observer: PersonId,
    pub government: GovernmentId,
    pub army: ArmyId,
    pub western_territory: TerritoryId,
    pub central_territory: TerritoryId,
    pub eastern_territory: TerritoryId,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Scenario {
    pub start_time: SimTime,
    pub world: WorldSnapshot,
    pub knowledge: KnowledgeSnapshot,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domain_records: Vec<DomainRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadValueType {
    Null,
    Boolean,
    Integer,
    String,
    Object,
    Array,
}

impl PayloadValueType {
    fn matches(&self, value: &Value) -> bool {
        match self {
            Self::Null => value.is_null(),
            Self::Boolean => value.is_boolean(),
            Self::Integer => value.as_i64().is_some() || value.as_u64().is_some(),
            Self::String => value.is_string(),
            Self::Object => value.is_object(),
            Self::Array => value.is_array(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PayloadProperty {
    pub value_type: PayloadValueType,
    pub required: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PayloadSchema {
    Any,
    Null,
    Boolean,
    Integer,
    String,
    Object {
        properties: BTreeMap<String, PayloadProperty>,
        allow_additional: bool,
    },
}

impl PayloadSchema {
    fn validate(&self, value: &Value) -> Result<(), CanwuError> {
        let scalar_matches = match self {
            Self::Any => return Ok(()),
            Self::Null => value.is_null(),
            Self::Boolean => value.is_boolean(),
            Self::Integer => value.as_i64().is_some() || value.as_u64().is_some(),
            Self::String => value.is_string(),
            Self::Object {
                properties,
                allow_additional,
            } => {
                let Some(object) = value.as_object() else {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidPayload,
                        "plugin command payload must be an object",
                    ));
                };
                for (name, property) in properties {
                    match object.get(name) {
                        Some(field) if !property.value_type.matches(field) => {
                            return Err(CanwuError::new(
                                ErrorCode::InvalidPayload,
                                format!("payload field {name} has the wrong type"),
                            ));
                        }
                        None if property.required => {
                            return Err(CanwuError::new(
                                ErrorCode::InvalidPayload,
                                format!("payload field {name} is required"),
                            ));
                        }
                        Some(_) | None => {}
                    }
                }
                if !allow_additional && object.keys().any(|name| !properties.contains_key(name)) {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidPayload,
                        "plugin command payload contains an undeclared field",
                    ));
                }
                return Ok(());
            }
        };
        if scalar_matches {
            Ok(())
        } else {
            Err(CanwuError::new(
                ErrorCode::InvalidPayload,
                "plugin command payload does not match its declared schema",
            ))
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PluginActionDescriptor {
    pub name: String,
    pub description: String,
    pub payload_schema: PayloadSchema,
    pub reads: Vec<StateKey>,
    pub writes: Vec<StateKey>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PluginDescriptor {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub semantic_hash: String,
    pub systems: Vec<SystemContract>,
    #[serde(default)]
    pub boundary_systems: Vec<BoundarySystemContract>,
    pub commands: Vec<PluginActionDescriptor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ingress: Vec<PluginIngressDescriptor>,
    pub schema_types: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub record_schemas: Vec<DomainRecordSchema>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PluginComponentRecord {
    pub plugin: String,
    pub state: StateKey,
    pub entity: EntityRef,
    pub component: String,
    pub value: Value,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct PluginComponentKey {
    plugin: String,
    state: StateKey,
    entity: EntityRef,
    component: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SystemDirective {
    SetComponent {
        state: StateKey,
        entity: EntityRef,
        component: String,
        value: Value,
        summary: String,
    },
    Emit {
        event_type: String,
        summary: String,
        affected: Vec<EntityRef>,
    },
    Schedule {
        after: SimDuration,
        directive: Box<SystemDirective>,
    },
}

pub struct SimulationView<'a> {
    state: &'a RuntimeState,
    state_owners: &'a BTreeMap<StateKey, String>,
    reader: Option<&'a str>,
    allowed_reads: Option<&'a [StateKey]>,
    allowed_ingress: Option<&'a HashSet<IngressId>>,
    ingress_plugin: Option<&'a str>,
    component_overlay: Option<&'a BTreeMap<PluginComponentKey, PluginComponentRecord>>,
    proposed_components: Option<&'a BTreeMap<PluginComponentKey, PluginComponentRecord>>,
    record_overlay: Option<&'a BTreeMap<DomainRecordRef, DomainRecord>>,
    proposed_records: Option<&'a BTreeMap<DomainRecordRef, DomainRecord>>,
    allocations: Option<&'a BTreeMap<ReservationRef, ReservationAllocation>>,
    allowed_reservations: Option<&'a [ReservationRef]>,
    random_session: Option<RefCell<random::RandomSession>>,
}

impl SimulationView<'_> {
    #[must_use]
    pub const fn time(&self) -> SimTime {
        self.state.scheduler.now
    }

    pub fn army(&self, id: ArmyId) -> Result<Option<&Army>, CanwuError> {
        self.require_read(&StateKey::core_armies())?;
        Ok(self.state.current.armies.get(&id))
    }

    pub fn person(&self, id: PersonId) -> Result<Option<&Person>, CanwuError> {
        self.require_read(&StateKey::core_people())?;
        Ok(self.state.current.people.get(&id))
    }

    pub fn government(&self, id: GovernmentId) -> Result<Option<&Government>, CanwuError> {
        self.require_read(&StateKey::core_governments())?;
        Ok(self.state.current.governments.get(&id))
    }

    pub fn territory(&self, id: TerritoryId) -> Result<Option<&Territory>, CanwuError> {
        self.require_read(&StateKey::core_territories())?;
        Ok(self.state.current.territories.get(&id))
    }

    pub fn route(&self, id: RouteId) -> Result<Option<&Route>, CanwuError> {
        self.require_read(&StateKey::core_routes())?;
        Ok(self.state.current.routes.get(&id))
    }

    pub fn actor_knowledge(&self, actor: PersonId) -> Result<Option<&ActorKnowledge>, CanwuError> {
        self.require_read(&StateKey::core_knowledge())?;
        Ok(self.state.current.knowledge.for_actor(actor))
    }

    pub fn command(&self, id: CommandId) -> Result<Option<&CommandRecord>, CanwuError> {
        self.require_read(&StateKey::core_commands())?;
        Ok(self
            .state
            .evidence
            .commands
            .iter()
            .find(|record| record.id == id))
    }

    pub fn event(&self, id: EventId) -> Result<Option<&SimEvent>, CanwuError> {
        self.require_read(&StateKey::core_events())?;
        Ok(self
            .state
            .evidence
            .events
            .iter()
            .find(|event| event.id == id))
    }

    pub fn ingress(&self, id: IngressId) -> Result<Option<&IngressRecord>, CanwuError> {
        self.require_read(&StateKey::core_ingress())?;
        if self
            .allowed_ingress
            .is_none_or(|allowed| !allowed.contains(&id))
        {
            return Ok(None);
        }
        let record = id
            .get()
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| self.state.evidence.ingress.get(index))
            .filter(|record| record.id == id);
        if let (Some(owner), Some(record)) = (self.ingress_plugin, record)
            && !matches!(
                &record.payload,
                IngressPayload::Plugin { plugin, .. } if plugin == owner
            )
        {
            return Ok(None);
        }
        Ok(record)
    }

    pub fn domain_record(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<&DomainRecord>, CanwuError> {
        self.require_read(&records::record_state_key(&reference.kind))?;
        Ok(self
            .record_overlay
            .and_then(|overlay| overlay.get(reference))
            .or_else(|| self.state.current.domain_records.get(reference)))
    }

    pub fn proposed_domain_record(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<&DomainRecord>, CanwuError> {
        self.require_read(&records::record_state_key(&reference.kind))?;
        Ok(self
            .proposed_records
            .and_then(|records| records.get(reference)))
    }

    pub fn reservation(
        &self,
        reservation: &ReservationRef,
    ) -> Result<Option<&ReservationAllocation>, CanwuError> {
        let reader = self.reader.unwrap_or("unscoped caller");
        if self
            .allowed_reservations
            .is_none_or(|allowed| !allowed.contains(reservation))
        {
            return Err(CanwuError::new(
                ErrorCode::UndeclaredStateRead,
                format!(
                    "system {reader} did not declare reservation read {}.{}.{}",
                    reservation.plugin, reservation.system, reservation.request
                ),
            ));
        }
        Ok(self.allocations.and_then(|values| values.get(reservation)))
    }

    pub fn random_range(
        &self,
        stream: &RandomStreamKey,
        upper_exclusive: u64,
        purpose: &str,
    ) -> Result<u64, CanwuError> {
        let Some(session) = &self.random_session else {
            return Err(CanwuError::new(
                ErrorCode::UndeclaredRandomStream,
                format!(
                    "system {} has no declared random streams",
                    self.reader.unwrap_or("unscoped caller")
                ),
            ));
        };
        session.borrow_mut().range(stream, upper_exclusive, purpose)
    }

    pub fn component(
        &self,
        state: &StateKey,
        entity: &EntityRef,
        component: &str,
    ) -> Result<Option<&Value>, CanwuError> {
        self.require_read(state)?;
        let Some(owner) = self.state_owners.get(state) else {
            return Err(CanwuError::new(
                ErrorCode::UndeclaredStateRead,
                format!(
                    "state {}.{} has no registered owner",
                    state.namespace, state.name
                ),
            ));
        };
        let key = component_key(owner, state, entity, component);
        Ok(self
            .component_overlay
            .and_then(|overlay| overlay.get(&key))
            .or_else(|| self.state.current.plugin_components.get(&key))
            .map(|record| &record.value))
    }

    pub fn proposed_component(
        &self,
        state: &StateKey,
        entity: &EntityRef,
        component: &str,
    ) -> Result<Option<&Value>, CanwuError> {
        self.require_read(state)?;
        let Some(owner) = self.state_owners.get(state) else {
            return Err(CanwuError::new(
                ErrorCode::UndeclaredStateRead,
                format!(
                    "state {}.{} has no registered owner",
                    state.namespace, state.name
                ),
            ));
        };
        let key = component_key(owner, state, entity, component);
        Ok(self
            .proposed_components
            .and_then(|proposals| proposals.get(&key))
            .map(|record| &record.value))
    }

    fn require_read(&self, state: &StateKey) -> Result<(), CanwuError> {
        if self
            .allowed_reads
            .is_some_and(|reads| !reads.contains(state))
        {
            return Err(CanwuError::new(
                ErrorCode::UndeclaredStateRead,
                format!(
                    "{} did not declare read access to {}.{}",
                    self.reader.unwrap_or("internal system"),
                    state.namespace,
                    state.name
                ),
            ));
        }
        Ok(())
    }

    fn finish_random_session(self) -> Option<random::RandomExecution> {
        self.random_session
            .map(RefCell::into_inner)
            .map(random::RandomSession::finish)
    }
}

pub type SimulationSystemHandler =
    fn(&SimulationView<'_>, &SimEvent) -> Result<Vec<SystemDirective>, CanwuError>;

pub type PluginCommandHandler =
    fn(&SimulationView<'_>, &CommandContext, &Value) -> Result<Vec<SystemDirective>, CanwuError>;

/// A stateless executable package whose persisted identity must change whenever
/// its authoritative behavior changes.
pub trait SimulationPlugin {
    fn name(&self) -> &str;
    /// Returns the package or rules release recorded in snapshots.
    fn version(&self) -> &str;
    /// Returns a lowercase 64-character author-controlled semantic hash.
    ///
    /// This must change when handler behavior changes even if the serialized
    /// registration descriptor remains structurally identical.
    fn semantic_hash(&self) -> &str;
    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError>;
}

#[derive(Clone, Default)]
pub struct PluginRegistry {
    descriptors: BTreeMap<String, PluginDescriptor>,
    active_plugins: BTreeSet<String>,
    systems: Vec<RegisteredSystem>,
    boundary_systems: Vec<RegisteredBoundarySystem>,
    commands: BTreeMap<(String, String), RegisteredCommand>,
    ingress: BTreeMap<(String, String), PluginIngressDescriptor>,
    state_owners: BTreeMap<StateKey, String>,
    immediate_write_states: BTreeMap<StateKey, String>,
    boundary_writers: BTreeMap<(BoundaryWriteStage, StateKey), (String, String)>,
    reservation_offerers: BTreeMap<StateKey, (String, String)>,
    random_stream_owners: BTreeMap<RandomStreamKey, (String, String)>,
    record_schemas: records::DomainRecordSchemas,
}

#[derive(Clone)]
struct RegisteredSystem {
    plugin: String,
    contract: SystemContract,
    handler: SimulationSystemHandler,
}

#[derive(Clone)]
struct RegisteredBoundarySystem {
    plugin: String,
    contract: BoundarySystemContract,
    handler: BoundarySystemHandler,
}

#[derive(Clone)]
struct RegisteredCommand {
    descriptor: PluginActionDescriptor,
    handler: PluginCommandHandler,
}

pub struct PluginRegistrar<'a> {
    plugin: String,
    registry: &'a mut PluginRegistry,
    schema: &'a mut SchemaRegistry,
}

impl PluginRegistrar<'_> {
    pub fn register_record_schema(
        &mut self,
        mut schema: DomainRecordSchema,
    ) -> Result<(), CanwuError> {
        schema.canonicalize();
        schema.validate().map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!("invalid domain record schema: {error}"),
            )
        })?;
        let state = schema.state_key();
        if state.namespace == CORE_STATE_NAMESPACE {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "plugins cannot register domain record kinds in the core namespace",
            ));
        }
        if self
            .registry
            .descriptors
            .get(&self.plugin)
            .is_some_and(|descriptor| {
                descriptor
                    .record_schemas
                    .iter()
                    .any(|candidate| candidate.kind == schema.kind)
            })
        {
            return Err(CanwuError::new(
                ErrorCode::DuplicateDomainRecordKind,
                format!(
                    "plugin {} registered record kind {} twice",
                    self.plugin, schema.kind
                ),
            ));
        }
        if let Some((owner, existing)) = self.registry.record_schemas.get(&schema.kind) {
            if owner != &self.plugin {
                return Err(CanwuError::new(
                    ErrorCode::DuplicateDomainRecordKind,
                    format!(
                        "domain record kind {} is already owned by plugin {owner}",
                        schema.kind
                    ),
                ));
            }
            if existing != &schema {
                return Err(CanwuError::new(
                    ErrorCode::PluginManifestMismatch,
                    format!(
                        "plugin {} changed the stored schema for domain record kind {}",
                        self.plugin, schema.kind
                    ),
                ));
            }
        }
        let mut candidate = self.registry.clone();
        if candidate.immediate_write_states.contains_key(&state) {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!(
                    "domain record kind {} is already exposed as immediate component state",
                    schema.kind
                ),
            ));
        }
        register_state_owners(
            &mut candidate.state_owners,
            &self.plugin,
            std::slice::from_ref(&state),
        )?;
        candidate
            .record_schemas
            .insert(schema.kind.clone(), (self.plugin.clone(), schema.clone()));
        let descriptor = candidate
            .descriptors
            .entry(self.plugin.clone())
            .or_default();
        descriptor.name.clone_from(&self.plugin);
        descriptor.record_schemas.push(schema);
        descriptor
            .record_schemas
            .sort_by(|left, right| left.kind.cmp(&right.kind));
        *self.registry = candidate;
        Ok(())
    }

    pub fn register_schema(&mut self, schema: TypeSchema) -> Result<(), CanwuError> {
        validate_type_schema(&schema)?;
        let type_name = schema.type_name.clone();
        let mut candidate_schema = self.schema.clone();
        let mut candidate_registry = self.registry.clone();
        if let Some(existing) = candidate_schema.get(&type_name) {
            if existing != &schema {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPluginRegistration,
                    format!(
                        "schema type {type_name} is already registered with a different definition"
                    ),
                ));
            }
        } else {
            candidate_schema.register(schema);
        }
        let descriptor = candidate_registry
            .descriptors
            .entry(self.plugin.clone())
            .or_default();
        if descriptor.schema_types.contains(&type_name) {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!(
                    "plugin {} registered schema type {} more than once",
                    self.plugin, type_name
                ),
            ));
        }
        descriptor.name.clone_from(&self.plugin);
        descriptor.schema_types.push(type_name);
        descriptor.schema_types.sort();
        *self.schema = candidate_schema;
        *self.registry = candidate_registry;
        Ok(())
    }

    pub fn register_system(
        &mut self,
        mut contract: SystemContract,
        handler: SimulationSystemHandler,
    ) -> Result<(), CanwuError> {
        validate_system_contract(&self.plugin, &mut contract)?;
        if self
            .registry
            .descriptors
            .get(&self.plugin)
            .is_some_and(|descriptor| {
                descriptor
                    .systems
                    .iter()
                    .any(|candidate| candidate.name == contract.name)
                    || descriptor
                        .boundary_systems
                        .iter()
                        .any(|candidate| candidate.name == contract.name)
            })
        {
            return Err(CanwuError::new(
                ErrorCode::DuplicatePluginSystem,
                format!(
                    "plugin {} already registered system {}",
                    self.plugin, contract.name
                ),
            ));
        }
        let mut candidate = self.registry.clone();
        if contract
            .writes
            .iter()
            .any(|state| is_domain_record_state(&candidate.record_schemas, state))
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "domain record kinds can only be mutated by phased boundary systems",
            ));
        }
        register_state_owners(&mut candidate.state_owners, &self.plugin, &contract.writes)?;
        register_immediate_write_states(
            &mut candidate.immediate_write_states,
            &candidate.boundary_writers,
            &self.plugin,
            &contract.writes,
        )?;
        {
            let descriptor = candidate
                .descriptors
                .entry(self.plugin.clone())
                .or_default();
            descriptor.name.clone_from(&self.plugin);
            descriptor.systems.push(contract.clone());
            descriptor
                .systems
                .sort_by(|left, right| (left.phase, &left.name).cmp(&(right.phase, &right.name)));
        }
        candidate.systems.push(RegisteredSystem {
            plugin: self.plugin.clone(),
            contract,
            handler,
        });
        candidate.systems.sort_by(|left, right| {
            (left.contract.phase, &left.plugin, &left.contract.name).cmp(&(
                right.contract.phase,
                &right.plugin,
                &right.contract.name,
            ))
        });
        *self.registry = candidate;
        Ok(())
    }

    pub fn register_boundary_system(
        &mut self,
        mut contract: BoundarySystemContract,
        handler: BoundarySystemHandler,
    ) -> Result<(), CanwuError> {
        validate_boundary_system_contract(&mut contract)?;
        if self
            .registry
            .descriptors
            .get(&self.plugin)
            .is_some_and(|descriptor| {
                descriptor
                    .systems
                    .iter()
                    .any(|candidate| candidate.name == contract.name)
                    || descriptor
                        .boundary_systems
                        .iter()
                        .any(|candidate| candidate.name == contract.name)
            })
        {
            return Err(CanwuError::new(
                ErrorCode::DuplicatePluginSystem,
                format!(
                    "plugin {} already registered system {}",
                    self.plugin, contract.name
                ),
            ));
        }
        let mut owned_state = contract.writes.clone();
        owned_state.extend(contract.reservation_offers.iter().cloned());
        owned_state.sort();
        owned_state.dedup();
        let mut candidate = self.registry.clone();
        register_state_owners(&mut candidate.state_owners, &self.plugin, &owned_state)?;
        register_boundary_writers(
            &mut candidate.boundary_writers,
            &candidate.immediate_write_states,
            &self.plugin,
            &contract.name,
            contract.phase,
            &contract.writes,
        )?;
        register_reservation_offerers(
            &mut candidate.reservation_offerers,
            &self.plugin,
            &contract.name,
            &contract.reservation_offers,
        )?;
        register_random_streams(
            &mut candidate.random_stream_owners,
            &self.plugin,
            &contract.name,
            &contract.random_streams,
        )?;
        {
            let descriptor = candidate
                .descriptors
                .entry(self.plugin.clone())
                .or_default();
            descriptor.name.clone_from(&self.plugin);
            descriptor.boundary_systems.push(contract.clone());
            descriptor
                .boundary_systems
                .sort_by(|left, right| (left.phase, &left.name).cmp(&(right.phase, &right.name)));
        }
        candidate.boundary_systems.push(RegisteredBoundarySystem {
            plugin: self.plugin.clone(),
            contract,
            handler,
        });
        candidate.boundary_systems.sort_by(|left, right| {
            (left.contract.phase, &left.plugin, &left.contract.name).cmp(&(
                right.contract.phase,
                &right.plugin,
                &right.contract.name,
            ))
        });
        *self.registry = candidate;
        Ok(())
    }

    pub fn register_command(
        &mut self,
        mut descriptor: PluginActionDescriptor,
        handler: PluginCommandHandler,
    ) -> Result<(), CanwuError> {
        validate_action_descriptor(&self.plugin, &mut descriptor)?;
        let command_key = (self.plugin.clone(), descriptor.name.clone());
        if self.registry.commands.contains_key(&command_key) {
            return Err(CanwuError::new(
                ErrorCode::DuplicatePluginCommand,
                format!(
                    "plugin {} already registered command {}",
                    self.plugin, descriptor.name
                ),
            ));
        }
        let mut candidate = self.registry.clone();
        if descriptor
            .writes
            .iter()
            .any(|state| is_domain_record_state(&candidate.record_schemas, state))
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "plugin commands cannot write domain record state directly",
            ));
        }
        register_state_owners(
            &mut candidate.state_owners,
            &self.plugin,
            &descriptor.writes,
        )?;
        register_immediate_write_states(
            &mut candidate.immediate_write_states,
            &candidate.boundary_writers,
            &self.plugin,
            &descriptor.writes,
        )?;
        {
            let plugin_descriptor = candidate
                .descriptors
                .entry(self.plugin.clone())
                .or_default();
            plugin_descriptor.name.clone_from(&self.plugin);
            plugin_descriptor.commands.push(descriptor.clone());
            plugin_descriptor
                .commands
                .sort_by(|left, right| left.name.cmp(&right.name));
        }
        candidate.commands.insert(
            command_key,
            RegisteredCommand {
                descriptor,
                handler,
            },
        );
        *self.registry = candidate;
        Ok(())
    }

    pub fn register_ingress(
        &mut self,
        descriptor: PluginIngressDescriptor,
    ) -> Result<(), CanwuError> {
        validate_ingress_descriptor(&descriptor)?;
        let key = (self.plugin.clone(), descriptor.name.clone());
        if self
            .registry
            .descriptors
            .get(&self.plugin)
            .is_some_and(|plugin| {
                plugin
                    .ingress
                    .iter()
                    .any(|candidate| candidate.name == descriptor.name)
            })
        {
            return Err(CanwuError::new(
                ErrorCode::DuplicatePluginIngress,
                format!(
                    "plugin {} already registered ingress type {}",
                    self.plugin, descriptor.name
                ),
            ));
        }
        if self
            .registry
            .ingress
            .get(&key)
            .is_some_and(|existing| existing != &descriptor)
        {
            return Err(CanwuError::new(
                ErrorCode::PluginManifestMismatch,
                format!(
                    "plugin {} changed the stored ingress type {}",
                    self.plugin, descriptor.name
                ),
            ));
        }
        let mut candidate = self.registry.clone();
        candidate.ingress.insert(key, descriptor.clone());
        let plugin_descriptor = candidate
            .descriptors
            .entry(self.plugin.clone())
            .or_default();
        plugin_descriptor.name.clone_from(&self.plugin);
        plugin_descriptor.ingress.push(descriptor);
        plugin_descriptor
            .ingress
            .sort_by(|left, right| left.name.cmp(&right.name));
        *self.registry = candidate;
        Ok(())
    }
}

impl PluginRegistry {
    pub fn register<P: SimulationPlugin + ?Sized>(
        &mut self,
        plugin: &P,
        schema: &mut SchemaRegistry,
    ) -> Result<(), CanwuError> {
        let raw_plugin_name = plugin.name();
        let plugin_name = raw_plugin_name.trim();
        if plugin_name.is_empty() || plugin_name != raw_plugin_name {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "plugin name must be non-empty and have no surrounding whitespace",
            ));
        }
        if self.active_plugins.contains(plugin_name) {
            return Err(CanwuError::new(
                ErrorCode::DuplicatePlugin,
                format!("plugin {plugin_name} is already registered"),
            ));
        }
        validate_plugin_identity(plugin_name, plugin.version(), plugin.semantic_hash())?;

        let expected_descriptor = self.descriptors.get(plugin_name).cloned();
        let mut candidate_registry = self.clone();
        let mut candidate_schema = schema.clone();
        candidate_registry.descriptors.insert(
            plugin_name.to_owned(),
            PluginDescriptor {
                name: plugin_name.to_owned(),
                version: plugin.version().to_owned(),
                semantic_hash: plugin.semantic_hash().to_owned(),
                ..PluginDescriptor::default()
            },
        );
        let mut registrar = PluginRegistrar {
            plugin: plugin_name.to_owned(),
            registry: &mut candidate_registry,
            schema: &mut candidate_schema,
        };
        plugin.register(&mut registrar)?;
        let Some(generated_descriptor) = candidate_registry.descriptors.get(plugin_name) else {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!("plugin {plugin_name} did not produce a descriptor"),
            ));
        };
        if let Some(expected) = expected_descriptor
            && generated_descriptor != &expected
        {
            return Err(CanwuError::new(
                ErrorCode::PluginManifestMismatch,
                format!("plugin {plugin_name} registration does not match the snapshot manifest"),
            ));
        }
        candidate_registry
            .active_plugins
            .insert(plugin_name.to_owned());
        *self = candidate_registry;
        *schema = candidate_schema;
        Ok(())
    }

    pub fn descriptors(&self) -> impl Iterator<Item = &PluginDescriptor> {
        self.descriptors.values()
    }

    fn from_descriptors(descriptors: Vec<PluginDescriptor>) -> Result<Self, CanwuError> {
        let mut registry = Self {
            descriptors: BTreeMap::new(),
            active_plugins: BTreeSet::new(),
            systems: Vec::new(),
            boundary_systems: Vec::new(),
            commands: BTreeMap::new(),
            ingress: BTreeMap::new(),
            state_owners: BTreeMap::new(),
            immediate_write_states: BTreeMap::new(),
            boundary_writers: BTreeMap::new(),
            reservation_offerers: BTreeMap::new(),
            random_stream_owners: BTreeMap::new(),
            record_schemas: BTreeMap::new(),
        };
        let mut previous_plugin = None;
        for mut descriptor in descriptors {
            let plugin = descriptor.name.trim().to_owned();
            if plugin.is_empty()
                || descriptor.name != plugin
                || descriptor.version.trim().is_empty()
                || descriptor.version != descriptor.version.trim()
                || !is_canonical_hash(&descriptor.semantic_hash)
                || registry.descriptors.contains_key(&plugin)
                || previous_plugin
                    .as_ref()
                    .is_some_and(|previous| previous >= &plugin)
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    "snapshot contains an invalid, unversioned, or duplicate plugin descriptor",
                ));
            }
            if descriptor
                .record_schemas
                .windows(2)
                .any(|pair| pair[0].kind >= pair[1].kind)
            {
                return invalid_snapshot("plugin record schemas are not in canonical order");
            }
            for schema in &mut descriptor.record_schemas {
                let original = schema.clone();
                schema.canonicalize();
                schema.validate().map_err(|error| {
                    invalid_snapshot_error(format!("invalid domain record schema: {error}"))
                })?;
                if *schema != original {
                    return invalid_snapshot(
                        "plugin record-schema declarations are not in canonical order",
                    );
                }
                let state = schema.state_key();
                if state.namespace == CORE_STATE_NAMESPACE {
                    return invalid_snapshot(
                        "plugin record schemas cannot use the reserved core namespace",
                    );
                }
                if let Some((owner, _)) = registry.record_schemas.get(&schema.kind) {
                    return invalid_snapshot(format!(
                        "domain record kind {} is owned by both {owner} and {plugin}",
                        schema.kind
                    ));
                }
                register_state_owners(
                    &mut registry.state_owners,
                    &plugin,
                    std::slice::from_ref(&state),
                )
                .map_err(|error| {
                    invalid_snapshot_error(format!(
                        "invalid domain record state ownership descriptor: {error}"
                    ))
                })?;
                registry
                    .record_schemas
                    .insert(schema.kind.clone(), (plugin.clone(), schema.clone()));
            }
            if descriptor
                .systems
                .windows(2)
                .any(|pair| (pair[0].phase, &pair[0].name) >= (pair[1].phase, &pair[1].name))
            {
                return invalid_snapshot("plugin systems are not in canonical order");
            }
            let mut system_names = BTreeSet::new();
            for contract in &mut descriptor.systems {
                if !system_names.insert(contract.name.clone()) {
                    return invalid_snapshot("plugin descriptor has duplicate system names");
                }
                let original = contract.clone();
                validate_system_contract(&plugin, contract).map_err(|error| {
                    invalid_snapshot_error(format!("invalid plugin system descriptor: {error}"))
                })?;
                if *contract != original {
                    return invalid_snapshot(
                        "plugin system reads and writes are not in canonical order",
                    );
                }
                if contract
                    .writes
                    .iter()
                    .any(|state| is_domain_record_state(&registry.record_schemas, state))
                {
                    return invalid_snapshot(
                        "plugin systems cannot expose domain records as immediate component state",
                    );
                }
                register_state_owners(&mut registry.state_owners, &plugin, &contract.writes)
                    .map_err(|error| {
                        invalid_snapshot_error(format!(
                            "invalid plugin state ownership descriptor: {error}"
                        ))
                    })?;
                register_immediate_write_states(
                    &mut registry.immediate_write_states,
                    &registry.boundary_writers,
                    &plugin,
                    &contract.writes,
                )
                .map_err(|error| {
                    invalid_snapshot_error(format!(
                        "invalid immediate state writer descriptor: {error}"
                    ))
                })?;
            }
            if descriptor
                .boundary_systems
                .windows(2)
                .any(|pair| (pair[0].phase, &pair[0].name) >= (pair[1].phase, &pair[1].name))
            {
                return invalid_snapshot("boundary systems are not in canonical order");
            }
            for contract in &mut descriptor.boundary_systems {
                if !system_names.insert(contract.name.clone()) {
                    return invalid_snapshot("plugin descriptor has duplicate system names");
                }
                let original = contract.clone();
                validate_boundary_system_contract(contract).map_err(|error| {
                    invalid_snapshot_error(format!("invalid boundary system descriptor: {error}"))
                })?;
                if *contract != original {
                    return invalid_snapshot(
                        "boundary system declarations are not in canonical order",
                    );
                }
                let mut owned_state = contract.writes.clone();
                owned_state.extend(contract.reservation_offers.iter().cloned());
                owned_state.sort();
                owned_state.dedup();
                register_state_owners(&mut registry.state_owners, &plugin, &owned_state).map_err(
                    |error| {
                        invalid_snapshot_error(format!(
                            "invalid boundary state ownership descriptor: {error}"
                        ))
                    },
                )?;
                register_boundary_writers(
                    &mut registry.boundary_writers,
                    &registry.immediate_write_states,
                    &plugin,
                    &contract.name,
                    contract.phase,
                    &contract.writes,
                )
                .map_err(|error| {
                    invalid_snapshot_error(format!("invalid boundary writer descriptor: {error}"))
                })?;
                register_reservation_offerers(
                    &mut registry.reservation_offerers,
                    &plugin,
                    &contract.name,
                    &contract.reservation_offers,
                )
                .map_err(|error| {
                    invalid_snapshot_error(format!(
                        "invalid reservation offerer descriptor: {error}"
                    ))
                })?;
                register_random_streams(
                    &mut registry.random_stream_owners,
                    &plugin,
                    &contract.name,
                    &contract.random_streams,
                )
                .map_err(|error| {
                    invalid_snapshot_error(format!(
                        "invalid random stream ownership descriptor: {error}"
                    ))
                })?;
            }
            if descriptor
                .commands
                .windows(2)
                .any(|pair| pair[0].name >= pair[1].name)
            {
                return invalid_snapshot("plugin commands are not in canonical order");
            }
            let mut command_names = BTreeSet::new();
            for action in &mut descriptor.commands {
                if !command_names.insert(action.name.clone()) {
                    return invalid_snapshot("plugin descriptor has duplicate command names");
                }
                let original = action.clone();
                validate_action_descriptor(&plugin, action).map_err(|error| {
                    invalid_snapshot_error(format!("invalid plugin command descriptor: {error}"))
                })?;
                if *action != original {
                    return invalid_snapshot(
                        "plugin command reads and writes are not in canonical order",
                    );
                }
                if action
                    .writes
                    .iter()
                    .any(|state| is_domain_record_state(&registry.record_schemas, state))
                {
                    return invalid_snapshot(
                        "plugin commands cannot expose domain records as immediate component state",
                    );
                }
                register_state_owners(&mut registry.state_owners, &plugin, &action.writes)
                    .map_err(|error| {
                        invalid_snapshot_error(format!(
                            "invalid plugin state ownership descriptor: {error}"
                        ))
                    })?;
                register_immediate_write_states(
                    &mut registry.immediate_write_states,
                    &registry.boundary_writers,
                    &plugin,
                    &action.writes,
                )
                .map_err(|error| {
                    invalid_snapshot_error(format!(
                        "invalid immediate state writer descriptor: {error}"
                    ))
                })?;
            }
            if descriptor
                .ingress
                .windows(2)
                .any(|pair| pair[0].name >= pair[1].name)
            {
                return invalid_snapshot("plugin ingress types are not in canonical order");
            }
            for ingress in &descriptor.ingress {
                validate_ingress_descriptor(ingress).map_err(|error| {
                    invalid_snapshot_error(format!("invalid plugin ingress descriptor: {error}"))
                })?;
                if registry
                    .ingress
                    .insert((plugin.clone(), ingress.name.clone()), ingress.clone())
                    .is_some()
                {
                    return invalid_snapshot("plugin descriptor has duplicate ingress types");
                }
            }
            let schema_types: BTreeSet<_> = descriptor.schema_types.iter().collect();
            if schema_types.len() != descriptor.schema_types.len()
                || descriptor
                    .schema_types
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
                || descriptor
                    .schema_types
                    .iter()
                    .any(|name| name.trim().is_empty() || name != name.trim())
            {
                return invalid_snapshot("plugin descriptor has invalid schema type names");
            }
            previous_plugin = Some(plugin.clone());
            registry.descriptors.insert(plugin, descriptor);
        }
        Ok(registry)
    }

    fn ensure_active(&self) -> Result<(), CanwuError> {
        let inactive: Vec<_> = self
            .descriptors
            .keys()
            .filter(|name| !self.active_plugins.contains(*name))
            .cloned()
            .collect();
        if inactive.is_empty() {
            return Ok(());
        }
        Err(CanwuError::new(
            ErrorCode::PluginNotActive,
            format!(
                "required plugin handlers are not active: {}",
                inactive.join(", ")
            ),
        ))
    }
}

fn validate_state_keys(keys: &mut Vec<StateKey>) -> Result<(), CanwuError> {
    for key in keys.iter() {
        if key.namespace.trim().is_empty()
            || key.name.trim().is_empty()
            || key.namespace != key.namespace.trim()
            || key.name != key.name.trim()
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "state keys require non-empty canonical namespace and name values",
            ));
        }
    }
    let unique: BTreeSet<_> = keys.drain(..).collect();
    keys.extend(unique);
    Ok(())
}

fn validate_plugin_identity(
    name: &str,
    version: &str,
    semantic_hash: &str,
) -> Result<(), CanwuError> {
    if name.trim().is_empty()
        || name != name.trim()
        || version.trim().is_empty()
        || version != version.trim()
        || !is_canonical_hash(semantic_hash)
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "plugins require canonical names, versions, and 64-character semantic hashes",
        ));
    }
    Ok(())
}

fn validate_system_contract(
    _plugin: &str,
    contract: &mut SystemContract,
) -> Result<(), CanwuError> {
    if contract.name.trim().is_empty() || contract.name != contract.name.trim() {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "plugin system name must be non-empty and have no surrounding whitespace",
        ));
    }
    if matches!(
        contract.phase,
        BoundaryPhase::EventIngress
            | BoundaryPhase::BoundarySnapshot
            | BoundaryPhase::AtomicDomainCommit
            | BoundaryPhase::ConditionalTransitionCommit
    ) {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            format!("boundary phase {:?} is owned by the kernel", contract.phase),
        ));
    }
    if contract.cadence != SystemCadence::EventDriven {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            format!(
                "system {} declares {:?} cadence, but the current runtime systems are event-driven only",
                contract.name, contract.cadence
            ),
        ));
    }
    if contract.visibility != StateVisibility::SameBoundary {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            format!(
                "event-driven system {} must declare same-boundary visibility until the phased boundary runtime is active",
                contract.name
            ),
        ));
    }
    validate_state_keys(&mut contract.reads)?;
    validate_state_keys(&mut contract.writes)?;
    if contract.reads.contains(&StateKey::core_ingress()) {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "canonical ingress can be read only by phased boundary systems",
        ));
    }
    Ok(())
}

fn validate_action_descriptor(
    _plugin: &str,
    descriptor: &mut PluginActionDescriptor,
) -> Result<(), CanwuError> {
    if descriptor.name.trim().is_empty() || descriptor.name != descriptor.name.trim() {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "plugin command names must be non-empty and have no surrounding whitespace",
        ));
    }
    if let PayloadSchema::Object { properties, .. } = &descriptor.payload_schema
        && properties
            .keys()
            .any(|name| name.trim().is_empty() || name != name.trim())
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "plugin payload schema property names cannot be empty",
        ));
    }
    validate_state_keys(&mut descriptor.reads)?;
    validate_state_keys(&mut descriptor.writes)?;
    if descriptor.reads.contains(&StateKey::core_ingress()) {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "plugin commands cannot inspect the canonical ingress queue",
        ));
    }
    Ok(())
}

fn validate_ingress_descriptor(descriptor: &PluginIngressDescriptor) -> Result<(), CanwuError> {
    if descriptor.name.trim().is_empty()
        || descriptor.name != descriptor.name.trim()
        || descriptor.description.trim().is_empty()
        || descriptor.description != descriptor.description.trim()
        || descriptor.class == IngressClass::Command
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "plugin ingress types require canonical names/descriptions and cannot claim the core command class",
        ));
    }
    if let PayloadSchema::Object { properties, .. } = &descriptor.payload_schema
        && properties
            .keys()
            .any(|name| name.trim().is_empty() || name != name.trim())
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "plugin ingress payload property names cannot be empty",
        ));
    }
    Ok(())
}

fn validate_boundary_system_contract(
    contract: &mut BoundarySystemContract,
) -> Result<(), CanwuError> {
    if contract.name.trim().is_empty() || contract.name != contract.name.trim() {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "boundary system name must be non-empty and canonical",
        ));
    }
    validate_state_keys(&mut contract.reads)?;
    validate_state_keys(&mut contract.writes)?;
    validate_state_keys(&mut contract.reservation_offers)?;
    validate_state_keys(&mut contract.reservation_requests)?;
    validate_reservation_refs(&mut contract.reservation_reads)?;
    validate_random_stream_keys(&mut contract.random_streams)?;
    validate_canonical_names(&mut contract.emits, "boundary event type")?;

    let may_propose_changes = matches!(
        contract.phase,
        BoundaryPhase::DomainDeltaProposal
            | BoundaryPhase::HistoricalCandidateEvaluation
            | BoundaryPhase::StrategicAggregation
            | BoundaryPhase::PerspectiveAndReportMaterialization
    );
    if (!contract.writes.is_empty() || !contract.emits.is_empty()) && !may_propose_changes {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            format!(
                "boundary system {} declares changes in kernel-owned phase {:?}",
                contract.name, contract.phase
            ),
        ));
    }
    let declares_reservations =
        !contract.reservation_offers.is_empty() || !contract.reservation_requests.is_empty();
    if declares_reservations && contract.phase != BoundaryPhase::ReservationAndAllocation {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            format!(
                "boundary system {} declares reservations outside reservation and allocation",
                contract.name
            ),
        ));
    }
    if !contract.reservation_reads.is_empty()
        && contract.phase <= BoundaryPhase::ReservationAndAllocation
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            format!(
                "boundary system {} reads allocations before reservation commit",
                contract.name
            ),
        ));
    }
    Ok(())
}

fn validate_reservation_refs(values: &mut Vec<ReservationRef>) -> Result<(), CanwuError> {
    if values.iter().any(|reservation| {
        reservation.plugin.trim().is_empty()
            || reservation.plugin != reservation.plugin.trim()
            || reservation.system.trim().is_empty()
            || reservation.system != reservation.system.trim()
            || reservation.request.trim().is_empty()
            || reservation.request != reservation.request.trim()
    }) {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "reservation read declarations must be non-empty and canonical",
        ));
    }
    let unique: BTreeSet<_> = values.drain(..).collect();
    values.extend(unique);
    Ok(())
}

fn validate_random_stream_keys(values: &mut Vec<RandomStreamKey>) -> Result<(), CanwuError> {
    if values.iter().any(|stream| {
        stream.namespace.trim().is_empty()
            || stream.namespace != stream.namespace.trim()
            || stream.name.trim().is_empty()
            || stream.name != stream.name.trim()
            || stream.version == 0
    }) {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "random stream declarations require canonical names and a nonzero version",
        ));
    }
    let unique: BTreeSet<_> = values.drain(..).collect();
    values.extend(unique);
    Ok(())
}

fn validate_canonical_names(values: &mut Vec<String>, label: &str) -> Result<(), CanwuError> {
    if values
        .iter()
        .any(|value| value.trim().is_empty() || value != value.trim())
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            format!("{label} declarations must be non-empty and canonical"),
        ));
    }
    let unique: BTreeSet<_> = values.drain(..).collect();
    values.extend(unique);
    Ok(())
}

fn register_state_owners(
    owners: &mut BTreeMap<StateKey, String>,
    plugin: &str,
    writes: &[StateKey],
) -> Result<(), CanwuError> {
    for key in writes {
        if key.namespace == CORE_STATE_NAMESPACE {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!(
                    "plugin {plugin} cannot claim reserved state {}.{}",
                    key.namespace, key.name
                ),
            ));
        }
        if let Some(existing) = owners.get(key)
            && existing != plugin
        {
            return Err(CanwuError::new(
                ErrorCode::DuplicateStateOwner,
                format!(
                    "state {}.{} is owned by both {existing} and {plugin}",
                    key.namespace, key.name
                ),
            ));
        }
    }
    for key in writes {
        owners.insert(key.clone(), plugin.to_owned());
    }
    Ok(())
}

fn register_boundary_writers(
    writers: &mut BTreeMap<(BoundaryWriteStage, StateKey), (String, String)>,
    immediate_writes: &BTreeMap<StateKey, String>,
    plugin: &str,
    system: &str,
    phase: BoundaryPhase,
    declared_states: &[StateKey],
) -> Result<(), CanwuError> {
    let Some(stage) = boundary_write_stage(phase) else {
        if declared_states.is_empty() {
            return Ok(());
        }
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            format!("boundary phase {phase:?} cannot own state writes"),
        ));
    };
    for state in declared_states {
        if let Some(immediate_plugin) = immediate_writes.get(state) {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!(
                    "boundary state {}.{} conflicts with immediate writes from plugin {immediate_plugin}",
                    state.namespace, state.name
                ),
            ));
        }
        if let Some((existing_plugin, existing_system)) = writers.get(&(stage, state.clone()))
            && (existing_plugin != plugin || existing_system != system)
        {
            return Err(CanwuError::new(
                ErrorCode::DuplicateBoundaryWriter,
                format!(
                    "boundary state {}.{} is written by both {existing_plugin}.{existing_system} and {plugin}.{system}",
                    state.namespace, state.name
                ),
            ));
        }
    }
    for state in declared_states {
        writers.insert(
            (stage, state.clone()),
            (plugin.to_owned(), system.to_owned()),
        );
    }
    Ok(())
}

fn register_immediate_write_states(
    immediate_writes: &mut BTreeMap<StateKey, String>,
    boundary_writers: &BTreeMap<(BoundaryWriteStage, StateKey), (String, String)>,
    plugin: &str,
    writes: &[StateKey],
) -> Result<(), CanwuError> {
    for state in writes {
        if boundary_writers
            .keys()
            .any(|(_, boundary_state)| boundary_state == state)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!(
                    "immediate state {}.{} conflicts with a phased boundary writer",
                    state.namespace, state.name
                ),
            ));
        }
        if immediate_writes
            .get(state)
            .is_some_and(|existing| existing != plugin)
        {
            return Err(CanwuError::new(
                ErrorCode::DuplicateStateOwner,
                format!(
                    "immediate state {}.{} is written by multiple plugins",
                    state.namespace, state.name
                ),
            ));
        }
    }
    for state in writes {
        immediate_writes.insert(state.clone(), plugin.to_owned());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum BoundaryWriteStage {
    Ordinary,
    Transition,
    Aggregation,
    Perspective,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum DomainRecordCommitStage {
    Ordinary,
    Transition,
    Aggregation,
    Perspective,
    Deferred,
}

impl DomainRecordCommitStage {
    const ALL: [Self; 5] = [
        Self::Ordinary,
        Self::Transition,
        Self::Aggregation,
        Self::Perspective,
        Self::Deferred,
    ];

    const fn ordinal(self) -> u8 {
        match self {
            Self::Ordinary => 1,
            Self::Transition => 2,
            Self::Aggregation => 3,
            Self::Perspective => 4,
            Self::Deferred => 5,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct DomainHistoryCut {
    boundary: usize,
    stage: u8,
}

impl DomainHistoryCut {
    const GENESIS: Self = Self {
        boundary: 0,
        stage: 0,
    };

    const fn after_boundaries(boundary: usize) -> Self {
        Self { boundary, stage: 5 }
    }

    const fn after_stage(boundary: usize, stage: DomainRecordCommitStage) -> Self {
        Self {
            boundary,
            stage: stage.ordinal(),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct BoundaryDomainEntityCuts {
    changes: BTreeMap<DomainRecordRef, Vec<DomainEntityStageChange>>,
}

impl BoundaryDomainEntityCuts {
    fn record(&mut self, stage: DomainRecordCommitStage, change: &DomainRecordChange) {
        let previous_live = change
            .previous
            .as_ref()
            .is_some_and(domain_record_is_live_entity);
        let current_live = domain_record_is_live_entity(&change.current);
        if previous_live != current_live {
            self.changes
                .entry(change.current.reference.clone())
                .or_default()
                .push(DomainEntityStageChange {
                    stage,
                    plugin: change.plugin.clone(),
                    system: change.system.clone(),
                    previous_live,
                    current_live,
                });
        }
    }

    fn is_live(
        &self,
        final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
        reference: &DomainRecordRef,
        stage: Option<DomainRecordCommitStage>,
    ) -> bool {
        let mut live = final_records
            .get(reference)
            .is_some_and(domain_record_is_live_entity);
        if let Some(changes) = self.changes.get(reference) {
            for change in changes.iter().rev() {
                if stage.is_some_and(|stage| change.stage <= stage) {
                    break;
                }
                live = change.previous_live;
            }
        }
        live
    }

    fn is_live_for_proposal(
        &self,
        final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
        reference: &DomainRecordRef,
        phase: BoundaryPhase,
        commit_stage: DomainRecordCommitStage,
        plugin: &str,
        system: &str,
    ) -> bool {
        let visible_after = match phase {
            BoundaryPhase::DomainDeltaProposal => None,
            BoundaryPhase::HistoricalCandidateEvaluation => Some(DomainRecordCommitStage::Ordinary),
            BoundaryPhase::StrategicAggregation => Some(DomainRecordCommitStage::Transition),
            BoundaryPhase::PerspectiveAndReportMaterialization => {
                Some(DomainRecordCommitStage::Aggregation)
            }
            BoundaryPhase::EventIngress
            | BoundaryPhase::BoundarySnapshot
            | BoundaryPhase::DerivedFieldSolve
            | BoundaryPhase::PerceptionAndAttentionRefresh
            | BoundaryPhase::DecisionAndAcceptedEffectIntake
            | BoundaryPhase::ReservationAndAllocation
            | BoundaryPhase::InvariantValidation
            | BoundaryPhase::AtomicDomainCommit
            | BoundaryPhase::ConditionalTransitionCommit
            | BoundaryPhase::SaveReplayAndDiagnosticHashing => return false,
        };
        let before_proposal = self.is_live(final_records, reference, visible_after);
        self.changes
            .get(reference)
            .and_then(|changes| {
                changes.iter().find(|change| {
                    change.stage == commit_stage
                        && change.plugin == plugin
                        && change.system == system
                })
            })
            .map_or(before_proposal, |change| change.current_live)
    }

    fn identity_exists_for_proposal(
        &self,
        final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
        reference: &DomainRecordRef,
        phase: BoundaryPhase,
        commit_stage: DomainRecordCommitStage,
        plugin: &str,
        system: &str,
    ) -> bool {
        if !final_records.contains_key(reference) {
            return false;
        }
        let visible_after = match phase {
            BoundaryPhase::DomainDeltaProposal => None,
            BoundaryPhase::HistoricalCandidateEvaluation => Some(DomainRecordCommitStage::Ordinary),
            BoundaryPhase::StrategicAggregation => Some(DomainRecordCommitStage::Transition),
            BoundaryPhase::PerspectiveAndReportMaterialization => {
                Some(DomainRecordCommitStage::Aggregation)
            }
            BoundaryPhase::EventIngress
            | BoundaryPhase::BoundarySnapshot
            | BoundaryPhase::DerivedFieldSolve
            | BoundaryPhase::PerceptionAndAttentionRefresh
            | BoundaryPhase::DecisionAndAcceptedEffectIntake
            | BoundaryPhase::ReservationAndAllocation
            | BoundaryPhase::InvariantValidation
            | BoundaryPhase::AtomicDomainCommit
            | BoundaryPhase::ConditionalTransitionCommit
            | BoundaryPhase::SaveReplayAndDiagnosticHashing => return false,
        };
        self.changes
            .get(reference)
            .and_then(|changes| {
                changes
                    .iter()
                    .find(|change| !change.previous_live && change.current_live)
            })
            .is_none_or(|creation| {
                visible_after.is_some_and(|stage| creation.stage <= stage)
                    || (creation.stage == commit_stage
                        && creation.plugin == plugin
                        && creation.system == system)
            })
    }
}

#[derive(Clone, Debug)]
struct DomainEntityStageChange {
    stage: DomainRecordCommitStage,
    plugin: String,
    system: String,
    previous_live: bool,
    current_live: bool,
}

#[derive(Clone, Debug)]
struct DomainRecordHistory {
    lifetimes: BTreeMap<DomainRecordRef, DomainEntityLifetime>,
}

impl DomainRecordHistory {
    fn from_initial_records(records: &BTreeMap<DomainRecordRef, DomainRecord>) -> Self {
        let lifetimes = records
            .values()
            .filter(|record| record.class == DomainRecordClass::Entity)
            .map(|record| {
                (
                    record.reference.clone(),
                    DomainEntityLifetime {
                        created_at: DomainHistoryCut::GENESIS,
                        deleted_at: record.is_deleted().then_some(DomainHistoryCut::GENESIS),
                    },
                )
            })
            .collect();
        Self { lifetimes }
    }

    fn apply_boundary(
        &mut self,
        boundary: usize,
        cuts: &BoundaryDomainEntityCuts,
    ) -> Result<(), CanwuError> {
        for (reference, changes) in &cuts.changes {
            for change in changes {
                let cut = DomainHistoryCut::after_stage(boundary, change.stage);
                match (change.previous_live, change.current_live) {
                    (false, true) => {
                        if self
                            .lifetimes
                            .insert(
                                reference.clone(),
                                DomainEntityLifetime {
                                    created_at: cut,
                                    deleted_at: None,
                                },
                            )
                            .is_some()
                        {
                            return invalid_snapshot(
                                "domain entity history recreates an existing stable identity",
                            );
                        }
                    }
                    (true, false) => {
                        let Some(lifetime) = self.lifetimes.get_mut(reference) else {
                            return invalid_snapshot(
                                "domain entity history deletes an identity before creation",
                            );
                        };
                        if lifetime.deleted_at.replace(cut).is_some() {
                            return invalid_snapshot(
                                "domain entity history deletes the same identity more than once",
                            );
                        }
                    }
                    (false, false) | (true, true) => {}
                }
            }
        }
        Ok(())
    }

    fn is_live(&self, reference: &DomainRecordRef, cut: DomainHistoryCut) -> bool {
        self.lifetimes.get(reference).is_some_and(|lifetime| {
            lifetime.created_at <= cut && lifetime.deleted_at.is_none_or(|deleted| cut < deleted)
        })
    }

    fn exists(&self, reference: &DomainRecordRef, cut: DomainHistoryCut) -> bool {
        self.lifetimes
            .get(reference)
            .is_some_and(|lifetime| lifetime.created_at <= cut)
    }

    fn before_time(snapshot: &SimulationSnapshot, at: SimTime) -> DomainHistoryCut {
        let count = snapshot
            .boundaries
            .partition_point(|boundary| boundary.at < at);
        DomainHistoryCut::after_boundaries(count)
    }
}

#[derive(Clone, Copy, Debug)]
struct DomainEntityLifetime {
    created_at: DomainHistoryCut,
    deleted_at: Option<DomainHistoryCut>,
}

fn domain_record_is_live_entity(record: &DomainRecord) -> bool {
    record.class == DomainRecordClass::Entity && !record.is_deleted()
}

const fn boundary_write_stage(phase: BoundaryPhase) -> Option<BoundaryWriteStage> {
    match phase {
        BoundaryPhase::DomainDeltaProposal => Some(BoundaryWriteStage::Ordinary),
        BoundaryPhase::HistoricalCandidateEvaluation => Some(BoundaryWriteStage::Transition),
        BoundaryPhase::StrategicAggregation => Some(BoundaryWriteStage::Aggregation),
        BoundaryPhase::PerspectiveAndReportMaterialization => Some(BoundaryWriteStage::Perspective),
        BoundaryPhase::EventIngress
        | BoundaryPhase::BoundarySnapshot
        | BoundaryPhase::DerivedFieldSolve
        | BoundaryPhase::PerceptionAndAttentionRefresh
        | BoundaryPhase::DecisionAndAcceptedEffectIntake
        | BoundaryPhase::ReservationAndAllocation
        | BoundaryPhase::InvariantValidation
        | BoundaryPhase::AtomicDomainCommit
        | BoundaryPhase::ConditionalTransitionCommit
        | BoundaryPhase::SaveReplayAndDiagnosticHashing => None,
    }
}

const fn domain_record_commit_stage(
    phase: BoundaryPhase,
    visibility: StateVisibility,
) -> Option<DomainRecordCommitStage> {
    let stage = match phase {
        BoundaryPhase::DomainDeltaProposal => DomainRecordCommitStage::Ordinary,
        BoundaryPhase::HistoricalCandidateEvaluation => DomainRecordCommitStage::Transition,
        BoundaryPhase::StrategicAggregation => DomainRecordCommitStage::Aggregation,
        BoundaryPhase::PerspectiveAndReportMaterialization => DomainRecordCommitStage::Perspective,
        BoundaryPhase::EventIngress
        | BoundaryPhase::BoundarySnapshot
        | BoundaryPhase::DerivedFieldSolve
        | BoundaryPhase::PerceptionAndAttentionRefresh
        | BoundaryPhase::DecisionAndAcceptedEffectIntake
        | BoundaryPhase::ReservationAndAllocation
        | BoundaryPhase::InvariantValidation
        | BoundaryPhase::AtomicDomainCommit
        | BoundaryPhase::ConditionalTransitionCommit
        | BoundaryPhase::SaveReplayAndDiagnosticHashing => return None,
    };
    Some(match visibility {
        StateVisibility::SameBoundary => stage,
        StateVisibility::NextBoundary => DomainRecordCommitStage::Deferred,
    })
}

fn register_reservation_offerers(
    offerers: &mut BTreeMap<StateKey, (String, String)>,
    plugin: &str,
    system: &str,
    offered_state: &[StateKey],
) -> Result<(), CanwuError> {
    for state in offered_state {
        if let Some((existing_plugin, existing_system)) = offerers.get(state)
            && (existing_plugin != plugin || existing_system != system)
        {
            return Err(CanwuError::new(
                ErrorCode::DuplicateReservationOfferer,
                format!(
                    "reservation state {}.{} is offered by both {existing_plugin}.{existing_system} and {plugin}.{system}",
                    state.namespace, state.name
                ),
            ));
        }
    }
    for state in offered_state {
        offerers.insert(state.clone(), (plugin.to_owned(), system.to_owned()));
    }
    Ok(())
}

fn register_random_streams(
    owners: &mut BTreeMap<RandomStreamKey, (String, String)>,
    plugin: &str,
    system: &str,
    streams: &[RandomStreamKey],
) -> Result<(), CanwuError> {
    for stream in streams {
        if stream.namespace != plugin || stream.namespace == CORE_STATE_NAMESPACE {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!(
                    "random stream {}.{}@{} must use its owning plugin namespace {plugin}",
                    stream.namespace, stream.name, stream.version
                ),
            ));
        }
        if let Some((existing_plugin, existing_system)) = owners.get(stream)
            && (existing_plugin != plugin || existing_system != system)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!(
                    "random stream {}.{}@{} is owned by both {existing_plugin}.{existing_system} and {plugin}.{system}",
                    stream.namespace, stream.name, stream.version
                ),
            ));
        }
    }
    for stream in streams {
        owners.insert(stream.clone(), (plugin.to_owned(), system.to_owned()));
    }
    Ok(())
}

fn validate_type_schema(schema: &TypeSchema) -> Result<(), CanwuError> {
    if schema.type_name.trim().is_empty() || schema.type_name != schema.type_name.trim() {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "plugin schema type name must be non-empty and have no surrounding whitespace",
        ));
    }
    let mut field_names = BTreeSet::new();
    for field in &schema.fields {
        if field.name.trim().is_empty()
            || field.name != field.name.trim()
            || field.value_type.trim().is_empty()
            || field.value_type != field.value_type.trim()
            || field
                .reference_type
                .as_ref()
                .is_some_and(|value| value.trim().is_empty() || value != value.trim())
            || !field_names.insert(&field.name)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!("schema {} contains an invalid field", schema.type_name),
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
struct ScheduleKey {
    at: SimTime,
    sequence: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ScheduledAction {
    ArmyArrival {
        army: ArmyId,
        destination: TerritoryId,
        order_event: EventId,
        correlation_id: u64,
    },
    KnowledgeReport {
        recipient: PersonId,
        army: ArmyId,
        location: TerritoryId,
        observed_at: SimTime,
        dispatch_event: EventId,
        correlation_id: u64,
    },
    PluginDirective {
        plugin: String,
        directive: Box<SystemDirective>,
        allowed_writes: Vec<StateKey>,
        cause: CauseRef,
        correlation_id: u64,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct ScheduledRecord {
    key: ScheduleKey,
    action: ScheduledAction,
}

const fn one_u64() -> u64 {
    1
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_one_u64(value: &u64) -> bool {
    *value == 1
}

fn command_attempt_slice_is_empty(value: &&[CommandAttemptRecord]) -> bool {
    value.is_empty()
}

fn command_attempt_id_slice_is_empty(value: &&[CommandAttemptId]) -> bool {
    value.is_empty()
}

fn domain_record_slice_is_empty(value: &&[DomainRecord]) -> bool {
    value.is_empty()
}

fn domain_record_change_slice_is_empty(value: &&[DomainRecordChange]) -> bool {
    value.is_empty()
}

fn ingress_record_slice_is_empty(value: &&[IngressRecord]) -> bool {
    value.is_empty()
}

#[derive(Clone)]
struct RuntimeEvidence {
    events: Vec<SimEvent>,
    commands: Vec<CommandRecord>,
    command_attempts: Vec<CommandAttemptRecord>,
    ingress: Vec<IngressRecord>,
    boundaries: Vec<BoundaryRecord>,
    random_draws: Vec<RandomDrawRecord>,
}

#[derive(Clone)]
struct OrderedCommitmentAccumulator {
    hasher: blake3::Hasher,
    len: usize,
}

impl OrderedCommitmentAccumulator {
    fn new(domain: &str) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(domain.as_bytes());
        hasher.update(&[0]);
        hasher.update(b"[");
        Self { hasher, len: 0 }
    }

    fn from_sorted_by<T, K, F>(domain: &str, values: &[T], mut key: F) -> Result<Self, CanwuError>
    where
        T: Serialize,
        K: Ord,
        F: FnMut(&T) -> K,
    {
        let mut ordered: Vec<_> = values.iter().collect();
        ordered.sort_by_key(|value| key(value));
        let mut accumulator = Self::new(domain);
        for value in ordered {
            accumulator.append(value)?;
        }
        Ok(accumulator)
    }

    fn append<T: Serialize>(&mut self, value: &T) -> Result<(), CanwuError> {
        let encoded = serde_json::to_vec(value).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("could not encode incremental commitment item: {error}"),
            )
        })?;
        if self.len != 0 {
            self.hasher.update(b",");
        }
        self.hasher.update(&encoded);
        self.len = self.len.checked_add(1).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::IdentifierExhausted,
                "incremental commitment item count is exhausted",
            )
        })?;
        Ok(())
    }

    fn sync_tail<T: Serialize>(&mut self, values: &[T]) -> Result<(), CanwuError> {
        if self.len > values.len() {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "append-only commitment cache exceeds its evidence journal",
            ));
        }
        for value in &values[self.len..] {
            self.append(value)?;
        }
        Ok(())
    }

    fn root(&self) -> String {
        let mut hasher = self.hasher.clone();
        hasher.update(b"]");
        hasher.finalize().to_hex().to_string()
    }
}

#[derive(Clone)]
struct RuntimeCommitmentCache {
    commands: OrderedCommitmentAccumulator,
    attempts: OrderedCommitmentAccumulator,
    events: OrderedCommitmentAccumulator,
    ingress: OrderedCommitmentAccumulator,
    random_draws: OrderedCommitmentAccumulator,
    world: Option<String>,
    knowledge: Option<String>,
    plugin_components: Option<String>,
    domain_records: Option<String>,
    scheduler: Option<String>,
    random_streams: Option<String>,
    identity: Option<String>,
}

impl RuntimeCommitmentCache {
    fn from_evidence(evidence: &RuntimeEvidence) -> Result<Self, CanwuError> {
        Ok(Self {
            commands: OrderedCommitmentAccumulator::from_sorted_by(
                "canwu.commitment.commands.accepted.v1",
                &evidence.commands,
                |record| record.id,
            )?,
            attempts: OrderedCommitmentAccumulator::from_sorted_by(
                "canwu.commitment.commands.attempts.v1",
                &evidence.command_attempts,
                |record| record.id,
            )?,
            events: OrderedCommitmentAccumulator::from_sorted_by(
                "canwu.commitment.events.v1",
                &evidence.events,
                |event| event.id,
            )?,
            ingress: OrderedCommitmentAccumulator::from_sorted_by(
                "canwu.commitment.ingress.v1",
                &evidence.ingress,
                |record| record.id,
            )?,
            random_draws: OrderedCommitmentAccumulator::from_sorted_by(
                "canwu.commitment.random.draws.v1",
                &evidence.random_draws,
                |draw| draw.id,
            )?,
            world: None,
            knowledge: None,
            plugin_components: None,
            domain_records: None,
            scheduler: None,
            random_streams: None,
            identity: None,
        })
    }

    fn sync(&mut self, evidence: &RuntimeEvidence) -> Result<(), CanwuError> {
        self.commands.sync_tail(&evidence.commands)?;
        self.attempts.sync_tail(&evidence.command_attempts)?;
        self.events.sync_tail(&evidence.events)?;
        self.ingress.sync_tail(&evidence.ingress)?;
        self.random_draws.sync_tail(&evidence.random_draws)?;
        Ok(())
    }

    fn roots(&self) -> JournalCommitmentRoots {
        JournalCommitmentRoots {
            commands: self.commands.root(),
            attempts: self.attempts.root(),
            events: self.events.root(),
            ingress: self.ingress.root(),
            random_draws: self.random_draws.root(),
        }
    }

    fn needs(&self) -> CommitmentDomains {
        let mut domains = CommitmentDomains::default();
        for (missing, domain) in [
            (self.world.is_none(), CommitmentDomains::WORLD),
            (self.knowledge.is_none(), CommitmentDomains::KNOWLEDGE),
            (
                self.plugin_components.is_none(),
                CommitmentDomains::PLUGIN_COMPONENTS,
            ),
            (
                self.domain_records.is_none(),
                CommitmentDomains::DOMAIN_RECORDS,
            ),
            (self.scheduler.is_none(), CommitmentDomains::SCHEDULER),
            (
                self.random_streams.is_none(),
                CommitmentDomains::RANDOM_STREAMS,
            ),
            (self.identity.is_none(), CommitmentDomains::IDENTITY),
        ] {
            if missing {
                domains.insert(domain);
            }
        }
        domains
    }

    fn invalidate(&mut self, domains: CommitmentDomains) {
        if domains.contains(CommitmentDomains::WORLD) {
            self.world = None;
        }
        if domains.contains(CommitmentDomains::KNOWLEDGE) {
            self.knowledge = None;
        }
        if domains.contains(CommitmentDomains::PLUGIN_COMPONENTS) {
            self.plugin_components = None;
        }
        if domains.contains(CommitmentDomains::DOMAIN_RECORDS) {
            self.domain_records = None;
        }
        if domains.contains(CommitmentDomains::SCHEDULER) {
            self.scheduler = None;
        }
        if domains.contains(CommitmentDomains::RANDOM_STREAMS) {
            self.random_streams = None;
        }
        if domains.contains(CommitmentDomains::IDENTITY) {
            self.identity = None;
        }
    }

    fn apply(&mut self, updates: RuntimeCommitmentRootUpdates) {
        if let Some(root) = updates.world {
            self.world = Some(root);
        }
        if let Some(root) = updates.knowledge {
            self.knowledge = Some(root);
        }
        if let Some(root) = updates.plugin_components {
            self.plugin_components = Some(root);
        }
        if let Some(root) = updates.domain_records {
            self.domain_records = Some(root);
        }
        if let Some(root) = updates.scheduler {
            self.scheduler = Some(root);
        }
        if let Some(root) = updates.random_streams {
            self.random_streams = Some(root);
        }
        if let Some(root) = updates.identity {
            self.identity = Some(root);
        }
    }

    fn domain_roots(&self) -> Result<RuntimeDomainCommitmentRoots, CanwuError> {
        let required = |root: &Option<String>, domain: &str| {
            root.clone().ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    format!("runtime commitment cache is missing its {domain} root"),
                )
            })
        };
        Ok(RuntimeDomainCommitmentRoots {
            world: required(&self.world, "world")?,
            knowledge: required(&self.knowledge, "knowledge")?,
            plugin_components: required(&self.plugin_components, "plugin-component")?,
            domain_records: required(&self.domain_records, "domain-record")?,
            scheduler: required(&self.scheduler, "scheduler")?,
            random_streams: required(&self.random_streams, "random-stream")?,
            identity: required(&self.identity, "identity")?,
        })
    }
}

#[derive(Clone)]
struct JournalCommitmentRoots {
    commands: String,
    attempts: String,
    events: String,
    ingress: String,
    random_draws: String,
}

#[derive(Clone, Copy, Default)]
struct CommitmentDomains(u8);

impl CommitmentDomains {
    const WORLD: Self = Self(1 << 0);
    const KNOWLEDGE: Self = Self(1 << 1);
    const PLUGIN_COMPONENTS: Self = Self(1 << 2);
    const DOMAIN_RECORDS: Self = Self(1 << 3);
    const SCHEDULER: Self = Self(1 << 4);
    const RANDOM_STREAMS: Self = Self(1 << 5);
    const IDENTITY: Self = Self(1 << 6);

    const fn contains(self, domain: Self) -> bool {
        self.0 & domain.0 == domain.0
    }

    fn insert(&mut self, domain: Self) {
        self.0 |= domain.0;
    }
}

impl std::ops::BitOr for CommitmentDomains {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[derive(Default)]
struct RuntimeCommitmentRootUpdates {
    world: Option<String>,
    knowledge: Option<String>,
    plugin_components: Option<String>,
    domain_records: Option<String>,
    scheduler: Option<String>,
    random_streams: Option<String>,
    identity: Option<String>,
}

#[derive(Clone)]
struct RuntimeDomainCommitmentRoots {
    world: String,
    knowledge: String,
    plugin_components: String,
    domain_records: String,
    scheduler: String,
    random_streams: String,
    identity: String,
}

#[derive(Clone)]
struct RuntimeScheduler {
    initial_time: SimTime,
    now: SimTime,
    actions: BTreeMap<ScheduleKey, ScheduledAction>,
    pending_ingress: BTreeSet<IngressQueueKey>,
}

#[derive(Clone)]
struct RuntimeCounters {
    next_event_id: u64,
    next_command_id: u64,
    next_command_attempt_id: u64,
    next_ingress_id: u64,
    next_boundary_id: u64,
    next_random_draw_id: u64,
    next_schedule_sequence: u64,
    next_correlation_id: u64,
    state_revision: u64,
    admitted_attempt_count: u64,
    admitted_command_count: u64,
    admitted_event_count: u64,
}

#[derive(Clone)]
struct RuntimeMetadata {
    initial_scenario: Option<Scenario>,
    run_manifest: RunManifest,
    run_manifest_hash: String,
    run_configuration: RunConfigurationSnapshot,
    checkpoint_hash: String,
    commitment_format_version: u32,
    commitment_roots: Option<CommitmentRoots>,
    commitment_cache: Option<RuntimeCommitmentCache>,
    plugin_registration_closed: bool,
    replay_revision_format_version: u32,
}

#[derive(Clone)]
struct RuntimeCurrentState {
    people: BTreeMap<PersonId, Person>,
    governments: BTreeMap<GovernmentId, Government>,
    territories: BTreeMap<TerritoryId, Territory>,
    routes: BTreeMap<RouteId, Route>,
    armies: BTreeMap<ArmyId, Army>,
    knowledge: KnowledgeSnapshot,
    plugin_components: BTreeMap<PluginComponentKey, PluginComponentRecord>,
    domain_records: BTreeMap<DomainRecordRef, DomainRecord>,
    root_seed: u64,
    random_streams: BTreeMap<RandomStreamKey, RandomStreamState>,
}

#[derive(Clone)]
struct RuntimeState {
    current: RuntimeCurrentState,
    scheduler: RuntimeScheduler,
    counters: RuntimeCounters,
    metadata: RuntimeMetadata,
    evidence: RuntimeEvidence,
}

struct RejectionTransactionCheckpoint {
    next_command_attempt_id: u64,
    state_revision: u64,
    plugin_registration_closed: bool,
    command_attempt_count: usize,
    checkpoint_hash: String,
    commitment_roots: Option<CommitmentRoots>,
    commitment_cache: Option<RuntimeCommitmentCache>,
}

impl RejectionTransactionCheckpoint {
    fn capture(state: &RuntimeState) -> Self {
        Self {
            next_command_attempt_id: state.counters.next_command_attempt_id,
            state_revision: state.counters.state_revision,
            plugin_registration_closed: state.metadata.plugin_registration_closed,
            command_attempt_count: state.evidence.command_attempts.len(),
            checkpoint_hash: state.metadata.checkpoint_hash.clone(),
            commitment_roots: state.metadata.commitment_roots.clone(),
            commitment_cache: state.metadata.commitment_cache.clone(),
        }
    }

    fn restore(self, state: &mut RuntimeState) {
        state.counters.next_command_attempt_id = self.next_command_attempt_id;
        state.counters.state_revision = self.state_revision;
        state.metadata.plugin_registration_closed = self.plugin_registration_closed;
        state
            .evidence
            .command_attempts
            .truncate(self.command_attempt_count);
        state.metadata.checkpoint_hash = self.checkpoint_hash;
        state.metadata.commitment_roots = self.commitment_roots;
        state.metadata.commitment_cache = self.commitment_cache;
    }
}

struct CommandTransactionCheckpoint {
    armies: BTreeMap<ArmyId, Army>,
    knowledge: KnowledgeSnapshot,
    plugin_components: BTreeMap<PluginComponentKey, PluginComponentRecord>,
    scheduled_actions: BTreeMap<ScheduleKey, ScheduledAction>,
    counters: RuntimeCounters,
    event_count: usize,
    command_count: usize,
    command_attempt_count: usize,
    plugin_registration_closed: bool,
    checkpoint_hash: String,
    commitment_roots: Option<CommitmentRoots>,
    commitment_cache: Option<RuntimeCommitmentCache>,
}

impl CommandTransactionCheckpoint {
    fn capture(state: &RuntimeState) -> Self {
        Self {
            armies: state.current.armies.clone(),
            knowledge: state.current.knowledge.clone(),
            plugin_components: state.current.plugin_components.clone(),
            scheduled_actions: state.scheduler.actions.clone(),
            counters: state.counters.clone(),
            event_count: state.evidence.events.len(),
            command_count: state.evidence.commands.len(),
            command_attempt_count: state.evidence.command_attempts.len(),
            plugin_registration_closed: state.metadata.plugin_registration_closed,
            checkpoint_hash: state.metadata.checkpoint_hash.clone(),
            commitment_roots: state.metadata.commitment_roots.clone(),
            commitment_cache: state.metadata.commitment_cache.clone(),
        }
    }

    fn restore(self, state: &mut RuntimeState) {
        state.current.armies = self.armies;
        state.current.knowledge = self.knowledge;
        state.current.plugin_components = self.plugin_components;
        state.scheduler.actions = self.scheduled_actions;
        state.counters = self.counters;
        state.evidence.events.truncate(self.event_count);
        state.evidence.commands.truncate(self.command_count);
        state
            .evidence
            .command_attempts
            .truncate(self.command_attempt_count);
        state.metadata.plugin_registration_closed = self.plugin_registration_closed;
        state.metadata.checkpoint_hash = self.checkpoint_hash;
        state.metadata.commitment_roots = self.commitment_roots;
        state.metadata.commitment_cache = self.commitment_cache;
    }
}

struct BoundaryTransactionCheckpoint {
    armies: BTreeMap<ArmyId, Army>,
    knowledge: KnowledgeSnapshot,
    plugin_components: BTreeMap<PluginComponentKey, PluginComponentRecord>,
    domain_records: BTreeMap<DomainRecordRef, DomainRecord>,
    random_streams: BTreeMap<RandomStreamKey, RandomStreamState>,
    scheduler: RuntimeScheduler,
    counters: RuntimeCounters,
    event_count: usize,
    command_count: usize,
    command_attempt_count: usize,
    ingress_count: usize,
    boundary_count: usize,
    random_draw_count: usize,
    plugin_registration_closed: bool,
    checkpoint_hash: String,
    commitment_roots: Option<CommitmentRoots>,
    commitment_cache: Option<RuntimeCommitmentCache>,
}

impl BoundaryTransactionCheckpoint {
    fn capture(state: &RuntimeState) -> Self {
        Self {
            armies: state.current.armies.clone(),
            knowledge: state.current.knowledge.clone(),
            plugin_components: state.current.plugin_components.clone(),
            domain_records: state.current.domain_records.clone(),
            random_streams: state.current.random_streams.clone(),
            scheduler: state.scheduler.clone(),
            counters: state.counters.clone(),
            event_count: state.evidence.events.len(),
            command_count: state.evidence.commands.len(),
            command_attempt_count: state.evidence.command_attempts.len(),
            ingress_count: state.evidence.ingress.len(),
            boundary_count: state.evidence.boundaries.len(),
            random_draw_count: state.evidence.random_draws.len(),
            plugin_registration_closed: state.metadata.plugin_registration_closed,
            checkpoint_hash: state.metadata.checkpoint_hash.clone(),
            commitment_roots: state.metadata.commitment_roots.clone(),
            commitment_cache: state.metadata.commitment_cache.clone(),
        }
    }

    fn restore(self, state: &mut RuntimeState) {
        state.current.armies = self.armies;
        state.current.knowledge = self.knowledge;
        state.current.plugin_components = self.plugin_components;
        state.current.domain_records = self.domain_records;
        state.current.random_streams = self.random_streams;
        state.scheduler = self.scheduler;
        state.counters = self.counters;
        state.evidence.events.truncate(self.event_count);
        state.evidence.commands.truncate(self.command_count);
        state
            .evidence
            .command_attempts
            .truncate(self.command_attempt_count);
        state.evidence.ingress.truncate(self.ingress_count);
        state.evidence.boundaries.truncate(self.boundary_count);
        state.evidence.random_draws.truncate(self.random_draw_count);
        state.metadata.plugin_registration_closed = self.plugin_registration_closed;
        state.metadata.checkpoint_hash = self.checkpoint_hash;
        state.metadata.commitment_roots = self.commitment_roots;
        state.metadata.commitment_cache = self.commitment_cache;
    }
}

struct ScheduledBatchTransactionCheckpoint {
    armies: BTreeMap<ArmyId, Army>,
    knowledge: KnowledgeSnapshot,
    plugin_components: BTreeMap<PluginComponentKey, PluginComponentRecord>,
    random_streams: BTreeMap<RandomStreamKey, RandomStreamState>,
    now: SimTime,
    scheduled_actions: BTreeMap<ScheduleKey, ScheduledAction>,
    counters: RuntimeCounters,
    event_count: usize,
    random_draw_count: usize,
    plugin_registration_closed: bool,
    checkpoint_hash: String,
    commitment_roots: Option<CommitmentRoots>,
    commitment_cache: Option<RuntimeCommitmentCache>,
}

impl ScheduledBatchTransactionCheckpoint {
    fn capture(state: &RuntimeState) -> Self {
        Self {
            armies: state.current.armies.clone(),
            knowledge: state.current.knowledge.clone(),
            plugin_components: state.current.plugin_components.clone(),
            random_streams: state.current.random_streams.clone(),
            now: state.scheduler.now,
            scheduled_actions: state.scheduler.actions.clone(),
            counters: state.counters.clone(),
            event_count: state.evidence.events.len(),
            random_draw_count: state.evidence.random_draws.len(),
            plugin_registration_closed: state.metadata.plugin_registration_closed,
            checkpoint_hash: state.metadata.checkpoint_hash.clone(),
            commitment_roots: state.metadata.commitment_roots.clone(),
            commitment_cache: state.metadata.commitment_cache.clone(),
        }
    }

    fn restore(self, state: &mut RuntimeState) {
        state.current.armies = self.armies;
        state.current.knowledge = self.knowledge;
        state.current.plugin_components = self.plugin_components;
        state.current.random_streams = self.random_streams;
        state.scheduler.now = self.now;
        state.scheduler.actions = self.scheduled_actions;
        state.counters = self.counters;
        state.evidence.events.truncate(self.event_count);
        state.evidence.random_draws.truncate(self.random_draw_count);
        state.metadata.plugin_registration_closed = self.plugin_registration_closed;
        state.metadata.checkpoint_hash = self.checkpoint_hash;
        state.metadata.commitment_roots = self.commitment_roots;
        state.metadata.commitment_cache = self.commitment_cache;
    }
}

struct ClockTransactionCheckpoint {
    now: SimTime,
    plugin_registration_closed: bool,
    checkpoint_hash: String,
    commitment_roots: Option<CommitmentRoots>,
    commitment_cache: Option<RuntimeCommitmentCache>,
}

impl ClockTransactionCheckpoint {
    fn capture(state: &RuntimeState) -> Self {
        Self {
            now: state.scheduler.now,
            plugin_registration_closed: state.metadata.plugin_registration_closed,
            checkpoint_hash: state.metadata.checkpoint_hash.clone(),
            commitment_roots: state.metadata.commitment_roots.clone(),
            commitment_cache: state.metadata.commitment_cache.clone(),
        }
    }

    fn restore(self, state: &mut RuntimeState) {
        state.scheduler.now = self.now;
        state.metadata.plugin_registration_closed = self.plugin_registration_closed;
        state.metadata.checkpoint_hash = self.checkpoint_hash;
        state.metadata.commitment_roots = self.commitment_roots;
        state.metadata.commitment_cache = self.commitment_cache;
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SimulationSnapshot {
    pub engine_version: String,
    pub snapshot_format_version: u32,
    #[serde(default)]
    pub run_manifest: Option<RunManifest>,
    #[serde(default)]
    pub run_manifest_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_configuration: Option<RunConfigurationSnapshot>,
    #[serde(default)]
    pub checkpoint_hash: String,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    /// Version of the domain-separated checkpoint commitment contract.
    pub commitment_format_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Persisted canonical roots verified before a snapshot becomes live.
    pub commitment_roots: Option<CommitmentRoots>,
    #[serde(default)]
    /// Version of the revision migration and checkpoint sub-contract.
    pub revision_format_version: u32,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    /// Monotonic revision after all persisted attempt and boundary transactions.
    pub state_revision: u64,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    /// Revision-evidence format available to exact replay; zero is migration-only.
    pub replay_revision_format_version: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    /// Version of the persisted boundary-admission cursor contract.
    pub admission_cursor_format_version: u32,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    /// Number of command-attempt records consumed by completed boundaries.
    pub admitted_attempt_count: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    /// Number of accepted-command records consumed by completed boundaries.
    pub admitted_command_count: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    /// Number of event records consumed as boundary ingress.
    pub admitted_event_count: u64,
    pub initial_time: SimTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_scenario: Option<Scenario>,
    pub now: SimTime,
    pub plugin_registration_closed: bool,
    pub world: WorldSnapshot,
    pub knowledge: KnowledgeSnapshot,
    pub events: Vec<SimEvent>,
    pub commands: Vec<CommandRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command_attempts: Vec<CommandAttemptRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ingress: Vec<IngressRecord>,
    #[serde(default)]
    pub boundaries: Vec<BoundaryRecord>,
    pub plugin_components: Vec<PluginComponentRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domain_records: Vec<DomainRecord>,
    pub plugin_descriptors: Vec<PluginDescriptor>,
    pub schema: SchemaRegistry,
    #[serde(default)]
    pub root_seed: u64,
    #[serde(default)]
    pub random_streams: Vec<RandomStreamState>,
    #[serde(default)]
    pub random_draws: Vec<RandomDrawRecord>,
    scheduled: Vec<ScheduledRecord>,
    #[serde(default, rename = "rng", skip_serializing_if = "Option::is_none")]
    legacy_rng: Option<DeterministicRng>,
    next_event_id: u64,
    next_command_id: u64,
    #[serde(default = "one_u64", skip_serializing_if = "is_one_u64")]
    next_command_attempt_id: u64,
    #[serde(default = "one_u64", skip_serializing_if = "is_one_u64")]
    next_ingress_id: u64,
    #[serde(default)]
    next_boundary_id: u64,
    #[serde(default)]
    next_random_draw_id: u64,
    next_schedule_sequence: u64,
    next_correlation_id: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
/// Complete recorded environment and input journal for exact replay.
pub struct ReplayJournal {
    pub engine_version: String,
    pub snapshot_format_version: u32,
    pub root_seed: u64,
    pub run_manifest: RunManifest,
    pub run_manifest_hash: String,
    pub run_configuration: RunConfigurationSnapshot,
    pub plugin_descriptors: Vec<PluginDescriptor>,
    pub plugin_registration_closed: bool,
    pub commands: Vec<CommandRecord>,
    pub command_attempts: Vec<CommandAttemptRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ingress: Vec<IngressRecord>,
    pub boundaries: Vec<BoundaryRecord>,
    pub final_time: SimTime,
    pub checkpoint_hash: String,
    /// Checkpoint commitment format reproduced by exact replay.
    pub commitment_format_version: u32,
    /// Revision-evidence format verified by this exact replay journal.
    pub revision_format_version: u32,
    /// Final persisted authoritative revision after replay.
    pub final_revision: u64,
}

#[derive(Deserialize)]
struct ReplayJournalWire {
    engine_version: String,
    snapshot_format_version: u32,
    root_seed: u64,
    run_manifest: RunManifest,
    run_manifest_hash: String,
    #[serde(default)]
    run_configuration: Option<RunConfigurationSnapshot>,
    plugin_descriptors: Vec<PluginDescriptor>,
    plugin_registration_closed: bool,
    commands: Vec<CommandRecord>,
    #[serde(default)]
    command_attempts: Vec<CommandAttemptRecord>,
    #[serde(default)]
    ingress: Vec<IngressRecord>,
    boundaries: Vec<BoundaryRecord>,
    final_time: SimTime,
    checkpoint_hash: String,
    #[serde(default)]
    commitment_format_version: u32,
    #[serde(default)]
    revision_format_version: u32,
    #[serde(default)]
    final_revision: u64,
}

impl<'de> Deserialize<'de> for ReplayJournal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ReplayJournalWire::deserialize(deserializer)?;
        let run_configuration = wire
            .run_configuration
            .map_or_else(|| inferred_run_configuration(&wire.run_manifest), Ok)
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            engine_version: wire.engine_version,
            snapshot_format_version: wire.snapshot_format_version,
            root_seed: wire.root_seed,
            run_manifest: wire.run_manifest,
            run_manifest_hash: wire.run_manifest_hash,
            run_configuration,
            plugin_descriptors: wire.plugin_descriptors,
            plugin_registration_closed: wire.plugin_registration_closed,
            commands: wire.commands,
            command_attempts: wire.command_attempts,
            ingress: wire.ingress,
            boundaries: wire.boundaries,
            final_time: wire.final_time,
            checkpoint_hash: wire.checkpoint_hash,
            commitment_format_version: wire.commitment_format_version,
            revision_format_version: wire.revision_format_version,
            final_revision: wire.final_revision,
        })
    }
}

pub struct Simulation {
    state: RuntimeState,
    schema: SchemaRegistry,
    plugins: PluginRegistry,
}

impl Simulation {
    /// Creates a simulation after validating that scenario references are sound.
    pub fn new(seed: u64, scenario: Scenario) -> Result<Self, CanwuError> {
        require_plugin_aware_initial_records(&scenario)?;
        let run_manifest = RunManifest::for_scenario("canwu.inline", "scenario", "1", &scenario)?;
        Self::new_with_configuration_snapshot(
            seed,
            scenario,
            run_manifest,
            RunConfigurationSnapshot::CompatibilityV1,
        )
    }

    /// Creates a simulation and activates the plugins required by initial
    /// application-defined records before returning a snapshot-capable runtime.
    pub fn new_with_plugins(
        seed: u64,
        scenario: Scenario,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        let run_manifest = RunManifest::for_scenario("canwu.inline", "scenario", "1", &scenario)?;
        Self::new_with_manifest_and_plugins(seed, scenario, run_manifest, plugins)
    }

    /// Creates a simulation with an exact, persisted run environment identity.
    pub fn new_with_manifest(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
    ) -> Result<Self, CanwuError> {
        require_plugin_aware_initial_records(&scenario)?;
        Self::new_with_configuration_snapshot(
            seed,
            scenario,
            run_manifest,
            RunConfigurationSnapshot::CompatibilityV1,
        )
    }

    /// Creates a manifested run and activates all initial domain packages before
    /// the runtime can be observed or snapshotted.
    pub fn new_with_manifest_and_plugins(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        let simulation = Self::new_with_configuration_snapshot(
            seed,
            scenario,
            run_manifest,
            RunConfigurationSnapshot::CompatibilityV1,
        )?;
        Self::activate_initial_plugins(simulation, plugins)
    }

    /// Creates a run whose six policy dimensions are persisted and bound to
    /// the run-configuration artifact in `run_manifest`.
    pub fn new_with_run_configuration(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        mut run_configuration: RunConfiguration,
    ) -> Result<Self, CanwuError> {
        require_plugin_aware_initial_records(&scenario)?;
        run_configuration.canonicalize();
        Self::new_with_configuration_snapshot(
            seed,
            scenario,
            run_manifest,
            RunConfigurationSnapshot::Declared(run_configuration),
        )
    }

    /// Creates a declared-policy run and activates all initial domain packages
    /// before the runtime can be observed or snapshotted.
    pub fn new_with_run_configuration_and_plugins(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        mut run_configuration: RunConfiguration,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        run_configuration.canonicalize();
        let simulation = Self::new_with_configuration_snapshot(
            seed,
            scenario,
            run_manifest,
            RunConfigurationSnapshot::Declared(run_configuration),
        )?;
        Self::activate_initial_plugins(simulation, plugins)
    }

    fn activate_initial_plugins(
        mut simulation: Self,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        for plugin in plugins {
            simulation.register_plugin(*plugin)?;
        }
        simulation.ensure_runtime_ready()?;
        Ok(simulation)
    }

    fn new_with_configuration_snapshot(
        seed: u64,
        mut scenario: Scenario,
        mut run_manifest: RunManifest,
        run_configuration: RunConfigurationSnapshot,
    ) -> Result<Self, CanwuError> {
        canonicalize_scenario(&mut scenario);
        validate_scenario(&scenario)?;
        manifest::canonicalize(&mut run_manifest);
        manifest::validate(&run_manifest, Some(&scenario), false)?;
        manifest::validate_run_configuration(&run_manifest, &run_configuration)?;
        validate_run_configuration_entities(
            &run_configuration,
            &scenario.world,
            &scenario.domain_records,
        )?;
        let run_manifest_hash = manifest::hash(&run_manifest)?;
        if scenario
            .world
            .armies
            .iter()
            .any(|army| army.transit.is_some())
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "initial scenarios cannot contain transit without admitted command/event/queue evidence",
            ));
        }
        let schema = base_schema();
        let plugins = PluginRegistry::default();
        let core_stream = RandomStreamState::initial(seed, random::core_report_delay_stream());
        let initial_scenario = Some(scenario.clone());
        let mut simulation = Self {
            state: RuntimeState {
                current: RuntimeCurrentState {
                    people: scenario
                        .world
                        .people
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    governments: scenario
                        .world
                        .governments
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    territories: scenario
                        .world
                        .territories
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    routes: scenario
                        .world
                        .routes
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    armies: scenario
                        .world
                        .armies
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    knowledge: scenario.knowledge,
                    plugin_components: BTreeMap::new(),
                    domain_records: scenario
                        .domain_records
                        .into_iter()
                        .map(|record| (record.reference.clone(), record))
                        .collect(),
                    root_seed: seed,
                    random_streams: BTreeMap::from([(core_stream.key.clone(), core_stream)]),
                },
                scheduler: RuntimeScheduler {
                    initial_time: scenario.start_time,
                    now: scenario.start_time,
                    actions: BTreeMap::new(),
                    pending_ingress: BTreeSet::new(),
                },
                counters: RuntimeCounters {
                    next_event_id: 1,
                    next_command_id: 1,
                    next_command_attempt_id: 1,
                    next_ingress_id: 1,
                    next_boundary_id: 1,
                    next_random_draw_id: 1,
                    next_schedule_sequence: 1,
                    next_correlation_id: 1,
                    state_revision: 0,
                    admitted_attempt_count: 0,
                    admitted_command_count: 0,
                    admitted_event_count: 0,
                },
                metadata: RuntimeMetadata {
                    initial_scenario,
                    run_manifest,
                    run_manifest_hash,
                    run_configuration,
                    checkpoint_hash: String::new(),
                    commitment_format_version: COMMITMENT_FORMAT_VERSION,
                    commitment_roots: None,
                    commitment_cache: None,
                    plugin_registration_closed: false,
                    replay_revision_format_version: STATE_REVISION_FORMAT_VERSION,
                },
                evidence: RuntimeEvidence {
                    events: Vec::new(),
                    commands: Vec::new(),
                    command_attempts: Vec::new(),
                    ingress: Vec::new(),
                    boundaries: Vec::new(),
                    random_draws: Vec::new(),
                },
            },
            schema,
            plugins,
        };
        simulation.refresh_checkpoint_hash()?;
        Ok(simulation)
    }

    pub fn demo(seed: u64) -> Result<(Self, DemoIds), CanwuError> {
        let (scenario, ids) = demo_scenario();
        Self::new(seed, scenario).map(|simulation| (simulation, ids))
    }

    /// Reconstructs caller-supplied core commands without proving a recorded
    /// package environment. Use [`Self::replay_from_journal`] for exact replay.
    pub fn replay(
        seed: u64,
        scenario: Scenario,
        commands: &[CommandRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        Self::replay_with_plugins(seed, scenario, &[], commands, final_time)
    }

    /// Reconstructs caller-supplied inputs under caller-supplied plugins.
    /// This is not an exact replay identity check.
    pub fn replay_with_plugins(
        seed: u64,
        scenario: Scenario,
        plugins: &[&dyn SimulationPlugin],
        commands: &[CommandRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        Self::replay_with_boundaries(seed, scenario, plugins, commands, &[], final_time)
    }

    /// Reconstructs caller-supplied inputs and compares supplied boundaries.
    /// Use [`Self::replay_from_journal`] when command-only runs must also bind
    /// their recorded run and plugin identities.
    pub fn replay_with_boundaries(
        seed: u64,
        scenario: Scenario,
        plugins: &[&dyn SimulationPlugin],
        commands: &[CommandRecord],
        boundaries: &[BoundaryRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        let run_manifest = RunManifest::for_scenario("canwu.inline", "scenario", "1", &scenario)?;
        Self::replay_with_run_manifest(
            seed,
            scenario,
            run_manifest,
            plugins,
            commands,
            boundaries,
            final_time,
        )
    }

    /// Reconstructs caller-supplied inputs under a caller-supplied run manifest.
    /// This is useful for fixtures; it does not establish recorded identity.
    pub fn replay_with_run_manifest(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        plugins: &[&dyn SimulationPlugin],
        commands: &[CommandRecord],
        boundaries: &[BoundaryRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        let simulation =
            Self::new_with_manifest_and_plugins(seed, scenario, run_manifest, plugins)?;
        Self::replay_records(simulation, commands, &[], &[], boundaries, final_time)
    }

    /// Reconstructs caller-supplied inputs under a caller-supplied declared run
    /// configuration. This does not establish recorded-environment identity.
    #[allow(clippy::too_many_arguments)]
    pub fn replay_with_run_configuration(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        run_configuration: RunConfiguration,
        plugins: &[&dyn SimulationPlugin],
        commands: &[CommandRecord],
        command_attempts: &[CommandAttemptRecord],
        boundaries: &[BoundaryRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        if command_attempts
            .iter()
            .any(|attempt| attempt.ingress == CommandIngress::FrozenReplay)
        {
            return Err(CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                "frozen replay attempts require an environment-bound replay journal",
            ));
        }
        let simulation = Self::new_with_run_configuration_and_plugins(
            seed,
            scenario,
            run_manifest,
            run_configuration,
            plugins,
        )?;
        Self::replay_records(
            simulation,
            commands,
            command_attempts,
            &[],
            boundaries,
            final_time,
        )
    }

    /// Replays only after the recorded engine, run, seed, and plugin manifests
    /// match, then verifies the final checkpoint commitment.
    pub fn replay_from_journal(
        scenario: Scenario,
        plugins: &[&dyn SimulationPlugin],
        journal: &ReplayJournal,
    ) -> Result<Self, CanwuError> {
        if !matches!(
            journal.commitment_format_version,
            0 | COMMITMENT_FORMAT_VERSION
        ) {
            return Err(CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                format!(
                    "replay journal commitment format {} is unsupported; this engine reads legacy format 0 and current format {COMMITMENT_FORMAT_VERSION}",
                    journal.commitment_format_version
                ),
            ));
        }
        if journal.revision_format_version != STATE_REVISION_FORMAT_VERSION {
            return Err(CanwuError::new(
                ErrorCode::LegacyReplayUnavailable,
                "legacy revision histories can continue after snapshot migration but cannot claim exact replay because historical state commitments cannot be reconstructed without their original runtime",
            ));
        }
        let expected_final_revision = authoritative_revision_count(
            journal.commands.len(),
            journal.command_attempts.len(),
            journal.boundaries.len(),
        )?;
        if journal.final_revision != expected_final_revision {
            return Err(CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                "replay journal final revision is inconsistent with its committed evidence",
            ));
        }
        let normalized = journal;
        if matches!(journal.run_manifest, RunManifest::MigratedLegacy { .. }) {
            return Err(CanwuError::new(
                ErrorCode::LegacyReplayUnavailable,
                "legacy checkpoints can continue after migration but lack enough recorded identity for exact replay",
            ));
        }
        manifest::validate(&normalized.run_manifest, Some(&scenario), false)?;
        manifest::validate_run_configuration(
            &normalized.run_manifest,
            &normalized.run_configuration,
        )?;
        if normalized.engine_version != ENGINE_VERSION
            || normalized.snapshot_format_version != SNAPSHOT_FORMAT_VERSION
            || !is_canonical_hash(&normalized.run_manifest_hash)
            || manifest::hash(&normalized.run_manifest)? != normalized.run_manifest_hash
            || !is_canonical_hash(&normalized.checkpoint_hash)
        {
            return Err(CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                "replay journal engine, format, or run identity does not match this runtime",
            ));
        }
        PluginRegistry::from_descriptors(normalized.plugin_descriptors.clone()).map_err(
            |error| {
                CanwuError::new(
                    ErrorCode::ReplayEnvironmentMismatch,
                    format!("replay journal plugin manifest is invalid: {error}"),
                )
            },
        )?;

        let mut simulation = Self::new_with_configuration_snapshot(
            normalized.root_seed,
            scenario,
            normalized.run_manifest.clone(),
            normalized.run_configuration.clone(),
        )?;
        if normalized.commitment_format_version == 0 {
            simulation.state.metadata.commitment_format_version = 0;
            simulation.state.metadata.commitment_roots = None;
            simulation.state.metadata.commitment_cache = None;
            simulation.refresh_checkpoint_hash()?;
        }
        let simulation = Self::activate_initial_plugins(simulation, plugins)?;
        let actual_descriptors: Vec<_> = simulation.plugin_descriptors().cloned().collect();
        if actual_descriptors != normalized.plugin_descriptors {
            return Err(CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                "active plugin identities and contracts do not match the replay journal",
            ));
        }

        let mut simulation = Self::replay_records(
            simulation,
            &normalized.commands,
            &normalized.command_attempts,
            &normalized.ingress,
            &normalized.boundaries,
            normalized.final_time,
        )?;
        if normalized.plugin_registration_closed
            && !simulation.state.metadata.plugin_registration_closed
        {
            simulation.advance(SimDuration::ZERO)?;
        }
        if simulation.state.metadata.plugin_registration_closed
            != normalized.plugin_registration_closed
        {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "replayed plugin-registration lifecycle does not match the recorded journal",
            ));
        }
        if simulation.revision() != normalized.final_revision {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "replayed final state revision does not match the recorded journal",
            ));
        }
        if simulation.checkpoint_hash() != normalized.checkpoint_hash {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "replayed final checkpoint does not match the recorded journal",
            ));
        }
        Ok(simulation)
    }

    fn replay_records(
        mut simulation: Self,
        commands: &[CommandRecord],
        attempts: &[CommandAttemptRecord],
        ingress: &[IngressRecord],
        boundaries: &[BoundaryRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        simulation.ensure_runtime_ready()?;
        if !attempts.is_empty() {
            return Self::replay_attempt_records(
                simulation, commands, attempts, ingress, boundaries, final_time,
            );
        }
        let mut next_ingress = 0;
        let mut next_command = 0;
        for (boundary_index, expected_boundary) in boundaries.iter().enumerate() {
            enqueue_replay_ingress_cut(
                &mut simulation,
                ingress,
                &mut next_ingress,
                boundary_index,
            )?;
            for admitted in &expected_boundary.admitted_commands {
                let Some(record) = commands.get(next_command) else {
                    return Err(CanwuError::new(
                        ErrorCode::ReplayMismatch,
                        "boundary replay admits a command absent from the journal",
                    ));
                };
                if record.id != *admitted {
                    return Err(CanwuError::new(
                        ErrorCode::ReplayMismatch,
                        "boundary replay command admission does not match journal order",
                    ));
                }
                replay_command_record(&mut simulation, record, expected_boundary.at)?;
                next_command += 1;
            }
            let receipt = simulation.settle_boundary(BoundaryRequest {
                at: expected_boundary.at,
                cadences: expected_boundary.cadences.clone(),
            })?;
            let Some(actual_boundary) = simulation.boundaries().last() else {
                return Err(CanwuError::new(
                    ErrorCode::ReplayMismatch,
                    "boundary replay did not append settlement evidence",
                ));
            };
            if receipt.boundary_id != expected_boundary.id || actual_boundary != expected_boundary {
                return Err(CanwuError::new(
                    ErrorCode::ReplayMismatch,
                    format!(
                        "regenerated boundary {} did not match its journal evidence",
                        expected_boundary.id
                    ),
                ));
            }
        }
        for record in &commands[next_command..] {
            replay_command_record(&mut simulation, record, final_time)?;
        }
        enqueue_replay_ingress_cut(
            &mut simulation,
            ingress,
            &mut next_ingress,
            boundaries.len(),
        )?;
        if next_ingress != ingress.len() {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "ingress journal contains an impossible future boundary issue cut",
            ));
        }
        if final_time < simulation.time() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "replay final time cannot precede the last command",
            ));
        }
        if final_time > simulation.time() {
            simulation.ensure_legacy_advance_does_not_cross_ingress(final_time)?;
            simulation.advance_to(final_time)?;
        }
        Ok(simulation)
    }

    fn replay_attempt_records(
        mut simulation: Self,
        commands: &[CommandRecord],
        attempts: &[CommandAttemptRecord],
        ingress: &[IngressRecord],
        boundaries: &[BoundaryRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        let command_ingress_requests: BTreeSet<_> = ingress
            .iter()
            .filter_map(|record| match &record.payload {
                IngressPayload::Command { request } => Some(request.request_id),
                IngressPayload::Plugin { .. } | IngressPayload::Calendar { .. } => None,
            })
            .collect();
        let mut next_ingress = 0;
        let mut next_attempt = 0;
        for (boundary_index, expected_boundary) in boundaries.iter().enumerate() {
            enqueue_replay_ingress_cut(
                &mut simulation,
                ingress,
                &mut next_ingress,
                boundary_index,
            )?;
            let mut admitted_commands = Vec::new();
            for admitted in &expected_boundary.admitted_attempts {
                let Some(record) = attempts.get(next_attempt) else {
                    return Err(CanwuError::new(
                        ErrorCode::ReplayMismatch,
                        "boundary replay admits a command attempt absent from the journal",
                    ));
                };
                if record.id != *admitted {
                    return Err(CanwuError::new(
                        ErrorCode::ReplayMismatch,
                        "boundary replay attempt admission does not match journal order",
                    ));
                }
                let queued = record
                    .request_id
                    .is_some_and(|request| command_ingress_requests.contains(&request));
                if !queued {
                    replay_attempt_record(&mut simulation, record, commands, expected_boundary.at)?;
                }
                if let CommandAttemptOutcome::Accepted { command_id } = record.outcome {
                    admitted_commands.push(command_id);
                }
                next_attempt += 1;
            }
            if admitted_commands != expected_boundary.admitted_commands {
                return Err(CanwuError::new(
                    ErrorCode::ReplayMismatch,
                    "boundary replay accepted-command cut disagrees with admitted attempts",
                ));
            }
            let receipt = simulation.settle_boundary(BoundaryRequest {
                at: expected_boundary.at,
                cadences: expected_boundary.cadences.clone(),
            })?;
            let Some(actual_boundary) = simulation.boundaries().last() else {
                return Err(CanwuError::new(
                    ErrorCode::ReplayMismatch,
                    "boundary replay did not append settlement evidence",
                ));
            };
            if receipt.boundary_id != expected_boundary.id || actual_boundary != expected_boundary {
                return Err(CanwuError::new(
                    ErrorCode::ReplayMismatch,
                    format!(
                        "regenerated boundary {} did not match its journal evidence",
                        expected_boundary.id
                    ),
                ));
            }
        }
        for record in &attempts[next_attempt..] {
            replay_attempt_record(&mut simulation, record, commands, final_time)?;
        }
        enqueue_replay_ingress_cut(
            &mut simulation,
            ingress,
            &mut next_ingress,
            boundaries.len(),
        )?;
        if next_ingress != ingress.len() {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "ingress journal contains an impossible future boundary issue cut",
            ));
        }
        if simulation.command_log() != commands {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "replayed accepted command journal does not match its recorded evidence",
            ));
        }
        if final_time < simulation.time() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "replay final time cannot precede the last command attempt",
            ));
        }
        if final_time > simulation.time() {
            simulation.ensure_legacy_advance_does_not_cross_ingress(final_time)?;
            simulation.advance_to(final_time)?;
        }
        Ok(simulation)
    }

    pub fn register_plugin<P: SimulationPlugin + ?Sized>(
        &mut self,
        plugin: &P,
    ) -> Result<(), CanwuError> {
        let plugin_name = plugin.name().trim();
        if plugin_name.is_empty() || plugin_name != plugin.name() {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "plugin name must be non-empty and have no surrounding whitespace",
            ));
        }
        let rehydrating = self.plugins.descriptors.contains_key(plugin_name)
            && !self.plugins.active_plugins.contains(plugin_name);
        if self.state.metadata.plugin_registration_closed && !rehydrating {
            return Err(CanwuError::new(
                ErrorCode::PluginRegistrationClosed,
                "new plugins must be registered before authoritative execution begins",
            ));
        }
        let state_start = self.state.clone();
        let schema_start = self.schema.clone();
        let plugins_start = self.plugins.clone();
        let result = (|| {
            self.plugins.register(plugin, &mut self.schema)?;
            self.invalidate_commitments(
                CommitmentDomains::RANDOM_STREAMS | CommitmentDomains::IDENTITY,
            );
            if !self.plugins.record_schemas.is_empty()
                && self.state.metadata.initial_scenario.is_none()
            {
                return Err(CanwuError::new(
                    ErrorCode::UnsupportedSnapshotVersion,
                    "this snapshot predates manifest-bound domain-record genesis and cannot activate record schemas",
                ));
            }
            records::validate_records_for_owner(
                &self.state.current.domain_records,
                &self.plugins.record_schemas,
                plugin_name,
                self.state.scheduler.now,
                &|entity| runtime_entity_exists(&self.state, entity),
            )?;
            for stream in self.plugins.random_stream_owners.keys() {
                self.state
                    .current
                    .random_streams
                    .entry(stream.clone())
                    .or_insert_with(|| {
                        RandomStreamState::initial(self.state.current.root_seed, stream.clone())
                    });
            }
            self.refresh_checkpoint_hash()
        })();
        if let Err(error) = result {
            self.state = state_start;
            self.schema = schema_start;
            self.plugins = plugins_start;
            return Err(error);
        }
        Ok(())
    }

    fn ensure_runtime_ready(&self) -> Result<(), CanwuError> {
        self.plugins.ensure_active()?;
        records::validate_record_store(
            &self.state.current.domain_records,
            &self.plugins.record_schemas,
            self.state.scheduler.now,
            &|entity| runtime_entity_exists(&self.state, entity),
        )
    }

    fn domain_record_feature_enabled(&self) -> bool {
        !self.plugins.record_schemas.is_empty()
            || !self.state.current.domain_records.is_empty()
            || self
                .state
                .evidence
                .boundaries
                .iter()
                .any(|boundary| !boundary.record_changes.is_empty())
    }

    fn bound_initial_scenario(&self) -> Option<&Scenario> {
        if self.domain_record_feature_enabled() {
            self.state.metadata.initial_scenario.as_ref()
        } else {
            None
        }
    }

    #[must_use]
    pub const fn time(&self) -> SimTime {
        self.state.scheduler.now
    }

    #[must_use]
    pub const fn run_manifest(&self) -> &RunManifest {
        &self.state.metadata.run_manifest
    }

    #[must_use]
    pub const fn run_configuration(&self) -> &RunConfigurationSnapshot {
        &self.state.metadata.run_configuration
    }

    #[must_use]
    /// Returns the persisted authoritative transaction revision.
    ///
    /// Accepted commands, persisted expected rejections, and completed
    /// settlement boundaries each advance it exactly once. Failed work, exact
    /// retries, bare clock movement, queued but unadmitted ingress, and plugin
    /// setup do not advance it; use the expected-time guard with external
    /// commands to detect clock and scheduled-work advancement.
    pub const fn revision(&self) -> u64 {
        self.state.counters.state_revision
    }

    #[must_use]
    pub fn run_manifest_hash(&self) -> &str {
        &self.state.metadata.run_manifest_hash
    }

    #[must_use]
    pub fn checkpoint_hash(&self) -> &str {
        &self.state.metadata.checkpoint_hash
    }

    /// Hash of simulated state and causal evidence. Run-purpose, controller,
    /// seat, observation, interaction, and trace policy remain save identity
    /// but are deliberately excluded from this authoritative result identity.
    pub fn authoritative_state_hash(&self) -> Result<String, CanwuError> {
        self.compute_boundary_state_hash()
    }

    #[must_use]
    pub fn world(&self) -> WorldSnapshot {
        WorldSnapshot {
            people: self.state.current.people.values().cloned().collect(),
            governments: self.state.current.governments.values().cloned().collect(),
            territories: self.state.current.territories.values().cloned().collect(),
            routes: self.state.current.routes.values().cloned().collect(),
            armies: self.state.current.armies.values().cloned().collect(),
        }
    }

    #[must_use]
    pub fn knowledge(&self) -> &KnowledgeSnapshot {
        &self.state.current.knowledge
    }

    #[must_use]
    pub fn events(&self) -> &[SimEvent] {
        &self.state.evidence.events
    }

    #[must_use]
    pub fn command_log(&self) -> &[CommandRecord] {
        &self.state.evidence.commands
    }

    #[must_use]
    pub fn command_attempts(&self) -> &[CommandAttemptRecord] {
        &self.state.evidence.command_attempts
    }

    #[must_use]
    pub fn ingress_log(&self) -> &[IngressRecord] {
        &self.state.evidence.ingress
    }

    #[must_use]
    pub fn domain_record(&self, reference: &DomainRecordRef) -> Option<&DomainRecord> {
        self.state.current.domain_records.get(reference)
    }

    pub fn domain_records(&self) -> impl Iterator<Item = &DomainRecord> {
        self.state.current.domain_records.values()
    }

    #[must_use]
    pub fn boundaries(&self) -> &[BoundaryRecord] {
        &self.state.evidence.boundaries
    }

    #[must_use]
    pub fn random_draws(&self) -> &[RandomDrawRecord] {
        &self.state.evidence.random_draws
    }

    #[must_use]
    pub fn boundary_head_hash(&self) -> Option<&str> {
        self.state
            .evidence
            .boundaries
            .last()
            .map(|record| record.hash.as_str())
    }

    #[must_use]
    pub const fn schema(&self) -> &SchemaRegistry {
        &self.schema
    }

    pub fn plugin_descriptors(&self) -> impl Iterator<Item = &PluginDescriptor> {
        self.plugins.descriptors()
    }

    #[must_use]
    pub fn replay_journal(&self) -> ReplayJournal {
        ReplayJournal {
            engine_version: ENGINE_VERSION.to_owned(),
            snapshot_format_version: SNAPSHOT_FORMAT_VERSION,
            root_seed: self.state.current.root_seed,
            run_manifest: self.state.metadata.run_manifest.clone(),
            run_manifest_hash: self.state.metadata.run_manifest_hash.clone(),
            run_configuration: self.state.metadata.run_configuration.clone(),
            plugin_descriptors: self.plugins.descriptors().cloned().collect(),
            plugin_registration_closed: self.state.metadata.plugin_registration_closed,
            commands: self.state.evidence.commands.clone(),
            command_attempts: self.state.evidence.command_attempts.clone(),
            ingress: self.state.evidence.ingress.clone(),
            boundaries: self.state.evidence.boundaries.clone(),
            final_time: self.state.scheduler.now,
            checkpoint_hash: self.state.metadata.checkpoint_hash.clone(),
            commitment_format_version: self.state.metadata.commitment_format_version,
            revision_format_version: self.state.metadata.replay_revision_format_version,
            final_revision: self.state.counters.state_revision,
        }
    }

    pub fn submit(&mut self, envelope: CommandEnvelope) -> Result<CommandReceipt, CanwuError> {
        match self.admit_command(None, None, envelope, CommandIngress::LegacyDirect, false)? {
            CommandOutcome::Accepted { receipt } => Ok(receipt),
            CommandOutcome::Rejected { rejection } => Err(rejection.error),
        }
    }

    pub fn enqueue_command(
        &mut self,
        due_at: SimTime,
        priority: i32,
        request: CommandRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        self.ensure_runtime_ready()?;
        self.ensure_canonical_ingress_can_start()?;
        self.ensure_command_ingress_family(CommandIngress::LiveRequest)?;
        for record in &self.state.evidence.ingress {
            let IngressPayload::Command { request: existing } = &record.payload else {
                continue;
            };
            if existing.request_id != request.request_id {
                continue;
            }
            if existing.as_ref() == &request
                && record.due_at == due_at
                && record.priority == priority
            {
                return Ok(IngressReceipt {
                    ingress_id: record.id,
                    issued_at: record.issued_at,
                    due_at: record.due_at,
                });
            }
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                format!(
                    "command request {} is already queued with different ingress content",
                    request.request_id
                ),
            ));
        }
        if self
            .state
            .evidence
            .command_attempts
            .iter()
            .any(|attempt| attempt.request_id == Some(request.request_id))
        {
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                format!(
                    "command request {} was already processed outside canonical ingress",
                    request.request_id
                ),
            ));
        }
        if request
            .envelope
            .expected_time
            .is_some_and(|expected| expected != due_at)
        {
            return Err(CanwuError::new(
                ErrorCode::SimulationTimeConflict,
                "queued command expected time must equal its due simulation time",
            ));
        }
        self.append_ingress(
            due_at,
            IngressClass::Command,
            priority,
            IngressPayload::Command {
                request: Box::new(request),
            },
            None,
            false,
        )
    }

    pub fn enqueue_plugin_ingress(
        &mut self,
        mut request: PluginIngressRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        self.ensure_runtime_ready()?;
        self.ensure_canonical_ingress_can_start()?;
        if self
            .state
            .metadata
            .run_configuration
            .declared()
            .is_some_and(|configuration| configuration.interaction == InteractionPolicy::ReadOnly)
        {
            return Err(CanwuError::new(
                ErrorCode::InteractionReadOnly,
                "the run interaction policy rejects newly authored plugin ingress",
            ));
        }
        let key = (request.plugin.clone(), request.packet_type.clone());
        let descriptor = self.plugins.ingress.get(&key).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!(
                    "plugin ingress type {}.{} is not registered",
                    request.plugin, request.packet_type
                ),
            )
        })?;
        descriptor.payload_schema.validate(&request.payload)?;
        request.affected_entities.sort();
        request.affected_entities.dedup();
        if request
            .affected_entities
            .iter()
            .any(|entity| !runtime_entity_identity_exists(&self.state, entity))
        {
            return Err(CanwuError::new(
                ErrorCode::EntityNotFound,
                "plugin ingress references an unknown entity identity",
            ));
        }
        if let Some(cause) = &request.cause {
            if matches!(
                cause,
                CauseRef::Boundary(_) | CauseRef::Command(_) | CauseRef::Event(_)
            ) {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPayload,
                    "boundary, command, and event causes are reserved for plugin-generated ingress",
                ));
            }
            validate_runtime_cause(&self.state, cause)?;
        }
        self.append_ingress(
            request.due_at,
            descriptor.class,
            request.priority,
            IngressPayload::Plugin {
                plugin: request.plugin,
                packet_type: request.packet_type,
                payload: request.payload,
                affected_entities: request.affected_entities,
            },
            request.cause,
            false,
        )
    }

    pub fn schedule_calendar_boundary(
        &mut self,
        due_at: SimTime,
        mut cadences: Vec<SystemCadence>,
    ) -> Result<IngressReceipt, CanwuError> {
        self.ensure_runtime_ready()?;
        self.ensure_canonical_ingress_can_start()?;
        if cadences.contains(&SystemCadence::EventDriven) {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "calendar ingress cannot declare event-driven cadence",
            ));
        }
        cadences.sort();
        cadences.dedup();
        if cadences.is_empty() {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "calendar ingress requires at least one scheduled cadence",
            ));
        }
        self.append_ingress(
            due_at,
            IngressClass::ScheduledSystem,
            0,
            IngressPayload::Calendar { cadences },
            Some(CauseRef::System("canwu.core.calendar".to_owned())),
            false,
        )
    }

    fn append_ingress(
        &mut self,
        due_at: SimTime,
        class: IngressClass,
        priority: i32,
        payload: IngressPayload,
        cause: Option<CauseRef>,
        after_current_boundary: bool,
    ) -> Result<IngressReceipt, CanwuError> {
        if due_at < self.state.scheduler.now {
            return Err(CanwuError::new(
                ErrorCode::LateIngress,
                format!(
                    "ingress due at {due_at} cannot be queued after committed time {}",
                    self.state.scheduler.now
                ),
            ));
        }
        let transaction_start = self.state.clone();
        let (id, next_id) = claim_counter(self.state.counters.next_ingress_id, "ingress ID")?;
        let boundary_count = u64::try_from(self.state.evidence.boundaries.len()).map_err(|_| {
            CanwuError::new(
                ErrorCode::IdentifierExhausted,
                "boundary count exceeds the ingress journal range",
            )
        })?;
        let eligible_boundary_count = if after_current_boundary {
            boundary_count.checked_add(1).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::IdentifierExhausted,
                    "ingress boundary eligibility exceeds the journal range",
                )
            })?
        } else {
            boundary_count
        };
        let record = IngressRecord {
            id: IngressId::new(id),
            issued_at: self.state.scheduler.now,
            eligible_boundary_count,
            due_at,
            class,
            priority,
            payload,
            cause,
        };
        self.state.counters.next_ingress_id = next_id;
        self.state
            .scheduler
            .pending_ingress
            .insert(IngressQueueKey::from_record(&record));
        self.state.evidence.ingress.push(record.clone());
        self.state.metadata.plugin_registration_closed = true;
        if let Err(error) = self.refresh_checkpoint_hash() {
            self.state = transaction_start;
            return Err(error);
        }
        Ok(IngressReceipt {
            ingress_id: record.id,
            issued_at: record.issued_at,
            due_at: record.due_at,
        })
    }

    pub fn process_command(
        &mut self,
        request: CommandRequest,
    ) -> Result<CommandOutcome, CanwuError> {
        self.ensure_runtime_ready()?;
        if !self.state.evidence.ingress.is_empty() {
            return Err(CanwuError::new(
                ErrorCode::MixedCommandIngress,
                "direct command requests cannot bypass an active canonical ingress journal",
            ));
        }
        self.admit_command(
            Some(request.request_id),
            Some(request.expected_revision),
            request.envelope,
            CommandIngress::LiveRequest,
            true,
        )
    }

    fn admit_command(
        &mut self,
        request_id: Option<CommandRequestId>,
        expected_revision: Option<u64>,
        envelope: CommandEnvelope,
        ingress: CommandIngress,
        record_attempt: bool,
    ) -> Result<CommandOutcome, CanwuError> {
        self.ensure_runtime_ready()?;
        self.ensure_command_ingress_family(ingress)?;
        if let Some(cached) =
            self.cached_command_outcome(request_id, expected_revision, &envelope)?
        {
            return Ok(cached);
        }

        let revision_before = self.revision();
        let admission = CommandAdmission {
            request_id,
            expected_revision,
            expected_time: envelope.expected_time,
            revision_before,
            ingress,
        };
        let attempt_id = if record_attempt {
            let (value, _) = claim_counter(
                self.state.counters.next_command_attempt_id,
                "command attempt ID",
            )?;
            CommandAttemptId::new(value)
        } else {
            CommandAttemptId::default()
        };
        let authority = match resolve_command_authority(&envelope) {
            Ok(authority) => authority,
            Err(error) if is_expected_command_rejection(&error.code) && record_attempt => {
                return self.record_command_rejection(attempt_id, admission, envelope, error);
            }
            Err(error) => return Err(error),
        };
        if let Err(error) = self.validate_command_ingress(&envelope.issuer, &authority, admission) {
            if is_expected_command_rejection(&error.code) && record_attempt {
                return self.record_command_rejection(attempt_id, admission, envelope, error);
            }
            return Err(error);
        }
        if let Some(expected_time) = envelope.expected_time
            && expected_time != self.state.scheduler.now
        {
            let error = CanwuError::new(
                ErrorCode::SimulationTimeConflict,
                format!(
                    "command expected time {expected_time}, but simulation is at {}",
                    self.state.scheduler.now
                ),
            );
            if record_attempt {
                return self.record_command_rejection(attempt_id, admission, envelope, error);
            }
            return Err(error);
        }

        let (command_id_value, next_command_id) =
            claim_counter(self.state.counters.next_command_id, "command ID")?;
        let (correlation_id, next_correlation_id) =
            claim_counter(self.state.counters.next_correlation_id, "correlation ID")?;
        let command_id = CommandId::new(command_id_value);
        let context = CommandContext {
            issuer: envelope.issuer.clone(),
            authority,
            run_policy: self.state.metadata.run_configuration.command_policy(),
            ingress: admission.ingress,
            attempt_id: record_attempt.then_some(attempt_id),
            command_id,
            request_id: admission.request_id,
            revision: admission.revision_before,
            simulation_time: self.state.scheduler.now,
            expected_revision: admission.expected_revision,
            expected_time: envelope.expected_time,
        };
        let prepared = match self.prepare_command(&envelope, &context) {
            Ok(prepared) => prepared,
            Err(error) if is_expected_command_rejection(&error.code) && record_attempt => {
                return self.record_command_rejection(attempt_id, admission, envelope, error);
            }
            Err(error) => return Err(error),
        };
        let next_attempt_id = if record_attempt {
            let (claimed_id, next_attempt_id) = claim_counter(
                self.state.counters.next_command_attempt_id,
                "command attempt ID",
            )?;
            if claimed_id != attempt_id.get() {
                return Err(CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    "command attempt allocation changed during application",
                ));
            }
            Some(next_attempt_id)
        } else {
            None
        };
        let revision = self.next_state_revision()?;
        let transaction = CommandTransactionCheckpoint::capture(&self.state);
        let event_start = self.state.evidence.events.len();
        self.state.counters.next_command_id = next_command_id;
        self.state.counters.next_correlation_id = next_correlation_id;
        self.invalidate_commitments(prepared.commitment_invalidation());

        if let Err(error) = self.apply_prepared(prepared, command_id, correlation_id) {
            transaction.restore(&mut self.state);
            if is_expected_command_rejection(&error.code) && record_attempt {
                return self.record_command_rejection(attempt_id, admission, envelope, error);
            }
            return Err(error);
        }
        let emitted_events: Vec<_> = self.state.evidence.events[event_start..]
            .iter()
            .map(|event| event.id)
            .collect();
        self.state.metadata.plugin_registration_closed = true;
        self.state.evidence.commands.push(CommandRecord {
            id: command_id,
            attempt_id: record_attempt.then_some(attempt_id),
            accepted_at: self.state.scheduler.now,
            envelope: envelope.clone(),
            emitted_events: if record_attempt {
                emitted_events.clone()
            } else {
                Vec::new()
            },
        });
        if let Some(next_attempt_id) = next_attempt_id {
            self.state.counters.next_command_attempt_id = next_attempt_id;
            self.state
                .evidence
                .command_attempts
                .push(CommandAttemptRecord {
                    id: attempt_id,
                    at: self.state.scheduler.now,
                    revision_before: admission.revision_before,
                    ingress: admission.ingress,
                    request_id: admission.request_id,
                    expected_revision: admission.expected_revision,
                    envelope,
                    outcome: CommandAttemptOutcome::Accepted { command_id },
                });
        }
        self.state.counters.state_revision = revision;
        if let Err(error) = self.refresh_checkpoint_hash() {
            transaction.restore(&mut self.state);
            return Err(error);
        }

        Ok(CommandOutcome::Accepted {
            receipt: CommandReceipt {
                attempt_id: record_attempt.then_some(attempt_id),
                command_id,
                request_id: admission.request_id,
                revision,
                accepted_at: self.state.scheduler.now,
                emitted_events,
            },
        })
    }

    fn ensure_command_ingress_family(&self, ingress: CommandIngress) -> Result<(), CanwuError> {
        let has_legacy_commands = self
            .state
            .evidence
            .commands
            .iter()
            .any(|record| record.attempt_id.is_none());
        let has_tracked_attempts = !self.state.evidence.command_attempts.is_empty()
            || !self.state.evidence.ingress.is_empty();
        if (ingress == CommandIngress::LegacyDirect && has_tracked_attempts)
            || (ingress != CommandIngress::LegacyDirect && has_legacy_commands)
        {
            return Err(CanwuError::new(
                ErrorCode::MixedCommandIngress,
                "legacy-direct commands and tracked request/replay attempts cannot coexist in one run",
            ));
        }
        Ok(())
    }

    fn ensure_canonical_ingress_can_start(&self) -> Result<(), CanwuError> {
        if runtime_has_unqueued_command_history(&self.state) {
            return Err(CanwuError::new(
                ErrorCode::MixedCommandIngress,
                "canonical ingress cannot be added after direct command history",
            ));
        }
        Ok(())
    }

    fn cached_command_outcome(
        &self,
        request_id: Option<CommandRequestId>,
        expected_revision: Option<u64>,
        envelope: &CommandEnvelope,
    ) -> Result<Option<CommandOutcome>, CanwuError> {
        let Some(request_id) = request_id else {
            return Ok(None);
        };
        let Some(attempt) = self
            .state
            .evidence
            .command_attempts
            .iter()
            .find(|attempt| attempt.request_id == Some(request_id))
        else {
            return Ok(None);
        };
        if attempt.expected_revision != expected_revision || &attempt.envelope != envelope {
            return Ok(Some(CommandOutcome::Rejected {
                rejection: CommandRejection {
                    attempt_id: None,
                    request_id: Some(request_id),
                    retained_revision: self.revision(),
                    rejected_at: self.state.scheduler.now,
                    error: CanwuError::new(
                        ErrorCode::IdempotencyConflict,
                        "this command request ID was already used for different input",
                    ),
                },
            }));
        }
        match &attempt.outcome {
            CommandAttemptOutcome::Accepted { command_id } => {
                let record = self
                    .state
                    .evidence
                    .commands
                    .iter()
                    .find(|record| record.id == *command_id)
                    .ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidSnapshot,
                            "accepted command attempt references a missing command",
                        )
                    })?;
                Ok(Some(CommandOutcome::Accepted {
                    receipt: CommandReceipt {
                        attempt_id: Some(attempt.id),
                        command_id: *command_id,
                        request_id: Some(request_id),
                        revision: attempt.revision_before + 1,
                        accepted_at: record.accepted_at,
                        emitted_events: record.emitted_events.clone(),
                    },
                }))
            }
            CommandAttemptOutcome::Rejected { error } => Ok(Some(CommandOutcome::Rejected {
                rejection: CommandRejection {
                    attempt_id: Some(attempt.id),
                    request_id: Some(request_id),
                    retained_revision: attempt.revision_before.checked_add(1).ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidSnapshot,
                            "cached command attempt revision is exhausted",
                        )
                    })?,
                    rejected_at: attempt.at,
                    error: error.clone(),
                },
            })),
        }
    }

    fn record_command_rejection(
        &mut self,
        attempt_id: CommandAttemptId,
        admission: CommandAdmission,
        envelope: CommandEnvelope,
        error: CanwuError,
    ) -> Result<CommandOutcome, CanwuError> {
        let (claimed_id, next_attempt_id) = claim_counter(
            self.state.counters.next_command_attempt_id,
            "command attempt ID",
        )?;
        if claimed_id != attempt_id.get() {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "command attempt allocation changed during rejection",
            ));
        }
        let revision = self.next_state_revision()?;
        let attempt = CommandAttemptRecord {
            id: attempt_id,
            at: self.state.scheduler.now,
            revision_before: admission.revision_before,
            ingress: admission.ingress,
            request_id: admission.request_id,
            expected_revision: admission.expected_revision,
            envelope,
            outcome: CommandAttemptOutcome::Rejected {
                error: error.clone(),
            },
        };
        let transaction = RejectionTransactionCheckpoint::capture(&self.state);
        self.state.counters.next_command_attempt_id = next_attempt_id;
        self.state.counters.state_revision = revision;
        self.state.metadata.plugin_registration_closed = true;
        self.state.evidence.command_attempts.push(attempt);
        if let Err(hash_error) = self.refresh_checkpoint_hash() {
            transaction.restore(&mut self.state);
            return Err(hash_error);
        }
        Ok(CommandOutcome::Rejected {
            rejection: CommandRejection {
                attempt_id: Some(attempt_id),
                request_id: admission.request_id,
                retained_revision: revision,
                rejected_at: self.state.scheduler.now,
                error,
            },
        })
    }

    fn validate_command_ingress(
        &self,
        issuer: &Issuer,
        authority: &CommandAuthority,
        admission: CommandAdmission,
    ) -> Result<(), CanwuError> {
        validate_command_ingress_policy(
            &self.state.metadata.run_configuration,
            issuer,
            authority,
            admission,
            &|entity| runtime_entity_exists(&self.state, entity),
        )
    }

    pub fn advance_canonical(
        &mut self,
        duration: SimDuration,
    ) -> Result<Vec<BoundaryReceipt>, CanwuError> {
        self.ensure_runtime_ready()?;
        if duration.is_negative() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "canonical simulation time cannot advance by a negative duration",
            ));
        }
        let target = self
            .state
            .scheduler
            .now
            .checked_add(duration)
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidDuration,
                    "canonical simulation target time exceeds the supported range",
                )
            })?;
        let mut receipts = Vec::new();
        while let Some(next_due) = self.next_canonical_due_time()
            && next_due <= target
        {
            let at = next_due.max(self.state.scheduler.now);
            receipts.push(self.settle_boundary(BoundaryRequest::at(at))?);
        }
        if self.state.scheduler.now < target {
            self.advance_to(target)?;
        }
        Ok(receipts)
    }

    pub fn step_canonical(&mut self) -> Result<Option<BoundaryReceipt>, CanwuError> {
        self.ensure_runtime_ready()?;
        let Some(next_due) = self.next_canonical_due_time() else {
            return Ok(None);
        };
        self.settle_boundary(BoundaryRequest::at(next_due.max(self.state.scheduler.now)))
            .map(Some)
    }

    fn next_canonical_due_time(&self) -> Option<SimTime> {
        let scheduled = self.state.scheduler.actions.keys().next().map(|key| key.at);
        let ingress = self
            .state
            .scheduler
            .pending_ingress
            .first()
            .map(|key| key.due_at);
        match (scheduled, ingress) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (Some(value), None) | (None, Some(value)) => Some(value),
            (None, None) => None,
        }
    }

    fn take_due_ingress(&mut self, at: SimTime) -> Vec<IngressId> {
        let mut admitted = Vec::new();
        while self
            .state
            .scheduler
            .pending_ingress
            .first()
            .is_some_and(|key| key.due_at <= at)
        {
            let key = self
                .state
                .scheduler
                .pending_ingress
                .pop_first()
                .expect("pending ingress was checked as non-empty");
            admitted.push(key.id);
        }
        admitted
    }

    pub fn advance(&mut self, duration: SimDuration) -> Result<Vec<SimEvent>, CanwuError> {
        self.ensure_runtime_ready()?;
        if duration.is_negative() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "simulation time cannot advance by a negative duration",
            ));
        }
        let target = self
            .state
            .scheduler
            .now
            .checked_add(duration)
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidDuration,
                    "simulation target time exceeds the supported range",
                )
            })?;
        self.ensure_legacy_advance_does_not_cross_ingress(target)?;
        self.advance_to(target)
    }

    pub fn step(&mut self) -> Result<Vec<SimEvent>, CanwuError> {
        self.ensure_runtime_ready()?;
        if self.state.scheduler.pending_ingress.first().is_some() {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "pending canonical ingress requires step_canonical",
            ));
        }
        let Some(next_time) = self.state.scheduler.actions.keys().next().map(|key| key.at) else {
            return Ok(Vec::new());
        };
        self.advance_to(next_time)
    }

    pub fn advance_until<F>(
        &mut self,
        maximum: SimDuration,
        mut condition: F,
    ) -> Result<Vec<SimEvent>, CanwuError>
    where
        F: FnMut(&Self) -> bool,
    {
        self.ensure_runtime_ready()?;
        if maximum.is_negative() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "advance_until maximum cannot be negative",
            ));
        }
        let target = self
            .state
            .scheduler
            .now
            .checked_add(maximum)
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidDuration,
                    "advance_until target time exceeds the supported range",
                )
            })?;
        self.ensure_legacy_advance_does_not_cross_ingress(target)?;
        let start = self.state.evidence.events.len();
        while self.state.scheduler.now < target && !condition(self) {
            let next_time = self
                .state
                .scheduler
                .actions
                .keys()
                .next()
                .map_or(target, |key| key.at.min(target));
            self.advance_to(next_time)?;
            if next_time == target {
                break;
            }
        }
        Ok(self.state.evidence.events[start..].to_vec())
    }

    fn ensure_legacy_advance_does_not_cross_ingress(
        &self,
        target: SimTime,
    ) -> Result<(), CanwuError> {
        if self
            .state
            .scheduler
            .pending_ingress
            .first()
            .is_some_and(|key| key.due_at <= target)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "legacy time advancement cannot cross pending canonical ingress; use advance_canonical",
            ));
        }
        Ok(())
    }

    pub fn settle_boundary(
        &mut self,
        mut request: BoundaryRequest,
    ) -> Result<BoundaryReceipt, CanwuError> {
        self.ensure_runtime_ready()?;
        if request.at < self.state.scheduler.now {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "a settlement boundary cannot precede committed simulation time",
            ));
        }
        if self
            .state
            .scheduler
            .pending_ingress
            .first()
            .is_some_and(|key| key.due_at < request.at)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "a settlement boundary cannot step past earlier canonical ingress",
            ));
        }
        if request.cadences.contains(&SystemCadence::EventDriven) {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "event-driven cadence is derived from admitted events, not caller supplied",
            ));
        }
        request.cadences.sort();
        request.cadences.dedup();

        let transaction = BoundaryTransactionCheckpoint::capture(&self.state);
        match self.settle_boundary_inner(request) {
            Ok(receipt) => Ok(receipt),
            Err(error) => {
                transaction.restore(&mut self.state);
                Err(error)
            }
        }
    }

    fn settle_boundary_inner(
        &mut self,
        mut request: BoundaryRequest,
    ) -> Result<BoundaryReceipt, CanwuError> {
        self.advance_to_before_boundary(request.at)?;

        let admitted_ingress = self.take_due_ingress(request.at);
        let admitted_ingress_index: HashSet<_> = admitted_ingress.iter().copied().collect();
        for ingress_id in &admitted_ingress {
            let index = usize::try_from(ingress_id.get().saturating_sub(1)).map_err(|_| {
                CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    "ingress ID exceeds the platform index range",
                )
            })?;
            let record = self
                .state
                .evidence
                .ingress
                .get(index)
                .cloned()
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidSnapshot,
                        "pending ingress references an unknown record",
                    )
                })?;
            match record.payload {
                IngressPayload::Command { request: command } => {
                    let CommandRequest {
                        request_id,
                        expected_revision,
                        envelope,
                    } = *command;
                    self.admit_command(
                        Some(request_id),
                        Some(expected_revision),
                        envelope,
                        CommandIngress::LiveRequest,
                        true,
                    )?;
                }
                IngressPayload::Calendar { cadences } => request.cadences.extend(cadences),
                IngressPayload::Plugin { .. } => {}
            }
        }
        self.execute_scheduled_at(request.at)?;
        request.cadences.sort();
        request.cadences.dedup();

        let admitted_attempt_count = u64::try_from(self.state.evidence.command_attempts.len())
            .map_err(|_| {
                invalid_snapshot_error("attempt journal exceeds admission cursor range")
            })?;
        let admitted_command_count =
            u64::try_from(self.state.evidence.commands.len()).map_err(|_| {
                invalid_snapshot_error("command journal exceeds admission cursor range")
            })?;
        let admitted_event_count = u64::try_from(self.state.evidence.events.len())
            .map_err(|_| invalid_snapshot_error("event journal exceeds admission cursor range"))?;
        let admitted_attempt_start = usize::try_from(self.state.counters.admitted_attempt_count)
            .map_err(|_| {
                invalid_snapshot_error("runtime attempt admission cursor exceeds platform range")
            })?;
        let admitted_command_start = usize::try_from(self.state.counters.admitted_command_count)
            .map_err(|_| {
                invalid_snapshot_error("runtime command admission cursor exceeds platform range")
            })?;
        let admitted_event_start = usize::try_from(self.state.counters.admitted_event_count)
            .map_err(|_| {
                invalid_snapshot_error("runtime event admission cursor exceeds platform range")
            })?;
        let admitted_attempts: Vec<_> = self
            .state
            .evidence
            .command_attempts
            .get(admitted_attempt_start..)
            .ok_or_else(|| {
                invalid_snapshot_error("runtime attempt admission cursor exceeds its journal")
            })?
            .iter()
            .map(|record| record.id)
            .collect();
        let admitted_commands: Vec<_> = self
            .state
            .evidence
            .commands
            .get(admitted_command_start..)
            .ok_or_else(|| {
                invalid_snapshot_error("runtime command admission cursor exceeds its journal")
            })?
            .iter()
            .map(|record| record.id)
            .collect();
        let admitted_events: Vec<_> = self
            .state
            .evidence
            .events
            .get(admitted_event_start..)
            .ok_or_else(|| {
                invalid_snapshot_error("runtime event admission cursor exceeds its journal")
            })?
            .iter()
            .map(|event| event.id)
            .collect();

        let (boundary_id_value, next_boundary_id) =
            claim_counter(self.state.counters.next_boundary_id, "boundary ID")?;
        let (correlation_id, next_correlation_id) = claim_counter(
            self.state.counters.next_correlation_id,
            "boundary correlation ID",
        )?;
        self.state.counters.next_boundary_id = next_boundary_id;
        self.state.counters.next_correlation_id = next_correlation_id;
        let boundary_id = BoundaryId::new(boundary_id_value);

        let boundary_snapshot = self.state.clone();
        let systems = self.plugins.boundary_systems.clone();
        let state_owners = self.plugins.state_owners.clone();
        let record_schemas = self.plugins.record_schemas.clone();
        let mut allocations = BTreeMap::new();
        let mut allocation_records = Vec::new();
        let mut reservation_offer_records = Vec::new();
        let mut reservation_request_records = Vec::new();
        let mut offers = Vec::new();
        let mut requests = Vec::new();
        let mut random_overlay = boundary_snapshot.current.random_streams.clone();
        let mut pending_random_draws = Vec::new();
        let mut visible_overlay = BTreeMap::new();
        let mut candidate_overlay = BTreeMap::new();
        let mut visible_record_overlay = BTreeMap::new();
        let mut candidate_record_overlay = BTreeMap::new();
        let mut ordinary = Vec::new();
        let mut transitions = Vec::new();
        let mut deferred = Vec::new();
        let mut evidence = PendingBoundaryEvidence::default();

        for phase in BoundaryPhase::ALL {
            match phase {
                BoundaryPhase::AtomicDomainCommit => {
                    let (same_boundary, next_boundary) =
                        partition_boundary_visibility(std::mem::take(&mut ordinary));
                    self.apply_boundary_stage(
                        boundary_id,
                        correlation_id,
                        same_boundary,
                        &mut evidence,
                    )?;
                    deferred.extend(next_boundary);
                    visible_overlay.clear();
                    candidate_overlay.clear();
                    visible_record_overlay.clear();
                    candidate_record_overlay.clear();
                }
                BoundaryPhase::ConditionalTransitionCommit => {
                    let (same_boundary, next_boundary) =
                        partition_boundary_visibility(std::mem::take(&mut transitions));
                    self.apply_boundary_stage(
                        boundary_id,
                        correlation_id,
                        same_boundary,
                        &mut evidence,
                    )?;
                    deferred.extend(next_boundary);
                    visible_overlay.clear();
                    visible_record_overlay.clear();
                }
                _ => {}
            }

            let mut phase_directives = Vec::new();
            for registered in systems.iter().filter(|registered| {
                registered.contract.phase == phase
                    && boundary_system_due(
                        &registered.contract,
                        &request.cadences,
                        !admitted_events.is_empty() || !admitted_ingress.is_empty(),
                    )
            }) {
                let reader = format!("{}.{}", registered.plugin, registered.contract.name);
                let view_state = if phase <= BoundaryPhase::InvariantValidation {
                    &boundary_snapshot
                } else {
                    &self.state
                };
                let random_session = random::RandomSession::new(
                    &random_overlay,
                    &registered.contract.random_streams,
                )?;
                let view = SimulationView {
                    state: view_state,
                    state_owners: &state_owners,
                    reader: Some(&reader),
                    allowed_reads: Some(&registered.contract.reads),
                    allowed_ingress: Some(&admitted_ingress_index),
                    ingress_plugin: Some(&registered.plugin),
                    component_overlay: Some(&visible_overlay),
                    proposed_components: (phase == BoundaryPhase::InvariantValidation)
                        .then_some(&candidate_overlay),
                    record_overlay: Some(&visible_record_overlay),
                    proposed_records: (phase == BoundaryPhase::InvariantValidation)
                        .then_some(&candidate_record_overlay),
                    allocations: Some(&allocations),
                    allowed_reservations: Some(&registered.contract.reservation_reads),
                    random_session: Some(RefCell::new(random_session)),
                };
                let context = BoundaryContext {
                    boundary_id,
                    at: request.at,
                    phase,
                    plugin: registered.plugin.clone(),
                    system: registered.contract.name.clone(),
                    admitted_attempts: admitted_attempts.clone(),
                    admitted_commands: admitted_commands.clone(),
                    admitted_ingress: admitted_ingress.clone(),
                    admitted_events: admitted_events.clone(),
                    emitted_events: evidence
                        .emissions
                        .iter()
                        .map(|emission| emission.event)
                        .collect(),
                };
                let proposal =
                    catch_unwind(AssertUnwindSafe(|| (registered.handler)(&view, &context)))
                        .map_err(|_| {
                            CanwuError::new(
                                ErrorCode::PluginPanicked,
                                format!(
                                    "boundary system {}.{} panicked",
                                    registered.plugin, registered.contract.name
                                ),
                            )
                        })??;
                validate_boundary_proposal(
                    &registered.plugin,
                    &registered.contract,
                    view_state,
                    &self.plugins,
                    &visible_record_overlay,
                    &proposal,
                )?;
                let random_execution = view
                    .finish_random_session()
                    .expect("boundary views always have a random session");
                random_overlay.extend(random_execution.states);
                pending_random_draws.extend(random_execution.draws.into_iter().map(|draw| {
                    PendingBoundaryRandomDraw {
                        plugin: registered.plugin.clone(),
                        system: registered.contract.name.clone(),
                        draw,
                    }
                }));
                offers.extend(
                    proposal
                        .offers
                        .into_iter()
                        .map(|offer| PendingReservationOffer {
                            plugin: registered.plugin.clone(),
                            system: registered.contract.name.clone(),
                            offer,
                        }),
                );
                requests.extend(proposal.requests.into_iter().map(|request| {
                    PendingReservationRequest {
                        reservation: ReservationRef::new(
                            &registered.plugin,
                            &registered.contract.name,
                            &request.request,
                        ),
                        request,
                    }
                }));
                phase_directives.extend(proposal.directives.into_iter().map(|directive| {
                    StagedBoundaryDirective {
                        plugin: registered.plugin.clone(),
                        system: registered.contract.name.clone(),
                        phase,
                        visibility: registered.contract.visibility,
                        directive,
                    }
                }));
            }

            match phase {
                BoundaryPhase::ReservationAndAllocation => {
                    let result = allocate_reservations(
                        std::mem::take(&mut offers),
                        std::mem::take(&mut requests),
                    )?;
                    allocations = result.by_reservation;
                    allocation_records = result.records;
                    reservation_offer_records = result.offers;
                    reservation_request_records = result.requests;
                }
                BoundaryPhase::DomainDeltaProposal => {
                    extend_boundary_record_candidate_overlay(
                        &boundary_snapshot,
                        &record_schemas,
                        &mut candidate_record_overlay,
                        &phase_directives,
                    )?;
                    extend_boundary_candidate_overlay(
                        &boundary_snapshot,
                        &candidate_record_overlay,
                        &mut candidate_overlay,
                        &phase_directives,
                    )?;
                    extend_boundary_record_overlay(
                        &boundary_snapshot,
                        &record_schemas,
                        &mut visible_record_overlay,
                        &phase_directives,
                    )?;
                    extend_boundary_overlay(
                        &boundary_snapshot,
                        &visible_record_overlay,
                        &mut visible_overlay,
                        &phase_directives,
                    )?;
                    ordinary.extend(phase_directives);
                }
                BoundaryPhase::HistoricalCandidateEvaluation => {
                    extend_boundary_record_overlay(
                        &self.state,
                        &record_schemas,
                        &mut visible_record_overlay,
                        &phase_directives,
                    )?;
                    extend_boundary_overlay(
                        &self.state,
                        &visible_record_overlay,
                        &mut visible_overlay,
                        &phase_directives,
                    )?;
                    transitions.extend(phase_directives);
                }
                BoundaryPhase::StrategicAggregation
                | BoundaryPhase::PerspectiveAndReportMaterialization => {
                    let (same_boundary, next_boundary) =
                        partition_boundary_visibility(phase_directives);
                    self.apply_boundary_stage(
                        boundary_id,
                        correlation_id,
                        same_boundary,
                        &mut evidence,
                    )?;
                    deferred.extend(next_boundary);
                }
                _ if !phase_directives.is_empty() => {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidBoundary,
                        format!("boundary phase {phase:?} cannot produce state directives"),
                    ));
                }
                _ => {}
            }
        }

        self.apply_boundary_stage(boundary_id, correlation_id, deferred, &mut evidence)?;
        let PendingBoundaryEvidence {
            changes,
            record_changes,
            emissions,
            generated_ingress,
        } = evidence;
        self.invalidate_commitments(CommitmentDomains::RANDOM_STREAMS);
        self.state.current.random_streams = random_overlay;
        let random_draws =
            self.append_boundary_random_draws(boundary_id, correlation_id, pending_random_draws)?;
        self.state.metadata.plugin_registration_closed = true;
        let state_hash = self.compute_boundary_state_hash()?;
        let previous_hash = self.state.evidence.boundaries.last().map_or_else(
            || GENESIS_BOUNDARY_HASH.to_owned(),
            |record| record.hash.clone(),
        );
        let mut record = BoundaryRecord {
            id: boundary_id,
            at: request.at,
            correlation_id,
            cadences: request.cadences,
            admitted_attempts,
            admitted_commands,
            admitted_ingress,
            generated_ingress: generated_ingress.clone(),
            admitted_events,
            reservation_offers: reservation_offer_records,
            reservation_requests: reservation_request_records,
            allocations: allocation_records.clone(),
            random_draws: random_draws.clone(),
            changes: changes.clone(),
            record_changes: record_changes.clone(),
            emissions: emissions.clone(),
            state_hash: Some(state_hash),
            previous_hash,
            hash: String::new(),
        };
        record.hash = compute_boundary_hash(&record)?;
        let boundary_hash = record.hash.clone();
        self.state.evidence.boundaries.push(record);
        self.state.counters.admitted_attempt_count = admitted_attempt_count;
        self.state.counters.admitted_command_count = admitted_command_count;
        self.state.counters.admitted_event_count = admitted_event_count;
        self.advance_state_revision()?;
        self.refresh_checkpoint_hash()?;
        Ok(BoundaryReceipt {
            boundary_id,
            settled_at: request.at,
            emitted_events: emissions
                .into_iter()
                .map(|emission| emission.event)
                .collect(),
            generated_ingress: generated_ingress
                .into_iter()
                .map(|generation| generation.ingress)
                .collect(),
            random_draws,
            boundary_hash,
            change_count: changes.len(),
            record_change_count: record_changes.len(),
            allocations: allocation_records,
        })
    }

    fn compute_boundary_state_hash(&self) -> Result<String, CanwuError> {
        let world = self.world();
        let plugin_components: Vec<_> = self
            .state
            .current
            .plugin_components
            .values()
            .cloned()
            .collect();
        let domain_records: Vec<_> = self
            .state
            .current
            .domain_records
            .values()
            .cloned()
            .collect();
        let plugin_descriptors: Vec<_> = self.plugins.descriptors().cloned().collect();
        let scheduled: Vec<_> = self
            .state
            .scheduler
            .actions
            .iter()
            .map(|(key, action)| ScheduledRecord {
                key: key.clone(),
                action: action.clone(),
            })
            .collect();
        let random_streams: Vec<_> = self
            .state
            .current
            .random_streams
            .values()
            .cloned()
            .collect();
        let (authoritative_manifest, authoritative_manifest_hash) = authoritative_run_identity(
            &self.state.metadata.run_manifest,
            &self.state.metadata.run_manifest_hash,
            &self.state.metadata.run_configuration,
        )?;
        let initial_scenario = self.bound_initial_scenario();
        state_hash(&StateHashMaterial {
            engine_version: ENGINE_VERSION,
            snapshot_format_version: SNAPSHOT_FORMAT_VERSION,
            run_manifest: &authoritative_manifest,
            run_manifest_hash: &authoritative_manifest_hash,
            initial_time: self.state.scheduler.initial_time,
            initial_scenario,
            now: self.state.scheduler.now,
            plugin_registration_closed: self.state.metadata.plugin_registration_closed,
            world: &world,
            knowledge: &self.state.current.knowledge,
            events: &self.state.evidence.events,
            commands: &self.state.evidence.commands,
            command_attempts: &self.state.evidence.command_attempts,
            ingress: &self.state.evidence.ingress,
            plugin_components: &plugin_components,
            domain_records: &domain_records,
            plugin_descriptors: &plugin_descriptors,
            schema: &self.schema,
            scheduled: &scheduled,
            root_seed: self.state.current.root_seed,
            random_streams: &random_streams,
            random_draws: &self.state.evidence.random_draws,
            next_event_id: self.state.counters.next_event_id,
            next_command_id: self.state.counters.next_command_id,
            next_command_attempt_id: self.state.counters.next_command_attempt_id,
            next_ingress_id: self.state.counters.next_ingress_id,
            next_boundary_id: self.state.counters.next_boundary_id,
            next_random_draw_id: self.state.counters.next_random_draw_id,
            next_schedule_sequence: self.state.counters.next_schedule_sequence,
            next_correlation_id: self.state.counters.next_correlation_id,
        })
    }

    fn compute_commitment_root_updates(
        &self,
        needs: CommitmentDomains,
    ) -> Result<RuntimeCommitmentRootUpdates, CanwuError> {
        let world = needs
            .contains(CommitmentDomains::WORLD)
            .then(|| world_commitment_root(&self.world()))
            .transpose()?;
        let knowledge = needs
            .contains(CommitmentDomains::KNOWLEDGE)
            .then(|| knowledge_commitment_root(&self.state.current.knowledge))
            .transpose()?;
        let plugin_components = needs
            .contains(CommitmentDomains::PLUGIN_COMPONENTS)
            .then(|| {
                let values: Vec<_> = self
                    .state
                    .current
                    .plugin_components
                    .values()
                    .cloned()
                    .collect();
                plugin_component_commitment_root(&values)
            })
            .transpose()?;
        let domain_records = needs
            .contains(CommitmentDomains::DOMAIN_RECORDS)
            .then(|| {
                let values: Vec<_> = self
                    .state
                    .current
                    .domain_records
                    .values()
                    .cloned()
                    .collect();
                domain_record_commitment_root(&values)
            })
            .transpose()?;
        let scheduler = needs
            .contains(CommitmentDomains::SCHEDULER)
            .then(|| {
                let scheduled: Vec<_> = self
                    .state
                    .scheduler
                    .actions
                    .iter()
                    .map(|(key, action)| ScheduledRecord {
                        key: key.clone(),
                        action: action.clone(),
                    })
                    .collect();
                scheduler_commitment_root(self.state.scheduler.now, &scheduled)
            })
            .transpose()?;
        let random_streams = needs
            .contains(CommitmentDomains::RANDOM_STREAMS)
            .then(|| {
                let values: Vec<_> = self
                    .state
                    .current
                    .random_streams
                    .values()
                    .cloned()
                    .collect();
                random_stream_commitment_root(&values)
            })
            .transpose()?;
        let identity = if needs.contains(CommitmentDomains::IDENTITY) {
            let descriptors: Vec<_> = self.plugins.descriptors().cloned().collect();
            let (manifest, manifest_hash) = authoritative_run_identity(
                &self.state.metadata.run_manifest,
                &self.state.metadata.run_manifest_hash,
                &self.state.metadata.run_configuration,
            )?;
            Some(identity_commitment_root(
                ENGINE_VERSION,
                SNAPSHOT_FORMAT_VERSION,
                &manifest,
                &manifest_hash,
                self.state.scheduler.initial_time,
                self.bound_initial_scenario(),
                &descriptors,
                &self.schema,
            )?)
        } else {
            None
        };
        Ok(RuntimeCommitmentRootUpdates {
            world,
            knowledge,
            plugin_components,
            domain_records,
            scheduler,
            random_streams,
            identity,
        })
    }

    fn invalidate_commitments(&mut self, domains: CommitmentDomains) {
        if let Some(cache) = self.state.metadata.commitment_cache.as_mut() {
            cache.invalidate(domains);
        }
    }

    fn refresh_checkpoint_hash(&mut self) -> Result<(), CanwuError> {
        if self.state.metadata.commitment_format_version == COMMITMENT_FORMAT_VERSION {
            let needs = {
                let cache = if let Some(cache) = self.state.metadata.commitment_cache.as_mut() {
                    cache
                } else {
                    self.state.metadata.commitment_cache =
                        Some(RuntimeCommitmentCache::from_evidence(&self.state.evidence)?);
                    self.state
                        .metadata
                        .commitment_cache
                        .as_mut()
                        .expect("the commitment cache was initialized")
                };
                cache.sync(&self.state.evidence)?;
                cache.needs()
            };
            let updates = self.compute_commitment_root_updates(needs)?;
            let boundary_head = self.boundary_head_hash().map(str::to_owned);
            let control = ControlCommitmentMaterial {
                plugin_registration_closed: self.state.metadata.plugin_registration_closed,
                next_event_id: self.state.counters.next_event_id,
                next_command_id: self.state.counters.next_command_id,
                next_command_attempt_id: self.state.counters.next_command_attempt_id,
                next_ingress_id: self.state.counters.next_ingress_id,
                next_boundary_id: self.state.counters.next_boundary_id,
                next_random_draw_id: self.state.counters.next_random_draw_id,
                next_schedule_sequence: self.state.counters.next_schedule_sequence,
                next_correlation_id: self.state.counters.next_correlation_id,
            };
            let (domain_roots, journal_roots) = {
                let cache = self
                    .state
                    .metadata
                    .commitment_cache
                    .as_mut()
                    .expect("the commitment cache was initialized");
                cache.apply(updates);
                (cache.domain_roots()?, cache.roots())
            };
            let roots = runtime_commitment_roots(
                &domain_roots,
                &journal_roots,
                self.state.current.root_seed,
                boundary_head.as_deref(),
                &control,
            )?;
            self.state.metadata.checkpoint_hash = checkpoint_hash_for_commitments(
                &roots,
                &self.state.metadata.run_manifest_hash,
                self.state.metadata.commitment_format_version,
                STATE_REVISION_FORMAT_VERSION,
                self.state.counters.state_revision,
                self.state.metadata.replay_revision_format_version,
            )?;
            self.state.metadata.commitment_roots = Some(roots);
        } else if self.state.metadata.commitment_format_version == 0 {
            let state_hash = self.compute_boundary_state_hash()?;
            self.state.metadata.checkpoint_hash = checkpoint_hash_for_configuration(
                &state_hash,
                self.boundary_head_hash(),
                &self.state.metadata.run_manifest_hash,
                &self.state.metadata.run_configuration,
                STATE_REVISION_FORMAT_VERSION,
                self.state.counters.state_revision,
                self.state.metadata.replay_revision_format_version,
            )?;
            self.state.metadata.commitment_roots = None;
            self.state.metadata.commitment_cache = None;
        } else {
            return Err(CanwuError::new(
                ErrorCode::UnsupportedSnapshotVersion,
                format!(
                    "commitment format {} is unsupported; this engine writes format {COMMITMENT_FORMAT_VERSION}",
                    self.state.metadata.commitment_format_version
                ),
            ));
        }
        Ok(())
    }

    fn next_state_revision(&self) -> Result<u64, CanwuError> {
        self.state
            .counters
            .state_revision
            .checked_add(1)
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::IdentifierExhausted,
                    "authoritative state revision space is exhausted",
                )
            })
    }

    fn advance_state_revision(&mut self) -> Result<u64, CanwuError> {
        let next = self.next_state_revision()?;
        self.state.counters.state_revision = next;
        Ok(next)
    }

    #[must_use]
    pub fn snapshot(&self) -> SimulationSnapshot {
        SimulationSnapshot {
            engine_version: ENGINE_VERSION.to_owned(),
            snapshot_format_version: SNAPSHOT_FORMAT_VERSION,
            run_manifest: Some(self.state.metadata.run_manifest.clone()),
            run_manifest_hash: self.state.metadata.run_manifest_hash.clone(),
            run_configuration: Some(self.state.metadata.run_configuration.clone()),
            checkpoint_hash: self.state.metadata.checkpoint_hash.clone(),
            commitment_format_version: self.state.metadata.commitment_format_version,
            commitment_roots: self.state.metadata.commitment_roots.clone(),
            revision_format_version: STATE_REVISION_FORMAT_VERSION,
            state_revision: self.state.counters.state_revision,
            replay_revision_format_version: self.state.metadata.replay_revision_format_version,
            admission_cursor_format_version: ADMISSION_CURSOR_FORMAT_VERSION,
            admitted_attempt_count: self.state.counters.admitted_attempt_count,
            admitted_command_count: self.state.counters.admitted_command_count,
            admitted_event_count: self.state.counters.admitted_event_count,
            initial_time: self.state.scheduler.initial_time,
            initial_scenario: self.bound_initial_scenario().cloned(),
            now: self.state.scheduler.now,
            plugin_registration_closed: self.state.metadata.plugin_registration_closed,
            world: self.world(),
            knowledge: self.state.current.knowledge.clone(),
            events: self.state.evidence.events.clone(),
            commands: self.state.evidence.commands.clone(),
            command_attempts: self.state.evidence.command_attempts.clone(),
            ingress: self.state.evidence.ingress.clone(),
            boundaries: self.state.evidence.boundaries.clone(),
            plugin_components: self
                .state
                .current
                .plugin_components
                .values()
                .cloned()
                .collect(),
            domain_records: self
                .state
                .current
                .domain_records
                .values()
                .cloned()
                .collect(),
            plugin_descriptors: self.plugins.descriptors().cloned().collect(),
            schema: self.schema.clone(),
            root_seed: self.state.current.root_seed,
            random_streams: self
                .state
                .current
                .random_streams
                .values()
                .cloned()
                .collect(),
            random_draws: self.state.evidence.random_draws.clone(),
            scheduled: self
                .state
                .scheduler
                .actions
                .iter()
                .map(|(key, action)| ScheduledRecord {
                    key: key.clone(),
                    action: action.clone(),
                })
                .collect(),
            legacy_rng: None,
            next_event_id: self.state.counters.next_event_id,
            next_command_id: self.state.counters.next_command_id,
            next_command_attempt_id: self.state.counters.next_command_attempt_id,
            next_ingress_id: self.state.counters.next_ingress_id,
            next_boundary_id: self.state.counters.next_boundary_id,
            next_random_draw_id: self.state.counters.next_random_draw_id,
            next_schedule_sequence: self.state.counters.next_schedule_sequence,
            next_correlation_id: self.state.counters.next_correlation_id,
        }
    }

    pub fn snapshot_json(&self) -> Result<String, CanwuError> {
        serde_json::to_string_pretty(&self.snapshot()).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("could not serialize snapshot: {error}"),
            )
        })
    }

    pub fn from_snapshot(snapshot: SimulationSnapshot) -> Result<Self, CanwuError> {
        let snapshot = migrate_snapshot(snapshot)?;
        validate_scenario(&Scenario {
            start_time: snapshot.now,
            world: snapshot.world.clone(),
            knowledge: snapshot.knowledge.clone(),
            domain_records: snapshot.domain_records.clone(),
        })?;
        let plugins = PluginRegistry::from_descriptors(snapshot.plugin_descriptors.clone())?;
        validate_snapshot(&snapshot, &plugins)?;
        let admitted_ingress: BTreeSet<_> = snapshot
            .boundaries
            .iter()
            .flat_map(|boundary| boundary.admitted_ingress.iter().copied())
            .collect();
        let pending_ingress = snapshot
            .ingress
            .iter()
            .filter(|record| !admitted_ingress.contains(&record.id))
            .map(IngressQueueKey::from_record)
            .collect();
        let initial_scenario = match snapshot.initial_scenario.clone() {
            Some(initial_scenario) => Some(initial_scenario),
            None if !snapshot.plugin_registration_closed => match snapshot.run_manifest.as_ref() {
                Some(run_manifest @ RunManifest::Declared { .. }) => {
                    let initial_scenario = Scenario {
                        start_time: snapshot.initial_time,
                        world: snapshot.world.clone(),
                        knowledge: snapshot.knowledge.clone(),
                        domain_records: snapshot.domain_records.clone(),
                    };
                    manifest::validate(run_manifest, Some(&initial_scenario), true)?;
                    Some(initial_scenario)
                }
                _ => None,
            },
            None => None,
        };
        let mut simulation = Self {
            state: RuntimeState {
                current: RuntimeCurrentState {
                    people: snapshot
                        .world
                        .people
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    governments: snapshot
                        .world
                        .governments
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    territories: snapshot
                        .world
                        .territories
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    routes: snapshot
                        .world
                        .routes
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    armies: snapshot
                        .world
                        .armies
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    knowledge: snapshot.knowledge,
                    plugin_components: snapshot
                        .plugin_components
                        .into_iter()
                        .map(|record| {
                            (
                                component_key(
                                    &record.plugin,
                                    &record.state,
                                    &record.entity,
                                    &record.component,
                                ),
                                record,
                            )
                        })
                        .collect(),
                    domain_records: snapshot
                        .domain_records
                        .into_iter()
                        .map(|record| (record.reference.clone(), record))
                        .collect(),
                    root_seed: snapshot.root_seed,
                    random_streams: snapshot
                        .random_streams
                        .into_iter()
                        .map(|state| (state.key.clone(), state))
                        .collect(),
                },
                scheduler: RuntimeScheduler {
                    initial_time: snapshot.initial_time,
                    now: snapshot.now,
                    actions: snapshot
                        .scheduled
                        .into_iter()
                        .map(|record| (record.key, record.action))
                        .collect(),
                    pending_ingress,
                },
                counters: RuntimeCounters {
                    next_event_id: snapshot.next_event_id,
                    next_command_id: snapshot.next_command_id,
                    next_command_attempt_id: snapshot.next_command_attempt_id,
                    next_ingress_id: snapshot.next_ingress_id,
                    next_boundary_id: snapshot.next_boundary_id,
                    next_random_draw_id: snapshot.next_random_draw_id,
                    next_schedule_sequence: snapshot.next_schedule_sequence,
                    next_correlation_id: snapshot.next_correlation_id,
                    state_revision: snapshot.state_revision,
                    admitted_attempt_count: snapshot.admitted_attempt_count,
                    admitted_command_count: snapshot.admitted_command_count,
                    admitted_event_count: snapshot.admitted_event_count,
                },
                metadata: RuntimeMetadata {
                    initial_scenario,
                    run_manifest: snapshot.run_manifest.clone().ok_or_else(|| {
                        invalid_snapshot_error("snapshot is missing its run manifest")
                    })?,
                    run_manifest_hash: snapshot.run_manifest_hash.clone(),
                    run_configuration: snapshot.run_configuration.clone().ok_or_else(|| {
                        invalid_snapshot_error("snapshot is missing its run configuration")
                    })?,
                    checkpoint_hash: snapshot.checkpoint_hash.clone(),
                    commitment_format_version: snapshot.commitment_format_version,
                    commitment_roots: snapshot.commitment_roots.clone(),
                    commitment_cache: None,
                    plugin_registration_closed: snapshot.plugin_registration_closed,
                    replay_revision_format_version: snapshot.replay_revision_format_version,
                },
                evidence: RuntimeEvidence {
                    events: snapshot.events,
                    commands: snapshot.commands,
                    command_attempts: snapshot.command_attempts,
                    ingress: snapshot.ingress,
                    boundaries: snapshot.boundaries,
                    random_draws: snapshot.random_draws,
                },
            },
            schema: snapshot.schema,
            plugins,
        };
        simulation.refresh_checkpoint_hash()?;
        Ok(simulation)
    }

    pub fn from_snapshot_json(json: &str) -> Result<Self, CanwuError> {
        let snapshot = serde_json::from_str(json).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("could not deserialize snapshot: {error}"),
            )
        })?;
        Self::from_snapshot(snapshot)
    }

    pub fn from_snapshot_with_plugins(
        snapshot: SimulationSnapshot,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        let mut simulation = Self::from_snapshot(snapshot)?;
        for plugin in plugins {
            simulation.register_plugin(*plugin)?;
        }
        simulation.ensure_runtime_ready()?;
        Ok(simulation)
    }

    pub fn from_snapshot_json_with_plugins(
        json: &str,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        let snapshot = serde_json::from_str(json).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("could not deserialize snapshot: {error}"),
            )
        })?;
        Self::from_snapshot_with_plugins(snapshot, plugins)
    }

    #[must_use]
    pub fn fork(&self) -> Self {
        Self {
            state: self.state.clone(),
            schema: self.schema.clone(),
            plugins: self.plugins.clone(),
        }
    }

    fn prepare_command(
        &self,
        envelope: &CommandEnvelope,
        context: &CommandContext,
    ) -> Result<PreparedCommand, CanwuError> {
        match &envelope.command {
            Command::MoveArmy { army, destination } => {
                let Some(actor) = decision_actor(&context.authority) else {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "move commands require an accountable actor origin",
                    ));
                };
                let person = self.state.current.people.get(&actor).ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::ActorNotFound,
                        format!("actor {actor} was not found"),
                    )
                    .with_entity(EntityRef::Person(actor))
                })?;
                let army_state = self.state.current.armies.get(army).ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::ArmyNotFound,
                        format!("army {army} was not found"),
                    )
                    .with_entity(EntityRef::Army(*army))
                })?;
                if army_state.commander != person.id {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        format!("{} does not command {}", person.name, army_state.name),
                    )
                    .with_entity(EntityRef::Person(person.id))
                    .with_entity(EntityRef::Army(*army)));
                }
                if army_state.transit.is_some() {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        format!("{} is already moving", army_state.name),
                    )
                    .with_entity(EntityRef::Army(*army)));
                }
                if !self.state.current.territories.contains_key(destination) {
                    return Err(CanwuError::new(
                        ErrorCode::DestinationNotFound,
                        format!("destination {destination} was not found"),
                    )
                    .with_entity(EntityRef::Territory(*destination)));
                }
                let route = self
                    .state
                    .current
                    .routes
                    .values()
                    .find(|route| route.connects(army_state.location, *destination))
                    .ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::NoRoute,
                            format!(
                                "no direct route connects territory {} to {destination}",
                                army_state.location
                            ),
                        )
                    })?;
                let arrival_at = self
                    .state
                    .scheduler
                    .now
                    .checked_add(SimDuration::minutes(route.travel_minutes))
                    .ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidDuration,
                            "army arrival time exceeds the supported range",
                        )
                    })?;
                Ok(PreparedCommand::MoveArmy {
                    army: *army,
                    actor,
                    from: army_state.location,
                    destination: *destination,
                    arrival_at,
                })
            }
            Command::DebugSetArmyMorale { army, morale } => {
                if envelope.issuer != Issuer::Debug {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "debug state edits require the explicit debug issuer",
                    ));
                }
                if *morale > 100 {
                    return Err(CanwuError::new(
                        ErrorCode::ValueOutOfRange,
                        "army morale must be between 0 and 100",
                    ));
                }
                let old_morale = self.state.current.armies.get(army).map_or_else(
                    || {
                        Err(CanwuError::new(
                            ErrorCode::ArmyNotFound,
                            format!("army {army} was not found"),
                        ))
                    },
                    |army_state| Ok(army_state.morale),
                )?;
                Ok(PreparedCommand::DebugMorale {
                    army: *army,
                    old_morale,
                    new_morale: *morale,
                })
            }
            Command::Plugin {
                plugin,
                command,
                payload,
            } => {
                let registered = self
                    .plugins
                    .commands
                    .get(&(plugin.clone(), command.clone()))
                    .ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::PluginCommandNotFound,
                            format!("plugin command {plugin}.{command} is not registered"),
                        )
                    })?;
                let handler = registered.handler;
                let descriptor = registered.descriptor.clone();
                descriptor.payload_schema.validate(payload)?;
                let reader = format!("{plugin}.{command}");
                let directives = catch_unwind(AssertUnwindSafe(|| {
                    handler(
                        &self.plugin_view(&reader, &descriptor.reads),
                        context,
                        payload,
                    )
                }))
                .map_err(|_| {
                    CanwuError::new(
                        ErrorCode::PluginPanicked,
                        format!("plugin command {plugin}.{command} panicked"),
                    )
                })??;
                validate_directives(
                    plugin,
                    &descriptor.writes,
                    &self.plugins.state_owners,
                    &self.plugins.record_schemas,
                    &|entity| runtime_entity_exists(&self.state, entity),
                    &directives,
                )?;
                Ok(PreparedCommand::Plugin {
                    plugin: plugin.clone(),
                    directives,
                    allowed_writes: descriptor.writes,
                })
            }
        }
    }

    fn apply_prepared(
        &mut self,
        prepared: PreparedCommand,
        command_id: CommandId,
        correlation_id: u64,
    ) -> Result<(), CanwuError> {
        match prepared {
            PreparedCommand::MoveArmy {
                army,
                actor,
                from,
                destination,
                arrival_at,
            } => {
                let army_state = self.state.current.armies.get_mut(&army).ok_or_else(|| {
                    CanwuError::new(ErrorCode::ArmyNotFound, "validated army disappeared")
                })?;
                army_state.transit = Some(TransitState {
                    from,
                    to: destination,
                    departed_at: self.state.scheduler.now,
                    arrives_at: arrival_at,
                });
                let event = self.emit(
                    EventKind::MoveOrdered {
                        army,
                        from,
                        to: destination,
                        arrival_at,
                    },
                    vec![
                        EntityRef::Army(army),
                        EntityRef::Person(actor),
                        EntityRef::Territory(from),
                        EntityRef::Territory(destination),
                    ],
                    format!("Army {army} was ordered from {from} to {destination}"),
                    Some(CauseRef::Command(command_id)),
                    correlation_id,
                )?;
                self.schedule_at(
                    arrival_at,
                    ScheduledAction::ArmyArrival {
                        army,
                        destination,
                        order_event: event,
                        correlation_id,
                    },
                )?;
            }
            PreparedCommand::DebugMorale {
                army,
                old_morale,
                new_morale,
            } => {
                self.state
                    .current
                    .armies
                    .get_mut(&army)
                    .ok_or_else(|| {
                        CanwuError::new(ErrorCode::ArmyNotFound, "validated army disappeared")
                    })?
                    .morale = new_morale;
                self.emit(
                    EventKind::DebugFieldChanged {
                        entity: EntityRef::Army(army),
                        field: "morale".to_owned(),
                        old_value: old_morale.to_string(),
                        new_value: new_morale.to_string(),
                    },
                    vec![EntityRef::Army(army)],
                    format!(
                        "Debug command changed army {army} morale {old_morale} -> {new_morale}"
                    ),
                    Some(CauseRef::Command(command_id)),
                    correlation_id,
                )?;
            }
            PreparedCommand::Plugin {
                plugin,
                directives,
                allowed_writes,
            } => {
                self.apply_directives(
                    &plugin,
                    directives,
                    &allowed_writes,
                    &CauseRef::Command(command_id),
                    correlation_id,
                )?;
            }
        }
        Ok(())
    }

    fn advance_to(&mut self, target: SimTime) -> Result<Vec<SimEvent>, CanwuError> {
        let start = self.state.evidence.events.len();
        while let Some(boundary_time) = self.state.scheduler.actions.keys().next().map(|key| key.at)
            && boundary_time <= target
        {
            let transaction = ScheduledBatchTransactionCheckpoint::capture(&self.state);
            let result = (|| {
                self.invalidate_commitments(CommitmentDomains::SCHEDULER);
                self.state.scheduler.now = boundary_time;
                while self
                    .state
                    .scheduler
                    .actions
                    .first_key_value()
                    .is_some_and(|(key, _)| key.at == boundary_time)
                {
                    let (_, action) = self
                        .state
                        .scheduler
                        .actions
                        .pop_first()
                        .expect("scheduler was checked as non-empty");
                    self.execute_scheduled(action)?;
                }
                self.state.metadata.plugin_registration_closed = true;
                self.refresh_checkpoint_hash()
            })();
            if let Err(error) = result {
                transaction.restore(&mut self.state);
                return Err(error);
            }
        }
        let transaction = ClockTransactionCheckpoint::capture(&self.state);
        self.invalidate_commitments(CommitmentDomains::SCHEDULER);
        self.state.scheduler.now = target;
        self.state.metadata.plugin_registration_closed = true;
        if let Err(error) = self.refresh_checkpoint_hash() {
            transaction.restore(&mut self.state);
            return Err(error);
        }
        Ok(self.state.evidence.events[start..].to_vec())
    }

    fn advance_to_before_boundary(&mut self, target: SimTime) -> Result<(), CanwuError> {
        while let Some(next) = self.state.scheduler.actions.keys().next().map(|key| key.at)
            && next < target
        {
            self.advance_to(next)?;
        }
        self.invalidate_commitments(CommitmentDomains::SCHEDULER);
        self.state.scheduler.now = target;
        self.state.metadata.plugin_registration_closed = true;
        self.refresh_checkpoint_hash()
    }

    fn execute_scheduled_at(&mut self, at: SimTime) -> Result<(), CanwuError> {
        if self
            .state
            .scheduler
            .actions
            .first_key_value()
            .is_some_and(|(key, _)| key.at == at)
        {
            self.invalidate_commitments(CommitmentDomains::SCHEDULER);
        }
        while self
            .state
            .scheduler
            .actions
            .first_key_value()
            .is_some_and(|(key, _)| key.at == at)
        {
            let (_, action) = self
                .state
                .scheduler
                .actions
                .pop_first()
                .expect("scheduler was checked as non-empty");
            self.execute_scheduled(action)?;
        }
        Ok(())
    }

    fn execute_scheduled(&mut self, action: ScheduledAction) -> Result<(), CanwuError> {
        match action {
            ScheduledAction::ArmyArrival {
                army,
                destination,
                order_event,
                correlation_id,
            } => self.execute_arrival(army, destination, order_event, correlation_id),
            ScheduledAction::KnowledgeReport {
                recipient,
                army,
                location,
                observed_at,
                dispatch_event,
                correlation_id,
            } => {
                self.update_army_knowledge(
                    recipient,
                    army,
                    location,
                    observed_at,
                    KnowledgeSource::Report {
                        source_event: dispatch_event,
                    },
                    850,
                );
                self.emit(
                    EventKind::KnowledgeUpdated {
                        recipient,
                        army,
                        known_location: location,
                    },
                    vec![EntityRef::Person(recipient), EntityRef::Army(army)],
                    format!(
                        "Person {recipient} received a report locating army {army} at {location}"
                    ),
                    Some(CauseRef::Event(dispatch_event)),
                    correlation_id,
                )?;
                Ok(())
            }
            ScheduledAction::PluginDirective {
                plugin,
                directive,
                allowed_writes,
                cause,
                correlation_id,
            } => {
                let directives = vec![*directive];
                validate_directives(
                    &plugin,
                    &allowed_writes,
                    &self.plugins.state_owners,
                    &self.plugins.record_schemas,
                    &|entity| runtime_entity_exists(&self.state, entity),
                    &directives,
                )?;
                self.apply_directives(&plugin, directives, &allowed_writes, &cause, correlation_id)
            }
        }
    }

    fn execute_arrival(
        &mut self,
        army: ArmyId,
        destination: TerritoryId,
        order_event: EventId,
        correlation_id: u64,
    ) -> Result<(), CanwuError> {
        self.invalidate_commitments(CommitmentDomains::WORLD);
        let commander = {
            let army_state = self.state.current.armies.get_mut(&army).ok_or_else(|| {
                CanwuError::new(ErrorCode::ArmyNotFound, "scheduled army no longer exists")
            })?;
            army_state.location = destination;
            army_state.transit = None;
            army_state.commander
        };
        let arrival_event = self.emit(
            EventKind::ArmyArrived {
                army,
                territory: destination,
            },
            vec![EntityRef::Army(army), EntityRef::Territory(destination)],
            format!("Army {army} arrived in territory {destination}"),
            Some(CauseRef::Event(order_event)),
            correlation_id,
        )?;

        self.update_army_knowledge(
            commander,
            army,
            destination,
            self.state.scheduler.now,
            KnowledgeSource::CommandResponsibility,
            1000,
        );
        self.emit(
            EventKind::KnowledgeUpdated {
                recipient: commander,
                army,
                known_location: destination,
            },
            vec![EntityRef::Person(commander), EntityRef::Army(army)],
            format!("Commander {commander} learned that army {army} arrived at {destination}"),
            Some(CauseRef::Event(arrival_event)),
            correlation_id,
        )?;

        let recipients: Vec<_> = self
            .state
            .current
            .people
            .keys()
            .copied()
            .filter(|person| *person != commander)
            .collect();
        for recipient in recipients {
            let (draw_id, jitter) = self.draw_random(
                &random::core_report_delay_stream(),
                12 * 60,
                "knowledge report delivery jitter",
                RandomDrawProducer::CoreSystem {
                    system: "canwu.core.knowledge-report-delay".to_owned(),
                },
                CauseRef::Event(arrival_event),
                correlation_id,
            )?;
            let jitter_minutes =
                i64::try_from(jitter).expect("report jitter is bounded to a small integer");
            let arrives_at = self
                .state
                .scheduler
                .now
                .checked_add(SimDuration::hours(36))
                .and_then(|time| time.checked_add(SimDuration::minutes(jitter_minutes)))
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidDuration,
                        "knowledge report arrival time exceeds the supported range",
                    )
                })?;
            let dispatch_event = self.emit(
                EventKind::ReportDispatched {
                    recipient,
                    army,
                    arrives_at,
                },
                vec![EntityRef::Person(recipient), EntityRef::Army(army)],
                format!("A report about army {army} was dispatched to person {recipient}"),
                Some(CauseRef::Event(arrival_event)),
                correlation_id,
            )?;
            self.record_random_outcome(
                draw_id,
                RandomDrawOutcome::KnowledgeReportDelivery {
                    recipient,
                    army,
                    dispatch_event,
                    arrives_at,
                },
            )?;
            self.schedule_at(
                arrives_at,
                ScheduledAction::KnowledgeReport {
                    recipient,
                    army,
                    location: destination,
                    observed_at: self.state.scheduler.now,
                    dispatch_event,
                    correlation_id,
                },
            )?;
        }
        Ok(())
    }

    fn update_army_knowledge(
        &mut self,
        recipient: PersonId,
        army: ArmyId,
        location: TerritoryId,
        observed_at: SimTime,
        source: KnowledgeSource,
        confidence_per_mille: u16,
    ) {
        self.invalidate_commitments(CommitmentDomains::KNOWLEDGE);
        let (strength, known_name) = self.state.current.armies.get(&army).map_or_else(
            || (0, None),
            |value| (value.strength, Some(value.name.clone())),
        );
        let actor = self
            .state
            .current
            .knowledge
            .actors
            .entry(recipient)
            .or_insert_with(|| ActorKnowledge {
                actor: recipient,
                armies: BTreeMap::new(),
            });
        actor.armies.insert(
            army,
            ArmyKnowledge {
                army,
                known_name,
                known_location: Some(location),
                estimated_strength: EstimateRange {
                    minimum: strength.saturating_mul(9) / 10,
                    maximum: strength.saturating_mul(11) / 10,
                },
                observed_at,
                learned_at: self.state.scheduler.now,
                confidence_per_mille,
                source,
            },
        );
    }

    fn emit(
        &mut self,
        kind: EventKind,
        affected_entities: Vec<EntityRef>,
        summary: String,
        cause: Option<CauseRef>,
        correlation_id: u64,
    ) -> Result<EventId, CanwuError> {
        let event = self.append_event(kind, affected_entities, summary, cause, correlation_id)?;
        let id = event.id;

        let systems = self.plugins.systems.clone();
        for registered in systems {
            let reader = format!("{}.{}", registered.plugin, registered.contract.name);
            let directives = catch_unwind(AssertUnwindSafe(|| {
                (registered.handler)(
                    &self.plugin_view(&reader, &registered.contract.reads),
                    &event,
                )
            }))
            .map_err(|_| {
                CanwuError::new(
                    ErrorCode::PluginPanicked,
                    format!(
                        "plugin system {}.{} panicked",
                        registered.plugin, registered.contract.name
                    ),
                )
            })??;
            validate_directives(
                &registered.plugin,
                &registered.contract.writes,
                &self.plugins.state_owners,
                &self.plugins.record_schemas,
                &|entity| runtime_entity_exists(&self.state, entity),
                &directives,
            )?;
            self.apply_directives(
                &registered.plugin,
                directives,
                &registered.contract.writes,
                &CauseRef::Event(id),
                correlation_id,
            )?;
        }
        Ok(id)
    }

    fn append_event(
        &mut self,
        kind: EventKind,
        affected_entities: Vec<EntityRef>,
        summary: String,
        cause: Option<CauseRef>,
        correlation_id: u64,
    ) -> Result<SimEvent, CanwuError> {
        let (event_id, next_event_id) =
            claim_counter(self.state.counters.next_event_id, "event ID")?;
        let id = EventId::new(event_id);
        self.state.counters.next_event_id = next_event_id;
        let event = SimEvent {
            id,
            timestamp: self.state.scheduler.now,
            kind,
            affected_entities,
            summary,
            cause,
            correlation_id,
        };
        self.state.evidence.events.push(event.clone());
        Ok(event)
    }

    fn draw_random(
        &mut self,
        stream: &RandomStreamKey,
        upper_exclusive: u64,
        purpose: &str,
        producer: RandomDrawProducer,
        cause: CauseRef,
        correlation_id: u64,
    ) -> Result<(RandomDrawId, u64), CanwuError> {
        if upper_exclusive == 0
            || purpose.trim().is_empty()
            || purpose != purpose.trim()
            || correlation_id == 0
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidRandomDraw,
                "random draws require a positive bound, canonical purpose, and correlation",
            ));
        }
        let (draw_id, next_random_draw_id) =
            claim_counter(self.state.counters.next_random_draw_id, "random draw ID")?;
        self.invalidate_commitments(CommitmentDomains::RANDOM_STREAMS);
        let state = self
            .state
            .current
            .random_streams
            .get_mut(stream)
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidRandomStream,
                    format!(
                        "random stream {}.{}@{} is not initialized",
                        stream.namespace, stream.name, stream.version
                    ),
                )
            })?;
        let next_position = state.position.checked_add(1).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::IdentifierExhausted,
                "random stream position is exhausted",
            )
        })?;
        let position = state.position;
        let mut generator = DeterministicRng::from_seed(state.generator_state);
        let value = generator.range(upper_exclusive);
        state.position = next_position;
        state.generator_state = generator.state();
        self.state.counters.next_random_draw_id = next_random_draw_id;
        let id = RandomDrawId::new(draw_id);
        self.state.evidence.random_draws.push(RandomDrawRecord {
            id,
            at: self.state.scheduler.now,
            stream: stream.clone(),
            position,
            upper_exclusive,
            value,
            purpose: purpose.to_owned(),
            producer,
            outcome: None,
            cause,
            correlation_id,
        });
        Ok((id, value))
    }

    fn record_random_outcome(
        &mut self,
        id: RandomDrawId,
        outcome: RandomDrawOutcome,
    ) -> Result<(), CanwuError> {
        let Some(draw) = self
            .state
            .evidence
            .random_draws
            .last_mut()
            .filter(|draw| draw.id == id)
        else {
            return Err(CanwuError::new(
                ErrorCode::InvalidRandomDraw,
                "random draw outcome does not match the latest pending draw",
            ));
        };
        if draw.outcome.replace(outcome).is_some() {
            return Err(CanwuError::new(
                ErrorCode::InvalidRandomDraw,
                "random draw outcome was already recorded",
            ));
        }
        Ok(())
    }

    fn append_boundary_random_draws(
        &mut self,
        boundary: BoundaryId,
        correlation_id: u64,
        draws: Vec<PendingBoundaryRandomDraw>,
    ) -> Result<Vec<RandomDrawId>, CanwuError> {
        let mut ids = Vec::with_capacity(draws.len());
        for pending in draws {
            let (draw_id, next_random_draw_id) =
                claim_counter(self.state.counters.next_random_draw_id, "random draw ID")?;
            let id = RandomDrawId::new(draw_id);
            self.state.counters.next_random_draw_id = next_random_draw_id;
            self.state.evidence.random_draws.push(RandomDrawRecord {
                id,
                at: self.state.scheduler.now,
                stream: pending.draw.stream,
                position: pending.draw.position,
                upper_exclusive: pending.draw.upper_exclusive,
                value: pending.draw.value,
                purpose: pending.draw.purpose,
                producer: RandomDrawProducer::BoundarySystem {
                    boundary,
                    plugin: pending.plugin,
                    system: pending.system,
                },
                outcome: Some(RandomDrawOutcome::BoundarySystemDecision),
                cause: CauseRef::Boundary(boundary),
                correlation_id,
            });
            ids.push(id);
        }
        Ok(ids)
    }

    fn apply_boundary_stage(
        &mut self,
        boundary_id: BoundaryId,
        correlation_id: u64,
        directives: Vec<StagedBoundaryDirective>,
        evidence: &mut PendingBoundaryEvidence,
    ) -> Result<(), CanwuError> {
        let changes = &mut evidence.changes;
        let record_changes = &mut evidence.record_changes;
        let emissions = &mut evidence.emissions;
        let generated_ingress = &mut evidence.generated_ingress;
        let mutation_requests: Vec<_> = directives
            .iter()
            .filter_map(|staged| match &staged.directive {
                BoundaryDirective::MutateRecord { mutation, summary } => {
                    Some(records::DomainMutationRequest {
                        plugin: &staged.plugin,
                        system: &staged.system,
                        visibility: staged.visibility,
                        mutation,
                        summary,
                    })
                }
                BoundaryDirective::SetComponent { .. }
                | BoundaryDirective::Emit { .. }
                | BoundaryDirective::ScheduleIngress { .. } => None,
            })
            .collect();
        let mut stage_record_changes = BTreeMap::new();
        if !mutation_requests.is_empty() {
            let (next_records, applied) = records::apply_mutation_bundle(
                &self.state.current.domain_records,
                &self.plugins.record_schemas,
                self.state.scheduler.now,
                &|entity| runtime_entity_exists(&self.state, entity),
                mutation_requests,
            )?;
            let first_index = record_changes.len();
            for (offset, change) in applied.iter().enumerate() {
                let index = first_index.checked_add(offset).ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::IdentifierExhausted,
                        "boundary record-change index exceeds the persistent identifier space",
                    )
                })?;
                let index = u64::try_from(index).map_err(|_| {
                    CanwuError::new(
                        ErrorCode::IdentifierExhausted,
                        "boundary record-change index exceeds the persistent identifier space",
                    )
                })?;
                stage_record_changes
                    .insert(change.current.reference.clone(), (index, change.clone()));
            }
            self.invalidate_commitments(CommitmentDomains::DOMAIN_RECORDS);
            self.state.current.domain_records = next_records;
            record_changes.extend(applied);
        }

        for staged in &directives {
            let unavailable = match &staged.directive {
                BoundaryDirective::SetComponent { entity, .. } => {
                    (!runtime_entity_exists(&self.state, entity)).then_some(entity)
                }
                BoundaryDirective::Emit { affected, .. } => affected
                    .iter()
                    .find(|entity| !runtime_entity_exists(&self.state, entity)),
                BoundaryDirective::ScheduleIngress { affected, .. } => affected
                    .iter()
                    .find(|entity| !runtime_entity_identity_exists(&self.state, entity)),
                BoundaryDirective::MutateRecord { .. } => None,
            };
            if let Some(entity) = unavailable {
                return Err(CanwuError::new(
                    ErrorCode::EntityNotFound,
                    format!(
                        "boundary stage {}.{} references unavailable entity {entity}",
                        staged.plugin, staged.system
                    ),
                )
                .with_entity(entity.clone()));
            }
        }

        for staged in directives {
            match staged.directive {
                BoundaryDirective::SetComponent {
                    state,
                    entity,
                    component,
                    value,
                    summary,
                } => {
                    let key = component_key(&staged.plugin, &state, &entity, &component);
                    self.invalidate_commitments(CommitmentDomains::PLUGIN_COMPONENTS);
                    let previous = self
                        .state
                        .current
                        .plugin_components
                        .get(&key)
                        .map(|record| record.value.clone());
                    self.state.current.plugin_components.insert(
                        key,
                        PluginComponentRecord {
                            plugin: staged.plugin.clone(),
                            state: state.clone(),
                            entity: entity.clone(),
                            component: component.clone(),
                            value: value.clone(),
                        },
                    );
                    let change_index = u64::try_from(changes.len()).map_err(|_| {
                        CanwuError::new(
                            ErrorCode::IdentifierExhausted,
                            "boundary change index exceeds the persistent identifier space",
                        )
                    })?;
                    changes.push(BoundaryChange {
                        plugin: staged.plugin.clone(),
                        system: staged.system.clone(),
                        state,
                        entity: entity.clone(),
                        component: component.clone(),
                        previous,
                        value,
                        visibility: staged.visibility,
                        summary: summary.clone(),
                    });
                    let event = self.append_event(
                        EventKind::Plugin {
                            plugin: staged.plugin.clone(),
                            event_type: format!("{component}_changed"),
                        },
                        vec![entity],
                        summary,
                        Some(CauseRef::Boundary(boundary_id)),
                        correlation_id,
                    )?;
                    emissions.push(BoundaryEmission {
                        plugin: staged.plugin,
                        system: staged.system,
                        event: event.id,
                        kind: BoundaryEmissionKind::Change { change_index },
                    });
                }
                BoundaryDirective::MutateRecord { mutation, .. } => {
                    let Some((change_index, change)) = stage_record_changes.get(mutation.target())
                    else {
                        return Err(CanwuError::new(
                            ErrorCode::InvalidBoundary,
                            "record mutation is missing its committed change evidence",
                        ));
                    };
                    let event = self.append_event(
                        EventKind::Plugin {
                            plugin: staged.plugin.clone(),
                            event_type: change.operation.event_type().to_owned(),
                        },
                        record_change_affected_entities(change),
                        change.summary.clone(),
                        Some(CauseRef::Boundary(boundary_id)),
                        correlation_id,
                    )?;
                    emissions.push(BoundaryEmission {
                        plugin: staged.plugin,
                        system: staged.system,
                        event: event.id,
                        kind: BoundaryEmissionKind::RecordChange {
                            change_index: *change_index,
                        },
                    });
                }
                BoundaryDirective::Emit {
                    event_type,
                    summary,
                    affected,
                } => {
                    let event = self.append_event(
                        EventKind::Plugin {
                            plugin: staged.plugin.clone(),
                            event_type,
                        },
                        affected,
                        summary,
                        Some(CauseRef::Boundary(boundary_id)),
                        correlation_id,
                    )?;
                    emissions.push(BoundaryEmission {
                        plugin: staged.plugin,
                        system: staged.system,
                        event: event.id,
                        kind: BoundaryEmissionKind::Explicit,
                    });
                }
                BoundaryDirective::ScheduleIngress {
                    after,
                    packet_type,
                    priority,
                    payload,
                    mut affected,
                } => {
                    self.ensure_canonical_ingress_can_start()?;
                    let descriptor = self
                        .plugins
                        .ingress
                        .get(&(staged.plugin.clone(), packet_type.clone()))
                        .ok_or_else(|| {
                            CanwuError::new(
                                ErrorCode::InvalidPayload,
                                format!(
                                    "boundary system {}.{} scheduled undeclared ingress type {packet_type}",
                                    staged.plugin, staged.system
                                ),
                            )
                        })?
                        .clone();
                    descriptor.payload_schema.validate(&payload)?;
                    affected.sort();
                    affected.dedup();
                    let due_at = self.state.scheduler.now.checked_add(after).ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidDuration,
                            "boundary-generated ingress exceeds the supported time range",
                        )
                    })?;
                    let receipt = self.append_ingress(
                        due_at,
                        descriptor.class,
                        priority,
                        IngressPayload::Plugin {
                            plugin: staged.plugin.clone(),
                            packet_type,
                            payload,
                            affected_entities: affected,
                        },
                        Some(CauseRef::Boundary(boundary_id)),
                        true,
                    )?;
                    generated_ingress.push(BoundaryIngressGeneration {
                        ingress: receipt.ingress_id,
                        plugin: staged.plugin,
                        system: staged.system,
                        phase: staged.phase,
                        visibility: staged.visibility,
                    });
                }
            }
        }
        validate_runtime_domain_dependents(&self.state)?;
        Ok(())
    }

    fn apply_directives(
        &mut self,
        plugin: &str,
        directives: Vec<SystemDirective>,
        allowed_writes: &[StateKey],
        cause: &CauseRef,
        correlation_id: u64,
    ) -> Result<(), CanwuError> {
        for directive in directives {
            match directive {
                SystemDirective::SetComponent {
                    state,
                    entity,
                    component,
                    value,
                    summary,
                } => {
                    let key = component_key(plugin, &state, &entity, &component);
                    self.invalidate_commitments(CommitmentDomains::PLUGIN_COMPONENTS);
                    self.state.current.plugin_components.insert(
                        key,
                        PluginComponentRecord {
                            plugin: plugin.to_owned(),
                            state,
                            entity: entity.clone(),
                            component: component.clone(),
                            value,
                        },
                    );
                    self.emit(
                        EventKind::Plugin {
                            plugin: plugin.to_owned(),
                            event_type: format!("{component}_changed"),
                        },
                        vec![entity],
                        summary,
                        Some(cause.clone()),
                        correlation_id,
                    )?;
                }
                SystemDirective::Emit {
                    event_type,
                    summary,
                    affected,
                } => {
                    self.emit(
                        EventKind::Plugin {
                            plugin: plugin.to_owned(),
                            event_type,
                        },
                        affected,
                        summary,
                        Some(cause.clone()),
                        correlation_id,
                    )?;
                }
                SystemDirective::Schedule { after, directive } => {
                    let at = self.state.scheduler.now.checked_add(after).ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidDuration,
                            "plugin scheduled time exceeds the supported range",
                        )
                    })?;
                    self.schedule_at(
                        at,
                        ScheduledAction::PluginDirective {
                            plugin: plugin.to_owned(),
                            directive,
                            allowed_writes: allowed_writes.to_vec(),
                            cause: cause.clone(),
                            correlation_id,
                        },
                    )?;
                }
            }
        }
        Ok(())
    }

    fn schedule_at(&mut self, at: SimTime, action: ScheduledAction) -> Result<(), CanwuError> {
        if at <= self.state.scheduler.now {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "scheduled work must target a strictly future simulation time",
            ));
        }
        let (sequence, next_sequence) = claim_counter(
            self.state.counters.next_schedule_sequence,
            "schedule sequence",
        )?;
        let key = ScheduleKey { at, sequence };
        self.state.counters.next_schedule_sequence = next_sequence;
        self.invalidate_commitments(CommitmentDomains::SCHEDULER);
        if self.state.scheduler.actions.insert(key, action).is_some() {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "the runtime attempted to reuse a schedule key",
            ));
        }
        Ok(())
    }

    fn plugin_view<'a>(&'a self, reader: &'a str, reads: &'a [StateKey]) -> SimulationView<'a> {
        SimulationView {
            state: &self.state,
            state_owners: &self.plugins.state_owners,
            reader: Some(reader),
            allowed_reads: Some(reads),
            allowed_ingress: None,
            ingress_plugin: None,
            component_overlay: None,
            proposed_components: None,
            record_overlay: None,
            proposed_records: None,
            allocations: None,
            allowed_reservations: None,
            random_session: None,
        }
    }
}

fn enqueue_replay_ingress_cut(
    simulation: &mut Simulation,
    ingress: &[IngressRecord],
    next_ingress: &mut usize,
    boundary_count: usize,
) -> Result<(), CanwuError> {
    let expected_boundary_count = u64::try_from(boundary_count).map_err(|_| {
        CanwuError::new(
            ErrorCode::ReplayMismatch,
            "boundary count exceeds ingress range",
        )
    })?;
    while let Some(record) = ingress.get(*next_ingress) {
        if record.eligible_boundary_count < expected_boundary_count {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "ingress journal skipped its recorded issue boundary",
            ));
        }
        if record.eligible_boundary_count > expected_boundary_count {
            break;
        }
        if record.issued_at < simulation.time() {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "ingress journal issue time precedes replay state",
            ));
        }
        if record.issued_at > simulation.time() {
            simulation.ensure_legacy_advance_does_not_cross_ingress(record.issued_at)?;
            simulation.advance_to(record.issued_at)?;
        }
        if let Some(actual) = simulation.state.evidence.ingress.get(*next_ingress) {
            if actual != record {
                return Err(CanwuError::new(
                    ErrorCode::ReplayMismatch,
                    "plugin-generated ingress does not match journal evidence",
                ));
            }
            *next_ingress += 1;
            continue;
        }
        if matches!(record.cause, Some(CauseRef::Boundary(_))) {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "recorded boundary-generated ingress was not reproduced by its plugin system",
            ));
        }
        let receipt = match &record.payload {
            IngressPayload::Command { request } => simulation.enqueue_command(
                record.due_at,
                record.priority,
                request.as_ref().clone(),
            )?,
            IngressPayload::Plugin {
                plugin,
                packet_type,
                payload,
                affected_entities,
            } => {
                let mut request = PluginIngressRequest::new(
                    plugin.clone(),
                    packet_type.clone(),
                    record.due_at,
                    payload.clone(),
                )
                .with_priority(record.priority);
                request.affected_entities.clone_from(affected_entities);
                request.cause.clone_from(&record.cause);
                simulation.enqueue_plugin_ingress(request)?
            }
            IngressPayload::Calendar { cadences } => {
                simulation.schedule_calendar_boundary(record.due_at, cadences.clone())?
            }
        };
        if receipt.ingress_id != record.id
            || simulation.state.evidence.ingress.last() != Some(record)
        {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "regenerated ingress record does not match journal evidence",
            ));
        }
        *next_ingress += 1;
    }
    Ok(())
}

fn replay_command_record(
    simulation: &mut Simulation,
    record: &CommandRecord,
    latest_time: SimTime,
) -> Result<(), CanwuError> {
    if record.accepted_at < simulation.time() || record.accepted_at > latest_time {
        return Err(CanwuError::new(
            ErrorCode::ReplayMismatch,
            "replay command timestamps do not match authoritative operation order",
        ));
    }
    simulation.ensure_legacy_advance_does_not_cross_ingress(record.accepted_at)?;
    simulation.advance_to(record.accepted_at)?;
    let CommandOutcome::Accepted { receipt } = simulation.admit_command(
        None,
        None,
        record.envelope.clone(),
        CommandIngress::LegacyDirect,
        false,
    )?
    else {
        return Err(CanwuError::new(
            ErrorCode::ReplayMismatch,
            "legacy replay command was rejected",
        ));
    };
    if receipt.command_id != record.id {
        return Err(CanwuError::new(
            ErrorCode::ReplayMismatch,
            "replay command IDs did not match the journal",
        ));
    }
    Ok(())
}

fn replay_attempt_record(
    simulation: &mut Simulation,
    record: &CommandAttemptRecord,
    commands: &[CommandRecord],
    latest_time: SimTime,
) -> Result<(), CanwuError> {
    if record.at < simulation.time() || record.at > latest_time {
        return Err(CanwuError::new(
            ErrorCode::ReplayMismatch,
            "replay command-attempt timestamps do not match authoritative operation order",
        ));
    }
    simulation.ensure_legacy_advance_does_not_cross_ingress(record.at)?;
    simulation.advance_to(record.at)?;
    let outcome = simulation.admit_command(
        record.request_id,
        record.expected_revision,
        record.envelope.clone(),
        record.ingress,
        true,
    )?;
    if simulation.command_attempts().last() != Some(record) {
        return Err(CanwuError::new(
            ErrorCode::ReplayMismatch,
            format!(
                "regenerated command attempt {} did not match its journal evidence",
                record.id
            ),
        ));
    }
    match (&record.outcome, outcome) {
        (CommandAttemptOutcome::Accepted { command_id }, CommandOutcome::Accepted { receipt })
            if receipt.command_id == *command_id =>
        {
            let index = usize::try_from(command_id.get().saturating_sub(1)).map_err(|_| {
                CanwuError::new(
                    ErrorCode::ReplayMismatch,
                    "replayed command ID exceeds the journal index range",
                )
            })?;
            if simulation.command_log().last() != commands.get(index) {
                return Err(CanwuError::new(
                    ErrorCode::ReplayMismatch,
                    "regenerated command record did not match its journal evidence",
                ));
            }
        }
        (
            CommandAttemptOutcome::Rejected { error: expected },
            CommandOutcome::Rejected { rejection },
        ) if rejection.error == *expected => {}
        _ => {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "replayed command-attempt outcome differs from its journal evidence",
            ));
        }
    }
    Ok(())
}

enum PreparedCommand {
    MoveArmy {
        army: ArmyId,
        actor: PersonId,
        from: TerritoryId,
        destination: TerritoryId,
        arrival_at: SimTime,
    },
    DebugMorale {
        army: ArmyId,
        old_morale: u16,
        new_morale: u16,
    },
    Plugin {
        plugin: String,
        directives: Vec<SystemDirective>,
        allowed_writes: Vec<StateKey>,
    },
}

impl PreparedCommand {
    fn commitment_invalidation(&self) -> CommitmentDomains {
        match self {
            Self::MoveArmy { .. } => {
                CommitmentDomains::WORLD
                    | CommitmentDomains::KNOWLEDGE
                    | CommitmentDomains::PLUGIN_COMPONENTS
                    | CommitmentDomains::SCHEDULER
            }
            Self::DebugMorale { .. } => {
                CommitmentDomains::WORLD
                    | CommitmentDomains::PLUGIN_COMPONENTS
                    | CommitmentDomains::SCHEDULER
            }
            Self::Plugin { .. } => {
                CommitmentDomains::PLUGIN_COMPONENTS | CommitmentDomains::SCHEDULER
            }
        }
    }
}

struct PendingReservationOffer {
    plugin: String,
    system: String,
    offer: ReservationOffer,
}

struct PendingReservationRequest {
    reservation: ReservationRef,
    request: ReservationRequest,
}

struct ReservationAllocationResult {
    by_reservation: BTreeMap<ReservationRef, ReservationAllocation>,
    offers: Vec<ReservationOfferRecord>,
    requests: Vec<ReservationRequestRecord>,
    records: Vec<ReservationAllocation>,
}

struct StagedBoundaryDirective {
    plugin: String,
    system: String,
    phase: BoundaryPhase,
    visibility: StateVisibility,
    directive: BoundaryDirective,
}

#[derive(Default)]
struct PendingBoundaryEvidence {
    changes: Vec<BoundaryChange>,
    record_changes: Vec<DomainRecordChange>,
    emissions: Vec<BoundaryEmission>,
    generated_ingress: Vec<BoundaryIngressGeneration>,
}

struct PendingBoundaryRandomDraw {
    plugin: String,
    system: String,
    draw: random::PendingRandomDraw,
}

fn boundary_system_due(
    contract: &BoundarySystemContract,
    cadences: &[SystemCadence],
    has_admitted_events: bool,
) -> bool {
    match contract.cadence {
        SystemCadence::EventDriven => has_admitted_events,
        _ => cadences.contains(&contract.cadence),
    }
}

fn boundary_has_event_ingress(record: &BoundaryRecord) -> bool {
    !record.admitted_events.is_empty() || !record.admitted_ingress.is_empty()
}

fn validate_boundary_proposal(
    plugin: &str,
    contract: &BoundarySystemContract,
    state: &RuntimeState,
    plugins: &PluginRegistry,
    record_overlay: &BTreeMap<DomainRecordRef, DomainRecord>,
    proposal: &BoundaryProposal,
) -> Result<(), CanwuError> {
    if contract.phase != BoundaryPhase::ReservationAndAllocation
        && (!proposal.offers.is_empty() || !proposal.requests.is_empty())
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            format!(
                "boundary system {plugin}.{} proposed reservations in phase {:?}",
                contract.name, contract.phase
            ),
        ));
    }

    let entity_exists = |entity: &EntityRef| {
        proposal_entity_exists(
            state,
            &plugins.record_schemas,
            record_overlay,
            proposal,
            entity,
        )
    };
    let mut offered_pools = BTreeSet::new();
    for offer in &proposal.offers {
        validate_reservation_pool(&offer.pool, &entity_exists)?;
        if !contract.reservation_offers.contains(&offer.pool.state)
            || plugins
                .state_owners
                .get(&offer.pool.state)
                .is_none_or(|owner| owner != plugin)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                format!(
                    "boundary system {plugin}.{} offered undeclared state {}.{}",
                    contract.name, offer.pool.state.namespace, offer.pool.state.name
                ),
            ));
        }
        if !offered_pools.insert(&offer.pool) {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                format!(
                    "boundary system {plugin}.{} offered the same reservation pool twice",
                    contract.name
                ),
            ));
        }
    }

    let mut request_names = BTreeSet::new();
    for request in &proposal.requests {
        validate_reservation_pool(&request.pool, &entity_exists)?;
        if request.request.trim().is_empty()
            || request.request != request.request.trim()
            || request.tie_break.trim().is_empty()
            || request.tie_break != request.tie_break.trim()
            || request.quantity == 0
            || !request_names.insert(&request.request)
            || !contract.reservation_requests.contains(&request.pool.state)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                format!(
                    "boundary system {plugin}.{} produced an invalid reservation request",
                    contract.name
                ),
            ));
        }
    }

    let mut component_keys = BTreeSet::new();
    let mut record_targets = BTreeSet::new();
    for directive in &proposal.directives {
        match directive {
            BoundaryDirective::SetComponent {
                state: state_key,
                entity,
                component,
                ..
            } => {
                if component.trim().is_empty()
                    || component != component.trim()
                    || !contract.writes.contains(state_key)
                    || plugins
                        .state_owners
                        .get(state_key)
                        .is_none_or(|owner| owner != plugin)
                    || is_domain_record_state(&plugins.record_schemas, state_key)
                {
                    return Err(CanwuError::new(
                        ErrorCode::UndeclaredStateWrite,
                        format!(
                            "boundary system {plugin}.{} produced an undeclared component write",
                            contract.name
                        ),
                    ));
                }
                if !entity_exists(entity) {
                    return Err(CanwuError::new(
                        ErrorCode::EntityNotFound,
                        format!(
                            "boundary system {plugin}.{} targeted missing entity {entity}",
                            contract.name
                        ),
                    )
                    .with_entity(entity.clone()));
                }
                let key = component_key(plugin, state_key, entity, component);
                if !component_keys.insert(key) {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidBoundary,
                        format!(
                            "boundary system {plugin}.{} wrote the same component twice",
                            contract.name
                        ),
                    ));
                }
            }
            BoundaryDirective::MutateRecord { mutation, summary } => {
                let target = mutation.target();
                let state_key = records::record_state_key(&target.kind);
                if !canonical_text(summary)
                    || !contract.writes.contains(&state_key)
                    || plugins
                        .state_owners
                        .get(&state_key)
                        .is_none_or(|owner| owner != plugin)
                    || plugins
                        .record_schemas
                        .get(&target.kind)
                        .is_none_or(|(owner, _)| owner != plugin)
                {
                    return Err(CanwuError::new(
                        ErrorCode::UndeclaredStateWrite,
                        format!(
                            "boundary system {plugin}.{} produced an undeclared record mutation",
                            contract.name
                        ),
                    ));
                }
                if !record_targets.insert(target.clone()) {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidBoundary,
                        format!(
                            "boundary system {plugin}.{} mutated the same record twice",
                            contract.name
                        ),
                    ));
                }
            }
            BoundaryDirective::Emit {
                event_type,
                affected,
                ..
            } => {
                if event_type.trim().is_empty()
                    || event_type != event_type.trim()
                    || !contract.emits.contains(event_type)
                {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidBoundary,
                        format!(
                            "boundary system {plugin}.{} emitted an undeclared event type",
                            contract.name
                        ),
                    ));
                }
                if affected.iter().any(|entity| !entity_exists(entity)) {
                    return Err(CanwuError::new(
                        ErrorCode::EntityNotFound,
                        format!(
                            "boundary system {plugin}.{} emitted an event for a missing entity",
                            contract.name
                        ),
                    ));
                }
            }
            BoundaryDirective::ScheduleIngress {
                after,
                packet_type,
                payload,
                affected,
                ..
            } => {
                let descriptor = plugins
                    .ingress
                    .get(&(plugin.to_owned(), packet_type.clone()))
                    .ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidPayload,
                            format!(
                                "boundary system {plugin}.{} scheduled undeclared ingress type {packet_type}",
                                contract.name
                            ),
                        )
                    })?;
                if after.is_negative() || state.scheduler.now.checked_add(*after).is_none() {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidDuration,
                        "boundary-generated ingress requires a nonnegative supported delay",
                    ));
                }
                descriptor.payload_schema.validate(payload)?;
                if affected.iter().any(|entity| {
                    !proposal_entity_identity_exists(
                        state,
                        &plugins.record_schemas,
                        proposal,
                        entity,
                    )
                }) {
                    return Err(CanwuError::new(
                        ErrorCode::EntityNotFound,
                        format!(
                            "boundary system {plugin}.{} scheduled ingress for an unknown entity identity",
                            contract.name
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_reservation_pool(
    pool: &ReservationPoolKey,
    entity_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(), CanwuError> {
    if pool.resource.trim().is_empty()
        || pool.resource != pool.resource.trim()
        || !entity_exists(&pool.entity)
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "reservation pools require a canonical resource and an existing entity",
        ));
    }
    Ok(())
}

fn extend_boundary_overlay(
    state: &RuntimeState,
    record_overlay: &BTreeMap<DomainRecordRef, DomainRecord>,
    overlay: &mut BTreeMap<PluginComponentKey, PluginComponentRecord>,
    directives: &[StagedBoundaryDirective],
) -> Result<(), CanwuError> {
    extend_boundary_component_overlay(state, record_overlay, overlay, directives, false)
}

fn extend_boundary_candidate_overlay(
    state: &RuntimeState,
    record_overlay: &BTreeMap<DomainRecordRef, DomainRecord>,
    overlay: &mut BTreeMap<PluginComponentKey, PluginComponentRecord>,
    directives: &[StagedBoundaryDirective],
) -> Result<(), CanwuError> {
    extend_boundary_component_overlay(state, record_overlay, overlay, directives, true)
}

fn extend_boundary_component_overlay(
    state: &RuntimeState,
    record_overlay: &BTreeMap<DomainRecordRef, DomainRecord>,
    overlay: &mut BTreeMap<PluginComponentKey, PluginComponentRecord>,
    directives: &[StagedBoundaryDirective],
    include_next_boundary: bool,
) -> Result<(), CanwuError> {
    for staged in directives.iter().filter(|staged| {
        include_next_boundary || staged.visibility == StateVisibility::SameBoundary
    }) {
        if let BoundaryDirective::SetComponent {
            state: state_key,
            entity,
            component,
            value,
            ..
        } = &staged.directive
        {
            let key = component_key(&staged.plugin, state_key, entity, component);
            if overlay.contains_key(&key) {
                return Err(CanwuError::new(
                    ErrorCode::InvalidBoundary,
                    "multiple boundary proposals target the same component",
                ));
            }
            if !runtime_entity_exists_with_record_overlay(state, record_overlay, entity) {
                return Err(CanwuError::new(
                    ErrorCode::EntityNotFound,
                    format!("boundary proposal targeted missing entity {entity}"),
                ));
            }
            overlay.insert(
                key,
                PluginComponentRecord {
                    plugin: staged.plugin.clone(),
                    state: state_key.clone(),
                    entity: entity.clone(),
                    component: component.clone(),
                    value: value.clone(),
                },
            );
        }
    }
    Ok(())
}

fn extend_boundary_record_overlay(
    state: &RuntimeState,
    schemas: &records::DomainRecordSchemas,
    overlay: &mut BTreeMap<DomainRecordRef, DomainRecord>,
    directives: &[StagedBoundaryDirective],
) -> Result<(), CanwuError> {
    extend_boundary_domain_record_overlay(state, schemas, overlay, directives, false)
}

fn extend_boundary_record_candidate_overlay(
    state: &RuntimeState,
    schemas: &records::DomainRecordSchemas,
    overlay: &mut BTreeMap<DomainRecordRef, DomainRecord>,
    directives: &[StagedBoundaryDirective],
) -> Result<(), CanwuError> {
    extend_boundary_domain_record_overlay(state, schemas, overlay, directives, true)
}

fn extend_boundary_domain_record_overlay(
    state: &RuntimeState,
    schemas: &records::DomainRecordSchemas,
    overlay: &mut BTreeMap<DomainRecordRef, DomainRecord>,
    directives: &[StagedBoundaryDirective],
    include_next_boundary: bool,
) -> Result<(), CanwuError> {
    let mut base = state.current.domain_records.clone();
    base.extend(
        overlay
            .iter()
            .map(|(reference, record)| (reference.clone(), record.clone())),
    );
    let requests: Vec<_> = directives
        .iter()
        .filter(|staged| {
            include_next_boundary || staged.visibility == StateVisibility::SameBoundary
        })
        .filter_map(|staged| match &staged.directive {
            BoundaryDirective::MutateRecord { mutation, summary } => {
                Some(records::DomainMutationRequest {
                    plugin: &staged.plugin,
                    system: &staged.system,
                    visibility: staged.visibility,
                    mutation,
                    summary,
                })
            }
            BoundaryDirective::SetComponent { .. }
            | BoundaryDirective::Emit { .. }
            | BoundaryDirective::ScheduleIngress { .. } => None,
        })
        .collect();
    if requests.is_empty() {
        return Ok(());
    }
    let (next, changes) = records::apply_mutation_bundle(
        &base,
        schemas,
        state.scheduler.now,
        &|entity| runtime_entity_exists(state, entity),
        requests,
    )?;
    validate_domain_dependents_with_records(state, &next)?;
    for change in changes {
        overlay.insert(change.current.reference.clone(), change.current);
    }
    Ok(())
}

fn partition_boundary_visibility(
    directives: Vec<StagedBoundaryDirective>,
) -> (Vec<StagedBoundaryDirective>, Vec<StagedBoundaryDirective>) {
    directives
        .into_iter()
        .partition(|staged| staged.visibility == StateVisibility::SameBoundary)
}

fn allocate_reservations(
    mut offers: Vec<PendingReservationOffer>,
    mut requests: Vec<PendingReservationRequest>,
) -> Result<ReservationAllocationResult, CanwuError> {
    offers.sort_by(|left, right| {
        left.offer
            .pool
            .cmp(&right.offer.pool)
            .then_with(|| left.plugin.cmp(&right.plugin))
            .then_with(|| left.system.cmp(&right.system))
    });
    let mut remaining = BTreeMap::new();
    let mut offer_records = Vec::new();
    for pending in offers {
        if remaining
            .insert(pending.offer.pool.clone(), pending.offer.capacity)
            .is_some()
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                format!(
                    "reservation pool was offered more than once, including by {}.{}",
                    pending.plugin, pending.system
                ),
            ));
        }
        offer_records.push(ReservationOfferRecord {
            plugin: pending.plugin,
            system: pending.system,
            offer: pending.offer,
        });
    }
    requests.sort_by(|left, right| {
        left.request
            .pool
            .cmp(&right.request.pool)
            .then_with(|| right.request.priority.cmp(&left.request.priority))
            .then_with(|| left.request.tie_break.cmp(&right.request.tie_break))
            .then_with(|| left.reservation.cmp(&right.reservation))
    });
    let mut seen = BTreeSet::new();
    let mut by_reservation = BTreeMap::new();
    let mut request_records = Vec::new();
    let mut records = Vec::new();
    for pending in requests {
        if !seen.insert(pending.reservation.clone()) {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "reservation request identity is duplicated",
            ));
        }
        request_records.push(ReservationRequestRecord {
            reservation: pending.reservation.clone(),
            request: pending.request.clone(),
        });
        let available = remaining.entry(pending.request.pool.clone()).or_default();
        let granted = pending.request.quantity.min(*available);
        *available -= granted;
        let disposition = if granted == pending.request.quantity {
            ReservationDisposition::Fulfilled
        } else if granted == 0 {
            ReservationDisposition::Rejected
        } else {
            ReservationDisposition::Partial
        };
        let allocation = ReservationAllocation {
            reservation: pending.reservation.clone(),
            pool: pending.request.pool,
            requested: pending.request.quantity,
            granted,
            remaining_after: *available,
            disposition,
        };
        by_reservation.insert(pending.reservation, allocation.clone());
        records.push(allocation);
    }
    Ok(ReservationAllocationResult {
        by_reservation,
        offers: offer_records,
        requests: request_records,
        records,
    })
}

fn validate_directives(
    plugin: &str,
    allowed_writes: &[StateKey],
    state_owners: &BTreeMap<StateKey, String>,
    record_schemas: &records::DomainRecordSchemas,
    entity_exists: &dyn Fn(&EntityRef) -> bool,
    directives: &[SystemDirective],
) -> Result<(), CanwuError> {
    for directive in directives {
        match directive {
            SystemDirective::SetComponent {
                state,
                entity,
                component,
                ..
            } => {
                if component.trim().is_empty() || component != component.trim() {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidPayload,
                        "plugin component name must be non-empty and canonical",
                    ));
                }
                if !allowed_writes.contains(state) {
                    return Err(CanwuError::new(
                        ErrorCode::UndeclaredStateWrite,
                        format!(
                            "plugin {plugin} did not declare write access to {}.{}",
                            state.namespace, state.name
                        ),
                    ));
                }
                if state_owners.get(state).is_none_or(|owner| owner != plugin) {
                    return Err(CanwuError::new(
                        ErrorCode::UndeclaredStateWrite,
                        format!(
                            "plugin {plugin} does not own state {}.{}",
                            state.namespace, state.name
                        ),
                    ));
                }
                if is_domain_record_state(record_schemas, state) {
                    return Err(CanwuError::new(
                        ErrorCode::UndeclaredStateWrite,
                        "domain record state cannot be written as an immediate component",
                    ));
                }
                if !entity_exists(entity) {
                    return Err(CanwuError::new(
                        ErrorCode::EntityNotFound,
                        format!("plugin {plugin} targeted missing entity {entity}"),
                    )
                    .with_entity(entity.clone()));
                }
            }
            SystemDirective::Emit { event_type, .. }
                if event_type.trim().is_empty() || event_type != event_type.trim() =>
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPayload,
                    "plugin event type must be non-empty and canonical",
                ));
            }
            SystemDirective::Emit { affected, .. }
                if affected.iter().any(|entity| !entity_exists(entity)) =>
            {
                return Err(CanwuError::new(
                    ErrorCode::EntityNotFound,
                    format!("plugin {plugin} emitted an event for a missing entity"),
                ));
            }
            SystemDirective::Schedule { after, directive } => {
                if *after <= SimDuration::ZERO {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidDuration,
                        "plugin systems must schedule work strictly in the future",
                    ));
                }
                validate_directives(
                    plugin,
                    allowed_writes,
                    state_owners,
                    record_schemas,
                    entity_exists,
                    std::slice::from_ref(directive),
                )?;
            }
            SystemDirective::Emit { .. } => {}
        }
    }
    Ok(())
}

fn resolve_command_authority(envelope: &CommandEnvelope) -> Result<CommandAuthority, CanwuError> {
    if let Some(authority) = &envelope.authority {
        return Ok(authority.clone());
    }
    match &envelope.issuer {
        Issuer::Actor(actor) => Ok(CommandAuthority::for_actor(*actor)),
        Issuer::Debug => Ok(CommandAuthority::no_responsible_actor("debug-command")),
        Issuer::System(system) => Ok(CommandAuthority::no_responsible_actor(format!(
            "system:{system}"
        ))),
        Issuer::Human(_)
        | Issuer::Ai(_)
        | Issuer::Institution(_)
        | Issuer::Replay(_)
        | Issuer::Experiment(_) => Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "typed command origins require an explicit authority context",
        )),
    }
}

fn validate_command_ingress_policy(
    run_configuration: &RunConfigurationSnapshot,
    issuer: &Issuer,
    authority: &CommandAuthority,
    admission: CommandAdmission,
    entity_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(), CanwuError> {
    let CommandAdmission {
        request_id,
        expected_revision,
        expected_time,
        revision_before: current_revision,
        ingress,
    } = admission;
    if request_id.is_some_and(|id| id.get() == 0) {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            "command request IDs must be nonzero",
        ));
    }
    if let Some(expected) = expected_revision
        && expected != current_revision
    {
        return Err(CanwuError::new(
            ErrorCode::SimulationRevisionConflict,
            format!(
                "command expected revision {expected}, but simulation is at revision {current_revision}"
            ),
        ));
    }
    validate_command_authority(authority, entity_exists)?;
    if matches!(issuer, Issuer::Replay(_)) != (ingress == CommandIngress::FrozenReplay) {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "replay command origins are valid only for frozen replay ingress",
        ));
    }

    let RunConfigurationSnapshot::Declared(configuration) = run_configuration else {
        return Ok(());
    };
    if ingress == CommandIngress::LegacyDirect {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "declared runs require tracked request or frozen replay ingress",
        ));
    }
    let external = !matches!(issuer, Issuer::System(_));
    if configuration.require_idempotency_keys && external && request_id.is_none() {
        return Err(CanwuError::new(
            ErrorCode::MissingIdempotencyKey,
            "this run requires a stable command request ID",
        ));
    }
    if configuration.require_idempotency_keys && external && expected_revision.is_none() {
        return Err(CanwuError::new(
            ErrorCode::SimulationRevisionConflict,
            "this run requires an expected command revision",
        ));
    }
    if configuration.interaction == InteractionPolicy::ReadOnly
        && !matches!(issuer, Issuer::Replay(_) | Issuer::System(_))
    {
        return Err(CanwuError::new(
            ErrorCode::InteractionReadOnly,
            "the run interaction policy rejects newly authored authoritative commands",
        ));
    }
    if external && expected_time.is_none() {
        return Err(CanwuError::new(
            ErrorCode::SimulationTimeConflict,
            "declared external commands require an expected simulation time",
        ));
    }

    match issuer {
        Issuer::Actor(_) => Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "declared runs require a typed human, AI, institution, replay, experiment, debug, or system origin",
        )),
        Issuer::Human(controller) => {
            let Some(binding) = &configuration.seat_binding else {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "human commands require the run's exact seat binding",
                ));
            };
            if configuration.controller != ControllerPolicy::HumanRoleBound
                || controller != &binding.controller_id
                || authority.seat_id.as_deref() != Some(binding.seat_id.as_str())
                || authority.permission_profile_id.as_deref()
                    != Some(binding.permission_profile_id.as_str())
                || !authority_matches_seat_binding(configuration.seat, binding, authority)
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "human command origin does not match the active controller, seat binding, and permission profile",
                ));
            }
            Ok(())
        }
        Issuer::Ai(controller) | Issuer::Institution(controller) => {
            if !canonical_text(controller)
                || matches!(
                    authority.decision_origin,
                    DecisionOrigin::NoResponsibleActor { .. }
                )
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "AI and institutional commands require a canonical controller and responsible decision origin",
                ));
            }
            Ok(())
        }
        Issuer::Replay(source) => {
            if !canonical_text(source)
                || ingress != CommandIngress::FrozenReplay
                || configuration.purpose != RunPurpose::Replay
                || configuration.controller != ControllerPolicy::ReplayController
                || configuration.interaction != InteractionPolicy::ReadOnly
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "replay command sources require a replay-purpose, replay-controller, read-only run",
                ));
            }
            if let Some(binding) = &configuration.seat_binding
                && (source != &binding.controller_id
                    || authority.seat_id.as_deref() != Some(binding.seat_id.as_str())
                    || authority.permission_profile_id.as_deref()
                        != Some(binding.permission_profile_id.as_str())
                    || !authority_matches_seat_binding(configuration.seat, binding, authority))
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "frozen replay input does not match its recorded controller and seat binding",
                ));
            }
            Ok(())
        }
        Issuer::Experiment(intervention) => {
            if configuration.interaction != InteractionPolicy::VersionedExperiment
                || !configuration.declared_interventions.contains(intervention)
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "experiment commands must name an intervention declared by the run",
                ));
            }
            Ok(())
        }
        Issuer::Debug => {
            if !configuration.diagnostic_commands_enabled {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "debug command authority is disabled by the run configuration",
                ));
            }
            Ok(())
        }
        Issuer::System(system) => {
            if !canonical_text(system)
                || !matches!(
                    authority.decision_origin,
                    DecisionOrigin::NoResponsibleActor { .. }
                )
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "system commands require a canonical system ID and typed no-responsible-actor origin",
                ));
            }
            Ok(())
        }
    }
}

fn validate_command_authority(
    authority: &CommandAuthority,
    entity_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(), CanwuError> {
    if authority
        .seat_id
        .as_ref()
        .is_some_and(|value| !canonical_text(value))
        || authority
            .permission_profile_id
            .as_ref()
            .is_some_and(|value| !canonical_text(value))
        || authority.seat_id.is_some() != authority.permission_profile_id.is_some()
        || authority
            .command_subject
            .as_ref()
            .is_some_and(|entity| !entity_exists(entity))
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "command authority contains an invalid seat, permission profile, or subject",
        ));
    }
    match &authority.decision_origin {
        DecisionOrigin::Actor { actor } => {
            if !entity_exists(&EntityRef::Person(*actor)) {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "command decision origin references a missing actor",
                ));
            }
        }
        DecisionOrigin::Institution {
            institution,
            responsible_actor,
        } => {
            if !entity_exists(institution)
                || responsible_actor.is_some_and(|actor| !entity_exists(&EntityRef::Person(actor)))
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "command decision origin references a missing institution or actor",
                ));
            }
        }
        DecisionOrigin::Council { council_id } if !canonical_text(council_id) => {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "command council origin requires a canonical ID",
            ));
        }
        DecisionOrigin::NoResponsibleActor { reason } if !canonical_text(reason) => {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "no-responsible-actor origins require a canonical reason",
            ));
        }
        DecisionOrigin::Council { .. } | DecisionOrigin::NoResponsibleActor { .. } => {}
    }
    Ok(())
}

fn authority_matches_seat_binding(
    seat: SeatPolicy,
    binding: &SeatBinding,
    authority: &CommandAuthority,
) -> bool {
    match (seat, &authority.decision_origin) {
        (SeatPolicy::CharacterBound, DecisionOrigin::Actor { actor }) => {
            binding.actor == Some(*actor) && binding.institution.is_none()
        }
        (
            SeatPolicy::InstitutionBound,
            DecisionOrigin::Institution {
                institution,
                responsible_actor,
            },
        ) => {
            binding.institution.as_ref() == Some(institution)
                && binding
                    .actor
                    .is_none_or(|actor| Some(actor) == *responsible_actor)
        }
        (SeatPolicy::ObserverSeat | SeatPolicy::AdvisorSeat, origin) => {
            let actor_matches = binding.actor.is_none_or(
                |expected| matches!(origin, DecisionOrigin::Actor { actor } if *actor == expected),
            );
            let institution_matches = binding.institution.as_ref().is_none_or(|expected| {
                matches!(
                    origin,
                    DecisionOrigin::Institution { institution, .. } if institution == expected
                )
            });
            actor_matches && institution_matches
        }
        _ => false,
    }
}

const fn decision_actor(authority: &CommandAuthority) -> Option<PersonId> {
    match &authority.decision_origin {
        DecisionOrigin::Actor { actor } => Some(*actor),
        DecisionOrigin::Institution {
            responsible_actor, ..
        } => *responsible_actor,
        DecisionOrigin::Council { .. } | DecisionOrigin::NoResponsibleActor { .. } => None,
    }
}

const fn is_expected_command_rejection(code: &ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::ActorNotFound
            | ErrorCode::ArmyNotFound
            | ErrorCode::DestinationNotFound
            | ErrorCode::EntityNotFound
            | ErrorCode::IdempotencyConflict
            | ErrorCode::InteractionReadOnly
            | ErrorCode::InvalidAuthority
            | ErrorCode::InvalidDuration
            | ErrorCode::InvalidPayload
            | ErrorCode::MissingIdempotencyKey
            | ErrorCode::MixedCommandIngress
            | ErrorCode::NoRoute
            | ErrorCode::PluginCommandNotFound
            | ErrorCode::SimulationRevisionConflict
            | ErrorCode::SimulationTimeConflict
            | ErrorCode::ValueOutOfRange
    )
}

fn canonical_text(value: &str) -> bool {
    !value.is_empty() && value == value.trim()
}

fn component_key(
    plugin: &str,
    state: &StateKey,
    entity: &EntityRef,
    component: &str,
) -> PluginComponentKey {
    PluginComponentKey {
        plugin: plugin.to_owned(),
        state: state.clone(),
        entity: entity.clone(),
        component: component.to_owned(),
    }
}

fn record_change_affected_entities(change: &DomainRecordChange) -> Vec<EntityRef> {
    (change.current.class == DomainRecordClass::Entity)
        .then(|| EntityRef::Domain(change.current.reference.clone()))
        .into_iter()
        .collect()
}

fn is_domain_record_state(schemas: &records::DomainRecordSchemas, state: &StateKey) -> bool {
    schemas.contains_key(&DomainRecordKind::new(&state.namespace, &state.name))
}

fn snapshot_command_attempt_preflight_error(
    snapshot: &SimulationSnapshot,
    attempt: &CommandAttemptRecord,
    history: &DomainRecordHistory,
    cut: DomainHistoryCut,
) -> Option<CanwuError> {
    let authority = match resolve_command_authority(&attempt.envelope) {
        Ok(authority) => authority,
        Err(error) => return Some(error),
    };
    if let Err(error) = validate_command_ingress_policy(
        snapshot
            .run_configuration
            .as_ref()
            .expect("snapshot run configuration is validated before command attempts"),
        &attempt.envelope.issuer,
        &authority,
        CommandAdmission {
            request_id: attempt.request_id,
            expected_revision: attempt.expected_revision,
            expected_time: attempt.envelope.expected_time,
            revision_before: attempt.revision_before,
            ingress: attempt.ingress,
        },
        &|entity| snapshot_entity_exists_in_history(snapshot, history, cut, entity),
    ) {
        return Some(error);
    }
    attempt.envelope.expected_time.and_then(|expected_time| {
        (expected_time != attempt.at).then(|| {
            CanwuError::new(
                ErrorCode::SimulationTimeConflict,
                format!(
                    "command expected time {expected_time}, but simulation is at {}",
                    attempt.at
                ),
            )
        })
    })
}

fn validate_snapshot(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
) -> Result<(), CanwuError> {
    if snapshot.engine_version.trim().is_empty() {
        return invalid_snapshot("snapshot engine version cannot be empty");
    }
    if snapshot.revision_format_version != STATE_REVISION_FORMAT_VERSION {
        return invalid_snapshot("snapshot state revision format is not current");
    }
    if snapshot.replay_revision_format_version > STATE_REVISION_FORMAT_VERSION {
        return invalid_snapshot("snapshot exact-replay revision format is unsupported");
    }
    if snapshot.admission_cursor_format_version != ADMISSION_CURSOR_FORMAT_VERSION {
        return invalid_snapshot("snapshot boundary-admission cursor format is not current");
    }
    let Some(run_manifest) = &snapshot.run_manifest else {
        return Err(CanwuError::new(
            ErrorCode::InvalidRunManifest,
            "snapshot is missing its run manifest",
        ));
    };
    let has_domain_feature = !plugins.record_schemas.is_empty()
        || !snapshot.domain_records.is_empty()
        || snapshot
            .boundaries
            .iter()
            .any(|boundary| !boundary.record_changes.is_empty());
    let initial_domain_records = if let Some(initial_scenario) = &snapshot.initial_scenario {
        if !has_domain_feature {
            return invalid_snapshot(
                "record-free snapshots must omit the domain-record initial scenario",
            );
        }
        let mut canonical = initial_scenario.clone();
        canonicalize_scenario(&mut canonical);
        if &canonical != initial_scenario || initial_scenario.start_time != snapshot.initial_time {
            return invalid_snapshot("snapshot initial scenario is not canonical or time-aligned");
        }
        validate_scenario(initial_scenario).map_err(|error| {
            invalid_snapshot_error(format!("snapshot initial scenario is invalid: {error}"))
        })?;
        manifest::validate(run_manifest, Some(initial_scenario), true)?;
        Some(
            initial_scenario
                .domain_records
                .iter()
                .map(|record| (record.reference.clone(), record.clone()))
                .collect::<BTreeMap<_, _>>(),
        )
    } else {
        manifest::validate(run_manifest, None, true)?;
        if has_domain_feature {
            return invalid_snapshot(
                "domain-record snapshots require their manifest-bound initial scenario",
            );
        }
        None
    };
    if !is_canonical_hash(&snapshot.run_manifest_hash)
        || manifest::hash(run_manifest)? != snapshot.run_manifest_hash
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidRunManifest,
            "snapshot run manifest hash is inconsistent",
        ));
    }
    let Some(run_configuration) = &snapshot.run_configuration else {
        return Err(CanwuError::new(
            ErrorCode::InvalidRunConfiguration,
            "snapshot is missing its run configuration",
        ));
    };
    manifest::validate_run_configuration(run_manifest, run_configuration)?;
    let (configuration_world, configuration_records) = snapshot.initial_scenario.as_ref().map_or(
        (&snapshot.world, snapshot.domain_records.as_slice()),
        |scenario| (&scenario.world, scenario.domain_records.as_slice()),
    );
    validate_run_configuration_entities(
        run_configuration,
        configuration_world,
        configuration_records,
    )?;
    validate_run_configuration_entities(
        run_configuration,
        &snapshot.world,
        &snapshot.domain_records,
    )?;
    if matches!(run_configuration, RunConfigurationSnapshot::Declared(_))
        && !snapshot.commands.is_empty()
        && snapshot.command_attempts.is_empty()
    {
        return invalid_snapshot(
            "declared runs cannot contain accepted commands without tracked attempt evidence",
        );
    }
    if !snapshot.ingress.is_empty()
        && has_unqueued_command_history(
            &snapshot.commands,
            &snapshot.command_attempts,
            &snapshot.ingress,
        )
    {
        return invalid_snapshot("canonical ingress cannot coexist with direct command history");
    }
    if snapshot.initial_time > snapshot.now {
        return invalid_snapshot("snapshot initial time cannot follow its current time");
    }
    let has_execution_evidence = snapshot.now != snapshot.initial_time
        || !snapshot.commands.is_empty()
        || !snapshot.command_attempts.is_empty()
        || !snapshot.ingress.is_empty()
        || !snapshot.events.is_empty()
        || !snapshot.boundaries.is_empty()
        || !snapshot.plugin_components.is_empty()
        || !snapshot.random_draws.is_empty()
        || snapshot
            .random_streams
            .iter()
            .any(|stream| stream.position != 0)
        || !snapshot.scheduled.is_empty()
        || snapshot.next_event_id != 1
        || snapshot.next_command_id != 1
        || snapshot.next_command_attempt_id != 1
        || snapshot.next_ingress_id != 1
        || snapshot.next_boundary_id != 1
        || snapshot.next_random_draw_id != 1
        || snapshot.next_schedule_sequence != 1
        || snapshot.next_correlation_id != 1;
    if has_execution_evidence && !snapshot.plugin_registration_closed {
        return invalid_snapshot(
            "snapshot execution evidence requires plugin registration to remain closed",
        );
    }
    validate_strict_id_order(&snapshot.world.people, |value| value.id, "people")?;
    validate_strict_id_order(&snapshot.world.governments, |value| value.id, "governments")?;
    validate_strict_id_order(&snapshot.world.territories, |value| value.id, "territories")?;
    validate_strict_id_order(&snapshot.world.routes, |value| value.id, "routes")?;
    validate_strict_id_order(&snapshot.world.armies, |value| value.id, "armies")?;
    let domain_records = validate_snapshot_domain_records(snapshot, plugins)?;
    let (max_boundary_id, max_boundary_correlation, domain_history, admission_cursors) =
        validate_boundary_records(
            snapshot,
            plugins,
            &domain_records,
            initial_domain_records.as_ref(),
        )?;
    if snapshot.admitted_attempt_count != admission_cursors.attempts
        || snapshot.admitted_command_count != admission_cursors.commands
        || snapshot.admitted_event_count != admission_cursors.events
    {
        return invalid_snapshot(
            "persisted admission cursors do not match the globally admitted journal prefixes",
        );
    }
    let max_ingress_id = validate_ingress_records(snapshot, plugins, &domain_history)?;
    let boundaries_before_attempt =
        boundaries_before_attempts(snapshot.command_attempts.len(), &snapshot.boundaries)?;
    let mut request_ids = BTreeSet::new();
    let mut accepted_attempts = BTreeMap::new();
    let mut command_boundary_counts = BTreeMap::new();
    let mut accepted_command_count = 0_u64;
    let mut previous_attempt = None;
    for (index, attempt) in snapshot.command_attempts.iter().enumerate() {
        let expected_id = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| {
                invalid_snapshot_error("command attempt index exceeds identifier space")
            })?;
        let expected_revision = u64::try_from(index)
            .map_err(|_| invalid_snapshot_error("command attempt index exceeds revision space"))?
            .checked_add(boundaries_before_attempt[index])
            .ok_or_else(|| invalid_snapshot_error("command revision space is exhausted"))?;
        if attempt.id.get() != expected_id
            || attempt.at < snapshot.initial_time
            || attempt.at > snapshot.now
            || attempt.revision_before != expected_revision
            || attempt.request_id.is_some() != attempt.expected_revision.is_some()
            || previous_attempt.is_some_and(|(at, id)| (attempt.at, attempt.id) <= (at, id))
            || attempt.request_id.is_some_and(|id| !request_ids.insert(id))
        {
            return invalid_snapshot("command attempt journal is not canonical");
        }
        let boundaries_before =
            usize::try_from(boundaries_before_attempt[index]).map_err(|_| {
                invalid_snapshot_error("command attempt boundary cut exceeds platform index space")
            })?;
        let attempt_cut = DomainHistoryCut::after_boundaries(boundaries_before);
        let preflight_error = snapshot_command_attempt_preflight_error(
            snapshot,
            attempt,
            &domain_history,
            attempt_cut,
        );
        match &attempt.outcome {
            CommandAttemptOutcome::Accepted { command_id } => {
                if preflight_error.is_some() {
                    return invalid_snapshot(
                        "accepted command attempt violates its recorded ingress policy",
                    );
                }
                let Some(next_command_count) = accepted_command_count.checked_add(1) else {
                    return invalid_snapshot("command identifier space is exhausted");
                };
                if command_id.get() != next_command_count
                    || attempt
                        .expected_revision
                        .is_some_and(|expected| expected != expected_revision)
                    || accepted_attempts.insert(*command_id, attempt).is_some()
                    || command_boundary_counts
                        .insert(*command_id, boundaries_before)
                        .is_some()
                {
                    return invalid_snapshot(
                        "accepted command attempt does not match command revision order",
                    );
                }
                accepted_command_count = next_command_count;
            }
            CommandAttemptOutcome::Rejected { error } => {
                if !is_expected_command_rejection(&error.code) {
                    return invalid_snapshot(
                        "command attempt journal contains a non-rejection engine failure",
                    );
                }
                if preflight_error
                    .as_ref()
                    .is_some_and(|expected| expected != error)
                {
                    return invalid_snapshot(
                        "rejected command attempt disagrees with deterministic ingress validation",
                    );
                }
            }
        }
        previous_attempt = Some((attempt.at, attempt.id));
    }
    if !snapshot.command_attempts.is_empty()
        && accepted_command_count
            != u64::try_from(snapshot.commands.len())
                .map_err(|_| invalid_snapshot_error("command count exceeds the revision range"))?
    {
        return invalid_snapshot("accepted command attempts do not cover the command journal");
    }
    let mut command_ids = BTreeSet::new();
    let mut previous_command = None;
    for (index, record) in snapshot.commands.iter().enumerate() {
        let expected_id = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| invalid_snapshot_error("command index exceeds identifier space"))?;
        if record.id.get() != expected_id || !command_ids.insert(record.id) {
            return invalid_snapshot("command IDs must be contiguous, unique, and nonzero");
        }
        if record.accepted_at < snapshot.initial_time
            || record.accepted_at > snapshot.now
            || record
                .envelope
                .expected_time
                .is_some_and(|expected| expected != record.accepted_at)
        {
            return invalid_snapshot("command timestamps are invalid");
        }
        if snapshot.command_attempts.is_empty() {
            if record.attempt_id.is_some() || !record.emitted_events.is_empty() {
                return invalid_snapshot(
                    "legacy commands cannot contain partial command-attempt evidence",
                );
            }
        } else {
            let Some(attempt) = accepted_attempts.get(&record.id) else {
                return invalid_snapshot("command is missing its accepted attempt evidence");
            };
            if record.attempt_id != Some(attempt.id)
                || record.accepted_at != attempt.at
                || record.envelope != attempt.envelope
                || record
                    .emitted_events
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
            {
                return invalid_snapshot("command and attempt evidence disagree");
            }
        }
        if previous_command.is_some_and(|(time, id)| (record.accepted_at, record.id) <= (time, id))
        {
            return invalid_snapshot("command records are not in canonical order");
        }
        let boundaries_before = command_boundary_counts
            .get(&record.id)
            .copied()
            .unwrap_or_else(|| boundaries_before_legacy_command(snapshot, record));
        let command_cut = DomainHistoryCut::after_boundaries(boundaries_before);
        validate_snapshot_command(
            snapshot,
            plugins,
            &record.envelope,
            &domain_history,
            command_cut,
        )?;
        previous_command = Some((record.accepted_at, record.id));
    }

    let mut event_ids = BTreeSet::new();
    let mut previous_event = None;
    for (index, event) in snapshot.events.iter().enumerate() {
        let expected_id = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| invalid_snapshot_error("event index exceeds identifier space"))?;
        if event.id.get() != expected_id || event.correlation_id == 0 || !event_ids.insert(event.id)
        {
            return invalid_snapshot("event IDs must be contiguous, unique, and nonzero");
        }
        if event.timestamp < snapshot.initial_time
            || event.timestamp > snapshot.now
            || previous_event.is_some_and(|(time, id)| (event.timestamp, event.id) <= (time, id))
        {
            return invalid_snapshot("events are not in canonical timestamp and ID order");
        }
        let event_entities = if matches!(event.cause, Some(CauseRef::Boundary(_))) {
            None
        } else if let Some(command_id) = event_command_root(&snapshot.events, event.id)
            .ok()
            .flatten()
        {
            let command = snapshot
                .commands
                .iter()
                .find(|command| command.id == command_id)
                .ok_or_else(|| {
                    invalid_snapshot_error("event references an unknown command root")
                })?;
            if event.timestamp == command.accepted_at {
                let boundaries_before =
                    if let Some(count) = command_boundary_counts.get(&command_id) {
                        *count
                    } else {
                        boundaries_before_legacy_command(snapshot, command)
                    };
                Some(DomainHistoryCut::after_boundaries(boundaries_before))
            } else {
                Some(DomainRecordHistory::before_time(snapshot, event.timestamp))
            }
        } else {
            Some(DomainRecordHistory::before_time(snapshot, event.timestamp))
        };
        let entity_exists = |entity: &EntityRef| {
            event_entities.map_or_else(
                || snapshot_entity_identity_exists(snapshot, entity),
                |cut| snapshot_entity_exists_in_history(snapshot, &domain_history, cut, entity),
            )
        };
        if event
            .affected_entities
            .iter()
            .any(|entity| !entity_exists(entity))
        {
            return invalid_snapshot("event references an unknown entity");
        }
        validate_event_kind(snapshot, plugins, event, &entity_exists)?;
        previous_event = Some((event.timestamp, event.id));
    }
    for event in &snapshot.events {
        match &event.cause {
            Some(CauseRef::Boundary(id)) => {
                let Some(boundary) = snapshot.boundaries.iter().find(|record| record.id == *id)
                else {
                    return invalid_snapshot("event references an unknown boundary cause");
                };
                if boundary.at != event.timestamp
                    || !boundary
                        .emissions
                        .iter()
                        .any(|emission| emission.event == event.id)
                {
                    return invalid_snapshot("event boundary cause does not own the event");
                }
            }
            Some(CauseRef::Command(id)) => {
                let Some(command) = snapshot.commands.iter().find(|record| record.id == *id) else {
                    return invalid_snapshot("event references an unknown command cause");
                };
                if command.accepted_at > event.timestamp {
                    return invalid_snapshot("event references a future command cause");
                }
            }
            Some(CauseRef::Event(id)) if !event_ids.contains(id) || id.get() >= event.id.get() => {
                return invalid_snapshot("event references an invalid parent event");
            }
            Some(CauseRef::System(name)) if name.trim().is_empty() => {
                return invalid_snapshot("event system cause cannot be empty");
            }
            Some(CauseRef::Event(_) | CauseRef::System(_)) | None => {}
        }
    }
    for command in &snapshot.commands {
        if snapshot.command_attempts.is_empty() {
            continue;
        }
        let mut expected_events = Vec::new();
        for event in snapshot
            .events
            .iter()
            .filter(|event| event.timestamp == command.accepted_at)
        {
            if event_command_root(&snapshot.events, event.id)? == Some(command.id) {
                expected_events.push(event.id);
            }
        }
        if command.emitted_events != expected_events {
            return invalid_snapshot(
                "command receipt events do not match their synchronous causal evidence",
            );
        }
    }
    let (max_random_draw_id, max_random_correlation) = validate_random_evidence(snapshot, plugins)?;
    let current_state_hash = snapshot_state_hash(snapshot)?;
    if snapshot.commitment_format_version == COMMITMENT_FORMAT_VERSION {
        let persisted_roots = snapshot.commitment_roots.as_ref().ok_or_else(|| {
            invalid_snapshot_error("current commitment snapshot is missing its domain roots")
        })?;
        if !commitment_roots_are_canonical(persisted_roots)
            || snapshot_commitment_roots(snapshot)? != *persisted_roots
        {
            return invalid_snapshot(
                "persisted commitment roots do not match the canonical snapshot domains",
            );
        }
    } else if snapshot.commitment_format_version != 0 || snapshot.commitment_roots.is_some() {
        return invalid_snapshot("snapshot commitment metadata is inconsistent");
    }
    let expected_checkpoint_hash = snapshot_checkpoint_hash(snapshot)?;
    if !is_canonical_hash(&snapshot.checkpoint_hash)
        || expected_checkpoint_hash != snapshot.checkpoint_hash
    {
        return invalid_snapshot(
            "checkpoint hash does not bind the persisted state to its boundary head",
        );
    }
    if matches!(snapshot.run_manifest, Some(RunManifest::Declared { .. }))
        && snapshot_is_at_boundary_head(snapshot)
        && snapshot
            .boundaries
            .last()
            .and_then(|record| record.state_hash.as_deref())
            != Some(current_state_hash.as_str())
    {
        return invalid_snapshot("boundary-head state commitment does not match persisted state");
    }
    let mut component_keys = BTreeSet::new();
    let mut previous_component = None;
    for record in &snapshot.plugin_components {
        if record.plugin.trim().is_empty()
            || record.component.trim().is_empty()
            || !plugins.descriptors.contains_key(&record.plugin)
            || !snapshot_entity_exists(snapshot, &record.entity)
            || plugins.state_owners.get(&record.state) != Some(&record.plugin)
            || is_domain_record_state(&plugins.record_schemas, &record.state)
            || (!plugins.immediate_write_states.contains_key(&record.state)
                && !plugins
                    .boundary_writers
                    .keys()
                    .any(|(_, state)| state == &record.state))
        {
            return invalid_snapshot("plugin component record is not owned or well formed");
        }
        let key = component_key(
            &record.plugin,
            &record.state,
            &record.entity,
            &record.component,
        );
        if previous_component
            .as_ref()
            .is_some_and(|previous| previous >= &key)
            || !component_keys.insert(key.clone())
        {
            return invalid_snapshot("snapshot contains duplicate plugin component records");
        }
        previous_component = Some(key);
    }

    let core_schema = base_schema();
    for required in core_schema.iter() {
        if snapshot.schema.get(&required.type_name) != Some(required) {
            return invalid_snapshot("snapshot is missing an exact core schema definition");
        }
    }
    let mut declared_plugin_schema = BTreeSet::new();
    for descriptor in plugins.descriptors.values() {
        for type_name in &descriptor.schema_types {
            if snapshot.schema.get(type_name).is_none() {
                return invalid_snapshot("plugin descriptor references a missing schema type");
            }
            declared_plugin_schema.insert(type_name.as_str());
        }
    }
    for schema in snapshot.schema.iter() {
        validate_type_schema(schema).map_err(|error| {
            invalid_snapshot_error(format!("snapshot schema is invalid: {error}"))
        })?;
    }
    if snapshot.schema.iter().any(|schema| {
        core_schema.get(&schema.type_name).is_none()
            && !declared_plugin_schema.contains(schema.type_name.as_str())
    }) {
        return invalid_snapshot("snapshot contains an unclaimed schema definition");
    }

    let mut schedule_keys = BTreeSet::new();
    let mut previous_schedule = None;
    let mut pending_arrivals = BTreeMap::<ArmyId, usize>::new();
    let mut pending_reports = BTreeSet::new();
    let mut max_schedule_sequence = 0;
    let mut max_correlation_id = snapshot
        .events
        .iter()
        .map(|event| event.correlation_id)
        .max()
        .unwrap_or(0)
        .max(max_boundary_correlation)
        .max(max_random_correlation);
    for record in &snapshot.scheduled {
        if record.key.at <= snapshot.now
            || record.key.sequence == 0
            || previous_schedule
                .as_ref()
                .is_some_and(|previous| previous >= &record.key)
            || !schedule_keys.insert(record.key.clone())
        {
            return invalid_snapshot("scheduled work is not future-dated or has a duplicate key");
        }
        previous_schedule = Some(record.key.clone());
        max_schedule_sequence = max_schedule_sequence.max(record.key.sequence);
        let correlation_id = scheduled_correlation_id(&record.action);
        if correlation_id == 0 {
            return invalid_snapshot("scheduled work correlation IDs must be nonzero");
        }
        max_correlation_id = max_correlation_id.max(correlation_id);
        match &record.action {
            ScheduledAction::ArmyArrival { army, .. } => {
                *pending_arrivals.entry(*army).or_default() += 1;
            }
            ScheduledAction::KnowledgeReport { dispatch_event, .. } => {
                if !pending_reports.insert(*dispatch_event) {
                    return invalid_snapshot(
                        "multiple pending reports reference the same dispatch event",
                    );
                }
            }
            ScheduledAction::PluginDirective { .. } => {}
        }
        validate_scheduled_action(snapshot, plugins, &event_ids, &record.key, &record.action)?;
    }
    for army in &snapshot.world.armies {
        let pending = pending_arrivals.get(&army.id).copied().unwrap_or(0);
        if (army.transit.is_some() && pending != 1) || (army.transit.is_none() && pending != 0) {
            return invalid_snapshot(
                "army transit state must have exactly one matching pending arrival",
            );
        }
    }
    for dispatch in snapshot
        .events
        .iter()
        .filter(|event| matches!(event.kind, EventKind::ReportDispatched { .. }))
    {
        let EventKind::ReportDispatched {
            recipient,
            army,
            arrives_at,
        } = dispatch.kind
        else {
            unreachable!("the iterator selected report dispatch events");
        };
        let Some(CauseRef::Event(arrival_id)) = dispatch.cause else {
            return invalid_snapshot("report dispatch must be caused by an army arrival");
        };
        let Some(arrival) = snapshot.events.iter().find(|event| event.id == arrival_id) else {
            return invalid_snapshot("report dispatch references a missing army arrival");
        };
        let EventKind::ArmyArrived {
            army: arrived_army,
            territory: arrived_location,
        } = arrival.kind
        else {
            return invalid_snapshot("report dispatch cause is not an army arrival event");
        };
        if arrived_army != army
            || arrival.timestamp != dispatch.timestamp
            || arrival.correlation_id != dispatch.correlation_id
        {
            return invalid_snapshot("report dispatch disagrees with its army arrival cause");
        }
        let delivery_events: Vec<_> = snapshot
            .events
            .iter()
            .filter(|event| {
                event.cause == Some(CauseRef::Event(dispatch.id))
                    && matches!(event.kind, EventKind::KnowledgeUpdated { .. })
            })
            .collect();
        if delivery_events.iter().any(|event| {
            !matches!(
                event.kind,
                EventKind::KnowledgeUpdated {
                    recipient: delivered_recipient,
                    army: delivered_army,
                    known_location,
                } if delivered_recipient == recipient
                    && delivered_army == army
                    && known_location == arrived_location
                    && event.timestamp == arrives_at
                    && event.correlation_id == dispatch.correlation_id
            )
        }) {
            return invalid_snapshot("report delivery disagrees with its dispatch event");
        }
        let deliveries = delivery_events.len();
        let pending = pending_reports.contains(&dispatch.id);
        let coherent = match arrives_at.cmp(&snapshot.now) {
            std::cmp::Ordering::Greater => pending && deliveries == 0,
            std::cmp::Ordering::Less => !pending && deliveries == 1,
            std::cmp::Ordering::Equal => usize::from(pending) + deliveries == 1,
        };
        if !coherent {
            return invalid_snapshot(
                "report dispatch must have exactly one pending or completed delivery",
            );
        }
    }

    validate_contiguous_or_exhausted_next_counter(
        snapshot.next_event_id,
        snapshot
            .events
            .iter()
            .map(|event| event.id.get())
            .max()
            .unwrap_or(0),
        "event",
    )?;
    validate_contiguous_or_exhausted_next_counter(
        snapshot.next_command_id,
        snapshot
            .commands
            .iter()
            .map(|command| command.id.get())
            .max()
            .unwrap_or(0),
        "command",
    )?;
    validate_contiguous_or_exhausted_next_counter(
        snapshot.next_command_attempt_id,
        snapshot
            .command_attempts
            .last()
            .map_or(0, |attempt| attempt.id.get()),
        "command attempt",
    )?;
    validate_contiguous_or_exhausted_next_counter(
        snapshot.next_ingress_id,
        max_ingress_id,
        "ingress",
    )?;
    validate_contiguous_next_counter(snapshot.next_boundary_id, max_boundary_id, "boundary")?;
    validate_contiguous_or_exhausted_next_counter(
        snapshot.next_random_draw_id,
        max_random_draw_id,
        "random draw",
    )?;
    validate_next_counter(
        snapshot.next_schedule_sequence,
        max_schedule_sequence,
        "schedule sequence",
    )?;
    let authoritative_commit_count = u64::try_from(snapshot.commands.len())
        .ok()
        .and_then(|commands| {
            u64::try_from(snapshot.boundaries.len())
                .ok()
                .and_then(|boundaries| commands.checked_add(boundaries))
        })
        .ok_or_else(|| {
            invalid_snapshot_error("authoritative commit count exceeds revision space")
        })?;
    validate_contiguous_next_counter(
        snapshot.next_correlation_id,
        authoritative_commit_count,
        "correlation",
    )?;
    if max_correlation_id > authoritative_commit_count {
        return invalid_snapshot("causal evidence references an uncommitted correlation");
    }
    let expected_state_revision = authoritative_revision_count(
        snapshot.commands.len(),
        snapshot.command_attempts.len(),
        snapshot.boundaries.len(),
    )?;
    if snapshot.state_revision != expected_state_revision {
        return invalid_snapshot(
            "persisted state revision does not match committed command, rejection, and boundary evidence",
        );
    }
    Ok(())
}

fn validate_random_evidence(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
) -> Result<(u64, u64), CanwuError> {
    if snapshot.legacy_rng.is_some() {
        return invalid_snapshot("current snapshots cannot retain the legacy global RNG");
    }
    if snapshot
        .random_streams
        .windows(2)
        .any(|pair| pair[0].key >= pair[1].key)
    {
        return invalid_snapshot("random streams are not in canonical order");
    }
    let expected_streams: BTreeSet<_> = std::iter::once(random::core_report_delay_stream())
        .chain(plugins.random_stream_owners.keys().cloned())
        .collect();
    let actual_streams: BTreeSet<_> = snapshot
        .random_streams
        .iter()
        .map(|state| state.key.clone())
        .collect();
    if actual_streams != expected_streams
        || snapshot
            .random_streams
            .iter()
            .any(|state| !state.is_coherent(snapshot.root_seed))
    {
        return invalid_snapshot("random stream state or ownership is inconsistent");
    }

    let mut boundary_draws = BTreeMap::new();
    for boundary in &snapshot.boundaries {
        if boundary
            .random_draws
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return invalid_snapshot("boundary random draw IDs are not canonical");
        }
        for id in &boundary.random_draws {
            if boundary_draws.insert(*id, boundary.id).is_some()
                || snapshot
                    .random_draws
                    .get(usize::try_from(id.get().saturating_sub(1)).unwrap_or(usize::MAX))
                    .is_none_or(|draw| draw.id != *id)
            {
                return invalid_snapshot("boundary references an unknown or duplicate random draw");
            }
        }
    }

    let mut replayed: BTreeMap<_, _> = snapshot
        .random_streams
        .iter()
        .map(|state| (state.key.clone(), (0_u64, state.seed)))
        .collect();
    let mut previous_draw = None;
    let mut max_correlation_id = 0;
    let core_stream = random::core_report_delay_stream();
    let mut report_draws = BTreeMap::new();
    for (index, draw) in snapshot.random_draws.iter().enumerate() {
        let expected_id = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| invalid_snapshot_error("random draw index exceeds identifier space"))?;
        if draw.id.get() != expected_id
            || draw.at < snapshot.initial_time
            || draw.at > snapshot.now
            || draw.correlation_id == 0
            || draw.upper_exclusive == 0
            || draw.value >= draw.upper_exclusive
            || draw.purpose.trim().is_empty()
            || draw.purpose != draw.purpose.trim()
            || previous_draw.is_some_and(|(at, id)| (draw.at, draw.id) <= (at, id))
        {
            return invalid_snapshot("random draw journal is not canonical");
        }
        let Some((position, generator_state)) = replayed.get_mut(&draw.stream) else {
            return invalid_snapshot("random draw references an unknown stream");
        };
        if draw.position != *position {
            return invalid_snapshot("random draw positions are not contiguous per stream");
        }
        let mut generator = DeterministicRng::from_seed(*generator_state);
        if generator.range(draw.upper_exclusive) != draw.value {
            return invalid_snapshot("random draw value does not match its stream state");
        }
        *position = position.checked_add(1).ok_or_else(|| {
            invalid_snapshot_error("random stream position exceeds identifier space")
        })?;
        *generator_state = generator.state();

        match &draw.producer {
            RandomDrawProducer::BoundarySystem {
                boundary,
                plugin,
                system,
            } => {
                let Some(record) = snapshot
                    .boundaries
                    .iter()
                    .find(|record| record.id == *boundary)
                else {
                    return invalid_snapshot("random draw references an unknown boundary");
                };
                let Some(contract) = snapshot_boundary_contract(plugins, plugin, system) else {
                    return invalid_snapshot("random draw references an unknown boundary system");
                };
                if boundary_draws.get(&draw.id) != Some(boundary)
                    || draw.at != record.at
                    || draw.correlation_id != record.correlation_id
                    || draw.cause != CauseRef::Boundary(*boundary)
                    || draw.outcome != Some(RandomDrawOutcome::BoundarySystemDecision)
                    || !contract.random_streams.contains(&draw.stream)
                    || !boundary_system_due(
                        contract,
                        &record.cadences,
                        boundary_has_event_ingress(record),
                    )
                    || plugins.random_stream_owners.get(&draw.stream)
                        != Some(&(plugin.clone(), system.clone()))
                {
                    return invalid_snapshot("boundary random draw provenance is inconsistent");
                }
            }
            RandomDrawProducer::CoreSystem { system } => {
                let CauseRef::Event(cause) = draw.cause else {
                    return invalid_snapshot("core random draw lacks an event cause");
                };
                let Some(event) = snapshot.events.iter().find(|event| event.id == cause) else {
                    return invalid_snapshot("core random draw references an unknown event");
                };
                let EventKind::ArmyArrived {
                    army: arrived_army, ..
                } = event.kind
                else {
                    return invalid_snapshot("core random draw cause is not an army arrival");
                };
                let Some(RandomDrawOutcome::KnowledgeReportDelivery {
                    recipient,
                    army,
                    dispatch_event,
                    arrives_at,
                }) = &draw.outcome
                else {
                    return invalid_snapshot("core random draw lacks report-delivery evidence");
                };
                let Some(dispatch) = snapshot
                    .events
                    .iter()
                    .find(|candidate| candidate.id == *dispatch_event)
                else {
                    return invalid_snapshot("core random draw outcome references a missing event");
                };
                let expected_arrives_at = draw
                    .at
                    .checked_add(SimDuration::hours(36))
                    .and_then(|time| {
                        i64::try_from(draw.value)
                            .ok()
                            .and_then(|value| time.checked_add(SimDuration::minutes(value)))
                    })
                    .ok_or_else(|| {
                        invalid_snapshot_error("core random draw value exceeds time range")
                    })?;
                if boundary_draws.contains_key(&draw.id)
                    || system != "canwu.core.knowledge-report-delay"
                    || draw.stream != core_stream
                    || draw.upper_exclusive != 12 * 60
                    || draw.purpose != "knowledge report delivery jitter"
                    || draw.at != event.timestamp
                    || draw.correlation_id != event.correlation_id
                    || *army != arrived_army
                    || *arrives_at != expected_arrives_at
                    || dispatch.timestamp != draw.at
                    || dispatch.correlation_id != draw.correlation_id
                    || dispatch.cause != Some(CauseRef::Event(cause))
                    || !matches!(
                        dispatch.kind,
                        EventKind::ReportDispatched {
                            recipient: dispatch_recipient,
                            army: dispatch_army,
                            arrives_at: dispatch_arrives,
                        } if dispatch_recipient == *recipient
                            && dispatch_army == *army
                            && dispatch_arrives == *arrives_at
                    )
                {
                    return invalid_snapshot("core random draw provenance is inconsistent");
                }
                if report_draws.insert(*dispatch_event, draw.id).is_some() {
                    return invalid_snapshot(
                        "report dispatch is backed by more than one core random draw",
                    );
                }
            }
        }
        max_correlation_id = max_correlation_id.max(draw.correlation_id);
        previous_draw = Some((draw.at, draw.id));
    }

    for state in &snapshot.random_streams {
        if replayed.get(&state.key) != Some(&(state.position, state.generator_state)) {
            return invalid_snapshot("random draw journal does not reproduce stream state");
        }
    }
    for event in &snapshot.events {
        if matches!(event.kind, EventKind::ReportDispatched { .. })
            && !report_draws.contains_key(&event.id)
        {
            return invalid_snapshot(
                "report dispatch must be backed by exactly one core random draw",
            );
        }
    }
    Ok((
        snapshot.random_draws.last().map_or(0, |draw| draw.id.get()),
        max_correlation_id,
    ))
}

fn validate_snapshot_domain_records(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
) -> Result<BTreeMap<DomainRecordRef, DomainRecord>, CanwuError> {
    if snapshot
        .domain_records
        .windows(2)
        .any(|pair| pair[0].reference >= pair[1].reference)
    {
        return invalid_snapshot("domain records are not in canonical stable-reference order");
    }
    let records: BTreeMap<_, _> = snapshot
        .domain_records
        .iter()
        .map(|record| (record.reference.clone(), record.clone()))
        .collect();
    if records.len() != snapshot.domain_records.len() {
        return invalid_snapshot("snapshot contains duplicate domain record references");
    }
    records::validate_record_store(&records, &plugins.record_schemas, snapshot.now, &|entity| {
        core_world_entity_exists(&snapshot.world, entity)
    })
    .map_err(|error| {
        invalid_snapshot_error(format!("snapshot domain-record state is invalid: {error}"))
    })?;
    Ok(records)
}

fn validate_ingress_records(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    history: &DomainRecordHistory,
) -> Result<u64, CanwuError> {
    let boundary_count = u64::try_from(snapshot.boundaries.len())
        .map_err(|_| invalid_snapshot_error("boundary count exceeds the ingress journal range"))?;
    let mut generated_by_boundary = BTreeMap::new();
    for boundary in &snapshot.boundaries {
        if boundary
            .generated_ingress
            .windows(2)
            .any(|pair| pair[0].ingress >= pair[1].ingress)
        {
            return invalid_snapshot(
                "boundary-generated ingress evidence is not in canonical identifier order",
            );
        }
        for generation in &boundary.generated_ingress {
            if generated_by_boundary
                .insert(generation.ingress, boundary.id)
                .is_some()
            {
                return invalid_snapshot(
                    "ingress is claimed as generated by more than one boundary",
                );
            }
            let index =
                usize::try_from(generation.ingress.get().saturating_sub(1)).map_err(|_| {
                    invalid_snapshot_error(
                        "boundary-generated ingress ID exceeds the platform index range",
                    )
                })?;
            let Some(record) = snapshot.ingress.get(index) else {
                return invalid_snapshot(
                    "boundary-generated ingress evidence references an unknown record",
                );
            };
            if record.id != generation.ingress
                || record.issued_at != boundary.at
                || record.eligible_boundary_count != boundary.id.get()
                || record.cause != Some(CauseRef::Boundary(boundary.id))
                || !matches!(&record.payload, IngressPayload::Plugin { .. })
            {
                return invalid_snapshot(
                    "boundary-generated ingress evidence disagrees with its record",
                );
            }
        }
    }
    let mut previous_issue = None;
    let mut command_request_ids = BTreeSet::new();
    for (index, record) in snapshot.ingress.iter().enumerate() {
        let expected_id = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| invalid_snapshot_error("ingress index exceeds identifier space"))?;
        if record.id.get() != expected_id
            || record.issued_at < snapshot.initial_time
            || record.issued_at > snapshot.now
            || record.due_at < record.issued_at
            || record.eligible_boundary_count > boundary_count
            || previous_issue.is_some_and(|previous| {
                (record.eligible_boundary_count, record.issued_at, record.id) <= previous
            })
        {
            return invalid_snapshot("ingress journal identity, time, or issue cut is invalid");
        }
        validate_snapshot_ingress_cause(snapshot, record)?;
        let issue_boundary_count =
            usize::try_from(record.eligible_boundary_count).map_err(|_| {
                invalid_snapshot_error("ingress issue cut exceeds the platform index range")
            })?;
        if issue_boundary_count > 0
            && record.issued_at < snapshot.boundaries[issue_boundary_count - 1].at
        {
            return invalid_snapshot("ingress predates its declared eligibility boundary");
        }
        let issue_cut = DomainHistoryCut::after_boundaries(issue_boundary_count);
        match &record.payload {
            IngressPayload::Command { request } => {
                if record.class != IngressClass::Command
                    || request.request_id.get() == 0
                    || !command_request_ids.insert(request.request_id)
                    || request
                        .envelope
                        .expected_time
                        .is_some_and(|expected| expected != record.due_at)
                    || record.cause.is_some()
                {
                    return invalid_snapshot("queued command ingress is not canonical");
                }
            }
            IngressPayload::Plugin {
                plugin,
                packet_type,
                payload,
                affected_entities,
            } => {
                let Some(descriptor) = plugins.ingress.get(&(plugin.clone(), packet_type.clone()))
                else {
                    return invalid_snapshot("plugin ingress references an undeclared packet type");
                };
                if record.class != descriptor.class
                    || affected_entities.windows(2).any(|pair| pair[0] >= pair[1])
                    || affected_entities.iter().any(|entity| {
                        !snapshot_entity_identity_exists_in_history(
                            snapshot, history, issue_cut, entity,
                        )
                    })
                {
                    return invalid_snapshot("plugin ingress class or entity evidence is invalid");
                }
                match &record.cause {
                    Some(CauseRef::Boundary(boundary))
                        if generated_by_boundary.get(&record.id) == Some(boundary) => {}
                    Some(CauseRef::Boundary(_)) => {
                        return invalid_snapshot(
                            "boundary-caused ingress lacks matching generation evidence",
                        );
                    }
                    Some(CauseRef::Command(_) | CauseRef::Event(_)) => {
                        return invalid_snapshot(
                            "command- and event-generated ingress is not supported by this snapshot format",
                        );
                    }
                    Some(CauseRef::System(_)) | None
                        if !generated_by_boundary.contains_key(&record.id) => {}
                    Some(CauseRef::System(_)) | None => {
                        return invalid_snapshot(
                            "external ingress is incorrectly claimed as boundary-generated",
                        );
                    }
                }
                if snapshot
                    .run_configuration
                    .as_ref()
                    .and_then(RunConfigurationSnapshot::declared)
                    .is_some_and(|configuration| {
                        configuration.interaction == InteractionPolicy::ReadOnly
                    })
                    && !matches!(&record.cause, Some(CauseRef::Boundary(_)))
                {
                    return invalid_snapshot(
                        "declared read-only runs cannot contain newly authored plugin ingress",
                    );
                }
                descriptor
                    .payload_schema
                    .validate(payload)
                    .map_err(|error| {
                        invalid_snapshot_error(format!(
                            "plugin ingress payload is invalid: {error}"
                        ))
                    })?;
            }
            IngressPayload::Calendar { cadences } => {
                if record.class != IngressClass::ScheduledSystem
                    || record.priority != 0
                    || cadences.is_empty()
                    || cadences.contains(&SystemCadence::EventDriven)
                    || cadences.windows(2).any(|pair| pair[0] >= pair[1])
                    || record.cause != Some(CauseRef::System("canwu.core.calendar".to_owned()))
                {
                    return invalid_snapshot("calendar ingress is not canonical");
                }
            }
        }
        previous_issue = Some((record.eligible_boundary_count, record.issued_at, record.id));
    }

    let attempts_by_request: BTreeMap<_, _> = snapshot
        .command_attempts
        .iter()
        .filter_map(|attempt| attempt.request_id.map(|request| (request, attempt)))
        .collect();
    let mut pending = BTreeSet::new();
    let mut cursor = 0;
    for (boundary_index, boundary) in snapshot.boundaries.iter().enumerate() {
        let available_after = u64::try_from(boundary_index)
            .map_err(|_| invalid_snapshot_error("boundary index exceeds ingress range"))?;
        while let Some(record) = snapshot.ingress.get(cursor)
            && record.eligible_boundary_count <= available_after
        {
            if record.issued_at > boundary.at {
                return invalid_snapshot("ingress is assigned to a boundary before it was issued");
            }
            pending.insert(IngressQueueKey::from_record(record));
            cursor += 1;
        }
        if pending.first().is_some_and(|key| key.due_at < boundary.at) {
            return invalid_snapshot("a boundary steps past earlier canonical ingress");
        }
        let expected: Vec<_> = pending
            .iter()
            .take_while(|key| key.due_at <= boundary.at)
            .map(|key| key.id)
            .collect();
        if boundary.admitted_ingress != expected {
            return invalid_snapshot(
                "boundary ingress admission does not match the canonical due queue",
            );
        }
        let mut expected_attempts = Vec::new();
        let mut expected_commands = Vec::new();
        let mut expected_cadences = BTreeSet::new();
        for ingress_id in expected {
            let index = usize::try_from(ingress_id.get().saturating_sub(1)).map_err(|_| {
                invalid_snapshot_error("admitted ingress ID exceeds the platform index range")
            })?;
            let record = &snapshot.ingress[index];
            if let IngressPayload::Command { request } = &record.payload {
                let Some(attempt) = attempts_by_request.get(&request.request_id) else {
                    return invalid_snapshot(
                        "admitted command ingress is missing its deterministic attempt outcome",
                    );
                };
                if attempt.at != boundary.at
                    || attempt.envelope != request.envelope
                    || attempt.expected_revision != Some(request.expected_revision)
                    || attempt.ingress != CommandIngress::LiveRequest
                {
                    return invalid_snapshot(
                        "command ingress, attempt outcome, and boundary admission disagree",
                    );
                }
                expected_attempts.push(attempt.id);
                if let CommandAttemptOutcome::Accepted { command_id } = attempt.outcome {
                    expected_commands.push(command_id);
                }
            } else if let IngressPayload::Calendar { cadences } = &record.payload {
                expected_cadences.extend(cadences.iter().cloned());
            }
            pending.remove(&IngressQueueKey::from_record(record));
        }
        if !snapshot.ingress.is_empty()
            && (boundary.admitted_attempts != expected_attempts
                || boundary.admitted_commands != expected_commands
                || expected_cadences
                    .iter()
                    .any(|cadence| !boundary.cadences.contains(cadence)))
        {
            return invalid_snapshot(
                "boundary command or calendar effects do not match admitted ingress order",
            );
        }
    }
    for record in &snapshot.ingress[cursor..] {
        if record.eligible_boundary_count != boundary_count {
            return invalid_snapshot("ingress issue cuts skip a completed boundary");
        }
        pending.insert(IngressQueueKey::from_record(record));
    }
    if pending.iter().any(|key| key.due_at < snapshot.now) {
        return invalid_snapshot("snapshot retains ingress overdue before committed time");
    }
    for key in &pending {
        let index = usize::try_from(key.id.get().saturating_sub(1)).map_err(|_| {
            invalid_snapshot_error("pending ingress ID exceeds the platform index range")
        })?;
        if let IngressPayload::Command { request } = &snapshot.ingress[index].payload
            && attempts_by_request.contains_key(&request.request_id)
        {
            return invalid_snapshot("pending command ingress already has an attempt outcome");
        }
    }
    Ok(snapshot.ingress.last().map_or(0, |record| record.id.get()))
}

fn validate_snapshot_ingress_cause(
    snapshot: &SimulationSnapshot,
    record: &IngressRecord,
) -> Result<(), CanwuError> {
    let valid = match &record.cause {
        None => true,
        Some(CauseRef::Boundary(id)) => {
            journal_record_by_id(&snapshot.boundaries, id.get(), |value| value.id.get())
                .is_some_and(|boundary| {
                    boundary.id == *id
                        && boundary.at <= record.issued_at
                        && id.get() <= record.eligible_boundary_count
                })
        }
        Some(CauseRef::Command(id)) => {
            journal_record_by_id(&snapshot.commands, id.get(), |value| value.id.get())
                .is_some_and(|command| command.accepted_at <= record.issued_at)
        }
        Some(CauseRef::Event(id)) => {
            journal_record_by_id(&snapshot.events, id.get(), |value| value.id.get())
                .is_some_and(|event| event.timestamp <= record.issued_at)
        }
        Some(CauseRef::System(name)) => canonical_text(name),
    };
    if valid {
        Ok(())
    } else {
        invalid_snapshot("ingress cause references unavailable or future evidence")
    }
}

fn journal_record_by_id<T>(
    records: &[T],
    id: u64,
    record_id: impl FnOnce(&T) -> u64,
) -> Option<&T> {
    let index = usize::try_from(id.checked_sub(1)?).ok()?;
    let record = records.get(index)?;
    (record_id(record) == id).then_some(record)
}

fn validate_boundary_records(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    final_domain_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    initial_domain_records: Option<&BTreeMap<DomainRecordRef, DomainRecord>>,
) -> Result<(u64, u64, DomainRecordHistory, PersistedAdmissionCursors), CanwuError> {
    let mut boundary_ids = BTreeSet::new();
    let mut emitted_events = BTreeSet::new();
    let mut boundary_correlations = BTreeSet::new();
    let mut boundary_values = BTreeMap::new();
    let mut domain_record_values = final_domain_records.clone();
    for boundary in snapshot.boundaries.iter().rev() {
        for change in boundary.record_changes.iter().rev() {
            let reference = &change.current.reference;
            if domain_record_values.get(reference) != Some(&change.current) {
                return invalid_snapshot(
                    "boundary domain-record history does not match its persisted successor",
                );
            }
            if let Some(previous) = &change.previous {
                domain_record_values.insert(reference.clone(), previous.clone());
            } else {
                domain_record_values.remove(reference);
            }
        }
    }
    let empty_initial_records = BTreeMap::new();
    let expected_initial_records = initial_domain_records.unwrap_or(&empty_initial_records);
    if &domain_record_values != expected_initial_records {
        return invalid_snapshot(
            "boundary domain-record history does not match the manifest-bound initial scenario",
        );
    }
    let initial_world = snapshot
        .initial_scenario
        .as_ref()
        .map_or(&snapshot.world, |scenario| &scenario.world);
    records::validate_record_store(
        &domain_record_values,
        &plugins.record_schemas,
        snapshot.initial_time,
        &|entity| core_world_entity_exists(initial_world, entity),
    )
    .map_err(|error| {
        invalid_snapshot_error(format!(
            "initial domain-record state reconstructed from boundary evidence is invalid: {error}"
        ))
    })?;
    let mut next_attempt = 0;
    let mut next_command = 0;
    let mut next_event = 0;
    let mut previous_boundary = None;
    let mut previous_hash = GENESIS_BOUNDARY_HASH.to_owned();
    let mut max_boundary_id = 0;
    let mut max_correlation_id = 0;
    let mut history = DomainRecordHistory::from_initial_records(&domain_record_values);
    let requires_state_hash = matches!(snapshot.run_manifest, Some(RunManifest::Declared { .. }));

    for (index, record) in snapshot.boundaries.iter().enumerate() {
        let expected_id = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| invalid_snapshot_error("boundary index exceeds identifier space"))?;
        if record.id.get() != expected_id || !boundary_ids.insert(record.id) {
            return invalid_snapshot("boundary IDs must be contiguous, unique, and nonzero");
        }
        if record.at < snapshot.initial_time
            || record.at > snapshot.now
            || previous_boundary.is_some_and(|(at, id)| (record.at, record.id) <= (at, id))
            || record.correlation_id == 0
            || !boundary_correlations.insert(record.correlation_id)
        {
            return invalid_snapshot("boundary time, order, or correlation is invalid");
        }
        if record.cadences.contains(&SystemCadence::EventDriven)
            || record.cadences.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return invalid_snapshot("boundary cadences are not canonical");
        }
        validate_boundary_admission(
            record,
            snapshot,
            &mut next_attempt,
            &mut next_command,
            &mut next_event,
        )?;
        let cuts =
            validate_boundary_record_changes(record, snapshot, plugins, &mut domain_record_values)?;
        validate_boundary_ingress_generation(
            record,
            snapshot,
            plugins,
            &domain_record_values,
            &cuts,
        )?;
        validate_boundary_reservations(record, snapshot, plugins, &domain_record_values, &cuts)?;
        validate_boundary_changes(record, snapshot, plugins, &domain_record_values, &cuts)?;
        for change in &record.changes {
            let key = component_key(
                &change.plugin,
                &change.state,
                &change.entity,
                &change.component,
            );
            if change.previous.as_ref() != boundary_values.get(&key) {
                return invalid_snapshot("boundary change previous-value evidence is inconsistent");
            }
            boundary_values.insert(key, change.value.clone());
        }
        validate_boundary_emissions(
            record,
            snapshot,
            plugins,
            &domain_record_values,
            &cuts,
            &mut emitted_events,
        )?;
        if record.previous_hash != previous_hash
            || !is_canonical_hash(&record.hash)
            || (requires_state_hash && record.state_hash.is_none())
            || record
                .state_hash
                .as_deref()
                .is_some_and(|hash| !is_canonical_hash(hash))
            || compute_boundary_hash(record).map_err(|error| {
                invalid_snapshot_error(format!("could not verify boundary hash: {error}"))
            })? != record.hash
        {
            return invalid_snapshot("boundary hash chain is inconsistent");
        }

        max_boundary_id = record.id.get();
        max_correlation_id = max_correlation_id.max(record.correlation_id);
        previous_boundary = Some((record.at, record.id));
        previous_hash.clone_from(&record.hash);
        history.apply_boundary(index + 1, &cuts)?;
    }
    let boundary_states: BTreeSet<_> = plugins
        .boundary_writers
        .keys()
        .map(|(_, state)| state.clone())
        .collect();
    let persisted_boundary_values: BTreeMap<_, _> = snapshot
        .plugin_components
        .iter()
        .filter(|record| boundary_states.contains(&record.state))
        .map(|record| {
            (
                component_key(
                    &record.plugin,
                    &record.state,
                    &record.entity,
                    &record.component,
                ),
                record.value.clone(),
            )
        })
        .collect();
    if persisted_boundary_values != boundary_values {
        return invalid_snapshot(
            "boundary changes do not materialize the persisted component state",
        );
    }
    if &domain_record_values != final_domain_records {
        return invalid_snapshot(
            "boundary domain-record changes do not materialize the persisted record state",
        );
    }
    Ok((
        max_boundary_id,
        max_correlation_id,
        history,
        PersistedAdmissionCursors {
            attempts: u64::try_from(next_attempt).map_err(|_| {
                invalid_snapshot_error("admitted attempt cursor exceeds persisted range")
            })?,
            commands: u64::try_from(next_command).map_err(|_| {
                invalid_snapshot_error("admitted command cursor exceeds persisted range")
            })?,
            events: u64::try_from(next_event).map_err(|_| {
                invalid_snapshot_error("admitted event cursor exceeds persisted range")
            })?,
        },
    ))
}

fn validate_boundary_admission(
    record: &BoundaryRecord,
    snapshot: &SimulationSnapshot,
    next_attempt: &mut usize,
    next_command: &mut usize,
    next_event: &mut usize,
) -> Result<(), CanwuError> {
    if record
        .admitted_attempts
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
        || record
            .admitted_commands
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || record
            .admitted_events
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return invalid_snapshot("boundary admission lists are not canonical");
    }
    let mut accepted_attempt_commands = Vec::new();
    for id in &record.admitted_attempts {
        let Some(attempt) = snapshot.command_attempts.get(*next_attempt) else {
            return invalid_snapshot("boundary admits a command attempt beyond the journal prefix");
        };
        if attempt.id != *id || attempt.at > record.at {
            return invalid_snapshot(
                "boundary command-attempt admission is out of order or premature",
            );
        }
        if let CommandAttemptOutcome::Accepted { command_id } = attempt.outcome {
            accepted_attempt_commands.push(command_id);
        }
        *next_attempt += 1;
    }
    if snapshot
        .command_attempts
        .get(*next_attempt)
        .is_some_and(|attempt| attempt.at < record.at)
    {
        return invalid_snapshot(
            "boundary omitted an earlier command attempt from its admission cut",
        );
    }
    if !snapshot.command_attempts.is_empty()
        && accepted_attempt_commands != record.admitted_commands
    {
        return invalid_snapshot(
            "boundary command admission does not match its accepted attempt evidence",
        );
    }
    for id in &record.admitted_commands {
        let Some(command) = snapshot.commands.get(*next_command) else {
            return invalid_snapshot("boundary admits a command beyond the journal prefix");
        };
        if command.id != *id || command.accepted_at > record.at {
            return invalid_snapshot("boundary command admission is out of order or premature");
        }
        *next_command += 1;
    }
    if snapshot
        .commands
        .get(*next_command)
        .is_some_and(|command| command.accepted_at < record.at)
    {
        return invalid_snapshot("boundary omitted an earlier command from its admission cut");
    }

    for id in &record.admitted_events {
        let Some(event) = snapshot.events.get(*next_event) else {
            return invalid_snapshot("boundary admits an event beyond the journal prefix");
        };
        if event.id != *id || event.timestamp > record.at {
            return invalid_snapshot("boundary event admission is out of order or premature");
        }
        match &event.cause {
            Some(CauseRef::Boundary(boundary)) if *boundary >= record.id => {
                return invalid_snapshot("boundary admitted an event from its own or a later cut");
            }
            Some(CauseRef::Command(command))
                if usize::try_from(command.get())
                    .map_or(true, |command_number| command_number > *next_command) =>
            {
                return invalid_snapshot("boundary admitted an event before its command cause");
            }
            Some(CauseRef::Event(parent))
                if usize::try_from(parent.get())
                    .map_or(true, |event_number| event_number > *next_event) =>
            {
                return invalid_snapshot("boundary admitted an event before its parent cause");
            }
            Some(
                CauseRef::Boundary(_)
                | CauseRef::Command(_)
                | CauseRef::Event(_)
                | CauseRef::System(_),
            )
            | None => {}
        }
        *next_event += 1;
    }
    if let Some(event) = snapshot.events.get(*next_event) {
        let precedes_current_emission = record
            .emissions
            .first()
            .is_some_and(|emission| event.id < emission.event);
        let comes_from_earlier_boundary = matches!(
            &event.cause,
            Some(CauseRef::Boundary(boundary)) if *boundary < record.id
        );
        let comes_from_admitted_command = matches!(
            &event.cause,
            Some(CauseRef::Command(command))
                if usize::try_from(command.get())
                    .is_ok_and(|command_number| command_number <= *next_command)
        );
        let comes_from_admitted_parent = matches!(
            &event.cause,
            Some(CauseRef::Event(parent))
                if usize::try_from(parent.get())
                    .is_ok_and(|event_number| event_number <= *next_event)
        );
        if event.timestamp < record.at
            || (event.timestamp == record.at
                && (precedes_current_emission
                    || comes_from_earlier_boundary
                    || comes_from_admitted_command
                    || comes_from_admitted_parent))
        {
            return invalid_snapshot("boundary omitted an existing event from its admission cut");
        }
    }
    Ok(())
}

fn validate_boundary_reservations(
    record: &BoundaryRecord,
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    cuts: &BoundaryDomainEntityCuts,
) -> Result<(), CanwuError> {
    if record.reservation_offers.windows(2).any(|pair| {
        (&pair[0].offer.pool, &pair[0].plugin, &pair[0].system)
            >= (&pair[1].offer.pool, &pair[1].plugin, &pair[1].system)
    }) {
        return invalid_snapshot("boundary reservation offers are not canonical");
    }
    let mut remaining = BTreeMap::new();
    for offered in &record.reservation_offers {
        validate_snapshot_reservation_pool(&offered.offer.pool, snapshot, final_records, cuts)?;
        let Some(contract) = snapshot_boundary_contract(plugins, &offered.plugin, &offered.system)
        else {
            return invalid_snapshot("reservation offer references an unknown boundary system");
        };
        if contract.phase != BoundaryPhase::ReservationAndAllocation
            || !boundary_system_due(
                contract,
                &record.cadences,
                boundary_has_event_ingress(record),
            )
            || !contract
                .reservation_offers
                .contains(&offered.offer.pool.state)
            || plugins.reservation_offerers.get(&offered.offer.pool.state)
                != Some(&(offered.plugin.clone(), offered.system.clone()))
            || remaining
                .insert(offered.offer.pool.clone(), offered.offer.capacity)
                .is_some()
        {
            return invalid_snapshot("boundary reservation offer is unauthorized or duplicated");
        }
    }

    if record.reservation_requests.windows(2).any(|pair| {
        compare_reservation_request_records(&pair[0], &pair[1]) != std::cmp::Ordering::Less
    }) || record.allocations.len() != record.reservation_requests.len()
    {
        return invalid_snapshot("boundary reservation requests or allocations are not canonical");
    }
    let mut request_refs = BTreeSet::new();
    for (requested, allocation) in record.reservation_requests.iter().zip(&record.allocations) {
        validate_snapshot_reservation_pool(&requested.request.pool, snapshot, final_records, cuts)?;
        let Some(contract) = snapshot_boundary_contract(
            plugins,
            &requested.reservation.plugin,
            &requested.reservation.system,
        ) else {
            return invalid_snapshot("reservation request references an unknown boundary system");
        };
        if requested.reservation.request != requested.request.request
            || requested.request.request.trim().is_empty()
            || requested.request.request != requested.request.request.trim()
            || requested.request.tie_break.trim().is_empty()
            || requested.request.tie_break != requested.request.tie_break.trim()
            || requested.request.quantity == 0
            || contract.phase != BoundaryPhase::ReservationAndAllocation
            || !boundary_system_due(
                contract,
                &record.cadences,
                boundary_has_event_ingress(record),
            )
            || !contract
                .reservation_requests
                .contains(&requested.request.pool.state)
            || !request_refs.insert(requested.reservation.clone())
        {
            return invalid_snapshot("boundary reservation request is invalid");
        }
        let available = remaining.entry(requested.request.pool.clone()).or_default();
        let granted = requested.request.quantity.min(*available);
        *available -= granted;
        let disposition = if granted == requested.request.quantity {
            ReservationDisposition::Fulfilled
        } else if granted == 0 {
            ReservationDisposition::Rejected
        } else {
            ReservationDisposition::Partial
        };
        let expected = ReservationAllocation {
            reservation: requested.reservation.clone(),
            pool: requested.request.pool.clone(),
            requested: requested.request.quantity,
            granted,
            remaining_after: *available,
            disposition,
        };
        if allocation != &expected {
            return invalid_snapshot("boundary reservation allocation evidence is inconsistent");
        }
    }
    Ok(())
}

fn compare_reservation_request_records(
    left: &ReservationRequestRecord,
    right: &ReservationRequestRecord,
) -> std::cmp::Ordering {
    left.request
        .pool
        .cmp(&right.request.pool)
        .then_with(|| right.request.priority.cmp(&left.request.priority))
        .then_with(|| left.request.tie_break.cmp(&right.request.tie_break))
        .then_with(|| left.reservation.cmp(&right.reservation))
}

fn validate_snapshot_reservation_pool(
    pool: &ReservationPoolKey,
    snapshot: &SimulationSnapshot,
    final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    cuts: &BoundaryDomainEntityCuts,
) -> Result<(), CanwuError> {
    if pool.resource.trim().is_empty()
        || pool.resource != pool.resource.trim()
        || !snapshot_entity_exists_at_boundary(snapshot, final_records, cuts, None, &pool.entity)
    {
        return invalid_snapshot("snapshot contains an invalid reservation pool");
    }
    Ok(())
}

fn snapshot_boundary_contract<'a>(
    plugins: &'a PluginRegistry,
    plugin: &str,
    system: &str,
) -> Option<&'a BoundarySystemContract> {
    plugins
        .descriptors
        .get(plugin)?
        .boundary_systems
        .iter()
        .find(|contract| contract.name == system)
}

fn validate_boundary_ingress_generation(
    record: &BoundaryRecord,
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    cuts: &BoundaryDomainEntityCuts,
) -> Result<(), CanwuError> {
    for generation in &record.generated_ingress {
        let Some(contract) =
            snapshot_boundary_contract(plugins, &generation.plugin, &generation.system)
        else {
            return invalid_snapshot("generated ingress references an unknown boundary system");
        };
        let Some(commit_stage) = domain_record_commit_stage(contract.phase, contract.visibility)
        else {
            return invalid_snapshot("generated ingress has no deterministic commit stage");
        };
        let index = usize::try_from(generation.ingress.get().saturating_sub(1)).map_err(|_| {
            invalid_snapshot_error("generated ingress ID exceeds the journal index range")
        })?;
        let Some(ingress) = snapshot.ingress.get(index) else {
            return invalid_snapshot("generated ingress references an unknown ingress record");
        };
        let IngressPayload::Plugin {
            plugin,
            affected_entities,
            ..
        } = &ingress.payload
        else {
            return invalid_snapshot("boundary systems may generate only plugin ingress");
        };
        let generated_delay = ingress.due_at.checked_sub(ingress.issued_at);
        if generation.phase != contract.phase
            || generation.visibility != contract.visibility
            || plugin != &generation.plugin
            || ingress.id != generation.ingress
            || ingress.issued_at != record.at
            || ingress.eligible_boundary_count != record.id.get()
            || ingress.cause != Some(CauseRef::Boundary(record.id))
            || generated_delay.is_none_or(SimDuration::is_negative)
            || generated_delay.and_then(|delay| ingress.issued_at.checked_add(delay))
                != Some(ingress.due_at)
            || !boundary_system_due(
                contract,
                &record.cadences,
                boundary_has_event_ingress(record),
            )
            || affected_entities.iter().any(|entity| match entity {
                EntityRef::Domain(reference) => !cuts.identity_exists_for_proposal(
                    final_records,
                    reference,
                    contract.phase,
                    commit_stage,
                    &generation.plugin,
                    &generation.system,
                ),
                _ => !core_world_entity_exists(&snapshot.world, entity),
            })
        {
            return invalid_snapshot(
                "generated ingress producer, commit stage, or entity provenance is inconsistent",
            );
        }
    }
    Ok(())
}

fn validate_boundary_changes(
    record: &BoundaryRecord,
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    cuts: &BoundaryDomainEntityCuts,
) -> Result<(), CanwuError> {
    let mut change_keys = BTreeSet::new();
    for change in &record.changes {
        let Some(contract) = snapshot_boundary_contract(plugins, &change.plugin, &change.system)
        else {
            return invalid_snapshot("boundary change references an unknown system");
        };
        let Some(stage) = boundary_write_stage(contract.phase) else {
            return invalid_snapshot("boundary change references a non-writing phase");
        };
        let Some(commit_stage) = domain_record_commit_stage(contract.phase, change.visibility)
        else {
            return invalid_snapshot("boundary change has no deterministic commit stage");
        };
        if change.component.trim().is_empty()
            || change.component != change.component.trim()
            || !snapshot_entity_exists_for_boundary_proposal(
                snapshot,
                final_records,
                cuts,
                contract,
                commit_stage,
                (&change.plugin, &change.system),
                &change.entity,
            )
            || !contract.writes.contains(&change.state)
            || !boundary_system_due(
                contract,
                &record.cadences,
                boundary_has_event_ingress(record),
            )
            || contract.visibility != change.visibility
            || plugins.state_owners.get(&change.state) != Some(&change.plugin)
            || plugins.boundary_writers.get(&(stage, change.state.clone()))
                != Some(&(change.plugin.clone(), change.system.clone()))
            || !change_keys.insert((
                change.plugin.clone(),
                change.system.clone(),
                change.state.clone(),
                change.entity.clone(),
                change.component.clone(),
            ))
        {
            return invalid_snapshot("boundary change is unauthorized or duplicated");
        }
    }
    Ok(())
}

fn validate_boundary_record_changes(
    record: &BoundaryRecord,
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    values: &mut BTreeMap<DomainRecordRef, DomainRecord>,
) -> Result<BoundaryDomainEntityCuts, CanwuError> {
    let mut by_stage = BTreeMap::<DomainRecordCommitStage, Vec<&DomainRecordChange>>::new();
    let mut previous_order = None;
    for change in &record.record_changes {
        let Some(contract) = snapshot_boundary_contract(plugins, &change.plugin, &change.system)
        else {
            return invalid_snapshot("domain-record change references an unknown boundary system");
        };
        let Some(write_stage) = boundary_write_stage(contract.phase) else {
            return invalid_snapshot("domain-record change references a non-writing phase");
        };
        let Some(commit_stage) = domain_record_commit_stage(contract.phase, change.visibility)
        else {
            return invalid_snapshot("domain-record change has no deterministic commit stage");
        };
        let reference = &change.current.reference;
        let state = records::record_state_key(&reference.kind);
        let order = (commit_stage, reference.clone());
        if !canonical_text(&change.summary)
            || previous_order
                .as_ref()
                .is_some_and(|previous| previous >= &order)
            || change
                .previous
                .as_ref()
                .is_some_and(|previous| previous.reference != *reference)
            || !contract.writes.contains(&state)
            || !boundary_system_due(
                contract,
                &record.cadences,
                boundary_has_event_ingress(record),
            )
            || contract.visibility != change.visibility
            || plugins.state_owners.get(&state) != Some(&change.plugin)
            || plugins.boundary_writers.get(&(write_stage, state.clone()))
                != Some(&(change.plugin.clone(), change.system.clone()))
            || plugins
                .record_schemas
                .get(&reference.kind)
                .is_none_or(|(owner, _)| owner != &change.plugin)
        {
            return invalid_snapshot(
                "boundary domain-record change is unauthorized, duplicated, or noncanonical",
            );
        }
        previous_order = Some(order);
        by_stage.entry(commit_stage).or_default().push(change);
    }

    let mut cuts = BoundaryDomainEntityCuts::default();
    for stage in DomainRecordCommitStage::ALL {
        if let Some(changes) = by_stage.get(&stage) {
            let mutations: Vec<_> = changes
                .iter()
                .map(|change| records::mutation_from_change(change))
                .collect();
            let requests: Vec<_> = changes
                .iter()
                .zip(&mutations)
                .map(|(change, mutation)| records::DomainMutationRequest {
                    plugin: &change.plugin,
                    system: &change.system,
                    visibility: change.visibility,
                    mutation,
                    summary: &change.summary,
                })
                .collect();
            let (next, applied) = records::apply_mutation_bundle(
                values,
                &plugins.record_schemas,
                record.at,
                &|entity| core_world_entity_exists(&snapshot.world, entity),
                requests,
            )
            .map_err(|error| {
                invalid_snapshot_error(format!(
                    "boundary domain-record transition is invalid: {error}"
                ))
            })?;
            let recorded: Vec<_> = changes.iter().map(|change| (*change).clone()).collect();
            if applied != recorded {
                return invalid_snapshot(
                    "boundary domain-record transition evidence disagrees with deterministic replay",
                );
            }
            for change in &applied {
                cuts.record(stage, change);
            }
            *values = next;
        }
    }
    Ok(cuts)
}

fn validate_boundary_emissions(
    record: &BoundaryRecord,
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    cuts: &BoundaryDomainEntityCuts,
    emitted_events: &mut BTreeSet<EventId>,
) -> Result<(), CanwuError> {
    if record
        .emissions
        .windows(2)
        .any(|pair| pair[0].event >= pair[1].event)
    {
        return invalid_snapshot("boundary emitted event IDs are not canonical");
    }
    let mut matched_changes = BTreeSet::new();
    let mut matched_record_changes = BTreeSet::new();
    for emission in &record.emissions {
        let Some(event) = snapshot
            .events
            .iter()
            .find(|event| event.id == emission.event)
        else {
            return invalid_snapshot("boundary references an unknown emitted event");
        };
        if event.timestamp != record.at
            || event.correlation_id != record.correlation_id
            || event.cause != Some(CauseRef::Boundary(record.id))
            || !emitted_events.insert(emission.event)
        {
            return invalid_snapshot("boundary emitted event evidence is inconsistent");
        }
        let EventKind::Plugin { plugin, event_type } = &event.kind else {
            return invalid_snapshot("boundary emitted a non-plugin event");
        };
        if plugin != &emission.plugin {
            return invalid_snapshot("boundary emission plugin provenance is inconsistent");
        }
        let Some(contract) =
            snapshot_boundary_contract(plugins, &emission.plugin, &emission.system)
        else {
            return invalid_snapshot("boundary emission references an unknown system");
        };
        if !boundary_system_due(
            contract,
            &record.cadences,
            boundary_has_event_ingress(record),
        ) {
            return invalid_snapshot("boundary emission source system was not due");
        }
        let Some(commit_stage) = domain_record_commit_stage(contract.phase, contract.visibility)
        else {
            return invalid_snapshot("boundary emission has no deterministic commit stage");
        };
        match emission.kind {
            BoundaryEmissionKind::Change { change_index } => {
                let index = usize::try_from(change_index).map_err(|_| {
                    invalid_snapshot_error("boundary change evidence index is out of range")
                })?;
                let Some(change) = record.changes.get(index) else {
                    return invalid_snapshot("boundary emission references an unknown change");
                };
                if !matched_changes.insert(change_index)
                    || emission.plugin != change.plugin
                    || emission.system != change.system
                    || event.summary != change.summary
                    || event.affected_entities != vec![change.entity.clone()]
                    || event_type != &format!("{}_changed", change.component)
                    || !snapshot_entity_exists_for_boundary_proposal(
                        snapshot,
                        final_records,
                        cuts,
                        contract,
                        commit_stage,
                        (&emission.plugin, &emission.system),
                        &change.entity,
                    )
                {
                    return invalid_snapshot("boundary change evidence provenance is inconsistent");
                }
            }
            BoundaryEmissionKind::RecordChange { change_index } => {
                let index = usize::try_from(change_index).map_err(|_| {
                    invalid_snapshot_error("boundary record-change evidence index is out of range")
                })?;
                let Some(change) = record.record_changes.get(index) else {
                    return invalid_snapshot(
                        "boundary emission references an unknown domain record change",
                    );
                };
                if !matched_record_changes.insert(change_index)
                    || emission.plugin != change.plugin
                    || emission.system != change.system
                    || event.summary != change.summary
                    || event.affected_entities != record_change_affected_entities(change)
                    || event_type != change.operation.event_type()
                {
                    return invalid_snapshot(
                        "boundary domain-record evidence provenance is inconsistent",
                    );
                }
            }
            BoundaryEmissionKind::Explicit => {
                if !contract.emits.contains(event_type)
                    || event.affected_entities.iter().any(|entity| {
                        !snapshot_entity_exists_for_boundary_proposal(
                            snapshot,
                            final_records,
                            cuts,
                            contract,
                            commit_stage,
                            (&emission.plugin, &emission.system),
                            entity,
                        )
                    })
                {
                    return invalid_snapshot(
                        "boundary explicit event is unauthorized or references unavailable state",
                    );
                }
            }
        }
    }
    if matched_changes.len() != record.changes.len() {
        return invalid_snapshot("boundary change is missing its emitted evidence event");
    }
    if matched_record_changes.len() != record.record_changes.len() {
        return invalid_snapshot(
            "boundary domain-record change is missing its emitted evidence event",
        );
    }
    Ok(())
}

fn boundaries_before_legacy_command(
    snapshot: &SimulationSnapshot,
    command: &CommandRecord,
) -> usize {
    snapshot
        .boundaries
        .iter()
        .position(|boundary| boundary.admitted_commands.contains(&command.id))
        .unwrap_or_else(|| {
            snapshot
                .boundaries
                .iter()
                .take_while(|boundary| boundary.at <= command.accepted_at)
                .count()
        })
}

fn validate_snapshot_command(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    envelope: &CommandEnvelope,
    history: &DomainRecordHistory,
    cut: DomainHistoryCut,
) -> Result<(), CanwuError> {
    match &envelope.issuer {
        Issuer::Actor(actor) if snapshot.world.person(*actor).is_none() => {
            return invalid_snapshot("command issuer actor is missing");
        }
        Issuer::System(name) if name.trim().is_empty() => {
            return invalid_snapshot("system command issuer cannot be empty");
        }
        Issuer::Human(name)
        | Issuer::Ai(name)
        | Issuer::Institution(name)
        | Issuer::Replay(name)
        | Issuer::Experiment(name)
            if !canonical_text(name) =>
        {
            return invalid_snapshot("typed command issuer ID is not canonical");
        }
        Issuer::Actor(_)
        | Issuer::Human(_)
        | Issuer::Ai(_)
        | Issuer::Institution(_)
        | Issuer::Replay(_)
        | Issuer::Experiment(_)
        | Issuer::Debug
        | Issuer::System(_) => {}
    }
    if let Some(authority) = &envelope.authority {
        validate_command_authority(authority, &|entity| {
            snapshot_entity_exists_in_history(snapshot, history, cut, entity)
        })
        .map_err(|error| {
            invalid_snapshot_error(format!("command authority is invalid: {error}"))
        })?;
    }
    match &envelope.command {
        Command::MoveArmy { army, destination } => {
            if snapshot.world.army(*army).is_none()
                || snapshot.world.territory(*destination).is_none()
            {
                return invalid_snapshot("move command references unknown entities");
            }
        }
        Command::DebugSetArmyMorale { army, morale } => {
            if snapshot.world.army(*army).is_none() || *morale > 100 {
                return invalid_snapshot("debug morale command is invalid");
            }
        }
        Command::Plugin {
            plugin,
            command,
            payload,
        } => {
            let Some(descriptor) = plugins.descriptors.get(plugin) else {
                return invalid_snapshot("plugin command references an unknown plugin");
            };
            let Some(action) = descriptor
                .commands
                .iter()
                .find(|candidate| candidate.name == *command)
            else {
                return invalid_snapshot("plugin command is absent from its manifest");
            };
            action.payload_schema.validate(payload).map_err(|error| {
                invalid_snapshot_error(format!("plugin command payload is invalid: {error}"))
            })?;
        }
    }
    Ok(())
}

fn event_command_root(
    events: &[SimEvent],
    mut event_id: EventId,
) -> Result<Option<CommandId>, CanwuError> {
    for _ in 0..events.len() {
        let index = event_id
            .get()
            .checked_sub(1)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| invalid_snapshot_error("event cause chain contains an invalid ID"))?;
        let event = events
            .get(index)
            .filter(|event| event.id == event_id)
            .ok_or_else(|| {
                invalid_snapshot_error("event cause chain references an unknown event")
            })?;
        match event.cause.as_ref() {
            Some(CauseRef::Command(command)) => return Ok(Some(*command)),
            Some(CauseRef::Event(parent)) => event_id = *parent,
            Some(CauseRef::Boundary(_) | CauseRef::System(_)) | None => return Ok(None),
        }
    }
    invalid_snapshot("event cause chain contains a cycle")
}

fn validate_event_kind(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    event: &SimEvent,
    entity_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(), CanwuError> {
    let valid = match &event.kind {
        EventKind::MoveOrdered {
            army,
            from,
            to,
            arrival_at,
        } => {
            snapshot.world.army(*army).is_some()
                && snapshot.world.territory(*from).is_some()
                && snapshot.world.territory(*to).is_some()
                && *arrival_at >= event.timestamp
        }
        EventKind::ArmyArrived { army, territory } => {
            snapshot.world.army(*army).is_some() && snapshot.world.territory(*territory).is_some()
        }
        EventKind::ReportDispatched {
            recipient,
            army,
            arrives_at,
        } => {
            snapshot.world.person(*recipient).is_some()
                && snapshot.world.army(*army).is_some()
                && *arrives_at >= event.timestamp
        }
        EventKind::KnowledgeUpdated {
            recipient,
            army,
            known_location,
        } => {
            snapshot.world.person(*recipient).is_some()
                && snapshot.world.army(*army).is_some()
                && snapshot.world.territory(*known_location).is_some()
        }
        EventKind::DebugFieldChanged { entity, .. } => entity_exists(entity),
        EventKind::Plugin { plugin, event_type } => {
            plugins.descriptors.contains_key(plugin) && !event_type.trim().is_empty()
        }
    };
    if valid {
        Ok(())
    } else {
        invalid_snapshot("event kind references invalid state")
    }
}

fn validate_scheduled_action(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    event_ids: &BTreeSet<EventId>,
    key: &ScheduleKey,
    action: &ScheduledAction,
) -> Result<(), CanwuError> {
    match action {
        ScheduledAction::ArmyArrival {
            army,
            destination,
            order_event,
            correlation_id,
        } => {
            let Some(army_state) = snapshot.world.army(*army) else {
                return invalid_snapshot("scheduled army arrival is invalid");
            };
            let Some(transit) = &army_state.transit else {
                return invalid_snapshot("scheduled arrival has no matching army transit");
            };
            let Some(order) = snapshot
                .events
                .iter()
                .find(|event| event.id == *order_event)
            else {
                return invalid_snapshot("scheduled arrival references an unknown order event");
            };
            let EventKind::MoveOrdered {
                army: ordered_army,
                from,
                to,
                arrival_at,
            } = &order.kind
            else {
                return invalid_snapshot("scheduled arrival does not reference a move order event");
            };
            let Some(CauseRef::Command(command_id)) = order.cause else {
                return invalid_snapshot("move order event does not reference its command");
            };
            let command_matches = snapshot.commands.iter().any(|record| {
                record.id == command_id
                    && record.accepted_at == order.timestamp
                    && matches!(
                        record.envelope.command,
                        Command::MoveArmy {
                            army: commanded_army,
                            destination: commanded_destination,
                        } if commanded_army == *army && commanded_destination == *destination
                    )
            });
            if !command_matches
                || *ordered_army != *army
                || *from != transit.from
                || *to != *destination
                || transit.to != *destination
                || *arrival_at != key.at
                || transit.arrives_at != key.at
                || order.timestamp != transit.departed_at
                || order.correlation_id != *correlation_id
            {
                return invalid_snapshot(
                    "scheduled arrival, transit, move command, and order event disagree",
                );
            }
        }
        ScheduledAction::KnowledgeReport {
            recipient,
            army,
            location,
            observed_at,
            dispatch_event,
            correlation_id,
        } => {
            if snapshot.world.person(*recipient).is_none()
                || snapshot.world.army(*army).is_none()
                || snapshot.world.territory(*location).is_none()
            {
                return invalid_snapshot("scheduled knowledge report is invalid");
            }
            let Some(dispatch) = snapshot
                .events
                .iter()
                .find(|event| event.id == *dispatch_event)
            else {
                return invalid_snapshot("scheduled report references an unknown dispatch event");
            };
            let EventKind::ReportDispatched {
                recipient: dispatched_recipient,
                army: dispatched_army,
                arrives_at,
            } = &dispatch.kind
            else {
                return invalid_snapshot(
                    "scheduled report does not reference a report dispatch event",
                );
            };
            let Some(CauseRef::Event(arrival_event_id)) = dispatch.cause else {
                return invalid_snapshot("report dispatch does not reference an arrival event");
            };
            let Some(arrival) = snapshot
                .events
                .iter()
                .find(|event| event.id == arrival_event_id)
            else {
                return invalid_snapshot("report dispatch references an unknown arrival event");
            };
            let EventKind::ArmyArrived {
                army: arrived_army,
                territory,
            } = &arrival.kind
            else {
                return invalid_snapshot("report dispatch cause is not an army arrival");
            };
            if *dispatched_recipient != *recipient
                || *dispatched_army != *army
                || *arrived_army != *army
                || *territory != *location
                || *arrives_at != key.at
                || dispatch.timestamp != arrival.timestamp
                || *observed_at != arrival.timestamp
                || dispatch.correlation_id != *correlation_id
                || arrival.correlation_id != *correlation_id
            {
                return invalid_snapshot(
                    "scheduled report, dispatch event, and arrival event disagree",
                );
            }
        }
        ScheduledAction::PluginDirective {
            plugin,
            directive,
            allowed_writes,
            cause,
            ..
        } => {
            let Some(descriptor) = plugins.descriptors.get(plugin) else {
                return invalid_snapshot("scheduled directive references an unknown plugin");
            };
            let mut canonical_writes = allowed_writes.clone();
            validate_state_keys(&mut canonical_writes).map_err(|error| {
                invalid_snapshot_error(format!(
                    "scheduled directive has invalid write declarations: {error}"
                ))
            })?;
            if canonical_writes != *allowed_writes
                || !descriptor
                    .commands
                    .iter()
                    .map(|action| &action.writes)
                    .chain(descriptor.systems.iter().map(|system| &system.writes))
                    .any(|writes| writes == allowed_writes)
            {
                return invalid_snapshot(
                    "scheduled directive write access does not match a plugin contract",
                );
            }
            match cause {
                CauseRef::Boundary(id)
                    if !snapshot.boundaries.iter().any(|record| record.id == *id) =>
                {
                    return invalid_snapshot("scheduled directive has an unknown boundary cause");
                }
                CauseRef::Command(id)
                    if !snapshot.commands.iter().any(|record| record.id == *id) =>
                {
                    return invalid_snapshot("scheduled directive has an unknown command cause");
                }
                CauseRef::Event(id) if !event_ids.contains(id) => {
                    return invalid_snapshot("scheduled directive has an unknown event cause");
                }
                CauseRef::System(name) if name.trim().is_empty() => {
                    return invalid_snapshot("scheduled directive has an empty system cause");
                }
                CauseRef::Boundary(_)
                | CauseRef::Command(_)
                | CauseRef::Event(_)
                | CauseRef::System(_) => {}
            }
            validate_directives(
                plugin,
                allowed_writes,
                &plugins.state_owners,
                &plugins.record_schemas,
                &|entity| snapshot_entity_exists(snapshot, entity),
                std::slice::from_ref(directive.as_ref()),
            )
            .map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    format!("scheduled plugin directive is invalid: {error}"),
                )
            })?;
        }
    }
    Ok(())
}

const fn scheduled_correlation_id(action: &ScheduledAction) -> u64 {
    match action {
        ScheduledAction::ArmyArrival { correlation_id, .. }
        | ScheduledAction::KnowledgeReport { correlation_id, .. }
        | ScheduledAction::PluginDirective { correlation_id, .. } => *correlation_id,
    }
}

fn validate_next_counter(next: u64, maximum_existing: u64, label: &str) -> Result<(), CanwuError> {
    if next == 0 || next <= maximum_existing {
        return invalid_snapshot(format!("next {label} counter is invalid"));
    }
    Ok(())
}

fn validate_contiguous_next_counter(
    next: u64,
    maximum_existing: u64,
    label: &str,
) -> Result<(), CanwuError> {
    let Some(expected) = maximum_existing.checked_add(1) else {
        return invalid_snapshot(format!("{label} identifier space is exhausted"));
    };
    if next != expected {
        return invalid_snapshot(format!("next {label} counter is not contiguous"));
    }
    Ok(())
}

fn validate_contiguous_or_exhausted_next_counter(
    next: u64,
    maximum_existing: u64,
    label: &str,
) -> Result<(), CanwuError> {
    if next == u64::MAX {
        return Ok(());
    }
    validate_contiguous_next_counter(next, maximum_existing, label)
}

fn claim_counter(current: u64, label: &str) -> Result<(u64, u64), CanwuError> {
    let Some(next) = current.checked_add(1) else {
        return Err(CanwuError::new(
            ErrorCode::IdentifierExhausted,
            format!("{label} space is exhausted"),
        ));
    };
    if current == 0 {
        return Err(CanwuError::new(
            ErrorCode::InvalidSnapshot,
            format!("next {label} counter cannot be zero"),
        ));
    }
    Ok((current, next))
}

fn validate_run_configuration_entities(
    run_configuration: &RunConfigurationSnapshot,
    world: &WorldSnapshot,
    domain_records: &[DomainRecord],
) -> Result<(), CanwuError> {
    let Some(binding) = run_configuration
        .declared()
        .and_then(|configuration| configuration.seat_binding.as_ref())
    else {
        return Ok(());
    };
    if binding
        .actor
        .is_some_and(|actor| world.person(actor).is_none())
        || binding
            .institution
            .as_ref()
            .is_some_and(|institution| !entity_exists_in_parts(world, domain_records, institution))
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidRunConfiguration,
            "run seat binding references an entity absent from the scenario or snapshot",
        ));
    }
    Ok(())
}

fn core_world_entity_exists(world: &WorldSnapshot, entity: &EntityRef) -> bool {
    match entity {
        EntityRef::Army(id) => world.army(*id).is_some(),
        EntityRef::Government(id) => world.government(*id).is_some(),
        EntityRef::Person(id) => world.person(*id).is_some(),
        EntityRef::Route(id) => world.route(*id).is_some(),
        EntityRef::Territory(id) => world.territory(*id).is_some(),
        EntityRef::Domain(_) | EntityRef::Organization(_) | EntityRef::Resource(_) => false,
    }
}

fn entity_exists_in_parts(
    world: &WorldSnapshot,
    domain_records: &[DomainRecord],
    entity: &EntityRef,
) -> bool {
    match entity {
        EntityRef::Domain(reference) => domain_records.iter().any(|record| {
            &record.reference == reference
                && record.class == DomainRecordClass::Entity
                && !record.is_deleted()
        }),
        _ => core_world_entity_exists(world, entity),
    }
}

fn snapshot_entity_exists(snapshot: &SimulationSnapshot, entity: &EntityRef) -> bool {
    entity_exists_in_parts(&snapshot.world, &snapshot.domain_records, entity)
}

fn snapshot_entity_exists_in_history(
    snapshot: &SimulationSnapshot,
    history: &DomainRecordHistory,
    cut: DomainHistoryCut,
    entity: &EntityRef,
) -> bool {
    match entity {
        EntityRef::Domain(reference) => history.is_live(reference, cut),
        _ => core_world_entity_exists(&snapshot.world, entity),
    }
}

fn snapshot_entity_identity_exists_in_history(
    snapshot: &SimulationSnapshot,
    history: &DomainRecordHistory,
    cut: DomainHistoryCut,
    entity: &EntityRef,
) -> bool {
    match entity {
        EntityRef::Domain(reference) => history.exists(reference, cut),
        _ => core_world_entity_exists(&snapshot.world, entity),
    }
}

fn snapshot_entity_exists_at_boundary(
    snapshot: &SimulationSnapshot,
    final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    cuts: &BoundaryDomainEntityCuts,
    stage: Option<DomainRecordCommitStage>,
    entity: &EntityRef,
) -> bool {
    match entity {
        EntityRef::Domain(reference) => cuts.is_live(final_records, reference, stage),
        _ => core_world_entity_exists(&snapshot.world, entity),
    }
}

fn snapshot_entity_exists_for_boundary_proposal(
    snapshot: &SimulationSnapshot,
    final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    cuts: &BoundaryDomainEntityCuts,
    contract: &BoundarySystemContract,
    commit_stage: DomainRecordCommitStage,
    source: (&str, &str),
    entity: &EntityRef,
) -> bool {
    match entity {
        EntityRef::Domain(reference) => {
            cuts.is_live(final_records, reference, Some(commit_stage))
                && cuts.is_live_for_proposal(
                    final_records,
                    reference,
                    contract.phase,
                    commit_stage,
                    source.0,
                    source.1,
                )
        }
        _ => core_world_entity_exists(&snapshot.world, entity),
    }
}

fn snapshot_entity_identity_exists(snapshot: &SimulationSnapshot, entity: &EntityRef) -> bool {
    match entity {
        EntityRef::Domain(reference) => snapshot.domain_records.iter().any(|record| {
            &record.reference == reference && record.class == DomainRecordClass::Entity
        }),
        _ => core_world_entity_exists(&snapshot.world, entity),
    }
}

fn runtime_entity_exists(state: &RuntimeState, entity: &EntityRef) -> bool {
    match entity {
        EntityRef::Army(id) => state.current.armies.contains_key(id),
        EntityRef::Domain(reference) => {
            records::domain_entity_exists(&state.current.domain_records, reference)
        }
        EntityRef::Government(id) => state.current.governments.contains_key(id),
        EntityRef::Person(id) => state.current.people.contains_key(id),
        EntityRef::Route(id) => state.current.routes.contains_key(id),
        EntityRef::Territory(id) => state.current.territories.contains_key(id),
        EntityRef::Organization(_) | EntityRef::Resource(_) => false,
    }
}

fn runtime_entity_identity_exists(state: &RuntimeState, entity: &EntityRef) -> bool {
    match entity {
        EntityRef::Domain(reference) => state
            .current
            .domain_records
            .get(reference)
            .is_some_and(|record| record.class == DomainRecordClass::Entity),
        _ => runtime_entity_exists(state, entity),
    }
}

fn runtime_has_unqueued_command_history(state: &RuntimeState) -> bool {
    has_unqueued_command_history(
        &state.evidence.commands,
        &state.evidence.command_attempts,
        &state.evidence.ingress,
    )
}

fn has_unqueued_command_history(
    commands: &[CommandRecord],
    attempts: &[CommandAttemptRecord],
    ingress: &[IngressRecord],
) -> bool {
    let queued_requests: BTreeSet<_> = ingress
        .iter()
        .filter_map(|record| match &record.payload {
            IngressPayload::Command { request } => Some(request.request_id),
            IngressPayload::Plugin { .. } | IngressPayload::Calendar { .. } => None,
        })
        .collect();
    commands.iter().any(|command| command.attempt_id.is_none())
        || attempts.iter().any(|attempt| {
            attempt
                .request_id
                .is_none_or(|request| !queued_requests.contains(&request))
        })
}

fn validate_runtime_cause(state: &RuntimeState, cause: &CauseRef) -> Result<(), CanwuError> {
    let valid = match cause {
        CauseRef::Boundary(id) => state
            .evidence
            .boundaries
            .iter()
            .any(|record| record.id == *id),
        CauseRef::Command(id) => state
            .evidence
            .commands
            .iter()
            .any(|record| record.id == *id),
        CauseRef::Event(id) => state.evidence.events.iter().any(|event| event.id == *id),
        CauseRef::System(name) => canonical_text(name),
    };
    if valid {
        Ok(())
    } else {
        Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            "ingress cause does not reference canonical committed evidence",
        ))
    }
}

fn runtime_entity_exists_with_record_overlay(
    state: &RuntimeState,
    record_overlay: &BTreeMap<DomainRecordRef, DomainRecord>,
    entity: &EntityRef,
) -> bool {
    match entity {
        EntityRef::Domain(reference) => record_overlay
            .get(reference)
            .or_else(|| state.current.domain_records.get(reference))
            .is_some_and(|record| {
                record.class == DomainRecordClass::Entity && !record.is_deleted()
            }),
        _ => runtime_entity_exists(state, entity),
    }
}

fn proposal_entity_exists(
    state: &RuntimeState,
    schemas: &records::DomainRecordSchemas,
    record_overlay: &BTreeMap<DomainRecordRef, DomainRecord>,
    proposal: &BoundaryProposal,
    entity: &EntityRef,
) -> bool {
    let EntityRef::Domain(reference) = entity else {
        return runtime_entity_exists(state, entity);
    };
    if let Some(mutation) = proposal.directives.iter().rev().find_map(|directive| {
        let BoundaryDirective::MutateRecord { mutation, .. } = directive else {
            return None;
        };
        (mutation.target() == reference).then_some(mutation)
    }) {
        return match mutation {
            DomainRecordMutation::Delete { .. } => false,
            DomainRecordMutation::Create { .. }
            | DomainRecordMutation::Update { .. }
            | DomainRecordMutation::Retire { .. } => schemas
                .get(&reference.kind)
                .is_some_and(|(_, schema)| schema.class == DomainRecordClass::Entity),
        };
    }
    runtime_entity_exists_with_record_overlay(state, record_overlay, entity)
}

fn proposal_entity_identity_exists(
    state: &RuntimeState,
    schemas: &records::DomainRecordSchemas,
    proposal: &BoundaryProposal,
    entity: &EntityRef,
) -> bool {
    let EntityRef::Domain(reference) = entity else {
        return runtime_entity_identity_exists(state, entity);
    };
    if state
        .current
        .domain_records
        .get(reference)
        .is_some_and(|record| record.class == DomainRecordClass::Entity)
    {
        return true;
    }
    proposal.directives.iter().any(|directive| {
        let BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Create { record },
            ..
        } = directive
        else {
            return false;
        };
        &record.reference == reference
            && schemas
                .get(&reference.kind)
                .is_some_and(|(_, schema)| schema.class == DomainRecordClass::Entity)
    })
}

fn validate_runtime_domain_dependents(state: &RuntimeState) -> Result<(), CanwuError> {
    validate_domain_dependents_with_records(state, &state.current.domain_records)
}

fn validate_domain_dependents_with_records(
    state: &RuntimeState,
    domain_records: &BTreeMap<DomainRecordRef, DomainRecord>,
) -> Result<(), CanwuError> {
    let unavailable = |entity: &EntityRef| matches!(entity, EntityRef::Domain(reference) if !records::domain_entity_exists(domain_records, reference));
    if state
        .current
        .plugin_components
        .values()
        .any(|component| unavailable(&component.entity))
    {
        return Err(CanwuError::new(
            ErrorCode::DomainRecordReferenced,
            "a domain entity with persisted plugin components cannot be deleted",
        ));
    }
    if state.scheduler.actions.values().any(|action| match action {
        ScheduledAction::PluginDirective { directive, .. } => {
            system_directive_has_entity(directive, &unavailable)
        }
        ScheduledAction::ArmyArrival { .. } | ScheduledAction::KnowledgeReport { .. } => false,
    }) {
        return Err(CanwuError::new(
            ErrorCode::DomainRecordReferenced,
            "a domain entity referenced by future scheduled work cannot be deleted",
        ));
    }
    if state
        .metadata
        .run_configuration
        .declared()
        .and_then(|configuration| configuration.seat_binding.as_ref())
        .and_then(|binding| binding.institution.as_ref())
        .is_some_and(unavailable)
    {
        return Err(CanwuError::new(
            ErrorCode::DomainRecordReferenced,
            "the institution bound to the active run seat cannot be deleted",
        ));
    }
    Ok(())
}

fn system_directive_has_entity(
    directive: &SystemDirective,
    predicate: &dyn Fn(&EntityRef) -> bool,
) -> bool {
    match directive {
        SystemDirective::SetComponent { entity, .. } => predicate(entity),
        SystemDirective::Emit { affected, .. } => affected.iter().any(predicate),
        SystemDirective::Schedule { directive, .. } => {
            system_directive_has_entity(directive, predicate)
        }
    }
}

fn migrate_snapshot(mut snapshot: SimulationSnapshot) -> Result<SimulationSnapshot, CanwuError> {
    match snapshot.snapshot_format_version {
        SNAPSHOT_FORMAT_VERSION => {
            if snapshot.engine_version != ENGINE_VERSION {
                return Err(CanwuError::new(
                    ErrorCode::UnsupportedSnapshotVersion,
                    format!(
                        "snapshot format {} from engine {} requires an explicit migration to engine {}",
                        snapshot.snapshot_format_version, snapshot.engine_version, ENGINE_VERSION
                    ),
                ));
            }
            if snapshot.legacy_rng.is_some() {
                return invalid_snapshot("format 4 snapshots cannot contain the legacy global RNG");
            }
            hydrate_snapshot_run_configuration(&mut snapshot)?;
            migrate_snapshot_revision(&mut snapshot)?;
            migrate_snapshot_admission_cursors(&mut snapshot)?;
            migrate_snapshot_commitments(&mut snapshot)?;
            Ok(snapshot)
        }
        2 => {
            if snapshot.run_manifest.is_some()
                || !snapshot.run_manifest_hash.is_empty()
                || !snapshot.checkpoint_hash.is_empty()
                || snapshot.commitment_format_version != 0
                || snapshot.commitment_roots.is_some()
            {
                return invalid_snapshot(
                    "format 2 snapshots cannot contain current manifest or commitment data",
                );
            }
            validate_legacy_ingress_shape(&snapshot)?;
            let checkpoint_hash = canonical_hash("canwu.legacy-checkpoint.v1", &snapshot)?;
            if !snapshot.boundaries.is_empty() || !matches!(snapshot.next_boundary_id, 0 | 1) {
                return invalid_snapshot("format 2 snapshots cannot contain phased-boundary state");
            }
            snapshot.boundaries.clear();
            snapshot.next_boundary_id = 1;
            migrate_format_3_snapshot(snapshot, 2, checkpoint_hash)
        }
        3 => {
            if snapshot.run_manifest.is_some()
                || !snapshot.run_manifest_hash.is_empty()
                || !snapshot.checkpoint_hash.is_empty()
                || snapshot.commitment_format_version != 0
                || snapshot.commitment_roots.is_some()
            {
                return invalid_snapshot(
                    "format 3 snapshots cannot contain current manifest or commitment data",
                );
            }
            validate_legacy_ingress_shape(&snapshot)?;
            let checkpoint_hash = canonical_hash("canwu.legacy-checkpoint.v1", &snapshot)?;
            migrate_format_3_snapshot(snapshot, 3, checkpoint_hash)
        }
        _ => Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "snapshot format {} from engine {} is unsupported; this engine reads formats 2, 3, and {}",
                snapshot.snapshot_format_version, snapshot.engine_version, SNAPSHOT_FORMAT_VERSION
            ),
        )),
    }
}

fn migrate_snapshot_revision(snapshot: &mut SimulationSnapshot) -> Result<(), CanwuError> {
    match snapshot.revision_format_version {
        STATE_REVISION_FORMAT_VERSION => Ok(()),
        0 => {
            if snapshot.state_revision != 0 || snapshot.replay_revision_format_version != 0 {
                return invalid_snapshot(
                    "legacy revision snapshots cannot contain current revision or replay provenance",
                );
            }
            let legacy_checkpoint = snapshot_checkpoint_hash(snapshot)?;
            if !is_canonical_hash(&snapshot.checkpoint_hash)
                || legacy_checkpoint != snapshot.checkpoint_hash
            {
                return invalid_snapshot(
                    "legacy checkpoint hash does not bind the pre-migration state",
                );
            }
            validate_legacy_boundary_hash_chain(&snapshot.boundaries)?;
            migrate_command_attempt_revisions(
                &mut snapshot.command_attempts,
                &snapshot.boundaries,
            )?;
            migrate_ingress_command_revisions(
                &mut snapshot.ingress,
                &snapshot.command_attempts,
                &snapshot.boundaries,
            )?;
            if snapshot_is_at_boundary_head(snapshot) {
                let migrated_state_hash = snapshot_state_hash(snapshot)?;
                snapshot
                    .boundaries
                    .last_mut()
                    .expect("boundary-head snapshots have a final boundary")
                    .state_hash = Some(migrated_state_hash);
            }
            rehash_snapshot_boundaries(snapshot)?;
            snapshot.state_revision = authoritative_revision_count(
                snapshot.commands.len(),
                snapshot.command_attempts.len(),
                snapshot.boundaries.len(),
            )?;
            snapshot.revision_format_version = STATE_REVISION_FORMAT_VERSION;
            snapshot.checkpoint_hash = snapshot_checkpoint_hash(snapshot)?;
            Ok(())
        }
        version => Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "state revision format {version} is unsupported; this engine reads legacy format 0 and current format {STATE_REVISION_FORMAT_VERSION}"
            ),
        )),
    }
}

fn migrate_snapshot_admission_cursors(snapshot: &mut SimulationSnapshot) -> Result<(), CanwuError> {
    match snapshot.admission_cursor_format_version {
        ADMISSION_CURSOR_FORMAT_VERSION => Ok(()),
        0 => {
            if snapshot.admitted_attempt_count != 0
                || snapshot.admitted_command_count != 0
                || snapshot.admitted_event_count != 0
            {
                return invalid_snapshot(
                    "legacy admission-cursor snapshots cannot contain current cursor values",
                );
            }
            let cursors = admission_cursors_from_boundaries(&snapshot.boundaries)?;
            snapshot.admitted_attempt_count = cursors.attempts;
            snapshot.admitted_command_count = cursors.commands;
            snapshot.admitted_event_count = cursors.events;
            snapshot.admission_cursor_format_version = ADMISSION_CURSOR_FORMAT_VERSION;
            Ok(())
        }
        version => Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "admission cursor format {version} is unsupported; this engine reads legacy format 0 and current format {ADMISSION_CURSOR_FORMAT_VERSION}"
            ),
        )),
    }
}

fn migrate_snapshot_commitments(snapshot: &mut SimulationSnapshot) -> Result<(), CanwuError> {
    match snapshot.commitment_format_version {
        COMMITMENT_FORMAT_VERSION => {
            if snapshot.commitment_roots.is_none() {
                return invalid_snapshot(
                    "current commitment snapshot is missing its persisted domain roots",
                );
            }
            Ok(())
        }
        0 => {
            if snapshot.commitment_roots.is_some() {
                return invalid_snapshot(
                    "legacy commitment snapshot cannot contain current domain roots",
                );
            }
            let legacy_checkpoint = snapshot_checkpoint_hash(snapshot)?;
            if !is_canonical_hash(&snapshot.checkpoint_hash)
                || legacy_checkpoint != snapshot.checkpoint_hash
            {
                return invalid_snapshot(
                    "legacy checkpoint hash does not bind the pre-commitment state",
                );
            }
            snapshot.commitment_roots = Some(snapshot_commitment_roots(snapshot)?);
            snapshot.commitment_format_version = COMMITMENT_FORMAT_VERSION;
            snapshot.checkpoint_hash = snapshot_checkpoint_hash(snapshot)?;
            Ok(())
        }
        version => Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "commitment format {version} is unsupported; this engine reads legacy format 0 and current format {COMMITMENT_FORMAT_VERSION}"
            ),
        )),
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PersistedAdmissionCursors {
    attempts: u64,
    commands: u64,
    events: u64,
}

fn admission_cursors_from_boundaries(
    boundaries: &[BoundaryRecord],
) -> Result<PersistedAdmissionCursors, CanwuError> {
    let mut cursors = PersistedAdmissionCursors::default();
    for boundary in boundaries {
        cursors.attempts = cursors
            .attempts
            .checked_add(
                u64::try_from(boundary.admitted_attempts.len()).map_err(|_| {
                    invalid_snapshot_error("admitted attempt count exceeds cursor range")
                })?,
            )
            .ok_or_else(|| invalid_snapshot_error("admitted attempt cursor is exhausted"))?;
        cursors.commands = cursors
            .commands
            .checked_add(
                u64::try_from(boundary.admitted_commands.len()).map_err(|_| {
                    invalid_snapshot_error("admitted command count exceeds cursor range")
                })?,
            )
            .ok_or_else(|| invalid_snapshot_error("admitted command cursor is exhausted"))?;
        cursors.events =
            cursors
                .events
                .checked_add(u64::try_from(boundary.admitted_events.len()).map_err(|_| {
                    invalid_snapshot_error("admitted event count exceeds cursor range")
                })?)
                .ok_or_else(|| invalid_snapshot_error("admitted event cursor is exhausted"))?;
    }
    Ok(cursors)
}

fn rehash_snapshot_boundaries(snapshot: &mut SimulationSnapshot) -> Result<(), CanwuError> {
    let mut previous_hash = GENESIS_BOUNDARY_HASH.to_owned();
    for boundary in &mut snapshot.boundaries {
        boundary.previous_hash.clone_from(&previous_hash);
        boundary.hash = compute_boundary_hash(boundary)?;
        previous_hash.clone_from(&boundary.hash);
    }
    Ok(())
}

fn validate_legacy_boundary_hash_chain(boundaries: &[BoundaryRecord]) -> Result<(), CanwuError> {
    let mut previous_hash = GENESIS_BOUNDARY_HASH.to_owned();
    for boundary in boundaries {
        if boundary.previous_hash != previous_hash
            || !is_canonical_hash(&boundary.hash)
            || boundary
                .state_hash
                .as_deref()
                .is_some_and(|hash| !is_canonical_hash(hash))
            || compute_boundary_hash(boundary)? != boundary.hash
        {
            return invalid_snapshot("legacy boundary hash chain is inconsistent");
        }
        previous_hash.clone_from(&boundary.hash);
    }
    Ok(())
}

fn migrate_ingress_command_revisions(
    ingress: &mut [IngressRecord],
    attempts: &[CommandAttemptRecord],
    boundaries: &[BoundaryRecord],
) -> Result<(), CanwuError> {
    let mut attempts_by_request = BTreeMap::new();
    for attempt in attempts {
        let Some(request_id) = attempt.request_id else {
            continue;
        };
        let expected_revision = attempt.expected_revision.ok_or_else(|| {
            invalid_snapshot_error("tracked command attempt is missing its revision guard")
        })?;
        if attempts_by_request
            .insert(request_id, expected_revision)
            .is_some()
        {
            return invalid_snapshot("command request ID is reused across attempt evidence");
        }
    }
    let (legacy_revision_by_cut, migrated_revision_by_cut) =
        revision_values_after_boundary_cuts(attempts, boundaries)?;
    let admitted_ingress: BTreeSet<_> = boundaries
        .iter()
        .flat_map(|boundary| boundary.admitted_ingress.iter().copied())
        .collect();
    for record in ingress {
        let IngressPayload::Command { request } = &mut record.payload else {
            continue;
        };
        if admitted_ingress.contains(&record.id) {
            request.expected_revision =
                *attempts_by_request
                    .get(&request.request_id)
                    .ok_or_else(|| {
                        invalid_snapshot_error(
                            "admitted command ingress is missing its deterministic attempt",
                        )
                    })?;
            continue;
        }
        let issue_cut = usize::try_from(record.eligible_boundary_count).map_err(|_| {
            invalid_snapshot_error("command ingress issue cut exceeds the platform index range")
        })?;
        let legacy_revision = *legacy_revision_by_cut.get(issue_cut).ok_or_else(|| {
            invalid_snapshot_error("command ingress issue cut exceeds boundary history")
        })?;
        let migrated_revision = migrated_revision_by_cut[issue_cut];
        request.expected_revision = translate_revision_guard(
            request.expected_revision,
            legacy_revision,
            migrated_revision,
        )?;
    }
    Ok(())
}

fn revision_values_after_boundary_cuts(
    attempts: &[CommandAttemptRecord],
    boundaries: &[BoundaryRecord],
) -> Result<(Vec<u64>, Vec<u64>), CanwuError> {
    let mut legacy = vec![0];
    let mut migrated = vec![0];
    let mut accepted_attempts = 0_u64;
    let mut admitted_attempts = 0_u64;
    for (boundary_index, boundary) in boundaries.iter().enumerate() {
        for attempt_id in &boundary.admitted_attempts {
            let attempt_index =
                usize::try_from(attempt_id.get().saturating_sub(1)).map_err(|_| {
                    invalid_snapshot_error("boundary attempt ID exceeds the journal index range")
                })?;
            let attempt = attempts.get(attempt_index).ok_or_else(|| {
                invalid_snapshot_error("boundary admits an unknown command attempt")
            })?;
            admitted_attempts = admitted_attempts.checked_add(1).ok_or_else(|| {
                invalid_snapshot_error("admitted attempt count exceeds revision space")
            })?;
            if matches!(attempt.outcome, CommandAttemptOutcome::Accepted { .. }) {
                accepted_attempts = accepted_attempts.checked_add(1).ok_or_else(|| {
                    invalid_snapshot_error("accepted attempt count exceeds revision space")
                })?;
            }
        }
        let completed_boundaries = u64::try_from(boundary_index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or_else(|| invalid_snapshot_error("boundary count exceeds revision space"))?;
        legacy.push(
            accepted_attempts
                .checked_add(completed_boundaries)
                .ok_or_else(|| invalid_snapshot_error("legacy revision space is exhausted"))?,
        );
        migrated.push(
            admitted_attempts
                .checked_add(completed_boundaries)
                .ok_or_else(|| invalid_snapshot_error("migrated revision space is exhausted"))?,
        );
    }
    Ok((legacy, migrated))
}

fn migrate_command_attempt_revisions(
    attempts: &mut [CommandAttemptRecord],
    boundaries: &[BoundaryRecord],
) -> Result<(), CanwuError> {
    let boundaries_before = boundaries_before_attempts(attempts.len(), boundaries)?;
    let mut accepted_commands = 0_u64;
    for (index, attempt) in attempts.iter_mut().enumerate() {
        let attempt_index = u64::try_from(index)
            .map_err(|_| invalid_snapshot_error("command attempt index exceeds revision space"))?;
        let legacy_revision = accepted_commands
            .checked_add(boundaries_before[index])
            .ok_or_else(|| invalid_snapshot_error("legacy command revision space is exhausted"))?;
        let migrated_revision = attempt_index
            .checked_add(boundaries_before[index])
            .ok_or_else(|| invalid_snapshot_error("command revision space is exhausted"))?;
        if attempt.revision_before != legacy_revision {
            return invalid_snapshot("legacy command attempt revision is inconsistent");
        }
        if let Some(expected) = attempt.expected_revision {
            attempt.expected_revision = Some(translate_revision_guard(
                expected,
                legacy_revision,
                migrated_revision,
            )?);
        }
        attempt.revision_before = migrated_revision;
        match &mut attempt.outcome {
            CommandAttemptOutcome::Accepted { .. } => {
                if attempt
                    .expected_revision
                    .is_some_and(|expected| expected != migrated_revision)
                {
                    return invalid_snapshot(
                        "legacy accepted attempt has a stale expected revision",
                    );
                }
                accepted_commands = accepted_commands.checked_add(1).ok_or_else(|| {
                    invalid_snapshot_error("accepted command count exceeds revision space")
                })?;
            }
            CommandAttemptOutcome::Rejected { error }
                if error.code == ErrorCode::SimulationRevisionConflict =>
            {
                let expected = attempt.expected_revision.ok_or_else(|| {
                    invalid_snapshot_error("revision conflict is missing its expected revision")
                })?;
                error.message = format!(
                    "command expected revision {expected}, but simulation is at revision {migrated_revision}"
                );
            }
            CommandAttemptOutcome::Rejected { .. } => {}
        }
    }
    Ok(())
}

fn translate_revision_guard(
    expected: u64,
    legacy_revision: u64,
    migrated_revision: u64,
) -> Result<u64, CanwuError> {
    if expected >= legacy_revision {
        Ok(migrated_revision
            .checked_add(expected - legacy_revision)
            .unwrap_or(if migrated_revision == u64::MAX {
                0
            } else {
                u64::MAX
            }))
    } else {
        migrated_revision
            .checked_sub(legacy_revision - expected)
            .ok_or_else(|| invalid_snapshot_error("migrated expected revision underflowed"))
    }
}

fn boundaries_before_attempts(
    attempt_count: usize,
    boundaries: &[BoundaryRecord],
) -> Result<Vec<u64>, CanwuError> {
    let boundary_count = u64::try_from(boundaries.len())
        .map_err(|_| invalid_snapshot_error("boundary count exceeds revision space"))?;
    let mut values = vec![boundary_count; attempt_count];
    for (boundary_index, boundary) in boundaries.iter().enumerate() {
        let prior_boundaries = u64::try_from(boundary_index)
            .map_err(|_| invalid_snapshot_error("boundary index exceeds revision space"))?;
        for attempt_id in &boundary.admitted_attempts {
            let attempt_index =
                usize::try_from(attempt_id.get().saturating_sub(1)).map_err(|_| {
                    invalid_snapshot_error("boundary attempt ID exceeds the journal index range")
                })?;
            let Some(value) = values.get_mut(attempt_index) else {
                return invalid_snapshot("boundary admits an unknown command attempt");
            };
            *value = prior_boundaries;
        }
    }
    Ok(values)
}

fn authoritative_revision_count(
    command_count: usize,
    attempt_count: usize,
    boundary_count: usize,
) -> Result<u64, CanwuError> {
    let command_transactions = if attempt_count == 0 {
        command_count
    } else {
        attempt_count
    };
    u64::try_from(command_transactions)
        .ok()
        .and_then(|commands| {
            u64::try_from(boundary_count)
                .ok()
                .and_then(|boundaries| commands.checked_add(boundaries))
        })
        .ok_or_else(|| invalid_snapshot_error("authoritative revision space is exhausted"))
}

fn validate_legacy_ingress_shape(snapshot: &SimulationSnapshot) -> Result<(), CanwuError> {
    if snapshot.revision_format_version != 0
        || snapshot.state_revision != 0
        || snapshot.replay_revision_format_version != 0
        || snapshot.admission_cursor_format_version != 0
        || snapshot.admitted_attempt_count != 0
        || snapshot.admitted_command_count != 0
        || snapshot.admitted_event_count != 0
        || snapshot.run_configuration.is_some()
        || snapshot.initial_scenario.is_some()
        || !snapshot.command_attempts.is_empty()
        || !snapshot.ingress.is_empty()
        || !snapshot.domain_records.is_empty()
        || snapshot.next_command_attempt_id != 1
        || snapshot.next_ingress_id != 1
        || snapshot
            .commands
            .iter()
            .any(|record| record.attempt_id.is_some() || !record.emitted_events.is_empty())
        || snapshot.boundaries.iter().any(|record| {
            !record.admitted_attempts.is_empty()
                || !record.admitted_ingress.is_empty()
                || !record.generated_ingress.is_empty()
                || !record.record_changes.is_empty()
        })
    {
        return invalid_snapshot(
            "legacy snapshots cannot contain current run-policy or command-attempt evidence",
        );
    }
    Ok(())
}

fn migrate_format_3_snapshot(
    mut snapshot: SimulationSnapshot,
    source_snapshot_format: u32,
    checkpoint_hash: String,
) -> Result<SimulationSnapshot, CanwuError> {
    if !snapshot.plugin_descriptors.is_empty() {
        return Err(CanwuError::new(
            ErrorCode::PluginManifestMismatch,
            "legacy plugin snapshots lack executable semantic identities and cannot be safely migrated",
        ));
    }
    if !snapshot.random_streams.is_empty()
        || !snapshot.random_draws.is_empty()
        || snapshot.next_random_draw_id != 0
    {
        return invalid_snapshot("legacy snapshots cannot contain scoped random state");
    }
    let legacy_rng = snapshot
        .legacy_rng
        .take()
        .ok_or_else(|| invalid_snapshot_error("legacy snapshot is missing its global RNG state"))?;
    let dispatch_count = u64::try_from(
        snapshot
            .events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::ReportDispatched { .. }))
            .count(),
    )
    .map_err(|_| invalid_snapshot_error("legacy random draw count exceeds identifier space"))?;
    let root_seed = DeterministicRng::seed_before(legacy_rng.state(), dispatch_count);
    let core_key = random::core_report_delay_stream();
    let mut core_state = RandomStreamState::initial(root_seed, core_key.clone());
    core_state.position = dispatch_count;
    core_state.generator_state = legacy_rng.state();

    let mut random_draws = Vec::new();
    for event in &snapshot.events {
        let EventKind::ReportDispatched {
            recipient,
            army,
            arrives_at,
        } = event.kind
        else {
            continue;
        };
        let jitter = arrives_at
            .as_minutes()
            .checked_sub(event.timestamp.as_minutes())
            .and_then(|duration| duration.checked_sub(SimDuration::hours(36).as_minutes()))
            .ok_or_else(|| {
                invalid_snapshot_error("legacy report timing exceeds the supported range")
            })?;
        let Ok(value) = u64::try_from(jitter) else {
            return invalid_snapshot("legacy report jitter is outside the scoped RNG contract");
        };
        let Some(CauseRef::Event(cause)) = &event.cause else {
            return invalid_snapshot("legacy report dispatch lacks its arrival-event cause");
        };
        if value >= 12 * 60 {
            return invalid_snapshot("legacy report jitter is outside the scoped RNG contract");
        }
        let id_value = u64::try_from(random_draws.len())
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| invalid_snapshot_error("legacy random draw IDs are exhausted"))?;
        random_draws.push(RandomDrawRecord {
            id: RandomDrawId::new(id_value),
            at: event.timestamp,
            stream: core_key.clone(),
            position: id_value - 1,
            upper_exclusive: 12 * 60,
            value,
            purpose: "knowledge report delivery jitter".to_owned(),
            producer: RandomDrawProducer::CoreSystem {
                system: "canwu.core.knowledge-report-delay".to_owned(),
            },
            outcome: Some(RandomDrawOutcome::KnowledgeReportDelivery {
                recipient,
                army,
                dispatch_event: event.id,
                arrives_at,
            }),
            cause: CauseRef::Event(*cause),
            correlation_id: event.correlation_id,
        });
    }

    let mut streams = BTreeMap::from([(core_key, core_state)]);
    for key in snapshot
        .plugin_descriptors
        .iter()
        .flat_map(|descriptor| &descriptor.boundary_systems)
        .flat_map(|contract| &contract.random_streams)
    {
        streams
            .entry(key.clone())
            .or_insert_with(|| RandomStreamState::initial(root_seed, key.clone()));
    }
    snapshot.root_seed = root_seed;
    snapshot.random_streams = streams.into_values().collect();
    snapshot.random_draws = random_draws;
    snapshot.next_random_draw_id = dispatch_count.checked_add(1).ok_or_else(|| {
        invalid_snapshot_error("legacy random draw counter exceeds identifier space")
    })?;

    let mut previous_hash = GENESIS_BOUNDARY_HASH.to_owned();
    for boundary in &mut snapshot.boundaries {
        boundary.random_draws.clear();
        boundary.state_hash = None;
        boundary.previous_hash.clone_from(&previous_hash);
        boundary.hash = compute_boundary_hash(boundary)?;
        previous_hash.clone_from(&boundary.hash);
    }
    let source_engine_version = snapshot.engine_version.clone();
    let run_manifest = RunManifest::migrated_legacy(
        source_engine_version,
        source_snapshot_format,
        checkpoint_hash,
    );
    snapshot.run_manifest_hash = manifest::hash(&run_manifest)?;
    snapshot.run_manifest = Some(run_manifest);
    snapshot.run_configuration = Some(RunConfigurationSnapshot::LegacyUnspecified);
    snapshot.state_revision = authoritative_revision_count(
        snapshot.commands.len(),
        snapshot.command_attempts.len(),
        snapshot.boundaries.len(),
    )?;
    snapshot.revision_format_version = STATE_REVISION_FORMAT_VERSION;
    let admission_cursors = admission_cursors_from_boundaries(&snapshot.boundaries)?;
    snapshot.admitted_attempt_count = admission_cursors.attempts;
    snapshot.admitted_command_count = admission_cursors.commands;
    snapshot.admitted_event_count = admission_cursors.events;
    snapshot.admission_cursor_format_version = ADMISSION_CURSOR_FORMAT_VERSION;
    ENGINE_VERSION.clone_into(&mut snapshot.engine_version);
    snapshot.snapshot_format_version = SNAPSHOT_FORMAT_VERSION;
    snapshot.checkpoint_hash = snapshot_checkpoint_hash(&snapshot)?;
    migrate_snapshot_commitments(&mut snapshot)?;
    Ok(snapshot)
}

fn hydrate_snapshot_run_configuration(snapshot: &mut SimulationSnapshot) -> Result<(), CanwuError> {
    if snapshot.run_configuration.is_some() {
        return Ok(());
    }
    let Some(run_manifest) = &snapshot.run_manifest else {
        return Err(CanwuError::new(
            ErrorCode::InvalidRunManifest,
            "snapshot is missing its run manifest",
        ));
    };
    snapshot.run_configuration = Some(inferred_run_configuration(run_manifest)?);
    Ok(())
}

fn inferred_run_configuration(
    run_manifest: &RunManifest,
) -> Result<RunConfigurationSnapshot, CanwuError> {
    Ok(match run_manifest {
        RunManifest::Declared {
            run_configuration, ..
        } if run_configuration.semantic_hash == policy::compatibility_configuration_hash()? => {
            RunConfigurationSnapshot::CompatibilityV1
        }
        RunManifest::Declared { .. } => RunConfigurationSnapshot::ManifestOnlyV1,
        RunManifest::MigratedLegacy { .. } => RunConfigurationSnapshot::LegacyUnspecified,
    })
}

#[derive(Serialize)]
struct StateHashMaterial<'a> {
    engine_version: &'a str,
    snapshot_format_version: u32,
    run_manifest: &'a RunManifest,
    run_manifest_hash: &'a str,
    initial_time: SimTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    initial_scenario: Option<&'a Scenario>,
    now: SimTime,
    plugin_registration_closed: bool,
    world: &'a WorldSnapshot,
    knowledge: &'a KnowledgeSnapshot,
    events: &'a [SimEvent],
    commands: &'a [CommandRecord],
    #[serde(skip_serializing_if = "command_attempt_slice_is_empty")]
    command_attempts: &'a [CommandAttemptRecord],
    #[serde(skip_serializing_if = "ingress_record_slice_is_empty")]
    ingress: &'a [IngressRecord],
    plugin_components: &'a [PluginComponentRecord],
    #[serde(skip_serializing_if = "domain_record_slice_is_empty")]
    domain_records: &'a [DomainRecord],
    plugin_descriptors: &'a [PluginDescriptor],
    schema: &'a SchemaRegistry,
    scheduled: &'a [ScheduledRecord],
    root_seed: u64,
    random_streams: &'a [RandomStreamState],
    random_draws: &'a [RandomDrawRecord],
    next_event_id: u64,
    next_command_id: u64,
    #[serde(skip_serializing_if = "is_one_u64")]
    next_command_attempt_id: u64,
    #[serde(skip_serializing_if = "is_one_u64")]
    next_ingress_id: u64,
    next_boundary_id: u64,
    next_random_draw_id: u64,
    next_schedule_sequence: u64,
    next_correlation_id: u64,
}

#[derive(Serialize)]
struct WorldCommitmentMaterial {
    people: String,
    governments: String,
    territories: String,
    routes: String,
    armies: String,
}

#[derive(Serialize)]
struct SchedulerCommitmentMaterial {
    now: SimTime,
    scheduled: String,
}

#[derive(Serialize)]
struct CommandCommitmentMaterial {
    commands: String,
    attempts: String,
}

#[derive(Serialize)]
struct RandomCommitmentMaterial {
    root_seed: u64,
    streams: String,
    draws: String,
}

#[derive(Serialize)]
struct IdentityCommitmentMaterial<'a> {
    engine_version: &'a str,
    snapshot_format_version: u32,
    run_manifest: &'a RunManifest,
    run_manifest_hash: &'a str,
    initial_time: SimTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    initial_scenario: Option<&'a Scenario>,
    plugin_descriptors: String,
    schema: &'a SchemaRegistry,
}

#[derive(Serialize)]
struct ControlCommitmentMaterial {
    plugin_registration_closed: bool,
    next_event_id: u64,
    next_command_id: u64,
    next_command_attempt_id: u64,
    next_ingress_id: u64,
    next_boundary_id: u64,
    next_random_draw_id: u64,
    next_schedule_sequence: u64,
    next_correlation_id: u64,
}

#[derive(Serialize)]
struct CheckpointHashMaterialV1<'a> {
    state_hash: &'a str,
    boundary_head: Option<&'a str>,
}

#[derive(Serialize)]
struct CheckpointHashMaterialV2<'a> {
    state_hash: &'a str,
    boundary_head: Option<&'a str>,
    run_manifest_hash: &'a str,
}

#[derive(Serialize)]
struct CheckpointHashMaterialV3<'a> {
    state_hash: &'a str,
    boundary_head: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_manifest_hash: Option<&'a str>,
    revision_format_version: u32,
    state_revision: u64,
    replay_revision_format_version: u32,
}

#[derive(Serialize)]
struct CheckpointHashMaterialV4<'a> {
    commitments: &'a CommitmentRoots,
    run_manifest_hash: &'a str,
    commitment_format_version: u32,
    revision_format_version: u32,
    state_revision: u64,
    replay_revision_format_version: u32,
}

fn state_hash(material: &StateHashMaterial<'_>) -> Result<String, CanwuError> {
    canonical_hash("canwu.boundary-state.v1", material)
}

fn canonical_sorted_hash_by<T, K, F>(
    domain: &str,
    values: &[T],
    mut key: F,
) -> Result<String, CanwuError>
where
    T: Serialize,
    K: Ord,
    F: FnMut(&T) -> K,
{
    let mut ordered: Vec<_> = values.iter().collect();
    ordered.sort_by_key(|value| key(value));
    canonical_hash(domain, &ordered)
}

fn world_commitment_root(world: &WorldSnapshot) -> Result<String, CanwuError> {
    canonical_hash(
        "canwu.commitment.world.v1",
        &WorldCommitmentMaterial {
            people: canonical_sorted_hash_by(
                "canwu.commitment.world.people.v1",
                &world.people,
                |value| value.id,
            )?,
            governments: canonical_sorted_hash_by(
                "canwu.commitment.world.governments.v1",
                &world.governments,
                |value| value.id,
            )?,
            territories: canonical_sorted_hash_by(
                "canwu.commitment.world.territories.v1",
                &world.territories,
                |value| value.id,
            )?,
            routes: canonical_sorted_hash_by(
                "canwu.commitment.world.routes.v1",
                &world.routes,
                |value| value.id,
            )?,
            armies: canonical_sorted_hash_by(
                "canwu.commitment.world.armies.v1",
                &world.armies,
                |value| value.id,
            )?,
        },
    )
}

fn knowledge_commitment_root(knowledge: &KnowledgeSnapshot) -> Result<String, CanwuError> {
    canonical_hash("canwu.commitment.knowledge.v1", knowledge)
}

fn plugin_component_commitment_root(
    components: &[PluginComponentRecord],
) -> Result<String, CanwuError> {
    canonical_sorted_hash_by(
        "canwu.commitment.plugin-components.v1",
        components,
        |record| {
            component_key(
                &record.plugin,
                &record.state,
                &record.entity,
                &record.component,
            )
        },
    )
}

fn domain_record_commitment_root(records: &[DomainRecord]) -> Result<String, CanwuError> {
    canonical_sorted_hash_by("canwu.commitment.domain-records.v1", records, |record| {
        record.reference.clone()
    })
}

fn scheduler_commitment_root(
    now: SimTime,
    scheduled: &[ScheduledRecord],
) -> Result<String, CanwuError> {
    canonical_hash(
        "canwu.commitment.scheduler.v1",
        &SchedulerCommitmentMaterial {
            now,
            scheduled: canonical_sorted_hash_by(
                "canwu.commitment.scheduler.entries.v1",
                scheduled,
                |record| record.key.clone(),
            )?,
        },
    )
}

fn random_stream_commitment_root(streams: &[RandomStreamState]) -> Result<String, CanwuError> {
    canonical_sorted_hash_by("canwu.commitment.random.streams.v1", streams, |stream| {
        stream.key.clone()
    })
}

#[allow(clippy::too_many_arguments)]
fn identity_commitment_root(
    engine_version: &str,
    snapshot_format_version: u32,
    run_manifest: &RunManifest,
    run_manifest_hash: &str,
    initial_time: SimTime,
    initial_scenario: Option<&Scenario>,
    plugin_descriptors: &[PluginDescriptor],
    schema: &SchemaRegistry,
) -> Result<String, CanwuError> {
    canonical_hash(
        "canwu.commitment.identity.v1",
        &IdentityCommitmentMaterial {
            engine_version,
            snapshot_format_version,
            run_manifest,
            run_manifest_hash,
            initial_time,
            initial_scenario,
            plugin_descriptors: canonical_sorted_hash_by(
                "canwu.commitment.identity.plugins.v1",
                plugin_descriptors,
                |descriptor| descriptor.name.clone(),
            )?,
            schema,
        },
    )
}

fn commitment_roots(
    material: &StateHashMaterial<'_>,
    boundary_head: Option<&str>,
    journal_roots: Option<&JournalCommitmentRoots>,
) -> Result<CommitmentRoots, CanwuError> {
    let world = world_commitment_root(material.world)?;
    let knowledge = knowledge_commitment_root(material.knowledge)?;
    let plugin_components = plugin_component_commitment_root(material.plugin_components)?;
    let domain_records = domain_record_commitment_root(material.domain_records)?;
    let scheduler = scheduler_commitment_root(material.now, material.scheduled)?;
    let command_root = match journal_roots {
        Some(roots) => roots.commands.clone(),
        None => canonical_sorted_hash_by(
            "canwu.commitment.commands.accepted.v1",
            material.commands,
            |record| record.id,
        )?,
    };
    let attempt_root = match journal_roots {
        Some(roots) => roots.attempts.clone(),
        None => canonical_sorted_hash_by(
            "canwu.commitment.commands.attempts.v1",
            material.command_attempts,
            |record| record.id,
        )?,
    };
    let commands = canonical_hash(
        "canwu.commitment.commands.v1",
        &CommandCommitmentMaterial {
            commands: command_root,
            attempts: attempt_root,
        },
    )?;
    let events = match journal_roots {
        Some(roots) => roots.events.clone(),
        None => canonical_sorted_hash_by("canwu.commitment.events.v1", material.events, |event| {
            event.id
        })?,
    };
    let ingress = match journal_roots {
        Some(roots) => roots.ingress.clone(),
        None => {
            canonical_sorted_hash_by("canwu.commitment.ingress.v1", material.ingress, |record| {
                record.id
            })?
        }
    };
    let random = canonical_hash(
        "canwu.commitment.random.v1",
        &RandomCommitmentMaterial {
            root_seed: material.root_seed,
            streams: random_stream_commitment_root(material.random_streams)?,
            draws: match journal_roots {
                Some(roots) => roots.random_draws.clone(),
                None => canonical_sorted_hash_by(
                    "canwu.commitment.random.draws.v1",
                    material.random_draws,
                    |draw| draw.id,
                )?,
            },
        },
    )?;
    let boundary_chain = canonical_hash(
        "canwu.commitment.boundary-chain.v1",
        boundary_head.unwrap_or(GENESIS_BOUNDARY_HASH),
    )?;
    let identity = identity_commitment_root(
        material.engine_version,
        material.snapshot_format_version,
        material.run_manifest,
        material.run_manifest_hash,
        material.initial_time,
        material.initial_scenario,
        material.plugin_descriptors,
        material.schema,
    )?;
    let control = canonical_hash(
        "canwu.commitment.control.v1",
        &ControlCommitmentMaterial {
            plugin_registration_closed: material.plugin_registration_closed,
            next_event_id: material.next_event_id,
            next_command_id: material.next_command_id,
            next_command_attempt_id: material.next_command_attempt_id,
            next_ingress_id: material.next_ingress_id,
            next_boundary_id: material.next_boundary_id,
            next_random_draw_id: material.next_random_draw_id,
            next_schedule_sequence: material.next_schedule_sequence,
            next_correlation_id: material.next_correlation_id,
        },
    )?;
    Ok(CommitmentRoots {
        world,
        knowledge,
        plugin_components,
        domain_records,
        scheduler,
        commands,
        events,
        ingress,
        random,
        boundary_chain,
        identity,
        control,
    })
}

fn runtime_commitment_roots(
    domain: &RuntimeDomainCommitmentRoots,
    journal: &JournalCommitmentRoots,
    root_seed: u64,
    boundary_head: Option<&str>,
    control: &ControlCommitmentMaterial,
) -> Result<CommitmentRoots, CanwuError> {
    let commands = canonical_hash(
        "canwu.commitment.commands.v1",
        &CommandCommitmentMaterial {
            commands: journal.commands.clone(),
            attempts: journal.attempts.clone(),
        },
    )?;
    let random = canonical_hash(
        "canwu.commitment.random.v1",
        &RandomCommitmentMaterial {
            root_seed,
            streams: domain.random_streams.clone(),
            draws: journal.random_draws.clone(),
        },
    )?;
    Ok(CommitmentRoots {
        world: domain.world.clone(),
        knowledge: domain.knowledge.clone(),
        plugin_components: domain.plugin_components.clone(),
        domain_records: domain.domain_records.clone(),
        scheduler: domain.scheduler.clone(),
        commands,
        events: journal.events.clone(),
        ingress: journal.ingress.clone(),
        random,
        boundary_chain: canonical_hash(
            "canwu.commitment.boundary-chain.v1",
            boundary_head.unwrap_or(GENESIS_BOUNDARY_HASH),
        )?,
        identity: domain.identity.clone(),
        control: canonical_hash("canwu.commitment.control.v1", control)?,
    })
}

fn authoritative_run_identity(
    run_manifest: &RunManifest,
    run_manifest_hash: &str,
    run_configuration: &RunConfigurationSnapshot,
) -> Result<(RunManifest, String), CanwuError> {
    if !matches!(run_configuration, RunConfigurationSnapshot::Declared(_)) {
        return Ok((run_manifest.clone(), run_manifest_hash.to_owned()));
    }
    let mut authoritative_manifest = run_manifest.clone();
    let RunManifest::Declared {
        run_configuration, ..
    } = &mut authoritative_manifest
    else {
        return Err(CanwuError::new(
            ErrorCode::InvalidRunManifest,
            "declared run policy requires a declared run manifest",
        ));
    };
    **run_configuration = ArtifactManifest::new(
        "canwu.core",
        "authoritative-policy-excluded",
        "1",
        policy::authoritative_configuration_hash()?,
    )?;
    let authoritative_manifest_hash = manifest::hash(&authoritative_manifest)?;
    Ok((authoritative_manifest, authoritative_manifest_hash))
}

fn snapshot_state_hash(snapshot: &SimulationSnapshot) -> Result<String, CanwuError> {
    let Some(run_manifest) = &snapshot.run_manifest else {
        return Err(CanwuError::new(
            ErrorCode::InvalidRunManifest,
            "snapshot is missing its run manifest",
        ));
    };
    let run_configuration = snapshot.run_configuration.as_ref().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidRunConfiguration,
            "snapshot is missing its run configuration",
        )
    })?;
    let (authoritative_manifest, authoritative_manifest_hash) =
        authoritative_run_identity(run_manifest, &snapshot.run_manifest_hash, run_configuration)?;
    state_hash(&StateHashMaterial {
        engine_version: &snapshot.engine_version,
        snapshot_format_version: snapshot.snapshot_format_version,
        run_manifest: &authoritative_manifest,
        run_manifest_hash: &authoritative_manifest_hash,
        initial_time: snapshot.initial_time,
        initial_scenario: snapshot.initial_scenario.as_ref(),
        now: snapshot.now,
        plugin_registration_closed: snapshot.plugin_registration_closed,
        world: &snapshot.world,
        knowledge: &snapshot.knowledge,
        events: &snapshot.events,
        commands: &snapshot.commands,
        command_attempts: &snapshot.command_attempts,
        ingress: &snapshot.ingress,
        plugin_components: &snapshot.plugin_components,
        domain_records: &snapshot.domain_records,
        plugin_descriptors: &snapshot.plugin_descriptors,
        schema: &snapshot.schema,
        scheduled: &snapshot.scheduled,
        root_seed: snapshot.root_seed,
        random_streams: &snapshot.random_streams,
        random_draws: &snapshot.random_draws,
        next_event_id: snapshot.next_event_id,
        next_command_id: snapshot.next_command_id,
        next_command_attempt_id: snapshot.next_command_attempt_id,
        next_ingress_id: snapshot.next_ingress_id,
        next_boundary_id: snapshot.next_boundary_id,
        next_random_draw_id: snapshot.next_random_draw_id,
        next_schedule_sequence: snapshot.next_schedule_sequence,
        next_correlation_id: snapshot.next_correlation_id,
    })
}

fn snapshot_commitment_roots(snapshot: &SimulationSnapshot) -> Result<CommitmentRoots, CanwuError> {
    let Some(run_manifest) = &snapshot.run_manifest else {
        return Err(CanwuError::new(
            ErrorCode::InvalidRunManifest,
            "snapshot is missing its run manifest",
        ));
    };
    let run_configuration = snapshot.run_configuration.as_ref().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidRunConfiguration,
            "snapshot is missing its run configuration",
        )
    })?;
    let (authoritative_manifest, authoritative_manifest_hash) =
        authoritative_run_identity(run_manifest, &snapshot.run_manifest_hash, run_configuration)?;
    commitment_roots(
        &StateHashMaterial {
            engine_version: &snapshot.engine_version,
            snapshot_format_version: snapshot.snapshot_format_version,
            run_manifest: &authoritative_manifest,
            run_manifest_hash: &authoritative_manifest_hash,
            initial_time: snapshot.initial_time,
            initial_scenario: snapshot.initial_scenario.as_ref(),
            now: snapshot.now,
            plugin_registration_closed: snapshot.plugin_registration_closed,
            world: &snapshot.world,
            knowledge: &snapshot.knowledge,
            events: &snapshot.events,
            commands: &snapshot.commands,
            command_attempts: &snapshot.command_attempts,
            ingress: &snapshot.ingress,
            plugin_components: &snapshot.plugin_components,
            domain_records: &snapshot.domain_records,
            plugin_descriptors: &snapshot.plugin_descriptors,
            schema: &snapshot.schema,
            scheduled: &snapshot.scheduled,
            root_seed: snapshot.root_seed,
            random_streams: &snapshot.random_streams,
            random_draws: &snapshot.random_draws,
            next_event_id: snapshot.next_event_id,
            next_command_id: snapshot.next_command_id,
            next_command_attempt_id: snapshot.next_command_attempt_id,
            next_ingress_id: snapshot.next_ingress_id,
            next_boundary_id: snapshot.next_boundary_id,
            next_random_draw_id: snapshot.next_random_draw_id,
            next_schedule_sequence: snapshot.next_schedule_sequence,
            next_correlation_id: snapshot.next_correlation_id,
        },
        snapshot
            .boundaries
            .last()
            .map(|record| record.hash.as_str()),
        None,
    )
}

fn checkpoint_hash(state_hash: &str, boundary_head: Option<&str>) -> Result<String, CanwuError> {
    canonical_hash(
        "canwu.checkpoint.v1",
        &CheckpointHashMaterialV1 {
            state_hash,
            boundary_head,
        },
    )
}

fn checkpoint_hash_for_configuration(
    state_hash: &str,
    boundary_head: Option<&str>,
    run_manifest_hash: &str,
    run_configuration: &RunConfigurationSnapshot,
    revision_format_version: u32,
    state_revision: u64,
    replay_revision_format_version: u32,
) -> Result<String, CanwuError> {
    if revision_format_version == STATE_REVISION_FORMAT_VERSION {
        return canonical_hash(
            "canwu.checkpoint.v3",
            &CheckpointHashMaterialV3 {
                state_hash,
                boundary_head,
                run_manifest_hash: matches!(
                    run_configuration,
                    RunConfigurationSnapshot::Declared(_)
                )
                .then_some(run_manifest_hash),
                revision_format_version,
                state_revision,
                replay_revision_format_version,
            },
        );
    }
    if revision_format_version != 0 || state_revision != 0 || replay_revision_format_version != 0 {
        return Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "state revision format {revision_format_version} is unsupported; this engine writes format {STATE_REVISION_FORMAT_VERSION}"
            ),
        ));
    }
    if !matches!(run_configuration, RunConfigurationSnapshot::Declared(_)) {
        return checkpoint_hash(state_hash, boundary_head);
    }
    canonical_hash(
        "canwu.checkpoint.v2",
        &CheckpointHashMaterialV2 {
            state_hash,
            boundary_head,
            run_manifest_hash,
        },
    )
}

fn checkpoint_hash_for_commitments(
    commitments: &CommitmentRoots,
    run_manifest_hash: &str,
    commitment_format_version: u32,
    revision_format_version: u32,
    state_revision: u64,
    replay_revision_format_version: u32,
) -> Result<String, CanwuError> {
    if commitment_format_version != COMMITMENT_FORMAT_VERSION {
        return Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "commitment format {commitment_format_version} is unsupported; this engine writes format {COMMITMENT_FORMAT_VERSION}"
            ),
        ));
    }
    if revision_format_version != STATE_REVISION_FORMAT_VERSION {
        return Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "commitment format {commitment_format_version} requires state revision format {STATE_REVISION_FORMAT_VERSION}"
            ),
        ));
    }
    canonical_hash(
        "canwu.checkpoint.v4",
        &CheckpointHashMaterialV4 {
            commitments,
            run_manifest_hash,
            commitment_format_version,
            revision_format_version,
            state_revision,
            replay_revision_format_version,
        },
    )
}

fn snapshot_checkpoint_hash(snapshot: &SimulationSnapshot) -> Result<String, CanwuError> {
    match snapshot.commitment_format_version {
        COMMITMENT_FORMAT_VERSION => checkpoint_hash_for_commitments(
            snapshot.commitment_roots.as_ref().ok_or_else(|| {
                invalid_snapshot_error("current commitment snapshot is missing its domain roots")
            })?,
            &snapshot.run_manifest_hash,
            snapshot.commitment_format_version,
            snapshot.revision_format_version,
            snapshot.state_revision,
            snapshot.replay_revision_format_version,
        ),
        0 => {
            if snapshot.commitment_roots.is_some() {
                return invalid_snapshot(
                    "legacy commitment snapshot cannot contain current domain roots",
                );
            }
            let state_hash = snapshot_state_hash(snapshot)?;
            checkpoint_hash_for_configuration(
                &state_hash,
                snapshot
                    .boundaries
                    .last()
                    .map(|record| record.hash.as_str()),
                &snapshot.run_manifest_hash,
                snapshot.run_configuration.as_ref().ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidRunConfiguration,
                        "snapshot is missing its run configuration",
                    )
                })?,
                snapshot.revision_format_version,
                snapshot.state_revision,
                snapshot.replay_revision_format_version,
            )
        }
        version => Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "commitment format {version} is unsupported; this engine reads legacy format 0 and current format {COMMITMENT_FORMAT_VERSION}"
            ),
        )),
    }
}

fn snapshot_is_at_boundary_head(snapshot: &SimulationSnapshot) -> bool {
    let Some(last) = snapshot.boundaries.last() else {
        return false;
    };
    if last.at != snapshot.now {
        return false;
    }
    let admitted_attempts: BTreeSet<_> = snapshot
        .boundaries
        .iter()
        .flat_map(|record| record.admitted_attempts.iter().copied())
        .collect();
    if admitted_attempts.len() != snapshot.command_attempts.len() {
        return false;
    }
    let admitted_commands: BTreeSet<_> = snapshot
        .boundaries
        .iter()
        .flat_map(|record| record.admitted_commands.iter().copied())
        .collect();
    if admitted_commands.len() != snapshot.commands.len() {
        return false;
    }
    let admitted_ingress: BTreeSet<_> = snapshot
        .boundaries
        .iter()
        .flat_map(|record| record.admitted_ingress.iter().copied())
        .collect();
    if admitted_ingress.len() != snapshot.ingress.len() {
        return false;
    }
    let accounted_events: BTreeSet<_> = snapshot
        .boundaries
        .iter()
        .flat_map(|record| {
            record
                .admitted_events
                .iter()
                .copied()
                .chain(record.emissions.iter().map(|emission| emission.event))
        })
        .collect();
    accounted_events.len() == snapshot.events.len()
}

fn compute_boundary_hash(record: &BoundaryRecord) -> Result<String, CanwuError> {
    #[derive(Serialize)]
    struct BoundaryHashMaterial<'a> {
        id: BoundaryId,
        at: SimTime,
        correlation_id: u64,
        cadences: &'a [SystemCadence],
        #[serde(skip_serializing_if = "command_attempt_id_slice_is_empty")]
        admitted_attempts: &'a [CommandAttemptId],
        admitted_commands: &'a [CommandId],
        #[serde(skip_serializing_if = "Option::is_none")]
        admitted_ingress: Option<&'a [IngressId]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        generated_ingress: Option<&'a [BoundaryIngressGeneration]>,
        admitted_events: &'a [EventId],
        reservation_offers: &'a [ReservationOfferRecord],
        reservation_requests: &'a [ReservationRequestRecord],
        allocations: &'a [ReservationAllocation],
        random_draws: &'a [RandomDrawId],
        changes: &'a [BoundaryChange],
        #[serde(skip_serializing_if = "domain_record_change_slice_is_empty")]
        record_changes: &'a [DomainRecordChange],
        emissions: &'a [BoundaryEmission],
        state_hash: &'a Option<String>,
        previous_hash: &'a str,
    }

    canonical_hash(
        "canwu.boundary-record.v1",
        &BoundaryHashMaterial {
            id: record.id,
            at: record.at,
            correlation_id: record.correlation_id,
            cadences: &record.cadences,
            admitted_attempts: &record.admitted_attempts,
            admitted_commands: &record.admitted_commands,
            admitted_ingress: (!record.admitted_ingress.is_empty())
                .then_some(record.admitted_ingress.as_slice()),
            generated_ingress: (!record.generated_ingress.is_empty())
                .then_some(record.generated_ingress.as_slice()),
            admitted_events: &record.admitted_events,
            reservation_offers: &record.reservation_offers,
            reservation_requests: &record.reservation_requests,
            allocations: &record.allocations,
            random_draws: &record.random_draws,
            changes: &record.changes,
            record_changes: &record.record_changes,
            emissions: &record.emissions,
            state_hash: &record.state_hash,
            previous_hash: &record.previous_hash,
        },
    )
}

fn canonical_hash<T: Serialize + ?Sized>(domain: &str, value: &T) -> Result<String, CanwuError> {
    let encoded = serde_json::to_vec(value).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidSnapshot,
            format!("could not encode deterministic hash material: {error}"),
        )
    })?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain.as_bytes());
    hasher.update(&[0]);
    hasher.update(&encoded);
    Ok(hasher.finalize().to_hex().to_string())
}

fn is_canonical_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn commitment_roots_are_canonical(roots: &CommitmentRoots) -> bool {
    [
        &roots.world,
        &roots.knowledge,
        &roots.plugin_components,
        &roots.domain_records,
        &roots.scheduler,
        &roots.commands,
        &roots.events,
        &roots.ingress,
        &roots.random,
        &roots.boundary_chain,
        &roots.identity,
        &roots.control,
    ]
    .into_iter()
    .all(|root| is_canonical_hash(root))
}

fn invalid_snapshot_error(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidSnapshot, message)
}

fn invalid_snapshot<T>(message: impl Into<String>) -> Result<T, CanwuError> {
    Err(invalid_snapshot_error(message))
}

fn require_plugin_aware_initial_records(scenario: &Scenario) -> Result<(), CanwuError> {
    if scenario.domain_records.is_empty() {
        return Ok(());
    }
    Err(CanwuError::new(
        ErrorCode::PluginNotActive,
        "scenarios with initial domain records require a plugin-aware constructor",
    ))
}

fn canonicalize_scenario(scenario: &mut Scenario) {
    scenario.world.people.sort_by_key(|value| value.id);
    scenario.world.governments.sort_by_key(|value| value.id);
    scenario.world.territories.sort_by_key(|value| value.id);
    scenario.world.routes.sort_by_key(|value| value.id);
    scenario.world.armies.sort_by_key(|value| value.id);
    scenario
        .domain_records
        .sort_by(|left, right| left.reference.cmp(&right.reference));
}

fn validate_scenario(scenario: &Scenario) -> Result<(), CanwuError> {
    validate_unique_ids(&scenario.world.people, |value| value.id, "person")?;
    validate_unique_ids(&scenario.world.governments, |value| value.id, "government")?;
    validate_unique_ids(&scenario.world.territories, |value| value.id, "territory")?;
    validate_unique_ids(&scenario.world.routes, |value| value.id, "route")?;
    validate_unique_ids(&scenario.world.armies, |value| value.id, "army")?;

    for person in &scenario.world.people {
        if scenario.world.government(person.government).is_none()
            || scenario.world.territory(person.current_location).is_none()
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!(
                    "person {} references a missing government or location",
                    person.id
                ),
            ));
        }
    }
    for government in &scenario.world.governments {
        if scenario.world.territory(government.capital).is_none() {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("government {} references a missing capital", government.id),
            ));
        }
    }
    for territory in &scenario.world.territories {
        if scenario.world.government(territory.controller).is_none()
            || !territory.position.x.is_finite()
            || !territory.position.y.is_finite()
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!(
                    "territory {} has a missing controller or non-finite position",
                    territory.id
                ),
            ));
        }
    }
    for army in &scenario.world.armies {
        if scenario.world.person(army.commander).is_none()
            || scenario.world.government(army.government).is_none()
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!(
                    "army {} references a missing commander or government",
                    army.id
                ),
            ));
        }
        if scenario.world.territory(army.location).is_none() {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("army {} references a missing location", army.id),
            ));
        }
        if let Some(transit) = &army.transit
            && (scenario.world.territory(transit.from).is_none()
                || scenario.world.territory(transit.to).is_none()
                || transit.arrives_at < transit.departed_at
                || transit.departed_at > scenario.start_time
                || army.location != transit.from)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("army {} has invalid transit state", army.id),
            ));
        }
    }
    for route in &scenario.world.routes {
        if scenario.world.territory(route.from).is_none()
            || scenario.world.territory(route.to).is_none()
            || route.travel_minutes <= 0
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("route {} has invalid endpoints or travel time", route.id),
            ));
        }
    }
    records::validate_initial_records(&scenario.domain_records, scenario.start_time, &|entity| {
        core_world_entity_exists(&scenario.world, entity)
    })?;
    for (actor_id, actor) in &scenario.knowledge.actors {
        if actor.actor != *actor_id || scenario.world.person(*actor_id).is_none() {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("knowledge actor {actor_id} is inconsistent or missing"),
            ));
        }
        for (army_id, record) in &actor.armies {
            if record.army != *army_id
                || scenario.world.army(*army_id).is_none()
                || record
                    .known_location
                    .is_some_and(|location| scenario.world.territory(location).is_none())
                || record.estimated_strength.minimum > record.estimated_strength.maximum
                || record.confidence_per_mille > 1000
                || record.observed_at > record.learned_at
                || record.observed_at > scenario.start_time
                || record.learned_at > scenario.start_time
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    format!("knowledge record for actor {actor_id} and army {army_id} is invalid"),
                ));
            }
        }
    }
    Ok(())
}

fn validate_unique_ids<T, I, F>(values: &[T], mut id_of: F, label: &str) -> Result<(), CanwuError>
where
    I: Copy + Default + Display + Ord,
    F: FnMut(&T) -> I,
{
    let mut ids = BTreeSet::new();
    for value in values {
        let id = id_of(value);
        if id == I::default() {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("{label} IDs must be nonzero"),
            ));
        }
        if !ids.insert(id) {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("duplicate {label} ID {id}"),
            ));
        }
    }
    Ok(())
}

fn validate_strict_id_order<T, I, F>(
    values: &[T],
    mut id_of: F,
    label: &str,
) -> Result<(), CanwuError>
where
    I: Copy + Ord,
    F: FnMut(&T) -> I,
{
    if values
        .windows(2)
        .any(|pair| id_of(&pair[0]) >= id_of(&pair[1]))
    {
        return invalid_snapshot(format!("snapshot {label} are not in canonical ID order"));
    }
    Ok(())
}

fn field(name: &str, value_type: &str, description: &str) -> FieldSchema {
    FieldSchema {
        name: name.to_owned(),
        value_type: value_type.to_owned(),
        description: description.to_owned(),
        reference_type: None,
        writable_via_debug_command: false,
    }
}

fn base_schema() -> SchemaRegistry {
    let mut schema = SchemaRegistry::default();
    schema.register(TypeSchema {
        type_name: "person".to_owned(),
        description: "Historical actor with roles and a location".to_owned(),
        fields: vec![
            field("id", "PersonId", "Stable person identifier"),
            field("name", "String", "Display name"),
            field("government", "GovernmentId", "Government membership"),
            field("current_location", "TerritoryId", "Current territory"),
            field("roles", "Vec<String>", "Offices and authorities"),
        ],
    });
    schema.register(TypeSchema {
        type_name: "army".to_owned(),
        description: "Mobile military organization".to_owned(),
        fields: vec![
            field("id", "ArmyId", "Stable army identifier"),
            field("commander", "PersonId", "Commanding person"),
            field("location", "TerritoryId", "Ground-truth territory"),
            field("strength", "u32", "Ground-truth personnel strength"),
            FieldSchema {
                name: "morale".to_owned(),
                value_type: "u16".to_owned(),
                description: "Morale from 0 through 100".to_owned(),
                reference_type: None,
                writable_via_debug_command: true,
            },
            field("transit", "Option<TransitState>", "Pending movement"),
        ],
    });
    schema.register(TypeSchema {
        type_name: "territory".to_owned(),
        description: "Administrative and geographic unit".to_owned(),
        fields: vec![
            field("id", "TerritoryId", "Stable territory identifier"),
            field("controller", "GovernmentId", "Controlling government"),
            field("position", "MapPoint", "Abstract visualization point"),
        ],
    });
    schema.register(TypeSchema {
        type_name: "route".to_owned(),
        description: "Travel connection between territories".to_owned(),
        fields: vec![
            field("from", "TerritoryId", "First route endpoint"),
            field("to", "TerritoryId", "Second route endpoint"),
            field("travel_minutes", "i64", "Deterministic travel duration"),
            field("terrain", "String", "Terrain classification"),
        ],
    });
    schema.register(TypeSchema {
        type_name: "event".to_owned(),
        description: "Inspectable state-change or information event".to_owned(),
        fields: vec![field("timestamp", "SimTime", "Simulation occurrence time")],
    });
    schema
}

#[must_use]
pub fn demo_scenario() -> (Scenario, DemoIds) {
    let ids = DemoIds {
        commander: PersonId::new(1),
        observer: PersonId::new(2),
        government: GovernmentId::new(1),
        army: ArmyId::new(1),
        western_territory: TerritoryId::new(1),
        central_territory: TerritoryId::new(2),
        eastern_territory: TerritoryId::new(3),
    };
    let world = WorldSnapshot {
        people: vec![
            Person {
                id: ids.commander,
                name: "General Shen".to_owned(),
                government: ids.government,
                current_location: ids.central_territory,
                roles: vec!["army_commander".to_owned()],
            },
            Person {
                id: ids.observer,
                name: "Minister Luo".to_owned(),
                government: ids.government,
                current_location: ids.western_territory,
                roles: vec!["civil_minister".to_owned()],
            },
        ],
        governments: vec![Government {
            id: ids.government,
            name: "State of Yun".to_owned(),
            capital: ids.central_territory,
        }],
        territories: vec![
            Territory {
                id: ids.western_territory,
                name: "Westford".to_owned(),
                controller: ids.government,
                position: MapPoint { x: 80.0, y: 180.0 },
            },
            Territory {
                id: ids.central_territory,
                name: "Yun Capital".to_owned(),
                controller: ids.government,
                position: MapPoint { x: 240.0, y: 120.0 },
            },
            Territory {
                id: ids.eastern_territory,
                name: "Eastwatch".to_owned(),
                controller: ids.government,
                position: MapPoint { x: 420.0, y: 210.0 },
            },
        ],
        routes: vec![
            Route {
                id: RouteId::new(1),
                name: "Western Post Road".to_owned(),
                from: ids.western_territory,
                to: ids.central_territory,
                travel_minutes: SimDuration::hours(12).as_minutes(),
                terrain: "road".to_owned(),
            },
            Route {
                id: RouteId::new(2),
                name: "Eastern River Road".to_owned(),
                from: ids.central_territory,
                to: ids.eastern_territory,
                travel_minutes: SimDuration::hours(18).as_minutes(),
                terrain: "river_road".to_owned(),
            },
        ],
        armies: vec![Army {
            id: ids.army,
            name: "First Field Army".to_owned(),
            government: ids.government,
            commander: ids.commander,
            location: ids.central_territory,
            strength: 8_000,
            morale: 72,
            transit: None,
        }],
    };
    let initial_time = SimTime::EPOCH;
    let mut knowledge = KnowledgeSnapshot::default();
    knowledge.actors.insert(
        ids.commander,
        ActorKnowledge {
            actor: ids.commander,
            armies: BTreeMap::from([(
                ids.army,
                ArmyKnowledge {
                    army: ids.army,
                    known_name: Some("First Field Army".to_owned()),
                    known_location: Some(ids.central_territory),
                    estimated_strength: EstimateRange {
                        minimum: 8_000,
                        maximum: 8_000,
                    },
                    observed_at: initial_time,
                    learned_at: initial_time,
                    confidence_per_mille: 1000,
                    source: KnowledgeSource::CommandResponsibility,
                },
            )]),
        },
    );
    knowledge.actors.insert(
        ids.observer,
        ActorKnowledge {
            actor: ids.observer,
            armies: BTreeMap::from([(
                ids.army,
                ArmyKnowledge {
                    army: ids.army,
                    known_name: Some("First Field Army".to_owned()),
                    known_location: Some(ids.central_territory),
                    estimated_strength: EstimateRange {
                        minimum: 7_000,
                        maximum: 9_000,
                    },
                    observed_at: initial_time,
                    learned_at: initial_time,
                    confidence_per_mille: 700,
                    source: KnowledgeSource::ScenarioRecord,
                },
            )]),
        },
    );
    (
        Scenario {
            start_time: initial_time,
            world,
            knowledge,
            domain_records: Vec::new(),
        },
        ids,
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unnecessary_wraps)]

    use super::*;

    #[derive(Debug, Eq, PartialEq)]
    struct CacheFingerprint {
        journals: [String; 5],
        domains: [Option<String>; 7],
    }

    fn cache_fingerprint(simulation: &Simulation) -> CacheFingerprint {
        let cache = simulation
            .state
            .metadata
            .commitment_cache
            .as_ref()
            .expect("current runtimes should maintain a commitment cache");
        CacheFingerprint {
            journals: [
                cache.commands.root(),
                cache.attempts.root(),
                cache.events.root(),
                cache.ingress.root(),
                cache.random_draws.root(),
            ],
            domains: [
                cache.world.clone(),
                cache.knowledge.clone(),
                cache.plugin_components.clone(),
                cache.domain_records.clone(),
                cache.scheduler.clone(),
                cache.random_streams.clone(),
                cache.identity.clone(),
            ],
        }
    }

    macro_rules! test_plugin_identity {
        ($hash:literal) => {
            fn version(&self) -> &'static str {
                "test-v1"
            }

            fn semantic_hash(&self) -> &'static str {
                $hash
            }
        };
    }

    struct AuthorityPlugin;
    struct ChangedAuthorityPlugin;

    fn authority_command(
        view: &SimulationView<'_>,
        context: &CommandContext,
        _payload: &Value,
    ) -> Result<Vec<SystemDirective>, CanwuError> {
        let actor = PersonId::new(1);
        let army = ArmyId::new(1);
        if context.issuer != Issuer::Actor(actor) {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "the command issuer does not own this test action",
            ));
        }
        if view.army(army)?.is_none() {
            return Err(CanwuError::new(
                ErrorCode::ArmyNotFound,
                "the test army does not exist",
            ));
        }
        Ok(vec![SystemDirective::SetComponent {
            state: StateKey::new("military", "stance"),
            entity: EntityRef::Army(army),
            component: "stance".to_owned(),
            value: Value::String("hold".to_owned()),
            summary: "The authorized actor changed the army stance".to_owned(),
        }])
    }

    fn register_authority(registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_command(
            PluginActionDescriptor {
                name: "set_stance".to_owned(),
                description: "Set a test stance".to_owned(),
                payload_schema: PayloadSchema::Null,
                reads: vec![StateKey::core_armies()],
                writes: vec![StateKey::new("military", "stance")],
            },
            authority_command,
        )
    }

    impl SimulationPlugin for AuthorityPlugin {
        fn name(&self) -> &'static str {
            "authority-test"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000001");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            register_authority(registrar)
        }
    }

    impl SimulationPlugin for ChangedAuthorityPlugin {
        fn name(&self) -> &'static str {
            "authority-test"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000013");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            register_authority(registrar)
        }
    }

    struct MarkerPlugin {
        name: &'static str,
        writes: Vec<StateKey>,
    }

    fn marker_system(
        _view: &SimulationView<'_>,
        event: &SimEvent,
    ) -> Result<Vec<SystemDirective>, CanwuError> {
        if !matches!(event.kind, EventKind::MoveOrdered { .. }) {
            return Ok(Vec::new());
        }
        Ok(vec![SystemDirective::Emit {
            event_type: "marker".to_owned(),
            summary: "movement marker".to_owned(),
            affected: Vec::new(),
        }])
    }

    impl SimulationPlugin for MarkerPlugin {
        fn name(&self) -> &str {
            self.name
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000002");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            let mut contract = SystemContract::event_driven(
                "movement-marker",
                BoundaryPhase::PerspectiveAndReportMaterialization,
            );
            contract.writes.clone_from(&self.writes);
            registrar.register_system(contract, marker_system)
        }
    }

    struct FailingPlugin;

    fn failing_command(
        _view: &SimulationView<'_>,
        _context: &CommandContext,
        payload: &Value,
    ) -> Result<Vec<SystemDirective>, CanwuError> {
        let mutation = SystemDirective::SetComponent {
            state: StateKey::new("failure-fixture", "flag"),
            entity: EntityRef::Army(ArmyId::new(1)),
            component: "flag".to_owned(),
            value: Value::Bool(true),
            summary: "Set a flag before the injected failure".to_owned(),
        };
        if payload.get("scheduled").and_then(Value::as_bool) == Some(true) {
            Ok(vec![SystemDirective::Schedule {
                after: SimDuration::days(1),
                directive: Box::new(mutation),
            }])
        } else {
            Ok(vec![mutation])
        }
    }

    fn failing_event_system(
        _view: &SimulationView<'_>,
        event: &SimEvent,
    ) -> Result<Vec<SystemDirective>, CanwuError> {
        if matches!(
            &event.kind,
            EventKind::Plugin { plugin, event_type }
                if plugin == "failing-test" && event_type == "flag_changed"
        ) {
            Ok(vec![SystemDirective::Schedule {
                after: SimDuration::ZERO,
                directive: Box::new(SystemDirective::Emit {
                    event_type: "unreachable".to_owned(),
                    summary: "This directive must be rejected".to_owned(),
                    affected: Vec::new(),
                }),
            }])
        } else {
            Ok(Vec::new())
        }
    }

    fn panicking_command(
        _view: &SimulationView<'_>,
        _context: &CommandContext,
        _payload: &Value,
    ) -> Result<Vec<SystemDirective>, CanwuError> {
        panic!("injected plugin panic")
    }

    impl SimulationPlugin for FailingPlugin {
        fn name(&self) -> &'static str {
            "failing-test"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000003");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            registrar.register_system(
                SystemContract::event_driven(
                    "reject-flag-event",
                    BoundaryPhase::InvariantValidation,
                ),
                failing_event_system,
            )?;
            registrar.register_command(
                PluginActionDescriptor {
                    name: "mutate".to_owned(),
                    description: "Exercise transactional rollback".to_owned(),
                    payload_schema: PayloadSchema::Object {
                        properties: BTreeMap::from([(
                            "scheduled".to_owned(),
                            PayloadProperty {
                                value_type: PayloadValueType::Boolean,
                                required: true,
                            },
                        )]),
                        allow_additional: false,
                    },
                    reads: Vec::new(),
                    writes: vec![StateKey::new("failure-fixture", "flag")],
                },
                failing_command,
            )?;
            registrar.register_command(
                PluginActionDescriptor {
                    name: "panic".to_owned(),
                    description: "Exercise the plugin panic boundary".to_owned(),
                    payload_schema: PayloadSchema::Null,
                    reads: Vec::new(),
                    writes: Vec::new(),
                },
                panicking_command,
            )
        }
    }

    fn no_op_command(
        _view: &SimulationView<'_>,
        _context: &CommandContext,
        _payload: &Value,
    ) -> Result<Vec<SystemDirective>, CanwuError> {
        Ok(Vec::new())
    }

    fn no_op_boundary(
        _view: &SimulationView<'_>,
        _context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        Ok(BoundaryProposal::default())
    }

    struct JournalCommandPlugin;

    impl SimulationPlugin for JournalCommandPlugin {
        fn name(&self) -> &'static str {
            "journal-command"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000004");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            registrar.register_command(
                PluginActionDescriptor {
                    name: "noop".to_owned(),
                    description: "Append deterministic command evidence".to_owned(),
                    payload_schema: PayloadSchema::Null,
                    reads: Vec::new(),
                    writes: Vec::new(),
                },
                no_op_command,
            )
        }
    }

    struct BoundaryGhostPlugin;

    impl SimulationPlugin for BoundaryGhostPlugin {
        fn name(&self) -> &'static str {
            "boundary-ghost-test"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000005");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            registrar.register_command(
                PluginActionDescriptor {
                    name: "seed".to_owned(),
                    description: "Own immediate state for the conflict fixture".to_owned(),
                    payload_schema: PayloadSchema::Null,
                    reads: Vec::new(),
                    writes: vec![StateKey::new("boundary-conflict", "immediate")],
                },
                no_op_command,
            )?;
            let mut rejected = BoundarySystemContract::new(
                "rejected",
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::Daily,
            );
            rejected.writes = vec![
                StateKey::new("boundary-conflict", "immediate"),
                StateKey::new("boundary-ghost", "value"),
            ];
            if registrar
                .register_boundary_system(rejected, no_op_boundary)
                .is_ok()
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPluginRegistration,
                    "the boundary ghost fixture expected a writer-mode conflict",
                ));
            }
            Ok(())
        }
    }

    struct GhostPlugin;

    impl SimulationPlugin for GhostPlugin {
        fn name(&self) -> &'static str {
            "ghost-test"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000006");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            let ignored = registrar.register_command(
                PluginActionDescriptor {
                    name: "ignored".to_owned(),
                    description: "A deliberately rejected registration".to_owned(),
                    payload_schema: PayloadSchema::Null,
                    reads: Vec::new(),
                    writes: vec![
                        StateKey::new("fresh-domain", "value"),
                        StateKey::new("shared-domain", "balance"),
                    ],
                },
                no_op_command,
            );
            if ignored.is_ok() {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPluginRegistration,
                    "the ghost fixture expected an ownership conflict",
                ));
            }
            Ok(())
        }
    }

    fn seed_secret(
        _view: &SimulationView<'_>,
        _context: &CommandContext,
        _payload: &Value,
    ) -> Result<Vec<SystemDirective>, CanwuError> {
        Ok(vec![SystemDirective::SetComponent {
            state: StateKey::new("secret-domain", "value"),
            entity: EntityRef::Army(ArmyId::new(1)),
            component: "value".to_owned(),
            value: Value::String("classified".to_owned()),
            summary: "Seed classified state".to_owned(),
        }])
    }

    struct SecretPlugin;

    impl SimulationPlugin for SecretPlugin {
        fn name(&self) -> &'static str {
            "secret-owner"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000007");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            registrar.register_command(
                PluginActionDescriptor {
                    name: "seed".to_owned(),
                    description: "Seed owned state".to_owned(),
                    payload_schema: PayloadSchema::Null,
                    reads: Vec::new(),
                    writes: vec![StateKey::new("secret-domain", "value")],
                },
                seed_secret,
            )
        }
    }

    fn undeclared_read(
        view: &SimulationView<'_>,
        _context: &CommandContext,
        _payload: &Value,
    ) -> Result<Vec<SystemDirective>, CanwuError> {
        let _ = view.component(
            &StateKey::new("secret-domain", "value"),
            &EntityRef::Army(ArmyId::new(1)),
            "value",
        )?;
        Ok(Vec::new())
    }

    fn undeclared_write(
        _view: &SimulationView<'_>,
        _context: &CommandContext,
        _payload: &Value,
    ) -> Result<Vec<SystemDirective>, CanwuError> {
        Ok(vec![SystemDirective::SetComponent {
            state: StateKey::new("secret-domain", "value"),
            entity: EntityRef::Army(ArmyId::new(1)),
            component: "value".to_owned(),
            value: Value::String("overwritten".to_owned()),
            summary: "Attempt an undeclared write".to_owned(),
        }])
    }

    fn missing_entity_write(
        _view: &SimulationView<'_>,
        _context: &CommandContext,
        _payload: &Value,
    ) -> Result<Vec<SystemDirective>, CanwuError> {
        Ok(vec![SystemDirective::SetComponent {
            state: StateKey::new("access-domain", "declared"),
            entity: EntityRef::Army(ArmyId::new(999)),
            component: "declared".to_owned(),
            value: Value::Bool(true),
            summary: "Attempt to write state for a missing entity".to_owned(),
        }])
    }

    struct UndeclaredAccessPlugin;

    impl SimulationPlugin for UndeclaredAccessPlugin {
        fn name(&self) -> &'static str {
            "undeclared-access"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000008");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            registrar.register_command(
                PluginActionDescriptor {
                    name: "missing".to_owned(),
                    description: "Attempt to target a missing entity".to_owned(),
                    payload_schema: PayloadSchema::Null,
                    reads: Vec::new(),
                    writes: vec![StateKey::new("access-domain", "declared")],
                },
                missing_entity_write,
            )?;
            registrar.register_command(
                PluginActionDescriptor {
                    name: "read".to_owned(),
                    description: "Attempt an undeclared read".to_owned(),
                    payload_schema: PayloadSchema::Null,
                    reads: Vec::new(),
                    writes: Vec::new(),
                },
                undeclared_read,
            )?;
            registrar.register_command(
                PluginActionDescriptor {
                    name: "write".to_owned(),
                    description: "Attempt an undeclared write".to_owned(),
                    payload_schema: PayloadSchema::Null,
                    reads: Vec::new(),
                    writes: vec![StateKey::new("access-domain", "declared")],
                },
                undeclared_write,
            )
        }
    }

    fn collision_a(
        _view: &SimulationView<'_>,
        _context: &CommandContext,
        _payload: &Value,
    ) -> Result<Vec<SystemDirective>, CanwuError> {
        Ok(vec![SystemDirective::SetComponent {
            state: StateKey::new("collision-a", "b/person:1/c"),
            entity: EntityRef::Person(PersonId::new(1)),
            component: "b/person:1/c".to_owned(),
            value: Value::String("first".to_owned()),
            summary: "Write the first adversarial key".to_owned(),
        }])
    }

    fn collision_b(
        _view: &SimulationView<'_>,
        _context: &CommandContext,
        _payload: &Value,
    ) -> Result<Vec<SystemDirective>, CanwuError> {
        Ok(vec![SystemDirective::SetComponent {
            state: StateKey::new("collision-b", "c"),
            entity: EntityRef::Person(PersonId::new(1)),
            component: "c".to_owned(),
            value: Value::String("second".to_owned()),
            summary: "Write the second adversarial key".to_owned(),
        }])
    }

    struct CollisionPluginA;

    struct CollisionPluginB;

    impl SimulationPlugin for CollisionPluginA {
        fn name(&self) -> &'static str {
            "a"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000009");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            registrar.register_command(
                PluginActionDescriptor {
                    name: "write".to_owned(),
                    description: "Write an adversarial component key".to_owned(),
                    payload_schema: PayloadSchema::Null,
                    reads: Vec::new(),
                    writes: vec![StateKey::new("collision-a", "b/person:1/c")],
                },
                collision_a,
            )
        }
    }

    impl SimulationPlugin for CollisionPluginB {
        fn name(&self) -> &'static str {
            "a/person:1/b"
        }

        test_plugin_identity!("000000000000000000000000000000000000000000000000000000000000000a");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            registrar.register_command(
                PluginActionDescriptor {
                    name: "write".to_owned(),
                    description: "Write a second adversarial component key".to_owned(),
                    payload_schema: PayloadSchema::Null,
                    reads: Vec::new(),
                    writes: vec![StateKey::new("collision-b", "c")],
                },
                collision_b,
            )
        }
    }

    fn grain_pool() -> ReservationPoolKey {
        ReservationPoolKey::new(
            StateKey::new("logistics", "grain"),
            EntityRef::Territory(TerritoryId::new(1)),
            "grain",
        )
    }

    fn primary_random_stream() -> RandomStreamKey {
        RandomStreamKey::new("random-primary", "daily-roll", 1)
    }

    fn noise_random_stream() -> RandomStreamKey {
        RandomStreamKey::new("random-noise", "daily-noise", 1)
    }

    fn failure_random_stream() -> RandomStreamKey {
        RandomStreamKey::new("boundary-rollback", "rollback-proof", 1)
    }

    fn roll_primary(
        view: &SimulationView<'_>,
        _context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        let roll = view.random_range(&primary_random_stream(), 100, "daily primary roll")?;
        Ok(BoundaryProposal {
            directives: vec![BoundaryDirective::SetComponent {
                state: StateKey::new("random-primary", "roll"),
                entity: EntityRef::Territory(TerritoryId::new(1)),
                component: "value".to_owned(),
                value: Value::from(roll),
                summary: format!("Primary random stream rolled {roll}"),
            }],
            ..BoundaryProposal::default()
        })
    }

    fn draw_noise(
        view: &SimulationView<'_>,
        _context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        let _ = view.random_range(&noise_random_stream(), 10_000, "unrelated daily noise")?;
        Ok(BoundaryProposal::default())
    }

    struct PrimaryRandomPlugin;
    struct ChangedPrimaryRandomPlugin;
    struct NoiseRandomPlugin;

    fn register_primary_random(registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut contract = BoundarySystemContract::new(
            "roll",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        contract.writes = vec![StateKey::new("random-primary", "roll")];
        contract.random_streams = vec![primary_random_stream()];
        registrar.register_boundary_system(contract, roll_primary)
    }

    impl SimulationPlugin for PrimaryRandomPlugin {
        fn name(&self) -> &'static str {
            "random-primary"
        }

        test_plugin_identity!("000000000000000000000000000000000000000000000000000000000000000b");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            register_primary_random(registrar)
        }
    }

    impl SimulationPlugin for ChangedPrimaryRandomPlugin {
        fn name(&self) -> &'static str {
            "random-primary"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000012");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            register_primary_random(registrar)
        }
    }

    impl SimulationPlugin for NoiseRandomPlugin {
        fn name(&self) -> &'static str {
            "random-noise"
        }

        test_plugin_identity!("000000000000000000000000000000000000000000000000000000000000000c");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            let mut contract = BoundarySystemContract::new(
                "draw",
                BoundaryPhase::DerivedFieldSolve,
                SystemCadence::Daily,
            );
            contract.random_streams = vec![noise_random_stream()];
            registrar.register_boundary_system(contract, draw_noise)
        }
    }

    fn offer_grain(
        _view: &SimulationView<'_>,
        _context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        Ok(BoundaryProposal {
            offers: vec![ReservationOffer {
                pool: grain_pool(),
                capacity: 10,
            }],
            ..BoundaryProposal::default()
        })
    }

    fn high_request(
        _view: &SimulationView<'_>,
        _context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        Ok(BoundaryProposal {
            requests: vec![ReservationRequest {
                request: "grain".to_owned(),
                pool: grain_pool(),
                quantity: 7,
                priority: 10,
                tie_break: "high".to_owned(),
            }],
            ..BoundaryProposal::default()
        })
    }

    fn low_request(
        _view: &SimulationView<'_>,
        _context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        Ok(BoundaryProposal {
            requests: vec![ReservationRequest {
                request: "grain".to_owned(),
                pool: grain_pool(),
                quantity: 7,
                priority: 0,
                tie_break: "low".to_owned(),
            }],
            ..BoundaryProposal::default()
        })
    }

    fn record_grant(
        view: &SimulationView<'_>,
        context: &BoundaryContext,
        plugin: &str,
        state: StateKey,
        component: &str,
    ) -> Result<BoundaryProposal, CanwuError> {
        let reservation = ReservationRef::new(plugin, "request", "grain");
        let allocation = view.reservation(&reservation)?.ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidBoundary,
                format!(
                    "{} could not find allocation {reservation:?}",
                    context.system
                ),
            )
        })?;
        Ok(BoundaryProposal {
            directives: vec![BoundaryDirective::SetComponent {
                state,
                entity: EntityRef::Territory(TerritoryId::new(1)),
                component: component.to_owned(),
                value: Value::from(allocation.granted),
                summary: format!("Recorded a grant of {} grain", allocation.granted),
            }],
            ..BoundaryProposal::default()
        })
    }

    fn record_high_grant(
        view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        record_grant(
            view,
            context,
            "high-claim",
            StateKey::new("allocation", "high"),
            "high",
        )
    }

    fn record_low_grant(
        view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        record_grant(
            view,
            context,
            "low-claim",
            StateKey::new("allocation", "low"),
            "low",
        )
    }

    fn validate_visibility(
        view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        let entity = EntityRef::Territory(TerritoryId::new(1));
        let high = view
            .component(&StateKey::new("allocation", "high"), &entity, "high")?
            .and_then(Value::as_u64);
        let low = view
            .component(&StateKey::new("allocation", "low"), &entity, "low")?
            .and_then(Value::as_u64);
        let proposed_high = view
            .proposed_component(&StateKey::new("allocation", "high"), &entity, "high")?
            .and_then(Value::as_u64);
        let proposed_low = view
            .proposed_component(&StateKey::new("allocation", "low"), &entity, "low")?
            .and_then(Value::as_u64);
        let expected_current_low = (context.boundary_id.get() > 1).then_some(3);
        if high != Some(7)
            || low != expected_current_low
            || proposed_high != Some(7)
            || proposed_low != Some(3)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "validators must see all proposals without exposing next-boundary state as current",
            ));
        }
        Ok(BoundaryProposal::default())
    }

    struct GrainSupplyPlugin;
    struct HighClaimPlugin;
    struct LowClaimPlugin;
    struct VisibilityValidatorPlugin;

    impl SimulationPlugin for GrainSupplyPlugin {
        fn name(&self) -> &'static str {
            "grain-supply"
        }

        test_plugin_identity!("000000000000000000000000000000000000000000000000000000000000000d");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            let mut contract = BoundarySystemContract::new(
                "offer",
                BoundaryPhase::ReservationAndAllocation,
                SystemCadence::Daily,
            );
            contract.reservation_offers = vec![StateKey::new("logistics", "grain")];
            registrar.register_boundary_system(contract, offer_grain)
        }
    }

    impl SimulationPlugin for HighClaimPlugin {
        fn name(&self) -> &'static str {
            "high-claim"
        }

        test_plugin_identity!("000000000000000000000000000000000000000000000000000000000000000e");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            let mut request = BoundarySystemContract::new(
                "request",
                BoundaryPhase::ReservationAndAllocation,
                SystemCadence::Daily,
            );
            request.reservation_requests = vec![StateKey::new("logistics", "grain")];
            registrar.register_boundary_system(request, high_request)?;
            let mut apply = BoundarySystemContract::new(
                "apply",
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::Daily,
            );
            apply.writes = vec![StateKey::new("allocation", "high")];
            apply.reservation_reads = vec![ReservationRef::new("high-claim", "request", "grain")];
            apply.visibility = StateVisibility::SameBoundary;
            registrar.register_boundary_system(apply, record_high_grant)
        }
    }

    impl SimulationPlugin for LowClaimPlugin {
        fn name(&self) -> &'static str {
            "low-claim"
        }

        test_plugin_identity!("000000000000000000000000000000000000000000000000000000000000000f");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            let mut request = BoundarySystemContract::new(
                "request",
                BoundaryPhase::ReservationAndAllocation,
                SystemCadence::Daily,
            );
            request.reservation_requests = vec![StateKey::new("logistics", "grain")];
            registrar.register_boundary_system(request, low_request)?;
            let mut apply = BoundarySystemContract::new(
                "apply",
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::Daily,
            );
            apply.writes = vec![StateKey::new("allocation", "low")];
            apply.reservation_reads = vec![ReservationRef::new("low-claim", "request", "grain")];
            registrar.register_boundary_system(apply, record_low_grant)
        }
    }

    impl SimulationPlugin for VisibilityValidatorPlugin {
        fn name(&self) -> &'static str {
            "visibility-validator"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000010");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            let mut contract = BoundarySystemContract::new(
                "validate",
                BoundaryPhase::InvariantValidation,
                SystemCadence::Daily,
            );
            contract.reads = vec![
                StateKey::new("allocation", "high"),
                StateKey::new("allocation", "low"),
            ];
            registrar.register_boundary_system(contract, validate_visibility)
        }
    }

    fn stage_boundary_rollback_mutations(
        view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        if context.boundary_id.get() != 2 {
            return Ok(BoundaryProposal::default());
        }
        let _ = view.random_range(&failure_random_stream(), 100, "rollback proof")?;
        Ok(BoundaryProposal {
            directives: vec![
                BoundaryDirective::SetComponent {
                    state: StateKey::new("boundary-rollback", "value"),
                    entity: EntityRef::Army(ArmyId::new(1)),
                    component: "value".to_owned(),
                    value: Value::Bool(true),
                    summary: "Stage a value before transaction failure".to_owned(),
                },
                BoundaryDirective::ScheduleIngress {
                    after: SimDuration::hours(1),
                    packet_type: "follow-up".to_owned(),
                    priority: 0,
                    payload: serde_json::json!({ "label": "rollback proof" }),
                    affected: vec![EntityRef::Army(ArmyId::new(1))],
                },
            ],
            ..BoundaryProposal::default()
        })
    }

    struct BoundaryRollbackPlugin;

    impl SimulationPlugin for BoundaryRollbackPlugin {
        fn name(&self) -> &'static str {
            "boundary-rollback"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000011");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            registrar.register_ingress(PluginIngressDescriptor {
                name: "follow-up".to_owned(),
                description: "A rollback fixture packet".to_owned(),
                class: IngressClass::Information,
                payload_schema: object_payload_schema("label"),
            })?;
            let mut propose = BoundarySystemContract::new(
                "propose",
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::Daily,
            );
            propose.writes = vec![StateKey::new("boundary-rollback", "value")];
            propose.random_streams = vec![failure_random_stream()];
            propose.visibility = StateVisibility::SameBoundary;
            registrar.register_boundary_system(propose, stage_boundary_rollback_mutations)
        }
    }

    struct RecordLifecyclePlugin;
    struct RecordDeleteOnlyPlugin;
    struct RecordCyclePlugin;
    struct RecordSeatDeletionPlugin;

    fn office_kind() -> DomainRecordKind {
        DomainRecordKind::new("cm-fixture", "office")
    }

    fn obligation_kind() -> DomainRecordKind {
        DomainRecordKind::new("cm-fixture", "obligation")
    }

    fn office_reference(id: &str) -> DomainRecordRef {
        DomainRecordRef::new("cm-fixture", "office", id)
    }

    fn obligation_reference() -> DomainRecordRef {
        DomainRecordRef::new("cm-fixture", "obligation", "standing-order")
    }

    fn object_payload_schema(field: &str) -> PayloadSchema {
        PayloadSchema::Object {
            properties: BTreeMap::from([(
                field.to_owned(),
                PayloadProperty {
                    value_type: PayloadValueType::String,
                    required: true,
                },
            )]),
            allow_additional: false,
        }
    }

    fn office_draft(id: &str, name: &str) -> DomainRecordDraft {
        DomainRecordDraft {
            reference: office_reference(id),
            payload: serde_json::json!({ "name": name }),
            references: vec![DomainReference {
                role: "holder".to_owned(),
                target: DomainReferenceTarget::Core(EntityRef::Person(PersonId::new(1))),
            }],
        }
    }

    fn obligation_draft(office: &str, status: &str) -> DomainRecordDraft {
        DomainRecordDraft {
            reference: obligation_reference(),
            payload: serde_json::json!({ "status": status }),
            references: vec![DomainReference {
                role: "office".to_owned(),
                target: DomainReferenceTarget::Domain(office_reference(office)),
            }],
        }
    }

    fn initial_record(
        owner: &str,
        class: DomainRecordClass,
        draft: DomainRecordDraft,
    ) -> DomainRecord {
        DomainRecord {
            reference: draft.reference,
            owner: owner.to_owned(),
            class,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: draft.payload,
            references: draft.references,
        }
    }

    fn rehash_tampered_snapshot(snapshot: &mut SimulationSnapshot) {
        let mut previous_hash = GENESIS_BOUNDARY_HASH.to_owned();
        for boundary in &mut snapshot.boundaries {
            boundary.previous_hash.clone_from(&previous_hash);
            boundary.hash =
                compute_boundary_hash(boundary).expect("tampered boundary should still hash");
            previous_hash.clone_from(&boundary.hash);
        }
        refresh_snapshot_commitments_and_checkpoint(snapshot);
    }

    fn refresh_snapshot_commitments_and_checkpoint(snapshot: &mut SimulationSnapshot) {
        if snapshot.commitment_format_version == COMMITMENT_FORMAT_VERSION {
            snapshot.commitment_roots = Some(
                snapshot_commitment_roots(snapshot)
                    .expect("snapshot domains should produce commitment roots"),
            );
        }
        snapshot.checkpoint_hash = snapshot_checkpoint_hash(snapshot)
            .expect("tampered snapshot should still have a coherent outer commitment");
    }

    fn downgrade_snapshot_commitments(snapshot: &mut SimulationSnapshot) {
        snapshot.commitment_format_version = 0;
        snapshot.commitment_roots = None;
    }

    fn record_lifecycle_proposal(context: &BoundaryContext, delete_only: bool) -> BoundaryProposal {
        let directives = match context.boundary_id.get() {
            1 => vec![
                BoundaryDirective::MutateRecord {
                    mutation: DomainRecordMutation::Create {
                        record: office_draft("office-a", "Grand Secretariat"),
                    },
                    summary: "Create the original office".to_owned(),
                },
                BoundaryDirective::MutateRecord {
                    mutation: DomainRecordMutation::Create {
                        record: office_draft("office-b", "Successor Secretariat"),
                    },
                    summary: "Create the successor office".to_owned(),
                },
                BoundaryDirective::MutateRecord {
                    mutation: DomainRecordMutation::Create {
                        record: obligation_draft("office-a", "open"),
                    },
                    summary: "Create an obligation assigned to the original office".to_owned(),
                },
                BoundaryDirective::SetComponent {
                    state: StateKey::new("cm-fixture", "marker"),
                    entity: EntityRef::Domain(office_reference("office-b")),
                    component: "status".to_owned(),
                    value: Value::String("created".to_owned()),
                    summary: "Mark the successor office as created".to_owned(),
                },
            ],
            2 => vec![
                BoundaryDirective::MutateRecord {
                    mutation: DomainRecordMutation::Create {
                        record: office_draft("office-c", "Later Secretariat"),
                    },
                    summary: "Create the later successor office".to_owned(),
                },
                BoundaryDirective::MutateRecord {
                    mutation: DomainRecordMutation::Retire {
                        record: office_reference("office-a"),
                        expected_version: 1,
                        successor: Some(office_reference("office-b")),
                    },
                    summary: "Retire the original office with a stable successor".to_owned(),
                },
            ],
            3 if delete_only => vec![BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Delete {
                    record: office_reference("office-a"),
                    expected_version: 2,
                },
                summary: "Attempt to delete a still-referenced office".to_owned(),
            }],
            3 => vec![BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Retire {
                    record: office_reference("office-b"),
                    expected_version: 1,
                    successor: Some(office_reference("office-c")),
                },
                summary: "Extend the persisted office succession chain".to_owned(),
            }],
            4 => vec![
                BoundaryDirective::MutateRecord {
                    mutation: DomainRecordMutation::Update {
                        record: obligation_draft("office-c", "transferred"),
                        expected_version: 1,
                    },
                    summary: "Transfer the obligation to the successor office".to_owned(),
                },
                BoundaryDirective::MutateRecord {
                    mutation: DomainRecordMutation::Delete {
                        record: office_reference("office-a"),
                        expected_version: 2,
                    },
                    summary: "Delete the unreferenced retired office".to_owned(),
                },
            ],
            5 => vec![BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Update {
                    record: office_draft("office-c", "Stale Secretariat"),
                    expected_version: 99,
                },
                summary: "Attempt a stale office update".to_owned(),
            }],
            _ => Vec::new(),
        };
        BoundaryProposal {
            directives,
            ..BoundaryProposal::default()
        }
    }

    fn apply_record_lifecycle(
        _view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        Ok(record_lifecycle_proposal(context, false))
    }

    fn apply_invalid_record_delete(
        _view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        Ok(record_lifecycle_proposal(context, true))
    }

    fn observe_record_proposal(
        _view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        let directives = (context.boundary_id.get() == 1)
            .then(|| BoundaryDirective::Emit {
                event_type: "proposal_probe".to_owned(),
                affected: vec![EntityRef::Person(PersonId::new(1))],
                summary: "Observe the record proposal boundary".to_owned(),
            })
            .into_iter()
            .collect();
        Ok(BoundaryProposal {
            directives,
            ..BoundaryProposal::default()
        })
    }

    fn validate_record_lifecycle_view(
        view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        let original = view.domain_record(&office_reference("office-a"))?;
        let proposed_successor = view.proposed_domain_record(&office_reference("office-b"))?;
        let obligation = view.domain_record(&obligation_reference())?;
        let valid = match context.boundary_id.get() {
            1 => {
                original.is_some_and(DomainRecord::is_active)
                    && obligation.is_some_and(DomainRecord::is_active)
            }
            2 => original.is_some_and(|record| {
                matches!(
                    &record.lifecycle,
                    DomainRecordLifecycle::Retired {
                        successor: Some(successor),
                        ..
                    } if successor == &office_reference("office-b")
                )
            }),
            3 => {
                original.is_some_and(|record| {
                    matches!(
                        &record.lifecycle,
                        DomainRecordLifecycle::Retired {
                            successor: Some(successor),
                            ..
                        } if successor == &office_reference("office-b")
                    )
                }) && proposed_successor.is_some_and(|record| {
                    matches!(
                        &record.lifecycle,
                        DomainRecordLifecycle::Retired {
                            successor: Some(successor),
                            ..
                        } if successor == &office_reference("office-c")
                    )
                })
            }
            4 => {
                original.is_some_and(DomainRecord::is_deleted)
                    && obligation.is_some_and(|record| {
                        record.references.iter().any(|reference| {
                            reference.target
                                == DomainReferenceTarget::Domain(office_reference("office-c"))
                        })
                    })
            }
            _ => true,
        };
        if !valid {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "invariant systems did not receive the deterministic domain-record proposal",
            ));
        }
        Ok(BoundaryProposal::default())
    }

    fn register_record_fixture(
        registrar: &mut PluginRegistrar<'_>,
        handler: BoundarySystemHandler,
    ) -> Result<(), CanwuError> {
        let mut writes = register_record_schemas(registrar)?;
        writes.push(StateKey::new("cm-fixture", "marker"));

        let mut lifecycle = BoundarySystemContract::new(
            "lifecycle",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        lifecycle.writes.clone_from(&writes);
        lifecycle.emits = vec!["record_probe".to_owned()];
        lifecycle.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(lifecycle, handler)?;

        let mut observer = BoundarySystemContract::new(
            "observer",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        observer.emits = vec!["proposal_probe".to_owned()];
        observer.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(observer, observe_record_proposal)?;

        let mut invariant = BoundarySystemContract::new(
            "validate-lifecycle",
            BoundaryPhase::InvariantValidation,
            SystemCadence::Daily,
        );
        invariant.reads = writes;
        registrar.register_boundary_system(invariant, validate_record_lifecycle_view)
    }

    fn register_record_schemas(
        registrar: &mut PluginRegistrar<'_>,
    ) -> Result<Vec<StateKey>, CanwuError> {
        let mut office = DomainRecordSchema::new(office_kind(), DomainRecordClass::Entity);
        office.payload_schema = object_payload_schema("name");
        office.references = vec![DomainReferenceSchema {
            role: "holder".to_owned(),
            targets: vec![DomainReferenceTargetKind::Core(
                canwu_core::CoreEntityKind::Person,
            )],
            required: true,
            multiple: false,
            allow_retired: false,
        }];
        let office_state = office.state_key();
        registrar.register_record_schema(office)?;

        let mut obligation = DomainRecordSchema::new(obligation_kind(), DomainRecordClass::Record);
        obligation.payload_schema = object_payload_schema("status");
        obligation.references = vec![DomainReferenceSchema {
            role: "office".to_owned(),
            targets: vec![DomainReferenceTargetKind::Domain(office_kind())],
            required: true,
            multiple: false,
            allow_retired: true,
        }];
        let obligation_state = obligation.state_key();
        registrar.register_record_schema(obligation)?;
        Ok(vec![office_state, obligation_state])
    }

    impl SimulationPlugin for RecordLifecyclePlugin {
        fn name(&self) -> &'static str {
            "cm-record-lifecycle"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000021");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            register_record_fixture(registrar, apply_record_lifecycle)
        }
    }

    impl SimulationPlugin for RecordDeleteOnlyPlugin {
        fn name(&self) -> &'static str {
            "cm-record-delete-only"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000022");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            register_record_fixture(registrar, apply_invalid_record_delete)
        }
    }

    fn apply_record_cycle(
        _view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        let directives = match context.boundary_id.get() {
            1 => vec![
                BoundaryDirective::MutateRecord {
                    mutation: DomainRecordMutation::Create {
                        record: office_draft("office-a", "First Office"),
                    },
                    summary: "Create the first office".to_owned(),
                },
                BoundaryDirective::MutateRecord {
                    mutation: DomainRecordMutation::Create {
                        record: office_draft("office-b", "Second Office"),
                    },
                    summary: "Create the second office".to_owned(),
                },
            ],
            2 => vec![
                BoundaryDirective::MutateRecord {
                    mutation: DomainRecordMutation::Retire {
                        record: office_reference("office-a"),
                        expected_version: 1,
                        successor: Some(office_reference("office-b")),
                    },
                    summary: "Attempt the first half of a successor cycle".to_owned(),
                },
                BoundaryDirective::MutateRecord {
                    mutation: DomainRecordMutation::Retire {
                        record: office_reference("office-b"),
                        expected_version: 1,
                        successor: Some(office_reference("office-a")),
                    },
                    summary: "Attempt the second half of a successor cycle".to_owned(),
                },
            ],
            _ => Vec::new(),
        };
        Ok(BoundaryProposal {
            directives,
            ..BoundaryProposal::default()
        })
    }

    impl SimulationPlugin for RecordCyclePlugin {
        fn name(&self) -> &'static str {
            "cm-record-cycle"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000023");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            let Some(office_state) = register_record_schemas(registrar)?.into_iter().next() else {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPluginRegistration,
                    "record cycle fixture is missing its office state",
                ));
            };
            let mut cycle = BoundarySystemContract::new(
                "cycle",
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::Daily,
            );
            cycle.writes = vec![office_state];
            cycle.visibility = StateVisibility::SameBoundary;
            registrar.register_boundary_system(cycle, apply_record_cycle)
        }
    }

    fn apply_record_seat_deletion(
        _view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        let directives = match context.boundary_id.get() {
            1 => vec![BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Retire {
                    record: office_reference("office-a"),
                    expected_version: 1,
                    successor: None,
                },
                summary: "Retire the institution-bound office".to_owned(),
            }],
            2 => vec![BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Delete {
                    record: office_reference("office-a"),
                    expected_version: 2,
                },
                summary: "Delete the retired institution-bound office".to_owned(),
            }],
            _ => Vec::new(),
        };
        Ok(BoundaryProposal {
            directives,
            ..BoundaryProposal::default()
        })
    }

    impl SimulationPlugin for RecordSeatDeletionPlugin {
        fn name(&self) -> &'static str {
            "cm-record-seat-deletion"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000024");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            let Some(office_state) = register_record_schemas(registrar)?.into_iter().next() else {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPluginRegistration,
                    "seat deletion fixture is missing its office state",
                ));
            };
            let mut lifecycle = BoundarySystemContract::new(
                "seat-deletion",
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::Daily,
            );
            lifecycle.writes = vec![office_state];
            lifecycle.visibility = StateVisibility::SameBoundary;
            registrar.register_boundary_system(lifecycle, apply_record_seat_deletion)
        }
    }

    struct CanonicalIngressPlugin;

    fn ingress_class_name(class: IngressClass) -> &'static str {
        match class {
            IngressClass::Command => "command",
            IngressClass::Communication => "communication",
            IngressClass::Acknowledgement => "acknowledgement",
            IngressClass::Information => "information",
            IngressClass::ScheduledSystem => "scheduled_system",
        }
    }

    fn consume_canonical_ingress(
        view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        for value in 1..=32 {
            let id = IngressId::new(value);
            if !context.admitted_ingress.contains(&id) && view.ingress(id)?.is_some() {
                return Err(CanwuError::new(
                    ErrorCode::InvalidBoundary,
                    "boundary systems must not observe ingress before admission",
                ));
            }
        }
        let mut order = Vec::new();
        for ingress_id in &context.admitted_ingress {
            let Some(record) = view.ingress(*ingress_id)? else {
                continue;
            };
            let IngressPayload::Plugin {
                plugin,
                packet_type,
                ..
            } = &record.payload
            else {
                continue;
            };
            if plugin == "canonical-ingress" {
                order.push(format!(
                    "{}:{packet_type}:{}",
                    ingress_class_name(record.class),
                    record.priority
                ));
            }
        }
        if order.is_empty() {
            return Ok(BoundaryProposal::default());
        }
        Ok(BoundaryProposal {
            directives: vec![BoundaryDirective::SetComponent {
                state: StateKey::new("ingress-fixture", "received"),
                entity: EntityRef::Person(PersonId::new(1)),
                component: "canonical-order".to_owned(),
                value: serde_json::json!(order),
                summary: "Record canonical ingress order".to_owned(),
            }],
            ..BoundaryProposal::default()
        })
    }

    fn mark_daily_calendar(
        _view: &SimulationView<'_>,
        _context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        Ok(BoundaryProposal {
            directives: vec![BoundaryDirective::SetComponent {
                state: StateKey::new("ingress-fixture", "calendar"),
                entity: EntityRef::Person(PersonId::new(1)),
                component: "daily".to_owned(),
                value: Value::Bool(true),
                summary: "Run the queued daily calendar boundary".to_owned(),
            }],
            ..BoundaryProposal::default()
        })
    }

    impl SimulationPlugin for CanonicalIngressPlugin {
        fn name(&self) -> &'static str {
            "canonical-ingress"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000025");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            for (name, description, class) in [
                (
                    "dispatch",
                    "A command or communication packet in transit",
                    IngressClass::Communication,
                ),
                (
                    "ack",
                    "A deterministic command acknowledgement",
                    IngressClass::Acknowledgement,
                ),
                (
                    "report",
                    "A deterministic information packet",
                    IngressClass::Information,
                ),
            ] {
                registrar.register_ingress(PluginIngressDescriptor {
                    name: name.to_owned(),
                    description: description.to_owned(),
                    class,
                    payload_schema: object_payload_schema("label"),
                })?;
            }
            let mut consumer = BoundarySystemContract::new(
                "consume-ingress",
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::EventDriven,
            );
            consumer.reads = vec![StateKey::core_ingress()];
            consumer.writes = vec![StateKey::new("ingress-fixture", "received")];
            consumer.visibility = StateVisibility::SameBoundary;
            registrar.register_boundary_system(consumer, consume_canonical_ingress)?;

            let mut calendar = BoundarySystemContract::new(
                "daily-calendar",
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::Daily,
            );
            calendar.writes = vec![StateKey::new("ingress-fixture", "calendar")];
            calendar.visibility = StateVisibility::SameBoundary;
            registrar.register_boundary_system(calendar, mark_daily_calendar)
        }
    }

    struct GeneratedIngressPlugin;

    fn relay_generated_ingress(
        view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        let mut directives = Vec::new();
        for ingress_id in &context.admitted_ingress {
            let Some(record) = view.ingress(*ingress_id)? else {
                return Err(CanwuError::new(
                    ErrorCode::InvalidBoundary,
                    "generated-ingress context references a missing record",
                ));
            };
            let IngressPayload::Plugin {
                plugin,
                packet_type,
                affected_entities,
                ..
            } = &record.payload
            else {
                continue;
            };
            if plugin != "generated-ingress" {
                continue;
            }
            match packet_type.as_str() {
                "dispatch" => directives.push(BoundaryDirective::ScheduleIngress {
                    after: SimDuration::ZERO,
                    packet_type: "ack".to_owned(),
                    priority: 5,
                    payload: serde_json::json!({ "label": "automatic acknowledgement" }),
                    affected: affected_entities.clone(),
                }),
                "ack" => directives.push(BoundaryDirective::SetComponent {
                    state: StateKey::new("generated-ingress-fixture", "received"),
                    entity: EntityRef::Person(PersonId::new(1)),
                    component: "acknowledged".to_owned(),
                    value: Value::Bool(true),
                    summary: "Record the automatically generated acknowledgement".to_owned(),
                }),
                _ => {}
            }
        }
        Ok(BoundaryProposal {
            directives,
            ..BoundaryProposal::default()
        })
    }

    impl SimulationPlugin for GeneratedIngressPlugin {
        fn name(&self) -> &'static str {
            "generated-ingress"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000026");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            registrar.register_ingress(PluginIngressDescriptor {
                name: "dispatch".to_owned(),
                description: "A communication packet that requires acknowledgement".to_owned(),
                class: IngressClass::Communication,
                payload_schema: object_payload_schema("label"),
            })?;
            registrar.register_ingress(PluginIngressDescriptor {
                name: "ack".to_owned(),
                description: "A boundary-generated acknowledgement".to_owned(),
                class: IngressClass::Acknowledgement,
                payload_schema: object_payload_schema("label"),
            })?;
            let mut relay = BoundarySystemContract::new(
                "relay-ingress",
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::EventDriven,
            );
            relay.reads = vec![StateKey::core_ingress()];
            relay.writes = vec![StateKey::new("generated-ingress-fixture", "received")];
            relay.visibility = StateVisibility::SameBoundary;
            registrar.register_boundary_system(relay, relay_generated_ingress)
        }
    }

    fn move_order(ids: &DemoIds) -> CommandEnvelope {
        CommandEnvelope::new(
            Issuer::Actor(ids.commander),
            Command::MoveArmy {
                army: ids.army,
                destination: ids.eastern_territory,
            },
        )
    }

    fn manifest_for_configuration(
        scenario: &Scenario,
        configuration: &RunConfiguration,
    ) -> RunManifest {
        let scenario_manifest =
            ArtifactManifest::for_scenario("cm", "policy-fixture", "1", scenario)
                .expect("scenario identity should hash");
        let configuration_manifest =
            ArtifactManifest::for_run_configuration("cm", "run-configuration", "1", configuration)
                .expect("run configuration identity should hash");
        RunManifest::declared(scenario_manifest, configuration_manifest)
    }

    fn character_authority(
        actor: PersonId,
        army: ArmyId,
        seat_id: &str,
        permission_profile_id: &str,
    ) -> CommandAuthority {
        CommandAuthority {
            decision_origin: DecisionOrigin::Actor { actor },
            seat_id: Some(seat_id.to_owned()),
            permission_profile_id: Some(permission_profile_id.to_owned()),
            command_subject: Some(EntityRef::Army(army)),
        }
    }

    #[test]
    fn deterministic_seed_and_event_order_survive_equal_runs() {
        let (scenario, ids) = demo_scenario();
        let mut first = Simulation::new(35, scenario.clone()).expect("demo should load");
        first
            .submit(move_order(&ids))
            .expect("order should validate");
        first
            .advance(SimDuration::days(4))
            .expect("time should advance");
        let second = Simulation::replay(35, scenario, first.command_log(), first.time())
            .expect("journal should replay");
        assert_eq!(first.snapshot(), second.snapshot());
    }

    #[test]
    fn typed_ingress_is_idempotent_revision_guarded_and_replayable() {
        let (scenario, ids) = demo_scenario();
        let configuration = RunConfiguration::play_as_character(
            "seat.commander",
            "controller.human",
            ids.commander,
            "permission.military-command",
        );
        let manifest = manifest_for_configuration(&scenario, &configuration);
        let mut simulation = Simulation::new_with_run_configuration(
            35,
            scenario.clone(),
            manifest.clone(),
            configuration.clone(),
        )
        .expect("declared character run should load");
        let envelope = CommandEnvelope::new(
            Issuer::Human("controller.human".to_owned()),
            Command::MoveArmy {
                army: ids.army,
                destination: ids.eastern_territory,
            },
        )
        .with_authority(character_authority(
            ids.commander,
            ids.army,
            "seat.commander",
            "permission.military-command",
        ))
        .at_time(SimTime::EPOCH);
        let request = CommandRequest::new(CommandRequestId::new(1), 0, envelope.clone());
        let accepted = simulation
            .process_command(request.clone())
            .expect("typed request should produce an outcome");
        let CommandOutcome::Accepted { receipt } = &accepted else {
            panic!("matching controller and seat should be accepted");
        };
        assert_eq!(receipt.attempt_id, Some(CommandAttemptId::new(1)));
        assert_eq!(receipt.command_id, CommandId::new(1));
        assert_eq!(receipt.request_id, Some(CommandRequestId::new(1)));
        assert_eq!(receipt.revision, 1);
        assert_eq!(simulation.revision(), 1);

        let after_accept = simulation.snapshot();
        assert_eq!(
            simulation
                .process_command(request)
                .expect("an exact retry should be served from idempotency evidence"),
            accepted
        );
        assert_eq!(simulation.snapshot(), after_accept);

        let mut collision_envelope = envelope.clone();
        collision_envelope.command = Command::MoveArmy {
            army: ids.army,
            destination: ids.western_territory,
        };
        let collision = simulation
            .process_command(CommandRequest::new(
                CommandRequestId::new(1),
                1,
                collision_envelope,
            ))
            .expect("request-ID collision should be a structured outcome");
        let CommandOutcome::Rejected { rejection } = collision else {
            panic!("request-ID reuse with different input must be rejected");
        };
        assert_eq!(rejection.attempt_id, None);
        assert_eq!(rejection.retained_revision, 1);
        assert_eq!(rejection.error.code, ErrorCode::IdempotencyConflict);
        assert_eq!(simulation.snapshot(), after_accept);

        let stale_request = CommandRequest::new(CommandRequestId::new(2), 0, envelope);
        let stale = simulation
            .process_command(stale_request.clone())
            .expect("a stale request should remain structured evidence");
        let CommandOutcome::Rejected { rejection } = &stale else {
            panic!("stale revisions must be rejected");
        };
        assert_eq!(rejection.attempt_id, Some(CommandAttemptId::new(2)));
        assert_eq!(rejection.retained_revision, 2);
        assert_eq!(rejection.error.code, ErrorCode::SimulationRevisionConflict);
        assert_eq!(simulation.revision(), 2);
        assert_eq!(simulation.command_log().len(), 1);
        assert_eq!(simulation.command_attempts().len(), 2);

        let after_stale = simulation.snapshot();
        assert_eq!(
            simulation
                .process_command(stale_request)
                .expect("an exact rejected retry should be cached"),
            stale
        );
        assert_eq!(simulation.snapshot(), after_stale);
        let restored = Simulation::from_snapshot(after_stale.clone())
            .expect("typed ingress evidence should survive save/load");
        assert_eq!(restored.snapshot(), after_stale);

        let mut cyclic_cause = after_stale.clone();
        let event_id = cyclic_cause.events[0].id;
        cyclic_cause.events[0].cause = Some(CauseRef::Event(event_id));
        refresh_snapshot_commitments_and_checkpoint(&mut cyclic_cause);
        let Err(error) = Simulation::from_snapshot(cyclic_cause) else {
            panic!("event cause cycles must be rejected without unbounded traversal");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert!(error.message.contains("parent event"));

        let mut forged = after_stale;
        forged.command_attempts[0].envelope.issuer = Issuer::Human("controller.other".to_owned());
        forged.commands[0].envelope = forged.command_attempts[0].envelope.clone();
        refresh_snapshot_commitments_and_checkpoint(&mut forged);
        let Err(error) = Simulation::from_snapshot(forged) else {
            panic!("accepted attempts that violate recorded policy must not load");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert!(error.message.contains("ingress policy"));

        simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("attempt evidence should enter the next boundary");
        let boundary = simulation
            .boundaries()
            .last()
            .expect("the boundary should be recorded");
        assert_eq!(
            boundary.admitted_attempts,
            vec![CommandAttemptId::new(1), CommandAttemptId::new(2)]
        );
        assert_eq!(boundary.admitted_commands, vec![CommandId::new(1)]);
        let journal = simulation.replay_journal();
        let replayed_fixture = Simulation::replay_with_run_configuration(
            35,
            scenario.clone(),
            manifest,
            configuration,
            &[],
            simulation.command_log(),
            simulation.command_attempts(),
            simulation.boundaries(),
            simulation.time(),
        )
        .expect("declared caller-supplied request journal should replay");
        assert_eq!(simulation.snapshot(), replayed_fixture.snapshot());
        let replayed = Simulation::replay_from_journal(scenario, &[], &journal)
            .expect("accepted and rejected request evidence should replay exactly");
        assert_eq!(simulation.snapshot(), replayed.snapshot());
    }

    #[test]
    fn legacy_direct_and_tracked_request_ingress_cannot_mix() {
        let (scenario, ids) = demo_scenario();
        let mut legacy_first =
            Simulation::new(35, scenario.clone()).expect("compatibility run should load");
        legacy_first
            .submit(move_order(&ids))
            .expect("legacy direct command should be accepted");
        let after_legacy = legacy_first.snapshot();
        let error = legacy_first
            .process_command(CommandRequest::new(
                CommandRequestId::new(1),
                1,
                move_order(&ids),
            ))
            .expect_err("tracked requests cannot follow legacy-direct commands");
        assert_eq!(error.code, ErrorCode::MixedCommandIngress);
        assert_eq!(legacy_first.snapshot(), after_legacy);

        let mut tracked_first =
            Simulation::new(35, scenario.clone()).expect("compatibility run should load");
        let outcome = tracked_first
            .process_command(CommandRequest::new(
                CommandRequestId::new(1),
                0,
                CommandEnvelope::new(
                    Issuer::Actor(ids.observer),
                    Command::MoveArmy {
                        army: ids.army,
                        destination: ids.eastern_territory,
                    },
                ),
            ))
            .expect("domain rejection should remain tracked evidence");
        assert!(matches!(outcome, CommandOutcome::Rejected { .. }));
        let after_tracked = tracked_first.snapshot();
        let error = tracked_first
            .submit(move_order(&ids))
            .expect_err("legacy direct commands cannot follow tracked attempts");
        assert_eq!(error.code, ErrorCode::MixedCommandIngress);
        assert_eq!(tracked_first.snapshot(), after_tracked);
        Simulation::from_snapshot(after_tracked.clone())
            .expect("a rejection-only tracked journal should remain loadable");

        let error = tracked_first
            .schedule_calendar_boundary(SimTime::EPOCH, vec![SystemCadence::Daily])
            .expect_err("canonical ingress cannot begin after a direct tracked attempt");
        assert_eq!(error.code, ErrorCode::MixedCommandIngress);
        assert_eq!(tracked_first.snapshot(), after_tracked);

        tracked_first
            .append_ingress(
                SimTime::EPOCH,
                IngressClass::ScheduledSystem,
                0,
                IngressPayload::Calendar {
                    cadences: vec![SystemCadence::Daily],
                },
                Some(CauseRef::System("canwu.core.calendar".to_owned())),
                false,
            )
            .expect("the fixture should construct coherent mixed ingress evidence");
        let error = Simulation::from_snapshot(tracked_first.snapshot())
            .err()
            .expect("snapshot validation must reject mixed direct and canonical history");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut canonical_first =
            Simulation::new(35, scenario).expect("canonical compatibility run should load");
        canonical_first
            .schedule_calendar_boundary(SimTime::EPOCH, vec![SystemCadence::Daily])
            .expect("calendar ingress should establish the canonical family");
        let after_canonical = canonical_first.snapshot();
        let error = canonical_first
            .process_command(CommandRequest::new(
                CommandRequestId::new(2),
                0,
                move_order(&ids),
            ))
            .expect_err("direct tracked requests cannot bypass canonical ingress");
        assert_eq!(error.code, ErrorCode::MixedCommandIngress);
        assert_eq!(canonical_first.snapshot(), after_canonical);
    }

    #[test]
    fn declared_runs_reject_untracked_legacy_command_history() {
        let (scenario, ids) = demo_scenario();
        let configuration = RunConfiguration::play_as_character(
            "seat.commander",
            "controller.human",
            ids.commander,
            "permission.military-command",
        );
        let manifest = manifest_for_configuration(&scenario, &configuration);
        let mut declared = Simulation::new_with_run_configuration(
            35,
            scenario.clone(),
            manifest.clone(),
            configuration.clone(),
        )
        .expect("declared run should load");
        let envelope = CommandEnvelope::new(
            Issuer::Human("controller.human".to_owned()),
            Command::MoveArmy {
                army: ids.army,
                destination: ids.eastern_territory,
            },
        )
        .with_authority(character_authority(
            ids.commander,
            ids.army,
            "seat.commander",
            "permission.military-command",
        ))
        .at_time(SimTime::EPOCH);
        let before = declared.snapshot();
        let error = declared
            .submit(envelope)
            .expect_err("declared runs must not accept compatibility-only ingress");
        assert_eq!(error.code, ErrorCode::InvalidAuthority);
        assert_eq!(declared.snapshot(), before);

        let mut compatibility =
            Simulation::new(35, scenario.clone()).expect("compatibility run should load");
        compatibility
            .submit(move_order(&ids))
            .expect("legacy command fixture should be accepted");
        let mut forged = compatibility.snapshot();
        forged.run_manifest = before.run_manifest;
        forged.run_manifest_hash = before.run_manifest_hash;
        forged.run_configuration = before.run_configuration;
        refresh_snapshot_commitments_and_checkpoint(&mut forged);
        let Err(error) = Simulation::from_snapshot(forged) else {
            panic!("declared snapshots cannot smuggle untracked accepted commands");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert!(error.message.contains("tracked attempt evidence"));

        let Err(error) = Simulation::replay_with_run_configuration(
            35,
            scenario,
            manifest,
            configuration,
            &[],
            compatibility.command_log(),
            &[],
            &[],
            compatibility.time(),
        ) else {
            panic!("declared fixture replay cannot reinterpret legacy command input");
        };
        assert_eq!(error.code, ErrorCode::InvalidAuthority);
    }

    #[test]
    fn declared_revision_and_time_guards_cover_boundaries_and_clock() {
        let (scenario, ids) = demo_scenario();
        let configuration = RunConfiguration::play_as_character(
            "seat.commander",
            "controller.human",
            ids.commander,
            "permission.military-command",
        );
        let manifest = manifest_for_configuration(&scenario, &configuration);
        let command_at = |time| {
            CommandEnvelope::new(
                Issuer::Human("controller.human".to_owned()),
                Command::MoveArmy {
                    army: ids.army,
                    destination: ids.eastern_territory,
                },
            )
            .with_authority(character_authority(
                ids.commander,
                ids.army,
                "seat.commander",
                "permission.military-command",
            ))
            .at_time(time)
        };

        let mut after_boundary = Simulation::new_with_run_configuration(
            35,
            scenario.clone(),
            manifest.clone(),
            configuration.clone(),
        )
        .expect("declared run should load");
        after_boundary
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("boundary should publish");
        assert_eq!(after_boundary.revision(), 1);
        let stale = after_boundary
            .process_command(CommandRequest::new(
                CommandRequestId::new(1),
                0,
                command_at(SimTime::EPOCH),
            ))
            .expect("stale boundary revision should be retained as evidence");
        let CommandOutcome::Rejected { rejection } = stale else {
            panic!("a pre-boundary revision must be stale");
        };
        assert_eq!(rejection.error.code, ErrorCode::SimulationRevisionConflict);
        assert_eq!(rejection.retained_revision, 2);
        let accepted = after_boundary
            .process_command(CommandRequest::new(
                CommandRequestId::new(2),
                2,
                command_at(SimTime::EPOCH),
            ))
            .expect("current revision should be accepted");
        let CommandOutcome::Accepted { receipt } = accepted else {
            panic!("current revision and time should admit the command");
        };
        assert_eq!(receipt.revision, 3);
        assert_eq!(after_boundary.revision(), 3);
        let boundary_journal = after_boundary.replay_journal();
        let boundary_replay =
            Simulation::replay_from_journal(scenario.clone(), &[], &boundary_journal)
                .expect("boundary-relative revision evidence should replay exactly");
        assert_eq!(after_boundary.snapshot(), boundary_replay.snapshot());

        let mut after_clock =
            Simulation::new_with_run_configuration(35, scenario.clone(), manifest, configuration)
                .expect("declared run should load");
        after_clock
            .advance(SimDuration::hours(1))
            .expect("clock should advance");
        assert_eq!(after_clock.revision(), 0);
        let stale = after_clock
            .process_command(CommandRequest::new(
                CommandRequestId::new(1),
                0,
                command_at(SimTime::EPOCH),
            ))
            .expect("stale time should be retained as evidence");
        let CommandOutcome::Rejected { rejection } = stale else {
            panic!("a pre-advance simulation time must be stale");
        };
        assert_eq!(rejection.error.code, ErrorCode::SimulationTimeConflict);
        assert_eq!(rejection.retained_revision, 1);
        let accepted = after_clock
            .process_command(CommandRequest::new(
                CommandRequestId::new(2),
                1,
                command_at(after_clock.time()),
            ))
            .expect("current revision and time should be accepted");
        let CommandOutcome::Accepted { receipt } = accepted else {
            panic!("current clock guard should admit the command");
        };
        assert_eq!(receipt.revision, 2);
        let clock_journal = after_clock.replay_journal();
        let clock_replay = Simulation::replay_from_journal(scenario, &[], &clock_journal)
            .expect("clock-relative time evidence should replay exactly");
        assert_eq!(after_clock.snapshot(), clock_replay.snapshot());
    }

    #[test]
    fn authoritative_revision_is_persisted_migrated_and_rollback_safe() {
        let (scenario, ids) = demo_scenario();
        let invalid_morale = |morale| {
            CommandEnvelope::new(
                Issuer::Debug,
                Command::DebugSetArmyMorale {
                    army: ids.army,
                    morale,
                },
            )
        };
        let first_request = CommandRequest::new(CommandRequestId::new(1), 0, invalid_morale(101));
        let mut simulation =
            Simulation::new(35, scenario.clone()).expect("compatibility run should load");

        let first = simulation
            .process_command(first_request.clone())
            .expect("expected rejection should persist");
        let CommandOutcome::Rejected { rejection } = &first else {
            panic!("invalid morale must be rejected");
        };
        assert_eq!(rejection.retained_revision, 1);
        assert_eq!(simulation.revision(), 1);

        let after_first = simulation.snapshot();
        assert_eq!(
            simulation
                .process_command(first_request)
                .expect("an exact retry should return the recorded outcome"),
            first
        );
        assert_eq!(simulation.snapshot(), after_first);

        let second = simulation
            .process_command(CommandRequest::new(
                CommandRequestId::new(2),
                1,
                invalid_morale(102),
            ))
            .expect("a second expected rejection should persist");
        let CommandOutcome::Rejected { rejection } = second else {
            panic!("invalid morale must be rejected");
        };
        assert_eq!(rejection.retained_revision, 2);
        assert_eq!(simulation.revision(), 2);

        let before_conflict = simulation.snapshot();
        let conflict = simulation
            .process_command(CommandRequest::new(
                CommandRequestId::new(2),
                2,
                invalid_morale(103),
            ))
            .expect("a request-ID collision should return a non-persisted rejection");
        let CommandOutcome::Rejected { rejection } = conflict else {
            panic!("a request-ID collision must not be accepted");
        };
        assert_eq!(rejection.attempt_id, None);
        assert_eq!(rejection.error.code, ErrorCode::IdempotencyConflict);
        assert_eq!(rejection.retained_revision, 2);
        assert_eq!(simulation.snapshot(), before_conflict);

        simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("an empty boundary should publish");
        assert_eq!(simulation.revision(), 3);
        let first_boundary_snapshot = simulation.snapshot();
        let third = simulation
            .process_command(CommandRequest::new(
                CommandRequestId::new(3),
                3,
                invalid_morale(103),
            ))
            .expect("a post-boundary expected rejection should persist");
        let CommandOutcome::Rejected { rejection } = third else {
            panic!("invalid morale must be rejected");
        };
        assert_eq!(rejection.retained_revision, 4);
        simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH + SimDuration::hours(1)))
            .expect("a second empty boundary should publish");
        assert_eq!(simulation.revision(), 5);
        let current_snapshot = simulation.snapshot();
        let restored = Simulation::from_snapshot(current_snapshot.clone())
            .expect("current revision evidence should survive load");
        assert_eq!(restored.revision(), 5);
        assert_eq!(restored.snapshot(), current_snapshot);
        let replayed =
            Simulation::replay_from_journal(scenario.clone(), &[], &simulation.replay_journal())
                .expect("current revision evidence should replay exactly");
        assert_eq!(replayed.snapshot(), current_snapshot);

        let mut inconsistent_revision = current_snapshot.clone();
        inconsistent_revision.state_revision = 6;
        inconsistent_revision.checkpoint_hash = snapshot_checkpoint_hash(&inconsistent_revision)
            .expect("the inconsistent revision fixture should remain coherently hashed");
        let error = Simulation::from_snapshot(inconsistent_revision)
            .err()
            .expect("a rehashed revision without evidence must not load");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert!(error.message.contains("state revision"));

        let mut legacy_first_boundary = first_boundary_snapshot;
        legacy_first_boundary.command_attempts[1].revision_before = 0;
        legacy_first_boundary.command_attempts[1].expected_revision = Some(0);
        legacy_first_boundary.revision_format_version = 0;
        legacy_first_boundary.state_revision = 0;
        legacy_first_boundary.replay_revision_format_version = 0;
        let legacy_first_boundary_state_hash = snapshot_state_hash(&legacy_first_boundary)
            .expect("legacy first-boundary state should hash canonically");

        let mut legacy_snapshot = current_snapshot.clone();
        legacy_snapshot.command_attempts[1].revision_before = 0;
        legacy_snapshot.command_attempts[1].expected_revision = Some(0);
        legacy_snapshot.command_attempts[2].revision_before = 1;
        legacy_snapshot.command_attempts[2].expected_revision = Some(u64::MAX);
        legacy_snapshot.command_attempts[2].outcome = CommandAttemptOutcome::Rejected {
            error: CanwuError::new(
                ErrorCode::SimulationRevisionConflict,
                format!(
                    "command expected revision {}, but simulation is at revision 1",
                    u64::MAX
                ),
            ),
        };
        legacy_snapshot.revision_format_version = 0;
        legacy_snapshot.state_revision = 0;
        legacy_snapshot.replay_revision_format_version = 0;
        legacy_snapshot.boundaries[0].state_hash = Some(legacy_first_boundary_state_hash);
        let legacy_state_hash = snapshot_state_hash(&legacy_snapshot)
            .expect("legacy revision state should hash canonically");
        legacy_snapshot
            .boundaries
            .last_mut()
            .expect("the migration fixture has a boundary head")
            .state_hash = Some(legacy_state_hash);
        rehash_snapshot_boundaries(&mut legacy_snapshot)
            .expect("legacy boundary evidence should hash canonically");
        downgrade_snapshot_commitments(&mut legacy_snapshot);
        legacy_snapshot.checkpoint_hash = snapshot_checkpoint_hash(&legacy_snapshot)
            .expect("legacy checkpoint should bind its pre-migration state");
        let mut legacy_value =
            serde_json::to_value(legacy_snapshot).expect("legacy fixture should serialize");
        let legacy_object = legacy_value
            .as_object_mut()
            .expect("legacy snapshot JSON should be an object");
        legacy_object.remove("revision_format_version");
        legacy_object.remove("state_revision");
        legacy_object.remove("replay_revision_format_version");
        legacy_object.remove("admission_cursor_format_version");
        legacy_object.remove("admitted_attempt_count");
        legacy_object.remove("admitted_command_count");
        legacy_object.remove("admitted_event_count");
        let mut broken_chain = legacy_value.clone();
        broken_chain["boundaries"][0]["correlation_id"] = Value::from(999_u64);
        let error = Simulation::from_snapshot_json(
            &serde_json::to_string(&broken_chain).expect("tampered legacy fixture should encode"),
        )
        .err()
        .expect("migration must not launder a broken legacy boundary hash chain");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert!(error.message.contains("legacy boundary hash chain"));

        let migrated = Simulation::from_snapshot_json(
            &serde_json::to_string(&legacy_value).expect("legacy fixture should encode"),
        )
        .expect("legacy command revisions should migrate deterministically");
        assert_eq!(migrated.revision(), 5);
        assert_eq!(migrated.command_attempts()[1].revision_before, 1);
        assert_eq!(migrated.command_attempts()[2].revision_before, 3);
        assert_eq!(
            migrated.command_attempts()[2].expected_revision,
            Some(u64::MAX)
        );
        assert_eq!(migrated.snapshot().replay_revision_format_version, 0);
        let reloaded = Simulation::from_snapshot(migrated.snapshot())
            .expect("migration-only replay provenance should survive save and load");
        assert_eq!(reloaded.revision(), 5);

        let migrated_journal = reloaded.replay_journal();
        assert_eq!(migrated_journal.revision_format_version, 0);
        let error = Simulation::replay_from_journal(scenario, &[], &migrated_journal)
            .err()
            .expect("revision-migrated histories must not claim current exact replay");
        assert_eq!(error.code, ErrorCode::LegacyReplayUnavailable);

        let mut continued = reloaded;
        continued
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH + SimDuration::hours(2)))
            .expect("a revision-migrated snapshot should remain continuable");
        assert_eq!(continued.revision(), 6);
    }

    #[test]
    fn legacy_revision_migration_rebases_admitted_and_pending_command_ingress() {
        let (scenario, ids) = demo_scenario();
        let invalid_request = |request_id, revision, morale| {
            CommandRequest::new(
                CommandRequestId::new(request_id),
                revision,
                CommandEnvelope::new(
                    Issuer::Debug,
                    Command::DebugSetArmyMorale {
                        army: ids.army,
                        morale,
                    },
                ),
            )
        };
        let mut simulation =
            Simulation::new(47, scenario).expect("canonical migration run should load");
        simulation
            .enqueue_command(SimTime::EPOCH, 0, invalid_request(1, 0, 101))
            .expect("first invalid command should queue");
        simulation
            .step_canonical()
            .expect("first command boundary should settle")
            .expect("first command should create a boundary");
        assert_eq!(simulation.revision(), 2);
        let after_first_boundary = simulation.snapshot();

        let second_at = SimTime::EPOCH + SimDuration::hours(1);
        simulation
            .enqueue_command(second_at, 0, invalid_request(2, 2, 102))
            .expect("second invalid command should queue");
        simulation
            .advance_canonical(SimDuration::hours(1))
            .expect("second command boundary should settle");
        assert_eq!(simulation.revision(), 4);
        let after_second_boundary = simulation.snapshot();

        let pending_at = second_at + SimDuration::hours(1);
        simulation
            .enqueue_command(pending_at, 0, invalid_request(3, 4, 103))
            .expect("pending invalid command should queue");
        let current_snapshot = simulation.snapshot();

        let legacyize = |snapshot: &mut SimulationSnapshot| {
            snapshot.revision_format_version = 0;
            snapshot.state_revision = 0;
            snapshot.replay_revision_format_version = 0;
            for (index, attempt) in snapshot.command_attempts.iter_mut().enumerate() {
                let legacy_revision = u64::try_from(index).expect("fixture index should fit");
                attempt.revision_before = legacy_revision;
                attempt.expected_revision = Some(legacy_revision);
            }
            for (index, record) in snapshot.ingress.iter_mut().enumerate() {
                let IngressPayload::Command { request } = &mut record.payload else {
                    continue;
                };
                request.expected_revision = u64::try_from(index).expect("fixture index should fit");
            }
        };

        let mut legacy_first = after_first_boundary;
        legacyize(&mut legacy_first);
        let first_state_hash =
            snapshot_state_hash(&legacy_first).expect("legacy first command boundary should hash");

        let mut legacy_second = after_second_boundary;
        legacyize(&mut legacy_second);
        legacy_second.boundaries[0].state_hash = Some(first_state_hash.clone());
        let second_state_hash = snapshot_state_hash(&legacy_second)
            .expect("legacy second command boundary should hash");

        let mut legacy_snapshot = current_snapshot;
        legacyize(&mut legacy_snapshot);
        legacy_snapshot.boundaries[0].state_hash = Some(first_state_hash);
        legacy_snapshot.boundaries[1].state_hash = Some(second_state_hash);
        rehash_snapshot_boundaries(&mut legacy_snapshot)
            .expect("legacy ingress boundary chain should hash");
        downgrade_snapshot_commitments(&mut legacy_snapshot);
        legacy_snapshot.checkpoint_hash = snapshot_checkpoint_hash(&legacy_snapshot)
            .expect("legacy ingress checkpoint should hash");
        let mut legacy_value =
            serde_json::to_value(legacy_snapshot).expect("legacy ingress fixture should serialize");
        let legacy_object = legacy_value
            .as_object_mut()
            .expect("legacy ingress snapshot should be an object");
        legacy_object.remove("revision_format_version");
        legacy_object.remove("state_revision");
        legacy_object.remove("replay_revision_format_version");
        legacy_object.remove("admission_cursor_format_version");
        legacy_object.remove("admitted_attempt_count");
        legacy_object.remove("admitted_command_count");
        legacy_object.remove("admitted_event_count");

        let mut migrated = Simulation::from_snapshot_json(
            &serde_json::to_string(&legacy_value).expect("legacy ingress fixture should encode"),
        )
        .expect("admitted and pending command guards should migrate coherently");
        assert_eq!(migrated.revision(), 4);
        assert_eq!(
            migrated
                .command_attempts()
                .iter()
                .map(|attempt| (attempt.revision_before, attempt.expected_revision))
                .collect::<Vec<_>>(),
            vec![(0, Some(0)), (2, Some(2))]
        );
        assert_eq!(
            migrated
                .ingress_log()
                .iter()
                .filter_map(|record| match &record.payload {
                    IngressPayload::Command { request } => Some(request.expected_revision),
                    IngressPayload::Plugin { .. } | IngressPayload::Calendar { .. } => None,
                })
                .collect::<Vec<_>>(),
            vec![0, 2, 4]
        );
        assert_eq!(migrated.snapshot().replay_revision_format_version, 0);

        migrated
            .step_canonical()
            .expect("migrated pending command should settle")
            .expect("pending command should create a boundary");
        assert_eq!(migrated.revision(), 6);
        assert_eq!(
            migrated
                .command_attempts()
                .last()
                .expect("pending command should create an attempt")
                .revision_before,
            4
        );
    }

    #[test]
    fn admission_cursors_are_persisted_migrated_and_tamper_evident() {
        let (scenario, ids) = demo_scenario();
        let morale_request = |request_id, revision, morale| {
            CommandRequest::new(
                CommandRequestId::new(request_id),
                revision,
                CommandEnvelope::new(
                    Issuer::Debug,
                    Command::DebugSetArmyMorale {
                        army: ids.army,
                        morale,
                    },
                ),
            )
        };
        let mut simulation =
            Simulation::new(59, scenario.clone()).expect("cursor fixture should load");
        assert!(matches!(
            simulation
                .process_command(morale_request(1, 0, 80))
                .expect("first command should be accepted"),
            CommandOutcome::Accepted { .. }
        ));
        assert!(matches!(
            simulation
                .process_command(morale_request(2, 1, 101))
                .expect("expected rejection should persist"),
            CommandOutcome::Rejected { .. }
        ));
        simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("first cursor boundary should settle");
        let first_snapshot = simulation.snapshot();
        assert_eq!(first_snapshot.admitted_attempt_count, 2);
        assert_eq!(first_snapshot.admitted_command_count, 1);
        assert_eq!(first_snapshot.admitted_event_count, 1);

        assert!(matches!(
            simulation
                .process_command(morale_request(3, 3, 70))
                .expect("second command should be accepted"),
            CommandOutcome::Accepted { .. }
        ));
        simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("second cursor boundary should settle");
        let current_snapshot = simulation.snapshot();
        assert_eq!(current_snapshot.admission_cursor_format_version, 1);
        assert_eq!(current_snapshot.admitted_attempt_count, 3);
        assert_eq!(current_snapshot.admitted_command_count, 2);
        assert_eq!(current_snapshot.admitted_event_count, 2);

        let restored = Simulation::from_snapshot(current_snapshot.clone())
            .expect("persisted admission cursors should load");
        assert_eq!(restored.snapshot(), current_snapshot);
        let replayed = Simulation::replay_from_journal(scenario, &[], &simulation.replay_journal())
            .expect("admission cursors should reproduce under exact replay");
        assert_eq!(replayed.snapshot(), current_snapshot);

        let mut legacy_value = serde_json::to_value(current_snapshot.clone())
            .expect("cursor migration fixture should serialize");
        let legacy_object = legacy_value
            .as_object_mut()
            .expect("cursor migration snapshot should be an object");
        legacy_object.remove("admission_cursor_format_version");
        legacy_object.remove("admitted_attempt_count");
        legacy_object.remove("admitted_command_count");
        legacy_object.remove("admitted_event_count");
        let migrated = Simulation::from_snapshot_json(
            &serde_json::to_string(&legacy_value).expect("cursor migration fixture should encode"),
        )
        .expect("legacy admission cursors should derive from boundary prefixes");
        assert_eq!(migrated.snapshot(), current_snapshot);

        let mut tampered_cursor = current_snapshot.clone();
        tampered_cursor.admitted_attempt_count -= 1;
        let error = Simulation::from_snapshot(tampered_cursor)
            .err()
            .expect("a cursor detached from boundary evidence must not load");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert!(error.message.contains("admission cursors"));

        let mut migrated_gap = current_snapshot;
        migrated_gap.boundaries[0].admitted_attempts.remove(0);
        migrated_gap.admission_cursor_format_version = 0;
        migrated_gap.admitted_attempt_count = 0;
        migrated_gap.admitted_command_count = 0;
        migrated_gap.admitted_event_count = 0;
        rehash_tampered_snapshot(&mut migrated_gap);
        let error = Simulation::from_snapshot(migrated_gap)
            .err()
            .expect("legacy cursor migration must reject a journal-prefix gap");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    }

    #[test]
    fn expected_domain_rejections_survive_load_and_exact_replay() {
        let (scenario, ids) = demo_scenario();
        let mut simulation =
            Simulation::new(35, scenario.clone()).expect("compatibility run should load");
        simulation
            .register_plugin(&AuthorityPlugin)
            .expect("payload-validation plugin should register");
        let requests = [
            (
                CommandRequestId::new(1),
                CommandEnvelope::new(
                    Issuer::Actor(ids.commander),
                    Command::MoveArmy {
                        army: ArmyId::new(999),
                        destination: ids.eastern_territory,
                    },
                ),
            ),
            (
                CommandRequestId::new(2),
                CommandEnvelope::new(
                    Issuer::Debug,
                    Command::DebugSetArmyMorale {
                        army: ids.army,
                        morale: 101,
                    },
                ),
            ),
            (
                CommandRequestId::new(3),
                CommandEnvelope::new(
                    Issuer::Actor(ids.commander),
                    Command::Plugin {
                        plugin: "authority-test".to_owned(),
                        command: "set_stance".to_owned(),
                        payload: serde_json::json!({}),
                    },
                ),
            ),
            (
                CommandRequestId::new(4),
                CommandEnvelope::new(
                    Issuer::Actor(ids.commander),
                    Command::Plugin {
                        plugin: "missing-plugin".to_owned(),
                        command: "missing-command".to_owned(),
                        payload: Value::Null,
                    },
                ),
            ),
        ];
        let expected = [
            ErrorCode::ArmyNotFound,
            ErrorCode::ValueOutOfRange,
            ErrorCode::InvalidPayload,
            ErrorCode::PluginCommandNotFound,
        ];
        for ((request_id, envelope), expected_code) in requests.into_iter().zip(expected) {
            let revision_before = simulation.revision();
            let outcome = simulation
                .process_command(CommandRequest::new(request_id, revision_before, envelope))
                .expect("expected domain rejection should be a command outcome");
            let CommandOutcome::Rejected { rejection } = outcome else {
                panic!("invalid command fixture must be rejected");
            };
            assert_eq!(rejection.error.code, expected_code);
            assert_eq!(rejection.retained_revision, revision_before + 1);
        }
        assert!(simulation.command_log().is_empty());
        assert_eq!(simulation.command_attempts().len(), 4);
        assert_eq!(simulation.revision(), 4);

        let snapshot = simulation.snapshot();
        let restored = Simulation::from_snapshot(snapshot.clone())
            .expect("expected rejection evidence must not invalidate its own snapshot");
        assert_eq!(restored.snapshot(), snapshot);
        let journal = simulation.replay_journal();
        let replayed = Simulation::replay_from_journal(scenario, &[&AuthorityPlugin], &journal)
            .expect("expected rejection evidence should replay exactly");
        assert_eq!(simulation.snapshot(), replayed.snapshot());
    }

    #[test]
    fn read_only_and_frozen_replay_ingress_are_not_interchangeable() {
        let (scenario, ids) = demo_scenario();
        let observer_configuration = RunConfiguration::read_only_observer();
        let observer_manifest = manifest_for_configuration(&scenario, &observer_configuration);
        let mut observer = Simulation::new_with_run_configuration(
            35,
            scenario.clone(),
            observer_manifest,
            observer_configuration,
        )
        .expect("read-only observer run should load");
        let before = observer.snapshot();
        let live_human = observer
            .process_command(CommandRequest::new(
                CommandRequestId::new(1),
                0,
                CommandEnvelope::new(
                    Issuer::Human("controller.human".to_owned()),
                    Command::MoveArmy {
                        army: ids.army,
                        destination: ids.eastern_territory,
                    },
                )
                .with_authority(CommandAuthority::for_actor(ids.commander)),
            ))
            .expect("read-only rejection should be structured");
        let CommandOutcome::Rejected { rejection } = live_human else {
            panic!("read-only observer must reject a live human command");
        };
        assert_eq!(rejection.error.code, ErrorCode::InteractionReadOnly);
        assert_eq!(observer.world(), before.world);
        assert_eq!(observer.events(), before.events);
        assert_eq!(observer.command_log(), before.commands);
        assert_eq!(observer.random_draws(), before.random_draws);
        assert_eq!(observer.command_attempts().len(), 1);
        let observer_journal = observer.replay_journal();
        let observer_replay =
            Simulation::replay_from_journal(scenario.clone(), &[], &observer_journal)
                .expect("read-only rejection evidence should replay exactly");
        assert_eq!(observer.snapshot(), observer_replay.snapshot());

        let replay_configuration = RunConfiguration::replay_as_character(
            "seat.commander",
            "controller.recorded",
            ids.commander,
            "permission.military-command",
        );
        let replay_manifest = manifest_for_configuration(&scenario, &replay_configuration);
        let replay_envelope = CommandEnvelope::new(
            Issuer::Replay("controller.recorded".to_owned()),
            Command::MoveArmy {
                army: ids.army,
                destination: ids.eastern_territory,
            },
        )
        .with_authority(character_authority(
            ids.commander,
            ids.army,
            "seat.commander",
            "permission.military-command",
        ))
        .at_time(SimTime::EPOCH);

        let mut live_replay = Simulation::new_with_run_configuration(
            35,
            scenario.clone(),
            replay_manifest.clone(),
            replay_configuration.clone(),
        )
        .expect("replay run should load");
        let outcome = live_replay
            .process_command(CommandRequest::new(
                CommandRequestId::new(1),
                0,
                replay_envelope.clone(),
            ))
            .expect("live replay forgery should be a structured rejection");
        let CommandOutcome::Rejected { rejection } = outcome else {
            panic!("a live caller cannot self-identify as frozen replay");
        };
        assert_eq!(rejection.error.code, ErrorCode::InvalidAuthority);
        assert!(live_replay.command_log().is_empty());

        let mut frozen_source = Simulation::new_with_run_configuration(
            35,
            scenario.clone(),
            replay_manifest.clone(),
            replay_configuration.clone(),
        )
        .expect("frozen replay source should load");
        let outcome = frozen_source
            .admit_command(
                Some(CommandRequestId::new(7)),
                Some(0),
                replay_envelope,
                CommandIngress::FrozenReplay,
                true,
            )
            .expect("the trusted replay path should consume frozen input");
        assert!(matches!(outcome, CommandOutcome::Accepted { .. }));
        let Err(error) = Simulation::replay_with_run_configuration(
            35,
            scenario.clone(),
            replay_manifest,
            replay_configuration,
            &[],
            frozen_source.command_log(),
            frozen_source.command_attempts(),
            frozen_source.boundaries(),
            frozen_source.time(),
        ) else {
            panic!("caller-supplied fixture replay cannot consume frozen ingress");
        };
        assert_eq!(error.code, ErrorCode::ReplayEnvironmentMismatch);
        let frozen_journal = frozen_source.replay_journal();
        let frozen_replay = Simulation::replay_from_journal(scenario, &[], &frozen_journal)
            .expect("frozen controller input should replay exactly");
        assert_eq!(frozen_source.snapshot(), frozen_replay.snapshot());

        let mut forged_live_ingress = frozen_source.snapshot();
        forged_live_ingress.command_attempts[0].ingress = CommandIngress::LiveRequest;
        refresh_snapshot_commitments_and_checkpoint(&mut forged_live_ingress);
        let Err(error) = Simulation::from_snapshot(forged_live_ingress) else {
            panic!("live ingress cannot be relabeled as an accepted replay command");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    }

    #[test]
    fn observation_and_trace_policy_are_causally_inert() {
        let (scenario, _) = demo_scenario();
        let public_configuration = RunConfiguration::read_only_observer();
        let mut research_configuration = public_configuration.clone();
        research_configuration.observation = ObservationPolicy::ResearchFull;
        research_configuration.trace = TracePolicy::FullResearch;
        let mut public = Simulation::new_with_run_configuration(
            35,
            scenario.clone(),
            manifest_for_configuration(&scenario, &public_configuration),
            public_configuration,
        )
        .expect("public observer run should load");
        let mut research = Simulation::new_with_run_configuration(
            35,
            scenario.clone(),
            manifest_for_configuration(&scenario, &research_configuration),
            research_configuration,
        )
        .expect("research observer run should load");

        assert_ne!(public.run_manifest_hash(), research.run_manifest_hash());
        assert_ne!(public.checkpoint_hash(), research.checkpoint_hash());
        assert_eq!(
            public
                .authoritative_state_hash()
                .expect("public state should hash"),
            research
                .authoritative_state_hash()
                .expect("research state should hash")
        );
        let public_receipt = public
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("public boundary should settle");
        let research_receipt = research
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("research boundary should settle");
        assert_eq!(public_receipt.boundary_hash, research_receipt.boundary_hash);
        assert_eq!(
            public.boundaries()[0].state_hash,
            research.boundaries()[0].state_hash
        );
        assert_eq!(public.world(), research.world());
        assert_eq!(public.random_draws(), research.random_draws());
        assert_eq!(
            public.snapshot().random_streams,
            research.snapshot().random_streams
        );
        assert_ne!(public.checkpoint_hash(), research.checkpoint_hash());
    }

    #[test]
    fn invalid_command_does_not_mutate_any_serialized_state() {
        let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
        let before = simulation
            .snapshot_json()
            .expect("snapshot should serialize");
        let result = simulation.submit(CommandEnvelope::new(
            Issuer::Actor(ids.observer),
            Command::MoveArmy {
                army: ids.army,
                destination: ids.eastern_territory,
            },
        ));
        assert_eq!(
            result.expect_err("observer cannot command army").code,
            ErrorCode::InvalidAuthority
        );
        assert_eq!(
            before,
            simulation
                .snapshot_json()
                .expect("snapshot should serialize")
        );
    }

    #[test]
    fn movement_emits_events_and_executes_at_scheduled_time() {
        let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
        let receipt = simulation
            .submit(move_order(&ids))
            .expect("order should validate");
        assert_eq!(receipt.emitted_events.len(), 1);
        simulation
            .advance(SimDuration::hours(17))
            .expect("time should advance");
        assert_eq!(
            simulation
                .world()
                .army(ids.army)
                .expect("army exists")
                .location,
            ids.central_territory
        );
        let events = simulation
            .advance(SimDuration::hours(1))
            .expect("arrival should execute");
        assert!(
            events
                .iter()
                .any(|event| matches!(event.kind, EventKind::ArmyArrived { .. }))
        );
        assert_eq!(
            simulation
                .world()
                .army(ids.army)
                .expect("army exists")
                .location,
            ids.eastern_territory
        );
    }

    #[test]
    fn internal_runtime_partitions_preserve_flat_persistence_contracts() {
        let (scenario, ids) = demo_scenario();
        let mut simulation =
            Simulation::new(35, scenario.clone()).expect("the demo scenario should load");
        simulation
            .submit(move_order(&ids))
            .expect("the move should populate evidence and scheduled work");
        simulation
            .advance(SimDuration::days(1))
            .expect("the scheduled move should complete");

        let snapshot = simulation.snapshot();
        let snapshot_value =
            serde_json::to_value(&snapshot).expect("the snapshot should become JSON");
        let snapshot_object = snapshot_value
            .as_object()
            .expect("snapshot JSON should remain a flat object");
        for public_field in [
            "checkpoint_hash",
            "state_revision",
            "next_event_id",
            "admission_cursor_format_version",
            "events",
            "commands",
            "scheduled",
        ] {
            assert!(
                snapshot_object.contains_key(public_field),
                "snapshot should retain flat field {public_field}"
            );
        }
        for internal_owner in ["current", "metadata", "counters", "evidence", "scheduler"] {
            assert!(
                !snapshot_object.contains_key(internal_owner),
                "private owner {internal_owner} must not enter the snapshot wire shape"
            );
        }

        let json = serde_json::to_string(&snapshot).expect("the snapshot should serialize");
        let restored =
            Simulation::from_snapshot_json(&json).expect("the flat snapshot should restore");
        assert_eq!(restored.snapshot(), snapshot);

        let journal = restored.replay_journal();
        let journal_value =
            serde_json::to_value(&journal).expect("the replay journal should become JSON");
        let journal_object = journal_value
            .as_object()
            .expect("replay journal JSON should remain a flat object");
        for internal_owner in ["current", "metadata", "counters", "evidence", "scheduler"] {
            assert!(
                !journal_object.contains_key(internal_owner),
                "private owner {internal_owner} must not enter the journal wire shape"
            );
        }
        let replayed = Simulation::replay_from_journal(scenario, &[], &journal)
            .expect("the flat replay journal should remain exact");
        assert_eq!(replayed.snapshot(), snapshot);
    }

    #[test]
    fn domain_commitments_migrate_replay_and_reject_each_tampered_root() {
        let (scenario, ids) = demo_scenario();
        let mut simulation =
            Simulation::new(97, scenario.clone()).expect("the commitment fixture should load");
        simulation
            .submit(move_order(&ids))
            .expect("the commitment fixture should accept its command");
        simulation
            .advance(SimDuration::days(1))
            .expect("the commitment fixture should execute scheduled work");
        let snapshot = simulation.snapshot();
        assert_eq!(
            snapshot.commitment_format_version,
            COMMITMENT_FORMAT_VERSION
        );
        assert!(commitment_roots_are_canonical(
            snapshot
                .commitment_roots
                .as_ref()
                .expect("current snapshots should persist domain roots")
        ));

        let expected_roots = snapshot_commitment_roots(&snapshot)
            .expect("the canonical snapshot should produce roots");
        let mut reordered = snapshot.clone();
        reordered.world.people.reverse();
        reordered.world.governments.reverse();
        reordered.world.territories.reverse();
        reordered.world.routes.reverse();
        reordered.world.armies.reverse();
        reordered.events.reverse();
        reordered.commands.reverse();
        reordered.command_attempts.reverse();
        reordered.ingress.reverse();
        reordered.plugin_components.reverse();
        reordered.domain_records.reverse();
        reordered.plugin_descriptors.reverse();
        reordered.random_streams.reverse();
        reordered.random_draws.reverse();
        reordered.scheduled.reverse();
        assert_eq!(
            snapshot_commitment_roots(&reordered)
                .expect("collection insertion order should not affect roots"),
            expected_roots
        );

        for root_name in [
            "world",
            "knowledge",
            "plugin_components",
            "domain_records",
            "scheduler",
            "commands",
            "events",
            "ingress",
            "random",
            "boundary_chain",
            "identity",
            "control",
        ] {
            let mut forged = snapshot.clone();
            let mut roots_value = serde_json::to_value(
                forged
                    .commitment_roots
                    .as_ref()
                    .expect("the fixture should persist roots"),
            )
            .expect("commitment roots should become JSON");
            roots_value
                .as_object_mut()
                .expect("commitment roots should be an object")
                .insert(root_name.to_owned(), Value::String("0".repeat(64)));
            forged.commitment_roots = Some(
                serde_json::from_value(roots_value)
                    .expect("the forged commitment roots should deserialize"),
            );
            forged.checkpoint_hash = snapshot_checkpoint_hash(&forged)
                .expect("the forged roots should produce a coherent outer checkpoint");
            let error = Simulation::from_snapshot(forged)
                .err()
                .expect("every forged domain root must be rejected");
            assert_eq!(error.code, ErrorCode::InvalidSnapshot);
            assert!(error.message.contains("commitment roots"));
        }

        let mut legacy_snapshot = snapshot.clone();
        downgrade_snapshot_commitments(&mut legacy_snapshot);
        legacy_snapshot.checkpoint_hash = snapshot_checkpoint_hash(&legacy_snapshot)
            .expect("the legacy fixture should reproduce checkpoint v3");
        let legacy_checkpoint = legacy_snapshot.checkpoint_hash.clone();
        let migrated = Simulation::from_snapshot(legacy_snapshot.clone())
            .expect("a verified legacy checkpoint should derive current roots");
        assert_eq!(
            migrated.snapshot().commitment_format_version,
            COMMITMENT_FORMAT_VERSION
        );
        assert_ne!(migrated.checkpoint_hash(), legacy_checkpoint);

        let mut tampered_legacy = legacy_snapshot;
        tampered_legacy.world.armies[0].morale += 1;
        let error = Simulation::from_snapshot(tampered_legacy)
            .err()
            .expect("migration must verify the old checkpoint before deriving roots");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert!(error.message.contains("pre-commitment state"));

        let mut legacy_journal = simulation.replay_journal();
        legacy_journal.commitment_format_version = 0;
        legacy_journal.checkpoint_hash = legacy_checkpoint;
        let replayed = Simulation::replay_from_journal(scenario, &[], &legacy_journal)
            .expect("legacy commitment journals should replay under checkpoint v3");
        assert_eq!(replayed.snapshot().commitment_format_version, 0);
        assert!(replayed.snapshot().commitment_roots.is_none());
        assert_eq!(replayed.checkpoint_hash(), legacy_journal.checkpoint_hash);
    }

    #[test]
    fn cached_mutable_commitments_match_independent_snapshot_roots_after_each_mutation() {
        fn assert_exact(simulation: &Simulation) {
            let snapshot = simulation.snapshot();
            let expected = snapshot_commitment_roots(&snapshot)
                .expect("serialized state should independently reproduce every commitment root");
            assert_eq!(snapshot.commitment_roots.as_ref(), Some(&expected));
            let cache = simulation
                .state
                .metadata
                .commitment_cache
                .as_ref()
                .expect("current runtimes should maintain a private commitment cache");
            assert!(
                [
                    &cache.world,
                    &cache.knowledge,
                    &cache.plugin_components,
                    &cache.domain_records,
                    &cache.scheduler,
                    &cache.random_streams,
                    &cache.identity,
                ]
                .into_iter()
                .all(Option::is_some),
                "every invalidated domain must be refreshed before a transaction commits"
            );
        }

        let (scenario, ids) = demo_scenario();
        let mut simulation =
            Simulation::new(101, scenario.clone()).expect("cache fixture should load");
        assert_exact(&simulation);
        simulation
            .register_plugin(&AuthorityPlugin)
            .expect("component plugin should register");
        assert_exact(&simulation);
        simulation
            .register_plugin(&PrimaryRandomPlugin)
            .expect("random plugin should register");
        assert_exact(&simulation);

        let before_rejection = simulation
            .snapshot()
            .commitment_roots
            .expect("current snapshots should have roots");
        let rejected = simulation
            .process_command(CommandRequest::new(
                CommandRequestId::new(1),
                simulation.revision() + 1,
                CommandEnvelope::new(
                    Issuer::Debug,
                    Command::DebugSetArmyMorale {
                        army: ids.army,
                        morale: 75,
                    },
                ),
            ))
            .expect("stale input should become deterministic rejection evidence");
        assert!(matches!(rejected, CommandOutcome::Rejected { .. }));
        assert_exact(&simulation);
        let after_rejection = simulation
            .snapshot()
            .commitment_roots
            .expect("current snapshots should have roots");
        assert_eq!(before_rejection.world, after_rejection.world);
        assert_eq!(before_rejection.knowledge, after_rejection.knowledge);
        assert_eq!(
            before_rejection.plugin_components,
            after_rejection.plugin_components
        );
        assert_eq!(
            before_rejection.domain_records,
            after_rejection.domain_records
        );
        assert_eq!(before_rejection.scheduler, after_rejection.scheduler);
        assert_eq!(before_rejection.random, after_rejection.random);
        assert_eq!(before_rejection.identity, after_rejection.identity);
        assert_ne!(before_rejection.commands, after_rejection.commands);
        assert_ne!(before_rejection.control, after_rejection.control);

        simulation
            .process_command(CommandRequest::new(
                CommandRequestId::new(2),
                simulation.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(ids.commander),
                    Command::Plugin {
                        plugin: "authority-test".to_owned(),
                        command: "set_stance".to_owned(),
                        payload: Value::Null,
                    },
                ),
            ))
            .expect("component command should commit");
        assert_exact(&simulation);
        simulation
            .process_command(CommandRequest::new(
                CommandRequestId::new(3),
                simulation.revision(),
                move_order(&ids),
            ))
            .expect("movement command should commit");
        assert_exact(&simulation);
        simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
            .expect("random boundary should commit");
        assert_exact(&simulation);
        simulation
            .advance(SimDuration::days(1))
            .expect("scheduled arrival should commit");
        assert_exact(&simulation);

        let mut records = Simulation::new(103, scenario).expect("record cache fixture should load");
        records
            .register_plugin(&RecordLifecyclePlugin)
            .expect("record plugin should register");
        assert_exact(&records);
        records
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
            .expect("record mutation boundary should commit");
        assert_exact(&records);
    }

    #[test]
    fn rejection_transaction_restores_private_commitment_state_after_hash_failure() {
        let (scenario, ids) = demo_scenario();
        let mut simulation =
            Simulation::new(107, scenario).expect("rejection rollback fixture should load");
        let before = simulation.snapshot();
        simulation
            .state
            .metadata
            .commitment_cache
            .as_mut()
            .expect("current runtimes should maintain a commitment cache")
            .attempts
            .len = 2;

        let error = simulation
            .process_command(CommandRequest::new(
                CommandRequestId::new(1),
                0,
                CommandEnvelope::new(
                    Issuer::Debug,
                    Command::DebugSetArmyMorale {
                        army: ids.army,
                        morale: 101,
                    },
                ),
            ))
            .expect_err("a fatal commitment-cache mismatch must abort the rejection transaction");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert_eq!(simulation.snapshot(), before);
        let restored_cache = simulation
            .state
            .metadata
            .commitment_cache
            .as_ref()
            .expect("rollback should restore the private cache");
        assert_eq!(restored_cache.attempts.len, 2);
        assert!(simulation.command_attempts().is_empty());
        assert_eq!(simulation.state.counters.next_command_attempt_id, 1);
        assert_eq!(simulation.revision(), 0);

        simulation.state.metadata.commitment_cache = None;
        simulation
            .refresh_checkpoint_hash()
            .expect("discarding the injected corrupt cache should rebuild it from evidence");
        let outcome = simulation
            .process_command(CommandRequest::new(
                CommandRequestId::new(1),
                0,
                CommandEnvelope::new(
                    Issuer::Debug,
                    Command::DebugSetArmyMorale {
                        army: ids.army,
                        morale: 101,
                    },
                ),
            ))
            .expect("the repaired runtime should persist the same expected rejection");
        assert!(matches!(outcome, CommandOutcome::Rejected { .. }));
        let snapshot = simulation.snapshot();
        assert_eq!(
            snapshot.commitment_roots,
            Some(
                snapshot_commitment_roots(&snapshot)
                    .expect("the repaired rejection should independently reproduce its roots")
            )
        );
    }

    #[test]
    fn command_transaction_restores_writable_domains_after_hash_failure() {
        let (scenario, ids) = demo_scenario();
        let mut simulation =
            Simulation::new(109, scenario).expect("command rollback fixture should load");
        let before = simulation.snapshot();
        simulation
            .state
            .metadata
            .commitment_cache
            .as_mut()
            .expect("current runtimes should maintain a commitment cache")
            .commands
            .len = 2;
        let request = || {
            CommandRequest::new(
                CommandRequestId::new(1),
                0,
                CommandEnvelope::new(
                    Issuer::Debug,
                    Command::DebugSetArmyMorale {
                        army: ids.army,
                        morale: 73,
                    },
                ),
            )
        };

        let error = simulation
            .process_command(request())
            .expect_err("a fatal commitment-cache mismatch must abort command application");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert_eq!(simulation.snapshot(), before);
        let restored_cache = simulation
            .state
            .metadata
            .commitment_cache
            .as_ref()
            .expect("rollback should restore the private cache");
        assert_eq!(restored_cache.commands.len, 2);

        simulation.state.metadata.commitment_cache = None;
        simulation
            .refresh_checkpoint_hash()
            .expect("discarding the injected corrupt cache should rebuild it from evidence");
        let outcome = simulation
            .process_command(request())
            .expect("the repaired runtime should accept the same command");
        assert!(matches!(outcome, CommandOutcome::Accepted { .. }));
        let snapshot = simulation.snapshot();
        assert_eq!(
            snapshot.commitment_roots,
            Some(
                snapshot_commitment_roots(&snapshot)
                    .expect("the repaired command should independently reproduce its roots")
            )
        );
    }

    #[test]
    fn snapshot_round_trip_preserves_pending_work() {
        let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
        simulation
            .submit(move_order(&ids))
            .expect("order should validate");
        let json = simulation
            .snapshot_json()
            .expect("snapshot should serialize");
        let mut unsupported = simulation.snapshot();
        assert_eq!(unsupported.engine_version, ENGINE_VERSION);
        assert_eq!(unsupported.snapshot_format_version, SNAPSHOT_FORMAT_VERSION);
        unsupported.snapshot_format_version += 1;
        let Err(error) = Simulation::from_snapshot(unsupported) else {
            panic!("unknown snapshot formats must be rejected");
        };
        assert_eq!(error.code, ErrorCode::UnsupportedSnapshotVersion);

        let mut unmigrated_engine = simulation.snapshot();
        unmigrated_engine.engine_version = "0.4.0-other".to_owned();
        refresh_snapshot_commitments_and_checkpoint(&mut unmigrated_engine);
        let Err(error) = Simulation::from_snapshot(unmigrated_engine) else {
            panic!("current-format snapshots from another engine must require migration");
        };
        assert_eq!(error.code, ErrorCode::UnsupportedSnapshotVersion);

        let mut legacy_value = serde_json::to_value(simulation.snapshot())
            .expect("snapshot should convert to JSON value");
        let legacy_object = legacy_value
            .as_object_mut()
            .expect("snapshot JSON should be an object");
        legacy_object.insert("snapshot_format_version".to_owned(), Value::from(2));
        legacy_object.remove("run_manifest");
        legacy_object.remove("run_manifest_hash");
        legacy_object.remove("run_configuration");
        legacy_object.remove("checkpoint_hash");
        legacy_object.remove("commitment_format_version");
        legacy_object.remove("commitment_roots");
        legacy_object.remove("revision_format_version");
        legacy_object.remove("state_revision");
        legacy_object.remove("replay_revision_format_version");
        legacy_object.remove("admission_cursor_format_version");
        legacy_object.remove("admitted_attempt_count");
        legacy_object.remove("admitted_command_count");
        legacy_object.remove("admitted_event_count");
        legacy_object.remove("boundaries");
        legacy_object.remove("next_boundary_id");
        legacy_object.remove("root_seed");
        legacy_object.remove("random_streams");
        legacy_object.remove("random_draws");
        legacy_object.remove("next_random_draw_id");
        legacy_object.insert(
            "rng".to_owned(),
            serde_json::to_value(DeterministicRng::from_seed(35))
                .expect("legacy RNG fixture should serialize"),
        );
        let legacy_json =
            serde_json::to_string(&legacy_value).expect("legacy snapshot fixture should serialize");
        let migrated = Simulation::from_snapshot_json(&legacy_json)
            .expect("format 2 snapshot should migrate explicitly");
        assert_eq!(
            migrated.snapshot().snapshot_format_version,
            SNAPSHOT_FORMAT_VERSION
        );
        assert_eq!(migrated.snapshot().engine_version, ENGINE_VERSION);
        assert!(migrated.boundaries().is_empty());
        let legacy_journal = migrated.replay_journal();
        let (initial_scenario, _) = demo_scenario();
        let Err(error) = Simulation::replay_from_journal(initial_scenario, &[], &legacy_journal)
        else {
            panic!("identity-unbound legacy checkpoints must not claim exact replay");
        };
        assert_eq!(error.code, ErrorCode::LegacyReplayUnavailable);

        let mut restored = Simulation::from_snapshot_json(&json).expect("snapshot should restore");
        restored
            .advance(SimDuration::days(1))
            .expect("pending arrival should execute");
        assert_eq!(
            restored
                .world()
                .army(ids.army)
                .expect("army exists")
                .location,
            ids.eastern_territory
        );
        let mut changed_delivery = restored.snapshot();
        let mut changed_dispatch = None;
        for event in &mut changed_delivery.events {
            if let EventKind::ReportDispatched { arrives_at, .. } = &mut event.kind {
                *arrives_at += SimDuration::minutes(1);
                changed_dispatch = Some((event.id, *arrives_at));
                break;
            }
        }
        let (dispatch_event, changed_arrival) =
            changed_dispatch.expect("arrival should dispatch an observer report");
        let scheduled = changed_delivery
            .scheduled
            .iter_mut()
            .find(|record| {
                matches!(
                    record.action,
                    ScheduledAction::KnowledgeReport {
                        dispatch_event: candidate,
                        ..
                    } if candidate == dispatch_event
                )
            })
            .expect("the dispatched report should remain pending");
        scheduled.key.at = changed_arrival;
        refresh_snapshot_commitments_and_checkpoint(&mut changed_delivery);
        let Err(error) = Simulation::from_snapshot(changed_delivery) else {
            panic!("report timing must remain tied to its recorded random draw");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert!(error.message.contains("random draw"));

        let mut missing_draw = restored.snapshot();
        missing_draw.random_draws.clear();
        let core_stream = missing_draw
            .random_streams
            .iter_mut()
            .find(|state| state.key == random::core_report_delay_stream())
            .expect("the core report-delay stream should be persisted");
        core_stream.position = 0;
        core_stream.generator_state = core_stream.seed;
        missing_draw.next_random_draw_id = 1;
        refresh_snapshot_commitments_and_checkpoint(&mut missing_draw);
        let Err(error) = Simulation::from_snapshot(missing_draw) else {
            panic!("every report dispatch must retain its generating random draw");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert!(error.message.contains("core random draw"));

        let mut malformed_legacy =
            serde_json::to_value(restored.snapshot()).expect("snapshot should convert to JSON");
        let malformed_object = malformed_legacy
            .as_object_mut()
            .expect("snapshot JSON should be an object");
        malformed_object.insert("snapshot_format_version".to_owned(), Value::from(3));
        malformed_object.remove("run_manifest");
        malformed_object.remove("run_manifest_hash");
        malformed_object.remove("run_configuration");
        malformed_object.remove("checkpoint_hash");
        malformed_object.remove("commitment_format_version");
        malformed_object.remove("commitment_roots");
        malformed_object.remove("revision_format_version");
        malformed_object.remove("state_revision");
        malformed_object.remove("replay_revision_format_version");
        malformed_object.remove("admission_cursor_format_version");
        malformed_object.remove("admitted_attempt_count");
        malformed_object.remove("admitted_command_count");
        malformed_object.remove("admitted_event_count");
        malformed_object.remove("root_seed");
        malformed_object.remove("random_streams");
        malformed_object.remove("random_draws");
        malformed_object.remove("next_random_draw_id");
        malformed_object.insert(
            "rng".to_owned(),
            serde_json::to_value(DeterministicRng::from_seed(35))
                .expect("legacy RNG fixture should serialize"),
        );
        let malformed_dispatch = malformed_object
            .get_mut("events")
            .and_then(Value::as_array_mut)
            .and_then(|events| {
                events.iter_mut().find(|event| {
                    event
                        .get("kind")
                        .and_then(|kind| kind.get("type"))
                        .and_then(Value::as_str)
                        == Some("report_dispatched")
                })
            })
            .expect("the legacy fixture should contain a report dispatch");
        malformed_dispatch["timestamp"] = Value::from(i64::MAX);
        malformed_dispatch["kind"]["arrives_at"] = Value::from(i64::MIN);
        let malformed_json = serde_json::to_string(&malformed_legacy)
            .expect("malformed legacy fixture should still serialize");
        let Err(error) = Simulation::from_snapshot_json(&malformed_json) else {
            panic!("legacy report-time overflow must return a structured error");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert!(error.message.contains("legacy report timing"));

        let report_pending = restored
            .snapshot_json()
            .expect("pending reports should serialize");
        let mut report_restored = Simulation::from_snapshot_json(&report_pending)
            .expect("pending report evidence should restore");
        report_restored
            .advance(SimDuration::days(3))
            .expect("pending reports should be delivered");
        let delivered = report_restored
            .snapshot_json()
            .expect("delivered reports should serialize");
        Simulation::from_snapshot_json(&delivered)
            .expect("completed report evidence should restore without pending work");
    }

    #[test]
    fn pre_policy_format_four_journals_hydrate_compatibility_provenance() {
        let (scenario, ids) = demo_scenario();
        let mut simulation =
            Simulation::new(73, scenario.clone()).expect("compatibility run should load");
        simulation
            .submit(move_order(&ids))
            .expect("legacy command fixture should be accepted");
        let mut value =
            serde_json::to_value(simulation.replay_journal()).expect("journal should become JSON");
        let object = value
            .as_object_mut()
            .expect("replay journal JSON should be an object");
        object.remove("run_configuration");
        object.remove("command_attempts");
        let hydrated: ReplayJournal =
            serde_json::from_value(value).expect("pre-policy journal should deserialize");
        assert_eq!(
            hydrated.run_configuration,
            RunConfigurationSnapshot::CompatibilityV1
        );
        assert!(hydrated.command_attempts.is_empty());
        let replayed = Simulation::replay_from_journal(scenario.clone(), &[], &hydrated)
            .expect("pre-policy compatibility journal should replay exactly");
        assert_eq!(simulation.snapshot(), replayed.snapshot());

        let mut aliased = Simulation::new(74, scenario)
            .expect("compatibility run should load")
            .snapshot();
        aliased.run_configuration = Some(RunConfigurationSnapshot::ManifestOnlyV1);
        assert_eq!(
            snapshot_checkpoint_hash(&aliased)
                .expect("the provenance alias should remain checkpoint-neutral"),
            aliased.checkpoint_hash
        );
        let Err(error) = Simulation::from_snapshot(aliased) else {
            panic!("default run identity must have exactly one policy provenance");
        };
        assert_eq!(error.code, ErrorCode::InvalidRunManifest);
    }

    #[test]
    fn pre_policy_format_four_custom_run_identity_remains_loadable() {
        let (scenario, _) = demo_scenario();
        let mut legacy = Simulation::new(73, scenario.clone())
            .expect("compatibility run should load")
            .snapshot();
        let scenario_manifest =
            ArtifactManifest::for_scenario("legacy", "scenario", "1", &scenario)
                .expect("scenario should hash");
        let run_configuration =
            ArtifactManifest::from_bytes("legacy", "custom-run-policy", "7", b"opaque-policy")
                .expect("legacy policy identity should hash");
        let run_manifest = RunManifest::declared(scenario_manifest, run_configuration);
        legacy.run_manifest_hash = manifest::hash(&run_manifest).expect("manifest should hash");
        legacy.run_manifest = Some(run_manifest);
        legacy.run_configuration = Some(RunConfigurationSnapshot::ManifestOnlyV1);
        refresh_snapshot_commitments_and_checkpoint(&mut legacy);
        let expected = legacy.clone();

        let mut value = serde_json::to_value(legacy).expect("snapshot should become JSON");
        value
            .as_object_mut()
            .expect("snapshot JSON should be an object")
            .remove("run_configuration");
        let json = serde_json::to_string(&value).expect("legacy snapshot should serialize");
        let restored = Simulation::from_snapshot_json(&json)
            .expect("custom pre-policy format-4 identity should hydrate explicitly");
        assert_eq!(
            restored.run_configuration(),
            &RunConfigurationSnapshot::ManifestOnlyV1
        );
        assert_eq!(restored.snapshot(), expected);
        let journal = restored.replay_journal();
        let mut journal_value =
            serde_json::to_value(&journal).expect("custom journal should become JSON");
        let journal_object = journal_value
            .as_object_mut()
            .expect("custom journal JSON should be an object");
        journal_object.remove("run_configuration");
        journal_object.remove("command_attempts");
        let hydrated_journal: ReplayJournal = serde_json::from_value(journal_value)
            .expect("custom pre-policy journal should deserialize");
        assert_eq!(
            hydrated_journal.run_configuration,
            RunConfigurationSnapshot::ManifestOnlyV1
        );
        let replayed = Simulation::replay_from_journal(scenario, &[], &hydrated_journal)
            .expect("manifest-only format-4 evidence should remain exactly replayable");
        assert_eq!(restored.snapshot(), replayed.snapshot());
    }

    #[test]
    fn persistence_boundaries_reject_unloadable_or_noncanonical_state() {
        let (mut in_flight, in_flight_ids) = demo_scenario();
        in_flight.world.armies[0].transit = Some(TransitState {
            from: in_flight_ids.central_territory,
            to: in_flight_ids.eastern_territory,
            departed_at: in_flight.start_time,
            arrives_at: in_flight.start_time + SimDuration::days(1),
        });
        let Err(error) = Simulation::new(35, in_flight) else {
            panic!("initial transit without queue evidence must be rejected");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let (mut non_finite, _) = demo_scenario();
        non_finite.world.territories[0].position.x = f32::NAN;
        let Err(error) = Simulation::new(35, non_finite) else {
            panic!("non-finite map coordinates must be rejected");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
        simulation
            .submit(move_order(&ids))
            .expect("order should validate");
        let valid = simulation.snapshot();

        let mut past_schedule = valid.clone();
        past_schedule.scheduled[0].key.at =
            SimTime::from_minutes(past_schedule.now.as_minutes() - 1);
        let Err(error) = Simulation::from_snapshot(past_schedule) else {
            panic!("past scheduled work must be rejected");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut duplicate_arrival = valid.clone();
        let mut second_arrival = duplicate_arrival.scheduled[0].clone();
        second_arrival.key.sequence = duplicate_arrival.next_schedule_sequence;
        duplicate_arrival.next_schedule_sequence += 1;
        duplicate_arrival.scheduled.push(second_arrival);
        let Err(error) = Simulation::from_snapshot(duplicate_arrival) else {
            panic!("duplicate logical arrivals must be rejected");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut mismatched_arrival = valid.clone();
        mismatched_arrival.scheduled[0].key.at += SimDuration::minutes(1);
        let Err(error) = Simulation::from_snapshot(mismatched_arrival) else {
            panic!("arrival queue time must match transit and order evidence");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut stuck_transit = valid.clone();
        stuck_transit.scheduled.clear();
        let Err(error) = Simulation::from_snapshot(stuck_transit) else {
            panic!("an in-transit army must retain exactly one arrival action");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut reopened_registration = valid.clone();
        reopened_registration.plugin_registration_closed = false;
        let Err(error) = Simulation::from_snapshot(reopened_registration) else {
            panic!("executed snapshots must not reopen plugin registration");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut stale_counter = valid.clone();
        stale_counter.next_event_id = stale_counter
            .events
            .last()
            .expect("movement emitted an event")
            .id
            .get();
        let Err(error) = Simulation::from_snapshot(stale_counter) else {
            panic!("stale counters must be rejected");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut broken_reference = valid;
        broken_reference.world.armies[0].commander = PersonId::new(999);
        let Err(error) = Simulation::from_snapshot(broken_reference) else {
            panic!("broken entity references must be rejected");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut exhausted_counter = simulation.snapshot();
        exhausted_counter.next_command_id = u64::MAX;
        refresh_snapshot_commitments_and_checkpoint(&mut exhausted_counter);
        let mut restored =
            Simulation::from_snapshot(exhausted_counter).expect("the exhausted sentinel is valid");
        let before = restored.snapshot();
        let error = restored
            .submit(CommandEnvelope::new(
                Issuer::Debug,
                Command::DebugSetArmyMorale {
                    army: ids.army,
                    morale: 50,
                },
            ))
            .expect_err("counter exhaustion must be a structured failure");
        assert_eq!(error.code, ErrorCode::IdentifierExhausted);
        assert_eq!(before, restored.snapshot());
    }

    #[test]
    fn plugin_command_receives_issuer_and_namespaces_state() {
        let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
        simulation
            .register_plugin(&AuthorityPlugin)
            .expect("plugin should register");

        let before = simulation
            .snapshot_json()
            .expect("snapshot should serialize");
        let rejected = simulation.submit(CommandEnvelope::new(
            Issuer::Actor(ids.observer),
            Command::Plugin {
                plugin: "authority-test".to_owned(),
                command: "set_stance".to_owned(),
                payload: Value::Null,
            },
        ));
        assert_eq!(
            rejected.expect_err("wrong actor must be rejected").code,
            ErrorCode::InvalidAuthority
        );
        assert_eq!(
            before,
            simulation
                .snapshot_json()
                .expect("snapshot should serialize")
        );

        let invalid_payload = simulation.submit(CommandEnvelope::new(
            Issuer::Actor(ids.commander),
            Command::Plugin {
                plugin: "authority-test".to_owned(),
                command: "set_stance".to_owned(),
                payload: serde_json::json!({}),
            },
        ));
        assert_eq!(
            invalid_payload
                .expect_err("payloads must match their declared schema")
                .code,
            ErrorCode::InvalidPayload
        );
        assert_eq!(
            before,
            simulation
                .snapshot_json()
                .expect("payload rejection must not mutate the simulation")
        );

        simulation
            .submit(CommandEnvelope::new(
                Issuer::Actor(ids.commander),
                Command::Plugin {
                    plugin: "authority-test".to_owned(),
                    command: "set_stance".to_owned(),
                    payload: Value::Null,
                },
            ))
            .expect("authorized actor should be accepted");
        let snapshot = simulation.snapshot();
        assert_eq!(snapshot.plugin_components.len(), 1);
        assert_eq!(snapshot.plugin_components[0].plugin, "authority-test");
        assert_eq!(
            snapshot.plugin_components[0].state,
            StateKey::new("military", "stance")
        );
        assert_eq!(snapshot.plugin_components[0].component, "stance");
        assert_eq!(
            simulation
                .register_plugin(&MarkerPlugin {
                    name: "late-plugin",
                    writes: Vec::new(),
                })
                .expect_err("new plugins cannot appear after execution begins")
                .code,
            ErrorCode::PluginRegistrationClosed
        );
    }

    #[test]
    fn plugin_registration_is_atomic_and_rejects_duplicate_state_owners() {
        let (mut simulation, _) = Simulation::demo(35).expect("demo should load");
        simulation
            .register_plugin(&MarkerPlugin {
                name: "first-owner",
                writes: vec![StateKey::new("shared-domain", "balance")],
            })
            .expect("first owner should register");
        let before = simulation
            .snapshot_json()
            .expect("snapshot should serialize");
        let error = simulation
            .register_plugin(&MarkerPlugin {
                name: "second-owner",
                writes: vec![StateKey::new("shared-domain", "balance")],
            })
            .expect_err("a second owner must be rejected");
        assert_eq!(error.code, ErrorCode::DuplicateStateOwner);
        assert_eq!(
            before,
            simulation
                .snapshot_json()
                .expect("failed registration must not change state or manifests")
        );
        simulation
            .register_plugin(&GhostPlugin)
            .expect("a caught registrar error may not poison the candidate registry");
        simulation
            .register_plugin(&MarkerPlugin {
                name: "fresh-owner",
                writes: vec![StateKey::new("fresh-domain", "value")],
            })
            .expect("the failed multi-key claim must leave no ghost owner");
        simulation
            .register_plugin(&BoundaryGhostPlugin)
            .expect("a caught boundary registrar error may not poison the candidate registry");
        simulation
            .register_plugin(&MarkerPlugin {
                name: "boundary-ghost-owner",
                writes: vec![StateKey::new("boundary-ghost", "value")],
            })
            .expect("a later boundary-writer failure must leave no ghost owner");
    }

    #[test]
    fn phased_boundary_allocates_deterministically_and_respects_visibility() {
        let (scenario, _) = demo_scenario();
        let mut first = Simulation::new(35, scenario.clone()).expect("demo should load");
        first
            .register_plugin(&JournalCommandPlugin)
            .expect("journal command plugin should register");
        first
            .register_plugin(&GrainSupplyPlugin)
            .expect("supply plugin should register");
        first
            .register_plugin(&HighClaimPlugin)
            .expect("high claim should register");
        first
            .register_plugin(&LowClaimPlugin)
            .expect("low claim should register");
        first
            .register_plugin(&VisibilityValidatorPlugin)
            .expect("validator should register");

        let mut second = Simulation::new(35, scenario.clone()).expect("demo should load");
        second
            .register_plugin(&VisibilityValidatorPlugin)
            .expect("validator should register");
        second
            .register_plugin(&LowClaimPlugin)
            .expect("low claim should register");
        second
            .register_plugin(&HighClaimPlugin)
            .expect("high claim should register");
        second
            .register_plugin(&GrainSupplyPlugin)
            .expect("supply plugin should register");
        second
            .register_plugin(&JournalCommandPlugin)
            .expect("journal command plugin should register");

        for simulation in [&mut first, &mut second] {
            for _ in 0..2 {
                simulation
                    .submit(CommandEnvelope::new(
                        Issuer::Debug,
                        Command::Plugin {
                            plugin: "journal-command".to_owned(),
                            command: "noop".to_owned(),
                            payload: Value::Null,
                        },
                    ))
                    .expect("journal fixture command should be accepted");
            }
        }

        let request = BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily);
        let first_receipt = first
            .settle_boundary(request.clone())
            .expect("daily boundary should settle");
        let second_receipt = second
            .settle_boundary(request.clone())
            .expect("registration order must not change settlement");
        assert_eq!(first_receipt, second_receipt);
        assert_eq!(first.snapshot(), second.snapshot());
        let first_followup = first
            .settle_boundary(request.clone())
            .expect("a same-time follow-up boundary should settle");
        let second_followup = second
            .settle_boundary(request)
            .expect("the follow-up boundary must remain registration-order independent");
        assert_eq!(first_followup, second_followup);
        assert_eq!(first.snapshot(), second.snapshot());

        let allocations: BTreeMap<_, _> = first_receipt
            .allocations
            .iter()
            .map(|allocation| {
                (
                    allocation.reservation.plugin.as_str(),
                    (allocation.granted, allocation.disposition),
                )
            })
            .collect();
        assert_eq!(
            allocations.get("high-claim"),
            Some(&(7, ReservationDisposition::Fulfilled))
        );
        assert_eq!(
            allocations.get("low-claim"),
            Some(&(3, ReservationDisposition::Partial))
        );
        let components: BTreeMap<_, _> = first
            .snapshot()
            .plugin_components
            .into_iter()
            .map(|record| (record.component, record.value))
            .collect();
        assert_eq!(components.get("high").and_then(Value::as_u64), Some(7));
        assert_eq!(components.get("low").and_then(Value::as_u64), Some(3));

        let json = first
            .snapshot_json()
            .expect("settled boundary should serialize");
        let restored = Simulation::from_snapshot_json_with_plugins(
            &json,
            &[
                &GrainSupplyPlugin,
                &HighClaimPlugin,
                &LowClaimPlugin,
                &JournalCommandPlugin,
                &VisibilityValidatorPlugin,
            ],
        )
        .expect("settled boundary should rehydrate");
        assert_eq!(first.snapshot(), restored.snapshot());

        let plugins: &[&dyn SimulationPlugin] = &[
            &GrainSupplyPlugin,
            &HighClaimPlugin,
            &LowClaimPlugin,
            &JournalCommandPlugin,
            &VisibilityValidatorPlugin,
        ];
        let replayed = Simulation::replay_with_boundaries(
            35,
            scenario,
            plugins,
            first.command_log(),
            first.boundaries(),
            first.time(),
        )
        .expect("boundary journal should replay exactly");
        assert_eq!(first.snapshot(), replayed.snapshot());

        let mut corrupted_allocation = first.snapshot();
        corrupted_allocation.boundaries[0].allocations[0].granted += 1;
        let error = Simulation::from_snapshot_with_plugins(corrupted_allocation, plugins)
            .err()
            .expect("tampered allocation evidence must not load");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut corrupted_provenance = first.snapshot();
        corrupted_provenance.boundaries[0].emissions[0].system = "request".to_owned();
        let error = Simulation::from_snapshot_with_plugins(corrupted_provenance, plugins)
            .err()
            .expect("tampered boundary source provenance must not load");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut corrupted_command_cut = first.snapshot();
        corrupted_command_cut.boundaries[0].admitted_commands = vec![CommandId::new(2)];
        let error = Simulation::from_snapshot_with_plugins(corrupted_command_cut, plugins)
            .err()
            .expect("boundary admission must be a global command-journal prefix");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut corrupted_event_cut = first.snapshot();
        let later_event = corrupted_event_cut.boundaries[1].emissions[0].event;
        corrupted_event_cut.boundaries[0].admitted_events = vec![later_event];
        let error = Simulation::from_snapshot_with_plugins(corrupted_event_cut, plugins)
            .err()
            .expect("an earlier boundary cannot admit a later boundary event");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut corrupted_boundary_counter = first.snapshot();
        corrupted_boundary_counter.next_boundary_id += 1;
        let error = Simulation::from_snapshot_with_plugins(corrupted_boundary_counter, plugins)
            .err()
            .expect("the next boundary counter must not skip an identifier");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let (mut causal_cut, ids) = Simulation::demo(35).expect("demo should load");
        causal_cut
            .submit(move_order(&ids))
            .expect("movement should emit command-caused evidence");
        causal_cut
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("an evidence-only boundary should settle");
        assert!(causal_cut.boundaries()[0].emissions.is_empty());

        let mut omitted_same_time_event = causal_cut.snapshot();
        omitted_same_time_event.boundaries[0]
            .admitted_events
            .clear();
        let error = Simulation::from_snapshot(omitted_same_time_event)
            .err()
            .expect("a no-emission boundary cannot omit already caused same-time evidence");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut due_at_boundary = causal_cut.snapshot();
        due_at_boundary.scheduled[0].key.at = due_at_boundary.now;
        due_at_boundary.boundaries[0].state_hash = Some(
            snapshot_state_hash(&due_at_boundary)
                .expect("the structurally corrupted state should hash"),
        );
        due_at_boundary.boundaries[0].hash = compute_boundary_hash(&due_at_boundary.boundaries[0])
            .expect("the structurally corrupted boundary should hash");
        refresh_snapshot_commitments_and_checkpoint(&mut due_at_boundary);
        let error = Simulation::from_snapshot(due_at_boundary)
            .err()
            .expect("completed boundaries cannot retain due ingress");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert!(error.message.contains("future-dated"));
    }

    #[test]
    fn domain_record_lifecycle_is_atomic_replayable_and_tamper_evident() {
        let (scenario, _) = demo_scenario();
        let mut record_free = Simulation::new(87, scenario.clone())
            .expect("record-free compatibility fixture should load");
        let record_free_snapshot = record_free.snapshot();
        assert!(
            record_free_snapshot.initial_scenario.is_none(),
            "record-free format-4 state must retain its prior additive shape"
        );
        let mut redundant_initial_scenario = record_free_snapshot.clone();
        redundant_initial_scenario.initial_scenario = Some(scenario.clone());
        refresh_snapshot_commitments_and_checkpoint(&mut redundant_initial_scenario);
        let error = Simulation::from_snapshot(redundant_initial_scenario)
            .err()
            .expect("record-free snapshots must not carry ignored genesis state");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        let mut record_free_restored = Simulation::from_snapshot(record_free_snapshot)
            .expect("a pristine record-free snapshot should restore");
        record_free
            .register_plugin(&RecordLifecyclePlugin)
            .expect("the original pristine runtime should accept record schemas");
        record_free_restored
            .register_plugin(&RecordLifecyclePlugin)
            .expect("a restored pristine runtime must retain record-schema capability");
        assert_eq!(record_free.snapshot(), record_free_restored.snapshot());
        let mut initial_scenario = scenario.clone();
        initial_scenario.domain_records = vec![
            initial_record(
                "cm-record-lifecycle",
                DomainRecordClass::Entity,
                office_draft("office-a", "Grand Secretariat"),
            ),
            initial_record(
                "cm-record-lifecycle",
                DomainRecordClass::Entity,
                office_draft("office-b", "Successor Secretariat"),
            ),
            initial_record(
                "cm-record-lifecycle",
                DomainRecordClass::Record,
                obligation_draft("office-a", "open"),
            ),
        ];
        let error = Simulation::new(88, initial_scenario.clone())
            .err()
            .expect("initial domain records must not create a half-configured runtime");
        assert_eq!(error.code, ErrorCode::PluginNotActive);
        let initial = Simulation::new_with_plugins(88, initial_scenario, &[&RecordLifecyclePlugin])
            .expect("plugin-aware construction should validate initial domain records");
        let initial_json = initial
            .snapshot_json()
            .expect("configured initial record state should serialize");
        let initial_restored =
            Simulation::from_snapshot_json_with_plugins(&initial_json, &[&RecordLifecyclePlugin])
                .expect("configured initial record state should reload immediately");
        assert_eq!(initial.snapshot(), initial_restored.snapshot());

        let mut simulation =
            Simulation::new(89, scenario.clone()).expect("record fixture should load");
        simulation
            .register_plugin(&RecordLifecyclePlugin)
            .expect("record lifecycle plugin should register");
        let request = BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily);
        let created = simulation
            .settle_boundary(request.clone())
            .expect("record creation boundary should settle");
        let retired = simulation
            .settle_boundary(request.clone())
            .expect("record retirement boundary should settle");
        let succession = simulation
            .settle_boundary(request.clone())
            .expect("a later successor should retire without invalidating its predecessor");
        let deleted = simulation
            .settle_boundary(request)
            .expect("atomic reference transfer and deletion should settle");
        assert_eq!(created.record_change_count, 3);
        assert_eq!(retired.record_change_count, 2);
        assert_eq!(succession.record_change_count, 1);
        assert_eq!(deleted.record_change_count, 2);
        assert_eq!(
            created.change_count
                + retired.change_count
                + succession.change_count
                + deleted.change_count,
            1
        );

        let original = simulation
            .domain_record(&office_reference("office-a"))
            .expect("deleted office tombstone should remain addressable");
        assert!(original.is_deleted());
        assert_eq!(original.version, 3);
        let obligation = simulation
            .domain_record(&obligation_reference())
            .expect("transferred obligation should remain present");
        assert_eq!(obligation.version, 2);
        assert!(obligation.references.iter().any(|reference| {
            reference.target == DomainReferenceTarget::Domain(office_reference("office-c"))
        }));
        assert!(matches!(
            &simulation
                .domain_record(&office_reference("office-b"))
                .expect("the intermediate successor should remain addressable")
                .lifecycle,
            DomainRecordLifecycle::Retired {
                successor: Some(successor),
                ..
            } if successor == &office_reference("office-c")
        ));
        assert!(simulation.boundaries().iter().all(|boundary| {
            boundary.record_changes.len()
                == boundary
                    .emissions
                    .iter()
                    .filter(|emission| {
                        matches!(emission.kind, BoundaryEmissionKind::RecordChange { .. })
                    })
                    .count()
        }));

        let before_stale_update = simulation
            .snapshot_json()
            .expect("pre-conflict record state should serialize");
        let conflict = simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
            .expect_err("stale record versions must reject before commit");
        assert_eq!(conflict.code, ErrorCode::DomainRecordVersionConflict);
        assert_eq!(
            before_stale_update,
            simulation
                .snapshot_json()
                .expect("version conflicts must roll back the complete boundary")
        );
        let quiet = simulation
            .settle_boundary(
                BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Monthly),
            )
            .expect("an unrelated cadence should publish an empty later boundary");
        assert_eq!(quiet.change_count + quiet.record_change_count, 0);

        let json = simulation
            .snapshot_json()
            .expect("domain-record snapshot should serialize");
        let restored =
            Simulation::from_snapshot_json_with_plugins(&json, &[&RecordLifecyclePlugin])
                .expect("domain-record evidence should restore with exact plugin code");
        assert_eq!(simulation.snapshot(), restored.snapshot());

        let plugins: &[&dyn SimulationPlugin] = &[&RecordLifecyclePlugin];
        let replayed = Simulation::replay_with_boundaries(
            89,
            scenario,
            plugins,
            simulation.command_log(),
            simulation.boundaries(),
            simulation.time(),
        )
        .expect("domain-record boundary evidence should replay exactly");
        assert_eq!(simulation.snapshot(), replayed.snapshot());

        let mut cross_system_creation = simulation.snapshot();
        let observer_event = cross_system_creation.boundaries[0]
            .emissions
            .iter()
            .find_map(|emission| {
                (emission.system == "observer"
                    && matches!(emission.kind, BoundaryEmissionKind::Explicit))
                .then_some(emission.event)
            })
            .expect("the independent observer should emit boundary evidence");
        cross_system_creation
            .events
            .iter_mut()
            .find(|event| event.id == observer_event)
            .expect("the observer event should exist")
            .affected_entities = vec![EntityRef::Domain(office_reference("office-b"))];
        let final_state_hash = snapshot_state_hash(&cross_system_creation)
            .expect("the cross-system creation forgery should have coherent final state");
        cross_system_creation
            .boundaries
            .last_mut()
            .expect("the fixture should have a boundary head")
            .state_hash = Some(final_state_hash);
        rehash_tampered_snapshot(&mut cross_system_creation);
        let error = Simulation::from_snapshot_with_plugins(cross_system_creation, plugins)
            .err()
            .expect("one proposal cannot consume another system's same-stage creation");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut precreation_reference = simulation.snapshot();
        let marker_change = precreation_reference.boundaries[0]
            .changes
            .first_mut()
            .expect("the first boundary should persist its marker change");
        marker_change.entity = EntityRef::Domain(office_reference("office-c"));
        let marker_event = precreation_reference.boundaries[0]
            .emissions
            .iter()
            .find_map(|emission| {
                matches!(
                    emission.kind,
                    BoundaryEmissionKind::Change { change_index: 0 }
                )
                .then_some(emission.event)
            })
            .expect("the marker change should have causal event evidence");
        precreation_reference
            .events
            .iter_mut()
            .find(|event| event.id == marker_event)
            .expect("the marker change event should exist")
            .affected_entities = vec![EntityRef::Domain(office_reference("office-c"))];
        precreation_reference
            .plugin_components
            .iter_mut()
            .find(|record| record.component == "status")
            .expect("the persisted marker component should exist")
            .entity = EntityRef::Domain(office_reference("office-c"));
        precreation_reference
            .plugin_components
            .sort_by_key(|record| {
                component_key(
                    &record.plugin,
                    &record.state,
                    &record.entity,
                    &record.component,
                )
            });
        let final_state_hash = snapshot_state_hash(&precreation_reference)
            .expect("the pre-creation forgery should have coherent final state");
        precreation_reference
            .boundaries
            .last_mut()
            .expect("the fixture should have a boundary head")
            .state_hash = Some(final_state_hash);
        rehash_tampered_snapshot(&mut precreation_reference);
        let error = Simulation::from_snapshot_with_plugins(precreation_reference, plugins)
            .err()
            .expect("earlier evidence cannot reference an entity created by a later boundary");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut post_deletion_reference = simulation.snapshot();
        let (last_boundary_id, last_boundary_at, last_boundary_correlation) = {
            let last_boundary = post_deletion_reference
                .boundaries
                .last_mut()
                .expect("the fixture should have an empty later boundary");
            last_boundary.cadences = vec![SystemCadence::Daily];
            (
                last_boundary.id,
                last_boundary.at,
                last_boundary.correlation_id,
            )
        };
        let event_id = EventId::new(post_deletion_reference.next_event_id);
        post_deletion_reference.next_event_id = post_deletion_reference
            .next_event_id
            .checked_add(1)
            .expect("the tamper fixture should have event ID capacity");
        post_deletion_reference.events.push(SimEvent {
            id: event_id,
            timestamp: last_boundary_at,
            kind: EventKind::Plugin {
                plugin: "cm-record-lifecycle".to_owned(),
                event_type: "record_probe".to_owned(),
            },
            affected_entities: vec![EntityRef::Domain(office_reference("office-a"))],
            summary: "Forge evidence after the office was deleted".to_owned(),
            cause: Some(CauseRef::Boundary(last_boundary_id)),
            correlation_id: last_boundary_correlation,
        });
        post_deletion_reference
            .boundaries
            .last_mut()
            .expect("the fixture should retain its boundary head")
            .emissions
            .push(BoundaryEmission {
                plugin: "cm-record-lifecycle".to_owned(),
                system: "lifecycle".to_owned(),
                event: event_id,
                kind: BoundaryEmissionKind::Explicit,
            });
        let final_state_hash = snapshot_state_hash(&post_deletion_reference)
            .expect("the post-deletion forgery should have coherent final state");
        post_deletion_reference
            .boundaries
            .last_mut()
            .expect("the fixture should retain its boundary head")
            .state_hash = Some(final_state_hash);
        rehash_tampered_snapshot(&mut post_deletion_reference);
        let error = Simulation::from_snapshot_with_plugins(post_deletion_reference, plugins)
            .err()
            .expect("later evidence cannot reference a deleted domain entity");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut corrupted = simulation.snapshot();
        corrupted.boundaries[1].record_changes[0].system = "forged-system".to_owned();
        rehash_tampered_snapshot(&mut corrupted);
        let error = Simulation::from_snapshot_with_plugins(corrupted, plugins)
            .err()
            .expect("forged domain-record provenance must not load");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut shifted_to_genesis = simulation.snapshot();
        let forged_initial_records = shifted_to_genesis.domain_records.clone();
        shifted_to_genesis
            .initial_scenario
            .as_mut()
            .expect("new snapshots retain their manifest-bound initial scenario")
            .domain_records
            .clone_from(&forged_initial_records);
        shifted_to_genesis.boundaries.clear();
        shifted_to_genesis.events.clear();
        shifted_to_genesis.plugin_registration_closed = false;
        shifted_to_genesis.next_event_id = 1;
        shifted_to_genesis.next_boundary_id = 1;
        shifted_to_genesis.next_correlation_id = 1;
        refresh_snapshot_commitments_and_checkpoint(&mut shifted_to_genesis);
        let error = Simulation::from_snapshot_with_plugins(shifted_to_genesis, plugins)
            .err()
            .expect("record creations cannot be relabeled as manifest-bound genesis state");
        assert_eq!(error.code, ErrorCode::InvalidRunManifest);

        let mut stripped_feature = simulation.snapshot();
        stripped_feature.initial_scenario = None;
        stripped_feature.domain_records.clear();
        stripped_feature.boundaries.clear();
        stripped_feature.events.clear();
        stripped_feature.plugin_registration_closed = false;
        stripped_feature.next_event_id = 1;
        stripped_feature.next_boundary_id = 1;
        stripped_feature.next_correlation_id = 1;
        refresh_snapshot_commitments_and_checkpoint(&mut stripped_feature);
        let error = Simulation::from_snapshot_with_plugins(stripped_feature, plugins)
            .err()
            .expect("record schemas cannot downgrade to an unbound old-v4 snapshot shape");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    }

    #[test]
    fn domain_record_delete_rejects_live_references_and_rolls_back() {
        let (scenario, _) = demo_scenario();
        let mut simulation = Simulation::new(90, scenario).expect("record fixture should load");
        simulation
            .register_plugin(&RecordDeleteOnlyPlugin)
            .expect("invalid-delete fixture should register");
        let request = BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily);
        simulation
            .settle_boundary(request.clone())
            .expect("record creation should settle");
        simulation
            .settle_boundary(request.clone())
            .expect("record retirement should settle");
        let before = simulation
            .snapshot_json()
            .expect("pre-failure state should serialize");
        let error = simulation
            .settle_boundary(request)
            .expect_err("a referenced record cannot be deleted");
        assert_eq!(error.code, ErrorCode::DomainRecordReferenced);
        assert_eq!(
            before,
            simulation
                .snapshot_json()
                .expect("failed deletion must restore every persisted field")
        );
    }

    #[test]
    fn domain_record_successor_cycles_are_rejected_in_genesis_and_atomic_bundles() {
        let (scenario, _) = demo_scenario();
        let mut cyclic_genesis = scenario.clone();
        let mut first = initial_record(
            "cm-record-cycle",
            DomainRecordClass::Entity,
            office_draft("office-a", "First Office"),
        );
        first.lifecycle = DomainRecordLifecycle::Retired {
            at: SimTime::EPOCH,
            successor: Some(office_reference("office-b")),
        };
        let mut second = initial_record(
            "cm-record-cycle",
            DomainRecordClass::Entity,
            office_draft("office-b", "Second Office"),
        );
        second.lifecycle = DomainRecordLifecycle::Retired {
            at: SimTime::EPOCH,
            successor: Some(office_reference("office-a")),
        };
        cyclic_genesis.domain_records = vec![first, second];
        let error = Simulation::new_with_plugins(91, cyclic_genesis, &[&RecordCyclePlugin])
            .err()
            .expect("cyclic successor state must not enter a new run");
        assert_eq!(error.code, ErrorCode::InvalidDomainRecord);

        let mut simulation = Simulation::new(92, scenario).expect("cycle fixture should load");
        simulation
            .register_plugin(&RecordCyclePlugin)
            .expect("cycle fixture plugin should register");
        let request = BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily);
        simulation
            .settle_boundary(request.clone())
            .expect("cycle fixture records should be created");
        let before = simulation
            .snapshot_json()
            .expect("pre-cycle state should serialize");
        let error = simulation
            .settle_boundary(request)
            .expect_err("mutual successor retirement must reject atomically");
        assert_eq!(error.code, ErrorCode::InvalidDomainRecord);
        assert_eq!(
            before,
            simulation
                .snapshot_json()
                .expect("failed successor cycles must roll back the whole boundary")
        );
    }

    #[test]
    fn domain_record_snapshot_cannot_delete_the_bound_seat_institution() {
        let (scenario, _) = demo_scenario();
        let mut initial_scenario = scenario;
        initial_scenario.domain_records = vec![initial_record(
            "cm-record-seat-deletion",
            DomainRecordClass::Entity,
            office_draft("office-a", "Bound Office"),
        )];
        let mut simulation = Simulation::new_with_plugins(
            93,
            initial_scenario.clone(),
            &[&RecordSeatDeletionPlugin],
        )
        .expect("unbound seat-deletion fixture should load");
        let request = BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily);
        simulation
            .settle_boundary(request.clone())
            .expect("the fixture office should retire");
        simulation
            .settle_boundary(request)
            .expect("an unbound retired office may be deleted");

        let configuration = RunConfiguration {
            format_version: RUN_CONFIGURATION_FORMAT_VERSION,
            purpose: RunPurpose::Play,
            controller: ControllerPolicy::HumanRoleBound,
            seat: SeatPolicy::InstitutionBound,
            observation: ObservationPolicy::ActorBound,
            interaction: InteractionPolicy::EraInternalCommands,
            trace: TracePolicy::Causal,
            seat_binding: Some(SeatBinding {
                seat_id: "seat.bound-office".to_owned(),
                controller_id: "controller.human".to_owned(),
                actor: None,
                institution: Some(EntityRef::Domain(office_reference("office-a"))),
                permission_profile_id: "permission.institution".to_owned(),
            }),
            declared_interventions: Vec::new(),
            diagnostic_commands_enabled: false,
            require_idempotency_keys: true,
        };
        let mut forged = simulation.snapshot();
        let run_manifest = manifest_for_configuration(&initial_scenario, &configuration);
        forged.run_manifest_hash =
            manifest::hash(&run_manifest).expect("forged manifest should hash canonically");
        forged.run_manifest = Some(run_manifest);
        forged.run_configuration = Some(RunConfigurationSnapshot::Declared(configuration));
        let final_state_hash = snapshot_state_hash(&forged)
            .expect("the forged institution-bound final state should hash");
        forged
            .boundaries
            .last_mut()
            .expect("the forged fixture should have a boundary head")
            .state_hash = Some(final_state_hash);
        rehash_tampered_snapshot(&mut forged);
        let error = Simulation::from_snapshot_with_plugins(forged, &[&RecordSeatDeletionPlugin])
            .err()
            .expect("a snapshot cannot delete the institution bound to its active seat");
        assert_eq!(error.code, ErrorCode::InvalidRunConfiguration);
    }

    #[test]
    fn failed_phased_boundary_restores_every_writable_domain_and_retries_exactly() {
        let (scenario, ids) = demo_scenario();
        let record_plugin = RecordLifecyclePlugin;
        let rollback_plugin = BoundaryRollbackPlugin;
        let random_plugin = PrimaryRandomPlugin;
        let mut simulation = Simulation::new(35, scenario).expect("rollback fixture should load");
        simulation
            .register_plugin(&record_plugin)
            .expect("record fixture should register");
        simulation
            .register_plugin(&rollback_plugin)
            .expect("rollback fixture should register");
        simulation
            .register_plugin(&random_plugin)
            .expect("random fixture should register");
        simulation
            .enqueue_command(
                SimTime::EPOCH,
                0,
                CommandRequest::new(
                    CommandRequestId::new(1),
                    simulation.revision(),
                    move_order(&ids),
                ),
            )
            .expect("the initial movement should queue");
        simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
            .expect("the initial boundary should create records and schedule the arrival");
        let arrival_at = SimTime::EPOCH
            .checked_add(SimDuration::hours(18))
            .expect("arrival time should be representable");
        let return_order = CommandEnvelope::new(
            Issuer::Actor(ids.commander),
            Command::MoveArmy {
                army: ids.army,
                destination: ids.western_territory,
            },
        )
        .at_time(arrival_at);
        simulation
            .enqueue_command(
                arrival_at,
                0,
                CommandRequest::new(
                    CommandRequestId::new(2),
                    simulation.revision(),
                    return_order,
                ),
            )
            .expect("the equal-time return order should queue");

        let baseline = simulation.snapshot();
        let mut control = Simulation::from_snapshot_with_plugins(
            baseline.clone(),
            &[&record_plugin, &rollback_plugin, &random_plugin],
        )
        .expect("the pending rollback fixture should reload exactly");
        let next_random_draw_id = simulation.state.counters.next_random_draw_id;
        simulation.state.counters.next_random_draw_id = u64::MAX - 1;
        let cache_before = cache_fingerprint(&simulation);
        let before = simulation
            .snapshot_json()
            .expect("snapshot should serialize");
        let error = simulation
            .settle_boundary(BoundaryRequest::at(arrival_at).with_cadence(SystemCadence::Daily))
            .expect_err("random-draw identifier exhaustion must abort the whole boundary");
        assert_eq!(error.code, ErrorCode::IdentifierExhausted);
        assert_eq!(
            before,
            simulation
                .snapshot_json()
                .expect("failed settlement must restore every serialized field")
        );
        assert_eq!(cache_fingerprint(&simulation), cache_before);

        simulation.state.counters.next_random_draw_id = next_random_draw_id;
        assert_eq!(simulation.snapshot(), baseline);
        let retry = simulation
            .settle_boundary(BoundaryRequest::at(arrival_at).with_cadence(SystemCadence::Daily))
            .expect("the repaired boundary should settle");
        let control_receipt = control
            .settle_boundary(BoundaryRequest::at(arrival_at).with_cadence(SystemCadence::Daily))
            .expect("the control boundary should settle");
        assert_eq!(retry, control_receipt);
        assert_eq!(simulation.snapshot(), control.snapshot());
        assert!(retry.change_count > 0);
        assert!(retry.record_change_count > 0);
        assert!(!retry.generated_ingress.is_empty());
        assert!(!retry.random_draws.is_empty());
    }

    #[test]
    fn scoped_random_streams_are_isolated_recorded_hashed_and_replayable() {
        let (scenario, _) = demo_scenario();
        let mut primary_only = Simulation::new(73, scenario.clone()).expect("demo should load");
        primary_only
            .register_plugin(&PrimaryRandomPlugin)
            .expect("primary random plugin should register");

        let mut with_noise = Simulation::new(73, scenario.clone()).expect("demo should load");
        with_noise
            .register_plugin(&NoiseRandomPlugin)
            .expect("noise random plugin should register");
        with_noise
            .register_plugin(&PrimaryRandomPlugin)
            .expect("primary random plugin should register");

        let request = BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily);
        let primary_receipt = primary_only
            .settle_boundary(request.clone())
            .expect("primary boundary should settle");
        with_noise
            .settle_boundary(request)
            .expect("noise boundary should settle");

        let primary_draw = primary_only
            .random_draws()
            .first()
            .expect("the primary system should record its draw");
        let isolated_draw = with_noise
            .random_draws()
            .iter()
            .find(|draw| draw.stream == primary_random_stream())
            .expect("the primary stream should remain present with unrelated noise");
        assert_eq!(primary_draw.value, isolated_draw.value);
        assert_eq!(primary_draw.position, isolated_draw.position);
        assert_eq!(primary_draw.id, primary_receipt.random_draws[0]);
        assert_eq!(
            primary_draw.cause,
            CauseRef::Boundary(primary_receipt.boundary_id)
        );
        assert!(matches!(
            &primary_draw.producer,
            RandomDrawProducer::BoundarySystem {
                boundary,
                plugin,
                system,
            } if *boundary == primary_receipt.boundary_id
                && plugin == "random-primary"
                && system == "roll"
        ));

        let first_hash = primary_receipt.boundary_hash;
        let second_receipt = primary_only
            .settle_boundary(
                BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(1))
                    .with_cadence(SystemCadence::Daily),
            )
            .expect("second primary boundary should settle");
        let second_boundary = primary_only
            .boundaries()
            .last()
            .expect("second boundary should be recorded");
        assert_eq!(second_boundary.previous_hash, first_hash);
        assert_eq!(second_boundary.hash, second_receipt.boundary_hash);
        assert!(second_boundary.state_hash.is_some());
        assert_eq!(
            primary_only.boundary_head_hash(),
            Some(second_receipt.boundary_hash.as_str())
        );

        let restored = Simulation::from_snapshot_with_plugins(
            primary_only.snapshot(),
            &[&PrimaryRandomPlugin],
        )
        .expect("scoped random evidence should survive snapshot restoration");
        assert_eq!(primary_only.snapshot(), restored.snapshot());

        let mut changed_state = primary_only.snapshot();
        changed_state.world.armies[0].morale += 1;
        let Err(error) =
            Simulation::from_snapshot_with_plugins(changed_state, &[&PrimaryRandomPlugin])
        else {
            panic!("persisted state cannot change while retaining its checkpoint commitment");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert!(error.message.contains("commitment roots"));

        let mut missing_state_commitment = primary_only.snapshot();
        missing_state_commitment.boundaries[0].state_hash = None;
        missing_state_commitment.boundaries[0].hash =
            compute_boundary_hash(&missing_state_commitment.boundaries[0])
                .expect("the malformed legacy-style boundary should hash");
        let first_hash = missing_state_commitment.boundaries[0].hash.clone();
        missing_state_commitment.boundaries[1].previous_hash = first_hash;
        missing_state_commitment.boundaries[1].hash =
            compute_boundary_hash(&missing_state_commitment.boundaries[1])
                .expect("the dependent boundary should rehash");
        refresh_snapshot_commitments_and_checkpoint(&mut missing_state_commitment);
        let Err(error) = Simulation::from_snapshot_with_plugins(
            missing_state_commitment,
            &[&PrimaryRandomPlugin],
        ) else {
            panic!("declared format-4 runs require every boundary state commitment");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let replayed = Simulation::replay_with_boundaries(
            73,
            scenario,
            &[&PrimaryRandomPlugin],
            primary_only.command_log(),
            primary_only.boundaries(),
            primary_only.time(),
        )
        .expect("scoped draws and boundary hashes should replay exactly");
        assert_eq!(primary_only.snapshot(), replayed.snapshot());

        let mut corrupted_draw = primary_only.snapshot();
        corrupted_draw.random_draws[0].value = (corrupted_draw.random_draws[0].value + 1)
            % corrupted_draw.random_draws[0].upper_exclusive;
        let Err(error) =
            Simulation::from_snapshot_with_plugins(corrupted_draw, &[&PrimaryRandomPlugin])
        else {
            panic!("tampered random evidence must not load");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut corrupted_hash = primary_only.snapshot();
        corrupted_hash.boundaries[0].hash.replace_range(..1, "f");
        let Err(error) =
            Simulation::from_snapshot_with_plugins(corrupted_hash, &[&PrimaryRandomPlugin])
        else {
            panic!("tampered boundary hashes must not load");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    }

    #[test]
    fn run_and_plugin_manifests_bind_continuation_and_replay() {
        let (scenario, _) = demo_scenario();
        let scenario_manifest =
            ArtifactManifest::for_scenario("cm", "reference-scenario", "1", &scenario)
                .expect("scenario identity should hash");
        let run_configuration = RunConfiguration::read_only_observer();
        let run_configuration_manifest =
            ArtifactManifest::for_run_configuration("cm", "run-policy", "1", &run_configuration)
                .expect("run configuration should hash");
        let mut run_manifest = RunManifest::declared(scenario_manifest, run_configuration_manifest);
        let RunManifest::Declared {
            rules,
            content,
            localization_contracts,
            sources,
            ..
        } = &mut run_manifest
        else {
            unreachable!("the fixture creates a declared manifest");
        };
        rules.extend([
            ArtifactManifest::from_bytes("cm", "zeta-rules", "1", b"zeta")
                .expect("rule identity should hash"),
            ArtifactManifest::from_bytes("cm", "alpha-rules", "1", b"alpha")
                .expect("rule identity should hash"),
        ]);
        content.push(
            ArtifactManifest::from_bytes("cm", "historical-content", "1", b"content")
                .expect("content identity should hash"),
        );
        localization_contracts.push(
            ArtifactManifest::from_bytes("cm", "localization-contract", "1", b"keys-v1")
                .expect("localization identity should hash"),
        );
        sources.push(
            ArtifactManifest::from_bytes("cm", "source-ledger", "1", b"sources")
                .expect("source identity should hash"),
        );

        let mut simulation = Simulation::new_with_run_configuration(
            91,
            scenario.clone(),
            run_manifest,
            run_configuration.clone(),
        )
        .expect("declared run identity should be admitted");
        let RunManifest::Declared { rules, .. } = simulation.run_manifest() else {
            unreachable!("new runs retain a declared manifest");
        };
        assert_eq!(rules[0].name, "alpha-rules");
        assert_eq!(rules[1].name, "zeta-rules");
        assert!(is_canonical_hash(simulation.run_manifest_hash()));
        simulation
            .register_plugin(&PrimaryRandomPlugin)
            .expect("versioned plugin should register");
        simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
            .expect("manifest-bound boundary should settle");

        let exact_manifest = simulation.run_manifest().clone();
        let snapshot = simulation.snapshot();
        let restored =
            Simulation::from_snapshot_with_plugins(snapshot.clone(), &[&PrimaryRandomPlugin])
                .expect("the exact executable manifest should restore");
        assert_eq!(simulation.snapshot(), restored.snapshot());

        let Err(error) = Simulation::from_snapshot_with_plugins(
            snapshot.clone(),
            &[&ChangedPrimaryRandomPlugin],
        ) else {
            panic!("changed executable semantics must not rehydrate an exact descriptor");
        };
        assert_eq!(error.code, ErrorCode::PluginManifestMismatch);

        let mut changed_scenario = scenario.clone();
        changed_scenario.world.armies[0].strength += 1;
        let Err(error) = Simulation::new_with_run_configuration(
            91,
            changed_scenario,
            exact_manifest.clone(),
            run_configuration.clone(),
        ) else {
            panic!("a scenario must match its declared semantic identity");
        };
        assert_eq!(error.code, ErrorCode::InvalidRunManifest);

        let mut corrupted_manifest_hash = snapshot.clone();
        let replacement = if corrupted_manifest_hash.run_manifest_hash.starts_with('f') {
            "e"
        } else {
            "f"
        };
        corrupted_manifest_hash
            .run_manifest_hash
            .replace_range(..1, replacement);
        let Err(error) = Simulation::from_snapshot(corrupted_manifest_hash) else {
            panic!("a tampered run manifest hash must not load");
        };
        assert_eq!(error.code, ErrorCode::InvalidRunManifest);

        let replayed = Simulation::replay_with_run_configuration(
            91,
            scenario.clone(),
            exact_manifest.clone(),
            run_configuration.clone(),
            &[&PrimaryRandomPlugin],
            simulation.command_log(),
            simulation.command_attempts(),
            simulation.boundaries(),
            simulation.time(),
        )
        .expect("the exact run and plugin environment should replay");
        assert_eq!(simulation.snapshot(), replayed.snapshot());

        let mut changed_environment = exact_manifest;
        let RunManifest::Declared { content, .. } = &mut changed_environment else {
            unreachable!("the fixture retains a declared manifest");
        };
        content[0].semantic_hash =
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_owned();
        let Err(error) = Simulation::replay_with_run_configuration(
            91,
            scenario,
            changed_environment,
            run_configuration,
            &[&PrimaryRandomPlugin],
            simulation.command_log(),
            simulation.command_attempts(),
            simulation.boundaries(),
            simulation.time(),
        ) else {
            panic!("replay under changed content identity must fail");
        };
        assert_eq!(error.code, ErrorCode::ReplayMismatch);
    }

    #[test]
    fn plugin_reads_and_writes_are_limited_to_declared_owned_state() {
        let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
        simulation
            .register_plugin(&SecretPlugin)
            .expect("secret owner should register");
        simulation
            .register_plugin(&UndeclaredAccessPlugin)
            .expect("access fixture should register");
        simulation
            .submit(CommandEnvelope::new(
                Issuer::Actor(ids.commander),
                Command::Plugin {
                    plugin: "secret-owner".to_owned(),
                    command: "seed".to_owned(),
                    payload: Value::Null,
                },
            ))
            .expect("the owner should write its declared state");
        let before = simulation
            .snapshot_json()
            .expect("snapshot should serialize");

        for (command, expected) in [
            ("missing", ErrorCode::EntityNotFound),
            ("read", ErrorCode::UndeclaredStateRead),
            ("write", ErrorCode::UndeclaredStateWrite),
        ] {
            let error = simulation
                .submit(CommandEnvelope::new(
                    Issuer::Actor(ids.commander),
                    Command::Plugin {
                        plugin: "undeclared-access".to_owned(),
                        command: command.to_owned(),
                        payload: Value::Null,
                    },
                ))
                .expect_err("undeclared state access must fail");
            assert_eq!(error.code, expected);
            assert_eq!(
                before,
                simulation
                    .snapshot_json()
                    .expect("rejected access must leave no serialized change")
            );
        }
    }

    #[test]
    fn typed_component_keys_isolate_adversarial_plugin_and_state_names() {
        let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
        simulation
            .register_plugin(&CollisionPluginA)
            .expect("first collision fixture should register");
        simulation
            .register_plugin(&CollisionPluginB)
            .expect("second collision fixture should register");
        for (plugin, expected) in [("a", "first"), ("a/person:1/b", "second")] {
            simulation
                .submit(CommandEnvelope::new(
                    Issuer::Actor(ids.commander),
                    Command::Plugin {
                        plugin: plugin.to_owned(),
                        command: "write".to_owned(),
                        payload: Value::Null,
                    },
                ))
                .expect("adversarial key should remain isolated");
            assert!(
                simulation
                    .snapshot()
                    .plugin_components
                    .iter()
                    .any(|record| {
                        record.plugin == plugin
                            && record.value == Value::String(expected.to_owned())
                    })
            );
        }
        assert_eq!(simulation.snapshot().plugin_components.len(), 2);
    }

    #[test]
    fn plugin_event_order_does_not_depend_on_registration_order() {
        let (scenario, ids) = demo_scenario();
        let mut first = Simulation::new(35, scenario.clone()).expect("demo should load");
        first
            .register_plugin(&MarkerPlugin {
                name: "zeta",
                writes: Vec::new(),
            })
            .expect("zeta should register");
        first
            .register_plugin(&MarkerPlugin {
                name: "alpha",
                writes: Vec::new(),
            })
            .expect("alpha should register");

        let mut second = Simulation::new(35, scenario).expect("demo should load");
        second
            .register_plugin(&MarkerPlugin {
                name: "alpha",
                writes: Vec::new(),
            })
            .expect("alpha should register");
        second
            .register_plugin(&MarkerPlugin {
                name: "zeta",
                writes: Vec::new(),
            })
            .expect("zeta should register");

        first
            .submit(move_order(&ids))
            .expect("first order should validate");
        second
            .submit(move_order(&ids))
            .expect("second order should validate");
        assert_eq!(first.snapshot(), second.snapshot());
        let marker_plugins: Vec<_> = first
            .events()
            .iter()
            .filter_map(|event| match &event.kind {
                EventKind::Plugin { plugin, .. } => Some(plugin.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(marker_plugins, vec!["alpha", "zeta"]);
    }

    #[test]
    fn failed_command_application_rolls_back_every_serialized_change() {
        let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
        simulation
            .register_plugin(&FailingPlugin)
            .expect("plugin should register");
        let before = simulation
            .snapshot_json()
            .expect("snapshot should serialize");
        let error = simulation
            .submit(CommandEnvelope::new(
                Issuer::Actor(ids.commander),
                Command::Plugin {
                    plugin: "failing-test".to_owned(),
                    command: "mutate".to_owned(),
                    payload: serde_json::json!({ "scheduled": false }),
                },
            ))
            .expect_err("the injected failure should reject the command");
        assert_eq!(error.code, ErrorCode::InvalidDuration);
        assert_eq!(
            before,
            simulation
                .snapshot_json()
                .expect("failed command must leave no mutation, event, or consumed ID")
        );

        let panic_error = simulation
            .submit(CommandEnvelope::new(
                Issuer::Actor(ids.commander),
                Command::Plugin {
                    plugin: "failing-test".to_owned(),
                    command: "panic".to_owned(),
                    payload: Value::Null,
                },
            ))
            .expect_err("plugin panics must cross the boundary as structured errors");
        assert_eq!(panic_error.code, ErrorCode::PluginPanicked);
        assert_eq!(
            before,
            simulation
                .snapshot_json()
                .expect("a panicking plugin must leave no serialized change")
        );

        let (mut ceiling_scenario, ceiling_ids) = demo_scenario();
        ceiling_scenario.start_time = SimTime::from_minutes(i64::MAX - 60);
        let mut ceiling = Simulation::new(35, ceiling_scenario)
            .expect("a scenario near the time ceiling should load");
        let ceiling_before = ceiling
            .snapshot_json()
            .expect("the ceiling fixture should serialize");
        let movement_error = ceiling
            .submit(move_order(&ceiling_ids))
            .expect_err("movement whose arrival overflows simulation time must fail");
        assert_eq!(movement_error.code, ErrorCode::InvalidDuration);
        let advance_error = ceiling
            .advance(SimDuration::hours(2))
            .expect_err("advancing beyond the time domain must fail");
        assert_eq!(advance_error.code, ErrorCode::InvalidDuration);
        assert_eq!(
            ceiling_before,
            ceiling
                .snapshot_json()
                .expect("time overflow must leave the simulation unchanged")
        );

        ceiling
            .register_plugin(&FailingPlugin)
            .expect("registration should remain open after rejected execution");
        let scheduled_before = ceiling
            .snapshot_json()
            .expect("the registered ceiling fixture should serialize");
        let schedule_error = ceiling
            .submit(CommandEnvelope::new(
                Issuer::Actor(ceiling_ids.commander),
                Command::Plugin {
                    plugin: "failing-test".to_owned(),
                    command: "mutate".to_owned(),
                    payload: serde_json::json!({ "scheduled": true }),
                },
            ))
            .expect_err("plugin work whose target overflows simulation time must fail");
        assert_eq!(schedule_error.code, ErrorCode::InvalidDuration);
        assert_eq!(
            scheduled_before,
            ceiling
                .snapshot_json()
                .expect("rejected plugin scheduling must not mutate the simulation")
        );
    }

    #[test]
    fn failed_scheduled_batch_restores_every_writable_domain() {
        let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
        simulation
            .register_plugin(&FailingPlugin)
            .expect("plugin should register");
        simulation
            .submit(move_order(&ids))
            .expect("the arrival should schedule");
        let arrival_at = SimTime::EPOCH
            .checked_add(SimDuration::hours(18))
            .expect("arrival time should be representable");
        simulation
            .schedule_at(
                arrival_at,
                ScheduledAction::PluginDirective {
                    plugin: "failing-test".to_owned(),
                    directive: Box::new(SystemDirective::SetComponent {
                        state: StateKey::new("failure-fixture", "flag"),
                        entity: EntityRef::Army(ids.army),
                        component: "flag".to_owned(),
                        value: Value::Bool(true),
                        summary: "Set a flag after the arrival mutates state".to_owned(),
                    }),
                    allowed_writes: vec![StateKey::new("failure-fixture", "flag")],
                    cause: CauseRef::System("scheduled-rollback-fixture".to_owned()),
                    correlation_id: 0,
                },
            )
            .expect("the failing action should share the arrival timestamp");
        let cache_before = cache_fingerprint(&simulation);
        let before_boundary = simulation
            .snapshot_json()
            .expect("snapshot should serialize");
        let error = simulation
            .advance(SimDuration::hours(18))
            .expect_err("the scheduled batch should fail after the arrival");
        assert_eq!(error.code, ErrorCode::InvalidDuration);
        assert_eq!(
            before_boundary,
            simulation
                .snapshot_json()
                .expect("failed boundary must restore its clock, queue, state, events, and IDs")
        );
        assert_eq!(cache_fingerprint(&simulation), cache_before);
    }

    #[test]
    fn failed_clock_only_advance_restores_time_and_commitments() {
        let (mut simulation, _) = Simulation::demo(35).expect("demo should load");
        simulation
            .state
            .metadata
            .commitment_cache
            .as_mut()
            .expect("current runtimes should maintain a commitment cache")
            .events
            .len = 2;
        let cache_before = cache_fingerprint(&simulation);
        let before = simulation
            .snapshot_json()
            .expect("snapshot should serialize");
        let error = simulation
            .advance(SimDuration::hours(1))
            .expect_err("the corrupt cache must abort clock-only advancement");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert_eq!(
            before,
            simulation
                .snapshot_json()
                .expect("failed clock advancement must restore serialized state")
        );
        assert_eq!(cache_fingerprint(&simulation), cache_before);
    }

    #[test]
    fn snapshot_continuation_requires_exact_plugin_rehydration() {
        let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
        let plugin = AuthorityPlugin;
        simulation
            .register_plugin(&plugin)
            .expect("plugin should register");
        simulation
            .submit(CommandEnvelope::new(
                Issuer::Actor(ids.commander),
                Command::Plugin {
                    plugin: "authority-test".to_owned(),
                    command: "set_stance".to_owned(),
                    payload: Value::Null,
                },
            ))
            .expect("plugin command should succeed");
        let json = simulation
            .snapshot_json()
            .expect("snapshot should serialize");

        let mut restored = Simulation::from_snapshot_json(&json).expect("snapshot should load");
        assert_eq!(
            restored
                .advance(SimDuration::ZERO)
                .expect_err("continuation without handlers must be blocked")
                .code,
            ErrorCode::PluginNotActive
        );
        let mismatch = MarkerPlugin {
            name: "authority-test",
            writes: Vec::new(),
        };
        assert_eq!(
            restored
                .register_plugin(&mismatch)
                .expect_err("a different executable manifest must be rejected")
                .code,
            ErrorCode::PluginManifestMismatch
        );
        restored
            .register_plugin(&plugin)
            .expect("the exact plugin manifest should rehydrate");
        restored
            .advance(SimDuration::ZERO)
            .expect("rehydrated snapshot should continue");
        assert_eq!(simulation.snapshot(), restored.snapshot());
    }

    #[test]
    fn command_only_replay_journal_binds_the_recorded_plugin_environment() {
        let (scenario, ids) = demo_scenario();
        let mut registration_closed_only =
            Simulation::new(35, scenario.clone()).expect("demo should load");
        registration_closed_only
            .advance(SimDuration::ZERO)
            .expect("zero advance should close authoritative registration");
        let closure_journal = registration_closed_only.replay_journal();
        let closure_replay =
            Simulation::replay_from_journal(scenario.clone(), &[], &closure_journal)
                .expect("exact replay should reproduce registration closure without other work");
        assert_eq!(
            registration_closed_only.snapshot(),
            closure_replay.snapshot()
        );

        let plugin = AuthorityPlugin;
        let mut simulation = Simulation::new(35, scenario.clone()).expect("demo should load");
        simulation
            .register_plugin(&plugin)
            .expect("plugin should register");
        simulation
            .submit(CommandEnvelope::new(
                Issuer::Actor(ids.commander),
                Command::Plugin {
                    plugin: "authority-test".to_owned(),
                    command: "set_stance".to_owned(),
                    payload: Value::Null,
                },
            ))
            .expect("plugin command should succeed");

        let replay_without_plugins = Simulation::replay(
            35,
            scenario.clone(),
            simulation.command_log(),
            simulation.time(),
        );
        let Err(error) = replay_without_plugins else {
            panic!("plugin replay without executable handlers must fail");
        };
        assert_eq!(error.code, ErrorCode::PluginCommandNotFound);
        let journal = simulation.replay_journal();
        let exact =
            Simulation::replay_from_journal(scenario.clone(), &[&AuthorityPlugin], &journal)
                .expect("the exact command-only environment should replay");
        assert_eq!(simulation.snapshot(), exact.snapshot());

        let Err(error) =
            Simulation::replay_from_journal(scenario.clone(), &[&ChangedAuthorityPlugin], &journal)
        else {
            panic!("changed handler semantics must fail before command-only replay");
        };
        assert_eq!(error.code, ErrorCode::ReplayEnvironmentMismatch);

        let replayed = Simulation::replay_with_plugins(
            35,
            scenario,
            &[&plugin],
            simulation.command_log(),
            simulation.time(),
        )
        .expect("plugin-aware replay should succeed");
        assert_eq!(simulation.snapshot(), replayed.snapshot());
    }

    #[test]
    fn canonical_ingress_orders_commands_packets_and_calendar_work() {
        let (scenario, ids) = demo_scenario();
        let plugin = CanonicalIngressPlugin;
        let mut simulation =
            Simulation::new(41, scenario.clone()).expect("ingress fixture should load");
        simulation
            .register_plugin(&plugin)
            .expect("canonical ingress plugin should register");
        let due_at = SimTime::EPOCH
            .checked_add(SimDuration::hours(1))
            .expect("fixture due time should be representable");

        let information = simulation
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                "canonical-ingress",
                "report",
                due_at,
                serde_json::json!({ "label": "field report" }),
            ))
            .expect("information should queue");
        let low_priority = simulation
            .enqueue_plugin_ingress(
                PluginIngressRequest::new(
                    "canonical-ingress",
                    "dispatch",
                    due_at,
                    serde_json::json!({ "label": "routine dispatch" }),
                )
                .with_priority(-10),
            )
            .expect("low-priority communication should queue");
        let acknowledgement = simulation
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                "canonical-ingress",
                "ack",
                due_at,
                serde_json::json!({ "label": "received" }),
            ))
            .expect("acknowledgement should queue");
        let high_priority = simulation
            .enqueue_plugin_ingress(
                PluginIngressRequest::new(
                    "canonical-ingress",
                    "dispatch",
                    due_at,
                    serde_json::json!({ "label": "urgent dispatch" }),
                )
                .with_priority(10),
            )
            .expect("high-priority communication should queue");
        let calendar = simulation
            .schedule_calendar_boundary(due_at, vec![SystemCadence::Daily])
            .expect("daily calendar work should queue");
        let command_request = CommandRequest::new(
            CommandRequestId::new(77),
            0,
            move_order(&ids).at_time(due_at),
        );
        let command = simulation
            .enqueue_command(due_at, 0, command_request.clone())
            .expect("command should queue");
        let future_at = due_at
            .checked_add(SimDuration::hours(1))
            .expect("future ingress time should be representable");
        let future = simulation
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                "canonical-ingress",
                "report",
                future_at,
                serde_json::json!({ "label": "future report" }),
            ))
            .expect("future information should queue without becoming visible early");
        assert_eq!(
            simulation
                .enqueue_command(due_at, 0, command_request.clone())
                .expect("an exact queued retry should be idempotent"),
            command
        );
        assert_eq!(simulation.ingress_log().len(), 7);
        let collision = simulation
            .enqueue_command(due_at, 1, command_request)
            .expect_err("a queued request-ID collision must fail closed");
        assert_eq!(collision.code, ErrorCode::IdempotencyConflict);
        let mixed_before = simulation.snapshot();
        let mixed = simulation
            .submit(move_order(&ids))
            .expect_err("legacy commands cannot bypass queued tracked ingress");
        assert_eq!(mixed.code, ErrorCode::MixedCommandIngress);
        assert_eq!(simulation.snapshot(), mixed_before);

        let before_legacy_advance = simulation.snapshot();
        let error = simulation
            .advance(SimDuration::hours(1))
            .expect_err("legacy advancement cannot skip canonical ingress");
        assert_eq!(error.code, ErrorCode::InvalidBoundary);
        assert_eq!(simulation.snapshot(), before_legacy_advance);

        let before_skipped_boundary = simulation.snapshot();
        let error = simulation
            .settle_boundary(BoundaryRequest::at(future_at))
            .expect_err("manual settlement cannot skip an earlier ingress due time");
        assert_eq!(error.code, ErrorCode::InvalidBoundary);
        assert_eq!(simulation.snapshot(), before_skipped_boundary);

        let pending_json = simulation
            .snapshot_json()
            .expect("pending ingress should serialize");
        let mut restored = Simulation::from_snapshot_json_with_plugins(&pending_json, &[&plugin])
            .expect("pending ingress should restore with its plugin contract");
        assert_eq!(simulation.snapshot(), restored.snapshot());

        let receipts = simulation
            .advance_canonical(SimDuration::hours(1))
            .expect("canonical advancement should settle every due input");
        let restored_receipts = restored
            .advance_canonical(SimDuration::hours(1))
            .expect("restored canonical ingress should settle identically");
        assert_eq!(receipts, restored_receipts);
        assert_eq!(simulation.snapshot(), restored.snapshot());
        assert_eq!(receipts.len(), 1);

        let boundary = simulation
            .boundaries()
            .last()
            .expect("canonical advancement should publish a boundary");
        let mut altered_boundary = boundary.clone();
        altered_boundary.admitted_ingress.pop();
        assert_ne!(
            compute_boundary_hash(boundary).expect("boundary evidence should hash"),
            compute_boundary_hash(&altered_boundary)
                .expect("altered boundary evidence should hash"),
            "canonical admission evidence must be committed by the boundary chain",
        );
        assert_eq!(boundary.cadences, vec![SystemCadence::Daily]);
        assert_eq!(
            boundary.admitted_ingress,
            vec![
                command.ingress_id,
                high_priority.ingress_id,
                low_priority.ingress_id,
                acknowledgement.ingress_id,
                information.ingress_id,
                calendar.ingress_id,
            ]
        );
        assert_eq!(boundary.admitted_attempts, vec![CommandAttemptId::new(1)]);
        assert_eq!(boundary.admitted_commands, vec![CommandId::new(1)]);
        assert!(!boundary.admitted_ingress.contains(&future.ingress_id));
        let snapshot = simulation.snapshot();
        assert!(snapshot.plugin_components.iter().any(|component| {
            component.state == StateKey::new("ingress-fixture", "received")
                && component.value
                    == serde_json::json!([
                        "communication:dispatch:10",
                        "communication:dispatch:-10",
                        "acknowledgement:ack:0",
                        "information:report:0"
                    ])
        }));
        assert!(snapshot.plugin_components.iter().any(|component| {
            component.state == StateKey::new("ingress-fixture", "calendar")
                && component.value == Value::Bool(true)
        }));

        let post_boundary_due = future_at
            .checked_add(SimDuration::hours(1))
            .expect("post-boundary ingress time should be representable");
        simulation
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                "canonical-ingress",
                "report",
                post_boundary_due,
                serde_json::json!({ "label": "post-boundary report" }),
            ))
            .expect("future ingress may be authored after a completed boundary");
        let post_boundary_snapshot = simulation.snapshot();
        let post_boundary_restored =
            Simulation::from_snapshot_with_plugins(post_boundary_snapshot.clone(), &[&plugin])
                .expect("post-boundary pending ingress must not invalidate its own snapshot");
        assert_eq!(post_boundary_restored.snapshot(), post_boundary_snapshot);

        let late_before = simulation.snapshot();
        let late = simulation
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                "canonical-ingress",
                "report",
                SimTime::EPOCH,
                serde_json::json!({ "label": "late report" }),
            ))
            .expect_err("late ingress cannot rewrite an already committed boundary");
        assert_eq!(late.code, ErrorCode::LateIngress);
        assert_eq!(simulation.snapshot(), late_before);

        let journal = simulation.replay_journal();
        let replayed = Simulation::replay_from_journal(scenario, &[&plugin], &journal)
            .expect("canonical ingress should replay in its recorded environment");
        assert_eq!(simulation.snapshot(), replayed.snapshot());

        let mut reordered = simulation.snapshot();
        reordered.boundaries[0].admitted_ingress.swap(0, 1);
        rehash_tampered_snapshot(&mut reordered);
        let error = Simulation::from_snapshot_with_plugins(reordered, &[&plugin])
            .err()
            .expect("a rehashed noncanonical ingress order must not load");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut predating_issue_cut = simulation.snapshot();
        let last = predating_issue_cut
            .ingress
            .last_mut()
            .expect("the fixture retains post-boundary ingress");
        last.issued_at = SimTime::EPOCH;
        rehash_tampered_snapshot(&mut predating_issue_cut);
        let error = Simulation::from_snapshot_with_plugins(predating_issue_cut, &[&plugin])
            .err()
            .expect("ingress cannot predate its declared boundary issue cut");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut skipped_due_time = simulation.snapshot();
        let information_record = skipped_due_time
            .ingress
            .iter_mut()
            .find(|record| record.id == information.ingress_id)
            .expect("the information ingress should remain in the journal");
        information_record.due_at = SimTime::EPOCH;
        rehash_tampered_snapshot(&mut skipped_due_time);
        let error = Simulation::from_snapshot_with_plugins(skipped_due_time, &[&plugin])
            .err()
            .expect("a boundary cannot be forged past an earlier due ingress time");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    }

    #[test]
    fn command_ingress_precedes_equal_time_internal_scheduled_work() {
        let (scenario, ids) = demo_scenario();
        let mut simulation = Simulation::new(47, scenario).expect("ordering fixture should load");
        simulation
            .enqueue_command(
                SimTime::EPOCH,
                0,
                CommandRequest::new(CommandRequestId::new(1), 0, move_order(&ids)),
            )
            .expect("the initial movement should queue");
        simulation
            .step_canonical()
            .expect("the movement boundary should settle")
            .expect("the queued movement supplies due work");
        let arrival_at = SimTime::EPOCH
            .checked_add(SimDuration::hours(18))
            .expect("arrival time should be representable");
        let return_order = CommandEnvelope::new(
            Issuer::Actor(ids.commander),
            Command::MoveArmy {
                army: ids.army,
                destination: ids.western_territory,
            },
        )
        .at_time(arrival_at);
        simulation
            .enqueue_command(
                arrival_at,
                0,
                CommandRequest::new(
                    CommandRequestId::new(2),
                    simulation.revision(),
                    return_order,
                ),
            )
            .expect("the equal-time return order should queue");

        simulation
            .step_canonical()
            .expect("the equal-time boundary should settle")
            .expect("the arrival and command are both due");
        let attempt = simulation
            .command_attempts()
            .last()
            .expect("the queued command should leave attempt evidence");
        assert!(matches!(
            &attempt.outcome,
            CommandAttemptOutcome::Rejected { error }
                if error.code == ErrorCode::InvalidAuthority
                    && error.message.contains("already moving")
        ));
        assert_eq!(
            simulation
                .world()
                .army(ids.army)
                .expect("the army should remain present")
                .location,
            ids.eastern_territory,
            "the scheduled arrival executes after the command-class admission decision",
        );
    }

    #[test]
    fn exact_replay_cannot_advance_past_unadmitted_due_ingress() {
        let (scenario, _) = demo_scenario();
        let mut simulation =
            Simulation::new(49, scenario.clone()).expect("replay fixture should load");
        let due_at = SimTime::EPOCH
            .checked_add(SimDuration::hours(1))
            .expect("due time should be representable");
        simulation
            .schedule_calendar_boundary(due_at, vec![SystemCadence::Daily])
            .expect("calendar ingress should queue");
        let forged_final = due_at
            .checked_add(SimDuration::hours(1))
            .expect("forged final time should be representable");
        let mut forged_snapshot = simulation.snapshot();
        forged_snapshot.now = forged_final;
        refresh_snapshot_commitments_and_checkpoint(&mut forged_snapshot);
        let mut journal = simulation.replay_journal();
        journal.final_time = forged_final;
        journal.checkpoint_hash = forged_snapshot.checkpoint_hash;

        let error = Simulation::replay_from_journal(scenario, &[], &journal)
            .err()
            .expect("replay must not cross unadmitted due ingress");
        assert_eq!(error.code, ErrorCode::InvalidBoundary);
    }

    #[test]
    fn snapshot_ingress_reconstructs_ordered_command_and_calendar_effects() {
        let (scenario, ids) = demo_scenario();
        let mut commands =
            Simulation::new(51, scenario.clone()).expect("command fixture should load");
        for (request_id, revision, priority, morale) in [(1, 0, 10, 80), (2, 1, 0, 90)] {
            let envelope = CommandEnvelope::new(
                Issuer::Debug,
                Command::DebugSetArmyMorale {
                    army: ids.army,
                    morale,
                },
            )
            .at_time(SimTime::EPOCH);
            commands
                .enqueue_command(
                    SimTime::EPOCH,
                    priority,
                    CommandRequest::new(CommandRequestId::new(request_id), revision, envelope),
                )
                .expect("ordered command should queue");
        }
        commands
            .step_canonical()
            .expect("command boundary should settle")
            .expect("commands supply due work");
        commands
            .schedule_calendar_boundary(
                SimTime::EPOCH
                    .checked_add(SimDuration::hours(1))
                    .expect("future time should be representable"),
                vec![SystemCadence::Daily],
            )
            .expect("future ingress keeps the snapshot beyond its boundary head");
        let mut reordered_commands = commands.snapshot();
        reordered_commands.ingress[0].priority = 0;
        reordered_commands.ingress[1].priority = 10;
        reordered_commands.boundaries[0].admitted_ingress.swap(0, 1);
        rehash_tampered_snapshot(&mut reordered_commands);
        let error = Simulation::from_snapshot(reordered_commands)
            .err()
            .expect("queue order cannot be detached from command-attempt order");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut relabeled_attempt = commands.snapshot();
        relabeled_attempt.command_attempts[0].ingress = CommandIngress::FrozenReplay;
        rehash_tampered_snapshot(&mut relabeled_attempt);
        let error = Simulation::from_snapshot(relabeled_attempt)
            .err()
            .expect("queued command attempts must retain live-request provenance");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut calendar = Simulation::new(53, scenario).expect("calendar fixture should load");
        calendar
            .schedule_calendar_boundary(SimTime::EPOCH, vec![SystemCadence::Daily])
            .expect("calendar work should queue");
        calendar
            .step_canonical()
            .expect("calendar boundary should settle")
            .expect("calendar work supplies a boundary");
        calendar
            .schedule_calendar_boundary(
                SimTime::EPOCH
                    .checked_add(SimDuration::hours(1))
                    .expect("future time should be representable"),
                vec![SystemCadence::Daily],
            )
            .expect("future calendar work keeps the snapshot beyond its boundary head");
        let mut omitted_calendar = calendar.snapshot();
        omitted_calendar.boundaries[0].cadences.clear();
        rehash_tampered_snapshot(&mut omitted_calendar);
        let error = Simulation::from_snapshot(omitted_calendar)
            .err()
            .expect("admitted calendar work must appear in boundary cadence evidence");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    }

    #[test]
    fn generated_ingress_delay_must_be_representable() {
        let (mut scenario, _) = demo_scenario();
        let earliest = SimTime::from_minutes(i64::MIN);
        scenario.start_time = earliest;
        for actor in scenario.knowledge.actors.values_mut() {
            for army in actor.armies.values_mut() {
                army.observed_at = earliest;
                army.learned_at = earliest;
            }
        }
        let plugin = GeneratedIngressPlugin;
        let mut simulation =
            Simulation::new(55, scenario).expect("extreme-time ingress fixture should load");
        simulation
            .register_plugin(&plugin)
            .expect("generated ingress plugin should register");
        simulation
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                "generated-ingress",
                "dispatch",
                earliest,
                serde_json::json!({ "label": "dispatch" }),
            ))
            .expect("extreme-time dispatch should queue");
        simulation
            .step_canonical()
            .expect("extreme-time boundary should settle")
            .expect("dispatch supplies due work");
        let mut overflow = simulation.snapshot();
        overflow.ingress[1].due_at = SimTime::from_minutes(i64::MAX);
        rehash_tampered_snapshot(&mut overflow);
        let error = Simulation::from_snapshot_with_plugins(overflow, &[&plugin])
            .err()
            .expect("generated delay must fit the simulation duration domain");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    }

    #[test]
    fn boundary_generated_zero_delay_ingress_waits_for_the_next_same_time_boundary() {
        let (scenario, _) = demo_scenario();
        let plugin = GeneratedIngressPlugin;
        let mut simulation =
            Simulation::new(43, scenario.clone()).expect("generated ingress fixture should load");
        simulation
            .register_plugin(&plugin)
            .expect("generated ingress plugin should register");
        let dispatch = simulation
            .enqueue_plugin_ingress(
                PluginIngressRequest::new(
                    "generated-ingress",
                    "dispatch",
                    SimTime::EPOCH,
                    serde_json::json!({ "label": "dispatch" }),
                )
                .with_entity(EntityRef::Person(PersonId::new(1))),
            )
            .expect("dispatch should queue");

        let first = simulation
            .step_canonical()
            .expect("the first canonical boundary should settle")
            .expect("the dispatch supplies due work");
        assert_eq!(first.settled_at, SimTime::EPOCH);
        assert_eq!(first.generated_ingress, vec![IngressId::new(2)]);
        assert_eq!(simulation.boundaries().len(), 1);
        assert_eq!(
            simulation.boundaries()[0].admitted_ingress,
            vec![dispatch.ingress_id]
        );
        assert_eq!(
            simulation.boundaries()[0]
                .generated_ingress
                .iter()
                .map(|generation| generation.ingress)
                .collect::<Vec<_>>(),
            vec![IngressId::new(2)],
        );
        let generation = &simulation.boundaries()[0].generated_ingress[0];
        assert_eq!(generation.plugin, "generated-ingress");
        assert_eq!(generation.system, "relay-ingress");
        assert_eq!(generation.phase, BoundaryPhase::DomainDeltaProposal);
        assert_eq!(generation.visibility, StateVisibility::SameBoundary);
        let mut altered_boundary = simulation.boundaries()[0].clone();
        altered_boundary.generated_ingress.clear();
        assert_ne!(
            compute_boundary_hash(&simulation.boundaries()[0])
                .expect("generated ingress evidence should hash"),
            compute_boundary_hash(&altered_boundary)
                .expect("altered generation evidence should hash"),
            "generated ingress evidence must be committed by the boundary chain",
        );
        assert!(
            !simulation
                .snapshot()
                .plugin_components
                .iter()
                .any(|component| {
                    component.state == StateKey::new("generated-ingress-fixture", "received")
                })
        );

        let pending = simulation.snapshot();
        let mut restored = Simulation::from_snapshot_with_plugins(pending.clone(), &[&plugin])
            .expect("a pending generated acknowledgement should restore");
        assert_eq!(restored.snapshot(), pending);

        let second = simulation
            .step_canonical()
            .expect("the generated acknowledgement boundary should settle")
            .expect("the acknowledgement remains due at the same simulation time");
        let restored_second = restored
            .step_canonical()
            .expect("the restored acknowledgement boundary should settle")
            .expect("the restored acknowledgement remains due");
        assert_eq!(second, restored_second);
        assert_eq!(second.settled_at, SimTime::EPOCH);
        assert!(second.generated_ingress.is_empty());
        assert_eq!(simulation.boundaries().len(), 2);
        assert_eq!(
            simulation.boundaries()[1].admitted_ingress,
            vec![IngressId::new(2)]
        );
        assert!(
            simulation
                .snapshot()
                .plugin_components
                .iter()
                .any(|component| {
                    component.state == StateKey::new("generated-ingress-fixture", "received")
                        && component.value == Value::Bool(true)
                })
        );
        assert_eq!(simulation.snapshot(), restored.snapshot());

        let journal = simulation.replay_journal();
        let replayed = Simulation::replay_from_journal(scenario, &[&plugin], &journal)
            .expect("boundary-generated ingress should replay from its producing system");
        assert_eq!(simulation.snapshot(), replayed.snapshot());

        let mut missing_generation_evidence = simulation.snapshot();
        missing_generation_evidence.boundaries[0]
            .generated_ingress
            .clear();
        rehash_tampered_snapshot(&mut missing_generation_evidence);
        let error = Simulation::from_snapshot_with_plugins(missing_generation_evidence, &[&plugin])
            .err()
            .expect("boundary-caused ingress without producer evidence must not load");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);

        let mut false_producer = simulation.snapshot();
        false_producer.boundaries[0].generated_ingress[0].phase =
            BoundaryPhase::StrategicAggregation;
        rehash_tampered_snapshot(&mut false_producer);
        let error = Simulation::from_snapshot_with_plugins(false_producer, &[&plugin])
            .err()
            .expect("generated ingress must retain exact producer-stage provenance");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    }

    #[test]
    fn read_only_runs_reject_live_plugin_ingress_without_mutation() {
        let (scenario, _) = demo_scenario();
        let configuration = RunConfiguration::read_only_observer();
        let manifest = manifest_for_configuration(&scenario, &configuration);
        let plugin = CanonicalIngressPlugin;
        let mut simulation =
            Simulation::new_with_run_configuration(47, scenario, manifest, configuration)
                .expect("read-only ingress fixture should load");
        simulation
            .register_plugin(&plugin)
            .expect("read-only ingress plugin should register");
        let before = simulation.snapshot();
        let error = simulation
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                "canonical-ingress",
                "report",
                SimTime::EPOCH,
                serde_json::json!({ "label": "unauthorized live report" }),
            ))
            .expect_err("read-only runs cannot accept newly authored plugin ingress");
        assert_eq!(error.code, ErrorCode::InteractionReadOnly);
        assert_eq!(simulation.snapshot(), before);

        simulation
            .append_ingress(
                SimTime::EPOCH,
                IngressClass::Information,
                0,
                IngressPayload::Plugin {
                    plugin: "canonical-ingress".to_owned(),
                    packet_type: "report".to_owned(),
                    payload: serde_json::json!({ "label": "forged live report" }),
                    affected_entities: Vec::new(),
                },
                None,
                false,
            )
            .expect("the fixture should construct coherent but unauthorized evidence");
        let error = Simulation::from_snapshot_with_plugins(simulation.snapshot(), &[&plugin])
            .err()
            .expect("snapshot validation must reject impossible read-only live ingress");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    }
}
