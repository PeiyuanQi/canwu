//! Public-API fixture for downstream gap G-64 and engine finding E-44 of the
//! `canwu-movement` lifecycle plugin.
//!
//! G-64: `DelayLeg` holds a departed leg by replacing its arrival due. A hold
//! is set by the owner, shortened by the operator, set again by an incident,
//! and lifted; a hold that would end before the boundary or before the
//! arrival scheduled at departure, or that names a leg that is not the
//! current one, is a recorded rejection and never fails the boundary. The owner and
//! operator see each hold in their reports; a delayed remote observer learns
//! of it only from the arrival time. A later leg whose booking expires while
//! the leg is held fails through the recorded `capacity_unavailable` failure,
//! and a pending hold survives save/load and exact replay.
//!
//! E-44: incident evidence must be the current version of its record. A
//! superseded version is refused alike by a sealed run, which no longer holds
//! its evidence, by its unsealed twin, which does, and by exact replay.

#![allow(clippy::too_many_lines)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, CanwuRun, CapacityBooking, CapacityBookingId,
    CapacityBookingStatus, CommandAttemptOutcome, CommandEnvelope, CommandRequest,
    CommandRequestId, DomainRecord, DomainRecordClass, DomainRecordDraft, DomainRecordKind,
    DomainRecordLifecycle, DomainRecordMutation, DomainRecordRef, DomainRecordSchema,
    DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource, DomainReference,
    DomainReferenceSchema, DomainReferenceTarget, DomainReferenceTargetKind, DomainValueKindClass,
    EntityRef, IngressClass, IngressPayload, Issuer, ItineraryRevision, ItineraryRevisionId,
    ItineraryRevisionReason, KnowledgeHistoryView, KnowledgeHolderRef, KnowledgeQuery,
    LegExecutionId, LegExecutionStatus, MAX_KNOWLEDGE_PAGE_SIZE, MovementInitiative, MovementOrder,
    MovementOrderId, MovementSubject, MovementSubjectRole, PayloadSchema, PersonId,
    PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar, ROUTING_ALGORITHM_VERSION,
    RouteCost, RouteLeg, RoutePlan, RoutingConnectionRef, RoutingNodeRef, Scenario, SimDuration,
    SimTime, SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence,
    TransferMode, TransportExecutionId, TransportExecutionState, TypedDomainRecordRef,
};
use canwu_movement::{
    MOVEMENT_GRANTEE_ROLE, MOVEMENT_SUBJECT_ROLE, MovementBookingRequestV1, MovementCommandV1,
    MovementErrorCode, MovementIncidentV1, MovementLegDueKind, MovementLegHoldV1,
    MovementOperation, MovementOperationDisposition, MovementOperationOutcomeV1,
    MovementOperationScopeV1, MovementOrderRequestV1, MovementPhaseV1, MovementPlugin,
    MovementPoolOfferV1, MovementRemoteObserverV1, MovementReportV1, MovementState,
    movement_command, movement_incident_ingress, movement_report_knowledge_schema_id,
    movement_state,
};

const WORLD: &str = "test.world";
const WORLD_PLUGIN: &str = "test-world";
const REVISE_INCIDENT_INGRESS: &str = "revise_incident_v1";
const POOL: &str = "pool.ferry.b-c";

macro_rules! world_kind {
    ($marker:ident, $name:literal) => {
        struct $marker;
        impl DomainRecordType for $marker {
            type Payload = serde_json::Value;
            type Class = DomainValueKindClass;
            const NAMESPACE: &'static str = WORLD;
            const NAME: &'static str = $name;
        }
    };
}

world_kind!(Incident, "incident");
world_kind!(Warrant, "warrant");

fn incident_reference() -> TypedDomainRecordRef<Incident> {
    TypedDomainRecordRef::new("ambush-1")
}

fn incident_state_key() -> StateKey {
    DomainRecordSchema::for_record::<Incident>().state_key()
}

/// Owner of the application records the movement plugin cites. Each
/// `revise_incident_v1` packet makes a new version of the incident record,
/// which supersedes the version an earlier incident report cited.
struct WorldPlugin;

