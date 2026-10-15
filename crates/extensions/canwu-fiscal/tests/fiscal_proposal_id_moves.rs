//! A proposal ID one office gives up and another takes in the same boundary
//! moves between them in the binding directory.
//!
//! An auditor proposal ID is one namespace across institutions, and the
//! binding directory names the office that holds each. In one boundary, a
//! party's new nomination on a binding replaces its pending one, so that
//! office drops the old ID, and a later nomination of the same boundary uses
//! the freed ID for another institution. The boundary commits, the
//! directory names the new office, and the records stay exactly the
//! partition of the state they assemble to, whichever office's key sorts
//! first.

mod support;

use canwu_api::{EntityRef, OrganizationId, PersonId};
use canwu_fiscal::{
    FiscalAction, FiscalActionDisposition, FiscalAppointmentParty, FiscalAppointmentPolicy,
    FiscalAuthorityBinding, fiscal_office_key,
};
use std::collections::BTreeSet;
use support::{
    AUDITOR, AUDITOR_COMMISSION, DEPUTY, DEPUTY_COMMISSION, Draft, Fixture, SECOND_AUDITOR,
    SUCCESSOR, draft, enqueue, fiscal_state, granary, initial_commission, outcome, scenario,
    settle, submit, treasury,
};

fn council() -> EntityRef {
    EntityRef::Organization(OrganizationId::new(3))
}

fn policy_id(institution: &EntityRef) -> String {
    if *institution == treasury() {
        "appointment.treasury".to_owned()
    } else {
        "appointment.granary".to_owned()
    }
}

fn nominate(
    action_id: &str,
    nomination_id: &str,
    institution: &EntityRef,
    binding: &str,
    candidate: PersonId,
    commission: &str,
) -> Draft {
    draft(
        action_id,
        &policy_id(institution),
        FiscalAction::NominateAuditor {
            nomination_id: nomination_id.to_owned(),
            audit_binding_id: binding.to_owned(),
            institution: institution.clone(),
            candidate,
            authority_basis: initial_commission(commission),
        },
    )
}

/// Runs the move with `from` dropping the ID and `to` taking it.
fn move_proposal_id(from: &EntityRef, to: &EntityRef) {
    let fixture = Fixture::new();
    let mut scenario = scenario(|state| {
        state.add_authority_binding(FiscalAuthorityBinding {
            id: "authority.council".to_owned(),
            institution: council(),
            authorized_actor: Some(SUCCESSOR),
            acting_actor: None,
            authority_basis: None,
            rebound_by: None,
        });
        for institution in [treasury(), granary()] {
            state.add_appointment_policy(FiscalAppointmentPolicy {
                id: policy_id(&institution),
                institution,
                appointer: council(),
                nominators: BTreeSet::from([FiscalAppointmentParty::Actor { actor: DEPUTY }]),
                petitioners: BTreeSet::new(),
                appoints_without_nomination: true,
                removes_without_petition: true,
            });
        }
    });
    scenario.entities.push(council());
    let mut canwu = fixture.runtime(scenario);
    submit(
        &mut canwu,
        vec![(
            DEPUTY,
            nominate(
                "first",
                "moving",
                from,
                "audit.from",
                AUDITOR,
                AUDITOR_COMMISSION,
            ),
        )],
    );
    assert!(fiscal_state(&canwu).audit_proposal("moving").is_some());

    // In one boundary: the deputy's new nomination on the same binding
    // replaces the pending one, and the freed ID is used for the other
    // institution.
    enqueue(
        &mut canwu,
        vec![
            (
                DEPUTY,
                nominate(
                    "replace",
                    "replacement",
                    from,
                    "audit.from",
                    SECOND_AUDITOR,
                    DEPUTY_COMMISSION,
                ),
            ),
            (
                DEPUTY,
                nominate(
                    "take",
                    "moving",
                    to,
                    "audit.to",
                    AUDITOR,
                    AUDITOR_COMMISSION,
                ),
            ),
        ],
    );
    settle(&mut canwu);
    for action_id in ["replace", "take"] {
        assert_eq!(
            outcome(&canwu, action_id)
                .unwrap_or_else(|| panic!("{action_id} has an outcome"))
                .disposition,
            FiscalActionDisposition::Applied,
            "{action_id}"
        );
    }
    let state = fiscal_state(&canwu);
    let moved = state.audit_proposal("moving").expect("the moved proposal");
    assert_eq!(&moved.institution, to);
    let holder = canwu
        .domain_records()
        .filter(|record| record.reference.kind.name == "directory")
        .find_map(|record| {
            record.payload["proposals"]
                .get("moving")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .expect("the directory names the ID");
    assert_eq!(holder, fiscal_office_key(to));
    fixture.assert_restore_and_replay(&canwu);
}

#[test]
fn a_proposal_id_moves_to_an_office_whose_key_sorts_first() {
    let (first, second) = if fiscal_office_key(&treasury()) < fiscal_office_key(&granary()) {
        (treasury(), granary())
    } else {
        (granary(), treasury())
    };
    move_proposal_id(&second, &first);
}

#[test]
fn a_proposal_id_moves_to_an_office_whose_key_sorts_last() {
    let (first, second) = if fiscal_office_key(&treasury()) < fiscal_office_key(&granary()) {
        (treasury(), granary())
    } else {
        (granary(), treasury())
    };
    move_proposal_id(&first, &second);
}
