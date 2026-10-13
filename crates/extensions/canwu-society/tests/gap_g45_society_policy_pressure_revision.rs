//! Downstream gap G-45: a decision-provenanced runtime revision of policy
//! pressure and of influence edge and organization activity and reach.

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, CauseRef, Command, CommandAttemptOutcome,
    CommandAuthority, CommandEnvelope, CommandId, CommandRequest, CommandRequestId,
    DecisionAuthority, DecisionControllerBinding, DecisionEvaluation, DecisionIngressRequest,
    DecisionMutation, DecisionOrigin, DecisionPolicyIdentity, DecisionPolicyKind,
    DecisionRequestId, DecisionTicketId, DomainRecord, EntityRef, ErrorCode, Issuer,
    PluginIngressRequest, PluginIngressTarget, PluginRegistrar, Scenario, SimDuration, SimTime,
    SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence, UtilityProfile,
    WeightedUtilityPolicy,
};
use canwu_society::{
    AffiliationTarget, AwarenessBand, DispositionProfile, InfluenceEdgeRevision, InfluenceSource,
    InstitutionalAlignment, MAX_PRESSURE_REVISION_ROWS, ObserverProfile, OrganizationNode,
    OrganizationRevision, PRESSURE_REVISION_COMMAND, PRESSURE_REVISION_ORIGIN_REJECTION,
    PRESSURE_REVISION_STALE_REJECTION, PRESSURE_REVISION_SUPERSEDED_REJECTION, PolicyPressure,
    PolicyPressureRevision, PressureRevisionChoice, PressureRevisionDecision,
    SOCIETY_PRESSURE_REVISED_EVENT, SOCIETY_PRESSURE_REVISION_INGRESS,
    SOCIETY_PRESSURE_REVISION_REJECTED_EVENT, SocialInfluenceEdge, SocietyCohort,
    SocietyCohortExchangeLedger, SocietyCohortExchangeLedgerRecord, SocietyIngressQueueRecord,
    SocietyPlugin, SocietyPressureRevisionV1, SocietyState, SocietyStateRecord, TransitionRule,
    TransitionWeights, distribution_id, load_society_state, pressure_revision_ticket,
    projection_for_viewer, society_cohort_exchange_ledger_reference,
    society_ingress_queue_reference, society_state_reference, validate_society_runtime,
};
use std::collections::{BTreeMap, BTreeSet};

/// The council's alignment on the new idea for every cohort.
const ALIGNMENT: &str = "council-alignment";
/// The council's alignment on the new idea for the village only.
const WARD: &str = "ward-alignment";

