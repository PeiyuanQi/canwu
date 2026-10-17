//! Downstream gap G-40: a settlement cadence option on the society plugin,
//! no root rewrite in a settlement that changes nothing but timestamps, and
//! (0.18.0) one root write in a settlement that changes something.

use canwu_api::{
    BoundaryRequest, Canwu, DomainRecord, ErrorCode, Scenario, SimDuration, SimTime,
    SimulationPlugin, SystemCadence,
};
use canwu_society::{
    AffiliationTarget, AssentBand, AwarenessBand, DispositionBucket, DispositionDistribution,
    DispositionProfile, MobilizationBand, ObserverProfile, PublicAlignmentBand, SocietyCohort,
    SocietyPlugin, SocietyProjection, SocietyState, TransitionRule, TransitionWeights,
    VisibilityBand, compute_aggregates, distribution_id, load_society_state, projection_for_viewer,
    society_state_reference, validate_society_runtime,
};
use std::collections::{BTreeMap, BTreeSet};

const VILLAGERS: u64 = 100_000;
const DAILY_RATE: u32 = 1_234;
const DAYS: u32 = 30;

#[test]
#[allow(clippy::too_many_lines)]
fn gap_g40_society_settlement_cadence() {
    assert_eq!(
        SocietyPlugin::DAILY.settlement_cadence(),
        SystemCadence::Daily
    );
    assert_eq!(SocietyPlugin::default(), SocietyPlugin::DAILY);
    let monthly_plugin = SocietyPlugin::with_settlement_cadence(SystemCadence::Monthly);
    assert_eq!(monthly_plugin.settlement_cadence(), SystemCadence::Monthly);
    let monthly_plugins: [&dyn SimulationPlugin; 1] = [&monthly_plugin];
    let daily_plugins: [&dyn SimulationPlugin; 1] = [&SocietyPlugin::DAILY];

    // The cadence is part of the persisted plugin descriptor: the settlement
    // system carries it and the event-driven intakes do not.
    let mut monthly = simulation(diffusion(DAILY_RATE * DAYS), &monthly_plugins);
    let cadences = monthly
        .plugin_descriptors()
        .find(|descriptor| descriptor.name == "canwu-society")
        .expect("society descriptor")
        .boundary_systems
        .iter()
        .map(|system| system.cadence.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        cadences
            .iter()
            .filter(|cadence| **cadence == SystemCadence::Monthly)
            .count(),
        1
    );
    assert!(
        cadences
            .iter()
            .all(|cadence| matches!(cadence, SystemCadence::Monthly | SystemCadence::EventDriven))
    );
    // Event-driven is not a category a host schedules.
    let event_driven = SocietyPlugin::with_settlement_cadence(SystemCadence::EventDriven);
    assert_eq!(
        Canwu::new_with_plugins(
            40,
            scenario(vec![diffusion(1).into_record().expect("society record")]),
            &[&event_driven],
        )
        .err()
        .expect("an event-driven settlement cadence is refused")
        .code,
        ErrorCode::InvalidPluginRegistration
    );

    // A Monthly society ignores Daily boundaries: the root is not written.
    for days in 1..DAYS {
        settle(&mut monthly, days, &[SystemCadence::Daily]);
    }
    assert_eq!(root_version(&monthly), 1);
    assert_eq!(aware(&monthly), 0);
    // The boundary that carries the cadence settles once, at the rate
    // authored for one settlement: floor(100,000 x 37,020 / 1,000,000).
    settle(
        &mut monthly,
        DAYS,
        &[SystemCadence::Daily, SystemCadence::Monthly],
    );
    let moved_monthly = aware(&monthly);
    assert_eq!(moved_monthly, 3_702);

    // A Daily society settles the same span in thirty steps. A rate is per
    // settlement and the remainder of every division is carried, so the
    // people moved are exactly the floor of the summed exact shares: rounding
    // never accumulates.
    let mut daily = simulation(diffusion(DAILY_RATE), &daily_plugins);
    let mut exact_numerator = 0_u128;
    for days in 1..=DAYS {
        exact_numerator += u128::from(VILLAGERS - aware(&daily)) * u128::from(DAILY_RATE);
        settle(&mut daily, days, &[SystemCadence::Daily]);
    }
    let moved_daily = aware(&daily);
    assert_eq!(u128::from(moved_daily), exact_numerator / 1_000_000);
    let carried =
        load_society_state(&daily).expect("daily state").remainders["awareness::village"].remainder;
    assert_eq!(u128::from(carried), exact_numerator % 1_000_000);
    // The two cadences differ only by the compounding inside the coarser
    // period, which is bounded by that period's rate: the coarse settlement
    // moves at most (k - 1) x r x D / 1,000,000 + 1 more people than the k
    // fine settlements that moved D.
    assert!(moved_monthly >= moved_daily);
    assert!(
        u128::from(moved_monthly - moved_daily) * 1_000_000
            <= u128::from(DAYS - 1) * u128::from(DAILY_RATE) * u128::from(moved_daily) + 1_000_000
    );

    // A run is restored and replayed only with a plugin of its cadence.
    let snapshot = monthly.snapshot();
    let json = serde_json::to_string(&snapshot).expect("encode snapshot");
    assert_eq!(
        Canwu::from_snapshot_json_with_plugins(&json, &daily_plugins)
            .err()
            .expect("a Daily plugin cannot restore a Monthly run")
            .code,
        ErrorCode::PluginManifestMismatch
    );
    assert!(Canwu::replay_from_journal(&daily_plugins, &monthly.replay_journal()).is_err());
    let mut restored = Canwu::from_snapshot_json_with_plugins(&json, &monthly_plugins)
        .expect("restore the Monthly run");
    validate_society_runtime(&restored).expect("restored society runtime");
    assert_eq!(restored.snapshot(), snapshot);
    let mut replayed = Canwu::replay_from_journal(&monthly_plugins, &monthly.replay_journal())
        .expect("replay the Monthly run");
    assert_eq!(replayed.snapshot(), snapshot);
    for simulation in [&mut monthly, &mut restored, &mut replayed] {
        settle(simulation, 2 * DAYS, &[SystemCadence::Monthly]);
    }
    assert!(aware(&monthly) > moved_monthly);
    assert_eq!(restored.snapshot(), monthly.snapshot());
    assert_eq!(replayed.snapshot(), monthly.snapshot());
}

