//! A fiscal action applies at most once, without the state keeping every
//! outcome for the life of the run.
//!
//! Each action target keeps a ring of its recent outcomes and an outcome
//! floor. Admission refuses, from that hot state alone, a re-send whose
//! outcome is in the ring (`IdempotencyConflict`), a re-send whose outcome
//! has been evicted (its expected revision is at or below the floor), and a
//! revision the state never had. Settlement skips a copy of an action whose
//! outcome the ring holds or the boundary has already settled, even when an
//! eviction within the boundary took the first copy's outcome out of the
//! ring. Action IDs are per target.

mod support;

use canwu_api::{ErrorCode, PersonId};
use canwu_fiscal::{
    FISCAL_RECENT_OUTCOMES, FISCAL_REVISION_EXPIRED, FISCAL_REVISION_UNKNOWN,
    FiscalActionDisposition, FiscalActionTarget, FiscalPaymentForm, FiscalSettledAction,
};
use support::{
    Draft, Fixture, PRINCIPAL, RULE, TREASURY_AUTHORITY, admission_rejection, assessment, draft,
    enqueue_requests, fiscal_state, procedure_revision, scenario, settle, submit, treasury_scope,
};

/// An assessment of the treasury's first scope in a unit of its own, so that
/// many share one book.
fn assess(action_id: &str) -> Draft {
    draft(
        action_id,
        TREASURY_AUTHORITY,
        canwu_fiscal::FiscalAction::OpenAssessment {
            assessment_id: format!("{action_id}.assessment"),
            rule_id: RULE.to_owned(),
            scope_binding_id: treasury_scope(0),
            accounting_cycle_id: "cycle.1".to_owned(),
            quantity: 100,
            unit: format!("unit.{action_id}"),
            payment_form: FiscalPaymentForm::Grain,
            commutation_quote: None,
        },
    )
}

fn ledger() -> FiscalActionTarget {
    FiscalActionTarget::Ledger(treasury_scope(0))
}

/// The settled events of `action_id`.
fn settled(canwu: &canwu_api::Canwu, action_id: &str) -> Vec<FiscalSettledAction> {
    FiscalSettledAction::from_events(canwu.events())
        .into_iter()
        .filter(|settled| settled.action_id == action_id)
        .collect()
}

fn by_principal(drafts: impl IntoIterator<Item = Draft>) -> Vec<(PersonId, Draft)> {
    drafts.into_iter().map(|draft| (PRINCIPAL, draft)).collect()
}

/// The error that refused the most recent command attempt of `action_id`.
fn refused(canwu: &canwu_api::Canwu, action_id: &str) -> canwu_api::CanwuError {
    admission_rejection(canwu, action_id)
        .unwrap_or_else(|| panic!("{action_id} was not refused at admission"))
}

#[test]
#[allow(clippy::too_many_lines)]
fn a_resent_action_is_refused_while_its_outcome_is_hot_and_after_it_is_evicted() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|_| {}));

    // The first action applies; its outcome is in its scope's ring.
    let first_revision = procedure_revision(&canwu);
    let first = assess("first").against(first_revision);
    enqueue_requests(&mut canwu, &[(PRINCIPAL, first.clone())]);
    settle(&mut canwu);
    assert_eq!(settled(&canwu, "first").len(), 1);
    let state = fiscal_state(&canwu);
    let recent = state
        .recent_outcome(&ledger(), "first")
        .expect("the first outcome is hot");
    assert_eq!(recent.disposition, FiscalActionDisposition::Applied);
    assert_eq!(recent.settled_revision, first_revision);

    // Sent again, unchanged, it is refused at admission: its outcome is in
    // the ring.
    enqueue_requests(&mut canwu, &[(PRINCIPAL, first.clone())]);
    settle(&mut canwu);
    assert_eq!(
        refused(&canwu, "first").code,
        ErrorCode::IdempotencyConflict
    );
    assert_eq!(settled(&canwu, "first").len(), 1);

    // A revision the state never had is refused too.
    let ahead = assess("ahead").against(procedure_revision(&canwu) + 1);
    enqueue_requests(&mut canwu, &[(PRINCIPAL, ahead)]);
    settle(&mut canwu);
    let error = refused(&canwu, "ahead");
    assert_eq!(error.code, ErrorCode::ValueOutOfRange);
    assert_eq!(error.message, FISCAL_REVISION_UNKNOWN);
    assert!(settled(&canwu, "ahead").is_empty());

    // As many later outcomes as the ring holds evict the first one. The
    // state no longer knows its ID, but the request names a revision at or
    // below the floor, so it is still refused.
    submit(
        &mut canwu,
        by_principal((0..FISCAL_RECENT_OUTCOMES).map(|index| assess(&format!("later.{index}")))),
    );
    let state = fiscal_state(&canwu);
    let ring = state.outcome_ring(&ledger()).expect("the scope's ring");
    assert!(ring.get("first").is_none());
    assert_eq!(ring.recent.len(), FISCAL_RECENT_OUTCOMES);
    assert!(ring.floor >= first_revision);
    enqueue_requests(&mut canwu, &[(PRINCIPAL, first)]);
    settle(&mut canwu);
    let error = refused(&canwu, "first");
    assert_eq!(error.code, ErrorCode::ValueOutOfRange);
    assert_eq!(error.message, FISCAL_REVISION_EXPIRED);
    assert_eq!(settled(&canwu, "first").len(), 1);

    // Every outcome is still recorded for good in its settled event.
    for index in 0..FISCAL_RECENT_OUTCOMES {
        assert_eq!(
            settled(&canwu, &format!("later.{index}"))[0].disposition,
            FiscalActionDisposition::Applied
        );
    }

    // A fresh request names the current revision, which is above every
    // settled outcome's, so it is admitted and applies.
    submit(&mut canwu, by_principal([assess("fresh")]));
    assert_eq!(
        settled(&canwu, "fresh")[0].disposition,
        FiscalActionDisposition::Applied
    );

    fixture.assert_restore_and_replay(&canwu);
}

