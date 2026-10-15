//! Settlement of a boundary's fiscal packets (phase 7): archive maintenance,
//! action batches, receipt batches, and historical-context changes, each on
//! the records it touches and nothing else.

use crate::appointment::{
    apply_appointment, check_appointment, installed_basis, is_appointment_action, is_proposal,
};
use crate::archive::{
    FiscalArchiveRetentionPhaseV1, FiscalArchiveSettlement, VerifiedFiscalArchiveCommitV1,
};
use crate::derive::{context_regions, jurisdiction_candidates};
use crate::model::{
    CompiledFiscalCatalog, FISCAL_ACTING_BASIS_NOT_CURRENT, FISCAL_AUTHORITY_BASIS_NOT_CURRENT,
    FISCAL_BOUNDARY_ACTION_LIMIT, FISCAL_CONTEXT_OUTSIDE_PACK, FISCAL_CYCLE_ARCHIVED,
    FISCAL_CYCLE_CLOSED, FISCAL_HOT_BOOKS_EXHAUSTED, FISCAL_INGRESS_BUDGET_REJECTION,
    FISCAL_INGRESS_INVALID_REJECTION, FISCAL_ISSUER_NOT_BOUND, FISCAL_OBSERVER_REMOVED,
    FISCAL_PARTITION_KEYS_EXHAUSTED, FISCAL_RECEIPT_BOUNDARY_LIMIT, FISCAL_RECEIPT_CLAIM_MISMATCH,
    FISCAL_RECEIPT_CYCLE_CLOSED, FISCAL_RECEIPT_EVIDENCE_COUNT,
    FISCAL_RECEIPT_EVIDENCE_KIND_UNAPPROVED, FISCAL_RECEIPT_EVIDENCE_NOT_CURRENT,
    FISCAL_RECEIPT_EVIDENCE_REUSED, FISCAL_RECEIPT_ID_CONFLICT, FISCAL_RECEIPT_OPERATION_REUSED,
    FISCAL_RECEIPT_QUANTITY_EXCEEDED, FISCAL_RECEIPT_REQUEST_UNKNOWN, FISCAL_RECORD_ACTION_LIMIT,
    FISCAL_RECORD_BUDGET_EXHAUSTED, FISCAL_RECORD_RING_RESERVE_BYTES, FiscalAction,
    FiscalActionDisposition, FiscalActionIssuer, FiscalActionRequest, FiscalActionTarget,
    FiscalAdoptionStage, FiscalAdoptionState, FiscalAppointmentDecision, FiscalAssessment,
    FiscalAuditFinding, FiscalBookStatus, FiscalCycleBook, FiscalCycleTotal,
    FiscalExecutionEvidence, FiscalExecutionReceipt, FiscalExecutionReceiptPacket,
    FiscalExecutionRequest, FiscalExternalOperationRef, FiscalHistoricalContext,
    FiscalHistoricalContextPacket, FiscalIssuerRole, FiscalObserverBinding, FiscalOffice,
    FiscalOfficeCycle, FiscalPartitionKey, FiscalRebinding, FiscalReceiptDisposition,
    FiscalRemission, FiscalReportQueueEntry, FiscalScopeLedger, FiscalState,
    MAX_FISCAL_ACTIONS_PER_BOUNDARY, MAX_FISCAL_ACTIONS_PER_RECORD_PER_BOUNDARY,
    MAX_FISCAL_EVIDENCE_PER_RECORD, MAX_FISCAL_EXECUTION_RECEIPTS_PER_BOOK,
    MAX_FISCAL_HOT_BOOKS_PER_LEDGER, MAX_FISCAL_HOT_CYCLES_PER_OFFICE,
    MAX_FISCAL_LEDGER_RECORD_BYTES, MAX_FISCAL_LEDGER_WRITES_PER_BOUNDARY,
    MAX_FISCAL_PARTITION_KEYS, MAX_FISCAL_RECORD_BYTES, add_adoption_counts, invalid,
    validate_identifier, validate_jurisdiction, validate_ledger, validate_observer,
    validate_office,
};
use crate::plugin::{
    FISCAL_ACTION_SETTLED_EVENT, FISCAL_ARCHIVE_COMMIT_INGRESS, FISCAL_ARCHIVE_MAINTENANCE_EVENT,
    FISCAL_EXECUTION_RECEIPTS_INGRESS, FISCAL_HISTORICAL_CONTEXT_INGRESS,
    FISCAL_INGRESS_REJECTED_EVENT, PLUGIN_NAME, holder_matches, invalid_authority, invalid_payload,
    require_current_basis, stale,
};
use crate::records::{FiscalIndexOwner, fiscal_root_reference};
use crate::working::{Touch, Working};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryProposal, CanwuError, CommandId, EntityRef,
    ErrorCode, IngressId, KnowledgeHolderRef, SimTime, SimulationView,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const RECEIPT_RECORDED_EVENT: &str = "canwu.fiscal.execution_receipt_recorded.v1";
pub(crate) const CONTEXT_CHANGED_EVENT: &str = "canwu.fiscal.historical_context_changed.v1";
const STALE_ACTION: &str = "fiscal action was stale at its settlement boundary";
const STALE_OVERWRITE: &str = "fiscal action was stale at its settlement boundary: an earlier action of the boundary changed an adoption or binding it overwrites";

/// One admitted batch: the command that carried it, the binding it was
/// issued under, and each entry with the issuer role and target admission
/// resolved for it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct AdmittedFiscalBatch {
    pub(crate) authority_binding_id: String,
    pub(crate) command: CommandId,
    pub(crate) entries: Vec<AdmittedFiscalEntry>,
}

/// One admitted entry of a batch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct AdmittedFiscalEntry {
    pub(crate) action_id: String,
    pub(crate) expected_revision: u64,
    pub(crate) action: FiscalAction,
    /// Who issued the command and in which role for this entry, as
    /// admission resolved it. Settlement re-checks the role against the
    /// records it settles on.
    pub(crate) issuer: FiscalActionIssuer,
    /// The record whose ring keeps the entry's outcome.
    pub(crate) target: FiscalActionTarget,
}

impl AdmittedFiscalEntry {
    fn request(&self, authority_binding_id: &str) -> FiscalActionRequest {
        FiscalActionRequest {
            action_id: self.action_id.clone(),
            authority_binding_id: authority_binding_id.to_owned(),
            expected_revision: self.expected_revision,
            action: self.action.clone(),
        }
    }
}

/// The payload of a receipt batch packet.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FiscalExecutionReceiptBatch {
    pub(crate) receipts: Vec<FiscalExecutionReceiptPacket>,
}

/// One admitted ingress item of this plugin, decoded in settlement order.
pub(crate) enum FiscalPacket {
    Action(Box<AdmittedFiscalBatch>),
    Receipts(IngressId, Vec<FiscalExecutionReceiptPacket>),
    HistoricalContext(IngressId, FiscalHistoricalContextPacket),
    ArchiveCommit(IngressId, Box<VerifiedFiscalArchiveCommitV1>),
    ArchiveRetention(String),
    /// A packet that settlement skips: its ingress, its packet type, and the
    /// reason.
    Refused(IngressId, &'static str, &'static str),
}

/// The adoptions and bindings that actions of one boundary have changed.
/// A later action of the same boundary may read them but not overwrite them.
#[derive(Default)]
struct BoundaryOverwrites {
    /// Keyed `scope/adoption`.
    adoptions: BTreeSet<String>,
    authority_bindings: BTreeSet<String>,
    audit_bindings: BTreeSet<String>,
    observer_bindings: BTreeSet<String>,
}

/// One event a settlement emits, with the entities it concerns.
pub(crate) struct SettledEvent {
    pub(crate) event_type: &'static str,
    pub(crate) summary: String,
    pub(crate) affected: Vec<EntityRef>,
}

/// Why a receipt or historical-context change was refused: its stable
/// reason, and a message that says more.
pub(crate) struct Refused {
    pub(crate) reason: &'static str,
    pub(crate) message: String,
}

impl Refused {
    pub(crate) fn new(reason: &'static str, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }

    fn text(&self) -> String {
        format!("{}: {}", self.message, self.reason)
    }
}

impl From<CanwuError> for Refused {
    fn from(error: CanwuError) -> Self {
        Self::new(FISCAL_INGRESS_INVALID_REJECTION, error.message)
    }
}

/// Reports a packet that settlement skips without changing anything, by its
/// ingress, a label naming the packet, and the reason the summary ends with.
pub(crate) fn refused_ingress(
    ingress: IngressId,
    packet: &str,
    reason: &str,
    affected: Vec<EntityRef>,
) -> SettledEvent {
    SettledEvent {
        event_type: FISCAL_INGRESS_REJECTED_EVENT,
        summary: format!(
            "Rejected fiscal ingress {} ({packet}): {reason}",
            ingress.get()
        ),
        affected,
    }
}

/// One settled action as its `canwu.fiscal.action_settled.v1` event records
/// it: the durable record of every outcome, applied or rejected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FiscalSettledAction {
    /// The action's target, as [`FiscalActionTarget`] displays it.
    pub target: String,
    pub action_id: String,
    pub disposition: FiscalActionDisposition,
    /// `applied`, or why the action was rejected.
    pub reason: String,
}

impl FiscalSettledAction {
    pub(crate) fn summary(&self) -> String {
        format!(
            "Settled fiscal action {} on {} as {}: {}",
            self.action_id,
            self.target,
            match self.disposition {
                FiscalActionDisposition::Applied => "applied",
                FiscalActionDisposition::Rejected => "rejected",
            },
            self.reason
        )
    }

    /// Reads the summary of a `canwu.fiscal.action_settled.v1` event.
    #[must_use]
    pub fn from_summary(summary: &str) -> Option<Self> {
        let rest = summary.strip_prefix("Settled fiscal action ")?;
        let (action_id, rest) = rest.split_once(" on ")?;
        let (target, rest) = rest.split_once(" as ")?;
        let (disposition, reason) = rest.split_once(": ")?;
        Some(Self {
            target: target.to_owned(),
            action_id: action_id.to_owned(),
            disposition: match disposition {
                "applied" => FiscalActionDisposition::Applied,
                "rejected" => FiscalActionDisposition::Rejected,
                _ => return None,
            },
            reason: reason.to_owned(),
        })
    }

