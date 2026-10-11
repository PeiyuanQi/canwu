//! The boundaries of one simulated day: those settled earlier on the day, the
//! one being settled, and those still queued.
//!
//! A boundary system that carries work into a later boundary can budget the
//! boundaries its day opens only if it can count the day's other boundaries,
//! including those caused by sources it does not own. Retained boundary
//! evidence cannot answer that, because a seal removes it, so the scheduler
//! keeps a summary of every boundary settled on the day of the latest
//! boundary. The ledger is a derived index: it is a pure function of the
//! boundary and ingress journals, maintained at boundary commit, rebuilt from
//! the full journal on restore, and kept across a seal. Queued boundaries are
//! read from the canonical ingress queue and the scheduled-action queue.
//!
//! The read names other plugins' queued packet types and their timing, which
//! is future ground truth, so only phased boundary systems may declare it;
//! plugin commands and synchronous event-driven systems are refused at
//! registration, as they are for `canwu.core.ingress`.

use super::{
    BoundaryId, BoundaryRecord, CanwuError, CauseRef, ErrorCode, IngressClass, IngressId,
    IngressPayload, IngressQueueKey, IngressRecord, RuntimeEvidence, RuntimeScheduler,
    RuntimeState, ScheduleKey, SimTime, Simulation, SystemCadence,
};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::sync::Arc;

/// The kind of queued work a boundary admitted or will admit, without its
/// payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BoundaryIngressKind {
    /// A queued command request.
    Command,
    /// Plugin ingress for `plugin` of `packet_type`.
    Plugin { plugin: String, packet_type: String },
    /// A calendar request; its cadences are the boundary's
    /// [`BoundarySummary::cadences`].
    Calendar,
    /// Decision ingress.
    Decision,
    /// Owner-authorized maintenance.
    Maintenance,
}

/// Who queued an ingress item, by the kind of its recorded cause. Grouping by
/// kind rather than by the exact causing boundary or command keeps a
/// boundary's groups bounded by the registered packet types.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BoundaryIngressOrigin {
    /// Ordinary host ingress, which records no cause.
    Host,
    /// Work a boundary system scheduled ([`CauseRef::Boundary`]).
    BoundarySystem,
    /// Work a plugin command handler scheduled ([`CauseRef::Command`]).
    CommandHandler,
    /// Work recorded with an event as its cause ([`CauseRef::Event`]).
    Event,
    /// Engine work or host-labelled ingress ([`CauseRef::System`]), such as a
    /// calendar request (`canwu.core.calendar`).
    System { name: String },
}

impl BoundaryIngressOrigin {
    fn of(cause: Option<&CauseRef>) -> Self {
        match cause {
            None => Self::Host,
            Some(CauseRef::Boundary(_)) => Self::BoundarySystem,
            Some(CauseRef::Command(_)) => Self::CommandHandler,
            Some(CauseRef::Event(_)) => Self::Event,
            Some(CauseRef::System(name)) => Self::System { name: name.clone() },
        }
    }
}

/// How many queued items of one class, kind, and origin a boundary admitted
/// or will admit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundaryIngressCount {
    pub class: IngressClass,
    pub kind: BoundaryIngressKind,
    pub origin: BoundaryIngressOrigin,
    pub count: u64,
}

/// One settled, settling, or queued boundary of a simulated day.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundarySummary {
    /// The boundary's identity once it is being settled or has settled;
    /// `None` for a queued boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary_id: Option<BoundaryId>,
    pub at: SimTime,
    /// Canonical cadences: for a settled or settling boundary, those it
    /// carries (requested by the host or by admitted calendar ingress); for a
    /// queued boundary, those of its queued calendar ingress.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cadences: Vec<SystemCadence>,
    /// Queued items the boundary admitted or will admit, grouped by class,
    /// kind, and origin in the order the queue admits each group's first
    /// item. An empty list on a settled boundary means the host settled it
    /// directly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ingress: Vec<BoundaryIngressCount>,
    /// Legacy scheduled actions due at a queued boundary. Settled and settling
    /// summaries derive from boundary evidence, which does not record
    /// scheduled actions, so they always report zero.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub scheduled_actions: u64,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero(value: &u64) -> bool {
    *value == 0
}

impl BoundarySummary {
    fn queued(at: SimTime) -> Self {
        Self {
            boundary_id: None,
            at,
            cadences: Vec::new(),
            ingress: Vec::new(),
            scheduled_actions: 0,
        }
    }

