#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    ActorKnowledge, Army, ArmyId, ArmyKnowledge, BoundaryContext, BoundaryDirective, BoundaryPhase,
    BoundaryProposal, BoundaryRequest, BoundarySystemContract, Canwu, CanwuError, Command,
    CommandAuthority, CommandEnvelope, CommandOutcome, CommandRequest, CommandRequestId,
    DecisionOrigin, DomainEntityKindClass, DomainRecordDraft, DomainRecordLifecycle,
    DomainRecordSchema, DomainRecordType, DomainReference, DomainReferenceSchema,
    DomainReferenceTargetKind, DomainValueKindClass, EntityRef, ErrorCode, EstimateRange,
    Government, GovernmentId, Issuer, KnowledgeSnapshot, KnowledgeSource, MapPoint,
    PayloadProperty, PayloadSchema, PayloadValueType, Person, PersonId, PluginActionDescriptor,
    PluginRegistrar, RandomDrawProducer, RandomStreamKey, ReservationDisposition, ReservationOffer,
    ReservationPoolKey, ReservationRef, ReservationRequest, Route, RouteId, Scenario, SimDuration,
    SimTime, SimulationPlugin, SimulationSnapshot, SimulationView, StateKey, StateVisibility,
    SystemCadence, SystemDirective, Territory, TerritoryId, TypedDomainRecordRef, WorldSnapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const GOVERNANCE_PLUGIN: &str = "fixture-governance";
const APPOINTMENT_PLUGIN: &str = "fixture-appointments";
const SUPPLY_PLUGIN: &str = "fixture-supply";
const DEMAND_PLUGIN: &str = "fixture-demand";
const FORECAST_PLUGIN: &str = "fixture-forecast";

fn commander() -> PersonId {
    PersonId::new(1)
}

fn observer() -> PersonId {
    PersonId::new(2)
}

fn government() -> GovernmentId {
    GovernmentId::new(1)
}

fn army() -> ArmyId {
    ArmyId::new(1)
}

fn western_territory() -> TerritoryId {
    TerritoryId::new(1)
}

fn central_territory() -> TerritoryId {
    TerritoryId::new(2)
}

fn eastern_territory() -> TerritoryId {
    TerritoryId::new(3)
}

