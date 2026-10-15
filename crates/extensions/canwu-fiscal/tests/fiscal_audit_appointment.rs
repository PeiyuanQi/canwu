//! Who appoints and removes an institution's auditors.
//!
//! The treasury's appointment policy names a council as the appointer, the
//! granary and one person by name as nominators and petitioners, and lets
//! the council appoint without a nomination but not remove without a
//! petition. Nominations and petitions are recorded with their party and stay
//! pending until the council accepts or dismisses them or a later action
//! supersedes them; the binding and each finding name who nominated and who
//! decided, and a petition names who petitioned and who decided. Every
//! refusal is recorded: at admission as a rejected command, at settlement as
//! a rejected outcome. The treasury itself can no longer install or remove
//! its auditor, and an institution without a policy has no runtime
//! appointments at all. Restore and exact replay agree throughout.

mod support;

use canwu_api::{
    Canwu, DomainRecordVersionRef, EntityRef, ErrorCode, EvidenceRef, OrganizationId, PersonId,
    Scenario, SimTime,
};
use canwu_fiscal::{
    FISCAL_APPOINTMENT_POLICY_MISSING, FISCAL_ISSUER_NOT_APPOINTER, FISCAL_ISSUER_NOT_BOUND,
    FISCAL_ISSUER_NOT_LISTED, FISCAL_PETITION_REQUIRED, FISCAL_PROPOSAL_NOT_PENDING, FiscalAction,
    FiscalActionDisposition, FiscalActionIssuer, FiscalAppointmentDecision, FiscalAppointmentParty,
    FiscalAppointmentPolicy, FiscalAssessment, FiscalAuditBinding, FiscalAuditProposal,
    FiscalAuditProposalKind, FiscalAuditSeverity, FiscalAuthorityBinding, FiscalIssuerRole,
    FiscalPaymentForm, FiscalProposalOutcome, FiscalProposalRef, FiscalState,
    MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY, MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY,
};
use std::collections::BTreeSet;
use support::{
    AUDITOR, AUDITOR_COMMISSION, DEPUTY, DEPUTY_COMMISSION, Draft, Fixture, GRANARY_AUTHORITY,
    GRANARY_PRINCIPAL, PRINCIPAL, RULE, SECOND_AUDITOR, SUCCESSOR, TREASURY_AUTHORITY,
    admission_rejection, draft, fiscal_state, granary, initial_commission, initial_warrant,
    outcome, scenario, submit, treasury, treasury_scope,
};

const POLICY: &str = "appointment.treasury";
const COUNCIL_AUTHORITY: &str = "authority.council";
/// The person who decides for the council.
const COUNCIL_PRINCIPAL: PersonId = SUCCESSOR;
const AUDIT: &str = "audit.treasury";
const SECOND_AUDIT: &str = "audit.treasury.second";
const ASSESSMENT: &str = "treasury.assessment";

fn council() -> EntityRef {
    EntityRef::Organization(OrganizationId::new(3))
}

/// The treasury's policy and the council's authority binding, an assessment
/// to audit, and whatever `configure` adds; the council is a registered
/// institution.
fn appointment_scenario(configure: impl FnOnce(&mut FiscalState)) -> Scenario {
    let mut scenario = scenario(|state| {
        state.authority_bindings.insert(
            COUNCIL_AUTHORITY.to_owned(),
            FiscalAuthorityBinding {
                id: COUNCIL_AUTHORITY.to_owned(),
                institution: council(),
                authorized_actor: Some(COUNCIL_PRINCIPAL),
                acting_actor: None,
                authority_basis: None,
                rebound_by: None,
            },
        );
        let listed = BTreeSet::from([
            FiscalAppointmentParty::Institution {
                institution: granary(),
            },
            FiscalAppointmentParty::Actor { actor: DEPUTY },
        ]);
        state.audit_appointment_policies.insert(
            POLICY.to_owned(),
            FiscalAppointmentPolicy {
                id: POLICY.to_owned(),
                institution: treasury(),
                appointer: council(),
                nominators: listed.clone(),
                petitioners: listed,
                appoints_without_nomination: true,
                removes_without_petition: false,
            },
        );
        state.assessments.insert(
            ASSESSMENT.to_owned(),
            FiscalAssessment {
                id: ASSESSMENT.to_owned(),
                rule_id: RULE.to_owned(),
                scope_binding_id: treasury_scope(0),
                accounting_cycle_id: "cycle.authored".to_owned(),
                quantity: 500,
                unit: "grain".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: None,
                created_at: SimTime::EPOCH,
            },
        );
        configure(state);
    });
    scenario.entities.extend([council()]);
    scenario
}

