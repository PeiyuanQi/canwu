//! Downstream gap G-48: `FiscalAction::RebindAuthority` replaces the actors of
//! an authority binding at runtime.
//!
//! The rebind is admitted from the binding's authorized actor or from its
//! institution deciding as a body, never from an acting actor or a third
//! person. It is re-checked when it settles, names only persons that exist,
//! and installs only a basis that is the current version of its record. The
//! previous acting actor loses the binding in the commit that gives it to the
//! next one, and the binding names who rebound it.

mod support;

use canwu_api::{
    BoundaryRequest, Canwu, CommandAuthority, CommandEnvelope, CommandRequest, CommandRequestId,
    DecisionAction, DecisionAuthority, DecisionContext, DecisionControllerBinding,
    DecisionIngressRequest, DecisionMutation, DecisionOption, DecisionOrigin,
    DecisionPolicyIdentity, DecisionPolicyKind, DecisionRequestId, DecisionTicketDraft,
    DecisionTicketId, DomainRecordVersionRef, EntityRef, ErrorCode, EvidenceRef, Issuer, PersonId,
    PolicyDecision, SimDuration,
};
use canwu_fiscal::{
    FISCAL_ACTING_BASIS_NOT_CURRENT, FISCAL_AUTHORITY_BASIS_NOT_CURRENT, FISCAL_ISSUER_NOT_BOUND,
    FiscalAction, FiscalActionDisposition, FiscalActionIssuer, FiscalActionTarget,
    FiscalIssuerRole, fiscal_action_command, initial_catalog_version,
};
use support::{
    DEPUTY, DEPUTY_COMMISSION, Draft, Fixture, PRINCIPAL, SUCCESSOR, SUCCESSOR_COMMISSION,
    TREASURY_AUTHORITY, admission_rejection, assessment, current_commission, draft,
    enqueue_renewal, fiscal_state, initial_commission, initial_warrant, outcome, scenario,
    sealed_fiscal_state, settle, settled_at, submit, submit_sealed, tick_at, treasury,
    treasury_scope,
};

/// A decision controller that decides for the treasury.
struct Office {
    controller: &'static str,
    responsible_actor: Option<PersonId>,
}

/// The treasury deciding as a body: no responsible actor.
const OFFICE: Office = Office {
    controller: "treasury-office",
    responsible_actor: None,
};
/// The treasury's decision with its acting actor as the responsible actor.
const DEPUTY_OFFICE: Office = Office {
    controller: "treasury-office-deputy",
    responsible_actor: Some(DEPUTY),
};
/// The treasury's decision with a person the binding does not name.
const THIRD_OFFICE: Office = Office {
    controller: "treasury-office-third",
    responsible_actor: Some(SUCCESSOR),
};

fn rebind(
    action_id: &str,
    authorized_actor: Option<PersonId>,
    acting: Option<(PersonId, DomainRecordVersionRef)>,
) -> Draft {
    let (acting_actor, authority_basis) = acting.unzip();
    draft(
        action_id,
        TREASURY_AUTHORITY,
        FiscalAction::RebindAuthority {
            authority_binding_id: TREASURY_AUTHORITY.to_owned(),
            authorized_actor,
            acting_actor,
            authority_basis,
        },
    )
}

/// An assessment on its own accounting partition, so no two of them conflict.
fn assess(action_id: &str) -> Draft {
    assessment(action_id, TREASURY_AUTHORITY, &treasury_scope(0), action_id)
}

fn applied(canwu: &Canwu, action_id: &str) -> bool {
    outcome(canwu, action_id)
        .is_some_and(|outcome| outcome.disposition == FiscalActionDisposition::Applied)
}

fn rejection_reason(canwu: &Canwu, action_id: &str) -> String {
    let outcome = outcome(canwu, action_id).unwrap_or_else(|| panic!("{action_id} not settled"));
    assert_eq!(outcome.disposition, FiscalActionDisposition::Rejected);
    outcome.reason
}

/// Asserts that admission rejected the command as unauthorized, so the action
/// never reached settlement.
fn assert_unauthorized(canwu: &Canwu, action_id: &str, message: &str) {
    let error = admission_rejection(canwu, action_id)
        .unwrap_or_else(|| panic!("{action_id} was not rejected at admission"));
    assert_eq!(error.code, ErrorCode::InvalidAuthority);
    assert!(
        error.message.contains(message),
        "{action_id}: {}",
        error.message
    );
    assert!(outcome(canwu, action_id).is_none());
}