impl SimulationPlugin for WorldPlugin {
    fn name(&self) -> &'static str {
        WORLD_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000c2"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut warrant = DomainRecordSchema::for_record::<Warrant>();
        warrant.references = [
            (MOVEMENT_GRANTEE_ROLE, false),
            (MOVEMENT_SUBJECT_ROLE, true),
        ]
        .into_iter()
        .map(|(role, multiple)| DomainReferenceSchema {
            role: role.to_owned(),
            targets: vec![DomainReferenceTargetKind::AnyEntity],
            required: false,
            multiple,
            allow_retired: false,
        })
        .collect();
        for mut schema in [DomainRecordSchema::for_record::<Incident>(), warrant] {
            schema.payload_schema = PayloadSchema::Any;
            registrar.register_record_schema(schema)?;
        }
        registrar.register_ingress(PluginIngressDescriptor {
            name: REVISE_INCIDENT_INGRESS.to_owned(),
            description: "Revise the incident record as a new version".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut revise = BoundarySystemContract::new(
            "revise-incident-v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        revise.reads = vec![StateKey::core_ingress(), incident_state_key()];
        revise.writes = vec![incident_state_key()];
        revise.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(revise, revise_incident)
    }
}

fn revise_incident(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut revisions = 0_u64;
    for ingress_id in &context.admitted_ingress {
        if let Some(ingress) = view.ingress(*ingress_id)?
            && let IngressPayload::Plugin {
                plugin,
                packet_type,
                ..
            } = &ingress.payload
            && plugin == WORLD_PLUGIN
            && packet_type == REVISE_INCIDENT_INGRESS
        {
            revisions += 1;
        }
    }
    if revisions == 0 {
        return Ok(BoundaryProposal::default());
    }
    let record = view
        .typed_domain_record(&incident_reference())?
        .expect("incident record")
        .clone();
    let payload = serde_json::json!({ "id": "ambush-1", "revision": record.version + revisions });
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: DomainRecordDraft::from_typed(incident_reference(), &payload)?,
                expected_version: record.version,
            },
            summary: "Revise the incident record".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn t(minutes: i64) -> SimTime {
    SimTime::from_minutes(minutes)
}

fn holder(person: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(person))
}

fn world_record(name: &str, id: &str) -> DomainRecord {
    DomainRecord {
        reference: DomainRecordRef::new(WORLD, name, id),
        owner: WORLD_PLUGIN.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::json!({ "id": id }),
        references: Vec::new(),
    }
}

fn initial_version(name: &str, id: &str) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new(WORLD, name, id),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

/// Person 1's warrant to move persons 5, 6, and 7.
fn warrant() -> DomainRecord {
    let person = |id| DomainReferenceTarget::Core(EntityRef::Person(PersonId::new(id)));
    let mut record = world_record("warrant", "warrant-1");
    record.references = [
        (MOVEMENT_GRANTEE_ROLE, 1),
        (MOVEMENT_SUBJECT_ROLE, 5),
        (MOVEMENT_SUBJECT_ROLE, 6),
        (MOVEMENT_SUBJECT_ROLE, 7),
    ]
    .into_iter()
    .map(|(role, id)| DomainReference {
        role: role.to_owned(),
        target: person(id),
    })
    .collect();
    record.references.sort();
    record
}

fn scenario() -> Scenario {
    let people = (1..=7)
        .map(|person| EntityRef::Person(PersonId::new(person)))
        .collect();
    Scenario::new(t(0), people)
        .with_domain_records(vec![world_record("incident", "ambush-1"), warrant()])
}

fn movement_plugin() -> MovementPlugin {
    MovementPlugin::new([
        DomainRecordKind::for_type::<Incident>(),
        DomainRecordKind::for_type::<Warrant>(),
    ])
}

/// A plan through `stops` from `start`, each leg taking `minutes`.
fn plan(stops: &[&str], start: i64, minutes: i64) -> RoutePlan {
    let legs = stops
        .windows(2)
        .zip(0_i64..)
        .map(|(pair, index)| RouteLeg {
            connection: RoutingConnectionRef::new(format!("{}-{}", pair[0], pair[1])),
            from: RoutingNodeRef::new(pair[0]),
            to: RoutingNodeRef::new(pair[1]),
            mode: TransferMode::Foot,
            planned_departure_at: t(start + index * minutes),
            planned_arrival_at: t(start + (index + 1) * minutes),
        })
        .collect::<Vec<_>>();
    let arrival = legs.last().unwrap().planned_arrival_at;
    RoutePlan {
        algorithm_version: ROUTING_ALGORITHM_VERSION.to_owned(),
        policy_version: "fixture.policy.v1".to_owned(),
        planning_snapshot_digest: "fixture-snapshot".to_owned(),
        edge_conditions_digest: None,
        origin: RoutingNodeRef::new(stops[0]),
        destination: RoutingNodeRef::new(*stops.last().unwrap()),
        departure_at: t(start),
        estimated_arrival_at: arrival,
        cost: RouteCost {
            estimated_arrival_at: arrival,
            risk_per_mille: 0,
            resource_cost: 0,
            transfers: 0,
        },
        legs,
        digest: stops.join(">"),
    }
}

/// An order by person 1 at time 0 moving `traveller` along `route` under the
/// warrant, operated by person 2 and watched by person 3 thirty minutes late.
fn order(id: u64, traveller: u64, route: RoutePlan) -> MovementOperation {
    MovementOperation::Order(MovementOrderRequestV1 {
        execution: TransportExecutionId(id),
        order: MovementOrder {
            id: MovementOrderId(id),
            subjects: vec![MovementSubject {
                entity: EntityRef::Person(PersonId::new(traveller)),
                role: MovementSubjectRole::MovablePrincipal,
                quantity: None,
                expected_custody: None,
            }],
            origin: route.origin.clone(),
            destination: route.destination.clone(),
            plan: route,
            initiative: MovementInitiative::Commanded,
            ordered_at: t(0),
            expected_position_revision: 1,
        },
        delivery_attempt: None,
        operator: Some(holder(2)),
        remote_observers: vec![MovementRemoteObserverV1 {
            holder: holder(3),
            delay: SimDuration::minutes(30),
        }],
        authority_basis: Some(initial_version("warrant", "warrant-1")),
    })
}

fn delay(execution: u64, leg: u64, until: i64, reason: &str) -> MovementOperation {
    MovementOperation::DelayLeg {
        execution: TransportExecutionId(execution),
        leg: LegExecutionId(leg),
        until: t(until),
        reason: reason.to_owned(),
        retime_later_legs: false,
    }
}

fn report_incident<H: CanwuRun + ?Sized>(
    run: &mut H,
    key: &str,
    evidence: DomainRecordVersionRef,
    operation: MovementOperation,
) {
    let at = run.time();
    run.enqueue_plugin_ingress(
        movement_incident_ingress(
            &MovementIncidentV1 {
                operation_key: key.to_owned(),
                evidence,
                operation,
            },
            at,
        )
        .unwrap(),
    )
    .unwrap();
}

fn incident_outcome<H: CanwuRun + ?Sized>(run: &H, key: &str) -> MovementOperationOutcomeV1 {
    movement_state(run)
        .unwrap()
        .unwrap()
        .1
        .operation_outcome(
            &MovementOperationScopeV1::EvidenceKind(DomainRecordKind::for_type::<Incident>()),
            key,
        )
        .unwrap()
        .unwrap()
        .clone()
}

fn current_incident<H: CanwuRun + ?Sized>(run: &H) -> DomainRecordVersionRef {
    run.current_domain_record_version(incident_reference().as_untyped())
        .unwrap()
        .unwrap()
}

/// Makes a new version of the incident record in a boundary at `now`.
fn revise_incident_now(canwu: &mut Canwu) {
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            WORLD_PLUGIN,
            REVISE_INCIDENT_INGRESS,
            canwu.time(),
            serde_json::json!({}),
        ))
        .unwrap();
    canwu.advance_canonical(SimDuration::ZERO).unwrap();
}

