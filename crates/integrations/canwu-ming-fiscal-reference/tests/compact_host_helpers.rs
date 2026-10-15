//! The fiscal and adapter host helpers drive a compact runtime as they drive
//! a full-history one. The same fulfillment flow runs on `Canwu` and on a
//! `CompactedCanwu` that seals its evidence after every settled step (in two
//! phases, because fiscal state declares payload-required evidence), so each
//! helper reads a run whose earlier boundaries are sealed. Both runs reach the
//! same receipts, state, and checkpoint hashes.
//!
//! A run restored from a continuation checkpoint written with the
//! assessment open, the execution authorized, and the adapter result still
//! queued continues in lockstep with the run it was written from.

use canwu_api::{
    ArchiveProvider, ArchiveStore, ArchiveStoreOutcome, BoundaryReceipt, BoundaryRequest, Canwu,
    CanwuError, CanwuRun, CommandEnvelope, CommandRequest, CommandRequestId, CompactedCanwu,
    EntityRef, EvidenceJournalSegment, Issuer, KnowledgeHolderRef, PersonId, ResourceId,
    SimDuration,
};
use canwu_fiscal::{
    FiscalAction, FiscalActionBatch, FiscalActionEntry, FiscalExecutionKind,
    FiscalExecutionReceiptPacket, FiscalPaymentForm, FiscalReceiptDisposition, FiscalState,
    audit_fiscal_records, enqueue_execution_receipts, fiscal_action_command, fiscal_action_target,
    fiscal_reports_of_holder, fiscal_target_revision, load_fiscal_state,
};
use canwu_ming_fiscal_reference::{
    DEFAULT_SEED, MingFiscalExecutionAdapterPlugin, ReferenceFiscalExecutionEvidence,
    enqueue_reference_execution_result, ming_fiscal_plugin, ming_fiscal_reference_scenario,
    new_ming_fiscal_reference, reference_execution_evidence_version,
};
use canwu_reference_world::ReferenceWorldPlugin;
use std::cell::RefCell;
use std::collections::BTreeMap;

/// What the flow needs from a handle beyond [`CanwuRun`]: tracked commands,
/// settlement, and, for the compact handle, a seal.
trait Host: CanwuRun {
    fn enqueue_tracked(&mut self, request: CommandRequest);
    fn advance(&mut self) -> Vec<BoundaryReceipt>;
    fn settle_now(&mut self) -> BoundaryReceipt;
    fn seal(&mut self);
}

impl Host for Canwu {
    fn enqueue_tracked(&mut self, request: CommandRequest) {
        let at = self.time();
        self.enqueue_command(at, 0, request)
            .expect("command enqueues");
    }
    fn advance(&mut self) -> Vec<BoundaryReceipt> {
        self.advance_canonical(SimDuration::minutes(1))
            .expect("the run advances")
    }
    fn settle_now(&mut self) -> BoundaryReceipt {
        let at = self.time();
        self.settle_boundary(BoundaryRequest::at(at))
            .expect("the boundary settles")
    }
    fn seal(&mut self) {}
}

impl Host for CompactedCanwu {
    fn enqueue_tracked(&mut self, request: CommandRequest) {
        let at = self.time();
        self.enqueue_command(at, 0, request)
            .expect("command enqueues");
    }
    fn advance(&mut self) -> Vec<BoundaryReceipt> {
        self.advance_canonical(SimDuration::minutes(1))
            .expect("the run advances")
    }
    fn settle_now(&mut self) -> BoundaryReceipt {
        let at = self.time();
        self.settle_boundary(BoundaryRequest::at(at))
            .expect("the boundary settles")
    }
    /// Fiscal state declares payload-required evidence, so the compact run
    /// seals in two phases through an evidence store.
    fn seal(&mut self) {
        let Some(prepared) = self.prepare_evidence_seal().expect("the seal prepares") else {
            return;
        };
        SEGMENTS.with(|store| {
            store
                .store_evidence_segment(&prepared.segment)
                .expect("the segment stores");
            self.commit_evidence_seal(&prepared.token, store)
                .expect("the settled history seals");
        });
    }
}

thread_local! {
    /// Every segment the compact run sealed, which a later commit reads back
    /// for the payload-required evidence it still names.
    static SEGMENTS: SegmentStore = SegmentStore::default();
}

/// An in-memory evidence store for the two-phase seal.
#[derive(Default)]
struct SegmentStore(RefCell<BTreeMap<String, EvidenceJournalSegment>>);

impl ArchiveProvider for SegmentStore {
    fn load_evidence_segment(
        &self,
        segment_id: &str,
    ) -> Result<Option<EvidenceJournalSegment>, CanwuError> {
        Ok(self.0.borrow().get(segment_id).cloned())
    }
}