impl Office {
    fn policy(&self) -> DecisionPolicyIdentity {
        DecisionPolicyIdentity::new(DecisionPolicyKind::Human, self.controller, "1")
    }
}

/// Registers the three controllers that decide for the treasury.
fn register_offices(canwu: &mut Canwu) {
    for (request, office) in (1..).zip([&OFFICE, &DEPUTY_OFFICE, &THIRD_OFFICE]) {
        canwu
            .enqueue_decision(
                canwu.time(),
                0,
                DecisionIngressRequest::new(
                    DecisionRequestId::new(request),
                    canwu.revision(),
                    DecisionMutation::RegisterController {
                        controller: DecisionControllerBinding::new(
                            office.controller,
                            office.policy(),
                            DecisionAuthority::Institution {
                                institution: treasury(),
                                responsible_actor: office.responsible_actor,
                            },
                        )
                        .with_command_subject(treasury()),
                    },
                ),
            )
            .expect("controller registration");
    }
    settle(canwu);
}

/// Has the treasury decide the action through one of its controllers: a
/// ticket offers the command, and the controller selects it.
fn decide_as_institution(canwu: &mut Canwu, office: &Office, ticket: u64, action: Draft) {
    let command = fiscal_action_command(&action.current(canwu)).expect("fiscal command");
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
                        definition: "fixture.treasury-office".to_owned(),
                        decision_maker: treasury(),
                        assigned_controller: office.controller.to_owned(),
                        summary: "The treasury decides one fiscal action".to_owned(),
                        context: DecisionContext::new("fixture.office.v1", serde_json::json!({})),
                        options: vec![DecisionOption {
                            action: DecisionAction::Command {
                                command: serde_json::to_value(&command).expect("command option"),
                            },
                            ..DecisionOption::new("act", "Issue the fiscal action")
                        }],
                        deadline: Some(now + SimDuration::days(1)),
                        parent_ticket: None,
                    },
                },
            ),
        )
        .expect("ticket");
    settle(canwu);
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
                    controller_id: office.controller.to_owned(),
                    policy: office.policy(),
                    decision: PolicyDecision::selected("act", "The office issues the action"),
                    command_request_id: Some(command_request),
                },
            )
            .with_command(CommandRequest::new(
                command_request,
                revision,
                CommandEnvelope::new(Issuer::Human(office.controller.to_owned()), command)
                    .with_authority(CommandAuthority {
                        decision_origin: DecisionOrigin::Institution {
                            institution: treasury(),
                            responsible_actor: office.responsible_actor,
                        },
                        seat_id: None,
                        permission_profile_id: None,
                        command_subject: Some(treasury()),
                    })
                    .at_time(canwu.time()),
            )),
        )
        .expect("resolution");
    settle(canwu);
}