#[test]
#[allow(clippy::too_many_lines)]
fn gap_g45_society_policy_pressure_revision() {
    let ids = Canwu::demo_ids();
    let plugins: [&dyn SimulationPlugin; 1] = [&SocietyPlugin::DAILY];
    let mut canwu = Canwu::new_with_plugins(
        45,
        scenario(vec![society_state().into_record().expect("society record")]),
        &plugins,
    )
    .expect("society simulation");
    let mut decisions = Decisions::register(&mut canwu);

    // Before any revision the hall is closed and the council presses nothing,
    // so the only rule has a zero rate and a settlement moves nobody.
    daily(&mut canwu, 1);
    assert_eq!(aware(&canwu), 0);
    let watch = |canwu: &Canwu| {
        let viewer = canwu.viewer_context(ids.observer).expect("viewer context");
        projection_for_viewer(canwu, &viewer).expect("delivered projection")
    };
    let watched = watch(&canwu);
    assert_eq!(watched.observed_at, day(1));

    // A decision the council's controller did not take through a ticket, and
    // one issued by another actor, are refused before anything is queued.
    let forged_envelopes = [
        CommandEnvelope::new(
            Issuer::Ai("council-controller".to_owned()),
            revision_command(&open_hall()),
        )
        .with_authority(CommandAuthority {
            decision_origin: DecisionOrigin::Actor {
                actor: ids.commander,
            },
            seat_id: None,
            permission_profile_id: None,
            command_subject: Some(EntityRef::Government(ids.government)),
        }),
        CommandEnvelope::new(Issuer::Actor(ids.observer), revision_command(&open_hall())),
    ];
    for (request, envelope) in (9_001..).zip(forged_envelopes) {
        canwu
            .enqueue_command(
                canwu.time(),
                0,
                CommandRequest::new(CommandRequestId::new(request), canwu.revision(), envelope),
            )
            .expect("queue the forged command");
        canwu.step_canonical().expect("settle the forged command");
        assert_eq!(
            command_outcome(&canwu, request)
                .expect_err("a revision needs the validated ticket controller")
                .code,
            ErrorCode::InvalidAuthority
        );
    }
    // The packet the command queues is internal: the host cannot author it.
    assert_eq!(
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                "canwu-society",
                SOCIETY_PRESSURE_REVISION_INGRESS,
                canwu.time(),
                serde_json::to_value(SocietyPressureRevisionV1 {
                    decision: open_hall(),
                    issuer: EntityRef::Government(ids.government),
                    command: CommandId::new(1),
                })
                .expect("packet payload"),
            ))
            .expect_err("the pressure revision packet is internal ingress")
            .code,
        ErrorCode::InvalidAuthority
    );

    // The council may revise only pressure rows it issued and the edges and
    // organizations of its target, and only rows that exist. Its alignment
    // for the village alone changes nothing beyond the village: not the hall,
    // which acts on every cohort, not the market's contact, and not the
    // council's own pressure on the market. Each refusal is a recorded
    // command rejection, not a failed boundary.
    let mut unissued_policy = open_hall();
    unissued_policy.policies[0].policy_id = "guild-pressure".to_owned();
    let mut foreign_organization = open_hall();
    foreign_organization.organizations[0].organization_id = "rival-hall".to_owned();
    let mut ward_organization = ward_decision(1);
    ward_organization.policies.clear();
    ward_organization.influence_edges.clear();
    ward_organization.organizations = open_hall().organizations;
    let mut ward_market_edge = ward_decision(1);
    ward_market_edge.influence_edges[0].edge_id = "market-contact".to_owned();
    let mut ward_market_policy = ward_decision(1);
    ward_market_policy.policies[0].policy_id = "market-pressure".to_owned();
    for decision in [
        unissued_policy,
        foreign_organization,
        ward_organization,
        ward_market_edge,
        ward_market_policy,
    ] {
        assert_eq!(
            decisions
                .decide(&mut canwu, decision)
                .expect_err("a row outside the alignment's authority")
                .code,
            ErrorCode::InvalidAuthority
        );
    }
    let mut unknown = open_hall();
    unknown.organizations[0].organization_id = "missing-hall".to_owned();
    assert_eq!(
        decisions
            .decide(&mut canwu, unknown)
            .expect_err("an unknown row")
            .code,
        ErrorCode::EntityNotFound
    );
    let before = load_society_state(&canwu).expect("state before the revision");

    // The authorized decision is admitted, but changes nothing until the
    // society settles: it waits in the owner-side queue.
    decisions
        .decide(&mut canwu, open_hall())
        .expect("the council's controller revises its pressure");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("admit the queued revision");
    assert_eq!(load_society_state(&canwu).expect("queued state"), before);
    let queue = canwu
        .typed_domain_record(&society_ingress_queue_reference())
        .expect("society ingress queue")
        .decode_payload::<SocietyIngressQueueRecord>()
        .expect("queue payload");
    assert_eq!(queue.entries.len(), 1);
    assert_eq!(
        queue.entries[0].packet_type,
        SOCIETY_PRESSURE_REVISION_INGRESS
    );
    assert_eq!(queue.entries[0].admission_rejection, None);
    assert!(ledger(&canwu).pressure_revisions.is_empty());

    // The next settlement applies the revision before its transitions, so
    // the revised signals set that settlement's rates: influence 800 x 500 x
    // 1000 / 1e6 = 400 and coercion 500 give 400 x 500 + 500 x 200 = 300,000
    // per million of the 2,000 villagers.
    daily(&mut canwu, 2);
    let state = load_society_state(&canwu).expect("revised state");
    let pressure = &state.policies["council-pressure"];
    assert_eq!(pressure.coercion_per_mille, 500);
    assert_eq!(pressure.surveillance_per_mille, 300);
    assert_eq!(pressure.issuer, Some(EntityRef::Government(ids.government)));
    assert_eq!(pressure.decision_version, 1);
    assert_eq!(
        state.policies["guild-pressure"],
        before.policies["guild-pressure"]
    );
    let hall = &state.organizations["hall"];
    assert!(hall.active);
    assert_eq!(hall.base_reach_per_mille, 800);
    assert_eq!(hall.concealment_per_mille, 200);
    let contact = &state.influence_edges["hall-contact"];
    assert!(contact.active);
    assert_eq!(contact.reach_per_mille, 500);
    assert_eq!(contact.trust_per_mille, 1_000);
    assert_eq!(aware(&canwu), 600);
    // The revision opened an organization and moved villagers, but nothing
    // the village's observer can detect: its estimate is the same, so its
    // projection is kept with the time it last changed.
    assert_eq!(watch(&canwu), watched);

    // The outcome names the decision: its version, issuer, command, and the
    // admitted packet, whose ingress record the command caused.
    let outcomes = &ledger(&canwu).pressure_revisions[ALIGNMENT];
    let applied = outcomes.applied.as_ref().expect("applied outcome");
    assert_eq!(outcomes.applied_version(), 1);
    assert_eq!(applied.digest, open_hall().digest().expect("digest"));
    assert_eq!(applied.issuer, EntityRef::Government(ids.government));
    assert_eq!(applied.settled_at, day(2));
    assert_eq!(applied.rejection, None);
    assert_eq!(outcomes.rejected, None);
    let packet = canwu
        .ingress_log()
        .iter()
        .find(|record| record.id == applied.ingress)
        .expect("the applied packet");
    assert_eq!(packet.cause, Some(CauseRef::Command(applied.command)));
    assert_eq!(event_count(&canwu, SOCIETY_PRESSURE_REVISED_EVENT), 1);
    assert!(canwu.events().iter().any(|event| {
        event
            .summary
            .contains("pressure revision 1 of alignment council-alignment")
    }));

    // A resend of the applied decision is accepted and changes nothing: no
    // packet is queued and no record is written.
    let ingress_before = canwu.ingress_log().len();
    let settled = society_versions(&canwu);
    decisions
        .decide(&mut canwu, open_hall())
        .expect("a resend of the applied decision is idempotent");
    assert!(
        canwu.ingress_log()[ingress_before..]
            .iter()
            .all(|record| !is_revision_packet(record))
    );
    assert_eq!(society_versions(&canwu), settled);

    // The applied version with other content is refused at admission.
    let mut conflicting = open_hall();
    conflicting.policies[0].coercion_per_mille = 900;
    assert_eq!(
        decisions
            .decide(&mut canwu, conflicting)
            .expect_err("version 1 is bound to other content")
            .code,
        ErrorCode::IdempotencyConflict
    );

    // Two decisions taken against version 1 are both admitted while neither
    // has settled. The first applies; the second expected a version that is
    // no longer the applied one and is rejected whole.
    decisions
        .decide(&mut canwu, close_hall(2))
        .expect("the first decision against version 1");
    let mut racing = close_hall(3);
    racing.policies[0].coercion_per_mille = 1_000;
    decisions
        .decide(&mut canwu, racing)
        .expect("the racing decision is admitted");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("admit both revisions");
    daily(&mut canwu, 3);
    let state = load_society_state(&canwu).expect("state after the closure");
    assert!(!state.organizations["hall"].active);
    assert!(!state.influence_edges["hall-contact"].active);
    assert_eq!(state.policies["council-pressure"].coercion_per_mille, 0);
    assert_eq!(state.policies["council-pressure"].decision_version, 2);
    // With the hall closed and no coercion the rule's rate is zero again.
    assert_eq!(aware(&canwu), 600);
    let outcomes = &ledger(&canwu).pressure_revisions[ALIGNMENT];
    assert_eq!(outcomes.applied_version(), 2);
    let rejected = outcomes.rejected.as_ref().expect("rejected outcome");
    assert_eq!(rejected.decision_version, 3);
    assert_eq!(
        rejected.rejection.as_deref(),
        Some(PRESSURE_REVISION_STALE_REJECTION)
    );
    assert_eq!(event_count(&canwu, SOCIETY_PRESSURE_REVISED_EVENT), 2);
    assert_eq!(
        event_count(&canwu, SOCIETY_PRESSURE_REVISION_REJECTED_EVENT),
        1
    );

    // A resend of the superseded first decision cannot reopen the hall.
    decisions
        .decide(&mut canwu, open_hall())
        .expect("the superseded decision is admitted");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("admit the superseded revision");
    daily(&mut canwu, 4);
    assert_eq!(
        load_society_state(&canwu).expect("state after the resend"),
        state
    );
    let outcomes = &ledger(&canwu).pressure_revisions[ALIGNMENT];
    assert_eq!(outcomes.applied_version(), 2);
    let rejected = outcomes.rejected.as_ref().expect("rejected outcome");
    assert_eq!(rejected.decision_version, 1);
    assert_eq!(
        rejected.rejection.as_deref(),
        Some(PRESSURE_REVISION_SUPERSEDED_REJECTION)
    );

    // The village alignment has its own version sequence. Its first
    // decision revises the village's rows after the council's second, so the
    // pressure row's `decision_version` names the last decision applied to
    // it and goes down. Coercion 100 alone gives 100 x 200 = 20,000 per
    // million of the 1,400 neutral villagers.
    decisions
        .decide(&mut canwu, ward_decision(1))
        .expect("the village alignment revises the village's rows");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("admit the village revision");
    daily(&mut canwu, 5);
    let state = load_society_state(&canwu).expect("state after the village revision");
    assert_eq!(state.policies["council-pressure"].coercion_per_mille, 100);
    assert_eq!(state.policies["council-pressure"].decision_version, 1);
    assert!(state.influence_edges["hall-contact"].active);
    assert!(!state.organizations["hall"].active);
    assert_eq!(aware(&canwu), 628);
    let revisions = ledger(&canwu).pressure_revisions;
    assert_eq!(revisions[WARD].applied_version(), 1);
    assert_eq!(revisions[ALIGNMENT].applied_version(), 2);

    // The revised run restores and replays exactly, and keeps settling.
    let snapshot = canwu.snapshot();
    let mut restored = Canwu::from_snapshot_json_with_plugins(
        &serde_json::to_string(&snapshot).expect("encode snapshot"),
        &plugins,
    )
    .expect("restore the revised run");
    validate_society_runtime(&restored).expect("restored society runtime");
    assert_eq!(restored.snapshot(), snapshot);
    let mut replayed = Canwu::replay_from_journal(&plugins, &canwu.replay_journal())
        .expect("replay the revised run");
    assert_eq!(replayed.snapshot(), snapshot);
    for simulation in [&mut canwu, &mut restored, &mut replayed] {
        daily(simulation, 6);
    }
    assert_eq!(restored.snapshot(), canwu.snapshot());
    assert_eq!(replayed.snapshot(), canwu.snapshot());
}

