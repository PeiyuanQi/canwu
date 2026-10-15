//! One command carries a batch of fiscal actions under one binding, and each
//! entry is an ordinary action: its own target, staleness check, outcome, and
//! event.
//!
//! A batch of the most entries a command may carry settles in one boundary.
//! The issuer's role is checked again for every entry at settlement, so an
//! entry after a rebind that took the role away is rejected. Admission
//! refuses the whole command for an entry it can already refuse, naming the
//! entry and its reason, and for more entries than a batch may carry; a
//! batch may not name one action ID twice for one target. One institution's
//! books of one cycle and mechanism use at most four partition keys (unit
//! and payment form); an assessment that would add a fifth is rejected as its
//! own outcome.

mod support;

use canwu_api::{Canwu, ErrorCode, SimTime, SimulationGranularity};
use canwu_fiscal::{
    FISCAL_ISSUER_NOT_BOUND, FISCAL_PARTITION_KEYS_EXHAUSTED, FiscalAction, FiscalActionBatch,
    FiscalActionDisposition, FiscalAdoptionStage, FiscalAdoptionState, FiscalMechanism,
    FiscalScopeBinding, FiscalSettledAction, MAX_FISCAL_BATCH_ENTRIES, MAX_FISCAL_PARTITION_KEYS,
};
use support::{
    Fixture, OBSERVER, PRINCIPAL, RULE, SUCCESSOR, TREASURY_AUTHORITY, admission_rejection,
    assessment, draft, enqueue_requests, fiscal_state, ledger_revision, outcome, scenario, settle,
    treasury, treasury_scope,
};

fn wide_scope(index: usize) -> String {
    format!("scope.wide.{index:05}")
}

/// The base scenario with `count` more operational scopes of the treasury
/// and no observer.
fn wide_scenario(count: usize) -> canwu_api::Scenario {
    scenario(|state| {
        state.remove_observer(OBSERVER);
        for index in 0..count {
            let id = wide_scope(index);
            state.add_scope(FiscalScopeBinding {
                id: id.clone(),
                institution: treasury(),
                jurisdiction_id: "region.a".to_owned(),
                subject_scope: "registered_land".to_owned(),
                mechanism: FiscalMechanism::LandTax,
                authoritative_granularity: SimulationGranularity::Aggregate,
            });
            state
                .add_adoption(FiscalAdoptionState {
                    id: format!("adopt.wide.{index:05}"),
                    rule_id: RULE.to_owned(),
                    scope_binding_id: id,
                    stage: FiscalAdoptionStage::Implemented,
                    generation: 1,
                    changed_at: SimTime::EPOCH,
                    source_action_id: None,
                })
                .expect("an adoption of a new scope");
        }
    })
}

fn disposition(canwu: &Canwu, action_id: &str) -> FiscalActionDisposition {
    outcome(canwu, action_id)
        .unwrap_or_else(|| panic!("{action_id} has no outcome"))
        .disposition
}

#[test]
fn a_full_batch_settles_in_one_boundary_with_an_outcome_for_every_entry() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(wide_scenario(MAX_FISCAL_BATCH_ENTRIES));
    let entries = (0..MAX_FISCAL_BATCH_ENTRIES)
        .map(|index| {
            let scope = wide_scope(index);
            assessment(
                &format!("wide.{index:04}"),
                TREASURY_AUTHORITY,
                &scope,
                "2026",
            )
            .entry(ledger_revision(&canwu, &scope))
        })
        .collect::<Vec<_>>();
    let batch = FiscalActionBatch {
        authority_binding_id: TREASURY_AUTHORITY.to_owned(),
        entries,
    };

    // One entry more than a batch may carry is refused as a whole.
    let mut oversized = batch.clone();
    oversized
        .entries
        .push(assessment("wide.extra", TREASURY_AUTHORITY, &treasury_scope(0), "2026").entry(1));
    enqueue_requests(&mut canwu, &[(PRINCIPAL, oversized)]);
    settle(&mut canwu);
    let refused = admission_rejection(&canwu, "wide.extra").expect("the oversized batch");
    assert_eq!(refused.code, ErrorCode::InvalidPayload);

    // The full batch is admitted by one boundary and settled by the next,
    // with one outcome and one event for each entry.
    let events = FiscalSettledAction::from_events(canwu.events()).len();
    enqueue_requests(&mut canwu, &[(PRINCIPAL, batch)]);
    canwu.step_canonical().expect("the admitting boundary");
    let settling = canwu
        .step_canonical()
        .expect("the settling boundary")
        .expect("a boundary settles the batch");
    let settled = FiscalSettledAction::from_events(canwu.events());
    assert_eq!(settled.len() - events, MAX_FISCAL_BATCH_ENTRIES);
    assert!(
        settled[events..]
            .iter()
            .all(|outcome| outcome.disposition == FiscalActionDisposition::Applied)
    );
    assert!(settling.record_change_count >= MAX_FISCAL_BATCH_ENTRIES);
    let state = fiscal_state(&canwu);
    assert_eq!(state.assessments().count(), MAX_FISCAL_BATCH_ENTRIES);
    assert!(
        (0..MAX_FISCAL_BATCH_ENTRIES).all(|index| ledger_revision(&canwu, &wide_scope(index)) == 2)
    );
}

