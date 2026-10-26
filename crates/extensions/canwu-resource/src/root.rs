//! How the resource root is persisted: one live-root record, the hot terminal
//! records in buckets of their own, and the live records that grow with the
//! world (accounts, access grants, resource definitions, and unit revisions)
//! in live buckets.
//!
//! The resource root is one [`ResourceState`]. It persists as
//!
//! - the **live root**, the domain record `resource:runtime:v1`: every part of
//!   the state except the bodies of its hot terminal records (its archive
//!   candidates) and their candidate index, and except its accounts, access
//!   grants, definitions, and unit revisions, plus a
//!   [`ResourceHotTerminalLedgerV1`] and a [`ResourceLiveLedgerV1`] that count
//!   them;
//! - the **hot terminal buckets**, domain records
//!   `resource:hot-terminal:{bucket}`: each hot terminal record, with its
//!   terminal sequence, in the bucket its identity hashes to
//!   ([`resource_hot_terminal_bucket`]); and
//! - the **live buckets**, domain records `resource:live:{bucket}`: each
//!   account, access grant, definition, and unit revision in the bucket its
//!   identity hashes to ([`resource_live_bucket`]).
//!
//! Both kinds of bucket are records of one kind, `canwu.resource` `bucket`
//! ([`ResourceBucketRecord`]), so a reader declares one state key for both
//! ([`crate::resource_hot_terminal_state_key`]).
//!
//! A bucketed record adds nothing to the live root but its counts in a
//! ledger, so a boundary that settles an operation decodes, validates,
//! re-encodes, and journals the live root and the buckets of the records it
//! reads or writes, not the whole backlog or the whole world. The split is a
//! function of the state: the same state always persists as the same records
//! ([`ResourceState::into_records`]), and [`ResourceState::from_records`]
//! restores it.
//!
//! In a boundary the resource plugin works on a *partial* state: the live root
//! with only the bucketed records it reads attached. Every rule that counts or
//! aggregates bucketed records reads a ledger, so a partial state settles every
//! operation exactly as the whole state would, as long as the records an
//! operation reads by identity are attached. The plugin attaches every
//! bucketed record a packet names, and every record those name in turn,
//! before it settles the packet.

use crate::{
    CompletionCapacityGrantId, CompletionLeaseAcquisitionId, CompletionLockedTargetV1,
    PLUGIN_NAMESPACE, ProtectedFloorPolicyRevisionId, ResourceAccessGrantId,
    ResourceAccessGrantRecordV1, ResourceAccount, ResourceAccountId, ResourceDefinitionRevision,
    ResourceDefinitionRevisionId, ResourceError, ResourceLimitsV1, ResourceReservationId,
    ResourceState, ResourceTerminalArchivePayloadV1, ResourceTerminalRecordKeyV1,
    ResourceUnitRevision, ResourceUnitRevisionId, resource_canwu_error, resource_runtime_reference,
};
use canwu_api::{
    BoundaryDirective, CanwuError, CanwuRun, DomainRecord, DomainRecordClass, DomainRecordDraft,
    DomainRecordLifecycle, DomainRecordMutation, DomainRecordRef, DomainRecordType,
    DomainValueKindClass, ErrorCode, IDENTITY_EVIDENCE_DEPENDENCIES_FIELD,
    IdentityEvidenceDependenciesV1, KnowledgeHolderRef, SimulationView, TypedDomainRecordRef,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Format of a hot terminal bucket ([`ResourceHotTerminalBucketV1`]). 2 since
/// 0.20.0: the bucket declares its entries' exact evidence identity-only.
pub const RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION: u32 = 2;

/// Format of a live bucket ([`ResourceLiveBucketV1`]). 2 since 0.20.0: the
/// bucket carries the identity-only declaration its record kind requires,
/// always empty.
pub const RESOURCE_LIVE_BUCKET_FORMAT_VERSION: u32 = 2;

/// Hot terminal records one bucket holds, on average, at a full candidate
/// budget ([`resource_hot_terminal_bucket_count`]). An archive commit
/// rewrites each bucket it empties once, so it writes about one bucket per
/// this many archived records; an ordinary boundary rewrites the bucket of
/// each record it adds with the records that share it.
pub const RESOURCE_HOT_TERMINAL_RECORDS_PER_BUCKET: usize = 2;

/// Live records one live bucket holds, on average, when every live budget of
/// the root is full ([`resource_live_bucket_count`]).
pub const RESOURCE_LIVE_RECORDS_PER_BUCKET: usize = 4;

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
}

/// What one hot terminal record adds to the ledger, read from its body.
struct HotTerminalFacts<'a> {
    requester: Option<&'a KnowledgeHolderRef>,
    floor_policy: Option<&'a ProtectedFloorPolicyRevisionId>,
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
        let facts = match key {
            ResourceTerminalRecordKeyV1::Demand(id) => {
                let demand = state.demands.get(id).ok_or_else(|| {
                    ResourceError::NotFound("hot terminal demand is not attached".to_owned())
                })?;
                HotTerminalFacts {
                    requester: Some(&demand.requester),
                    floor_policy: demand.protected_floor_policy.as_ref(),
                }
            }
            _ => HotTerminalFacts {
                requester: None,
                floor_policy: None,
            },
        };
        self.count_facts(key, &facts, add)
    }

    /// Counts a hot terminal record in or out from its key and the facts its
    /// body holds.
    fn count_facts(
        &mut self,
        key: &ResourceTerminalRecordKeyV1,
        facts: &HotTerminalFacts<'_>,
        add: bool,
    ) -> Result<(), ResourceError> {
        step(&mut self.records, add)?;
        match key {
            ResourceTerminalRecordKeyV1::Outcome(_) => step(&mut self.outcomes, add)?,
            ResourceTerminalRecordKeyV1::Transfer(_) => step(&mut self.transfers, add)?,
            ResourceTerminalRecordKeyV1::ArchiveMaintenanceReceipt(_) => {
                step(&mut self.applied_archive_maintenance_receipts, add)?;
            }
            ResourceTerminalRecordKeyV1::Demand(_) => {
                let requester = facts.requester.ok_or_else(|| {
                    ResourceError::InvalidDefinition(
                        "hot terminal demand facts name no requester".to_owned(),
                    )
                })?;
                step_key(&mut self.demand_requesters, requester, add)?;
                if let Some(policy) = facts.floor_policy {
                    step_key(&mut self.demand_floor_policies, policy, add)?;
                }
            }
            ResourceTerminalRecordKeyV1::Reservation(_)
            | ResourceTerminalRecordKeyV1::AllocationLeg(_)
            | ResourceTerminalRecordKeyV1::Consumption(_)
            | ResourceTerminalRecordKeyV1::Fulfillment(_)
            | ResourceTerminalRecordKeyV1::Loss(_)
            | ResourceTerminalRecordKeyV1::ExternalCompletionParticipant(_)
            | ResourceTerminalRecordKeyV1::LeaseReceipt(_) => {}
        }
        Ok(())
    }

    /// Whether every count of `attached` is at most this ledger's.
    fn covers(&self, attached: &Self) -> bool {
        attached.records <= self.records
            && attached.outcomes <= self.outcomes
            && attached.transfers <= self.transfers
            && attached.applied_archive_maintenance_receipts
                <= self.applied_archive_maintenance_receipts
            && covers_map(&self.demand_requesters, &attached.demand_requesters)
            && covers_map(&self.demand_floor_policies, &attached.demand_floor_policies)
    }
}

fn covers_map<K: Ord>(ledger: &BTreeMap<K, u64>, attached: &BTreeMap<K, u64>) -> bool {
    attached
        .iter()
        .all(|(key, count)| ledger.get(key).is_some_and(|total| total >= count))
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

fn step_amount(value: &mut u64, amount: u64, add: bool) -> Result<(), ResourceError> {
    *value = if add {
        value.checked_add(amount)
    } else {
        value.checked_sub(amount)
    }
    .ok_or(ResourceError::Overflow)?;
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

/// The identity of one record a live bucket holds.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum ResourceLiveRecordKeyV1 {
    Account(ResourceAccountId),
    AccessGrant(ResourceAccessGrantId),
    Definition(ResourceDefinitionRevisionId),
    Unit(ResourceUnitRevisionId),
    /// The accounts one custodian holds ([`ResourceCustodianAccountsV1`]).
    Custodian(KnowledgeHolderRef),
}

/// The accounts one custodian holds: the index a reader of a holder's stock
/// attaches ([`ResourceRootReader::attach_custodian_accounts`]) instead of
/// reading every account. A custodian never changes, and an account is
/// created once, so the entry changes only when the custodian gains an
/// account.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceCustodianAccountsV1 {
    pub holder: KnowledgeHolderRef,
    pub accounts: BTreeSet<ResourceAccountId>,
}

/// The body of one record a live bucket holds.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "record", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ResourceLivePayloadV1 {
    Account(ResourceAccount),
    AccessGrant(ResourceAccessGrantRecordV1),
    Definition(ResourceDefinitionRevision),
    Unit(ResourceUnitRevision),
    Custodian(ResourceCustodianAccountsV1),
}

/// One live record as its bucket holds it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceLiveEntryV1 {
    /// The record's identity, which decides its bucket.
    pub key: ResourceLiveRecordKeyV1,
    /// The record itself.
    pub payload: ResourceLivePayloadV1,
}

/// One bucket of live records: the payload of `resource:live:{bucket}`. Its
/// entries are those whose identity hashes to `bucket`, in ascending key
/// order.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceLiveBucketV1 {
    /// [`RESOURCE_LIVE_BUCKET_FORMAT_VERSION`].
    pub format_version: u32,
    /// The bucket's index, below the root's live bucket count.
    pub bucket: u32,
    /// The bucket's live records, in ascending key order.
    pub entries: Vec<ResourceLiveEntryV1>,
    /// The identity-only declaration the bucket record kind requires. A live
    /// record's citations are read only while it is current, so it is always
    /// empty.
    #[serde(rename = "canwu_identity_evidence_dependencies")]
    pub evidence: IdentityEvidenceDependenciesV1,
}

/// Counts and totals over the live records of a resource root that persist
/// in live buckets, persisted in its live root.
///
/// In a persisted live root it counts every account, access grant,
/// definition, and unit revision, and `bytes` is the encoded size of every
/// live entry. In memory it counts only the records that are not attached to
/// the state: a whole state, with every record attached, holds the default.
/// Every rule that counts these records, or totals account balances (the
/// conservation check), or asks whether an account names a protected-floor
/// policy, reads the attached records and this ledger together.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceLiveLedgerV1 {
    pub accounts: u64,
    /// The balances of the counted accounts.
    pub balance: u128,
    /// The counted accounts by the protected-floor policy they name.
    pub account_floor_policies: BTreeMap<ProtectedFloorPolicyRevisionId, u64>,
    pub access_grants: u64,
    pub definitions: u64,
    pub units: u64,
    /// Custodian account indexes ([`ResourceCustodianAccountsV1`]).
    pub custodians: u64,
    /// Encoded bytes of the counted live entries.
    pub bytes: u64,
}

impl ResourceLiveLedgerV1 {
    /// Counts one live entry of `bytes` encoded bytes in or out.
    fn count(
        &mut self,
        payload: &ResourceLivePayloadV1,
        bytes: u64,
        add: bool,
    ) -> Result<(), ResourceError> {
        step_amount(&mut self.bytes, bytes, add)?;
        match payload {
            ResourceLivePayloadV1::Account(account) => {
                step(&mut self.accounts, add)?;
                let balance = u128::from(account.balance);
                self.balance = if add {
                    self.balance.checked_add(balance)
                } else {
                    self.balance.checked_sub(balance)
                }
                .ok_or(ResourceError::Overflow)?;
                if let Some(policy) = &account.protected_floor_policy {
                    step_key(&mut self.account_floor_policies, policy, add)?;
                }
            }
            ResourceLivePayloadV1::AccessGrant(_) => step(&mut self.access_grants, add)?,
            ResourceLivePayloadV1::Definition(_) => step(&mut self.definitions, add)?,
            ResourceLivePayloadV1::Unit(_) => step(&mut self.units, add)?,
            ResourceLivePayloadV1::Custodian(_) => step(&mut self.custodians, add)?,
        }
        Ok(())
    }