#[test]
fn gap_g45_society_pressure_revision_is_bounded() {
    let mut decision = open_hall();
    decision.validate().expect("a well-formed decision");
    decision.policies.clear();
    decision.organizations.clear();
    decision.influence_edges = (0..=MAX_PRESSURE_REVISION_ROWS)
        .map(|index| InfluenceEdgeRevision {
            edge_id: format!("edge-{index:04}"),
            active: true,
            reach_per_mille: 100,
        })
        .collect();
    assert_eq!(
        decision
            .validate()
            .expect_err("more rows than one decision may revise")
            .code,
        ErrorCode::InvalidDecision
    );
    decision.influence_edges.truncate(2);
    decision.validate().expect("two rows in order");
    decision.influence_edges.swap(0, 1);
    assert!(decision.validate().is_err(), "rows are in ascending order");
    decision.influence_edges.truncate(1);
    decision.expected_version = decision.decision_version;
    assert!(
        decision.validate().is_err(),
        "a decision is newer than the version it expects"
    );
    // A state applies a decision as a whole or not at all.
    let mut state = society_state();
    let mut partial = open_hall();
    partial.organizations[0].organization_id = "rival-hall".to_owned();
    let before = state.clone();
    assert_eq!(
        state
            .apply_pressure_revision(&partial)
            .expect_err("one row lies outside the alignment's authority")
            .code,
        ErrorCode::InvalidAuthority
    );
    assert_eq!(state, before);
    assert!(
        state
            .apply_pressure_revision(&open_hall())
            .expect("apply the whole decision")
    );
    assert!(
        !state
            .apply_pressure_revision(&open_hall())
            .expect("apply the same decision again"),
        "applying the same values changes nothing"
    );
}

