//! No 0.15.0 lifetime cap is reached: a run makes more assessments, action
//! outcomes, execution requests and receipts, and auditor proposals than the
//! 0.15.0 state could ever hold, and nothing is refused for lifetime.
//!
//! The run is a pipeline of rounds, one accounting cycle each, in which every
//! command batch fills the boundary's action limit. A round's batch assesses
//! the next cycle, authorizes collections in this one, closes the cycle
//! before last, and nominates auditors, the appointment policy's listed party
//! replacing its pending nomination each time and the appointer dismissing
//! the last one of the round before. The boundary that admits the batch
//! records the execution results of the last cycle's collections; the one
//! that settles it settles their receipts and the archive commit of the books
//! closed a round earlier. What the state holds stays bounded by a few
//! cycles, while the archive keeps every book and every cycle's totals.
//!
//! The run past every cap takes a release build:
//! `cargo test --release -p canwu-fiscal --test fiscal_archive_lifetime -- --ignored`.
//! The debug suite runs the same pipeline for a few rounds.

mod support;

use canwu_api::{Canwu, EntityRef, PersonId, ResourceId};
use canwu_fiscal::{
    FiscalAction, FiscalActionDisposition, FiscalAppointmentDecision, FiscalAppointmentParty,
    FiscalAppointmentPolicy, FiscalArchiveIngressReceiptV1, FiscalArchiveKeyV1,
    FiscalArchivedRecordV1, FiscalExecutionEvidence, FiscalExecutionKind, FiscalPaymentForm,
    FiscalSettledAction, MAX_FISCAL_ACTIONS_PER_BOUNDARY, PLUGIN_NAME, archived_fiscal_record,
    audit_fiscal_archive, enqueue_execution_receipt, enqueue_fiscal_archive,
    finalize_fiscal_archive_retention, fiscal_state_reference,
};
use std::collections::BTreeSet;
use support::{
    AUDITOR, AUDITOR_COMMISSION, DEPUTY, Draft, Fixture, MemoryArchiveStore, PRINCIPAL, RULE,
    TREASURY_AUTHORITY, close, draft, enqueue, enqueue_results, execution_claim, fiscal_state,
    initial_commission, receipt_packet, refusals, result_versions, scenario, settle, submit,
    treasury, treasury_scope,
};

/// The 0.15.0 lifetime caps: on assessments, on kept action outcomes, on
/// execution requests and receipts, and on auditor proposals, which every
/// resolved proposal counted against.
const OLD_ASSESSMENT_CAP: usize = 4_096;
const OLD_OUTCOME_CAP: usize = 16_384;
const OLD_REQUEST_CAP: usize = 8_192;
const OLD_RECEIPT_CAP: usize = 8_192;
const OLD_PROPOSAL_CAP: usize = 4_096;
/// The scopes of the treasury the run assesses.
const SCOPES: usize = 3;
/// Assessments per scope and cycle, each of a unit of its own, with two
/// collections of half its quantity against each.
const UNITS: usize = 5;
const POLICY: &str = "policy.treasury";
const AUDIT: &str = "audit.lifetime";

fn lifetime_scenario() -> canwu_api::Scenario {
    scenario(|state| {
        // Reports are covered elsewhere; this run is about what the state
        // holds.
        state.observer_bindings.clear();
        state.audit_appointment_policies.insert(
            POLICY.to_owned(),
            FiscalAppointmentPolicy {
                id: POLICY.to_owned(),
                institution: treasury(),
                appointer: treasury(),
                nominators: BTreeSet::from([FiscalAppointmentParty::Actor { actor: DEPUTY }]),
                petitioners: BTreeSet::new(),
                appoints_without_nomination: true,
                removes_without_petition: true,
            },
        );
    })
}

fn cycle(round: usize) -> String {
    format!("{round:05}")
}

fn unit(index: usize) -> String {
    format!("grain.{index}")
}

/// Every (scope, unit) partition of a cycle.
fn partitions() -> impl Iterator<Item = (usize, usize)> {
    (0..SCOPES).flat_map(|scope| (0..UNITS).map(move |unit| (scope, unit)))
}

/// Every collection of a cycle: two against each partition's assessment.
fn collections() -> impl Iterator<Item = (usize, usize, usize)> {
    partitions().flat_map(|(scope, unit)| (0..2).map(move |half| (scope, unit, half)))
}

fn assessment_id(round: usize, scope: usize, unit: usize) -> String {
    format!("assessment.{round}.{scope}.{unit}")
}

fn request_id(round: usize, scope: usize, unit: usize, half: usize) -> String {
    format!("request.{round}.{scope}.{unit}.{half}")
}