#[test]
#[allow(clippy::too_many_lines)]
fn gap_g48_fiscal_rebind_authority() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|state| {
        let binding = state
            .authority_binding_mut(TREASURY_AUTHORITY)
            .expect("treasury binding");
        binding.acting_actor = Some(DEPUTY);
        binding.authority_basis = Some(initial_commission(DEPUTY_COMMISSION));
    }));
    register_offices(&mut canwu);

    // An acting actor acts for the binding but cannot rebind it.
    submit(
        &mut canwu,
        vec![
            (DEPUTY, assess("deputy.current")),
            (DEPUTY, rebind("rebind.by-deputy", Some(DEPUTY), None)),
        ],
    );
    assert!(applied(&canwu, "deputy.current"));
    assert_unauthorized(&canwu, "rebind.by-deputy", "acting actor cannot rebind");

    // The application renews the commission. The binding still cites the old
    // version, so the acting actor is stranded: this is the gap.
    enqueue_renewal(&mut canwu, DEPUTY_COMMISSION);
    settle(&mut canwu);
    submit(&mut canwu, vec![(DEPUTY, assess("deputy.stranded"))]);
    assert_unauthorized(&canwu, "deputy.stranded", FISCAL_ACTING_BASIS_NOT_CURRENT);

    // Deciding through the institution does not change who decides. As the
    // responsible actor of an institution decision the acting actor is still
    // the acting actor, and a person the binding does not name holds no role,
    // so neither rebinds the binding or installs an auditor on it.
    decide_as_institution(
        &mut canwu,
        &DEPUTY_OFFICE,
        1,
        rebind("rebind.deputy-office", Some(DEPUTY), None),
    );
    assert_unauthorized(&canwu, "rebind.deputy-office", "acting actor cannot rebind");
    let auditor_basis = current_commission(&canwu, SUCCESSOR_COMMISSION);
    decide_as_institution(
        &mut canwu,
        &DEPUTY_OFFICE,
        2,
        draft(
            "rebind-audit.deputy-office",
            TREASURY_AUTHORITY,
            FiscalAction::RebindAudit {
                audit_binding_id: "audit.treasury".to_owned(),
                auditor: Some(SUCCESSOR),
                authority_basis: Some(auditor_basis),
            },
        ),
    );
    assert_unauthorized(
        &canwu,
        "rebind-audit.deputy-office",
        "acting actor cannot rebind",
    );
    decide_as_institution(
        &mut canwu,
        &THIRD_OFFICE,
        3,
        rebind("rebind.third-office", Some(SUCCESSOR), None),
    );
    assert_unauthorized(
        &canwu,
        "rebind.third-office",
        "decision origin does not satisfy the authority binding",
    );

    // A rebind installs only a basis that is current, and only an application
    // record of a kind the plugin was configured to read: not a fiscal record,
    // and not a kind it cannot read. Every person it names must exist.
    submit(
        &mut canwu,
        vec![
            (
                PRINCIPAL,
                rebind(
                    "rebind.superseded-basis",
                    Some(PRINCIPAL),
                    Some((DEPUTY, initial_commission(DEPUTY_COMMISSION))),
                ),
            ),
            (
                PRINCIPAL,
                rebind(
                    "rebind.fiscal-record",
                    Some(PRINCIPAL),
                    Some((DEPUTY, initial_catalog_version())),
                ),
            ),
            (
                PRINCIPAL,
                rebind(
                    "rebind.unread-kind",
                    Some(PRINCIPAL),
                    Some((DEPUTY, initial_warrant())),
                ),
            ),
            (
                PRINCIPAL,
                rebind("rebind.unknown-person", Some(PersonId::new(9_999)), None),
            ),
        ],
    );
    assert_unauthorized(
        &canwu,
        "rebind.superseded-basis",
        FISCAL_AUTHORITY_BASIS_NOT_CURRENT,
    );
    for action_id in [
        "rebind.fiscal-record",
        "rebind.unread-kind",
        "rebind.unknown-person",
    ] {
        let error = admission_rejection(&canwu, action_id)
            .unwrap_or_else(|| panic!("{action_id} was not rejected at admission"));
        assert_eq!(error.code, ErrorCode::EntityNotFound, "{action_id}");
        assert!(outcome(&canwu, action_id).is_none());
    }

    // The principal re-points the binding at the renewed commission, and the
    // acting actor is admitted again. The binding names who rebound it.
    let renewed = current_commission(&canwu, DEPUTY_COMMISSION);
    submit(
        &mut canwu,
        vec![(
            PRINCIPAL,
            rebind(
                "rebind.renewed",
                Some(PRINCIPAL),
                Some((DEPUTY, renewed.clone())),
            ),
        )],
    );
    assert!(applied(&canwu, "rebind.renewed"));
    let state = fiscal_state(&canwu);
    let binding = &state
        .authority_binding(TREASURY_AUTHORITY)
        .expect("authority binding");
    assert_eq!(binding.authority_basis.as_ref(), Some(&renewed));
    let rebound_by = binding.rebound_by.as_ref().expect("rebinding provenance");
    assert_eq!(rebound_by.action_id, "rebind.renewed");
    assert_eq!(
        rebound_by.issuer,
        FiscalActionIssuer {
            binding_id: TREASURY_AUTHORITY.to_owned(),
            role: FiscalIssuerRole::Principal,
            actor: Some(PRINCIPAL),
            authority_basis: None,
        }
    );
    assert_eq!(rebound_by.rebound_at, settled_at(&canwu, "rebind.renewed"));
    // The settled event names the institution and the persons the binding now
    // names, which the engine checked against its entity registry.
    assert!(canwu.events().iter().any(|event| {
        [
            treasury(),
            EntityRef::Person(PRINCIPAL),
            EntityRef::Person(DEPUTY),
        ]
        .iter()
        .all(|entity| event.affected_entities.contains(entity))
    }));
    submit(&mut canwu, vec![(DEPUTY, assess("deputy.renewed"))]);
    assert!(applied(&canwu, "deputy.renewed"));

    // One admission cut holds an action of the acting actor, the rebind that
    // replaces him, a second action of his, and one of his successor. They
    // settle in that order: the binding changes hands in one commit, with no
    // action applied under both actors and none under neither.
    let successor_basis = current_commission(&canwu, SUCCESSOR_COMMISSION);
    let replace = rebind(
        "rebind.successor",
        Some(PRINCIPAL),
        Some((SUCCESSOR, successor_basis.clone())),
    );
    submit(
        &mut canwu,
        vec![
            (DEPUTY, assess("deputy.before-rebind")),
            (PRINCIPAL, replace),
            (DEPUTY, assess("deputy.after-rebind")),
            (SUCCESSOR, assess("successor.too-early")),
        ],
    );
    assert!(applied(&canwu, "deputy.before-rebind"));
    assert!(applied(&canwu, "rebind.successor"));
    assert!(
        rejection_reason(&canwu, "deputy.after-rebind").ends_with(FISCAL_ACTING_BASIS_NOT_CURRENT)
    );
    assert_unauthorized(&canwu, "successor.too-early", "requires its bound actor");
    submit(
        &mut canwu,
        vec![
            (DEPUTY, assess("deputy.replaced")),
            (SUCCESSOR, assess("successor.bound")),
        ],
    );
    assert_unauthorized(&canwu, "deputy.replaced", "requires its bound actor");
    assert!(applied(&canwu, "successor.bound"));

    // Settlement re-checks the basis a rebind installs. Here the commission is
    // renewed in the boundary that admits the rebind, so the version it cites
    // is no longer current when the action settles, and the binding is kept.
    enqueue_renewal(&mut canwu, SUCCESSOR_COMMISSION);
    submit(
        &mut canwu,
        vec![(
            PRINCIPAL,
            rebind(
                "rebind.renewed-meanwhile",
                Some(PRINCIPAL),
                Some((SUCCESSOR, successor_basis)),
            ),
        )],
    );
    assert!(
        rejection_reason(&canwu, "rebind.renewed-meanwhile")
            .ends_with(FISCAL_AUTHORITY_BASIS_NOT_CURRENT)
    );
    assert_eq!(
        fiscal_state(&canwu)
            .authority_binding(TREASURY_AUTHORITY)
            .expect("authority binding")
            .rebound_by
            .as_ref()
            .expect("rebinding provenance")
            .action_id,
        "rebind.successor"
    );

    // The principal hands the binding over. An action of his that settles
    // after the handover is rejected, and from then on only his successor is
    // admitted.
    submit(
        &mut canwu,
        vec![
            (PRINCIPAL, rebind("rebind.handover", Some(SUCCESSOR), None)),
            (PRINCIPAL, assess("principal.after-handover")),
        ],
    );
    assert!(applied(&canwu, "rebind.handover"));
    assert!(
        rejection_reason(&canwu, "principal.after-handover").ends_with(FISCAL_ISSUER_NOT_BOUND)
    );
    submit(
        &mut canwu,
        vec![
            (PRINCIPAL, assess("principal.replaced")),
            (SUCCESSOR, assess("successor.principal")),
        ],
    );
    assert_unauthorized(&canwu, "principal.replaced", "requires its bound actor");
    assert!(applied(&canwu, "successor.principal"));

    // The institution deciding as a body, a controller with no responsible
    // actor, can rebind whoever the authorized actor is, so a binding whose
    // principal can no longer act is not stranded. While the binding names an
    // authorized actor, that controller is admitted for nothing else.
    decide_as_institution(&mut canwu, &OFFICE, 4, assess("office.assessment"));
    assert_unauthorized(
        &canwu,
        "office.assessment",
        "decision origin does not satisfy the authority binding",
    );
    decide_as_institution(
        &mut canwu,
        &OFFICE,
        5,
        rebind("rebind.by-institution", Some(PRINCIPAL), None),
    );
    assert!(applied(&canwu, "rebind.by-institution"));
    let state = fiscal_state(&canwu);
    let binding = &state
        .authority_binding(TREASURY_AUTHORITY)
        .expect("authority binding");
    assert_eq!(binding.authorized_actor, Some(PRINCIPAL));
    assert_eq!(
        binding
            .rebound_by
            .as_ref()
            .expect("rebinding provenance")
            .issuer,
        FiscalActionIssuer {
            binding_id: TREASURY_AUTHORITY.to_owned(),
            role: FiscalIssuerRole::Institution,
            actor: None,
            authority_basis: None,
        }
    );
    submit(&mut canwu, vec![(PRINCIPAL, assess("principal.restored"))]);
    assert!(applied(&canwu, "principal.restored"));

    // The institution role is the power to appoint, and that includes naming
    // no one. On a binding with no authorized actor the same controller then
    // holds the principal role itself, which is why a host registers such a
    // controller only for the body that appoints the binding's actors.
    decide_as_institution(&mut canwu, &OFFICE, 6, rebind("rebind.vacate", None, None));
    assert!(applied(&canwu, "rebind.vacate"));
    decide_as_institution(&mut canwu, &OFFICE, 7, assess("office.as-principal"));
    assert!(applied(&canwu, "office.as-principal"));
    submit(&mut canwu, vec![(PRINCIPAL, assess("principal.vacated"))]);
    assert_unauthorized(&canwu, "principal.vacated", "requires its bound actor");

    fixture.assert_restore_and_replay(&canwu);
}

