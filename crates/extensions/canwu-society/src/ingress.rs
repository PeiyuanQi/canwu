//! Canonical society ingress: cohort headcount rebases and the owner-side
//! queue shared with lifecycle deltas and pressure revisions.

use crate::PLUGIN_NAME;
use crate::lifecycle::SOCIETY_LIFECYCLE_DELTA_INGRESS;
use crate::model::{SocietyState, invalid};
use crate::revision::SOCIETY_PRESSURE_REVISION_INGRESS;
use canwu_api::{
    BoundaryId, CanwuError, DomainRecordType, DomainRecordVersionRef, DomainValueKindClass,
    IngressId, TypedDomainRecordRef,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// Public society ingress carrying one [`CohortHeadcountRebaseV1`].
pub const COHORT_REBASE_INGRESS: &str = "cohort_headcount_rebase_v1";

/// Rejection code recorded when the cited external stock version was not the
/// current version when the rebase was admitted.
pub const REBASE_STALE_STOCK_REJECTION: &str = "stale_external_stock";
/// Rejection code recorded when a queued packet payload cannot be decoded.
pub const SOCIETY_INGRESS_MALFORMED_REJECTION: &str = "malformed_payload";
/// Rejection code recorded when the cited stock is a society-owned record.
pub const REBASE_INVALID_STOCK_REJECTION: &str = "invalid_external_stock";
/// Rejection code recorded for a packet that arrives while
/// [`MAX_SOCIETY_INGRESS_QUEUE`] packets wait, and the error message of a
/// `revise_society_pressure` command refused for the same reason.
pub const SOCIETY_INGRESS_QUEUE_FULL_REJECTION: &str = "queue_full";
/// Most admitted packets that may wait for the next society settlement.
///
/// A packet that arrives while the queue is full is refused where its sender
/// can first learn of it. The boundary that admits it records a terminal
/// rejected outcome with [`SOCIETY_INGRESS_QUEUE_FULL_REJECTION`] in the
/// cohort exchange ledger, where every other outcome of its kind is recorded:
/// under `rebases` or `lifecycle_deltas` by ingress ID, or as the alignment's
/// latest rejected pressure revision. A `revise_society_pressure` command is
/// already refused at admission while the queue is full, as a recorded
/// `ValueOutOfRange` command rejection; its packet can still find the queue
/// full when it is admitted, if other packets filled it in between.
pub const MAX_SOCIETY_INGRESS_QUEUE: usize = 4_096;
// A batch record's ID is its slot number in four digits, so the IDs sort in
// slot order, and no queue needs more slots than it holds packets.
const _: () = assert!(MAX_SOCIETY_INGRESS_QUEUE <= 10_000);

/// Why a cohort's headcount is rebased against an external stock.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RebaseReason {
    /// The external conserved stock (for example a population record)
    /// changed, and the cohort follows it.
    ExternalStockChange,
    /// The cohort headcount is corrected to match the stock.
    Correction,
}

/// Rebases one cohort's headcount to an external conserved stock version.
///
/// The society plugin re-proportions every distribution of the cohort to the
/// new headcount (see [`SocietyState::rebase_cohort`]) and records the
/// outcome in [`crate::SocietyCohortExchangeLedger::rebases`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CohortHeadcountRebaseV1 {
    pub cohort_id: String,
    pub new_headcount: u64,
    /// Exact external stock version the new headcount was read from. It must
    /// be the current version of a record outside `canwu.society` when the
    /// rebase is admitted.
    pub external_stock: DomainRecordVersionRef,
    pub reason: RebaseReason,
}

/// The head record of the owner-side society ingress queue.
pub struct SocietyIngressQueueRecord;

impl DomainRecordType for SocietyIngressQueueRecord {
    type Payload = SocietyIngressQueue;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "canwu.society";
    const NAME: &'static str = "ingress-queue";
}

#[must_use]
pub fn society_ingress_queue_reference() -> TypedDomainRecordRef<SocietyIngressQueueRecord> {
    TypedDomainRecordRef::new("root")
}

/// One batch record of the owner-side society ingress queue: the packets one
/// boundary admitted.
pub struct SocietyIngressBatchRecord;

impl DomainRecordType for SocietyIngressBatchRecord {
    type Payload = SocietyIngressBatch;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "canwu.society";
    const NAME: &'static str = "ingress-batch";
}

/// The batch record in queue slot `slot`. Its ID is the slot number in four
/// digits, so batch records sort in slot order.
#[must_use]
pub fn society_ingress_batch_reference(
    slot: u32,
) -> TypedDomainRecordRef<SocietyIngressBatchRecord> {
    TypedDomainRecordRef::new(format!("{slot:04}"))
}

/// One admitted society packet awaiting the next society settlement.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QueuedSocietyIngress {
    pub ingress: IngressId,
    /// Boundary that admitted the packet.
    pub admitted_at: BoundaryId,
    pub packet_type: String,
    pub payload: Value,
    /// Rejection decided at admission (for example a stale stock version).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admission_rejection: Option<String>,
}

