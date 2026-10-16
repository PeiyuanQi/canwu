//! Gap G-49: a person can be created without a placement in the
//! `WorldSnapshot` projection.
//!
//! The first scenario has no government and no territory. A draft without a
//! placement creates a person that exists in the entity registry, has core
//! availability and custody, holds knowledge, and is bound to its creation
//! correlation, with no `Person` record. A draft cannot carry half a
//! placement, and a placement that names records the projection lacks fails
//! its boundary. Save/load, a sealed continuation, and exact replay reproduce
//! the person.
//!
//! In a populated projection, placed and unplaced creations coexist, and a
//! load refuses a `Person` record that no creation placed.

#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryCreatedPerson, BoundaryDirective, BoundaryId, BoundaryPhase,
    BoundaryProposal, BoundaryRequest, BoundarySystemContract, Canwu, CanwuError, Command,
    CommandAttemptOutcome, CommandAuthority, CommandEnvelope, CommandRequest, CommandRequestId,
    CreatedPerson, CustodyState, EntityRef, ErrorCode, EvidenceRef, GovernmentId, Issuer,
    KnowledgeHolderRef, KnowledgeOrigin, KnowledgeQuery, KnowledgeRecordDraft, KnowledgeRecordKind,
    KnowledgeSchemaId, KnowledgeSubject, KnowledgeSubjectSchema, KnowledgeSubjectTarget,
    KnowledgeSubjectTargetKind, KnowledgeWriteGrant, LifeState, OrganizationId, PayloadSchema,
    Person, PersonAvailability, PersonDraft, PersonId, PersonPlacement, PluginKnowledgeSchema,
    PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin, SimulationSnapshot,
    SimulationView, StateKey, StateVisibility, SystemCadence, TerritoryId, WorldSnapshot,
};
use serde_json::json;

const PLUGIN: &str = "fixture-household";
const WARD: &str = "ward";

const fn steward() -> PersonId {
    PersonId::new(1)
}

/// The first engine-allocated ID follows every scenario person.
const fn ward() -> PersonId {
    PersonId::new(2)
}

const fn household() -> EntityRef {
    EntityRef::Organization(OrganizationId::new(10))
}

fn lesson_schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(KnowledgeRecordKind::new("fixture.household", "lesson"), 1)
}

fn draft(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    placement: Option<PersonPlacement>,
) -> PersonDraft {
    PersonDraft {
        name: "Ward of the household".to_owned(),
        placement,
        roles: vec!["ward".to_owned()],
        availability: PersonAvailability::new(LifeState::Alive, CustodyState::Free, view.time()),
        provenance: EvidenceRef::Boundary(BoundaryId::new(context.boundary_id.get() - 1)),
    }
}