/// The treasury's audit binding as the scenario installs it.
fn installed_auditor(state: &mut FiscalState) {
    state.audit_bindings.insert(
        AUDIT.to_owned(),
        FiscalAuditBinding {
            id: AUDIT.to_owned(),
            institution: treasury(),
            auditor: AUDITOR,
            authority_basis: initial_commission(AUDITOR_COMMISSION),
            rebound_by: None,
            nomination: None,
        },
    );
}

fn nominate(
    action_id: &str,
    binding: &str,
    candidate: PersonId,
    authority_basis: DomainRecordVersionRef,
) -> Draft {
    nominate_for(AUDIT, action_id, binding, candidate, authority_basis)
}

/// A nomination for the treasury's audit binding `audit_binding`.
fn nominate_for(
    audit_binding: &str,
    action_id: &str,
    binding: &str,
    candidate: PersonId,
    authority_basis: DomainRecordVersionRef,
) -> Draft {
    draft(
        action_id,
        binding,
        FiscalAction::NominateAuditor {
            nomination_id: format!("{action_id}.nomination"),
            audit_binding_id: audit_binding.to_owned(),
            institution: treasury(),
            candidate,
            authority_basis,
        },
    )
}

fn petition(action_id: &str, binding: &str) -> Draft {
    draft(
        action_id,
        binding,
        FiscalAction::PetitionAuditorRemoval {
            petition_id: format!("{action_id}.petition"),
            audit_binding_id: AUDIT.to_owned(),
            grounds: BTreeSet::from([EvidenceRef::DomainRecordVersion(initial_warrant())]),
        },
    )
}

fn decide(action_id: &str, binding: &str, decision: FiscalAppointmentDecision) -> Draft {
    decide_on(AUDIT, action_id, binding, decision)
}

/// A decision on the audit binding `audit_binding`.
fn decide_on(
    audit_binding: &str,
    action_id: &str,
    binding: &str,
    decision: FiscalAppointmentDecision,
) -> Draft {
    draft(
        action_id,
        binding,
        FiscalAction::DecideAuditAppointment {
            audit_binding_id: audit_binding.to_owned(),
            decision,
        },
    )
}

fn rebind_audit(action_id: &str, binding: &str, auditor: Option<PersonId>) -> Draft {
    draft(
        action_id,
        binding,
        FiscalAction::RebindAudit {
            audit_binding_id: AUDIT.to_owned(),
            auditor,
            authority_basis: auditor.map(|_| initial_commission(DEPUTY_COMMISSION)),
        },
    )
}

fn audit(action_id: &str) -> Draft {
    draft(
        action_id,
        AUDIT,
        FiscalAction::RecordAudit {
            audit_id: format!("{action_id}.finding"),
            target_id: ASSESSMENT.to_owned(),
            severity: FiscalAuditSeverity::Concern,
            finding: "The assessed quantity was not verified".to_owned(),
            evidence: BTreeSet::new(),
        },
    )
}

fn issuer(binding: &str, role: FiscalIssuerRole, actor: PersonId) -> FiscalActionIssuer {
    FiscalActionIssuer {
        binding_id: binding.to_owned(),
        role,
        actor: Some(actor),
        authority_basis: None,
    }
}

