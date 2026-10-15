//! The reference validator decides from state a seal cannot change: committed
//! current records, the exact current or initial-scenario version of a
//! record, and the journal counters. A run whose receipt, commutation quote,
//! and audit cite evidence that a seal then archives validates alike as a
//! sealed run, an unsealed fork, a restore from the sealed segments, and an
//! exact replay; and an audit that cites evidence the run never issued is
//! refused alike by all of them.

#![allow(clippy::too_many_lines)]

use canwu_api::{
    ArchiveProvider, ArchiveStore, ArchiveStoreOutcome, BoundaryRequest, Canwu, CanwuError,
    CanwuRun, CommandEnvelope, CommandRequest, CommandRequestId, CompactedCanwu,
    DomainRecordVersionSource, EventId, EvidenceJournalSegment, EvidenceRef, Issuer, PersonId,
    SimDuration, SimulationPlugin,
};
use canwu_fiscal::{
    FiscalAction, FiscalActionRequest, FiscalAuditSeverity, FiscalState, FiscalStateRecord,
    fiscal_action_command, fiscal_state_reference,
};
use canwu_ming_fiscal_reference::{
    DEFAULT_SEED, MingFiscalExecutionAdapterPlugin, ming_fiscal_plugin,
    ming_fiscal_reference_scenario, new_ming_fiscal_reference, replay_ming_fiscal_reference,
    run_ming_fiscal_sample_cycle, validate_ming_fiscal_reference,
};
use canwu_reference_world::ReferenceWorldPlugin;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

const AUTHORITY: &str = "authority.revenue-minister";

/// What the flow needs from a runtime handle beyond the validator's reads.
trait Host: CanwuRun {
    fn enqueue(&mut self, request: CommandRequest);
    fn advance(&mut self);
}

macro_rules! impl_host {
    ($handle:ty) => {
        impl Host for $handle {
            fn enqueue(&mut self, request: CommandRequest) {
                let at = <$handle>::time(self);
                self.enqueue_command(at, 0, request)
                    .expect("the action queues");
            }
            fn advance(&mut self) {
                self.advance_canonical(SimDuration::minutes(1))
                    .expect("the run advances");
            }
        }
    };
}

impl_host!(Canwu);
impl_host!(CompactedCanwu);

fn fiscal_state<H: Host>(canwu: &H) -> canwu_fiscal::FiscalState {
    canwu
        .domain_record(fiscal_state_reference().as_untyped())
        .expect("fiscal state")
        .decode_payload::<FiscalStateRecord>()
        .expect("fiscal state payload")
}

/// Settles one fiscal action and requires it to have applied: `applied`
/// finds its effect in the fiscal state.
fn act<H: Host>(
    canwu: &mut H,
    actor: PersonId,
    request_id: u64,
    action: FiscalAction,
    applied: impl Fn(&FiscalState) -> bool,
) {
    let request = FiscalActionRequest {
        action_id: format!("action.{request_id}"),
        authority_binding_id: AUTHORITY.to_owned(),
        expected_procedure_revision: fiscal_state(canwu).procedure_revision,
        action,
    };
    let command = fiscal_action_command(&request).expect("fiscal command");
    let at = canwu.time();
    let revision = canwu.revision();
    canwu.enqueue(CommandRequest::new(
        CommandRequestId::new(request_id),
        revision,
        CommandEnvelope::new(Issuer::Actor(actor), command).at_time(at),
    ));
    canwu.advance();
    canwu.advance();
    assert!(applied(&fiscal_state(canwu)), "the action applies");
}

fn audit_recorded(id: &'static str) -> impl Fn(&FiscalState) -> bool {
    move |state| state.audits().any(|audit| audit.id == id)
}

fn audit(
    audit_id: &str,
    scope: &str,
    target_id: &str,
    evidence: BTreeSet<EvidenceRef>,
) -> FiscalAction {
    FiscalAction::RecordAudit {
        audit_id: audit_id.to_owned(),
        scope_binding_id: scope.to_owned(),
        target_id: target_id.to_owned(),
        severity: FiscalAuditSeverity::Concern,
        finding: "the collection is checked against its cited evidence".to_owned(),
        evidence,
    }
}