    /// Every settled action an event list records, in order.
    #[must_use]
    pub fn from_events(events: &[canwu_api::SimEvent]) -> Vec<Self> {
        events
            .iter()
            .filter(|event| {
                event.kind.plugin_identity() == Some((PLUGIN_NAME, FISCAL_ACTION_SETTLED_EVENT))
            })
            .filter_map(|event| Self::from_summary(&event.summary))
            .collect()
    }
}

/// One pass over a boundary's packets: the records it has loaded and what it
/// has produced so far.
struct Settlement<'a, 'v> {
    view: &'a SimulationView<'v>,
    catalog: &'a CompiledFiscalCatalog,
    at: SimTime,
    working: Working<'a, SimulationView<'v>>,
    applied_actions: usize,
    applied_per_target: BTreeMap<FiscalActionTarget, usize>,
    written_ledgers: BTreeSet<String>,
    overwrites: BoundaryOverwrites,
    /// The action IDs the boundary has settled, by target, so that a copy
    /// of an action is skipped even when an eviction took the first copy's
    /// outcome out of the ring within the boundary.
    settled: BTreeMap<FiscalActionTarget, BTreeSet<String>>,
    events: Vec<SettledEvent>,
    /// The serialized bytes of each working office without its cycle
    /// bookkeeping, for the budget of an action that changes only that.
    office_base_bytes: std::cell::RefCell<BTreeMap<String, usize>>,
}