fn fixture_scenario() -> Scenario {
    let initial_time = SimTime::EPOCH;
    let world = WorldSnapshot {
        people: vec![
            Person {
                id: commander(),
                name: "Commander Ren".to_owned(),
                government: government(),
                current_location: central_territory(),
                roles: vec!["field_commander".to_owned()],
                transit: None,
            },
            Person {
                id: observer(),
                name: "Archivist Lin".to_owned(),
                government: government(),
                current_location: western_territory(),
                roles: vec!["observer".to_owned()],
                transit: None,
            },
        ],
        governments: vec![Government {
            id: government(),
            name: "River State".to_owned(),
            capital: central_territory(),
        }],
        territories: vec![
            Territory {
                id: western_territory(),
                name: "West Gate".to_owned(),
                controller: government(),
                position: MapPoint { x: 0.0, y: 0.0 },
            },
            Territory {
                id: central_territory(),
                name: "Central Seat".to_owned(),
                controller: government(),
                position: MapPoint { x: 1.0, y: 0.0 },
            },
            Territory {
                id: eastern_territory(),
                name: "East Gate".to_owned(),
                controller: government(),
                position: MapPoint { x: 2.0, y: 0.0 },
            },
        ],
        routes: vec![
            Route {
                id: RouteId::new(1),
                name: "Western Road".to_owned(),
                from: western_territory(),
                to: central_territory(),
                travel_minutes: SimDuration::hours(12).as_minutes(),
                terrain: "road".to_owned(),
            },
            Route {
                id: RouteId::new(2),
                name: "Eastern Road".to_owned(),
                from: central_territory(),
                to: eastern_territory(),
                travel_minutes: SimDuration::hours(18).as_minutes(),
                terrain: "road".to_owned(),
            },
        ],
        armies: vec![Army {
            id: army(),
            name: "First Field Force".to_owned(),
            government: government(),
            commander: commander(),
            location: central_territory(),
            strength: 8_000,
            morale: 72,
            transit: None,
        }],
        letters: Vec::new(),
    };
    let knowledge = KnowledgeSnapshot {
        actors: BTreeMap::from([
            (
                commander(),
                ActorKnowledge {
                    actor: commander(),
                    armies: BTreeMap::from([(
                        army(),
                        ArmyKnowledge {
                            army: army(),
                            known_name: Some("First Field Force".to_owned()),
                            known_location: Some(central_territory()),
                            estimated_strength: EstimateRange {
                                minimum: 8_000,
                                maximum: 8_000,
                            },
                            observed_at: initial_time,
                            learned_at: initial_time,
                            confidence_per_mille: 1_000,
                            source: KnowledgeSource::CommandResponsibility,
                        },
                    )]),
                },
            ),
            (
                observer(),
                ActorKnowledge {
                    actor: observer(),
                    armies: BTreeMap::from([(
                        army(),
                        ArmyKnowledge {
                            army: army(),
                            known_name: Some("First Field Force".to_owned()),
                            known_location: Some(central_territory()),
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
            ),
        ]),
        records: BTreeMap::new(),
    };
    Scenario {
        start_time: initial_time,
        entities: vec![
            EntityRef::Army(army()),
            EntityRef::Government(government()),
            EntityRef::Person(commander()),
            EntityRef::Person(observer()),
            EntityRef::Route(RouteId::new(1)),
            EntityRef::Route(RouteId::new(2)),
            EntityRef::Territory(western_territory()),
            EntityRef::Territory(central_territory()),
            EntityRef::Territory(eastern_territory()),
        ],
        world,
        knowledge,
        domain_records: Vec::new(),
    }
}

fn string_object_schema(field: &str) -> PayloadSchema {
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

#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct OfficePayload {
    name: String,
}

struct Office;

impl DomainRecordType for Office {
    type Payload = OfficePayload;
    type Class = DomainEntityKindClass;

    const NAMESPACE: &'static str = "fixture.governance";
    const NAME: &'static str = "office";
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct AppointmentPayload {
    status: String,
}

struct Appointment;

impl DomainRecordType for Appointment {
    type Payload = AppointmentPayload;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "fixture.appointments";
    const NAME: &'static str = "appointment";
}

fn office_reference() -> TypedDomainRecordRef<Office> {
    TypedDomainRecordRef::new("council")
}

fn appointment_reference() -> TypedDomainRecordRef<Appointment> {
    TypedDomainRecordRef::new("council-secretary")
}

fn authorization_state() -> StateKey {
    StateKey::new("fixture.governance", "authorization")
}

fn authorize_operation(
    view: &SimulationView<'_>,
    context: &canwu_api::CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let target = ArmyId::new(
        payload
            .get("army")
            .and_then(Value::as_u64)
            .expect("the public payload schema validates the army ID"),
    );
    let Some(target_state) = view.army(target)? else {
        return Err(CanwuError::new(
            ErrorCode::ArmyNotFound,
            "the requested force is unavailable",
        ));
    };
    let accountable_actor = matches!(
        context.authority.decision_origin,
        DecisionOrigin::Actor { actor } if actor == target_state.commander
    );
    if context.issuer != Issuer::Actor(target_state.commander)
        || !accountable_actor
        || context.authority.command_subject != Some(EntityRef::Army(target))
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "the command requires the accountable force commander",
        ));
    }
    Ok(vec![SystemDirective::SetComponent {
        state: authorization_state(),
        entity: EntityRef::Army(target),
        component: "status".to_owned(),
        value: Value::String("approved".to_owned()),
        summary: "The accountable commander approved the operation".to_owned(),
    }])
}

fn create_office(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if context.boundary_id.get() != 1 {
        return Ok(BoundaryProposal::default());
    }
    let record = DomainRecordDraft::from_typed(
        office_reference(),
        &OfficePayload {
            name: "Council Office".to_owned(),
        },
    )?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation: canwu_api::DomainRecordMutation::Create { record },
            summary: "Create the council office".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn register_governance(registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
    let mut office = DomainRecordSchema::for_entity::<Office>();
    office.payload_schema = string_object_schema("name");
    let office_state = office.state_key();
    registrar.register_record_schema(office)?;

    registrar.register_command(
        PluginActionDescriptor {
            name: "authorize_operation".to_owned(),
            description: "Authorize an operation with accountable command authority".to_owned(),
            payload_schema: PayloadSchema::Object {
                properties: BTreeMap::from([(
                    "army".to_owned(),
                    PayloadProperty {
                        value_type: PayloadValueType::Integer,
                        required: true,
                    },
                )]),
                allow_additional: false,
            },
            reads: vec![StateKey::core_armies()],
            writes: vec![authorization_state()],
        },
        authorize_operation,
    )?;

    let mut create = BoundarySystemContract::new(
        "create-office",
        BoundaryPhase::DomainDeltaProposal,
        SystemCadence::Daily,
    );
    create.writes = vec![office_state];
    create.visibility = StateVisibility::SameBoundary;
    registrar.register_boundary_system(create, create_office)
}

struct GovernancePlugin;

impl SimulationPlugin for GovernancePlugin {
    fn name(&self) -> &'static str {
        GOVERNANCE_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000101"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        register_governance(registrar)
    }
}

struct ChangedGovernancePlugin;

impl SimulationPlugin for ChangedGovernancePlugin {
    fn name(&self) -> &'static str {
        GOVERNANCE_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000001ff"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        register_governance(registrar)
    }
}

fn create_appointment(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if context.boundary_id.get() != 1 {
        return Ok(BoundaryProposal::default());
    }
    if view.typed_domain_record(&office_reference())?.is_none() {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "the transition phase must observe the committed office candidate",
        ));
    }
    let mut record = DomainRecordDraft::from_typed(
        appointment_reference(),
        &AppointmentPayload {
            status: "active".to_owned(),
        },
    )?;
    record
        .references
        .push(DomainReference::from_typed("office", office_reference()));
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation: canwu_api::DomainRecordMutation::Create { record },
            summary: "Create an appointment after the office candidate commits".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

struct AppointmentPlugin;

impl SimulationPlugin for AppointmentPlugin {
    fn name(&self) -> &'static str {
        APPOINTMENT_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000102"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut appointment = DomainRecordSchema::for_record::<Appointment>();
        appointment.payload_schema = string_object_schema("status");
        appointment.references = vec![DomainReferenceSchema {
            role: "office".to_owned(),
            targets: vec![DomainReferenceTargetKind::for_domain::<Office>()],
            required: true,
            multiple: false,
            allow_retired: false,
        }];
        let appointment_state = appointment.state_key();
        registrar.register_record_schema(appointment)?;

        let mut transition = BoundarySystemContract::new(
            "appoint-secretary",
            BoundaryPhase::HistoricalCandidateEvaluation,
            SystemCadence::Daily,
        );
        transition.reads = vec![DomainRecordSchema::for_entity::<Office>().state_key()];
        transition.writes = vec![appointment_state];
        transition.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(transition, create_appointment)
    }
}

