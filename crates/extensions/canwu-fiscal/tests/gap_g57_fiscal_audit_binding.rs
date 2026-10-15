//! Downstream gap G-57: an auditor from outside an institution records audit
//! findings on it under an audit binding, and the finding names him.
//!
//! The audit binding admits `RecordAudit` on the audited institution's scopes
//! and nothing else, only while its own authority basis is current. Here both
//! institutions' appointment policies make each its own appointer, free to act
//! without a nomination or petition, so each installs, replaces, and removes
//! its audit bindings with `RebindAudit`, under the same rules as an authority
//! rebind. Such a binding therefore establishes who entered a finding, not
//! that he is independent of the institution he audits; see
//! `fiscal_audit_appointment.rs` for appointers above the audited
//! institution.

mod support;

use canwu_api::{Canwu, DomainRecordVersionRef, ErrorCode, PersonId, SimTime};
use canwu_fiscal::{
    FISCAL_AUTHORITY_BASIS_NOT_CURRENT, FISCAL_ISSUER_NOT_BOUND, FiscalAction,
    FiscalActionDisposition, FiscalActionIssuer, FiscalAppointmentPolicy, FiscalAssessment,
    FiscalAuditBinding, FiscalAuditSeverity, FiscalIssuerRole, FiscalPaymentForm, FiscalState,
};
use std::collections::BTreeSet;
use support::{
    AUDITOR, AUDITOR_COMMISSION, DEPUTY, Draft, Fixture, GRANARY_AUTHORITY, GRANARY_PRINCIPAL,
    GRANARY_SCOPE, PRINCIPAL, RULE, SECOND_AUDITOR, TREASURY_AUTHORITY, admission_rejection,
    assessment, current_commission, draft, enqueue_renewal, fiscal_state, granary,
    initial_commission, outcome, scenario, submit, treasury, treasury_scope,
};

const TREASURY_AUDIT: &str = "audit.treasury";
const GRANARY_AUDIT: &str = "audit.granary";
const TREASURY_ASSESSMENT: &str = "treasury.assessment";
const GRANARY_ASSESSMENT: &str = "granary.assessment";

/// Makes each institution its own appointer, free to act without a
/// nomination or petition: the 0.15.0 rule for audit bindings.
fn self_appointment(state: &mut FiscalState) {
    for (id, institution) in [
        ("appointment.treasury", treasury()),
        ("appointment.granary", granary()),
    ] {
        state.add_appointment_policy(FiscalAppointmentPolicy {
            id: id.to_owned(),
            institution: institution.clone(),
            appointer: institution,
            nominators: BTreeSet::new(),
            petitioners: BTreeSet::new(),
            appoints_without_nomination: true,
            removes_without_petition: true,
        });
    }
}

fn authored_assessment(state: &mut FiscalState, id: &str, scope: &str) {
    state
        .add_assessment(FiscalAssessment {
            id: id.to_owned(),
            rule_id: RULE.to_owned(),
            scope_binding_id: scope.to_owned(),
            accounting_cycle_id: "cycle.authored".to_owned(),
            quantity: 500,
            unit: "grain".to_owned(),
            payment_form: FiscalPaymentForm::Grain,
            commutation_quote: None,
            created_at: SimTime::EPOCH,
            source_action_id: None,
        })
        .expect("an authored assessment");
}

/// A finding on `target`, in the scope that holds it, issued under
/// `binding`.
fn audit(action_id: &str, binding: &str, target: &str) -> Draft {
    let scope = if target == GRANARY_ASSESSMENT {
        GRANARY_SCOPE.to_owned()
    } else {
        treasury_scope(0)
    };
    draft(
        action_id,
        binding,
        FiscalAction::RecordAudit {
            audit_id: format!("{action_id}.finding"),
            scope_binding_id: scope,
            target_id: target.to_owned(),
            severity: FiscalAuditSeverity::Concern,
            finding: "The assessed quantity was not verified".to_owned(),
            evidence: BTreeSet::new(),
        },
    )
}