impl ArchiveStore for SegmentStore {
    fn store_evidence_segment(
        &self,
        segment: &EvidenceJournalSegment,
    ) -> Result<ArchiveStoreOutcome, CanwuError> {
        let id = segment
            .archive
            .as_ref()
            .map(|archive| archive.header.segment_id.clone())
            .unwrap_or_default();
        Ok(match self.0.borrow_mut().insert(id, segment.clone()) {
            Some(_) => ArchiveStoreOutcome::AlreadyPresent,
            None => ArchiveStoreOutcome::Stored,
        })
    }
}

fn fiscal_state(run: &impl CanwuRun) -> FiscalState {
    load_fiscal_state(run)
        .expect("fiscal records")
        .expect("a fiscal state")
}

/// One settled step: settle what is due, admit its emissions, and seal.
fn step(run: &mut impl Host) -> Vec<BoundaryReceipt> {
    let mut receipts = run.advance();
    receipts.push(run.settle_now());
    run.seal();
    receipts
}

fn action(run: &mut impl Host, actor: PersonId, request_id: u64, action: FiscalAction) {
    let binding = "authority.revenue-minister";
    let target = fiscal_action_target(&*run, binding, &action).expect("the action's target");
    let expected_revision = fiscal_target_revision(&*run, &target)
        .expect("the target's revision")
        .expect("an existing target");
    let batch = FiscalActionBatch::single(
        binding,
        FiscalActionEntry {
            action_id: format!("compact.action.{request_id}"),
            expected_revision,
            action,
        },
    );
    let command = fiscal_action_command(&batch).expect("fiscal command");
    let at = run.time();
    run.enqueue_tracked(CommandRequest::new(
        CommandRequestId::new(request_id),
        run.revision(),
        CommandEnvelope::new(Issuer::Actor(actor), command).at_time(at),
    ));
}

/// The adapter result of the fulfillment flow.
fn execution_evidence() -> ReferenceFiscalExecutionEvidence {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    ReferenceFiscalExecutionEvidence {
        id: "evidence.compact".to_owned(),
        scope_binding_id: "scope.lower-yangzi.land".to_owned(),
        external_operation_id: "resource-transfer.compact".to_owned(),
        request_id: "compact.execution".to_owned(),
        quantity: 70,
        unit: "shi_grain_equivalent".to_owned(),
        payment_form: FiscalPaymentForm::Grain,
        execution_kind: FiscalExecutionKind::Collect,
        resource: ResourceId::new(1),
        source: EntityRef::Person(reference.world_ids.observer),
        target: EntityRef::Government(reference.world_ids.government),
        disposition: FiscalReceiptDisposition::Fulfilled,
    }
}

/// Assesses, authorizes, publishes adapter evidence, and settles a receipt
/// for it, with every step's history sealed before the next helper reads.
fn fulfill(run: &mut impl Host) -> Vec<BoundaryReceipt> {
    let mut receipts = open_and_authorize(run);
    let evidence = execution_evidence();
    let at = run.time();
    enqueue_reference_execution_result(run, at, &evidence).expect("adapter result enqueues");
    receipts.extend(step(run));
    receipts.extend(settle_receipt(run));
    receipts
}

/// Opens the assessment and authorizes its execution.
fn open_and_authorize(run: &mut impl Host) -> Vec<BoundaryReceipt> {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    let actor = reference.world_ids.observer;
    let government = reference.world_ids.government;
    let mut receipts = Vec::new();
    action(
        run,
        actor,
        1,
        FiscalAction::OpenAssessment {
            assessment_id: "compact.assessment".to_owned(),
            rule_id: "yellow_register_land_assessment".to_owned(),
            scope_binding_id: "scope.lower-yangzi.land".to_owned(),
            accounting_cycle_id: "hongwu-1391.cycle-1".to_owned(),
            quantity: 100,
            unit: "shi_grain_equivalent".to_owned(),
            payment_form: FiscalPaymentForm::Grain,
            commutation_quote: None,
        },
    );
    receipts.extend(step(run));
    action(
        run,
        actor,
        2,
        FiscalAction::AuthorizeExecution {
            request_id: "compact.execution".to_owned(),
            scope_binding_id: "scope.lower-yangzi.land".to_owned(),
            assessment_id: "compact.assessment".to_owned(),
            kind: FiscalExecutionKind::Collect,
            quantity: 70,
            unit: "shi_grain_equivalent".to_owned(),
            resource: ResourceId::new(1),
            source: EntityRef::Person(actor),
            target: EntityRef::Government(government),
            purpose: "compact host helpers".to_owned(),
        },
    );
    receipts.extend(step(run));
    receipts
}

