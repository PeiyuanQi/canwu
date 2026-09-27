//! Gap G-03: persons can be created at runtime with replay-stable IDs.

#![allow(clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryId, BoundaryPhase, BoundaryProposal,
    BoundaryRequest, BoundarySystemContract, Canwu, CanwuError, CreatedPerson, CustodyState,
    EntityRef, EvidenceRef, LifeState, PersonAvailability, PersonDraft, PersonId, PluginRegistrar,
    SimDuration, SimTime, SimulationPlugin, SimulationView, StateKey, StateVisibility,
    SystemCadence,
};
use serde_json::{Value, json};

const PLUGIN: &str = "fixture-lineage";

fn observation_state() -> StateKey {
    StateKey::new(PLUGIN, "observations")
}

/// The first engine-allocated ID follows every initial person identity.
fn first_runtime_person() -> PersonId {
    let ids = Canwu::demo_ids();
    PersonId::new(ids.commander.get().max(ids.observer.get()) + 1)
}

/// Creates one heir at boundary 2 and another at boundary 5, each citing the
/// preceding boundary as provenance.
fn propose_births(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let correlation = match context.boundary_id.get() {
        2 => "heir",
        5 => "second-heir",
        _ => return Ok(BoundaryProposal::default()),
    };
    let ids = Canwu::demo_ids();
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::CreatePerson {
            draft: PersonDraft {
                name: format!("Child of the house ({correlation})"),
                government: ids.government,
                current_location: ids.central_territory,
                roles: vec!["heir".to_owned()],
                availability: PersonAvailability::new(
                    LifeState::Alive,
                    CustodyState::Free,
                    view.time(),
                ),
                provenance: EvidenceRef::Boundary(BoundaryId::new(context.boundary_id.get() - 1)),
            },
            correlation: correlation.to_owned(),
            summary: "A child was born to the ruling house".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

/// Records, in every boundary, what a late-phase system can see of the heir.
fn observe_heir(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let bound: Vec<_> = view
        .persons_created_by_correlation(PLUGIN, "heir")?
        .into_iter()
        .map(|created| created.person.get())
        .collect();
    let visible = view.person(first_runtime_person())?.is_some();
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::SetComponent {
            state: observation_state(),
            entity: EntityRef::Government(Canwu::demo_ids().government),
            component: "heir".to_owned(),
            value: json!({"bound": bound, "visible": visible}),
            summary: "The court took note of the succession".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

struct LineagePlugin;

impl SimulationPlugin for LineagePlugin {
    fn name(&self) -> &'static str {
        PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000301"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut births = BoundarySystemContract::new(
            "births",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        births.reads = vec![StateKey::core_people()];
        births.writes = vec![StateKey::core_people()];
        registrar.register_boundary_system(births, propose_births)?;

        let mut observer = BoundarySystemContract::new(
            "observer",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        observer.reads = vec![StateKey::core_people()];
        observer.writes = vec![observation_state()];
        observer.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(observer, observe_heir)
    }
}

fn daily(days: i64) -> BoundaryRequest {
    BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(days)).with_cadence(SystemCadence::Daily)
}

fn observation(canwu: &Canwu, boundary: usize) -> Value {
    canwu.boundaries()[boundary]
        .changes
        .iter()
        .find(|change| change.component == "heir")
        .map(|change| change.value.clone())
        .expect("observer evidence")
}

#[test]
fn gap_g03_sim_create_person() {
    let lineage = LineagePlugin;
    let heir = first_runtime_person();
    let mut canwu = Canwu::demo(3131).expect("demo");
    canwu.register_plugin(&lineage).expect("lineage plugin");
    canwu.settle_boundary(daily(1)).expect("first boundary");
    let before_birth = canwu.fork();

    // The engine allocates the ID and binds it to the proposing correlation.
    let receipt = canwu.settle_boundary(daily(2)).expect("birth boundary");
    assert_eq!(
        receipt.created_persons,
        vec![CreatedPerson {
            plugin: PLUGIN.to_owned(),
            system: "births".to_owned(),
            correlation: "heir".to_owned(),
            person: heir,
        }]
    );
    assert!(canwu.entity_exists(&EntityRef::Person(heir)));
    assert_eq!(
        canwu
            .world()
            .person(heir)
            .map(|person| person.roles.clone()),
        Some(vec!["heir".to_owned()])
    );
    assert_eq!(
        canwu.person_availability(heir).map(|value| value.life),
        Some(LifeState::Alive)
    );

    // A cadence-free boundary admits the birth boundary's events, so its
    // evidence can be sealed before the next daily boundary.
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("event admission boundary");

    // The creation is invisible to later phases of its own boundary and
    // visible, with its correlation binding, from the next boundary on.
    let before_observation = canwu.fork();
    canwu
        .settle_boundary(daily(3))
        .expect("next daily boundary");
    assert_eq!(
        observation(&canwu, 1),
        json!({"bound": [], "visible": false})
    );
    assert_eq!(
        observation(&canwu, 3),
        json!({"bound": [heir.get()], "visible": true})
    );

    // The correlation binding does not depend on retained evidence: a run
    // that sealed the creation boundary settles to the same state.
    let mut sealed = before_observation.into_compacted().expect("compact");
    sealed
        .seal_evidence()
        .expect("seal")
        .expect("sealed segment");
    sealed
        .settle_boundary(daily(3))
        .expect("sealed next boundary");
    assert_eq!(sealed.checkpoint_hash(), canwu.checkpoint_hash());

    // A fork settles the same boundary to the same allocated ID.
    let mut fork = before_birth;
    assert_eq!(
        fork.settle_boundary(daily(2))
            .expect("fork birth")
            .created_persons,
        receipt.created_persons
    );

    // Save/load persists the allocation counter, and exact replay regenerates
    // the same IDs and creation evidence.
    let json = serde_json::to_string(&canwu.snapshot()).expect("snapshot json");
    let mut restored = Canwu::from_snapshot_json_with_plugins(&json, &[&lineage]).expect("restore");
    let second = canwu.settle_boundary(daily(4)).expect("second birth");
    assert_eq!(
        restored
            .settle_boundary(daily(4))
            .expect("restored second birth")
            .created_persons,
        second.created_persons
    );
    assert_eq!(
        second
            .created_persons
            .iter()
            .map(|created| created.person)
            .collect::<Vec<_>>(),
        vec![PersonId::new(heir.get() + 1)]
    );
    assert_eq!(restored.snapshot(), canwu.snapshot());
    let replayed =
        Canwu::replay_from_journal(&[&lineage], &canwu.replay_journal()).expect("exact replay");
    assert_eq!(replayed.snapshot(), canwu.snapshot());
    assert_eq!(
        replayed.boundaries()[1].created_persons,
        canwu.boundaries()[1].created_persons
    );
}