fn create(draft: PersonDraft, correlation: &str) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::CreatePerson {
            draft,
            correlation: correlation.to_owned(),
            summary: "The household took in a ward".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

/// Phase 7, `Daily`: creates the ward at boundary 2 with no placement.
fn take_in_ward(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if context.boundary_id.get() != 2 {
        return Ok(BoundaryProposal::default());
    }
    create(draft(view, context, None), WARD)
}

const fn first_government_and_territory() -> PersonPlacement {
    PersonPlacement {
        government: GovernmentId::new(1),
        current_location: TerritoryId::new(1),
    }
}

/// Phase 7, `Seasonal`: a placed draft, which needs its government and
/// territory to exist in the projection.
fn placed(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    create(
        draft(view, context, Some(first_government_and_territory())),
        "placed",
    )
}

fn bound_ward(view: &SimulationView<'_>) -> Result<Option<PersonId>, CanwuError> {
    Ok(view
        .persons_created_by_correlation(PLUGIN, WARD)?
        .first()
        .map(|created| created.person))
}

/// Phase 7, `Daily`: the household detains the ward at boundary 5.
fn confine_ward(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(ward) = bound_ward(view)?.filter(|_| context.boundary_id.get() == 5) else {
        return Ok(BoundaryProposal::default());
    };
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::SetPersonAvailability {
            person: ward,
            availability: PersonAvailability::new(
                LifeState::Alive,
                CustodyState::Detained,
                view.time(),
            )
            .with_custodian(household()),
            summary: "The household confined its ward".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

/// Phase 13, `Daily`: at boundary 4 the ward learns a lesson about itself.
/// The payload records whether the system saw a `Person` record for it.
fn teach_ward(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(ward) = bound_ward(view)?.filter(|_| context.boundary_id.get() == 4) else {
        return Ok(BoundaryProposal::default());
    };
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::PublishKnowledge {
            visibility: StateVisibility::SameBoundary,
            producer_correlation: None,
            supersession: canwu_api::KnowledgeSupersession::Explicit,
            groups: vec![canwu_api::KnowledgePublicationGroup {
                holders: vec![KnowledgeHolderRef::Person(ward)],
                records: vec![KnowledgeRecordDraft {
                    schema: lesson_schema(),
                    subjects: vec![KnowledgeSubject {
                        role: "pupil".to_owned(),
                        target: KnowledgeSubjectTarget::Entity(EntityRef::Person(ward)),
                    }],
                    payload: json!({ "has_person_record": view.person(ward)?.is_some() }),
                    as_of: None,
                    confidence_per_mille: 1_000,
                    origin: KnowledgeOrigin {
                        method: "fixture.lesson".to_owned(),
                        evidence: Vec::new(),
                    },
                    supersedes: Vec::new(),
                    contradicts: Vec::new(),
                }],
            }],
            summary: "The ward was taught".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

struct HouseholdPlugin;

impl SimulationPlugin for HouseholdPlugin {
    fn name(&self) -> &'static str {
        PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000004901"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: lesson_schema(),
            schema_hash: "0000000000000000000000000000000000000000000000000000000000004902"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: vec![KnowledgeSubjectSchema {
                role: "pupil".to_owned(),
                targets: vec![KnowledgeSubjectTargetKind::AnyEntity],
                required: true,
                multiple: false,
            }],
            retention: canwu_api::KnowledgeRetention::Keep,
        })?;
        for (name, cadence, handler) in [
            (
                "take-in",
                SystemCadence::Daily,
                take_in_ward as canwu_api::BoundarySystemHandler,
            ),
            ("placed", SystemCadence::Seasonal, placed),
        ] {
            let mut contract =
                BoundarySystemContract::new(name, BoundaryPhase::DomainDeltaProposal, cadence);
            contract.writes = vec![StateKey::core_people()];
            registrar.register_boundary_system(contract, handler)?;
        }
        let mut confine = BoundarySystemContract::new(
            "confine",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        confine.reads = vec![StateKey::core_people()];
        confine.writes = vec![StateKey::core_person_availability()];
        registrar.register_boundary_system(confine, confine_ward)?;
        let mut teach = BoundarySystemContract::new(
            "teach",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        teach.reads = vec![StateKey::core_people()];
        teach.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: lesson_schema(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        registrar.register_boundary_system(teach, teach_ward)
    }
}

fn at_day(day: i64, cadence: SystemCadence) -> BoundaryRequest {
    BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(day)).with_cadence(cadence)
}

/// The rejection the built-in movement command gives the ward, probed on a
/// fork so that the run under test is not changed.
fn movement_rejection(canwu: &Canwu) -> ErrorCode {
    let mut probe = canwu.fork();
    let now = probe.time();
    let envelope = CommandEnvelope::new(
        Issuer::Actor(ward()),
        Command::OrderMovement {
            subject: EntityRef::Person(ward()),
            destination: TerritoryId::new(1),
            cargo: Vec::new(),
        },
    )
    .with_authority(CommandAuthority::for_actor(ward()))
    .at_time(now);
    probe
        .enqueue_command(
            now,
            0,
            CommandRequest::new(CommandRequestId::new(1), probe.revision(), envelope),
        )
        .expect("command ingress");
    probe
        .step_canonical()
        .expect("command boundary")
        .expect("boundary");
    match &probe
        .command_attempts()
        .last()
        .expect("command attempt")
        .outcome
    {
        CommandAttemptOutcome::Rejected { error } => error.code.clone(),
        CommandAttemptOutcome::Accepted { .. } => panic!("the ward has nowhere to move from"),
    }
}

#[test]
fn gap_g49_sim_unplaced_person() {
    // A placed creation is recorded exactly as its `Person` record encodes,
    // so evidence and hashes of placed persons are unchanged; an unplaced
    // creation omits both placement fields.
    let placed = BoundaryCreatedPerson {
        id: PersonId::new(9),
        name: "Placed".to_owned(),
        government: Some(GovernmentId::new(1)),
        current_location: Some(TerritoryId::new(2)),
        roles: vec!["heir".to_owned()],
    };
    let record: Person = placed.person_record().expect("placed person record");
    assert_eq!(
        serde_json::to_string(&placed).expect("creation json"),
        serde_json::to_string(&record).expect("person json")
    );
    let unplaced = BoundaryCreatedPerson {
        id: ward(),
        name: "Ward of the household".to_owned(),
        government: None,
        current_location: None,
        roles: vec!["ward".to_owned()],
    };
    assert!(!unplaced.is_placed() && unplaced.person_record().is_none());
    assert_eq!(
        serde_json::to_value(&unplaced).expect("creation json"),
        json!({ "id": ward(), "name": "Ward of the household", "roles": ["ward"] })
    );

    // A draft carries a whole placement or none, at decoding too: half a
    // placement and the flat 0.14.0 shape fail to decode, so neither can
    // become an unplaced draft by accident.
    let placed_draft = PersonDraft {
        name: "Placed".to_owned(),
        placement: Some(first_government_and_territory()),
        roles: Vec::new(),
        availability: PersonAvailability::new(LifeState::Alive, CustodyState::Free, SimTime::EPOCH),
        provenance: EvidenceRef::Boundary(BoundaryId::new(1)),
    };
    let encoded = serde_json::to_value(&placed_draft).expect("draft json");
    assert_eq!(
        serde_json::from_value::<PersonDraft>(encoded.clone()).expect("placed draft"),
        placed_draft
    );
    let mut half = encoded.clone();
    half["placement"]
        .as_object_mut()
        .expect("placement object")
        .remove("current_location");
    assert!(serde_json::from_value::<PersonDraft>(half).is_err());
    let mut flat = encoded;
    let placement = flat
        .as_object_mut()
        .expect("draft object")
        .remove("placement")
        .expect("placement value");
    flat["government"] = placement["government"].clone();
    flat["current_location"] = placement["current_location"].clone();
    assert!(serde_json::from_value::<PersonDraft>(flat).is_err());

    let plugin = HouseholdPlugin;
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![household(), EntityRef::Person(steward())],
    );
    let mut canwu = Canwu::new_with_plugins(4949, scenario, &[&plugin]).expect("household run");
    canwu
        .settle_boundary(at_day(1, SystemCadence::Daily))
        .expect("first boundary");

    // The draft carries no placement, and the projection has no government
    // or territory: the person is created in the entity registry only.
    let receipt = canwu
        .settle_boundary(at_day(2, SystemCadence::Daily))
        .expect("an unplaced draft is admitted");
    assert_eq!(
        receipt.created_persons,
        vec![CreatedPerson {
            plugin: PLUGIN.to_owned(),
            system: "take-in".to_owned(),
            correlation: WARD.to_owned(),
            person: ward(),
        }]
    );
    assert_eq!(canwu.boundaries()[1].created_persons[0].person, unplaced);
    assert!(canwu.entity_exists(&EntityRef::Person(ward())));
    assert_eq!(canwu.world(), WorldSnapshot::default());
    assert_eq!(
        canwu.person_availability(ward()).map(|value| value.custody),
        Some(CustodyState::Free)
    );

    // A placed draft still needs records that exist. The refusal commits
    // nothing.
    let before = (canwu.revision(), canwu.checkpoint_hash().to_owned());
    let error = canwu
        .settle_boundary(at_day(2, SystemCadence::Seasonal))
        .expect_err("a placement in an empty projection fails its boundary");
    assert_eq!(error.code, ErrorCode::EntityNotFound);
    assert_eq!(
        (canwu.revision(), canwu.checkpoint_hash().to_owned()),
        before
    );

    // A cadence-free boundary admits the creation boundary's events, so its
    // evidence can be sealed before the next daily boundary.
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("event admission boundary");
    let before_lesson = canwu.fork();

    // The ward holds knowledge about itself, read through its own viewer.
    // The publishing system saw no `Person` record for it.
    canwu
        .settle_boundary(at_day(3, SystemCadence::Daily))
        .expect("the ward is a knowledge holder and subject");
    let lessons = canwu
        .viewer_for_actor(ward())
        .expect("ward viewer")
        .query_knowledge(&KnowledgeQuery::default())
        .expect("ward knowledge");
    assert_eq!(lessons.records.len(), 1);
    assert_eq!(
        lessons.records[0].payload,
        json!({ "has_person_record": false })
    );
    assert!(
        canwu
            .viewer_for_actor(steward())
            .expect("steward viewer")
            .query_knowledge(&KnowledgeQuery::default())
            .expect("steward knowledge")
            .records
            .is_empty()
    );

    // The creation binding does not depend on retained evidence: a run that
    // sealed the creation boundary settles to the same state.
    let mut sealed = before_lesson.into_compacted().expect("compact");
    sealed
        .seal_evidence()
        .expect("seal")
        .expect("sealed segment");
    sealed
        .settle_boundary(at_day(3, SystemCadence::Daily))
        .expect("sealed lesson boundary");
    assert_eq!(sealed.checkpoint_hash(), canwu.checkpoint_hash());

    // The built-in movement command reads the projection: the ward has no
    // record there, so it has nowhere to move from.
    assert_eq!(movement_rejection(&canwu), ErrorCode::ActorNotFound);

    // Save/load keeps the ward out of the projection, and the restored run
    // continues exactly as the live run does: core custody applies to the
    // ward, and a detained ward's commands are refused at admission.
    let json = canwu.snapshot_json().expect("snapshot json");
    let mut restored = Canwu::from_snapshot_json_with_plugins(&json, &[&plugin]).expect("restore");
    assert_eq!(restored.snapshot(), canwu.snapshot());
    for run in [&mut canwu, &mut restored] {
        run.settle_boundary(at_day(4, SystemCadence::Daily))
            .expect("the household confines its ward");
    }
    assert_eq!(restored.snapshot(), canwu.snapshot());
    let confined = canwu
        .person_availability(ward())
        .expect("ward availability");
    assert_eq!(confined.custody, CustodyState::Detained);
    assert_eq!(confined.custodian, Some(household()));
    assert_eq!(movement_rejection(&canwu), ErrorCode::IssuerUnavailable);
    assert_eq!(canwu.world(), WorldSnapshot::default());

    let replayed =
        Canwu::replay_from_journal(&[&plugin], &canwu.replay_journal()).expect("exact replay");
    assert_eq!(replayed.snapshot(), canwu.snapshot());

    // The registry a load validates holds exactly the scenario's identities
    // and the created persons, placed or not.
    let mut forged = canwu.snapshot();
    forged
        .entities
        .retain(|entity| entity != &EntityRef::Person(ward()));
    refused_by_registry(&forged, &plugin);
}

fn refused_by_registry(forged: &SimulationSnapshot, plugin: &HouseholdPlugin) {
    let error = Canwu::from_snapshot_json_with_plugins(
        &serde_json::to_string(forged).expect("forged json"),
        &[plugin],
    )
    .err()
    .expect("a snapshot whose persons disagree with its creations must not load");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(
        error.message.contains("entity registry"),
        "{}",
        error.message
    );
}

#[test]
fn placed_and_unplaced_creations_share_a_populated_projection() {
    let plugin = HouseholdPlugin;
    let ids = Canwu::demo_ids();
    let unplaced = PersonId::new(ids.commander.get().max(ids.observer.get()) + 1);
    let settled = PersonId::new(unplaced.get() + 1);
    let mut canwu = Canwu::demo(4950).expect("demo");
    canwu.register_plugin(&plugin).expect("household plugin");
    canwu
        .settle_boundary(at_day(1, SystemCadence::Daily))
        .expect("first boundary");
    canwu
        .settle_boundary(at_day(2, SystemCadence::Daily))
        .expect("unplaced creation");
    canwu
        .settle_boundary(at_day(2, SystemCadence::Seasonal))
        .expect("placed creation");

    // Only the placed person joins the projection, where its draft put it.
    let world = canwu.world();
    assert!(world.person(unplaced).is_none());
    let record = world.person(settled).expect("placed person record");
    assert_eq!(
        (record.government, record.current_location),
        (ids.government, ids.western_territory)
    );
    assert_eq!(
        canwu.boundaries()[2].created_persons[0]
            .person
            .person_record()
            .as_ref(),
        Some(record)
    );
    for person in [unplaced, settled] {
        assert!(canwu.entity_exists(&EntityRef::Person(person)));
    }

    let snapshot = canwu.snapshot();
    let restored = Canwu::from_snapshot_json_with_plugins(
        &serde_json::to_string(&snapshot).expect("snapshot json"),
        &[&plugin],
    )
    .expect("restore");
    assert_eq!(restored.snapshot(), snapshot);
    let replayed =
        Canwu::replay_from_journal(&[&plugin], &canwu.replay_journal()).expect("exact replay");
    assert_eq!(replayed.snapshot(), snapshot);

    // A load refuses a `Person` record for the unplaced creation, and a
    // placed creation whose record is gone.
    let mut forged_record = snapshot.clone();
    forged_record.world.people.push(Person {
        id: unplaced,
        ..record.clone()
    });
    forged_record.world.people.sort_by_key(|person| person.id);
    refused_by_registry(&forged_record, &plugin);
    let mut missing_record = snapshot;
    missing_record
        .world
        .people
        .retain(|person| person.id != settled);
    refused_by_registry(&missing_record, &plugin);
}
