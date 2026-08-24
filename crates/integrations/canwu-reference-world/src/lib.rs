//! Replaceable reference world model built only on Canwu's supported public API.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

use canwu_api::{
    ArmyId, BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal,
    BoundarySystemContract, Canwu, CanwuError, Command, CommandContext, DecisionOrigin,
    DomainRecord, DomainRecordClass, DomainRecordDraft, DomainRecordLifecycle,
    DomainRecordMutation, DomainRecordSchema, DomainRecordType, DomainValueKindClass, EntityRef,
    ErrorCode, GovernmentId, IngressClass, IngressPayload, Issuer, LetterId, PayloadSchema,
    PersonId, PluginActionDescriptor, PluginIngressDescriptor, PluginRegistrar, ResourceId,
    RouteId, RoutingConnection, RoutingConnectionRef, RoutingEndpoint, RoutingEndpointKind,
    RoutingError, RoutingNetwork, RoutingNodeRef, Scenario, SimDuration, SimTime, SimulationPlugin,
    SimulationView, StateKey, StateVisibility, SystemCadence, SystemDirective, TerritoryId,
    TransferMode, TraversalModel, TypedDomainRecordRef,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PLUGIN_NAME: &str = "canwu.reference-world";
pub const ORDER_MOVEMENT_COMMAND: &str = "order_movement_v1";
const MOVEMENT_INGRESS: &str = "movement_transition_v1";
const MOVEMENT_SYSTEM: &str = "apply_movement_transition_v1";
const MOVEMENT_RESERVATION_COMPONENT: &str = "pending";
const PLUGIN_VERSION: &str = "1";
const SEMANTIC_HASH: &str = "30f403c9a79e8653deea4ee7547fb24e1cb18ed795068bd049b7fb0ad7833326";
const WORLD_RECORD_ID: &str = "primary";

pub struct ReferenceWorldState;

impl DomainRecordType for ReferenceWorldState {
    type Payload = WorldSnapshot;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = PLUGIN_NAME;
    const NAME: &'static str = "world_state";
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ReferenceWorldPlugin;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReferenceWorldIds {
    pub commander: PersonId,
    pub observer: PersonId,
    pub government: GovernmentId,
    pub army: ArmyId,
    pub western_territory: TerritoryId,
    pub central_territory: TerritoryId,
    pub eastern_territory: TerritoryId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MovementCommand {
    pub subject: EntityRef,
    pub destination: TerritoryId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cargo: Vec<LetterId>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum MovementStage {
    Started,
    Arrived,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct MovementTransition {
    stage: MovementStage,
    subject: EntityRef,
    from: TerritoryId,
    destination: TerritoryId,
    arrives_at: SimTime,
    cargo: Vec<LetterId>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct MapPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Person {
    pub id: PersonId,
    pub name: String,
    pub government: GovernmentId,
    pub current_location: TerritoryId,
    pub roles: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transit: Option<PersonTransitState>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PersonTransitState {
    pub from: TerritoryId,
    pub to: TerritoryId,
    pub departed_at: SimTime,
    pub arrives_at: SimTime,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LetterStatus {
    HeldByPerson,
    InTransit,
    HeldAtLocation,
    Delivered,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LetterCargo {
    pub id: LetterId,
    pub sender: PersonId,
    pub recipient: PersonId,
    pub body: String,
    pub status: LetterStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier: Option<PersonId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<TerritoryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<SimTime>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Government {
    pub id: GovernmentId,
    pub name: String,
    pub capital: TerritoryId,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Territory {
    pub id: TerritoryId,
    pub name: String,
    pub controller: GovernmentId,
    pub position: MapPoint,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Route {
    pub id: RouteId,
    pub name: String,
    pub from: TerritoryId,
    pub to: TerritoryId,
    pub travel_minutes: i64,
    pub terrain: String,
}

impl Route {
    #[must_use]
    pub fn connects(&self, first: TerritoryId, second: TerritoryId) -> bool {
        (self.from == first && self.to == second) || (self.from == second && self.to == first)
    }

    #[must_use]
    pub fn other_end(&self, territory: TerritoryId) -> Option<TerritoryId> {
        if self.from == territory {
            Some(self.to)
        } else if self.to == territory {
            Some(self.from)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransitState {
    pub from: TerritoryId,
    pub to: TerritoryId,
    pub departed_at: SimTime,
    pub arrives_at: SimTime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Army {
    pub id: ArmyId,
    pub name: String,
    pub government: GovernmentId,
    pub commander: PersonId,
    pub location: TerritoryId,
    pub strength: u32,
    pub morale: u16,
    pub transit: Option<TransitState>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct WorldSnapshot {
    pub people: Vec<Person>,
    pub governments: Vec<Government>,
    pub territories: Vec<Territory>,
    pub routes: Vec<Route>,
    pub armies: Vec<Army>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub letters: Vec<LetterCargo>,
}

impl WorldSnapshot {
    #[must_use]
    pub fn person(&self, id: PersonId) -> Option<&Person> {
        self.people.iter().find(|person| person.id == id)
    }

    #[must_use]
    pub fn government(&self, id: GovernmentId) -> Option<&Government> {
        self.governments
            .iter()
            .find(|government| government.id == id)
    }

    #[must_use]
    pub fn territory(&self, id: TerritoryId) -> Option<&Territory> {
        self.territories.iter().find(|territory| territory.id == id)
    }

    #[must_use]
    pub fn route(&self, id: RouteId) -> Option<&Route> {
        self.routes.iter().find(|route| route.id == id)
    }

    #[must_use]
    pub fn army(&self, id: ArmyId) -> Option<&Army> {
        self.armies.iter().find(|army| army.id == id)
    }

    #[must_use]
    pub fn letter(&self, id: LetterId) -> Option<&LetterCargo> {
        self.letters.iter().find(|letter| letter.id == id)
    }

    #[must_use]
    pub fn route_between(&self, from: TerritoryId, to: TerritoryId) -> Option<&Route> {
        self.routes.iter().find(|route| route.connects(from, to))
    }

    pub fn adjacent_territories(
        &self,
        territory: TerritoryId,
    ) -> impl Iterator<Item = TerritoryId> + '_ {
        self.routes
            .iter()
            .filter_map(move |route| route.other_end(territory))
    }

    #[must_use]
    pub fn travel_duration(&self, from: TerritoryId, to: TerritoryId) -> Option<SimDuration> {
        self.route_between(from, to)
            .map(|route| SimDuration::minutes(route.travel_minutes))
    }

    #[must_use]
    pub fn distance(&self, from: TerritoryId, to: TerritoryId) -> Option<f32> {
        let first = self.territory(from)?.position;
        let second = self.territory(to)?.position;
        Some((second.x - first.x).hypot(second.y - first.y))
    }

    pub fn territories_within(
        &self,
        center: TerritoryId,
        radius: f32,
    ) -> impl Iterator<Item = &Territory> {
        let center_position = self.territory(center).map(|territory| territory.position);
        self.territories.iter().filter(move |territory| {
            center_position.is_some_and(|point| {
                (territory.position.x - point.x).hypot(territory.position.y - point.y) <= radius
            })
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorldDiff {
    pub changed_armies: Vec<ArmyId>,
    pub changed_people: Vec<PersonId>,
    pub changed_territories: Vec<TerritoryId>,
    pub changed_letters: Vec<LetterId>,
}

impl WorldDiff {
    #[must_use]
    pub fn between(before: &WorldSnapshot, after: &WorldSnapshot) -> Self {
        Self {
            changed_armies: after
                .armies
                .iter()
                .filter(|army| before.army(army.id) != Some(*army))
                .map(|army| army.id)
                .collect(),
            changed_people: after
                .people
                .iter()
                .filter(|person| before.person(person.id) != Some(*person))
                .map(|person| person.id)
                .collect(),
            changed_territories: after
                .territories
                .iter()
                .filter(|territory| before.territory(territory.id) != Some(*territory))
                .map(|territory| territory.id)
                .collect(),
            changed_letters: after
                .letters
                .iter()
                .filter(|letter| before.letter(letter.id) != Some(*letter))
                .map(|letter| letter.id)
                .collect(),
        }
    }
}

pub fn planning_snapshot_from_world(
    world: &WorldSnapshot,
    observer: impl Into<String>,
    observed_at: SimTime,
    knowledge_cut: impl Into<String>,
    topology_version: impl Into<String>,
    mode: TransferMode,
) -> Result<canwu_api::PlanningSnapshot, RoutingError> {
    let topology_version = topology_version.into();
    let endpoints = world
        .territories
        .iter()
        .map(|territory| RoutingEndpoint {
            id: RoutingNodeRef::new(format!("territory/{}", territory.id.get())),
            kind: RoutingEndpointKind::Settlement,
        })
        .collect::<Vec<_>>();
    let mut connections = Vec::with_capacity(world.routes.len() * 2);
    for route in &world.routes {
        let forward = RoutingConnection {
            id: RoutingConnectionRef::new(format!("route/{}/forward", route.id.get())),
            from: RoutingNodeRef::new(format!("territory/{}", route.from.get())),
            to: RoutingNodeRef::new(format!("territory/{}", route.to.get())),
            mode,
            traversal: TraversalModel::Fixed {
                duration: SimDuration::minutes(route.travel_minutes),
            },
            available_from: None,
            available_until: None,
            risk_per_mille: 0,
            resource_cost: 0,
        };
        let reverse = RoutingConnection {
            id: RoutingConnectionRef::new(format!("route/{}/reverse", route.id.get())),
            from: forward.to.clone(),
            to: forward.from.clone(),
            ..forward.clone()
        };
        connections.push(forward);
        connections.push(reverse);
    }
    let network = RoutingNetwork::new(topology_version.clone(), endpoints, connections)?;
    Ok(canwu_api::PlanningSnapshot {
        observer: observer.into(),
        observed_at,
        valid_until: None,
        knowledge_cut: knowledge_cut.into(),
        topology_version,
        timetable_version: None,
        network,
    })
}

#[must_use]
pub fn world_record_ref() -> TypedDomainRecordRef<ReferenceWorldState> {
    TypedDomainRecordRef::new(WORLD_RECORD_ID)
}

pub fn scenario(start_time: SimTime, world: &WorldSnapshot) -> Result<Scenario, CanwuError> {
    validate_world(world, start_time)?;
    let entities = world_entities(world);
    let record = DomainRecord {
        reference: world_record_ref().into_untyped(),
        owner: PLUGIN_NAME.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::to_value(world).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                format!("reference world could not be encoded: {error}"),
            )
        })?,
        references: Vec::new(),
    };
    Ok(Scenario::new(start_time, entities).with_domain_records(vec![record]))
}

pub fn demo_scenario() -> Result<(Scenario, ReferenceWorldIds), CanwuError> {
    let ids = ReferenceWorldIds {
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
                transit: None,
            },
            Person {
                id: ids.observer,
                name: "Minister Luo".to_owned(),
                government: ids.government,
                current_location: ids.western_territory,
                roles: vec!["civil_minister".to_owned()],
                transit: None,
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
        letters: Vec::new(),
    };
    Ok((scenario(SimTime::EPOCH, &world)?, ids))
}

pub fn snapshot(canwu: &Canwu) -> Result<WorldSnapshot, CanwuError> {
    let record = canwu
        .typed_domain_record(&world_record_ref())
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "reference world state is unavailable",
            )
        })?;
    record.decode_payload::<ReferenceWorldState>()
}

pub fn order_movement(
    issuer: Issuer,
    command: &MovementCommand,
) -> Result<canwu_api::CommandEnvelope, CanwuError> {
    Ok(canwu_api::CommandEnvelope::new(
        issuer,
        Command::Plugin {
            plugin: PLUGIN_NAME.to_owned(),
            command: ORDER_MOVEMENT_COMMAND.to_owned(),
            payload: encode(command)?,
        },
    ))
}

impl SimulationPlugin for ReferenceWorldPlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }

    fn version(&self) -> &'static str {
        PLUGIN_VERSION
    }

    fn semantic_hash(&self) -> &'static str {
        SEMANTIC_HASH
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_record::<ReferenceWorldState>();
        schema.payload_schema = PayloadSchema::Any;
        let state = schema.state_key();
        let reservation = movement_reservation_key();
        registrar.register_record_schema(schema)?;
        registrar.register_command(
            PluginActionDescriptor {
                name: ORDER_MOVEMENT_COMMAND.to_owned(),
                description: "Order movement in the replaceable reference world".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: vec![state.clone(), reservation.clone()],
                writes: vec![reservation],
            },
            prepare_movement,
        )?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: MOVEMENT_INGRESS.to_owned(),
            description: "Apply a scheduled reference-world movement transition".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut system = BoundarySystemContract::new(
            MOVEMENT_SYSTEM,
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        system.reads = vec![StateKey::core_ingress(), state.clone()];
        system.writes = vec![state];
        system.emits = vec![
            "canwu.reference-world.movement_started.v1".to_owned(),
            "canwu.reference-world.movement_arrived.v1".to_owned(),
        ];
        system.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(system, apply_movement_transitions)
    }
}

fn prepare_movement(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let command: MovementCommand = serde_json::from_value(payload.clone()).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("reference-world movement payload is invalid: {error}"),
        )
    })?;
    validate_movement_command(&command)?;
    let DecisionOrigin::Actor { actor } = context.authority.decision_origin else {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "reference-world movement requires an accountable actor",
        ));
    };
    if context
        .authority
        .command_subject
        .as_ref()
        .is_some_and(|subject| subject != &command.subject)
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "authority command subject does not match the movement subject",
        ));
    }
    let record = view
        .typed_domain_record(&world_record_ref())?
        .ok_or_else(|| invalid_world("reference world state is unavailable"))?;
    let world = record.decode_payload::<ReferenceWorldState>()?;
    reject_active_movement_reservation(view, &command.subject, context.simulation_time)?;
    if world.territory(command.destination).is_none() {
        return Err(CanwuError::new(
            ErrorCode::DestinationNotFound,
            "movement destination is unavailable",
        )
        .with_entity(EntityRef::Territory(command.destination)));
    }
    let (from, duration) = movement_origin_and_duration(&world, actor, &command)?;
    let arrives_at = context
        .simulation_time
        .checked_add(duration)
        .ok_or_else(|| CanwuError::new(ErrorCode::InvalidDuration, "arrival time overflowed"))?;
    let transition = MovementTransition {
        stage: MovementStage::Started,
        subject: command.subject.clone(),
        from,
        destination: command.destination,
        arrives_at,
        cargo: command.cargo.clone(),
    };
    let mut arrival = transition.clone();
    arrival.stage = MovementStage::Arrived;
    let affected = movement_affected(&transition);
    Ok(vec![
        SystemDirective::SetComponent {
            state: movement_reservation_key(),
            entity: transition.subject.clone(),
            component: MOVEMENT_RESERVATION_COMPONENT.to_owned(),
            value: encode(&transition)?,
            summary: "Reserve a reference-world movement subject".to_owned(),
        },
        SystemDirective::EnqueuePluginIngress {
            after: SimDuration::ZERO,
            packet_type: MOVEMENT_INGRESS.to_owned(),
            priority: 0,
            payload: encode(&transition)?,
            affected: affected.clone(),
        },
        SystemDirective::EnqueuePluginIngress {
            after: duration,
            packet_type: MOVEMENT_INGRESS.to_owned(),
            priority: 0,
            payload: encode(&arrival)?,
            affected,
        },
    ])
}

fn movement_reservation_key() -> StateKey {
    StateKey::new(PLUGIN_NAME, "movement_reservation")
}

fn reject_active_movement_reservation(
    view: &SimulationView<'_>,
    subject: &EntityRef,
    now: SimTime,
) -> Result<(), CanwuError> {
    let Some(value) = view.component(
        &movement_reservation_key(),
        subject,
        MOVEMENT_RESERVATION_COMPONENT,
    )?
    else {
        return Ok(());
    };
    let reservation: MovementTransition =
        serde_json::from_value(value.clone()).map_err(|error| {
            invalid_world(format!(
                "stored movement reservation could not be decoded: {error}"
            ))
        })?;
    if reservation.stage != MovementStage::Started || &reservation.subject != subject {
        return Err(invalid_world("stored movement reservation is inconsistent"));
    }
    if reservation.arrives_at > now {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "reference movement subject already has a pending movement",
        )
        .with_entity(subject.clone()));
    }
    Ok(())
}

fn validate_movement_command(command: &MovementCommand) -> Result<(), CanwuError> {
    if command.cargo.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            "movement cargo IDs must be sorted and unique",
        ));
    }
    Ok(())
}

