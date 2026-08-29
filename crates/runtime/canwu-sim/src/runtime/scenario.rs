use super::{
    ActorKnowledge, Army, ArmyId, ArmyKnowledge, CanwuError, DomainRecord, EntityRef, ErrorCode,
    EstimateRange, FieldSchema, Government, GovernmentId, KnowledgeSnapshot, KnowledgeSource,
    LetterStatus, MapPoint, Person, PersonId, Route, RouteId, SchemaRegistry, SimDuration, SimTime,
    Territory, TerritoryId, TypeSchema, WorldSnapshot, core_world_entity_exists, invalid_snapshot,
    records,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<EntityRef>,
    #[serde(default)]
    pub world: WorldSnapshot,
    pub knowledge: KnowledgeSnapshot,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domain_records: Vec<DomainRecord>,
}

impl Scenario {
    #[must_use]
    pub fn new(start_time: SimTime, entities: Vec<EntityRef>) -> Self {
        Self {
            start_time,
            entities,
            world: WorldSnapshot::default(),
            knowledge: KnowledgeSnapshot::default(),
            domain_records: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_domain_records(mut self, domain_records: Vec<DomainRecord>) -> Self {
        self.domain_records = domain_records;
        self
    }
}

pub(super) fn legacy_entities(world: &WorldSnapshot) -> Vec<EntityRef> {
    let mut entities = Vec::with_capacity(
        world.people.len()
            + world.governments.len()
            + world.territories.len()
            + world.routes.len()
            + world.armies.len()
            + world.letters.len(),
    );
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
            .map(|value| EntityRef::Resource(super::ResourceId::new(value.id.get()))),
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

pub(super) fn require_plugin_aware_initial_records(scenario: &Scenario) -> Result<(), CanwuError> {
    if scenario.domain_records.is_empty() {
        return Ok(());
    }
    Err(CanwuError::new(
        ErrorCode::PluginNotActive,
        "scenarios with initial domain records require a plugin-aware constructor",
    ))
}

pub(super) fn canonicalize_scenario(scenario: &mut Scenario) {
    if scenario.entities.is_empty() {
        scenario.entities = legacy_entities(&scenario.world);
    }
    scenario.entities.sort();
    scenario.entities.dedup();
    scenario.world.people.sort_by_key(|value| value.id);
    scenario.world.governments.sort_by_key(|value| value.id);
    scenario.world.territories.sort_by_key(|value| value.id);
    scenario.world.routes.sort_by_key(|value| value.id);
    scenario.world.armies.sort_by_key(|value| value.id);
    scenario.world.letters.sort_by_key(|value| value.id);
    scenario
        .domain_records
        .sort_by(|left, right| left.reference.cmp(&right.reference));
}

pub(super) fn validate_scenario(scenario: &Scenario) -> Result<(), CanwuError> {
    if !scenario.knowledge.records.is_empty() {
        return Err(CanwuError::new(
            ErrorCode::InvalidKnowledgeRecord,
            "scenario authors cannot preselect generic knowledge IDs, times, or origins",
        ));
    }
    validate_scenario_state(scenario)
}

pub(super) fn validate_scenario_state(scenario: &Scenario) -> Result<(), CanwuError> {
    validate_entities(&scenario.entities)?;
    validate_unique_ids(&scenario.world.people, |value| value.id, "person")?;
    validate_unique_ids(&scenario.world.governments, |value| value.id, "government")?;
    validate_unique_ids(&scenario.world.territories, |value| value.id, "territory")?;
    validate_unique_ids(&scenario.world.routes, |value| value.id, "route")?;
    validate_unique_ids(&scenario.world.armies, |value| value.id, "army")?;
    validate_unique_ids(&scenario.world.letters, |value| value.id, "letter")?;
    let expected_legacy_entities = legacy_entities(&scenario.world);
    if expected_legacy_entities
        .iter()
        .any(|entity| scenario.entities.binary_search(entity).is_err())
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidSnapshot,
            "scenario entity registry is missing an identity from its populated compatibility world",
        ));
    }

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
        if let Some(transit) = &person.transit
            && (scenario.world.territory(transit.from).is_none()
                || scenario.world.territory(transit.to).is_none()
                || transit.arrives_at <= transit.departed_at
                || transit.departed_at > scenario.start_time
                || person.current_location != transit.from)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("person {} has invalid transit state", person.id),
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
    for letter in &scenario.world.letters {
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
        if letter.body.len() > 65_536
            || scenario.world.person(letter.sender).is_none()
            || scenario.world.person(letter.recipient).is_none()
            || letter
                .location
                .is_some_and(|location| scenario.world.territory(location).is_none())
            || !custody_valid
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("letter {} has invalid custody or payload state", letter.id),
            ));
        }
        if let Some(carrier) = letter.carrier
            && scenario.world.person(carrier).is_none()
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("letter {} references a missing carrier", letter.id),
            ));
        }
    }
    records::validate_initial_records(&scenario.domain_records, scenario.start_time, &|entity| {
        scenario.entities.binary_search(entity).is_ok()
            || core_world_entity_exists(&scenario.world, entity)
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

fn validate_entities(entities: &[EntityRef]) -> Result<(), CanwuError> {
    if entities.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(CanwuError::new(
            ErrorCode::InvalidSnapshot,
            "scenario entity identities must be unique and canonically sorted",
        ));
    }
    if entities.iter().any(|entity| match entity {
        EntityRef::Army(id) => id.get() == 0,
        EntityRef::Domain(_) => true,
        EntityRef::Government(id) => id.get() == 0,
        EntityRef::Organization(id) => id.get() == 0,
        EntityRef::Person(id) => id.get() == 0,
        EntityRef::Resource(id) => id.get() == 0,
        EntityRef::Route(id) => id.get() == 0,
        EntityRef::Territory(id) => id.get() == 0,
    }) {
        return Err(CanwuError::new(
            ErrorCode::InvalidSnapshot,
            "scenario entities require nonzero core identities; domain entities come from domain records",
        ));
    }
    Ok(())
}

pub(super) fn validate_unique_ids<T, I, F>(
    values: &[T],
    mut id_of: F,
    label: &str,
) -> Result<(), CanwuError>
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

pub(super) fn validate_strict_id_order<T, I, F>(
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

pub(super) fn base_schema() -> SchemaRegistry {
    let mut schema = SchemaRegistry::default();
    schema
        .register(TypeSchema {
            type_name: "person".to_owned(),
            description: "Historical actor with roles and a location".to_owned(),
            fields: vec![
                field("id", "PersonId", "Stable person identifier"),
                field("name", "String", "Display name"),
                field("government", "GovernmentId", "Government membership"),
                field("current_location", "TerritoryId", "Current territory"),
                field("roles", "Vec<String>", "Offices and authorities"),
            ],
        })
        .expect("base schema types are unique");
    schema
        .register(TypeSchema {
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
        })
        .expect("base schema types are unique");
    schema
        .register(TypeSchema {
            type_name: "territory".to_owned(),
            description: "Administrative and geographic unit".to_owned(),
            fields: vec![
                field("id", "TerritoryId", "Stable territory identifier"),
                field("controller", "GovernmentId", "Controlling government"),
                field("position", "MapPoint", "Abstract visualization point"),
            ],
        })
        .expect("base schema types are unique");
    schema
        .register(TypeSchema {
            type_name: "route".to_owned(),
            description: "Travel connection between territories".to_owned(),
            fields: vec![
                field("from", "TerritoryId", "First route endpoint"),
                field("to", "TerritoryId", "Second route endpoint"),
                field("travel_minutes", "i64", "Deterministic travel duration"),
                field("terrain", "String", "Terrain classification"),
            ],
        })
        .expect("base schema types are unique");
    schema
        .register(TypeSchema {
            type_name: "event".to_owned(),
            description: "Inspectable state-change or information event".to_owned(),
            fields: vec![field("timestamp", "SimTime", "Simulation occurrence time")],
        })
        .expect("base schema types are unique");
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
            entities: vec![
                EntityRef::Army(ids.army),
                EntityRef::Government(ids.government),
                EntityRef::Person(ids.commander),
                EntityRef::Person(ids.observer),
                EntityRef::Route(RouteId::new(1)),
                EntityRef::Route(RouteId::new(2)),
                EntityRef::Territory(ids.western_territory),
                EntityRef::Territory(ids.central_territory),
                EntityRef::Territory(ids.eastern_territory),
            ],
            world,
            knowledge,
            domain_records: Vec::new(),
        },
        ids,
    )
}