fn council_issuer(actor: PersonId) -> FiscalActionIssuer {
    issuer(COUNCIL_AUTHORITY, FiscalIssuerRole::Principal, actor)
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

/// Asserts that admission rejected the command with `code`, so the action
/// never reached settlement.
fn assert_rejected(canwu: &Canwu, action_id: &str, code: &ErrorCode, message: &str) {
    let error = admission_rejection(canwu, action_id)
        .unwrap_or_else(|| panic!("{action_id} was not rejected at admission"));
    assert_eq!(&error.code, code, "{action_id}: {}", error.message);
    assert!(
        error.message.contains(message),
        "{action_id}: {}",
        error.message
    );
    assert!(outcome(canwu, action_id).is_none());
}

fn assert_unauthorized(canwu: &Canwu, action_id: &str, message: &str) {
    assert_rejected(canwu, action_id, &ErrorCode::InvalidAuthority, message);
}

fn proposal(canwu: &Canwu, id: &str) -> FiscalAuditProposal {
    fiscal_state(canwu).audit_proposals[id].clone()
}

/// Asserts how a proposal was resolved, and by which action.
fn assert_resolved(canwu: &Canwu, id: &str, outcome: FiscalProposalOutcome, action_id: &str) {
    let resolution = proposal(canwu, id)
        .resolution
        .unwrap_or_else(|| panic!("{id} is still pending"));
    assert_eq!(resolution.outcome, outcome, "{id}");
    assert_eq!(resolution.resolved_by.action_id, action_id, "{id}");
}

#[test]
#[allow(clippy::too_many_lines)]
fn the_appointer_appoints_an_auditor_with_and_without_a_nomination() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(appointment_scenario(|_| {}));

    // A person the policy lists by name, acting under the policy, and the
    // granary, acting under its own authority binding, each nominate a
    // candidate for a binding the treasury does not have yet.
    submit(
        &mut canwu,
        vec![
            (
                DEPUTY,
                nominate(
                    "nominate.deputy",
                    POLICY,
                    AUDITOR,
                    initial_commission(AUDITOR_COMMISSION),
                ),
            ),
            (
                GRANARY_PRINCIPAL,
                nominate(
                    "nominate.granary",
                    GRANARY_AUTHORITY,
                    SECOND_AUDITOR,
                    initial_commission(DEPUTY_COMMISSION),
                ),
            ),
        ],
    );
    assert!(applied(&canwu, "nominate.deputy"));
    assert!(applied(&canwu, "nominate.granary"));
    let deputy = issuer(POLICY, FiscalIssuerRole::ListedActor, DEPUTY);
    let nomination = proposal(&canwu, "nominate.deputy.nomination");
    assert!(nomination.is_pending());
    assert_eq!(
        nomination.party,
        FiscalAppointmentParty::Actor { actor: DEPUTY }
    );
    assert_eq!(nomination.proposed_by.issuer, deputy);
    assert_eq!(
        nomination.kind,
        FiscalAuditProposalKind::Nomination {
            candidate: AUDITOR,
            authority_basis: initial_commission(AUDITOR_COMMISSION),
        }
    );
    assert_eq!(
        proposal(&canwu, "nominate.granary.nomination").party,
        FiscalAppointmentParty::Institution {
            institution: granary(),
        }
    );
    assert!(!fiscal_state(&canwu).audit_bindings.contains_key(AUDIT));

    // The council accepts the person's nomination. The binding names who
    // decided and who nominated, and the other nomination is superseded.
    submit(
        &mut canwu,
        vec![(
            COUNCIL_PRINCIPAL,
            decide(
                "decide.accept",
                COUNCIL_AUTHORITY,
                FiscalAppointmentDecision::AcceptNomination {
                    nomination_id: "nominate.deputy.nomination".to_owned(),
                },
            ),
        )],
    );
    assert!(applied(&canwu, "decide.accept"));
    let binding = fiscal_state(&canwu).audit_bindings[AUDIT].clone();
    assert_eq!(binding.institution, treasury());
    assert_eq!(binding.auditor, AUDITOR);
    let decided = binding
        .rebound_by
        .clone()
        .expect("the appointer's decision");
    assert_eq!(decided.action_id, "decide.accept");
    assert_eq!(decided.issuer, council_issuer(COUNCIL_PRINCIPAL));
    let nominated = FiscalProposalRef {
        proposal_id: "nominate.deputy.nomination".to_owned(),
        proposed_by: deputy,
    };
    assert_eq!(binding.nomination, Some(nominated.clone()));
    assert_resolved(
        &canwu,
        "nominate.deputy.nomination",
        FiscalProposalOutcome::Accepted,
        "decide.accept",
    );
    assert_resolved(
        &canwu,
        "nominate.granary.nomination",
        FiscalProposalOutcome::Superseded,
        "decide.accept",
    );

    // The auditor's finding records who nominated him and who decided.
    submit(&mut canwu, vec![(AUDITOR, audit("audit.first"))]);
    let finding = fiscal_state(&canwu).audits["audit.first.finding"].clone();
    assert_eq!(finding.appointed_by, Some(decided));
    assert_eq!(finding.nomination, Some(nominated.clone()));

    // The policy lets the council appoint without a nomination, but not
    // remove without a petition, and replacing the seated auditor by another
    // is a removal too: his tenure holds. Into a vacant binding the council
    // appoints directly, and that binding names no nominator.
    let direct = || FiscalAppointmentDecision::Appoint {
        institution: treasury(),
        auditor: SECOND_AUDITOR,
        authority_basis: initial_commission(DEPUTY_COMMISSION),
    };
    submit(
        &mut canwu,
        vec![
            (
                COUNCIL_PRINCIPAL,
                decide("decide.displace", COUNCIL_AUTHORITY, direct()),
            ),
            (
                COUNCIL_PRINCIPAL,
                decide_on(SECOND_AUDIT, "decide.direct", COUNCIL_AUTHORITY, direct()),
            ),
        ],
    );
    assert_unauthorized(&canwu, "decide.displace", FISCAL_PETITION_REQUIRED);
    assert!(applied(&canwu, "decide.direct"));
    let state = fiscal_state(&canwu);
    assert_eq!(state.audit_bindings[AUDIT].auditor, AUDITOR);
    let second = &state.audit_bindings[SECOND_AUDIT];
    assert_eq!(second.auditor, SECOND_AUDITOR);
    assert_eq!(second.nomination, None);
    assert_eq!(
        second.rebound_by.as_ref().map(|by| by.action_id.as_str()),
        Some("decide.direct")
    );
    assert_eq!(
        state.audits["audit.first.finding"].nomination,
        Some(nominated)
    );

    fixture.assert_restore_and_replay(&canwu);
}

