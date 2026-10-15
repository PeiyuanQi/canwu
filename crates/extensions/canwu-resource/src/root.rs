//! How the resource root is persisted: one live-root record and the hot
//! terminal records in buckets of their own.
//!
//! The resource root is one [`ResourceState`]. It persists as
//!
//! - the **live root**, the domain record `resource:runtime:v1`: every part of
//!   the state except the bodies of its hot terminal records (its archive
//!   candidates) and their candidate index, plus a
//!   [`ResourceHotTerminalLedgerV1`] that counts them; and
//! - the **hot terminal buckets**, domain records
//!   `resource:hot-terminal:{bucket}`: each hot terminal record, with its
//!   terminal sequence, in the bucket its identity hashes to
//!   ([`resource_hot_terminal_bucket`]).
//!
//! A hot terminal record adds nothing to the live root but its counts in the
//! ledger, so a boundary that settles an operation decodes, validates,
//! re-encodes, and journals the live root and the buckets of the terminal
//! records it reads or writes, not the whole backlog. The split is a function of the state: the same state
//! always persists as the same records ([`ResourceState::into_records`]), and
//! [`ResourceState::from_records`] restores it.
//!
//! In a boundary the resource plugin works on a *partial* state: the live root
//! with only the hot terminal records it reads attached. Every rule that
//! counts or aggregates hot terminal records reads the ledger, so a partial
//! state settles every operation exactly as the whole state would, as long as
//! the records an operation reads by identity are attached. The plugin
//! attaches every hot terminal record a packet names, and every record those
//! name in turn, before it settles the packet.

use crate::{
    CompletionLeaseAcquisitionId, PLUGIN_NAMESPACE, ProtectedFloorPolicyRevisionId, ResourceError,
    ResourceLimitsV1, ResourceReservationId, ResourceState, ResourceTerminalArchivePayloadV1,
    ResourceTerminalRecordKeyV1, resource_canwu_error, resource_runtime_reference,
};
use canwu_api::{
    BoundaryDirective, CanwuError, CanwuRun, DomainRecord, DomainRecordClass, DomainRecordDraft,
    DomainRecordLifecycle, DomainRecordMutation, DomainRecordRef, DomainRecordType,
    DomainValueKindClass, ErrorCode, KnowledgeHolderRef, SimulationView, TypedDomainRecordRef,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Format of a [`ResourceHotTerminalBucketV1`].
pub const RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION: u32 = 1;

/// Counts and aggregates over the hot terminal records of a resource root,
/// persisted in its live root.
///
/// Every rule of the root that counts hot terminal records, or asks whether
/// any of them has some property, reads this ledger instead of the records,
/// so the live root alone answers it. The resource state keeps it current
/// whenever a record becomes an archive candidate or leaves the root.
///
/// `bytes` is the exception: in a persisted live root it is the encoded size
/// of every hot terminal entry, and in memory it counts only the entries that
/// are not attached to the state. A whole state therefore holds `0`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceHotTerminalLedgerV1 {
    /// Hot terminal records: the root's archive candidates.
    pub records: u64,
    /// Hot operation outcomes, which `max_operation_outcomes` bounds.
    pub outcomes: u64,
    /// Hot terminal transfers, which `max_transfers` counts with the active
    /// ones.
    pub transfers: u64,
    /// Hot maintenance receipts of applied archive commits, which
    /// `max_archive_maintenance_receipts` bounds.
    pub applied_archive_maintenance_receipts: u64,
    /// Encoded bytes of hot terminal entries (see the type documentation).
    pub bytes: u64,
    /// Hot terminal demands by requester.
    #[serde(with = "holder_counts")]
    pub demand_requesters: BTreeMap<KnowledgeHolderRef, u64>,
    /// Hot terminal demands by the protected-floor policy they name.
    pub demand_floor_policies: BTreeMap<ProtectedFloorPolicyRevisionId, u64>,
    /// Hot completion lease receipts by acquisition.
    pub lease_receipt_acquisitions: BTreeMap<CompletionLeaseAcquisitionId, u64>,
}

impl ResourceHotTerminalLedgerV1 {
    /// The ledger of the attached hot terminal records of `state`, with
    /// `bytes` zero.
    fn of_attached(state: &ResourceState) -> Result<Self, ResourceError> {
        let mut ledger = Self::default();
        for key in state.terminal_archive_candidates.values() {
            ledger.count(state, key, true)?;
        }
        Ok(ledger)
    }

    /// Counts the attached hot terminal record `key` of `state` in or out.
    fn count(
        &mut self,
        state: &ResourceState,
        key: &ResourceTerminalRecordKeyV1,
        add: bool,
    ) -> Result<(), ResourceError> {
        step(&mut self.records, add)?;
        match key {
            ResourceTerminalRecordKeyV1::Outcome(_) => step(&mut self.outcomes, add)?,
            ResourceTerminalRecordKeyV1::Transfer(_) => step(&mut self.transfers, add)?,
            ResourceTerminalRecordKeyV1::ArchiveMaintenanceReceipt(_) => {
                step(&mut self.applied_archive_maintenance_receipts, add)?;
            }
            ResourceTerminalRecordKeyV1::Demand(id) => {
                let demand = state.demands.get(id).ok_or_else(|| {
                    ResourceError::NotFound("hot terminal demand is not attached".to_owned())
                })?;
                step_key(&mut self.demand_requesters, &demand.requester, add)?;
                if let Some(policy) = &demand.protected_floor_policy {
                    step_key(&mut self.demand_floor_policies, policy, add)?;
                }
            }
            ResourceTerminalRecordKeyV1::LeaseReceipt(sequence) => {
                let receipt = state
                    .completion_leases
                    .receipts
                    .get(sequence)
                    .ok_or_else(|| {
                        ResourceError::NotFound(
                            "hot terminal completion receipt is not attached".to_owned(),
                        )
                    })?;
                step_key(
                    &mut self.lease_receipt_acquisitions,
                    &receipt.acquisition,
                    add,
                )?;
            }
            ResourceTerminalRecordKeyV1::Reservation(_)
            | ResourceTerminalRecordKeyV1::AllocationLeg(_)
            | ResourceTerminalRecordKeyV1::Consumption(_)
            | ResourceTerminalRecordKeyV1::Fulfillment(_)
            | ResourceTerminalRecordKeyV1::Loss(_)
            | ResourceTerminalRecordKeyV1::ExternalCompletionParticipant(_) => {}
        }
        Ok(())
    }

    /// Whether every count of `attached` is at most this ledger's.
    fn covers(&self, attached: &Self) -> bool {
        fn covers_map<K: Ord>(ledger: &BTreeMap<K, u64>, attached: &BTreeMap<K, u64>) -> bool {
            attached
                .iter()
                .all(|(key, count)| ledger.get(key).is_some_and(|total| total >= count))
        }
        attached.records <= self.records
            && attached.outcomes <= self.outcomes
            && attached.transfers <= self.transfers
            && attached.applied_archive_maintenance_receipts
                <= self.applied_archive_maintenance_receipts
            && covers_map(&self.demand_requesters, &attached.demand_requesters)
            && covers_map(&self.demand_floor_policies, &attached.demand_floor_policies)
            && covers_map(
                &self.lease_receipt_acquisitions,
                &attached.lease_receipt_acquisitions,
            )
    }
}