/// Settles the boundary's fiscal packets in settlement order, writing each
/// record it changes once, and queues the report work the changes owe.
///
/// No packet fails the boundary. An action is applied or rejected as its
/// own outcome; a receipt, a historical-context change, or an archive packet
/// settles or is refused with an event and changes nothing.
pub(crate) fn settle_fiscal_packets(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    catalog: &CompiledFiscalCatalog,
    packets: &[FiscalPacket],
) -> Result<BoundaryProposal, CanwuError> {
    let Some(working) = Working::new(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let mut settlement = Settlement {
        view,
        catalog,
        at: context.at,
        working,
        applied_actions: 0,
        applied_per_target: BTreeMap::new(),
        written_ledgers: BTreeSet::new(),
        overwrites: BoundaryOverwrites::default(),
        settled: BTreeMap::new(),
        events: Vec::new(),
        office_base_bytes: std::cell::RefCell::new(BTreeMap::new()),
    };
    for packet in packets {
        match packet {
            FiscalPacket::ArchiveCommit(ingress, commit) => {
                settlement.settle_archive_commit(*ingress, commit)?;
            }
            FiscalPacket::ArchiveRetention(handle_id) => {
                settlement.working.ensure_archive()?;
                if settlement
                    .working
                    .state
                    .acknowledge_fiscal_archive_retention(handle_id)
                {
                    settlement
                        .working
                        .touch(crate::records::fiscal_archive_reference().into_untyped());
                }
            }
            FiscalPacket::Action(batch) => {
                for entry in &batch.entries {
                    settlement.settle_entry(batch, entry)?;
                }
            }
            FiscalPacket::Receipts(ingress, receipts) => {
                for receipt in receipts {
                    settlement.settle_receipt(*ingress, receipt)?;
                }
            }
            FiscalPacket::HistoricalContext(ingress, packet) => {
                settlement.settle_context(*ingress, packet)?;
            }
            FiscalPacket::Refused(ingress, packet_type, reason) => {
                settlement
                    .events
                    .push(refused_ingress(*ingress, packet_type, reason, Vec::new()));
            }
        }
    }
    let Settlement {
        mut working,
        events,
        at,
        ..
    } = settlement;
    working.queue_changed_books(at)?;
    let mut directives = working.directives(catalog, "Settled fiscal ingress")?;
    directives.extend(events.into_iter().map(|event| BoundaryDirective::Emit {
        event_type: event.event_type.to_owned(),
        summary: event.summary,
        affected: event.affected,
    }));
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

impl Settlement<'_, '_> {
    /// Settles one archive commit on a copy of the archive head, the ledgers
    /// of the books it names, and their offices' cycle bookkeeping.
    fn settle_archive_commit(
        &mut self,
        ingress: IngressId,
        commit: &VerifiedFiscalArchiveCommitV1,
    ) -> Result<(), CanwuError> {
        self.working.ensure_archive()?;
        let mut touch = Touch {
            archive: true,
            ..Touch::default()
        };
        for book in &commit.selected_books {
            if validate_identifier(&book.scope_binding_id, "scope binding").is_err() {
                continue;
            }
            if self.working.ensure_ledger(&book.scope_binding_id)? {
                touch.ledgers.insert(book.scope_binding_id.clone());
                let office = self.working.state.ledgers[&book.scope_binding_id]
                    .office
                    .clone();
                if self.working.ensure_office_key(&office)? {
                    touch.cycle_offices.insert(office);
                }
            }
        }
        let mut copy = self.working.copy(&touch);
        match copy.apply_fiscal_archive_commit(commit, self.at) {
            Ok(FiscalArchiveSettlement::Duplicate { handle_id }) => {
                self.events.push(SettledEvent {
                    event_type: FISCAL_ARCHIVE_MAINTENANCE_EVENT,
                    summary: format!(
                        "Settled fiscal archive commit for handle {handle_id} as stale duplicate: nothing recorded"
                    ),
                    affected: Vec::new(),
                });
            }
            Ok(FiscalArchiveSettlement::Recorded(receipt)) => {
                self.working.absorb(&touch, copy)?;
                self.events.push(SettledEvent {
                    event_type: FISCAL_ARCHIVE_MAINTENANCE_EVENT,
                    summary: format!(
                        "Settled fiscal archive commit {} as {}: {} books, {} institution totals, {} receipts",
                        receipt.id,
                        if receipt.disposition == FiscalArchiveRetentionPhaseV1::Committed {
                            "applied"
                        } else {
                            "stale"
                        },
                        receipt.archived_books,
                        receipt.archived_totals,
                        receipt.archived_receipts
                    ),
                    affected: Vec::new(),
                });
            }
            Err(error) => self.events.push(refused_ingress(
                ingress,
                FISCAL_ARCHIVE_COMMIT_INGRESS,
                &format!("{}: {FISCAL_INGRESS_INVALID_REJECTION}", error.message),
                Vec::new(),
            )),
        }
        Ok(())
    }

    /// Settles one entry of a batch: records its outcome on its target. An
    /// entry whose action ID already has an outcome for its target, in the
    /// ring or settled earlier in this boundary, is skipped without one.
    fn settle_entry(
        &mut self,
        batch: &AdmittedFiscalBatch,
        entry: &AdmittedFiscalEntry,
    ) -> Result<(), CanwuError> {
        let target = &entry.target;
        // Records are never deleted, and admission found the target.
        if !self.working.ensure_target(target)? {
            return Err(invalid("an admitted fiscal action's target is missing"));
        }
        if self
            .working
            .state
            .recent_outcome(target, &entry.action_id)
            .is_some()
            || self
                .settled
                .get(target)
                .is_some_and(|settled| settled.contains(&entry.action_id))
        {
            return Ok(());
        }
        let request = entry.request(&batch.authority_binding_id);
        let entry_revision = self.working.entry_revisions.get(target).copied();
        let verdict = if Some(entry.expected_revision) == entry_revision {
            self.load_for(&request, &entry.issuer, target)?;
            settlement_authority(self.view, &self.working.state, &request, &entry.issuer)
                .and_then(|()| self.apply(&request, entry))
        } else {
            Err(stale(STALE_ACTION))
        };
        let disposition = if verdict.is_ok() {
            FiscalActionDisposition::Applied
        } else {
            FiscalActionDisposition::Rejected
        };
        let affected = if verdict.is_ok() {
            rebound_entities(&self.working.state, &request)
        } else {
            Vec::new()
        };
        self.working
            .state
            .record_outcome(target, &entry.action_id, disposition)?;
        self.working.touch_target(target);
        if let FiscalActionTarget::Ledger(scope) = target {
            self.written_ledgers.insert(scope.clone());
        }
        self.settled
            .entry(target.clone())
            .or_default()
            .insert(entry.action_id.clone());
        let reason = match &verdict {
            Ok(()) => "applied".to_owned(),
            Err(error) => error.to_string(),
        };
        self.events.push(SettledEvent {
            event_type: FISCAL_ACTION_SETTLED_EVENT,
            summary: FiscalSettledAction {
                target: target.to_string(),
                action_id: entry.action_id.clone(),
                disposition,
                reason,
            }
            .summary(),
            affected,
        });
        Ok(())
    }

    /// Loads every record an action reads besides its target: the office of
    /// its issuer's binding, the scope's office or jurisdiction, a
    /// transition's region and its scopes, and the offices the binding
    /// directory names for the bindings and proposals an appointment action
    /// cites or creates.
    #[allow(clippy::too_many_lines)]
    fn load_for(
        &mut self,
        request: &FiscalActionRequest,
        issuer: &FiscalActionIssuer,
        target: &FiscalActionTarget,
    ) -> Result<(), CanwuError> {
        let working = &mut self.working;
        working.ensure_binding_office(&issuer.binding_id)?;
        working.ensure_target(target)?;
        match &request.action {
            FiscalAction::ChangeAdoption {
                scope_binding_id, ..
            }
            | FiscalAction::OpenAssessment {
                scope_binding_id, ..
            }
            | FiscalAction::RecordAudit {
                scope_binding_id, ..
            } => {
                if working.ensure_ledger(scope_binding_id)? {
                    let ledger = &working.state.ledgers[scope_binding_id];
                    let (office, region) =
                        (ledger.office.clone(), ledger.scope.jurisdiction_id.clone());
                    working.ensure_office_key(&office)?;
                    working.ensure_jurisdiction(&region)?;
                }
            }
            FiscalAction::GrantRemission {
                scope_binding_id, ..
            }
            | FiscalAction::AuthorizeExecution {
                scope_binding_id, ..
            }
            | FiscalAction::CloseAccountingCycle {
                scope_binding_id, ..
            } => {
                working.ensure_ledger(scope_binding_id)?;
            }
            FiscalAction::ApplyTransition {
                target_scope_bindings,
                ..
            } => {
                if let FiscalActionTarget::Jurisdiction(region) = target {
                    for scope in
                        working.index_scopes(&FiscalIndexOwner::Jurisdiction(region.clone()))?
                    {
                        working.ensure_ledger(&scope)?;
                    }
                }
                for scope in target_scope_bindings.values() {
                    working.ensure_ledger(scope)?;
                }
            }
            FiscalAction::RebindAuthority {
                authority_binding_id,
                ..
            } => {
                working.ensure_binding_office(authority_binding_id)?;
            }
            FiscalAction::RebindAudit {
                audit_binding_id, ..
            } => {
                working.ensure_binding_office(audit_binding_id)?;
            }
            FiscalAction::NominateAuditor {
                nomination_id,
                audit_binding_id,
                institution,
                ..
            } => {
                working.ensure_proposal_office(nomination_id)?;
                working.ensure_binding_office(audit_binding_id)?;
                working.ensure_office(institution)?;
            }
            FiscalAction::PetitionAuditorRemoval {
                petition_id,
                audit_binding_id,
                ..
            } => {
                working.ensure_proposal_office(petition_id)?;
                working.ensure_binding_office(audit_binding_id)?;
            }
            FiscalAction::DecideAuditAppointment {
                audit_binding_id,
                decision,
            } => {
                working.ensure_binding_office(audit_binding_id)?;
                match decision {
                    FiscalAppointmentDecision::AcceptNomination { nomination_id: id }
                    | FiscalAppointmentDecision::AcceptPetition { petition_id: id }
                    | FiscalAppointmentDecision::Dismiss { proposal_id: id } => {
                        working.ensure_proposal_office(id)?;
                    }
                    FiscalAppointmentDecision::Appoint { institution, .. } => {
                        working.ensure_office(institution)?;
                    }
                    FiscalAppointmentDecision::Remove => {}
                }
            }
            FiscalAction::RebindObserver {
                observer_binding_id,
                ..
            }
            | FiscalAction::RemoveObserver {
                observer_binding_id,
            } => {
                working.ensure_observer(observer_binding_id)?;
            }
        }
        // The policy that governs the audited institution names its
        // appointer, whose office an appointer's decision reads.
        if is_appointment_action(&request.action) {
            let appointers: Vec<EntityRef> = working
                .state
                .offices
                .values()
                .filter_map(|office| office.appointment_policy.as_ref())
                .map(|policy| policy.appointer.clone())
                .collect();
            for appointer in appointers {
                working.ensure_office(&appointer)?;
            }
        }
        Ok(())
    }

    /// The records an action changes, for its copy.
    fn touch_for(&self, request: &FiscalActionRequest, target: &FiscalActionTarget) -> Touch {
        let state = &self.working.state;
        let mut touch = Touch::default();
        let scope_office =
            |scope: &str| state.ledgers.get(scope).map(|ledger| ledger.office.clone());
        match &request.action {
            FiscalAction::ChangeAdoption {
                scope_binding_id, ..
            } => {
                touch.ledgers.insert(scope_binding_id.clone());
                if let Some(ledger) = state.ledgers.get(scope_binding_id) {
                    touch
                        .jurisdictions
                        .insert(ledger.scope.jurisdiction_id.clone());
                }
            }
            FiscalAction::OpenAssessment {
                scope_binding_id, ..
            } => {
                touch.ledgers.insert(scope_binding_id.clone());
                touch.cycle_offices.extend(scope_office(scope_binding_id));
            }
            FiscalAction::RecordAudit {
                scope_binding_id, ..
            } => {
                touch.ledgers.insert(scope_binding_id.clone());
                touch.offices.extend(scope_office(scope_binding_id));
            }
            FiscalAction::GrantRemission {
                scope_binding_id, ..
            }
            | FiscalAction::AuthorizeExecution {
                scope_binding_id, ..
            }
            | FiscalAction::CloseAccountingCycle {
                scope_binding_id, ..
            } => {
                touch.ledgers.insert(scope_binding_id.clone());
            }
            FiscalAction::ApplyTransition { .. } => {
                if let FiscalActionTarget::Jurisdiction(region) = target {
                    touch.jurisdictions.insert(region.clone());
                    touch.ledgers.extend(
                        state
                            .ledgers
                            .values()
                            .filter(|ledger| ledger.scope.jurisdiction_id == *region)
                            .map(|ledger| ledger.scope.id.clone()),
                    );
                }
            }
            FiscalAction::RebindObserver {
                observer_binding_id,
                ..
            }
            | FiscalAction::RemoveObserver {
                observer_binding_id,
            } => {
                touch.observers.insert(observer_binding_id.clone());
            }
            FiscalAction::RebindAuthority { .. }
            | FiscalAction::RebindAudit { .. }
            | FiscalAction::NominateAuditor { .. }
            | FiscalAction::PetitionAuditorRemoval { .. }
            | FiscalAction::DecideAuditAppointment { .. } => {
                // Every office the action loaded may be one it changes.
                touch.offices.extend(state.offices.keys().cloned());
            }
        }
        touch
    }

    /// Applies a current, authorized action to a copy of the records it
    /// changes, unless the boundary has already applied as many as it may,
    /// and takes the copy back only if every changed record validates and
    /// fits its budget, the action overwrites nothing an earlier action of
    /// this boundary changed, and the ledgers it writes fit the boundary.
    /// Only an action that gets this far uses the boundary's limits.
    #[allow(clippy::too_many_lines)]
    fn apply(
        &mut self,
        request: &FiscalActionRequest,
        entry: &AdmittedFiscalEntry,
    ) -> Result<(), CanwuError> {
        let target = &entry.target;
        if self.applied_actions >= MAX_FISCAL_ACTIONS_PER_BOUNDARY {
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                FISCAL_BOUNDARY_ACTION_LIMIT,
            ));
        }
        if self
            .applied_per_target
            .get(target)
            .is_some_and(|count| *count >= MAX_FISCAL_ACTIONS_PER_RECORD_PER_BOUNDARY)
        {
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                FISCAL_RECORD_ACTION_LIMIT,
            ));
        }
        let touch = self.touch_for(request, target);
        let mut copy = self.working.copy(&touch);
        apply_action(&mut copy, self.catalog, request, &entry.issuer, self.at)?;
        let before = &self.working.state;
        // Ledgers whose adoptions changed: their counts move in their
        // jurisdiction, and a transition advances each one's revision, since
        // what an issuer of an action on the scope decided against changed.
        let mut adoption_changes = Vec::new();
        for (scope, ledger) in &copy.ledgers {
            let previous = &before.ledgers[scope];
            if previous.adoptions != ledger.adoptions {
                adoption_changes.push(scope.clone());
            }
        }
        for scope in &adoption_changes {
            let previous = before.ledgers[scope]
                .adoptions
                .values()
                .cloned()
                .collect::<Vec<_>>();
            let ledger = copy
                .ledgers
                .get_mut(scope)
                .ok_or_else(|| invalid("fiscal ledger copy is unavailable"))?;
            if matches!(request.action, FiscalAction::ApplyTransition { .. }) {
                ledger.revision = ledger
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| invalid("fiscal record revision overflowed"))?;
            }
            let region = ledger.scope.jurisdiction_id.clone();
            let current = ledger.adoptions.values().cloned().collect::<Vec<_>>();
            let jurisdiction = copy
                .jurisdictions
                .get_mut(&region)
                .ok_or_else(|| invalid("fiscal jurisdiction copy is unavailable"))?;
            add_adoption_counts(&mut jurisdiction.adoption_counts, &previous, -1);
            add_adoption_counts(&mut jurisdiction.adoption_counts, &current, 1);
        }
        let context = copy.root.historical_context.clone();
        for jurisdiction in copy.jurisdictions.values_mut() {
            jurisdiction.transition_candidates =
                jurisdiction_candidates(jurisdiction, self.catalog, &context, self.at);
        }
        self.check_copy(&touch, &copy)?;
        let overwrites = changed_overwrites(&self.working.state, &touch, &copy);
        if !overwrites.adoptions.is_disjoint(&self.overwrites.adoptions)
            || !overwrites
                .authority_bindings
                .is_disjoint(&self.overwrites.authority_bindings)
            || !overwrites
                .audit_bindings
                .is_disjoint(&self.overwrites.audit_bindings)
            || !overwrites
                .observer_bindings
                .is_disjoint(&self.overwrites.observer_bindings)
        {
            return Err(stale(STALE_OVERWRITE));
        }
        let written: BTreeSet<String> = copy
            .ledgers
            .iter()
            .filter(|(scope, ledger)| self.working.state.ledgers.get(*scope) != Some(*ledger))
            .map(|(scope, _)| scope.clone())
            .chain(match target {
                FiscalActionTarget::Ledger(scope) => Some(scope.clone()),
                _ => None,
            })
            .filter(|scope| !self.written_ledgers.contains(scope))
            .collect();
        if self.written_ledgers.len() + written.len() > MAX_FISCAL_LEDGER_WRITES_PER_BOUNDARY {
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                FISCAL_BOUNDARY_ACTION_LIMIT,
            ));
        }
        self.overwrites.adoptions.extend(overwrites.adoptions);
        self.overwrites
            .authority_bindings
            .extend(overwrites.authority_bindings);
        self.overwrites
            .audit_bindings
            .extend(overwrites.audit_bindings);
        self.overwrites
            .observer_bindings
            .extend(overwrites.observer_bindings);
        self.written_ledgers.extend(written);
        self.working.absorb(&touch, copy)?;
        for key in &touch.offices {
            self.office_base_bytes.get_mut().remove(key);
        }
        self.applied_actions += 1;
        *self.applied_per_target.entry(target.clone()).or_default() += 1;
        Ok(())
    }

    /// The bytes an office is held to once an action changed only its cycle
    /// bookkeeping (`office`, copied with nothing else): its other fields as
    /// the working office has them, the new bookkeeping, the room the
    /// archive will need for it, and the field's name.
    fn cycle_office_bytes(
        &self,
        key: &str,
        current: &FiscalOffice,
        office: &FiscalOffice,
    ) -> Result<usize, CanwuError> {
        let cached = self.office_base_bytes.borrow().get(key).copied();
        let base = if let Some(base) = cached {
            base
        } else {
            let base = json_len(current)?.saturating_sub(json_len(&current.cycles)?);
            self.office_base_bytes
                .borrow_mut()
                .insert(key.to_owned(), base);
            base
        };
        Ok(base
            + OFFICE_CYCLES_FIELD_BYTES
            + json_len(&office.cycles)?
            + office_archive_reserve(&office.cycles)?)
    }

    /// Checks every record a copy changed: it validates on its own, fits its
    /// byte budget with room for its outcome ring, and a binding or proposal
    /// ID an office gained is held by no other office.
    #[allow(clippy::too_many_lines)]
    fn check_copy(&self, touch: &Touch, copy: &FiscalState) -> Result<(), CanwuError> {
        let state = &self.working.state;
        let budget = |bytes: usize, limit: usize| {
            if bytes + FISCAL_RECORD_RING_RESERVE_BYTES > limit {
                Err(CanwuError::new(
                    ErrorCode::ValueOutOfRange,
                    FISCAL_RECORD_BUDGET_EXHAUSTED,
                ))
            } else {
                Ok(())
            }
        };
        for (scope, ledger) in &copy.ledgers {
            if state.ledgers.get(scope) != Some(ledger) {
                validate_ledger(ledger, self.catalog)?;
                budget(ledger_bytes(ledger)?, MAX_FISCAL_LEDGER_RECORD_BYTES)?;
            }
        }
        for (key, office) in &copy.offices {
            let Some(current) = state.offices.get(key) else {
                continue;
            };
            if touch.offices.contains(key) {
                if current == office {
                    continue;
                }
                validate_office(office)?;
                budget(office_budget_bytes(office)?, MAX_FISCAL_RECORD_BYTES)?;
                let (bindings, proposals) = (
                    crate::model::office_binding_ids(office)
                        .filter(|id| {
                            crate::model::office_binding_ids(current).all(|held| held != *id)
                        })
                        .cloned()
                        .collect::<Vec<_>>(),
                    office
                        .audit_proposals
                        .keys()
                        .filter(|id| !current.audit_proposals.contains_key(*id))
                        .cloned()
                        .collect::<Vec<_>>(),
                );
                for id in bindings {
                    if self.working.binding_office(&id)?.is_some_and(|holder| {
                        holder != *key
                            && state.offices.get(&holder).is_some_and(|holder| {
                                crate::model::office_binding_ids(holder).any(|held| *held == id)
                            })
                    }) {
                        return Err(invalid_payload(
                            "a fiscal binding ID is held by another institution's office",
                        ));
                    }
                }
                for id in proposals {
                    if self.working.proposal_office(&id)?.is_some_and(|holder| {
                        holder != *key
                            && state
                                .offices
                                .get(&holder)
                                .is_some_and(|holder| holder.audit_proposals.contains_key(&id))
                    }) {
                        return Err(CanwuError::new(
                            ErrorCode::IdempotencyConflict,
                            "fiscal nomination or petition identity already exists",
                        ));
                    }
                }
            } else if current.cycles != office.cycles {
                if office.cycles.len() > MAX_FISCAL_HOT_CYCLES_PER_OFFICE {
                    return Err(CanwuError::new(
                        ErrorCode::ValueOutOfRange,
                        FISCAL_HOT_BOOKS_EXHAUSTED,
                    ));
                }
                if office.cycles.values().any(|entry| {
                    entry
                        .partition_keys
                        .values()
                        .any(|keys| keys.len() > MAX_FISCAL_PARTITION_KEYS)
                }) {
                    return Err(CanwuError::new(
                        ErrorCode::ValueOutOfRange,
                        FISCAL_PARTITION_KEYS_EXHAUSTED,
                    ));
                }
                budget(
                    self.cycle_office_bytes(key, current, office)?,
                    MAX_FISCAL_RECORD_BYTES,
                )?;
            }
        }
        for (region, jurisdiction) in &copy.jurisdictions {
            if state.jurisdictions.get(region) != Some(jurisdiction) {
                validate_jurisdiction(jurisdiction, self.catalog)?;
                budget(json_len(jurisdiction)?, MAX_FISCAL_RECORD_BYTES)?;
            }
        }
        for (id, observer) in &copy.observers {
            if state.observers.get(id) != Some(observer) {
                validate_observer(observer)?;
            }
        }
        Ok(())
    }

    /// Settles one receipt of a receipt batch in the book of its request, on
    /// a copy of its ledger, or refuses it with an event that changes
    /// nothing.
    fn settle_receipt(
        &mut self,
        ingress: IngressId,
        packet: &FiscalExecutionReceiptPacket,
    ) -> Result<(), CanwuError> {
        let label = format!("{FISCAL_EXECUTION_RECEIPTS_INGRESS} {}", packet.receipt_id);
        let scope = &packet.scope_binding_id;
        let exists = validate_identifier(scope, "scope binding").is_ok()
            && self.working.ensure_ledger(scope)?;
        let affected: Vec<EntityRef> = self
            .working
            .state
            .ledgers
            .get(scope)
            .and_then(|ledger| registered_entity(&ledger.scope.institution))
            .into_iter()
            .collect();
        let refuse = |settlement: &mut Self, refused: Refused| {
            settlement.events.push(refused_ingress(
                ingress,
                &label,
                &refused.text(),
                affected.clone(),
            ));
        };
        if !exists {
            refuse(
                self,
                Refused::new(
                    FISCAL_RECEIPT_REQUEST_UNKNOWN,
                    "fiscal execution receipt names no hot execution request of its scope",
                ),
            );
            return Ok(());
        }
        if !self.written_ledgers.contains(scope)
            && self.written_ledgers.len() >= MAX_FISCAL_LEDGER_WRITES_PER_BOUNDARY
        {
            refuse(
                self,
                Refused::new(
                    FISCAL_RECEIPT_BOUNDARY_LIMIT,
                    "the boundary has written as many fiscal ledgers as it may",
                ),
            );
            return Ok(());
        }
        let touch = Touch {
            ledgers: BTreeSet::from([scope.clone()]),
            ..Touch::default()
        };
        let mut copy = self.working.copy(&touch);
        match settle_execution_receipt(
            self.view,
            &mut copy,
            packet.clone(),
            ingress,
            self.at,
            self.catalog,
        ) {
            Ok(false) => {}
            Ok(true) => {
                let bytes = ledger_bytes(&copy.ledgers[scope])?;
                if bytes + FISCAL_RECORD_RING_RESERVE_BYTES > MAX_FISCAL_LEDGER_RECORD_BYTES {
                    refuse(
                        self,
                        Refused::new(
                            FISCAL_INGRESS_BUDGET_REJECTION,
                            "the receipt would put its ledger past its byte budget",
                        ),
                    );
                    return Ok(());
                }
                self.working.absorb(&touch, copy)?;
                self.written_ledgers.insert(scope.clone());
                self.events.push(SettledEvent {
                    event_type: RECEIPT_RECORDED_EVENT,
                    summary: format!(
                        "Recorded external fiscal execution evidence {}",
                        packet.receipt_id
                    ),
                    affected,
                });
            }
            Err(refused) => refuse(self, refused),
        }
        Ok(())
    }

    /// Settles one historical-context change: the root takes the new
    /// context, and every jurisdiction whose candidates it can change takes
    /// them, or the change is refused with an event that changes nothing.
    fn settle_context(
        &mut self,
        ingress: IngressId,
        packet: &FiscalHistoricalContextPacket,
    ) -> Result<(), CanwuError> {
        let refuse = |settlement: &mut Self, refused: Refused| {
            settlement.events.push(refused_ingress(
                ingress,
                FISCAL_HISTORICAL_CONTEXT_INGRESS,
                &refused.text(),
                Vec::new(),
            ));
        };
        if !self.catalog.historical_scope.contains(packet.year) {
            refuse(
                self,
                Refused::new(
                    FISCAL_CONTEXT_OUTSIDE_PACK,
                    "fiscal historical context lies outside the pack",
                ),
            );
            return Ok(());
        }
        let before = self.working.state.root.historical_context.clone();
        let mut root = self.working.state.root.clone();
        root.historical_context = FiscalHistoricalContext {
            year: packet.year,
            mode: packet.mode,
            updated_at: self.at,
        };
        root.context_revision = root
            .context_revision
            .checked_add(1)
            .ok_or_else(|| invalid("fiscal context revision overflowed"))?;
        if let Err(error) = crate::model::validate_root(&root, self.catalog) {
            refuse(self, error.into());
            return Ok(());
        }
        let mut changed = Vec::new();
        for region in context_regions(self.catalog, &before, &root.historical_context) {
            if !self.working.ensure_jurisdiction(&region)? {
                continue;
            }
            let mut jurisdiction = self.working.state.jurisdictions[&region].clone();
            jurisdiction.transition_candidates = jurisdiction_candidates(
                &jurisdiction,
                self.catalog,
                &root.historical_context,
                self.at,
            );
            if jurisdiction != self.working.state.jurisdictions[&region] {
                if json_len(&jurisdiction)? + FISCAL_RECORD_RING_RESERVE_BYTES
                    > MAX_FISCAL_RECORD_BYTES
                {
                    refuse(
                        self,
                        Refused::new(
                            FISCAL_INGRESS_BUDGET_REJECTION,
                            "the context change would put a jurisdiction past its byte budget",
                        ),
                    );
                    return Ok(());
                }
                changed.push(jurisdiction);
            }
        }
        self.working.state.root = root;
        self.working.touch(fiscal_root_reference().into_untyped());
        for jurisdiction in changed {
            let region = jurisdiction.region_id.clone();
            self.working
                .state
                .jurisdictions
                .insert(region.clone(), jurisdiction);
            self.working
                .touch(crate::records::fiscal_jurisdiction_reference(&region).into_untyped());
        }
        self.events.push(SettledEvent {
            event_type: CONTEXT_CHANGED_EVENT,
            summary: "Changed the explicit fiscal historical context".to_owned(),
            affected: Vec::new(),
        });
        Ok(())
    }
}

