//! Deterministic runtime, validated commands, scheduling, plugins, and snapshots.

#![allow(
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::too_many_lines
)]

mod boundary;
mod manifest;
mod policy;
mod random;
mod records;

pub use boundary::{
    BoundaryChange, BoundaryContext, BoundaryDirective, BoundaryEmission, BoundaryEmissionKind,
    BoundaryProposal, BoundaryReceipt, BoundaryRecord, BoundaryRequest, BoundarySystemContract,
    BoundarySystemHandler, ReservationAllocation, ReservationDisposition, ReservationOffer,
    ReservationOfferRecord, ReservationPoolKey, ReservationRef, ReservationRequest,
    ReservationRequestRecord,
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
    DomainRecordKind, DomainRecordRef, EntityRef, EventId, FieldSchema, GovernmentId, PersonId,
    RandomDrawId, RouteId, SchemaRegistry, TerritoryId, TypeSchema,
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
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::panic::{AssertUnwindSafe, catch_unwind};

pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SNAPSHOT_FORMAT_VERSION: u32 = 4;
const CORE_STATE_NAMESPACE: &str = "canwu.core";
const GENESIS_BOUNDARY_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

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
    pub revision: u64,
    pub accepted_at: SimTime,
    pub emitted_events: Vec<EventId>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommandRejection {
    pub attempt_id: Option<CommandAttemptId>,
    pub request_id: Option<CommandRequestId>,
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
        self.state.now
    }

    pub fn army(&self, id: ArmyId) -> Result<Option<&Army>, CanwuError> {
        self.require_read(&StateKey::core_armies())?;
        Ok(self.state.armies.get(&id))
    }

    pub fn person(&self, id: PersonId) -> Result<Option<&Person>, CanwuError> {
        self.require_read(&StateKey::core_people())?;
        Ok(self.state.people.get(&id))
    }

    pub fn government(&self, id: GovernmentId) -> Result<Option<&Government>, CanwuError> {
        self.require_read(&StateKey::core_governments())?;
        Ok(self.state.governments.get(&id))
    }

    pub fn territory(&self, id: TerritoryId) -> Result<Option<&Territory>, CanwuError> {
        self.require_read(&StateKey::core_territories())?;
        Ok(self.state.territories.get(&id))
    }

    pub fn route(&self, id: RouteId) -> Result<Option<&Route>, CanwuError> {
        self.require_read(&StateKey::core_routes())?;
        Ok(self.state.routes.get(&id))
    }

    pub fn actor_knowledge(&self, actor: PersonId) -> Result<Option<&ActorKnowledge>, CanwuError> {
        self.require_read(&StateKey::core_knowledge())?;
        Ok(self.state.knowledge.for_actor(actor))
    }

    pub fn command(&self, id: CommandId) -> Result<Option<&CommandRecord>, CanwuError> {
        self.require_read(&StateKey::core_commands())?;
        Ok(self.state.commands.iter().find(|record| record.id == id))
    }

    pub fn event(&self, id: EventId) -> Result<Option<&SimEvent>, CanwuError> {
        self.require_read(&StateKey::core_events())?;
        Ok(self.state.events.iter().find(|event| event.id == id))
    }

    pub fn domain_record(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<&DomainRecord>, CanwuError> {
        self.require_read(&records::record_state_key(&reference.kind))?;
        Ok(self
            .record_overlay
            .and_then(|overlay| overlay.get(reference))
            .or_else(|| self.state.domain_records.get(reference)))
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
            .or_else(|| self.state.plugin_components.get(&key))
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

#[derive(Clone)]
struct RuntimeState {
    initial_time: SimTime,
    initial_scenario: Option<Scenario>,
    now: SimTime,
    run_manifest: RunManifest,
    run_manifest_hash: String,
    run_configuration: RunConfigurationSnapshot,
    checkpoint_hash: String,
    plugin_registration_closed: bool,
    people: BTreeMap<PersonId, Person>,
    governments: BTreeMap<GovernmentId, Government>,
    territories: BTreeMap<TerritoryId, Territory>,
    routes: BTreeMap<RouteId, Route>,
    armies: BTreeMap<ArmyId, Army>,
    knowledge: KnowledgeSnapshot,
    scheduler: BTreeMap<ScheduleKey, ScheduledAction>,
    events: Vec<SimEvent>,
    commands: Vec<CommandRecord>,
    command_attempts: Vec<CommandAttemptRecord>,
    boundaries: Vec<BoundaryRecord>,
    plugin_components: BTreeMap<PluginComponentKey, PluginComponentRecord>,
    domain_records: BTreeMap<DomainRecordRef, DomainRecord>,
    root_seed: u64,
    random_streams: BTreeMap<RandomStreamKey, RandomStreamState>,
    random_draws: Vec<RandomDrawRecord>,
    next_event_id: u64,
    next_command_id: u64,
    next_command_attempt_id: u64,
    next_boundary_id: u64,
    next_random_draw_id: u64,
    next_schedule_sequence: u64,
    next_correlation_id: u64,
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
    pub boundaries: Vec<BoundaryRecord>,
    pub final_time: SimTime,
    pub checkpoint_hash: String,
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
    boundaries: Vec<BoundaryRecord>,
    final_time: SimTime,
    checkpoint_hash: String,
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
            boundaries: wire.boundaries,
            final_time: wire.final_time,
            checkpoint_hash: wire.checkpoint_hash,
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
                initial_time: scenario.start_time,
                initial_scenario,
                now: scenario.start_time,
                run_manifest,
                run_manifest_hash,
                run_configuration,
                checkpoint_hash: String::new(),
                plugin_registration_closed: false,
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
                scheduler: BTreeMap::new(),
                events: Vec::new(),
                commands: Vec::new(),
                command_attempts: Vec::new(),
                boundaries: Vec::new(),
                plugin_components: BTreeMap::new(),
                domain_records: scenario
                    .domain_records
                    .into_iter()
                    .map(|record| (record.reference.clone(), record))
                    .collect(),
                root_seed: seed,
                random_streams: BTreeMap::from([(core_stream.key.clone(), core_stream)]),
                random_draws: Vec::new(),
                next_event_id: 1,
                next_command_id: 1,
                next_command_attempt_id: 1,
                next_boundary_id: 1,
                next_random_draw_id: 1,
                next_schedule_sequence: 1,
                next_correlation_id: 1,
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
        Self::replay_records(simulation, commands, &[], boundaries, final_time)
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
        if matches!(journal.run_manifest, RunManifest::MigratedLegacy { .. }) {
            return Err(CanwuError::new(
                ErrorCode::LegacyReplayUnavailable,
                "legacy checkpoints can continue after migration but lack enough recorded identity for exact replay",
            ));
        }
        manifest::validate(&journal.run_manifest, Some(&scenario), false)?;
        manifest::validate_run_configuration(&journal.run_manifest, &journal.run_configuration)?;
        if journal.engine_version != ENGINE_VERSION
            || journal.snapshot_format_version != SNAPSHOT_FORMAT_VERSION
            || !is_canonical_hash(&journal.run_manifest_hash)
            || manifest::hash(&journal.run_manifest)? != journal.run_manifest_hash
            || !is_canonical_hash(&journal.checkpoint_hash)
        {
            return Err(CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                "replay journal engine, format, or run identity does not match this runtime",
            ));
        }
        PluginRegistry::from_descriptors(journal.plugin_descriptors.clone()).map_err(|error| {
            CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                format!("replay journal plugin manifest is invalid: {error}"),
            )
        })?;

        let simulation = Self::new_with_configuration_snapshot(
            journal.root_seed,
            scenario,
            journal.run_manifest.clone(),
            journal.run_configuration.clone(),
        )?;
        let simulation = Self::activate_initial_plugins(simulation, plugins)?;
        let actual_descriptors: Vec<_> = simulation.plugin_descriptors().cloned().collect();
        if actual_descriptors != journal.plugin_descriptors {
            return Err(CanwuError::new(
                ErrorCode::ReplayEnvironmentMismatch,
                "active plugin identities and contracts do not match the replay journal",
            ));
        }

        let mut simulation = Self::replay_records(
            simulation,
            &journal.commands,
            &journal.command_attempts,
            &journal.boundaries,
            journal.final_time,
        )?;
        if journal.plugin_registration_closed && !simulation.state.plugin_registration_closed {
            simulation.advance(SimDuration::ZERO)?;
        }
        if simulation.state.plugin_registration_closed != journal.plugin_registration_closed {
            return Err(CanwuError::new(
                ErrorCode::ReplayMismatch,
                "replayed plugin-registration lifecycle does not match the recorded journal",
            ));
        }
        if simulation.checkpoint_hash() != journal.checkpoint_hash {
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
        boundaries: &[BoundaryRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        simulation.ensure_runtime_ready()?;
        if !attempts.is_empty() {
            return Self::replay_attempt_records(
                simulation, commands, attempts, boundaries, final_time,
            );
        }
        let mut next_command = 0;
        for expected_boundary in boundaries {
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
        if final_time < simulation.time() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "replay final time cannot precede the last command",
            ));
        }
        if final_time > simulation.time() {
            simulation.advance_to(final_time)?;
        }
        Ok(simulation)
    }

    fn replay_attempt_records(
        mut simulation: Self,
        commands: &[CommandRecord],
        attempts: &[CommandAttemptRecord],
        boundaries: &[BoundaryRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        let mut next_attempt = 0;
        for expected_boundary in boundaries {
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
                replay_attempt_record(&mut simulation, record, commands, expected_boundary.at)?;
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
        if self.state.plugin_registration_closed && !rehydrating {
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
            if !self.plugins.record_schemas.is_empty() && self.state.initial_scenario.is_none() {
                return Err(CanwuError::new(
                    ErrorCode::UnsupportedSnapshotVersion,
                    "this snapshot predates manifest-bound domain-record genesis and cannot activate record schemas",
                ));
            }
            records::validate_records_for_owner(
                &self.state.domain_records,
                &self.plugins.record_schemas,
                plugin_name,
                self.state.now,
                &|entity| runtime_entity_exists(&self.state, entity),
            )?;
            for stream in self.plugins.random_stream_owners.keys() {
                self.state
                    .random_streams
                    .entry(stream.clone())
                    .or_insert_with(|| {
                        RandomStreamState::initial(self.state.root_seed, stream.clone())
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
            &self.state.domain_records,
            &self.plugins.record_schemas,
            self.state.now,
            &|entity| runtime_entity_exists(&self.state, entity),
        )
    }

    fn domain_record_feature_enabled(&self) -> bool {
        !self.plugins.record_schemas.is_empty()
            || !self.state.domain_records.is_empty()
            || self
                .state
                .boundaries
                .iter()
                .any(|boundary| !boundary.record_changes.is_empty())
    }

    fn bound_initial_scenario(&self) -> Option<&Scenario> {
        if self.domain_record_feature_enabled() {
            self.state.initial_scenario.as_ref()
        } else {
            None
        }
    }

    #[must_use]
    pub const fn time(&self) -> SimTime {
        self.state.now
    }

    #[must_use]
    pub const fn run_manifest(&self) -> &RunManifest {
        &self.state.run_manifest
    }

    #[must_use]
    pub const fn run_configuration(&self) -> &RunConfigurationSnapshot {
        &self.state.run_configuration
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.state.next_correlation_id.saturating_sub(1)
    }

    #[must_use]
    pub fn run_manifest_hash(&self) -> &str {
        &self.state.run_manifest_hash
    }

    #[must_use]
    pub fn checkpoint_hash(&self) -> &str {
        &self.state.checkpoint_hash
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
            people: self.state.people.values().cloned().collect(),
            governments: self.state.governments.values().cloned().collect(),
            territories: self.state.territories.values().cloned().collect(),
            routes: self.state.routes.values().cloned().collect(),
            armies: self.state.armies.values().cloned().collect(),
        }
    }

    #[must_use]
    pub fn knowledge(&self) -> &KnowledgeSnapshot {
        &self.state.knowledge
    }

    #[must_use]
    pub fn events(&self) -> &[SimEvent] {
        &self.state.events
    }

    #[must_use]
    pub fn command_log(&self) -> &[CommandRecord] {
        &self.state.commands
    }

    #[must_use]
    pub fn command_attempts(&self) -> &[CommandAttemptRecord] {
        &self.state.command_attempts
    }

    #[must_use]
    pub fn domain_record(&self, reference: &DomainRecordRef) -> Option<&DomainRecord> {
        self.state.domain_records.get(reference)
    }

    pub fn domain_records(&self) -> impl Iterator<Item = &DomainRecord> {
        self.state.domain_records.values()
    }

    #[must_use]
    pub fn boundaries(&self) -> &[BoundaryRecord] {
        &self.state.boundaries
    }

    #[must_use]
    pub fn random_draws(&self) -> &[RandomDrawRecord] {
        &self.state.random_draws
    }

    #[must_use]
    pub fn boundary_head_hash(&self) -> Option<&str> {
        self.state
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
            root_seed: self.state.root_seed,
            run_manifest: self.state.run_manifest.clone(),
            run_manifest_hash: self.state.run_manifest_hash.clone(),
            run_configuration: self.state.run_configuration.clone(),
            plugin_descriptors: self.plugins.descriptors().cloned().collect(),
            plugin_registration_closed: self.state.plugin_registration_closed,
            commands: self.state.commands.clone(),
            command_attempts: self.state.command_attempts.clone(),
            boundaries: self.state.boundaries.clone(),
            final_time: self.state.now,
            checkpoint_hash: self.state.checkpoint_hash.clone(),
        }
    }

    pub fn submit(&mut self, envelope: CommandEnvelope) -> Result<CommandReceipt, CanwuError> {
        match self.admit_command(None, None, envelope, CommandIngress::LegacyDirect, false)? {
            CommandOutcome::Accepted { receipt } => Ok(receipt),
            CommandOutcome::Rejected { rejection } => Err(rejection.error),
        }
    }

    pub fn process_command(
        &mut self,
        request: CommandRequest,
    ) -> Result<CommandOutcome, CanwuError> {
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
            let (value, _) =
                claim_counter(self.state.next_command_attempt_id, "command attempt ID")?;
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
            && expected_time != self.state.now
        {
            let error = CanwuError::new(
                ErrorCode::SimulationTimeConflict,
                format!(
                    "command expected time {expected_time}, but simulation is at {}",
                    self.state.now
                ),
            );
            if record_attempt {
                return self.record_command_rejection(attempt_id, admission, envelope, error);
            }
            return Err(error);
        }

        let (command_id_value, next_command_id) =
            claim_counter(self.state.next_command_id, "command ID")?;
        let (correlation_id, next_correlation_id) =
            claim_counter(self.state.next_correlation_id, "correlation ID")?;
        let command_id = CommandId::new(command_id_value);
        let context = CommandContext {
            issuer: envelope.issuer.clone(),
            authority,
            run_policy: self.state.run_configuration.command_policy(),
            ingress: admission.ingress,
            attempt_id: record_attempt.then_some(attempt_id),
            command_id,
            request_id: admission.request_id,
            revision: admission.revision_before,
            simulation_time: self.state.now,
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
        let transaction_start = self.state.clone();
        let event_start = self.state.events.len();
        self.state.next_command_id = next_command_id;
        self.state.next_correlation_id = next_correlation_id;

        if let Err(error) = self.apply_prepared(prepared, command_id, correlation_id) {
            self.state = transaction_start;
            if is_expected_command_rejection(&error.code) && record_attempt {
                return self.record_command_rejection(attempt_id, admission, envelope, error);
            }
            return Err(error);
        }
        let emitted_events: Vec<_> = self.state.events[event_start..]
            .iter()
            .map(|event| event.id)
            .collect();
        self.state.plugin_registration_closed = true;
        self.state.commands.push(CommandRecord {
            id: command_id,
            attempt_id: record_attempt.then_some(attempt_id),
            accepted_at: self.state.now,
            envelope: envelope.clone(),
            emitted_events: if record_attempt {
                emitted_events.clone()
            } else {
                Vec::new()
            },
        });
        if record_attempt {
            let (_, next_attempt_id) =
                claim_counter(self.state.next_command_attempt_id, "command attempt ID")?;
            self.state.next_command_attempt_id = next_attempt_id;
            self.state.command_attempts.push(CommandAttemptRecord {
                id: attempt_id,
                at: self.state.now,
                revision_before: admission.revision_before,
                ingress: admission.ingress,
                request_id: admission.request_id,
                expected_revision: admission.expected_revision,
                envelope,
                outcome: CommandAttemptOutcome::Accepted { command_id },
            });
        }
        if let Err(error) = self.refresh_checkpoint_hash() {
            self.state = transaction_start;
            return Err(error);
        }

        Ok(CommandOutcome::Accepted {
            receipt: CommandReceipt {
                attempt_id: record_attempt.then_some(attempt_id),
                command_id,
                request_id: admission.request_id,
                revision: admission.revision_before + 1,
                accepted_at: self.state.now,
                emitted_events,
            },
        })
    }

    fn ensure_command_ingress_family(&self, ingress: CommandIngress) -> Result<(), CanwuError> {
        let has_legacy_commands = self
            .state
            .commands
            .iter()
            .any(|record| record.attempt_id.is_none());
        let has_tracked_attempts = !self.state.command_attempts.is_empty();
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
                    rejected_at: self.state.now,
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
                    retained_revision: attempt.revision_before,
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
        let transaction_start = self.state.clone();
        let (claimed_id, next_attempt_id) =
            claim_counter(self.state.next_command_attempt_id, "command attempt ID")?;
        if claimed_id != attempt_id.get() {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "command attempt allocation changed during rejection",
            ));
        }
        self.state.next_command_attempt_id = next_attempt_id;
        self.state.plugin_registration_closed = true;
        self.state.command_attempts.push(CommandAttemptRecord {
            id: attempt_id,
            at: self.state.now,
            revision_before: admission.revision_before,
            ingress: admission.ingress,
            request_id: admission.request_id,
            expected_revision: admission.expected_revision,
            envelope,
            outcome: CommandAttemptOutcome::Rejected {
                error: error.clone(),
            },
        });
        if let Err(hash_error) = self.refresh_checkpoint_hash() {
            self.state = transaction_start;
            return Err(hash_error);
        }
        Ok(CommandOutcome::Rejected {
            rejection: CommandRejection {
                attempt_id: Some(attempt_id),
                request_id: admission.request_id,
                retained_revision: admission.revision_before,
                rejected_at: self.state.now,
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
            &self.state.run_configuration,
            issuer,
            authority,
            admission,
            &|entity| runtime_entity_exists(&self.state, entity),
        )
    }

    pub fn advance(&mut self, duration: SimDuration) -> Result<Vec<SimEvent>, CanwuError> {
        self.ensure_runtime_ready()?;
        if duration.is_negative() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "simulation time cannot advance by a negative duration",
            ));
        }
        let target = self.state.now.checked_add(duration).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDuration,
                "simulation target time exceeds the supported range",
            )
        })?;
        self.advance_to(target)
    }

    pub fn step(&mut self) -> Result<Vec<SimEvent>, CanwuError> {
        self.ensure_runtime_ready()?;
        let Some(next_time) = self.state.scheduler.keys().next().map(|key| key.at) else {
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
        let target = self.state.now.checked_add(maximum).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDuration,
                "advance_until target time exceeds the supported range",
            )
        })?;
        let start = self.state.events.len();
        while self.state.now < target && !condition(self) {
            let next_time = self
                .state
                .scheduler
                .keys()
                .next()
                .map_or(target, |key| key.at.min(target));
            self.advance_to(next_time)?;
            if next_time == target {
                break;
            }
        }
        Ok(self.state.events[start..].to_vec())
    }

    pub fn settle_boundary(
        &mut self,
        mut request: BoundaryRequest,
    ) -> Result<BoundaryReceipt, CanwuError> {
        self.ensure_runtime_ready()?;
        if request.at < self.state.now {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "a settlement boundary cannot precede committed simulation time",
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

        let transaction_start = self.state.clone();
        match self.settle_boundary_inner(request) {
            Ok(receipt) => Ok(receipt),
            Err(error) => {
                self.state = transaction_start;
                Err(error)
            }
        }
    }

    fn settle_boundary_inner(
        &mut self,
        request: BoundaryRequest,
    ) -> Result<BoundaryReceipt, CanwuError> {
        self.advance_to(request.at)?;

        let previously_admitted_attempts: BTreeSet<_> = self
            .state
            .boundaries
            .iter()
            .flat_map(|record| record.admitted_attempts.iter().copied())
            .collect();
        let previously_admitted_commands: BTreeSet<_> = self
            .state
            .boundaries
            .iter()
            .flat_map(|record| record.admitted_commands.iter().copied())
            .collect();
        let previously_admitted_events: BTreeSet<_> = self
            .state
            .boundaries
            .iter()
            .flat_map(|record| record.admitted_events.iter().copied())
            .collect();
        let admitted_attempts: Vec<_> = self
            .state
            .command_attempts
            .iter()
            .map(|record| record.id)
            .filter(|id| !previously_admitted_attempts.contains(id))
            .collect();
        let admitted_commands: Vec<_> = self
            .state
            .commands
            .iter()
            .map(|record| record.id)
            .filter(|id| !previously_admitted_commands.contains(id))
            .collect();
        let admitted_events: Vec<_> = self
            .state
            .events
            .iter()
            .map(|event| event.id)
            .filter(|id| !previously_admitted_events.contains(id))
            .collect();

        let (boundary_id_value, next_boundary_id) =
            claim_counter(self.state.next_boundary_id, "boundary ID")?;
        let (correlation_id, next_correlation_id) =
            claim_counter(self.state.next_correlation_id, "boundary correlation ID")?;
        self.state.next_boundary_id = next_boundary_id;
        self.state.next_correlation_id = next_correlation_id;
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
        let mut random_overlay = boundary_snapshot.random_streams.clone();
        let mut pending_random_draws = Vec::new();
        let mut visible_overlay = BTreeMap::new();
        let mut candidate_overlay = BTreeMap::new();
        let mut visible_record_overlay = BTreeMap::new();
        let mut candidate_record_overlay = BTreeMap::new();
        let mut ordinary = Vec::new();
        let mut transitions = Vec::new();
        let mut deferred = Vec::new();
        let mut changes = Vec::new();
        let mut record_changes = Vec::new();
        let mut emissions = Vec::new();

        for phase in BoundaryPhase::ALL {
            match phase {
                BoundaryPhase::AtomicDomainCommit => {
                    let (same_boundary, next_boundary) =
                        partition_boundary_visibility(std::mem::take(&mut ordinary));
                    self.apply_boundary_stage(
                        boundary_id,
                        correlation_id,
                        same_boundary,
                        &mut changes,
                        &mut record_changes,
                        &mut emissions,
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
                        &mut changes,
                        &mut record_changes,
                        &mut emissions,
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
                        !admitted_events.is_empty(),
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
                    admitted_events: admitted_events.clone(),
                    emitted_events: emissions.iter().map(|emission| emission.event).collect(),
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
                    &state_owners,
                    &record_schemas,
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
                        &mut changes,
                        &mut record_changes,
                        &mut emissions,
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

        self.apply_boundary_stage(
            boundary_id,
            correlation_id,
            deferred,
            &mut changes,
            &mut record_changes,
            &mut emissions,
        )?;
        self.state.random_streams = random_overlay;
        let random_draws =
            self.append_boundary_random_draws(boundary_id, correlation_id, pending_random_draws)?;
        self.state.plugin_registration_closed = true;
        let state_hash = self.compute_boundary_state_hash()?;
        let previous_hash = self.state.boundaries.last().map_or_else(
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
        self.state.boundaries.push(record);
        self.refresh_checkpoint_hash()?;
        Ok(BoundaryReceipt {
            boundary_id,
            settled_at: request.at,
            emitted_events: emissions
                .into_iter()
                .map(|emission| emission.event)
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
        let plugin_components: Vec<_> = self.state.plugin_components.values().cloned().collect();
        let domain_records: Vec<_> = self.state.domain_records.values().cloned().collect();
        let plugin_descriptors: Vec<_> = self.plugins.descriptors().cloned().collect();
        let scheduled: Vec<_> = self
            .state
            .scheduler
            .iter()
            .map(|(key, action)| ScheduledRecord {
                key: key.clone(),
                action: action.clone(),
            })
            .collect();
        let random_streams: Vec<_> = self.state.random_streams.values().cloned().collect();
        let (authoritative_manifest, authoritative_manifest_hash) = authoritative_run_identity(
            &self.state.run_manifest,
            &self.state.run_manifest_hash,
            &self.state.run_configuration,
        )?;
        let initial_scenario = self.bound_initial_scenario();
        state_hash(&StateHashMaterial {
            engine_version: ENGINE_VERSION,
            snapshot_format_version: SNAPSHOT_FORMAT_VERSION,
            run_manifest: &authoritative_manifest,
            run_manifest_hash: &authoritative_manifest_hash,
            initial_time: self.state.initial_time,
            initial_scenario,
            now: self.state.now,
            plugin_registration_closed: self.state.plugin_registration_closed,
            world: &world,
            knowledge: &self.state.knowledge,
            events: &self.state.events,
            commands: &self.state.commands,
            command_attempts: &self.state.command_attempts,
            plugin_components: &plugin_components,
            domain_records: &domain_records,
            plugin_descriptors: &plugin_descriptors,
            schema: &self.schema,
            scheduled: &scheduled,
            root_seed: self.state.root_seed,
            random_streams: &random_streams,
            random_draws: &self.state.random_draws,
            next_event_id: self.state.next_event_id,
            next_command_id: self.state.next_command_id,
            next_command_attempt_id: self.state.next_command_attempt_id,
            next_boundary_id: self.state.next_boundary_id,
            next_random_draw_id: self.state.next_random_draw_id,
            next_schedule_sequence: self.state.next_schedule_sequence,
            next_correlation_id: self.state.next_correlation_id,
        })
    }

    fn refresh_checkpoint_hash(&mut self) -> Result<(), CanwuError> {
        let state_hash = self.compute_boundary_state_hash()?;
        self.state.checkpoint_hash = checkpoint_hash_for_configuration(
            &state_hash,
            self.boundary_head_hash(),
            &self.state.run_manifest_hash,
            &self.state.run_configuration,
        )?;
        Ok(())
    }

    #[must_use]
    pub fn snapshot(&self) -> SimulationSnapshot {
        SimulationSnapshot {
            engine_version: ENGINE_VERSION.to_owned(),
            snapshot_format_version: SNAPSHOT_FORMAT_VERSION,
            run_manifest: Some(self.state.run_manifest.clone()),
            run_manifest_hash: self.state.run_manifest_hash.clone(),
            run_configuration: Some(self.state.run_configuration.clone()),
            checkpoint_hash: self.state.checkpoint_hash.clone(),
            initial_time: self.state.initial_time,
            initial_scenario: self.bound_initial_scenario().cloned(),
            now: self.state.now,
            plugin_registration_closed: self.state.plugin_registration_closed,
            world: self.world(),
            knowledge: self.state.knowledge.clone(),
            events: self.state.events.clone(),
            commands: self.state.commands.clone(),
            command_attempts: self.state.command_attempts.clone(),
            boundaries: self.state.boundaries.clone(),
            plugin_components: self.state.plugin_components.values().cloned().collect(),
            domain_records: self.state.domain_records.values().cloned().collect(),
            plugin_descriptors: self.plugins.descriptors().cloned().collect(),
            schema: self.schema.clone(),
            root_seed: self.state.root_seed,
            random_streams: self.state.random_streams.values().cloned().collect(),
            random_draws: self.state.random_draws.clone(),
            scheduled: self
                .state
                .scheduler
                .iter()
                .map(|(key, action)| ScheduledRecord {
                    key: key.clone(),
                    action: action.clone(),
                })
                .collect(),
            legacy_rng: None,
            next_event_id: self.state.next_event_id,
            next_command_id: self.state.next_command_id,
            next_command_attempt_id: self.state.next_command_attempt_id,
            next_boundary_id: self.state.next_boundary_id,
            next_random_draw_id: self.state.next_random_draw_id,
            next_schedule_sequence: self.state.next_schedule_sequence,
            next_correlation_id: self.state.next_correlation_id,
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
                initial_time: snapshot.initial_time,
                initial_scenario,
                now: snapshot.now,
                run_manifest: snapshot.run_manifest.clone().ok_or_else(|| {
                    invalid_snapshot_error("snapshot is missing its run manifest")
                })?,
                run_manifest_hash: snapshot.run_manifest_hash.clone(),
                run_configuration: snapshot.run_configuration.clone().ok_or_else(|| {
                    invalid_snapshot_error("snapshot is missing its run configuration")
                })?,
                checkpoint_hash: snapshot.checkpoint_hash.clone(),
                plugin_registration_closed: snapshot.plugin_registration_closed,
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
                scheduler: snapshot
                    .scheduled
                    .into_iter()
                    .map(|record| (record.key, record.action))
                    .collect(),
                events: snapshot.events,
                commands: snapshot.commands,
                command_attempts: snapshot.command_attempts,
                boundaries: snapshot.boundaries,
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
                random_draws: snapshot.random_draws,
                next_event_id: snapshot.next_event_id,
                next_command_id: snapshot.next_command_id,
                next_command_attempt_id: snapshot.next_command_attempt_id,
                next_boundary_id: snapshot.next_boundary_id,
                next_random_draw_id: snapshot.next_random_draw_id,
                next_schedule_sequence: snapshot.next_schedule_sequence,
                next_correlation_id: snapshot.next_correlation_id,
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
                let person = self.state.people.get(&actor).ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::ActorNotFound,
                        format!("actor {actor} was not found"),
                    )
                    .with_entity(EntityRef::Person(actor))
                })?;
                let army_state = self.state.armies.get(army).ok_or_else(|| {
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
                if !self.state.territories.contains_key(destination) {
                    return Err(CanwuError::new(
                        ErrorCode::DestinationNotFound,
                        format!("destination {destination} was not found"),
                    )
                    .with_entity(EntityRef::Territory(*destination)));
                }
                let route = self
                    .state
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
                let old_morale = self.state.armies.get(army).map_or_else(
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
                let army_state = self.state.armies.get_mut(&army).ok_or_else(|| {
                    CanwuError::new(ErrorCode::ArmyNotFound, "validated army disappeared")
                })?;
                army_state.transit = Some(TransitState {
                    from,
                    to: destination,
                    departed_at: self.state.now,
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
        let start = self.state.events.len();
        while let Some(boundary_time) = self.state.scheduler.keys().next().map(|key| key.at)
            && boundary_time <= target
        {
            let boundary_start = self.state.clone();
            self.state.now = boundary_time;
            while self
                .state
                .scheduler
                .first_key_value()
                .is_some_and(|(key, _)| key.at == boundary_time)
            {
                let (_, action) = self
                    .state
                    .scheduler
                    .pop_first()
                    .expect("scheduler was checked as non-empty");
                if let Err(error) = self.execute_scheduled(action) {
                    self.state = boundary_start;
                    return Err(error);
                }
            }
            self.state.plugin_registration_closed = true;
            if let Err(error) = self.refresh_checkpoint_hash() {
                self.state = boundary_start;
                return Err(error);
            }
        }
        let target_start = self.state.clone();
        self.state.now = target;
        self.state.plugin_registration_closed = true;
        if let Err(error) = self.refresh_checkpoint_hash() {
            self.state = target_start;
            return Err(error);
        }
        Ok(self.state.events[start..].to_vec())
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
        let commander = {
            let army_state = self.state.armies.get_mut(&army).ok_or_else(|| {
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
            self.state.now,
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
                    observed_at: self.state.now,
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
        let (strength, known_name) = self.state.armies.get(&army).map_or_else(
            || (0, None),
            |value| (value.strength, Some(value.name.clone())),
        );
        let actor = self
            .state
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
                learned_at: self.state.now,
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
        let (event_id, next_event_id) = claim_counter(self.state.next_event_id, "event ID")?;
        let id = EventId::new(event_id);
        self.state.next_event_id = next_event_id;
        let event = SimEvent {
            id,
            timestamp: self.state.now,
            kind,
            affected_entities,
            summary,
            cause,
            correlation_id,
        };
        self.state.events.push(event.clone());
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
            claim_counter(self.state.next_random_draw_id, "random draw ID")?;
        let state = self.state.random_streams.get_mut(stream).ok_or_else(|| {
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
        self.state.next_random_draw_id = next_random_draw_id;
        let id = RandomDrawId::new(draw_id);
        self.state.random_draws.push(RandomDrawRecord {
            id,
            at: self.state.now,
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
                claim_counter(self.state.next_random_draw_id, "random draw ID")?;
            let id = RandomDrawId::new(draw_id);
            self.state.next_random_draw_id = next_random_draw_id;
            self.state.random_draws.push(RandomDrawRecord {
                id,
                at: self.state.now,
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
        changes: &mut Vec<BoundaryChange>,
        record_changes: &mut Vec<DomainRecordChange>,
        emissions: &mut Vec<BoundaryEmission>,
    ) -> Result<(), CanwuError> {
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
                BoundaryDirective::SetComponent { .. } | BoundaryDirective::Emit { .. } => None,
            })
            .collect();
        let mut stage_record_changes = BTreeMap::new();
        if !mutation_requests.is_empty() {
            let (next_records, applied) = records::apply_mutation_bundle(
                &self.state.domain_records,
                &self.plugins.record_schemas,
                self.state.now,
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
            self.state.domain_records = next_records;
            record_changes.extend(applied);
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
                    let previous = self
                        .state
                        .plugin_components
                        .get(&key)
                        .map(|record| record.value.clone());
                    self.state.plugin_components.insert(
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
                    self.state.plugin_components.insert(
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
                    let at = self.state.now.checked_add(after).ok_or_else(|| {
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
        if at <= self.state.now {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "scheduled work must target a strictly future simulation time",
            ));
        }
        let (sequence, next_sequence) =
            claim_counter(self.state.next_schedule_sequence, "schedule sequence")?;
        let key = ScheduleKey { at, sequence };
        self.state.next_schedule_sequence = next_sequence;
        if self.state.scheduler.insert(key, action).is_some() {
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
    visibility: StateVisibility,
    directive: BoundaryDirective,
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

fn validate_boundary_proposal(
    plugin: &str,
    contract: &BoundarySystemContract,
    state: &RuntimeState,
    state_owners: &BTreeMap<StateKey, String>,
    record_schemas: &records::DomainRecordSchemas,
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
        proposal_entity_exists(state, record_schemas, record_overlay, proposal, entity)
    };
    let mut offered_pools = BTreeSet::new();
    for offer in &proposal.offers {
        validate_reservation_pool(&offer.pool, &entity_exists)?;
        if !contract.reservation_offers.contains(&offer.pool.state)
            || state_owners
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
                    || state_owners
                        .get(state_key)
                        .is_none_or(|owner| owner != plugin)
                    || is_domain_record_state(record_schemas, state_key)
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
                    || state_owners
                        .get(&state_key)
                        .is_none_or(|owner| owner != plugin)
                    || record_schemas
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
    let mut base = state.domain_records.clone();
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
            BoundaryDirective::SetComponent { .. } | BoundaryDirective::Emit { .. } => None,
        })
        .collect();
    if requests.is_empty() {
        return Ok(());
    }
    let (next, changes) = records::apply_mutation_bundle(
        &base,
        schemas,
        state.now,
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
        &|entity| snapshot_entity_identity_exists(snapshot, entity),
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
    if snapshot.initial_time > snapshot.now {
        return invalid_snapshot("snapshot initial time cannot follow its current time");
    }
    let has_execution_evidence = snapshot.now != snapshot.initial_time
        || !snapshot.commands.is_empty()
        || !snapshot.command_attempts.is_empty()
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
    let boundary_count = u64::try_from(snapshot.boundaries.len())
        .map_err(|_| invalid_snapshot_error("boundary count exceeds the revision range"))?;
    let mut boundaries_before_attempt = vec![boundary_count; snapshot.command_attempts.len()];
    for (boundary_index, boundary) in snapshot.boundaries.iter().enumerate() {
        let prior_boundaries = u64::try_from(boundary_index)
            .map_err(|_| invalid_snapshot_error("boundary index exceeds the revision range"))?;
        for attempt_id in &boundary.admitted_attempts {
            let attempt_index =
                usize::try_from(attempt_id.get().saturating_sub(1)).map_err(|_| {
                    invalid_snapshot_error("boundary attempt ID exceeds the journal index range")
                })?;
            let Some(value) = boundaries_before_attempt.get_mut(attempt_index) else {
                return invalid_snapshot("boundary admits an unknown command attempt");
            };
            *value = prior_boundaries;
        }
    }
    let mut request_ids = BTreeSet::new();
    let mut accepted_attempts = BTreeMap::new();
    let mut accepted_command_count = 0_u64;
    let mut previous_attempt = None;
    for (index, attempt) in snapshot.command_attempts.iter().enumerate() {
        let expected_id = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| {
                invalid_snapshot_error("command attempt index exceeds identifier space")
            })?;
        let expected_revision = accepted_command_count
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
        let preflight_error = snapshot_command_attempt_preflight_error(snapshot, attempt);
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
        validate_snapshot_command(snapshot, plugins, &record.envelope)?;
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
        if event
            .affected_entities
            .iter()
            .any(|entity| !snapshot_entity_identity_exists(snapshot, entity))
        {
            return invalid_snapshot("event references an unknown entity");
        }
        validate_event_kind(snapshot, plugins, event)?;
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
    let (max_boundary_id, max_boundary_correlation) = validate_boundary_records(
        snapshot,
        plugins,
        &domain_records,
        initial_domain_records.as_ref(),
    )?;
    let (max_random_draw_id, max_random_correlation) = validate_random_evidence(snapshot, plugins)?;
    let current_state_hash = snapshot_state_hash(snapshot)?;
    let expected_checkpoint_hash = checkpoint_hash_for_configuration(
        &current_state_hash,
        snapshot
            .boundaries
            .last()
            .map(|record| record.hash.as_str()),
        &snapshot.run_manifest_hash,
        run_configuration,
    )?;
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
                        !record.admitted_events.is_empty(),
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

fn validate_boundary_records(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    final_domain_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    initial_domain_records: Option<&BTreeMap<DomainRecordRef, DomainRecord>>,
) -> Result<(u64, u64), CanwuError> {
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
        validate_boundary_reservations(record, snapshot, plugins)?;
        validate_boundary_changes(record, snapshot, plugins)?;
        validate_boundary_record_changes(record, snapshot, plugins, &mut domain_record_values)?;
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
        validate_boundary_emissions(record, snapshot, plugins, &mut emitted_events)?;
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
    Ok((max_boundary_id, max_correlation_id))
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
) -> Result<(), CanwuError> {
    if record.reservation_offers.windows(2).any(|pair| {
        (&pair[0].offer.pool, &pair[0].plugin, &pair[0].system)
            >= (&pair[1].offer.pool, &pair[1].plugin, &pair[1].system)
    }) {
        return invalid_snapshot("boundary reservation offers are not canonical");
    }
    let mut remaining = BTreeMap::new();
    for offered in &record.reservation_offers {
        validate_snapshot_reservation_pool(&offered.offer.pool, snapshot)?;
        let Some(contract) = snapshot_boundary_contract(plugins, &offered.plugin, &offered.system)
        else {
            return invalid_snapshot("reservation offer references an unknown boundary system");
        };
        if contract.phase != BoundaryPhase::ReservationAndAllocation
            || !boundary_system_due(
                contract,
                &record.cadences,
                !record.admitted_events.is_empty(),
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
        validate_snapshot_reservation_pool(&requested.request.pool, snapshot)?;
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
                !record.admitted_events.is_empty(),
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
) -> Result<(), CanwuError> {
    if pool.resource.trim().is_empty()
        || pool.resource != pool.resource.trim()
        || !snapshot_entity_identity_exists(snapshot, &pool.entity)
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

fn validate_boundary_changes(
    record: &BoundaryRecord,
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
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
        if change.component.trim().is_empty()
            || change.component != change.component.trim()
            || !snapshot_entity_exists(snapshot, &change.entity)
            || !contract.writes.contains(&change.state)
            || !boundary_system_due(
                contract,
                &record.cadences,
                !record.admitted_events.is_empty(),
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
) -> Result<(), CanwuError> {
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
                !record.admitted_events.is_empty(),
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

    for changes in by_stage.values() {
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
        *values = next;
    }
    Ok(())
}

fn validate_boundary_emissions(
    record: &BoundaryRecord,
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
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
            !record.admitted_events.is_empty(),
        ) {
            return invalid_snapshot("boundary emission source system was not due");
        }

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
                if !contract.emits.contains(event_type) {
                    return invalid_snapshot(
                        "boundary event type is absent from its source system manifest",
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

fn validate_snapshot_command(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    envelope: &CommandEnvelope,
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
            snapshot_entity_identity_exists(snapshot, entity)
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
        EventKind::DebugFieldChanged { entity, .. } => {
            snapshot_entity_identity_exists(snapshot, entity)
        }
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
        EntityRef::Army(id) => state.armies.contains_key(id),
        EntityRef::Domain(reference) => {
            records::domain_entity_exists(&state.domain_records, reference)
        }
        EntityRef::Government(id) => state.governments.contains_key(id),
        EntityRef::Person(id) => state.people.contains_key(id),
        EntityRef::Route(id) => state.routes.contains_key(id),
        EntityRef::Territory(id) => state.territories.contains_key(id),
        EntityRef::Organization(_) | EntityRef::Resource(_) => false,
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
            .or_else(|| state.domain_records.get(reference))
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

fn validate_runtime_domain_dependents(state: &RuntimeState) -> Result<(), CanwuError> {
    validate_domain_dependents_with_records(state, &state.domain_records)
}

fn validate_domain_dependents_with_records(
    state: &RuntimeState,
    domain_records: &BTreeMap<DomainRecordRef, DomainRecord>,
) -> Result<(), CanwuError> {
    let unavailable = |entity: &EntityRef| matches!(entity, EntityRef::Domain(reference) if !records::domain_entity_exists(domain_records, reference));
    if state
        .plugin_components
        .values()
        .any(|component| unavailable(&component.entity))
    {
        return Err(CanwuError::new(
            ErrorCode::DomainRecordReferenced,
            "a domain entity with persisted plugin components cannot be deleted",
        ));
    }
    if state.scheduler.values().any(|action| match action {
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
            Ok(snapshot)
        }
        2 => {
            if snapshot.run_manifest.is_some()
                || !snapshot.run_manifest_hash.is_empty()
                || !snapshot.checkpoint_hash.is_empty()
            {
                return invalid_snapshot("format 2 snapshots cannot contain current manifest data");
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
            {
                return invalid_snapshot("format 3 snapshots cannot contain current manifest data");
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

fn validate_legacy_ingress_shape(snapshot: &SimulationSnapshot) -> Result<(), CanwuError> {
    if snapshot.run_configuration.is_some()
        || snapshot.initial_scenario.is_some()
        || !snapshot.command_attempts.is_empty()
        || !snapshot.domain_records.is_empty()
        || snapshot.next_command_attempt_id != 1
        || snapshot
            .commands
            .iter()
            .any(|record| record.attempt_id.is_some() || !record.emitted_events.is_empty())
        || snapshot
            .boundaries
            .iter()
            .any(|record| !record.admitted_attempts.is_empty() || !record.record_changes.is_empty())
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
    ENGINE_VERSION.clone_into(&mut snapshot.engine_version);
    snapshot.snapshot_format_version = SNAPSHOT_FORMAT_VERSION;
    snapshot.checkpoint_hash = snapshot_checkpoint_hash(&snapshot)?;
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

fn state_hash(material: &StateHashMaterial<'_>) -> Result<String, CanwuError> {
    canonical_hash("canwu.boundary-state.v1", material)
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
        next_boundary_id: snapshot.next_boundary_id,
        next_random_draw_id: snapshot.next_random_draw_id,
        next_schedule_sequence: snapshot.next_schedule_sequence,
        next_correlation_id: snapshot.next_correlation_id,
    })
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
) -> Result<String, CanwuError> {
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

fn snapshot_checkpoint_hash(snapshot: &SimulationSnapshot) -> Result<String, CanwuError> {
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
    )
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
        RandomStreamKey::new("boundary-failure", "rollback-proof", 1)
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

    fn staged_failure(
        view: &SimulationView<'_>,
        _context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        let _ = view.random_range(&failure_random_stream(), 100, "rollback proof")?;
        Ok(BoundaryProposal {
            directives: vec![BoundaryDirective::SetComponent {
                state: StateKey::new("boundary-failure", "value"),
                entity: EntityRef::Army(ArmyId::new(1)),
                component: "value".to_owned(),
                value: Value::Bool(true),
                summary: "Stage a value before invariant failure".to_owned(),
            }],
            ..BoundaryProposal::default()
        })
    }

    fn reject_boundary(
        _view: &SimulationView<'_>,
        _context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "injected boundary invariant failure",
        ))
    }

    struct BoundaryFailurePlugin;

    impl SimulationPlugin for BoundaryFailurePlugin {
        fn name(&self) -> &'static str {
            "boundary-failure"
        }

        test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000011");

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            let mut propose = BoundarySystemContract::new(
                "propose",
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::Daily,
            );
            propose.writes = vec![StateKey::new("boundary-failure", "value")];
            propose.random_streams = vec![failure_random_stream()];
            propose.visibility = StateVisibility::SameBoundary;
            registrar.register_boundary_system(propose, staged_failure)?;
            registrar.register_boundary_system(
                BoundarySystemContract::new(
                    "reject",
                    BoundaryPhase::InvariantValidation,
                    SystemCadence::Daily,
                ),
                reject_boundary,
            )
        }
    }

    struct RecordLifecyclePlugin;
    struct RecordDeleteOnlyPlugin;

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
                        record: office_draft("office-c", "Later Secretariat"),
                    },
                    summary: "Create the later successor office".to_owned(),
                },
                BoundaryDirective::MutateRecord {
                    mutation: DomainRecordMutation::Create {
                        record: obligation_draft("office-a", "open"),
                    },
                    summary: "Create an obligation assigned to the original office".to_owned(),
                },
            ],
            2 => vec![BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Retire {
                    record: office_reference("office-a"),
                    expected_version: 1,
                    successor: Some(office_reference("office-b")),
                },
                summary: "Retire the original office with a stable successor".to_owned(),
            }],
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

        let writes = vec![office_state, obligation_state];
        let mut lifecycle = BoundarySystemContract::new(
            "lifecycle",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        lifecycle.writes.clone_from(&writes);
        lifecycle.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(lifecycle, handler)?;

        let mut invariant = BoundarySystemContract::new(
            "validate-lifecycle",
            BoundaryPhase::InvariantValidation,
            SystemCadence::Daily,
        );
        invariant.reads = writes;
        registrar.register_boundary_system(invariant, validate_record_lifecycle_view)
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
        assert_eq!(rejection.retained_revision, 1);
        assert_eq!(rejection.error.code, ErrorCode::SimulationRevisionConflict);
        assert_eq!(simulation.revision(), 1);
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
        cyclic_cause.checkpoint_hash = snapshot_checkpoint_hash(&cyclic_cause)
            .expect("cyclic cause fixture should carry a coherent container commitment");
        let Err(error) = Simulation::from_snapshot(cyclic_cause) else {
            panic!("event cause cycles must be rejected without unbounded traversal");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert!(error.message.contains("parent event"));

        let mut forged = after_stale;
        forged.command_attempts[0].envelope.issuer = Issuer::Human("controller.other".to_owned());
        forged.commands[0].envelope = forged.command_attempts[0].envelope.clone();
        forged.checkpoint_hash = snapshot_checkpoint_hash(&forged)
            .expect("the forged fixture should carry a coherent container commitment");
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
            Simulation::new(35, scenario).expect("compatibility run should load");
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
        Simulation::from_snapshot(after_tracked)
            .expect("a rejection-only tracked journal should remain loadable");
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
        forged.checkpoint_hash = snapshot_checkpoint_hash(&forged)
            .expect("forged container should have a coherent commitment");
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
        assert_eq!(rejection.retained_revision, 1);
        let accepted = after_boundary
            .process_command(CommandRequest::new(
                CommandRequestId::new(2),
                1,
                command_at(SimTime::EPOCH),
            ))
            .expect("current revision should be accepted");
        let CommandOutcome::Accepted { receipt } = accepted else {
            panic!("current revision and time should admit the command");
        };
        assert_eq!(receipt.revision, 2);
        assert_eq!(after_boundary.revision(), 2);
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
        assert_eq!(rejection.retained_revision, 0);
        let accepted = after_clock
            .process_command(CommandRequest::new(
                CommandRequestId::new(2),
                0,
                command_at(after_clock.time()),
            ))
            .expect("current revision and time should be accepted");
        let CommandOutcome::Accepted { receipt } = accepted else {
            panic!("current clock guard should admit the command");
        };
        assert_eq!(receipt.revision, 1);
        let clock_journal = after_clock.replay_journal();
        let clock_replay = Simulation::replay_from_journal(scenario, &[], &clock_journal)
            .expect("clock-relative time evidence should replay exactly");
        assert_eq!(after_clock.snapshot(), clock_replay.snapshot());
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
            CommandRequest::new(
                CommandRequestId::new(1),
                0,
                CommandEnvelope::new(
                    Issuer::Actor(ids.commander),
                    Command::MoveArmy {
                        army: ArmyId::new(999),
                        destination: ids.eastern_territory,
                    },
                ),
            ),
            CommandRequest::new(
                CommandRequestId::new(2),
                0,
                CommandEnvelope::new(
                    Issuer::Debug,
                    Command::DebugSetArmyMorale {
                        army: ids.army,
                        morale: 101,
                    },
                ),
            ),
            CommandRequest::new(
                CommandRequestId::new(3),
                0,
                CommandEnvelope::new(
                    Issuer::Actor(ids.commander),
                    Command::Plugin {
                        plugin: "authority-test".to_owned(),
                        command: "set_stance".to_owned(),
                        payload: serde_json::json!({}),
                    },
                ),
            ),
            CommandRequest::new(
                CommandRequestId::new(4),
                0,
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
        for (request, expected_code) in requests.into_iter().zip(expected) {
            let outcome = simulation
                .process_command(request)
                .expect("expected domain rejection should be a command outcome");
            let CommandOutcome::Rejected { rejection } = outcome else {
                panic!("invalid command fixture must be rejected");
            };
            assert_eq!(rejection.error.code, expected_code);
            assert_eq!(rejection.retained_revision, 0);
        }
        assert!(simulation.command_log().is_empty());
        assert_eq!(simulation.command_attempts().len(), 4);

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
        forged_live_ingress.checkpoint_hash = snapshot_checkpoint_hash(&forged_live_ingress)
            .expect("the forged fixture should carry a coherent container commitment");
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
        unmigrated_engine.checkpoint_hash = snapshot_checkpoint_hash(&unmigrated_engine)
            .expect("the other-engine fixture should carry a coherent commitment");
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
        changed_delivery.checkpoint_hash = snapshot_checkpoint_hash(&changed_delivery)
            .expect("the causally inconsistent fixture should hash");
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
        missing_draw.checkpoint_hash =
            snapshot_checkpoint_hash(&missing_draw).expect("the draw-omission fixture should hash");
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
        legacy.checkpoint_hash = snapshot_checkpoint_hash(&legacy)
            .expect("manifest-only fixture should retain its old commitment");
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
        exhausted_counter.checkpoint_hash = snapshot_checkpoint_hash(&exhausted_counter)
            .expect("the coherent exhausted fixture should hash");
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
        due_at_boundary.checkpoint_hash = snapshot_checkpoint_hash(&due_at_boundary)
            .expect("the structurally corrupted fixture should still hash");
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
        redundant_initial_scenario.checkpoint_hash =
            snapshot_checkpoint_hash(&redundant_initial_scenario)
                .expect("redundant record-free genesis should still hash coherently");
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
        assert_eq!(created.record_change_count, 4);
        assert_eq!(retired.record_change_count, 1);
        assert_eq!(succession.record_change_count, 1);
        assert_eq!(deleted.record_change_count, 2);
        assert_eq!(
            created.change_count
                + retired.change_count
                + succession.change_count
                + deleted.change_count,
            0
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

        let mut corrupted = simulation.snapshot();
        corrupted.boundaries[1].record_changes[0].system = "forged-system".to_owned();
        let mut previous_hash = GENESIS_BOUNDARY_HASH.to_owned();
        for boundary in &mut corrupted.boundaries {
            boundary.previous_hash.clone_from(&previous_hash);
            boundary.hash =
                compute_boundary_hash(boundary).expect("tampered boundary should still hash");
            previous_hash.clone_from(&boundary.hash);
        }
        corrupted.checkpoint_hash = snapshot_checkpoint_hash(&corrupted)
            .expect("tampered snapshot should still have a coherent outer commitment");
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
        shifted_to_genesis.checkpoint_hash = snapshot_checkpoint_hash(&shifted_to_genesis)
            .expect("history-shift forgery should have a coherent outer commitment");
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
        stripped_feature.checkpoint_hash = snapshot_checkpoint_hash(&stripped_feature)
            .expect("feature-strip forgery should have a coherent outer commitment");
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
    fn failed_phased_boundary_restores_ingress_time_state_and_identifiers() {
        let (mut simulation, _) = Simulation::demo(35).expect("demo should load");
        simulation
            .register_plugin(&BoundaryFailurePlugin)
            .expect("failure fixture should register");
        let before = simulation
            .snapshot_json()
            .expect("snapshot should serialize");
        let error = simulation
            .settle_boundary(
                BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(1))
                    .with_cadence(SystemCadence::Daily),
            )
            .expect_err("invariant rejection must abort the whole boundary");
        assert_eq!(error.code, ErrorCode::InvalidBoundary);
        assert_eq!(
            before,
            simulation
                .snapshot_json()
                .expect("failed settlement must restore every serialized field")
        );
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
        assert!(error.message.contains("checkpoint hash"));

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
        missing_state_commitment.checkpoint_hash =
            snapshot_checkpoint_hash(&missing_state_commitment)
                .expect("the malformed checkpoint should hash");
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
    fn failed_scheduled_boundary_restores_clock_queue_and_state() {
        let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
        simulation
            .register_plugin(&FailingPlugin)
            .expect("plugin should register");
        simulation
            .submit(CommandEnvelope::new(
                Issuer::Actor(ids.commander),
                Command::Plugin {
                    plugin: "failing-test".to_owned(),
                    command: "mutate".to_owned(),
                    payload: serde_json::json!({ "scheduled": true }),
                },
            ))
            .expect("scheduling the valid directive should succeed");
        let before_boundary = simulation
            .snapshot_json()
            .expect("snapshot should serialize");
        let error = simulation
            .advance(SimDuration::days(1))
            .expect_err("the scheduled boundary should fail");
        assert_eq!(error.code, ErrorCode::InvalidDuration);
        assert_eq!(
            before_boundary,
            simulation
                .snapshot_json()
                .expect("failed boundary must restore its clock, queue, state, events, and IDs")
        );
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
}