/// An in-memory evidence store for the two-phase seal that payload-required
/// fiscal evidence needs.
#[derive(Default)]
struct Segments(RefCell<BTreeMap<String, EvidenceJournalSegment>>);

impl ArchiveProvider for Segments {
    fn load_evidence_segment(
        &self,
        segment_id: &str,
    ) -> Result<Option<EvidenceJournalSegment>, CanwuError> {
        Ok(self.0.borrow().get(segment_id).cloned())
    }
}

impl ArchiveStore for Segments {
    fn store_evidence_segment(
        &self,
        segment: &EvidenceJournalSegment,
    ) -> Result<ArchiveStoreOutcome, CanwuError> {
        let id = segment
            .archive
            .as_ref()
            .expect("a sealed segment is indexed")
            .header
            .segment_id
            .clone();
        Ok(match self.0.borrow_mut().insert(id, segment.clone()) {
            Some(_) => ArchiveStoreOutcome::AlreadyPresent,
            None => ArchiveStoreOutcome::Stored,
        })
    }
}

#[test]
fn the_validator_answers_alike_sealed_unsealed_restored_and_replayed() {
    let actor = ming_fiscal_reference_scenario("hongwu-1391")
        .expect("fixture")
        .world_ids
        .observer;
    let mut base = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("reference run");
    // An assessment, its execution, typed adapter evidence, and the receipt
    // that cites that evidence.
    run_ming_fiscal_sample_cycle(&mut base, "sealed").expect("sample cycle");

    // An assessment quoting the fiscal state's version of this moment; the
    // action that applies it supersedes that version.
    let quoted = base
        .current_domain_record_version(fiscal_state_reference().as_untyped())
        .expect("provenance")
        .expect("the fiscal state has a current version");
    assert!(matches!(
        quoted.established_by,
        DomainRecordVersionSource::BoundaryChange { .. }
    ));
    let sample = fiscal_state(&base)
        .assessments()
        .find(|assessment| assessment.id == "sealed.assessment")
        .cloned()
        .expect("the sample assessment is hot");
    let scope = sample.scope_binding_id.clone();
    act(
        &mut base,
        actor,
        100,
        FiscalAction::OpenAssessment {
            assessment_id: "quoted.assessment".to_owned(),
            rule_id: sample.rule_id.clone(),
            scope_binding_id: scope.clone(),
            accounting_cycle_id: format!("{}-quoted", sample.accounting_cycle_id),
            quantity: 40,
            unit: sample.unit.clone(),
            payment_form: sample.payment_form,
            commutation_quote: Some(quoted.clone()),
        },
        |state| {
            state
                .assessments()
                .any(|assessment| assessment.id == "quoted.assessment")
        },
    );
    assert!(
        base.replay_stable_domain_record_version(&quoted).is_none(),
        "the quoted version is superseded"
    );

    // An audit citing an event, a command, a boundary, and the superseded
    // version.
    let cited = BTreeSet::from([
        EvidenceRef::Event(base.events()[0].id),
        EvidenceRef::Command(base.commands()[0].id),
        EvidenceRef::Boundary(base.boundaries()[0].id),
        EvidenceRef::DomainRecordVersion(quoted.clone()),
    ]);
    act(
        &mut base,
        actor,
        101,
        audit("sealed.audit", &scope, "sealed.assessment", cited),
        audit_recorded("sealed.audit"),
    );
    validate_ming_fiscal_reference(&base).expect("the live run validates");

    // A quiet boundary admits what the last one emitted, so the whole
    // history can seal. The run is then forked unsealed, sealed in two
    // phases, and restored from the sealed segment; it is also replayed
    // exactly.
    let now = base.time();
    base.settle_boundary(BoundaryRequest::at(now))
        .expect("a quiet boundary settles");
    let journal = base.replay_journal();
    let mut unsealed = base.fork();
    let mut sealed = base.into_compacted().expect("compact runtime");
    let prepared = sealed
        .prepare_evidence_seal()
        .expect("the seal prepares")
        .expect("the run holds evidence");
    let store = Segments::default();
    store
        .store_evidence_segment(&prepared.segment)
        .expect("the segment stores");
    sealed
        .commit_evidence_seal(&prepared.token, &store)
        .expect("the history seals");
    let DomainRecordVersionSource::BoundaryChange { boundary, .. } = quoted.established_by else {
        unreachable!("checked above");
    };
    assert!(
        prepared
            .segment
            .boundaries
            .iter()
            .any(|sealed_boundary| sealed_boundary.id == boundary),
        "the seal archives the boundary that established the quoted version"
    );
    let plugins: [&dyn SimulationPlugin; 3] = [
        &ReferenceWorldPlugin,
        &MingFiscalExecutionAdapterPlugin,
        &ming_fiscal_plugin(),
    ];
    let mut restored = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
        sealed.checkpoint().expect("compact checkpoint"),
        vec![prepared.segment.clone()],
        &plugins,
    )
    .expect("the sealed segment restores");
    let replayed = replay_ming_fiscal_reference(&journal).expect("the exact replay validates");

    validate_ming_fiscal_reference(&unsealed).expect("the unsealed fork validates");
    validate_ming_fiscal_reference(&sealed).expect("the sealed run validates");
    validate_ming_fiscal_reference(&restored).expect("the restore validates");
    validate_ming_fiscal_reference(&replayed).expect("the replay validates");

    // An audit citing an event the run never issued is admitted by the
    // fiscal plugin, which only bounds audit evidence, and refused by the
    // validator.
    let mut probe = unsealed.fork();
    let unissued = BTreeSet::from([EvidenceRef::Event(EventId::new(
        sealed.evidence_cursor().expect("cursor").event_count + 1_000,
    ))]);
    act(
        &mut probe,
        actor,
        102,
        audit("probe.audit", &scope, "quoted.assessment", unissued),
        audit_recorded("probe.audit"),
    );
    assert!(validate_ming_fiscal_reference(&probe).is_err());

    // An audit citing a version of the fiscal state the run never reached,
    // by a boundary the run settled, is refused alike by the unsealed fork,
    // the sealed run, the restore, and the exact replay.
    let mut future = quoted.clone();
    future.version = 999;
    assert!(
        unsealed
            .domain_record(fiscal_state_reference().as_untyped())
            .expect("fiscal state")
            .version
            < 999,
        "the state never reached the cited version"
    );
    let late = || {
        audit(
            "late.audit",
            &scope,
            "quoted.assessment",
            BTreeSet::from([EvidenceRef::DomainRecordVersion(future.clone())]),
        )
    };
    act(
        &mut unsealed,
        actor,
        102,
        late(),
        audit_recorded("late.audit"),
    );
    act(
        &mut sealed,
        actor,
        102,
        late(),
        audit_recorded("late.audit"),
    );
    act(
        &mut restored,
        actor,
        102,
        late(),
        audit_recorded("late.audit"),
    );
    let hash = CanwuRun::checkpoint_hash(&unsealed).to_owned();
    assert_eq!(CanwuRun::checkpoint_hash(&sealed), hash);
    assert_eq!(CanwuRun::checkpoint_hash(&restored), hash);
    let refusals = [
        validate_ming_fiscal_reference(&unsealed),
        validate_ming_fiscal_reference(&sealed),
        validate_ming_fiscal_reference(&restored),
        replay_ming_fiscal_reference(&unsealed.replay_journal()).map(|_| ()),
    ]
    .map(|result| {
        result
            .expect_err("a citation of a future version is refused")
            .message
    });
    assert!(refusals[0].contains("audit evidence is unavailable"));
    assert!(refusals.iter().all(|message| message == &refusals[0]));
}