/// A tracked movement command by `person` at `at`.
fn movement_request(
    id: u64,
    revision: u64,
    at: SimTime,
    person: u64,
    key: &str,
    operation: MovementOperation,
) -> CommandRequest {
    CommandRequest::new(
        CommandRequestId::new(id),
        revision,
        CommandEnvelope::new(
            Issuer::Actor(PersonId::new(person)),
            movement_command(&MovementCommandV1 {
                holder: holder(person),
                operation_key: key.to_owned(),
                operation,
            })
            .unwrap(),
        )
        .at_time(at),
    )
}

fn command_outcome<H: CanwuRun + ?Sized>(
    run: &H,
    person: u64,
    key: &str,
) -> MovementOperationOutcomeV1 {
    movement_state(run)
        .unwrap()
        .unwrap()
        .1
        .operation_outcome(&MovementOperationScopeV1::Holder(holder(person)), key)
        .unwrap()
        .unwrap()
        .clone()
}

struct Host {
    canwu: Canwu,
    next_request: u64,
}

impl Host {
    /// Admits one tracked command and settles its boundary, which must not
    /// fail whatever the operation's outcome.
    fn accept(&mut self, person: u64, key: &str, operation: MovementOperation) {
        self.next_request += 1;
        let now = self.canwu.time();
        let request = movement_request(
            self.next_request,
            self.canwu.revision(),
            now,
            person,
            key,
            operation,
        );
        self.canwu.enqueue_command(now, 0, request).unwrap();
        self.canwu.advance_canonical(SimDuration::ZERO).unwrap();
        let outcome = &self.canwu.command_attempts().last().unwrap().outcome;
        assert!(
            matches!(outcome, CommandAttemptOutcome::Accepted { .. }),
            "{key}: {outcome:?}"
        );
    }

