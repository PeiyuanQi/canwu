//! A batch set settles several offices' batches in one round.
//!
//! A person who is the principal of one office and acts for another issues
//! both offices' batches as one `FiscalActionBatchSet`, deciding in his own
//! name: one boundary admits the set and the next, at the same instant,
//! settles every batch, leaving the books and outcomes the same batches leave
//! when each office's batch is its own command in its own round. A set is one
//! command under one issuer, refused as a whole with the batch named: an
//! institution's decision covers its own institution's batches only, and a
//! decision in a person's own name holds what the person holds when he issues
//! directly (an acting actor cannot rebind) and is admitted for a set only,
//! never for a single batch: an auditor's finding decided in his own name is
//! refused as one batch and admitted, under his audit binding, in a set.

mod support;

use canwu_api::{
    BoundaryReceipt, Canwu, CanwuError, Command, CommandAttemptOutcome, CommandAuthority,
    CommandEnvelope, CommandRequest, CommandRequestId, DecisionAction, DecisionAuthority,
    DecisionContext, DecisionControllerBinding, DecisionIngressRequest, DecisionMutation,
    DecisionOption, DecisionOrigin, DecisionPolicyIdentity, DecisionPolicyKind, DecisionRequestId,
    DecisionTicketDraft, DecisionTicketId, EntityRef, ErrorCode, Issuer, PolicyDecision,
    SimDuration, SimTime,
};
use canwu_fiscal::{
    FiscalAction, FiscalActionBatch, FiscalActionBatchSet, FiscalActionDisposition,
    FiscalActionIssuer, FiscalAssessment, FiscalAuditBinding, FiscalAuditSeverity,
    FiscalIssuerRole, FiscalPaymentForm, FiscalSettledAction, FiscalState, fiscal_action_command,
    fiscal_action_set_command,
};
use std::collections::BTreeSet;
use support::{
    AUDITOR_COMMISSION, DEPUTY_COMMISSION, Fixture, GRANARY_AUTHORITY, GRANARY_SCOPE, PRINCIPAL,
    RULE, TREASURY_AUTHORITY, assessment, draft, enqueue_requests, fiscal_state, granary,
    initial_commission, outcome, scenario, settled_at, treasury, treasury_scope,
};

const CYCLE: &str = "2026";
/// Decides in the principal's own name.
const HOLDER: &str = "holder";
/// Decides for the treasury, with the principal as its responsible actor.
const TREASURY_OFFICE: &str = "treasury-office";
/// The principal's audit binding on the granary.
const GRANARY_AUDIT: &str = "audit.granary";
const GRANARY_ASSESSMENT: &str = "granary.authored";

/// The treasury's principal audits the granary, whose book holds one
/// assessment.
fn audits_granary(state: &mut FiscalState) {
    state.add_audit_binding(FiscalAuditBinding {
        id: GRANARY_AUDIT.to_owned(),
        institution: granary(),
        auditor: PRINCIPAL,
        authority_basis: initial_commission(AUDITOR_COMMISSION),
        rebound_by: None,
        nomination: None,
    });
    state
        .add_assessment(FiscalAssessment {
            id: GRANARY_ASSESSMENT.to_owned(),
            rule_id: RULE.to_owned(),
            scope_binding_id: GRANARY_SCOPE.to_owned(),
            accounting_cycle_id: CYCLE.to_owned(),
            quantity: 100,
            unit: "grain".to_owned(),
            payment_form: FiscalPaymentForm::Grain,
            commutation_quote: None,
            created_at: SimTime::EPOCH,
            source_action_id: None,
        })
        .expect("an authored assessment");
}

/// The treasury's principal also acts for the granary, under a commission.
fn acts_for_granary(state: &mut FiscalState) {
    let binding = state
        .authority_binding_mut(GRANARY_AUTHORITY)
        .expect("granary binding");
    binding.acting_actor = Some(PRINCIPAL);
    binding.authority_basis = Some(initial_commission(DEPUTY_COMMISSION));
}

/// Each office's batch of the day: two treasury scopes and the granary's one
/// are assessed, each entry against its target's revision now.
fn office_batches(canwu: &Canwu) -> [FiscalActionBatch; 2] {
    let mut treasury_batch =
        assessment("treasury.0", TREASURY_AUTHORITY, &treasury_scope(0), CYCLE).current(canwu);
    treasury_batch.entries.extend(
        assessment("treasury.1", TREASURY_AUTHORITY, &treasury_scope(1), CYCLE)
            .current(canwu)
            .entries,
    );
    let granary_batch =
        assessment("granary.0", GRANARY_AUTHORITY, GRANARY_SCOPE, CYCLE).current(canwu);
    [treasury_batch, granary_batch]
}

/// Settles every boundary due now, without advancing the clock.
fn round(canwu: &mut Canwu) -> Vec<BoundaryReceipt> {
    canwu
        .advance_canonical(SimDuration::ZERO)
        .expect("fixture boundaries")
}

