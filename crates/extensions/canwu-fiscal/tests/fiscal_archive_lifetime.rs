//! No 0.15.0 lifetime cap is reached: a run makes more assessments, action
//! outcomes, execution requests and receipts, and auditor proposals than the
//! 0.15.0 state could ever hold, and nothing is refused for lifetime.
//!
//! The run is a pipeline of rounds, one accounting cycle each, of
//! [`ROUND_ACTIONS`] actions in two batch commands. A round's actions assess
//! the next cycle, authorize collections in this one, close the cycle before
//! last, and nominate auditors, the appointment policy's listed party
//! replacing its pending nomination each time and the appointer dismissing
//! the last one of the round before. The boundary that admits the batches
//! records the execution results of the last cycle's collections; the one
//! that settles them settles their receipts, in one receipt batch, and the
//! archive commit of the books closed a round earlier. What the records hold
//! stays bounded by a few cycles, round after round, while the archive keeps
//! every book and every cycle's totals.

mod support;

use canwu_api::{Canwu, EntityRef, PersonId, ResourceId};
use canwu_fiscal::{
    FiscalAction, FiscalActionDisposition, FiscalAppointmentDecision, FiscalAppointmentParty,
    FiscalAppointmentPolicy, FiscalArchiveIngressReceiptV1, FiscalArchiveKeyV1,
    FiscalArchivedRecordV1, FiscalExecutionEvidence, FiscalExecutionKind, FiscalPaymentForm,
    FiscalSettledAction, FiscalState, MAX_FISCAL_ARCHIVE_COMMIT_RECORDS, archived_fiscal_record,
    audit_fiscal_archive, enqueue_execution_receipts, enqueue_fiscal_archive,
    finalize_fiscal_archive_retention, prepare_fiscal_archive,
};
use std::collections::BTreeSet;
use support::{
    AUDITOR, AUDITOR_COMMISSION, DEPUTY, Draft, Fixture, MemoryArchiveStore, OBSERVER, PRINCIPAL,
    RULE, TREASURY_AUTHORITY, close, draft, enqueue_batches, enqueue_results, execution_claim,
    fiscal_state, initial_commission, receipt_packet, refusals, result_versions, scenario, settle,
    submit, treasury, treasury_scope,
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
/// collections of half its quantity against each: as many as one
/// institution's books of a cycle may use partition keys.
const UNITS: usize = 4;
/// Actions one round settles: its assessments, collections, closes, and
/// dismissal, and nominations for the rest.
const ROUND_ACTIONS: usize = 64;
const POLICY: &str = "policy.treasury";
const AUDIT: &str = "audit.lifetime";

fn lifetime_scenario() -> canwu_api::Scenario {
    scenario(|state| {
        // Reports are covered elsewhere; this run is about what the records
        // hold.
        state.remove_observer(OBSERVER);
        state.add_appointment_policy(FiscalAppointmentPolicy {
            id: POLICY.to_owned(),
            institution: treasury(),
            appointer: treasury(),
            nominators: BTreeSet::from([FiscalAppointmentParty::Actor { actor: DEPUTY }]),
            petitioners: BTreeSet::new(),
            appoints_without_nomination: true,
            removes_without_petition: true,
        });
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
    let last = ROUND_ACTIONS - other_actions(round) - 1;
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
        let receipts: Vec<_> = results
            .iter()
            .zip(result_versions(canwu, results))
            .map(|((_, claim), version)| {
                receipt_packet(
                    &format!("receipt.{round}.{}", claim.id),
                    &claim.scope_binding_id,
                    &claim.request_id,
                    [version],
                )
            })
            .collect();
        enqueue_execution_receipts(canwu, now, &receipts).expect("receipts");
    }
    if let Some(commit) = commit {
        finalize_fiscal_archive_retention(canwu, store, &commit).expect("finalized");
    }
    if fiscal_state(canwu).books().all(|(_, book)| book.is_open()) {
        return None;
    }
    let prepared = prepare_fiscal_archive(canwu, MAX_FISCAL_ARCHIVE_COMMIT_RECORDS, store)
        .expect("the closed books");
    Some(enqueue_fiscal_archive(canwu, &prepared, store).expect("queued"))
}

/// Settles a round's commands in two boundaries: the first admits them, as
/// one batch for each issuer and binding, and records `results`, if any; the
/// second settles the batches, then the receipts of those results and the
/// archive maintenance queued between the two. Every entry names the
/// revision its target has before the first boundary, which only admits and
/// records, so none is stale.
fn batch(
    canwu: &mut Canwu,
    store: &MemoryArchiveStore,
    commands: Vec<(PersonId, Draft)>,
    results: Option<(usize, &[(bool, FiscalExecutionEvidence)])>,
    commit: Option<FiscalArchiveIngressReceiptV1>,
) -> Option<FiscalArchiveIngressReceiptV1> {
    enqueue_batches(canwu, commands);
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
    let nominations = ROUND_ACTIONS - batch.len();
    assert_eq!(nominations, ROUND_ACTIONS - other_actions(round));
    batch.extend((0..nominations).map(|index| nominate(round, index)));
    batch
}

/// What the fiscal records hold that a long run could grow: the most hot
/// books one ledger holds, the hot cycles of the treasury's office, the most
/// recent outcomes any ring holds, and the archive head's retention handles
/// and maintenance receipts. The auditor proposals the office keeps, and
/// their directory entries, grow by one resolved proposal a round to the
/// policy's bound, and are checked against it apart.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Shape {
    hot_books: usize,
    office_cycles: usize,
    ring: usize,
    handles: usize,
    receipts: usize,
    proposals: usize,
    directory: usize,
}

impl Shape {
    /// The shape without the proposals and their directory entries.
    fn fixed(&self) -> Shape {
        Shape {
            proposals: 0,
            directory: 0,
            ..self.clone()
        }
    }

    /// Whether the proposals are within the policy's bounds: its resolved
    /// proposals and the one party's pending ones, each with one directory
    /// entry beside the three binding and policy IDs.
    fn proposals_bounded(&self) -> bool {
        self.proposals
            <= canwu_fiscal::MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY
                + canwu_fiscal::MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY
            && self.directory == 3 + self.proposals
    }
}

fn shape(canwu: &Canwu, state: &FiscalState) -> Shape {
    let rings = state
        .ledgers
        .values()
        .map(|ledger| &ledger.outcomes)
        .chain(state.offices.values().map(|office| &office.outcomes))
        .chain(
            state
                .jurisdictions
                .values()
                .map(|jurisdiction| &jurisdiction.outcomes),
        );
    Shape {
        hot_books: state
            .ledgers
            .values()
            .map(|ledger| ledger.books.len())
            .max()
            .unwrap_or_default(),
        office_cycles: state
            .office(&treasury())
            .map_or(0, |office| office.cycles.len()),
        ring: rings
            .map(|ring| ring.recent.len())
            .max()
            .unwrap_or_default(),
        handles: state.archive.pending_handles.len(),
        receipts: state.archive.maintenance_receipts.len(),
        proposals: state.audit_proposals().count(),
        directory: canwu
            .domain_records()
            .filter(|record| record.reference.kind.name == "directory")
            .map(|record| {
                ["bindings", "proposals"]
                    .iter()
                    .filter_map(|key| record.payload.get(*key).and_then(|map| map.as_object()))
                    .map(serde_json::Map::len)
                    .sum::<usize>()
            })
            .sum(),
    }
}

/// The bytes of the largest fiscal record of a run, the catalog aside.
fn largest_record(canwu: &Canwu) -> usize {
    canwu
        .domain_records()
        .filter(|record| {
            record.reference.kind.namespace == "canwu.fiscal"
                && record.reference.kind.name != "catalog"
        })
        .map(|record| serde_json::to_vec(&record.payload).expect("payload").len())
        .max()
        .unwrap_or_default()
}

/// A run of the pipeline: what it settled, and the shape of its records and
/// its largest record after each round.
struct Run {
    fixture: Fixture,
    canwu: Canwu,
    shapes: Vec<Shape>,
    largest: Vec<usize>,
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

    submit(
        &mut canwu,
        partitions()
            .map(|(scope, index)| assess(0, scope, index))
            .collect(),
    );
    let mut commit = None;
    let mut shapes = Vec::with_capacity(rounds);
    let mut largest = Vec::with_capacity(rounds);
    for round in 0..rounds {
        let recorded = (round > 0).then(|| results(round - 1));
        let recorded = recorded.as_deref().map(|results| (round - 1, results));
        commit = batch(&mut canwu, &store, round_batch(round), recorded, commit);
        shapes.push(shape(&canwu, &fiscal_state(&canwu)));
        largest.push(largest_record(&canwu));
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
        state.audit_proposals().count() <= 64,
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
        shapes,
        largest,
    }
}

/// The pipeline: once it is full, from its fourth round, every round leaves
/// the records the same shape (hot books per ledger, the office's hot
/// cycles, rings, retention handles and receipts, proposals, and directory
/// entries), and its largest record stays bounded.
#[test]
fn the_lifetime_pipeline_holds_records_of_a_fixed_shape() {
    let run = run(8);
    let full = &run.shapes[3..];
    assert!(
        full.iter()
            .all(|shape| shape.fixed() == full[0].fixed() && shape.proposals_bounded()),
        "{full:#?}"
    );
    assert!(full[0].hot_books <= 4, "{:?}", full[0]);
    assert!(full[0].ring <= canwu_fiscal::FISCAL_RECENT_OUTCOMES);
    assert!(full[0].handles <= 1);
    let largest = run.largest.iter().max().expect("rounds");
    assert!(*largest < 64 * 1024, "{largest} bytes");
    let restored = Canwu::from_snapshot_json_with_plugins(
        &run.canwu.snapshot_json().expect("snapshot"),
        &run.fixture.plugins(),
    )
    .expect("restored");
    assert_eq!(restored.checkpoint_hash(), run.canwu.checkpoint_hash());
}

/// Past every 0.15.0 lifetime cap, in the debug suite: every record keeps
/// the shape it has from the pipeline's fourth round on.
#[test]
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
    let full = &run.shapes[3..];
    assert!(
        full.iter()
            .all(|shape| shape.fixed() == full[0].fixed() && shape.proposals_bounded()),
        "{:?}",
        full[0]
    );
    let largest = run.largest.iter().max().expect("rounds");
    assert!(*largest < 64 * 1024, "{largest} bytes");
}
