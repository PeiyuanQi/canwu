//! Accounting cycles close per scope and leave the fiscal state through the
//! fiscal archive, which keeps every book and every institution's per-cycle
//! totals for good.
//!
//! A closed book takes only audit findings. An archive commit takes closed
//! books out of their ledgers, raises each scope's watermark to the cycle it
//! archived, and, in the commit that archives an institution's last hot book
//! of a cycle, archives the institution's totals of that cycle once and
//! raises the office watermark. No new book is made at or below either
//! watermark. Commits are prepared, stored, verified, and queued by the host;
//! they settle before every other fiscal packet, a stale or duplicate commit
//! ends as a no-op, and an acknowledgement retires its handle once. On a run
//! that has sealed, a book whose citations its last seal has not covered is
//! refused at preparation; a run restored from a continuation checkpoint has
//! sealed nothing until it seals. A sealed run archives exactly as its
//! unsealed fork, a restore from its sealed segments, and its exact replay
//! do.

mod support;

use canwu_api::{
    BoundaryRequest, Canwu, CompactedCanwu, EntityRef, GovernmentId, KnowledgeHolderRef, PersonId,
    SimTime,
};
use canwu_fiscal::{
    FISCAL_ARCHIVE_BLOB_NAMESPACE, FISCAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
    FISCAL_ARCHIVE_MAINTENANCE_EVENT, FISCAL_CYCLE_ARCHIVED, FISCAL_CYCLE_CLOSED,
    FISCAL_INGRESS_INVALID_REJECTION, FISCAL_RECEIPT_CYCLE_CLOSED, FiscalAction,
    FiscalActionDisposition, FiscalActionIssuer, FiscalArchiveError, FiscalArchiveKeyV1,
    FiscalArchiveRetentionPhaseV1, FiscalArchivedRecordV1, FiscalAuditSeverity, FiscalBookStatus,
    FiscalIssuerRole, FiscalObserverBinding, FiscalPaymentForm, FiscalRebinding,
    FiscalSettledAction, FiscalState, PLUGIN_NAME, archived_fiscal_record, audit_fiscal_archive,
    enqueue_execution_receipts, enqueue_fiscal_archive, finalize_fiscal_archive_retention,
    prepare_fiscal_archive,
};
use std::collections::BTreeSet;
use support::{
    AUDITOR, AUDITOR_COMMISSION, DEPUTY, Draft, Fixture, GRANARY_PRINCIPAL, MemoryArchiveStore,
    MemoryEvidenceArchive, PRINCIPAL, RULE, SUCCESSOR, TREASURY_AUTHORITY, archive, assessment,
    close, collection, current_commission, draft, enqueue_archive, execution_claim, fiscal_report,
    fiscal_state, ledger_revision, observer_binding, published_report_records, receipt_packet,
    record_results, refusals, renewal_at, scenario, seal, sealed_fiscal_state, settle, submit,
    submit_sealed, treasury, treasury_scope,
};

fn by_principal(drafts: impl IntoIterator<Item = Draft>) -> Vec<(PersonId, Draft)> {
    drafts.into_iter().map(|draft| (PRINCIPAL, draft)).collect()
}

fn outcome(canwu: &Canwu, action_id: &str) -> FiscalSettledAction {
    FiscalSettledAction::from_events(canwu.events())
        .into_iter()
        .rev()
        .find(|settled| settled.action_id == action_id)
        .unwrap_or_else(|| panic!("{action_id} has no settled event"))
}

fn applied(canwu: &Canwu, action_id: &str) -> bool {
    outcome(canwu, action_id).disposition == FiscalActionDisposition::Applied
}

fn rejected_with(canwu: &Canwu, action_id: &str, reason: &str) -> bool {
    let outcome = outcome(canwu, action_id);
    outcome.disposition == FiscalActionDisposition::Rejected && outcome.reason.ends_with(reason)
}

fn book_key(scope: &str, cycle: &str) -> FiscalArchiveKeyV1 {
    FiscalArchiveKeyV1::Book {
        scope_binding_id: scope.to_owned(),
        accounting_cycle_id: cycle.to_owned(),
    }
}

fn totals_key(cycle: &str) -> FiscalArchiveKeyV1 {
    FiscalArchiveKeyV1::InstitutionTotals {
        institution: treasury(),
        accounting_cycle_id: cycle.to_owned(),
    }
}

/// A closed book of one assessment of 100 grain, authored with the state.
fn closed_book(state: &mut FiscalState, scope: usize, cycle: &str, assessed: u64) {
    state
        .add_assessment(canwu_fiscal::FiscalAssessment {
            id: format!("authored.{scope}.{cycle}"),
            rule_id: RULE.to_owned(),
            scope_binding_id: treasury_scope(scope),
            accounting_cycle_id: cycle.to_owned(),
            quantity: assessed,
            unit: "grain".to_owned(),
            payment_form: FiscalPaymentForm::Grain,
            commutation_quote: None,
            created_at: SimTime::EPOCH,
            source_action_id: None,
        })
        .expect("authored assessment");
    state
        .ledgers
        .get_mut(&treasury_scope(scope))
        .and_then(|ledger| ledger.books.get_mut(cycle))
        .expect("the authored book")
        .status = FiscalBookStatus::Closed {
        closed_by: FiscalRebinding {
            action_id: "authored.close".to_owned(),
            issuer: FiscalActionIssuer {
                binding_id: TREASURY_AUTHORITY.to_owned(),
                role: FiscalIssuerRole::Principal,
                actor: Some(PRINCIPAL),
                authority_basis: None,
            },
            rebound_at: SimTime::EPOCH,
        },
    };
}

