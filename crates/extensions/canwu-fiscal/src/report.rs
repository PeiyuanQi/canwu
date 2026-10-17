//! Fiscal reports: the system that publishes them (phase 13), their report
//! lines, and the reads that put them back together.
//!
//! Settlement queues, for each observer, the report parts its boundary's
//! changes touch, due as the observer's cadence schedules it. The report
//! system reads the root's earliest due time, then the report schedule, and
//! decodes only the queue pages whose earliest due time has come; it takes
//! their due entries in order, computes only the parts each entry names
//! from the ledgers they cover (summed per institution and mechanism at
//! institution granularity), compares each part's digest with the one the
//! observer record holds, and publishes the summary and the parts that
//! changed. An interval observer's summary is published at every due time
//! once its report has been published at least once (alone when nothing
//! changed), and its entry is queued again at its next due time. Parked
//! entries are checked again in rotation. What does not fit the boundary's
//! publication limits stays queued, and a boundary that did some of the
//! work owed schedules an internal drain packet so that the next boundary
//! at the same instant continues.

use crate::derive::assemble_fiscal_reports;
use crate::derive::{institution_row_of_totals, part_entry, parts_of, rendered_row};
use crate::model::{
    BookFigures, CompiledFiscalCatalog, EMPTY_PART_ENTRY, FISCAL_PARKED_CHECKS_PER_BOUNDARY,
    FISCAL_REPORT_ROWS_PER_PART, FiscalCycleTotal, FiscalMechanism, FiscalObserver,
    FiscalObserverBinding, FiscalReport, FiscalReportBudget, FiscalReportCycleSlot,
    FiscalReportGranularity, FiscalReportQueueEntry, FiscalReportRow, FiscalReportSummary,
    MAX_FISCAL_REPORT_CYCLE_SLOTS, MAX_FISCAL_REPORT_PARTS_PER_CYCLE, invalid, sum_report_totals,
};
use crate::plugin::{FISCAL_REPORT_DRAIN_INGRESS, PLUGIN_NAME, encode_error};
use crate::records::{
    FiscalIndexOwner, FiscalObserverRecord, fiscal_ledger_reference, fiscal_observer_reference,
    fiscal_root_reference, office_reference, report_queue_page,
};
use crate::working::{RecordSource, Working, active_record, bound_queue_entry, field};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryProposal, CanwuError, CanwuRun, CanwuViewer,
    DomainRecordKind, DomainRecordLifecycle, DomainRecordRef, DomainRecordVersionRef, EvidenceRef,
    KnowledgeHolderRef, KnowledgeLimitsV1, KnowledgeOrigin, KnowledgePublicationBudget,
    KnowledgePublicationGroup, KnowledgeQuery, KnowledgeRecordDraft, KnowledgeRecordKind,
    KnowledgeRetention, KnowledgeSchemaId, KnowledgeSubject, KnowledgeSubjectSchema,
    KnowledgeSubjectTarget, KnowledgeSubjectTargetKind, KnowledgeSupersession, LifeState,
    MAX_KNOWLEDGE_PAGE_SIZE, PayloadSchema, PluginKnowledgeSchema, SimTime, SimulationView,
    StateVisibility,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const FISCAL_REPORT_KNOWLEDGE: &str = "fiscal_report";
/// Version 1 was one record for an observer's whole report. Version 2 is a
/// summary record and part records, each on a report line of its own.
/// Version 3 numbers the lines from each binding's own slot, lets an
/// institution hold a report without a person, and marks a withdrawn
/// summary. Version 4 names the lines on each binding's own observer record,
/// cuts parts by scope rows and cycle slots, and lists the cycles in the
/// summary. Version 5 carries the binding's granularity and cadence in the
/// summary, institution rows (with no scope) and an institution part's
/// `books_summed`. Version 6 encodes a part compactly: its institutions
/// once each in a table, and its rows and facts as arrays
/// ([`crate::FiscalReportPart`]).
const FISCAL_REPORT_SCHEMA_VERSION: u32 = 6;
const FISCAL_REPORT_SCHEMA_HASH: &str =
    "29bc8d2c97c4c4e08b6d280d2935b65d90dfd1295757ec1850464db505a1c78c";
/// The subject role every fiscal report record carries, on its observer
/// record.
const REPORT_OBSERVER_ROLE: &str = "fiscal_observer";
/// Report lines one cycle slot takes: one for each part.
const REPORT_LINES_PER_SLOT: u32 = 1_024;
/// Hexadecimal places of a report line number.
const REPORT_LINE_PLACES: u32 = 5;
const _: () = assert!(MAX_FISCAL_REPORT_PARTS_PER_CYCLE <= REPORT_LINES_PER_SLOT as usize);
const _: () = assert!(
    MAX_FISCAL_REPORT_CYCLE_SLOTS * (REPORT_LINES_PER_SLOT as usize)
        < 1 << (4 * REPORT_LINE_PLACES)
);

/// The knowledge schema of fiscal report records, at its one writable
/// version.
///
/// A record of this schema is one [`crate::FiscalReportRecord`]: the summary or
/// one part of one observer's report. Use [`fiscal_reports`] to read whole
/// reports.
#[must_use]
pub fn fiscal_report_knowledge_schema_id() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new(PLUGIN_NAME, FISCAL_REPORT_KNOWLEDGE),
        FISCAL_REPORT_SCHEMA_VERSION,
    )
}

/// The report schema as the plugin registers it, with the retention the
/// host chose (`FiscalPlugin::with_report_retention`).
pub(crate) fn fiscal_report_knowledge_schema(
    retention: KnowledgeRetention,
) -> PluginKnowledgeSchema {
    PluginKnowledgeSchema {
        id: fiscal_report_knowledge_schema_id(),
        schema_hash: FISCAL_REPORT_SCHEMA_HASH.to_owned(),
        writable: true,
        payload_schema: PayloadSchema::Any,
        subjects: report_subject_schemas(),
        retention,
    }
}

/// The observer record every report record is about, and one role for each
/// digit of each place of a report line number, on the same record.
fn report_subject_schemas() -> Vec<KnowledgeSubjectSchema> {
    let on_observer = |role: String, required: bool| KnowledgeSubjectSchema {
        role,
        targets: vec![KnowledgeSubjectTargetKind::Domain(
            DomainRecordKind::for_type::<FiscalObserverRecord>(),
        )],
        required,
        multiple: false,
    };
    let mut subjects: Vec<_> = (0..REPORT_LINE_PLACES)
        .flat_map(|place| (0..16).map(move |digit| report_line_role(place, digit)))
        .map(|role| on_observer(role, false))
        .collect();
    subjects.push(on_observer(REPORT_OBSERVER_ROLE.to_owned(), true));
    subjects.sort_by(|left, right| left.role.cmp(&right.role));
    subjects
}

fn report_line_role(place: u32, digit: u32) -> String {
    format!("report_line_{place}_{digit:x}")
}

/// The line of a summary.
const SUMMARY_LINE: u32 = 0;