#[test]
#[allow(clippy::too_many_lines)]
fn gap_g40_society_idle_settlement_writes_no_root() {
    let plugins: [&dyn SimulationPlugin; 1] = [&SocietyPlugin::DAILY];

    // A society whose only rule has a zero rate. The first settlement
    // materializes the three derived views in one write; transitions change
    // nothing, so it stamps no transition.
    let mut idle = simulation(observed(0), &plugins);
    settle(&mut idle, 1, &[SystemCadence::Daily]);
    assert_eq!(root_changes(&idle), 1);
    let materialized = load_society_state(&idle).expect("materialized state");
    assert_eq!(materialized.last_transition_at, None);
    assert_eq!(materialized.last_mobilization_at, Some(day(1)));
    assert_eq!(materialized.last_aggregation_at, Some(day(1)));
    assert_eq!(materialized.last_projection_at, Some(day(1)));
    assert!(!materialized.mobilization_candidates.is_empty());
    assert!(!materialized.projections.is_empty());
    let version = root_version(&idle);
    // Every later settlement changes nothing but would-be timestamps: no
    // system writes, and the views stay current under their first stamps.
    for days in 2..=6 {
        settle(&mut idle, days, &[SystemCadence::Daily]);
        assert_eq!(root_changes(&idle), 0);
    }
    assert_eq!(root_version(&idle), version);
    let state = load_society_state(&idle).expect("idle state");
    assert_eq!(state, materialized);
    assert_eq!(state.aggregates, compute_aggregates(&state));

    // A rate of 100 per million moves one of 2,000 people every fifth
    // settlement. The four settlements between carry a remainder: one write,
    // which leaves the derived views valid.
    let mut carrying = simulation(observed(100), &plugins);
    settle(&mut carrying, 1, &[SystemCadence::Daily]);
    assert_eq!(root_changes(&carrying), 1);
    for days in 2..=4 {
        settle(&mut carrying, days, &[SystemCadence::Daily]);
        assert_eq!(root_changes(&carrying), 1);
        let state = load_society_state(&carrying).expect("carrying state");
        assert_eq!(state.last_transition_at, Some(day(i64::from(days))));
        assert_eq!(state.last_aggregation_at, Some(day(1)));
        assert_eq!(state.last_projection_at, Some(day(1)));
        assert_eq!(aware(&carrying), 0);
    }
    // The settlement that moves a person clears the aggregates and the
    // mobilization candidates and materializes them again in the same
    // write. The observer cannot detect awareness, so its estimate did not
    // change and its projection is kept with the time it last changed.
    settle(&mut carrying, 5, &[SystemCadence::Daily]);
    assert_eq!(root_changes(&carrying), 1);
    assert_eq!(aware(&carrying), 1);
    let moved = load_society_state(&carrying).expect("moved state");
    assert_eq!(moved.last_aggregation_at, Some(day(5)));
    assert_eq!(moved.last_mobilization_at, Some(day(5)));
    assert_eq!(moved.last_projection_at, Some(day(1)));
    assert_eq!(
        moved
            .projections
            .values()
            .next()
            .expect("projection")
            .observed_at,
        day(1)
    );
    // Transition state changes at most once per instant: a second boundary
    // at the same time that carries the cadence moves nothing.
    settle(&mut carrying, 5, &[SystemCadence::Daily]);
    assert_eq!(root_changes(&carrying), 0);
    assert_eq!(load_society_state(&carrying).expect("same instant"), moved);

    // A society that moves people in every settlement writes one version
    // per settlement, with every view it cleared materialized again; until
    // 0.18.0 it wrote four. A view that comes out empty (nobody is
    // mobilized, nobody observes) is still stamped once after each change
    // that cleared it.
    let mut active = simulation(diffusion(DAILY_RATE), &plugins);
    for days in 1..=3 {
        settle(&mut active, days, &[SystemCadence::Daily]);
        assert_eq!(root_changes(&active), 1);
        let state = load_society_state(&active).expect("active state");
        let now = Some(day(i64::from(days)));
        assert_eq!(state.last_transition_at, now);
        assert_eq!(state.last_mobilization_at, now);
        assert_eq!(state.last_aggregation_at, now);
        assert_eq!(state.last_projection_at, now);
        assert!(state.mobilization_candidates.is_empty() && state.projections.is_empty());
    }

    // Boundaries that write nothing restore and replay exactly.
    assert_restores_and_replays(idle, &plugins);
    assert_restores_and_replays(carrying, &plugins);
    assert_restores_and_replays(active, &plugins);
}

