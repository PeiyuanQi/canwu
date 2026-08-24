use canwu_api::{
    BoundaryRequest, Canwu, CommandRequestId, DecisionAuthority, DecisionControllerBinding,
    DecisionEvaluation, DecisionIngressRequest, DecisionMutation, DecisionPolicyIdentity,
    DecisionPolicyKind, DecisionRequestId, DecisionTicketId, EntityRef, Scenario, SimDuration,
    SimTime, SystemCadence, UtilityProfile, WeightedUtilityPolicy,
};
use canwu_society::{
    AffiliationTarget, AssentBand, AwarenessBand, DispositionBucket, DispositionDistribution,
    DispositionProfile, InfluenceSource, InstitutionalAlignment, MobilizationBand, ObserverProfile,
    OrganizationNode, OrganizationalTieBand, PolicyChoice, PolicyDecision, PolicyPressure,
    PracticeBand, PublicAlignmentBand, SocialInfluenceEdge, SocietyCohort, SocietyPlugin,
    SocietyState, TransitionRule, TransitionWeights, VisibilityBand, distribution_id,
    from_society_snapshot_json, institutional_policy_ticket, load_society_state,
    projection_for_viewer,
};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let plugin = SocietyPlugin;
    let demo = Canwu::demo(42)?;
    let snapshot = demo.snapshot();
    let scenario = Scenario {
        start_time: snapshot.initial_time,
        entities: snapshot.entities,
        world: snapshot.world,
        knowledge: snapshot.knowledge,
        domain_records: vec![tutorial_state().into_record()?],
    };
    let replay_scenario = scenario.clone();
    let mut canwu = Canwu::new_with_plugins(42, scenario, &[&plugin])?;

    settle_day(&mut canwu, 1)?;
    let before_policy = load_society_state(&canwu)?;
    print_village("day 1: local diffusion", &before_policy);

    register_policy_decision(&mut canwu)?;
    let policy = WeightedUtilityPolicy::new(
        "institutional-policy",
        "1",
        UtilityProfile {
            weights: BTreeMap::from([("control".to_owned(), 1)]),
        },
    );
    let evaluation = canwu.drive_decision(
        canwu.time(),
        0,
        DecisionRequestId::new(3),
        Some(CommandRequestId::new(1)),
        DecisionTicketId::new(1),
        &policy,
    )?;
    assert!(matches!(evaluation, DecisionEvaluation::Prepared(_)));
    canwu
        .step_canonical()?
        .ok_or("the policy resolution boundary was not created")?;

    let immediately_after_decision = load_society_state(&canwu)?;
    assert_eq!(
        immediately_after_decision.distributions, before_policy.distributions,
        "the policy choice must not instantly rewrite population dispositions"
    );
    println!(
        "policy selected; population dispositions remain unchanged until the next daily boundary"
    );

    settle_day(&mut canwu, 2)?;
    let after_policy = load_society_state(&canwu)?;
    print_village("day 2: institutional response applied", &after_policy);

    let ids = Canwu::demo_ids();
    let viewer = canwu.viewer_context(ids.observer)?;
    let projection = projection_for_viewer(&canwu, &viewer)?;
    let truth = &after_policy.aggregates[&distribution_id("village", "new-idea")];
    let estimate = &projection.entries[&distribution_id("village", "new-idea")];
    println!(
        "observer estimate: public={} tied={} (ground truth public={} tied={})",
        estimate.estimated_publicly_aligned,
        estimate.estimated_organizationally_tied,
        truth.publicly_aligned,
        truth.organizationally_tied
    );

    let saved = canwu.snapshot_json()?;
    let restored = from_society_snapshot_json(&saved)?;
    assert_eq!(restored.snapshot(), canwu.snapshot());
    let forked = canwu.fork();
    assert_eq!(forked.snapshot(), canwu.snapshot());
    let replayed =
        Canwu::replay_from_journal(replay_scenario, &[&plugin], &canwu.replay_journal())?;
    assert_eq!(replayed.snapshot(), canwu.snapshot());
    println!("snapshot restore, fork, and exact replay reproduced the same authoritative state");
    Ok(())
}

fn settle_day(canwu: &mut Canwu, day: i64) -> Result<(), Box<dyn Error>> {
    canwu.settle_boundary(
        BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(day))
            .with_cadence(SystemCadence::Daily),
    )?;
    Ok(())
}