fn step(value: &mut u64, add: bool) -> Result<(), ResourceError> {
    *value = if add {
        value.checked_add(1)
    } else {
        value.checked_sub(1)
    }
    .ok_or(ResourceError::Overflow)?;
    Ok(())
}

fn step_key<K: Ord + Clone>(
    map: &mut BTreeMap<K, u64>,
    key: &K,
    add: bool,
) -> Result<(), ResourceError> {
    if add {
        let count = map.entry(key.clone()).or_default();
        *count = count.checked_add(1).ok_or(ResourceError::Overflow)?;
    } else {
        let count = map.get_mut(key).ok_or(ResourceError::Overflow)?;
        *count = count.checked_sub(1).ok_or(ResourceError::Overflow)?;
        if *count == 0 {
            map.remove(key);
        }
    }
    Ok(())
}

// A holder is not a JSON object key: persist the counts as an entry array in
// ascending holder order, and refuse a zero count, a duplicate, or another
// order, so a ledger has one encoding.
mod holder_counts {
    use super::*;

    pub fn serialize<S>(
        value: &BTreeMap<KnowledgeHolderRef, u64>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value.iter().collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<KnowledgeHolderRef, u64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = Vec::<(KnowledgeHolderRef, u64)>::deserialize(deserializer)?;
        let mut counts = BTreeMap::new();
        for (holder, count) in entries {
            if count == 0
                || counts
                    .last_key_value()
                    .is_some_and(|(last, _): (&KnowledgeHolderRef, _)| last >= &holder)
            {
                return Err(serde::de::Error::custom(
                    "hot terminal holder counts must be positive and strictly ascending",
                ));
            }
            counts.insert(holder, count);
        }
        Ok(counts)
    }
}

/// One hot terminal record as its bucket holds it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceHotTerminalEntryV1 {
    /// Its terminal sequence: its position among the archive candidates.
    pub sequence: u64,
    /// The record's identity, which decides its bucket.
    pub key: ResourceTerminalRecordKeyV1,
    /// The record itself.
    pub payload: ResourceTerminalArchivePayloadV1,
    /// For a demand, the reservations of its closure that are still hot. They
    /// are hot terminal records of their own; the list lets a reader of the
    /// demand attach them.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub reservations: BTreeSet<ResourceReservationId>,
}

impl ResourceHotTerminalEntryV1 {
    /// The identities this record names that a reader may need attached
    /// beside it: the closure of a demand, and the records a reservation,
    /// allocation leg, consumption, fulfillment, transfer, or loss belongs to.
    fn references(&self) -> Vec<ResourceTerminalRecordKeyV1> {
        let mut references = payload_references(&self.payload);
        references.extend(
            self.reservations
                .iter()
                .cloned()
                .map(ResourceTerminalRecordKeyV1::Reservation),
        );
        references
    }

    /// Bytes this entry adds to the encoded root.
    fn encoded_len(&self) -> Result<u64, ResourceError> {
        let bytes = serde_json::to_vec(self).map_err(|error| {
            ResourceError::InvalidDefinition(format!(
                "resource hot terminal entry cannot be encoded: {error}"
            ))
        })?;
        u64::try_from(bytes.len()).map_err(|_| ResourceError::Overflow)
    }

    /// Whether `payload` is the record `key` names.
    fn names_its_payload(&self) -> bool {
        use ResourceTerminalArchivePayloadV1 as Payload;
        use ResourceTerminalRecordKeyV1 as Key;
        match (&self.key, &self.payload) {
            (Key::Outcome(key), Payload::Outcome(value)) => &value.operation_key == key,
            (Key::Demand(id), Payload::Demand(value)) => &value.id == id,
            (Key::Reservation(id), Payload::Reservation(value)) => &value.id == id,
            (Key::AllocationLeg(id), Payload::AllocationLeg(value)) => &value.id == id,
            (Key::Consumption(id), Payload::Consumption(value)) => &value.id == id,
            (Key::Fulfillment(id), Payload::Fulfillment(value)) => &value.id == id,
            (Key::Loss(id), Payload::Loss(value)) => &value.id == id,
            (Key::Transfer(id), Payload::Transfer(value)) => &value.id == id,
            (
                Key::ExternalCompletionParticipant(id),
                Payload::ExternalCompletionParticipant(value),
            ) => &value.grant.acquisition == id,
            (Key::LeaseReceipt(sequence), Payload::LeaseReceipt(value)) => {
                &value.sequence == sequence
            }
            (
                Key::ArchiveMaintenanceReceipt(sequence),
                Payload::ArchiveMaintenanceReceipt(value),
            ) => &value.sequence == sequence,
            _ => false,
        }
    }
}

fn payload_references(
    payload: &ResourceTerminalArchivePayloadV1,
) -> Vec<ResourceTerminalRecordKeyV1> {
    use ResourceTerminalArchivePayloadV1 as Payload;
    use ResourceTerminalRecordKeyV1 as Key;
    match payload {
        Payload::Reservation(value) => vec![
            Key::AllocationLeg(value.allocation_leg.clone()),
            Key::Demand(value.demand.clone()),
        ],
        Payload::AllocationLeg(value) => vec![
            Key::Reservation(value.reservation.clone()),
            Key::Demand(value.demand.clone()),
        ],
        Payload::Consumption(value) => vec![
            Key::AllocationLeg(value.allocation_leg.clone()),
            Key::Demand(value.demand.clone()),
        ],
        Payload::Fulfillment(value) => std::iter::once(Key::Demand(value.demand.clone()))
            .chain(
                value
                    .allocation_legs
                    .iter()
                    .cloned()
                    .map(Key::AllocationLeg),
            )
            .collect(),
        Payload::Transfer(value) => vec![Key::AllocationLeg(value.allocation_leg.clone())],
        Payload::Loss(value) => value.transfer.iter().cloned().map(Key::Transfer).collect(),
        Payload::Outcome(_)
        | Payload::Demand(_)
        | Payload::ExternalCompletionParticipant(_)
        | Payload::LeaseReceipt(_)
        | Payload::ArchiveMaintenanceReceipt(_) => Vec::new(),
    }
}

/// One bucket of hot terminal records: the payload of
/// `resource:hot-terminal:{bucket}`. Its entries are those whose identity
/// hashes to `bucket`, in ascending key order.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceHotTerminalBucketV1 {
    /// [`RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION`].
    pub format_version: u32,
    /// The bucket's index, below the root's bucket count.
    pub bucket: u32,
    /// The bucket's hot terminal records, in ascending key order. A bucket
    /// emptied by an archive commit stays as a record with no entries.
    pub entries: Vec<ResourceHotTerminalEntryV1>,
}

/// Domain record type of a hot terminal bucket.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResourceHotTerminalBucketRecord;

impl DomainRecordType for ResourceHotTerminalBucketRecord {
    type Payload = ResourceHotTerminalBucketV1;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = PLUGIN_NAMESPACE;
    const NAME: &'static str = "hot-terminal";
}

/// The domain record of hot terminal bucket `bucket`.
#[must_use]
pub fn resource_hot_terminal_bucket_reference(
    bucket: u32,
) -> TypedDomainRecordRef<ResourceHotTerminalBucketRecord> {
    TypedDomainRecordRef::new(format!("resource:hot-terminal:{bucket}"))
}

/// How many hot terminal buckets a root with `limits` uses: the candidate
/// budget rounded up to a power of two, so a full budget holds about one
/// record per bucket. Fixed for the life of a root.
#[must_use]
pub fn resource_hot_terminal_bucket_count(limits: &ResourceLimitsV1) -> u32 {
    u32::try_from(limits.max_archive_candidates.max(1).next_power_of_two()).unwrap_or(1 << 31)
}

/// The text a hot terminal record is bucketed by: the identity it carries.
/// Receipts, which are named by sequence, get a form no identity can take.
fn key_text(key: &ResourceTerminalRecordKeyV1) -> std::borrow::Cow<'_, str> {
    use ResourceTerminalRecordKeyV1 as Key;
    match key {
        Key::Outcome(value) => value.as_str().into(),
        Key::Demand(value) => value.as_str().into(),
        Key::Reservation(value) => value.as_str().into(),
        Key::AllocationLeg(value) => value.as_str().into(),
        Key::Consumption(value) => value.as_str().into(),
        Key::Fulfillment(value) => value.as_str().into(),
        Key::Loss(value) => value.as_str().into(),
        Key::Transfer(value) => value.as_str().into(),
        Key::ExternalCompletionParticipant(value) => value.as_str().into(),
        Key::LeaseReceipt(sequence) => format!("#lease-receipt#{sequence}").into(),
        Key::ArchiveMaintenanceReceipt(sequence) => {
            format!("#archive-maintenance-receipt#{sequence}").into()
        }
    }
}

fn text_bucket(text: &str, buckets: u32) -> u32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"canwu.resource.hot-terminal-bucket.v1");
    hasher.update(&[0]);
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    let mut low = [0_u8; 4];
    low.copy_from_slice(&digest.as_bytes()[..4]);
    u32::from_le_bytes(low) & buckets.saturating_sub(1)
}

/// The bucket of a root with `buckets` buckets
/// ([`resource_hot_terminal_bucket_count`]) that holds the hot terminal record
/// `key`. Records of different kinds that carry the same identity share a
/// bucket.
#[must_use]
pub fn resource_hot_terminal_bucket(key: &ResourceTerminalRecordKeyV1, buckets: u32) -> u32 {
    text_bucket(&key_text(key), buckets)
}

/// Every hot terminal record a string could name.
fn keys_named_by(text: &str) -> Vec<ResourceTerminalRecordKeyV1> {
    use ResourceTerminalRecordKeyV1 as Key;
    let mut keys = Vec::new();
    if let Ok(id) = crate::ResourceOperationKey::new(text) {
        keys.push(Key::Outcome(id));
    }
    if let Ok(id) = crate::ResourceDemandId::new(text) {
        keys.push(Key::Demand(id));
    }
    if let Ok(id) = ResourceReservationId::new(text) {
        keys.push(Key::Reservation(id));
    }
    if let Ok(id) = crate::ResourceAllocationLegId::new(text) {
        keys.push(Key::AllocationLeg(id));
    }
    if let Ok(id) = crate::ResourceConsumptionId::new(text) {
        keys.push(Key::Consumption(id));
    }
    if let Ok(id) = crate::ResourceFulfillmentId::new(text) {
        keys.push(Key::Fulfillment(id));
    }
    if let Ok(id) = crate::ResourceLossId::new(text) {
        keys.push(Key::Loss(id));
    }
    if let Ok(id) = crate::ResourceTransferId::new(text) {
        keys.push(Key::Transfer(id));
    }
    if let Ok(id) = CompletionLeaseAcquisitionId::new(text) {
        keys.push(Key::ExternalCompletionParticipant(id));
    }
    keys
}

