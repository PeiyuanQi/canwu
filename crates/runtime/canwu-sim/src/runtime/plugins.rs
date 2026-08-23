use super::knowledge::{KnowledgeLimitsV1, PluginKnowledgeSchema, validate_schema_set};
use super::records::DomainRecordSchema;
use super::{
    BoundaryPhase, BoundarySystemContract, BoundarySystemHandler, BoundaryWriteStage,
    CORE_STATE_NAMESPACE, CanwuError, CommandContext, EntityRef, ErrorCode, IngressClass,
    PluginIngressDescriptor, RandomStreamKey, ReservationRef, SchemaRegistry, SimDuration,
    SimEvent, SimulationView, StateKey, StateVisibility, SystemCadence, SystemContract, TypeSchema,
    boundary_write_stage, canonical_text, invalid_snapshot, invalid_snapshot_error,
    is_canonical_hash, is_domain_record_state, knowledge, records, validate_type_schema,
};
use canwu_event::EventAudience;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

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
    pub(super) fn validate(&self, value: &Value) -> Result<(), CanwuError> {
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
    /// Declarative visibility policies for emitted plugin event types.
    /// Unlisted event types are private to player-facing projections.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub event_audiences: BTreeMap<String, EventAudience>,
    pub systems: Vec<SystemContract>,
    #[serde(default)]
    pub boundary_systems: Vec<BoundarySystemContract>,
    pub commands: Vec<PluginActionDescriptor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ingress: Vec<PluginIngressDescriptor>,
    pub schema_types: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub record_schemas: Vec<DomainRecordSchema>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub knowledge_schemas: Vec<PluginKnowledgeSchema>,
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
pub(super) struct PluginComponentKey {
    pub(super) plugin: String,
    pub(super) state: StateKey,
    pub(super) entity: EntityRef,
    pub(super) component: String,
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
    /// Queues ingress for the issuing plugin. A zero delay is still admitted
    /// only at the next boundary cut.
    EnqueuePluginIngress {
        after: SimDuration,
        packet_type: String,
        priority: i32,
        payload: Value,
        affected: Vec<EntityRef>,
    },
}

/// Compatibility-only synchronous event reactor.
///
/// The handler runs inside the event's current transaction. Emitting another
/// event from its directives re-enters the same reactor graph and is bounded
/// by [`super::MAX_SYNCHRONOUS_REACTION_DEPTH`]. New mechanics should use a phased
/// [`BoundarySystemHandler`] instead.
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
    pub(super) descriptors: BTreeMap<String, PluginDescriptor>,
    pub(super) active_plugins: BTreeSet<String>,
    pub(super) systems: Vec<RegisteredSystem>,
    pub(super) boundary_systems: Vec<RegisteredBoundarySystem>,
    pub(super) commands: BTreeMap<(String, String), RegisteredCommand>,
    pub(super) ingress: BTreeMap<(String, String), PluginIngressDescriptor>,
    pub(super) state_owners: BTreeMap<StateKey, String>,
    pub(super) immediate_write_states: BTreeMap<StateKey, String>,
    pub(super) boundary_writers: BTreeMap<(BoundaryWriteStage, StateKey), (String, String)>,
    pub(super) reservation_offerers: BTreeMap<StateKey, (String, String)>,
    pub(super) random_stream_owners: BTreeMap<RandomStreamKey, (String, String)>,
    pub(super) record_schemas: records::DomainRecordSchemas,
    pub(super) knowledge_schemas: knowledge::KnowledgeSchemas,
    pub(super) knowledge_kind_owners: knowledge::KnowledgeKindOwners,
}

#[derive(Clone)]
pub(super) struct RegisteredSystem {
    pub(super) plugin: String,
    pub(super) contract: SystemContract,
    pub(super) handler: SimulationSystemHandler,
}

#[derive(Clone)]
pub(super) struct RegisteredBoundarySystem {
    pub(super) plugin: String,
    pub(super) contract: BoundarySystemContract,
    pub(super) handler: BoundarySystemHandler,
}

#[derive(Clone)]
pub(super) struct RegisteredCommand {
    pub(super) descriptor: PluginActionDescriptor,
    pub(super) handler: PluginCommandHandler,
}