    /// Whether the ledger counts no record: every live record is attached.
    fn is_empty(&self) -> bool {
        self == &Self::default()
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

/// A key of the work list that attaches records to a partial state.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum RootKey {
    Terminal(ResourceTerminalRecordKeyV1),
    Live(ResourceLiveRecordKeyV1),
    /// The exact targets of the grants of a completion lease acquisition, or
    /// of an external participant grant, held in the live root.
    Lease(CompletionLeaseAcquisitionId),
    /// The exact targets of one completion lease grant held in the live root.
    LeaseGrant(CompletionCapacityGrantId),
}

impl From<ResourceTerminalRecordKeyV1> for RootKey {
    fn from(value: ResourceTerminalRecordKeyV1) -> Self {
        Self::Terminal(value)
    }
}

impl From<ResourceLiveRecordKeyV1> for RootKey {
    fn from(value: ResourceLiveRecordKeyV1) -> Self {
        Self::Live(value)
    }
}

fn account_key(id: &ResourceAccountId) -> RootKey {
    RootKey::Live(ResourceLiveRecordKeyV1::Account(id.clone()))
}

fn target_keys(target: &CompletionLockedTargetV1) -> Option<RootKey> {
    use ResourceTerminalRecordKeyV1 as Key;
    match target {
        CompletionLockedTargetV1::Account { id, .. } => Some(account_key(id)),
        CompletionLockedTargetV1::AllocationLeg { id, .. } => {
            Some(RootKey::Terminal(Key::AllocationLeg(id.clone())))
        }
        CompletionLockedTargetV1::Demand { id, .. } => {
            Some(RootKey::Terminal(Key::Demand(id.clone())))
        }
        CompletionLockedTargetV1::Transfer { id, .. } => {
            Some(RootKey::Terminal(Key::Transfer(id.clone())))
        }
        CompletionLockedTargetV1::ExternalRecord { .. } => None,
    }
}

impl ResourceHotTerminalEntryV1 {
    /// The records this record names that a reader may need attached beside
    /// it: the closure of a demand, and the records a reservation, allocation
    /// leg, consumption, fulfillment, transfer, or loss belongs to.
    fn references(&self) -> Vec<RootKey> {
        let mut references = payload_references(&self.payload);
        references.extend(
            self.reservations
                .iter()
                .cloned()
                .map(|id| RootKey::Terminal(ResourceTerminalRecordKeyV1::Reservation(id))),
        );
        references
    }

    /// Bytes this entry adds to the encoded root.
    fn encoded_len(&self) -> Result<u64, ResourceError> {
        encoded_len(self)
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

impl ResourceLiveEntryV1 {
    /// The records this live record names that a reader may need attached
    /// beside it: the definition and unit of an account or an access grant,
    /// and the canonical unit of a definition.
    fn references(&self) -> Vec<RootKey> {
        live_payload_references(&self.payload)
    }

    fn names_its_payload(&self) -> bool {
        use ResourceLivePayloadV1 as Payload;
        use ResourceLiveRecordKeyV1 as Key;
        match (&self.key, &self.payload) {
            (Key::Account(id), Payload::Account(value)) => &value.id == id,
            (Key::AccessGrant(id), Payload::AccessGrant(value)) => &value.grant.grant_id == id,
            (Key::Definition(id), Payload::Definition(value)) => &value.id == id,
            (Key::Unit(id), Payload::Unit(value)) => &value.id == id,
            (Key::Custodian(holder), Payload::Custodian(value)) => &value.holder == holder,
            _ => false,
        }
    }
}

fn encoded_len<T: Serialize>(value: &T) -> Result<u64, ResourceError> {
    /// Counts the bytes written to it and keeps none.
    struct Counter(u64);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len() as u64);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value).map_err(|error| {
        ResourceError::InvalidDefinition(format!(
            "resource bucket entry cannot be encoded: {error}"
        ))
    })?;
    Ok(counter.0)
}

fn live_payload_references(payload: &ResourceLivePayloadV1) -> Vec<RootKey> {
    use ResourceLiveRecordKeyV1 as Key;
    match payload {
        ResourceLivePayloadV1::Account(value) => vec![
            RootKey::Live(Key::Definition(value.resource_revision.clone())),
            RootKey::Live(Key::Unit(value.unit_revision.clone())),
        ],
        ResourceLivePayloadV1::AccessGrant(value) => vec![
            RootKey::Live(Key::Definition(value.grant.resource_revision.clone())),
            RootKey::Live(Key::Unit(value.grant.unit_revision.clone())),
        ],
        ResourceLivePayloadV1::Definition(value) => {
            vec![RootKey::Live(Key::Unit(value.canonical_unit.clone()))]
        }
        // The accounts an index lists are attached only when asked for.
        ResourceLivePayloadV1::Unit(_) | ResourceLivePayloadV1::Custodian(_) => Vec::new(),
    }
}

fn demand_references(value: &crate::ResourceDemand) -> Vec<RootKey> {
    use ResourceLiveRecordKeyV1 as Key;
    let mut references = vec![
        RootKey::Live(Key::Definition(value.resource_revision.clone())),
        RootKey::Live(Key::Unit(value.unit_revision.clone())),
    ];
    match &value.source_policy {
        crate::ResourceDemandSourcePolicyV1::Pooled => {}
        crate::ResourceDemandSourcePolicyV1::ExactAccounts(accounts) => {
            references.extend(accounts.iter().map(account_key));
        }
        crate::ResourceDemandSourcePolicyV1::Granted { grant_id, accounts } => {
            references.push(RootKey::Live(Key::AccessGrant(grant_id.clone())));
            references.extend(accounts.iter().map(account_key));
        }
    }
    references
}

fn payload_references(payload: &ResourceTerminalArchivePayloadV1) -> Vec<RootKey> {
    use ResourceLiveRecordKeyV1 as Live;
    use ResourceTerminalArchivePayloadV1 as Payload;
    use ResourceTerminalRecordKeyV1 as Key;
    match payload {
        Payload::Reservation(value) => vec![
            RootKey::Terminal(Key::AllocationLeg(value.allocation_leg.clone())),
            RootKey::Terminal(Key::Demand(value.demand.clone())),
            account_key(&value.account),
        ],
        Payload::AllocationLeg(value) => vec![
            RootKey::Terminal(Key::Reservation(value.reservation.clone())),
            RootKey::Terminal(Key::Demand(value.demand.clone())),
            account_key(&value.account),
        ],
        Payload::Consumption(value) => vec![
            RootKey::Terminal(Key::AllocationLeg(value.allocation_leg.clone())),
            RootKey::Terminal(Key::Demand(value.demand.clone())),
            RootKey::Terminal(Key::Fulfillment(value.fulfillment.clone())),
            account_key(&value.account),
        ],
        Payload::Fulfillment(value) => {
            std::iter::once(RootKey::Terminal(Key::Demand(value.demand.clone())))
                .chain(
                    value
                        .allocation_legs
                        .iter()
                        .cloned()
                        .map(|id| RootKey::Terminal(Key::AllocationLeg(id))),
                )
                .collect()
        }
        Payload::Transfer(value) => {
            let mut references = vec![
                RootKey::Terminal(Key::AllocationLeg(value.allocation_leg.clone())),
                account_key(&value.source),
            ];
            references.extend(value.destination.iter().map(account_key));
            references.extend(
                value
                    .access_grant
                    .iter()
                    .cloned()
                    .map(|id| RootKey::Live(Live::AccessGrant(id))),
            );
            references
        }
        Payload::Loss(value) => value
            .transfer
            .iter()
            .cloned()
            .map(|id| RootKey::Terminal(Key::Transfer(id)))
            .chain(value.account.iter().map(account_key))
            .collect(),
        Payload::Demand(value) => demand_references(value),
        Payload::Outcome(_)
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
    /// The exact evidence of its entries
    /// ([`ResourceTerminalArchivePayloadV1::exact_evidence`]), declared
    /// identity-only: a seal and a continuation checkpoint keep proof that
    /// each cited version exists until its record leaves for the archive,
    /// even after a later version of that record supersedes it within the
    /// instant. Sorted, without duplicates or initial-scenario versions.
    #[serde(rename = "canwu_identity_evidence_dependencies")]
    pub evidence: IdentityEvidenceDependenciesV1,
}

/// The identity-only declaration of a hot terminal bucket that holds
/// `entries`: the exact evidence of each.
fn hot_bucket_declaration<'a>(
    entries: impl IntoIterator<Item = &'a ResourceHotTerminalEntryV1>,
) -> IdentityEvidenceDependenciesV1 {
    let evidence = entries
        .into_iter()
        .flat_map(|entry| entry.payload.exact_evidence())
        .collect::<Vec<_>>();
    crate::identity_declaration(&evidence)
}

/// The declaration every live bucket carries: none.
fn empty_declaration() -> IdentityEvidenceDependenciesV1 {
    IdentityEvidenceDependenciesV1::new(Vec::new())
}

/// Domain record type of the resource buckets, hot terminal
/// (`resource:hot-terminal:{bucket}`, payload [`ResourceHotTerminalBucketV1`])
/// and live (`resource:live:{bucket}`, payload [`ResourceLiveBucketV1`]).
/// Its payload is read as JSON because the two shapes share the kind.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResourceBucketRecord;

impl DomainRecordType for ResourceBucketRecord {
    type Payload = Value;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = PLUGIN_NAMESPACE;
    const NAME: &'static str = "bucket";
}

const HOT_TERMINAL_BUCKET_PREFIX: &str = "resource:hot-terminal:";
const LIVE_BUCKET_PREFIX: &str = "resource:live:";

/// The domain record of hot terminal bucket `bucket`.
#[must_use]
pub fn resource_hot_terminal_bucket_reference(
    bucket: u32,
) -> TypedDomainRecordRef<ResourceBucketRecord> {
    TypedDomainRecordRef::new(format!("{HOT_TERMINAL_BUCKET_PREFIX}{bucket}"))
}

/// The domain record of live bucket `bucket`.
#[must_use]
pub fn resource_live_bucket_reference(bucket: u32) -> TypedDomainRecordRef<ResourceBucketRecord> {
    TypedDomainRecordRef::new(format!("{LIVE_BUCKET_PREFIX}{bucket}"))
}

fn power_of_two_buckets(records: usize, per_bucket: usize) -> u32 {
    u32::try_from(
        records
            .div_ceil(per_bucket)
            .max(1)
            .checked_next_power_of_two()
            .unwrap_or(1 << 31),
    )
    .unwrap_or(1 << 31)
}

/// How many hot terminal buckets a root with `limits` uses: the candidate
/// budget divided by [`RESOURCE_HOT_TERMINAL_RECORDS_PER_BUCKET`], rounded up
/// to a power of two. Fixed for the life of a root.
#[must_use]
pub fn resource_hot_terminal_bucket_count(limits: &ResourceLimitsV1) -> u32 {
    power_of_two_buckets(
        limits.max_archive_candidates,
        RESOURCE_HOT_TERMINAL_RECORDS_PER_BUCKET,
    )
}

/// How many live buckets a root with `limits` uses: every account, access
/// grant, definition, unit revision, and custodian index the root may hold
/// (one custodian per account at most), divided by
/// [`RESOURCE_LIVE_RECORDS_PER_BUCKET`], rounded up to a power of two. Fixed
/// for the life of a root. A bucket is a domain record only once it holds a
/// record, so an unused budget costs nothing.
#[must_use]
pub fn resource_live_bucket_count(limits: &ResourceLimitsV1) -> u32 {
    power_of_two_buckets(
        limits
            .max_accounts
            .saturating_add(crate::MAX_RESOURCE_ACCESS_GRANTS)
            .saturating_add(limits.max_definitions)
            .saturating_add(limits.max_unit_revisions)
            .saturating_add(limits.max_accounts),
        RESOURCE_LIVE_RECORDS_PER_BUCKET,
    )
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

fn text_bucket(domain: &[u8], kind: &str, text: &str, buckets: u32) -> u32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(&[0]);
    hasher.update(kind.as_bytes());
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
    text_bucket(
        b"canwu.resource.hot-terminal-bucket.v2",
        "",
        &key_text(key),
        buckets,
    )
}

/// The live bucket of a root with `buckets` live buckets
/// ([`resource_live_bucket_count`]) that holds the live record `key`.
#[must_use]
pub fn resource_live_bucket(key: &ResourceLiveRecordKeyV1, buckets: u32) -> u32 {
    let (kind, text): (&str, std::borrow::Cow<'_, str>) = match key {
        ResourceLiveRecordKeyV1::Account(id) => ("account", id.as_str().into()),
        ResourceLiveRecordKeyV1::AccessGrant(id) => ("access_grant", id.as_str().into()),
        ResourceLiveRecordKeyV1::Definition(id) => ("definition", id.as_str().into()),
        ResourceLiveRecordKeyV1::Unit(id) => ("unit", id.as_str().into()),
        // A holder's canonical encoding, which names it exactly.
        ResourceLiveRecordKeyV1::Custodian(holder) => (
            "custodian",
            serde_json::to_string(holder).unwrap_or_default().into(),
        ),
    };
    text_bucket(b"canwu.resource.live-bucket.v1", kind, &text, buckets)
}

/// The two bucket spaces: what parsing a stored bucket of either needs.
trait BucketKey: Ord + Clone + std::fmt::Debug + DeserializeOwned {
    /// The decoded entry a bucket of this space holds.
    type Entry: Clone + std::fmt::Debug;
    const FORMAT_VERSION: u32;
    /// Whether the bucket's identity-only declaration may name evidence.
    const DECLARES_EVIDENCE: bool;
    fn reference(bucket: u32) -> DomainRecordRef;
    fn bucket_of(&self, buckets: u32) -> u32;
}

impl BucketKey for ResourceTerminalRecordKeyV1 {
    type Entry = ResourceHotTerminalEntryV1;
    const FORMAT_VERSION: u32 = RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION;
    const DECLARES_EVIDENCE: bool = true;