#[test]
fn every_entry_is_checked_against_the_role_its_issuer_holds_when_it_settles() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|_| {}));
    let office = canwu_fiscal::FiscalActionTarget::Office(treasury());
    let batch = FiscalActionBatch {
        authority_binding_id: TREASURY_AUTHORITY.to_owned(),
        entries: vec![
            assessment("before", TREASURY_AUTHORITY, &treasury_scope(0), "2026")
                .entry(ledger_revision(&canwu, &treasury_scope(0))),
            draft(
                "handover",
                TREASURY_AUTHORITY,
                FiscalAction::RebindAuthority {
                    authority_binding_id: TREASURY_AUTHORITY.to_owned(),
                    authorized_actor: Some(SUCCESSOR),
                    acting_actor: None,
                    authority_basis: None,
                },
            )
            .entry(support::target_revision(&canwu, &office)),
            assessment("after", TREASURY_AUTHORITY, &treasury_scope(1), "2026")
                .entry(ledger_revision(&canwu, &treasury_scope(1))),
        ],
    };
    enqueue_requests(&mut canwu, &[(PRINCIPAL, batch)]);
    settle(&mut canwu);
    assert_eq!(
        disposition(&canwu, "before"),
        FiscalActionDisposition::Applied
    );
    assert_eq!(
        disposition(&canwu, "handover"),
        FiscalActionDisposition::Applied
    );
    assert_eq!(
        disposition(&canwu, "after"),
        FiscalActionDisposition::Rejected
    );
    assert!(
        outcome(&canwu, "after")
            .expect("the outcome")
            .reason
            .ends_with(FISCAL_ISSUER_NOT_BOUND)
    );
    fixture.assert_restore_and_replay(&canwu);
}

#[test]
fn an_inadmissible_entry_refuses_its_batch_naming_the_entry() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|_| {}));
    let current = |canwu: &Canwu, scope: usize| ledger_revision(canwu, &treasury_scope(scope));

    // An entry on a scope that does not exist.
    let unknown = FiscalActionBatch {
        authority_binding_id: TREASURY_AUTHORITY.to_owned(),
        entries: vec![
            assessment("fine", TREASURY_AUTHORITY, &treasury_scope(0), "2026")
                .entry(current(&canwu, 0)),
            assessment("nowhere", TREASURY_AUTHORITY, "scope.nowhere", "2026").entry(1),
        ],
    };
    // One action ID twice for one target.
    let twice = FiscalActionBatch {
        authority_binding_id: TREASURY_AUTHORITY.to_owned(),
        entries: vec![
            assessment("twice", TREASURY_AUTHORITY, &treasury_scope(1), "2026")
                .entry(current(&canwu, 1)),
            assessment("twice", TREASURY_AUTHORITY, &treasury_scope(1), "2027")
                .entry(current(&canwu, 1)),
        ],
    };
    enqueue_requests(&mut canwu, &[(PRINCIPAL, unknown)]);
    settle(&mut canwu);
    let error = admission_rejection(&canwu, "nowhere").expect("refused");
    assert_eq!(error.code, ErrorCode::EntityNotFound);
    assert!(error.message.starts_with("entry 1: "), "{}", error.message);
    enqueue_requests(&mut canwu, &[(PRINCIPAL, twice)]);
    settle(&mut canwu);
    let error = admission_rejection(&canwu, "twice").expect("refused");
    assert_eq!(error.code, ErrorCode::InvalidPayload);
    assert!(error.message.starts_with("entry 1: "), "{}", error.message);
    // Nothing of either batch settled.
    assert!(outcome(&canwu, "fine").is_none());
    assert!(outcome(&canwu, "twice").is_none());
    assert_eq!(current(&canwu, 0), 1);

    // The same action ID on two targets is two actions.
    let two_targets = FiscalActionBatch {
        authority_binding_id: TREASURY_AUTHORITY.to_owned(),
        entries: vec![
            assessment("shared", TREASURY_AUTHORITY, &treasury_scope(0), "2026")
                .entry(current(&canwu, 0)),
            assessment("shared", TREASURY_AUTHORITY, &treasury_scope(1), "2026")
                .entry(current(&canwu, 1)),
        ],
    };
    enqueue_requests(&mut canwu, &[(PRINCIPAL, two_targets)]);
    settle(&mut canwu);
    let shared: Vec<_> = FiscalSettledAction::from_events(canwu.events())
        .into_iter()
        .filter(|outcome| outcome.action_id == "shared")
        .collect();
    assert_eq!(shared.len(), 2);
    assert!(
        shared
            .iter()
            .all(|outcome| outcome.disposition == FiscalActionDisposition::Applied)
    );
    fixture.assert_restore_and_replay(&canwu);
}

