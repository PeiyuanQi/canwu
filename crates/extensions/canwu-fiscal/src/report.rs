//! Fiscal reports: the system that publishes them (phase 13), their report
//! lines, and the reads that put them back together.
//!
//! Settlement queues, for each observer, the report parts its boundary's
//! changes touch. The report system takes the queue in order, computes only
//! the parts each entry names from the ledgers they cover, compares each
//! part's digest with the one the observer record holds, and publishes the
//! summary and the parts that changed. What does not fit the boundary's
//! publication limits stays queued, and the boundary schedules an internal
//! drain packet so that the next boundary at the same instant continues.

use crate::derive::assemble_fiscal_reports;
use crate::derive::{part_entry, parts_of, report_row_of_totals};
use crate::model::{
    CompiledFiscalCatalog, EMPTY_PART_ENTRY, FISCAL_REPORT_ROWS_PER_PART, FiscalCycleBook,
    FiscalCycleTotal, FiscalObserver, FiscalReport, FiscalReportCycleSlot, FiscalReportPart,
    FiscalReportQueueEntry, FiscalReportRecord, FiscalReportRow, FiscalReportSummary,
    FiscalScopeBinding, MAX_FISCAL_REPORT_CYCLE_SLOTS, MAX_FISCAL_REPORT_PARTS_PER_CYCLE,
    MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY, invalid,
};
use crate::plugin::{FISCAL_REPORT_DRAIN_INGRESS, PLUGIN_NAME, encode_error};
use crate::records::{
    FiscalIndexOwner, FiscalObserverRecord, fiscal_ledger_reference, fiscal_observer_reference,
    fiscal_root_reference,
};
use crate::working::{Working, active_record, bound_queue_entry, field};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryProposal, CanwuError, CanwuRun, CanwuViewer,
    DomainRecordKind, DomainRecordVersionRef, EvidenceRef, KnowledgeHolderRef, KnowledgeLimitsV1,
    KnowledgeOrigin, KnowledgeQuery, KnowledgeRecordDraft, KnowledgeRecordKind, KnowledgeSchemaId,
    KnowledgeSubject, KnowledgeSubjectSchema, KnowledgeSubjectTarget, KnowledgeSubjectTargetKind,
    LifeState, MAX_KNOWLEDGE_PAGE_SIZE, PayloadSchema, PluginKnowledgeSchema, SimTime,
    SimulationView, StateVisibility,
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
/// summary.
const FISCAL_REPORT_SCHEMA_VERSION: u32 = 4;
const FISCAL_REPORT_SCHEMA_HASH: &str =
    "e4264ff89f0988b3cec589587ba23313a7cc8609be6ee557491e7adcf6eecef6";
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
/// A record of this schema is one [`FiscalReportRecord`]: the summary or one
/// part of one observer's report. Use [`fiscal_reports`] to read whole
/// reports.
#[must_use]
pub fn fiscal_report_knowledge_schema_id() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new(PLUGIN_NAME, FISCAL_REPORT_KNOWLEDGE),
        FISCAL_REPORT_SCHEMA_VERSION,
    )
}