fn movement_origin_and_duration(
    world: &WorldSnapshot,
    actor: PersonId,
    command: &MovementCommand,
) -> Result<(TerritoryId, SimDuration), CanwuError> {
    let from = match &command.subject {
        EntityRef::Army(army_id) => {
            if !command.cargo.is_empty() {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPayload,
                    "reference army movement does not accept letter cargo",
                ));
            }
            let army = world.army(*army_id).ok_or_else(|| {
                CanwuError::new(ErrorCode::EntityNotFound, "reference army is unavailable")
                    .with_entity(EntityRef::Army(*army_id))
            })?;
            if army.commander != actor || army.transit.is_some() {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "actor does not command an idle reference army",
                ));
            }
            army.location
        }
        EntityRef::Person(person_id) => {
            if *person_id != actor {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "reference person movement must be self-directed",
                ));
            }
            let person = world.person(*person_id).ok_or_else(|| {
                CanwuError::new(ErrorCode::EntityNotFound, "reference person is unavailable")
                    .with_entity(EntityRef::Person(*person_id))
            })?;
            if person.transit.is_some() {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "reference person is already moving",
                ));
            }
            validate_person_cargo(world, *person_id, &command.cargo)?;
            person.current_location
        }
        _ => {
            return Err(CanwuError::new(
                ErrorCode::InvalidPayload,
                "reference movement supports only army and person subjects",
            ));
        }
    };
    Ok((from, travel_duration(world, from, command.destination)?))
}