/// The line of part `part` of the cycle in slot `slot`.
fn part_line(slot: u16, part: u32) -> u32 {
    1 + u32::from(slot) * REPORT_LINES_PER_SLOT + part
}

/// The subjects of the records on one report line of one observer binding.
///
/// `SimulationView::knowledge_heads` selects the records of a holder that
/// carry every subject it is given, so each line needs a subject no other
/// line has. A subject is a role and a target; the target is the observer's
/// own record, so lines only have to differ within one binding, and the
/// line is told by the roles: the five hexadecimal digits of its number,
/// each a role of its own place. Two lines differ in at least one place, so
/// neither line's subjects contain the other's, and re-pointing or removing
/// one binding moves no other binding's lines.
fn report_line_subjects(observer_id: &str, line: u32) -> Vec<KnowledgeSubject> {
    let target =
        KnowledgeSubjectTarget::DomainRecord(fiscal_observer_reference(observer_id).into_untyped());
    let mut subjects: Vec<_> = (0..REPORT_LINE_PLACES)
        .map(|place| KnowledgeSubject {
            role: report_line_role(place, (line >> (4 * place)) & 0xf),
            target: target.clone(),
        })
        .collect();
    subjects.push(KnowledgeSubject {
        role: REPORT_OBSERVER_ROLE.to_owned(),
        target,
    });
    subjects.sort();
    subjects
}

/// Whether the engine would refuse to publish knowledge to this holder.
///
/// It refuses a holder who does not exist or is dead. An observer record
/// references its holder, so the holder exists; what is left is a person's
/// life state. A government holder is always reachable. The report system
/// runs after the last phase that can change a life state, so it reads the
/// same answer the engine checks a publication against.
pub(crate) fn holder_is_unreachable(
    view: &SimulationView<'_>,
    holder: &KnowledgeHolderRef,
) -> Result<bool, CanwuError> {
    match holder {
        KnowledgeHolderRef::Person(person) => Ok(view
            .person_availability(*person)?
            .is_some_and(|availability| availability.life == LifeState::Dead)),
        KnowledgeHolderRef::Entity(_) => Ok(false),
    }
}

/// The internal packet, due at once, that makes the next boundary come and
/// publish the report work this one left queued.
pub(crate) fn drain_directive() -> BoundaryDirective {
    BoundaryDirective::ScheduleIngress {
        after: canwu_api::SimDuration::ZERO,
        packet_type: FISCAL_REPORT_DRAIN_INGRESS.to_owned(),
        priority: 0,
        payload: Value::Object(serde_json::Map::new()),
        affected: Vec::new(),
    }
}

/// The payload bytes a publication keeps for its summary while parts are
/// added: the most one knowledge record may carry, so a summary always fits.
const SUMMARY_RESERVE_BYTES: usize = KnowledgeLimitsV1::CURRENT.payload_bytes_per_record;

/// The records one boundary publishes, within fiscal's share of the
/// boundary's publication budget.
///
/// Every publication to one holder is one group of one holder, `Explicit`:
/// each draft supersedes what the holder's line holds now. Groups are packed
/// into directives up to the per-directive bounds, so a boundary that
/// publishes to thousands of holders makes a few directives and a few
/// `KnowledgePublished` events.
struct Publisher<'a, 'v> {
    view: &'a SimulationView<'v>,
    kind: KnowledgeRecordKind,
    at: SimTime,
    boundary: u64,
    /// What the report system may still publish in this boundary: the least
    /// of the engine's budget left at its cut and fiscal's share, less what
    /// it has published.
    budget: KnowledgePublicationBudget,
    /// The groups of the directive being filled, and their bodies.
    groups: Vec<KnowledgePublicationGroup>,
    group_bodies: usize,
    directives: Vec<BoundaryDirective>,
    /// Whether the boundary has done any of the work owed: published a part
    /// or a withdrawal, or finished an entry. A summary alone republished
    /// beside parts that did not fit is no progress.
    progress: bool,
}

impl<'a, 'v> Publisher<'a, 'v> {
    fn new(
        view: &'a SimulationView<'v>,
        context: &BoundaryContext,
        share: FiscalReportBudget,
    ) -> Self {
        let engine = view.knowledge_publication_budget();
        Self {
            view,
            kind: fiscal_report_knowledge_schema_id().kind,
            at: context.at,
            boundary: context.boundary_id.get(),
            budget: KnowledgePublicationBudget {
                records: engine.records.min(share.bodies),
                holder_entries: engine.holder_entries.min(share.entries),
                payload_bytes: engine.payload_bytes.min(share.payload_bytes),
                ..engine
            },
            groups: Vec::new(),
            group_bodies: 0,
            directives: Vec::new(),
            progress: false,
        }
    }

    /// How many records the next publication to one holder may carry.
    fn room(&self) -> usize {
        self.budget
            .records
            .min(self.budget.holder_entries)
            .min(self.budget.records_per_batch)
            .min(self.budget.holder_entries_per_batch)
    }

    /// The payload bytes the parts of the next publication may carry,
    /// keeping room for its summary.
    fn part_bytes(&self) -> usize {
        self.budget
            .payload_bytes
            .saturating_sub(SUMMARY_RESERVE_BYTES)
    }

    /// Whether a publication, a summary at least, still fits.
    fn can_publish(&self) -> bool {
        self.room() > 0 && self.budget.payload_bytes >= SUMMARY_RESERVE_BYTES
    }

    /// The draft of one record on one line of an observer, superseding what
    /// the holder's line holds now. `payload` is the record's payload, built
    /// directly (see [`summary_payload`] and [`part_payload`]).
    fn draft(
        &self,
        holder: &KnowledgeHolderRef,
        observer: &FiscalObserver,
        line: u32,
        payload: Value,
        evidence: Vec<EvidenceRef>,
    ) -> Result<KnowledgeRecordDraft, CanwuError> {
        let subjects = report_line_subjects(&observer.binding.id, line);
        let supersedes = self
            .view
            .knowledge_heads(holder, &self.kind, &subjects)?
            .records;
        Ok(KnowledgeRecordDraft {
            schema: fiscal_report_knowledge_schema_id(),
            subjects,
            payload,
            as_of: Some(self.at),
            confidence_per_mille: observer.binding.confidence_per_mille,
            origin: KnowledgeOrigin {
                method: "fiscal_authority_report_v5".to_owned(),
                evidence,
            },
            supersedes,
            contradicts: Vec::new(),
        })
    }

    /// Publishes the records to one holder as one group of the directive
    /// being filled, taking their bodies, entries, and `bytes` from the
    /// budget; a group that would pass the per-directive bounds starts a new
    /// directive. `bytes` is what the caller measured of their payloads with
    /// `json_len`, the compact JSON length the engine charges. The caller
    /// cuts the records to [`Self::room`] and [`Self::part_bytes`] first.
    fn publish(
        &mut self,
        holder: &KnowledgeHolderRef,
        records: Vec<KnowledgeRecordDraft>,
        bytes: usize,
    ) -> Result<(), CanwuError> {
        let bodies = records.len();
        let bound = self
            .budget
            .records_per_batch
            .min(self.budget.holder_entries_per_batch);
        if self.group_bodies + bodies > bound {
            self.flush();
        }
        if !self.budget.take(bodies, bodies, bytes) {
            return Err(invalid(
                "a fiscal report publication passes the budget it was cut to",
            ));
        }
        self.group_bodies += bodies;
        self.groups.push(KnowledgePublicationGroup {
            holders: vec![holder.clone()],
            records,
        });
        Ok(())
    }