    fn reference(bucket: u32) -> DomainRecordRef {
        resource_hot_terminal_bucket_reference(bucket).into_untyped()
    }

    fn bucket_of(&self, buckets: u32) -> u32 {
        resource_hot_terminal_bucket(self, buckets)
    }
}

impl BucketKey for ResourceLiveRecordKeyV1 {
    type Entry = ResourceLiveEntryV1;
    const FORMAT_VERSION: u32 = RESOURCE_LIVE_BUCKET_FORMAT_VERSION;
    const DECLARES_EVIDENCE: bool = false;

    fn reference(bucket: u32) -> DomainRecordRef {
        resource_live_bucket_reference(bucket).into_untyped()
    }

    fn bucket_of(&self, buckets: u32) -> u32 {
        resource_live_bucket(self, buckets)
    }
}

/// Every record a string could name.
fn keys_named_by(text: &str) -> Vec<RootKey> {
    use ResourceLiveRecordKeyV1 as Live;
    use ResourceTerminalRecordKeyV1 as Key;
    let mut keys = Vec::new();
    if let Ok(id) = crate::ResourceOperationKey::new(text) {
        keys.push(RootKey::Terminal(Key::Outcome(id)));
    }
    if let Ok(id) = crate::ResourceDemandId::new(text) {
        keys.push(RootKey::Terminal(Key::Demand(id)));
    }
    if let Ok(id) = ResourceReservationId::new(text) {
        keys.push(RootKey::Terminal(Key::Reservation(id)));
    }
    if let Ok(id) = crate::ResourceAllocationLegId::new(text) {
        keys.push(RootKey::Terminal(Key::AllocationLeg(id)));
    }
    if let Ok(id) = crate::ResourceConsumptionId::new(text) {
        keys.push(RootKey::Terminal(Key::Consumption(id)));
    }
    if let Ok(id) = crate::ResourceFulfillmentId::new(text) {
        keys.push(RootKey::Terminal(Key::Fulfillment(id)));
    }
    if let Ok(id) = crate::ResourceLossId::new(text) {
        keys.push(RootKey::Terminal(Key::Loss(id)));
    }
    if let Ok(id) = crate::ResourceTransferId::new(text) {
        keys.push(RootKey::Terminal(Key::Transfer(id)));
    }
    if let Ok(id) = CompletionLeaseAcquisitionId::new(text) {
        keys.push(RootKey::Terminal(Key::ExternalCompletionParticipant(
            id.clone(),
        )));
        keys.push(RootKey::Lease(id));
    }
    if let Ok(id) = CompletionCapacityGrantId::new(text) {
        keys.push(RootKey::LeaseGrant(id));
    }
    if let Ok(id) = ResourceAccountId::new(text) {
        keys.push(RootKey::Live(Live::Account(id)));
    }
    if let Ok(id) = ResourceAccessGrantId::new(text) {
        keys.push(RootKey::Live(Live::AccessGrant(id)));
    }
    if let Ok(id) = ResourceDefinitionRevisionId::new(text) {
        keys.push(RootKey::Live(Live::Definition(id)));
    }
    if let Ok(id) = ResourceUnitRevisionId::new(text) {
        keys.push(RootKey::Live(Live::Unit(id)));
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

    /// Whether every account, access grant, definition, and unit revision of
    /// this root is attached to it.
    #[must_use]
    pub fn live_records_attached(&self) -> bool {
        self.live.is_empty()
    }

    /// Accounts of this root, attached or not.
    #[must_use]
    pub fn account_count(&self) -> usize {
        self.accounts
            .len()
            .saturating_add(usize::try_from(self.live.accounts).unwrap_or(usize::MAX))
    }

    /// Access grants of this root, attached or not.
    #[must_use]
    pub fn access_grant_count(&self) -> usize {
        self.access_grants
            .len()
            .saturating_add(usize::try_from(self.live.access_grants).unwrap_or(usize::MAX))
    }

    /// Resource definition revisions of this root, attached or not.
    #[must_use]
    pub fn definition_count(&self) -> usize {
        self.definitions
            .len()
            .saturating_add(usize::try_from(self.live.definitions).unwrap_or(usize::MAX))
    }

    /// Unit revisions of this root, attached or not.
    #[must_use]
    pub fn unit_count(&self) -> usize {
        self.units
            .len()
            .saturating_add(usize::try_from(self.live.units).unwrap_or(usize::MAX))
    }

    /// The balances of every account of this root, attached or not.
    pub fn total_account_balance(&self) -> Result<u128, ResourceError> {
        self.accounts
            .values()
            .try_fold(self.live.balance, |total, account| {
                total
                    .checked_add(u128::from(account.balance))
                    .ok_or(ResourceError::Overflow)
            })
    }

    /// Recomputes the custodian index ([`Self::custodian_accounts`],
    /// [`ResourceCustodianAccountsV1`]) from the accounts of a whole state.
    ///
    /// Required after inserting accounts into [`Self::accounts`] directly:
    /// validation, and so every operation applied in memory, refuses an
    /// account missing from its custodian's index. Every resource operation
    /// and [`Self::install_opening_account`] keep the index current, and
    /// [`Self::into_records`] derives it, so only fixtures that edit the map
    /// themselves call this.
    pub fn refresh_custodian_index(&mut self) {
        self.custodian_accounts = BTreeMap::new();
        for account in self.accounts.values() {
            self.custodian_accounts
                .entry(account.custodian.clone())
                .or_default()
                .insert(account.id.clone());
        }
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

    /// Whether the state holds the live record `key`.
    fn holds_live(&self, key: &ResourceLiveRecordKeyV1) -> bool {
        match key {
            ResourceLiveRecordKeyV1::Account(id) => self.accounts.contains_key(id),
            ResourceLiveRecordKeyV1::AccessGrant(id) => self.access_grants.contains_key(id),
            ResourceLiveRecordKeyV1::Definition(id) => self.definitions.contains_key(id),
            ResourceLiveRecordKeyV1::Unit(id) => self.units.contains_key(id),
            ResourceLiveRecordKeyV1::Custodian(holder) => {
                self.custodian_accounts.contains_key(holder)
            }
        }
    }

    /// The live entry of the attached live record `key`.
    fn live_entry(
        &self,
        key: &ResourceLiveRecordKeyV1,
    ) -> Result<ResourceLiveEntryV1, ResourceError> {
        let missing = || ResourceError::NotFound("resource live record is not attached".to_owned());
        let payload = match key {
            ResourceLiveRecordKeyV1::Account(id) => {
                ResourceLivePayloadV1::Account(self.accounts.get(id).cloned().ok_or_else(missing)?)
            }
            ResourceLiveRecordKeyV1::AccessGrant(id) => ResourceLivePayloadV1::AccessGrant(
                self.access_grants.get(id).cloned().ok_or_else(missing)?,
            ),
            ResourceLiveRecordKeyV1::Definition(id) => ResourceLivePayloadV1::Definition(
                self.definitions.get(id).cloned().ok_or_else(missing)?,
            ),
            ResourceLiveRecordKeyV1::Unit(id) => {
                ResourceLivePayloadV1::Unit(self.units.get(id).cloned().ok_or_else(missing)?)
            }
            ResourceLiveRecordKeyV1::Custodian(holder) => {
                ResourceLivePayloadV1::Custodian(ResourceCustodianAccountsV1 {
                    holder: holder.clone(),
                    accounts: self
                        .custodian_accounts
                        .get(holder)
                        .cloned()
                        .ok_or_else(missing)?,
                })
            }
        };
        Ok(ResourceLiveEntryV1 {
            key: key.clone(),
            payload,
        })
    }

    /// Attaches one stored live record. The caller takes it out of the
    /// ledger.
    fn attach_live(&mut self, entry: ResourceLiveEntryV1) -> Result<(), ResourceError> {
        if !entry.names_its_payload() {
            return Err(ResourceError::InvalidDefinition(
                "resource live entry does not hold the record its key names".to_owned(),
            ));
        }
        let replaced = match entry.payload {
            ResourceLivePayloadV1::Account(value) => {
                self.accounts.insert(value.id.clone(), value).is_some()
            }
            ResourceLivePayloadV1::AccessGrant(value) => self
                .access_grants
                .insert(value.grant.grant_id.clone(), value)
                .is_some(),
            ResourceLivePayloadV1::Definition(value) => {
                self.definitions.insert(value.id.clone(), value).is_some()
            }
            ResourceLivePayloadV1::Unit(value) => {
                self.units.insert(value.id.clone(), value).is_some()
            }
            ResourceLivePayloadV1::Custodian(value) => {
                if value.accounts.is_empty() {
                    return Err(ResourceError::InvalidDefinition(
                        "resource custodian index lists no account".to_owned(),
                    ));
                }
                self.custodian_accounts
                    .insert(value.holder, value.accounts)
                    .is_some()
            }
        };
        if replaced {
            return Err(ResourceError::InvalidDefinition(
                "resource live record is already in the root".to_owned(),
            ));
        }
        Ok(())
    }

    /// Splits this state into its live root and the entries of its attached
    /// bucketed records. The live root's ledgers then count the encoded bytes
    /// of every hot terminal entry and every live record, attached or not.
    pub(crate) fn split_parts(
        mut self,
    ) -> Result<
        (
            Self,
            Vec<ResourceHotTerminalEntryV1>,
            Vec<ResourceLiveEntryV1>,
        ),
        ResourceError,
    > {
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
        let mut live = Vec::with_capacity(
            self.accounts.len()
                + self.access_grants.len()
                + self.definitions.len()
                + self.units.len(),
        );
        live.extend(
            std::mem::take(&mut self.accounts)
                .into_iter()
                .map(|(id, value)| ResourceLiveEntryV1 {
                    key: ResourceLiveRecordKeyV1::Account(id),
                    payload: ResourceLivePayloadV1::Account(value),
                }),
        );
        live.extend(
            std::mem::take(&mut self.access_grants)
                .into_iter()
                .map(|(id, value)| ResourceLiveEntryV1 {
                    key: ResourceLiveRecordKeyV1::AccessGrant(id),
                    payload: ResourceLivePayloadV1::AccessGrant(value),
                }),
        );
        live.extend(
            std::mem::take(&mut self.definitions)
                .into_iter()
                .map(|(id, value)| ResourceLiveEntryV1 {
                    key: ResourceLiveRecordKeyV1::Definition(id),
                    payload: ResourceLivePayloadV1::Definition(value),
                }),
        );
        live.extend(
            std::mem::take(&mut self.units)
                .into_iter()
                .map(|(id, value)| ResourceLiveEntryV1 {
                    key: ResourceLiveRecordKeyV1::Unit(id),
                    payload: ResourceLivePayloadV1::Unit(value),
                }),
        );
        live.extend(
            std::mem::take(&mut self.custodian_accounts)
                .into_iter()
                .map(|(holder, accounts)| ResourceLiveEntryV1 {
                    key: ResourceLiveRecordKeyV1::Custodian(holder.clone()),
                    payload: ResourceLivePayloadV1::Custodian(ResourceCustodianAccountsV1 {
                        holder,
                        accounts,
                    }),
                }),
        );
        for entry in &live {
            let size = encoded_len(entry)?;
            self.live.count(&entry.payload, size, true)?;
        }
        Ok((self, entries, live))
    }

    /// Bytes the root occupies persisted: its live root encoded, every hot
    /// terminal entry, and every live entry. This is what `max_state_bytes`
    /// bounds.
    pub(crate) fn encoded_root_bytes(&self) -> Result<usize, ResourceError> {
        let (live, _, _) = self.clone().split_parts()?;
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
            .and_then(|bytes| {
                usize::try_from(live.live.bytes)
                    .ok()
                    .and_then(|live| bytes.checked_add(live))
            })
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
    /// then one record per non-empty hot terminal bucket and one per
    /// non-empty live bucket, each in bucket order. This is how a scenario
    /// installs a resource root.
    pub fn into_records(mut self) -> Result<Vec<DomainRecord>, CanwuError> {
        if !self.hot_terminal_records_attached() || !self.live_records_attached() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "a resource root persists whole: some bucketed records are not attached",
            ));
        }
        self.refresh_continuation();
        // The custodian index of a whole state is a function of its
        // accounts, so a fixture that placed accounts in the map directly
        // installs a consistent one.
        self.refresh_custodian_index();
        self.validate().map_err(resource_canwu_error)?;
        let version = self.state_revision.get();
        let buckets = resource_hot_terminal_bucket_count(&self.limits);
        let live_buckets = resource_live_bucket_count(&self.limits);
        let (live, entries, live_entries) = self.split_parts().map_err(resource_canwu_error)?;
        let mut bucketed = BTreeMap::<u32, Vec<ResourceHotTerminalEntryV1>>::new();
        for entry in entries {
            bucketed
                .entry(resource_hot_terminal_bucket(&entry.key, buckets))
                .or_default()
                .push(entry);
        }
        let mut live_bucketed = BTreeMap::<u32, Vec<ResourceLiveEntryV1>>::new();
        for entry in live_entries {
            live_bucketed
                .entry(resource_live_bucket(&entry.key, live_buckets))
                .or_default()
                .push(entry);
        }
        let mut records = vec![live_root_record(live, version)?];
        for (bucket, mut entries) in bucketed {
            entries.sort_by(|left, right| left.key.cmp(&right.key));
            let evidence = hot_bucket_declaration(&entries);
            records.push(bucket_record(
                resource_hot_terminal_bucket_reference(bucket).into_untyped(),
                encode_value(&ResourceHotTerminalBucketV1 {
                    format_version: RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION,
                    bucket,
                    entries,
                    evidence,
                })?,
            )?);
        }
        for (bucket, mut entries) in live_bucketed {
            entries.sort_by(|left, right| left.key.cmp(&right.key));
            records.push(bucket_record(
                resource_live_bucket_reference(bucket).into_untyped(),
                encode_value(&ResourceLiveBucketV1 {
                    format_version: RESOURCE_LIVE_BUCKET_FORMAT_VERSION,
                    bucket,
                    entries,
                    evidence: empty_declaration(),
                })?,
            )?);
        }
        Ok(records)
    }

    /// Restores a whole state from its live root and the records of its
    /// buckets, hot terminal and live, in any order (empty buckets may be
    /// included), checking that each entry lies in its bucket, that the
    /// ledgers count exactly the entries, and that the state is valid.
    pub fn from_records<'a>(
        live_root: &DomainRecord,
        buckets: impl IntoIterator<Item = &'a DomainRecord>,
    ) -> Result<Self, CanwuError> {
        let mut state = decode_live_root(live_root)?;
        let count = resource_hot_terminal_bucket_count(&state.limits);
        let live_count = resource_live_bucket_count(&state.limits);
        let mut seen = BTreeSet::new();
        let mut bytes = 0_u64;
        let mut live_ledger = ResourceLiveLedgerV1::default();
        for record in buckets {
            if record.reference.id.starts_with(LIVE_BUCKET_PREFIX) {
                let (bucket, entries, _) =
                    stored_bucket_entries::<ResourceLiveRecordKeyV1>(record, live_count)?;
                if !seen.insert((true, bucket)) {
                    return Err(invalid("resource live bucket is listed twice"));
                }
                for value in entries.into_values() {
                    let entry = decode_canonical::<ResourceLiveEntryV1>(value)?;
                    let size = encoded_len(&entry).map_err(resource_canwu_error)?;
                    live_ledger
                        .count(&entry.payload, size, true)
                        .map_err(resource_canwu_error)?;
                    state.attach_live(entry).map_err(resource_canwu_error)?;
                }
                continue;
            }
            let (bucket, entries, declared) =
                stored_bucket_entries::<ResourceTerminalRecordKeyV1>(record, count)?;
            if !seen.insert((false, bucket)) {
                return Err(invalid("resource hot terminal bucket is listed twice"));
            }
            let mut decoded = Vec::with_capacity(entries.len());
            for value in entries.into_values() {
                let entry = decode_canonical::<ResourceHotTerminalEntryV1>(value)?;
                bytes = bytes
                    .checked_add(entry.encoded_len().map_err(resource_canwu_error)?)
                    .ok_or_else(|| invalid("resource hot terminal bytes overflow"))?;
                decoded.push(entry);
            }
            check_hot_bucket_declaration(&declared, &decoded)?;
            for entry in decoded {
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
        if live_ledger != state.live {
            return Err(invalid(
                "resource live ledger differs from the stored live entries",
            ));
        }
        state.hot_terminal.bytes = 0;
        state.live = ResourceLiveLedgerV1::default();
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
/// without the bodies of its hot terminal records and of its accounts, access
/// grants, definitions, and unit revisions, which its ledgers count and its
/// buckets hold.
///
/// It encodes as that state does. Read a root through a
/// [`ResourceRootReader`] ([`ResourceRootReader::load_run`] in a host
/// application, [`ResourceRootReader::load`] in a boundary system or
/// admission), [`crate::resource_state`], or [`resource_root_in_view`], which
/// attach the bucketed records; the live root alone answers no question
/// about them.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ResourceLiveRootV1(ResourceState);

impl ResourceLiveRootV1 {
    /// The live part of the root. Its maps lack every bucketed record.
    #[must_use]
    pub fn live(&self) -> &ResourceState {
        &self.0
    }

    /// The live part of the root. Its maps lack every bucketed record.
    #[must_use]
    pub fn into_live(self) -> ResourceState {
        self.0
    }
}

pub(crate) fn decode_live_root(record: &DomainRecord) -> Result<ResourceState, CanwuError> {
    let state = record
        .decode_payload::<crate::ResourceRuntimeRecord>()?
        .into_live();
    if !state.terminal_archive_candidates.is_empty()
        || !state.accounts.is_empty()
        || !state.access_grants.is_empty()
        || !state.definitions.is_empty()
        || !state.units.is_empty()
        || !state.custodian_accounts.is_empty()
    {
        return Err(invalid(
            "a resource live root holds no bucketed records itself",
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

/// The draft of the live root `live`, which has no attached bucketed record.
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

fn encode_value<T: Serialize>(value: &T) -> Result<Value, CanwuError> {
    serde_json::to_value(value).map_err(|error| invalid(error.to_string()))
}

fn bucket_record(reference: DomainRecordRef, payload: Value) -> Result<DomainRecord, CanwuError> {
    let draft = DomainRecordDraft::new(reference, payload);
    Ok(DomainRecord {
        reference: draft.reference,
        owner: crate::PLUGIN_NAME.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: draft.references,
    })
}

/// Where buckets are read: the simulation view of a boundary system or
/// admission, or the host application's runtime. A source is a handle whose
/// records live for `'s`, so the work that reads it may own it.
pub(crate) trait HotTerminalSource<'s> {
    fn hot_terminal_record(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<&'s DomainRecord>, CanwuError>;

    /// The references of every stored bucket, hot terminal and live, so a
    /// whole read visits the buckets that exist rather than every index.
    fn bucket_references(&self) -> Result<Vec<DomainRecordRef>, CanwuError>;

    /// The run's own shared copy of a record ([`CanwuRun::domain_record_shared`]),
    /// which a kept root holds to tell an unchanged record from a changed one,
    /// or `None` from a source that keeps no shared copies (a view).
    fn shared_record(&self, _reference: &DomainRecordRef) -> Option<Arc<DomainRecord>> {
        None
    }
}

/// Records one page of a bucket listing reads.
const BUCKET_LISTING_PAGE: usize = 4_096;

impl<'s> HotTerminalSource<'s> for &'s SimulationView<'_> {
    fn hot_terminal_record(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<&'s DomainRecord>, CanwuError> {
        let view: &'s SimulationView<'_> = self;
        view.domain_record(reference)
    }

    fn bucket_references(&self) -> Result<Vec<DomainRecordRef>, CanwuError> {
        let kind = canwu_api::DomainRecordKind::for_type::<ResourceBucketRecord>();
        let mut references = Vec::new();
        loop {
            let page =
                self.domain_records_of_kind_after(&kind, references.last(), BUCKET_LISTING_PAGE)?;
            let full = page.len() == BUCKET_LISTING_PAGE;
            references.extend(page.into_iter().map(|record| record.reference));
            if !full {
                return Ok(references);
            }
        }
    }
}

/// A runtime handle, [`canwu_api::Canwu`] or [`canwu_api::CompactedCanwu`],
/// read as the source of a resource root's buckets.
struct RunSource<'r, H: ?Sized>(&'r H);

impl<'r, H: CanwuRun + ?Sized> HotTerminalSource<'r> for RunSource<'r, H> {
    fn hot_terminal_record(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<&'r DomainRecord>, CanwuError> {
        let run: &'r H = self.0;
        Ok(run.domain_record(reference))
    }

    fn bucket_references(&self) -> Result<Vec<DomainRecordRef>, CanwuError> {
        let kind = canwu_api::DomainRecordKind::for_type::<ResourceBucketRecord>();
        Ok(self
            .0
            .domain_records_of_kind(&kind)
            .map(|record| record.reference.clone())
            .collect())
    }

    fn shared_record(&self, reference: &DomainRecordRef) -> Option<Arc<DomainRecord>> {
        run_shared_record(self.0, reference)
    }
}

/// The run's shared copy of a record, only when it is the very allocation
/// [`CanwuRun::domain_record`] reads: a run that answers with another copy
/// (a fresh one per call, or one cached by version) offers nothing to reuse,
/// so a kept root never vouches for content it did not read.
fn run_shared_record<H: CanwuRun + ?Sized>(
    run: &H,
    reference: &DomainRecordRef,
) -> Option<Arc<DomainRecord>> {
    let read = run.domain_record(reference)?;
    run.domain_record_shared(reference)
        .filter(|shared| std::ptr::eq(Arc::as_ptr(shared), read))
}

/// What a [`ResourceRootReader`] reads: a boundary's view, or a run handle
/// (behind a box, so a reader over any handle has one type).
enum ReaderSource<'v, 'a> {
    View(&'v SimulationView<'a>),
    Run(Box<dyn HotTerminalSource<'v> + 'v>),
}

impl<'v> HotTerminalSource<'v> for ReaderSource<'v, '_> {
    fn hot_terminal_record(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<&'v DomainRecord>, CanwuError> {
        match self {
            Self::View(view) => view.hot_terminal_record(reference),
            Self::Run(run) => run.hot_terminal_record(reference),
        }
    }

    fn bucket_references(&self) -> Result<Vec<DomainRecordRef>, CanwuError> {
        match self {
            Self::View(view) => view.bucket_references(),
            Self::Run(run) => run.bucket_references(),
        }
    }

    fn shared_record(&self, reference: &DomainRecordRef) -> Option<Arc<DomainRecord>> {
        match self {
            Self::View(_) => None,
            Self::Run(run) => run.shared_record(reference),
        }
    }
}

/// One bucket as it was stored when the work read it: its version, and each
/// entry's encoded value by key. An entry is decoded only when attached.
struct StoredBucket<'s, K: BucketKey> {
    version: Option<u64>,
    entries: BTreeMap<K, &'s Value>,
    /// The bucket's identity-only declaration as it was stored.
    declared: IdentityEvidenceDependenciesV1,
    /// The run's own shared copy of the bucket record, when the source keeps
    /// one: a kept root holds it to know the bucket again.
    shared: Option<Arc<DomainRecord>>,
    /// Entries of this very record decoded by an earlier reader, with their
    /// encoded sizes, taken instead of decoding them again.
    decoded: BTreeMap<K, (K::Entry, u64)>,
}

impl<K: BucketKey> StoredBucket<'_, K> {
    /// A bucket that is not stored.
    fn absent() -> Self {
        Self {
            version: None,
            entries: BTreeMap::new(),
            declared: empty_declaration(),
            shared: None,
            decoded: BTreeMap::new(),
        }
    }

    /// What a kept root keeps of this bucket: the record, the parsed keys,
    /// the declaration, and the kept decodes of entries not attached (the
    /// attached ones are kept with the state).
    fn keep(self) -> KeptBucket<K> {
        let record = match (self.version, self.shared) {
            (None, _) => KeptRecord::Absent,
            (Some(_), Some(shared)) => KeptRecord::Shared(shared),
            (Some(_), None) => KeptRecord::Unshared,
        };
        KeptBucket {
            record,
            version: self.version,
            keys: self.entries.into_keys().collect(),
            declared: self.declared,
            decoded: self.decoded,
        }
    }
}

/// A bucket record as a kept root saw it.
#[derive(Clone, Debug)]
enum KeptRecord {
    /// No such bucket was stored.
    Absent,
    /// The run's own copy of the stored record.
    Shared(Arc<DomainRecord>),
    /// A stored record whose source kept no shared copy: never reused.
    Unshared,
}

/// One bucket a reader read, kept so that a later reader of the same record
/// neither parses nor decodes it again.
#[derive(Clone, Debug)]
struct KeptBucket<K: BucketKey> {
    record: KeptRecord,
    version: Option<u64>,
    /// The record's keys, in the order of its entries.
    keys: Vec<K>,
    declared: IdentityEvidenceDependenciesV1,
    decoded: BTreeMap<K, (K::Entry, u64)>,
}

impl<K: BucketKey> KeptBucket<K> {
    /// Whether `source` holds the very record this bucket was read from, or
    /// still no record where none was stored.
    fn still_held<'s, S: HotTerminalSource<'s> + ?Sized>(&self, source: &S, bucket: u32) -> bool {
        match (&self.record, source.shared_record(&K::reference(bucket))) {
            (KeptRecord::Absent, None) => true,
            (KeptRecord::Shared(kept), Some(now)) => Arc::ptr_eq(kept, &now),
            _ => false,
        }
    }

    /// The stored bucket again, from the record `source` holds, which
    /// [`Self::still_held`] found to be the one it was read from.
    fn restore<'s, S: HotTerminalSource<'s> + ?Sized>(
        self,
        source: &S,
        bucket: u32,
    ) -> Result<StoredBucket<'s, K>, CanwuError> {
        let KeptRecord::Shared(shared) = self.record else {
            return Ok(StoredBucket::absent());
        };
        let record = source
            .hot_terminal_record(&K::reference(bucket))?
            .ok_or_else(|| invalid("a kept resource bucket is no longer stored"))?;
        let values = record
            .payload
            .get("entries")
            .and_then(Value::as_array)
            .filter(|values| values.len() == self.keys.len())
            .ok_or_else(|| invalid("a kept resource bucket differs from its stored record"))?;
        Ok(StoredBucket {
            version: self.version,
            entries: self.keys.into_iter().zip(values).collect(),
            declared: self.declared,
            shared: Some(shared),
            decoded: self.decoded,
        })
    }
}

/// Reads bucket `bucket` of a space of `count` buckets: from `inherited`
/// when a kept root read this very record, otherwise from `source`, counting
/// a stored bucket it parses in `parsed`.
fn read_bucket<'s, K: BucketKey, S: HotTerminalSource<'s> + ?Sized>(
    source: &S,
    bucket: u32,
    count: u32,
    inherited: &mut BTreeMap<u32, KeptBucket<K>>,
    parsed: &mut usize,
) -> Result<StoredBucket<'s, K>, CanwuError> {
    if let Some(kept) = inherited.remove(&bucket) {
        return kept.restore(source, bucket);
    }
    let reference = K::reference(bucket);
    let Some(record) = source.hot_terminal_record(&reference)? else {
        return Ok(StoredBucket::absent());
    };
    let (_, entries, declared) = stored_bucket_entries(record, count)?;
    *parsed += 1;
    Ok(StoredBucket {
        version: Some(record.version),
        entries,
        declared,
        shared: source.shared_record(&reference),
        decoded: BTreeMap::new(),
    })
}

/// A hot terminal record attached by the work, as it was stored, with its
/// encoded size.
#[derive(Clone, Debug)]
struct AttachedEntry {
    bucket: u32,
    entry: ResourceHotTerminalEntryV1,
    size: u64,
}

/// A live record attached by the work, as it was stored, with its encoded
/// size.
#[derive(Clone, Debug)]
struct AttachedLive {
    bucket: u32,
    entry: ResourceLiveEntryV1,
    size: u64,
}

/// The keys of one stored bucket of a space with `count` buckets, with each
/// entry's encoded value and the bucket's identity-only declaration,
/// checking that it is the bucket its identity names, that its keys ascend,
/// that each lies in this bucket, and that a live bucket declares nothing.
/// Entry bodies are checked when they are attached, and a hot terminal
/// bucket's declaration against them when the whole root is read.
fn stored_bucket_entries<K: BucketKey>(
    record: &DomainRecord,
    count: u32,
) -> Result<(u32, BTreeMap<K, &Value>, IdentityEvidenceDependenciesV1), CanwuError> {
    let malformed = || invalid("resource bucket is malformed or holds a record of another bucket");
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
    let declared = fields
        .get(IDENTITY_EVIDENCE_DEPENDENCIES_FIELD)
        .map(IdentityEvidenceDependenciesV1::deserialize)
        .transpose()
        .map_err(|_| malformed())?
        .ok_or_else(malformed)?;
    if fields.len() != 4
        || declared.format_version != canwu_api::IDENTITY_EVIDENCE_DEPENDENCIES_FORMAT_VERSION
        || (!K::DECLARES_EVIDENCE && !declared.dependencies.is_empty())
        || fields.get("format_version").and_then(Value::as_u64)
            != Some(u64::from(K::FORMAT_VERSION))
        || bucket >= count
        || !record.reference.kind.matches_type::<ResourceBucketRecord>()
        || record.reference != K::reference(bucket)
        || record.owner != crate::PLUGIN_NAME
        || record.lifecycle != DomainRecordLifecycle::Active
    {
        return Err(malformed());
    }
    let mut keyed = BTreeMap::new();
    for entry in entries {
        let key = entry
            .get("key")
            .map(K::deserialize)
            .transpose()
            .map_err(|_| malformed())?
            .ok_or_else(malformed)?;
        if key.bucket_of(count) != bucket
            || keyed
                .last_key_value()
                .is_some_and(|(last, _): (&K, _)| last >= &key)
        {
            return Err(malformed());
        }
        keyed.insert(key, entry);
    }
    Ok((bucket, keyed, declared))
}

/// Refuses a hot terminal bucket whose stored declaration is not the one its
/// entries make.
fn check_hot_bucket_declaration<'a>(
    declared: &IdentityEvidenceDependenciesV1,
    entries: impl IntoIterator<Item = &'a ResourceHotTerminalEntryV1>,
) -> Result<(), CanwuError> {
    if *declared != hot_bucket_declaration(entries) {
        return Err(invalid(
            "a resource hot terminal bucket declares other evidence than its entries cite",
        ));
    }
    Ok(())
}

/// Decodes one stored entry, which must be in its canonical encoding: what
/// the entry it decodes to encodes as.
fn decode_canonical<T: DeserializeOwned + Serialize>(value: &Value) -> Result<T, CanwuError> {
    let entry = T::deserialize(value)
        .map_err(|error| invalid(format!("resource bucket entry cannot be decoded: {error}")))?;
    if serde_json::to_value(&entry).ok().as_ref() != Some(value) {
        return Err(invalid(
            "resource bucket entry is not in its canonical encoding",
        ));
    }
    Ok(entry)
}

/// The facts the ledger counts of a stored hot terminal entry, read from its
/// encoded value without decoding it.
fn stored_hot_facts(
    key: &ResourceTerminalRecordKeyV1,
    value: &Value,
) -> Result<
    (
        Option<KnowledgeHolderRef>,
        Option<ProtectedFloorPolicyRevisionId>,
    ),
    CanwuError,
> {
    if !matches!(key, ResourceTerminalRecordKeyV1::Demand(_)) {
        return Ok((None, None));
    }
    let demand = value
        .get("payload")
        .and_then(|payload| payload.get("value"))
        .ok_or_else(|| invalid("resource hot terminal demand entry is malformed"))?;
    let requester = demand
        .get("requester")
        .map(KnowledgeHolderRef::deserialize)
        .transpose()
        .map_err(|error| invalid(error.to_string()))?
        .ok_or_else(|| invalid("resource hot terminal demand entry names no requester"))?;
    let policy = match demand.get("protected_floor_policy") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            ProtectedFloorPolicyRevisionId::deserialize(value)
                .map_err(|error| invalid(error.to_string()))?,
        ),
    };
    Ok((Some(requester), policy))
}

