//! The fiscal and adapter host helpers drive a compact runtime as they drive
//! a full-history one. The same fulfillment flow runs on `Canwu` and on a
//! `CompactedCanwu` that seals its evidence after every settled step (in two
//! phases, because fiscal state declares payload-required evidence), so each
//! helper reads a run whose earlier boundaries are sealed. Both runs reach the
//! same receipts, state, and checkpoint hashes.

use canwu_api::{
    ArchiveProvider, ArchiveStore, ArchiveStoreOutcome, BoundaryReceipt, BoundaryRequest, Canwu,
    CanwuError, CanwuRun, CommandEnvelope, CommandRequest, CommandRequestId, CompactedCanwu,
    EntityRef, EvidenceJournalSegment, Issuer, PersonId, ResourceId, SimDuration,
};
use canwu_fiscal::{
    FiscalAction, FiscalActionRequest, FiscalExecutionKind, FiscalExecutionReceiptPacket,
    FiscalPaymentForm, FiscalReceiptDisposition, FiscalState, FiscalStateRecord,
    enqueue_execution_receipt, fiscal_action_command, fiscal_state_reference,
};
use canwu_ming_fiscal_reference::{
    DEFAULT_SEED, ReferenceFiscalExecutionEvidence, enqueue_reference_execution_result,
    ming_fiscal_reference_scenario, new_ming_fiscal_reference,
    reference_execution_evidence_version,
};
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
    run.domain_record(&fiscal_state_reference().into_untyped())
        .expect("fiscal state")
        .decode_payload::<FiscalStateRecord>()
        .expect("fiscal state payload")
}

/// One settled step: settle what is due, admit its emissions, and seal.
fn step(run: &mut impl Host) -> Vec<BoundaryReceipt> {
    let mut receipts = run.advance();
    receipts.push(run.settle_now());
    run.seal();
    receipts
}

fn action(run: &mut impl Host, actor: PersonId, request_id: u64, action: FiscalAction) {
    let request = FiscalActionRequest {
        action_id: format!("compact.action.{request_id}"),
        authority_binding_id: "authority.revenue-minister".to_owned(),
        expected_procedure_revision: fiscal_state(run).procedure_revision,
        action,
    };
    let command = fiscal_action_command(&request).expect("fiscal command");
    let at = run.time();
    run.enqueue_tracked(CommandRequest::new(
        CommandRequestId::new(request_id),
        run.revision(),
        CommandEnvelope::new(Issuer::Actor(actor), command).at_time(at),
    ));
}

/// Assesses, authorizes, publishes adapter evidence, and settles a receipt
/// for it, with every step's history sealed before the next helper reads.
fn fulfill(run: &mut impl Host) -> Vec<BoundaryReceipt> {
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
    let evidence = ReferenceFiscalExecutionEvidence {
        id: "evidence.compact".to_owned(),
        scope_binding_id: "scope.lower-yangzi.land".to_owned(),
        external_operation_id: "resource-transfer.compact".to_owned(),
        request_id: "compact.execution".to_owned(),
        quantity: 70,
        unit: "shi_grain_equivalent".to_owned(),
        payment_form: FiscalPaymentForm::Grain,
        execution_kind: FiscalExecutionKind::Collect,
        resource: ResourceId::new(1),
        source: EntityRef::Person(actor),
        target: EntityRef::Government(government),
        disposition: FiscalReceiptDisposition::Fulfilled,
    };
    let at = run.time();
    enqueue_reference_execution_result(run, at, &evidence).expect("adapter result enqueues");
    receipts.extend(step(run));
    // The evidence's establishing boundary is sealed by now in the compact
    // run; its exact version still comes from the current-version index.
    let version =
        reference_execution_evidence_version(run, &evidence.id).expect("exact evidence version");
    let at = run.time();
    enqueue_execution_receipt(
        run,
        at,
        &FiscalExecutionReceiptPacket {
            receipt_id: "receipt.compact".to_owned(),
            scope_binding_id: "scope.lower-yangzi.land".to_owned(),
            request_id: "compact.execution".to_owned(),
            external_evidence: [version].into_iter().collect(),
        },
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
    assert!(
        compact.boundaries().len() < full.boundaries().len(),
        "the compact run sealed its earlier boundaries"
    );
}