fn validate_person_cargo(
    world: &WorldSnapshot,
    person_id: PersonId,
    cargo: &[LetterId],
) -> Result<(), CanwuError> {
    for letter_id in cargo {
        let letter = world.letter(*letter_id).ok_or_else(|| {
            CanwuError::new(ErrorCode::EntityNotFound, "movement cargo is unavailable")
                .with_entity(EntityRef::Resource(ResourceId::new(letter_id.get())))
        })?;
        if letter.status != LetterStatus::HeldByPerson || letter.carrier != Some(person_id) {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "movement cargo is not held by the moving person",
            ));
        }
    }
    Ok(())
}

fn apply_movement_transitions(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let current = view
        .typed_domain_record(&world_record_ref())?
        .ok_or_else(|| invalid_world("reference world state is unavailable"))?;
    let mut world = current.decode_payload::<ReferenceWorldState>()?;
    let mut emissions = Vec::new();
    let mut changed = false;
    for ingress_id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*ingress_id)? else {
            continue;
        };
        let IngressPayload::Plugin {
            plugin,
            packet_type,
            payload,
            ..
        } = &ingress.payload
        else {
            continue;
        };
        if plugin != PLUGIN_NAME || packet_type != MOVEMENT_INGRESS {
            continue;
        }
        let transition: MovementTransition =
            serde_json::from_value(payload.clone()).map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidPayload,
                    format!("reference movement transition is invalid: {error}"),
                )
            })?;
        apply_transition(&mut world, &transition, context.at)?;
        changed = true;
        emissions.push(BoundaryDirective::Emit {
            event_type: match transition.stage {
                MovementStage::Started => "canwu.reference-world.movement_started.v1",
                MovementStage::Arrived => "canwu.reference-world.movement_arrived.v1",
            }
            .to_owned(),
            summary: match transition.stage {
                MovementStage::Started => "Reference-world movement started",
                MovementStage::Arrived => "Reference-world movement arrived",
            }
            .to_owned(),
            affected: movement_affected(&transition),
        });
    }
    if !changed {
        return Ok(BoundaryProposal::default());
    }
    let mut directives = vec![BoundaryDirective::MutateRecord {
        mutation: DomainRecordMutation::Update {
            record: DomainRecordDraft::from_typed(world_record_ref(), &world)?,
            expected_version: current.version,
        },
        summary: "Apply reference-world movement transitions".to_owned(),
    }];
    directives.extend(emissions);
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn apply_transition(
    world: &mut WorldSnapshot,
    transition: &MovementTransition,
    at: SimTime,
) -> Result<(), CanwuError> {
    match transition.subject {
        EntityRef::Army(army_id) => {
            apply_army_transition(&mut world.armies, army_id, transition, at)?;
        }
        EntityRef::Person(person_id) => apply_person_transition(
            &mut world.people,
            &mut world.letters,
            person_id,
            transition,
            at,
        )?,
        _ => return Err(invalid_world("scheduled movement subject is unsupported")),
    }
    Ok(())
}