fn capacity_pool() -> ReservationPoolKey {
    ReservationPoolKey::new(
        StateKey::new("fixture.logistics", "capacity"),
        EntityRef::Territory(western_territory()),
        "transport",
    )
}

fn offer_capacity(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        offers: vec![ReservationOffer {
            pool: capacity_pool(),
            capacity: 10,
        }],
        ..BoundaryProposal::default()
    })
}

struct SupplyPlugin;

impl SimulationPlugin for SupplyPlugin {
    fn name(&self) -> &'static str {
        SUPPLY_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000103"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut offer = BoundarySystemContract::new(
            "offer-capacity",
            BoundaryPhase::ReservationAndAllocation,
            SystemCadence::Daily,
        );
        offer.reservation_offers = vec![capacity_pool().state];
        registrar.register_boundary_system(offer, offer_capacity)
    }
}

fn high_request(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        requests: vec![ReservationRequest {
            request: "transport".to_owned(),
            pool: capacity_pool(),
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
            request: "transport".to_owned(),
            pool: capacity_pool(),
            quantity: 7,
            priority: 0,
            tie_break: "low".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn high_state() -> StateKey {
    StateKey::new("fixture.demand", "high")
}

fn low_state() -> StateKey {
    StateKey::new("fixture.demand", "low")
}

fn record_grant(
    view: &SimulationView<'_>,
    system: &str,
    state: StateKey,
    component: &str,
) -> Result<BoundaryProposal, CanwuError> {
    let reservation = ReservationRef::new(DEMAND_PLUGIN, system, "transport");
    let allocation = view.reservation(&reservation)?.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidBoundary,
            "the declared reservation is missing allocation evidence",
        )
    })?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::SetComponent {
            state,
            entity: EntityRef::Territory(western_territory()),
            component: component.to_owned(),
            value: Value::from(allocation.granted),
            summary: format!("Record a transport grant of {}", allocation.granted),
        }],
        ..BoundaryProposal::default()
    })
}