fn lookup(
    canwu: &Canwu,
    store: &MemoryArchiveStore,
    key: &FiscalArchiveKeyV1,
) -> Option<FiscalArchivedRecordV1> {
    archived_fiscal_record(&fiscal_state(canwu), store, key).expect("an archive lookup")
}

/// A book is assessed, collected on, and closed; a closed book takes only
/// audit findings; the archive then takes it out of the state with its
/// totals, and no new book is made at or below the cycle it archived.
#[test]
#[allow(clippy::too_many_lines)]
fn a_closed_book_takes_only_findings_and_leaves_the_state_with_its_totals() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|_| {}));
    let scope = treasury_scope(0);
    submit(
        &mut canwu,
        by_principal([assessment("assess", TREASURY_AUTHORITY, &scope, "2026")]),
    );
    submit(
        &mut canwu,
        by_principal([collection(
            "authorize",
            &scope,
            "assess.assessment",
            "request.a",
            60,
        )]),
    );
    let evidence = record_results(
        &mut canwu,
        &[(false, execution_claim("result.a", &scope, "request.a", 40))],
    );
    let now = canwu.time();
    enqueue_execution_receipts(
        &mut canwu,
        now,
        &[receipt_packet("receipt.a", &scope, "request.a", evidence)],
    )
    .expect("receipt");
    settle(&mut canwu);
    submit(&mut canwu, by_principal([close("close", &scope, "2026")]));
    assert!(applied(&canwu, "close"));

    // A closed book takes no assessment, remission, or authorization, and no
    // receipt; it still takes an audit finding.
    let late_evidence = record_results(
        &mut canwu,
        &[(
            false,
            execution_claim("result.late", &scope, "request.a", 10),
        )],
    );
    let now = canwu.time();
    enqueue_execution_receipts(
        &mut canwu,
        now,
        &[receipt_packet(
            "receipt.late",
            &scope,
            "request.a",
            late_evidence,
        )],
    )
    .expect("late receipt");
    submit(
        &mut canwu,
        by_principal([
            draft(
                "late.assess",
                TREASURY_AUTHORITY,
                FiscalAction::OpenAssessment {
                    assessment_id: "late.assessment".to_owned(),
                    rule_id: RULE.to_owned(),
                    scope_binding_id: scope.clone(),
                    accounting_cycle_id: "2026".to_owned(),
                    quantity: 5,
                    unit: "grain.late".to_owned(),
                    payment_form: FiscalPaymentForm::Grain,
                    commutation_quote: None,
                },
            ),
            draft(
                "late.remit",
                TREASURY_AUTHORITY,
                FiscalAction::GrantRemission {
                    remission_id: "late.remission".to_owned(),
                    scope_binding_id: scope.clone(),
                    assessment_id: "assess.assessment".to_owned(),
                    quantity: 5,
                    reason: "flood".to_owned(),
                },
            ),
            collection(
                "late.authorize",
                &scope,
                "assess.assessment",
                "request.late",
                5,
            ),
            close("close.again", &scope, "2026"),
            draft(
                "late.audit",
                TREASURY_AUTHORITY,
                FiscalAction::RecordAudit {
                    audit_id: "late.finding".to_owned(),
                    scope_binding_id: scope.clone(),
                    target_id: "receipt.a".to_owned(),
                    severity: FiscalAuditSeverity::Notice,
                    finding: "checked before archive".to_owned(),
                    evidence: BTreeSet::new(),
                },
            ),
        ]),
    );
    for action_id in ["late.assess", "late.remit", "late.authorize", "close.again"] {
        assert!(
            rejected_with(&canwu, action_id, FISCAL_CYCLE_CLOSED),
            "{action_id}"
        );
    }
    assert!(applied(&canwu, "late.audit"));
    assert!(
        refusals(&canwu)
            .iter()
            .any(|summary| summary.contains("receipt.late")
                && summary.ends_with(FISCAL_RECEIPT_CYCLE_CLOSED))
    );

    // The archive takes the closed book, with its finding, out of the state.
    // Its scope's watermark and its office's rise to its cycle, and the
    // institution's totals of the cycle are archived with it.
    let store = MemoryArchiveStore::default();
    let revision = ledger_revision(&canwu, &scope);
    archive(&mut canwu, &store, 64);
    let state = fiscal_state(&canwu);
    assert_eq!(
        ledger_revision(&canwu, &scope),
        revision,
        "maintenance makes nothing stale"
    );
    let ledger = state.ledger(&scope).expect("the scope");
    assert!(ledger.books.is_empty());
    assert_eq!(ledger.watermark.as_deref(), Some("2026"));
    let office = state.office(&treasury()).expect("the treasury's office");
    assert!(office.cycles.is_empty());
    assert_eq!(office.watermark.as_deref(), Some("2026"));
    assert!(state.archive.pending_handles.is_empty());
    assert_eq!(state.archive.archived_book_count, 1);
    assert_eq!(state.archive.archived_totals_count, 1);

    let Some(FiscalArchivedRecordV1::Book(book)) =
        lookup(&canwu, &store, &book_key(&scope, "2026"))
    else {
        panic!("the archived book resolves");
    };
    assert_eq!(book.book.audits.len(), 1);
    assert_eq!(book.book.execution_receipts.len(), 1);
    assert_eq!(book.totals.len(), 1);
    assert_eq!(
        (
            book.totals[0].assessed,
            book.totals[0].collected,
            book.totals[0].outstanding
        ),
        (100, 40, 60)
    );
    let Some(FiscalArchivedRecordV1::InstitutionTotals(totals)) =
        lookup(&canwu, &store, &totals_key("2026"))
    else {
        panic!("the institution's totals resolve");
    };
    assert_eq!(totals.books, 1);
    assert_eq!(totals.totals, book.totals);
    assert_eq!(lookup(&canwu, &store, &book_key(&scope, "2027")), None);
    audit_fiscal_archive(&store, &state).expect("the archive audits");

    // No new book at or below either watermark: not in the archived scope,
    // and not in another scope of the institution. Above them a book opens.
    submit(
        &mut canwu,
        by_principal([
            assessment("again.2026", TREASURY_AUTHORITY, &scope, "2026"),
            assessment("again.2025", TREASURY_AUTHORITY, &scope, "2025"),
            assessment("other.2026", TREASURY_AUTHORITY, &treasury_scope(1), "2026"),
            assessment("other.2027", TREASURY_AUTHORITY, &treasury_scope(1), "2027"),
        ]),
    );
    for action_id in ["again.2026", "again.2025", "other.2026"] {
        assert!(
            rejected_with(&canwu, action_id, FISCAL_CYCLE_ARCHIVED),
            "{action_id}"
        );
    }
    assert!(applied(&canwu, "other.2027"));
    // The reports follow: the observer is told only the hot book.
    assert_eq!(fiscal_report(&canwu, PRINCIPAL).projection.facts.len(), 1);

    fixture.assert_restore_and_replay(&canwu);
}