fn policy(controller: &str) -> DecisionPolicyIdentity {
    DecisionPolicyIdentity::new(DecisionPolicyKind::Human, controller, "1")
}

/// What a controller's command carries as its authority, and the entity it
/// decides for.
fn origin(controller: &str) -> (DecisionOrigin, EntityRef) {
    if controller == HOLDER {
        (
            DecisionOrigin::Actor { actor: PRINCIPAL },
            EntityRef::Person(PRINCIPAL),
        )
    } else {
        (
            DecisionOrigin::Institution {
                institution: treasury(),
                responsible_actor: Some(PRINCIPAL),
            },
            treasury(),
        )
    }
}

fn register_controllers(canwu: &mut Canwu) {
    for (request, controller) in (1..).zip([HOLDER, TREASURY_OFFICE]) {
        let (origin, subject) = origin(controller);
        let authority = match origin {
            DecisionOrigin::Actor { actor } => DecisionAuthority::Actor { actor },
            _ => DecisionAuthority::Institution {
                institution: treasury(),
                responsible_actor: Some(PRINCIPAL),
            },
        };
        canwu
            .enqueue_decision(
                canwu.time(),
                0,
                DecisionIngressRequest::new(
                    DecisionRequestId::new(request),
                    canwu.revision(),
                    DecisionMutation::RegisterController {
                        controller: DecisionControllerBinding::new(
                            controller,
                            policy(controller),
                            authority,
                        )
                        .with_command_subject(subject),
                    },
                ),
            )
            .expect("controller registration");
    }
    round(canwu);
}

/// Has `controller` decide `command`: a ticket offers it, and the controller
/// selects it. Returns the boundaries of the round that admits the command
/// and settles what it queued.
fn decide(
    canwu: &mut Canwu,
    controller: &str,
    ticket: u64,
    command: Command,
) -> Vec<BoundaryReceipt> {
    let (origin, subject) = origin(controller);
    let now = canwu.time();
    canwu
        .enqueue_decision(
            now,
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(ticket * 10),
                canwu.revision(),
                DecisionMutation::Open {
                    ticket: DecisionTicketDraft {
                        id: DecisionTicketId::new(ticket),
                        definition: "fixture.fiscal-day".to_owned(),
                        decision_maker: subject.clone(),
                        assigned_controller: controller.to_owned(),
                        summary: "Issue the day's fiscal actions".to_owned(),
                        context: DecisionContext::new("fixture.day.v1", serde_json::json!({})),
                        options: vec![DecisionOption {
                            action: DecisionAction::Command {
                                command: serde_json::to_value(&command).expect("command option"),
                            },
                            ..DecisionOption::new("act", "Issue the fiscal actions")
                        }],
                        deadline: Some(now + SimDuration::days(1)),
                        parent_ticket: None,
                    },
                },
            ),
        )
        .expect("ticket");
    round(canwu);
    let revision = canwu.revision();
    let command_request = CommandRequestId::new(1_000_000 + ticket);
    canwu
        .enqueue_decision(
            canwu.time(),
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(ticket * 10 + 1),
                revision,
                DecisionMutation::Resolve {
                    ticket_id: DecisionTicketId::new(ticket),
                    expected_version: 1,
                    controller_id: controller.to_owned(),
                    policy: policy(controller),
                    decision: PolicyDecision::selected("act", "The day's actions are issued"),
                    command_request_id: Some(command_request),
                },
            )
            .with_command(CommandRequest::new(
                command_request,
                revision,
                CommandEnvelope::new(Issuer::Human(controller.to_owned()), command)
                    .with_authority(CommandAuthority {
                        decision_origin: origin,
                        seat_id: None,
                        permission_profile_id: None,
                        command_subject: Some(subject),
                    })
                    .at_time(canwu.time()),
            )),
        )
        .expect("resolution");
    round(canwu)
}

fn last_rejection(canwu: &Canwu) -> CanwuError {
    match &canwu
        .command_attempts()
        .last()
        .expect("a command attempt")
        .outcome
    {
        CommandAttemptOutcome::Rejected { error } => error.clone(),
        CommandAttemptOutcome::Accepted { .. } => panic!("the command was accepted"),
    }
}