    fn incident(
        &mut self,
        key: &str,
        evidence: DomainRecordVersionRef,
        operation: MovementOperation,
    ) {
        report_incident(&mut self.canwu, key, evidence, operation);
        self.canwu.advance_canonical(SimDuration::ZERO).unwrap();
    }

    fn run_until(&mut self, until: i64) {
        let duration = t(until).checked_sub(self.canwu.time()).unwrap();
        self.canwu.advance_canonical(duration).unwrap();
    }

    fn state(&self) -> MovementState {
        movement_state(&self.canwu).unwrap().unwrap().1
    }

    fn outcome(&self, person: u64, key: &str) -> MovementOperationOutcomeV1 {
        command_outcome(&self.canwu, person, key)
    }

    fn hold(&self, execution: u64) -> Option<MovementLegHoldV1> {
        self.state()
            .order_for_execution(TransportExecutionId(execution))
            .unwrap()
            .hold
            .clone()
    }

    fn arrival_due(&self, execution: u64) -> SimTime {
        self.state()
            .order_for_execution(TransportExecutionId(execution))
            .unwrap()
            .pending_due
            .as_ref()
            .unwrap()
            .due_at
    }
}

fn rejected(outcome: &MovementOperationOutcomeV1) -> Option<MovementErrorCode> {
    assert_eq!(outcome.disposition, MovementOperationDisposition::Rejected);
    outcome.rejection_code
}

fn applied(outcome: &MovementOperationOutcomeV1) {
    assert_eq!(
        outcome.disposition,
        MovementOperationDisposition::Applied,
        "{outcome:?}"
    );
}

/// The movement reports a person holds about one execution, oldest first.
fn reports(canwu: &Canwu, person: u64, execution: u64) -> Vec<MovementReportV1> {
    canwu
        .viewer_for_actor(PersonId::new(person))
        .unwrap()
        .query_knowledge(&KnowledgeQuery {
            schemas: vec![movement_report_knowledge_schema_id()],
            view: KnowledgeHistoryView::FullHistory,
            limit: MAX_KNOWLEDGE_PAGE_SIZE,
            ..KnowledgeQuery::default()
        })
        .unwrap()
        .records
        .into_iter()
        .map(|record| serde_json::from_value::<MovementReportV1>(record.payload).unwrap())
        .filter(|report| report.execution == TransportExecutionId(execution))
        .collect()
}

/// Each published hold as (report time, held until, scheduled arrival). A
/// report republished for another change, such as a booking expiring, shows
/// the same hold again and is counted once.
fn published_holds(reports: &[MovementReportV1]) -> Vec<(SimTime, SimTime, SimTime)> {
    let mut holds = reports
        .iter()
        .filter_map(|report| {
            let hold = report.detail.as_ref()?.hold.as_ref()?;
            Some((report.observed_as_of, hold.until, hold.scheduled_at))
        })
        .collect::<Vec<_>>();
    holds.dedup();
    holds
}