    fn add_ingress(&mut self, record: &IngressRecord) -> Result<(), CanwuError> {
        let kind = match &record.payload {
            IngressPayload::Command { .. } => BoundaryIngressKind::Command,
            IngressPayload::Plugin {
                plugin,
                packet_type,
                ..
            } => BoundaryIngressKind::Plugin {
                plugin: plugin.clone(),
                packet_type: packet_type.clone(),
            },
            IngressPayload::Calendar { .. } => BoundaryIngressKind::Calendar,
            IngressPayload::Decision { .. } => BoundaryIngressKind::Decision,
            IngressPayload::Maintenance { .. } => BoundaryIngressKind::Maintenance,
            IngressPayload::PluginCancellation { .. } => {
                return Err(CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    "a terminal ingress cancellation is never queued or admitted",
                ));
            }
        };
        let origin = BoundaryIngressOrigin::of(record.cause.as_ref());
        // Groups are bounded by class, registered packet type, and origin
        // kind, so this scan stays short however many items are admitted.
        if let Some(existing) = self.ingress.iter_mut().find(|existing| {
            existing.class == record.class && existing.kind == kind && existing.origin == origin
        }) {
            existing.count = existing.count.checked_add(1).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::IdentifierExhausted,
                    "boundary ingress count exceeds its range",
                )
            })?;
        } else {
            self.ingress.push(BoundaryIngressCount {
                class: record.class,
                kind,
                origin,
                count: 1,
            });
        }
        Ok(())
    }
}

/// The boundaries of one simulated day, as
/// [`crate::SimulationView::boundaries_on_day`] returns them.
///
/// Every list is in settlement order. A queued boundary is one distinct due
/// time in the canonical ingress or scheduled-action queue, so `queued` is a
/// lower bound: work not yet queued, a zero-delay follow-on that opens a
/// second boundary at the same instant, and a boundary the host settles
/// directly are not listed until they exist.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DayBoundaries {
    /// The zero-based simulated day ([`SimTime::day`]).
    pub day: i64,
    /// Boundaries settled earlier on this day.
    pub settled: Vec<BoundarySummary>,
    /// The boundary being settled, when the view belongs to a boundary system
    /// on this day.
    pub current: Option<BoundarySummary>,
    /// Boundaries still queued on this day, by due time.
    pub queued: Vec<BoundarySummary>,
}

impl DayBoundaries {
    /// The settled, current, and queued boundaries together.
    #[must_use]
    pub fn boundary_count(&self) -> usize {
        self.settled.len() + usize::from(self.current.is_some()) + self.queued.len()
    }
}

impl Simulation {
    /// Trusted host read of the boundaries of simulated day `day`
    /// ([`SimTime::day`]): those settled on the day and those still queued on
    /// it at or after the committed time. Boundary systems read the same
    /// ledger, with the boundary being settled, through
    /// [`crate::SimulationView::boundaries_on_day`]. The result names every
    /// plugin's queued packet types, so do not expose it to player, agent,
    /// observer, or remote clients.
    ///
    /// # Errors
    ///
    /// Returns `EvidenceUnavailable` for a day before the current simulated
    /// day, whose settled boundaries are no longer kept.
    pub fn boundaries_on_day(&self, day: i64) -> Result<DayBoundaries, CanwuError> {
        day_boundaries(&self.state, day, self.state.scheduler.now, None)
    }
}

/// Builds the day's boundaries as seen at `now`, with the boundary being
/// settled when the reader is a boundary system.
pub(super) fn day_boundaries(
    state: &RuntimeState,
    day: i64,
    now: SimTime,
    current: Option<&BoundarySummary>,
) -> Result<DayBoundaries, CanwuError> {
    if day < now.day() {
        return Err(CanwuError::new(
            ErrorCode::EvidenceUnavailable,
            "settled boundaries are kept only from the current simulated day on",
        ));
    }
    Ok(DayBoundaries {
        day,
        settled: state
            .scheduler
            .day_boundaries
            .iter()
            .filter(|summary| summary.at.day() == day)
            .map(|summary| summary.as_ref().clone())
            .collect(),
        current: current.filter(|summary| summary.at.day() == day).cloned(),
        queued: queued_boundaries(&state.scheduler, &state.evidence, day, now)?,
    })
}