fn apply_army_transition(
    armies: &mut [Army],
    army_id: ArmyId,
    transition: &MovementTransition,
    at: SimTime,
) -> Result<(), CanwuError> {
    let army = armies
        .iter_mut()
        .find(|army| army.id == army_id)
        .ok_or_else(|| invalid_world("scheduled reference army is unavailable"))?;
    match transition.stage {
        MovementStage::Started => {
            if army.location != transition.from || army.transit.is_some() {
                return Err(invalid_world("reference army start state is inconsistent"));
            }
            army.transit = Some(TransitState {
                from: transition.from,
                to: transition.destination,
                departed_at: at,
                arrives_at: transition.arrives_at,
            });
        }
        MovementStage::Arrived => {
            if army.transit.as_ref().is_none_or(|transit| {
                transit.from != transition.from
                    || transit.to != transition.destination
                    || transit.arrives_at != at
            }) {
                return Err(invalid_world(
                    "reference army arrival state is inconsistent",
                ));
            }
            army.location = transition.destination;
            army.transit = None;
        }
    }
    Ok(())
}

fn apply_person_transition(
    people: &mut [Person],
    letters: &mut [LetterCargo],
    person_id: PersonId,
    transition: &MovementTransition,
    at: SimTime,
) -> Result<(), CanwuError> {
    let person = people
        .iter_mut()
        .find(|person| person.id == person_id)
        .ok_or_else(|| invalid_world("scheduled reference person is unavailable"))?;
    match transition.stage {
        MovementStage::Started => {
            if person.current_location != transition.from || person.transit.is_some() {
                return Err(invalid_world(
                    "reference person start state is inconsistent",
                ));
            }
            person.transit = Some(PersonTransitState {
                from: transition.from,
                to: transition.destination,
                departed_at: at,
                arrives_at: transition.arrives_at,
            });
            set_cargo_in_transit(letters, &transition.cargo)?;
        }
        MovementStage::Arrived => {
            if person.transit.as_ref().is_none_or(|transit| {
                transit.from != transition.from
                    || transit.to != transition.destination
                    || transit.arrives_at != at
            }) {
                return Err(invalid_world(
                    "reference person arrival state is inconsistent",
                ));
            }
            person.current_location = transition.destination;
            person.transit = None;
            deliver_cargo(
                letters,
                person_id,
                transition.destination,
                at,
                &transition.cargo,
            )?;
        }
    }
    Ok(())
}

