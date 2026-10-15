//! An action is stale only when its own target record changed since its
//! issuer looked.
//!
//! Each scope ledger, office, jurisdiction, and observer has a revision of its
//! own, advanced by every outcome and receipt on it. An action names the
//! revision of its target and applies only when that is the target's revision
//! at the start of the boundary that settles it. Activity on another scope, a
//! historical-context change, and archive maintenance leave it current; an
//! outcome on its own scope, and a transition that changes the scope's
//! adoptions, make it stale. Within one boundary, a later action may not
//! overwrite an adoption an earlier one changed, a transition's included.

mod support;

use canwu_api::{Canwu, PersonId};
use canwu_fiscal::{
    FiscalAction, FiscalActionDisposition, FiscalActionTarget, FiscalAdoptionStage,
    FiscalHistoricalContextPacket, FiscalHistoricalMode, fiscal_historical_context_ingress,
};
use std::collections::BTreeMap;
use support::{
    Draft, Fixture, GRANARY_AUTHORITY, GRANARY_PRINCIPAL, GRANARY_SCOPE, PRINCIPAL, REFORM,
    REFORMED_RULE, RULE, TREASURY_AUTHORITY, assessment, draft, enqueue_requests, fiscal_state,
    ledger_revision, outcome, scenario, settle, submit, target_revision, treasury, treasury_scope,
};

fn disposition(canwu: &Canwu, action_id: &str) -> FiscalActionDisposition {
    outcome(canwu, action_id)
        .unwrap_or_else(|| panic!("{action_id} has no outcome"))
        .disposition
}

fn reason(canwu: &Canwu, action_id: &str) -> String {
    outcome(canwu, action_id)
        .unwrap_or_else(|| panic!("{action_id} has no outcome"))
        .reason
}

fn assess(action_id: &str, scope: usize, cycle: &str) -> Draft {
    assessment(action_id, TREASURY_AUTHORITY, &treasury_scope(scope), cycle)
}

fn context(canwu: &mut Canwu, year: i32) {
    let now = canwu.time();
    canwu
        .enqueue_plugin_ingress(
            fiscal_historical_context_ingress(
                now,
                &FiscalHistoricalContextPacket {
                    year,
                    mode: FiscalHistoricalMode::Counterfactual,
                },
            )
            .expect("context packet"),
        )
        .expect("context ingress");
}

fn reform(action_id: &str) -> (PersonId, Draft) {
    (
        PRINCIPAL,
        draft(
            action_id,
            TREASURY_AUTHORITY,
            FiscalAction::ApplyTransition {
                transition_id: REFORM.to_owned(),
                target_scope_bindings: BTreeMap::from([(
                    REFORMED_RULE.to_owned(),
                    treasury_scope(0),
                )]),
            },
        ),
    )
}

#[test]
#[allow(clippy::too_many_lines)]
fn an_action_is_stale_only_for_its_own_target() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|_| {}));

    // Two issuers decide against what they see now: one on the treasury's
    // first scope, one on its second.
    let first = assess("first", 0, "2026").against(ledger_revision(&canwu, &treasury_scope(0)));
    let second = assess("second", 1, "2026").against(ledger_revision(&canwu, &treasury_scope(1)));
    let behind = assess("behind", 0, "2027").against(ledger_revision(&canwu, &treasury_scope(0)));

    // Activity on another scope, on another institution, and a change of
    // the historical context settle first.
    submit(
        &mut canwu,
        vec![
            (PRINCIPAL, assess("other.scope", 2, "2026")),
            (
                GRANARY_PRINCIPAL,
                assessment("other.office", GRANARY_AUTHORITY, GRANARY_SCOPE, "2026"),
            ),
        ],
    );
    context(&mut canwu, 151);
    settle(&mut canwu);

    // Both actions are still current: neither of their scopes changed.
    enqueue_requests(&mut canwu, &[(PRINCIPAL, first), (PRINCIPAL, second)]);
    settle(&mut canwu);
    assert_eq!(
        disposition(&canwu, "first"),
        FiscalActionDisposition::Applied
    );
    assert_eq!(
        disposition(&canwu, "second"),
        FiscalActionDisposition::Applied
    );

    // One decided before the first scope's last outcome is stale, and the
    // revision its rejection records moves the scope on once more.
    let before = ledger_revision(&canwu, &treasury_scope(0));
    enqueue_requests(&mut canwu, &[(PRINCIPAL, behind)]);
    settle(&mut canwu);
    assert_eq!(
        disposition(&canwu, "behind"),
        FiscalActionDisposition::Rejected
    );
    assert!(reason(&canwu, "behind").contains("stale"));
    assert_eq!(ledger_revision(&canwu, &treasury_scope(0)), before + 1);

    // Each outcome advanced its own target's revision and no other: the
    // treasury's office and the region did not move.
    let office = FiscalActionTarget::Office(treasury());
    let region = FiscalActionTarget::Jurisdiction("region.a".to_owned());
    assert_eq!(target_revision(&canwu, &office), 1);
    assert_eq!(target_revision(&canwu, &region), 1);

    fixture.assert_restore_and_replay(&canwu);
}