/// Two commands with one action ID, both admitted before either settles,
/// apply once: the second copy is skipped without an outcome or an event.
/// So is a copy that settles after an eviction within the same boundary
/// took the first copy's outcome out of the ring.
#[test]
fn copies_settled_in_one_boundary_apply_once() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|_| {}));

    let revision = procedure_revision(&canwu);
    let copy = assess("copied").against(revision);
    enqueue_requests(&mut canwu, &[(PRINCIPAL, copy.clone()), (PRINCIPAL, copy)]);
    settle(&mut canwu);
    assert_eq!(settled(&canwu, "copied").len(), 1);
    assert_eq!(
        fiscal_state(&canwu)
            .assessments()
            .filter(|assessment| assessment.id == "copied.assessment")
            .count(),
        1
    );

    // The first copy settles, then as many outcomes as the ring holds evict
    // it, then the second copy reaches settlement in the same boundary. The
    // boundary remembers what it settled, so nothing applies twice.
    let revision = procedure_revision(&canwu);
    let mut commands = vec![(PRINCIPAL, assess("evicted").against(revision))];
    commands.extend((0..FISCAL_RECENT_OUTCOMES).map(|index| {
        (
            PRINCIPAL,
            assess(&format!("between.{index}")).against(revision),
        )
    }));
    commands.push((PRINCIPAL, assess("evicted").against(revision)));
    enqueue_requests(&mut canwu, &commands);
    settle(&mut canwu);
    let state = fiscal_state(&canwu);
    assert!(state.recent_outcome(&ledger(), "evicted").is_none());
    assert_eq!(settled(&canwu, "evicted").len(), 1);
    assert_eq!(
        state
            .assessments()
            .filter(|assessment| assessment.id == "evicted.assessment")
            .count(),
        1
    );

    fixture.assert_restore_and_replay(&canwu);
}

/// Rings, floors, and the skip are per target, so two targets may use one
/// action ID, and each keeps its own outcome.
#[test]
fn action_ids_are_per_target() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|_| {}));
    submit(
        &mut canwu,
        by_principal([
            assessment("shared", TREASURY_AUTHORITY, &treasury_scope(0), "cycle.1"),
            draft(
                "shared",
                TREASURY_AUTHORITY,
                canwu_fiscal::FiscalAction::OpenAssessment {
                    assessment_id: "shared.other".to_owned(),
                    rule_id: RULE.to_owned(),
                    scope_binding_id: treasury_scope(1),
                    accounting_cycle_id: "cycle.1".to_owned(),
                    quantity: 100,
                    unit: "grain".to_owned(),
                    payment_form: FiscalPaymentForm::Grain,
                    commutation_quote: None,
                },
            ),
        ]),
    );
    let outcomes = settled(&canwu, "shared");
    assert_eq!(outcomes.len(), 2);
    assert!(
        outcomes
            .iter()
            .all(|outcome| outcome.disposition == FiscalActionDisposition::Applied)
    );
    assert_ne!(outcomes[0].target, outcomes[1].target);
    let state = fiscal_state(&canwu);
    for scope in [0, 1] {
        assert!(
            state
                .recent_outcome(&FiscalActionTarget::Ledger(treasury_scope(scope)), "shared")
                .is_some()
        );
    }
    fixture.assert_restore_and_replay(&canwu);
}