/// A packet that did not come from the revision command is refused, and a
/// revision queued before a seal settles in the sealed run exactly as in its
/// full replay.
#[test]
#[allow(clippy::too_many_lines)]
fn gap_g45_society_pressure_revision_origin_and_sealed_replay() {
    let society = SocietyPlugin::with_settlement_cadence(SystemCadence::Monthly);
    let forger = ForgingPlugin;
    let plugins: [&dyn SimulationPlugin; 2] = [&society, &forger];
    let mut canwu = Canwu::new_with_plugins(
        45,
        scenario(vec![society_state().into_record().expect("society record")]),
        &plugins,
    )
    .expect("society simulation");
    let mut decisions = Decisions::register(&mut canwu);
    let before = load_society_state(&canwu).expect("initial state");

    // Another plugin's boundary system schedules a well-formed packet that
    // names a real alignment and decision. The intake marks it; the
    // settlement rejects it without touching society state.
    canwu
        .settle_boundary(BoundaryRequest::at(day(1)).with_cadence(SystemCadence::Annual))
        .expect("the forging system schedules its packet");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("admit the forged packet");
    let queue = canwu
        .typed_domain_record(&society_ingress_queue_reference())
        .expect("society ingress queue")
        .decode_payload::<SocietyIngressQueueRecord>()
        .expect("queue payload");
    assert_eq!(
        queue.entries[0].admission_rejection.as_deref(),
        Some(PRESSURE_REVISION_ORIGIN_REJECTION)
    );
    canwu
        .settle_boundary(BoundaryRequest::at(day(30)).with_cadence(SystemCadence::Monthly))
        .expect("the first Monthly settlement");
    let state = load_society_state(&canwu).expect("state after the forged packet");
    assert_eq!(state.policies, before.policies);
    assert_eq!(state.influence_edges, before.influence_edges);
    assert_eq!(state.organizations, before.organizations);
    assert!(ledger(&canwu).pressure_revisions.is_empty());
    assert_eq!(
        event_count(&canwu, SOCIETY_PRESSURE_REVISION_REJECTED_EVENT),
        1
    );

    // The council's own decision is admitted and queued, then the run seals
    // its evidence before the next Monthly settlement.
    decisions
        .decide(&mut canwu, open_hall())
        .expect("the council's controller revises its pressure");
    for _ in 0..3 {
        canwu
            .settle_boundary(BoundaryRequest::at(canwu.time()))
            .expect("admit the revision and settle quiet boundaries");
    }
    validate_society_runtime(&canwu).expect("the queued revision matches its ingress record");
    let mut sealed = canwu.into_compacted().expect("compact mode should start");
    let segments = vec![
        sealed
            .seal_evidence()
            .expect("settled history should seal")
            .expect("settled history should contain evidence"),
    ];
    sealed
        .settle_boundary(BoundaryRequest::at(day(60)).with_cadence(SystemCadence::Monthly))
        .expect("the sealed run settles the queued revision");
    let state = sealed
        .typed_domain_record(&society_state_reference())
        .expect("society root")
        .decode_payload::<SocietyStateRecord>()
        .expect("society payload");
    assert!(state.organizations["hall"].active);
    assert_eq!(state.policies["council-pressure"].decision_version, 1);
    let ledger = sealed
        .typed_domain_record(&society_cohort_exchange_ledger_reference())
        .expect("ledger")
        .decode_payload::<SocietyCohortExchangeLedgerRecord>()
        .expect("ledger payload");
    assert_eq!(ledger.pressure_revisions[ALIGNMENT].applied_version(), 1);

    let replayed = Canwu::replay_from_journal(
        &plugins,
        &sealed
            .replay_journal_with_segments(segments.clone())
            .expect("the sealed run should produce an exact replay journal"),
    )
    .expect("the sealed run should replay exactly");
    assert_eq!(
        replayed.snapshot(),
        sealed
            .snapshot_with_segments(segments)
            .expect("the sealed archive should reconstruct a full snapshot")
    );
}