/// An institution's totals of a cycle are archived once, by the commit that
/// takes its last hot book of the cycle, whatever order its scopes' books
/// and its cycles are archived in. A hot book below the office watermark is
/// unaffected by it.
#[test]
fn institution_totals_are_archived_once_by_the_commit_that_empties_the_cycle() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|state| {
        closed_book(state, 0, "2026", 100);
        closed_book(state, 1, "2026", 200);
        closed_book(state, 2, "2027", 300);
        closed_book(state, 3, "2025", 400);
    }));
    let store = MemoryArchiveStore::default();
    let office_cycle = |canwu: &Canwu, cycle: &str| {
        fiscal_state(canwu)
            .office(&treasury())
            .and_then(|office| office.cycles.get(cycle).cloned())
    };

    // One book at a time, in (scope, cycle) order. The first of the two
    // books of 2026 leaves its totals in the office.
    archive(&mut canwu, &store, 1);
    let entry = office_cycle(&canwu, "2026").expect("2026 is still hot");
    assert_eq!((entry.hot_books, entry.archived_books), (1, 1));
    assert_eq!(entry.archived_totals[0].assessed, 100);
    assert_eq!(lookup(&canwu, &store, &totals_key("2026")), None);

    // The second empties the cycle and archives the totals of both.
    archive(&mut canwu, &store, 1);
    assert!(office_cycle(&canwu, "2026").is_none());
    let Some(FiscalArchivedRecordV1::InstitutionTotals(totals)) =
        lookup(&canwu, &store, &totals_key("2026"))
    else {
        panic!("the totals of 2026");
    };
    assert_eq!((totals.books, totals.totals[0].assessed), (2, 300));

    // 2027 archives next and raises the office watermark past the hot book
    // of 2025, which still takes a finding and archives later, with totals
    // of its own, without lowering the watermark.
    archive(&mut canwu, &store, 1);
    let state = fiscal_state(&canwu);
    assert_eq!(
        state
            .ledger(&treasury_scope(3))
            .expect("scope 3")
            .books
            .len(),
        1
    );
    assert_eq!(
        state
            .office(&treasury())
            .and_then(|office| office.watermark.clone())
            .as_deref(),
        Some("2027")
    );
    submit(
        &mut canwu,
        by_principal([draft(
            "audit.2025",
            TREASURY_AUTHORITY,
            FiscalAction::RecordAudit {
                audit_id: "audit.2025.finding".to_owned(),
                scope_binding_id: treasury_scope(3),
                target_id: "authored.3.2025".to_owned(),
                severity: FiscalAuditSeverity::Notice,
                finding: "late".to_owned(),
                evidence: BTreeSet::new(),
            },
        )]),
    );
    assert!(applied(&canwu, "audit.2025"));
    archive(&mut canwu, &store, 1);
    let state = fiscal_state(&canwu);
    assert!(state.books().next().is_none());
    assert_eq!(state.archive.archived_book_count, 4);
    assert_eq!(state.archive.archived_totals_count, 3);
    assert_eq!(
        state
            .office(&treasury())
            .and_then(|office| office.watermark.clone())
            .as_deref(),
        Some("2027")
    );
    for (cycle, assessed) in [("2025", 400), ("2026", 300), ("2027", 300)] {
        let Some(FiscalArchivedRecordV1::InstitutionTotals(totals)) =
            lookup(&canwu, &store, &totals_key(cycle))
        else {
            panic!("the totals of {cycle}");
        };
        assert_eq!(totals.totals[0].assessed, assessed, "{cycle}");
    }
    // Each commit after the first archived the receipt of the one before.
    assert_eq!(state.archive.archived_receipt_count, 3);
    audit_fiscal_archive(&store, &state).expect("the archive audits");
    fixture.assert_restore_and_replay(&canwu);
}

