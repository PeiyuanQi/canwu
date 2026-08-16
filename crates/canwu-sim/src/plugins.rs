use super::{
    BoundaryPhase, BoundarySystemContract, BoundarySystemHandler, BoundaryWriteStage,
    CORE_STATE_NAMESPACE, CanwuError, ErrorCode, IngressClass, PayloadSchema,
    PluginActionDescriptor, PluginCommandHandler, PluginDescriptor, PluginIngressDescriptor,
    PluginRegistrar, PluginRegistry, RandomStreamKey, RegisteredBoundarySystem, RegisteredCommand,
    RegisteredSystem, ReservationRef, SchemaRegistry, SimulationPlugin, SimulationSystemHandler,
    StateKey, StateVisibility, SystemCadence, SystemContract, TypeSchema, boundary_write_stage,
    invalid_snapshot, invalid_snapshot_error, is_canonical_hash, is_domain_record_state,
    validate_type_schema,
};
use crate::records::DomainRecordSchema;
use std::collections::{BTreeMap, BTreeSet};

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