    /// Closes the directive being filled.
    fn flush(&mut self) {
        if self.groups.is_empty() {
            return;
        }
        let index = self.directives.len();
        self.directives.push(BoundaryDirective::PublishKnowledge {
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some(format!("fiscal-reports::{}::{index}", self.boundary)),
            supersession: KnowledgeSupersession::Explicit,
            groups: std::mem::take(&mut self.groups),
            summary: "Publish holder-relative fiscal reports".to_owned(),
        });
        self.group_bodies = 0;
    }

    /// The directives of everything published.
    fn into_directives(mut self) -> Vec<BoundaryDirective> {
        self.flush();
        self.directives
    }
}

/// Publishes the report work the queue owes, in order of due time and
/// observer binding ID, within the boundary's publication limits.
///
/// A boundary before the root's earliest due time, with no parked entry,
/// returns after one read of the root. Otherwise the report system reads the
/// report schedule and decodes only the pages whose earliest due time has
/// come. An observer whose holder cannot receive knowledge is parked: its
/// work stays queued and is not due; each run checks the holders of the next
/// [`FISCAL_PARKED_CHECKS_PER_BOUNDARY`] parked entries after the schedule's
/// cursor, wrapping round, and publishes what a revived holder is owed, so a
/// revived holder is published to within `ceil(parked / 16)` runs. An
/// interval observer, once what its entry owed is published, is queued again
/// at its next due time after this boundary, whether or not anything was
/// published. Work that does not fit the limits stays queued, and a
/// boundary that did some of the work owed (published a part or a
/// withdrawal, or finished an entry) schedules an internal
/// [`FISCAL_REPORT_DRAIN_INGRESS`] packet, due at once. A boundary that could
/// do none of it, because the systems before this one spent the publication
/// budget, schedules none: its work waits for the next boundary that runs
/// the report system (one that carries an event or ingress, or the wake of
/// [`crate::FiscalPlugin::with_report_cadence`]), so the drains at one
/// instant always end.
///
/// The record a new one supersedes is read from the committed knowledge
/// ledger with `SimulationView::knowledge_heads`, never from retained
/// evidence, and due times and the rotation come from the queue pages and
/// the report schedule, so a run that sealed its evidence publishes exactly
/// what its replay does.
pub(crate) fn materialize_reports(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    catalog: impl FnOnce() -> Result<Option<CompiledFiscalCatalog>, CanwuError>,
    share: FiscalReportBudget,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(root) = active_record(view, &fiscal_root_reference())? else {
        return Ok(BoundaryProposal::default());
    };
    let earliest_due: Option<SimTime> = field(&root.payload, "earliest_due")?;
    let parked: bool = field(&root.payload, "parked")?;
    if !parked && earliest_due.is_none_or(|due| due > context.at) {
        return Ok(BoundaryProposal::default());
    }
    let Some(catalog) = catalog()? else {
        return Ok(BoundaryProposal::default());
    };
    let Some(mut working) = Working::new(view)? else {
        return Ok(BoundaryProposal::default());
    };
    working.ensure_schedule()?;
    let mut pages: BTreeSet<String> = working
        .state
        .report_schedule
        .page_due
        .iter()
        .filter(|(_, due)| **due <= context.at)
        .map(|(page, _)| page.clone())
        .collect();
    if parked {
        pages.extend(check_parked(&mut working)?);
    }
    let mut queued = Vec::new();
    for page in &pages {
        for (observer, entry) in &working.queue_page(page)?.entries {
            if !entry.parked && entry.due_at <= context.at {
                queued.push((entry.due_at, observer.clone(), page.clone()));
            }
        }
    }
    queued.sort();
    let mut publisher = Publisher::new(view, context, share);
    let mut owes_more = false;
    let mut reads = ReportReads::default();
    for (_, observer_id, page) in queued {
        // Once the boundary can publish nothing more, the observers after
        // this one keep their queued work, untouched, for the drain.
        if !publisher.can_publish() {
            owes_more = true;
            continue;
        }
        let Some(mut entry) = working.queue_page(&page)?.entries.remove(&observer_id) else {
            continue;
        };
        let done = if working.ensure_observer(&observer_id)? {
            publish_observer(
                &mut working,
                &mut reads,
                &mut publisher,
                &observer_id,
                &mut entry,
            )?
        } else {
            true
        };
        if done {
            publisher.progress = true;
            // An interval observer always has its entry: once published, it
            // is due again at its next due time.
            let next = working
                .state
                .observers
                .get(&observer_id)
                .map(|observer| observer.binding.scheduled_cadence())
                .filter(|cadence| cadence.is_interval())
                .and_then(|cadence| cadence.due_after(context.at));
            if let Some(due_at) = next {
                working.queue_page(&page)?.entries.insert(
                    observer_id,
                    FiscalReportQueueEntry {
                        due_at,
                        ..FiscalReportQueueEntry::default()
                    },
                );
            }
        } else {
            owes_more |= !entry.parked;
            working
                .queue_page(&page)?
                .entries
                .insert(observer_id, entry);
        }
    }
    // A drain only follows a boundary that did some of the work owed, so
    // the drains at one instant end: when systems before this one spent the
    // budget, the work waits for the next boundary that runs this system.
    let drain = owes_more && publisher.progress && view.can_schedule_ingress();
    let mut directives = working.directives(&catalog, "Published fiscal reports")?;
    directives.extend(publisher.into_directives());
    if drain {
        directives.push(drain_directive());
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Publishes what one observer is owed, as far as the boundary's limits
/// allow, and returns whether its queue entry is done. The entry is left
/// with what is still owed, or parked when the holder cannot receive
/// knowledge now. An interval observer whose report was published before
/// gets its summary even when nothing changed: it is at a due time.
#[allow(clippy::too_many_lines)]
fn publish_observer(
    working: &mut Working<'_, SimulationView<'_>>,
    reads: &mut ReportReads,
    publisher: &mut Publisher<'_, '_>,
    observer_id: &str,
    entry: &mut FiscalReportQueueEntry,
) -> Result<bool, CanwuError> {
    let view = publisher.view;
    let mut observer = working
        .state
        .observers
        .get(observer_id)
        .cloned()
        .ok_or_else(|| invalid("fiscal observer is not loaded"))?;
    let source = view
        .current_domain_record_version(fiscal_observer_reference(observer_id).as_untyped())?
        .ok_or_else(|| invalid("fiscal observer record has no current version"))?;
    // Withdrawn summaries for the holders the binding left.
    let mut done = true;
    for holder in observer.report.withdraw.clone() {
        if !publisher.can_publish() {
            done = false;
            break;
        }
        observer.report.withdraw.remove(&holder);
        // A holder that can no longer receive knowledge keeps what it holds.
        if holder_is_unreachable(view, &holder)? {
            publisher.progress = true;
            continue;
        }
        let summary = FiscalReportSummary {
            observer_id: observer_id.to_owned(),
            actor: match &holder {
                KnowledgeHolderRef::Person(person) => Some(*person),
                KnowledgeHolderRef::Entity(_) => None,
            },
            as_of: publisher.at,
            confidence_per_mille: observer.binding.confidence_per_mille,
            granularity: observer.binding.granularity,
            cadence: observer.binding.cadence,
            cycles: BTreeMap::new(),
            fact_count: 0,
            pending: BTreeMap::new(),
            omitted_cycles: 0,
            withdrawn: true,
        };
        let payload = summary_payload(&summary)?;
        let bytes = crate::settle::json_len(&payload)?;
        let record = publisher.draft(
            &holder,
            &observer,
            SUMMARY_LINE,
            payload,
            vec![EvidenceRef::DomainRecordVersion(source.clone())],
        )?;
        publisher.publish(&holder, vec![record], bytes)?;
        publisher.progress = true;
    }
    // The loaded observer stays in the working set as it was until it is
    // replaced, so it is what a changed one is compared with.
    let finish = |working: &mut Working<'_, SimulationView<'_>>, observer: FiscalObserver| {
        if working.state.observers.get(observer_id) != Some(&observer) {
            working
                .state
                .observers
                .insert(observer_id.to_owned(), observer);
            working.touch(fiscal_observer_reference(observer_id).into_untyped());
        }
    };
    // Withdrawals that did not all fit leave the entry as it is for the
    // drain, and so do withdrawals that used up the boundary before an
    // interval report, which owes at least its summary.
    if !done
        || !publisher.can_publish()
            && observer.binding.is_active()
            && observer.binding.cadence.is_interval()
    {
        entry.parked = false;
        finish(working, observer);
        return Ok(false);
    }
    if !observer.binding.is_active() {
        finish(working, observer);
        return Ok(true);
    }
    let holder = observer.binding.knowledge_holder.clone();
    if holder_is_unreachable(view, &holder)? {
        entry.parked = true;
        finish(working, observer);
        return Ok(false);
    }
    entry.parked = false;
    // The cycles the report shows: every hot cycle of a visible office, as
    // far as there are slots.
    let mut visible = BTreeSet::new();
    for rows in &observer.rows {
        visible.extend(working.office_cycles(&rows.office)?);
    }
    let report = &mut observer.report;
    report
        .omitted_cycles
        .retain(|cycle| visible.contains(cycle));
    // A cycle that was left out for want of a slot has books whose parts no
    // queue entry names any more; a cycle new to the report has only books
    // whose creation queued their parts.
    let previously_omitted = report.omitted_cycles.clone();
    let leaving: BTreeSet<String> = report
        .cycles
        .keys()
        .filter(|cycle| !visible.contains(*cycle))
        .cloned()
        .collect();
    let mut used: BTreeSet<u16> = report.cycles.values().map(|slot| slot.slot).collect();
    let mut fresh = BTreeSet::new();
    let waiting: Vec<String> = report
        .omitted_cycles
        .iter()
        .cloned()
        .chain(
            visible
                .iter()
                .filter(|cycle| {
                    !report.cycles.contains_key(*cycle) && !report.omitted_cycles.contains(*cycle)
                })
                .cloned(),
        )
        .collect();
    for cycle in waiting {
        let free = (0..u16::try_from(MAX_FISCAL_REPORT_CYCLE_SLOTS).unwrap_or(u16::MAX))
            .find(|slot| !used.contains(slot));
        match free {
            Some(slot) => {
                used.insert(slot);
                report.omitted_cycles.remove(&cycle);
                report
                    .cycles
                    .insert(cycle.clone(), FiscalReportCycleSlot::new(slot));
                fresh.insert(cycle);
            }
            None => {
                report.omitted_cycles.insert(cycle);
            }
        }
    }
    // What is owed, in order: the empty parts of leaving cycles first, then
    // the rest by cycle and part.
    let row_count = observer.row_count();
    let all_parts = parts_of(row_count);
    let mut owed = BTreeMap::<(bool, String, u32), (u16, bool)>::new();
    let mut add = |leaving: bool, cycle: &str, part: u32, slot: u16, force: bool| {
        let value = owed
            .entry((!leaving, cycle.to_owned(), part))
            .or_insert((slot, force));
        value.1 |= force;
    };
    for (cycle, slot) in &observer.report.cycles {
        if leaving.contains(cycle) {
            for part in slot.parts_with_facts() {
                add(true, cycle, part, slot.slot, true);
            }
        } else if entry.whole || fresh.contains(cycle) && previously_omitted.contains(cycle) {
            for part in 0..all_parts {
                add(false, cycle, part, slot.slot, entry.whole);
            }
        }
    }
    for (cycle, parts) in &entry.cycles {
        let Some(slot) = observer
            .report
            .cycles
            .get(cycle)
            .filter(|_| !leaving.contains(cycle))
        else {
            continue;
        };
        if parts.is_empty() {
            for part in 0..all_parts {
                add(false, cycle, part, slot.slot, entry.republish);
            }
        } else {
            for part in parts.iter().filter(|part| **part < all_parts) {
                add(false, cycle, *part, slot.slot, entry.republish);
            }
        }
    }
    // The parts that fit the budget beside the summary, in order; the rest
    // are pending, and the drain publishes them.
    let room = publisher.room().saturating_sub(1);
    let byte_room = publisher.part_bytes();
    let mut bytes = 0_usize;
    let mut full = false;
    let mut changed_parts: Vec<(u32, Value, Vec<EvidenceRef>)> = Vec::new();
    let mut remaining = BTreeMap::<String, BTreeSet<u32>>::new();
    let mut remaining_forced = false;
    for ((not_leaving, cycle, part), (slot, force)) in owed {
        full |= changed_parts.len() >= room;
        if full {
            remaining.entry(cycle).or_default().insert(part);
            remaining_forced |= force;
            continue;
        }
        let read = if not_leaving {
            part_rows(working, reads, &observer, &cycle, part)?
        } else {
            PartRows::default()
        };
        let entry_hex = part_entry(&read.rows, &mut reads.digest);
        let stored = observer
            .report
            .cycles
            .get(&cycle)
            .map_or(EMPTY_PART_ENTRY, |slot| slot.entry(part))
            .to_owned();
        if !force && stored == entry_hex {
            continue;
        }
        let books_summed = match observer.binding.granularity {
            FiscalReportGranularity::Scope => None,
            FiscalReportGranularity::Institution => Some(read.books_summed),
        };
        let payload = part_payload(observer_id, &cycle, slot, part, read.rows, books_summed)?;
        let size = crate::settle::json_len(&payload)?;
        if bytes.saturating_add(size) > byte_room {
            full = true;
            remaining.entry(cycle).or_default().insert(part);
            remaining_forced |= force;
            continue;
        }
        bytes += size;
        if let Some(cycle_slot) = observer.report.cycles.get_mut(&cycle) {
            cycle_slot.set_entry(part, &entry_hex);
        }
        let evidence = if read.evidence.is_empty() {
            vec![EvidenceRef::DomainRecordVersion(source.clone())]
        } else {
            read.evidence
        };
        changed_parts.push((part_line(slot, part), payload, evidence));
    }
    // A leaving cycle whose parts are all withdrawn frees its slot.
    observer
        .report
        .cycles
        .retain(|cycle, slot| !(leaving.contains(cycle) && slot.is_empty()));
    // A report with nothing to show is not published: an observer whose
    // institutions hold no hot book yet is told nothing. An interval report
    // that was published before is confirmed at every due time, its summary
    // alone when nothing changed, so that its `as_of` advances.
    let confirm = observer.binding.cadence.is_interval() && observer.report.as_of.is_some();
    if !changed_parts.is_empty() || confirm {
        observer.report.as_of = Some(publisher.at);
        let shown: BTreeMap<String, u16> = observer
            .report
            .cycles
            .iter()
            .filter(|(cycle, _)| visible.contains(*cycle))
            .map(|(cycle, slot)| (cycle.clone(), slot.slot))
            .collect();
        let fact_count = observer
            .report
            .cycles
            .iter()
            .filter(|(cycle, _)| visible.contains(*cycle))
            .map(|(_, slot)| slot.fact_count())
            .fold(0_u32, u32::saturating_add);
        let summary = FiscalReportSummary {
            observer_id: observer_id.to_owned(),
            actor: observer.binding.actor,
            as_of: publisher.at,
            confidence_per_mille: observer.binding.confidence_per_mille,
            granularity: observer.binding.granularity,
            cadence: observer.binding.cadence,
            cycles: shown,
            fact_count,
            pending: remaining
                .iter()
                .filter(|(cycle, _)| visible.contains(*cycle))
                .map(|(cycle, parts)| (cycle.clone(), parts.clone()))
                .collect(),
            omitted_cycles: u32::try_from(observer.report.omitted_cycles.len()).unwrap_or(u32::MAX),
            withdrawn: false,
        };
        let payload = summary_payload(&summary)?;
        bytes += crate::settle::json_len(&payload)?;
        let mut records = Vec::with_capacity(changed_parts.len() + 1);
        records.push(publisher.draft(
            &holder,
            &observer,
            SUMMARY_LINE,
            payload,
            vec![EvidenceRef::DomainRecordVersion(source.clone())],
        )?);
        publisher.progress |= !changed_parts.is_empty();
        for (line, payload, evidence) in changed_parts {
            records.push(publisher.draft(&holder, &observer, line, payload, evidence)?);
        }
        publisher.publish(&holder, records, bytes)?;
    }
    // A cycle left out for want of a slot takes the slot a leaving cycle
    // freed in the next pass, which the boundary's drain brings at once.
    if remaining.is_empty()
        && !observer.report.omitted_cycles.is_empty()
        && observer.report.cycles.len() < MAX_FISCAL_REPORT_CYCLE_SLOTS
    {
        remaining.extend(
            observer
                .report
                .omitted_cycles
                .iter()
                .map(|cycle| (cycle.clone(), BTreeSet::new())),
        );
    }
    finish(working, observer);
    if remaining.is_empty() {
        return Ok(true);
    }
    // What is left of a large report can name more parts than queued work
    // may; owing the cycle whole instead publishes the same parts, since the
    // ones published here now match their digests.
    let mut next = FiscalReportQueueEntry {
        due_at: entry.due_at,
        cycles: remaining,
        whole: false,
        republish: remaining_forced,
        parked: false,
    };
    bound_queue_entry(&mut next);
    *entry = next;
    Ok(false)
}

/// What the rows of one scope are made of in one cycle: its mechanism and
/// its book's totals. Its institution is its office's, and its ledger
/// version is read for a scope row when the row cites it.
struct ScopeBook {
    mechanism: FiscalMechanism,
    totals: Vec<FiscalCycleTotal>,
}

/// One institution's hot books of one cycle summed per mechanism: the totals
/// per (unit, payment form), and how many books with an assessment were
/// summed.
type InstitutionCycle = BTreeMap<FiscalMechanism, (Vec<FiscalCycleTotal>, u32)>;

/// What one run of the report system has read, shared by every observer it
/// publishes to: every observer of an office reads the same books.
#[derive(Default)]
pub(crate) struct ReportReads {
    /// Each scope's book of each cycle, by cycle and scope; `None` when the
    /// scope has no book of the cycle.
    books: BTreeMap<String, BTreeMap<String, Option<ScopeBook>>>,
    /// Each office's books of each cycle summed, by cycle and office key.
    institutions: BTreeMap<String, BTreeMap<String, InstitutionCycle>>,
    /// Each office record's current version, by office key.
    offices: BTreeMap<String, Option<DomainRecordVersionRef>>,
    /// The reference of a ledger record, reused for every scope read
    /// ([`ledger_reference`]).
    ledger: Option<DomainRecordRef>,
    /// The buffer part digests are encoded into.
    digest: Vec<u8>,
}

/// The rows of one part, the versions they cite, and, for institution rows,
/// the books they were summed from.
#[derive(Default)]
struct PartRows {
    rows: Vec<FiscalReportRow>,
    evidence: Vec<EvidenceRef>,
    books_summed: u32,
}

/// The rows of part `part` of `cycle` in an observer's report, and what
/// they cite: the versions of the ledgers scope rows were read from, or the
/// current version of the office of each institution row with the books it
/// summed. A scope's row is read from its ledger record's book of the cycle
/// without decoding the rest; an office's index pages are decoded once per
/// boundary.
///
/// A scope row's institution is its block's: an office's index lists only
/// the scopes of the office's institution, and is never rewritten.
fn part_rows(
    working: &mut Working<'_, SimulationView<'_>>,
    reads: &mut ReportReads,
    observer: &FiscalObserver,
    cycle: &str,
    part: u32,
) -> Result<PartRows, CanwuError> {
    let per_part = u32::try_from(FISCAL_REPORT_ROWS_PER_PART).unwrap_or(u32::MAX);
    let first = part.saturating_mul(per_part);
    let end = first.saturating_add(per_part).min(observer.row_count());
    let granularity = observer.binding.granularity;
    let confidence = observer.binding.confidence_per_mille;
    let mut read = PartRows::default();
    for block in &observer.rows {
        let block_end = block.first_row.saturating_add(block.span(granularity));
        let (low, high) = (first.max(block.first_row), end.min(block_end));
        if low >= high {
            continue;
        }
        match granularity {
            FiscalReportGranularity::Scope => {
                let owner = FiscalIndexOwner::Office(block.office.clone());
                let ranks = low - block.first_row..high - block.first_row;
                working.read_index_pages(&owner, ranks.clone())?;
                for (rank, scope) in working.index_scopes_in(&owner, ranks)? {
                    let Some(book) = scope_book(working, reads, scope, cycle)? else {
                        continue;
                    };
                    let Some(row) = rendered_row(
                        block.first_row + rank,
                        &block.institution,
                        book.mechanism,
                        scope,
                        &book.totals,
                        confidence,
                    ) else {
                        continue;
                    };
                    read.rows.push(row);
                    read.evidence.extend(
                        working
                            .source()
                            .current_domain_record_version(ledger_reference(
                                &mut reads.ledger,
                                scope,
                            ))?
                            .map(EvidenceRef::DomainRecordVersion),
                    );
                }
            }
            FiscalReportGranularity::Institution => {
                institution_cycle(working, reads, &block.office, block.scopes, cycle)?;
                let sums = &reads.institutions[cycle][&block.office];
                let mut cited = false;
                for row in low..high {
                    let Some(mechanism) = usize::try_from(row - block.first_row)
                        .ok()
                        .and_then(|index| block.mechanisms.get(index))
                    else {
                        continue;
                    };
                    let Some((totals, books)) = sums.get(mechanism) else {
                        continue;
                    };
                    if let Some(row) = institution_row_of_totals(
                        row,
                        &block.institution,
                        *mechanism,
                        totals,
                        confidence,
                    ) {
                        read.rows.push(row);
                        read.books_summed = read.books_summed.saturating_add(*books);
                        cited = true;
                    }
                }
                if cited {
                    if !reads.offices.contains_key(&block.office) {
                        let version = working.source().current_domain_record_version(
                            office_reference(&block.office).as_untyped(),
                        )?;
                        reads.offices.insert(block.office.clone(), version);
                    }
                    read.evidence.extend(
                        reads.offices[&block.office]
                            .clone()
                            .map(EvidenceRef::DomainRecordVersion),
                    );
                }
            }
        }
    }
    // Rows follow the observer's offices and each office's ranks; a part
    // over two offices names their records out of ID order, and the engine
    // takes a record's evidence only sorted.
    read.evidence.sort();
    read.evidence.dedup();
    Ok(read)
}

/// The reference of `scope`'s ledger record, written into `slot`, which is
/// reused for every scope a run of the report system reads.
fn ledger_reference<'s>(slot: &'s mut Option<DomainRecordRef>, scope: &str) -> &'s DomainRecordRef {
    let reference = slot.get_or_insert_with(|| fiscal_ledger_reference("").into_untyped());
    reference.id.clear();
    reference.id.push_str(scope);
    reference
}