/// One commit holds as many books as it may, cut into blobs of 64 records,
/// each resolving through the cumulative index; a later commit extends the
/// index and lists the pages it no longer reaches.
#[test]
fn a_commit_spans_several_blobs_and_the_index_is_cumulative() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|state| {
        for scope in 0..4 {
            for cycle in 0..20 {
                closed_book(state, scope, &format!("{:04}", 2000 + cycle), 10);
            }
        }
    }));
    let store = MemoryArchiveStore::default();
    let prepared = prepare_fiscal_archive(&canwu, 70, &store).expect("a large batch");
    // In (scope, cycle) order: every book of the first three scopes and the
    // first ten of the last, and the totals of the ten cycles they empty.
    assert_eq!(prepared.selected_books.len(), 70);
    assert_eq!(prepared.pages.len(), 2);
    assert_eq!(prepared.selected_count(), 80);
    let first = enqueue_fiscal_archive(&mut canwu, &prepared, &store).expect("queued");
    settle(&mut canwu);
    finalize_fiscal_archive_retention(&mut canwu, &store, &first).expect("finalized");
    settle(&mut canwu);
    assert_eq!(store.count(FISCAL_ARCHIVE_BLOB_NAMESPACE), 2);

    let second = prepare_fiscal_archive(&canwu, 64, &store).expect("the rest");
    assert_eq!(second.selected_receipts.len(), 1);
    assert_eq!(second.selected_books.len(), 10);
    assert!(!second.superseded_lookup_pages.is_empty());
    let receipt = enqueue_fiscal_archive(&mut canwu, &second, &store).expect("queued");
    settle(&mut canwu);
    finalize_fiscal_archive_retention(&mut canwu, &store, &receipt).expect("finalized");
    settle(&mut canwu);

    let state = fiscal_state(&canwu);
    assert_eq!(state.archive.archived_book_count, 80);
    assert_eq!(state.archive.archived_totals_count, 20);
    assert_eq!(store.count(FISCAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE), 2);
    for scope in 0..4 {
        for cycle in [2000, 2019] {
            assert!(matches!(
                lookup(
                    &canwu,
                    &store,
                    &book_key(&treasury_scope(scope), &cycle.to_string())
                ),
                Some(FiscalArchivedRecordV1::Book(_))
            ));
        }
    }
    let Some(FiscalArchivedRecordV1::MaintenanceReceipt(head_receipt)) = lookup(
        &canwu,
        &store,
        &FiscalArchiveKeyV1::MaintenanceReceipt {
            receipt_id: second.selected_receipts[0].clone(),
        },
    ) else {
        panic!("the first commit's receipt resolves");
    };
    assert_eq!(head_receipt.archived_books, 70);
    audit_fiscal_archive(&store, &state).expect("the archive audits");
}

/// The same batch queued twice settles once applied and once stale; a batch
/// prepared from the same state but queued after another commit settled is
/// stale; acknowledging a handle twice is harmless. Commits settle before the
/// actions of their boundary, so an action admitted ahead of a commit does
/// not make it stale.
#[test]
#[allow(clippy::too_many_lines)]
fn stale_and_duplicate_commits_terminate_and_commits_settle_first() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|state| {
        closed_book(state, 0, "2026", 100);
        closed_book(state, 1, "2026", 200);
        closed_book(state, 2, "2026", 300);
    }));
    let store = MemoryArchiveStore::default();
    let one = prepare_fiscal_archive(&canwu, 1, &store).expect("one book");
    let two = prepare_fiscal_archive(&canwu, 2, &store).expect("two books");
    // The same batch queued three times: the first applies, and its copies
    // settle as stale duplicates that record nothing.
    let first = enqueue_fiscal_archive(&mut canwu, &one, &store).expect("queued");
    for _ in 0..2 {
        let duplicate = enqueue_fiscal_archive(&mut canwu, &one, &store).expect("queued again");
        assert_eq!(first.retention_handle_id, duplicate.retention_handle_id);
    }
    let other = enqueue_fiscal_archive(&mut canwu, &two, &store).expect("queued");
    settle(&mut canwu);
    let maintenance: Vec<_> = canwu
        .events()
        .iter()
        .filter(|event| {
            event.kind.plugin_identity() == Some((PLUGIN_NAME, FISCAL_ARCHIVE_MAINTENANCE_EVENT))
        })
        .map(|event| event.summary.clone())
        .collect();
    assert_eq!(maintenance.len(), 4, "{maintenance:?}");
    assert!(maintenance[0].contains(" as applied:"));
    assert!(
        maintenance[1..3]
            .iter()
            .all(|summary| summary.contains(" as stale duplicate:"))
    );
    assert!(maintenance[3].contains(" as stale:"));
    let state = fiscal_state(&canwu);
    assert_eq!(state.archive.archived_book_count, 1);
    assert_eq!(
        state.archive.pending_handles[&first.retention_handle_id].phase,
        FiscalArchiveRetentionPhaseV1::Committed
    );
    assert_eq!(
        state.archive.pending_handles[&other.retention_handle_id].phase,
        FiscalArchiveRetentionPhaseV1::RejectedStale
    );
    assert_eq!(state.archive.maintenance_receipts.len(), 2);

    // Finalizing the applied handle twice queues two acknowledgements; the
    // second is a no-op. Its stale duplicate's receipt goes with it, while
    // the applied receipt stays for the next commit.
    finalize_fiscal_archive_retention(&mut canwu, &store, &first).expect("finalized");
    finalize_fiscal_archive_retention(&mut canwu, &store, &first).expect("finalized again");
    finalize_fiscal_archive_retention(&mut canwu, &store, &other).expect("stale finalized");
    settle(&mut canwu);
    let state = fiscal_state(&canwu);
    assert!(state.archive.pending_handles.is_empty());
    assert_eq!(state.archive.maintenance_receipts.len(), 1);
    assert_eq!(
        store.handle_phase(&other.retention_handle_id),
        Some(FiscalArchiveRetentionPhaseV1::RejectedStale)
    );
    assert!(
        finalize_fiscal_archive_retention(&mut canwu, &store, &first).is_err(),
        "a retired handle is not finalized again"
    );

    // An action admitted in one boundary settles in the next; a commit
    // queued in between settles first in that boundary, so the action, which
    // advances its scope's revision, does not make it stale.
    let revision = ledger_revision(&canwu, &treasury_scope(3));
    support::enqueue_requests(
        &mut canwu,
        &[(
            PRINCIPAL,
            assessment("between", TREASURY_AUTHORITY, &treasury_scope(3), "2030").against(revision),
        )],
    );
    canwu.step_canonical().expect("the admitting boundary");
    let receipt = enqueue_archive(&mut canwu, &store, 64);
    settle(&mut canwu);
    assert!(applied(&canwu, "between"));
    let state = fiscal_state(&canwu);
    assert_eq!(state.archive.archived_book_count, 3);
    assert_eq!(
        state.archive.pending_handles[&receipt.retention_handle_id].phase,
        FiscalArchiveRetentionPhaseV1::Committed
    );
    // With no closed book left, a batch archives the head receipt alone. An
    // action on a book it does not select leaves it current: it binds the
    // head and the books it selects, nothing else.
    let prepared = prepare_fiscal_archive(&canwu, 64, &store).expect("the head receipt alone");
    assert!(prepared.selected_books.is_empty());
    assert_eq!(prepared.selected_receipts.len(), 1);
    submit(
        &mut canwu,
        by_principal([assessment(
            "after",
            TREASURY_AUTHORITY,
            &treasury_scope(3),
            "2031",
        )]),
    );
    assert!(applied(&canwu, "after"));
    assert_eq!(
        prepare_fiscal_archive(&canwu, 64, &store).expect("the same batch"),
        prepared
    );
    enqueue_fiscal_archive(&mut canwu, &prepared, &store).expect("a batch that is still current");
    settle(&mut canwu);
    // Once a commit moved the head, the same batch is of an earlier head and
    // is refused at enqueue.
    let error = enqueue_fiscal_archive(&mut canwu, &prepared, &store)
        .expect_err("a batch of an earlier head");
    assert!(matches!(error, FiscalArchiveError::Mismatch(_)), "{error}");
    fixture.assert_restore_and_replay(&canwu);
}