/// The serialized size of a ledger record's payload: the ledger and its
/// evidence continuation, measured exactly.
fn ledger_bytes(ledger: &FiscalScopeLedger) -> Result<usize, CanwuError> {
    crate::records::ledger_payload_bytes(ledger)
}

/// The serialized size of a value, without keeping the bytes.
pub(crate) fn json_len(value: &impl Serialize) -> Result<usize, CanwuError> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value)
        .map_err(|error| invalid(format!("fiscal record could not be sized: {error}")))?;
    Ok(counter.0)
}

/// Bytes the name of an office's `cycles` field and its separators take.
const OFFICE_CYCLES_FIELD_BYTES: usize = 16;

/// The bytes one hot cycle's bookkeeping takes at most while it stays hot:
/// the counts at their largest, and a total of the largest amounts for every
/// partition key the cycle uses (and for any total it holds of another key).
/// Archive commits only lower the hot-book count, raise the archived-book
/// count, and merge totals of the cycle's partition keys, which stay until
/// the cycle leaves, so the bookkeeping never passes this.
fn largest_cycle_bytes(entry: &FiscalOfficeCycle) -> Result<usize, CanwuError> {
    let mut keys = BTreeSet::new();
    for (mechanism, partition) in &entry.partition_keys {
        for key in partition {
            keys.insert((*mechanism, key.unit.as_str(), key.payment_form));
        }
    }
    for total in &entry.archived_totals {
        keys.insert((total.mechanism, total.unit.as_str(), total.payment_form));
    }
    json_len(&FiscalOfficeCycle {
        hot_books: u32::MAX,
        partition_keys: entry.partition_keys.clone(),
        archived_books: u32::MAX,
        archived_totals: keys
            .into_iter()
            .map(|(mechanism, unit, payment_form)| FiscalCycleTotal {
                mechanism,
                unit: unit.to_owned(),
                payment_form,
                assessed: u64::MAX,
                remission_granted: u64::MAX,
                collected: u64::MAX,
                remitted: u64::MAX,
                disbursed: u64::MAX,
                reserved: u64::MAX,
                returned: u64::MAX,
                outstanding: u64::MAX,
            })
            .collect(),
    })
}