/// Every string in `value` shaped like a namespaced identity, each once.
fn named_identities(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::String(text) => {
            if text.len() <= 160
                && text.contains(':')
                && text.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
                })
            {
                out.insert(text.clone());
            }
        }
        Value::Array(values) => {
            for value in values {
                named_identities(value, out);
            }
        }
        Value::Object(fields) => {
            for (field, value) in fields {
                // A map keyed by identities names them in its keys.
                named_identities(&Value::String(field.clone()), out);
                named_identities(value, out);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

impl ResourceState {
    /// Hot terminal records of this root: its archive candidates, attached or
    /// not.
    #[must_use]
    pub fn hot_terminal_records(&self) -> usize {
        usize::try_from(self.hot_terminal.records).unwrap_or(usize::MAX)
    }

    /// Hot operation outcomes of this root, attached or not.
    #[must_use]
    pub fn hot_operation_outcomes(&self) -> usize {
        usize::try_from(self.hot_terminal.outcomes).unwrap_or(usize::MAX)
    }

    /// Whether every hot terminal record of this root is attached to it.
    #[must_use]
    pub fn hot_terminal_records_attached(&self) -> bool {
        u64::try_from(self.terminal_archive_candidates.len()).ok()
            == Some(self.hot_terminal.records)
    }

    /// Recomputes the hot terminal ledger from the attached records, for a
    /// whole state whose candidates were edited directly. Every resource
    /// operation keeps the ledger current; this is for fixtures that place
    /// candidates in the maps themselves.
    pub fn refresh_hot_terminal_ledger(&mut self) -> Result<(), ResourceError> {
        self.hot_terminal = ResourceHotTerminalLedgerV1::of_attached(self)?;
        Ok(())
    }

    /// Records that the attached record `key` became an archive candidate
    /// under `sequence`, and counts it in the ledger.
    pub(crate) fn add_hot_candidate(
        &mut self,
        sequence: u64,
        key: ResourceTerminalRecordKeyV1,
    ) -> Result<(), ResourceError> {
        let mut ledger = std::mem::take(&mut self.hot_terminal);
        let counted = ledger.count(self, &key, true);
        self.hot_terminal = ledger;
        counted?;
        if self
            .terminal_archive_candidates
            .insert(sequence, key)
            .is_some()
        {
            return Err(ResourceError::InvalidDefinition(
                "resource terminal archive sequence is already a candidate".to_owned(),
            ));
        }
        Ok(())
    }

    /// Takes the attached record `key` out of the ledger. Its body must still
    /// be attached; the caller removes it and its candidate entry.
    pub(crate) fn drop_hot_candidate(
        &mut self,
        key: &ResourceTerminalRecordKeyV1,
    ) -> Result<(), ResourceError> {
        let mut ledger = std::mem::take(&mut self.hot_terminal);
        let counted = ledger.count(self, key, false);
        self.hot_terminal = ledger;
        counted
    }

    /// Whether the archive candidate `key` is attached.
    pub(crate) fn hot_candidate_attached(&self, key: &ResourceTerminalRecordKeyV1) -> bool {
        self.terminal_archive_candidates
            .values()
            .any(|candidate| candidate == key)
    }

    /// The bucket entry of the attached hot terminal record `key`.
    pub(crate) fn hot_terminal_entry(
        &self,
        sequence: u64,
        key: &ResourceTerminalRecordKeyV1,
    ) -> Result<ResourceHotTerminalEntryV1, ResourceError> {
        use ResourceTerminalArchivePayloadV1 as Payload;
        use ResourceTerminalRecordKeyV1 as Key;
        let missing =
            || ResourceError::NotFound("resource hot terminal record is not attached".to_owned());
        let payload = match key {
            Key::Outcome(id) => {
                Payload::Outcome(self.outcomes.get(id).cloned().ok_or_else(missing)?)
            }
            Key::Demand(id) => Payload::Demand(self.demands.get(id).cloned().ok_or_else(missing)?),
            Key::Reservation(id) => {
                Payload::Reservation(self.reservations.get(id).cloned().ok_or_else(missing)?)
            }
            Key::AllocationLeg(id) => {
                Payload::AllocationLeg(self.allocation_legs.get(id).cloned().ok_or_else(missing)?)
            }
            Key::Consumption(id) => {
                Payload::Consumption(self.consumptions.get(id).cloned().ok_or_else(missing)?)
            }
            Key::Fulfillment(id) => {
                Payload::Fulfillment(self.fulfillments.get(id).cloned().ok_or_else(missing)?)
            }
            Key::Loss(id) => Payload::Loss(self.losses.get(id).cloned().ok_or_else(missing)?),
            Key::Transfer(id) => {
                Payload::Transfer(self.transfers.get(id).cloned().ok_or_else(missing)?)
            }
            Key::ExternalCompletionParticipant(id) => Payload::ExternalCompletionParticipant(
                self.external_completion_participants
                    .terminal_grants
                    .get(id)
                    .cloned()
                    .ok_or_else(missing)?,
            ),
            Key::LeaseReceipt(id) => Payload::LeaseReceipt(
                self.completion_leases
                    .receipts
                    .get(id)
                    .cloned()
                    .ok_or_else(missing)?,
            ),
            Key::ArchiveMaintenanceReceipt(id) => Payload::ArchiveMaintenanceReceipt(
                self.archive_maintenance_receipts
                    .get(id)
                    .cloned()
                    .ok_or_else(missing)?,
            ),
        };
        let reservations = match key {
            Key::Demand(id) => self
                .reservation_by_demand
                .get(id)
                .cloned()
                .unwrap_or_default(),
            _ => BTreeSet::new(),
        };
        Ok(ResourceHotTerminalEntryV1 {
            sequence,
            key: key.clone(),
            payload,
            reservations,
        })
    }

    /// Removes the body of the attached hot terminal record `key` and its
    /// index entries, and nothing else: no ledger change and none of the
    /// effects archiving it has.
    fn detach_hot_terminal(&mut self, key: &ResourceTerminalRecordKeyV1) {
        use ResourceTerminalRecordKeyV1 as Key;
        match key {
            Key::Outcome(id) => {
                self.outcomes.remove(id);
            }
            Key::Demand(id) => {
                self.demands.remove(id);
                self.reservation_by_demand.remove(id);
            }
            Key::Reservation(id) => {
                if let Some(reservation) = self.reservations.remove(id)
                    && let Some(ids) = self.reservation_by_demand.get_mut(&reservation.demand)
                {
                    ids.remove(id);
                    if ids.is_empty() {
                        self.reservation_by_demand.remove(&reservation.demand);
                    }
                }
            }
            Key::AllocationLeg(id) => {
                self.allocation_legs.remove(id);
            }
            Key::Consumption(id) => {
                self.consumptions.remove(id);
            }
            Key::Fulfillment(id) => {
                self.fulfillments.remove(id);
            }
            Key::Loss(id) => {
                self.losses.remove(id);
            }
            Key::Transfer(id) => {
                self.transfers.remove(id);
            }
            Key::ExternalCompletionParticipant(id) => {
                self.external_completion_participants
                    .terminal_grants
                    .remove(id);
            }
            Key::LeaseReceipt(id) => {
                self.completion_leases.receipts.remove(id);
            }
            Key::ArchiveMaintenanceReceipt(id) => {
                self.archive_maintenance_receipts.remove(id);
            }
        }
    }

    /// Attaches one stored hot terminal record: its body, its candidate
    /// entry, and, for a reservation, its place in the reservation index. The
    /// ledger already counts it.
    fn attach_hot_terminal(
        &mut self,
        entry: ResourceHotTerminalEntryV1,
    ) -> Result<(), ResourceError> {
        use ResourceTerminalArchivePayloadV1 as Payload;
        if !entry.names_its_payload()
            || (!entry.reservations.is_empty()
                && !matches!(entry.key, ResourceTerminalRecordKeyV1::Demand(_)))
        {
            return Err(ResourceError::InvalidDefinition(
                "resource hot terminal entry does not hold the record its key names".to_owned(),
            ));
        }
        let duplicate = || {
            ResourceError::InvalidDefinition(
                "resource hot terminal record is already in the root".to_owned(),
            )
        };
        if self
            .terminal_archive_candidates
            .insert(entry.sequence, entry.key.clone())
            .is_some()
        {
            return Err(duplicate());
        }
        let replaced = match entry.payload {
            Payload::Outcome(value) => self
                .outcomes
                .insert(value.operation_key.clone(), value)
                .is_some(),
            Payload::Demand(value) => self.demands.insert(value.id.clone(), value).is_some(),
            Payload::Reservation(value) => {
                self.reservation_by_demand
                    .entry(value.demand.clone())
                    .or_default()
                    .insert(value.id.clone());
                self.reservations.insert(value.id.clone(), value).is_some()
            }
            Payload::AllocationLeg(value) => self
                .allocation_legs
                .insert(value.id.clone(), value)
                .is_some(),
            Payload::Consumption(value) => {
                self.consumptions.insert(value.id.clone(), value).is_some()
            }
            Payload::Fulfillment(value) => {
                self.fulfillments.insert(value.id.clone(), value).is_some()
            }
            Payload::Loss(value) => self.losses.insert(value.id.clone(), value).is_some(),
            Payload::Transfer(value) => self.transfers.insert(value.id.clone(), value).is_some(),
            Payload::ExternalCompletionParticipant(value) => self
                .external_completion_participants
                .terminal_grants
                .insert(value.grant.acquisition.clone(), value)
                .is_some(),
            Payload::LeaseReceipt(value) => self
                .completion_leases
                .receipts
                .insert(value.sequence, value)
                .is_some(),
            Payload::ArchiveMaintenanceReceipt(value) => self
                .archive_maintenance_receipts
                .insert(value.sequence, value)
                .is_some(),
        };
        if replaced {
            return Err(duplicate());
        }
        Ok(())
    }

    /// Splits this state into its live root and the entries of its attached
    /// hot terminal records. The live root's ledger then counts the encoded
    /// bytes of every hot terminal entry, attached or not.
    pub(crate) fn split_hot_terminal(
        mut self,
    ) -> Result<(Self, Vec<ResourceHotTerminalEntryV1>), ResourceError> {
        let mut entries = Vec::with_capacity(self.terminal_archive_candidates.len());
        for (sequence, key) in &self.terminal_archive_candidates {
            entries.push(self.hot_terminal_entry(*sequence, key)?);
        }
        let mut bytes = self.hot_terminal.bytes;
        for entry in &entries {
            bytes = bytes
                .checked_add(entry.encoded_len()?)
                .ok_or(ResourceError::Overflow)?;
            self.detach_hot_terminal(&entry.key);
        }
        self.terminal_archive_candidates.clear();
        self.hot_terminal.bytes = bytes;
        Ok((self, entries))
    }

    /// Bytes the root occupies persisted: its live root encoded, and every
    /// hot terminal entry. This is what `max_state_bytes` bounds.
    pub(crate) fn encoded_root_bytes(&self) -> Result<usize, ResourceError> {
        let (live, _) = self.clone().split_hot_terminal()?;
        // The live root encodes as its state does.
        let head = serde_json::to_vec(&live)
            .map_err(|error| {
                ResourceError::InvalidDefinition(format!(
                    "resource authoritative state cannot be encoded: {error}"
                ))
            })?
            .len();
        usize::try_from(live.hot_terminal.bytes)
            .ok()
            .and_then(|bytes| bytes.checked_add(head))
            .ok_or(ResourceError::Overflow)
    }

    /// Checks the hot terminal ledger against the attached records: equal
    /// when every record is attached, covering them otherwise, and within the
    /// root's budgets.
    pub(crate) fn validate_hot_terminal_ledger(&self) -> Result<(), ResourceError> {
        let attached = ResourceHotTerminalLedgerV1::of_attached(self)?;
        let consistent = if self.hot_terminal_records_attached() {
            ResourceHotTerminalLedgerV1 {
                bytes: self.hot_terminal.bytes,
                ..attached.clone()
            } == self.hot_terminal
                && self.hot_terminal.bytes == 0
        } else {
            self.hot_terminal.covers(&attached)
        };
        // Outcomes, consumptions, fulfillments, losses, and lease receipts are
        // archive candidates from the moment they are recorded.
        let always_candidates = self.outcomes.len()
            + self.consumptions.len()
            + self.fulfillments.len()
            + self.losses.len()
            + self.completion_leases.receipts.len();
        let attached_always = self
            .terminal_archive_candidates
            .values()
            .filter(|key| {
                matches!(
                    key,
                    ResourceTerminalRecordKeyV1::Outcome(_)
                        | ResourceTerminalRecordKeyV1::Consumption(_)
                        | ResourceTerminalRecordKeyV1::Fulfillment(_)
                        | ResourceTerminalRecordKeyV1::Loss(_)
                        | ResourceTerminalRecordKeyV1::LeaseReceipt(_)
                )
            })
            .count();
        if !consistent || always_candidates != attached_always {
            return Err(ResourceError::InvalidDefinition(
                "resource hot terminal ledger differs from the hot terminal records".to_owned(),
            ));
        }
        if self.hot_terminal_records() > self.limits.max_archive_candidates
            || self.hot_operation_outcomes() > self.limits.max_operation_outcomes
            || usize::try_from(self.hot_terminal.applied_archive_maintenance_receipts)
                .unwrap_or(usize::MAX)
                > self.limits.max_archive_maintenance_receipts
        {
            return Err(ResourceError::LimitExceeded(
                "resource authoritative records exceed configured limits".to_owned(),
            ));
        }
        Ok(())
    }

    /// The domain records that persist this whole state: its live root,
    /// then one record per non-empty hot terminal bucket in bucket order.
    /// This is how a scenario installs a resource root.
    pub fn into_records(mut self) -> Result<Vec<DomainRecord>, CanwuError> {
        if !self.hot_terminal_records_attached() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "a resource root persists whole: some hot terminal records are not attached",
            ));
        }
        self.refresh_continuation();
        self.validate().map_err(resource_canwu_error)?;
        let version = self.state_revision.get();
        let buckets = resource_hot_terminal_bucket_count(&self.limits);
        let (live, entries) = self.split_hot_terminal().map_err(resource_canwu_error)?;
        let mut bucketed = BTreeMap::<u32, Vec<ResourceHotTerminalEntryV1>>::new();
        for entry in entries {
            bucketed
                .entry(resource_hot_terminal_bucket(&entry.key, buckets))
                .or_default()
                .push(entry);
        }
        let mut records = vec![live_root_record(live, version)?];
        for (bucket, mut entries) in bucketed {
            entries.sort_by(|left, right| left.key.cmp(&right.key));
            records.push(bucket_record(bucket, entries, 1)?);
        }
        Ok(records)
    }

    /// Restores a whole state from its live root and the records of its hot
    /// terminal buckets (any order; empty buckets may be included), checking
    /// that each entry lies in its bucket, that the ledger counts exactly the
    /// entries, and that the state is valid.
    pub fn from_records<'a>(
        live_root: &DomainRecord,
        buckets: impl IntoIterator<Item = &'a DomainRecord>,
    ) -> Result<Self, CanwuError> {
        let mut state = decode_live_root(live_root)?;
        let count = resource_hot_terminal_bucket_count(&state.limits);
        let mut seen = BTreeSet::new();
        let mut bytes = 0_u64;
        for record in buckets {
            let (bucket, entries) = stored_bucket_entries(record, count)?;
            if !seen.insert(bucket) {
                return Err(invalid("resource hot terminal bucket is listed twice"));
            }
            for value in entries.into_values() {
                let entry = decode_entry(value)?;
                bytes = bytes
                    .checked_add(entry.encoded_len().map_err(resource_canwu_error)?)
                    .ok_or_else(|| invalid("resource hot terminal bytes overflow"))?;
                state
                    .attach_hot_terminal(entry)
                    .map_err(resource_canwu_error)?;
            }
        }
        if bytes != state.hot_terminal.bytes {
            return Err(invalid(
                "resource hot terminal ledger bytes differ from the stored entries",
            ));
        }
        state.hot_terminal.bytes = 0;
        check_demand_closures(&state)?;
        state.validate().map_err(resource_canwu_error)?;
        Ok(state)
    }
}