/// The head of the owner-side queue of admitted rebases, lifecycle deltas,
/// and pressure revisions (schema version 2).
///
/// The queue is this head and one [`SocietyIngressBatch`] for each boundary
/// that admitted a packet to it since the last society settlement, in slots
/// `0` to `batches - 1`. The event-driven phase-12 intake writes the packets a
/// boundary admits into the next slot and counts them here; the phase-7
/// settlement, which runs on the plugin's settlement cadence, applies every
/// batch in slot order, empties the slots it used, and resets the head.
/// Each record of the queue is therefore written in proportion to what one
/// boundary admits or one settlement consumes, never rewritten whole as the
/// queue grows, and slots are reused after a settlement, so there are never
/// more batch records than [`MAX_SOCIETY_INGRESS_QUEUE`].
///
/// Only the intake adds packets: an initial scenario cannot seed a
/// non-empty queue. At most [`MAX_SOCIETY_INGRESS_QUEUE`] packets wait at
/// once, and every slot in use holds at least one.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SocietyIngressQueue {
    pub schema_version: u32,
    /// Packets waiting, over all batches.
    pub queued: u32,
    /// Batch records in use: slots `0` to `batches - 1`.
    pub batches: u32,
}

impl SocietyIngressQueue {
    /// Version 1 was one record holding every queued packet; version 2 is a
    /// head over batch records.
    pub const SCHEMA_VERSION: u32 = 2;

    /// An empty queue at the current schema version.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            queued: 0,
            batches: 0,
        }
    }

    /// # Errors
    ///
    /// Returns `InvalidDomainRecord` when the schema is not version 2, more
    /// than [`MAX_SOCIETY_INGRESS_QUEUE`] packets wait, or the batch count
    /// cannot hold the packets (a batch in use holds at least one).
    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(invalid("unsupported society ingress queue version"));
        }
        if usize::try_from(self.queued).map_or(true, |queued| queued > MAX_SOCIETY_INGRESS_QUEUE)
            || self.batches > self.queued
            || (self.batches == 0) != (self.queued == 0)
        {
            return Err(invalid(
                "society ingress queue head counts more packets or batches than it may hold",
            ));
        }
        Ok(())
    }
}

/// The packets one boundary admitted to the owner-side society ingress
/// queue, in admission order; empty while its slot is not in use.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SocietyIngressBatch {
    pub schema_version: u32,
    pub entries: Vec<QueuedSocietyIngress>,
}

impl SocietyIngressBatch {
    pub const SCHEMA_VERSION: u32 = 1;

    /// An empty batch at the current schema version.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }

    /// # Errors
    ///
    /// Returns `InvalidDomainRecord` when the schema, a packet type, or an
    /// admission verdict is invalid, an ingress appears twice, or the entries
    /// were not all admitted by one boundary.
    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(invalid("unsupported society ingress batch version"));
        }
        if self.entries.len() > MAX_SOCIETY_INGRESS_QUEUE {
            return Err(invalid(
                "society ingress batch exceeds the queue's capacity",
            ));
        }
        let mut seen = BTreeSet::new();
        for entry in &self.entries {
            if !seen.insert(entry.ingress)
                || entry.admitted_at != self.entries[0].admitted_at
                || !matches!(
                    entry.packet_type.as_str(),
                    COHORT_REBASE_INGRESS
                        | SOCIETY_LIFECYCLE_DELTA_INGRESS
                        | SOCIETY_PRESSURE_REVISION_INGRESS
                )
                || entry
                    .admission_rejection
                    .as_ref()
                    .is_some_and(String::is_empty)
            {
                return Err(invalid("society ingress batch contains an invalid entry"));
            }
        }
        Ok(())
    }
}

/// Joins the batches of the queue `head` describes, given in slot order, into
/// the queued packets in admission order.
///
/// # Errors
///
/// Returns `InvalidDomainRecord` when a batch is invalid or empty, the
/// batches were not admitted by strictly later boundaries in slot order, an
/// ingress appears twice, or the batches do not hold exactly the packets and
/// batches the head counts.
pub(crate) fn queue_entries(
    head: &SocietyIngressQueue,
    batches: impl IntoIterator<Item = SocietyIngressBatch>,
) -> Result<Vec<QueuedSocietyIngress>, CanwuError> {
    let mut entries: Vec<QueuedSocietyIngress> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut count = 0_u32;
    for batch in batches {
        batch.validate()?;
        let Some(first) = batch.entries.first() else {
            return Err(invalid("a society ingress batch in use is empty"));
        };
        if entries
            .last()
            .is_some_and(|last| first.admitted_at <= last.admitted_at)
        {
            return Err(invalid(
                "society ingress batches are not in the order their boundaries admitted them",
            ));
        }
        for entry in batch.entries {
            if !seen.insert(entry.ingress) {
                return Err(invalid("a society ingress packet is queued twice"));
            }
            entries.push(entry);
        }
        count = count
            .checked_add(1)
            .ok_or_else(|| invalid("society ingress batch count overflowed"))?;
    }
    if count != head.batches || u32::try_from(entries.len()).ok() != Some(head.queued) {
        return Err(invalid(
            "society ingress batches do not hold the packets their queue head counts",
        ));
    }
    Ok(entries)
}