pub(crate) fn fiscal_report_knowledge_schema() -> PluginKnowledgeSchema {
    PluginKnowledgeSchema {
        id: fiscal_report_knowledge_schema_id(),
        schema_hash: FISCAL_REPORT_SCHEMA_HASH.to_owned(),
        writable: true,
        payload_schema: PayloadSchema::Any,
        subjects: report_subject_schemas(),
        retention: canwu_api::KnowledgeRetention::Keep,
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
fn drain_directive() -> BoundaryDirective {
    BoundaryDirective::ScheduleIngress {
        after: canwu_api::SimDuration::ZERO,
        packet_type: FISCAL_REPORT_DRAIN_INGRESS.to_owned(),
        priority: 0,
        payload: Value::Object(serde_json::Map::new()),
        affected: Vec::new(),
    }
}

/// The records one boundary publishes, within its limits.
struct Publisher<'a, 'v> {
    view: &'a SimulationView<'v>,
    kind: KnowledgeRecordKind,
    at: SimTime,
    boundary: u64,
    publications_left: usize,
    records_left: usize,
    directives: Vec<BoundaryDirective>,
}

impl Publisher<'_, '_> {
    /// How many records the next publication may carry.
    fn room(&self) -> usize {
        if self.publications_left == 0 {
            0
        } else {
            self.records_left
                .min(KnowledgeLimitsV1::CURRENT.records_per_batch)
        }
    }

    /// The draft of one record on one line of an observer, superseding what
    /// the holder's line holds now.
    fn draft(
        &self,
        holder: &KnowledgeHolderRef,
        observer: &FiscalObserver,
        line: u32,
        record: &FiscalReportRecord,
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
            payload: serde_json::to_value(record).map_err(encode_error)?,
            as_of: Some(self.at),
            confidence_per_mille: observer.binding.confidence_per_mille,
            origin: KnowledgeOrigin {
                method: "fiscal_authority_report_v4".to_owned(),
                evidence,
            },
            supersedes,
            contradicts: Vec::new(),
        })
    }

    /// Publishes the records to one holder, under a correlation no other
    /// publication of the boundary has.
    fn publish(
        &mut self,
        holder: &KnowledgeHolderRef,
        records: Vec<KnowledgeRecordDraft>,
        correlation: String,
        summary: &str,
    ) {
        self.publications_left = self.publications_left.saturating_sub(1);
        self.records_left = self.records_left.saturating_sub(records.len());
        self.directives.push(BoundaryDirective::PublishKnowledge {
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some(correlation),
            supersession: canwu_api::KnowledgeSupersession::Explicit,
            groups: vec![canwu_api::KnowledgePublicationGroup {
                holders: vec![holder.clone()],
                records,
            }],
            summary: summary.to_owned(),
        });
    }
}