fn assess(round: usize, scope: usize, index: usize) -> (PersonId, Draft) {
    (
        PRINCIPAL,
        draft(
            &format!("assess.{round}.{scope}.{index}"),
            TREASURY_AUTHORITY,
            FiscalAction::OpenAssessment {
                assessment_id: assessment_id(round, scope, index),
                rule_id: RULE.to_owned(),
                scope_binding_id: treasury_scope(scope),
                accounting_cycle_id: cycle(round),
                quantity: 100,
                unit: unit(index),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: None,
            },
        ),
    )
}

fn authorize(round: usize, scope: usize, index: usize, half: usize) -> (PersonId, Draft) {
    (
        PRINCIPAL,
        draft(
            &format!("authorize.{round}.{scope}.{index}.{half}"),
            TREASURY_AUTHORITY,
            FiscalAction::AuthorizeExecution {
                request_id: request_id(round, scope, index, half),
                scope_binding_id: treasury_scope(scope),
                assessment_id: assessment_id(round, scope, index),
                kind: FiscalExecutionKind::Collect,
                quantity: 50,
                unit: unit(index),
                resource: ResourceId::new(1),
                source: EntityRef::Person(PRINCIPAL),
                target: treasury(),
                purpose: "lifetime collection".to_owned(),
            },
        ),
    )
}

fn nominate(round: usize, index: usize) -> (PersonId, Draft) {
    (
        DEPUTY,
        draft(
            &format!("nominate.{round}.{index}"),
            POLICY,
            FiscalAction::NominateAuditor {
                nomination_id: format!("nomination.{round}.{index}"),
                audit_binding_id: AUDIT.to_owned(),
                institution: treasury(),
                candidate: AUDITOR,
                authority_basis: initial_commission(AUDITOR_COMMISSION),
            },
        ),
    )
}

/// How many actions of `round`'s batch are not nominations.
fn other_actions(round: usize) -> usize {
    usize::from(round > 0) + if round >= 2 { SCOPES } else { 0 } + 3 * SCOPES * UNITS
}

/// The appointer's dismissal of the nomination `round` left pending, its
/// last.
fn dismiss(round: usize) -> (PersonId, Draft) {
    let last = MAX_FISCAL_ACTIONS_PER_BOUNDARY - other_actions(round) - 1;
    (
        PRINCIPAL,
        draft(
            &format!("dismiss.{round}"),
            TREASURY_AUTHORITY,
            FiscalAction::DecideAuditAppointment {
                audit_binding_id: AUDIT.to_owned(),
                decision: FiscalAppointmentDecision::Dismiss {
                    proposal_id: format!("nomination.{round}.{last}"),
                },
            },
        ),
    )
}

fn closes(round: usize) -> impl Iterator<Item = (PersonId, Draft)> {
    (0..SCOPES).map(move |scope| {
        (
            PRINCIPAL,
            close(
                &format!("close.{round}.{scope}"),
                &treasury_scope(scope),
                &cycle(round),
            ),
        )
    })
}

/// The execution results of `round`'s collections. They are recorded again
/// under the same IDs every round, as new versions, so the run keeps a
/// bounded number of records; each carries an operation of its own.
fn results(round: usize) -> Vec<(bool, FiscalExecutionEvidence)> {
    collections()
        .map(|(scope, index, half)| {
            let mut claim = execution_claim(
                &format!("result.{scope}.{index}.{half}"),
                &treasury_scope(scope),
                &request_id(round, scope, index, half),
                50,
            );
            claim.unit = unit(index);
            claim.external_operation_id = format!("operation.{round}.{scope}.{index}.{half}");
            (false, claim)
        })
        .collect()
}

/// Queues what the next boundary settles besides actions: the receipts of
/// the results `recorded` names, the acknowledgement of the last archive
/// commit, and a commit of every closed book. Returns the new commit's
/// ingress receipt, if there was a closed book.
fn queue_maintenance(
    canwu: &mut Canwu,
    store: &MemoryArchiveStore,
    recorded: Option<(usize, &[(bool, FiscalExecutionEvidence)])>,
    commit: Option<FiscalArchiveIngressReceiptV1>,
) -> Option<FiscalArchiveIngressReceiptV1> {
    if let Some((round, results)) = recorded {
        let now = canwu.time();
        for ((_, claim), version) in results.iter().zip(result_versions(canwu, results)) {
            enqueue_execution_receipt(
                canwu,
                now,
                &receipt_packet(
                    &format!("receipt.{round}.{}", claim.id),
                    &claim.scope_binding_id,
                    &claim.request_id,
                    [version],
                ),
            )
            .expect("receipt");
        }
    }
    if let Some(commit) = commit {
        finalize_fiscal_archive_retention(canwu, store, &commit).expect("finalized");
    }
    let state = fiscal_state(canwu);
    if state.books().all(|(_, book)| book.is_open()) {
        return None;
    }
    let prepared = state
        .prepare_fiscal_archive(MAX_FISCAL_ACTIONS_PER_BOUNDARY, store, None)
        .expect("the closed books");
    Some(enqueue_fiscal_archive(canwu, &prepared, store, None).expect("queued"))
}