/// An assessment in `unit` of `scope` and `cycle`.
fn assessment_in(action_id: &str, scope: usize, cycle: &str, unit: &str) -> support::Draft {
    let mut draft = assessment(action_id, TREASURY_AUTHORITY, &treasury_scope(scope), cycle);
    if let FiscalAction::OpenAssessment { unit: of, .. } = &mut draft.action {
        unit.clone_into(of);
    }
    draft
}

#[test]
fn an_assessment_past_the_partition_keys_of_its_cycle_is_rejected() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|_| {}));
    // The treasury's books of 2026 use every partition key they may, all in
    // its first scope.
    let keys: Vec<_> = (0..MAX_FISCAL_PARTITION_KEYS)
        .map(|unit| {
            (
                PRINCIPAL,
                assessment_in(&format!("key.{unit}"), 0, "2026", &format!("grain.{unit}")),
            )
        })
        .collect();
    support::enqueue_batches(&mut canwu, keys);
    settle(&mut canwu);
    // A fifth key in another scope of the same institution and cycle is
    // rejected; a key already in use there, and the fifth in another cycle,
    // apply.
    let fifth = format!("grain.{MAX_FISCAL_PARTITION_KEYS}");
    support::submit(
        &mut canwu,
        vec![
            (PRINCIPAL, assessment_in("fifth", 1, "2026", &fifth)),
            (PRINCIPAL, assessment_in("again", 2, "2026", "grain.0")),
            (PRINCIPAL, assessment_in("next.cycle", 3, "2027", &fifth)),
        ],
    );
    let fifth_outcome = outcome(&canwu, "fifth").expect("an outcome");
    assert_eq!(fifth_outcome.disposition, FiscalActionDisposition::Rejected);
    assert!(
        fifth_outcome
            .reason
            .ends_with(FISCAL_PARTITION_KEYS_EXHAUSTED),
        "{}",
        fifth_outcome.reason
    );
    assert_eq!(
        disposition(&canwu, "again"),
        FiscalActionDisposition::Applied
    );
    assert_eq!(
        disposition(&canwu, "next.cycle"),
        FiscalActionDisposition::Applied
    );
    fixture.assert_restore_and_replay(&canwu);
}

/// One boundary applies at most `MAX_FISCAL_ACTIONS_PER_BOUNDARY` actions;
/// the next current, authorized action of the boundary is rejected as its own
/// outcome with a stable reason, and the boundary commits. Release only: the
/// run writes 65,537 ledgers in one boundary, which a debug build takes
/// minutes to do:
/// `cargo test --release -p canwu-fiscal --test fiscal_action_batches -- --ignored`.
#[test]
#[ignore = "release-only boundary of 65,537 actions"]
fn the_action_past_the_boundary_limit_is_rejected_as_an_outcome() {
    use canwu_fiscal::{FISCAL_BOUNDARY_ACTION_LIMIT, MAX_FISCAL_ACTIONS_PER_BOUNDARY};
    let actions = MAX_FISCAL_ACTIONS_PER_BOUNDARY + 1;
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(wide_scenario(actions));
    let batches: Vec<_> = (0..actions)
        .collect::<Vec<_>>()
        .chunks(MAX_FISCAL_BATCH_ENTRIES)
        .map(|chunk| {
            (
                PRINCIPAL,
                FiscalActionBatch {
                    authority_binding_id: TREASURY_AUTHORITY.to_owned(),
                    entries: chunk
                        .iter()
                        .map(|index| {
                            let scope = wide_scope(*index);
                            assessment(
                                &format!("wide.{index:05}"),
                                TREASURY_AUTHORITY,
                                &scope,
                                "2026",
                            )
                            .entry(ledger_revision(&canwu, &scope))
                        })
                        .collect(),
                },
            )
        })
        .collect();
    enqueue_requests(&mut canwu, &batches);
    settle(&mut canwu);
    let settled = FiscalSettledAction::from_events(canwu.events());
    assert_eq!(settled.len(), actions);
    let rejected: Vec<_> = settled
        .iter()
        .filter(|outcome| outcome.disposition == FiscalActionDisposition::Rejected)
        .collect();
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].action_id, format!("wide.{:05}", actions - 1));
    assert!(rejected[0].reason.ends_with(FISCAL_BOUNDARY_ACTION_LIMIT));
}