/// An observer's projection, including its `observed_at`, changes only in a
/// settlement that changes that observer's own estimate, so the time tells
/// an observer nothing about cohorts outside its scope.
#[test]
fn gap_g40_society_projection_time_follows_the_observer_estimate() {
    let ids = Canwu::demo_ids();
    let plugins: [&dyn SimulationPlugin; 1] = [&SocietyPlugin::DAILY];
    let mut canwu = simulation(watched(), &plugins);
    let watch = |canwu: &Canwu, actor| {
        let viewer = canwu.viewer_context(actor).expect("viewer context");
        projection_for_viewer(canwu, &viewer).expect("delivered projection")
    };
    let public = |projection: &SocietyProjection, cohort: &str| {
        projection.entries[&distribution_id(cohort, "new-idea")].estimated_publicly_aligned
    };

    // The market conforms in every settlement; a villager conforms in the
    // second and the fifth.
    settle(&mut canwu, 1, &[SystemCadence::Daily]);
    settle(&mut canwu, 2, &[SystemCadence::Daily]);
    let village = watch(&canwu, ids.observer);
    assert_eq!(village.observed_at, day(2));
    assert_eq!(public(&village, "village"), 1);
    assert_eq!(watch(&canwu, ids.commander).observed_at, day(2));

    // While only the market moves, the market's observer is told so and the
    // village's observer keeps its projection unchanged, time included.
    for days in 3..=4 {
        let market_before = watch(&canwu, ids.commander);
        settle(&mut canwu, days, &[SystemCadence::Daily]);
        let market = watch(&canwu, ids.commander);
        assert_eq!(market.observed_at, day(i64::from(days)));
        assert!(public(&market, "market") > public(&market_before, "market"));
        assert_eq!(watch(&canwu, ids.observer), village);
    }

    // The village observer's time moves when its own estimate does.
    settle(&mut canwu, 5, &[SystemCadence::Daily]);
    let village = watch(&canwu, ids.observer);
    assert_eq!(village.observed_at, day(5));
    assert_eq!(public(&village, "village"), 2);
    assert_restores_and_replays(canwu, &plugins);
}