/// Summarizes a settled or settling boundary from its identity, time,
/// canonical cadences, and admitted ingress.
pub(super) fn summarize_boundary(
    evidence: &RuntimeEvidence,
    boundary_id: BoundaryId,
    at: SimTime,
    cadences: &[SystemCadence],
    admitted_ingress: &[IngressId],
) -> Result<BoundarySummary, CanwuError> {
    let mut summary = BoundarySummary {
        boundary_id: Some(boundary_id),
        cadences: cadences.to_vec(),
        ..BoundarySummary::queued(at)
    };
    for id in admitted_ingress {
        let record = evidence.retained_ingress(*id).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "a boundary admitted ingress that is not retained",
            )
        })?;
        summary.add_ingress(record)?;
    }
    Ok(summary)
}

fn summarize_record(
    evidence: &RuntimeEvidence,
    record: &BoundaryRecord,
) -> Result<BoundarySummary, CanwuError> {
    summarize_boundary(
        evidence,
        record.id,
        record.at,
        &record.cadences,
        &record.admitted_ingress,
    )
}

/// Adds a boundary to the ledger at commit, starting a new day's ledger when
/// the boundary falls on a later day than the ledger's. Entries are shared,
/// so the boundary transaction checkpoint copies pointers, not summaries.
pub(super) fn record_settled_boundary(scheduler: &mut RuntimeScheduler, summary: BoundarySummary) {
    if scheduler
        .day_boundaries
        .last()
        .is_some_and(|last| last.at.day() != summary.at.day())
    {
        scheduler.day_boundaries.clear();
    }
    scheduler.day_boundaries.push(Arc::new(summary));
}

/// Rebuilds the ledger from a fully retained boundary journal: the boundaries
/// that share the latest boundary's day.
pub(super) fn rebuild_day_boundaries(
    evidence: &RuntimeEvidence,
) -> Result<Vec<Arc<BoundarySummary>>, CanwuError> {
    let Some(last) = evidence.boundaries.last() else {
        return Ok(Vec::new());
    };
    let day = last.at.day();
    let first = evidence
        .boundaries
        .iter()
        .rposition(|record| record.at.day() != day)
        .map_or(0, |index| index + 1);
    evidence.boundaries[first..]
        .iter()
        .map(|record| summarize_record(evidence, record).map(Arc::new))
        .collect()
}

/// Groups the queued work due on `day`, at or after `now`, by due time.
fn queued_boundaries(
    scheduler: &RuntimeScheduler,
    evidence: &RuntimeEvidence,
    day: i64,
    now: SimTime,
) -> Result<Vec<BoundarySummary>, CanwuError> {
    let Some(day_start) = day.checked_mul(SimTime::MINUTES_PER_DAY) else {
        return Ok(Vec::new());
    };
    let from = now.max(SimTime::from_minutes(day_start));
    let first_ingress = IngressQueueKey {
        due_at: from,
        class: IngressClass::Command,
        priority: Reverse(i32::MAX),
        issued_at: SimTime::from_minutes(i64::MIN),
        id: IngressId::new(0),
    };
    let mut queued: BTreeMap<SimTime, BoundarySummary> = BTreeMap::new();
    for key in scheduler
        .pending_ingress
        .range(first_ingress..)
        .take_while(|key| key.due_at.day() == day)
    {
        let record = evidence.retained_ingress(key.id).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "pending ingress references an unknown record",
            )
        })?;
        let summary = queued
            .entry(key.due_at)
            .or_insert_with(|| BoundarySummary::queued(key.due_at));
        if let IngressPayload::Calendar { cadences } = &record.payload {
            summary.cadences.extend(cadences.iter().cloned());
        }
        summary.add_ingress(record)?;
    }
    let first_action = ScheduleKey {
        at: from,
        sequence: 0,
    };
    for key in scheduler
        .actions
        .range(first_action..)
        .map(|(key, _)| key)
        .take_while(|key| key.at.day() == day)
    {
        let summary = queued
            .entry(key.at)
            .or_insert_with(|| BoundarySummary::queued(key.at));
        summary.scheduled_actions = summary.scheduled_actions.saturating_add(1);
    }
    Ok(queued
        .into_values()
        .map(|mut summary| {
            summary.cadences.sort();
            summary.cadences.dedup();
            summary
        })
        .collect())
}