fn set_cargo_in_transit(letters: &mut [LetterCargo], cargo: &[LetterId]) -> Result<(), CanwuError> {
    for id in cargo {
        let letter = letters
            .iter_mut()
            .find(|letter| letter.id == *id)
            .ok_or_else(|| invalid_world("scheduled letter cargo is unavailable"))?;
        letter.status = LetterStatus::InTransit;
    }
    Ok(())
}

fn deliver_cargo(
    letters: &mut [LetterCargo],
    person_id: PersonId,
    destination: TerritoryId,
    at: SimTime,
    cargo: &[LetterId],
) -> Result<(), CanwuError> {
    for id in cargo {
        let letter = letters
            .iter_mut()
            .find(|letter| letter.id == *id)
            .ok_or_else(|| invalid_world("scheduled letter cargo is unavailable"))?;
        letter.status = if letter.recipient == person_id {
            LetterStatus::Delivered
        } else {
            LetterStatus::HeldByPerson
        };
        letter.carrier = (letter.status == LetterStatus::HeldByPerson).then_some(person_id);
        letter.location = (letter.status == LetterStatus::Delivered).then_some(destination);
        letter.delivered_at = (letter.status == LetterStatus::Delivered).then_some(at);
    }
    Ok(())
}

fn movement_affected(transition: &MovementTransition) -> Vec<EntityRef> {
    let mut affected = vec![
        transition.subject.clone(),
        EntityRef::Territory(transition.from),
        EntityRef::Territory(transition.destination),
    ];
    affected.extend(
        transition
            .cargo
            .iter()
            .map(|id| EntityRef::Resource(ResourceId::new(id.get()))),
    );
    affected.sort();
    affected.dedup();
    affected
}