pub struct PluginRegistrar<'a> {
    pub(super) plugin: String,
    pub(super) registry: &'a mut PluginRegistry,
    pub(super) schema: &'a mut SchemaRegistry,
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

    pub fn register_knowledge_schema(
        &mut self,
        mut schema: PluginKnowledgeSchema,
    ) -> Result<(), CanwuError> {
        schema.canonicalize();
        schema.validate().map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!("invalid knowledge schema: {error}"),
            )
        })?;
        let current_count = self
            .registry
            .descriptors
            .get(&self.plugin)
            .map_or(0, |descriptor| descriptor.knowledge_schemas.len());
        if current_count >= KnowledgeLimitsV1::CURRENT.schemas_per_plugin {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "plugin knowledge schema limit exceeded",
            ));
        }
        if self
            .registry
            .descriptors
            .get(&self.plugin)
            .is_some_and(|descriptor| {
                descriptor
                    .knowledge_schemas
                    .iter()
                    .any(|candidate| candidate.id == schema.id)
            })
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!(
                    "plugin {} registered knowledge schema {:?} twice",
                    self.plugin, schema.id
                ),
            ));
        }
        if let Some(owner) = self.registry.knowledge_kind_owners.get(&schema.id.kind)
            && owner != &self.plugin
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!(
                    "knowledge kind {:?} is already owned by plugin {owner}",
                    schema.id.kind
                ),
            ));
        }
        if let Some((owner, existing)) = self.registry.knowledge_schemas.get(&schema.id) {
            if owner != &self.plugin {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPluginRegistration,
                    format!(
                        "knowledge schema {:?} is already owned by plugin {owner}",
                        schema.id
                    ),
                ));
            }
            if existing != &schema {
                return Err(CanwuError::new(
                    ErrorCode::PluginManifestMismatch,
                    format!(
                        "plugin {} changed the stored knowledge schema {:?}",
                        self.plugin, schema.id
                    ),
                ));
            }
        }
        if schema.writable
            && self
                .registry
                .knowledge_schemas
                .values()
                .any(|(owner, existing)| {
                    owner == &self.plugin
                        && existing.id != schema.id
                        && existing.id.kind == schema.id.kind
                        && existing.writable
                })
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!(
                    "knowledge kind {:?} already has a writable version",
                    schema.id.kind
                ),
            ));
        }
        let mut candidate = self.registry.clone();
        candidate
            .knowledge_kind_owners
            .entry(schema.id.kind.clone())
            .or_insert_with(|| self.plugin.clone());
        candidate
            .knowledge_schemas
            .insert(schema.id.clone(), (self.plugin.clone(), schema.clone()));
        let descriptor = candidate
            .descriptors
            .entry(self.plugin.clone())
            .or_default();
        descriptor.name.clone_from(&self.plugin);
        descriptor.knowledge_schemas.push(schema);
        descriptor
            .knowledge_schemas
            .sort_by(|left, right| left.id.cmp(&right.id));
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

    /// Declares the player-facing audience for one emitted plugin event type.
    ///
    /// This policy is persisted in the plugin descriptor and is independent
    /// from the plugin system subscription graph. Event types without a
    /// declaration remain private to player-facing projections.
    pub fn register_event_audience(
        &mut self,
        event_type: impl Into<String>,
        audience: EventAudience,
    ) -> Result<(), CanwuError> {
        let event_type = event_type.into();
        validate_event_audience_name(&event_type)?;
        validate_event_audience(&audience)?;
        let mut candidate = self.registry.clone();
        let descriptor = candidate
            .descriptors
            .entry(self.plugin.clone())
            .or_default();
        descriptor.name.clone_from(&self.plugin);
        if descriptor
            .event_audiences
            .insert(event_type.clone(), audience)
            .is_some()
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!(
                    "plugin {} already declared event audience for {event_type}",
                    self.plugin
                ),
            ));
        }
        *self.registry = candidate;
        Ok(())
    }

    pub fn register_system(
        &mut self,
        mut contract: SystemContract,
        handler: SimulationSystemHandler,
    ) -> Result<(), CanwuError> {
        // This registration path is retained for the movement slice and
        // legacy plugins. New mechanics should register a phased boundary
        // system so their writes are staged and committed atomically.
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
        validate_knowledge_write_grants(&self.plugin, &contract, &self.registry.knowledge_schemas)?;
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
        validate_schema_set(
            &candidate_registry.knowledge_schemas,
            &candidate_registry.knowledge_kind_owners,
        )
        .map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!("invalid knowledge schema set: {error}"),
            )
        })?;
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

    pub(super) fn event_audience(&self, plugin: &str, event_type: &str) -> EventAudience {
        self.descriptors
            .get(plugin)
            .and_then(|descriptor| descriptor.event_audiences.get(event_type))
            .cloned()
            .unwrap_or_default()
    }

    pub(super) fn from_descriptors(descriptors: Vec<PluginDescriptor>) -> Result<Self, CanwuError> {
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
            knowledge_schemas: BTreeMap::new(),
            knowledge_kind_owners: BTreeMap::new(),
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
            if descriptor.knowledge_schemas.len() > KnowledgeLimitsV1::CURRENT.schemas_per_plugin
                || descriptor
                    .knowledge_schemas
                    .windows(2)
                    .any(|pair| pair[0].id >= pair[1].id)
            {
                return invalid_snapshot(
                    "plugin knowledge schemas are not in canonical order or exceed their limit",
                );
            }
            for schema in &mut descriptor.knowledge_schemas {
                let original = schema.clone();
                schema.canonicalize();
                schema.validate().map_err(|error| {
                    invalid_snapshot_error(format!("invalid knowledge schema: {error}"))
                })?;
                if *schema != original {
                    return invalid_snapshot(
                        "plugin knowledge-schema declarations are not in canonical order",
                    );
                }
                if let Some(owner) = registry.knowledge_kind_owners.get(&schema.id.kind) {
                    if owner != &plugin {
                        return invalid_snapshot(format!(
                            "knowledge kind {:?} is owned by both {owner} and {plugin}",
                            schema.id.kind
                        ));
                    }
                } else {
                    registry
                        .knowledge_kind_owners
                        .insert(schema.id.kind.clone(), plugin.clone());
                }
                if registry
                    .knowledge_schemas
                    .insert(schema.id.clone(), (plugin.clone(), schema.clone()))
                    .is_some()
                {
                    return invalid_snapshot("knowledge schema ID is duplicated");
                }
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
                validate_knowledge_write_grants(&plugin, contract, &registry.knowledge_schemas)
                    .map_err(|error| {
                        invalid_snapshot_error(format!(
                            "invalid boundary knowledge writer descriptor: {error}"
                        ))
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
            for (event_type, audience) in &descriptor.event_audiences {
                validate_event_audience_name(event_type).map_err(|error| {
                    invalid_snapshot_error(format!("invalid plugin event audience: {error}"))
                })?;
                validate_event_audience(audience).map_err(|error| {
                    invalid_snapshot_error(format!("invalid plugin event audience: {error}"))
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
        validate_schema_set(&registry.knowledge_schemas, &registry.knowledge_kind_owners).map_err(
            |error| invalid_snapshot_error(format!("invalid knowledge schema set: {error}")),
        )?;
        Ok(registry)
    }

    pub(super) fn ensure_active(&self) -> Result<(), CanwuError> {
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

pub(super) fn validate_state_keys(keys: &mut Vec<StateKey>) -> Result<(), CanwuError> {
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
    for grant in &mut contract.knowledge_writes {
        grant.visibilities.sort();
        grant.visibilities.dedup();
        if grant.visibilities.is_empty() {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "knowledge write grants require at least one visibility",
            ));
        }
    }
    contract
        .knowledge_writes
        .sort_by(|left, right| left.schema.cmp(&right.schema));
    if contract
        .knowledge_writes
        .windows(2)
        .any(|pair| pair[0].schema >= pair[1].schema)
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "knowledge write grants must name unique schemas in canonical order",
        ));
    }
    if !contract.knowledge_writes.is_empty()
        && !matches!(
            contract.phase,
            BoundaryPhase::PerceptionAndAttentionRefresh
                | BoundaryPhase::PerspectiveAndReportMaterialization
        )
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "knowledge publication is available only in phases 4 and 13",
        ));
    }
    if contract.plugin_ingress_targets.iter().any(|target| {
        target.target_plugin.trim().is_empty()
            || target.target_plugin != target.target_plugin.trim()
            || target.packet_type.trim().is_empty()
            || target.packet_type != target.packet_type.trim()
    }) {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "cross-plugin ingress targets require canonical plugin and packet names",
        ));
    }
    contract.plugin_ingress_targets.sort();
    if contract
        .plugin_ingress_targets
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "cross-plugin ingress targets must be unique",
        ));
    }

    let may_propose_changes = matches!(
        contract.phase,
        BoundaryPhase::DomainDeltaProposal
            | BoundaryPhase::HistoricalCandidateEvaluation
            | BoundaryPhase::StrategicAggregation
            | BoundaryPhase::PerspectiveAndReportMaterialization
    );
    if (!contract.writes.is_empty()
        || !contract.emits.is_empty()
        || !contract.plugin_ingress_targets.is_empty())
        && !may_propose_changes
    {
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

fn validate_knowledge_write_grants(
    plugin: &str,
    contract: &BoundarySystemContract,
    schemas: &super::knowledge::KnowledgeSchemas,
) -> Result<(), CanwuError> {
    for grant in &contract.knowledge_writes {
        let Some((owner, schema)) = schemas.get(&grant.schema) else {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!(
                    "boundary system {plugin}.{} names an unregistered knowledge schema",
                    contract.name
                ),
            ));
        };
        if owner != plugin || !schema.writable {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!(
                    "boundary system {plugin}.{} cannot write a foreign or read-only knowledge schema",
                    contract.name
                ),
            ));
        }
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