#[test]
fn a_nomination_from_a_party_the_policy_does_not_list_is_refused() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(appointment_scenario(|_| {}));
    let basis = || initial_commission(AUDITOR_COMMISSION);
    submit(
        &mut canwu,
        vec![
            // The audited institution, the appointer, and a person the policy
            // does not name are not nominators.
            (
                PRINCIPAL,
                nominate("nominate.audited", TREASURY_AUTHORITY, AUDITOR, basis()),
            ),
            (
                COUNCIL_PRINCIPAL,
                nominate("nominate.appointer", COUNCIL_AUTHORITY, AUDITOR, basis()),
            ),
            (
                AUDITOR,
                nominate("nominate.unlisted-person", POLICY, AUDITOR, basis()),
            ),
            // A listed person acts under the policy for proposals only.
            (
                DEPUTY,
                decide(
                    "decide.listed-person",
                    POLICY,
                    FiscalAppointmentDecision::Appoint {
                        institution: treasury(),
                        auditor: AUDITOR,
                        authority_basis: basis(),
                    },
                ),
            ),
        ],
    );
    assert_unauthorized(&canwu, "nominate.audited", FISCAL_ISSUER_NOT_LISTED);
    assert_unauthorized(&canwu, "nominate.appointer", FISCAL_ISSUER_NOT_LISTED);
    assert_unauthorized(&canwu, "nominate.unlisted-person", FISCAL_ISSUER_NOT_LISTED);
    assert_unauthorized(
        &canwu,
        "decide.listed-person",
        "admits only the nominations and removal petitions",
    );
    let state = fiscal_state(&canwu);
    assert!(state.audit_proposals.is_empty());
    assert!(state.audit_bindings.is_empty());

    fixture.assert_restore_and_replay(&canwu);
}