/// A scope's book of one cycle, read once per run of the report system.
fn scope_book<'r>(
    working: &Working<'_, SimulationView<'_>>,
    reads: &'r mut ReportReads,
    scope: &str,
    cycle: &str,
) -> Result<Option<&'r ScopeBook>, CanwuError> {
    if !reads.books.contains_key(cycle) {
        reads.books.insert(cycle.to_owned(), BTreeMap::new());
    }
    let books = reads
        .books
        .get_mut(cycle)
        .ok_or_else(|| invalid("fiscal report reads lost a cycle"))?;
    if !books.contains_key(scope) {
        let read = ledger_book(working, ledger_reference(&mut reads.ledger, scope), cycle)?;
        books.insert(scope.to_owned(), read);
    }
    Ok(books.get(scope).and_then(Option::as_ref))
}

/// Sums an office's hot books of one cycle per mechanism, once per run of
/// the report system: every scope's book of the cycle is read through the
/// scope reads that scope rows share.
fn institution_cycle(
    working: &mut Working<'_, SimulationView<'_>>,
    reads: &mut ReportReads,
    office: &str,
    scopes: u32,
    cycle: &str,
) -> Result<(), CanwuError> {
    if reads
        .institutions
        .get(cycle)
        .is_some_and(|offices| offices.contains_key(office))
    {
        return Ok(());
    }
    let owner = FiscalIndexOwner::Office(office.to_owned());
    working.read_index_pages(&owner, 0..scopes)?;
    let mut sums = InstitutionCycle::new();
    for (_, scope) in working.index_scopes_in(&owner, 0..scopes)? {
        let Some(book) = scope_book(working, reads, scope, cycle)? else {
            continue;
        };
        if book.totals.is_empty() {
            continue;
        }
        let (sum, books) = sums.entry(book.mechanism).or_default();
        sum_report_totals(sum, &book.totals);
        *books = books.saturating_add(1);
    }
    reads
        .institutions
        .entry(cycle.to_owned())
        .or_default()
        .insert(office.to_owned(), sums);
    Ok(())
}

