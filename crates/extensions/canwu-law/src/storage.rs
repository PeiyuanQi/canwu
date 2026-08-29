// This module is intentionally compiled only for internal contract tests while
// the format-8 persistence boundary remains dormant.
#![allow(dead_code)]

use canwu_api::{CanwuError, ErrorCode, SimTime, canonical_hash};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};

pub const LEGAL_STORAGE_FORMAT_VERSION: u32 = 1;
const COMPACTION_TOKEN_DOMAIN: &str = "canwu.law.compaction-token.v1";
const SOURCE_MEMBERSHIP_ROOT_DOMAIN: &str = "canwu.law.source-membership.v1";
const MEMBERSHIP_ROOT_DOMAIN: &str = "canwu.law.archive-membership.v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LegalShardKind {
    Order,
    Jurisdiction,
    Coordinator,
    CultureDependency,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct LegalShardKey {
    pub kind: LegalShardKind,
    pub legal_order: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<String>,
}

impl LegalShardKey {
    #[must_use]
    pub fn order(legal_order: impl Into<String>) -> Self {
        Self {
            kind: LegalShardKind::Order,
            legal_order: legal_order.into(),
            jurisdiction: None,
        }
    }

    #[must_use]
    pub fn jurisdiction(legal_order: impl Into<String>, jurisdiction: impl Into<String>) -> Self {
        Self {
            kind: LegalShardKind::Jurisdiction,
            legal_order: legal_order.into(),
            jurisdiction: Some(jurisdiction.into()),
        }
    }