#[test]
#[allow(clippy::too_many_lines)]
fn a_petitioner_impeaches_the_auditor_and_the_appointer_removes_him() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(appointment_scenario(installed_auditor));

    // The granary and the listed person each petition for the auditor's
    // removal, on grounds.
    submit(
        &mut canwu,
        vec![
            (
                GRANARY_PRINCIPAL,
                petition("petition.granary", GRANARY_AUTHORITY),
            ),
            (DEPUTY, petition("petition.deputy", POLICY)),
        ],
    );
    let petitioned = proposal(&canwu, "petition.granary.petition");
    assert!(petitioned.is_pending());
    assert_eq!(
        petitioned.kind,
        FiscalAuditProposalKind::RemovalPetition {
            auditor: AUDITOR,
            grounds: BTreeSet::from([EvidenceRef::DomainRecordVersion(initial_warrant())]),
        }
    );
    let granary_issuer = issuer(
        GRANARY_AUTHORITY,
        FiscalIssuerRole::Principal,
        GRANARY_PRINCIPAL,
    );
    assert_eq!(petitioned.proposed_by.issuer, granary_issuer);

    // The policy requires a petition for a removal, and only the appointer
    // decides: a petitioner cannot grant his own petition.
    submit(
        &mut canwu,
        vec![
            (
                COUNCIL_PRINCIPAL,
                decide(
                    "decide.unpetitioned",
                    COUNCIL_AUTHORITY,
                    FiscalAppointmentDecision::Remove,
                ),
            ),
            (
                GRANARY_PRINCIPAL,
                decide(
                    "decide.by-petitioner",
                    GRANARY_AUTHORITY,
                    FiscalAppointmentDecision::AcceptPetition {
                        petition_id: "petition.granary.petition".to_owned(),
                    },
                ),
            ),
        ],
    );
    assert_unauthorized(&canwu, "decide.unpetitioned", FISCAL_PETITION_REQUIRED);
    assert_unauthorized(&canwu, "decide.by-petitioner", FISCAL_ISSUER_NOT_APPOINTER);

    // Authority is checked again at settlement: the council rebinds its
    // principal in the boundary that settles his decision, which is then
    // rejected, and the petition stays pending.
    let accept = || FiscalAppointmentDecision::AcceptPetition {
        petition_id: "petition.granary.petition".to_owned(),
    };
    submit(
        &mut canwu,
        vec![
            (
                COUNCIL_PRINCIPAL,
                draft(
                    "rebind.council",
                    COUNCIL_AUTHORITY,
                    FiscalAction::RebindAuthority {
                        authority_binding_id: COUNCIL_AUTHORITY.to_owned(),
                        authorized_actor: Some(GRANARY_PRINCIPAL),
                        acting_actor: None,
                        authority_basis: None,
                    },
                ),
            ),
            (
                COUNCIL_PRINCIPAL,
                decide("decide.superseded-principal", COUNCIL_AUTHORITY, accept()),
            ),
        ],
    );
    assert!(applied(&canwu, "rebind.council"));
    assert!(
        rejection_reason(&canwu, "decide.superseded-principal").ends_with(FISCAL_ISSUER_NOT_BOUND)
    );
    assert!(proposal(&canwu, "petition.granary.petition").is_pending());

    // The council's new principal grants the petition. The binding is
    // removed, the petition records who petitioned and who decided, and the
    // other petition is superseded.
    submit(
        &mut canwu,
        vec![(
            GRANARY_PRINCIPAL,
            decide("decide.remove", COUNCIL_AUTHORITY, accept()),
        )],
    );
    assert!(applied(&canwu, "decide.remove"));
    assert!(!fiscal_state(&canwu).audit_bindings.contains_key(AUDIT));
    let granted = proposal(&canwu, "petition.granary.petition");
    assert_eq!(granted.proposed_by.issuer, granary_issuer);
    let resolution = granted.resolution.expect("the decision");
    assert_eq!(resolution.outcome, FiscalProposalOutcome::Accepted);
    assert_eq!(resolution.resolved_by.action_id, "decide.remove");
    assert_eq!(
        resolution.resolved_by.issuer,
        council_issuer(GRANARY_PRINCIPAL)
    );
    assert_resolved(
        &canwu,
        "petition.deputy.petition",
        FiscalProposalOutcome::Superseded,
        "decide.remove",
    );

    // The removed auditor records nothing more.
    submit(&mut canwu, vec![(AUDITOR, audit("audit.after-removal"))]);
    assert_unauthorized(&canwu, "audit.after-removal", "binding is unavailable");

    fixture.assert_restore_and_replay(&canwu);
}