#[test]
#[allow(clippy::too_many_lines)]
fn a_transition_moves_every_ledger_whose_adoptions_it_changes() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|_| {}));
    // A context change settled at the run's start time is refused, since a
    // context dated at the start must hold the catalog's initial year; the
    // run moves time on first.
    support::enqueue_tick(&mut canwu);
    settle(&mut canwu);
    context(&mut canwu, 165);
    settle(&mut canwu);
    assert!(
        fiscal_state(&canwu)
            .jurisdiction("region.a")
            .expect("the region")
            .transition_candidates
            .contains_key(&format!("{REFORM}::region.a"))
    );

    // Decided before the transition: an assessment of the granary's scope,
    // whose adoption of the old rule the transition suspends, and an
    // adoption change on the treasury's third scope.
    let granary_revision = ledger_revision(&canwu, GRANARY_SCOPE);
    let late_granary = assessment("late.granary", GRANARY_AUTHORITY, GRANARY_SCOPE, "2026")
        .against(granary_revision);
    let region = FiscalActionTarget::Jurisdiction("region.a".to_owned());
    let region_revision = target_revision(&canwu, &region);
    let third_revision = ledger_revision(&canwu, &treasury_scope(2));

    // In one boundary: the transition, then an adoption change of an
    // adoption it suspended. The later action would overwrite what the
    // transition changed, so it is stale.
    enqueue_requests(
        &mut canwu,
        &[
            (PRINCIPAL, reform("reform").1.against(region_revision)),
            (
                PRINCIPAL,
                draft(
                    "overwrite.suspended",
                    TREASURY_AUTHORITY,
                    FiscalAction::ChangeAdoption {
                        adoption_id: "adopt.treasury.2".to_owned(),
                        rule_id: RULE.to_owned(),
                        scope_binding_id: treasury_scope(2),
                        stage: FiscalAdoptionStage::Entrenched,
                    },
                )
                .against(third_revision),
            ),
        ],
    );
    settle(&mut canwu);
    assert_eq!(
        disposition(&canwu, "reform"),
        FiscalActionDisposition::Applied
    );
    assert_eq!(
        disposition(&canwu, "overwrite.suspended"),
        FiscalActionDisposition::Rejected
    );
    assert!(reason(&canwu, "overwrite.suspended").contains("overwrites"));
    let state = fiscal_state(&canwu);
    assert_eq!(
        state
            .ledger(&treasury_scope(2))
            .expect("the scope")
            .adoptions["adopt.treasury.2"]
            .stage,
        FiscalAdoptionStage::Suspended
    );

    // The transition advanced the revision of every ledger whose adoptions
    // it changed, its target region's by its outcome, so an action decided
    // before it on the granary's scope is now stale.
    assert_eq!(ledger_revision(&canwu, GRANARY_SCOPE), granary_revision + 1);
    assert_eq!(target_revision(&canwu, &region), region_revision + 1);
    enqueue_requests(&mut canwu, &[(GRANARY_PRINCIPAL, late_granary)]);
    settle(&mut canwu);
    assert_eq!(
        disposition(&canwu, "late.granary"),
        FiscalActionDisposition::Rejected
    );
    assert!(reason(&canwu, "late.granary").contains("stale"));

    // A transition already applied is no longer a candidate, and the same
    // action ID on the same target is refused while its outcome is hot.
    submit(&mut canwu, vec![reform("reform.again")]);
    assert_eq!(
        disposition(&canwu, "reform.again"),
        FiscalActionDisposition::Rejected
    );
    fixture.assert_restore_and_replay(&canwu);
}