/// Settles a batch of commands in two boundaries: the first admits it and
/// records `results`, if any; the second settles the batch, then the
/// receipts of those results and the archive maintenance queued between the
/// two. Every command names the procedure revision the run has before the
/// first boundary, which only admits and records, so none is stale.
fn batch(
    canwu: &mut Canwu,
    store: &MemoryArchiveStore,
    commands: Vec<(PersonId, Draft)>,
    results: Option<(usize, &[(bool, FiscalExecutionEvidence)])>,
    commit: Option<FiscalArchiveIngressReceiptV1>,
) -> Option<FiscalArchiveIngressReceiptV1> {
    enqueue(canwu, commands);
    if let Some((_, results)) = results {
        enqueue_results(canwu, results);
    }
    canwu.step_canonical().expect("the admitting boundary");
    let commit = queue_maintenance(canwu, store, results, commit);
    settle(canwu);
    commit
}

/// The batch of `round`.
fn round_batch(round: usize) -> Vec<(PersonId, Draft)> {
    let mut batch = Vec::new();
    if round > 0 {
        batch.push(dismiss(round - 1));
    }
    if round >= 2 {
        batch.extend(closes(round - 2));
    }
    batch.extend(collections().map(|(scope, index, half)| authorize(round, scope, index, half)));
    batch.extend(partitions().map(|(scope, index)| assess(round + 1, scope, index)));
    let nominations = MAX_FISCAL_ACTIONS_PER_BOUNDARY - batch.len();
    assert_eq!(
        nominations,
        MAX_FISCAL_ACTIONS_PER_BOUNDARY - other_actions(round)
    );
    batch.extend((0..nominations).map(|index| nominate(round, index)));
    batch
}

/// A run of the pipeline: what it settled, and the fiscal state's size after
/// each round, without its resolved proposals.
struct Run {
    fixture: Fixture,
    canwu: Canwu,
    state_bytes: Vec<usize>,
}

/// Runs `rounds` rounds of the pipeline, then the receipts of the last, then
/// closes and archives every book left, and checks what every run holds:
/// nothing rejected, refused, or dropped; every book archived and every
/// cycle's totals in the archive, each cycle but the last, which was only
/// assessed, collecting all it assessed; the resolved proposals bounded; the
/// archive audits.
fn run(rounds: usize) -> Run {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(lifetime_scenario());
    let store = MemoryArchiveStore::default();
    // The state's bytes, without the resolved proposals, which grow by one
    // dismissal a round to their bound of 64.
    let state_bytes = |canwu: &Canwu| {
        let mut payload = canwu
            .typed_domain_record(&fiscal_state_reference())
            .expect("the fiscal state")
            .payload
            .clone();
        payload
            .as_object_mut()
            .expect("an object")
            .remove("audit_proposals");
        serde_json::to_vec(&payload).expect("payload").len()
    };

    submit(
        &mut canwu,
        partitions()
            .map(|(scope, index)| assess(0, scope, index))
            .collect(),
    );
    let mut commit = None;
    let mut sizes = Vec::with_capacity(rounds);
    for round in 0..rounds {
        let recorded = (round > 0).then(|| results(round - 1));
        let recorded = recorded.as_deref().map(|results| (round - 1, results));
        commit = batch(&mut canwu, &store, round_batch(round), recorded, commit);
        sizes.push(state_bytes(&canwu));
    }
    let last = std::iter::once(dismiss(rounds - 1)).chain(closes(rounds - 2));
    let recorded = results(rounds - 1);
    let recorded = Some((rounds - 1, recorded.as_slice()));
    commit = batch(&mut canwu, &store, last.collect(), recorded, commit);
    commit = queue_maintenance(&mut canwu, &store, None, commit);
    let last = closes(rounds - 1).chain(closes(rounds));
    commit = batch(&mut canwu, &store, last.collect(), None, commit);
    commit = queue_maintenance(&mut canwu, &store, None, commit);
    settle(&mut canwu);
    finalize_fiscal_archive_retention(&mut canwu, &store, &commit.expect("the last commit"))
        .expect("finalized");
    settle(&mut canwu);

    let settled = FiscalSettledAction::from_events(canwu.events());
    let rejected: Vec<_> = settled
        .iter()
        .filter(|outcome| outcome.disposition == FiscalActionDisposition::Rejected)
        .collect();
    assert!(rejected.is_empty(), "{:?}", rejected.first());
    assert!(
        refusals(&canwu).is_empty(),
        "{:?}",
        refusals(&canwu).first()
    );
    assert!(!canwu.events().iter().any(|event| {
        event.kind.plugin_identity() == Some((PLUGIN_NAME, "canwu.fiscal.action_dropped.v1"))
    }));

    let state = fiscal_state(&canwu);
    assert!(state.books().next().is_none());
    let cycles = rounds + 1;
    assert_eq!(
        state.archive.archived_book_count,
        u64::try_from(cycles * SCOPES).expect("books")
    );
    assert_eq!(
        state.archive.archived_totals_count,
        u64::try_from(cycles).expect("cycles")
    );
    assert!(state.archive.pending_handles.is_empty());
    assert!(
        state.audit_proposals.len() <= 64,
        "resolved proposals stay bounded"
    );
    for index in 0..cycles {
        let Some(FiscalArchivedRecordV1::InstitutionTotals(totals)) = archived_fiscal_record(
            &state,
            &store,
            &FiscalArchiveKeyV1::InstitutionTotals {
                institution: treasury(),
                accounting_cycle_id: cycle(index),
            },
        )
        .expect("a lookup") else {
            panic!("the totals of cycle {index}");
        };
        assert_eq!(totals.books, u32::try_from(SCOPES).expect("scopes"));
        assert_eq!(totals.totals.len(), UNITS);
        for total in &totals.totals {
            let assessed = 100 * u64::try_from(SCOPES).expect("scopes");
            assert_eq!(total.assessed, assessed);
            assert_eq!(total.collected, if index < rounds { assessed } else { 0 });
        }
    }
    audit_fiscal_archive(&store, &state).expect("the archive audits");
    Run {
        fixture,
        canwu,
        state_bytes: sizes,
    }
}