#[test]
fn the_appointer_dismisses_a_petition() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(appointment_scenario(installed_auditor));
    submit(
        &mut canwu,
        vec![(DEPUTY, petition("petition.first", POLICY))],
    );
    // The council dismisses the petition, and in the same boundary grants
    // it: admission saw it pending, but settlement checks again and rejects
    // the second decision.
    let grant = || FiscalAppointmentDecision::AcceptPetition {
        petition_id: "petition.first.petition".to_owned(),
    };
    submit(
        &mut canwu,
        vec![
            (
                COUNCIL_PRINCIPAL,
                decide(
                    "decide.dismiss",
                    COUNCIL_AUTHORITY,
                    FiscalAppointmentDecision::Dismiss {
                        proposal_id: "petition.first.petition".to_owned(),
                    },
                ),
            ),
            (
                COUNCIL_PRINCIPAL,
                decide("decide.same-boundary", COUNCIL_AUTHORITY, grant()),
            ),
        ],
    );
    assert!(applied(&canwu, "decide.dismiss"));
    assert!(
        rejection_reason(&canwu, "decide.same-boundary").ends_with(FISCAL_PROPOSAL_NOT_PENDING)
    );
    assert_resolved(
        &canwu,
        "petition.first.petition",
        FiscalProposalOutcome::Dismissed,
        "decide.dismiss",
    );
    assert_eq!(fiscal_state(&canwu).audit_bindings[AUDIT].auditor, AUDITOR);

    // A dismissed petition cannot be granted later, and the auditor keeps
    // recording findings.
    submit(
        &mut canwu,
        vec![
            (
                COUNCIL_PRINCIPAL,
                decide("decide.late", COUNCIL_AUTHORITY, grant()),
            ),
            (AUDITOR, audit("audit.kept")),
        ],
    );
    assert_rejected(
        &canwu,
        "decide.late",
        &ErrorCode::InvalidPayload,
        FISCAL_PROPOSAL_NOT_PENDING,
    );
    assert!(applied(&canwu, "audit.kept"));

    // A petitioner's new petition on the binding replaces his pending one,
    // which is dropped rather than kept.
    submit(
        &mut canwu,
        vec![(DEPUTY, petition("petition.second", POLICY))],
    );
    submit(
        &mut canwu,
        vec![(DEPUTY, petition("petition.third", POLICY))],
    );
    let state = fiscal_state(&canwu);
    assert!(
        !state
            .audit_proposals
            .contains_key("petition.second.petition")
    );
    assert!(state.audit_proposals["petition.third.petition"].is_pending());

    fixture.assert_restore_and_replay(&canwu);
}