#[test]
fn one_set_settles_two_offices_batches_in_one_round() {
    let fixture = Fixture::new();

    // Each office's batch as its own command, in its own round.
    let mut rounds = fixture.runtime(scenario(acts_for_granary));
    for batch in office_batches(&rounds) {
        enqueue_requests(&mut rounds, &[(PRINCIPAL, batch)]);
        assert_eq!(round(&mut rounds).len(), 2);
    }

    let mut canwu = fixture.runtime(scenario(acts_for_granary));
    register_controllers(&mut canwu);
    let [treasury_batch, granary_batch] = office_batches(&canwu);
    let set = FiscalActionBatchSet {
        batches: vec![treasury_batch.clone(), granary_batch],
    };
    let set_command = fiscal_action_set_command(&set).expect("set command");

    // The treasury's decision covers the treasury's batch, not the
    // granary's, so the set is refused as a whole, naming that batch.
    decide(&mut canwu, TREASURY_OFFICE, 1, set_command.clone());
    let error = last_rejection(&canwu);
    assert_eq!(error.code, ErrorCode::InvalidAuthority);
    assert!(
        error.message.starts_with("batch 1: entry 0: ")
            && error
                .message
                .contains("decision subject does not match the bound institution"),
        "{}",
        error.message
    );
    // A decision in the holder's own name is not admitted for one batch,
    // whose institution must be the decision's subject.
    let single = fiscal_action_command(&treasury_batch).expect("batch command");
    decide(&mut canwu, HOLDER, 2, single);
    assert_eq!(last_rejection(&canwu).code, ErrorCode::InvalidAuthority);
    // In his own name he acts for the granary, which does not let him
    // rebind it.
    let rebind = draft(
        "granary.rebind",
        GRANARY_AUTHORITY,
        FiscalAction::RebindAuthority {
            authority_binding_id: GRANARY_AUTHORITY.to_owned(),
            authorized_actor: Some(PRINCIPAL),
            acting_actor: None,
            authority_basis: None,
        },
    )
    .current(&canwu);
    let overreach = FiscalActionBatchSet {
        batches: vec![treasury_batch, rebind],
    };
    let overreach = fiscal_action_set_command(&overreach).expect("set command");
    decide(&mut canwu, HOLDER, 3, overreach);
    let error = last_rejection(&canwu);
    assert!(
        error.code == ErrorCode::InvalidAuthority
            && error.message.starts_with("batch 1: entry 0: ")
            && error.message.contains("acting actor cannot rebind"),
        "{}",
        error.message
    );
    assert!(FiscalSettledAction::from_events(canwu.events()).is_empty());

    // In his own name the holder issues both offices' batches as one
    // command: one boundary admits it, the next settles both batches.
    assert_eq!(decide(&mut canwu, HOLDER, 4, set_command).len(), 2);
    assert_eq!(
        settled_at(&canwu, "granary.0"),
        settled_at(&canwu, "treasury.0")
    );
    assert_eq!(
        FiscalSettledAction::from_events(canwu.events()),
        FiscalSettledAction::from_events(rounds.events())
    );
    assert_eq!(fiscal_state(&canwu), fiscal_state(&rounds));
    fixture.assert_restore_and_replay(&canwu);
}

#[test]
fn an_own_name_audit_finding_is_admitted_in_a_set_only() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(audits_granary));
    register_controllers(&mut canwu);
    let finding = draft(
        "granary.audit",
        GRANARY_AUDIT,
        FiscalAction::RecordAudit {
            audit_id: "granary.audit.finding".to_owned(),
            scope_binding_id: GRANARY_SCOPE.to_owned(),
            target_id: GRANARY_ASSESSMENT.to_owned(),
            severity: FiscalAuditSeverity::Concern,
            finding: "The assessed quantity was not verified".to_owned(),
            evidence: BTreeSet::new(),
        },
    )
    .current(&canwu);

    // As one batch, a finding's decision must speak for the audited
    // institution, so the auditor's decision in his own name is refused.
    let single = fiscal_action_command(&finding).expect("batch command");
    decide(&mut canwu, HOLDER, 1, single);
    let error = last_rejection(&canwu);
    assert!(
        error.code == ErrorCode::InvalidAuthority
            && error.message.contains("requires its bound auditor"),
        "{}",
        error.message
    );
    assert!(FiscalSettledAction::from_events(canwu.events()).is_empty());

    // In a set, the same decision issues his treasury's batch and the
    // finding, each under the role he holds there.
    let [treasury_batch, _] = office_batches(&canwu);
    let set = FiscalActionBatchSet {
        batches: vec![treasury_batch, finding],
    };
    let set = fiscal_action_set_command(&set).expect("set command");
    assert_eq!(decide(&mut canwu, HOLDER, 2, set).len(), 2);
    for action_id in ["treasury.0", "granary.audit"] {
        assert_eq!(
            outcome(&canwu, action_id).map(|settled| settled.disposition),
            Some(FiscalActionDisposition::Applied),
            "{action_id}"
        );
    }
    let entered_by = fiscal_state(&canwu)
        .audits()
        .find(|finding| finding.id == "granary.audit.finding")
        .and_then(|finding| finding.entered_by.clone());
    assert_eq!(
        entered_by,
        Some(FiscalActionIssuer {
            binding_id: GRANARY_AUDIT.to_owned(),
            role: FiscalIssuerRole::Auditor,
            actor: Some(PRINCIPAL),
            authority_basis: Some(initial_commission(AUDITOR_COMMISSION)),
        })
    );
    fixture.assert_restore_and_replay(&canwu);
}