/// A sealed run decides a rebind, the acting actor it installs, and an idle
/// boundary exactly as its full-retention replay does. Each decision reads the
/// current-version provenance index, never evidence the seal removed.
#[test]
#[allow(clippy::too_many_lines)]
fn sealed_run_rebinds_and_idles_like_its_replay() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|state| {
        // A published report would keep its evidence from sealing.
        state.remove_observer(support::OBSERVER);
        let binding = state
            .authority_binding_mut(TREASURY_AUTHORITY)
            .expect("treasury binding");
        binding.acting_actor = Some(DEPUTY);
        binding.authority_basis = Some(initial_commission(DEPUTY_COMMISSION));
    }));

    // A boundary writes the fiscal state, and two renewals leave a superseded
    // and a current commission version. The seal removes the boundaries that
    // established all three.
    submit(&mut canwu, vec![(DEPUTY, assess("deputy.before-seal"))]);
    enqueue_renewal(&mut canwu, DEPUTY_COMMISSION);
    settle(&mut canwu);
    let superseded = current_commission(&canwu, DEPUTY_COMMISSION);
    enqueue_renewal(&mut canwu, DEPUTY_COMMISSION);
    settle(&mut canwu);
    let current = current_commission(&canwu, DEPUTY_COMMISSION);
    // Later boundaries admit the emissions, so the history can seal.
    for _ in 0..2 {
        canwu
            .settle_boundary(BoundaryRequest::at(canwu.time()))
            .expect("a quiet boundary before sealing");
    }
    let mut sealed = canwu.into_compacted().expect("compact mode should start");
    let segments = vec![
        sealed
            .seal_evidence()
            .expect("settled history should seal")
            .expect("settled history should contain evidence"),
    ];
    assert!(
        sealed
            .archived_evidence_receipt(&EvidenceRef::DomainRecordVersion(superseded.clone()))
            .is_none()
    );

    submit_sealed(
        &mut sealed,
        vec![
            (
                PRINCIPAL,
                rebind(
                    "rebind.superseded",
                    Some(PRINCIPAL),
                    Some((DEPUTY, superseded)),
                ),
            ),
            (
                PRINCIPAL,
                rebind(
                    "rebind.current",
                    Some(PRINCIPAL),
                    Some((DEPUTY, current.clone())),
                ),
            ),
        ],
    );
    submit_sealed(&mut sealed, vec![(DEPUTY, assess("deputy.after-seal"))]);
    sealed
        .enqueue_plugin_ingress(tick_at(sealed.time()))
        .expect("tick ingress");
    sealed
        .advance_canonical(SimDuration::minutes(1))
        .expect("an idle boundary");
    let state = sealed_fiscal_state(&sealed);
    let office = FiscalActionTarget::Office(treasury());
    assert!(state.recent_outcome(&office, "rebind.superseded").is_none());
    assert_eq!(
        state
            .recent_outcome(&office, "rebind.current")
            .expect("the current rebind's outcome")
            .disposition,
        FiscalActionDisposition::Applied
    );
    assert_eq!(
        state
            .recent_outcome(
                &FiscalActionTarget::Ledger(treasury_scope(0)),
                "deputy.after-seal"
            )
            .expect("the deputy's assessment outcome")
            .disposition,
        FiscalActionDisposition::Applied
    );
    assert_eq!(
        state
            .authority_binding(TREASURY_AUTHORITY)
            .expect("authority binding")
            .authority_basis,
        Some(current)
    );

    let plugins = fixture.plugins();
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