fn travel_duration(
    world: &WorldSnapshot,
    from: TerritoryId,
    destination: TerritoryId,
) -> Result<SimDuration, CanwuError> {
    let duration = world
        .travel_duration(from, destination)
        .ok_or_else(|| CanwuError::new(ErrorCode::NoRoute, "reference route is unavailable"))?;
    if duration <= SimDuration::ZERO {
        return Err(CanwuError::new(
            ErrorCode::InvalidDuration,
            "reference route duration must be positive",
        ));
    }
    Ok(duration)
}

fn world_entities(world: &WorldSnapshot) -> Vec<EntityRef> {
    let mut entities = Vec::new();
    entities.extend(world.armies.iter().map(|value| EntityRef::Army(value.id)));
    entities.extend(
        world
            .governments
            .iter()
            .map(|value| EntityRef::Government(value.id)),
    );
    entities.extend(world.people.iter().map(|value| EntityRef::Person(value.id)));
    entities.extend(
        world
            .letters
            .iter()
            .map(|value| EntityRef::Resource(ResourceId::new(value.id.get()))),
    );
    entities.extend(world.routes.iter().map(|value| EntityRef::Route(value.id)));
    entities.extend(
        world
            .territories
            .iter()
            .map(|value| EntityRef::Territory(value.id)),
    );
    entities.sort();
    entities.dedup();
    entities
}

