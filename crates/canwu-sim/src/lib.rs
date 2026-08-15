//! Deterministic runtime, validated commands, scheduling, plugins, and snapshots.

#![allow(
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::too_many_lines
)]

use canwu_core::{
    ArmyId, CommandId, DeterministicRng, EntityRef, EventId, FieldSchema, GovernmentId, PersonId,
    RouteId, SchemaRegistry, TerritoryId, TypeSchema,
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
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::panic::{AssertUnwindSafe, catch_unwind};

pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SNAPSHOT_FORMAT_VERSION: u32 = 2;
const CORE_STATE_NAMESPACE: &str = "canwu.core";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    ActorNotFound,
    ArmyNotFound,
    DestinationNotFound,
    DuplicatePlugin,
    DuplicatePluginCommand,
    DuplicatePluginSystem,
    DuplicateStateOwner,
    EntityNotFound,
    InvalidAuthority,
    InvalidDuration,
    InvalidPayload,
    InvalidPluginRegistration,
    InvalidSnapshot,
    IdentifierExhausted,
    NoRoute,
    PluginCommandNotFound,
    PluginManifestMismatch,
    PluginNotActive,
    PluginPanicked,
    PluginRegistrationClosed,
    SimulationTimeConflict,
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
        ErrorCode::DuplicatePlugin => "duplicate_plugin",
        ErrorCode::DuplicatePluginCommand => "duplicate_plugin_command",
        ErrorCode::DuplicatePluginSystem => "duplicate_plugin_system",
        ErrorCode::DuplicateStateOwner => "duplicate_state_owner",
        ErrorCode::EntityNotFound => "entity_not_found",
        ErrorCode::InvalidAuthority => "invalid_authority",
        ErrorCode::InvalidDuration => "invalid_duration",
        ErrorCode::InvalidPayload => "invalid_payload",
        ErrorCode::InvalidPluginRegistration => "invalid_plugin_registration",
        ErrorCode::InvalidSnapshot => "invalid_snapshot",
        ErrorCode::IdentifierExhausted => "identifier_exhausted",
        ErrorCode::NoRoute => "no_route",
        ErrorCode::PluginCommandNotFound => "plugin_command_not_found",
        ErrorCode::PluginManifestMismatch => "plugin_manifest_mismatch",
        ErrorCode::PluginNotActive => "plugin_not_active",
        ErrorCode::PluginPanicked => "plugin_panicked",
        ErrorCode::PluginRegistrationClosed => "plugin_registration_closed",
        ErrorCode::SimulationTimeConflict => "simulation_time_conflict",
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
    Debug,
    System(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandContext {
    pub issuer: Issuer,
    pub command_id: CommandId,
    pub simulation_time: SimTime,
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
    pub command: Command,
    pub expected_time: Option<SimTime>,
}

impl CommandEnvelope {
    #[must_use]
    pub const fn new(issuer: Issuer, command: Command) -> Self {
        Self {
            issuer,
            command,
            expected_time: None,
        }
    }

    #[must_use]
    pub const fn at_time(mut self, expected_time: SimTime) -> Self {
        self.expected_time = Some(expected_time);
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommandRecord {
    pub id: CommandId,
    pub accepted_at: SimTime,
    pub envelope: CommandEnvelope,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandReceipt {
    pub command_id: CommandId,
    pub accepted_at: SimTime,
    pub emitted_events: Vec<EventId>,
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
    pub systems: Vec<SystemContract>,
    pub commands: Vec<PluginActionDescriptor>,
    pub schema_types: Vec<String>,
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
        Ok(self
            .state
            .plugin_components
            .get(&component_key(owner, state, entity, component))
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
}

pub type SimulationSystemHandler =
    fn(&SimulationView<'_>, &SimEvent) -> Result<Vec<SystemDirective>, CanwuError>;

pub type PluginCommandHandler =
    fn(&SimulationView<'_>, &CommandContext, &Value) -> Result<Vec<SystemDirective>, CanwuError>;

pub trait SimulationPlugin {
    fn name(&self) -> &str;
    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError>;
}

#[derive(Clone, Default)]
pub struct PluginRegistry {
    descriptors: BTreeMap<String, PluginDescriptor>,
    active_plugins: BTreeSet<String>,
    systems: Vec<RegisteredSystem>,
    commands: BTreeMap<(String, String), RegisteredCommand>,
    state_owners: BTreeMap<StateKey, String>,
}

#[derive(Clone)]
struct RegisteredSystem {
    plugin: String,
    contract: SystemContract,
    handler: SimulationSystemHandler,
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
    pub fn register_schema(&mut self, schema: TypeSchema) -> Result<(), CanwuError> {
        validate_type_schema(&schema)?;
        let type_name = schema.type_name.clone();
        if let Some(existing) = self.schema.get(&type_name) {
            if existing != &schema {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPluginRegistration,
                    format!(
                        "schema type {type_name} is already registered with a different definition"
                    ),
                ));
            }
        } else {
            self.schema.register(schema);
        }
        let descriptor = self
            .registry
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
        register_state_owners(
            &mut self.registry.state_owners,
            &self.plugin,
            &contract.writes,
        )?;
        {
            let descriptor = self
                .registry
                .descriptors
                .entry(self.plugin.clone())
                .or_default();
            descriptor.name.clone_from(&self.plugin);
            descriptor.systems.push(contract.clone());
            descriptor
                .systems
                .sort_by(|left, right| (left.phase, &left.name).cmp(&(right.phase, &right.name)));
        }
        self.registry.systems.push(RegisteredSystem {
            plugin: self.plugin.clone(),
            contract,
            handler,
        });
        self.registry.systems.sort_by(|left, right| {
            (left.contract.phase, &left.plugin, &left.contract.name).cmp(&(
                right.contract.phase,
                &right.plugin,
                &right.contract.name,
            ))
        });
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
        register_state_owners(
            &mut self.registry.state_owners,
            &self.plugin,
            &descriptor.writes,
        )?;
        {
            let plugin_descriptor = self
                .registry
                .descriptors
                .entry(self.plugin.clone())
                .or_default();
            plugin_descriptor.name.clone_from(&self.plugin);
            plugin_descriptor.commands.push(descriptor.clone());
            plugin_descriptor
                .commands
                .sort_by(|left, right| left.name.cmp(&right.name));
        }
        self.registry.commands.insert(
            command_key,
            RegisteredCommand {
                descriptor,
                handler,
            },
        );
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

        let expected_descriptor = self.descriptors.get(plugin_name).cloned();
        let mut candidate_registry = self.clone();
        let mut candidate_schema = schema.clone();
        candidate_registry.descriptors.insert(
            plugin_name.to_owned(),
            PluginDescriptor {
                name: plugin_name.to_owned(),
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
            commands: BTreeMap::new(),
            state_owners: BTreeMap::new(),
        };
        let mut previous_plugin = None;
        for mut descriptor in descriptors {
            let plugin = descriptor.name.trim().to_owned();
            if plugin.is_empty()
                || descriptor.name != plugin
                || registry.descriptors.contains_key(&plugin)
                || previous_plugin
                    .as_ref()
                    .is_some_and(|previous| previous >= &plugin)
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    "snapshot contains an empty or duplicate plugin descriptor",
                ));
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
                register_state_owners(&mut registry.state_owners, &plugin, &contract.writes)
                    .map_err(|error| {
                        invalid_snapshot_error(format!(
                            "invalid plugin state ownership descriptor: {error}"
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
                register_state_owners(&mut registry.state_owners, &plugin, &action.writes)
                    .map_err(|error| {
                        invalid_snapshot_error(format!(
                            "invalid plugin state ownership descriptor: {error}"
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
        directive: SystemDirective,
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

#[derive(Clone)]
struct RuntimeState {
    initial_time: SimTime,
    now: SimTime,
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
    plugin_components: BTreeMap<PluginComponentKey, PluginComponentRecord>,
    rng: DeterministicRng,
    next_event_id: u64,
    next_command_id: u64,
    next_schedule_sequence: u64,
    next_correlation_id: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SimulationSnapshot {
    pub engine_version: String,
    pub snapshot_format_version: u32,
    pub initial_time: SimTime,
    pub now: SimTime,
    pub plugin_registration_closed: bool,
    pub world: WorldSnapshot,
    pub knowledge: KnowledgeSnapshot,
    pub events: Vec<SimEvent>,
    pub commands: Vec<CommandRecord>,
    pub plugin_components: Vec<PluginComponentRecord>,
    pub plugin_descriptors: Vec<PluginDescriptor>,
    pub schema: SchemaRegistry,
    scheduled: Vec<ScheduledRecord>,
    rng: DeterministicRng,
    next_event_id: u64,
    next_command_id: u64,
    next_schedule_sequence: u64,
    next_correlation_id: u64,
}

pub struct Simulation {
    state: RuntimeState,
    schema: SchemaRegistry,
    plugins: PluginRegistry,
}

impl Simulation {
    /// Creates a simulation after validating that scenario references are sound.
    pub fn new(seed: u64, scenario: Scenario) -> Result<Self, CanwuError> {
        validate_scenario(&scenario)?;
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
        Ok(Self {
            state: RuntimeState {
                initial_time: scenario.start_time,
                now: scenario.start_time,
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
                plugin_components: BTreeMap::new(),
                rng: DeterministicRng::from_seed(seed),
                next_event_id: 1,
                next_command_id: 1,
                next_schedule_sequence: 1,
                next_correlation_id: 1,
            },
            schema,
            plugins,
        })
    }

    pub fn demo(seed: u64) -> Result<(Self, DemoIds), CanwuError> {
        let (scenario, ids) = demo_scenario();
        Self::new(seed, scenario).map(|simulation| (simulation, ids))
    }

    pub fn replay(
        seed: u64,
        scenario: Scenario,
        commands: &[CommandRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        Self::replay_with_plugins(seed, scenario, &[], commands, final_time)
    }

    pub fn replay_with_plugins(
        seed: u64,
        scenario: Scenario,
        plugins: &[&dyn SimulationPlugin],
        commands: &[CommandRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        let mut simulation = Self::new(seed, scenario)?;
        for plugin in plugins {
            simulation.register_plugin(*plugin)?;
        }
        for record in commands {
            if record.accepted_at < simulation.time() || record.accepted_at > final_time {
                return Err(CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    "replay command timestamps must be ordered and no later than final time",
                ));
            }
            simulation.advance(record.accepted_at - simulation.time())?;
            let receipt = simulation.submit(record.envelope.clone())?;
            if receipt.command_id != record.id {
                return Err(CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    "replay command IDs did not match the journal",
                ));
            }
        }
        if final_time < simulation.time() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "replay final time cannot precede the last command",
            ));
        }
        simulation.advance(final_time - simulation.time())?;
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
        self.plugins.register(plugin, &mut self.schema)
    }

    #[must_use]
    pub const fn time(&self) -> SimTime {
        self.state.now
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
    pub const fn schema(&self) -> &SchemaRegistry {
        &self.schema
    }

    pub fn plugin_descriptors(&self) -> impl Iterator<Item = &PluginDescriptor> {
        self.plugins.descriptors()
    }

    pub fn submit(&mut self, envelope: CommandEnvelope) -> Result<CommandReceipt, CanwuError> {
        self.plugins.ensure_active()?;
        if let Some(expected_time) = envelope.expected_time
            && expected_time != self.state.now
        {
            return Err(CanwuError::new(
                ErrorCode::SimulationTimeConflict,
                format!(
                    "command expected time {expected_time}, but simulation is at {}",
                    self.state.now
                ),
            ));
        }

        let (command_id_value, next_command_id) =
            claim_counter(self.state.next_command_id, "command ID")?;
        let (correlation_id, next_correlation_id) =
            claim_counter(self.state.next_correlation_id, "correlation ID")?;
        let command_id = CommandId::new(command_id_value);
        let context = CommandContext {
            issuer: envelope.issuer.clone(),
            command_id,
            simulation_time: self.state.now,
            expected_time: envelope.expected_time,
        };
        let prepared = self.prepare_command(&envelope, &context)?;
        let transaction_start = self.state.clone();
        let event_start = self.state.events.len();
        self.state.next_command_id = next_command_id;
        self.state.next_correlation_id = next_correlation_id;

        if let Err(error) = self.apply_prepared(prepared, command_id, correlation_id) {
            self.state = transaction_start;
            return Err(error);
        }
        self.state.plugin_registration_closed = true;
        self.state.commands.push(CommandRecord {
            id: command_id,
            accepted_at: self.state.now,
            envelope,
        });

        Ok(CommandReceipt {
            command_id,
            accepted_at: self.state.now,
            emitted_events: self.state.events[event_start..]
                .iter()
                .map(|event| event.id)
                .collect(),
        })
    }

    pub fn advance(&mut self, duration: SimDuration) -> Result<Vec<SimEvent>, CanwuError> {
        self.plugins.ensure_active()?;
        if duration.is_negative() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "simulation time cannot advance by a negative duration",
            ));
        }
        let target = self.state.now + duration;
        self.advance_to(target)
    }

    pub fn step(&mut self) -> Result<Vec<SimEvent>, CanwuError> {
        self.plugins.ensure_active()?;
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
        self.plugins.ensure_active()?;
        if maximum.is_negative() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "advance_until maximum cannot be negative",
            ));
        }
        let target = self.state.now + maximum;
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

    #[must_use]
    pub fn snapshot(&self) -> SimulationSnapshot {
        SimulationSnapshot {
            engine_version: ENGINE_VERSION.to_owned(),
            snapshot_format_version: SNAPSHOT_FORMAT_VERSION,
            initial_time: self.state.initial_time,
            now: self.state.now,
            plugin_registration_closed: self.state.plugin_registration_closed,
            world: self.world(),
            knowledge: self.state.knowledge.clone(),
            events: self.state.events.clone(),
            commands: self.state.commands.clone(),
            plugin_components: self.state.plugin_components.values().cloned().collect(),
            plugin_descriptors: self.plugins.descriptors().cloned().collect(),
            schema: self.schema.clone(),
            scheduled: self
                .state
                .scheduler
                .iter()
                .map(|(key, action)| ScheduledRecord {
                    key: key.clone(),
                    action: action.clone(),
                })
                .collect(),
            rng: self.state.rng,
            next_event_id: self.state.next_event_id,
            next_command_id: self.state.next_command_id,
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
        if snapshot.snapshot_format_version != SNAPSHOT_FORMAT_VERSION {
            return Err(CanwuError::new(
                ErrorCode::UnsupportedSnapshotVersion,
                format!(
                    "snapshot format {} from engine {} is unsupported; this engine reads format {}",
                    snapshot.snapshot_format_version,
                    snapshot.engine_version,
                    SNAPSHOT_FORMAT_VERSION
                ),
            ));
        }
        validate_scenario(&Scenario {
            start_time: snapshot.now,
            world: snapshot.world.clone(),
            knowledge: snapshot.knowledge.clone(),
        })?;
        let plugins = PluginRegistry::from_descriptors(snapshot.plugin_descriptors.clone())?;
        validate_snapshot(&snapshot, &plugins)?;
        Ok(Self {
            state: RuntimeState {
                initial_time: snapshot.initial_time,
                now: snapshot.now,
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
                rng: snapshot.rng,
                next_event_id: snapshot.next_event_id,
                next_command_id: snapshot.next_command_id,
                next_schedule_sequence: snapshot.next_schedule_sequence,
                next_correlation_id: snapshot.next_correlation_id,
            },
            schema: snapshot.schema,
            plugins,
        })
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
        simulation.plugins.ensure_active()?;
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
                let Issuer::Actor(actor) = envelope.issuer else {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "move commands require an actor issuer",
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
                Ok(PreparedCommand::MoveArmy {
                    army: *army,
                    actor,
                    from: army_state.location,
                    destination: *destination,
                    arrival_at: self.state.now + SimDuration::minutes(route.travel_minutes),
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
        }
        self.state.now = target;
        self.state.plugin_registration_closed = true;
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
                let directives = vec![directive];
                validate_directives(
                    &plugin,
                    &allowed_writes,
                    &self.plugins.state_owners,
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
            let jitter_minutes = i64::try_from(self.state.rng.range(12 * 60))
                .expect("report jitter is bounded to a small integer");
            let arrives_at =
                self.state.now + SimDuration::hours(36) + SimDuration::minutes(jitter_minutes);
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
                    self.schedule_at(
                        self.state.now + after,
                        ScheduledAction::PluginDirective {
                            plugin: plugin.to_owned(),
                            directive: *directive,
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
        }
    }
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

fn validate_directives(
    plugin: &str,
    allowed_writes: &[StateKey],
    state_owners: &BTreeMap<StateKey, String>,
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
                if after.is_negative() {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidDuration,
                        "plugin systems cannot schedule work in the past",
                    ));
                }
                validate_directives(
                    plugin,
                    allowed_writes,
                    state_owners,
                    entity_exists,
                    std::slice::from_ref(directive),
                )?;
            }
            SystemDirective::Emit { .. } => {}
        }
    }
    Ok(())
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

fn validate_snapshot(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
) -> Result<(), CanwuError> {
    if snapshot.engine_version.trim().is_empty() {
        return invalid_snapshot("snapshot engine version cannot be empty");
    }
    if snapshot.initial_time > snapshot.now {
        return invalid_snapshot("snapshot initial time cannot follow its current time");
    }
    let has_execution_evidence = snapshot.now != snapshot.initial_time
        || !snapshot.commands.is_empty()
        || !snapshot.events.is_empty()
        || !snapshot.plugin_components.is_empty()
        || !snapshot.scheduled.is_empty()
        || snapshot.next_event_id != 1
        || snapshot.next_command_id != 1
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
    let mut command_ids = BTreeSet::new();
    let mut previous_command = None;
    for record in &snapshot.commands {
        if record.id.get() == 0 || !command_ids.insert(record.id) {
            return invalid_snapshot("command IDs must be unique and nonzero");
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
        if previous_command.is_some_and(|(time, id)| (record.accepted_at, record.id) <= (time, id))
        {
            return invalid_snapshot("command records are not in canonical order");
        }
        validate_snapshot_command(snapshot, plugins, &record.envelope)?;
        previous_command = Some((record.accepted_at, record.id));
    }

    let mut event_ids = BTreeSet::new();
    let mut previous_event = None;
    for event in &snapshot.events {
        if event.id.get() == 0 || event.correlation_id == 0 || !event_ids.insert(event.id) {
            return invalid_snapshot("event IDs must be unique and nonzero");
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
            .any(|entity| !snapshot_entity_exists(&snapshot.world, entity))
        {
            return invalid_snapshot("event references an unknown entity");
        }
        validate_event_kind(snapshot, plugins, event)?;
        previous_event = Some((event.timestamp, event.id));
    }
    for event in &snapshot.events {
        match &event.cause {
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

    let mut component_keys = BTreeSet::new();
    let mut previous_component = None;
    for record in &snapshot.plugin_components {
        if record.plugin.trim().is_empty()
            || record.component.trim().is_empty()
            || !plugins.descriptors.contains_key(&record.plugin)
            || !snapshot_entity_exists(&snapshot.world, &record.entity)
            || plugins.state_owners.get(&record.state) != Some(&record.plugin)
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
        .unwrap_or(0);
    for record in &snapshot.scheduled {
        if record.key.at < snapshot.now
            || record.key.sequence == 0
            || previous_schedule
                .as_ref()
                .is_some_and(|previous| previous >= &record.key)
            || !schedule_keys.insert(record.key.clone())
        {
            return invalid_snapshot("scheduled work has a past or duplicate key");
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

    validate_next_counter(
        snapshot.next_event_id,
        snapshot
            .events
            .iter()
            .map(|event| event.id.get())
            .max()
            .unwrap_or(0),
        "event",
    )?;
    validate_next_counter(
        snapshot.next_command_id,
        snapshot
            .commands
            .iter()
            .map(|command| command.id.get())
            .max()
            .unwrap_or(0),
        "command",
    )?;
    validate_next_counter(
        snapshot.next_schedule_sequence,
        max_schedule_sequence,
        "schedule sequence",
    )?;
    validate_next_counter(
        snapshot.next_correlation_id,
        max_correlation_id,
        "correlation",
    )?;
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
        Issuer::Actor(_) | Issuer::Debug | Issuer::System(_) => {}
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
            snapshot_entity_exists(&snapshot.world, entity)
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
                CauseRef::Command(_) | CauseRef::Event(_) | CauseRef::System(_) => {}
            }
            validate_directives(
                plugin,
                allowed_writes,
                &plugins.state_owners,
                &|entity| snapshot_entity_exists(&snapshot.world, entity),
                std::slice::from_ref(directive),
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

fn snapshot_entity_exists(world: &WorldSnapshot, entity: &EntityRef) -> bool {
    match entity {
        EntityRef::Army(id) => world.army(*id).is_some(),
        EntityRef::Government(id) => world.government(*id).is_some(),
        EntityRef::Person(id) => world.person(*id).is_some(),
        EntityRef::Route(id) => world.route(*id).is_some(),
        EntityRef::Territory(id) => world.territory(*id).is_some(),
        EntityRef::Organization(_) | EntityRef::Resource(_) => false,
    }
}

fn runtime_entity_exists(state: &RuntimeState, entity: &EntityRef) -> bool {
    match entity {
        EntityRef::Army(id) => state.armies.contains_key(id),
        EntityRef::Government(id) => state.governments.contains_key(id),
        EntityRef::Person(id) => state.people.contains_key(id),
        EntityRef::Route(id) => state.routes.contains_key(id),
        EntityRef::Territory(id) => state.territories.contains_key(id),
        EntityRef::Organization(_) | EntityRef::Resource(_) => false,
    }
}

fn invalid_snapshot_error(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidSnapshot, message)
}

fn invalid_snapshot<T>(message: impl Into<String>) -> Result<T, CanwuError> {
    Err(invalid_snapshot_error(message))
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
        },
        ids,
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unnecessary_wraps)]

    use super::*;

    struct AuthorityPlugin;

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

    impl SimulationPlugin for AuthorityPlugin {
        fn name(&self) -> &'static str {
            "authority-test"
        }

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
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
                after: SimDuration::minutes(-1),
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

    struct GhostPlugin;

    impl SimulationPlugin for GhostPlugin {
        fn name(&self) -> &'static str {
            "ghost-test"
        }

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

    fn move_order(ids: &DemoIds) -> CommandEnvelope {
        CommandEnvelope::new(
            Issuer::Actor(ids.commander),
            Command::MoveArmy {
                army: ids.army,
                destination: ids.eastern_territory,
            },
        )
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
    fn plugin_command_journal_replays_only_with_recorded_plugins() {
        let (scenario, ids) = demo_scenario();
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