/// A scope's mechanism and its book's totals of one cycle, read from its
/// ledger record without decoding it: the scope's mechanism, and of the
/// book only what its totals are summed from ([`BookFigures`]).
fn ledger_book(
    working: &Working<'_, SimulationView<'_>>,
    reference: &DomainRecordRef,
    cycle: &str,
) -> Result<Option<ScopeBook>, CanwuError> {
    if let Some(ledger) = working.state.ledgers.get(&reference.id) {
        return ledger
            .books
            .get(cycle)
            .map(|book| {
                Ok(ScopeBook {
                    mechanism: ledger.scope.mechanism,
                    totals: book.totals(ledger.scope.mechanism)?,
                })
            })
            .transpose();
    }
    let Some(record) = working
        .source()
        .fiscal_record(reference)?
        .filter(|record| record.lifecycle == DomainRecordLifecycle::Active)
    else {
        return Ok(None);
    };
    let Some(book) = record
        .payload
        .get("books")
        .and_then(|books| books.get(cycle))
    else {
        return Ok(None);
    };
    let decode = |error: serde_json::Error| invalid(format!("fiscal ledger is invalid: {error}"));
    let mechanism = record
        .payload
        .get("scope")
        .and_then(|scope| scope.get("mechanism"))
        .map(FiscalMechanism::deserialize)
        .transpose()
        .map_err(decode)?
        .ok_or_else(|| invalid("fiscal ledger has no scope binding"))?;
    let totals = BookFigures::deserialize(book)
        .map_err(decode)?
        .totals(mechanism)?;
    Ok(Some(ScopeBook { mechanism, totals }))
}