fn day(days: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::days(days)
}

/// Settles any canonical ingress due before `days`, then a Daily boundary.
fn daily(canwu: &mut Canwu, days: i64) {
    canwu
        .advance_canonical(day(days) - canwu.time())
        .expect("settle due canonical ingress");
    canwu
        .settle_boundary(BoundaryRequest::at(day(days)).with_cadence(SystemCadence::Daily))
        .expect("daily society boundary");
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

fn ledger(canwu: &Canwu) -> SocietyCohortExchangeLedger {
    canwu
        .typed_domain_record(&society_cohort_exchange_ledger_reference())
        .map_or_else(SocietyCohortExchangeLedger::empty, |record| {
            record
                .decode_payload::<SocietyCohortExchangeLedgerRecord>()
                .expect("ledger payload")
        })
}

/// Versions of the society root and ledger records.
fn society_versions(canwu: &Canwu) -> (u64, u64) {
    (
        canwu
            .typed_domain_record(&society_state_reference())
            .expect("society root")
            .version,
        canwu
            .typed_domain_record(&society_cohort_exchange_ledger_reference())
            .expect("society ledger")
            .version,
    )
}

fn event_count(canwu: &Canwu, event_type: &str) -> usize {
    canwu
        .events()
        .iter()
        .filter(|event| event.kind.plugin_identity() == Some(("canwu-society", event_type)))
        .count()
}

fn is_revision_packet(record: &canwu_api::IngressRecord) -> bool {
    matches!(
        &record.payload,
        canwu_api::IngressPayload::Plugin { packet_type, .. }
            if packet_type == SOCIETY_PRESSURE_REVISION_INGRESS
    )
}

fn revision_command(decision: &PressureRevisionDecision) -> Command {
    Command::Plugin {
        plugin: "canwu-society".to_owned(),
        command: PRESSURE_REVISION_COMMAND.to_owned(),
        payload: serde_json::to_value(decision).expect("revision payload"),
    }
}

/// Version 1: the council opens the hall, restores its contact with the
/// village, and presses the idea.
fn open_hall() -> PressureRevisionDecision {
    PressureRevisionDecision {
        alignment_id: ALIGNMENT.to_owned(),
        decision_version: 1,
        expected_version: 0,
        policies: vec![PolicyPressureRevision {
            policy_id: "council-pressure".to_owned(),
            support_per_mille: 0,
            legal_access_per_mille: 0,
            surveillance_per_mille: 300,
            censorship_per_mille: 0,
            coercion_per_mille: 500,
            material_penalty_per_mille: 0,
            disruption_per_mille: 0,
            migration_pressure_per_mille: 0,
        }],
        influence_edges: vec![InfluenceEdgeRevision {
            edge_id: "hall-contact".to_owned(),
            active: true,
            reach_per_mille: 500,
        }],
        organizations: vec![OrganizationRevision {
            organization_id: "hall".to_owned(),
            active: true,
            base_reach_per_mille: 800,
        }],
    }
}

/// A decision through the village alignment: the council presses the
/// village lightly and restores the hall's contact with it.
fn ward_decision(decision_version: u64) -> PressureRevisionDecision {
    PressureRevisionDecision {
        alignment_id: WARD.to_owned(),
        decision_version,
        expected_version: decision_version - 1,
        policies: vec![PolicyPressureRevision {
            policy_id: "council-pressure".to_owned(),
            support_per_mille: 0,
            legal_access_per_mille: 0,
            surveillance_per_mille: 0,
            censorship_per_mille: 0,
            coercion_per_mille: 100,
            material_penalty_per_mille: 0,
            disruption_per_mille: 0,
            migration_pressure_per_mille: 0,
        }],
        influence_edges: vec![InfluenceEdgeRevision {
            edge_id: "hall-contact".to_owned(),
            active: true,
            reach_per_mille: 500,
        }],
        organizations: Vec::new(),
    }
}

/// A later version: the council closes the hall and withdraws its pressure.
fn close_hall(decision_version: u64) -> PressureRevisionDecision {
    PressureRevisionDecision {
        alignment_id: ALIGNMENT.to_owned(),
        decision_version,
        expected_version: 1,
        policies: vec![PolicyPressureRevision {
            policy_id: "council-pressure".to_owned(),
            support_per_mille: 0,
            legal_access_per_mille: 0,
            surveillance_per_mille: 0,
            censorship_per_mille: 0,
            coercion_per_mille: 0,
            material_penalty_per_mille: 0,
            disruption_per_mille: 0,
            migration_pressure_per_mille: 0,
        }],
        influence_edges: vec![InfluenceEdgeRevision {
            edge_id: "hall-contact".to_owned(),
            active: false,
            reach_per_mille: 0,
        }],
        organizations: vec![OrganizationRevision {
            organization_id: "hall".to_owned(),
            active: false,
            base_reach_per_mille: 0,
        }],
    }
}

/// Opens one ticket per decision for the council's registered controller and
/// resolves it, which admits the revision command with ticket authority.
struct Decisions {
    next: u64,
}

impl Decisions {
    fn register(canwu: &mut Canwu) -> Self {
        let ids = Canwu::demo_ids();
        let controller = DecisionControllerBinding::new(
            "council-controller",
            DecisionPolicyIdentity::new(DecisionPolicyKind::Utility, "council-policy", "1"),
            DecisionAuthority::Actor {
                actor: ids.commander,
            },
        )
        .with_command_subject(EntityRef::Government(ids.government));
        canwu
            .enqueue_decision(
                canwu.time(),
                0,
                DecisionIngressRequest::new(
                    DecisionRequestId::new(1),
                    canwu.revision(),
                    DecisionMutation::RegisterController { controller },
                ),
            )
            .expect("register the council controller");
        canwu
            .step_canonical()
            .expect("admit the controller")
            .expect("controller boundary");
        Self { next: 1 }
    }

    /// Returns the error that rejected the revision command, if any.
    fn decide(
        &mut self,
        canwu: &mut Canwu,
        decision: PressureRevisionDecision,
    ) -> Result<(), CanwuError> {
        let ids = Canwu::demo_ids();
        self.next += 1;
        let ticket_id = DecisionTicketId::new(self.next);
        let ticket = pressure_revision_ticket(
            ticket_id,
            "fixture.pressure-revision",
            EntityRef::Person(ids.commander),
            "council-controller",
            "Revise the council's pressure",
            vec![PressureRevisionChoice {
                id: "revise".to_owned(),
                label: "Revise".to_owned(),
                decision,
                utility_inputs: BTreeMap::from([("control".to_owned(), 1)]),
            }],
            None,
        )?;
        canwu.enqueue_decision(
            canwu.time(),
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(self.next * 2),
                canwu.revision(),
                DecisionMutation::Open { ticket },
            ),
        )?;
        canwu.step_canonical()?;
        let policy = WeightedUtilityPolicy::new(
            "council-policy",
            "1",
            UtilityProfile {
                weights: BTreeMap::from([("control".to_owned(), 1)]),
            },
        );
        let evaluation = canwu.drive_decision(
            canwu.time(),
            0,
            DecisionRequestId::new(self.next * 2 + 1),
            Some(CommandRequestId::new(self.next)),
            ticket_id,
            &policy,
        )?;
        assert!(matches!(evaluation, DecisionEvaluation::Prepared(_)));
        canwu.step_canonical()?;
        command_outcome(canwu, self.next)
    }
}