fn record_high_grant(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    record_grant(view, "high-request", high_state(), "granted")
}

fn record_low_grant(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    record_grant(view, "low-request", low_state(), "granted")
}

fn validate_visibility(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let entity = EntityRef::Territory(western_territory());
    let high = view
        .component(&high_state(), &entity, "granted")?
        .and_then(Value::as_u64);
    let low = view
        .component(&low_state(), &entity, "granted")?
        .and_then(Value::as_u64);
    let proposed_high = view
        .proposed_component(&high_state(), &entity, "granted")?
        .and_then(Value::as_u64);
    let proposed_low = view
        .proposed_component(&low_state(), &entity, "granted")?
        .and_then(Value::as_u64);
    let expected_current_low = (context.boundary_id.get() > 1).then_some(3);
    if high != Some(7)
        || low != expected_current_low
        || proposed_high != Some(7)
        || proposed_low != Some(3)
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "same-boundary and next-boundary visibility diverged from the public contract",
        ));
    }
    Ok(BoundaryProposal::default())
}

struct DemandPlugin;

impl SimulationPlugin for DemandPlugin {
    fn name(&self) -> &'static str {
        DEMAND_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000104"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        for (name, handler) in [
            (
                "high-request",
                high_request as canwu_api::BoundarySystemHandler,
            ),
            (
                "low-request",
                low_request as canwu_api::BoundarySystemHandler,
            ),
        ] {
            let mut request = BoundarySystemContract::new(
                name,
                BoundaryPhase::ReservationAndAllocation,
                SystemCadence::Daily,
            );
            request.reservation_requests = vec![capacity_pool().state];
            registrar.register_boundary_system(request, handler)?;
        }

        let mut high = BoundarySystemContract::new(
            "apply-high",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        high.writes = vec![high_state()];
        high.reservation_reads = vec![ReservationRef::new(
            DEMAND_PLUGIN,
            "high-request",
            "transport",
        )];
        high.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(high, record_high_grant)?;

        let mut low = BoundarySystemContract::new(
            "apply-low",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        low.writes = vec![low_state()];
        low.reservation_reads = vec![ReservationRef::new(
            DEMAND_PLUGIN,
            "low-request",
            "transport",
        )];
        low.visibility = StateVisibility::NextBoundary;
        registrar.register_boundary_system(low, record_low_grant)?;

        let mut validator = BoundarySystemContract::new(
            "validate-visibility",
            BoundaryPhase::InvariantValidation,
            SystemCadence::Daily,
        );
        validator.reads = vec![high_state(), low_state()];
        registrar.register_boundary_system(validator, validate_visibility)
    }
}