/// Publishes the report work the queue owes, in order of due time and
/// observer binding ID, within the boundary's publication limits.
///
/// A boundary in which nothing is queued returns after one read of the root.
/// An observer whose holder cannot receive knowledge is parked: its work
/// stays queued, and the report system checks the holder again in every
/// boundary that runs it, so a revived holder is published to in the
/// boundary that revives him. Work that does not fit the limits stays queued,
/// and the boundary schedules an internal [`FISCAL_REPORT_DRAIN_INGRESS`]
/// packet, due at once.
///
/// The record a new one supersedes is read from the committed knowledge
/// ledger with `SimulationView::knowledge_heads`, never from retained
/// evidence, so a run that sealed its evidence supersedes exactly what its
/// replay does.
pub(crate) fn materialize_reports(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    catalog: impl FnOnce() -> Result<Option<CompiledFiscalCatalog>, CanwuError>,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(root) = active_record(view, &fiscal_root_reference())? else {
        return Ok(BoundaryProposal::default());
    };
    let pages: BTreeSet<String> = field(&root.payload, "queue_pages")?;
    if pages.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    let Some(catalog) = catalog()? else {
        return Ok(BoundaryProposal::default());
    };
    let Some(mut working) = Working::new(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let mut queued = Vec::new();
    for page in &pages {
        for (observer, entry) in &working.queue_page(page)?.entries {
            queued.push((entry.due_at, observer.clone(), page.clone()));
        }
    }
    queued.sort();
    let mut publisher = Publisher {
        view,
        kind: fiscal_report_knowledge_schema_id().kind,
        at: context.at,
        boundary: context.boundary_id.get(),
        publications_left: 64,
        records_left: MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY,
        directives: Vec::new(),
    };
    let mut owes_more = false;
    let mut cache = RowCache::new();
    for (due_at, observer_id, page) in queued {
        if due_at > context.at {
            continue;
        }
        // Once the boundary can publish nothing more, the observers after
        // this one keep their queued work, untouched, for the drain.
        if publisher.room() == 0 {
            owes_more |= working
                .queue_page(&page)?
                .entries
                .get(&observer_id)
                .is_some_and(|entry| !entry.parked);
            continue;
        }
        let Some(mut entry) = working.queue_page(&page)?.entries.remove(&observer_id) else {
            continue;
        };
        let done = if working.ensure_observer(&observer_id)? {
            publish_observer(
                &mut working,
                &mut cache,
                &mut publisher,
                &observer_id,
                &mut entry,
            )?
        } else {
            true
        };
        if !done {
            owes_more |= !entry.parked;
            working
                .queue_page(&page)?
                .entries
                .insert(observer_id, entry);
        }
    }
    let mut directives = working.directives(&catalog, "Published fiscal reports")?;
    directives.append(&mut publisher.directives);
    if owes_more && view.can_schedule_ingress() {
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
/// knowledge now.
#[allow(clippy::too_many_lines)]
fn publish_observer(
    working: &mut Working<'_, SimulationView<'_>>,
    cache: &mut RowCache,
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
    let original = observer.clone();
    let source = view
        .current_domain_record_version(fiscal_observer_reference(observer_id).as_untyped())?
        .ok_or_else(|| invalid("fiscal observer record has no current version"))?;
    // Withdrawn summaries for the holders the binding left.
    let mut done = true;
    for holder in observer.report.withdraw.clone() {
        if publisher.room() == 0 {
            done = false;
            break;
        }
        observer.report.withdraw.remove(&holder);
        // A holder that can no longer receive knowledge keeps what it holds.
        if holder_is_unreachable(view, &holder)? {
            continue;
        }
        let summary = FiscalReportRecord::Summary(FiscalReportSummary {
            observer_id: observer_id.to_owned(),
            actor: match &holder {
                KnowledgeHolderRef::Person(person) => Some(*person),
                KnowledgeHolderRef::Entity(_) => None,
            },
            as_of: publisher.at,
            confidence_per_mille: observer.binding.confidence_per_mille,
            cycles: BTreeMap::new(),
            fact_count: 0,
            pending: BTreeMap::new(),
            omitted_cycles: 0,
            withdrawn: true,
        });
        let record = publisher.draft(
            &holder,
            &observer,
            SUMMARY_LINE,
            &summary,
            vec![EvidenceRef::DomainRecordVersion(source.clone())],
        )?;
        let index = observer.report.withdraw.len();
        publisher.publish(
            &holder,
            vec![record],
            format!(
                "fiscal-report-withdrawal::{observer_id}::{index}::{}",
                publisher.boundary
            ),
            "Withdraw one fiscal report from a holder",
        );
    }
    let finish = |working: &mut Working<'_, SimulationView<'_>>, observer: FiscalObserver| {
        if observer != original {
            working
                .state
                .observers
                .insert(observer_id.to_owned(), observer);
            working.touch(fiscal_observer_reference(observer_id).into_untyped());
        }
    };
    if !done {
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
    let room = publisher.room().saturating_sub(1);
    let mut changed_parts: Vec<(u32, FiscalReportRecord, Vec<EvidenceRef>)> = Vec::new();
    let mut remaining = BTreeMap::<String, BTreeSet<u32>>::new();
    let mut remaining_forced = false;
    for ((not_leaving, cycle, part), (slot, force)) in owed {
        if changed_parts.len() >= room {
            remaining.entry(cycle).or_default().insert(part);
            remaining_forced |= force;
            continue;
        }
        let (rows, evidence) = if not_leaving {
            part_rows(working, cache, &observer, &cycle, part)?
        } else {
            (Vec::new(), Vec::new())
        };
        let entry_hex = part_entry(&rows)?;
        let stored = observer
            .report
            .cycles
            .get(&cycle)
            .map_or(EMPTY_PART_ENTRY, |slot| slot.entry(part))
            .to_owned();
        if !force && stored == entry_hex {
            continue;
        }
        if let Some(cycle_slot) = observer.report.cycles.get_mut(&cycle) {
            cycle_slot.set_entry(part, &entry_hex);
        }
        let evidence = if evidence.is_empty() {
            vec![EvidenceRef::DomainRecordVersion(source.clone())]
        } else {
            evidence
        };
        changed_parts.push((
            part_line(slot, part),
            FiscalReportRecord::Part(FiscalReportPart {
                observer_id: observer_id.to_owned(),
                accounting_cycle_id: cycle,
                slot,
                part,
                rows,
            }),
            evidence,
        ));
    }
    // A leaving cycle whose parts are all withdrawn frees its slot.
    observer
        .report
        .cycles
        .retain(|cycle, slot| !(leaving.contains(cycle) && slot.is_empty()));
    // A report with nothing to show is not published: an observer whose
    // institutions hold no hot book yet is told nothing.
    if !changed_parts.is_empty() {
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
        let summary = FiscalReportRecord::Summary(FiscalReportSummary {
            observer_id: observer_id.to_owned(),
            actor: observer.binding.actor,
            as_of: publisher.at,
            confidence_per_mille: observer.binding.confidence_per_mille,
            cycles: shown,
            fact_count,
            pending: remaining
                .iter()
                .filter(|(cycle, _)| visible.contains(*cycle))
                .map(|(cycle, parts)| (cycle.clone(), parts.clone()))
                .collect(),
            omitted_cycles: u32::try_from(observer.report.omitted_cycles.len()).unwrap_or(u32::MAX),
            withdrawn: false,
        });
        let mut records = Vec::with_capacity(changed_parts.len() + 1);
        records.push(publisher.draft(
            &holder,
            &observer,
            SUMMARY_LINE,
            &summary,
            vec![EvidenceRef::DomainRecordVersion(source.clone())],
        )?);
        for (line, record, evidence) in &changed_parts {
            records.push(publisher.draft(&holder, &observer, *line, record, evidence.clone())?);
        }
        publisher.publish(
            &holder,
            records,
            format!("fiscal-report::{observer_id}::{}", publisher.boundary),
            "Publish the changed parts of one holder-relative fiscal report",
        );
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

/// What the rows of one scope are made of in one cycle: its binding, its
/// book's totals, and its ledger's current version; `None` when the scope
/// has no book of the cycle.
type ScopeCycle = Option<(
    FiscalScopeBinding,
    Vec<FiscalCycleTotal>,
    Option<DomainRecordVersionRef>,
)>;

/// The scope cycles one run of the report system has read, by scope and
/// cycle: every observer of an office reads the same books.
pub(crate) type RowCache = BTreeMap<(String, String), ScopeCycle>;

/// The rows of part `part` of `cycle` in an observer's report, and the
/// versions of the ledgers they were read from. A scope's row is read from
/// its ledger record's book of the cycle without decoding the rest.
fn part_rows(
    working: &mut Working<'_, SimulationView<'_>>,
    cache: &mut RowCache,
    observer: &FiscalObserver,
    cycle: &str,
    part: u32,
) -> Result<(Vec<FiscalReportRow>, Vec<EvidenceRef>), CanwuError> {
    let per_part = u32::try_from(FISCAL_REPORT_ROWS_PER_PART).unwrap_or(u32::MAX);
    let first = part.saturating_mul(per_part);
    let end = first.saturating_add(per_part).min(observer.row_count());
    let mut rows = Vec::new();
    let mut evidence = Vec::new();
    for row in first..end {
        let Some(block) = observer
            .rows
            .iter()
            .find(|block| row >= block.first_row && row - block.first_row < block.scopes)
        else {
            continue;
        };
        let rank = row - block.first_row;
        let Some(scope) =
            working.index_scope(&FiscalIndexOwner::Office(block.office.clone()), rank)?
        else {
            return Err(invalid(
                "a fiscal office index lists fewer scopes than its office has",
            ));
        };
        let key = (scope, cycle.to_owned());
        if !cache.contains_key(&key) {
            let read = ledger_book(working, &key.0, cycle)?;
            cache.insert(key.clone(), read);
        }
        let Some((binding, totals, version)) = cache.get(&key).and_then(Option::as_ref) else {
            continue;
        };
        if let Some(row) =
            report_row_of_totals(row, binding, totals, observer.binding.confidence_per_mille)
        {
            rows.push(row);
            evidence.extend(version.clone().map(EvidenceRef::DomainRecordVersion));
        }
    }
    // Rows follow the observer's offices and each office's ranks; a part
    // over two offices names their ledgers out of ID order, and the engine
    // takes a record's evidence only sorted.
    evidence.sort();
    evidence.dedup();
    Ok((rows, evidence))
}

/// A scope's binding and its book's totals of one cycle, read from its
/// ledger record without decoding the rest, and the ledger's current
/// version.
fn ledger_book(
    working: &Working<'_, SimulationView<'_>>,
    scope: &str,
    cycle: &str,
) -> Result<ScopeCycle, CanwuError> {
    let reference = fiscal_ledger_reference(scope);
    let view = working.source();
    let version = view.current_domain_record_version(reference.as_untyped())?;
    if let Some(ledger) = working.state.ledgers.get(scope) {
        return ledger
            .books
            .get(cycle)
            .map(|book| {
                Ok((
                    ledger.scope.clone(),
                    book.totals(ledger.scope.mechanism)?,
                    version,
                ))
            })
            .transpose();
    }
    let Some(record) = active_record(view, &reference)? else {
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
    let binding = record
        .payload
        .get("scope")
        .map(FiscalScopeBinding::deserialize)
        .transpose()
        .map_err(decode)?
        .ok_or_else(|| invalid("fiscal ledger has no scope binding"))?;
    let book = FiscalCycleBook::deserialize(book).map_err(decode)?;
    let totals = book.totals(binding.mechanism)?;
    Ok(Some((binding, totals, version)))
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