/// One party cannot take the room other parties need. A party has at most
/// `MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY` proposals pending under a policy;
/// its next one for a binding it already proposed on replaces the pending
/// one, and another party keeps its own room. Appointing the seated auditor
/// again is not a removal, so the petition against him stays pending. However
/// many proposals the appointer resolves, the policy keeps only its latest
/// resolved ones, so the room for proposals never runs out.
#[test]
#[allow(clippy::too_many_lines)]
fn one_party_cannot_exhaust_the_room_for_proposals() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(appointment_scenario(installed_auditor));
    let basis = || initial_commission(AUDITOR_COMMISSION);
    let vacancy = |index: usize| format!("audit.vacancy.{index}");
    let deputy = FiscalAppointmentParty::Actor { actor: DEPUTY };
    let nominations = |round: usize| -> Vec<(PersonId, Draft)> {
        (0..MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY)
            .map(|index| {
                (
                    DEPUTY,
                    nominate_for(
                        &vacancy(index),
                        &format!("nominate.{round:02}.{index}"),
                        POLICY,
                        AUDITOR,
                        basis(),
                    ),
                )
            })
            .collect()
    };
    let pending_of = |canwu: &Canwu, party: &FiscalAppointmentParty| -> Vec<(String, String)> {
        fiscal_state(canwu)
            .audit_proposals
            .values()
            .filter(|proposal| proposal.is_pending() && proposal.party == *party)
            .map(|proposal| (proposal.audit_binding_id.clone(), proposal.id.clone()))
            .collect()
    };

    submit(&mut canwu, nominations(0));
    submit(
        &mut canwu,
        vec![
            (
                DEPUTY,
                nominate_for(
                    &vacancy(MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY),
                    "nominate.extra",
                    POLICY,
                    AUDITOR,
                    basis(),
                ),
            ),
            (
                DEPUTY,
                nominate_for(
                    &vacancy(0),
                    "nominate.renewed",
                    POLICY,
                    SECOND_AUDITOR,
                    basis(),
                ),
            ),
            (
                GRANARY_PRINCIPAL,
                petition("petition.granary", GRANARY_AUTHORITY),
            ),
        ],
    );
    assert_rejected(
        &canwu,
        "nominate.extra",
        &ErrorCode::ValueOutOfRange,
        "as many nominations and petitions pending",
    );
    assert!(applied(&canwu, "nominate.renewed"));
    assert!(applied(&canwu, "petition.granary"));
    assert!(
        !fiscal_state(&canwu)
            .audit_proposals
            .contains_key("nominate.00.0.nomination")
    );
    assert_eq!(
        pending_of(&canwu, &deputy).len(),
        MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY
    );

    // Appointing the seated auditor again removes nobody.
    submit(
        &mut canwu,
        vec![(
            COUNCIL_PRINCIPAL,
            decide(
                "decide.reappoint",
                COUNCIL_AUTHORITY,
                FiscalAppointmentDecision::Appoint {
                    institution: treasury(),
                    auditor: AUDITOR,
                    authority_basis: basis(),
                },
            ),
        )],
    );
    assert!(applied(&canwu, "decide.reappoint"));
    assert!(proposal(&canwu, "petition.granary.petition").is_pending());

    // The council dismisses every nomination, round after round, until it
    // has resolved more proposals than the policy keeps.
    let rounds =
        MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY / MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY + 1;
    for round in 0..rounds {
        if round > 0 {
            submit(&mut canwu, nominations(round));
        }
        let dismissals = pending_of(&canwu, &deputy)
            .into_iter()
            .enumerate()
            .map(|(index, (binding, proposal_id))| {
                (
                    COUNCIL_PRINCIPAL,
                    decide_on(
                        &binding,
                        &format!("dismiss.{round:02}.{index}"),
                        COUNCIL_AUTHORITY,
                        FiscalAppointmentDecision::Dismiss { proposal_id },
                    ),
                )
            })
            .collect();
        submit(&mut canwu, dismissals);
    }
    let state = fiscal_state(&canwu);
    let resolved = state
        .audit_proposals
        .values()
        .filter(|proposal| !proposal.is_pending())
        .count();
    assert_eq!(resolved, MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY);
    // The first round's are gone and the last round's are kept.
    assert!(
        !state
            .audit_proposals
            .contains_key("nominate.renewed.nomination")
    );
    assert!(
        state
            .audit_proposals
            .contains_key(&format!("nominate.{:02}.0.nomination", rounds - 1))
    );
    submit(
        &mut canwu,
        vec![(
            DEPUTY,
            nominate_for(&vacancy(0), "nominate.after", POLICY, AUDITOR, basis()),
        )],
    );
    assert!(applied(&canwu, "nominate.after"));

    fixture.assert_restore_and_replay(&canwu);
}