/// Restores and exactly replays the run, then checks that all three keep
/// settling to the same state.
fn assert_restores_and_replays(mut simulation: Canwu, plugins: &[&dyn SimulationPlugin]) {
    let snapshot = simulation.snapshot();
    let mut restored = Canwu::from_snapshot_json_with_plugins(
        &serde_json::to_string(&snapshot).expect("encode snapshot"),
        plugins,
    )
    .expect("restore the run");
    validate_society_runtime(&restored).expect("restored society runtime");
    assert_eq!(restored.snapshot(), snapshot);
    let mut replayed =
        Canwu::replay_from_journal(plugins, &simulation.replay_journal()).expect("replay the run");
    assert_eq!(replayed.snapshot(), snapshot);
    for simulation in [&mut simulation, &mut restored, &mut replayed] {
        for days in 7..=11 {
            settle(simulation, days, &[SystemCadence::Daily]);
        }
    }
    assert_eq!(restored.snapshot(), simulation.snapshot());
    assert_eq!(replayed.snapshot(), simulation.snapshot());
}

fn day(days: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::days(days)
}

fn settle(canwu: &mut Canwu, days: u32, cadences: &[SystemCadence]) {
    let mut request = BoundaryRequest::at(day(i64::from(days)));
    for cadence in cadences {
        request = request.with_cadence(cadence.clone());
    }
    canwu.settle_boundary(request).expect("settle the boundary");
}

fn root_version(canwu: &Canwu) -> u64 {
    canwu
        .typed_domain_record(&society_state_reference())
        .expect("society root")
        .version
}

/// How many times the latest boundary wrote the society root.
fn root_changes(canwu: &Canwu) -> usize {
    canwu
        .boundaries()
        .last()
        .expect("a settled boundary")
        .record_changes
        .iter()
        .filter(|change| change.current.reference == society_state_reference().into_untyped())
        .count()
}

fn aware(canwu: &Canwu) -> u64 {
    load_society_state(canwu)
        .expect("society state")
        .distributions
        .get(&distribution_id("village", "new-idea"))
        .map_or(0, |distribution| {
            distribution
                .buckets
                .iter()
                .filter(|bucket| bucket.profile.awareness == AwarenessBand::Aware)
                .map(|bucket| bucket.headcount)
                .sum()
        })
}

fn simulation(state: SocietyState, plugins: &[&dyn SimulationPlugin]) -> Canwu {
    Canwu::new_with_plugins(
        40,
        scenario(vec![state.into_record().expect("society record")]),
        plugins,
    )
    .expect("society simulation")
}

fn scenario(domain_records: Vec<DomainRecord>) -> Scenario {
    let snapshot = Canwu::demo(42).expect("demo scenario").snapshot();
    Scenario {
        start_time: snapshot.initial_time,
        entities: snapshot.entities,
        world: snapshot.world,
        knowledge: snapshot.knowledge,
        domain_records,
    }
}