/// More stale commits than the head has room for: the head keeps at most
/// [`canwu_fiscal::MAX_FISCAL_ARCHIVE_PENDING_HANDLES`] unacknowledged stale
/// handles, and each stale commit past them is refused with an event and
/// changes nothing, so the boundary commits.
#[test]
fn stale_commits_past_the_heads_room_are_refused_without_failing_the_boundary() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|state| {
        for scope in 0..4 {
            for cycle in 0..33 {
                closed_book(state, scope, &format!("{:04}", 2000 + cycle), 10);
            }
        }
    }));
    let store = MemoryArchiveStore::default();
    let batches = 130;
    let mut receipts = Vec::new();
    for limit in 1..=batches {
        let prepared = prepare_fiscal_archive(&canwu, limit, &store).expect("a batch");
        receipts.push(enqueue_fiscal_archive(&mut canwu, &prepared, &store).expect("queued"));
    }
    let refused_before = refusals(&canwu).len();
    settle(&mut canwu);
    let room = canwu_fiscal::MAX_FISCAL_ARCHIVE_PENDING_HANDLES;
    let state = fiscal_state(&canwu);
    assert_eq!(state.archive.archived_book_count, 1);
    assert_eq!(state.archive.pending_handles.len(), 1 + room);
    assert_eq!(state.archive.maintenance_receipts.len(), 1 + room);
    let refused = refusals(&canwu)[refused_before..].to_vec();
    assert_eq!(refused.len(), batches - 1 - room, "{:?}", refused.first());
    assert!(
        refused
            .iter()
            .all(|summary| summary.contains("stale commits")
                && summary.ends_with(FISCAL_INGRESS_INVALID_REJECTION))
    );
    // Acknowledged, the recorded handles leave; a refused commit has no
    // handle in the state, so it cannot be acknowledged.
    for receipt in &receipts[..=room] {
        finalize_fiscal_archive_retention(&mut canwu, &store, receipt).expect("finalized");
    }
    assert!(finalize_fiscal_archive_retention(&mut canwu, &store, &receipts[room + 1]).is_err());
    settle(&mut canwu);
    let state = fiscal_state(&canwu);
    assert!(state.archive.pending_handles.is_empty());
    assert_eq!(state.archive.maintenance_receipts.len(), 1);
    fixture.assert_restore_and_replay(&canwu);
}

/// A boundary whose only change of the state is archive maintenance, an
/// acknowledgement or a commit of the head receipt alone, publishes no
/// report: maintenance changes no book an observer sees and advances no
/// revision, and what an earlier boundary published is not published again.
#[test]
fn archive_maintenance_alone_publishes_no_report() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|state| {
        closed_book(state, 0, "2026", 100);
    }));
    let store = MemoryArchiveStore::default();
    submit(
        &mut canwu,
        by_principal([assessment(
            "assess",
            TREASURY_AUTHORITY,
            &treasury_scope(1),
            "2027",
        )]),
    );
    let records = |canwu: &Canwu| published_report_records(canwu, PRINCIPAL).len();
    let before_commit = records(&canwu);
    let receipt = enqueue_archive(&mut canwu, &store, 1);
    settle(&mut canwu);
    // The commit takes a book out, so the report changes once.
    let after_commit = records(&canwu);
    assert!(after_commit > before_commit);
    finalize_fiscal_archive_retention(&mut canwu, &store, &receipt).expect("finalized");
    settle(&mut canwu);
    assert_eq!(
        records(&canwu),
        after_commit,
        "an acknowledgement published"
    );
    let receipt = enqueue_archive(&mut canwu, &store, 1);
    settle(&mut canwu);
    assert_eq!(
        records(&canwu),
        after_commit,
        "a commit of the head receipt alone published"
    );
    finalize_fiscal_archive_retention(&mut canwu, &store, &receipt).expect("finalized");
    settle(&mut canwu);
    assert_eq!(
        records(&canwu),
        after_commit,
        "an acknowledgement published"
    );
    // A change of what the observer sees publishes again.
    submit(
        &mut canwu,
        by_principal([assessment(
            "assess.more",
            TREASURY_AUTHORITY,
            &treasury_scope(2),
            "2027",
        )]),
    );
    assert!(records(&canwu) > after_commit);
}