/// Every hot terminal demand lists exactly the hot reservations that belong
/// to it, which the reservation index of a whole state holds.
fn check_demand_closures(state: &ResourceState) -> Result<(), CanwuError> {
    for key in state.terminal_archive_candidates.values() {
        if let ResourceTerminalRecordKeyV1::Reservation(id) = key {
            let demand = &state.reservations[id].demand;
            if !state.hot_candidate_attached(&ResourceTerminalRecordKeyV1::Demand(demand.clone())) {
                return Err(invalid(
                    "a hot terminal reservation belongs to a demand that is not hot terminal",
                ));
            }
        }
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}

/// The payload of the live root `resource:runtime:v1`: the resource state
/// without the bodies of its hot terminal records, which its ledger counts
/// and its buckets hold.
///
/// It encodes as that state does. Read a root through
/// [`crate::resource_state`] (host application) or
/// [`resource_root_in_view`] (boundary system or admission), which attach
/// the hot terminal records; the live root alone answers no question about
/// them.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ResourceLiveRootV1(ResourceState);

impl ResourceLiveRootV1 {
    /// The live part of the root. Its maps lack every hot terminal record.
    #[must_use]
    pub fn live(&self) -> &ResourceState {
        &self.0
    }

    /// The live part of the root. Its maps lack every hot terminal record.
    #[must_use]
    pub fn into_live(self) -> ResourceState {
        self.0
    }
}

pub(crate) fn decode_live_root(record: &DomainRecord) -> Result<ResourceState, CanwuError> {
    let state = record
        .decode_payload::<crate::ResourceRuntimeRecord>()?
        .into_live();
    if !state.terminal_archive_candidates.is_empty() {
        return Err(invalid(
            "a resource live root holds no hot terminal records itself",
        ));
    }
    if record.reference != resource_runtime_reference().into_untyped()
        || record.version != state.state_revision.get()
    {
        return Err(invalid(
            "resource runtime root identity or revision differs",
        ));
    }
    Ok(state)
}

/// The draft of the live root `live`, which has no attached hot terminal
/// record.
pub(crate) fn live_root_draft(live: ResourceState) -> Result<DomainRecordDraft, CanwuError> {
    DomainRecordDraft::from_typed(resource_runtime_reference(), &ResourceLiveRootV1(live))
}

fn live_root_record(live: ResourceState, version: u64) -> Result<DomainRecord, CanwuError> {
    let draft = live_root_draft(live)?;
    Ok(DomainRecord {
        reference: draft.reference,
        owner: crate::PLUGIN_NAME.to_owned(),
        class: DomainRecordClass::Record,
        version,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: draft.references,
    })
}

fn bucket_draft(
    bucket: u32,
    entries: Vec<ResourceHotTerminalEntryV1>,
) -> Result<DomainRecordDraft, CanwuError> {
    DomainRecordDraft::from_typed(
        resource_hot_terminal_bucket_reference(bucket),
        &ResourceHotTerminalBucketV1 {
            format_version: RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION,
            bucket,
            entries,
        },
    )
}

fn bucket_record(
    bucket: u32,
    entries: Vec<ResourceHotTerminalEntryV1>,
    version: u64,
) -> Result<DomainRecord, CanwuError> {
    let draft = bucket_draft(bucket, entries)?;
    Ok(DomainRecord {
        reference: draft.reference,
        owner: crate::PLUGIN_NAME.to_owned(),
        class: DomainRecordClass::Record,
        version,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: draft.references,
    })
}

/// Where hot terminal buckets are read: the simulation view of a boundary
/// system or admission, or the host application's runtime.
pub(crate) trait HotTerminalSource {
    fn hot_terminal_record(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<&DomainRecord>, CanwuError>;
}

impl HotTerminalSource for SimulationView<'_> {
    fn hot_terminal_record(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<&DomainRecord>, CanwuError> {
        self.domain_record(reference)
    }
}

/// A runtime handle, [`canwu_api::Canwu`] or [`canwu_api::CompactedCanwu`],
/// read as the source of a resource root's buckets.
struct RunSource<'r, H: ?Sized>(&'r H);

impl<H: CanwuRun + ?Sized> HotTerminalSource for RunSource<'_, H> {
    fn hot_terminal_record(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<&DomainRecord>, CanwuError> {
        Ok(self.0.domain_record(reference))
    }
}

/// One bucket as it was stored when the work read it: its version, and each
/// entry's encoded value by key. An entry is decoded only when attached.
struct StoredBucket<'s> {
    version: Option<u64>,
    entries: BTreeMap<ResourceTerminalRecordKeyV1, &'s Value>,
}