fn forecast_stream() -> RandomStreamKey {
    RandomStreamKey::new(FORECAST_PLUGIN, "daily", 1)
}

fn forecast_state() -> StateKey {
    StateKey::new("fixture.forecast", "daily")
}

fn draw_forecast(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let value = view.random_range(&forecast_stream(), 100, "daily public fixture forecast")?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::SetComponent {
            state: forecast_state(),
            entity: EntityRef::Territory(western_territory()),
            component: "value".to_owned(),
            value: Value::from(value),
            summary: "Record a scoped deterministic forecast".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

struct ForecastPlugin;

impl SimulationPlugin for ForecastPlugin {
    fn name(&self) -> &'static str {
        FORECAST_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000105"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut forecast = BoundarySystemContract::new(
            "draw",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        forecast.writes = vec![forecast_state()];
        forecast.random_streams = vec![forecast_stream()];
        forecast.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(forecast, draw_forecast)
    }
}

fn failure_stream() -> RandomStreamKey {
    RandomStreamKey::new("fixture-failure", "rollback", 1)
}

fn fail_second_boundary(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if context.boundary_id.get() != 2 {
        return Ok(BoundaryProposal::default());
    }
    let _ = view.random_range(&failure_stream(), 100, "rollback evidence")?;
    Ok(BoundaryProposal {
        directives: vec![
            BoundaryDirective::SetComponent {
                state: StateKey::new("fixture.failure", "state"),
                entity: EntityRef::Army(army()),
                component: "staged".to_owned(),
                value: Value::Bool(true),
                summary: "Stage a value before validation fails".to_owned(),
            },
            BoundaryDirective::SetComponent {
                state: StateKey::new("fixture.failure", "state"),
                entity: EntityRef::Army(ArmyId::new(999)),
                component: "invalid".to_owned(),
                value: Value::Bool(true),
                summary: "Reference an unavailable entity".to_owned(),
            },
        ],
        ..BoundaryProposal::default()
    })
}

struct FailurePlugin;

impl SimulationPlugin for FailurePlugin {
    fn name(&self) -> &'static str {
        "fixture-failure"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000106"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut failure = BoundarySystemContract::new(
            "fail-second-boundary",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        failure.writes = vec![StateKey::new("fixture.failure", "state")];
        failure.random_streams = vec![failure_stream()];
        failure.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(failure, fail_second_boundary)
    }
}

fn authority(actor: PersonId) -> CommandAuthority {
    CommandAuthority {
        decision_origin: DecisionOrigin::Actor { actor },
        seat_id: None,
        permission_profile_id: None,
        command_subject: Some(EntityRef::Army(army())),
    }
}

fn authorization_envelope(actor: PersonId) -> CommandEnvelope {
    CommandEnvelope::new(
        Issuer::Actor(actor),
        Command::Plugin {
            plugin: GOVERNANCE_PLUGIN.to_owned(),
            command: "authorize_operation".to_owned(),
            payload: json!({ "army": army() }),
        },
    )
    .with_authority(authority(actor))
}

fn request(id: u64, revision: u64, envelope: CommandEnvelope) -> CommandRequest {
    CommandRequest::new(CommandRequestId::new(id), revision, envelope)
}

fn component<'a>(
    snapshot: &'a SimulationSnapshot,
    state: &StateKey,
    component: &str,
) -> Option<&'a Value> {
    snapshot
        .plugin_components
        .iter()
        .find(|record| &record.state == state && record.component == component)
        .map(|record| &record.value)
}