/// Archiving a book ends its declaration of the versions it cites, so on a
/// run that seals its evidence a book whose citations are newer than the
/// run's last sealed cut is refused at preparation: the host seals first,
/// then archives. The cut is the run's own, so a host cannot pass a wrong
/// one.
#[test]
#[allow(clippy::too_many_lines)]
fn a_book_citing_evidence_newer_than_the_last_seal_is_refused_until_it_is_sealed() {
    let fixture = Fixture::new();
    let plugins = fixture.plugins();
    let mut canwu = fixture.runtime(scenario(|_| {}));
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("a quiet boundary");
    assert_eq!(canwu.sealed_evidence_cursor(), None);
    let mut sealed = canwu.into_compacted().expect("compact interface");
    let evidence = MemoryEvidenceArchive::default();
    // A seal made here, before the quote exists.
    let earlier = seal(&mut sealed, &evidence);
    assert_eq!(sealed.sealed_evidence_cursor(), Some(earlier.end));
    sealed
        .enqueue_plugin_ingress(renewal_at(sealed.time(), AUDITOR_COMMISSION))
        .expect("commission renewal ingress");
    sealed
        .advance_canonical(canwu_api::SimDuration::minutes(1))
        .expect("the renewal settles");
    let quote = current_commission(&sealed, AUDITOR_COMMISSION);
    assert!(matches!(
        quote.established_by,
        canwu_api::DomainRecordVersionSource::BoundaryChange { .. }
    ));
    let scope = treasury_scope(0);
    submit_sealed(
        &mut sealed,
        by_principal([draft(
            "quoted",
            TREASURY_AUTHORITY,
            FiscalAction::OpenAssessment {
                assessment_id: "quoted.assessment".to_owned(),
                rule_id: RULE.to_owned(),
                scope_binding_id: scope.clone(),
                accounting_cycle_id: "2026".to_owned(),
                quantity: 100,
                unit: "grain".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: Some(quote.clone()),
            },
        )]),
    );
    submit_sealed(&mut sealed, by_principal([close("close", &scope, "2026")]));
    let settled = FiscalSettledAction::from_events(sealed.events());
    for action in ["quoted", "close"] {
        assert!(
            settled.iter().any(|outcome| outcome.action_id == action
                && outcome.disposition == FiscalActionDisposition::Applied),
            "{action} applies"
        );
    }

    let store = MemoryArchiveStore::default();
    let error = prepare_fiscal_archive(&sealed, 64, &store)
        .expect_err("the quote is newer than the run's last seal");
    assert_eq!(
        error,
        FiscalArchiveError::UnsealedCitations {
            scope_binding_id: scope.clone(),
            accounting_cycle_id: "2026".to_owned(),
        }
    );

    // Sealed again, the quote is in a segment, and the book archives.
    let segment = seal(&mut sealed, &evidence);
    assert_eq!(sealed.sealed_evidence_cursor(), Some(segment.end));
    let prepared = prepare_fiscal_archive(&sealed, 64, &store).expect("every citation is sealed");
    let receipt =
        enqueue_fiscal_archive(&mut sealed, &prepared, &store).expect("queued on the compact run");
    sealed
        .advance_canonical(canwu_api::SimDuration::minutes(1))
        .expect("the commit settles");
    finalize_fiscal_archive_retention(&mut sealed, &store, &receipt).expect("finalized");
    sealed
        .advance_canonical(canwu_api::SimDuration::minutes(1))
        .expect("the acknowledgement settles");
    let state = sealed_fiscal_state(&sealed);
    assert!(state.books().next().is_none());
    // The scope's ledger no longer declares the quote as required evidence.
    let record = sealed
        .typed_domain_record(&canwu_fiscal::fiscal_ledger_reference(&scope))
        .expect("the scope's ledger record");
    assert!(
        !record.payload.to_string().contains(&quote.record.id),
        "the archived book's quote is no longer declared"
    );
    let segments = vec![earlier, segment];
    let journal = sealed
        .replay_journal_with_segments(segments.clone())
        .expect("an exact replay journal");
    let replayed = Canwu::replay_from_journal(&plugins, &journal).expect("the sealed run replays");
    assert_eq!(
        replayed.snapshot(),
        sealed
            .snapshot_with_segments(segments)
            .expect("the full snapshot")
    );
}