/// The indexes of the buckets `source` stores: hot terminal, then live.
fn stored_bucket_indexes<'s, S: HotTerminalSource<'s> + ?Sized>(
    source: &S,
) -> Result<(Vec<u32>, Vec<u32>), CanwuError> {
    let mut hot = Vec::new();
    let mut live = Vec::new();
    for reference in source.bucket_references()? {
        let (space, index) =
            if let Some(index) = reference.id.strip_prefix(HOT_TERMINAL_BUCKET_PREFIX) {
                (&mut hot, index)
            } else if let Some(index) = reference.id.strip_prefix(LIVE_BUCKET_PREFIX) {
                (&mut live, index)
            } else {
                return Err(invalid("a resource bucket record has an unknown identity"));
            };
        space.push(
            index
                .parse()
                .map_err(|_| invalid("a resource bucket record has an unknown identity"))?,
        );
    }
    Ok((hot, live))
}

/// What a work read, kept without its source ([`ResourceRootWork::keep`]).
struct KeptWork {
    state: ResourceState,
    attached: BTreeMap<ResourceTerminalRecordKeyV1, AttachedEntry>,
    live_attached: BTreeMap<ResourceLiveRecordKeyV1, AttachedLive>,
    hot: BTreeMap<u32, KeptBucket<ResourceTerminalRecordKeyV1>>,
    live: BTreeMap<u32, KeptBucket<ResourceLiveRecordKeyV1>>,
}