/// The room an office keeps, beside its bytes, for what archive commits
/// will write into its hot cycles: for each cycle, its largest bookkeeping
/// (see [`largest_cycle_bytes`]) less what it takes now. A commit moves
/// bytes from this room into the record, never past it.
pub(crate) fn office_archive_reserve(
    cycles: &BTreeMap<String, FiscalOfficeCycle>,
) -> Result<usize, CanwuError> {
    let mut reserve = 0;
    for entry in cycles.values() {
        reserve += largest_cycle_bytes(entry)?.saturating_sub(json_len(entry)?);
    }
    Ok(reserve)
}

/// What an office's record is held to against its byte budget: its bytes,
/// and the room its hot cycles keep for archive commits.
pub(crate) fn office_budget_bytes(office: &FiscalOffice) -> Result<usize, CanwuError> {
    Ok(json_len(office)? + office_archive_reserve(&office.cycles)?)
}

/// The adoptions, authority and audit bindings, and observer bindings that a
/// copy changed against the working set.
fn changed_overwrites(
    state: &FiscalState,
    touch: &Touch,
    copy: &FiscalState,
) -> BoundaryOverwrites {
    let mut changes = BoundaryOverwrites::default();
    for (scope, ledger) in &copy.ledgers {
        let previous = &state.ledgers[scope];
        changes.adoptions.extend(
            changed_keys(&previous.adoptions, &ledger.adoptions)
                .into_iter()
                .map(|adoption| format!("{scope}/{adoption}")),
        );
    }
    // An office copied with its cycle bookkeeping only has no bindings to
    // compare.
    for (key, office) in copy
        .offices
        .iter()
        .filter(|(key, _)| touch.offices.contains(*key))
    {
        let previous = &state.offices[key];
        changes.authority_bindings.extend(changed_keys(
            &previous.authority_bindings,
            &office.authority_bindings,
        ));
        changes.audit_bindings.extend(changed_keys(
            &previous.audit_bindings,
            &office.audit_bindings,
        ));
    }
    for (id, observer) in &copy.observers {
        if state.observers[id].binding != observer.binding {
            changes.observer_bindings.insert(id.clone());
        }
    }
    changes
}

/// Keys whose entry was added, replaced, or removed between two maps.
fn changed_keys<T: PartialEq>(
    before: &BTreeMap<String, T>,
    after: &BTreeMap<String, T>,
) -> BTreeSet<String> {
    after
        .iter()
        .filter(|(key, value)| before.get(*key) != Some(*value))
        .map(|(key, _)| key)
        .chain(before.keys().filter(|key| !after.contains_key(*key)))
        .cloned()
        .collect()
}

/// Re-checks, against the records an action settles on, that its issuer
/// still holds the role admission resolved. A rebind settled since then may
/// have replaced the binding's actors, and the authority basis of an acting
/// actor or auditor may no longer be current. An appointment action is
/// checked against its policy again, and what it cites must still be
/// pending. The basis a rebind, nomination, or appointment installs must
/// also still be current.
pub(crate) fn settlement_authority(
    view: &SimulationView<'_>,
    state: &FiscalState,
    request: &FiscalActionRequest,
    issuer: &FiscalActionIssuer,
) -> Result<(), CanwuError> {
    let not_bound = || invalid_authority(FISCAL_ISSUER_NOT_BOUND);
    let action = &request.action;
    let appointment = is_appointment_action(action);
    // The roles admit different actions, so an item that pairs a role with an
    // action admission would never have paired is not what admission queued.
    let role_admits_action = match issuer.role {
        FiscalIssuerRole::Principal => true,
        FiscalIssuerRole::Acting => !appointment,
        FiscalIssuerRole::Institution => appointment,
        FiscalIssuerRole::Auditor => matches!(action, FiscalAction::RecordAudit { .. }),
        FiscalIssuerRole::ListedActor => is_proposal(action),
    };
    if !role_admits_action || issuer.binding_id != request.authority_binding_id {
        return Err(not_bound());
    }
    if issuer.role == FiscalIssuerRole::Auditor {
        let binding = state
            .audit_binding(&issuer.binding_id)
            .filter(|binding| {
                issuer.actor == Some(binding.auditor)
                    && issuer.authority_basis.as_ref() == Some(&binding.authority_basis)
            })
            .ok_or_else(not_bound)?;
        require_current_basis(
            view,
            &binding.authority_basis,
            FISCAL_AUTHORITY_BASIS_NOT_CURRENT,
        )?;
    } else if issuer.role == FiscalIssuerRole::ListedActor {
        // Policies do not change, so whether the person is listed is
        // checked below with the rest of the appointment.
        if issuer.actor.is_none() || state.appointment_policy(&issuer.binding_id).is_none() {
            return Err(not_bound());
        }
    } else {
        let binding = state
            .authority_binding(&issuer.binding_id)
            .ok_or_else(not_bound)?;
        match issuer.role {
            FiscalIssuerRole::Principal if binding.authorized_actor != issuer.actor => {
                return Err(not_bound());
            }
            FiscalIssuerRole::Acting => {
                let basis = issuer
                    .authority_basis
                    .as_ref()
                    .filter(|basis| binding.authority_basis.as_ref() == Some(*basis))
                    .ok_or_else(|| invalid_authority(FISCAL_ACTING_BASIS_NOT_CURRENT))?;
                if issuer.actor.is_none() || binding.acting_actor != issuer.actor {
                    return Err(not_bound());
                }
                require_current_basis(view, basis, FISCAL_ACTING_BASIS_NOT_CURRENT)?;
            }
            // The institution role names no person. On the running binding
            // the same decision holds it, or the principal role once the
            // binding names no authorized actor; either admits a rebind.
            FiscalIssuerRole::Institution if issuer.actor.is_some() => {
                return Err(not_bound());
            }
            FiscalIssuerRole::Principal
            | FiscalIssuerRole::Institution
            | FiscalIssuerRole::Auditor
            | FiscalIssuerRole::ListedActor => {}
        }
    }
    check_appointment(state, request, issuer)?;
    match installed_basis(state, action) {
        Some(basis) => require_current_basis(view, basis, FISCAL_AUTHORITY_BASIS_NOT_CURRENT),
        None => Ok(()),
    }
}

/// An entity an event may name: one of the engine's entity registry, which
/// never loses an identity. The engine fails a boundary whose event names an
/// entity that does not exist, and the plugin cannot tell whether a domain
/// record that was an institution when a command was admitted still exists
/// when its action settles, so a domain-record institution is left out.
pub(crate) fn registered_entity(entity: &EntityRef) -> Option<EntityRef> {
    (!matches!(entity, EntityRef::Domain(_))).then(|| entity.clone())
}

/// The persons and governments a rebind, nomination, direct appointment, or
/// observer re-pointing names.
pub(crate) fn named_entities(action: &FiscalAction) -> Vec<EntityRef> {
    match action {
        FiscalAction::RebindAuthority {
            authorized_actor,
            acting_actor,
            ..
        } => authorized_actor
            .iter()
            .chain(acting_actor)
            .copied()
            .map(EntityRef::Person)
            .collect(),
        FiscalAction::RebindAudit { auditor, .. } => {
            auditor.iter().copied().map(EntityRef::Person).collect()
        }
        FiscalAction::NominateAuditor { candidate, .. } => vec![EntityRef::Person(*candidate)],
        FiscalAction::DecideAuditAppointment {
            decision: FiscalAppointmentDecision::Appoint { auditor, .. },
            ..
        } => vec![EntityRef::Person(*auditor)],
        FiscalAction::RebindObserver {
            knowledge_holder, ..
        } => match knowledge_holder {
            KnowledgeHolderRef::Person(person) => vec![EntityRef::Person(*person)],
            KnowledgeHolderRef::Entity(entity) => vec![entity.clone()],
        },
        _ => Vec::new(),
    }
}

/// The institution, persons, and governments an applied rebind, appointment,
/// or observer action concerns, and nothing for any other action. The engine
/// checks the affected entities of an emitted event against its entity
/// registry when the boundary commits, so such an action can never commit a
/// binding to an entity that does not exist.
fn rebound_entities(state: &FiscalState, request: &FiscalActionRequest) -> Vec<EntityRef> {
    if !is_appointment_action(&request.action) {
        return Vec::new();
    }
    let mut affected: Vec<_> = state
        .authority_binding(&request.authority_binding_id)
        .and_then(|binding| registered_entity(&binding.institution))
        .into_iter()
        .chain(named_entities(&request.action))
        .collect();
    affected.sort();
    affected.dedup();
    affected
}

struct ValidatedExecutionEvidence {
    quantity: u64,
    disposition: FiscalReceiptDisposition,
    external_operations: BTreeSet<FiscalExternalOperationRef>,
}