/// Renews the auditor's commission on a compact run and closes a book of
/// `cycle` whose one assessment cites the renewed version as its quote.
/// Returns the quote.
fn close_quoted_book(
    run: &mut CompactedCanwu,
    scope: &str,
    cycle: &str,
) -> canwu_api::DomainRecordVersionRef {
    run.enqueue_plugin_ingress(renewal_at(run.time(), AUDITOR_COMMISSION))
        .expect("commission renewal ingress");
    run.advance_canonical(canwu_api::SimDuration::minutes(1))
        .expect("the renewal settles");
    let quote = current_commission(&*run, AUDITOR_COMMISSION);
    let (open, shut) = (format!("quoted.{cycle}"), format!("close.{cycle}"));
    submit_sealed(
        run,
        by_principal([draft(
            &open,
            TREASURY_AUTHORITY,
            FiscalAction::OpenAssessment {
                assessment_id: format!("quoted.assessment.{cycle}"),
                rule_id: RULE.to_owned(),
                scope_binding_id: scope.to_owned(),
                accounting_cycle_id: cycle.to_owned(),
                quantity: 100,
                unit: "grain".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: Some(quote.clone()),
            },
        )]),
    );
    submit_sealed(run, by_principal([close(&shut, scope, cycle)]));
    let settled = FiscalSettledAction::from_events(run.events());
    for action in [&open, &shut] {
        assert!(
            settled.iter().any(|outcome| outcome.action_id == *action
                && outcome.disposition == FiscalActionDisposition::Applied),
            "{action} applies"
        );
    }
    quote
}

/// Prepares, queues, settles, and acknowledges one archive commit of every
/// closed book on a compact run.
fn archive_compact(run: &mut CompactedCanwu, store: &MemoryArchiveStore) {
    let prepared = prepare_fiscal_archive(&*run, 64, store).expect("an archive batch");
    let receipt = enqueue_fiscal_archive(run, &prepared, store).expect("the commit is queued");
    run.advance_canonical(canwu_api::SimDuration::minutes(1))
        .expect("the commit settles");
    finalize_fiscal_archive_retention(run, store, &receipt).expect("finalized");
    run.advance_canonical(canwu_api::SimDuration::minutes(1))
        .expect("the acknowledgement settles");
}

/// A run restored from a continuation checkpoint holds no sealed segment,
/// so its base is not a seal (downstream gap G-91): it archives a closed
/// book citing a version established after the restore, and the
/// continuation checkpoint it writes after the commit restores. Once the
/// restored run seals, its own cut applies: a book citing a version newer
/// than that seal is refused until a later seal covers it.
#[test]
fn a_continuation_restored_run_archives_citations_newer_than_its_base_until_it_seals() {
    let fixture = Fixture::new();
    let plugins = fixture.plugins();
    let mut canwu = fixture.runtime(scenario(|_| {}));
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("a quiet boundary");
    let (checkpoint, hash) = canwu
        .continuation_checkpoint()
        .expect("a continuation checkpoint");
    let base = checkpoint.settled_cut;
    let mut restored =
        CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
            .expect("the save restores");
    assert_eq!(
        restored.continuation_base().map(|base| base.cut),
        Some(base)
    );
    assert_eq!(
        restored.sealed_evidence_cursor(),
        None,
        "a continuation restore is not a seal"
    );

    let scope = treasury_scope(0);
    let quote = close_quoted_book(&mut restored, &scope, "2026");
    assert!(
        matches!(
            quote.established_by,
            canwu_api::DomainRecordVersionSource::BoundaryChange { boundary, .. }
                if boundary.get() > base.boundary_count
        ),
        "the quote is newer than the restore's base"
    );
    let store = MemoryArchiveStore::default();
    archive_compact(&mut restored, &store);
    let state = sealed_fiscal_state(&restored);
    assert!(state.books().next().is_none(), "the book is archived");
    let record = restored
        .typed_domain_record(&canwu_fiscal::fiscal_ledger_reference(&scope))
        .expect("the scope's ledger record");
    assert!(
        !record.payload.to_string().contains(&quote.record.id),
        "the archived book's quote is no longer declared"
    );

    // The next save, which no longer carries the quote, restores, and the
    // archived book is read from the store through the restored head.
    let (checkpoint, hash) = restored
        .continuation_checkpoint()
        .expect("the checkpoint after the commit");
    let mut again =
        CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
            .expect("the save after the commit restores");
    assert_eq!(again.checkpoint_hash(), restored.checkpoint_hash());
    let Some(FiscalArchivedRecordV1::Book(archived)) = archived_fiscal_record(
        &sealed_fiscal_state(&again),
        &store,
        &book_key(&scope, "2026"),
    )
    .expect("an archive lookup") else {
        panic!("the archived book resolves");
    };
    assert!(
        archived
            .book
            .assessments
            .values()
            .any(|assessment| assessment.commutation_quote.as_ref() == Some(&quote)),
        "the archived book keeps its citation"
    );

    // Once the restored run has sealed, a citation newer than its own seal
    // is refused until a later seal covers it.
    let evidence = MemoryEvidenceArchive::default();
    again
        .settle_boundary(BoundaryRequest::at(again.time()))
        .expect("a quiet boundary");
    let earlier = seal(&mut again, &evidence);
    assert_eq!(again.sealed_evidence_cursor(), Some(earlier.end));
    close_quoted_book(&mut again, &scope, "2027");
    assert_eq!(
        prepare_fiscal_archive(&again, 64, &store).expect_err("the quote is newer than the seal"),
        FiscalArchiveError::UnsealedCitations {
            scope_binding_id: scope.clone(),
            accounting_cycle_id: "2027".to_owned(),
        }
    );
    let segment = seal(&mut again, &evidence);
    assert_eq!(again.sealed_evidence_cursor(), Some(segment.end));
    archive_compact(&mut again, &store);
    assert!(sealed_fiscal_state(&again).books().next().is_none());
}