/// Checks the holders of the next [`FISCAL_PARKED_CHECKS_PER_BOUNDARY`]
/// parked entries after the root's cursor, in order of (queue
/// page, observer binding ID), wrapping round, and unparks each whose holder
/// can receive knowledge again or whose binding was removed; it is then due,
/// since a parked entry was due when it was parked. Moves the cursor to the
/// last entry checked, or clears it when the run checked every parked entry,
/// and returns the pages of the entries it unparked.
///
/// A check reads the observer's binding and, for a person, the person's
/// availability: a fixed cost per run, however many observers are parked.
/// With more parked entries than one run checks, every run moves the cursor
/// and so writes the root.
fn check_parked(
    working: &mut Working<'_, SimulationView<'_>>,
) -> Result<BTreeSet<String>, CanwuError> {
    let schedule = &working.state.report_schedule;
    let cursor = working.state.root.parked_cursor.clone();
    let parked = schedule
        .parked_pages
        .values()
        .map(|count| usize::try_from(*count).unwrap_or(usize::MAX))
        .fold(0_usize, usize::saturating_add);
    let checks = parked.min(FISCAL_PARKED_CHECKS_PER_BOUNDARY);
    if checks == 0 {
        return Ok(BTreeSet::new());
    }
    // The pages in rotation order: the cursor's page (its entries after the
    // cursor), the pages after it, the pages before it, and the cursor's
    // page again (its entries up to the cursor).
    let start = cursor.as_deref().map(report_queue_page);
    let mut order: Vec<(String, Option<bool>)> = Vec::new();
    match &start {
        Some(start) => {
            if schedule.parked_pages.contains_key(start) {
                order.push((start.clone(), Some(true)));
            }
            order.extend(
                schedule
                    .parked_pages
                    .keys()
                    .filter(|page| *page > start)
                    .chain(schedule.parked_pages.keys().filter(|page| *page < start))
                    .map(|page| (page.clone(), None)),
            );
            if schedule.parked_pages.contains_key(start) {
                order.push((start.clone(), Some(false)));
            }
        }
        None => order.extend(
            schedule
                .parked_pages
                .keys()
                .map(|page| (page.clone(), None)),
        ),
    }
    let mut checked: Vec<(String, String)> = Vec::new();
    for (page, after_cursor) in order {
        if checked.len() >= checks {
            break;
        }
        let entries = &working.queue_page(&page)?.entries;
        for (observer, _) in entries.iter().filter(|(observer, entry)| {
            entry.parked
                && match (after_cursor, cursor.as_deref()) {
                    (Some(true), Some(cursor)) => observer.as_str() > cursor,
                    (Some(false), Some(cursor)) => observer.as_str() <= cursor,
                    _ => true,
                }
        }) {
            if checked.len() >= checks {
                break;
            }
            checked.push((page.clone(), observer.clone()));
        }
    }
    let view = working.source();
    let mut unparked = BTreeSet::new();
    for (page, observer) in &checked {
        let ready = match active_record(view, &fiscal_observer_reference(observer))? {
            Some(record) => {
                let binding = record
                    .payload
                    .get("binding")
                    .map(FiscalObserverBinding::deserialize)
                    .transpose()
                    .map_err(|error| invalid(format!("fiscal observer is invalid: {error}")))?
                    .ok_or_else(|| invalid("fiscal observer has no binding"))?;
                !binding.is_active() || !holder_is_unreachable(view, &binding.knowledge_holder)?
            }
            None => true,
        };
        if ready && let Some(entry) = working.queue_page(page)?.entries.get_mut(observer) {
            entry.parked = false;
            unparked.insert(page.clone());
        }
    }
    // A run that checked every parked entry leaves no cursor, so that a few
    // parked entries cost no write in a boundary that writes nothing else.
    let moved = if checked.len() < parked {
        checked.pop().map(|(_, last)| last)
    } else {
        None
    };
    if working.state.root.parked_cursor != moved {
        working.state.root.parked_cursor = moved;
        working.touch(fiscal_root_reference().into_untyped());
    }
    Ok(unparked)
}