/// Settles one execution receipt in the book of its request, or refuses it
/// with its reason. A receipt whose ID a hot receipt of the scope already has
/// with the same request and evidence is a resend and changes nothing. A
/// recorded receipt advances its ledger's revision and its book's.
fn settle_execution_receipt(
    view: &SimulationView<'_>,
    state: &mut FiscalState,
    packet: FiscalExecutionReceiptPacket,
    ingress_id: IngressId,
    at: SimTime,
    catalog: &CompiledFiscalCatalog,
) -> Result<bool, Refused> {
    let unknown = || {
        Refused::new(
            FISCAL_RECEIPT_REQUEST_UNKNOWN,
            "fiscal execution receipt names no hot execution request of its scope",
        )
    };
    let ledger = state.ledger(&packet.scope_binding_id).ok_or_else(unknown)?;
    let book = ledger
        .book_of_request(&packet.request_id)
        .ok_or_else(unknown)?;
    // A resent receipt cites the same exact versions, so it would derive the
    // same content. Settle it unchanged before its evidence is checked again,
    // since that evidence may no longer be current.
    if let Some(existing) = ledger
        .books
        .values()
        .find_map(|book| book.execution_receipts.get(&packet.receipt_id))
    {
        if existing.request_id == packet.request_id
            && existing.external_evidence == packet.external_evidence
        {
            return Ok(false);
        }
        return Err(Refused::new(
            FISCAL_RECEIPT_ID_CONFLICT,
            "fiscal receipt identity was reused with different content",
        ));
    }
    if !book.is_open() {
        return Err(Refused::new(
            FISCAL_RECEIPT_CYCLE_CLOSED,
            "fiscal execution receipt names a request of a closed book",
        ));
    }
    if book.execution_receipts.len() >= MAX_FISCAL_EXECUTION_RECEIPTS_PER_BOOK {
        return Err(Refused::new(
            FISCAL_INGRESS_BUDGET_REJECTION,
            "the book holds as many execution receipts as it may",
        ));
    }
    let evidence = validate_execution_evidence(
        view,
        &state.root.execution_evidence_kinds,
        ledger,
        book,
        &packet,
    )?;
    let cycle = book.accounting_cycle_id.clone();
    let scope = packet.scope_binding_id.clone();
    let ValidatedExecutionEvidence {
        quantity,
        disposition,
        external_operations,
    } = evidence;
    let receipt = FiscalExecutionReceipt {
        id: packet.receipt_id.clone(),
        request_id: packet.request_id,
        quantity,
        disposition,
        external_evidence: packet.external_evidence,
        external_operations,
        accepted_ingress: ingress_id,
        observed_at: at,
    };
    let ledger = state.ledger_mut(&scope)?;
    let book = ledger
        .books
        .get_mut(&cycle)
        .ok_or_else(|| invalid("fiscal accounting cycle book is unavailable"))?;
    book.execution_receipts
        .insert(packet.receipt_id.clone(), receipt);
    book.touch()?;
    ledger.revision = ledger
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("fiscal record revision overflowed"))?;
    // A receipt changes one book of one ledger, so that ledger is what can
    // break; the copy is dropped when it does.
    validate_ledger(ledger, catalog)?;
    Ok(true)
}

/// Checks a receipt's evidence against its request and its scope's hot
/// receipts, and derives what the receipt records.
#[allow(clippy::too_many_lines)]
fn validate_execution_evidence(
    view: &SimulationView<'_>,
    approved: &BTreeSet<canwu_api::DomainRecordKind>,
    ledger: &FiscalScopeLedger,
    book: &FiscalCycleBook,
    packet: &FiscalExecutionReceiptPacket,
) -> Result<ValidatedExecutionEvidence, Refused> {
    if packet.external_evidence.is_empty()
        || packet.external_evidence.len() > MAX_FISCAL_EVIDENCE_PER_RECORD
    {
        return Err(Refused::new(
            FISCAL_RECEIPT_EVIDENCE_COUNT,
            "fiscal execution receipt has an invalid external evidence count",
        ));
    }
    let request = book
        .execution_requests
        .get(&packet.request_id)
        .ok_or_else(|| {
            Refused::new(
                FISCAL_RECEIPT_REQUEST_UNKNOWN,
                "fiscal execution receipt names an unavailable execution request",
            )
        })?;
    let hot_receipts = || {
        ledger
            .books
            .values()
            .flat_map(|book| book.execution_receipts.values())
    };
    let consumed_operations: BTreeSet<_> = hot_receipts()
        .flat_map(|receipt| receipt.external_operations.iter())
        .collect();
    let consumed_evidence: BTreeSet<_> = hot_receipts()
        .flat_map(|receipt| receipt.external_evidence.iter())
        .collect();
    let mismatch = || {
        Refused::new(
            FISCAL_RECEIPT_CLAIM_MISMATCH,
            "typed fiscal execution evidence does not match its request or receipt",
        )
    };
    let mut evidenced_quantity = 0_u64;
    let mut disposition = None;
    let mut external_operations = BTreeSet::new();
    for evidence in &packet.external_evidence {
        if !approved.contains(&evidence.record.kind) {
            return Err(Refused::new(
                FISCAL_RECEIPT_EVIDENCE_KIND_UNAPPROVED,
                "fiscal execution receipt cites an unapproved external evidence kind",
            ));
        }
        if consumed_evidence.contains(evidence) {
            return Err(Refused::new(
                FISCAL_RECEIPT_EVIDENCE_REUSED,
                "one external execution result cannot settle multiple fiscal receipts",
            ));
        }
        // An earlier version's body and time can depend on what a seal kept,
        // so the receipt would settle differently in a compact run than in
        // its exact replay.
        let not_current = || {
            Refused::new(
                FISCAL_RECEIPT_EVIDENCE_NOT_CURRENT,
                "fiscal execution receipt must cite the current version of its external evidence",
            )
        };
        let (record, established_at) = view
            .replay_stable_domain_record_version(evidence)
            .map_err(|_| not_current())?
            .ok_or_else(not_current)?;
        let claim =
            FiscalExecutionEvidence::deserialize(&record.payload).map_err(|_| mismatch())?;
        let operation = FiscalExternalOperationRef {
            evidence_kind: evidence.record.kind.clone(),
            external_operation_id: claim.external_operation_id.clone(),
        };
        if consumed_operations.contains(&operation) {
            return Err(Refused::new(
                FISCAL_RECEIPT_OPERATION_REUSED,
                "external fiscal operation was already settled by another receipt",
            ));
        }
        if !external_operations.insert(operation) {
            return Err(Refused::new(
                FISCAL_RECEIPT_OPERATION_REUSED,
                "external fiscal operation was repeated within one receipt",
            ));
        }
        if claim.id != evidence.record.id
            || claim.id.trim().is_empty()
            || claim.external_operation_id.trim().is_empty()
            || claim.scope_binding_id != packet.scope_binding_id
            || claim.request_id != request.id
            || claim.unit != request.unit
            || claim.payment_form != request.payment_form
            || claim.execution_kind != request.kind
            || claim.resource != request.resource
            || claim.source != request.source
            || claim.target != request.target
            || claim.disposition.counts_as_fulfillment() == (claim.quantity == 0)
            || disposition.is_some_and(|value| value != claim.disposition)
            || established_at < request.requested_at
        {
            return Err(mismatch());
        }
        disposition = Some(claim.disposition);
        evidenced_quantity = evidenced_quantity
            .checked_add(claim.quantity)
            .ok_or_else(|| {
                Refused::new(
                    FISCAL_RECEIPT_QUANTITY_EXCEEDED,
                    "fiscal execution evidence quantity overflowed",
                )
            })?;
    }
    let disposition = disposition.ok_or_else(mismatch)?;
    let fulfilled = book
        .execution_receipts
        .values()
        .filter(|receipt| {
            receipt.request_id == request.id && receipt.disposition.counts_as_fulfillment()
        })
        .try_fold(0_u64, |total, receipt| total.checked_add(receipt.quantity));
    let fulfilled = if disposition.counts_as_fulfillment() {
        fulfilled.and_then(|total| total.checked_add(evidenced_quantity))
    } else {
        fulfilled
    };
    if evidenced_quantity > request.quantity
        || fulfilled.is_none_or(|total| total > request.quantity)
    {
        return Err(Refused::new(
            FISCAL_RECEIPT_QUANTITY_EXCEEDED,
            "fiscal execution evidence exceeds its authorized quantity",
        ));
    }
    Ok(ValidatedExecutionEvidence {
        quantity: evidenced_quantity,
        disposition,
        external_operations,
    })
}

/// Requires `id` to be free among the scope's hot objects of one kind.
fn reject_existing<'a, T: 'a>(
    mut maps: impl Iterator<Item = &'a BTreeMap<String, T>>,
    id: &str,
    label: &str,
) -> Result<(), CanwuError> {
    if maps.any(|values| values.contains_key(id)) {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            format!("fiscal {label} identity already exists"),
        ));
    }
    Ok(())
}

/// The open book of a scope that holds `assessment_id`, by its cycle.
fn open_book_of_assessment(
    ledger: &FiscalScopeLedger,
    assessment_id: &str,
    label: &str,
) -> Result<String, CanwuError> {
    let book = ledger
        .book_of_assessment(assessment_id)
        .ok_or_else(|| invalid(format!("{label} names an unknown assessment")))?;
    if !book.is_open() {
        return Err(invalid(FISCAL_CYCLE_CLOSED));
    }
    Ok(book.accounting_cycle_id.clone())
}

fn book_mut<'a>(
    state: &'a mut FiscalState,
    scope: &str,
    cycle: &str,
) -> Result<&'a mut FiscalCycleBook, CanwuError> {
    state
        .ledger_mut(scope)?
        .books
        .get_mut(cycle)
        .ok_or_else(|| invalid("fiscal accounting cycle book is unavailable"))
}

