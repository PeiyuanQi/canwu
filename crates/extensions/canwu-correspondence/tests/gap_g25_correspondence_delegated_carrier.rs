//! Public-API fixture for gap G-25: a carrier other than the sender carries
//! correspondence under a delegation claim, and the carrier's knowledge
//! ledger, not the sender's, drives route planning. The delegation is a
//! persisted record, so a run with sealed evidence admits it exactly as its
//! replay does.

#![allow(clippy::too_many_lines)]

#[path = "../examples/support/mod.rs"]
mod support;

use canwu_api::{
    Canwu, CommandAttemptOutcome, CommandEnvelope, CommandId, CommandRequest, CommandRequestId,
    DecisionAuthority, DecisionControllerBinding, DecisionEvaluation, DecisionIngressRequest,
    DecisionMutation, DecisionPolicyIdentity, DecisionPolicyKind, DecisionRequestId,
    DecisionTicketId, EntityRef, ErrorCode, Issuer, KnowledgeHolderRef, KnowledgeQuery, Person,
    PersonId, PluginIngressRequest, RoutingNodeRef, RoutingPolicy, RoutingRequest, SimDuration,
    SimTime, SimulationPlugin, TransportExecutionId, UtilityProfile, WeightedUtilityPolicy,
    plan_route,
};
use canwu_correspondence::{
    CARRY_CORRESPONDENCE_CAPABILITY, CarrierAuthority, CarrierDelegationRecord,
    CarrierDelegationRequest, CorrespondenceCapacityAdmission, CorrespondenceOperationRecord,
    CorrespondencePlugin, CorrespondenceStatus, InitiateCorrespondenceRequest, KNOWLEDGE_INGRESS,
    PLUGIN_NAME, carrier_delegation_command, carrier_delegation_ref,
    correspondence_decision_ticket, correspondence_operation_ref, planning_knowledge_query,
    planning_snapshot_from_knowledge_result,
};
use canwu_information::{DelegationClaimV1, InformationOperationId, InformationPlugin};
use std::collections::BTreeMap;
use support::{network_seed, scenario_with_prepared_dispatch};

const CONTROLLER: &str = "sender-policy";

fn install(canwu: &mut Canwu, seed: &canwu_correspondence::NetworkKnowledgeSeed) {
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            KNOWLEDGE_INGRESS,
            canwu.time(),
            serde_json::to_value(seed).unwrap(),
        ))
        .unwrap();
    canwu.step_canonical().unwrap().unwrap();
}

/// Opens one send decision for the sender and lets its policy issue the
/// correspondence command, optionally running `between` once the decision is
/// open. Shared by the ordinary and the compacted host.
macro_rules! open_and_drive_send {
    ($canwu:expr, $sender:expr, $request:expr, $round:expr) => {
        open_and_drive_send!($canwu, $sender, $request, $round, |_host| {})
    };
    ($canwu:expr, $sender:expr, $request:expr, $round:expr, |$host:ident| $between:block) => {{
        let (canwu, sender, request, round) = ($canwu, $sender, $request, $round);
        let mut ticket = correspondence_decision_ticket(
            DecisionTicketId::new(round),
            EntityRef::Person(sender),
            CONTROLLER,
            "Decide whether to send through the delegated carrier",
            Some(request.due_at),
            request,
        )
        .unwrap();
        for option in &mut ticket.options {
            option
                .utility_inputs
                .insert("send".to_owned(), if option.id == "send" { 100 } else { 0 });
        }
        canwu
            .enqueue_decision(
                canwu.time(),
                0,
                DecisionIngressRequest::new(
                    DecisionRequestId::new(10 * round),
                    canwu.revision(),
                    DecisionMutation::Open { ticket },
                ),
            )
            .unwrap();
        canwu.step_canonical().unwrap().unwrap();
        {
            let $host = &mut *canwu;
            $between
        }
        let policy = WeightedUtilityPolicy::new(
            "send-policy",
            "1",
            UtilityProfile {
                weights: BTreeMap::from([("send".to_owned(), 1)]),
            },
        );
        assert!(matches!(
            canwu
                .drive_decision(
                    canwu.time(),
                    0,
                    DecisionRequestId::new(10 * round + 1),
                    Some(CommandRequestId::new(round)),
                    DecisionTicketId::new(round),
                    &policy,
                )
                .unwrap(),
            DecisionEvaluation::Prepared(_)
        ));
        canwu.step_canonical().unwrap().unwrap();
    }};
}