#[test]
fn representative_public_contracts_are_atomic_replayable_and_binding_safe() {
    let scenario = fixture_scenario();
    let governance = GovernancePlugin;
    let appointments = AppointmentPlugin;
    let supply = SupplyPlugin;
    let demand = DemandPlugin;
    let forecast = ForecastPlugin;
    let plugins: [&dyn SimulationPlugin; 5] =
        [&governance, &appointments, &supply, &demand, &forecast];
    let mut canwu = Canwu::new_with_plugins(77, scenario.clone(), &plugins)
        .expect("the representative package set should register through public APIs");

    let rejected = canwu
        .process_command(request(
            1,
            canwu.revision(),
            authorization_envelope(observer()),
        ))
        .expect("an authority rejection is structured evidence");
    let CommandOutcome::Rejected { rejection } = rejected else {
        panic!("the observer must not receive commander authority");
    };
    assert_eq!(rejection.error.code, ErrorCode::InvalidAuthority);
    assert_eq!(canwu.revision(), 1);

    let accepted = canwu
        .process_command(request(
            2,
            canwu.revision(),
            authorization_envelope(commander()),
        ))
        .expect("the accountable commander should be admitted");
    assert!(matches!(accepted, CommandOutcome::Accepted { .. }));

    let movement = CommandEnvelope::new(
        Issuer::Actor(commander()),
        Command::OrderMovement {
            subject: EntityRef::Army(army()),
            destination: eastern_territory(),
            cargo: Vec::new(),
        },
    )
    .with_authority(authority(commander()));
    let movement = canwu
        .process_command(request(3, canwu.revision(), movement))
        .expect("the commanded movement should be admitted");
    assert!(matches!(movement, CommandOutcome::Accepted { .. }));
    canwu
        .advance(SimDuration::days(1))
        .expect("scheduled movement should complete deterministically");

    let commander_view = canwu
        .knowledge()
        .for_actor(commander())
        .expect("the commander knowledge ledger should be available");
    let observer_view = canwu
        .knowledge()
        .for_actor(observer())
        .expect("the observer knowledge ledger should be available");
    assert_eq!(
        commander_view.armies[&army()].known_location,
        Some(eastern_territory())
    );
    assert_eq!(
        observer_view.armies[&army()].known_location,
        Some(central_territory())
    );

    let first = canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()).with_cadence(SystemCadence::Daily))
        .expect("the first representative boundary should settle");
    assert_eq!(first.allocations.len(), 2);
    assert_eq!(first.allocations[0].granted, 7);
    assert_eq!(
        first.allocations[0].disposition,
        ReservationDisposition::Fulfilled
    );
    assert_eq!(first.allocations[1].granted, 3);
    assert_eq!(
        first.allocations[1].disposition,
        ReservationDisposition::Partial
    );
    assert!(first.record_change_count >= 2);

    let office = canwu
        .typed_domain_record(&office_reference())
        .expect("the governance package should create its typed entity");
    assert!(matches!(office.lifecycle, DomainRecordLifecycle::Active));
    assert_eq!(
        office
            .decode_payload::<Office>()
            .expect("the office payload should decode"),
        OfficePayload {
            name: "Council Office".to_owned(),
        }
    );
    let appointment = canwu
        .typed_domain_record(&appointment_reference())
        .expect("the conditional transition should create its typed record");
    assert_eq!(
        appointment
            .decode_payload::<Appointment>()
            .expect("the appointment payload should decode"),
        AppointmentPayload {
            status: "active".to_owned(),
        }
    );

    let second = canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()).with_cadence(SystemCadence::Daily))
        .expect("the second boundary should expose deferred state as current");
    assert_eq!(second.allocations[0].granted, 7);
    assert_eq!(second.allocations[1].granted, 3);
    assert_eq!(canwu.random_draws().len(), 3);
    let forecast_draws: Vec<_> = canwu
        .random_draws()
        .iter()
        .filter(|draw| draw.stream == forecast_stream())
        .collect();
    assert_eq!(forecast_draws.len(), 2);
    assert!(forecast_draws.iter().all(|draw| matches!(
        &draw.producer,
        RandomDrawProducer::BoundarySystem { plugin, system, .. }
            if plugin == FORECAST_PLUGIN && system == "draw"
    )));
    let settled = canwu.snapshot();
    assert_eq!(
        component(&settled, &high_state(), "granted"),
        Some(&json!(7))
    );
    assert_eq!(
        component(&settled, &low_state(), "granted"),
        Some(&json!(3))
    );
    assert_eq!(
        component(&settled, &authorization_state(), "status"),
        Some(&json!("approved"))
    );

    let snapshot_json = canwu
        .snapshot_json()
        .expect("the representative run should serialize");
    let restored = Canwu::from_snapshot_json_with_plugins(&snapshot_json, &plugins)
        .expect("the exact package environment should restore the snapshot");
    assert_eq!(canwu.snapshot(), restored.snapshot());

    let journal = canwu.replay_journal();
    let replayed = Canwu::replay_from_journal(scenario.clone(), &plugins, &journal)
        .expect("the exact public journal should replay");
    assert_eq!(canwu.snapshot(), replayed.snapshot());

    let original = canwu.snapshot();
    let mut fork = canwu.fork();
    let fork_movement = CommandEnvelope::new(
        Issuer::Actor(commander()),
        Command::OrderMovement {
            subject: EntityRef::Army(army()),
            destination: central_territory(),
            cargo: Vec::new(),
        },
    )
    .with_authority(authority(commander()));
    let fork_outcome = fork
        .process_command(request(4, fork.revision(), fork_movement))
        .expect("the fork should accept an independent continuation");
    assert!(matches!(fork_outcome, CommandOutcome::Accepted { .. }));
    assert_eq!(canwu.snapshot(), original);
    assert_ne!(fork.checkpoint_hash(), canwu.checkpoint_hash());

    let mut tampered = canwu.snapshot();
    tampered
        .plugin_components
        .first_mut()
        .expect("the fixture records plugin state")
        .value = Value::String("tampered".to_owned());
    let tampered_json = serde_json::to_string(&tampered).expect("the tampered value should encode");
    let Err(tamper_error) = Canwu::from_snapshot_json_with_plugins(&tampered_json, &plugins) else {
        panic!("a modified committed domain must be rejected");
    };
    assert_eq!(tamper_error.code, ErrorCode::InvalidSnapshot);

    let changed_governance = ChangedGovernancePlugin;
    let changed_plugins: [&dyn SimulationPlugin; 5] = [
        &changed_governance,
        &appointments,
        &supply,
        &demand,
        &forecast,
    ];
    let Err(package_error) =
        Canwu::from_snapshot_json_with_plugins(&snapshot_json, &changed_plugins)
    else {
        panic!("a package with changed executable semantics must be rejected");
    };
    assert_eq!(package_error.code, ErrorCode::PluginManifestMismatch);

    let failure = FailurePlugin;
    let rollback_plugins: [&dyn SimulationPlugin; 6] = [
        &governance,
        &appointments,
        &supply,
        &demand,
        &forecast,
        &failure,
    ];
    let mut rollback = Canwu::new_with_plugins(77, scenario, &rollback_plugins)
        .expect("the rollback package set should register");
    rollback
        .settle_boundary(BoundaryRequest::at(rollback.time()).with_cadence(SystemCadence::Daily))
        .expect("the first rollback boundary should settle");
    let before_failure = rollback.snapshot();
    let error = rollback
        .settle_boundary(BoundaryRequest::at(rollback.time()).with_cadence(SystemCadence::Daily))
        .expect_err("the invalid staged entity should abort the complete boundary");
    assert_eq!(error.code, ErrorCode::EntityNotFound);
    assert_eq!(rollback.snapshot(), before_failure);
}