/// Audit binding IDs are one namespace, but a decision on one institution's
/// binding supersedes only that institution's proposals. The council fills,
/// on the granary, an ID the treasury has a nomination pending for: the
/// nomination stays pending, cannot be accepted while the ID is the
/// granary's, and the treasury's appointer dismisses it.
#[test]
fn an_appointment_on_one_institution_leaves_another_institutions_proposals_alone() {
    const SHARED: &str = "audit.shared";
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(appointment_scenario(|state| {
        state.audit_appointment_policies.insert(
            "appointment.granary".to_owned(),
            FiscalAppointmentPolicy {
                id: "appointment.granary".to_owned(),
                institution: granary(),
                appointer: council(),
                nominators: BTreeSet::new(),
                petitioners: BTreeSet::new(),
                appoints_without_nomination: true,
                removes_without_petition: true,
            },
        );
    }));
    submit(
        &mut canwu,
        vec![(
            DEPUTY,
            nominate_for(
                SHARED,
                "nominate.treasury",
                POLICY,
                AUDITOR,
                initial_commission(AUDITOR_COMMISSION),
            ),
        )],
    );
    submit(
        &mut canwu,
        vec![(
            COUNCIL_PRINCIPAL,
            decide_on(
                SHARED,
                "decide.granary",
                COUNCIL_AUTHORITY,
                FiscalAppointmentDecision::Appoint {
                    institution: granary(),
                    auditor: SECOND_AUDITOR,
                    authority_basis: initial_commission(DEPUTY_COMMISSION),
                },
            ),
        )],
    );
    assert!(applied(&canwu, "decide.granary"));
    assert_eq!(
        fiscal_state(&canwu).audit_bindings[SHARED].institution,
        granary()
    );
    assert!(proposal(&canwu, "nominate.treasury.nomination").is_pending());

    submit(
        &mut canwu,
        vec![(
            COUNCIL_PRINCIPAL,
            decide_on(
                SHARED,
                "decide.accept",
                COUNCIL_AUTHORITY,
                FiscalAppointmentDecision::AcceptNomination {
                    nomination_id: "nominate.treasury.nomination".to_owned(),
                },
            ),
        )],
    );
    assert_unauthorized(
        &canwu,
        "decide.accept",
        "rebound only from its audited institution",
    );
    submit(
        &mut canwu,
        vec![(
            COUNCIL_PRINCIPAL,
            decide_on(
                SHARED,
                "decide.dismiss",
                COUNCIL_AUTHORITY,
                FiscalAppointmentDecision::Dismiss {
                    proposal_id: "nominate.treasury.nomination".to_owned(),
                },
            ),
        )],
    );
    assert_resolved(
        &canwu,
        "nominate.treasury.nomination",
        FiscalProposalOutcome::Dismissed,
        "decide.dismiss",
    );

    fixture.assert_restore_and_replay(&canwu);
}

#[test]
fn the_audited_institution_cannot_appoint_or_remove_its_own_auditor() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(appointment_scenario(installed_auditor));
    submit(
        &mut canwu,
        vec![
            (
                PRINCIPAL,
                rebind_audit("rebind.remove", TREASURY_AUTHORITY, None),
            ),
            (
                PRINCIPAL,
                rebind_audit("rebind.replace", TREASURY_AUTHORITY, Some(SECOND_AUDITOR)),
            ),
            (
                PRINCIPAL,
                decide(
                    "decide.by-audited",
                    TREASURY_AUTHORITY,
                    FiscalAppointmentDecision::Remove,
                ),
            ),
            // The granary has no appointment policy, so nobody installs an
            // auditor on it at runtime.
            (
                GRANARY_PRINCIPAL,
                draft(
                    "rebind.no-policy",
                    GRANARY_AUTHORITY,
                    FiscalAction::RebindAudit {
                        audit_binding_id: "audit.granary".to_owned(),
                        auditor: Some(AUDITOR),
                        authority_basis: Some(initial_commission(AUDITOR_COMMISSION)),
                    },
                ),
            ),
        ],
    );
    assert_unauthorized(&canwu, "rebind.remove", FISCAL_ISSUER_NOT_APPOINTER);
    assert_unauthorized(&canwu, "rebind.replace", FISCAL_ISSUER_NOT_APPOINTER);
    assert_unauthorized(&canwu, "decide.by-audited", FISCAL_ISSUER_NOT_APPOINTER);
    assert_unauthorized(
        &canwu,
        "rebind.no-policy",
        FISCAL_APPOINTMENT_POLICY_MISSING,
    );
    let state = fiscal_state(&canwu);
    assert_eq!(state.audit_bindings.len(), 1);
    assert_eq!(state.audit_bindings[AUDIT].auditor, AUDITOR);

    fixture.assert_restore_and_replay(&canwu);
}