    fn validate(&self) -> Result<(), CanwuError> {
        require_identifier(&self.legal_order, "legal shard order")?;
        match (self.kind, &self.jurisdiction) {
            (LegalShardKind::Jurisdiction, Some(jurisdiction)) => {
                require_identifier(jurisdiction, "legal shard jurisdiction")
            }
            (LegalShardKind::Jurisdiction, None) => Err(invalid(
                "jurisdiction shards require an exact jurisdiction identity",
            )),
            (_, Some(_)) => Err(invalid(
                "only jurisdiction shards may carry a jurisdiction identity",
            )),
            (_, None) => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LegalObjectKind {
    Proposal,
    Procedure,
    Participation,
    PendingIntent,
    Outbox,
    Source,
    Rule,
    LawVersion,
    Case,
    Finding,
    Ruling,
    Conflict,
    Succession,
    Coordinator,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct LegalObjectId {
    pub kind: LegalObjectKind,
    pub id: String,
    pub home_shard: LegalShardKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_discriminator: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct LegalVersionRef {
    pub object: LegalObjectId,
    pub version_ordinal: u64,
    pub content_commitment: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct LegalHeadRef {
    pub object: LegalObjectId,
    pub version: LegalVersionRef,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ArchiveObjectId {
    pub content_id: String,
    pub blob_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveReachabilityState {
    Prepared,
    Stored,
    Verified,
    DurableIngress,
    Committed,
    RejectedStale,
    Abandoned,
}

impl ArchiveReachabilityState {
    #[must_use]
    pub const fn protects_object(self) -> bool {
        matches!(
            self,
            Self::Prepared | Self::Stored | Self::Verified | Self::DurableIngress | Self::Committed
        )
    }

    #[must_use]
    pub fn may_advance_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Prepared, Self::Stored | Self::Abandoned)
                | (Self::Stored, Self::Verified | Self::Abandoned)
                | (Self::Verified, Self::DurableIngress | Self::Abandoned)
                | (
                    Self::DurableIngress,
                    Self::Committed | Self::RejectedStale | Self::Abandoned
                )
        ) || self == next
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArchiveObjectReceipt {
    pub object: ArchiveObjectId,
    pub owner_shard: LegalShardKey,
    pub archive_batch_sequence: u64,
    pub member_index: u64,
    pub codec: String,
    pub stored_bytes: u64,
    pub decoded_bytes: u64,
    pub source_root: String,
    pub verified_plan_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "location", rename_all = "snake_case")]
pub enum LegalVersionLocation {
    Hot,
    Archived { receipt: Box<ArchiveObjectReceipt> },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalArchiveMembership {
    pub version: LegalVersionRef,
    pub location: LegalVersionLocation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalArchiveHead {
    pub shard: LegalShardKey,
    pub committed_batch_count: u64,
    pub archived_member_count: u64,
    pub membership_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_content_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalCompactionCandidate {
    pub version: LegalVersionRef,
    pub record_class: String,
    pub closed_at: SimTime,
    pub encoded_bytes: u64,
    pub dependencies_resolved: bool,
    pub current_projection_retained: bool,
}

impl LegalCompactionCandidate {
    #[must_use]
    pub const fn is_eligible(&self) -> bool {
        self.dependencies_resolved && self.current_projection_retained
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalCompactionBudgets {
    pub max_records: usize,
    pub max_source_bytes: u64,
}

impl Default for LegalCompactionBudgets {
    fn default() -> Self {
        Self {
            max_records: 128,
            max_source_bytes: 4 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreparedLegalCompaction {
    pub token: String,
    pub shard: LegalShardKey,
    pub archive_batch_sequence: u64,
    pub source_membership_root: String,
    pub candidates: Vec<LegalCompactionCandidate>,
    pub source_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalStorageState {
    pub format_version: u32,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "ordered_map_serde"
    )]
    pub heads: BTreeMap<LegalObjectId, LegalHeadRef>,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "ordered_map_serde"
    )]
    pub membership: BTreeMap<LegalVersionRef, LegalArchiveMembership>,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "ordered_map_serde"
    )]
    pub compaction_candidates: BTreeMap<LegalVersionRef, LegalCompactionCandidate>,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "ordered_map_serde"
    )]
    pub archive_heads: BTreeMap<LegalShardKey, LegalArchiveHead>,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "ordered_map_serde"
    )]
    pub reachability: BTreeMap<ArchiveObjectId, ArchiveReachabilityState>,
}

impl Default for LegalStorageState {
    fn default() -> Self {
        Self {
            format_version: LEGAL_STORAGE_FORMAT_VERSION,
            heads: BTreeMap::new(),
            membership: BTreeMap::new(),
            compaction_candidates: BTreeMap::new(),
            archive_heads: BTreeMap::new(),
            reachability: BTreeMap::new(),
        }
    }
}

impl LegalStorageState {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.heads.is_empty()
            && self.membership.is_empty()
            && self.compaction_candidates.is_empty()
            && self.archive_heads.is_empty()
            && self.reachability.is_empty()
    }

    pub fn record_hot_head(&mut self, head: LegalHeadRef) -> Result<(), CanwuError> {
        validate_head(&head)?;
        if self.compaction_candidates.contains_key(&head.version) {
            return Err(invalid(
                "a legal version selected for compaction cannot become a current head",
            ));
        }
        if let Some(current) = self.heads.get(&head.object) {
            if current == &head {
                return Ok(());
            }
            if head.version.version_ordinal <= current.version.version_ordinal {
                return Err(invalid("legal head versions must advance monotonically"));
            }
        }
        if matches!(
            self.membership.get(&head.version),
            Some(LegalArchiveMembership {
                location: LegalVersionLocation::Archived { .. },
                ..
            })
        ) {
            return Err(invalid("an archived legal version cannot become hot again"));
        }
        self.membership.insert(
            head.version.clone(),
            LegalArchiveMembership {
                version: head.version.clone(),
                location: LegalVersionLocation::Hot,
            },
        );
        self.heads.insert(head.object.clone(), head);
        Ok(())
    }

    pub fn mark_compaction_candidate(
        &mut self,
        candidate: LegalCompactionCandidate,
    ) -> Result<(), CanwuError> {
        validate_version(&candidate.version)?;
        require_identifier(&candidate.record_class, "legal archive record class")?;
        if candidate.encoded_bytes == 0 {
            return Err(invalid(
                "legal compaction candidates must have nonzero bytes",
            ));
        }
        if !matches!(
            self.membership.get(&candidate.version),
            Some(LegalArchiveMembership {
                location: LegalVersionLocation::Hot,
                ..
            })
        ) {
            return Err(invalid(
                "only a hot exact legal version may become a candidate",
            ));
        }
        if self
            .heads
            .get(&candidate.version.object)
            .is_some_and(|head| head.version == candidate.version)
        {
            return Err(invalid("a current legal head cannot be compacted"));
        }
        self.compaction_candidates
            .insert(candidate.version.clone(), candidate);
        Ok(())
    }

    pub fn select_compaction_batch(
        &self,
        shard: &LegalShardKey,
        budgets: LegalCompactionBudgets,
    ) -> Result<Option<PreparedLegalCompaction>, CanwuError> {
        shard.validate()?;
        if budgets.max_records == 0 || budgets.max_source_bytes == 0 {
            return Err(invalid("legal compaction budgets must be nonzero"));
        }
        let mut candidates = self
            .compaction_candidates
            .values()
            .filter(|candidate| {
                candidate.version.object.home_shard == *shard
                    && candidate.is_eligible()
                    && self
                        .heads
                        .get(&candidate.version.object)
                        .is_none_or(|head| head.version != candidate.version)
            })
            .cloned()
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            (
                &left.record_class,
                left.closed_at,
                left.version.version_ordinal,
                &left.version.object.id,
                &left.version.object.local_discriminator,
            )
                .cmp(&(
                    &right.record_class,
                    right.closed_at,
                    right.version.version_ordinal,
                    &right.version.object.id,
                    &right.version.object.local_discriminator,
                ))
        });

        let mut selected = Vec::new();
        let mut source_bytes = 0_u64;
        for candidate in candidates {
            if selected.len() == budgets.max_records {
                break;
            }
            let next_bytes = source_bytes
                .checked_add(candidate.encoded_bytes)
                .ok_or_else(|| invalid("legal compaction byte count overflowed"))?;
            if next_bytes > budgets.max_source_bytes {
                continue;
            }
            source_bytes = next_bytes;
            selected.push(candidate);
        }
        if selected.is_empty() {
            return Ok(None);
        }

        let archive_batch_sequence = match self.archive_heads.get(shard) {
            Some(head) => head
                .committed_batch_count
                .checked_add(1)
                .ok_or_else(|| invalid("legal archive batch sequence is exhausted"))?,
            None => 1,
        };
        let source_membership_root = self.source_membership_root_for_shard(shard)?;
        let token = canonical_hash(
            COMPACTION_TOKEN_DOMAIN,
            &(
                LEGAL_STORAGE_FORMAT_VERSION,
                shard,
                archive_batch_sequence,
                &source_membership_root,
                &selected,
                source_bytes,
            ),
        )?;
        Ok(Some(PreparedLegalCompaction {
            token,
            shard: shard.clone(),
            archive_batch_sequence,
            source_membership_root,
            candidates: selected,
            source_bytes,
        }))
    }

    pub fn commit_compaction(
        &mut self,
        prepared: &PreparedLegalCompaction,
        receipts: Vec<ArchiveObjectReceipt>,
    ) -> Result<(), CanwuError> {
        let mut next = self.clone();
        next.commit_compaction_in_place(prepared, receipts)?;
        next.validate()?;
        *self = next;
        Ok(())
    }

    fn commit_compaction_in_place(
        &mut self,
        prepared: &PreparedLegalCompaction,
        receipts: Vec<ArchiveObjectReceipt>,
    ) -> Result<(), CanwuError> {
        let expected = self
            .select_compaction_batch(
                &prepared.shard,
                LegalCompactionBudgets {
                    max_records: prepared.candidates.len(),
                    max_source_bytes: prepared.source_bytes,
                },
            )?
            .ok_or_else(|| invalid("prepared legal compaction no longer has eligible members"))?;
        if expected != *prepared
            || self.source_membership_root_for_shard(&prepared.shard)?
                != prepared.source_membership_root
        {
            return Err(invalid("prepared legal compaction is stale"));
        }
        if receipts.len() != prepared.candidates.len() {
            return Err(invalid(
                "archive receipts do not cover the prepared membership",
            ));
        }

        let mut receipts = receipts;
        receipts.sort_by_key(|receipt| receipt.member_index);
        for (index, (candidate, receipt)) in prepared.candidates.iter().zip(&receipts).enumerate() {
            if receipt.owner_shard != prepared.shard
                || receipt.archive_batch_sequence != prepared.archive_batch_sequence
                || receipt.member_index
                    != u64::try_from(index).map_err(|_| {
                        invalid("legal archive membership index exceeds the persistent range")
                    })?
                || receipt.stored_bytes == 0
                || receipt.decoded_bytes == 0
                || receipt.source_root != prepared.source_membership_root
                || receipt.verified_plan_hash != prepared.token
            {
                return Err(invalid("archive receipt disagrees with the prepared batch"));
            }
            validate_hash(&receipt.object.content_id, "archive content ID")?;
            validate_hash(&receipt.object.blob_id, "archive blob ID")?;
            validate_hash(&receipt.source_root, "archive source root")?;
            validate_hash(&receipt.verified_plan_hash, "archive plan hash")?;
            require_identifier(&receipt.codec, "archive codec")?;
            if self.reachability.get(&receipt.object)
                != Some(&ArchiveReachabilityState::DurableIngress)
            {
                return Err(invalid(
                    "archive receipt must have durable ingress reachability before commit",
                ));
            }
            if candidate.version.object.home_shard != prepared.shard {
                return Err(invalid(
                    "prepared legal compaction contains a foreign-shard member",
                ));
            }
        }

        for (candidate, receipt) in prepared.candidates.iter().zip(&receipts) {
            self.membership.insert(
                candidate.version.clone(),
                LegalArchiveMembership {
                    version: candidate.version.clone(),
                    location: LegalVersionLocation::Archived {
                        receipt: Box::new(receipt.clone()),
                    },
                },
            );
            self.compaction_candidates.remove(&candidate.version);
        }
        for object in receipts
            .iter()
            .map(|receipt| receipt.object.clone())
            .collect::<BTreeSet<_>>()
        {
            self.reachability
                .insert(object, ArchiveReachabilityState::Committed);
        }

        let archived_member_count = self
            .membership
            .values()
            .filter(|membership| {
                membership.version.object.home_shard == prepared.shard
                    && matches!(membership.location, LegalVersionLocation::Archived { .. })
            })
            .count();
        let archived_member_count = u64::try_from(archived_member_count)
            .map_err(|_| invalid("legal archive member count exceeds the persistent range"))?;
        let membership_root = self.archived_membership_root_for_shard(&prepared.shard)?;
        let last_content_id = receipts
            .last()
            .map(|receipt| receipt.object.content_id.clone());
        self.archive_heads.insert(
            prepared.shard.clone(),
            LegalArchiveHead {
                shard: prepared.shard.clone(),
                committed_batch_count: prepared.archive_batch_sequence,
                archived_member_count,
                membership_root,
                last_content_id,
            },
        );
        Ok(())
    }

    pub fn advance_reachability(
        &mut self,
        object: ArchiveObjectId,
        next: ArchiveReachabilityState,
    ) -> Result<(), CanwuError> {
        validate_hash(&object.content_id, "archive content ID")?;
        validate_hash(&object.blob_id, "archive blob ID")?;
        let current = self
            .reachability
            .get(&object)
            .copied()
            .unwrap_or(ArchiveReachabilityState::Prepared);
        if !current.may_advance_to(next) {
            return Err(invalid("archive reachability transition is invalid"));
        }
        self.reachability.insert(object, next);
        Ok(())
    }

    #[must_use]
    pub fn leased_archive_object_ids(&self) -> BTreeSet<ArchiveObjectId> {
        self.reachability
            .iter()
            .filter_map(|(object, state)| state.protects_object().then_some(object.clone()))
            .collect()
    }

    #[must_use]
    pub fn reachable_archive_object_ids(&self) -> BTreeSet<ArchiveObjectId> {
        let mut reachable = self.leased_archive_object_ids();
        for membership in self.membership.values() {
            if let LegalVersionLocation::Archived { receipt } = &membership.location {
                reachable.insert(receipt.object.clone());
            }
        }
        reachable
    }

    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.format_version != LEGAL_STORAGE_FORMAT_VERSION {
            return Err(invalid("unsupported legal storage format"));
        }
        for (object, head) in &self.heads {
            validate_head(head)?;
            if object != &head.object
                || !matches!(
                    self.membership.get(&head.version),
                    Some(LegalArchiveMembership {
                        location: LegalVersionLocation::Hot,
                        ..
                    })
                )
            {
                return Err(invalid("legal hot head or membership is inconsistent"));
            }
        }
        for (version, membership) in &self.membership {
            validate_version(version)?;
            if version != &membership.version {
                return Err(invalid("legal archive membership key is inconsistent"));
            }
            if let LegalVersionLocation::Archived { receipt } = &membership.location {
                validate_archive_receipt(receipt)?;
                if receipt.owner_shard != version.object.home_shard
                    || self.reachability.get(&receipt.object)
                        != Some(&ArchiveReachabilityState::Committed)
                {
                    return Err(invalid("archived legal membership is unreachable"));
                }
                let archive_head =
                    self.archive_heads
                        .get(&receipt.owner_shard)
                        .ok_or_else(|| {
                            invalid("archived legal membership has no committed shard archive head")
                        })?;
                if receipt.archive_batch_sequence == 0
                    || receipt.archive_batch_sequence > archive_head.committed_batch_count
                {
                    return Err(invalid(
                        "archived legal membership has an invalid batch sequence",
                    ));
                }
            }
        }
        for (version, candidate) in &self.compaction_candidates {
            validate_candidate(candidate)?;
            if version != &candidate.version
                || !matches!(
                    self.membership.get(version),
                    Some(LegalArchiveMembership {
                        location: LegalVersionLocation::Hot,
                        ..
                    })
                )
                || self
                    .heads
                    .get(&version.object)
                    .is_some_and(|head| head.version == *version)
            {
                return Err(invalid("legal compaction candidate is inconsistent"));
            }
        }
        for (object, state) in &self.reachability {
            validate_archive_object_id(object)?;
            if *state == ArchiveReachabilityState::Committed
                && !self.membership.values().any(|membership| {
                    matches!(
                        &membership.location,
                        LegalVersionLocation::Archived { receipt }
                            if receipt.object == *object
                    )
                })
            {
                return Err(invalid(
                    "committed archive reachability must be owned by archived membership",
                ));
            }
        }
        for (shard, head) in &self.archive_heads {
            validate_archive_head(head)?;
            if shard != &head.shard {
                return Err(invalid("legal archive head key is inconsistent"));
            }
            let mut archived_receipts = self
                .membership
                .values()
                .filter_map(|membership| match &membership.location {
                    LegalVersionLocation::Archived { receipt }
                        if membership.version.object.home_shard == *shard =>
                    {
                        Some(receipt.as_ref())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            archived_receipts
                .sort_by_key(|receipt| (receipt.archive_batch_sequence, receipt.member_index));
            if archived_receipts
                .first()
                .is_some_and(|receipt| receipt.archive_batch_sequence != 1)
            {
                return Err(invalid("legal archive batch sequences must begin at one"));
            }
            let archived_member_count = u64::try_from(archived_receipts.len())
                .map_err(|_| invalid("legal archive member count exceeds the persistent range"))?;
            let mut expected_batch = 1_u64;
            let mut expected_member = 0_u64;
            let mut batch_binding: Option<(&str, &str)> = None;
            let mut object_bindings = BTreeMap::<ArchiveObjectId, ArchiveObjectReceipt>::new();
            for receipt in &archived_receipts {
                if receipt.archive_batch_sequence == expected_batch {
                    if receipt.member_index != expected_member {
                        return Err(invalid(
                            "legal archive batch member indexes must be contiguous",
                        ));
                    }
                } else if receipt.archive_batch_sequence
                    == expected_batch
                        .checked_add(1)
                        .ok_or_else(|| invalid("legal archive batch sequence is exhausted"))?
                {
                    expected_batch = receipt.archive_batch_sequence;
                    expected_member = 0;
                    batch_binding = None;
                    if receipt.member_index != 0 {
                        return Err(invalid(
                            "legal archive batch member indexes must begin at zero",
                        ));
                    }
                } else {
                    return Err(invalid("legal archive batch sequences must be contiguous"));
                }
                let receipt_binding = (
                    receipt.source_root.as_str(),
                    receipt.verified_plan_hash.as_str(),
                );
                if batch_binding.is_some_and(|binding| binding != receipt_binding) {
                    return Err(invalid(
                        "one legal archive batch must share its source and verification roots",
                    ));
                }
                batch_binding = Some(receipt_binding);
                if let Some(previous) = object_bindings.insert(
                    receipt.object.clone(),
                    ArchiveObjectReceipt {
                        member_index: 0,
                        ..(*receipt).clone()
                    },
                ) {
                    let current = ArchiveObjectReceipt {
                        member_index: 0,
                        ..(*receipt).clone()
                    };
                    if previous != current {
                        return Err(invalid(
                            "one archive object has conflicting persisted receipt metadata",
                        ));
                    }
                }
                expected_member = expected_member
                    .checked_add(1)
                    .ok_or_else(|| invalid("legal archive member index is exhausted"))?;
            }
            let terminal_content_id = archived_receipts
                .last()
                .map(|receipt| &receipt.object.content_id);
            if head.archived_member_count != archived_member_count
                || head.committed_batch_count != expected_batch
                || head.membership_root != self.archived_membership_root_for_shard(shard)?
                || head.last_content_id.as_ref() != terminal_content_id
            {
                return Err(invalid("legal archive head is inconsistent"));
            }
        }
        Ok(())
    }

    fn source_membership_root_for_shard(
        &self,
        shard: &LegalShardKey,
    ) -> Result<String, CanwuError> {
        let members = self
            .membership
            .iter()
            .filter(|(version, _)| version.object.home_shard == *shard)
            .collect::<Vec<_>>();
        canonical_hash(SOURCE_MEMBERSHIP_ROOT_DOMAIN, &(shard, members))
    }

    fn archived_membership_root_for_shard(
        &self,
        shard: &LegalShardKey,
    ) -> Result<String, CanwuError> {
        let members = self
            .membership
            .iter()
            .filter(|(version, membership)| {
                version.object.home_shard == *shard
                    && matches!(membership.location, LegalVersionLocation::Archived { .. })
            })
            .collect::<Vec<_>>();
        canonical_hash(MEMBERSHIP_ROOT_DOMAIN, &(shard, members))
    }
}

fn validate_candidate(candidate: &LegalCompactionCandidate) -> Result<(), CanwuError> {
    validate_version(&candidate.version)?;
    require_identifier(&candidate.record_class, "legal archive record class")?;
    if candidate.encoded_bytes == 0 {
        return Err(invalid(
            "legal compaction candidates must have nonzero bytes",
        ));
    }
    Ok(())
}

fn validate_archive_object_id(object: &ArchiveObjectId) -> Result<(), CanwuError> {
    validate_hash(&object.content_id, "archive content ID")?;
    validate_hash(&object.blob_id, "archive blob ID")
}

fn validate_archive_receipt(receipt: &ArchiveObjectReceipt) -> Result<(), CanwuError> {
    validate_archive_object_id(&receipt.object)?;
    receipt.owner_shard.validate()?;
    if receipt.archive_batch_sequence == 0
        || receipt.stored_bytes == 0
        || receipt.decoded_bytes == 0
    {
        return Err(invalid("archive receipt contains an invalid count"));
    }
    require_identifier(&receipt.codec, "archive codec")?;
    validate_hash(&receipt.source_root, "archive source root")?;
    validate_hash(&receipt.verified_plan_hash, "archive plan hash")
}

fn validate_archive_head(head: &LegalArchiveHead) -> Result<(), CanwuError> {
    head.shard.validate()?;
    if head.committed_batch_count == 0 || head.archived_member_count == 0 {
        return Err(invalid(
            "legal archive heads must describe committed members",
        ));
    }
    validate_hash(&head.membership_root, "legal archive membership root")?;
    if let Some(content_id) = &head.last_content_id {
        validate_hash(content_id, "legal archive last content ID")?;
    }
    Ok(())
}

fn validate_head(head: &LegalHeadRef) -> Result<(), CanwuError> {
    validate_version(&head.version)?;
    if head.object != head.version.object {
        return Err(invalid("legal head object and version object disagree"));
    }
    Ok(())
}

fn validate_version(version: &LegalVersionRef) -> Result<(), CanwuError> {
    version.object.home_shard.validate()?;
    require_identifier(&version.object.id, "legal object ID")?;
    if let Some(discriminator) = &version.object.local_discriminator {
        require_identifier(discriminator, "legal object discriminator")?;
    }
    if version.version_ordinal == 0 {
        return Err(invalid("legal version ordinals must be nonzero"));
    }
    validate_hash(&version.content_commitment, "legal version commitment")
}

fn validate_hash(value: &str, label: &str) -> Result<(), CanwuError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(format!("{label} must be a canonical SHA-256 hash")));
    }
    Ok(())
}

fn require_identifier(value: &str, label: &str) -> Result<(), CanwuError> {
    if value.is_empty()
        || value.trim() != value
        || value.chars().any(char::is_control)
        || value.chars().any(char::is_whitespace)
    {
        return Err(invalid(format!(
            "{label} must be canonical non-whitespace text"
        )));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}

mod ordered_map_serde {
    use super::*;

    pub fn serialize<S, K, V>(map: &BTreeMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        K: Serialize + Ord,
        V: Serialize,
    {
        map.iter().collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, D, K, V>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        D: Deserializer<'de>,
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
    {
        let entries = Vec::<(K, V)>::deserialize(deserializer)?;
        let entry_count = entries.len();
        let map = entries.into_iter().collect::<BTreeMap<_, _>>();
        if map.len() != entry_count {
            return Err(D::Error::custom("ordered map contains duplicate keys"));
        }
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: char) -> String {
        std::iter::repeat_n(byte, 64).collect()
    }

    fn version(id: &str, ordinal: u64) -> LegalVersionRef {
        LegalVersionRef {
            object: LegalObjectId {
                kind: LegalObjectKind::LawVersion,
                id: id.to_owned(),
                home_shard: LegalShardKey::order("order"),
                local_discriminator: None,
            },
            version_ordinal: ordinal,
            content_commitment: hash('a'),
        }
    }

    fn advance_to_durable(state: &mut LegalStorageState, object: &ArchiveObjectId) {
        for reachability in [
            ArchiveReachabilityState::Stored,
            ArchiveReachabilityState::Verified,
            ArchiveReachabilityState::DurableIngress,
        ] {
            state
                .advance_reachability(object.clone(), reachability)
                .unwrap();
        }
    }

    fn receipt(
        prepared: &PreparedLegalCompaction,
        member_index: u64,
        hash_byte: char,
    ) -> ArchiveObjectReceipt {
        ArchiveObjectReceipt {
            object: ArchiveObjectId {
                content_id: hash(hash_byte),
                blob_id: hash(char::from_u32(u32::from(hash_byte) + 1).unwrap()),
            },
            owner_shard: prepared.shard.clone(),
            archive_batch_sequence: prepared.archive_batch_sequence,
            member_index,
            codec: "zstd-fixed".to_owned(),
            stored_bytes: 10,
            decoded_bytes: 20,
            source_root: prepared.source_membership_root.clone(),
            verified_plan_hash: prepared.token.clone(),
        }
    }

    #[test]
    fn compaction_selection_is_deterministic_and_budgeted() {
        let mut state = LegalStorageState::default();
        for (id, ordinal, bytes, closed_at) in [
            ("late", 3, 6, SimTime::from_minutes(30)),
            ("early", 1, 5, SimTime::from_minutes(10)),
            ("middle", 2, 5, SimTime::from_minutes(20)),
        ] {
            let old_version = version(id, ordinal);
            state
                .record_hot_head(LegalHeadRef {
                    object: old_version.object.clone(),
                    version: old_version.clone(),
                })
                .unwrap();
            let replacement = version(&format!("{id}-head"), ordinal + 10);
            state
                .record_hot_head(LegalHeadRef {
                    object: old_version.object.clone(),
                    version: LegalVersionRef {
                        object: old_version.object.clone(),
                        ..replacement
                    },
                })
                .unwrap();
            state
                .mark_compaction_candidate(LegalCompactionCandidate {
                    version: old_version,
                    record_class: "law_version".to_owned(),
                    closed_at,
                    encoded_bytes: bytes,
                    dependencies_resolved: true,
                    current_projection_retained: true,
                })
                .unwrap();
        }

        let prepared = state
            .select_compaction_batch(
                &LegalShardKey::order("order"),
                LegalCompactionBudgets {
                    max_records: 2,
                    max_source_bytes: 10,
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            prepared
                .candidates
                .iter()
                .map(|candidate| candidate.version.object.id.as_str())
                .collect::<Vec<_>>(),
            vec!["early", "middle"]
        );
        assert_eq!(prepared.source_bytes, 10);
        let encoded = serde_json::to_string(&state).unwrap();
        let decoded: LegalStorageState = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, state);
    }

    #[test]
    fn current_heads_cannot_be_archived() {
        let mut state = LegalStorageState::default();
        let version = version("current", 1);
        state
            .record_hot_head(LegalHeadRef {
                object: version.object.clone(),
                version: version.clone(),
            })
            .unwrap();
        let error = state
            .mark_compaction_candidate(LegalCompactionCandidate {
                version,
                record_class: "law_version".to_owned(),
                closed_at: SimTime::EPOCH,
                encoded_bytes: 1,
                dependencies_resolved: true,
                current_projection_retained: true,
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidDomainRecord);
    }

    #[test]
    fn current_heads_cannot_regress_or_reenter_from_candidates() {
        let mut state = LegalStorageState::default();
        let first = version("law", 1);
        state
            .record_hot_head(LegalHeadRef {
                object: first.object.clone(),
                version: first.clone(),
            })
            .unwrap();
        let second = LegalVersionRef {
            object: first.object.clone(),
            version_ordinal: 2,
            content_commitment: hash('d'),
        };
        state
            .record_hot_head(LegalHeadRef {
                object: second.object.clone(),
                version: second,
            })
            .unwrap();
        state
            .mark_compaction_candidate(LegalCompactionCandidate {
                version: first.clone(),
                record_class: "law_version".to_owned(),
                closed_at: SimTime::EPOCH,
                encoded_bytes: 1,
                dependencies_resolved: true,
                current_projection_retained: true,
            })
            .unwrap();

        let error = state
            .record_hot_head(LegalHeadRef {
                object: first.object.clone(),
                version: first,
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidDomainRecord);
    }

    #[test]
    fn rejected_stale_objects_stop_protecting_storage() {
        let mut state = LegalStorageState::default();
        let object = ArchiveObjectId {
            content_id: hash('b'),
            blob_id: hash('c'),
        };
        state
            .advance_reachability(object.clone(), ArchiveReachabilityState::Stored)
            .unwrap();
        state
            .advance_reachability(object.clone(), ArchiveReachabilityState::Verified)
            .unwrap();
        state
            .advance_reachability(object.clone(), ArchiveReachabilityState::DurableIngress)
            .unwrap();
        assert!(state.reachable_archive_object_ids().contains(&object));
        state
            .advance_reachability(object.clone(), ArchiveReachabilityState::RejectedStale)
            .unwrap();
        assert!(!state.reachable_archive_object_ids().contains(&object));
    }

    #[test]
    fn invalid_late_receipt_leaves_compaction_state_unchanged() {
        let mut state = LegalStorageState::default();
        for (id, ordinal) in [("first", 1), ("second", 2)] {
            let old_version = version(id, ordinal);
            state
                .record_hot_head(LegalHeadRef {
                    object: old_version.object.clone(),
                    version: old_version.clone(),
                })
                .unwrap();
            state
                .record_hot_head(LegalHeadRef {
                    object: old_version.object.clone(),
                    version: LegalVersionRef {
                        object: old_version.object.clone(),
                        version_ordinal: ordinal + 10,
                        content_commitment: hash('d'),
                    },
                })
                .unwrap();
            state
                .mark_compaction_candidate(LegalCompactionCandidate {
                    version: old_version,
                    record_class: "law_version".to_owned(),
                    closed_at: SimTime::from_minutes(i64::try_from(ordinal).unwrap()),
                    encoded_bytes: 10,
                    dependencies_resolved: true,
                    current_projection_retained: true,
                })
                .unwrap();
        }
        let prepared = state
            .select_compaction_batch(
                &LegalShardKey::order("order"),
                LegalCompactionBudgets {
                    max_records: 2,
                    max_source_bytes: 20,
                },
            )
            .unwrap()
            .unwrap();
        let mut receipts = [receipt(&prepared, 0, 'b'), receipt(&prepared, 1, 'd')].to_vec();
        for receipt in &receipts {
            advance_to_durable(&mut state, &receipt.object);
        }
        receipts[1].member_index = 99;
        let before = state.clone();

        assert!(state.commit_compaction(&prepared, receipts).is_err());
        assert_eq!(state, before);
    }

    #[test]
    fn verified_durable_batch_commits_atomically_and_validates() {
        let mut state = LegalStorageState::default();
        let old_version = version("law", 1);
        state
            .record_hot_head(LegalHeadRef {
                object: old_version.object.clone(),
                version: old_version.clone(),
            })
            .unwrap();
        state
            .record_hot_head(LegalHeadRef {
                object: old_version.object.clone(),
                version: LegalVersionRef {
                    object: old_version.object.clone(),
                    version_ordinal: 2,
                    content_commitment: hash('d'),
                },
            })
            .unwrap();
        let second_version = state
            .heads
            .get(&old_version.object)
            .unwrap()
            .version
            .clone();
        state
            .mark_compaction_candidate(LegalCompactionCandidate {
                version: old_version.clone(),
                record_class: "law_version".to_owned(),
                closed_at: SimTime::EPOCH,
                encoded_bytes: 10,
                dependencies_resolved: true,
                current_projection_retained: true,
            })
            .unwrap();
        let prepared = state
            .select_compaction_batch(
                &LegalShardKey::order("order"),
                LegalCompactionBudgets {
                    max_records: 1,
                    max_source_bytes: 10,
                },
            )
            .unwrap()
            .unwrap();
        let first_receipt = receipt(&prepared, 0, 'b');
        advance_to_durable(&mut state, &first_receipt.object);

        state
            .commit_compaction(&prepared, vec![first_receipt.clone()])
            .unwrap();

        assert!(matches!(
            state.membership.get(&old_version),
            Some(LegalArchiveMembership {
                location: LegalVersionLocation::Archived { .. },
                ..
            })
        ));
        assert_eq!(
            state.reachability.get(&first_receipt.object),
            Some(&ArchiveReachabilityState::Committed)
        );
        state
            .record_hot_head(LegalHeadRef {
                object: old_version.object.clone(),
                version: LegalVersionRef {
                    object: old_version.object.clone(),
                    version_ordinal: 3,
                    content_commitment: hash('e'),
                },
            })
            .unwrap();
        state
            .mark_compaction_candidate(LegalCompactionCandidate {
                version: second_version,
                record_class: "law_version".to_owned(),
                closed_at: SimTime::from_minutes(1),
                encoded_bytes: 10,
                dependencies_resolved: true,
                current_projection_retained: true,
            })
            .unwrap();
        let second_prepared = state
            .select_compaction_batch(
                &LegalShardKey::order("order"),
                LegalCompactionBudgets {
                    max_records: 1,
                    max_source_bytes: 10,
                },
            )
            .unwrap()
            .unwrap();
        let second_receipt = receipt(&second_prepared, 0, 'd');
        advance_to_durable(&mut state, &second_receipt.object);
        state
            .commit_compaction(&second_prepared, vec![second_receipt])
            .unwrap();
        state.validate().unwrap();

        let shard = LegalShardKey::order("order");
        let mut inflated_sequence = state.clone();
        inflated_sequence
            .archive_heads
            .get_mut(&shard)
            .unwrap()
            .committed_batch_count = 999;
        assert!(inflated_sequence.validate().is_err());

        let mut stale_terminal = state.clone();
        stale_terminal
            .archive_heads
            .get_mut(&shard)
            .unwrap()
            .last_content_id = Some(first_receipt.object.content_id.clone());
        assert!(stale_terminal.validate().is_err());

        let encoded = serde_json::to_string(&state).unwrap();
        let decoded: LegalStorageState = serde_json::from_str(&encoded).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded, state);
    }

    #[test]
    fn one_verified_segment_can_commit_multiple_members() {
        let mut state = LegalStorageState::default();
        for (id, ordinal) in [("first", 1), ("second", 2)] {
            let old_version = version(id, ordinal);
            state
                .record_hot_head(LegalHeadRef {
                    object: old_version.object.clone(),
                    version: old_version.clone(),
                })
                .unwrap();
            state
                .record_hot_head(LegalHeadRef {
                    object: old_version.object.clone(),
                    version: LegalVersionRef {
                        object: old_version.object.clone(),
                        version_ordinal: ordinal + 10,
                        content_commitment: hash('d'),
                    },
                })
                .unwrap();
            state
                .mark_compaction_candidate(LegalCompactionCandidate {
                    version: old_version,
                    record_class: "law_version".to_owned(),
                    closed_at: SimTime::from_minutes(i64::try_from(ordinal).unwrap()),
                    encoded_bytes: 10,
                    dependencies_resolved: true,
                    current_projection_retained: true,
                })
                .unwrap();
        }
        let prepared = state
            .select_compaction_batch(
                &LegalShardKey::order("order"),
                LegalCompactionBudgets {
                    max_records: 2,
                    max_source_bytes: 20,
                },
            )
            .unwrap()
            .unwrap();
        let shared_object = ArchiveObjectId {
            content_id: hash('b'),
            blob_id: hash('c'),
        };
        advance_to_durable(&mut state, &shared_object);
        let receipts = (0_u64..2)
            .map(|member_index| ArchiveObjectReceipt {
                object: shared_object.clone(),
                owner_shard: prepared.shard.clone(),
                archive_batch_sequence: prepared.archive_batch_sequence,
                member_index,
                codec: "zstd-fixed".to_owned(),
                stored_bytes: 10,
                decoded_bytes: 20,
                source_root: prepared.source_membership_root.clone(),
                verified_plan_hash: prepared.token.clone(),
            })
            .collect::<Vec<_>>();

        state.commit_compaction(&prepared, receipts).unwrap();

        assert_eq!(
            state.reachability.get(&shared_object),
            Some(&ArchiveReachabilityState::Committed)
        );
        assert_eq!(
            state
                .membership
                .values()
                .filter(|membership| matches!(
                    membership.location,
                    LegalVersionLocation::Archived { .. }
                ))
                .count(),
            2
        );
        state.validate().unwrap();

        for tampered_index in [0_u64, 2] {
            let mut tampered = state.clone();
            let membership = tampered
                .membership
                .values_mut()
                .filter(|membership| {
                    matches!(membership.location, LegalVersionLocation::Archived { .. })
                })
                .nth(1)
                .unwrap();
            let LegalVersionLocation::Archived { receipt } = &mut membership.location else {
                unreachable!();
            };
            receipt.member_index = tampered_index;
            let root = tampered
                .archived_membership_root_for_shard(&prepared.shard)
                .unwrap();
            tampered
                .archive_heads
                .get_mut(&prepared.shard)
                .unwrap()
                .membership_root = root;
            assert!(tampered.validate().is_err());
        }
    }
}