/// Settles the execution receipt for the published adapter evidence.
fn settle_receipt(run: &mut impl Host) -> Vec<BoundaryReceipt> {
    let evidence = execution_evidence();
    let mut receipts = Vec::new();
    // The evidence's establishing boundary is sealed by now in the compact
    // run; its exact version still comes from the current-version index.
    let version =
        reference_execution_evidence_version(run, &evidence.id).expect("exact evidence version");
    let at = run.time();
    enqueue_execution_receipts(
        run,
        at,
        &[FiscalExecutionReceiptPacket {
            receipt_id: "receipt.compact".to_owned(),
            scope_binding_id: "scope.lower-yangzi.land".to_owned(),
            request_id: "compact.execution".to_owned(),
            external_evidence: [version].into_iter().collect(),
        }],
    )
    .expect("receipt enqueues");
    receipts.extend(step(run));
    receipts
}

#[test]
fn fiscal_and_adapter_helpers_drive_a_sealed_compact_run_as_a_full_one() {
    let mut full = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    let mut compact = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391")
        .expect("runtime")
        .into_compacted()
        .expect("compact runtime");
    let full_receipts = fulfill(&mut full);
    let compact_receipts = fulfill(&mut compact);
    assert_eq!(compact_receipts, full_receipts);
    assert_eq!(compact.checkpoint_hash(), full.checkpoint_hash());
    assert_eq!(
        compact.authoritative_state_hash().expect("state hash"),
        full.authoritative_state_hash().expect("state hash")
    );
    let state = fiscal_state(&compact);
    assert_eq!(state, fiscal_state(&full));
    assert_eq!(
        state
            .execution_receipts()
            .find(|receipt| receipt.id == "receipt.compact")
            .map(|receipt| receipt.quantity),
        Some(70)
    );
    // The host's audit and its report read work on the sealed handle too.
    assert_eq!(
        audit_fiscal_records(&compact).expect("the sealed run's records audit"),
        state
    );
    let observer = ming_fiscal_reference_scenario("hongwu-1391")
        .expect("fixture")
        .world_ids
        .observer;
    let holder = KnowledgeHolderRef::Person(observer);
    let reports = fiscal_reports_of_holder(&compact, &holder).expect("sealed reports");
    assert!(!reports.is_empty(), "the observer holds a report");
    assert_eq!(
        reports,
        fiscal_reports_of_holder(&full, &holder).expect("full reports")
    );
    assert!(
        compact.boundaries().len() < full.boundaries().len(),
        "the compact run sealed its earlier boundaries"
    );
}

#[test]
fn a_continuation_restored_fiscal_run_continues_in_lockstep() {
    let mut full = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    open_and_authorize(&mut full);
    // The adapter result is queued for a later instant, so it is pending at
    // the cut beside the open assessment and the authorized execution.
    let at = full.time() + SimDuration::minutes(1);
    enqueue_reference_execution_result(&mut full, at, &execution_evidence())
        .expect("adapter result enqueues");
    let (checkpoint, hash) = full
        .continuation_checkpoint()
        .expect("the continuation checkpoint builds");
    assert_eq!(checkpoint.pending_ingress.len(), 1);
    let world = ReferenceWorldPlugin;
    let adapter = MingFiscalExecutionAdapterPlugin;
    let fiscal = ming_fiscal_plugin();
    let mut restored = CompactedCanwu::from_continuation_checkpoint_json_verified(
        &checkpoint.to_json().expect("checkpoint JSON"),
        &hash,
        &[&world, &adapter, &fiscal],
    )
    .expect("the fiscal run restores");
    assert_eq!(restored.checkpoint_hash(), full.checkpoint_hash());

    let mut full_receipts = step(&mut full);
    let mut restored_receipts = step(&mut restored);
    full_receipts.extend(settle_receipt(&mut full));
    restored_receipts.extend(settle_receipt(&mut restored));
    for _ in 0..30 {
        full_receipts.extend(step(&mut full));
        restored_receipts.extend(step(&mut restored));
        assert_eq!(restored.checkpoint_hash(), full.checkpoint_hash());
    }
    assert_eq!(restored_receipts, full_receipts);
    assert_eq!(
        restored.authoritative_state_hash().expect("state hash"),
        full.authoritative_state_hash().expect("state hash")
    );
    let state = fiscal_state(&restored);
    assert_eq!(state, fiscal_state(&full));
    assert_eq!(
        state
            .execution_receipts()
            .find(|receipt| receipt.id == "receipt.compact")
            .map(|receipt| receipt.quantity),
        Some(70)
    );
    assert_eq!(
        restored
            .continuation_checkpoint()
            .expect("restored checkpoint")
            .1,
        full.continuation_checkpoint().expect("full checkpoint").1
    );
}