fn print_village(label: &str, state: &SocietyState) {
    let aggregate = &state.aggregates[&distribution_id("village", "new-idea")];
    println!(
        "{label}: aware={} assenting={} public={} hidden={} of {}",
        aggregate.aware,
        aggregate.assenting,
        aggregate.publicly_aligned,
        aggregate.hidden,
        aggregate.headcount
    );
}

#[allow(clippy::too_many_lines)]
fn tutorial_state() -> SocietyState {
    let ids = Canwu::demo_ids();
    let neutral = DispositionProfile::neutral();
    let source = DispositionProfile {
        awareness: AwarenessBand::Aware,
        assent: AssentBand::Sympathetic,
        practice: PracticeBand::Occasional,
        public_alignment: PublicAlignmentBand::Advocating,
        organizational_tie: OrganizationalTieBand::Member,
        mobilization: MobilizationBand::Latent,
        visibility: VisibilityBand::Public,
    };
    let aware = DispositionProfile {
        awareness: AwarenessBand::Aware,
        ..neutral
    };
    let sympathetic = DispositionProfile {
        awareness: AwarenessBand::Aware,
        assent: AssentBand::Sympathetic,
        ..neutral
    };
    let conforming = DispositionProfile {
        public_alignment: PublicAlignmentBand::Conforming,
        visibility: VisibilityBand::Public,
        ..sympathetic
    };

    let mut state = SocietyState::default();
    state.cohorts.insert(
        "market".to_owned(),
        SocietyCohort {
            id: "market".to_owned(),
            territory: ids.western_territory,
            headcount: 1_000,
            classification: BTreeMap::from([("setting".to_owned(), "market".to_owned())]),
        },
    );
    state.cohorts.insert(
        "village".to_owned(),
        SocietyCohort {
            id: "village".to_owned(),
            territory: ids.central_territory,
            headcount: 2_000,
            classification: BTreeMap::from([("setting".to_owned(), "village".to_owned())]),
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
    insert_distribution(
        &mut state,
        "market",
        1_000,
        vec![(neutral, 800), (source, 200)],
    );
    insert_distribution(&mut state, "village", 2_000, vec![(neutral, 2_000)]);
    state.organizations.insert(
        "local-network".to_owned(),
        OrganizationNode {
            id: "local-network".to_owned(),
            target_id: "new-idea".to_owned(),
            base_reach_per_mille: 300,
            concealment_per_mille: 500,
            active: true,
        },
    );
    state.influence_edges.insert(
        "market-contact".to_owned(),
        SocialInfluenceEdge {
            id: "market-contact".to_owned(),
            source: InfluenceSource::Cohort("market".to_owned()),
            target_cohort_id: "village".to_owned(),
            target_id: "new-idea".to_owned(),
            channel: "local-contact".to_owned(),
            reach_per_mille: 800,
            trust_per_mille: 800,
            active: true,
        },
    );
    state.influence_edges.insert(
        "network-contact".to_owned(),
        SocialInfluenceEdge {
            id: "network-contact".to_owned(),
            source: InfluenceSource::Organization("local-network".to_owned()),
            target_cohort_id: "village".to_owned(),
            target_id: "new-idea".to_owned(),
            channel: "organized-contact".to_owned(),
            reach_per_mille: 700,
            trust_per_mille: 800,
            active: true,
        },
    );
    state.institutional_alignments.insert(
        "council-alignment".to_owned(),
        InstitutionalAlignment {
            id: "council-alignment".to_owned(),
            institution: EntityRef::Government(ids.government),
            target_id: "new-idea".to_owned(),
            affected_cohorts: BTreeSet::from(["village".to_owned()]),
            support_per_mille: 100,
            enforcement_per_mille: 0,
            access_grant_per_mille: 300,
            authorized_actor: Some(ids.commander),
            last_decision_version: 0,
        },
    );
    state.policies.insert(
        "local-policy".to_owned(),
        PolicyPressure {
            id: "local-policy".to_owned(),
            target_id: "new-idea".to_owned(),
            affected_cohorts: BTreeSet::from(["village".to_owned()]),
            support_per_mille: 0,
            legal_access_per_mille: 200,
            surveillance_per_mille: 100,
            censorship_per_mille: 0,
            coercion_per_mille: 0,
            material_penalty_per_mille: 0,
            disruption_per_mille: 0,
            migration_pressure_per_mille: 0,
        },
    );
    insert_rule(
        &mut state,
        "01-awareness",
        neutral,
        aware,
        TransitionWeights {
            influence: 200_000,
            ..TransitionWeights::default()
        },
    );
    insert_rule(
        &mut state,
        "02-assent",
        aware,
        sympathetic,
        TransitionWeights {
            influence: 100_000,
            institutional_support: 100_000,
            policy_coercion: -50_000,
            ..TransitionWeights::default()
        },
    );
    insert_rule(
        &mut state,
        "03-public",
        sympathetic,
        conforming,
        TransitionWeights {
            institutional_enforcement: 300_000,
            policy_coercion: 300_000,
            ..TransitionWeights::default()
        },
    );
    state.observer_profiles.insert(
        ids.observer.get().to_string(),
        ObserverProfile {
            actor: ids.observer,
            cohorts: BTreeSet::from(["village".to_owned()]),
            targets: BTreeSet::from(["new-idea".to_owned()]),
            public_detection_per_mille: 800,
            private_detection_per_mille: 200,
            false_positive_per_mille: 10,
            confidence_per_mille: 700,
        },
    );
    state
}

fn insert_distribution(
    state: &mut SocietyState,
    cohort_id: &str,
    headcount: u64,
    buckets: Vec<(DispositionProfile, u64)>,
) {
    let id = distribution_id(cohort_id, "new-idea");
    let bucket_total: u64 = buckets.iter().map(|(_, count)| count).sum();
    assert_eq!(bucket_total, headcount);
    state.distributions.insert(
        id.clone(),
        DispositionDistribution {
            id,
            cohort_id: cohort_id.to_owned(),
            target_id: "new-idea".to_owned(),
            buckets: buckets
                .into_iter()
                .map(|(profile, headcount)| DispositionBucket { profile, headcount })
                .collect(),
        },
    );
}

fn insert_rule(
    state: &mut SocietyState,
    id: &str,
    from: DispositionProfile,
    to: DispositionProfile,
    weights: TransitionWeights,
) {
    state.transition_rules.insert(
        id.to_owned(),
        TransitionRule {
            id: id.to_owned(),
            target_id: "new-idea".to_owned(),
            affected_cohorts: BTreeSet::from(["village".to_owned()]),
            from,
            to,
            base_rate_per_million: 0,
            weights,
        },
    );
}

fn register_policy_decision(canwu: &mut Canwu) -> Result<(), Box<dyn Error>> {
    let ids = Canwu::demo_ids();
    let controller = DecisionControllerBinding::new(
        "council-controller",
        DecisionPolicyIdentity::new(DecisionPolicyKind::Utility, "institutional-policy", "1"),
        DecisionAuthority::Actor {
            actor: ids.commander,
        },
    )
    .with_command_subject(EntityRef::Government(ids.government));
    canwu.enqueue_decision(
        canwu.time(),
        0,
        DecisionIngressRequest::new(
            DecisionRequestId::new(1),
            canwu.revision(),
            DecisionMutation::RegisterController { controller },
        ),
    )?;
    let ticket = institutional_policy_ticket(
        DecisionTicketId::new(1),
        "tutorial.institutional-policy",
        EntityRef::Person(ids.commander),
        "council-controller",
        "Choose the institution's public response",
        vec![
            PolicyChoice {
                id: "limited-access".to_owned(),
                label: "Allow limited access".to_owned(),
                decision: PolicyDecision {
                    alignment_id: "council-alignment".to_owned(),
                    decision_version: 1,
                    support_per_mille: 300,
                    enforcement_per_mille: 0,
                    access_grant_per_mille: 600,
                },
                utility_inputs: BTreeMap::from([("control".to_owned(), 10)]),
            },
            PolicyChoice {
                id: "public-conformity".to_owned(),
                label: "Require public conformity".to_owned(),
                decision: PolicyDecision {
                    alignment_id: "council-alignment".to_owned(),
                    decision_version: 1,
                    support_per_mille: 300,
                    enforcement_per_mille: 800,
                    access_grant_per_mille: 700,
                },
                utility_inputs: BTreeMap::from([("control".to_owned(), 100)]),
            },
        ],
        None,
    )?;
    canwu.enqueue_decision(
        canwu.time(),
        0,
        DecisionIngressRequest::new(
            DecisionRequestId::new(2),
            canwu.revision(),
            DecisionMutation::Open { ticket },
        ),
    )?;
    canwu
        .step_canonical()?
        .ok_or("the decision setup boundary was not created")?;
    Ok(())
}