/// The resource root as one boundary system, admission, or host read works
/// on it: the live root, with the bucketed records it reads attached.
pub(crate) struct ResourceRootWork<'s, S: HotTerminalSource<'s>> {
    source: S,
    pub(crate) live_version: u64,
    pub(crate) state: ResourceState,
    bucket_count: u32,
    buckets: BTreeMap<u32, StoredBucket<'s, ResourceTerminalRecordKeyV1>>,
    attached: BTreeMap<ResourceTerminalRecordKeyV1, AttachedEntry>,
    live_bucket_count: u32,
    live_buckets: BTreeMap<u32, StoredBucket<'s, ResourceLiveRecordKeyV1>>,
    live_attached: BTreeMap<ResourceLiveRecordKeyV1, AttachedLive>,
    /// Hot terminal records an archive commit removed without attaching
    /// them, by bucket.
    archived: BTreeMap<ResourceTerminalRecordKeyV1, u32>,
    /// Hot terminal buckets a kept root read whose records the source still
    /// holds, not read again yet.
    inherited: BTreeMap<u32, KeptBucket<ResourceTerminalRecordKeyV1>>,
    /// Live buckets a kept root read whose records the source still holds.
    live_inherited: BTreeMap<u32, KeptBucket<ResourceLiveRecordKeyV1>>,
    /// Stored buckets this work parsed from the source.
    parsed: usize,
}