/// A hot terminal record attached by the work, as it was stored.
struct AttachedEntry {
    bucket: u32,
    entry: ResourceHotTerminalEntryV1,
}

/// The keys of one stored bucket of a root with `count` buckets, with each
/// entry's encoded value, checking that it is the bucket its identity names,
/// that its keys ascend, and that each lies in this bucket. Entry bodies are
/// checked when they are attached.
fn stored_bucket_entries(
    record: &DomainRecord,
    count: u32,
) -> Result<(u32, BTreeMap<ResourceTerminalRecordKeyV1, &Value>), CanwuError> {
    let malformed =
        || invalid("resource hot terminal bucket is malformed or holds a record of another bucket");
    let fields = record.payload.as_object().ok_or_else(malformed)?;
    let bucket = fields
        .get("bucket")
        .and_then(Value::as_u64)
        .and_then(|bucket| u32::try_from(bucket).ok())
        .ok_or_else(malformed)?;
    let entries = fields
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(malformed)?;
    if fields.len() != 3
        || fields.get("format_version").and_then(Value::as_u64)
            != Some(u64::from(RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION))
        || bucket >= count
        || !record
            .reference
            .kind
            .matches_type::<ResourceHotTerminalBucketRecord>()
        || record.reference != resource_hot_terminal_bucket_reference(bucket).into_untyped()
        || record.owner != crate::PLUGIN_NAME
        || record.lifecycle != DomainRecordLifecycle::Active
    {
        return Err(malformed());
    }
    let mut keyed = BTreeMap::new();
    for entry in entries {
        let key = entry
            .get("key")
            .map(ResourceTerminalRecordKeyV1::deserialize)
            .transpose()
            .map_err(|_| malformed())?
            .ok_or_else(malformed)?;
        if resource_hot_terminal_bucket(&key, count) != bucket
            || keyed
                .last_key_value()
                .is_some_and(|(last, _): (&ResourceTerminalRecordKeyV1, _)| last >= &key)
        {
            return Err(malformed());
        }
        keyed.insert(key, entry);
    }
    Ok((bucket, keyed))
}

/// Decodes one stored entry, which must be in its canonical encoding: what
/// the entry it decodes to encodes as.
fn decode_entry(value: &Value) -> Result<ResourceHotTerminalEntryV1, CanwuError> {
    let entry = ResourceHotTerminalEntryV1::deserialize(value).map_err(|error| {
        invalid(format!(
            "resource hot terminal entry cannot be decoded: {error}"
        ))
    })?;
    if serde_json::to_value(&entry).ok().as_ref() != Some(value) {
        return Err(invalid(
            "resource hot terminal entry is not in its canonical encoding",
        ));
    }
    Ok(entry)
}

/// The resource root as one boundary system, admission, or host read works
/// on it: the live root, with the hot terminal records it reads attached
/// from their buckets.
pub(crate) struct ResourceRootWork<'s, S: HotTerminalSource + ?Sized> {
    source: &'s S,
    pub(crate) live_version: u64,
    pub(crate) state: ResourceState,
    bucket_count: u32,
    buckets: BTreeMap<u32, StoredBucket<'s>>,
    attached: BTreeMap<ResourceTerminalRecordKeyV1, AttachedEntry>,
}

