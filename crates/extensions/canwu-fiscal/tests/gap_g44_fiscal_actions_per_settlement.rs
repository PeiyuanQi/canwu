//! Downstream gap G-44: several fiscal actions settle in one boundary, in
//! admission order, each validated against the state the previous one left.
//!
//! Every action of a boundary is compared with the procedure revision the
//! state had when the boundary began. A rejected action leaves the actions
//! after it alone, a later action may build on an earlier one but not
//! overwrite what it changed, and the number applied per boundary is bounded.
//! No action can fail the boundary for the others: not by its size, and not
//! by reaching the state's byte budget, which the crate's unit tests cover,
//! where a small state can reach it. Outcomes are no longer kept for the life
//! of the state, so no action finds its outcome slots exhausted
//! (`fiscal_action_idempotency` and `fiscal_archive_lifetime` hold that).

mod support;

use canwu_api::{BoundaryReceipt, Canwu, ErrorCode, PersonId};
use canwu_fiscal::{
    FISCAL_BOUNDARY_ACTION_LIMIT, FiscalAction, FiscalActionDisposition, FiscalAdoptionStage,
    FiscalAuditSeverity, FiscalPaymentForm, MAX_FISCAL_ACTION_TEXT_BYTES,
    MAX_FISCAL_ACTIONS_PER_BOUNDARY,
};
use std::collections::BTreeSet;
use support::{
    Draft, Fixture, GRANARY_AUTHORITY, GRANARY_PRINCIPAL, GRANARY_SCOPE, PENDING_SCOPE, PRINCIPAL,
    RULE, TREASURY_AUTHORITY, TREASURY_SCOPES, admission_rejection, assessment, draft,
    enqueue_requests, fiscal_state, outcome, procedure_revision, scenario, settle, settled_at,
    submit, treasury_scope,
};

fn assess(action_id: &str, scope: &str, cycle: &str) -> Draft {
    assessment(action_id, TREASURY_AUTHORITY, scope, cycle)
}

/// An assessment of one cycle in a unit of its own, so that many share a
/// book.
fn assess_unit(action_id: &str, scope: &str, cycle: &str, unit: &str) -> Draft {
    draft(
        action_id,
        TREASURY_AUTHORITY,
        FiscalAction::OpenAssessment {
            assessment_id: format!("{action_id}.assessment"),
            rule_id: RULE.to_owned(),
            scope_binding_id: scope.to_owned(),
            accounting_cycle_id: cycle.to_owned(),
            quantity: 100,
            unit: unit.to_owned(),
            payment_form: FiscalPaymentForm::Grain,
            commutation_quote: None,
        },
    )
}

fn change_adoption(
    action_id: &str,
    adoption_id: &str,
    scope: &str,
    stage: FiscalAdoptionStage,
) -> Draft {
    draft(
        action_id,
        TREASURY_AUTHORITY,
        FiscalAction::ChangeAdoption {
            adoption_id: adoption_id.to_owned(),
            rule_id: RULE.to_owned(),
            scope_binding_id: scope.to_owned(),
            stage,
        },
    )
}

fn by_principal(drafts: impl IntoIterator<Item = Draft>) -> Vec<(PersonId, Draft)> {
    drafts.into_iter().map(|draft| (PRINCIPAL, draft)).collect()
}

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

/// A finding on `target` with a text of `finding_bytes` bytes.
fn audit(action_id: &str, target: &str, finding_bytes: usize) -> Draft {
    draft(
        action_id,
        TREASURY_AUTHORITY,
        FiscalAction::RecordAudit {
            audit_id: format!("{action_id}.finding"),
            scope_binding_id: treasury_scope(0),
            target_id: target.to_owned(),
            severity: FiscalAuditSeverity::Concern,
            finding: "x".repeat(finding_bytes),
            evidence: BTreeSet::new(),
        },
    )
}

fn reports(receipts: &[BoundaryReceipt]) -> usize {
    receipts
        .iter()
        .map(|receipt| receipt.knowledge_record_count)
        .sum()
}