/// The payload of a summary record, built directly: what
/// `serde_json::to_value(FiscalReportRecord::Summary(summary))` gives,
/// without serializing through the record.
fn summary_payload(summary: &FiscalReportSummary) -> Result<Value, CanwuError> {
    let mut map = serde_json::Map::new();
    map.insert("type".to_owned(), Value::from("summary"));
    map.insert(
        "observer_id".to_owned(),
        Value::from(summary.observer_id.clone()),
    );
    map.insert("actor".to_owned(), json(&summary.actor)?);
    map.insert("as_of".to_owned(), json(&summary.as_of)?);
    map.insert(
        "confidence_per_mille".to_owned(),
        Value::from(summary.confidence_per_mille),
    );
    if summary.granularity != FiscalReportGranularity::default() {
        map.insert("granularity".to_owned(), json(&summary.granularity)?);
    }
    if summary.cadence.is_interval() {
        map.insert("cadence".to_owned(), json(&summary.cadence)?);
    }
    if !summary.cycles.is_empty() {
        map.insert(
            "cycles".to_owned(),
            Value::Object(
                summary
                    .cycles
                    .iter()
                    .map(|(cycle, slot)| (cycle.clone(), Value::from(*slot)))
                    .collect(),
            ),
        );
    }
    map.insert("fact_count".to_owned(), Value::from(summary.fact_count));
    if !summary.pending.is_empty() {
        map.insert(
            "pending".to_owned(),
            Value::Object(
                crate::model::part_bitmap_entries(&summary.pending)
                    .into_iter()
                    .map(|(cycle, bitmap)| (cycle, Value::from(bitmap)))
                    .collect(),
            ),
        );
    }
    if summary.omitted_cycles != 0 {
        map.insert(
            "omitted_cycles".to_owned(),
            Value::from(summary.omitted_cycles),
        );
    }
    if summary.withdrawn {
        map.insert("withdrawn".to_owned(), Value::Bool(true));
    }
    Ok(Value::Object(map))
}

/// The payload of a part record, built directly from its rows, which it
/// takes: what `serde_json::to_value(FiscalReportRecord::Part(part))` gives,
/// without the intermediate record or copies of the rows. A part is encoded
/// compactly (see [`crate::FiscalReportPart`]): each institution once, and
/// each row and fact as an array, so its value holds a few arrays per row
/// rather than an object per row, fact, and amount.
fn part_payload(
    observer_id: &str,
    cycle: &str,
    slot: u16,
    part: u32,
    rows: Vec<FiscalReportRow>,
    books_summed: Option<u32>,
) -> Result<Value, CanwuError> {
    let mut map = serde_json::Map::new();
    map.insert("type".to_owned(), Value::from("part"));
    map.insert("observer_id".to_owned(), Value::from(observer_id));
    map.insert("accounting_cycle_id".to_owned(), Value::from(cycle));
    map.insert("slot".to_owned(), Value::from(slot));
    map.insert("part".to_owned(), Value::from(part));
    if !rows.is_empty() {
        let mut institutions: Vec<canwu_api::EntityRef> = Vec::new();
        let mut table = Vec::new();
        let mut encoded = Vec::with_capacity(rows.len());
        for row in rows {
            let known = institutions
                .iter()
                .position(|institution| *institution == row.institution);
            let index = if let Some(index) = known {
                index
            } else {
                table.push(json(&row.institution)?);
                institutions.push(row.institution);
                institutions.len() - 1
            };
            let mut facts = Vec::with_capacity(row.facts.len());
            for fact in row.facts {
                facts.push(Value::Array(vec![
                    Value::from(fact.unit),
                    json(&fact.payment_form)?,
                    Value::from(fact.assessed.minimum),
                    Value::from(fact.assessed.maximum),
                    Value::from(fact.collected.minimum),
                    Value::from(fact.collected.maximum),
                    Value::from(fact.outstanding.minimum),
                    Value::from(fact.outstanding.maximum),
                ]));
            }
            encoded.push(Value::Array(vec![
                Value::from(row.row),
                Value::from(u32::try_from(index).unwrap_or(u32::MAX)),
                json(&row.mechanism)?,
                Value::from(row.scope_binding_id),
                Value::Array(facts),
            ]));
        }
        map.insert("institutions".to_owned(), Value::Array(table));
        map.insert("rows".to_owned(), Value::Array(encoded));
    }
    if let Some(books) = books_summed {
        map.insert("books_summed".to_owned(), Value::from(books));
    }
    Ok(Value::Object(map))
}

