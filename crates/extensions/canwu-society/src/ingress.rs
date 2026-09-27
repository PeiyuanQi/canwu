//! Canonical society ingress: cohort headcount rebases and the owner-side
//! queue shared with lifecycle deltas.

use crate::PLUGIN_NAME;
use crate::lifecycle::SOCIETY_LIFECYCLE_DELTA_INGRESS;
use crate::model::{SocietyState, invalid};
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
/// Most admitted packets that may wait for the next Daily settlement; the
/// intake rejects further packets with a `society_ingress_rejected_v1` event.
pub const MAX_SOCIETY_INGRESS_QUEUE: usize = 4_096;

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

/// One admitted society packet awaiting the next Daily settlement.
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

/// Owner-side queue of admitted rebases and lifecycle deltas.
///
/// The event-driven phase-12 intake appends packets in admission order; the
/// Daily phase-7 settlement consumes the whole queue it observes. Only the
/// intake writes entries: an initial scenario cannot seed a non-empty queue,
/// and at most [`MAX_SOCIETY_INGRESS_QUEUE`] entries wait at once.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SocietyIngressQueue {
    pub schema_version: u32,
    pub entries: Vec<QueuedSocietyIngress>,
}

impl SocietyIngressQueue {
    pub const SCHEMA_VERSION: u32 = 1;

    /// # Errors
    ///
    /// Returns `InvalidDomainRecord` when the schema, packet types, ingress
    /// identities, or admission order are invalid.
    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(invalid("unsupported society ingress queue version"));
        }
        if self.entries.len() > MAX_SOCIETY_INGRESS_QUEUE {
            return Err(invalid("society ingress queue exceeds its capacity"));
        }
        let mut seen = BTreeSet::new();
        let mut previous = None;
        for entry in &self.entries {
            if !seen.insert(entry.ingress)
                || previous.is_some_and(|boundary| entry.admitted_at < boundary)
                || !matches!(
                    entry.packet_type.as_str(),
                    COHORT_REBASE_INGRESS | SOCIETY_LIFECYCLE_DELTA_INGRESS
                )
                || entry
                    .admission_rejection
                    .as_ref()
                    .is_some_and(String::is_empty)
            {
                return Err(invalid("society ingress queue contains an invalid entry"));
            }
            previous = Some(entry.admitted_at);
        }
        Ok(())
    }
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
            COHORT_REBASE_INGRESS | SOCIETY_LIFECYCLE_DELTA_INGRESS
        )
}