/// A rebind of `audit_binding`, issued under the authority binding `issuer`.
fn rebind_audit(
    action_id: &str,
    issuer: &str,
    audit_binding: &str,
    grant: Option<(PersonId, DomainRecordVersionRef)>,
) -> Draft {
    let (auditor, authority_basis) = grant.unzip();
    draft(
        action_id,
        issuer,
        FiscalAction::RebindAudit {
            audit_binding_id: audit_binding.to_owned(),
            auditor,
            authority_basis,
        },
    )
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

/// The issuer the settled finding of `action_id` names.
fn entered_by(canwu: &Canwu, action_id: &str) -> FiscalActionIssuer {
    let id = format!("{action_id}.finding");
    fiscal_state(canwu)
        .audits()
        .find(|finding| finding.id == id)
        .and_then(|finding| finding.entered_by.clone())
        .expect("the finding names its issuer")
}

#[test]
#[allow(clippy::too_many_lines)]
fn gap_g57_fiscal_audit_binding() {
    let fixture = Fixture::new();
    let commission = initial_commission(AUDITOR_COMMISSION);
    let mut canwu = fixture.runtime(scenario(|state| {
        self_appointment(state);
        authored_assessment(state, TREASURY_ASSESSMENT, &treasury_scope(0));
        authored_assessment(state, GRANARY_ASSESSMENT, GRANARY_SCOPE);
        state.add_audit_binding(FiscalAuditBinding {
            id: TREASURY_AUDIT.to_owned(),
            institution: treasury(),
            auditor: AUDITOR,
            authority_basis: initial_commission(AUDITOR_COMMISSION),
            rebound_by: None,
            nomination: None,
        });
    }));

    // The auditor, who holds no role in the treasury, records a finding on it,
    // and the finding names him, his binding, and his commission. A finding
    // the treasury enters itself names its own actor instead.
    submit(
        &mut canwu,
        vec![
            (
                AUDITOR,
                audit("audit.external", TREASURY_AUDIT, TREASURY_ASSESSMENT),
            ),
            (
                PRINCIPAL,
                audit("audit.internal", TREASURY_AUTHORITY, TREASURY_ASSESSMENT),
            ),
        ],
    );
    assert!(applied(&canwu, "audit.external"));
    assert_eq!(
        entered_by(&canwu, "audit.external"),
        FiscalActionIssuer {
            binding_id: TREASURY_AUDIT.to_owned(),
            role: FiscalIssuerRole::Auditor,
            actor: Some(AUDITOR),
            authority_basis: Some(commission.clone()),
        }
    );
    assert_eq!(
        entered_by(&canwu, "audit.internal"),
        FiscalActionIssuer {
            binding_id: TREASURY_AUTHORITY.to_owned(),
            role: FiscalIssuerRole::Principal,
            actor: Some(PRINCIPAL),
            authority_basis: None,
        }
    );

    // The audit binding grants nothing else: no other institution's scopes,
    // no other fiscal action, no place under the treasury's authority binding,
    // and nothing to anyone but its auditor.
    submit(
        &mut canwu,
        vec![
            (
                AUDITOR,
                audit(
                    "audit.other-institution",
                    TREASURY_AUDIT,
                    GRANARY_ASSESSMENT,
                ),
            ),
            (
                AUDITOR,
                assessment(
                    "auditor.assessment",
                    TREASURY_AUDIT,
                    &treasury_scope(1),
                    "cycle.1",
                ),
            ),
            (
                AUDITOR,
                audit("audit.as-treasury", TREASURY_AUTHORITY, TREASURY_ASSESSMENT),
            ),
            (
                DEPUTY,
                audit("audit.not-auditor", TREASURY_AUDIT, TREASURY_ASSESSMENT),
            ),
            (
                AUDITOR,
                rebind_audit(
                    "audit.self-rebind",
                    TREASURY_AUDIT,
                    TREASURY_AUDIT,
                    Some((AUDITOR, commission.clone())),
                ),
            ),
        ],
    );
    assert_unauthorized(&canwu, "audit.other-institution", "does not own");
    assert_unauthorized(&canwu, "auditor.assessment", "admits only audit findings");
    assert_unauthorized(&canwu, "audit.as-treasury", "requires its bound actor");
    assert_unauthorized(&canwu, "audit.not-auditor", "requires its bound auditor");
    assert_unauthorized(&canwu, "audit.self-rebind", "admits only audit findings");

    // The commission is the auditor's authority. It is renewed in the boundary
    // that admits his next finding, so the finding is rejected when it settles,
    // and from then on admission rejects him.
    enqueue_renewal(&mut canwu, AUDITOR_COMMISSION);
    submit(
        &mut canwu,
        vec![(
            AUDITOR,
            audit(
                "audit.renewed-meanwhile",
                TREASURY_AUDIT,
                TREASURY_ASSESSMENT,
            ),
        )],
    );
    assert!(
        rejection_reason(&canwu, "audit.renewed-meanwhile")
            .ends_with(FISCAL_AUTHORITY_BASIS_NOT_CURRENT)
    );
    submit(
        &mut canwu,
        vec![(
            AUDITOR,
            audit("audit.stranded", TREASURY_AUDIT, TREASURY_ASSESSMENT),
        )],
    );
    assert_unauthorized(&canwu, "audit.stranded", FISCAL_AUTHORITY_BASIS_NOT_CURRENT);

    // Only the audited institution rebinds its audit bindings. An audit
    // binding cannot take the ID of an authority binding, and its auditor must
    // exist.
    let renewed = current_commission(&canwu, AUDITOR_COMMISSION);
    submit(
        &mut canwu,
        vec![
            (
                GRANARY_PRINCIPAL,
                rebind_audit(
                    "rebind.by-granary",
                    GRANARY_AUTHORITY,
                    TREASURY_AUDIT,
                    Some((SECOND_AUDITOR, renewed.clone())),
                ),
            ),
            (
                PRINCIPAL,
                rebind_audit(
                    "rebind.taken-id",
                    TREASURY_AUTHORITY,
                    GRANARY_AUTHORITY,
                    Some((SECOND_AUDITOR, renewed.clone())),
                ),
            ),
            (
                PRINCIPAL,
                rebind_audit(
                    "rebind.superseded-basis",
                    TREASURY_AUTHORITY,
                    TREASURY_AUDIT,
                    Some((AUDITOR, commission)),
                ),
            ),
            (
                PRINCIPAL,
                rebind_audit(
                    "rebind.unknown-auditor",
                    TREASURY_AUTHORITY,
                    TREASURY_AUDIT,
                    Some((PersonId::new(9_999), renewed.clone())),
                ),
            ),
        ],
    );
    assert_unauthorized(
        &canwu,
        "rebind.by-granary",
        "only from its audited institution",
    );
    assert_rejected(
        &canwu,
        "rebind.taken-id",
        &ErrorCode::InvalidPayload,
        "ID no authority binding uses",
    );
    assert_unauthorized(
        &canwu,
        "rebind.superseded-basis",
        FISCAL_AUTHORITY_BASIS_NOT_CURRENT,
    );
    assert_rejected(
        &canwu,
        "rebind.unknown-auditor",
        &ErrorCode::EntityNotFound,
        "missing entity",
    );

    // The treasury re-points the binding at the renewed commission, then gives
    // it to a second auditor. In the cut that holds the replacement, the first
    // auditor's finding before it settles and his finding after it does not.
    submit(
        &mut canwu,
        vec![(
            PRINCIPAL,
            rebind_audit(
                "rebind.renewed",
                TREASURY_AUTHORITY,
                TREASURY_AUDIT,
                Some((AUDITOR, renewed.clone())),
            ),
        )],
    );
    assert!(applied(&canwu, "rebind.renewed"));
    submit(
        &mut canwu,
        vec![
            (
                AUDITOR,
                audit("audit.before-rebind", TREASURY_AUDIT, TREASURY_ASSESSMENT),
            ),
            (
                PRINCIPAL,
                rebind_audit(
                    "rebind.second-auditor",
                    TREASURY_AUTHORITY,
                    TREASURY_AUDIT,
                    Some((SECOND_AUDITOR, renewed.clone())),
                ),
            ),
            (
                AUDITOR,
                audit("audit.after-rebind", TREASURY_AUDIT, TREASURY_ASSESSMENT),
            ),
        ],
    );
    assert!(applied(&canwu, "audit.before-rebind"));
    assert!(applied(&canwu, "rebind.second-auditor"));
    assert!(rejection_reason(&canwu, "audit.after-rebind").ends_with(FISCAL_ISSUER_NOT_BOUND));
    let state = fiscal_state(&canwu);
    let binding = &state.audit_binding(TREASURY_AUDIT).expect("audit binding");
    assert_eq!(binding.auditor, SECOND_AUDITOR);
    let rebound_by = binding.rebound_by.as_ref().expect("rebinding provenance");
    assert_eq!(rebound_by.action_id, "rebind.second-auditor");
    assert_eq!(
        rebound_by.issuer,
        FiscalActionIssuer {
            binding_id: TREASURY_AUTHORITY.to_owned(),
            role: FiscalIssuerRole::Principal,
            actor: Some(PRINCIPAL),
            authority_basis: None,
        }
    );
    submit(
        &mut canwu,
        vec![
            (
                AUDITOR,
                audit("audit.replaced", TREASURY_AUDIT, TREASURY_ASSESSMENT),
            ),
            (
                SECOND_AUDITOR,
                audit("audit.second", TREASURY_AUDIT, TREASURY_ASSESSMENT),
            ),
        ],
    );
    assert_unauthorized(&canwu, "audit.replaced", "requires its bound auditor");
    assert_eq!(
        entered_by(&canwu, "audit.second"),
        FiscalActionIssuer {
            binding_id: TREASURY_AUDIT.to_owned(),
            role: FiscalIssuerRole::Auditor,
            actor: Some(SECOND_AUDITOR),
            authority_basis: Some(renewed.clone()),
        }
    );

    // Another institution installs an audit binding of its own at runtime. It
    // covers that institution only.
    submit(
        &mut canwu,
        vec![(
            GRANARY_PRINCIPAL,
            rebind_audit(
                "rebind.granary-audit",
                GRANARY_AUTHORITY,
                GRANARY_AUDIT,
                Some((AUDITOR, renewed)),
            ),
        )],
    );
    assert!(applied(&canwu, "rebind.granary-audit"));
    submit(
        &mut canwu,
        vec![
            (
                AUDITOR,
                audit("audit.granary", GRANARY_AUDIT, GRANARY_ASSESSMENT),
            ),
            (
                AUDITOR,
                audit(
                    "audit.granary-on-treasury",
                    GRANARY_AUDIT,
                    TREASURY_ASSESSMENT,
                ),
            ),
        ],
    );
    assert!(applied(&canwu, "audit.granary"));
    assert_unauthorized(&canwu, "audit.granary-on-treasury", "does not own");

    // Removing an audit binding ends its auditor's authority.
    submit(
        &mut canwu,
        vec![(
            PRINCIPAL,
            rebind_audit("rebind.removed", TREASURY_AUTHORITY, TREASURY_AUDIT, None),
        )],
    );
    assert!(applied(&canwu, "rebind.removed"));
    assert!(fiscal_state(&canwu).audit_binding(TREASURY_AUDIT).is_none());
    submit(
        &mut canwu,
        vec![(
            SECOND_AUDITOR,
            audit("audit.unbound", TREASURY_AUDIT, TREASURY_ASSESSMENT),
        )],
    );
    assert_unauthorized(&canwu, "audit.unbound", "binding is unavailable");

    fixture.assert_restore_and_replay(&canwu);
}