/// Applies one action to the records of `state` it changes. The caller
/// validates the result.
#[allow(clippy::too_many_lines)]
pub(crate) fn apply_action(
    state: &mut FiscalState,
    catalog: &CompiledFiscalCatalog,
    request: &FiscalActionRequest,
    issuer: &FiscalActionIssuer,
    at: SimTime,
) -> Result<(), CanwuError> {
    let source = Some(request.action_id.clone());
    match &request.action {
        FiscalAction::ChangeAdoption {
            adoption_id,
            rule_id,
            scope_binding_id,
            stage,
        } => {
            let rule = catalog
                .rules
                .get(rule_id)
                .ok_or_else(|| invalid("fiscal adoption names an unknown rule"))?;
            let year = state.root.historical_context.year;
            let ledger = state
                .ledger(scope_binding_id)
                .ok_or_else(|| invalid("fiscal adoption names an unknown scope"))?;
            let scope = &ledger.scope;
            if rule.mechanism != scope.mechanism
                || !rule.jurisdiction_ids.contains(&scope.jurisdiction_id)
            {
                return Err(invalid(
                    "fiscal adoption rule is incompatible with its scope",
                ));
            }
            let generation = if let Some(existing) = ledger.adoptions.get(adoption_id) {
                if existing.rule_id != *rule_id {
                    return Err(invalid(
                        "fiscal adoption identity cannot change its rule or scope",
                    ));
                }
                existing
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| invalid("fiscal adoption generation overflowed"))?
            } else {
                if stage.is_operational()
                    && catalog
                        .transitions
                        .values()
                        .any(|transition| transition.to_rule_ids.contains(rule_id))
                {
                    return Err(invalid(
                        "operational transition targets require an atomic fiscal transition",
                    ));
                }
                if !rule.legal_window.contains(year) {
                    return Err(invalid("fiscal adoption is outside its legal window"));
                }
                1
            };
            state.ledger_mut(scope_binding_id)?.adoptions.insert(
                adoption_id.clone(),
                FiscalAdoptionState {
                    id: adoption_id.clone(),
                    rule_id: rule_id.clone(),
                    scope_binding_id: scope_binding_id.clone(),
                    stage: *stage,
                    generation,
                    changed_at: at,
                    source_action_id: source,
                },
            );
        }
        FiscalAction::ApplyTransition {
            transition_id,
            target_scope_bindings,
        } => apply_transition(
            state,
            catalog,
            transition_id,
            target_scope_bindings,
            &request.action_id,
            at,
        )?,
        FiscalAction::OpenAssessment {
            assessment_id,
            rule_id,
            scope_binding_id,
            accounting_cycle_id,
            quantity,
            unit,
            payment_form,
            commutation_quote,
        } => {
            let rule = catalog
                .rules
                .get(rule_id)
                .ok_or_else(|| invalid("fiscal assessment names an unknown rule"))?;
            if !rule
                .legal_window
                .contains(state.root.historical_context.year)
            {
                return Err(invalid(
                    "fiscal assessment rule is outside its legal window",
                ));
            }
            validate_identifier(accounting_cycle_id, "fiscal accounting cycle")?;
            let ledger = state
                .ledger(scope_binding_id)
                .ok_or_else(|| invalid("fiscal assessment names an unknown scope"))?;
            reject_existing(
                ledger.books.values().map(|book| &book.assessments),
                assessment_id,
                "assessment",
            )?;
            let institution = ledger.scope.institution.clone();
            let mechanism = ledger.scope.mechanism;
            match ledger.books.get(accounting_cycle_id) {
                Some(book) if !book.is_open() => return Err(invalid(FISCAL_CYCLE_CLOSED)),
                Some(_) => {}
                None => {
                    if ledger.books.len() >= MAX_FISCAL_HOT_BOOKS_PER_LEDGER {
                        return Err(CanwuError::new(
                            ErrorCode::ValueOutOfRange,
                            FISCAL_HOT_BOOKS_EXHAUSTED,
                        ));
                    }
                    let office = state
                        .office(&institution)
                        .ok_or_else(|| invalid("fiscal assessment scope has no office"))?;
                    if ledger
                        .watermark
                        .iter()
                        .chain(office.watermark.as_ref())
                        .any(|watermark| accounting_cycle_id <= watermark)
                    {
                        return Err(invalid(FISCAL_CYCLE_ARCHIVED));
                    }
                    if !office.cycles.contains_key(accounting_cycle_id)
                        && office.cycles.len() >= crate::model::MAX_FISCAL_HOT_CYCLES_PER_OFFICE
                    {
                        return Err(CanwuError::new(
                            ErrorCode::ValueOutOfRange,
                            FISCAL_HOT_BOOKS_EXHAUSTED,
                        ));
                    }
                    state.ledger_mut(scope_binding_id)?.books.insert(
                        accounting_cycle_id.clone(),
                        FiscalCycleBook::open(accounting_cycle_id.clone()),
                    );
                    let entry = state
                        .office_mut(&institution)
                        .cycles
                        .entry(accounting_cycle_id.clone())
                        .or_default();
                    entry.hot_books = entry
                        .hot_books
                        .checked_add(1)
                        .ok_or_else(|| invalid("fiscal office book count overflowed"))?;
                }
            }
            // The institution's books of the cycle and mechanism use at most
            // a few partition keys, so an institution row stays bounded.
            let keys = state
                .office_mut(&institution)
                .cycles
                .get_mut(accounting_cycle_id)
                .ok_or_else(|| invalid("fiscal office cycle bookkeeping is unavailable"))?
                .partition_keys
                .entry(mechanism)
                .or_default();
            keys.insert(FiscalPartitionKey {
                unit: unit.clone(),
                payment_form: *payment_form,
            });
            if keys.len() > MAX_FISCAL_PARTITION_KEYS {
                return Err(CanwuError::new(
                    ErrorCode::ValueOutOfRange,
                    FISCAL_PARTITION_KEYS_EXHAUSTED,
                ));
            }
            let book = book_mut(state, scope_binding_id, accounting_cycle_id)?;
            book.assessments.insert(
                assessment_id.clone(),
                FiscalAssessment {
                    id: assessment_id.clone(),
                    rule_id: rule_id.clone(),
                    scope_binding_id: scope_binding_id.clone(),
                    accounting_cycle_id: accounting_cycle_id.clone(),
                    quantity: *quantity,
                    unit: unit.clone(),
                    payment_form: *payment_form,
                    commutation_quote: commutation_quote.clone(),
                    created_at: at,
                    source_action_id: source,
                },
            );
            book.touch()?;
        }
        FiscalAction::GrantRemission {
            remission_id,
            scope_binding_id,
            assessment_id,
            quantity,
            reason,
        } => {
            let ledger = state
                .ledger(scope_binding_id)
                .ok_or_else(|| invalid("fiscal remission names an unknown scope"))?;
            reject_existing(
                ledger.books.values().map(|book| &book.remissions),
                remission_id,
                "remission",
            )?;
            let cycle = open_book_of_assessment(ledger, assessment_id, "fiscal remission")?;
            let book = book_mut(state, scope_binding_id, &cycle)?;
            book.remissions.insert(
                remission_id.clone(),
                FiscalRemission {
                    id: remission_id.clone(),
                    assessment_id: assessment_id.clone(),
                    quantity: *quantity,
                    reason: reason.clone(),
                    granted_at: at,
                    source_action_id: source,
                },
            );
            book.touch()?;
        }
        FiscalAction::AuthorizeExecution {
            request_id,
            scope_binding_id,
            assessment_id,
            kind,
            quantity,
            unit,
            resource,
            source: execution_source,
            target,
            purpose,
        } => {
            let ledger = state
                .ledger(scope_binding_id)
                .ok_or_else(|| invalid("execution authorization names an unknown scope"))?;
            reject_existing(
                ledger.books.values().map(|book| &book.execution_requests),
                request_id,
                "execution request",
            )?;
            let cycle = open_book_of_assessment(ledger, assessment_id, "execution authorization")?;
            let payment_form = ledger.books[&cycle].assessments[assessment_id].payment_form;
            let institution = ledger.scope.institution.clone();
            let book = book_mut(state, scope_binding_id, &cycle)?;
            book.execution_requests.insert(
                request_id.clone(),
                FiscalExecutionRequest {
                    id: request_id.clone(),
                    assessment_id: assessment_id.clone(),
                    institution,
                    kind: *kind,
                    quantity: *quantity,
                    unit: unit.clone(),
                    payment_form,
                    resource: *resource,
                    source: execution_source.clone(),
                    target: target.clone(),
                    purpose: purpose.clone(),
                    requested_at: at,
                    source_action_id: source,
                },
            );
            book.touch()?;
        }
        FiscalAction::RecordAudit {
            audit_id,
            scope_binding_id,
            target_id,
            severity,
            finding,
            evidence,
        } => {
            let ledger = state
                .ledger(scope_binding_id)
                .ok_or_else(|| invalid("fiscal audit names an unknown scope"))?;
            reject_existing(
                ledger.books.values().map(|book| &book.audits),
                audit_id,
                "audit",
            )?;
            let cycle = ledger
                .book_of_audit_target(target_id)
                .map(|book| book.accounting_cycle_id.clone())
                .ok_or_else(|| invalid("fiscal audit target is unavailable"))?;
            // A finding entered under an audit binding keeps how its auditor
            // was appointed, as the binding holds it now.
            let (appointed_by, nomination) = match issuer.role {
                FiscalIssuerRole::Auditor => state
                    .audit_binding(&issuer.binding_id)
                    .map(|binding| (binding.rebound_by.clone(), binding.nomination.clone()))
                    .unwrap_or_default(),
                _ => (None, None),
            };
            let book = book_mut(state, scope_binding_id, &cycle)?;
            book.audits.insert(
                audit_id.clone(),
                FiscalAuditFinding {
                    id: audit_id.clone(),
                    target_id: target_id.clone(),
                    severity: *severity,
                    finding: finding.clone(),
                    evidence: evidence.clone(),
                    recorded_at: at,
                    entered_by: Some(issuer.clone()),
                    appointed_by,
                    nomination,
                    source_action_id: source,
                },
            );
            book.touch()?;
        }
        FiscalAction::CloseAccountingCycle {
            scope_binding_id,
            accounting_cycle_id,
        } => {
            let book = book_mut(state, scope_binding_id, accounting_cycle_id)?;
            if !book.is_open() {
                return Err(invalid(FISCAL_CYCLE_CLOSED));
            }
            book.status = FiscalBookStatus::Closed {
                closed_by: rebinding(request, issuer, at),
            };
            book.touch()?;
        }
        FiscalAction::RebindAuthority {
            authority_binding_id,
            authorized_actor,
            acting_actor,
            authority_basis,
        } => {
            if *authority_binding_id != request.authority_binding_id {
                return Err(invalid_authority(
                    "a fiscal authority rebind must be issued under the binding it replaces",
                ));
            }
            let binding = state
                .authority_binding_mut(authority_binding_id)
                .ok_or_else(|| {
                    invalid_authority(
                        "a fiscal authority rebind must be issued under the binding it replaces",
                    )
                })?;
            binding.authorized_actor = *authorized_actor;
            binding.acting_actor = *acting_actor;
            binding.authority_basis.clone_from(authority_basis);
            binding.rebound_by = Some(rebinding(request, issuer, at));
        }
        FiscalAction::RebindAudit { .. }
        | FiscalAction::NominateAuditor { .. }
        | FiscalAction::PetitionAuditorRemoval { .. }
        | FiscalAction::DecideAuditAppointment { .. } => {
            apply_appointment(state, request, issuer, at)?;
        }
        FiscalAction::RebindObserver {
            observer_binding_id,
            actor,
            knowledge_holder,
        } => {
            if !holder_matches(*actor, knowledge_holder) {
                return Err(invalid_payload(
                    "a fiscal observer holder is its actor's person or, without an actor, a government",
                ));
            }
            let observer = active_observer(state, observer_binding_id)?;
            let previous =
                std::mem::replace(&mut observer.knowledge_holder, knowledge_holder.clone());
            observer.actor = *actor;
            observer.rebound_by = Some(rebinding(request, issuer, at));
            if previous != *knowledge_holder {
                // The old holder gets a withdrawn summary, and the new one
                // the whole report, which it holds none of yet.
                let report = &mut state
                    .observers
                    .get_mut(observer_binding_id)
                    .ok_or_else(|| invalid("fiscal observer binding is unavailable"))?
                    .report;
                report.withdraw.remove(knowledge_holder);
                report.withdraw.insert(previous);
                state.report_queue.insert(
                    observer_binding_id.clone(),
                    FiscalReportQueueEntry {
                        due_at: at,
                        whole: true,
                        ..FiscalReportQueueEntry::default()
                    },
                );
            }
        }
        FiscalAction::RemoveObserver {
            observer_binding_id,
        } => {
            let observer = active_observer(state, observer_binding_id)?;
            observer.removed = Some(rebinding(request, issuer, at));
            let holder = observer.knowledge_holder.clone();
            state
                .observers
                .get_mut(observer_binding_id)
                .ok_or_else(|| invalid("fiscal observer binding is unavailable"))?
                .report
                .withdraw
                .insert(holder);
            state.report_queue.insert(
                observer_binding_id.clone(),
                FiscalReportQueueEntry {
                    due_at: at,
                    ..FiscalReportQueueEntry::default()
                },
            );
        }
    }
    Ok(())
}