/// One small value as JSON.
fn json<T: serde::Serialize + ?Sized>(value: &T) -> Result<Value, CanwuError> {
    serde_json::to_value(value).map_err(encode_error)
}

/// Reads the fiscal reports the viewer's holder currently holds, keyed by
/// observer binding ID, and puts each back together from its records.
///
/// A holder has one report for each observer binding it holds. Each is what
/// the binding lets the holder see and nothing else; see
/// [`assemble_fiscal_reports`] for how the records are assembled. A report
/// withdrawn from the holder is left out.
pub fn fiscal_reports(
    viewer: &CanwuViewer<'_>,
) -> Result<BTreeMap<String, FiscalReport>, CanwuError> {
    let mut query = KnowledgeQuery {
        schemas: vec![fiscal_report_knowledge_schema_id()],
        limit: MAX_KNOWLEDGE_PAGE_SIZE,
        ..KnowledgeQuery::default()
    };
    let mut payloads = Vec::new();
    loop {
        let page = viewer.query_knowledge(&query)?;
        payloads.extend(page.records.into_iter().map(|record| record.payload));
        let Some(cursor) = page.next else {
            break;
        };
        query.after = Some(cursor);
    }
    assemble_fiscal_reports(&payloads)
}

/// Reads the fiscal reports one holder currently holds, as a trusted host:
/// [`fiscal_reports`] for any holder, a government included.
pub fn fiscal_reports_of_holder<H: CanwuRun + ?Sized>(
    canwu: &H,
    holder: &KnowledgeHolderRef,
) -> Result<BTreeMap<String, FiscalReport>, CanwuError> {
    let mut query = KnowledgeQuery {
        schemas: vec![fiscal_report_knowledge_schema_id()],
        limit: MAX_KNOWLEDGE_PAGE_SIZE,
        ..KnowledgeQuery::default()
    };
    let mut payloads = Vec::new();
    loop {
        let page = canwu.admin_query_knowledge(holder.clone(), &query)?;
        payloads.extend(page.records.into_iter().map(|record| record.payload));
        let Some(cursor) = page.next else {
            break;
        };
        query.after = Some(cursor);
    }
    assemble_fiscal_reports(&payloads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        FiscalAmountEstimate, FiscalPaymentForm, FiscalReportCadence, FiscalReportPart,
        FiscalReportRecord, FiscalReportRowFact,
    };
    use canwu_api::{DomainRecordRef, EntityRef, OrganizationId, PersonId, SimDuration};

    fn row(row: u32, scope: &str) -> FiscalReportRow {
        let range = |minimum| FiscalAmountEstimate {
            minimum,
            maximum: minimum + 9,
        };
        FiscalReportRow {
            row,
            institution: if scope.is_empty() {
                EntityRef::Domain(DomainRecordRef::new("app", "office", "o.1"))
            } else {
                EntityRef::Organization(OrganizationId::new(3))
            },
            mechanism: FiscalMechanism::SaltMonopoly,
            scope_binding_id: scope.to_owned(),
            facts: vec![
                FiscalReportRowFact {
                    unit: "grain".to_owned(),
                    payment_form: FiscalPaymentForm::Grain,
                    assessed: range(100),
                    collected: range(40),
                    outstanding: range(60),
                },
                FiscalReportRowFact {
                    unit: "silver".to_owned(),
                    payment_form: FiscalPaymentForm::Silver,
                    assessed: range(u64::MAX - 9),
                    collected: range(0),
                    outstanding: range(7),
                },
            ],
        }
    }

    /// The payloads the report system builds directly are exactly what the
    /// records serialize to, so a holder decodes them as
    /// [`FiscalReportRecord`] and a field added to the record must be added
    /// to the encoder as well.
    #[test]
    fn direct_payloads_are_the_serialized_records() {
        let summaries = [
            FiscalReportSummary {
                observer_id: "observer.court".to_owned(),
                actor: Some(PersonId::new(4)),
                as_of: SimTime::from_minutes(1_441),
                confidence_per_mille: 850,
                granularity: FiscalReportGranularity::Institution,
                cadence: FiscalReportCadence::Interval {
                    every: SimDuration::days(30),
                    offset: SimDuration::days(2),
                },
                cycles: BTreeMap::from([("1644".to_owned(), 0), ("1645".to_owned(), 63)]),
                fact_count: 12,
                pending: BTreeMap::from([("1645".to_owned(), BTreeSet::from([0, 9, 1_023]))]),
                omitted_cycles: 2,
                withdrawn: false,
            },
            FiscalReportSummary {
                observer_id: "observer.official".to_owned(),
                actor: None,
                as_of: SimTime::EPOCH,
                confidence_per_mille: 1_000,
                granularity: FiscalReportGranularity::Scope,
                cadence: FiscalReportCadence::OnChange,
                cycles: BTreeMap::new(),
                fact_count: 0,
                pending: BTreeMap::new(),
                omitted_cycles: 0,
                withdrawn: true,
            },
        ];
        for summary in summaries {
            assert_eq!(
                summary_payload(&summary).expect("summary payload"),
                serde_json::to_value(FiscalReportRecord::Summary(summary)).expect("summary")
            );
        }
        for (rows, books_summed) in [
            (vec![row(16, "scope.a"), row(17, "scope.b")], None),
            (vec![row(32, ""), row(47, "")], Some(6_000)),
            // Two institutions: the table lists each once, in first use.
            (
                vec![row(16, "scope.a"), row(32, ""), row(48, "scope.c")],
                None,
            ),
            (Vec::new(), Some(0)),
            (Vec::new(), None),
        ] {
            let part = FiscalReportPart {
                observer_id: "observer.court".to_owned(),
                accounting_cycle_id: "1644".to_owned(),
                slot: 3,
                part: 2,
                rows: rows.clone(),
                books_summed,
            };
            let payload = part_payload("observer.court", "1644", 3, 2, rows, books_summed)
                .expect("part payload");
            assert_eq!(
                payload,
                serde_json::to_value(FiscalReportRecord::Part(part.clone())).expect("part")
            );
            assert_eq!(
                serde_json::from_value::<FiscalReportRecord>(payload).expect("decoded part"),
                FiscalReportRecord::Part(part)
            );
        }
    }
}