fn validate_event_audience_name(event_type: &str) -> Result<(), CanwuError> {
    if !canonical_text(event_type) {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "plugin event audience names must be non-empty and canonical",
        ));
    }
    Ok(())
}

fn validate_event_audience(audience: &EventAudience) -> Result<(), CanwuError> {
    match audience {
        EventAudience::Actor(actor) if actor.get() == 0 => {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "plugin event audience actors must use positive actor IDs",
            ));
        }
        EventAudience::Actors(actors) => {
            if actors.is_empty() || actors.iter().any(|actor| actor.get() == 0) {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPluginRegistration,
                    "plugin event audience actor lists must contain positive actor IDs",
                ));
            }
            if actors.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPluginRegistration,
                    "plugin event audience actor lists must be sorted and unique",
                ));
            }
        }
        _ => {}
    }
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

#[cfg(test)]
mod tests {
    use super::super::{KnowledgeSubjectSchema, KnowledgeSubjectTargetKind};
    use super::*;
    use canwu_core::{CoreEntityKind, KnowledgeRecordKind, KnowledgeSchemaId};

    struct KnowledgeSchemaPlugin {
        name: &'static str,
        schemas: Vec<PluginKnowledgeSchema>,
    }

    impl SimulationPlugin for KnowledgeSchemaPlugin {
        fn name(&self) -> &str {
            self.name
        }