/// The pipeline, in the debug suite: once it is full, from its fourth round,
/// the state holds the same few cycles every round, differing only in the
/// digits of its counters and identifiers.
#[test]
fn the_lifetime_pipeline_holds_a_state_bounded_by_a_few_cycles() {
    let run = run(8);
    let full = &run.state_bytes[3..];
    let (least, most) = (
        full.iter().min().expect("rounds"),
        full.iter().max().expect("rounds"),
    );
    assert!(most - least < 512, "{full:?}");
    assert!(run.state_bytes[0] < *least);
    let restored = Canwu::from_snapshot_json_with_plugins(
        &run.canwu.snapshot_json().expect("snapshot"),
        &run.fixture.plugins(),
    )
    .expect("restored");
    assert_eq!(restored.checkpoint_hash(), run.canwu.checkpoint_hash());
}

/// Past every 0.15.0 lifetime cap. Release only: in Phase 1 every boundary
/// rewrites the whole fiscal state record, which a debug build takes about
/// 0.3 s a boundary to do at this run's 170 KB, about six minutes for its
/// 560 boundaries; a release build runs it in under a minute. (Restoring it
/// would load the whole retained history of an unsealed run, which the
/// pipeline's debug test restores at a smaller size.)
#[test]
#[ignore = "release-only lifetime run past every 0.15.0 cap"]
fn a_long_run_passes_every_old_lifetime_cap_without_a_refusal() {
    // Enough rounds for more requests and receipts than 0.15.0 could hold,
    // which is also more assessments, outcomes, and proposals.
    let rounds = OLD_RECEIPT_CAP / (2 * SCOPES * UNITS) + 1;
    let run = run(rounds);
    let settled = FiscalSettledAction::from_events(run.canwu.events());
    let count = |prefix: &str| {
        settled
            .iter()
            .filter(|outcome| outcome.action_id.starts_with(prefix))
            .count()
    };
    assert!(count("assess.") > OLD_ASSESSMENT_CAP);
    assert!(count("authorize.") > OLD_REQUEST_CAP);
    assert!(count("nominate.") > OLD_PROPOSAL_CAP);
    assert!(settled.len() > OLD_OUTCOME_CAP);
    // Every collection was settled by one receipt, and every cycle's totals
    // record it.
    assert!(rounds * 2 * SCOPES * UNITS > OLD_RECEIPT_CAP);
    let largest = run.state_bytes.iter().max().expect("rounds");
    assert!(*largest < 192 * 1024, "{largest} bytes");
}