impl<'s, S: HotTerminalSource<'s>> ResourceRootWork<'s, S> {
    /// The live root `live` read from `source`, with nothing attached.
    pub(crate) fn new(source: S, live: &DomainRecord) -> Result<Self, CanwuError> {
        let state = decode_live_root(live)?;
        Ok(Self::resume(
            source,
            live.version,
            state,
            BTreeMap::new(),
            BTreeMap::new(),
        ))
    }

    /// The work over `state`, the live root of version `live_version` with
    /// the `attached` and `live_attached` records attached.
    fn resume(
        source: S,
        live_version: u64,
        state: ResourceState,
        attached: BTreeMap<ResourceTerminalRecordKeyV1, AttachedEntry>,
        live_attached: BTreeMap<ResourceLiveRecordKeyV1, AttachedLive>,
    ) -> Self {
        Self {
            source,
            live_version,
            bucket_count: resource_hot_terminal_bucket_count(&state.limits),
            live_bucket_count: resource_live_bucket_count(&state.limits),
            state,
            buckets: BTreeMap::new(),
            attached,
            live_buckets: BTreeMap::new(),
            live_attached,
            archived: BTreeMap::new(),
            inherited: BTreeMap::new(),
            live_inherited: BTreeMap::new(),
            parsed: 0,
        }
    }

    fn bucket(
        &mut self,
        bucket: u32,
    ) -> Result<&StoredBucket<'s, ResourceTerminalRecordKeyV1>, CanwuError> {
        if !self.buckets.contains_key(&bucket) {
            let stored = read_bucket(
                &self.source,
                bucket,
                self.bucket_count,
                &mut self.inherited,
                &mut self.parsed,
            )?;
            self.buckets.insert(bucket, stored);
        }
        Ok(&self.buckets[&bucket])
    }

    fn live_bucket(
        &mut self,
        bucket: u32,
    ) -> Result<&StoredBucket<'s, ResourceLiveRecordKeyV1>, CanwuError> {
        if !self.live_buckets.contains_key(&bucket) {
            let stored = read_bucket(
                &self.source,
                bucket,
                self.live_bucket_count,
                &mut self.live_inherited,
                &mut self.parsed,
            )?;
            self.live_buckets.insert(bucket, stored);
        }
        Ok(&self.live_buckets[&bucket])
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
    fn held_references(&self, key: &ResourceTerminalRecordKeyV1) -> Vec<RootKey> {
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
            // attached its closure already. Its source accounts, grant,
            // definition, and unit are live records.
            Key::Demand(id) => state.demands.get(id).cloned().map(Payload::Demand),
            Key::Outcome(_)
            | Key::ExternalCompletionParticipant(_)
            | Key::LeaseReceipt(_)
            | Key::ArchiveMaintenanceReceipt(_) => None,
        };
        payload.as_ref().map(payload_references).unwrap_or_default()
    }

    /// The exact targets the lease grants of `acquisition` lock, local or
    /// external.
    fn lease_references(&self, acquisition: &CompletionLeaseAcquisitionId) -> Vec<RootKey> {
        let leases = &self.state.completion_leases;
        let local = leases
            .acquisitions
            .get(acquisition)
            .into_iter()
            .flat_map(|acquisition| acquisition.grants.values())
            .filter_map(|grant| leases.grants.get(grant))
            .flat_map(|grant| grant.target_versions.iter());
        let external = self
            .state
            .external_completion_participants
            .participant(acquisition)
            .into_iter()
            .flat_map(|participant| participant.grant.target_versions.iter());
        local.chain(external).filter_map(target_keys).collect()
    }

    /// Attaches the bucketed records `keys` name, and every record those name
    /// in turn: the closure of a demand, what a reservation, allocation leg,
    /// consumption, fulfillment, transfer, or loss belongs to, the accounts,
    /// grants, definitions, and units those name, and the targets of a named
    /// completion lease, followed through live-root records as well. A key
    /// that names no record is ignored. Without `follow_live` only hot
    /// terminal records are attached.
    pub(crate) fn attach_keys(
        &mut self,
        keys: impl IntoIterator<Item = RootKey>,
        follow_live: bool,
    ) -> Result<(), CanwuError> {
        let mut pending: Vec<_> = keys.into_iter().collect();
        let mut visited = BTreeSet::new();
        while let Some(key) = pending.pop() {
            if !visited.insert(key.clone()) {
                continue;
            }
            match key {
                RootKey::Terminal(key) => {
                    let references = self.attach_terminal(&key)?;
                    pending.extend(references);
                }
                RootKey::Live(key) => {
                    if follow_live {
                        let references = self.attach_live_key(&key)?;
                        pending.extend(references);
                    }
                }
                RootKey::Lease(acquisition) => {
                    if follow_live {
                        pending.extend(self.lease_references(&acquisition));
                    }
                }
                RootKey::LeaseGrant(grant) => {
                    if follow_live {
                        pending.extend(
                            self.state
                                .completion_leases
                                .grants
                                .get(&grant)
                                .into_iter()
                                .flat_map(|grant| grant.target_versions.iter())
                                .filter_map(target_keys)
                                .collect::<Vec<_>>(),
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Attaches one hot terminal record, and returns what it names.
    fn attach_terminal(
        &mut self,
        key: &ResourceTerminalRecordKeyV1,
    ) -> Result<Vec<RootKey>, CanwuError> {
        if self.holds(key) {
            return Ok(self.held_references(key));
        }
        // Attached before and no longer held, or archived without being
        // attached: this work archived it.
        if self.attached.contains_key(key) || self.archived.contains_key(key) {
            return Ok(Vec::new());
        }
        let bucket = resource_hot_terminal_bucket(key, self.bucket_count);
        let Some(value) = self.bucket(bucket)?.entries.get(key).copied() else {
            return Ok(Vec::new());
        };
        let kept = self
            .buckets
            .get_mut(&bucket)
            .and_then(|stored| stored.decoded.remove(key));
        let (entry, size) = if let Some(kept) = kept {
            kept
        } else {
            let entry = decode_canonical::<ResourceHotTerminalEntryV1>(value)?;
            if &entry.key != key {
                return Err(invalid(
                    "resource hot terminal entry differs from its stored key",
                ));
            }
            let size = entry.encoded_len().map_err(resource_canwu_error)?;
            (entry, size)
        };
        let references = entry.references();
        self.state
            .attach_hot_terminal(entry.clone())
            .map_err(resource_canwu_error)?;
        self.state.hot_terminal.bytes = self
            .state
            .hot_terminal
            .bytes
            .checked_sub(size)
            .ok_or_else(|| invalid("resource hot terminal ledger bytes are short"))?;
        self.attached.insert(
            key.clone(),
            AttachedEntry {
                bucket,
                entry,
                size,
            },
        );
        Ok(references)
    }

    /// Attaches one live record, and returns what it names.
    fn attach_live_key(
        &mut self,
        key: &ResourceLiveRecordKeyV1,
    ) -> Result<Vec<RootKey>, CanwuError> {
        if self.state.holds_live(key) {
            return Ok(self
                .state
                .live_entry(key)
                .map(|entry| entry.references())
                .unwrap_or_default());
        }
        if self.live_attached.contains_key(key) {
            return Ok(Vec::new());
        }
        let bucket = resource_live_bucket(key, self.live_bucket_count);
        let Some(value) = self.live_bucket(bucket)?.entries.get(key).copied() else {
            return Ok(Vec::new());
        };
        let kept = self
            .live_buckets
            .get_mut(&bucket)
            .and_then(|stored| stored.decoded.remove(key));
        let (entry, size) = if let Some(kept) = kept {
            kept
        } else {
            let entry = decode_canonical::<ResourceLiveEntryV1>(value)?;
            if &entry.key != key {
                return Err(invalid("resource live entry differs from its stored key"));
            }
            let size = encoded_len(&entry).map_err(resource_canwu_error)?;
            (entry, size)
        };
        let references = entry.references();
        let mut ledger = std::mem::take(&mut self.state.live);
        let counted = ledger.count(&entry.payload, size, false);
        self.state.live = ledger;
        counted.map_err(|_| invalid("resource live ledger is short of a stored live entry"))?;
        self.state
            .attach_live(entry.clone())
            .map_err(resource_canwu_error)?;
        self.live_attached.insert(
            key.clone(),
            AttachedLive {
                bucket,
                entry,
                size,
            },
        );
        Ok(references)
    }

    /// Attaches the hot terminal records `keys` name, with their closures,
    /// live records included ([`Self::attach_keys`]).
    pub(crate) fn attach(
        &mut self,
        keys: impl IntoIterator<Item = ResourceTerminalRecordKeyV1>,
    ) -> Result<(), CanwuError> {
        self.attach_keys(keys.into_iter().map(RootKey::Terminal), true)
    }

    /// Attaches the live records `keys` name, with what they name.
    pub(crate) fn attach_live(
        &mut self,
        keys: impl IntoIterator<Item = ResourceLiveRecordKeyV1>,
    ) -> Result<(), CanwuError> {
        self.attach_keys(keys.into_iter().map(RootKey::Live), true)
    }

    /// Attaches every record that `value` names by an identity string, with
    /// their closures ([`Self::attach_keys`]).
    pub(crate) fn attach_named(&mut self, value: &Value) -> Result<(), CanwuError> {
        let mut texts = BTreeSet::new();
        named_identities(value, &mut texts);
        let keys = texts
            .iter()
            .flat_map(|text| keys_named_by(text))
            .collect::<Vec<_>>();
        self.attach_keys(keys, true)
    }

    /// Attaches the live records every live demand names, and every account
    /// when one of `demands` draws on the pooled source policy: what an
    /// allocation pass over those demands reads.
    pub(crate) fn attach_demand_sources(
        &mut self,
        demands: &BTreeSet<crate::ResourceDemandId>,
    ) -> Result<(), CanwuError> {
        let mut keys = Vec::new();
        let mut pooled = false;
        for id in demands {
            if let Some(demand) = self.state.demands.get(id) {
                pooled |= matches!(
                    demand.source_policy,
                    crate::ResourceDemandSourcePolicyV1::Pooled
                );
                keys.push(RootKey::Terminal(ResourceTerminalRecordKeyV1::Demand(
                    id.clone(),
                )));
            }
        }
        if pooled {
            self.attach_all_accounts()?;
        }
        self.attach_keys(keys, true)
    }

    /// Attaches every account of the root, with the definitions and units
    /// they name: what a pooled demand draws on. Access grants and custodian
    /// indexes stay where they are.
    fn attach_all_accounts(&mut self) -> Result<(), CanwuError> {
        let (_, buckets) = self.stored_bucket_indexes()?;
        let mut keys = Vec::new();
        for bucket in buckets {
            keys.extend(
                self.live_bucket(bucket)?
                    .entries
                    .keys()
                    .filter(|key| matches!(key, ResourceLiveRecordKeyV1::Account(_)))
                    .cloned()
                    .map(RootKey::Live),
            );
        }
        self.attach_keys(keys, true)
    }

    /// The indexes of the stored buckets: hot terminal, then live.
    fn stored_bucket_indexes(&self) -> Result<(Vec<u32>, Vec<u32>), CanwuError> {
        stored_bucket_indexes(&self.source)
    }

    /// What the work read, without the source: the state, the attached
    /// records, and every bucket it read or inherited.
    fn keep(self) -> KeptWork {
        let mut hot = self.inherited;
        hot.extend(
            self.buckets
                .into_iter()
                .map(|(bucket, stored)| (bucket, stored.keep())),
        );
        let mut live = self.live_inherited;
        live.extend(
            self.live_buckets
                .into_iter()
                .map(|(bucket, stored)| (bucket, stored.keep())),
        );
        KeptWork {
            state: self.state,
            attached: self.attached,
            live_attached: self.live_attached,
            hot,
            live,
        }
    }

    /// Attaches every live record of the root.
    pub(crate) fn attach_all_live(&mut self) -> Result<(), CanwuError> {
        let (_, buckets) = self.stored_bucket_indexes()?;
        let mut keys = Vec::new();
        for bucket in buckets {
            keys.extend(
                self.live_bucket(bucket)?
                    .entries
                    .keys()
                    .cloned()
                    .map(RootKey::Live),
            );
        }
        self.attach_keys(keys, true)
    }

    /// Attaches every hot terminal and live record of the root, which makes
    /// the state whole.
    pub(crate) fn attach_all(&mut self) -> Result<(), CanwuError> {
        let (buckets, _) = self.stored_bucket_indexes()?;
        let mut keys = Vec::new();
        for bucket in buckets {
            keys.extend(
                self.bucket(bucket)?
                    .entries
                    .keys()
                    .cloned()
                    .map(RootKey::Terminal),
            );
        }
        self.attach_keys(keys, false)?;
        self.attach_all_live()?;
        if !self.state.hot_terminal_records_attached()
            || self.state.hot_terminal.bytes != 0
            || !self.state.live_records_attached()
        {
            return Err(invalid(
                "resource bucket ledgers differ from the stored entries",
            ));
        }
        // Every stored entry is attached now, so each bucket's declaration
        // is checked against all of its entries.
        let mut by_bucket = BTreeMap::<u32, Vec<&ResourceHotTerminalEntryV1>>::new();
        for attached in self.attached.values() {
            by_bucket
                .entry(attached.bucket)
                .or_default()
                .push(&attached.entry);
        }
        for (bucket, stored) in &self.buckets {
            check_hot_bucket_declaration(
                &stored.declared,
                by_bucket.get(bucket).into_iter().flatten().copied(),
            )?;
        }
        Ok(())
    }

    /// Applies one verified archive commit to the root: a commit prepared
    /// from this root archives its selected records, each taken out of its
    /// bucket and out of the ledger, and moves the archive head; any other
    /// commit is stale. Records the work has not attached are removed from
    /// their stored entries without being decoded, so a commit reads and
    /// rewrites each bucket it empties once.
    pub(crate) fn apply_archive_commit(
        &mut self,
        commit: &crate::VerifiedResourceArchiveCommitV1,
    ) -> Result<crate::ResourceArchiveMaintenanceReceiptV1, CanwuError> {
        commit.validate().map_err(resource_canwu_error)?;
        let applied = self
            .state
            .archive_source_root()
            .map_err(resource_canwu_error)?
            == commit.expected_source_root;
        if applied {
            let selected: BTreeSet<_> = commit.selected.iter().cloned().collect();
            let attached: BTreeSet<_> = self
                .state
                .terminal_archive_candidates
                .values()
                .filter(|candidate| selected.contains(*candidate))
                .cloned()
                .collect();
            if selected.len() != commit.selected.len() {
                return Err(invalid(
                    "an applied resource archive commit selects a record twice",
                ));
            }
            let not_candidate = || {
                CanwuError::new(
                    ErrorCode::InvalidDomainRecord,
                    "an applied resource archive commit selects a record that is not a hot candidate",
                )
            };
            let mut demands = BTreeSet::new();
            for key in &commit.selected {
                if attached.contains(key) {
                    self.state
                        .remove_hot_terminal(key)
                        .map_err(resource_canwu_error)?;
                    continue;
                }
                if self.holds(key) || self.archived.contains_key(key) {
                    return Err(not_candidate());
                }
                let bucket = resource_hot_terminal_bucket(key, self.bucket_count);
                let value = self
                    .bucket(bucket)?
                    .entries
                    .get(key)
                    .copied()
                    .ok_or_else(not_candidate)?;
                let size = encoded_len(value).map_err(resource_canwu_error)?;
                let (requester, policy) = stored_hot_facts(key, value)?;
                if let ResourceTerminalRecordKeyV1::Reservation(_) = key {
                    let demand = value
                        .get("payload")
                        .and_then(|payload| payload.get("value"))
                        .and_then(|reservation| reservation.get("demand"))
                        .map(crate::ResourceDemandId::deserialize)
                        .transpose()
                        .map_err(|error| invalid(error.to_string()))?
                        .ok_or_else(|| invalid("resource hot terminal reservation is malformed"))?;
                    demands.insert(demand);
                }
                let facts = HotTerminalFacts {
                    requester: requester.as_ref(),
                    floor_policy: policy.as_ref(),
                };
                self.state
                    .hot_terminal
                    .count_facts(key, &facts, false)
                    .map_err(resource_canwu_error)?;
                self.state.hot_terminal.bytes = self
                    .state
                    .hot_terminal
                    .bytes
                    .checked_sub(size)
                    .ok_or_else(|| invalid("resource hot terminal ledger bytes are short"))?;
                self.archived.insert(key.clone(), bucket);
            }
            self.state
                .terminal_archive_candidates
                .retain(|_, candidate| !selected.contains(candidate));
            // A commit may archive part of a demand's closure: the demand,
            // still hot, is rewritten with the reservations it keeps.
            let kept = demands
                .into_iter()
                .filter(|demand| {
                    !selected.contains(&ResourceTerminalRecordKeyV1::Demand(demand.clone()))
                })
                .map(|demand| RootKey::Terminal(ResourceTerminalRecordKeyV1::Demand(demand)))
                .collect::<Vec<_>>();
            self.attach_keys(kept, false)?;
        }
        self.state
            .record_archive_commit(commit, applied)
            .map_err(resource_canwu_error)
    }

    /// The record mutations that persist the state as it now is: the live
    /// root, as an update of the version this work read, and every bucket
    /// whose entries changed. A record that stays attached and unchanged is
    /// not written again. The caller sets the state revision and validates
    /// the state first. The work is spent: its state is left empty.
    ///
    /// A record the work did not attach that its bucket already holds means
    /// the settlement read past a stored record: it fails with
    /// `invalid_boundary`, an engine invariant, instead of writing over it.
    pub(crate) fn persist(&mut self, summary: &str) -> Result<Vec<BoundaryDirective>, CanwuError> {
        let empty = ResourceState::empty(self.state.limits).map_err(resource_canwu_error)?;
        let state = std::mem::replace(&mut self.state, empty);
        let (live, entries, live_entries) = state.split_parts().map_err(resource_canwu_error)?;
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
        for (key, bucket) in &self.archived {
            changed.entry(*bucket).or_default().insert(key.clone());
        }
        for key in present.keys() {
            if !self.attached.contains_key(key) {
                changed
                    .entry(resource_hot_terminal_bucket(key, self.bucket_count))
                    .or_default()
                    .insert(key.clone());
            }
        }
        let live_present: BTreeMap<_, _> = live_entries
            .into_iter()
            .map(|entry| (entry.key.clone(), entry))
            .collect();
        let mut live_changed = BTreeMap::<u32, BTreeSet<ResourceLiveRecordKeyV1>>::new();
        for (key, attached) in &self.live_attached {
            if live_present.get(key) != Some(&attached.entry) {
                live_changed
                    .entry(attached.bucket)
                    .or_default()
                    .insert(key.clone());
            }
        }
        for key in live_present.keys() {
            if !self.live_attached.contains_key(key) {
                live_changed
                    .entry(resource_live_bucket(key, self.live_bucket_count))
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
        let read_past = || {
            CanwuError::new(
                ErrorCode::InvalidBoundary,
                "a resource boundary would overwrite a bucketed record it did not read",
            )
        };
        for (bucket, keys) in changed {
            let (version, mut entries) = {
                let stored = self.bucket(bucket)?;
                let entries: BTreeMap<_, _> = stored
                    .entries
                    .iter()
                    .map(|(key, value)| (key.clone(), (*value).clone()))
                    .collect();
                (stored.version, entries)
            };
            let attached = &self.attached;
            for key in keys {
                match present.get(&key) {
                    Some(entry) => {
                        // A record this work did not attach is new to the
                        // root. One already stored was read past: the
                        // settlement could not have seen it, so writing over
                        // it would record a second, unseen answer for it.
                        if !attached.contains_key(&key) && entries.contains_key(&key) {
                            return Err(read_past());
                        }
                        entries.insert(key, encode_value(entry)?);
                    }
                    None => {
                        entries.remove(&key);
                    }
                }
            }
            // The declaration covers every entry the bucket keeps: those this
            // work holds, and the stored ones it did not attach, decoded here.
            let mut kept = Vec::with_capacity(entries.len());
            for (key, value) in &entries {
                match present.get(key) {
                    Some(entry) => kept.push(entry.clone()),
                    None => kept.push(ResourceHotTerminalEntryV1::deserialize(value).map_err(
                        |error| {
                            invalid(format!("resource bucket entry cannot be decoded: {error}"))
                        },
                    )?),
                }
            }
            let evidence = encode_value(&hot_bucket_declaration(&kept))?;
            directives.push(bucket_directive(
                resource_hot_terminal_bucket_reference(bucket).into_untyped(),
                serde_json::json!({
                    "format_version": RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION,
                    "bucket": bucket,
                    "entries": entries.into_values().collect::<Vec<_>>(),
                    IDENTITY_EVIDENCE_DEPENDENCIES_FIELD: evidence,
                }),
                version,
                summary,
            ));
        }
        for (bucket, keys) in live_changed {
            let (version, mut entries) = {
                let stored = self.live_bucket(bucket)?;
                let entries: BTreeMap<_, _> = stored
                    .entries
                    .iter()
                    .map(|(key, value)| (key.clone(), (*value).clone()))
                    .collect();
                (stored.version, entries)
            };
            let attached = &self.live_attached;
            for key in keys {
                let entry = live_present
                    .get(&key)
                    .ok_or_else(|| invalid("a resource live record left the root"))?;
                if !attached.contains_key(&key) && entries.contains_key(&key) {
                    return Err(read_past());
                }
                entries.insert(key, encode_value(entry)?);
            }
            directives.push(bucket_directive(
                resource_live_bucket_reference(bucket).into_untyped(),
                serde_json::json!({
                    "format_version": RESOURCE_LIVE_BUCKET_FORMAT_VERSION,
                    "bucket": bucket,
                    "entries": entries.into_values().collect::<Vec<_>>(),
                    IDENTITY_EVIDENCE_DEPENDENCIES_FIELD: encode_value(&empty_declaration())?,
                }),
                version,
                summary,
            ));
        }
        Ok(directives)
    }
}

fn bucket_directive(
    reference: DomainRecordRef,
    payload: Value,
    version: Option<u64>,
    summary: &str,
) -> BoundaryDirective {
    let record = DomainRecordDraft::new(reference, payload);
    BoundaryDirective::MutateRecord {
        mutation: match version {
            None => DomainRecordMutation::Create { record },
            Some(expected_version) => DomainRecordMutation::Update {
                record,
                expected_version,
            },
        },
        summary: summary.to_owned(),
    }
}

/// The resource root as a boundary system or command admission of another
/// plugin reads it, or a host application reads it from a run handle: the
/// live root, with the bucketed records it asks for attached.
///
/// [`Self::attach`] attaches the hot terminal records some keys name, and
/// every record those name in turn (the closure of a demand, what a
/// reservation, allocation leg, consumption, fulfillment, transfer, or loss
/// belongs to, and the accounts, access grants, definitions, and units they
/// name). [`Self::attach_live`] attaches accounts, access grants,
/// definitions, and unit revisions by identity. The state then answers every
/// question about its live root and about the attached records exactly as the
/// whole root would; a bucketed record it was not asked for is absent from
/// its maps. Its ledgers ([`ResourceState::hot_terminal_records`],
/// [`ResourceState::account_count`], [`ResourceState::total_account_balance`])
/// count every record. Reading in a boundary needs the `canwu.resource`
/// `runtime` and `bucket` state keys ([`crate::resource_hot_terminal_state_key`]).
///
/// A host reads the same way from a run handle with [`Self::load_run`]: for
/// the same committed state and the same requests, the reader holds exactly
/// the state a boundary system's reader built from
/// [`Self::load`] holds before anything is staged in its boundary. That read
/// is a trusted-host read of every custodian's ground truth: the host
/// selects what belongs to an actor before anything reaches it (the
/// holder-relative report is [`crate::resource_report`]).
///
/// [`Self::keep`] turns a reader into a [`KeptResourceRoot`] that borrows
/// nothing, and [`Self::reattach`] repeats its requests on the run's current
/// root, reading again only the buckets whose records changed.
pub struct ResourceRootReader<'v, 'a> {
    live: Arc<DomainRecord>,
    /// Whether `live` is the run's own copy of the live root.
    live_shared: bool,
    work: ResourceRootWork<'v, ReaderSource<'v, 'a>>,
    requests: RootRequests,
    /// Whether a request failed: its state is then partial, and a re-attach
    /// repeats every request instead of taking it.
    failed: bool,
}

/// What a reader was asked to attach, as sets: the attached state is the
/// closure of their union, whatever the order of the requests.
#[derive(Clone, Debug, Default)]
struct RootRequests {
    all: bool,
    terminal: BTreeSet<ResourceTerminalRecordKeyV1>,
    live: BTreeSet<ResourceLiveRecordKeyV1>,
    custodians: BTreeSet<KnowledgeHolderRef>,
}

impl<'v, 'a> ResourceRootReader<'v, 'a> {
    /// The live root of the view, with nothing attached, or `None` without a
    /// resource root.
    pub fn load(view: &'v SimulationView<'a>) -> Result<Option<Self>, CanwuError> {
        let Some(live) = view.typed_domain_record(&resource_runtime_reference())? else {
            return Ok(None);
        };
        let work = ResourceRootWork::new(ReaderSource::View(view), live)?;
        Ok(Some(Self {
            live: Arc::new(live.clone()),
            live_shared: false,
            work,
            requests: RootRequests::default(),
            failed: false,
        }))
    }

    /// Notes a failed request, so that what this reader keeps is read again.
    fn note<T>(&mut self, result: Result<T, CanwuError>) -> Result<T, CanwuError> {
        self.failed |= result.is_err();
        result
    }

    /// Attaches the hot terminal records `keys` name, with their closures. A
    /// key that names no hot terminal record is ignored.
    pub fn attach(
        &mut self,
        keys: impl IntoIterator<Item = ResourceTerminalRecordKeyV1>,
    ) -> Result<(), CanwuError> {
        let keys = keys.into_iter().collect::<Vec<_>>();
        self.requests.terminal.extend(keys.iter().cloned());
        let attached = self.work.attach(keys);
        self.note(attached)
    }

    /// Attaches the accounts, access grants, definitions, and unit revisions
    /// `keys` name, with the definitions and units they name. A key that
    /// names no record is ignored.
    pub fn attach_live(
        &mut self,
        keys: impl IntoIterator<Item = ResourceLiveRecordKeyV1>,
    ) -> Result<(), CanwuError> {
        let keys = keys.into_iter().collect::<Vec<_>>();
        self.requests.live.extend(keys.iter().cloned());
        let attached = self.work.attach_live(keys);
        self.note(attached)
    }

    /// Attaches every account `holder` holds, through the custodian index,
    /// and returns their identities. It reads the index and those accounts,
    /// not the other accounts of the root.
    pub fn attach_custodian_accounts(
        &mut self,
        holder: &KnowledgeHolderRef,
    ) -> Result<BTreeSet<ResourceAccountId>, CanwuError> {
        self.requests.custodians.insert(holder.clone());
        let attached = self.attach_custodian_index(holder);
        self.note(attached)
    }

    fn attach_custodian_index(
        &mut self,
        holder: &KnowledgeHolderRef,
    ) -> Result<BTreeSet<ResourceAccountId>, CanwuError> {
        self.work
            .attach_live([ResourceLiveRecordKeyV1::Custodian(holder.clone())])?;
        let accounts = self
            .work
            .state
            .custodian_accounts
            .get(holder)
            .cloned()
            .unwrap_or_default();
        self.work.attach_live(
            accounts
                .iter()
                .cloned()
                .map(ResourceLiveRecordKeyV1::Account),
        )?;
        Ok(accounts)
    }

    /// Attaches every hot terminal and live record of the root, which makes
    /// the state whole: the state [`crate::resource_state`] reads from a run,
    /// and [`whole_resource_root_in_view`] in a boundary. It lists the stored
    /// buckets by their record kind, decodes each, and checks that the
    /// ledgers count exactly the stored entries, that each hot terminal
    /// bucket declares what its entries cite, and that every hot terminal
    /// reservation's demand is hot terminal too.
    pub fn attach_all(&mut self) -> Result<(), CanwuError> {
        self.requests.all = true;
        let attached = self
            .work
            .attach_all()
            .and_then(|()| check_demand_closures(&self.work.state));
        self.note(attached)
    }

    /// The live root record, whose version is the root's state revision.
    #[must_use]
    pub fn live_record(&self) -> &DomainRecord {
        &self.live
    }

    /// The live root with the attached records.
    #[must_use]
    pub fn state(&self) -> &ResourceState {
        &self.work.state
    }

    /// The live root with the attached records.
    #[must_use]
    pub fn into_state(self) -> ResourceState {
        self.work.state
    }

    /// How many stored buckets this reader parsed and decoded from its
    /// source. A reader from [`Self::reattach`] counts only the buckets whose
    /// records changed since the kept root read them (or that it read for
    /// the first time); one whose root did not change at all counts none.
    ///
    /// A diagnostic of decode work, not of all work (a re-attach after the
    /// live root changed also attaches every kept record again), and it
    /// depends on the host process's history (a fork or a reload shares or
    /// copies records): it must never feed a simulation input.
    #[must_use]
    pub fn buckets_decoded(&self) -> usize {
        self.work.parsed
    }

    /// What this reader read, kept without borrowing its source: the state,
    /// the decoded records, its requests, and the run's own copy of every
    /// record it read ([`canwu_api::CanwuRun::domain_record_shared`]).
    /// Nothing is copied or decoded again. A kept root holds those copies, so
    /// a record the run has since replaced stays in memory until the kept
    /// root is dropped or re-attached.
    ///
    /// A reader over a view keeps no shared copy (a view has none), so a
    /// re-attach of what it kept reads everything again, and so does one of
    /// a reader whose request failed.
    #[must_use]
    pub fn keep(self) -> KeptResourceRoot {
        let work = self.work.keep();
        KeptResourceRoot {
            live: self.live,
            live_shared: self.live_shared,
            failed: self.failed,
            requests: self.requests,
            state: work.state,
            attached: work.attached,
            live_attached: work.live_attached,
            hot: work.hot,
            live_buckets: work.live,
        }
    }

    /// The live root record and the state, for the crate's own readers.
    fn into_parts(self) -> (DomainRecord, ResourceState) {
        (Arc::unwrap_or_clone(self.live), self.work.state)
    }

    /// Repeats `requests` on this reader.
    fn replay(&mut self, requests: RootRequests) -> Result<(), CanwuError> {
        if requests.all {
            return self.attach_all();
        }
        self.attach(requests.terminal)?;
        self.attach_live(requests.live)?;
        for holder in &requests.custodians {
            self.attach_custodian_accounts(holder)?;
        }
        Ok(())
    }
}

impl<'r> ResourceRootReader<'r, 'r> {
    /// The live root of a run, from either handle ([`canwu_api::Canwu`],
    /// [`canwu_api::CompactedCanwu`], or any [`CanwuRun`]), with nothing
    /// attached, or `None` without a resource root.
    ///
    /// The host-side reader: its requests read the live root and the buckets
    /// of the records they name, as [`Self::load`] reads a view, never every
    /// bucket. A trusted-host read: see the type's documentation.
    pub fn load_run<H: CanwuRun + ?Sized>(run: &'r H) -> Result<Option<Self>, CanwuError> {
        let Some((live, live_shared)) = run_live_root(run) else {
            return Ok(None);
        };
        let work = ResourceRootWork::new(ReaderSource::Run(Box::new(RunSource(run))), &live)?;
        Ok(Some(Self {
            live,
            live_shared,
            work,
            requests: RootRequests::default(),
            failed: false,
        }))
    }

    /// A reader of the run's current root that holds what a reader loaded
    /// with [`Self::load_run`] and given `kept`'s requests holds, built from
    /// `kept` as far as the run still holds the records `kept` read: or
    /// `None` without a resource root.
    ///
    /// A record counts as unchanged only when the run holds the very copy
    /// `kept` read: the allocation that [`CanwuRun::domain_record`] reads and
    /// [`CanwuRun::domain_record_shared`] returns, compared with
    /// [`Arc::ptr_eq`]. That is never true of a record a step rewrote, of a
    /// fork that wrote it, or of another run such as a restored one,
    /// whatever their version numbers. A run handle whose
    /// `domain_record_shared` returns another copy than the one
    /// `domain_record` reads (a fresh copy per call, or one cached by version)
    /// is read correctly but offers nothing to reuse: every re-attach reads
    /// everything again.
    ///
    /// When the live root and every bucket `kept` read are unchanged (and,
    /// for a whole root, no bucket was added), the reader takes `kept`'s
    /// state and decodes nothing. Otherwise it decodes the live root,
    /// repeats the requests, and decodes only the buckets whose records
    /// changed or that it reads for the first time, taking `kept`'s decoded
    /// entries for the rest ([`Self::buckets_decoded`] counts the decodes).
    /// That costs a decode of the live root and an attach of every record
    /// the requests reach again, cloned from `kept`: a host that needs work
    /// in proportion to what changed reads targeted records instead of a
    /// whole root.
    ///
    /// A request that failed on the kept reader is repeated, so the re-attach
    /// fails where a reader loaded afresh with the same requests fails.
    pub fn reattach<H: CanwuRun + ?Sized>(
        run: &'r H,
        kept: KeptResourceRoot,
    ) -> Result<Option<Self>, CanwuError> {
        let Some((live, live_now_shared)) = run_live_root(run) else {
            return Ok(None);
        };
        let source = RunSource(run);
        let KeptResourceRoot {
            live: kept_live,
            live_shared,
            failed,
            requests,
            state,
            mut attached,
            mut live_attached,
            mut hot,
            mut live_buckets,
        } = kept;
        let (hot_read, live_read) = (hot.len(), live_buckets.len());
        hot.retain(|bucket, kept| kept.still_held(&source, *bucket));
        live_buckets.retain(|bucket, kept| kept.still_held(&source, *bucket));
        let unchanged = !failed
            && live_shared
            && live_now_shared
            && Arc::ptr_eq(&kept_live, &live)
            && hot.len() == hot_read
            && live_buckets.len() == live_read
            && (!requests.all || stored_buckets_are_kept(&source, &hot, &live_buckets)?);
        let source = ReaderSource::Run(Box::new(source));
        if unchanged {
            let mut work =
                ResourceRootWork::resume(source, live.version, state, attached, live_attached);
            work.inherited = hot;
            work.live_inherited = live_buckets;
            return Ok(Some(Self {
                live,
                live_shared: live_now_shared,
                work,
                requests,
                failed: false,
            }));
        }
        // The kept decodes of attached records whose buckets are unchanged
        // serve the repeated requests.
        for (key, entry) in std::mem::take(&mut attached) {
            if let Some(bucket) = hot.get_mut(&entry.bucket) {
                bucket.decoded.insert(key, (entry.entry, entry.size));
            }
        }
        for (key, entry) in std::mem::take(&mut live_attached) {
            if let Some(bucket) = live_buckets.get_mut(&entry.bucket) {
                bucket.decoded.insert(key, (entry.entry, entry.size));
            }
        }
        let mut work = ResourceRootWork::new(source, &live)?;
        // Bucket indexes name the same buckets only under the same counts,
        // which a root keeps for its life; another root's are not reused.
        if work.bucket_count == resource_hot_terminal_bucket_count(&state.limits)
            && work.live_bucket_count == resource_live_bucket_count(&state.limits)
        {
            work.inherited = hot;
            work.live_inherited = live_buckets;
        }
        let mut reader = Self {
            live,
            live_shared: live_now_shared,
            work,
            requests: RootRequests::default(),
            failed: false,
        };
        reader.replay(requests)?;
        Ok(Some(reader))
    }
}

/// The run's live root, and whether it is the run's own shared copy
/// ([`run_shared_record`]); a run that offers none is read through a copy.
fn run_live_root<H: CanwuRun + ?Sized>(run: &H) -> Option<(Arc<DomainRecord>, bool)> {
    let reference = resource_runtime_reference().into_untyped();
    match run_shared_record(run, &reference) {
        Some(shared) => Some((shared, true)),
        None => run
            .domain_record(&reference)
            .map(|live| (Arc::new(live.clone()), false)),
    }
}

/// Whether every bucket `source` stores is one of the kept `hot` and `live`
/// buckets that hold a record: what a whole root's re-attach checks besides
/// the kept buckets themselves, since a whole read reads every stored bucket.
fn stored_buckets_are_kept<'s, S: HotTerminalSource<'s> + ?Sized>(
    source: &S,
    hot: &BTreeMap<u32, KeptBucket<ResourceTerminalRecordKeyV1>>,
    live: &BTreeMap<u32, KeptBucket<ResourceLiveRecordKeyV1>>,
) -> Result<bool, CanwuError> {
    fn stored<K: BucketKey>(kept: Option<&KeptBucket<K>>) -> bool {
        kept.is_some_and(|kept| matches!(kept.record, KeptRecord::Shared(_)))
    }
    let (stored_hot, stored_live) = stored_bucket_indexes(source)?;
    Ok(stored_hot.iter().all(|bucket| stored(hot.get(bucket)))
        && stored_live.iter().all(|bucket| stored(live.get(bucket))))
}

/// What a [`ResourceRootReader`] read, kept by a host between steps without
/// borrowing the run: the state it read, the records it decoded, what it was
/// asked to attach, and the run's own copy of each record it read.
/// [`ResourceRootReader::reattach`] turns it into a reader of the run's
/// current root that reads again only what changed.
///
/// It is host memory, never saved: it holds no state of the run that the run
/// does not hold itself, and a restore or a new run makes every record new.
#[derive(Clone, Debug)]
pub struct KeptResourceRoot {
    live: Arc<DomainRecord>,
    /// Whether `live` is the run's own copy (a reader over a run).
    live_shared: bool,
    /// Whether a request of the kept reader failed.
    failed: bool,
    requests: RootRequests,
    state: ResourceState,
    attached: BTreeMap<ResourceTerminalRecordKeyV1, AttachedEntry>,
    live_attached: BTreeMap<ResourceLiveRecordKeyV1, AttachedLive>,
    hot: BTreeMap<u32, KeptBucket<ResourceTerminalRecordKeyV1>>,
    live_buckets: BTreeMap<u32, KeptBucket<ResourceLiveRecordKeyV1>>,
}

impl KeptResourceRoot {
    /// The live root record the reader read.
    #[must_use]
    pub fn live_record(&self) -> &DomainRecord {
        &self.live
    }

    /// The state the reader read: the live root with the records it
    /// attached, as it was when it was kept. Like the reader's, it is
    /// every custodian's ground truth, a trusted-host read: the host selects
    /// what belongs to an actor before anything reaches it.
    #[must_use]
    pub fn state(&self) -> &ResourceState {
        &self.state
    }
}

/// Reads the resource root in a boundary system or in command admission:
/// the live root, with the hot terminal records `keys` name attached, and
/// what they name ([`ResourceRootReader`]). Returns the live root record
/// beside the state, or `None` without a resource root.
pub fn resource_root_in_view(
    view: &SimulationView<'_>,
    keys: impl IntoIterator<Item = ResourceTerminalRecordKeyV1>,
) -> Result<Option<(DomainRecord, ResourceState)>, CanwuError> {
    let Some(mut reader) = ResourceRootReader::load(view)? else {
        return Ok(None);
    };
    reader.attach(keys)?;
    Ok(Some(reader.into_parts()))
}

/// Reads the whole resource root in a boundary system: the live root and
/// every bucketed record. Its cost grows with the hot terminal backlog and
/// with the number of accounts, so a boundary system that needs only some
/// records reads them with [`ResourceRootReader`].
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
/// live root record and the state with every bucketed record attached
/// ([`ResourceRootReader::attach_all`]).
pub(crate) fn whole_resource_root<H: CanwuRun + ?Sized>(
    canwu: &H,
) -> Result<Option<(DomainRecord, ResourceState)>, CanwuError> {
    let Some(mut reader) = ResourceRootReader::load_run(canwu)? else {
        return Ok(None);
    };
    reader.attach_all()?;
    Ok(Some(reader.into_parts()))
}