#[test]
#[allow(clippy::too_many_lines)]
fn gap_g44_fiscal_actions_per_settlement() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|_| {}));

    // One issuer admits an assessment for every scope against the revision it
    // read. All of them settle in the one boundary that follows, in admission
    // order, and the observer's report is published once for the boundary, as
    // a summary and one part, not once for each action.
    let entry_revision = procedure_revision(&canwu);
    let receipts = submit(
        &mut canwu,
        by_principal(
            (0..TREASURY_SCOPES)
                .map(|index| assess(&format!("batch.{index}"), &treasury_scope(index), "cycle.1")),
        ),
    );
    assert_eq!(reports(&receipts), 2);
    let state = fiscal_state(&canwu);
    for index in 0..TREASURY_SCOPES {
        let action_id = format!("batch.{index}");
        assert_eq!(
            disposition(&canwu, &action_id),
            FiscalActionDisposition::Applied
        );
        assert_eq!(
            settled_at(&canwu, &action_id),
            settled_at(&canwu, "batch.0")
        );
    }
    assert_eq!(state.assessments().count(), TREASURY_SCOPES);
    assert_eq!(state.aggregates.len(), TREASURY_SCOPES);
    assert_eq!(
        state.procedure_revision,
        entry_revision + u64::try_from(TREASURY_SCOPES).expect("scope count")
    );

    // An action decided against the revision before that boundary is stale.
    enqueue_requests(
        &mut canwu,
        &[(
            PRINCIPAL,
            assess("stale.revision", &treasury_scope(0), "cycle.2").against(entry_revision),
        )],
    );
    settle(&mut canwu);
    assert_eq!(
        disposition(&canwu, "stale.revision"),
        FiscalActionDisposition::Rejected
    );
    assert!(reason(&canwu, "stale.revision").contains("stale"));

    // A rejected action does not poison the valid actions around it: the
    // second one assesses a partition the first has just taken.
    submit(
        &mut canwu,
        by_principal([
            assess("mixed.first", &treasury_scope(0), "cycle.3"),
            assess("mixed.duplicate", &treasury_scope(0), "cycle.3"),
            assess("mixed.last", &treasury_scope(1), "cycle.3"),
        ]),
    );
    assert_eq!(
        disposition(&canwu, "mixed.first"),
        FiscalActionDisposition::Applied
    );
    assert_eq!(
        disposition(&canwu, "mixed.duplicate"),
        FiscalActionDisposition::Rejected
    );
    assert!(!reason(&canwu, "mixed.duplicate").contains("stale"));
    assert_eq!(
        disposition(&canwu, "mixed.last"),
        FiscalActionDisposition::Applied
    );

    // A later action may build on an earlier one of the same boundary: the
    // assessment needs the adoption that the action before it makes
    // operational.
    submit(
        &mut canwu,
        by_principal([
            change_adoption(
                "chain.adopt",
                "adopt.treasury.pending",
                PENDING_SCOPE,
                FiscalAdoptionStage::Implemented,
            ),
            assess("chain.assess", PENDING_SCOPE, "cycle.3"),
        ]),
    );
    assert_eq!(
        disposition(&canwu, "chain.adopt"),
        FiscalActionDisposition::Applied
    );
    assert_eq!(
        disposition(&canwu, "chain.assess"),
        FiscalActionDisposition::Applied
    );

    // It may not overwrite what an earlier one changed: its issuer decided
    // against the adoption as it stood before the boundary.
    submit(
        &mut canwu,
        by_principal([
            change_adoption(
                "overwrite.first",
                "adopt.treasury.0",
                &treasury_scope(0),
                FiscalAdoptionStage::Audited,
            ),
            change_adoption(
                "overwrite.second",
                "adopt.treasury.0",
                &treasury_scope(0),
                FiscalAdoptionStage::Entrenched,
            ),
        ]),
    );
    assert_eq!(
        disposition(&canwu, "overwrite.first"),
        FiscalActionDisposition::Applied
    );
    assert_eq!(
        disposition(&canwu, "overwrite.second"),
        FiscalActionDisposition::Rejected
    );
    assert!(reason(&canwu, "overwrite.second").contains("stale"));
    assert_eq!(
        fiscal_state(&canwu)
            .ledger(&treasury_scope(0))
            .expect("the treasury's first scope")
            .adoptions["adopt.treasury.0"]
            .stage,
        FiscalAdoptionStage::Audited
    );

    // One boundary applies a bounded number of actions. The action past the
    // limit is rejected with a stable reason and consumes its identity; the
    // same assessment under a new identity settles in a later boundary.
    submit(
        &mut canwu,
        by_principal((0..=MAX_FISCAL_ACTIONS_PER_BOUNDARY).map(|index| {
            assess_unit(
                &format!("flood.{index}"),
                &treasury_scope(2),
                "flood.cycle",
                &format!("grain.{index}"),
            )
        })),
    );
    assert!((0..MAX_FISCAL_ACTIONS_PER_BOUNDARY).all(|index| {
        disposition(&canwu, &format!("flood.{index}")) == FiscalActionDisposition::Applied
    }));
    let over_limit = format!("flood.{MAX_FISCAL_ACTIONS_PER_BOUNDARY}");
    assert_eq!(
        disposition(&canwu, &over_limit),
        FiscalActionDisposition::Rejected
    );
    assert!(reason(&canwu, &over_limit).ends_with(FISCAL_BOUNDARY_ACTION_LIMIT));
    submit(
        &mut canwu,
        by_principal([assess_unit(
            "flood.retried",
            &treasury_scope(2),
            "flood.cycle",
            &format!("grain.{MAX_FISCAL_ACTIONS_PER_BOUNDARY}"),
        )]),
    );
    assert_eq!(
        disposition(&canwu, "flood.retried"),
        FiscalActionDisposition::Applied
    );

    // Only an action that reaches the state uses the limit. A full boundary's
    // worth of stale actions from one institution leaves a current action of
    // another institution untouched.
    let revision = procedure_revision(&canwu);
    let mut commands: Vec<_> = (0..MAX_FISCAL_ACTIONS_PER_BOUNDARY)
        .map(|index| {
            (
                GRANARY_PRINCIPAL,
                assessment(
                    &format!("junk.{index}"),
                    GRANARY_AUTHORITY,
                    GRANARY_SCOPE,
                    &format!("junk.cycle.{index}"),
                )
                .against(revision - 1),
            )
        })
        .collect();
    commands.push((
        PRINCIPAL,
        assess("after-junk", &treasury_scope(3), "cycle.3").against(revision),
    ));
    enqueue_requests(&mut canwu, &commands);
    settle(&mut canwu);
    assert!(reason(&canwu, "junk.0").contains("stale"));
    assert_eq!(
        disposition(&canwu, "after-junk"),
        FiscalActionDisposition::Applied
    );

    // Free text is bounded when the command is admitted, so an oversized
    // action never reaches the state or the actions admitted with it.
    submit(
        &mut canwu,
        by_principal([
            audit(
                "oversized.text",
                "batch.0.assessment",
                MAX_FISCAL_ACTION_TEXT_BYTES + 1,
            ),
            audit(
                "largest.text",
                "batch.0.assessment",
                MAX_FISCAL_ACTION_TEXT_BYTES,
            ),
        ]),
    );
    assert_eq!(
        admission_rejection(&canwu, "oversized.text")
            .expect("an oversized finding is rejected at admission")
            .code,
        ErrorCode::InvalidPayload
    );
    assert!(outcome(&canwu, "oversized.text").is_none());
    assert_eq!(
        disposition(&canwu, "largest.text"),
        FiscalActionDisposition::Applied
    );

    fixture.assert_restore_and_replay(&canwu);
}