#[test]
fn gap_g64_movement_delay_leg() {
    let movement = movement_plugin();
    let plugins: [&dyn SimulationPlugin; 2] = [&movement, &WorldPlugin];
    let mut host = Host {
        canwu: Canwu::new_with_plugins(64, scenario(), &plugins).unwrap(),
        next_request: 0,
    };

    // Two movements a > b > c, ten minutes a leg from minute 10. The second
    // books its b > c crossing for the leg's planned window, 20 to 30.
    host.accept(
        1,
        "pool",
        MovementOperation::OfferPool(MovementPoolOfferV1 {
            pool: POOL.to_owned(),
            resource: "ferry_crossing".to_owned(),
            window_from: t(0),
            window_until: t(400),
            quantity: 1,
        }),
    );
    host.accept(1, "order-1", order(1, 5, plan(&["a", "b", "c"], 10, 10)));
    host.accept(1, "order-2", order(2, 6, plan(&["a", "b", "c"], 10, 10)));
    host.accept(1, "order-3", order(3, 7, plan(&["a", "b", "c"], 10, 10)));
    host.accept(
        2,
        "book-2",
        MovementOperation::RequestBooking(MovementBookingRequestV1 {
            booking: CapacityBooking::new(
                CapacityBookingId(1),
                TransportExecutionId(2),
                "ferry_crossing".to_owned(),
                t(20),
                t(30),
                1,
                0,
            )
            .unwrap(),
            pool: POOL.to_owned(),
            leg: LegExecutionId(2),
            tie_break: "execution-2".to_owned(),
            replaces: None,
        }),
    );

    // A leg that is not the current one cannot be held.
    host.accept(1, "unstarted", delay(1, 2, 40, "ford in flood"));
    assert_eq!(
        rejected(&host.outcome(1, "unstarted")),
        Some(MovementErrorCode::InvalidState)
    );

    // Both first legs depart at 10 and are scheduled to arrive at 20.
    host.run_until(12);
    assert_eq!(host.arrival_due(1), t(20));

    // Refusals are recorded and leave the boundary standing: a hold ending
    // before the boundary or before the scheduled arrival, a leg that is not
    // the departed one, an unbounded reason, and a person without a grant.
    host.accept(1, "past", delay(1, 1, 11, "ford in flood"));
    host.accept(1, "faster", delay(1, 1, 15, "ford in flood"));
    host.accept(1, "next-leg", delay(1, 2, 40, "ford in flood"));
    host.accept(1, "long-reason", delay(1, 1, 40, &"x".repeat(257)));
    host.accept(4, "intrude", delay(1, 1, 40, "ford in flood"));
    for (person, key, code) in [
        (1, "past", MovementErrorCode::InvalidOperation),
        (1, "faster", MovementErrorCode::InvalidOperation),
        (1, "next-leg", MovementErrorCode::InvalidState),
        (1, "long-reason", MovementErrorCode::InvalidOperation),
        (4, "intrude", MovementErrorCode::Unauthorized),
    ] {
        assert_eq!(rejected(&host.outcome(person, key)), Some(code), "{key}");
    }
    assert_eq!(host.arrival_due(1), t(20));
    assert_eq!(host.hold(1), None);

    // The owner holds the first leg until 40 and the second until 60.
    host.accept(1, "hold-1", delay(1, 1, 40, "ford in flood"));
    host.accept(1, "hold-2", delay(2, 1, 60, "escort delayed"));
    host.accept(1, "hold-3", delay(3, 1, 50, "axle broken"));
    assert_eq!(host.arrival_due(1), t(40));
    assert_eq!(
        host.hold(1),
        Some(MovementLegHoldV1 {
            leg: LegExecutionId(1),
            kind: MovementLegDueKind::Arrival,
            scheduled_at: t(20),
            until: t(40),
            reason: "ford in flood".to_owned(),
            held_at: t(12),
            retime_later_legs: false,
        })
    );

    // A held leg is still under way, so it cannot be rerouted.
    host.accept(
        1,
        "reroute",
        MovementOperation::Reroute {
            execution: TransportExecutionId(1),
            revision: ItineraryRevision {
                id: ItineraryRevisionId(2),
                predecessor: Some(ItineraryRevisionId(1)),
                plan: plan(&["a", "d", "c"], 12, 10),
                planned_at: t(12),
                valid_from: t(12),
                reason: ItineraryRevisionReason::Disaster {
                    explanation: "ford in flood".to_owned(),
                },
                superseded_at: None,
                evidence: Vec::new(),
            },
        },
    );
    assert_eq!(
        rejected(&host.outcome(1, "reroute")),
        Some(MovementErrorCode::InvalidState)
    );

    // A held leg can still fail: the failure clears the hold, and the
    // interrupted movement can then be rerouted from where it stopped.
    host.accept(
        2,
        "fail-held",
        MovementOperation::FailLeg {
            execution: TransportExecutionId(3),
            leg: LegExecutionId(1),
            reason: "axle beyond repair".to_owned(),
        },
    );
    applied(&host.outcome(2, "fail-held"));
    assert_eq!(host.hold(3), None);
    assert_eq!(
        host.state().executions[&TransportExecutionId(3)].state,
        TransportExecutionState::ReplanPending
    );
    host.accept(
        1,
        "reroute-3",
        MovementOperation::Reroute {
            execution: TransportExecutionId(3),
            revision: ItineraryRevision {
                id: ItineraryRevisionId(2),
                predecessor: Some(ItineraryRevisionId(1)),
                plan: plan(&["b", "c"], 15, 10),
                planned_at: t(12),
                valid_from: t(15),
                reason: ItineraryRevisionReason::Recovery {
                    explanation: "fresh cart".to_owned(),
                },
                superseded_at: None,
                evidence: Vec::new(),
            },
        },
    );
    applied(&host.outcome(1, "reroute-3"));

    // The operator shortens the first hold; only the latest hold is kept.
    host.run_until(14);
    host.accept(2, "shorten", delay(1, 1, 30, "ford falling"));
    applied(&host.outcome(2, "shorten"));
    let hold = host.hold(1).unwrap();
    assert_eq!(
        (hold.until, hold.reason.as_str(), hold.held_at),
        (t(30), "ford falling", t(14))
    );

    // An incident may hold a leg only on the current version of its
    // evidence: once the incident record is revised, its first version is
    // refused (though it opened the scenario) and the revision is accepted.
    host.run_until(16);
    revise_incident_now(&mut host.canwu);
    let revised = current_incident(&host.canwu);
    assert_eq!(revised.version, 2);
    host.incident(
        "superseded",
        initial_version("incident", "ambush-1"),
        delay(1, 1, 36, "bridge out"),
    );
    host.incident("bridge", revised, delay(1, 1, 36, "bridge out"));
    assert_eq!(
        rejected(&incident_outcome(&host.canwu, "superseded")),
        Some(MovementErrorCode::Evidence)
    );
    applied(&incident_outcome(&host.canwu, "bridge"));
    assert_eq!(host.arrival_due(1), t(36));

    // The owner lifts the hold: the leg arrives as scheduled at departure.
    host.run_until(18);
    host.accept(1, "lift", delay(1, 1, 20, "ford open"));
    applied(&host.outcome(1, "lift"));
    assert_eq!(host.arrival_due(1), t(20));

    // Once a hold has run past the scheduled arrival, a hold ending between
    // the scheduled arrival and now would schedule due work in the past: it
    // is a recorded rejection, not a failed boundary.
    host.run_until(30);
    host.accept(1, "behind-now", delay(2, 1, 25, "escort delayed"));
    assert_eq!(
        rejected(&host.outcome(1, "behind-now")),
        Some(MovementErrorCode::InvalidOperation)
    );
    assert_eq!(host.arrival_due(2), t(60));

    // The second hold is still pending when the run is saved.
    let mut restored =
        Canwu::from_snapshot_json_with_plugins(&host.canwu.snapshot_json().unwrap(), &plugins)
            .unwrap();
    host.run_until(45);
    restored
        .advance_canonical(t(45).checked_sub(restored.time()).unwrap())
        .unwrap();
    assert_eq!(movement_state(&restored).unwrap().unwrap().1, host.state());
    assert_eq!(restored.checkpoint_hash(), host.canwu.checkpoint_hash());
    assert_eq!(host.arrival_due(2), t(60));

    // The lifted movement ran to plan; the earlier holds' due packets were
    // withdrawn when each hold replaced them.
    let state = host.state();
    let one = &state.executions[&TransportExecutionId(1)];
    assert_eq!(one.state, TransportExecutionState::Settled);
    assert_eq!(
        one.legs
            .iter()
            .map(|leg| (leg.actual_departure_at, leg.actual_arrival_at))
            .collect::<Vec<_>>(),
        vec![(Some(t(10)), Some(t(20))), (Some(t(20)), Some(t(30)))]
    );
    // The rerouted movement left at 15 and settled at 25.
    let three = &state.executions[&TransportExecutionId(3)];
    assert_eq!(three.state, TransportExecutionState::Settled);
    assert_eq!(
        (
            three.legs[2].actual_departure_at,
            three.legs[2].actual_arrival_at
        ),
        (Some(t(15)), Some(t(25)))
    );

    // The held movement arrived at 60, after its next leg's booking window
    // closed, so the next leg failed through the recorded leg failure.
    host.run_until(65);
    let state = host.state();
    let two = &state.executions[&TransportExecutionId(2)];
    assert_eq!(two.legs[0].actual_arrival_at, Some(t(60)));
    assert_eq!(two.legs[1].status, LegExecutionStatus::Failed);
    assert_eq!(two.legs[1].failed_at, Some(t(60)));
    assert_eq!(
        two.legs[1].failure_reason.as_deref(),
        Some("capacity_unavailable")
    );
    assert_eq!(two.bookings[0].status, CapacityBookingStatus::Expired);
    assert_eq!(two.state, TransportExecutionState::ReplanPending);
    assert_eq!(host.hold(2), None);

    // The owner and operator saw every hold when it was set, and none after
    // the leg arrived.
    host.run_until(95);
    restored
        .advance_canonical(t(95).checked_sub(restored.time()).unwrap())
        .unwrap();
    assert_eq!(restored.checkpoint_hash(), host.canwu.checkpoint_hash());
    for person in [1, 2] {
        let one = reports(&host.canwu, person, 1);
        assert_eq!(
            published_holds(&one),
            vec![
                (t(12), t(40), t(20)),
                (t(14), t(30), t(20)),
                (t(16), t(36), t(20)),
                (t(18), t(20), t(20)),
            ]
        );
        let last = one.last().unwrap();
        assert_eq!(
            (last.phase, last.observed_as_of),
            (MovementPhaseV1::Settled, t(30))
        );
        assert_eq!(last.detail.as_ref().unwrap().hold, None);
        let two = reports(&host.canwu, person, 2);
        assert_eq!(published_holds(&two), vec![(t(12), t(60), t(20))]);
        assert_eq!(two.last().unwrap().detail.as_ref().unwrap().hold, None);
    }

    // The remote observer never saw a hold, though its view passed the
    // second hold's time at 42 while the leg was still held: its reports
    // change only when a leg departs, arrives, or fails, and the held leg's
    // late arrival is how it learns of the hold.
    for (execution, times) in [(1, vec![0, 10, 20, 30]), (2, vec![0, 10, 60])] {
        let remote = reports(&host.canwu, 3, execution);
        assert!(remote.iter().all(|report| report.detail.is_none()));
        assert_eq!(
            remote
                .iter()
                .map(|report| report.observed_as_of)
                .collect::<Vec<_>>(),
            times.into_iter().map(t).collect::<Vec<_>>()
        );
    }
    let remote = reports(&host.canwu, 3, 2);
    let last = remote.last().unwrap();
    assert_eq!(last.legs[0].arrived_at, Some(t(60)));
    assert_eq!(last.phase, MovementPhaseV1::Interrupted);

    // Exact replay reproduces every hold and refusal.
    let replayed = Canwu::replay_from_journal(&plugins, &host.canwu.replay_journal()).unwrap();
    assert_eq!(movement_state(&replayed).unwrap().unwrap().1, host.state());
    assert_eq!(replayed.checkpoint_hash(), host.canwu.checkpoint_hash());
}