/// A run that closes, archives, and publishes across a seal does so exactly
/// as its unsealed fork, a restore from its sealed segments, and its exact
/// replay: no boundary reads retained evidence or the archive store.
#[test]
#[allow(clippy::too_many_lines)]
fn a_sealed_run_archives_as_its_fork_restore_and_replay_do() {
    let fixture = Fixture::new();
    let plugins = fixture.plugins();
    let government = KnowledgeHolderRef::Entity(EntityRef::Government(GovernmentId::new(1)));
    let mut base = fixture.runtime(scenario(|state| {
        for (id, actor) in [
            ("observer.controlled", DEPUTY),
            ("observer.other", AUDITOR),
            ("observer.removed", GRANARY_PRINCIPAL),
        ] {
            state.add_observer(FiscalObserverBinding {
                controlled_by: Some(treasury()),
                ..observer_binding(id, actor, BTreeSet::from([treasury()]))
            });
        }
    }));
    let scope = treasury_scope(0);
    submit(
        &mut base,
        by_principal([assessment("assess", TREASURY_AUTHORITY, &scope, "2026")]),
    );
    submit(
        &mut base,
        by_principal([collection(
            "authorize",
            &scope,
            "assess.assessment",
            "request.a",
            60,
        )]),
    );
    let evidence = record_results(
        &mut base,
        &[(false, execution_claim("result.a", &scope, "request.a", 60))],
    );
    let now = base.time();
    enqueue_execution_receipts(
        &mut base,
        now,
        &[receipt_packet("receipt.a", &scope, "request.a", evidence)],
    )
    .expect("receipt");
    settle(&mut base);
    submit(&mut base, by_principal([close("close", &scope, "2026")]));
    base.settle_boundary(BoundaryRequest::at(base.time()))
        .expect("a quiet boundary before sealing");

    let mut unsealed = base.fork();
    let mut sealed = base.into_compacted().expect("compact interface");
    let segment = seal(&mut sealed, &MemoryEvidenceArchive::default());
    let mut from_sealed = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
        sealed.checkpoint().expect("compact checkpoint"),
        vec![segment.clone()],
        &plugins,
    )
    .expect("the sealed segments restore the compact run");

    let unsealed_store = MemoryArchiveStore::default();
    archive(&mut unsealed, &unsealed_store, 64);
    // After the seal: re-point one observer binding to another person and one
    // to a government, remove a third, and assess again.
    let later = || {
        [
            by_principal([draft(
                "rebind",
                TREASURY_AUTHORITY,
                FiscalAction::RebindObserver {
                    observer_binding_id: "observer.controlled".to_owned(),
                    actor: Some(SUCCESSOR),
                    knowledge_holder: KnowledgeHolderRef::Person(SUCCESSOR),
                },
            )]),
            by_principal([draft(
                "rebind.government",
                TREASURY_AUTHORITY,
                FiscalAction::RebindObserver {
                    observer_binding_id: "observer.other".to_owned(),
                    actor: None,
                    knowledge_holder: KnowledgeHolderRef::Entity(EntityRef::Government(
                        GovernmentId::new(1),
                    )),
                },
            )]),
            by_principal([draft(
                "remove",
                TREASURY_AUTHORITY,
                FiscalAction::RemoveObserver {
                    observer_binding_id: "observer.removed".to_owned(),
                },
            )]),
            by_principal([assessment(
                "later",
                TREASURY_AUTHORITY,
                &treasury_scope(1),
                "2027",
            )]),
        ]
    };
    for commands in later() {
        submit(&mut unsealed, commands);
    }
    let mut roots = Vec::new();
    for run in [&mut sealed, &mut from_sealed] {
        let store = MemoryArchiveStore::default();
        let prepared = prepare_fiscal_archive(&*run, 64, &store).expect("the archive batch");
        let receipt =
            enqueue_fiscal_archive(run, &prepared, &store).expect("queued on the compact run");
        run.advance_canonical(canwu_api::SimDuration::minutes(1))
            .expect("the commit settles");
        finalize_fiscal_archive_retention(run, &store, &receipt).expect("finalized");
        run.advance_canonical(canwu_api::SimDuration::minutes(1))
            .expect("the acknowledgement settles");
        for commands in later() {
            submit_sealed(run, commands);
        }
        roots.push(sealed_fiscal_state(run).archive.directory_root);
    }
    let state = fiscal_state(&unsealed);
    assert_eq!(state.archive.archived_book_count, 1);
    for root in &roots {
        assert_eq!(*root, state.archive.directory_root);
    }
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    assert_eq!(from_sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    for holder in [
        KnowledgeHolderRef::Person(PRINCIPAL),
        KnowledgeHolderRef::Person(DEPUTY),
        KnowledgeHolderRef::Person(SUCCESSOR),
        KnowledgeHolderRef::Person(AUDITOR),
        KnowledgeHolderRef::Person(GRANARY_PRINCIPAL),
        government.clone(),
    ] {
        let ledger = |knowledge: &canwu_api::KnowledgeSnapshot| {
            support::held_records(knowledge, &holder)
                .unwrap_or_else(|| panic!("{holder:?} holds reports"))
        };
        assert_eq!(ledger(sealed.knowledge()), ledger(unsealed.knowledge()));
        assert_eq!(
            ledger(from_sealed.knowledge()),
            ledger(unsealed.knowledge())
        );
    }
    assert!(
        canwu_fiscal::fiscal_reports_of_holder(&unsealed, &government)
            .expect("the government's reports")
            .contains_key("observer.other")
    );
    assert_eq!(
        fiscal_report(&unsealed, PRINCIPAL).projection.facts.len(),
        1
    );

    let journal = sealed
        .replay_journal_with_segments(vec![segment])
        .expect("an exact replay journal");
    let replayed = Canwu::replay_from_journal(&plugins, &journal).expect("the sealed run replays");
    assert_eq!(replayed.snapshot(), unsealed.snapshot());
}