impl<'s, S: HotTerminalSource + ?Sized> ResourceRootWork<'s, S> {
    /// The live root `live` read from `source`, with nothing attached.
    pub(crate) fn new(source: &'s S, live: &DomainRecord) -> Result<Self, CanwuError> {
        let state = decode_live_root(live)?;
        Ok(Self {
            source,
            live_version: live.version,
            bucket_count: resource_hot_terminal_bucket_count(&state.limits),
            state,
            buckets: BTreeMap::new(),
            attached: BTreeMap::new(),
        })
    }

    fn bucket(&mut self, bucket: u32) -> Result<&StoredBucket<'s>, CanwuError> {
        if !self.buckets.contains_key(&bucket) {
            let source: &'s S = self.source;
            let stored = match source
                .hot_terminal_record(resource_hot_terminal_bucket_reference(bucket).as_untyped())?
            {
                None => StoredBucket {
                    version: None,
                    entries: BTreeMap::new(),
                },
                Some(record) => StoredBucket {
                    version: Some(record.version),
                    entries: stored_bucket_entries(record, self.bucket_count)?.1,
                },
            };
            self.buckets.insert(bucket, stored);
        }
        Ok(&self.buckets[&bucket])
    }

    /// Whether the state holds the body of `key`, live or attached.
    fn holds(&self, key: &ResourceTerminalRecordKeyV1) -> bool {
        use ResourceTerminalRecordKeyV1 as Key;
        let state = &self.state;
        match key {
            Key::Outcome(id) => state.outcomes.contains_key(id),
            Key::Demand(id) => state.demands.contains_key(id),
            Key::Reservation(id) => state.reservations.contains_key(id),
            Key::AllocationLeg(id) => state.allocation_legs.contains_key(id),
            Key::Consumption(id) => state.consumptions.contains_key(id),
            Key::Fulfillment(id) => state.fulfillments.contains_key(id),
            Key::Loss(id) => state.losses.contains_key(id),
            Key::Transfer(id) => state.transfers.contains_key(id),
            Key::ExternalCompletionParticipant(id) => state
                .external_completion_participants
                .participant(id)
                .is_some(),
            Key::LeaseReceipt(id) => state.completion_leases.receipts.contains_key(id),
            Key::ArchiveMaintenanceReceipt(id) => {
                state.archive_maintenance_receipts.contains_key(id)
            }
        }
    }

    /// The records a record the state holds names that a reader may need
    /// attached beside it.
    fn held_references(
        &self,
        key: &ResourceTerminalRecordKeyV1,
    ) -> Vec<ResourceTerminalRecordKeyV1> {
        use ResourceTerminalArchivePayloadV1 as Payload;
        use ResourceTerminalRecordKeyV1 as Key;
        let state = &self.state;
        let payload = match key {
            Key::Reservation(id) => state
                .reservations
                .get(id)
                .cloned()
                .map(Payload::Reservation),
            Key::AllocationLeg(id) => state
                .allocation_legs
                .get(id)
                .cloned()
                .map(Payload::AllocationLeg),
            Key::Transfer(id) => state.transfers.get(id).cloned().map(Payload::Transfer),
            Key::Consumption(id) => state
                .consumptions
                .get(id)
                .cloned()
                .map(Payload::Consumption),
            Key::Fulfillment(id) => state
                .fulfillments
                .get(id)
                .cloned()
                .map(Payload::Fulfillment),
            Key::Loss(id) => state.losses.get(id).cloned().map(Payload::Loss),
            // A demand's reservations become hot terminal with it, so a live
            // demand names no hot terminal reservation, and an attached one
            // attached its closure already.
            Key::Outcome(_)
            | Key::Demand(_)
            | Key::ExternalCompletionParticipant(_)
            | Key::LeaseReceipt(_)
            | Key::ArchiveMaintenanceReceipt(_) => None,
        };
        payload.as_ref().map(payload_references).unwrap_or_default()
    }

    /// Attaches the hot terminal records `keys` name, and every record those
    /// name in turn: the closure of a demand, and what a reservation,
    /// allocation leg, consumption, fulfillment, transfer, or loss belongs
    /// to, followed through live records as well. A key that names no hot
    /// terminal record is ignored.
    pub(crate) fn attach(
        &mut self,
        keys: impl IntoIterator<Item = ResourceTerminalRecordKeyV1>,
    ) -> Result<(), CanwuError> {
        let mut pending: Vec<_> = keys.into_iter().collect();
        let mut visited = BTreeSet::new();
        while let Some(key) = pending.pop() {
            if !visited.insert(key.clone()) {
                continue;
            }
            if self.holds(&key) {
                pending.extend(self.held_references(&key));
                continue;
            }
            // Attached before and no longer held: this work archived it.
            if self.attached.contains_key(&key) {
                continue;
            }
            let bucket = resource_hot_terminal_bucket(&key, self.bucket_count);
            let Some(value) = self.bucket(bucket)?.entries.get(&key).copied() else {
                continue;
            };
            let entry = decode_entry(value)?;
            if entry.key != key {
                return Err(invalid(
                    "resource hot terminal entry differs from its stored key",
                ));
            }
            let size = entry.encoded_len().map_err(resource_canwu_error)?;
            pending.extend(entry.references());
            self.state
                .attach_hot_terminal(entry.clone())
                .map_err(resource_canwu_error)?;
            self.state.hot_terminal.bytes = self
                .state
                .hot_terminal
                .bytes
                .checked_sub(size)
                .ok_or_else(|| invalid("resource hot terminal ledger bytes are short"))?;
            self.attached.insert(key, AttachedEntry { bucket, entry });
        }
        Ok(())
    }

    /// Attaches every hot terminal record that `value` names by an identity
    /// string, with their closures ([`Self::attach`]).
    pub(crate) fn attach_named(&mut self, value: &Value) -> Result<(), CanwuError> {
        let mut texts = BTreeSet::new();
        named_identities(value, &mut texts);
        let keys = texts
            .iter()
            .flat_map(|text| keys_named_by(text))
            .collect::<Vec<_>>();
        self.attach(keys)
    }

    /// Attaches every hot terminal record of the root, which makes the state
    /// whole.
    pub(crate) fn attach_all(&mut self) -> Result<(), CanwuError> {
        let mut keys = Vec::new();
        for bucket in 0..self.bucket_count {
            keys.extend(self.bucket(bucket)?.entries.keys().cloned());
        }
        self.attach(keys)?;
        if !self.state.hot_terminal_records_attached() || self.state.hot_terminal.bytes != 0 {
            return Err(invalid(
                "resource hot terminal ledger differs from the stored entries",
            ));
        }
        Ok(())
    }

    /// The record mutations that persist the state as it now is: the live
    /// root, as an update of the version this work read, and every bucket
    /// whose entries changed. A record that stays attached and unchanged is
    /// not written again. The caller sets the state revision and validates
    /// the state first. The work is spent: its state is left empty.
    ///
    /// A record the work did not attach that its bucket already holds means
    /// the settlement read past a hot terminal record: it fails with
    /// `invalid_boundary`, an engine invariant, instead of writing over it.
    pub(crate) fn persist(&mut self, summary: &str) -> Result<Vec<BoundaryDirective>, CanwuError> {
        let empty = ResourceState::empty(self.state.limits).map_err(resource_canwu_error)?;
        let state = std::mem::replace(&mut self.state, empty);
        let (live, entries) = state.split_hot_terminal().map_err(resource_canwu_error)?;
        let present: BTreeMap<_, _> = entries
            .into_iter()
            .map(|entry| (entry.key.clone(), entry))
            .collect();
        let mut changed = BTreeMap::<u32, BTreeSet<ResourceTerminalRecordKeyV1>>::new();
        for (key, attached) in &self.attached {
            if present.get(key) != Some(&attached.entry) {
                changed
                    .entry(attached.bucket)
                    .or_default()
                    .insert(key.clone());
            }
        }
        for key in present.keys() {
            if !self.attached.contains_key(key) {
                changed
                    .entry(resource_hot_terminal_bucket(key, self.bucket_count))
                    .or_default()
                    .insert(key.clone());
            }
        }
        let mut directives = vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: live_root_draft(live)?,
                expected_version: self.live_version,
            },
            summary: summary.to_owned(),
        }];
        for (bucket, keys) in changed {
            let stored = self.bucket(bucket)?;
            let version = stored.version;
            let mut entries: BTreeMap<_, _> = stored
                .entries
                .iter()
                .map(|(key, value)| (key.clone(), (*value).clone()))
                .collect();
            for key in keys {
                match present.get(&key) {
                    Some(entry) => {
                        // A record this work did not attach is new to the
                        // root. One already stored was read past: the
                        // settlement could not have seen it, so writing over
                        // it would record a second, unseen answer for it.
                        if !self.attached.contains_key(&key) && entries.contains_key(&key) {
                            return Err(CanwuError::new(
                                ErrorCode::InvalidBoundary,
                                "a resource boundary would overwrite a hot terminal record it did not read",
                            ));
                        }
                        let value = serde_json::to_value(entry)
                            .map_err(|error| invalid(error.to_string()))?;
                        entries.insert(key, value);
                    }
                    None => {
                        entries.remove(&key);
                    }
                }
            }
            let record = DomainRecordDraft::new(
                resource_hot_terminal_bucket_reference(bucket).into_untyped(),
                serde_json::json!({
                    "format_version": RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION,
                    "bucket": bucket,
                    "entries": entries.into_values().collect::<Vec<_>>(),
                }),
            );
            directives.push(BoundaryDirective::MutateRecord {
                mutation: match version {
                    None => DomainRecordMutation::Create { record },
                    Some(expected_version) => DomainRecordMutation::Update {
                        record,
                        expected_version,
                    },
                },
                summary: summary.to_owned(),
            });
        }
        Ok(directives)
    }
}