/// E-44: a sealed run no longer holds the evidence of a superseded record
/// version while its unsealed twin and its exact replay still do, so an
/// incident or an order's delivery attempt citing one is refused by all of
/// them.
#[test]
fn e44_incident_evidence_is_current_alike_in_sealed_and_unsealed_runs() {
    let movement = movement_plugin();
    let plugins: [&dyn SimulationPlugin; 2] = [&movement, &WorldPlugin];
    let mut host = Host {
        canwu: Canwu::new_with_plugins(44, scenario(), &plugins).unwrap(),
        next_request: 0,
    };
    host.accept(1, "order-1", order(1, 5, plan(&["a", "b"], 1, 100)));
    host.run_until(2);

    // Version 2 is made by a boundary and superseded by version 3; nothing
    // live depends on it, so the seal drops its evidence.
    revise_incident_now(&mut host.canwu);
    let superseded = current_incident(&host.canwu);
    host.run_until(3);
    revise_incident_now(&mut host.canwu);
    let current = current_incident(&host.canwu);
    assert_eq!((superseded.version, current.version), (2, 3));
    for _ in 0..2 {
        let now = host.canwu.time();
        host.canwu
            .settle_boundary(BoundaryRequest::at(now))
            .unwrap();
    }
    let mut unsealed = host.canwu.fork();
    let mut sealed = host.canwu.into_compacted().unwrap();
    let segment = sealed.seal_evidence().unwrap().unwrap();
    assert!(unsealed.domain_record_version_evidence_exists(&superseded));
    assert!(!sealed.domain_record_version_evidence_exists(&superseded));

    // An order's delivery attempt follows the same rule: an order citing the
    // superseded version is a recorded rejection, one citing the current
    // version is applied, in both runs alike.
    let with_attempt = |id: u64, traveller: u64, attempt: &DomainRecordVersionRef| {
        let mut operation = order(id, traveller, plan(&["a", "b"], 1, 100));
        if let MovementOperation::Order(request) = &mut operation {
            request.delivery_attempt = Some(attempt.clone());
        }
        operation
    };
    for (id, key, operation) in [
        (101, "superseded-attempt", with_attempt(2, 6, &superseded)),
        (102, "current-attempt", with_attempt(3, 7, &current)),
    ] {
        let (at, revision) = (unsealed.time(), unsealed.revision());
        assert_eq!(
            (sealed.time(), sealed.revision()),
            (at, revision),
            "sealed and unsealed runs diverged before {key}"
        );
        let request = movement_request(id, revision, at, 1, key, operation);
        unsealed.enqueue_command(at, 0, request.clone()).unwrap();
        sealed.enqueue_command(at, 0, request).unwrap();
        unsealed.advance_canonical(SimDuration::ZERO).unwrap();
        sealed.advance_canonical(SimDuration::ZERO).unwrap();
    }
    for run in [&unsealed as &dyn CanwuRun, &sealed] {
        assert_eq!(
            rejected(&command_outcome(run, 1, "superseded-attempt")),
            Some(MovementErrorCode::Evidence)
        );
        applied(&command_outcome(run, 1, "current-attempt"));
    }

    // Every incident operation checks the same rule: a leg failure and a
    // hold citing the superseded version are refused, a hold citing the
    // current one is applied, in both runs alike.
    for (key, evidence, operation) in [
        (
            "superseded-fail",
            superseded.clone(),
            MovementOperation::FailLeg {
                execution: TransportExecutionId(1),
                leg: LegExecutionId(1),
                reason: "ambushed".to_owned(),
            },
        ),
        (
            "superseded-delay",
            superseded.clone(),
            delay(1, 1, 150, "ambushed"),
        ),
        (
            "current-delay",
            current.clone(),
            delay(1, 1, 150, "ambushed"),
        ),
    ] {
        report_incident(&mut unsealed, key, evidence.clone(), operation.clone());
        report_incident(&mut sealed, key, evidence, operation);
    }
    unsealed.advance_canonical(SimDuration::ZERO).unwrap();
    sealed.advance_canonical(SimDuration::ZERO).unwrap();
    for run in [&unsealed as &dyn CanwuRun, &sealed] {
        for key in ["superseded-fail", "superseded-delay"] {
            assert_eq!(
                rejected(&incident_outcome(run, key)),
                Some(MovementErrorCode::Evidence),
                "{key}"
            );
        }
        applied(&incident_outcome(run, "current-delay"));
    }
    assert_eq!(
        movement_state(&sealed).unwrap().unwrap().1,
        movement_state(&unsealed).unwrap().unwrap().1
    );
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());

    // The held leg arrives at 150 in both runs, and exact replay of the
    // sealed run equals both.
    unsealed
        .advance_canonical(SimDuration::minutes(150))
        .unwrap();
    sealed.advance_canonical(SimDuration::minutes(150)).unwrap();
    let state = movement_state(&unsealed).unwrap().unwrap().1;
    assert_eq!(
        state.executions[&TransportExecutionId(1)].legs[0].actual_arrival_at,
        Some(t(150))
    );
    assert_eq!(movement_state(&sealed).unwrap().unwrap().1, state);
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    let journal = sealed
        .replay_journal_with_segments(vec![segment.clone()])
        .unwrap();
    let replayed = Canwu::replay_from_journal(&plugins, &journal).unwrap();
    assert_eq!(
        replayed.snapshot(),
        sealed.snapshot_with_segments(vec![segment]).unwrap()
    );
    assert_eq!(replayed.snapshot(), unsealed.snapshot());
}