/// One cohort and one rule that moves neutral villagers to aware at a base
/// rate with no signals, so its effective rate is constant.
fn diffusion(rate_per_million: u32) -> SocietyState {
    let ids = Canwu::demo_ids();
    let neutral = DispositionProfile::neutral();
    let mut state = SocietyState::default();
    state.cohorts.insert(
        "village".to_owned(),
        SocietyCohort {
            id: "village".to_owned(),
            territory: ids.central_territory,
            headcount: VILLAGERS,
            classification: BTreeMap::new(),
        },
    );
    state.targets.insert(
        "new-idea".to_owned(),
        AffiliationTarget {
            id: "new-idea".to_owned(),
            parent: None,
            neutral_profile: neutral,
            metadata: BTreeMap::new(),
        },
    );
    state.transition_rules.insert(
        "awareness".to_owned(),
        TransitionRule {
            id: "awareness".to_owned(),
            target_id: "new-idea".to_owned(),
            affected_cohorts: BTreeSet::from(["village".to_owned()]),
            from: neutral,
            to: DispositionProfile {
                awareness: AwarenessBand::Aware,
                ..neutral
            },
            base_rate_per_million: rate_per_million,
            weights: TransitionWeights::default(),
        },
    );
    state
}

/// A market whose people conform publicly in every settlement and a
/// village where one person conforms in the second settlement and one in the
/// fifth. Each cohort has its own observer, who detects public alignment
/// exactly.
fn watched() -> SocietyState {
    let ids = Canwu::demo_ids();
    let neutral = DispositionProfile::neutral();
    let mut state = SocietyState::default();
    state.targets.insert(
        "new-idea".to_owned(),
        AffiliationTarget {
            id: "new-idea".to_owned(),
            parent: None,
            neutral_profile: neutral,
            metadata: BTreeMap::new(),
        },
    );
    for (cohort, territory, headcount, rate, actor) in [
        (
            "market",
            ids.western_territory,
            1_000,
            100_000,
            ids.commander,
        ),
        ("village", ids.central_territory, 2_000, 250, ids.observer),
    ] {
        state.cohorts.insert(
            cohort.to_owned(),
            SocietyCohort {
                id: cohort.to_owned(),
                territory,
                headcount,
                classification: BTreeMap::new(),
            },
        );
        state.transition_rules.insert(
            format!("{cohort}-conformity"),
            TransitionRule {
                id: format!("{cohort}-conformity"),
                target_id: "new-idea".to_owned(),
                affected_cohorts: BTreeSet::from([cohort.to_owned()]),
                from: neutral,
                to: DispositionProfile {
                    public_alignment: PublicAlignmentBand::Conforming,
                    visibility: VisibilityBand::Public,
                    ..neutral
                },
                base_rate_per_million: rate,
                weights: TransitionWeights::default(),
            },
        );
        state.observer_profiles.insert(
            actor.get().to_string(),
            ObserverProfile {
                actor,
                cohorts: BTreeSet::from([cohort.to_owned()]),
                targets: BTreeSet::new(),
                public_detection_per_mille: 1_000,
                private_detection_per_mille: 1_000,
                false_positive_per_mille: 0,
                confidence_per_mille: 1_000,
            },
        );
    }
    state
}

/// The diffusion society at village scale with a mobilized minority and an
/// observer, so every derived view has content to materialize.
fn observed(rate_per_million: u32) -> SocietyState {
    let ids = Canwu::demo_ids();
    let neutral = DispositionProfile::neutral();
    let mut state = diffusion(rate_per_million);
    state
        .cohorts
        .get_mut("village")
        .expect("village cohort")
        .headcount = 2_100;
    state.distributions.insert(
        distribution_id("village", "new-idea"),
        DispositionDistribution {
            id: distribution_id("village", "new-idea"),
            cohort_id: "village".to_owned(),
            target_id: "new-idea".to_owned(),
            buckets: vec![
                DispositionBucket {
                    profile: neutral,
                    headcount: 2_000,
                },
                DispositionBucket {
                    profile: DispositionProfile {
                        assent: AssentBand::Committed,
                        mobilization: MobilizationBand::Active,
                        ..neutral
                    },
                    headcount: 100,
                },
            ],
        },
    );
    state.observer_profiles.insert(
        ids.observer.get().to_string(),
        ObserverProfile {
            actor: ids.observer,
            cohorts: BTreeSet::new(),
            targets: BTreeSet::new(),
            public_detection_per_mille: 800,
            private_detection_per_mille: 200,
            false_positive_per_mille: 10,
            confidence_per_mille: 700,
        },
    );
    state
}