/// The outcome of the command attempt that carried `request`.
fn command_outcome(canwu: &Canwu, request: u64) -> Result<(), CanwuError> {
    let attempt = canwu
        .command_attempts()
        .iter()
        .find(|attempt| attempt.request_id == Some(CommandRequestId::new(request)))
        .expect("the command was attempted");
    match &attempt.outcome {
        CommandAttemptOutcome::Accepted { .. } => Ok(()),
        CommandAttemptOutcome::Rejected { error } => Err(error.clone()),
    }
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

#[allow(clippy::too_many_lines)]
fn society_state() -> SocietyState {
    let ids = Canwu::demo_ids();
    let neutral = DispositionProfile::neutral();
    let mut state = SocietyState::default();
    for (id, territory, headcount) in [
        ("market", ids.western_territory, 1_000),
        ("village", ids.central_territory, 2_000),
    ] {
        state.cohorts.insert(
            id.to_owned(),
            SocietyCohort {
                id: id.to_owned(),
                territory,
                headcount,
                classification: BTreeMap::new(),
            },
        );
    }
    for id in ["new-idea", "old-custom"] {
        state.targets.insert(
            id.to_owned(),
            AffiliationTarget {
                id: id.to_owned(),
                parent: None,
                neutral_profile: neutral,
                metadata: BTreeMap::new(),
            },
        );
    }
    // The council decides on the new idea for every cohort through one
    // alignment and for the village alone through another.
    for (id, affected_cohorts) in [
        (ALIGNMENT, BTreeSet::new()),
        (WARD, BTreeSet::from(["village".to_owned()])),
    ] {
        state.institutional_alignments.insert(
            id.to_owned(),
            InstitutionalAlignment {
                id: id.to_owned(),
                institution: EntityRef::Government(ids.government),
                target_id: "new-idea".to_owned(),
                affected_cohorts,
                support_per_mille: 0,
                enforcement_per_mille: 0,
                access_grant_per_mille: 0,
                authorized_actor: Some(ids.commander),
                last_decision_version: 0,
            },
        );
    }
    // The council's pressure on the village and on the market, and a row
    // nobody issued.
    for (id, issuer, cohort) in [
        (
            "council-pressure",
            Some(EntityRef::Government(ids.government)),
            "village",
        ),
        (
            "market-pressure",
            Some(EntityRef::Government(ids.government)),
            "market",
        ),
        ("guild-pressure", None, "market"),
    ] {
        state.policies.insert(
            id.to_owned(),
            PolicyPressure {
                id: id.to_owned(),
                target_id: "new-idea".to_owned(),
                affected_cohorts: BTreeSet::from([cohort.to_owned()]),
                support_per_mille: 0,
                legal_access_per_mille: 0,
                surveillance_per_mille: 0,
                censorship_per_mille: 0,
                coercion_per_mille: 0,
                material_penalty_per_mille: 0,
                disruption_per_mille: 0,
                migration_pressure_per_mille: 0,
                issuer,
                decision_version: 0,
            },
        );
    }
    // A closed hall for the new idea and an organization of another target.
    for (id, target) in [("hall", "new-idea"), ("rival-hall", "old-custom")] {
        state.organizations.insert(
            id.to_owned(),
            OrganizationNode {
                id: id.to_owned(),
                target_id: target.to_owned(),
                base_reach_per_mille: 0,
                concealment_per_mille: 200,
                active: false,
            },
        );
    }
    // The hall's contact with the village, and a contact with the market,
    // which the village alignment does not cover.
    for (id, cohort) in [("hall-contact", "village"), ("market-contact", "market")] {
        state.influence_edges.insert(
            id.to_owned(),
            SocialInfluenceEdge {
                id: id.to_owned(),
                source: InfluenceSource::Organization("hall".to_owned()),
                target_cohort_id: cohort.to_owned(),
                target_id: "new-idea".to_owned(),
                channel: "hall".to_owned(),
                reach_per_mille: 0,
                trust_per_mille: 1_000,
                active: false,
            },
        );
    }
    // An observer of the village who detects public alignment and, less
    // well, organization and mobilization, but not awareness.
    state.observer_profiles.insert(
        ids.observer.get().to_string(),
        ObserverProfile {
            actor: ids.observer,
            cohorts: BTreeSet::from(["village".to_owned()]),
            targets: BTreeSet::new(),
            public_detection_per_mille: 800,
            private_detection_per_mille: 500,
            false_positive_per_mille: 10,
            confidence_per_mille: 700,
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
            base_rate_per_million: 0,
            weights: TransitionWeights {
                influence: 500_000,
                policy_coercion: 200_000,
                ..TransitionWeights::default()
            },
        },
    );
    state
}

const FORGER: &str = "test-pressure-forger";

/// A plugin whose boundary system schedules a society pressure revision
/// packet directly, without the revision command and its authority check.
struct ForgingPlugin;

impl SimulationPlugin for ForgingPlugin {
    fn name(&self) -> &'static str {
        FORGER
    }

    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn semantic_hash(&self) -> &'static str {
        "5d0c1b2a39485766758493a2b1c0d9e8f7061524334251607f8e9dacbb0a1928"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut forge = BoundarySystemContract::new(
            "forge-pressure-revision",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Annual,
        );
        forge.reads = vec![StateKey::core_ingress()];
        forge.plugin_ingress_targets = vec![PluginIngressTarget {
            target_plugin: "canwu-society".to_owned(),
            packet_type: SOCIETY_PRESSURE_REVISION_INGRESS.to_owned(),
        }];
        forge.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(forge, forge_pressure_revision)
    }
}

fn forge_pressure_revision(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let ids = Canwu::demo_ids();
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::SchedulePluginIngress {
            target_plugin: "canwu-society".to_owned(),
            after: SimDuration::ZERO,
            packet_type: SOCIETY_PRESSURE_REVISION_INGRESS.to_owned(),
            priority: 0,
            payload: serde_json::to_value(SocietyPressureRevisionV1 {
                decision: open_hall(),
                issuer: EntityRef::Government(ids.government),
                command: CommandId::new(1),
            })
            .map_err(|error| CanwuError::new(ErrorCode::InvalidPayload, error.to_string()))?,
            affected: Vec::new(),
        }],
        ..BoundaryProposal::default()
    })
}