fn active_observer<'a>(
    state: &'a mut FiscalState,
    observer_binding_id: &str,
) -> Result<&'a mut FiscalObserverBinding, CanwuError> {
    state
        .observers
        .get_mut(observer_binding_id)
        .map(|observer| &mut observer.binding)
        .ok_or_else(|| invalid("fiscal observer binding is unavailable"))
        .and_then(|observer| {
            if observer.is_active() {
                Ok(observer)
            } else {
                Err(invalid(FISCAL_OBSERVER_REMOVED))
            }
        })
}

pub(crate) fn rebinding(
    request: &FiscalActionRequest,
    issuer: &FiscalActionIssuer,
    at: SimTime,
) -> FiscalRebinding {
    FiscalRebinding {
        action_id: request.action_id.clone(),
        issuer: issuer.clone(),
        rebound_at: at,
    }
}

/// Applies an eligible transition in its region: every adoption in the
/// region of a rule the transition supersedes or suspends is suspended, and
/// each target rule is implemented in the scope the action binds it to. The
/// region's ledgers must all be in `state`.
fn apply_transition(
    state: &mut FiscalState,
    catalog: &CompiledFiscalCatalog,
    transition_id: &str,
    target_scope_bindings: &BTreeMap<String, String>,
    action_id: &str,
    at: SimTime,
) -> Result<(), CanwuError> {
    let transition = catalog
        .transitions
        .get(transition_id)
        .ok_or_else(|| invalid("fiscal transition is unavailable"))?;
    if target_scope_bindings.keys().collect::<BTreeSet<_>>()
        != transition.to_rule_ids.iter().collect::<BTreeSet<_>>()
    {
        return Err(invalid(
            "fiscal transition must bind every target rule exactly once",
        ));
    }
    let mut target_jurisdiction = None;
    for (rule_id, scope_id) in target_scope_bindings {
        let rule = &catalog.rules[rule_id];
        let scope = state
            .scope_binding(scope_id)
            .ok_or_else(|| invalid("fiscal transition target scope is unavailable"))?;
        if scope.mechanism != rule.mechanism {
            return Err(invalid(
                "fiscal transition target is incompatible with its jurisdiction or mechanism",
            ));
        }
        if target_jurisdiction
            .replace(scope.jurisdiction_id.clone())
            .is_some_and(|jurisdiction| jurisdiction != scope.jurisdiction_id)
        {
            return Err(invalid(
                "fiscal transition targets must share one jurisdiction",
            ));
        }
    }
    let target_jurisdiction = target_jurisdiction
        .ok_or_else(|| invalid("fiscal transition has no target jurisdiction"))?;
    let candidate_id = format!("{transition_id}::{target_jurisdiction}");
    if !state
        .jurisdiction(&target_jurisdiction)
        .is_some_and(|jurisdiction| {
            jurisdiction
                .transition_candidates
                .contains_key(&candidate_id)
        })
    {
        return Err(invalid("fiscal transition is not currently eligible"));
    }
    for ledger in state
        .ledgers
        .values_mut()
        .filter(|ledger| ledger.scope.jurisdiction_id == target_jurisdiction)
    {
        for adoption in ledger.adoptions.values_mut().filter(|adoption| {
            transition
                .supersedes_or_suspends
                .contains(&adoption.rule_id)
        }) {
            adoption.stage = FiscalAdoptionStage::Suspended;
            adoption.generation = adoption
                .generation
                .checked_add(1)
                .ok_or_else(|| invalid("fiscal adoption generation overflowed"))?;
            adoption.changed_at = at;
            adoption.source_action_id = Some(action_id.to_owned());
        }
    }
    for (rule_id, scope_id) in target_scope_bindings {
        let ledger = state.ledger_mut(scope_id)?;
        let existing_id = ledger
            .adoptions
            .values()
            .find_map(|adoption| (adoption.rule_id == *rule_id).then(|| adoption.id.clone()));
        let adoption_id = existing_id
            .unwrap_or_else(|| format!("adopt.transition.{transition_id}.{scope_id}.{rule_id}"));
        let generation = ledger
            .adoptions
            .get(&adoption_id)
            .map_or(Ok(1), |adoption| {
                adoption
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| invalid("fiscal adoption generation overflowed"))
            })?;
        ledger.adoptions.insert(
            adoption_id.clone(),
            FiscalAdoptionState {
                id: adoption_id,
                rule_id: rule_id.clone(),
                scope_binding_id: scope_id.clone(),
                stage: FiscalAdoptionStage::Implemented,
                generation,
                changed_at: at,
                source_action_id: Some(action_id.to_owned()),
            },
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        FISCAL_RECENT_OUTCOMES, FiscalMechanism, FiscalOutcomeRing, FiscalPaymentForm,
        FiscalRecentOutcome, MAX_FISCAL_IDENTIFIER_BYTES, merge_cycle_totals,
    };
    use canwu_api::OrganizationId;

    fn total(mechanism: FiscalMechanism, unit: &str, amount: u64) -> FiscalCycleTotal {
        FiscalCycleTotal {
            mechanism,
            unit: unit.to_owned(),
            payment_form: FiscalPaymentForm::SaltCertificate,
            assessed: amount,
            remission_granted: amount,
            collected: amount,
            remitted: amount,
            disbursed: amount,
            reserved: amount,
            returned: amount,
            outstanding: amount,
        }
    }

    /// Archive commits merge totals of a cycle's partition keys and count
    /// the books they take; none of it makes the office's budgeted bytes
    /// grow, so a settled office never passes its budget through the
    /// archive.
    #[test]
    fn archive_merges_never_grow_an_office_past_its_budgeted_bytes() {
        let mut office = FiscalOffice::new(EntityRef::Organization(OrganizationId::new(1)));
        let units: Vec<String> = (0..MAX_FISCAL_PARTITION_KEYS)
            .map(|index| format!("{index}{}", "u".repeat(MAX_FISCAL_IDENTIFIER_BYTES - 1)))
            .collect();
        let mut entry = FiscalOfficeCycle {
            hot_books: 1_000,
            ..FiscalOfficeCycle::default()
        };
        entry.partition_keys.insert(
            FiscalMechanism::TreasuryAndRemittance,
            units
                .iter()
                .map(|unit| FiscalPartitionKey {
                    unit: unit.clone(),
                    payment_form: FiscalPaymentForm::SaltCertificate,
                })
                .collect(),
        );
        office.cycles.insert("2026".to_owned(), entry);
        let mut budgeted = office_budget_bytes(&office).expect("bytes");
        for round in 0..64_u32 {
            let entry = office.cycles.get_mut("2026").expect("the cycle");
            entry.hot_books -= 1;
            entry.archived_books = 1_000_000_000 + round;
            let unit = &units[usize::try_from(round).expect("round") % units.len()];
            merge_cycle_totals(
                &mut entry.archived_totals,
                &[total(
                    FiscalMechanism::TreasuryAndRemittance,
                    unit,
                    10_u64.pow(round % 19),
                )],
            )
            .expect("merged");
            let now = office_budget_bytes(&office).expect("bytes");
            assert!(now <= budgeted, "round {round}: {now} > {budgeted}");
            assert!(
                json_len(&office.cycles["2026"]).expect("bytes")
                    <= largest_cycle_bytes(&office.cycles["2026"]).expect("bytes")
            );
            budgeted = now;
        }
        // Every partition key now holds a total; the room left is the
        // difference to the largest one.
        assert_eq!(
            office.cycles["2026"].archived_totals.len(),
            MAX_FISCAL_PARTITION_KEYS
        );
    }

    /// What settlement writes to a record without its budget check, a full
    /// outcome ring, the digits of the largest revision, and the ring's
    /// field name, fits the half of the ring reserve that activation keeps.
    #[test]
    fn a_full_ring_fits_half_the_ring_reserve() {
        let ring = FiscalOutcomeRing {
            recent: (0..FISCAL_RECENT_OUTCOMES)
                .map(|_| FiscalRecentOutcome {
                    action_id: "a".repeat(MAX_FISCAL_IDENTIFIER_BYTES),
                    disposition: FiscalActionDisposition::Rejected,
                    settled_revision: u64::MAX,
                })
                .collect(),
            floor: u64::MAX,
        };
        let revision_digits = u64::MAX.to_string().len();
        let field = r#","outcomes":"#.len();
        assert!(
            json_len(&ring).expect("bytes") + revision_digits + field
                <= FISCAL_RECORD_RING_RESERVE_BYTES / 2
        );
    }
}