fn validate_world(world: &WorldSnapshot, start_time: SimTime) -> Result<(), CanwuError> {
    let entities = world_entities(world);
    let expected = world.people.len()
        + world.governments.len()
        + world.territories.len()
        + world.routes.len()
        + world.armies.len()
        + world.letters.len();
    if entities.len() != expected {
        return Err(invalid_world("reference world identities must be unique"));
    }
    for person in &world.people {
        if person.id.get() == 0
            || world.government(person.government).is_none()
            || world.territory(person.current_location).is_none()
            || person.transit.is_some()
        {
            return Err(invalid_world("reference person is inconsistent"));
        }
    }
    for government in &world.governments {
        if government.id.get() == 0 || world.territory(government.capital).is_none() {
            return Err(invalid_world("reference government is inconsistent"));
        }
    }
    for territory in &world.territories {
        if territory.id.get() == 0
            || world.government(territory.controller).is_none()
            || !territory.position.x.is_finite()
            || !territory.position.y.is_finite()
        {
            return Err(invalid_world("reference territory is inconsistent"));
        }
    }
    for route in &world.routes {
        if route.id.get() == 0
            || route.travel_minutes <= 0
            || world.territory(route.from).is_none()
            || world.territory(route.to).is_none()
        {
            return Err(invalid_world("reference route is inconsistent"));
        }
    }
    for army in &world.armies {
        if army.id.get() == 0
            || army.transit.is_some()
            || world.person(army.commander).is_none()
            || world.government(army.government).is_none()
            || world.territory(army.location).is_none()
        {
            return Err(invalid_world("reference army is inconsistent"));
        }
    }
    for letter in &world.letters {
        let custody_valid = match letter.status {
            LetterStatus::HeldByPerson | LetterStatus::InTransit => {
                letter.carrier.is_some()
                    && letter.location.is_none()
                    && letter.delivered_at.is_none()
            }
            LetterStatus::HeldAtLocation => {
                letter.carrier.is_none()
                    && letter.location.is_some()
                    && letter.delivered_at.is_none()
            }
            LetterStatus::Delivered => {
                letter.carrier.is_none()
                    && letter.location.is_some()
                    && letter.delivered_at.is_some()
            }
        };
        if letter.id.get() == 0
            || letter.body.len() > 65_536
            || world.person(letter.sender).is_none()
            || world.person(letter.recipient).is_none()
            || letter
                .carrier
                .is_some_and(|carrier| world.person(carrier).is_none())
            || letter
                .location
                .is_some_and(|location| world.territory(location).is_none())
            || letter.delivered_at.is_some_and(|at| at > start_time)
            || !custody_valid
        {
            return Err(invalid_world("reference letter is inconsistent"));
        }
    }
    Ok(())
}

fn encode<T: Serialize>(value: &T) -> Result<Value, CanwuError> {
    serde_json::to_value(value).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("reference-world payload could not be encoded: {error}"),
        )
    })
}