        fn version(&self) -> &'static str {
            "1"
        }

        fn semantic_hash(&self) -> &'static str {
            "0000000000000000000000000000000000000000000000000000000000000001"
        }

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            for schema in &self.schemas {
                registrar.register_knowledge_schema(schema.clone())?;
            }
            Ok(())
        }
    }

    fn knowledge_kind() -> KnowledgeRecordKind {
        KnowledgeRecordKind::new("fixture.knowledge", "assessment")
    }

    fn knowledge_schema(version: u32, writable: bool) -> PluginKnowledgeSchema {
        PluginKnowledgeSchema {
            id: KnowledgeSchemaId::new(knowledge_kind(), version),
            schema_hash: format!("{version:064x}"),
            writable,
            payload_schema: PayloadSchema::Any,
            subjects: vec![],
        }
    }

    #[test]
    fn duplicate_schema_and_writable_conflicts_roll_back_registration() {
        let duplicate = KnowledgeSchemaPlugin {
            name: "duplicate-knowledge",
            schemas: vec![knowledge_schema(1, true), knowledge_schema(1, true)],
        };
        let mut registry = PluginRegistry::default();
        let mut types = SchemaRegistry::default();
        assert!(registry.register(&duplicate, &mut types).is_err());
        assert!(registry.descriptors.is_empty());
        assert!(registry.knowledge_schemas.is_empty());
        assert!(registry.knowledge_kind_owners.is_empty());

        let two_writable = KnowledgeSchemaPlugin {
            name: "two-writable-knowledge",
            schemas: vec![knowledge_schema(1, true), knowledge_schema(2, true)],
        };
        assert!(registry.register(&two_writable, &mut types).is_err());
        assert!(registry.descriptors.is_empty());
        assert!(registry.knowledge_schemas.is_empty());

        let first_owner = KnowledgeSchemaPlugin {
            name: "first-knowledge-owner",
            schemas: vec![knowledge_schema(1, true)],
        };
        registry
            .register(&first_owner, &mut types)
            .expect("the first kind owner should register");
        let before = registry.clone();
        let second_owner = KnowledgeSchemaPlugin {
            name: "second-knowledge-owner",
            schemas: vec![knowledge_schema(2, true)],
        };
        assert!(registry.register(&second_owner, &mut types).is_err());
        assert_eq!(registry.descriptors, before.descriptors);
        assert_eq!(registry.knowledge_schemas, before.knowledge_schemas);
        assert_eq!(registry.knowledge_kind_owners, before.knowledge_kind_owners);
    }

    #[test]
    fn schema_hash_mismatch_blocks_exact_rehydration() {
        let plugin = KnowledgeSchemaPlugin {
            name: "rehydrated-knowledge",
            schemas: vec![knowledge_schema(1, true)],
        };
        let mut registry = PluginRegistry::default();
        let mut types = SchemaRegistry::default();
        registry
            .register(&plugin, &mut types)
            .expect("fixture plugin should register");
        let mut descriptors = registry.descriptors().cloned().collect::<Vec<_>>();
        descriptors[0].knowledge_schemas[0].schema_hash =
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_owned();
        let mut rehydrated = PluginRegistry::from_descriptors(descriptors)
            .expect("the altered descriptor remains structurally valid");
        let error = rehydrated
            .register(&plugin, &mut SchemaRegistry::default())
            .expect_err("exact rehydration must compare the persisted schema hash");
        assert_eq!(error.code, ErrorCode::PluginManifestMismatch);
    }

    #[test]
    fn schema_limit_accepts_boundary_and_rejects_plus_one_atomically() {
        let boundary = KnowledgeSchemaPlugin {
            name: "knowledge-limit-boundary",
            schemas: (1..=KnowledgeLimitsV1::CURRENT.schemas_per_plugin)
                .map(|version| {
                    knowledge_schema(
                        u32::try_from(version).expect("schema limit fits u32"),
                        version == 1,
                    )
                })
                .collect(),
        };
        let mut registry = PluginRegistry::default();
        registry
            .register(&boundary, &mut SchemaRegistry::default())
            .expect("the exact schema limit should be admitted");
        assert_eq!(
            registry.knowledge_schemas.len(),
            KnowledgeLimitsV1::CURRENT.schemas_per_plugin
        );

        let overflow = KnowledgeSchemaPlugin {
            name: "knowledge-limit-overflow",
            schemas: (1..=KnowledgeLimitsV1::CURRENT.schemas_per_plugin + 1)
                .map(|version| {
                    knowledge_schema(
                        u32::try_from(version).expect("schema limit fits u32"),
                        version == 1,
                    )
                })
                .collect(),
        };
        let mut rejected = PluginRegistry::default();
        let error = rejected
            .register(&overflow, &mut SchemaRegistry::default())
            .expect_err("schema limit plus one must reject the whole plugin");
        assert_eq!(error.code, ErrorCode::InvalidPluginRegistration);
        assert!(rejected.descriptors.is_empty());
        assert!(rejected.knowledge_schemas.is_empty());
    }

    #[test]
    fn knowledge_schema_registration_canonicalizes_roles_and_targets() {
        let mut schema = knowledge_schema(1, true);
        schema.subjects = vec![
            KnowledgeSubjectSchema {
                role: "zeta".to_owned(),
                targets: vec![
                    KnowledgeSubjectTargetKind::AnyEntity,
                    KnowledgeSubjectTargetKind::Core(CoreEntityKind::Person),
                    KnowledgeSubjectTargetKind::AnyEntity,
                ],
                required: false,
                multiple: true,
            },
            KnowledgeSubjectSchema {
                role: "alpha".to_owned(),
                targets: vec![KnowledgeSubjectTargetKind::Event],
                required: true,
                multiple: false,
            },
        ];
        let plugin = KnowledgeSchemaPlugin {
            name: "canonical-knowledge",
            schemas: vec![schema],
        };
        let mut registry = PluginRegistry::default();
        registry
            .register(&plugin, &mut SchemaRegistry::default())
            .expect("registrar should canonicalize declarations transactionally");
        let stored = &registry
            .descriptors
            .get(plugin.name)
            .expect("descriptor exists")
            .knowledge_schemas[0];
        assert_eq!(stored.subjects[0].role, "alpha");
        assert_eq!(stored.subjects[1].role, "zeta");
        assert_eq!(stored.subjects[1].targets.len(), 2);
        assert!(stored.validate().is_ok());
    }

    #[test]
    fn invalid_schema_version_and_hash_roll_back_registration() {
        let mut version_zero = knowledge_schema(0, true);
        version_zero.schema_hash =
            "0000000000000000000000000000000000000000000000000000000000000000".to_owned();
        let invalid_version = KnowledgeSchemaPlugin {
            name: "invalid-knowledge-version",
            schemas: vec![version_zero],
        };
        let mut registry = PluginRegistry::default();
        let mut types = SchemaRegistry::default();
        assert!(registry.register(&invalid_version, &mut types).is_err());
        assert!(registry.descriptors.is_empty());
        assert!(registry.knowledge_schemas.is_empty());

        let mut bad_hash = knowledge_schema(1, true);
        bad_hash.schema_hash = "not-a-canonical-hash".to_owned();
        let invalid_hash = KnowledgeSchemaPlugin {
            name: "invalid-knowledge-hash",
            schemas: vec![bad_hash],
        };
        assert!(registry.register(&invalid_hash, &mut types).is_err());
        assert!(registry.descriptors.is_empty());
        assert!(registry.knowledge_schemas.is_empty());
    }

    #[test]
    fn knowledge_write_grants_reject_invalid_phase_and_foreign_owner() {
        #[allow(clippy::unnecessary_wraps)]
        fn no_op_boundary(
            _view: &crate::SimulationView<'_>,
            _context: &crate::BoundaryContext,
        ) -> Result<crate::BoundaryProposal, CanwuError> {
            Ok(crate::BoundaryProposal::default())
        }

        let owner = KnowledgeSchemaPlugin {
            name: "knowledge-grant-owner",
            schemas: vec![knowledge_schema(1, true)],
        };
        let foreign = KnowledgeSchemaPlugin {
            name: "knowledge-grant-foreign",
            schemas: vec![PluginKnowledgeSchema {
                id: KnowledgeSchemaId::new(
                    KnowledgeRecordKind::new("fixture.foreign", "assessment"),
                    1,
                ),
                schema_hash: "f000000000000000000000000000000000000000000000000000000000000000"
                    .to_owned(),
                writable: true,
                payload_schema: PayloadSchema::Any,
                subjects: Vec::new(),
            }],
        };
        let mut registry = PluginRegistry::default();
        let mut types = SchemaRegistry::default();
        registry
            .register(&owner, &mut types)
            .expect("knowledge owner should register");
        registry
            .register(&foreign, &mut types)
            .expect("foreign fixture should register");
        let before = registry.clone();

        let mut phase7 = BoundarySystemContract::new(
            "invalid-phase7-publication",
            crate::BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        phase7.knowledge_writes = vec![crate::KnowledgeWriteGrant {
            schema: knowledge_schema(1, true).id,
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        let mut owner_registry = registry.clone();
        let mut owner_types = types.clone();
        let mut registrar = PluginRegistrar {
            plugin: owner.name.to_owned(),
            registry: &mut owner_registry,
            schema: &mut owner_types,
        };
        let error = registrar
            .register_boundary_system(phase7, no_op_boundary)
            .expect_err("phase 7 must reject knowledge publication grants");
        assert_eq!(error.code, ErrorCode::InvalidPluginRegistration);
        assert_eq!(owner_registry.descriptors, before.descriptors);

        let mut foreign_grant = BoundarySystemContract::new(
            "foreign-knowledge-grant",
            crate::BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        foreign_grant.knowledge_writes = vec![crate::KnowledgeWriteGrant {
            schema: knowledge_schema(1, true).id,
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        let mut foreign_registry = registry;
        let mut registrar = PluginRegistrar {
            plugin: foreign.name.to_owned(),
            registry: &mut foreign_registry,
            schema: &mut types,
        };
        let error = registrar
            .register_boundary_system(foreign_grant, no_op_boundary)
            .expect_err("a plugin cannot claim another plugin's writable schema");
        assert_eq!(error.code, ErrorCode::InvalidPluginRegistration);
        assert_eq!(foreign_registry.descriptors, before.descriptors);
    }
}