/// The resource root as a boundary system or command admission of another
/// plugin reads it: the live root, with the hot terminal records it asks for
/// attached.
///
/// [`Self::attach`] attaches the hot terminal records some keys name, and
/// every record those name in turn (the closure of a demand, and what a
/// reservation, allocation leg, consumption, fulfillment, transfer, or loss
/// belongs to). The state then answers every question about its live part
/// and about the attached records exactly as the whole root would; a hot
/// terminal record it was not asked for is absent from its maps. Its ledger
/// ([`ResourceState::hot_terminal_records`],
/// [`ResourceState::hot_operation_outcomes`]) counts every hot terminal
/// record. Reading needs the `canwu.resource` `runtime` and `hot-terminal`
/// state keys ([`crate::resource_hot_terminal_state_key`]).
pub struct ResourceRootReader<'v, 'a> {
    live: DomainRecord,
    work: ResourceRootWork<'v, SimulationView<'a>>,
}

impl<'v, 'a> ResourceRootReader<'v, 'a> {
    /// The live root of the view, with nothing attached, or `None` without a
    /// resource root.
    pub fn load(view: &'v SimulationView<'a>) -> Result<Option<Self>, CanwuError> {
        let Some(live) = view.typed_domain_record(&resource_runtime_reference())? else {
            return Ok(None);
        };
        let live = live.clone();
        let work = ResourceRootWork::new(view, &live)?;
        Ok(Some(Self { live, work }))
    }

    /// Attaches the hot terminal records `keys` name, with their closures. A
    /// key that names no hot terminal record is ignored.
    pub fn attach(
        &mut self,
        keys: impl IntoIterator<Item = ResourceTerminalRecordKeyV1>,
    ) -> Result<(), CanwuError> {
        self.work.attach(keys)
    }

    /// The live root record, whose version is the root's state revision.
    #[must_use]
    pub fn live_record(&self) -> &DomainRecord {
        &self.live
    }

    /// The live root with the attached hot terminal records.
    #[must_use]
    pub fn state(&self) -> &ResourceState {
        &self.work.state
    }

    /// The live root with the attached hot terminal records.
    #[must_use]
    pub fn into_state(self) -> ResourceState {
        self.work.state
    }
}

/// Reads the resource root in a boundary system or in command admission:
/// the live root, with the hot terminal records `keys` name attached
/// ([`ResourceRootReader`]). Returns the live root record beside the state,
/// or `None` without a resource root.
pub fn resource_root_in_view(
    view: &SimulationView<'_>,
    keys: impl IntoIterator<Item = ResourceTerminalRecordKeyV1>,
) -> Result<Option<(DomainRecord, ResourceState)>, CanwuError> {
    let Some(mut reader) = ResourceRootReader::load(view)? else {
        return Ok(None);
    };
    reader.attach(keys)?;
    Ok(Some((reader.live, reader.work.state)))
}

/// Reads the whole resource root in a boundary system: the live root and
/// every hot terminal record. Its cost grows with the hot terminal backlog,
/// so a boundary system that needs only some records reads them with
/// [`resource_root_in_view`].
pub fn whole_resource_root_in_view(
    view: &SimulationView<'_>,
) -> Result<Option<(DomainRecord, ResourceState)>, CanwuError> {
    let Some(live) = view.typed_domain_record(&resource_runtime_reference())? else {
        return Ok(None);
    };
    let live = live.clone();
    let mut work = ResourceRootWork::new(view, &live)?;
    work.attach_all()?;
    Ok(Some((live, work.state)))
}

/// Reads the whole resource root of a run, from either runtime handle: the
/// live root record and the state with every hot terminal record attached.
pub(crate) fn whole_resource_root<H: CanwuRun + ?Sized>(
    canwu: &H,
) -> Result<Option<(DomainRecord, ResourceState)>, CanwuError> {
    let Some(live) = canwu_api::typed_domain_record(canwu, &resource_runtime_reference()) else {
        return Ok(None);
    };
    let live = live.clone();
    let source = RunSource(canwu);
    let mut work = ResourceRootWork::new(&source, &live)?;
    work.attach_all()?;
    check_demand_closures(&work.state)?;
    Ok(Some((live, work.state)))
}