fn invalid_world(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_flow_saves_loads_forks_and_replays_exactly() {
        let (scenario, ids) = demo_scenario().expect("reference scenario should be valid");
        let plugin = ReferenceWorldPlugin;
        let mut canwu = Canwu::new_with_plugins(35, scenario.clone(), &[&plugin])
            .expect("reference run should initialize");
        let envelope = order_movement(
            Issuer::Actor(ids.commander),
            &MovementCommand {
                subject: EntityRef::Army(ids.army),
                destination: ids.eastern_territory,
                cargo: Vec::new(),
            },
        )
        .expect("reference movement command should encode")
        .at_time(canwu.time());
        canwu
            .enqueue_command(
                canwu.time(),
                0,
                canwu_api::CommandRequest::new(
                    canwu_api::CommandRequestId::new(1),
                    canwu.revision(),
                    envelope,
                ),
            )
            .expect("reference movement should be queued");
        canwu
            .advance_canonical(SimDuration::hours(19))
            .expect("reference movement should settle");
        assert_eq!(
            snapshot(&canwu)
                .expect("reference projection should decode")
                .army(ids.army)
                .expect("demo army should exist")
                .location,
            ids.eastern_territory
        );

        let saved = canwu.snapshot_json().expect("snapshot should serialize");
        let loaded = Canwu::from_snapshot_json_with_plugins(&saved, &[&plugin])
            .expect("snapshot should load with its integration");
        assert_eq!(loaded.checkpoint_hash(), canwu.checkpoint_hash());
        assert_eq!(snapshot(&loaded), snapshot(&canwu));

        let fork = canwu.fork();
        assert_eq!(fork.checkpoint_hash(), canwu.checkpoint_hash());
        assert_eq!(snapshot(&fork), snapshot(&canwu));

        let journal = canwu.replay_journal();
        let replayed = Canwu::replay_from_journal(scenario, &[&plugin], &journal)
            .expect("plugin-aware replay should reproduce the run");
        assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());
        assert_eq!(snapshot(&replayed), snapshot(&canwu));
    }

    #[test]
    fn reference_letters_require_consistent_custody_and_existing_places() {
        let (scenario, ids) = demo_scenario().expect("reference scenario should be valid");
        let mut world = scenario.domain_records[0]
            .decode_payload::<ReferenceWorldState>()
            .expect("reference scenario should contain its typed world");
        let letter = LetterCargo {
            id: LetterId::new(1),
            sender: ids.commander,
            recipient: ids.observer,
            body: "Request for aid".to_owned(),
            status: LetterStatus::HeldByPerson,
            carrier: Some(ids.commander),
            location: None,
            delivered_at: None,
        };
        world.letters.push(letter.clone());
        validate_world(&world, SimTime::EPOCH).expect("valid held letter should pass");

        let mut missing_carrier = world.clone();
        missing_carrier.letters[0].carrier = None;
        assert_eq!(
            validate_world(&missing_carrier, SimTime::EPOCH)
                .expect_err("held letter without a carrier must fail")
                .code,
            ErrorCode::InvalidDomainRecord
        );

        let mut missing_place = world.clone();
        missing_place.letters[0] = LetterCargo {
            status: LetterStatus::HeldAtLocation,
            carrier: None,
            location: Some(TerritoryId::new(999)),
            ..letter.clone()
        };
        assert_eq!(
            validate_world(&missing_place, SimTime::EPOCH)
                .expect_err("letter at an unknown place must fail")
                .code,
            ErrorCode::InvalidDomainRecord
        );

        let mut invalid_delivery = world;
        invalid_delivery.letters[0] = LetterCargo {
            status: LetterStatus::Delivered,
            carrier: None,
            location: Some(ids.central_territory),
            delivered_at: None,
            ..letter
        };
        assert_eq!(
            validate_world(&invalid_delivery, SimTime::EPOCH)
                .expect_err("delivered letter without delivery time must fail")
                .code,
            ErrorCode::InvalidDomainRecord
        );
    }

    #[test]
    fn a_second_pending_movement_is_rejected_without_blocking_settlement() {
        let (scenario, ids) = demo_scenario().expect("reference scenario should be valid");
        let replay_scenario = scenario.clone();
        let plugin = ReferenceWorldPlugin;
        let mut canwu = Canwu::new_with_plugins(35, scenario, &[&plugin])
            .expect("reference run should initialize");
        let movement = MovementCommand {
            subject: EntityRef::Army(ids.army),
            destination: ids.eastern_territory,
            cargo: Vec::new(),
        };
        let first_revision = canwu.revision();
        canwu
            .enqueue_command(
                canwu.time(),
                0,
                canwu_api::CommandRequest::new(
                    canwu_api::CommandRequestId::new(1),
                    first_revision,
                    order_movement(Issuer::Actor(ids.commander), &movement)
                        .expect("first movement should encode")
                        .at_time(canwu.time()),
                ),
            )
            .expect("first movement should queue");
        canwu
            .enqueue_command(
                canwu.time(),
                0,
                canwu_api::CommandRequest::new(
                    canwu_api::CommandRequestId::new(2),
                    first_revision + 1,
                    order_movement(Issuer::Actor(ids.commander), &movement)
                        .expect("second movement should encode")
                        .at_time(canwu.time()),
                ),
            )
            .expect("second movement should queue");
        canwu
            .step_canonical()
            .expect("the command boundary should settle")
            .expect("two due commands should produce a boundary");
        assert!(matches!(
            canwu.command_attempts()[0].outcome,
            canwu_api::CommandAttemptOutcome::Accepted { .. }
        ));
        let canwu_api::CommandAttemptOutcome::Rejected { error } =
            &canwu.command_attempts()[1].outcome
        else {
            panic!("second pending movement must be rejected");
        };
        assert_eq!(error.code, ErrorCode::InvalidAuthority);

        canwu
            .advance_canonical(SimDuration::hours(19))
            .expect("the accepted movement must still settle");
        assert_eq!(
            snapshot(&canwu)
                .expect("reference projection should decode")
                .army(ids.army)
                .expect("demo army should exist")
                .location,
            ids.eastern_territory
        );
        let replayed =
            Canwu::replay_from_journal(replay_scenario, &[&plugin], &canwu.replay_journal())
                .expect("the accepted and rejected movements should replay exactly");
        assert_eq!(replayed.snapshot(), canwu.snapshot());
    }
}