/// Sends through the ordinary host and returns the command attempt outcome.
fn send(
    canwu: &mut Canwu,
    sender: PersonId,
    request: &InitiateCorrespondenceRequest,
    round: u64,
) -> CommandAttemptOutcome {
    open_and_drive_send!(&mut *canwu, sender, request, round);
    canwu.command_attempts().last().unwrap().outcome.clone()
}

fn load(canwu: &Canwu, key: &str) -> Option<canwu_correspondence::CorrespondenceOperation> {
    canwu
        .typed_domain_record(&correspondence_operation_ref(key))
        .map(|record| {
            record
                .decode_payload::<CorrespondenceOperationRecord>()
                .unwrap()
        })
}

fn ledger_size(canwu: &Canwu, holder: &KnowledgeHolderRef) -> usize {
    canwu
        .admin_query_knowledge(holder.clone(), &KnowledgeQuery::default())
        .unwrap()
        .records
        .len()
}

#[test]
fn gap_g25_correspondence_delegated_carrier() {
    let plugins: &[&dyn SimulationPlugin] = &[&InformationPlugin, &CorrespondencePlugin];
    let (mut scenario, sender, recipient, prepared_dispatch) = scenario_with_prepared_dispatch();
    let template = scenario
        .world
        .people
        .iter()
        .find(|person| person.id == sender)
        .cloned()
        .unwrap();
    let envoy = PersonId::new(
        scenario
            .world
            .people
            .iter()
            .map(|person| person.id.get())
            .max()
            .unwrap()
            + 1,
    );
    scenario.world.people.push(Person {
        id: envoy,
        name: "Envoy".to_owned(),
        roles: vec!["envoy".to_owned()],
        ..template
    });
    scenario.entities.push(EntityRef::Person(envoy));
    scenario.entities.sort();
    let mut canwu = Canwu::new_with_plugins(2025, scenario, plugins).unwrap();
    let carrier = KnowledgeHolderRef::Person(envoy);
    let sender_holder = KnowledgeHolderRef::Person(sender);
    let destination = RoutingNodeRef::new("beijing/delivery/recipient");

    // The envoy knows the direct railway; the sender knows only the slower
    // route through Nanjing, so the accepted route shows whose ledger planned.
    install(
        &mut canwu,
        &network_seed(
            carrier.clone(),
            recipient.clone(),
            destination.clone(),
            true,
        ),
    );
    let mut sender_seed = network_seed(
        sender_holder.clone(),
        recipient.clone(),
        destination.clone(),
        true,
    );
    sender_seed.seed_key = "sender-network".to_owned();
    sender_seed
        .connections
        .retain(|known| known.connection.id.as_str() != "wuxi-beijing-direct");
    install(&mut canwu, &sender_seed);
    let sender_ledger = ledger_size(&canwu, &sender_holder);

    // The envoy accepts carrying under its own command authority. The sender
    // cannot accept on the envoy's behalf, and a claim without the carrying
    // capability or one that has already expired is not accepted at all.
    let start = canwu.time();
    let claim = |not_before: Option<SimTime>, expires_at: Option<SimTime>| DelegationClaimV1 {
        format_version: 1,
        performed_by: EntityRef::Person(envoy),
        performed_for: sender_holder.clone(),
        capabilities: vec![CARRY_CORRESPONDENCE_CAPABILITY.to_owned()],
        not_before,
        expires_at,
    };
    let valid_claim = claim(None, Some(start + SimDuration::days(10)));
    assert_rejected(&delegate(&mut canwu, sender, &valid_claim, 101));
    let mut wrong_capability = valid_claim.clone();
    wrong_capability.capabilities = vec!["carry_goods".to_owned()];
    assert_rejected(&delegate(&mut canwu, envoy, &wrong_capability, 102));
    assert_rejected(&delegate(&mut canwu, envoy, &claim(None, Some(start)), 103));
    let short = accepted(&delegate(
        &mut canwu,
        envoy,
        &claim(None, Some(start + SimDuration::hours(1))),
        104,
    ));
    let mut other_principal = valid_claim.clone();
    other_principal.performed_for = recipient.clone();
    let other_principal = accepted(&delegate(&mut canwu, envoy, &other_principal, 105));

    // Two hours later the short delegation has expired.
    canwu.advance_canonical(SimDuration::hours(2)).unwrap();
    canwu
        .enqueue_decision(
            canwu.time(),
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(1),
                canwu.revision(),
                DecisionMutation::RegisterController {
                    controller: DecisionControllerBinding::new(
                        CONTROLLER,
                        DecisionPolicyIdentity::new(
                            DecisionPolicyKind::Utility,
                            "send-policy",
                            "1",
                        ),
                        DecisionAuthority::Actor { actor: sender },
                    ),
                },
            ),
        )
        .unwrap();
    canwu.step_canonical().unwrap().unwrap();
    let now = canwu.time();
    assert_eq!(now, start + SimDuration::hours(2));
    let request = |delegation: Option<CommandId>| InitiateCorrespondenceRequest {
        operation_key: "envoy-letter".to_owned(),
        sender: EntityRef::Person(sender),
        recipient: recipient.clone(),
        carrier: carrier.clone(),
        channel_profile: "sealed-letter".to_owned(),
        origin: RoutingNodeRef::new("wuxi/hub"),
        due_at: now + SimDuration::days(10),
        prepared_dispatch: prepared_dispatch.clone(),
        delivery_attempt_operation: InformationOperationId::new(
            "fixture.correspondence",
            "envoy-attempt",
        ),
        routing_policy: RoutingPolicy::default(),
        capacity_admission: CorrespondenceCapacityAdmission::Unconstrained,
        execution_id: TransportExecutionId(25),
        automatic_opportunity: None,
        carrier_delegation: delegation,
    };
    let mut round = 0;
    let mut assert_not_admitted = |canwu: &mut Canwu, delegation: Option<CommandId>| {
        round += 1;
        assert_rejected(&send(canwu, sender, &request(delegation), round));
        assert!(load(canwu, "envoy-letter").is_none());
    };

    // Without a delegation, with one that has expired by dispatch, or with
    // one accepting carrying for another principal, naming the envoy is not
    // authority to plan from its ledger.
    assert_not_admitted(&mut canwu, None);
    assert_not_admitted(&mut canwu, Some(short));
    assert_not_admitted(&mut canwu, Some(other_principal));

    // The carrier's newest delegation for a principal replaces the earlier
    // one: a delegation not yet valid is refused, and once replaced by a
    // valid one it can no longer be cited.
    let not_yet = accepted(&delegate(
        &mut canwu,
        envoy,
        &claim(Some(start + SimDuration::days(1)), None),
        106,
    ));
    canwu.step_canonical().unwrap().unwrap();
    assert_not_admitted(&mut canwu, Some(not_yet));
    let valid = accepted(&delegate(&mut canwu, envoy, &valid_claim, 107));
    canwu.step_canonical().unwrap().unwrap();
    assert_not_admitted(&mut canwu, Some(not_yet));
    let record = canwu
        .typed_domain_record(
            &carrier_delegation_ref(&EntityRef::Person(envoy), &sender_holder).unwrap(),
        )
        .unwrap()
        .decode_payload::<CarrierDelegationRecord>()
        .unwrap();
    assert_eq!(record.delegation, valid);

    // Sealing the delegation command's evidence into an archive changes
    // nothing: the compacted run admits the envoy from the persisted
    // delegation, and its replay from the stitched journal agrees exactly.
    let mut sealed = canwu.fork().into_compacted().unwrap();
    let segments = sealed
        .seal_evidence()
        .unwrap()
        .into_iter()
        .collect::<Vec<_>>();
    assert!(!segments.is_empty());
    open_and_drive_send!(&mut sealed, sender, &request(Some(valid)), 6);
    for _ in 0..80 {
        if sealed
            .typed_domain_record(&correspondence_operation_ref("envoy-letter"))
            .is_some_and(|record| {
                record
                    .decode_payload::<CorrespondenceOperationRecord>()
                    .is_ok_and(|operation| operation.status.is_terminal())
            })
        {
            break;
        }
        sealed.step_canonical().unwrap().unwrap();
    }
    let sealed_operation = sealed
        .typed_domain_record(&correspondence_operation_ref("envoy-letter"))
        .unwrap()
        .decode_payload::<CorrespondenceOperationRecord>()
        .unwrap();
    assert_eq!(sealed_operation.status, CorrespondenceStatus::Settled);
    let journal = sealed
        .replay_journal_with_segments(segments.clone())
        .unwrap();
    assert_eq!(
        Canwu::replay_from_journal(plugins, &journal)
            .unwrap()
            .snapshot(),
        sealed.snapshot_with_segments(segments).unwrap()
    );

    // The envoy's own delegation admits it, and its ledger plans the route.
    // A newer delegation the envoy issues while the send is admitted is
    // recorded in the same boundary; the admitted start still settles under
    // the delegation it cited instead of stalling the run.
    let mut renewed_claim = valid_claim.clone();
    renewed_claim.expires_at = Some(start + SimDuration::days(20));
    let renewed;
    open_and_drive_send!(&mut canwu, sender, &request(Some(valid)), 6, |host| {
        renewed = accepted(&delegate(host, envoy, &renewed_claim, 108));
    });
    assert!(matches!(
        canwu.command_attempts().last().unwrap().outcome,
        CommandAttemptOutcome::Accepted { .. }
    ));
    for _ in 0..80 {
        if load(&canwu, "envoy-letter").is_some_and(|operation| operation.status.is_terminal()) {
            break;
        }
        canwu.step_canonical().unwrap().unwrap();
    }
    let operation = load(&canwu, "envoy-letter").unwrap();
    assert_eq!(operation.status, CorrespondenceStatus::Settled);
    assert_eq!(operation.intent.carrier, carrier);
    assert_eq!(
        operation.intent.carrier_authority,
        Some(CarrierAuthority {
            delegation: valid,
            claim: valid_claim,
        })
    );
    let current = canwu
        .typed_domain_record(
            &carrier_delegation_ref(&EntityRef::Person(envoy), &sender_holder).unwrap(),
        )
        .unwrap()
        .decode_payload::<CarrierDelegationRecord>()
        .unwrap();
    assert_eq!(current.delegation, renewed);
    assert_eq!(
        operation.route_plan.legs[0].connection.as_str(),
        "wuxi-beijing-direct"
    );

    // The host repeats the public holder-planning builder over the envoy's
    // ledger and reaches the plugin's route and read cut.
    let hosted = canwu
        .admin_query_knowledge(carrier.clone(), &planning_knowledge_query())
        .unwrap();
    assert_eq!(
        operation.address.read_cut.holder_projection_root,
        hosted.read_cut.holder_projection_root
    );
    let (snapshot, _) =
        planning_snapshot_from_knowledge_result(&hosted, operation.intent.accepted_at).unwrap();
    let hosted_route = plan_route(
        &snapshot,
        &RoutingRequest {
            origin: operation.intent.origin.clone(),
            destination,
            departure_at: operation.intent.accepted_at,
            policy: operation.intent.routing_policy.clone(),
        },
    )
    .unwrap();
    assert_eq!(hosted_route.legs, operation.route_plan.legs);

    // Planning read the envoy's ledger without disclosing it to the sender.
    assert_eq!(ledger_size(&canwu, &sender_holder), sender_ledger);

    let restored =
        Canwu::from_snapshot_json_with_plugins(&canwu.snapshot_json().unwrap(), plugins).unwrap();
    assert_eq!(restored.snapshot(), canwu.snapshot());
    let replayed = Canwu::replay_from_journal(plugins, &canwu.replay_journal()).unwrap();
    assert_eq!(replayed.snapshot(), canwu.snapshot());
}

/// Issues one carrier delegation under `issuer`'s own command authority.
fn delegate(
    canwu: &mut Canwu,
    issuer: PersonId,
    claim: &DelegationClaimV1,
    request: u64,
) -> CommandAttemptOutcome {
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(request),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(issuer),
                    carrier_delegation_command(&CarrierDelegationRequest {
                        claim: claim.clone(),
                    })
                    .unwrap(),
                )
                .at_time(canwu.time()),
            ),
        )
        .unwrap();
    canwu.step_canonical().unwrap().unwrap();
    canwu.command_attempts().last().unwrap().outcome.clone()
}

fn accepted(outcome: &CommandAttemptOutcome) -> CommandId {
    match outcome {
        CommandAttemptOutcome::Accepted { command_id } => *command_id,
        CommandAttemptOutcome::Rejected { error } => panic!("command was rejected: {error:?}"),
    }
}

fn assert_rejected(outcome: &CommandAttemptOutcome) {
    assert!(matches!(
        outcome,
        CommandAttemptOutcome::Rejected { error } if error.code == ErrorCode::InvalidAuthority
    ));
}