impl SocietyState {
    /// Atomically rebases one cohort to a new headcount and returns the
    /// previous headcount.
    ///
    /// Every distribution of the cohort is re-proportioned to exactly
    /// `new_headcount`. Each bucket first receives the floor of its exact
    /// share `headcount * new_headcount / previous_headcount`; the remaining
    /// units go one each to the buckets with the largest fractional
    /// remainders, ties going to the later bucket in canonical profile order
    /// (the rule cohort transfers use). Every bucket therefore stays within
    /// one unit of its exact share, and other cohorts are untouched. Derived
    /// aggregates, mobilization candidates, and projections are cleared for
    /// rematerialization.
    ///
    /// # Errors
    ///
    /// Returns `InvalidDomainRecord` when the cohort is unknown, the new
    /// headcount is zero, or the result violates a society invariant.
    pub fn rebase_cohort(
        &mut self,
        cohort_id: &str,
        new_headcount: u64,
    ) -> Result<u64, CanwuError> {
        let previous = self
            .cohorts
            .get(cohort_id)
            .ok_or_else(|| invalid(format!("rebase names unknown cohort {cohort_id}")))?
            .headcount;
        if new_headcount == 0 {
            return Err(invalid("a cohort cannot be rebased to zero population"));
        }
        if new_headcount == previous {
            return Ok(previous);
        }
        let mut draft = self.clone();
        // Merge duplicate buckets first so each profile is re-proportioned
        // once.
        draft.canonicalize()?;
        for distribution in draft
            .distributions
            .values_mut()
            .filter(|distribution| distribution.cohort_id == cohort_id)
        {
            let allocations = rebase_allocations(
                distribution
                    .buckets
                    .iter()
                    .map(|bucket| bucket.headcount)
                    .collect(),
                previous,
                new_headcount,
            )?;
            for (bucket, headcount) in distribution.buckets.iter_mut().zip(allocations) {
                bucket.headcount = headcount;
            }
        }
        draft
            .cohorts
            .get_mut(cohort_id)
            .ok_or_else(|| invalid("rebased cohort disappeared"))?
            .headcount = new_headcount;
        draft.invalidate_derived_state();
        draft.canonicalize()?;
        draft.validate()?;
        *self = draft;
        Ok(previous)
    }
}

fn rebase_allocations(
    buckets: Vec<u64>,
    previous: u64,
    new_headcount: u64,
) -> Result<Vec<u64>, CanwuError> {
    let previous = u128::from(previous);
    let target = u128::from(new_headcount);
    let mut allocations = Vec::with_capacity(buckets.len());
    let mut remainders = Vec::with_capacity(buckets.len());
    let mut assigned = 0_u128;
    for (index, headcount) in buckets.into_iter().enumerate() {
        let product = u128::from(headcount) * target;
        let base = product / previous;
        assigned += base;
        allocations.push(base);
        remainders.push((product % previous, index));
    }
    remainders.sort_by(|left, right| right.cmp(left));
    let leftover = usize::try_from(target - assigned)
        .map_err(|_| invalid("rebase remainder exceeded the platform range"))?;
    for (_, index) in remainders.into_iter().take(leftover) {
        allocations[index] += 1;
    }
    allocations
        .into_iter()
        .map(|value| {
            u64::try_from(value).map_err(|_| invalid("rebased bucket exceeded the u64 range"))
        })
        .collect()
}

/// Returns the admission rejection for a rebase whose cited stock version
/// is (`stock_is_current`) or is not the current version at the admission
/// cut.
pub(crate) fn rebase_admission_rejection(
    rebase: &CohortHeadcountRebaseV1,
    stock_is_current: bool,
) -> Option<String> {
    if rebase.external_stock.record.kind.namespace == crate::model::SOCIETY_NAMESPACE {
        Some(REBASE_INVALID_STOCK_REJECTION.to_owned())
    } else if stock_is_current {
        None
    } else {
        Some(REBASE_STALE_STOCK_REJECTION.to_owned())
    }
}

/// Returns whether a packet type is consumed through the owner-side queue.
pub(crate) fn is_queued_packet(plugin: &str, packet_type: &str) -> bool {
    plugin == PLUGIN_NAME
        && matches!(
            packet_type,
            COHORT_REBASE_INGRESS
                | SOCIETY_LIFECYCLE_DELTA_INGRESS
                | SOCIETY_PRESSURE_REVISION_INGRESS
        )
}
