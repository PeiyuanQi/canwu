use canwu_api::{CanwuError, ErrorCode, SimTime, canonical_hash};
use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub const LEGAL_STORAGE_FORMAT_VERSION: u32 = 2;
pub const LEGAL_SHARD_PAYLOAD_FORMAT_VERSION: u32 = 1;
pub const LEGAL_ARCHIVE_BLOB_FORMAT_VERSION: u32 = 1;
pub const LEGAL_ARCHIVE_INDEX_FORMAT_VERSION: u32 = 2;
pub const LEGAL_ARCHIVE_RETENTION_FORMAT_VERSION: u32 = 1;
pub const LEGAL_ARCHIVE_INDEX_BUCKET_COUNT: u32 = 65_536;
pub const MAX_LEGAL_ARCHIVE_PAGE_ENTRIES: usize = 64;
pub const MAX_LEGAL_ARCHIVE_PAGE_BYTES: usize = 1024 * 1024;
pub const MAX_LEGAL_ARCHIVE_DIRECTORY_BYTES: usize = 64 * 1024 * 1024;
pub const LEGAL_ARCHIVE_BLOB_NAMESPACE: &str = "canwu.law.archive.blob";
pub const LEGAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE: &str = "canwu.law.archive.index.directory";
pub const LEGAL_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE: &str = "canwu.law.archive.index.membership";
pub const LEGAL_ARCHIVE_TEMPORAL_PAGE_NAMESPACE: &str = "canwu.law.archive.index.temporal";
const COMPACTION_TOKEN_DOMAIN: &str = "canwu.law.compaction-token.v1";
const SOURCE_MEMBERSHIP_ROOT_DOMAIN: &str = "canwu.law.source-membership.v1";
const MEMBERSHIP_ROOT_DOMAIN: &str = "canwu.law.archive-membership.v1";
const EFFECTIVE_TIME_ROOT_DOMAIN: &str = "canwu.law.archive-effective-time.v1";
const RECORDED_TIME_ROOT_DOMAIN: &str = "canwu.law.archive-recorded-time.v1";
const MEMBERSHIP_APPEND_ROOT_DOMAIN: &str = "canwu.law.archive-membership-append.v1";
const EFFECTIVE_APPEND_ROOT_DOMAIN: &str = "canwu.law.archive-effective-append.v1";
const RECORDED_APPEND_ROOT_DOMAIN: &str = "canwu.law.archive-recorded-append.v1";

pub const LEGAL_TEMPORAL_WIDTH: u8 = 64;
pub const MAX_LEGAL_TEMPORAL_CELLS_PER_INTERVAL: usize = 128;
pub const MAX_LEGAL_TEMPORAL_QUERY_CANDIDATES: usize = 4_096;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalTimeInterval {
    pub start: SimTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_exclusive: Option<SimTime>,
}

impl LegalTimeInterval {
    pub fn validate(self) -> Result<(), CanwuError> {
        if self.end_exclusive.is_some_and(|end| end <= self.start) {
            return Err(invalid(
                "legal temporal interval end must be later than its start",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalDyadicCell {
    pub prefix_bits: u64,
    pub prefix_length: u8,
}

impl LegalDyadicCell {
    fn validate(self) -> Result<(), CanwuError> {
        if self.prefix_length > LEGAL_TEMPORAL_WIDTH
            || self.prefix_bits != masked_prefix(self.prefix_bits, self.prefix_length)
        {
            return Err(invalid("legal dyadic cell is not canonical"));
        }
        Ok(())
    }

    #[must_use]
    pub fn contains(self, time: SimTime) -> bool {
        self.prefix_bits == masked_prefix(encode_time(time), self.prefix_length)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalTemporalQueryBudget {
    pub max_candidates_per_dimension: usize,
    pub max_intersection_members: usize,
    pub max_provider_calls: usize,
    pub max_segments: usize,
    pub max_decoded_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalTemporalQueryUsage {
    pub provider_calls: usize,
    pub segments: usize,
    pub decoded_bytes: u64,
}

impl Default for LegalTemporalQueryBudget {
    fn default() -> Self {
        Self {
            max_candidates_per_dimension: 1_024,
            max_intersection_members: 512,
            max_provider_calls: 256,
            max_segments: 255,
            max_decoded_bytes: 256 * 1024 * 1024,
        }
    }
}

#[derive(Default)]
pub(crate) struct LegalTemporalQueryMeter {
    provider_calls: usize,
    segments: usize,
    decoded_bytes: u64,
}

impl LegalTemporalQueryMeter {
    pub(crate) fn validate_budget(budget: LegalTemporalQueryBudget) -> Result<(), CanwuError> {
        if budget.max_candidates_per_dimension == 0
            || budget.max_candidates_per_dimension > MAX_LEGAL_TEMPORAL_QUERY_CANDIDATES
            || budget.max_intersection_members == 0
            || budget.max_provider_calls == 0
            || budget.max_segments == 0
            || budget.max_decoded_bytes == 0
        {
            return Err(query_budget_error(
                "legal temporal provider budget is invalid",
            ));
        }
        Ok(())
    }

    pub(crate) fn begin_provider_call(
        &mut self,
        budget: LegalTemporalQueryBudget,
        segment: bool,
    ) -> Result<(), CanwuError> {
        if self.provider_calls == budget.max_provider_calls
            || (segment && self.segments == budget.max_segments)
        {
            return Err(query_budget_error(
                "legal temporal provider query exceeded its I/O budget",
            ));
        }
        self.provider_calls += 1;
        if segment {
            self.segments += 1;
        }
        Ok(())
    }

    pub(crate) fn record_decoded<T: Serialize>(
        &mut self,
        value: &T,
        budget: LegalTemporalQueryBudget,
    ) -> Result<(), CanwuError> {
        let decoded = u64::try_from(
            serde_json::to_vec(value)
                .map_err(|error| invalid(format!("legal query page cannot be sized: {error}")))?
                .len(),
        )
        .map_err(|_| query_budget_error("legal query decoded byte count exceeds u64"))?;
        self.decoded_bytes = self
            .decoded_bytes
            .checked_add(decoded)
            .ok_or_else(|| query_budget_error("legal query decoded byte count overflowed"))?;
        if self.decoded_bytes > budget.max_decoded_bytes {
            return Err(query_budget_error(
                "legal temporal provider query exceeded its decoded-byte budget",
            ));
        }
        Ok(())
    }

    pub(crate) const fn usage(&self) -> LegalTemporalQueryUsage {
        LegalTemporalQueryUsage {
            provider_calls: self.provider_calls,
            segments: self.segments,
            decoded_bytes: self.decoded_bytes,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalTemporalIndex {
    #[serde(default)]
    buckets: BTreeMap<LegalDyadicCell, BTreeMap<LegalVersionRef, String>>,
}

impl LegalTemporalIndex {
    pub fn insert(
        &mut self,
        interval: LegalTimeInterval,
        version: &LegalVersionRef,
        primary_member_commitment: &str,
    ) -> Result<usize, CanwuError> {
        interval.validate()?;
        validate_version(version)?;
        validate_hash(
            primary_member_commitment,
            "legal temporal primary member commitment",
        )?;
        let cells = decompose_legal_time_interval(interval)?;
        for cell in &cells {
            let previous = self
                .buckets
                .entry(*cell)
                .or_default()
                .insert(version.clone(), primary_member_commitment.to_owned());
            if previous.is_some_and(|previous| previous != primary_member_commitment) {
                return Err(invalid(
                    "legal temporal member was rebound to a different primary commitment",
                ));
            }
        }
        Ok(cells.len())
    }

    pub fn remove(
        &mut self,
        interval: LegalTimeInterval,
        version: &LegalVersionRef,
    ) -> Result<(), CanwuError> {
        for cell in decompose_legal_time_interval(interval)? {
            let bucket = self
                .buckets
                .get_mut(&cell)
                .ok_or_else(|| invalid("legal temporal removal omitted a canonical cell"))?;
            if bucket.remove(version).is_none() {
                return Err(invalid("legal temporal removal omitted its exact member"));
            }
            if bucket.is_empty() {
                self.buckets.remove(&cell);
            }
        }
        Ok(())
    }

    pub fn point_candidates(
        &self,
        time: SimTime,
        max_candidates: usize,
    ) -> Result<BTreeMap<LegalVersionRef, String>, CanwuError> {
        if max_candidates == 0 || max_candidates > MAX_LEGAL_TEMPORAL_QUERY_CANDIDATES {
            return Err(query_budget_error(
                "legal temporal candidate budget is zero or exceeds the hard limit",
            ));
        }
        let encoded = encode_time(time);
        let mut candidates = BTreeMap::new();
        for prefix_length in 0..=LEGAL_TEMPORAL_WIDTH {
            let cell = LegalDyadicCell {
                prefix_bits: masked_prefix(encoded, prefix_length),
                prefix_length,
            };
            if let Some(bucket) = self.buckets.get(&cell) {
                for (version, commitment) in bucket {
                    if candidates.len() == max_candidates && !candidates.contains_key(version) {
                        return Err(query_budget_error(
                            "legal temporal point query exceeded its candidate budget",
                        ));
                    }
                    if candidates
                        .insert(version.clone(), commitment.clone())
                        .is_some_and(|previous| previous != *commitment)
                    {
                        return Err(invalid(
                            "legal temporal index disagrees about a primary member",
                        ));
                    }
                }
            }
        }
        Ok(candidates)
    }

    pub fn root(&self, domain: &str) -> Result<String, CanwuError> {
        let entries = self
            .buckets
            .iter()
            .flat_map(|(cell, bucket)| {
                bucket
                    .iter()
                    .map(move |(version, commitment)| (cell, version, commitment))
            })
            .collect::<Vec<_>>();
        canonical_hash(domain, &entries)
    }

    #[must_use]
    pub fn cell_count(&self) -> usize {
        self.buckets.len()
    }

    #[must_use]
    pub fn bucket_entry_count(&self) -> usize {
        self.buckets.values().map(BTreeMap::len).sum()
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalBitemporalIndex {
    pub effective: LegalTemporalIndex,
    pub recorded: LegalTemporalIndex,
}

impl LegalBitemporalIndex {
    pub fn insert(
        &mut self,
        effective: LegalTimeInterval,
        recorded: LegalTimeInterval,
        version: &LegalVersionRef,
        primary_member_commitment: &str,
    ) -> Result<(usize, usize), CanwuError> {
        let effective_cells =
            self.effective
                .insert(effective, version, primary_member_commitment)?;
        let recorded_cells =
            match self
                .recorded
                .insert(recorded, version, primary_member_commitment)
            {
                Ok(cells) => cells,
                Err(error) => {
                    self.effective.remove(effective, version)?;
                    return Err(error);
                }
            };
        Ok((effective_cells, recorded_cells))
    }

    pub fn point_query(
        &self,
        effective_at: SimTime,
        recorded_at: SimTime,
        budget: LegalTemporalQueryBudget,
    ) -> Result<Vec<LegalVersionRef>, CanwuError> {
        if budget.max_intersection_members == 0 {
            return Err(query_budget_error(
                "legal temporal intersection budget must be nonzero",
            ));
        }
        let effective = self
            .effective
            .point_candidates(effective_at, budget.max_candidates_per_dimension)?;
        let recorded = self
            .recorded
            .point_candidates(recorded_at, budget.max_candidates_per_dimension)?;
        let mut result = Vec::new();
        for (version, commitment) in effective {
            if recorded.get(&version) == Some(&commitment) {
                if result.len() == budget.max_intersection_members {
                    return Err(query_budget_error(
                        "legal bitemporal query exceeded its intersection budget",
                    ));
                }
                result.push(version);
            }
        }
        Ok(result)
    }

    pub fn roots(&self) -> Result<(String, String), CanwuError> {
        Ok((
            self.effective.root(EFFECTIVE_TIME_ROOT_DOMAIN)?,
            self.recorded.root(RECORDED_TIME_ROOT_DOMAIN)?,
        ))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalTemporalScaleMetrics {
    pub source_versions: u64,
    pub dyadic_cells: u64,
    pub bucket_entries: u64,
    pub membership_pages: u64,
    pub effective_pages: u64,
    pub recorded_pages: u64,
    pub max_membership_page_entries: u64,
    pub max_membership_page_encoded_bytes: u64,
    pub max_temporal_page_entries: u64,
    pub max_temporal_page_encoded_bytes: u64,
    pub max_interval_expansion: u64,
    pub estimated_resident_structural_bytes: u64,
    pub point_query_samples: u64,
    pub point_query_elapsed_micros: u64,
    pub point_query_max_candidates: u64,
    pub point_query_max_provider_calls: u64,
    pub point_query_max_segments: u64,
    pub point_query_max_decoded_bytes: u64,
    pub archive_batches: u64,
    pub peak_hot_compaction_candidates: u64,
    pub max_archive_batch_elapsed_micros: u64,
    pub exact_restart_queries: u64,
    pub reachable_archive_objects: u64,
    pub canonical_ingress_retention_roots: u64,
    pub provider_backing_store_bytes: u64,
    pub provider_index_entries: u64,
    pub retention_handles: u64,
    pub retention_committed_roots: u64,
    pub retention_committed_objects: u64,
    pub retention_terminal_payload_items: u64,
    pub root_hash: String,
    pub archive_head: LegalArchiveHead,
}

pub fn format8_legal_temporal_scale_probe(
    version_count: usize,
) -> Result<LegalTemporalScaleMetrics, CanwuError> {
    if version_count == 0 {
        return Err(invalid(
            "legal temporal scale probe requires at least one source version",
        ));
    }
    let shard = LegalShardKey::culture_dependency("format8-scale");
    let provider = LegalTemporalScaleProvider::try_new()?;
    let mut storage = LegalStorageState::default();
    storage.directory.active_shards.insert(shard.clone());
    let mut max_interval_expansion = 0_u64;
    let mut archive_batches = 0_u64;
    let mut peak_hot_compaction_candidates = 0_u64;
    let mut max_archive_batch_elapsed_micros = 0_u64;
    let mut canonical_ingress_retention_roots = 0_u64;
    for chunk_start in (1..=version_count).step_by(4_096) {
        let batch_started = Instant::now();
        let chunk_end = chunk_start.saturating_add(4_096).min(version_count + 1);
        let mut pending_blobs = BTreeMap::<LegalVersionRef, LegalArchiveBlob>::new();
        for ordinal in chunk_start..chunk_end {
            let ordinal = u64::try_from(ordinal)
                .map_err(|_| invalid("legal temporal scale key exceeds u64"))?;
            let (candidate, blob) = format8_legal_scale_candidate(ordinal, &shard)?;
            let (effective, recorded) = legal_archive_intervals(&candidate, &blob)?;
            max_interval_expansion = max_interval_expansion.max(
                decompose_legal_time_interval(effective)?
                    .len()
                    .max(decompose_legal_time_interval(recorded)?.len()) as u64,
            );
            storage.membership.insert(
                candidate.version.clone(),
                LegalArchiveMembership {
                    version: candidate.version.clone(),
                    location: LegalVersionLocation::Hot,
                    effective_interval: None,
                    recorded_interval: None,
                },
            );
            storage.mark_compaction_candidate(candidate.clone())?;
            pending_blobs.insert(candidate.version, blob);
        }
        peak_hot_compaction_candidates = peak_hot_compaction_candidates.max(
            u64::try_from(storage.compaction_candidates.len())
                .map_err(|_| invalid("legal scale hot candidate count exceeds u64"))?,
        );
        let compaction = storage
            .select_compaction_batch(
                &shard,
                LegalCompactionBudgets {
                    max_records: 4_096,
                    max_source_bytes: u64::MAX,
                },
            )?
            .ok_or_else(|| invalid("legal scale chunk produced no compaction batch"))?;
        if compaction.candidates.len() != pending_blobs.len() {
            return Err(invalid(
                "legal scale chunk did not fit one bounded compaction batch",
            ));
        }
        let batch_blobs = compaction
            .candidates
            .iter()
            .map(|candidate| {
                pending_blobs
                    .get(&candidate.version)
                    .cloned()
                    .ok_or_else(|| invalid("legal scale candidate lost its blob"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let receipts = batch_blobs
            .iter()
            .enumerate()
            .map(|(index, blob)| {
                let bytes = u64::try_from(
                    serde_json::to_vec(blob)
                        .map_err(|error| {
                            invalid(format!("legal scale blob cannot be encoded: {error}"))
                        })?
                        .len(),
                )
                .map_err(|_| invalid("legal scale blob exceeds u64"))?;
                Ok(ArchiveObjectReceipt {
                    object: ArchiveObjectId {
                        content_id: blob.content_id()?,
                        blob_id: blob.blob_id()?,
                    },
                    owner_shard: shard.clone(),
                    archive_batch_sequence: compaction.archive_batch_sequence,
                    member_index: u64::try_from(index)
                        .map_err(|_| invalid("legal scale member index exceeds u64"))?,
                    codec: "json-canonical-v1".to_owned(),
                    stored_bytes: bytes,
                    decoded_bytes: bytes,
                    source_root: compaction.source_membership_root.clone(),
                    verified_plan_hash: compaction.token.clone(),
                })
            })
            .collect::<Result<Vec<_>, CanwuError>>()?;
        let prepared = PreparedLegalArchiveBatch {
            format_version: LEGAL_ARCHIVE_BLOB_FORMAT_VERSION,
            compaction: compaction.clone(),
            blobs: batch_blobs,
            receipts,
            previous_archive_head: storage.archive_heads.get(&shard).cloned(),
        };
        let verified = prepared.store_and_verify(&provider)?;
        if verified.pending_reachability.directory_root != verified.archive_head.membership_root {
            return Err(invalid(
                "legal scale pending ingress root disagrees with the authenticated directory",
            ));
        }
        let retention = crate::plugin::legal_archive_ingress_retention(&verified);
        canonical_ingress_retention_roots = canonical_ingress_retention_roots.max(
            u64::try_from(retention.len())
                .map_err(|_| invalid("legal scale retention count exceeds u64"))?,
        );
        for receipt in &verified.receipts {
            storage
                .advance_reachability(receipt.object.clone(), ArchiveReachabilityState::Stored)?;
            storage
                .advance_reachability(receipt.object.clone(), ArchiveReachabilityState::Verified)?;
            storage.advance_reachability(
                receipt.object.clone(),
                ArchiveReachabilityState::DurableIngress,
            )?;
        }
        provider.mark_legal_archive_retention_durable(&verified.retention_handle_id)?;
        storage.commit_compaction(&verified.compaction, verified.receipts.clone())?;
        storage
            .archive_heads
            .insert(shard.clone(), verified.archive_head.clone());
        provider.commit_legal_archive_retention(
            &verified.retention_handle_id,
            &verified.archive_head.membership_root,
        )?;
        provider.finish_committed_head(&verified.archive_head)?;
        storage.membership.retain(|_, membership| {
            !matches!(membership.location, LegalVersionLocation::Archived { .. })
        });
        storage.archived_membership_materialized = false;
        storage
            .reachability
            .retain(|_, state| *state != ArchiveReachabilityState::Committed);
        archive_batches = archive_batches
            .checked_add(1)
            .ok_or_else(|| invalid("legal scale archive batch count overflowed"))?;
        max_archive_batch_elapsed_micros = max_archive_batch_elapsed_micros.max(
            u64::try_from(batch_started.elapsed().as_micros())
                .map_err(|_| invalid("legal scale batch duration exceeds u64"))?,
        );
    }
    let archive_head = storage
        .archive_heads
        .get(&shard)
        .cloned()
        .ok_or_else(|| invalid("legal scale archive head is missing"))?;
    let directory = provider
        .load_legal_archive_index_directory(&archive_head.membership_root)?
        .ok_or_else(|| invalid("legal scale archive directory is unavailable"))?;
    let directory_id = directory.directory_id()?;
    let root_only = storage;
    root_only.validate()?;
    root_only.authenticated_archive_directory(&shard, &provider)?;

    let samples = [0_usize, version_count / 2, version_count - 1]
        .into_iter()
        .collect::<BTreeSet<_>>();
    for index in &samples {
        let (_, expected_blob) = format8_legal_scale_candidate(
            u64::try_from(index.saturating_add(1))
                .map_err(|_| invalid("legal temporal sample key exceeds u64"))?,
            &shard,
        )?;
        let membership = root_only
            .authenticated_archive_membership(&expected_blob.version, &provider)?
            .ok_or_else(|| invalid("root-only legal restart lost exact archived membership"))?;
        let LegalVersionLocation::Archived { receipt } = membership.location else {
            return Err(invalid(
                "root-only legal restart returned hot archive membership",
            ));
        };
        if provider.load_legal_archive(&receipt.object.blob_id)? != Some(expected_blob) {
            return Err(invalid(
                "root-only legal restart lost exact archived payload",
            ));
        }
    }
    let reachability = root_only.archive_reachability_with_provider(&provider)?;
    let reachable_archive_objects = reachability.objects.len();
    if reachable_archive_objects != version_count {
        return Err(invalid(
            "root-only legal GC reachability omitted archived objects",
        ));
    }
    drop(reachability);

    let mut max_membership_page_entries = 0_u64;
    let mut max_membership_page_encoded_bytes = 0_u64;
    for page_id in directory.membership_pages.values() {
        let page = provider
            .load_legal_archive_membership_page(page_id)?
            .ok_or_else(|| invalid("legal scale membership page is unavailable"))?;
        max_membership_page_entries =
            max_membership_page_entries.max(page.memberships.len() as u64);
        let encoded_bytes = serde_json::to_vec(&page)
            .map_err(|error| {
                invalid(format!(
                    "legal scale membership page cannot be sized: {error}"
                ))
            })?
            .len() as u64;
        max_membership_page_encoded_bytes = max_membership_page_encoded_bytes.max(encoded_bytes);
    }

    let mut max_temporal_page_entries = 0_u64;
    let mut max_temporal_page_encoded_bytes = 0_u64;
    let mut bucket_entries = 0_usize;
    let mut dyadic_cells = BTreeSet::new();
    for page_id in directory.effective_pages.values().flatten() {
        let page = provider
            .load_legal_archive_temporal_page(page_id)?
            .ok_or_else(|| invalid("legal scale effective page is unavailable"))?;
        max_temporal_page_entries = max_temporal_page_entries.max(page.entries.len() as u64);
        let encoded_bytes = serde_json::to_vec(&page)
            .map_err(|error| {
                invalid(format!(
                    "legal scale temporal page cannot be sized: {error}"
                ))
            })?
            .len() as u64;
        max_temporal_page_encoded_bytes = max_temporal_page_encoded_bytes.max(encoded_bytes);
        bucket_entries = bucket_entries.saturating_add(page.entries.len());
        dyadic_cells.extend(page.entries.iter().map(|entry| entry.cell));
    }
    for page_id in directory.recorded_pages.values().flatten() {
        let page = provider
            .load_legal_archive_temporal_page(page_id)?
            .ok_or_else(|| invalid("legal scale recorded page is unavailable"))?;
        max_temporal_page_entries = max_temporal_page_entries.max(page.entries.len() as u64);
        let encoded_bytes = serde_json::to_vec(&page)
            .map_err(|error| {
                invalid(format!(
                    "legal scale temporal page cannot be sized: {error}"
                ))
            })?
            .len() as u64;
        max_temporal_page_encoded_bytes = max_temporal_page_encoded_bytes.max(encoded_bytes);
    }
    let dyadic_cells = dyadic_cells.len();
    let query_samples = version_count.min(100);
    let query_started = Instant::now();
    let mut point_query_max_candidates = 0_u64;
    let mut point_query_max_provider_calls = 0_u64;
    let mut point_query_max_segments = 0_u64;
    let mut point_query_max_decoded_bytes = 0_u64;
    for sample in 0..query_samples {
        let ordinal = sample
            .saturating_mul(version_count)
            .checked_div(query_samples)
            .unwrap_or(0);
        let at = SimTime::from_minutes(
            i64::try_from(ordinal)
                .map_err(|_| invalid("legal temporal query sample exceeds i64"))?,
        );
        let (candidates, usage) = root_only.archived_versions_at_with_provider_usage(
            &shard,
            at,
            at,
            LegalTemporalQueryBudget {
                max_candidates_per_dimension: MAX_LEGAL_TEMPORAL_QUERY_CANDIDATES,
                max_intersection_members: MAX_LEGAL_TEMPORAL_QUERY_CANDIDATES,
                ..LegalTemporalQueryBudget::default()
            },
            &provider,
        )?;
        point_query_max_candidates = point_query_max_candidates.max(candidates.len() as u64);
        point_query_max_provider_calls =
            point_query_max_provider_calls.max(usage.provider_calls as u64);
        point_query_max_segments = point_query_max_segments.max(usage.segments as u64);
        point_query_max_decoded_bytes = point_query_max_decoded_bytes.max(usage.decoded_bytes);
    }
    let point_query_elapsed_micros = query_started.elapsed().as_micros() as u64;
    let entry_bytes =
        size_of::<LegalArchiveTemporalEntry>().saturating_add(size_of::<LegalArchiveMembership>());
    let (provider_backing_store_bytes, provider_index_entries) =
        provider.backing_store_metrics()?;
    let (
        retention_handles,
        retention_committed_roots,
        retention_committed_objects,
        retention_terminal_payload_items,
    ) = provider.retention_metrics()?;
    Ok(LegalTemporalScaleMetrics {
        source_versions: version_count as u64,
        dyadic_cells: dyadic_cells as u64,
        bucket_entries: bucket_entries as u64,
        membership_pages: directory.membership_pages.len() as u64,
        effective_pages: directory
            .effective_pages
            .values()
            .map(Vec::len)
            .sum::<usize>() as u64,
        recorded_pages: directory
            .recorded_pages
            .values()
            .map(Vec::len)
            .sum::<usize>() as u64,
        max_membership_page_entries,
        max_membership_page_encoded_bytes,
        max_temporal_page_entries,
        max_temporal_page_encoded_bytes,
        max_interval_expansion,
        estimated_resident_structural_bytes: (bucket_entries as u64)
            .saturating_mul(entry_bytes as u64),
        point_query_samples: query_samples as u64,
        point_query_elapsed_micros,
        point_query_max_candidates,
        point_query_max_provider_calls,
        point_query_max_segments,
        point_query_max_decoded_bytes,
        archive_batches,
        peak_hot_compaction_candidates,
        max_archive_batch_elapsed_micros,
        exact_restart_queries: samples.len() as u64,
        reachable_archive_objects: reachable_archive_objects as u64,
        canonical_ingress_retention_roots,
        provider_backing_store_bytes,
        provider_index_entries,
        retention_handles,
        retention_committed_roots,
        retention_committed_objects,
        retention_terminal_payload_items,
        root_hash: directory_id,
        archive_head,
    })
}

fn format8_legal_scale_candidate(
    ordinal: u64,
    shard: &LegalShardKey,
) -> Result<(LegalCompactionCandidate, LegalArchiveBlob), CanwuError> {
    let at = SimTime::from_minutes(
        i64::try_from(ordinal.saturating_sub(1))
            .map_err(|_| invalid("legal temporal scale time exceeds i64"))?,
    );
    let retirement = crate::LegalRetirement {
        id: format!("retirement:format8-scale:{ordinal}"),
        kind: "culture_generation".to_owned(),
        record: canwu_api::DomainRecordRef::new(
            "canwu.culture",
            "target",
            format!("format8-scale:{ordinal}"),
        ),
        cultural_target: Some(crate::CulturalTargetGenerationRef {
            target: format!("format8-scale:{ordinal}"),
            generation: 1,
        }),
        retired_at: at,
        successor: None,
        reason: "Format-8 production-path legal archive scale record".to_owned(),
        evidence: Vec::new(),
    };
    let payload = serde_json::to_value(&retirement)
        .map_err(|error| invalid(format!("legal scale record cannot be encoded: {error}")))?;
    let version = LegalVersionRef {
        object: LegalObjectId {
            kind: LegalObjectKind::Retirement,
            id: retirement.id.clone(),
            home_shard: shard.clone(),
            local_discriminator: retirement
                .cultural_target
                .as_ref()
                .map(|target| format!("{}@{}", target.target, target.generation)),
        },
        version_ordinal: 1,
        content_commitment: legal_archive_content_commitment("retirement", &payload)?,
    };
    let blob = LegalArchiveBlob {
        format_version: LEGAL_ARCHIVE_BLOB_FORMAT_VERSION,
        version: version.clone(),
        record_class: "retirement".to_owned(),
        payload,
    };
    let encoded_bytes = u64::try_from(
        serde_json::to_vec(&blob)
            .map_err(|error| invalid(format!("legal scale blob cannot be sized: {error}")))?
            .len(),
    )
    .map_err(|_| invalid("legal scale blob exceeds u64"))?;
    Ok((
        LegalCompactionCandidate {
            version,
            record_class: "retirement".to_owned(),
            closed_at: at,
            encoded_bytes,
            dependencies_resolved: true,
            current_projection_retained: true,
        },
        blob,
    ))
}

static NEXT_LEGAL_SCALE_STORE_ID: AtomicU64 = AtomicU64::new(1);

struct ScaleTempDirectory(PathBuf);

impl Drop for ScaleTempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct AppendJsonStore {
    path: PathBuf,
    file: RefCell<File>,
    index: RefCell<BTreeMap<String, (u64, u64)>>,
}

impl AppendJsonStore {
    fn create(root: &Path, name: &str) -> Result<Self, CanwuError> {
        let path = root.join(format!("{name}.jsonl"));
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| invalid(format!("legal scale store cannot create {name}: {error}")))?;
        Ok(Self {
            path,
            file: RefCell::new(file),
            index: RefCell::new(BTreeMap::new()),
        })
    }

    fn load<V: DeserializeOwned>(&self, key: &str) -> Result<Option<V>, CanwuError> {
        validate_scale_store_key(key)?;
        let Some((offset, len)) = self.index.borrow().get(key).copied() else {
            return Ok(None);
        };
        let len = usize::try_from(len)
            .map_err(|_| invalid("legal scale store value length exceeds usize"))?;
        let mut bytes = vec![0_u8; len];
        let mut file = self.file.borrow_mut();
        file.seek(SeekFrom::Start(offset))
            .and_then(|_| file.read_exact(&mut bytes))
            .map_err(|error| invalid(format!("legal scale store cannot read {key}: {error}")))?;
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| invalid(format!("legal scale store value is invalid: {error}")))
    }

    fn store<V: Serialize>(
        &self,
        key: String,
        value: &V,
    ) -> Result<LegalArchiveStoreOutcome, CanwuError> {
        validate_scale_store_key(&key)?;
        let bytes = serde_json::to_vec(value)
            .map_err(|error| invalid(format!("legal scale store value cannot encode: {error}")))?;
        if let Some((offset, len)) = self.index.borrow().get(&key).copied() {
            let len = usize::try_from(len)
                .map_err(|_| invalid("legal scale store value length exceeds usize"))?;
            let mut existing = vec![0_u8; len];
            let mut file = self.file.borrow_mut();
            file.seek(SeekFrom::Start(offset))
                .and_then(|_| file.read_exact(&mut existing))
                .map_err(|error| {
                    invalid(format!("legal scale store cannot verify {key}: {error}"))
                })?;
            if existing != bytes {
                return Err(invalid(
                    "legal scale archive identity contains different content",
                ));
            }
            return Ok(LegalArchiveStoreOutcome::AlreadyStored);
        }
        let len = u64::try_from(bytes.len())
            .map_err(|_| invalid("legal scale store value length exceeds u64"))?;
        let mut file = self.file.borrow_mut();
        let offset = file
            .seek(SeekFrom::End(0))
            .map_err(|error| invalid(format!("legal scale store cannot seek: {error}")))?;
        file.write_all(&bytes)
            .map_err(|error| invalid(format!("legal scale store cannot append: {error}")))?;
        self.index.borrow_mut().insert(key, (offset, len));
        Ok(LegalArchiveStoreOutcome::Stored)
    }

    fn metrics(&self) -> Result<(u64, u64), CanwuError> {
        let bytes = fs::metadata(&self.path)
            .map_err(|error| {
                invalid(format!(
                    "legal scale store metadata is unavailable: {error}"
                ))
            })?
            .len();
        let entries = u64::try_from(self.index.borrow().len())
            .map_err(|_| invalid("legal scale store index count exceeds u64"))?;
        Ok((bytes, entries))
    }

    fn retain_keys(&self, retained: &BTreeSet<String>) {
        self.index
            .borrow_mut()
            .retain(|key, _| retained.contains(key));
    }
}

fn validate_scale_store_key(key: &str) -> Result<(), CanwuError> {
    if key.len() != 64
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(
            "legal scale store key is not a canonical content address",
        ));
    }
    Ok(())
}

struct LegalTemporalScaleProvider {
    blobs: AppendJsonStore,
    memberships: RefCell<BTreeMap<LegalVersionRef, LegalArchiveMembership>>,
    directories: AppendJsonStore,
    membership_pages: AppendJsonStore,
    temporal_pages: AppendJsonStore,
    retention: RefCell<LegalArchiveRetentionLedger>,
    _temp_directory: ScaleTempDirectory,
}

impl LegalTemporalScaleProvider {
    fn try_new() -> Result<Self, CanwuError> {
        let sequence = NEXT_LEGAL_SCALE_STORE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "canwu-law-format8-scale-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).map_err(|error| {
            invalid(format!(
                "legal scale store cannot create temporary directory: {error}"
            ))
        })?;
        let temp_directory = ScaleTempDirectory(root.clone());
        Ok(Self {
            blobs: AppendJsonStore::create(&root, "blobs")?,
            memberships: RefCell::new(BTreeMap::new()),
            directories: AppendJsonStore::create(&root, "directories")?,
            membership_pages: AppendJsonStore::create(&root, "membership-pages")?,
            temporal_pages: AppendJsonStore::create(&root, "temporal-pages")?,
            retention: RefCell::new(LegalArchiveRetentionLedger::default()),
            _temp_directory: temp_directory,
        })
    }

    fn backing_store_metrics(&self) -> Result<(u64, u64), CanwuError> {
        let mut bytes = 0_u64;
        let mut entries = 0_u64;
        for store in [
            &self.blobs,
            &self.directories,
            &self.membership_pages,
            &self.temporal_pages,
        ] {
            let (store_bytes, store_entries) = store.metrics()?;
            bytes = bytes
                .checked_add(store_bytes)
                .ok_or_else(|| invalid("legal scale store byte count overflowed"))?;
            entries = entries
                .checked_add(store_entries)
                .ok_or_else(|| invalid("legal scale store entry count overflowed"))?;
        }
        Ok((bytes, entries))
    }

    fn retention_metrics(&self) -> Result<(u64, u64, u64, u64), CanwuError> {
        let retention = self.retention.borrow();
        let handles = u64::try_from(retention.handles.len())
            .map_err(|_| invalid("legal retention handle count exceeds u64"))?;
        let roots = u64::try_from(retention.committed_roots.len())
            .map_err(|_| invalid("legal retained root count exceeds u64"))?;
        let committed_objects =
            retention
                .committed_roots
                .values()
                .try_fold(0_u64, |total, reachability| {
                    total
                        .checked_add(reachability.objects.len() as u64)
                        .ok_or_else(|| invalid("legal retained object count overflowed"))
                })?;
        let terminal_payload_items =
            retention
                .handles
                .values()
                .try_fold(0_u64, |total, handle| {
                    total
                        .checked_add(handle.reachability.objects.len() as u64)
                        .and_then(|value| {
                            value.checked_add(handle.reachability.index_page_ids.len() as u64)
                        })
                        .ok_or_else(|| invalid("legal terminal retention payload count overflowed"))
                })?;
        Ok((handles, roots, committed_objects, terminal_payload_items))
    }

    fn finish_committed_head(&self, head: &LegalArchiveHead) -> Result<(), CanwuError> {
        let directory = self
            .directories
            .load::<LegalArchiveIndexDirectory>(&head.membership_root)?
            .ok_or_else(|| invalid("committed legal scale directory is unavailable"))?;
        if directory.directory_id()? != head.membership_root
            || directory.effective_root()? != head.effective_time_root
            || directory.recorded_root()? != head.recorded_time_root
        {
            return Err(invalid(
                "committed legal scale directory disagrees with its head",
            ));
        }
        let membership_pages = directory
            .membership_pages
            .values()
            .cloned()
            .collect::<BTreeSet<_>>();
        let temporal_pages = directory
            .effective_pages
            .values()
            .flatten()
            .chain(directory.recorded_pages.values().flatten())
            .cloned()
            .collect::<BTreeSet<_>>();
        self.directories
            .retain_keys(&BTreeSet::from([head.membership_root.clone()]));
        self.membership_pages.retain_keys(&membership_pages);
        self.temporal_pages.retain_keys(&temporal_pages);
        self.memberships.borrow_mut().clear();
        Ok(())
    }
}

impl Default for LegalTemporalScaleProvider {
    fn default() -> Self {
        Self::try_new().expect("legal temporal scale provider should create its temp store")
    }
}

impl LegalArchiveProvider for LegalTemporalScaleProvider {
    fn load_legal_archive(&self, blob_id: &str) -> Result<Option<LegalArchiveBlob>, CanwuError> {
        self.blobs.load(blob_id)
    }

    fn load_legal_archive_membership(
        &self,
        version: &LegalVersionRef,
    ) -> Result<Option<LegalArchiveMembership>, CanwuError> {
        Ok(self.memberships.borrow().get(version).cloned())
    }

    fn load_legal_archive_index_directory(
        &self,
        directory_id: &str,
    ) -> Result<Option<LegalArchiveIndexDirectory>, CanwuError> {
        self.directories.load(directory_id)
    }

    fn load_legal_archive_membership_page(
        &self,
        page_id: &str,
    ) -> Result<Option<LegalArchiveMembershipPage>, CanwuError> {
        self.membership_pages.load(page_id)
    }

    fn load_legal_archive_temporal_page(
        &self,
        page_id: &str,
    ) -> Result<Option<LegalArchiveTemporalPage>, CanwuError> {
        self.temporal_pages.load(page_id)
    }
}

impl LegalArchiveStore for LegalTemporalScaleProvider {
    fn load_legal_archive_retention_handle(
        &self,
        handle_id: &str,
    ) -> Result<Option<LegalArchiveRetentionHandle>, CanwuError> {
        Ok(self.retention.borrow().handles.get(handle_id).cloned())
    }

    fn prepare_legal_archive_retention(
        &self,
        prepared: &PreparedLegalArchiveBatch,
    ) -> Result<String, CanwuError> {
        self.retention.borrow_mut().prepare(prepared)
    }

    fn verify_legal_archive_retention(
        &self,
        handle_id: &str,
        directory: &LegalArchiveIndexDirectory,
        new_objects: &BTreeSet<ArchiveObjectId>,
    ) -> Result<(), CanwuError> {
        self.retention
            .borrow_mut()
            .verify_and_bind(handle_id, directory, new_objects, self)
    }

    fn mark_legal_archive_retention_durable(&self, handle_id: &str) -> Result<(), CanwuError> {
        self.retention.borrow_mut().mark_durable_ingress(handle_id)
    }

    fn commit_legal_archive_retention(
        &self,
        handle_id: &str,
        target_root: &str,
    ) -> Result<(), CanwuError> {
        self.retention.borrow_mut().commit(handle_id, target_root)
    }

    fn reject_stale_legal_archive_retention(&self, handle_id: &str) -> Result<(), CanwuError> {
        self.retention.borrow_mut().reject_stale(handle_id)
    }

    fn abandon_legal_archive_retention(&self, handle_id: &str) -> Result<(), CanwuError> {
        self.retention.borrow_mut().abandon(handle_id)
    }

    fn store_legal_archive(
        &self,
        blob: &LegalArchiveBlob,
    ) -> Result<LegalArchiveStoreOutcome, CanwuError> {
        self.blobs.store(blob.blob_id()?, blob)
    }

    fn store_legal_archive_membership(
        &self,
        membership: &LegalArchiveMembership,
    ) -> Result<LegalArchiveStoreOutcome, CanwuError> {
        store_scale_value(
            &self.memberships,
            membership.version.clone(),
            membership.clone(),
        )
    }

    fn store_legal_archive_index_directory(
        &self,
        directory: &LegalArchiveIndexDirectory,
    ) -> Result<LegalArchiveStoreOutcome, CanwuError> {
        self.directories.store(directory.directory_id()?, directory)
    }

    fn store_legal_archive_membership_page(
        &self,
        page: &LegalArchiveMembershipPage,
    ) -> Result<LegalArchiveStoreOutcome, CanwuError> {
        self.membership_pages.store(page.page_id()?, page)
    }

    fn store_legal_archive_temporal_page(
        &self,
        page: &LegalArchiveTemporalPage,
    ) -> Result<LegalArchiveStoreOutcome, CanwuError> {
        self.temporal_pages.store(page.page_id()?, page)
    }
}

fn store_scale_value<K: Ord, V: Clone + PartialEq>(
    values: &RefCell<BTreeMap<K, V>>,
    key: K,
    value: V,
) -> Result<LegalArchiveStoreOutcome, CanwuError> {
    let mut values = values.borrow_mut();
    if let Some(existing) = values.get(&key) {
        if existing != &value {
            return Err(invalid(
                "legal scale archive identity contains different content",
            ));
        }
        return Ok(LegalArchiveStoreOutcome::AlreadyStored);
    }
    values.insert(key, value);
    Ok(LegalArchiveStoreOutcome::Stored)
}

pub fn decompose_legal_time_interval(
    interval: LegalTimeInterval,
) -> Result<Vec<LegalDyadicCell>, CanwuError> {
    interval.validate()?;
    let mut start = u128::from(encode_time(interval.start));
    let end = interval
        .end_exclusive
        .map_or(1_u128 << 64, |end| u128::from(encode_time(end)));
    if end <= start {
        return Err(invalid(
            "legal temporal interval is empty after canonical time encoding",
        ));
    }
    let mut cells = Vec::new();
    while start < end {
        let remaining = end - start;
        let alignment_size = if start == 0 {
            1_u128 << 64
        } else {
            1_u128 << start.trailing_zeros().min(64)
        };
        let remaining_size = 1_u128 << remaining.ilog2();
        let size = alignment_size.min(remaining_size);
        let size_bits = size.trailing_zeros() as u8;
        let prefix_length = LEGAL_TEMPORAL_WIDTH - size_bits;
        let cell = LegalDyadicCell {
            prefix_bits: masked_prefix(start as u64, prefix_length),
            prefix_length,
        };
        cell.validate()?;
        cells.push(cell);
        if cells.len() > MAX_LEGAL_TEMPORAL_CELLS_PER_INTERVAL {
            return Err(invalid(
                "legal temporal interval exceeds its canonical cell bound",
            ));
        }
        start += size;
    }
    Ok(cells)
}

fn encode_time(time: SimTime) -> u64 {
    time.as_minutes().cast_unsigned() ^ (1_u64 << 63)
}

fn masked_prefix(value: u64, prefix_length: u8) -> u64 {
    match prefix_length {
        0 => 0,
        64 => value,
        length => value & (u64::MAX << (64 - length)),
    }
}

fn query_budget_error(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::QueryBudgetExceeded, message)
}

const fn default_true() -> bool {
    true
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

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

    #[must_use]
    pub fn coordinator(legal_order: impl Into<String>) -> Self {
        Self {
            kind: LegalShardKind::Coordinator,
            legal_order: legal_order.into(),
            jurisdiction: None,
        }
    }

    #[must_use]
    pub fn culture_dependency(legal_order: impl Into<String>) -> Self {
        Self {
            kind: LegalShardKind::CultureDependency,
            legal_order: legal_order.into(),
            jurisdiction: None,
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
    Publicity,
    Rule,
    LawVersion,
    Case,
    Finding,
    Ruling,
    Conflict,
    Succession,
    Coordinator,
    Retirement,
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

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalArchiveReachability {
    pub objects: BTreeSet<ArchiveObjectId>,
    pub index_page_ids: BTreeSet<String>,
    pub directory_ids: BTreeSet<String>,
    pub membership_page_ids: BTreeSet<String>,
    pub temporal_page_ids: BTreeSet<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_interval: Option<LegalTimeInterval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_interval: Option<LegalTimeInterval>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalArchiveMembershipPage {
    pub format_version: u32,
    pub shard: LegalShardKey,
    pub bucket: u16,
    pub memberships: Vec<LegalArchiveMembership>,
}

impl LegalArchiveMembershipPage {
    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.format_version != LEGAL_ARCHIVE_INDEX_FORMAT_VERSION
            || u32::from(self.bucket) >= LEGAL_ARCHIVE_INDEX_BUCKET_COUNT
            || self.memberships.is_empty()
            || self.memberships.len() > MAX_LEGAL_ARCHIVE_PAGE_ENTRIES
            || self
                .memberships
                .windows(2)
                .any(|pair| pair[0].version >= pair[1].version)
        {
            return Err(invalid("legal archive membership page is malformed"));
        }
        self.shard.validate()?;
        for membership in &self.memberships {
            validate_version(&membership.version)?;
            if membership.version.object.home_shard != self.shard
                || legal_archive_index_bucket(
                    "canwu.law.archive-membership-bucket.v1",
                    &membership.version,
                )? != self.bucket
                || !matches!(membership.location, LegalVersionLocation::Archived { .. })
            {
                return Err(invalid(
                    "legal archive membership page contains a misplaced member",
                ));
            }
        }
        let encoded = serde_json::to_vec(self).map_err(|error| {
            invalid(format!(
                "legal archive membership page cannot be encoded: {error}"
            ))
        })?;
        if encoded.len() > MAX_LEGAL_ARCHIVE_PAGE_BYTES {
            return Err(invalid(
                "legal archive membership page exceeds the hard byte limit",
            ));
        }
        Ok(())
    }

    pub fn page_id(&self) -> Result<String, CanwuError> {
        self.validate()?;
        canonical_hash("canwu.law.archive-membership-page.v1", self)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LegalTemporalAxis {
    Effective,
    Recorded,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalArchiveTemporalEntry {
    pub cell: LegalDyadicCell,
    pub version: LegalVersionRef,
    pub primary_member_commitment: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalArchiveTemporalPage {
    pub format_version: u32,
    pub shard: LegalShardKey,
    pub axis: LegalTemporalAxis,
    pub bucket: u16,
    pub segment: u32,
    pub entries: Vec<LegalArchiveTemporalEntry>,
}

impl LegalArchiveTemporalPage {
    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.format_version != LEGAL_ARCHIVE_INDEX_FORMAT_VERSION
            || u32::from(self.bucket) >= LEGAL_ARCHIVE_INDEX_BUCKET_COUNT
            || self.entries.is_empty()
            || self.entries.len() > MAX_LEGAL_ARCHIVE_PAGE_ENTRIES
            || self.entries.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(invalid("legal archive temporal page is malformed"));
        }
        self.shard.validate()?;
        let domain = match self.axis {
            LegalTemporalAxis::Effective => "canwu.law.archive-effective-bucket.v1",
            LegalTemporalAxis::Recorded => "canwu.law.archive-recorded-bucket.v1",
        };
        for entry in &self.entries {
            entry.cell.validate()?;
            validate_version(&entry.version)?;
            validate_hash(
                &entry.primary_member_commitment,
                "legal temporal primary member commitment",
            )?;
            if entry.version.object.home_shard != self.shard
                || legal_archive_index_bucket(domain, &entry.cell)? != self.bucket
            {
                return Err(invalid(
                    "legal archive temporal page contains a misplaced entry",
                ));
            }
        }
        let encoded = serde_json::to_vec(self).map_err(|error| {
            invalid(format!(
                "legal archive temporal page cannot be encoded: {error}"
            ))
        })?;
        if encoded.len() > MAX_LEGAL_ARCHIVE_PAGE_BYTES {
            return Err(invalid(
                "legal archive temporal page exceeds the hard byte limit",
            ));
        }
        Ok(())
    }

    pub fn page_id(&self) -> Result<String, CanwuError> {
        self.validate()?;
        canonical_hash("canwu.law.archive-temporal-page.v1", self)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalArchiveIndexDirectory {
    pub format_version: u32,
    pub shard: LegalShardKey,
    pub archived_member_count: u64,
    pub membership_pages: BTreeMap<u16, String>,
    pub effective_pages: BTreeMap<u16, Vec<String>>,
    pub recorded_pages: BTreeMap<u16, Vec<String>>,
}

impl LegalArchiveIndexDirectory {
    #[must_use]
    pub fn empty(shard: LegalShardKey) -> Self {
        Self {
            format_version: LEGAL_ARCHIVE_INDEX_FORMAT_VERSION,
            shard,
            archived_member_count: 0,
            membership_pages: BTreeMap::new(),
            effective_pages: BTreeMap::new(),
            recorded_pages: BTreeMap::new(),
        }
    }

    pub fn validate(&self) -> Result<(), CanwuError> {
        self.shard.validate()?;
        if self.format_version != LEGAL_ARCHIVE_INDEX_FORMAT_VERSION
            || (self.archived_member_count == 0) != self.membership_pages.is_empty()
        {
            return Err(invalid("legal archive index directory is malformed"));
        }
        if self.membership_pages.iter().any(|(bucket, page_id)| {
            u32::from(*bucket) >= LEGAL_ARCHIVE_INDEX_BUCKET_COUNT
                || validate_hash(page_id, "page ID").is_err()
        }) {
            return Err(invalid("legal archive membership directory is malformed"));
        }
        for pages in [&self.effective_pages, &self.recorded_pages] {
            if pages.iter().any(|(bucket, page_ids)| {
                u32::from(*bucket) >= LEGAL_ARCHIVE_INDEX_BUCKET_COUNT
                    || page_ids.is_empty()
                    || page_ids
                        .iter()
                        .any(|page_id| validate_hash(page_id, "page ID").is_err())
                    || page_ids.iter().collect::<BTreeSet<_>>().len() != page_ids.len()
            }) {
                return Err(invalid("legal archive temporal directory is malformed"));
            }
        }
        let encoded = serde_json::to_vec(self).map_err(|error| {
            invalid(format!(
                "legal archive index directory cannot be encoded: {error}"
            ))
        })?;
        if encoded.len() > MAX_LEGAL_ARCHIVE_DIRECTORY_BYTES {
            return Err(invalid(
                "legal archive index directory exceeds the hard byte limit",
            ));
        }
        Ok(())
    }

    pub fn directory_id(&self) -> Result<String, CanwuError> {
        self.validate()?;
        canonical_hash("canwu.law.archive-index-directory.v1", self)
    }

    pub fn effective_root(&self) -> Result<String, CanwuError> {
        canonical_hash(EFFECTIVE_TIME_ROOT_DOMAIN, &self.effective_pages)
    }

    pub fn recorded_root(&self) -> Result<String, CanwuError> {
        canonical_hash(RECORDED_TIME_ROOT_DOMAIN, &self.recorded_pages)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalArchiveHead {
    pub shard: LegalShardKey,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub index_format_version: u32,
    pub committed_batch_count: u64,
    pub archived_member_count: u64,
    pub membership_root: String,
    pub effective_time_root: String,
    pub recorded_time_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_content_id: Option<String>,
}

/// Stable shard topology. Moving a shard out of the hot set changes only this
/// directory and leaves historical home-shard routes valid.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalDirectoryRecord {
    pub schema_version: u32,
    #[serde(default)]
    pub active_shards: BTreeSet<LegalShardKey>,
    #[serde(default)]
    pub archive_only_shards: BTreeSet<LegalShardKey>,
    #[serde(default, with = "ordered_map_serde")]
    pub object_routes: BTreeMap<String, LegalShardKey>,
    #[serde(default, with = "ordered_map_serde")]
    pub due_shards: BTreeMap<SimTime, BTreeSet<LegalShardKey>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalOrderShardRecord {
    pub shard: LegalShardKey,
    pub rule_ids: BTreeSet<String>,
    pub normative_heads: BTreeMap<String, LegalHeadRef>,
    pub scheduled_heads: BTreeMap<String, Vec<LegalVersionRef>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalJurisdictionShardRecord {
    pub shard: LegalShardKey,
    pub open_procedures: BTreeSet<String>,
    pub case_ids: BTreeSet<String>,
    pub finding_ids: BTreeSet<String>,
    pub ruling_ids: BTreeSet<String>,
    pub conflict_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalCoordinatorRecord {
    pub coordinator_id: String,
    pub participants: Vec<LegalShardKey>,
    pub expected_versions: BTreeMap<String, u64>,
    pub phase: String,
    pub terminal_result: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalCultureDependencyRecord {
    pub target: String,
    pub generation: u64,
    pub live_dependents: BTreeSet<LegalVersionRef>,
    pub scheduled_dependents: BTreeSet<LegalVersionRef>,
    pub operative_dependents: BTreeSet<LegalVersionRef>,
}

/// Persisted plan/core portion of the live law state. Large semantic
/// collections are removed from this object and stored in independent shard
/// records before it enters the kernel domain-record store.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalPlanState {
    pub format_version: u32,
    pub plan_hash: String,
    pub fields: BTreeMap<String, serde_json::Value>,
    #[serde(rename = "canwu_identity_evidence_dependencies")]
    pub evidence_dependencies: canwu_api::IdentityEvidenceDependenciesV1,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalShardState {
    pub format_version: u32,
    pub shard: LegalShardKey,
    pub fields: BTreeMap<String, serde_json::Value>,
    #[serde(rename = "canwu_identity_evidence_dependencies")]
    pub evidence_dependencies: canwu_api::IdentityEvidenceDependenciesV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalDirectoryState {
    pub format_version: u32,
    pub directory: LegalDirectoryRecord,
    #[serde(with = "ordered_map_serde")]
    pub shard_record_ids: BTreeMap<LegalShardKey, String>,
    #[serde(with = "ordered_map_serde")]
    pub archive_head_record_ids: BTreeMap<LegalShardKey, String>,
    #[serde(rename = "canwu_identity_evidence_dependencies")]
    pub evidence_dependencies: canwu_api::IdentityEvidenceDependenciesV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalArchiveHeadState {
    pub format_version: u32,
    pub shard: LegalShardKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<LegalArchiveHead>,
    #[serde(rename = "canwu_identity_evidence_dependencies")]
    pub evidence_dependencies: canwu_api::IdentityEvidenceDependenciesV1,
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

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LegalCompactionOrderKey {
    shard: LegalShardKey,
    record_class: String,
    closed_at: SimTime,
    version_ordinal: u64,
    object_id: String,
    local_discriminator: Option<String>,
    version: LegalVersionRef,
}

impl LegalCompactionOrderKey {
    fn from_candidate(candidate: &LegalCompactionCandidate) -> Self {
        Self {
            shard: candidate.version.object.home_shard.clone(),
            record_class: candidate.record_class.clone(),
            closed_at: candidate.closed_at,
            version_ordinal: candidate.version.version_ordinal,
            object_id: candidate.version.object.id.clone(),
            local_discriminator: candidate.version.object.local_discriminator.clone(),
            version: candidate.version.clone(),
        }
    }

    fn shard_start(shard: &LegalShardKey) -> Self {
        Self {
            shard: shard.clone(),
            record_class: String::new(),
            closed_at: SimTime::from_minutes(i64::MIN),
            version_ordinal: 0,
            object_id: String::new(),
            local_discriminator: None,
            version: LegalVersionRef {
                object: LegalObjectId {
                    kind: LegalObjectKind::Proposal,
                    id: String::new(),
                    home_shard: shard.clone(),
                    local_discriminator: None,
                },
                version_ordinal: 0,
                content_commitment: String::new(),
            },
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LegalCandidateAccumulator {
    count: u64,
    xor: [u8; 32],
    sum: [u64; 4],
}

impl LegalCandidateAccumulator {
    fn digest(candidate: &LegalCompactionCandidate) -> Result<[u8; 32], CanwuError> {
        let encoded = serde_json::to_vec(&("canwu.law.compaction-candidate.v1", candidate))
            .map_err(|error| invalid(format!("legal candidate cannot be encoded: {error}")))?;
        Ok(*blake3::hash(&encoded).as_bytes())
    }

    fn insert(&mut self, candidate: &LegalCompactionCandidate) -> Result<(), CanwuError> {
        let digest = Self::digest(candidate)?;
        self.count = self
            .count
            .checked_add(1)
            .ok_or_else(|| invalid("legal candidate count is exhausted"))?;
        for (index, byte) in digest.iter().enumerate() {
            self.xor[index] ^= byte;
        }
        for (index, chunk) in digest.as_chunks::<8>().0.iter().enumerate() {
            self.sum[index] = self.sum[index].wrapping_add(u64::from_be_bytes(*chunk));
        }
        Ok(())
    }

    fn remove(&mut self, candidate: &LegalCompactionCandidate) -> Result<(), CanwuError> {
        let digest = Self::digest(candidate)?;
        self.count = self
            .count
            .checked_sub(1)
            .ok_or_else(|| invalid("legal candidate count underflowed"))?;
        for (index, byte) in digest.iter().enumerate() {
            self.xor[index] ^= byte;
        }
        for (index, chunk) in digest.as_chunks::<8>().0.iter().enumerate() {
            self.sum[index] = self.sum[index].wrapping_sub(u64::from_be_bytes(*chunk));
        }
        Ok(())
    }
}

/// Shard-local hot storage indexes. Keeping these out of the coordinator
/// record prevents an unrelated legal ingress from decoding or rebuilding
/// the world's complete compaction debt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LegalStorageShardState {
    pub(crate) format_version: u32,
    pub(crate) shard: LegalShardKey,
    #[serde(with = "ordered_map_serde")]
    pub(crate) heads: BTreeMap<LegalObjectId, LegalHeadRef>,
    #[serde(with = "ordered_map_serde")]
    pub(crate) membership: BTreeMap<LegalVersionRef, LegalArchiveMembership>,
    #[serde(with = "ordered_map_serde")]
    pub(crate) compaction_candidates: BTreeMap<LegalVersionRef, LegalCompactionCandidate>,
    pub(crate) compaction_order: BTreeSet<LegalCompactionOrderKey>,
    pub(crate) candidate_accumulator: Option<LegalCandidateAccumulator>,
}

impl LegalStorageShardState {
    fn validate(&self) -> Result<(), CanwuError> {
        if self.format_version != LEGAL_STORAGE_FORMAT_VERSION {
            return Err(invalid("legal storage shard uses an unsupported format"));
        }
        self.shard.validate()?;
        if self
            .heads
            .iter()
            .any(|(object, head)| object != &head.object || object.home_shard != self.shard)
            || self.membership.iter().any(|(version, membership)| {
                version != &membership.version
                    || version.object.home_shard != self.shard
                    || !matches!(membership.location, LegalVersionLocation::Hot)
            })
            || self
                .compaction_candidates
                .iter()
                .any(|(version, candidate)| {
                    version != &candidate.version || version.object.home_shard != self.shard
                })
            || self
                .compaction_order
                .iter()
                .any(|key| key.shard != self.shard)
        {
            return Err(invalid(
                "legal storage shard contains misplaced hot indexes",
            ));
        }
        let expected_order = self
            .compaction_candidates
            .values()
            .filter(|candidate| candidate.is_eligible())
            .map(LegalCompactionOrderKey::from_candidate)
            .collect::<BTreeSet<_>>();
        if self.compaction_order != expected_order {
            return Err(invalid(
                "legal storage shard compaction order is inconsistent",
            ));
        }
        let mut expected_accumulator = LegalCandidateAccumulator::default();
        for candidate in self.compaction_candidates.values() {
            expected_accumulator.insert(candidate)?;
        }
        let expected_accumulator =
            (!self.compaction_candidates.is_empty()).then_some(expected_accumulator);
        if self.candidate_accumulator != expected_accumulator {
            return Err(invalid(
                "legal storage shard candidate accumulator is inconsistent",
            ));
        }
        Ok(())
    }
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
    pub examined_candidates: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalArchiveBlob {
    pub format_version: u32,
    pub version: LegalVersionRef,
    pub record_class: String,
    pub payload: serde_json::Value,
}

impl LegalArchiveBlob {
    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.format_version != LEGAL_ARCHIVE_BLOB_FORMAT_VERSION {
            return Err(invalid("unsupported legal archive blob format"));
        }
        validate_version(&self.version)?;
        require_identifier(&self.record_class, "legal archive record class")?;
        if !self.payload.is_object() {
            return Err(invalid("legal archive payload must be an object"));
        }
        if self.version.content_commitment
            != legal_archive_content_commitment(&self.record_class, &self.payload)?
        {
            return Err(invalid(
                "legal archive payload disagrees with its version commitment",
            ));
        }
        Ok(())
    }

    pub fn content_id(&self) -> Result<String, CanwuError> {
        self.validate()?;
        canonical_hash(
            "canwu.law.archive-content.v1",
            &(&self.version, &self.record_class, &self.payload),
        )
    }

    pub fn blob_id(&self) -> Result<String, CanwuError> {
        self.validate()?;
        canonical_hash("canwu.law.archive-blob.v1", self)
    }
}

pub fn legal_archive_content_commitment(
    record_class: &str,
    payload: &serde_json::Value,
) -> Result<String, CanwuError> {
    require_identifier(record_class, "legal archive record class")?;
    if !payload.is_object() {
        return Err(invalid("legal archive payload must be an object"));
    }
    canonical_hash(
        "canwu.law.archive-record-content.v1",
        &(record_class, payload),
    )
}

fn legal_archive_index_bucket<T: Serialize + ?Sized>(
    domain: &str,
    value: &T,
) -> Result<u16, CanwuError> {
    let encoded = serde_json::to_vec(&(domain, value)).map_err(|error| {
        invalid(format!(
            "legal archive index key cannot be encoded: {error}"
        ))
    })?;
    let digest = blake3::hash(&encoded);
    Ok(u16::from_be_bytes([
        digest.as_bytes()[0],
        digest.as_bytes()[1],
    ]))
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedLegalArchiveBatch {
    pub format_version: u32,
    pub compaction: PreparedLegalCompaction,
    pub blobs: Vec<LegalArchiveBlob>,
    pub receipts: Vec<ArchiveObjectReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_archive_head: Option<LegalArchiveHead>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedLegalArchiveCommit {
    pub(crate) format_version: u32,
    pub(crate) compaction: PreparedLegalCompaction,
    pub(crate) receipts: Vec<ArchiveObjectReceipt>,
    pub(crate) archive_head: LegalArchiveHead,
    pub(crate) pending_reachability: LegalPendingArchiveReachability,
    pub(crate) retention_handle_id: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_field_names)]
pub(crate) struct LegalPendingArchiveReachability {
    /// One authenticated directory root transitively owns every new blob and
    /// index page until canonical ingress reaches a terminal state. The
    /// plugin reachability participant expands this root before mark/sweep,
    /// so ingress size is independent of accumulated archive history.
    pub(crate) directory_root: String,
}

impl VerifiedLegalArchiveCommit {
    pub(crate) fn validate(&self) -> Result<(), CanwuError> {
        if self.format_version != LEGAL_ARCHIVE_BLOB_FORMAT_VERSION
            || self.compaction.candidates.is_empty()
            || self.compaction.candidates.len() != self.receipts.len()
            || self.compaction.archive_batch_sequence == 0
            || self.archive_head.shard != self.compaction.shard
            || self.archive_head.committed_batch_count != self.compaction.archive_batch_sequence
            || self.archive_head.membership_root != self.pending_reachability.directory_root
        {
            return Err(invalid("verified legal archive commit is inconsistent"));
        }
        validate_hash(
            &self.pending_reachability.directory_root,
            "pending legal archive directory root",
        )?;
        validate_hash(
            &self.retention_handle_id,
            "legal archive retention handle ID",
        )?;
        validate_hash(&self.compaction.token, "legal compaction token")?;
        validate_hash(
            &self.compaction.source_membership_root,
            "legal compaction source root",
        )?;
        validate_archive_head(&self.archive_head)?;
        for (index, (candidate, receipt)) in self
            .compaction
            .candidates
            .iter()
            .zip(&self.receipts)
            .enumerate()
        {
            validate_candidate(candidate)?;
            validate_archive_receipt(receipt)?;
            if candidate.version.object.home_shard != self.compaction.shard
                || receipt.owner_shard != self.compaction.shard
                || receipt.archive_batch_sequence != self.compaction.archive_batch_sequence
                || receipt.member_index != index as u64
                || receipt.source_root != self.compaction.source_membership_root
                || receipt.verified_plan_hash != self.compaction.token
            {
                return Err(invalid(
                    "verified legal archive receipt is not bound to its compaction member",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn authenticate_store_binding(
        &self,
        store: &dyn LegalArchiveStore,
    ) -> Result<(), CanwuError> {
        self.validate()?;
        let handle = store
            .load_legal_archive_retention_handle(&self.retention_handle_id)?
            .ok_or_else(|| invalid("verified legal archive retention handle is unavailable"))?;
        if handle.handle_id != self.retention_handle_id
            || handle.compaction_token != self.compaction.token
            || handle.source_root != self.compaction.source_membership_root
            || handle.target_root.as_deref()
                != Some(self.pending_reachability.directory_root.as_str())
            || !matches!(
                handle.phase,
                LegalArchiveRetentionPhase::Verified
                    | LegalArchiveRetentionPhase::DurableIngress
                    | LegalArchiveRetentionPhase::Committed
                    | LegalArchiveRetentionPhase::RejectedStale
            )
        {
            return Err(invalid(
                "verified legal archive commit is not bound to the store retention handle",
            ));
        }
        let directory = store
            .load_legal_archive_index_directory(&self.pending_reachability.directory_root)?
            .ok_or_else(|| invalid("verified legal archive directory is unavailable"))?;
        directory.validate()?;
        if directory.directory_id()? != self.pending_reachability.directory_root
            || directory.shard != self.archive_head.shard
            || directory.archived_member_count != self.archive_head.archived_member_count
            || directory.effective_root()? != self.archive_head.effective_time_root
            || directory.recorded_root()? != self.archive_head.recorded_time_root
        {
            return Err(invalid(
                "verified legal archive head disagrees with its authenticated directory",
            ));
        }
        let reachability =
            authenticate_legal_archive_root(store, &self.pending_reachability.directory_root)?;
        if u64::try_from(reachability.objects.len())
            .map_err(|_| invalid("legal archive reachability count exceeds u64"))?
            != directory.archived_member_count
        {
            return Err(invalid(
                "verified legal archive directory member count disagrees with its closure",
            ));
        }
        for (candidate, receipt) in self.compaction.candidates.iter().zip(&self.receipts) {
            let bucket = legal_archive_index_bucket(
                "canwu.law.archive-membership-bucket.v1",
                &candidate.version,
            )?;
            let page_id = directory.membership_pages.get(&bucket).ok_or_else(|| {
                invalid("verified legal archive member is absent from its directory")
            })?;
            let page = store
                .load_legal_archive_membership_page(page_id)?
                .ok_or_else(|| invalid("verified legal archive membership page is unavailable"))?;
            page.validate()?;
            if page.shard != directory.shard
                || page.bucket != bucket
                || page.page_id()? != *page_id
                || page
                    .memberships
                    .binary_search_by(|membership| membership.version.cmp(&candidate.version))
                    .ok()
                    .is_none_or(|index| {
                        !matches!(
                            &page.memberships[index].location,
                            LegalVersionLocation::Archived { receipt: indexed }
                                if indexed.as_ref() == receipt
                        )
                    })
            {
                return Err(invalid(
                    "verified legal archive receipt is absent from its authenticated membership page",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LegalArchiveStoreOutcome {
    Stored,
    AlreadyStored,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LegalArchiveRetentionPhase {
    Prepared,
    Verified,
    DurableIngress,
    Committed,
    RejectedStale,
    Abandoned,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalArchiveRetentionHandle {
    pub format_version: u32,
    pub handle_id: String,
    pub compaction_token: String,
    pub source_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_root: Option<String>,
    pub reachability: LegalArchiveReachability,
    pub prepared_epoch: u64,
    pub phase: LegalArchiveRetentionPhase,
}

/// Persistable store-side mark/sweep interlock. An unbound prepared handle
/// blocks a new GC epoch while objects are being written. A verified handle
/// marks the new object delta and the proposed root's current page closure;
/// the committed previous root continues to protect the older closure until
/// durable ingress atomically hands ownership to the new root.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalArchiveRetentionLedger {
    pub format_version: u32,
    pub gc_epoch: u64,
    pub handles: BTreeMap<String, LegalArchiveRetentionHandle>,
    pub committed_roots: BTreeMap<String, LegalArchiveReachability>,
}

impl Default for LegalArchiveRetentionLedger {
    fn default() -> Self {
        Self {
            format_version: LEGAL_ARCHIVE_RETENTION_FORMAT_VERSION,
            gc_epoch: 0,
            handles: BTreeMap::new(),
            committed_roots: BTreeMap::new(),
        }
    }
}

impl LegalArchiveRetentionLedger {
    pub fn prepare(&mut self, prepared: &PreparedLegalArchiveBatch) -> Result<String, CanwuError> {
        self.validate()?;
        let compaction = &prepared.compaction;
        validate_hash(&compaction.token, "legal compaction token")?;
        validate_hash(
            &compaction.source_membership_root,
            "legal compaction source root",
        )?;
        let handle_id = canonical_hash(
            "canwu.law.archive-retention-handle.v1",
            &(
                &compaction.token,
                &compaction.source_membership_root,
                &compaction.shard,
                compaction.archive_batch_sequence,
                prepared
                    .previous_archive_head
                    .as_ref()
                    .map(|head| head.membership_root.as_str()),
                self.gc_epoch,
            ),
        )?;
        let handle = LegalArchiveRetentionHandle {
            format_version: LEGAL_ARCHIVE_RETENTION_FORMAT_VERSION,
            handle_id: handle_id.clone(),
            compaction_token: compaction.token.clone(),
            source_root: compaction.source_membership_root.clone(),
            previous_root: prepared
                .previous_archive_head
                .as_ref()
                .map(|head| head.membership_root.clone()),
            target_root: None,
            reachability: LegalArchiveReachability::default(),
            prepared_epoch: self.gc_epoch,
            phase: LegalArchiveRetentionPhase::Prepared,
        };
        if let Some(existing) = self.handles.get(&handle_id) {
            if existing.handle_id != handle.handle_id
                || existing.compaction_token != handle.compaction_token
                || existing.source_root != handle.source_root
                || existing.previous_root != handle.previous_root
                || existing.prepared_epoch != handle.prepared_epoch
                || matches!(
                    existing.phase,
                    LegalArchiveRetentionPhase::Committed
                        | LegalArchiveRetentionPhase::RejectedStale
                        | LegalArchiveRetentionPhase::Abandoned
                )
            {
                return Err(invalid(
                    "legal archive retention handle collides with different content",
                ));
            }
            return Ok(handle_id);
        }
        self.handles.insert(handle_id.clone(), handle);
        Ok(handle_id)
    }

    pub fn verify_and_bind(
        &mut self,
        handle_id: &str,
        directory: &LegalArchiveIndexDirectory,
        new_objects: &BTreeSet<ArchiveObjectId>,
        provider: &dyn LegalArchiveProvider,
    ) -> Result<(), CanwuError> {
        let handle = self
            .handles
            .get(handle_id)
            .cloned()
            .ok_or_else(|| invalid("legal archive retention handle is unknown"))?;
        if !matches!(
            handle.phase,
            LegalArchiveRetentionPhase::Prepared | LegalArchiveRetentionPhase::Verified
        ) || handle.prepared_epoch != self.gc_epoch
        {
            return Err(invalid(
                "legal archive retention verification crossed a GC epoch",
            ));
        }
        directory.validate()?;
        let target_root = directory.directory_id()?;
        let loaded_directory = provider
            .load_legal_archive_index_directory(&target_root)?
            .ok_or_else(|| invalid("retained legal archive directory root is unavailable"))?;
        if loaded_directory != *directory || loaded_directory.directory_id()? != target_root {
            return Err(invalid(
                "retained legal archive directory root failed authentication",
            ));
        }
        let empty_reachability = LegalArchiveReachability::default();
        let base = if let Some(previous_root) = handle.previous_root.as_deref() {
            self.committed_roots
                .get(previous_root)
                .ok_or_else(|| invalid("previous legal archive retention root is not committed"))?
        } else {
            if !self.committed_roots.is_empty() {
                return Err(invalid(
                    "first legal archive retention root cannot replace committed history",
                ));
            }
            &empty_reachability
        };
        let mut reachability = LegalArchiveReachability {
            objects: new_objects.clone(),
            index_page_ids: BTreeSet::from([target_root.clone()]),
            directory_ids: BTreeSet::from([target_root.clone()]),
            membership_page_ids: directory.membership_pages.values().cloned().collect(),
            temporal_page_ids: directory
                .effective_pages
                .values()
                .flatten()
                .chain(directory.recorded_pages.values().flatten())
                .cloned()
                .collect(),
        };
        reachability
            .index_page_ids
            .extend(reachability.membership_page_ids.iter().cloned());
        reachability
            .index_page_ids
            .extend(reachability.temporal_page_ids.iter().cloned());
        for object in new_objects {
            validate_archive_object_id(object)?;
            if base.objects.contains(object) {
                return Err(invalid(
                    "new legal archive retention object already belongs to the previous root",
                ));
            }
            let blob = provider
                .load_legal_archive(&object.blob_id)?
                .ok_or_else(|| invalid("new retained legal archive blob is unavailable"))?;
            blob.validate()?;
            if blob.content_id()? != object.content_id || blob.blob_id()? != object.blob_id {
                return Err(invalid(
                    "new retained legal archive blob failed authentication",
                ));
            }
        }
        for (bucket, page_id) in &directory.membership_pages {
            if base.membership_page_ids.contains(page_id) {
                continue;
            }
            let page = provider
                .load_legal_archive_membership_page(page_id)?
                .ok_or_else(|| invalid("new legal archive membership page is unavailable"))?;
            page.validate()?;
            if page.shard != directory.shard
                || page.bucket != *bucket
                || page.page_id()? != *page_id
            {
                return Err(invalid(
                    "new legal archive membership page failed authentication",
                ));
            }
        }
        for (axis, pages) in [
            (LegalTemporalAxis::Effective, &directory.effective_pages),
            (LegalTemporalAxis::Recorded, &directory.recorded_pages),
        ] {
            for (bucket, page_ids) in pages {
                for (segment, page_id) in page_ids.iter().enumerate() {
                    if base.temporal_page_ids.contains(page_id) {
                        continue;
                    }
                    let page = provider
                        .load_legal_archive_temporal_page(page_id)?
                        .ok_or_else(|| invalid("new legal archive temporal page is unavailable"))?;
                    page.validate()?;
                    if page.shard != directory.shard
                        || page.axis != axis
                        || page.bucket != *bucket
                        || page.segment != segment as u32
                        || page.page_id()? != *page_id
                    {
                        return Err(invalid(
                            "new legal archive temporal page failed authentication",
                        ));
                    }
                }
            }
        }
        let retained_member_count = u64::try_from(base.objects.len())
            .ok()
            .and_then(|count| count.checked_add(new_objects.len() as u64))
            .ok_or_else(|| invalid("legal archive retention member count is exhausted"))?;
        if retained_member_count != directory.archived_member_count {
            return Err(invalid(
                "legal archive retention closure member count is inconsistent",
            ));
        }
        if handle.phase == LegalArchiveRetentionPhase::Verified
            && (handle.target_root.as_deref() != Some(target_root.as_str())
                || handle.reachability != reachability)
        {
            return Err(invalid(
                "legal archive retention root changed after verification",
            ));
        }
        let handle = self
            .handles
            .get_mut(handle_id)
            .ok_or_else(|| invalid("legal archive retention handle disappeared"))?;
        handle.target_root = Some(target_root);
        handle.reachability = reachability;
        handle.phase = LegalArchiveRetentionPhase::Verified;
        self.validate()
    }

    pub fn mark_durable_ingress(&mut self, handle_id: &str) -> Result<(), CanwuError> {
        let phase = self
            .handles
            .get(handle_id)
            .ok_or_else(|| invalid("legal archive retention handle is unknown"))?
            .phase;
        if matches!(
            phase,
            LegalArchiveRetentionPhase::DurableIngress
                | LegalArchiveRetentionPhase::Committed
                | LegalArchiveRetentionPhase::RejectedStale
        ) {
            return Ok(());
        }
        self.transition(
            handle_id,
            LegalArchiveRetentionPhase::Verified,
            LegalArchiveRetentionPhase::DurableIngress,
        )
    }

    pub fn commit(&mut self, handle_id: &str, target_root: &str) -> Result<(), CanwuError> {
        let handle = self
            .handles
            .get(handle_id)
            .cloned()
            .ok_or_else(|| invalid("legal archive retention handle is unknown"))?;
        if handle.target_root.as_deref() != Some(target_root) {
            return Err(invalid(
                "only matching durable legal archive ingress may commit retention",
            ));
        }
        if handle.phase == LegalArchiveRetentionPhase::Committed {
            return Ok(());
        }
        if handle.phase != LegalArchiveRetentionPhase::DurableIngress {
            return Err(invalid(
                "only matching durable legal archive ingress may commit retention",
            ));
        }
        if let Some(existing) = self.committed_roots.get(target_root)
            && existing != &handle.reachability
        {
            return Err(invalid(
                "committed legal archive root is bound to another closure",
            ));
        }
        let previous = if let Some(previous_root) = handle.previous_root.as_deref() {
            Some(self.committed_roots.remove(previous_root).ok_or_else(|| {
                invalid("previous legal archive retention root disappeared before commit")
            })?)
        } else {
            None
        };
        let committed = self
            .handles
            .get_mut(handle_id)
            .ok_or_else(|| invalid("legal archive retention handle disappeared"))?;
        let mut reachability = std::mem::take(&mut committed.reachability);
        committed.phase = LegalArchiveRetentionPhase::Committed;
        if let Some(previous) = previous {
            reachability.objects.extend(previous.objects);
        }
        self.committed_roots
            .insert(target_root.to_owned(), reachability);
        self.validate()
    }

    pub fn reject_stale(&mut self, handle_id: &str) -> Result<(), CanwuError> {
        self.transition(
            handle_id,
            LegalArchiveRetentionPhase::DurableIngress,
            LegalArchiveRetentionPhase::RejectedStale,
        )?;
        self.handles
            .get_mut(handle_id)
            .ok_or_else(|| invalid("legal archive retention handle disappeared"))?
            .reachability = LegalArchiveReachability::default();
        self.validate()
    }

    pub fn abandon(&mut self, handle_id: &str) -> Result<(), CanwuError> {
        let handle = self
            .handles
            .get_mut(handle_id)
            .ok_or_else(|| invalid("legal archive retention handle is unknown"))?;
        if matches!(handle.phase, LegalArchiveRetentionPhase::Committed) {
            return Err(invalid(
                "committed legal archive retention cannot be abandoned",
            ));
        }
        handle.phase = LegalArchiveRetentionPhase::Abandoned;
        handle.reachability = LegalArchiveReachability::default();
        self.validate()
    }

    pub fn begin_gc_epoch(&mut self) -> Result<u64, CanwuError> {
        if self.handles.values().any(|handle| {
            handle.phase == LegalArchiveRetentionPhase::Prepared && handle.target_root.is_none()
        }) {
            return Err(CanwuError::new(
                ErrorCode::ArchiveNotReady,
                "legal archive GC cannot cross an unbound prepare/verify handoff",
            ));
        }
        self.gc_epoch = self
            .gc_epoch
            .checked_add(1)
            .ok_or_else(|| invalid("legal archive GC epoch is exhausted"))?;
        Ok(self.gc_epoch)
    }

    #[must_use]
    pub fn reachable(&self) -> LegalArchiveReachability {
        let mut reachable = LegalArchiveReachability::default();
        for closure in self.committed_roots.values().chain(
            self.handles
                .values()
                .filter(|handle| {
                    matches!(
                        handle.phase,
                        LegalArchiveRetentionPhase::Verified
                            | LegalArchiveRetentionPhase::DurableIngress
                    )
                })
                .map(|handle| &handle.reachability),
        ) {
            reachable.objects.extend(closure.objects.iter().cloned());
            reachable
                .index_page_ids
                .extend(closure.index_page_ids.iter().cloned());
            reachable
                .directory_ids
                .extend(closure.directory_ids.iter().cloned());
            reachable
                .membership_page_ids
                .extend(closure.membership_page_ids.iter().cloned());
            reachable
                .temporal_page_ids
                .extend(closure.temporal_page_ids.iter().cloned());
        }
        reachable
    }

    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.format_version != LEGAL_ARCHIVE_RETENTION_FORMAT_VERSION {
            return Err(invalid("unsupported legal archive retention format"));
        }
        for (handle_id, handle) in &self.handles {
            validate_hash(handle_id, "legal archive retention handle ID")?;
            validate_hash(&handle.compaction_token, "legal compaction token")?;
            validate_hash(&handle.source_root, "legal compaction source root")?;
            if let Some(previous_root) = &handle.previous_root {
                validate_hash(previous_root, "previous legal archive root")?;
            }
            if handle.format_version != LEGAL_ARCHIVE_RETENTION_FORMAT_VERSION
                || handle.handle_id != *handle_id
                || handle.prepared_epoch > self.gc_epoch
                || (handle.phase == LegalArchiveRetentionPhase::Prepared
                    && (handle.target_root.is_some()
                        || handle.reachability != LegalArchiveReachability::default()))
                || (matches!(
                    handle.phase,
                    LegalArchiveRetentionPhase::Verified
                        | LegalArchiveRetentionPhase::DurableIngress
                        | LegalArchiveRetentionPhase::Committed
                ) && handle.target_root.is_none())
                || (matches!(
                    handle.phase,
                    LegalArchiveRetentionPhase::Committed
                        | LegalArchiveRetentionPhase::RejectedStale
                        | LegalArchiveRetentionPhase::Abandoned
                ) && handle.reachability != LegalArchiveReachability::default())
            {
                return Err(invalid("legal archive retention handle is inconsistent"));
            }
            if let Some(root) = &handle.target_root {
                validate_hash(root, "retained legal archive root")?;
            }
        }
        for root in self.committed_roots.keys() {
            validate_hash(root, "committed legal archive root")?;
        }
        Ok(())
    }

    fn transition(
        &mut self,
        handle_id: &str,
        expected: LegalArchiveRetentionPhase,
        next: LegalArchiveRetentionPhase,
    ) -> Result<(), CanwuError> {
        let handle = self
            .handles
            .get_mut(handle_id)
            .ok_or_else(|| invalid("legal archive retention handle is unknown"))?;
        if handle.phase == next {
            return Ok(());
        }
        if handle.phase != expected {
            return Err(invalid("legal archive retention transition is invalid"));
        }
        handle.phase = next;
        Ok(())
    }
}

pub trait LegalArchiveProvider {
    fn load_legal_archive(&self, blob_id: &str) -> Result<Option<LegalArchiveBlob>, CanwuError>;

    fn load_legal_archive_membership(
        &self,
        _version: &LegalVersionRef,
    ) -> Result<Option<LegalArchiveMembership>, CanwuError> {
        Ok(None)
    }

    fn load_legal_archive_index_directory(
        &self,
        _directory_id: &str,
    ) -> Result<Option<LegalArchiveIndexDirectory>, CanwuError> {
        Ok(None)
    }

    fn load_legal_archive_membership_page(
        &self,
        _page_id: &str,
    ) -> Result<Option<LegalArchiveMembershipPage>, CanwuError> {
        Ok(None)
    }

    fn load_legal_archive_temporal_page(
        &self,
        _page_id: &str,
    ) -> Result<Option<LegalArchiveTemporalPage>, CanwuError> {
        Ok(None)
    }
}

pub trait LegalArchiveStore: LegalArchiveProvider {
    fn load_legal_archive_retention_handle(
        &self,
        _handle_id: &str,
    ) -> Result<Option<LegalArchiveRetentionHandle>, CanwuError> {
        Err(invalid(
            "legal archive store does not expose its retention ledger",
        ))
    }

    fn prepare_legal_archive_retention(
        &self,
        _prepared: &PreparedLegalArchiveBatch,
    ) -> Result<String, CanwuError> {
        Err(invalid(
            "legal archive store does not provide a retention ledger",
        ))
    }

    fn verify_legal_archive_retention(
        &self,
        _handle_id: &str,
        _directory: &LegalArchiveIndexDirectory,
        _new_objects: &BTreeSet<ArchiveObjectId>,
    ) -> Result<(), CanwuError> {
        Err(invalid(
            "legal archive store does not provide a retention ledger",
        ))
    }

    fn mark_legal_archive_retention_durable(&self, _handle_id: &str) -> Result<(), CanwuError> {
        Err(invalid(
            "legal archive store does not provide a retention ledger",
        ))
    }

    fn commit_legal_archive_retention(
        &self,
        _handle_id: &str,
        _target_root: &str,
    ) -> Result<(), CanwuError> {
        Err(invalid(
            "legal archive store does not provide a retention ledger",
        ))
    }

    fn reject_stale_legal_archive_retention(&self, _handle_id: &str) -> Result<(), CanwuError> {
        Err(invalid(
            "legal archive store does not provide a retention ledger",
        ))
    }

    fn abandon_legal_archive_retention(&self, _handle_id: &str) -> Result<(), CanwuError> {
        Err(invalid(
            "legal archive store does not provide a retention ledger",
        ))
    }

    fn store_legal_archive(
        &self,
        blob: &LegalArchiveBlob,
    ) -> Result<LegalArchiveStoreOutcome, CanwuError>;

    fn store_legal_archive_membership(
        &self,
        membership: &LegalArchiveMembership,
    ) -> Result<LegalArchiveStoreOutcome, CanwuError>;

    fn store_legal_archive_index_directory(
        &self,
        _directory: &LegalArchiveIndexDirectory,
    ) -> Result<LegalArchiveStoreOutcome, CanwuError> {
        Err(invalid(
            "legal archive store does not support index directories",
        ))
    }

    fn store_legal_archive_membership_page(
        &self,
        _page: &LegalArchiveMembershipPage,
    ) -> Result<LegalArchiveStoreOutcome, CanwuError> {
        Err(invalid(
            "legal archive store does not support membership pages",
        ))
    }

    fn store_legal_archive_temporal_page(
        &self,
        _page: &LegalArchiveTemporalPage,
    ) -> Result<LegalArchiveStoreOutcome, CanwuError> {
        Err(invalid(
            "legal archive store does not support temporal pages",
        ))
    }
}

pub(crate) fn authenticate_legal_archive_root(
    provider: &dyn LegalArchiveProvider,
    directory_root: &str,
) -> Result<LegalArchiveReachability, CanwuError> {
    validate_hash(directory_root, "legal archive directory root")?;
    let directory = provider
        .load_legal_archive_index_directory(directory_root)?
        .ok_or_else(|| invalid("retained legal archive directory root is unavailable"))?;
    directory.validate()?;
    if directory.directory_id()? != directory_root {
        return Err(invalid(
            "retained legal archive directory root failed authentication",
        ));
    }
    let mut reachability = LegalArchiveReachability::default();
    reachability.directory_ids.insert(directory_root.to_owned());
    reachability
        .index_page_ids
        .insert(directory_root.to_owned());
    for (bucket, page_id) in &directory.membership_pages {
        let page = provider
            .load_legal_archive_membership_page(page_id)?
            .ok_or_else(|| invalid("retained legal archive membership page is unavailable"))?;
        page.validate()?;
        if page.shard != directory.shard || page.bucket != *bucket || page.page_id()? != *page_id {
            return Err(invalid(
                "retained legal archive membership page failed authentication",
            ));
        }
        reachability.membership_page_ids.insert(page_id.clone());
        reachability.index_page_ids.insert(page_id.clone());
        for membership in page.memberships {
            let LegalVersionLocation::Archived { receipt } = membership.location else {
                return Err(invalid("retained legal archive membership is not archived"));
            };
            validate_archive_receipt(&receipt)?;
            if receipt.owner_shard != directory.shard {
                return Err(invalid(
                    "retained legal archive membership receipt is misplaced",
                ));
            }
            let blob = provider
                .load_legal_archive(&receipt.object.blob_id)?
                .ok_or_else(|| invalid("retained legal archive blob is unavailable"))?;
            blob.validate()?;
            if blob.version != membership.version
                || blob.content_id()? != receipt.object.content_id
                || blob.blob_id()? != receipt.object.blob_id
            {
                return Err(invalid("retained legal archive blob failed authentication"));
            }
            reachability.objects.insert(receipt.object);
        }
    }
    for (axis, pages) in [
        (LegalTemporalAxis::Effective, &directory.effective_pages),
        (LegalTemporalAxis::Recorded, &directory.recorded_pages),
    ] {
        for (bucket, page_ids) in pages {
            for (segment, page_id) in page_ids.iter().enumerate() {
                let page = provider
                    .load_legal_archive_temporal_page(page_id)?
                    .ok_or_else(|| {
                        invalid("retained legal archive temporal page is unavailable")
                    })?;
                page.validate()?;
                if page.shard != directory.shard
                    || page.axis != axis
                    || page.bucket != *bucket
                    || page.segment != segment as u32
                    || page.page_id()? != *page_id
                {
                    return Err(invalid(
                        "retained legal archive temporal page failed authentication",
                    ));
                }
                reachability.temporal_page_ids.insert(page_id.clone());
                reachability.index_page_ids.insert(page_id.clone());
            }
        }
    }
    Ok(reachability)
}

impl PreparedLegalArchiveBatch {
    pub fn store_and_verify(
        &self,
        store: &dyn LegalArchiveStore,
    ) -> Result<VerifiedLegalArchiveCommit, CanwuError> {
        let retention_handle_id = store.prepare_legal_archive_retention(self)?;
        let result = self.store_and_verify_under_retention(store, &retention_handle_id);
        if result.is_err() {
            let _ = store.abandon_legal_archive_retention(&retention_handle_id);
        }
        result
    }

    fn store_and_verify_under_retention(
        &self,
        store: &dyn LegalArchiveStore,
        retention_handle_id: &str,
    ) -> Result<VerifiedLegalArchiveCommit, CanwuError> {
        if self.format_version != LEGAL_ARCHIVE_BLOB_FORMAT_VERSION
            || self.blobs.len() != self.compaction.candidates.len()
            || self.receipts.len() != self.blobs.len()
        {
            return Err(invalid("prepared legal archive batch is inconsistent"));
        }
        let mut directory = if let Some(previous) = &self.previous_archive_head {
            if previous.shard != self.compaction.shard
                || previous.committed_batch_count.checked_add(1)
                    != Some(self.compaction.archive_batch_sequence)
            {
                return Err(invalid(
                    "prepared legal archive previous head is inconsistent",
                ));
            }
            let directory = store
                .load_legal_archive_index_directory(&previous.membership_root)?
                .ok_or_else(|| invalid("previous legal archive index directory is unavailable"))?;
            directory.validate()?;
            if directory.directory_id()? != previous.membership_root
                || directory.effective_root()? != previous.effective_time_root
                || directory.recorded_root()? != previous.recorded_time_root
                || directory.archived_member_count != previous.archived_member_count
                || directory.shard != previous.shard
            {
                return Err(invalid(
                    "previous legal archive index directory failed root verification",
                ));
            }
            directory
        } else {
            if self.compaction.archive_batch_sequence != 1 {
                return Err(invalid("first legal archive batch must use sequence one"));
            }
            LegalArchiveIndexDirectory::empty(self.compaction.shard.clone())
        };
        let mut membership_pages =
            BTreeMap::<u16, BTreeMap<LegalVersionRef, LegalArchiveMembership>>::new();
        let mut effective_pages = BTreeMap::<u16, BTreeSet<LegalArchiveTemporalEntry>>::new();
        let mut recorded_pages = BTreeMap::<u16, BTreeSet<LegalArchiveTemporalEntry>>::new();
        let mut memberships = Vec::with_capacity(self.receipts.len());
        for ((candidate, blob), receipt) in self
            .compaction
            .candidates
            .iter()
            .zip(&self.blobs)
            .zip(&self.receipts)
        {
            blob.validate()?;
            if blob.version != candidate.version
                || blob.record_class != candidate.record_class
                || receipt.object.content_id != blob.content_id()?
                || receipt.object.blob_id != blob.blob_id()?
            {
                return Err(invalid(
                    "legal archive blob disagrees with its candidate or receipt",
                ));
            }
            let _ = store.store_legal_archive(blob)?;
            let loaded = store
                .load_legal_archive(&receipt.object.blob_id)?
                .ok_or_else(|| invalid("stored legal archive blob is not readable"))?;
            if loaded != *blob || loaded.blob_id()? != receipt.object.blob_id {
                return Err(invalid("stored legal archive blob failed verification"));
            }
            let (effective_interval, recorded_interval) = legal_archive_intervals(candidate, blob)?;
            let membership = LegalArchiveMembership {
                version: candidate.version.clone(),
                location: LegalVersionLocation::Archived {
                    receipt: Box::new(receipt.clone()),
                },
                effective_interval: Some(effective_interval),
                recorded_interval: Some(recorded_interval),
            };
            let _ = store.store_legal_archive_membership(&membership)?;
            if store.load_legal_archive_membership(&candidate.version)? != Some(membership) {
                return Err(invalid(
                    "stored legal archive membership failed verification",
                ));
            }
            let membership = store
                .load_legal_archive_membership(&candidate.version)?
                .ok_or_else(|| invalid("stored legal archive membership is unavailable"))?;
            let membership_bucket = legal_archive_index_bucket(
                "canwu.law.archive-membership-bucket.v1",
                &candidate.version,
            )?;
            let members = match membership_pages.entry(membership_bucket) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(load_membership_page(store, &directory, membership_bucket)?)
                }
            };
            if members
                .insert(candidate.version.clone(), membership.clone())
                .is_some()
            {
                return Err(invalid("legal archive membership was already indexed"));
            }
            let effective = membership
                .effective_interval
                .ok_or_else(|| invalid("archived membership lacks effective time"))?;
            let recorded = membership
                .recorded_interval
                .ok_or_else(|| invalid("archived membership lacks recorded time"))?;
            insert_temporal_entries(
                store,
                &directory,
                LegalTemporalAxis::Effective,
                effective,
                &candidate.version,
                &mut effective_pages,
            )?;
            insert_temporal_entries(
                store,
                &directory,
                LegalTemporalAxis::Recorded,
                recorded,
                &candidate.version,
                &mut recorded_pages,
            )?;
            memberships.push(membership);
        }
        for (bucket, members) in membership_pages {
            let page = LegalArchiveMembershipPage {
                format_version: LEGAL_ARCHIVE_INDEX_FORMAT_VERSION,
                shard: self.compaction.shard.clone(),
                bucket,
                memberships: members.into_values().collect(),
            };
            let page_id = page.page_id()?;
            let _ = store.store_legal_archive_membership_page(&page)?;
            let loaded = store
                .load_legal_archive_membership_page(&page_id)?
                .ok_or_else(|| invalid("stored legal membership page is unavailable"))?;
            if loaded != page || loaded.page_id()? != page_id {
                return Err(invalid("stored legal membership page failed verification"));
            }
            directory.membership_pages.insert(bucket, page_id);
        }
        persist_temporal_pages(
            store,
            &mut directory,
            LegalTemporalAxis::Effective,
            effective_pages,
        )?;
        persist_temporal_pages(
            store,
            &mut directory,
            LegalTemporalAxis::Recorded,
            recorded_pages,
        )?;
        directory.archived_member_count = directory
            .archived_member_count
            .checked_add(memberships.len() as u64)
            .ok_or_else(|| invalid("legal archive member count is exhausted"))?;
        let directory_id = directory.directory_id()?;
        let _ = store.store_legal_archive_index_directory(&directory)?;
        let loaded_directory = store
            .load_legal_archive_index_directory(&directory_id)?
            .ok_or_else(|| invalid("stored legal archive index directory is unavailable"))?;
        if loaded_directory != directory || loaded_directory.directory_id()? != directory_id {
            return Err(invalid(
                "stored legal archive index directory failed verification",
            ));
        }
        let archive_head = LegalArchiveHead {
            shard: self.compaction.shard.clone(),
            index_format_version: LEGAL_ARCHIVE_INDEX_FORMAT_VERSION,
            committed_batch_count: self.compaction.archive_batch_sequence,
            archived_member_count: directory.archived_member_count,
            membership_root: directory_id.clone(),
            effective_time_root: directory.effective_root()?,
            recorded_time_root: directory.recorded_root()?,
            last_content_id: self
                .receipts
                .last()
                .map(|receipt| receipt.object.content_id.clone())
                .or_else(|| {
                    self.previous_archive_head
                        .as_ref()
                        .and_then(|head| head.last_content_id.clone())
                }),
        };
        let commit = VerifiedLegalArchiveCommit {
            format_version: LEGAL_ARCHIVE_BLOB_FORMAT_VERSION,
            compaction: self.compaction.clone(),
            receipts: self.receipts.clone(),
            archive_head,
            pending_reachability: LegalPendingArchiveReachability {
                directory_root: directory_id,
            },
            retention_handle_id: retention_handle_id.to_owned(),
        };
        commit.validate()?;
        store.verify_legal_archive_retention(
            retention_handle_id,
            &directory,
            &self
                .receipts
                .iter()
                .map(|receipt| receipt.object.clone())
                .collect(),
        )?;
        Ok(commit)
    }
}

fn payload_time(payload: &serde_json::Value, field: &str) -> Result<Option<SimTime>, CanwuError> {
    let Some(value) = payload.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|error| {
            invalid(format!(
                "legal archive time field {field} is invalid: {error}"
            ))
        })
}

fn legal_archive_intervals(
    candidate: &LegalCompactionCandidate,
    blob: &LegalArchiveBlob,
) -> Result<(LegalTimeInterval, LegalTimeInterval), CanwuError> {
    let point = point_interval(candidate.closed_at)?;
    let interval = |start: SimTime, end_exclusive: Option<SimTime>| {
        let interval = LegalTimeInterval {
            start,
            end_exclusive,
        };
        interval.validate().map(|()| interval)
    };
    match blob.record_class.as_str() {
        "law_version" => {
            let adopted = payload_time(&blob.payload, "adopted_at")?
                .ok_or_else(|| invalid("archived law version lacks adopted_at"))?;
            let effective = payload_time(&blob.payload, "retrospective_from")?
                .or(payload_time(&blob.payload, "effective_at")?)
                .ok_or_else(|| invalid("archived law version lacks effective_at"))?;
            Ok((
                interval(effective, point.end_exclusive)?,
                interval(adopted, point.end_exclusive)?,
            ))
        }
        "source" => {
            let adopted = payload_time(&blob.payload, "adopted_at")?
                .ok_or_else(|| invalid("archived legal source lacks adopted_at"))?;
            let effective = payload_time(&blob.payload, "effective_at")?
                .ok_or_else(|| invalid("archived legal source lacks effective_at"))?;
            let end = payload_time(&blob.payload, "expires_at")?.or(point.end_exclusive);
            Ok((interval(effective, end)?, interval(adopted, end)?))
        }
        "conflict" => {
            let recorded = payload_time(&blob.payload, "recorded_at")?
                .ok_or_else(|| invalid("archived conflict lacks recorded_at"))?;
            let effective = payload_time(&blob.payload, "effective_from")?
                .ok_or_else(|| invalid("archived conflict lacks effective_from"))?;
            let end = payload_time(&blob.payload, "effective_until")?.or(point.end_exclusive);
            Ok((
                interval(effective, end)?,
                interval(recorded, point.end_exclusive)?,
            ))
        }
        "ruling" => {
            let effective = payload_time(&blob.payload, "effective_from")?
                .ok_or_else(|| invalid("archived ruling lacks effective_from"))?;
            let end = payload_time(&blob.payload, "effective_until")?.or(point.end_exclusive);
            Ok((
                interval(effective, end)?,
                interval(effective, point.end_exclusive)?,
            ))
        }
        "succession" => {
            let effective = payload_time(&blob.payload, "effective_at")?
                .ok_or_else(|| invalid("archived succession lacks effective_at"))?;
            Ok((
                interval(effective, point.end_exclusive)?,
                interval(effective, point.end_exclusive)?,
            ))
        }
        _ => Ok((point, point)),
    }
}

fn load_membership_page(
    provider: &dyn LegalArchiveProvider,
    directory: &LegalArchiveIndexDirectory,
    bucket: u16,
) -> Result<BTreeMap<LegalVersionRef, LegalArchiveMembership>, CanwuError> {
    let Some(page_id) = directory.membership_pages.get(&bucket) else {
        return Ok(BTreeMap::new());
    };
    let page = provider
        .load_legal_archive_membership_page(page_id)?
        .ok_or_else(|| invalid("legal archive membership page is unavailable"))?;
    page.validate()?;
    if page.shard != directory.shard || page.bucket != bucket || page.page_id()? != *page_id {
        return Err(invalid(
            "legal archive membership page failed directory verification",
        ));
    }
    Ok(page
        .memberships
        .into_iter()
        .map(|membership| (membership.version.clone(), membership))
        .collect())
}

fn load_temporal_page(
    provider: &dyn LegalArchiveProvider,
    directory: &LegalArchiveIndexDirectory,
    axis: LegalTemporalAxis,
    bucket: u16,
) -> Result<BTreeSet<LegalArchiveTemporalEntry>, CanwuError> {
    let pages = match axis {
        LegalTemporalAxis::Effective => &directory.effective_pages,
        LegalTemporalAxis::Recorded => &directory.recorded_pages,
    };
    let Some(page_ids) = pages.get(&bucket) else {
        return Ok(BTreeSet::new());
    };
    let mut entries = BTreeSet::new();
    for (segment, page_id) in page_ids.iter().enumerate() {
        let page = provider
            .load_legal_archive_temporal_page(page_id)?
            .ok_or_else(|| invalid("legal archive temporal page is unavailable"))?;
        page.validate()?;
        if page.shard != directory.shard
            || page.axis != axis
            || page.bucket != bucket
            || page.segment != segment as u32
            || page.page_id()? != *page_id
        {
            return Err(invalid(
                "legal archive temporal page failed directory verification",
            ));
        }
        for entry in page.entries {
            if !entries.insert(entry) {
                return Err(invalid(
                    "legal archive temporal segments contain a duplicate entry",
                ));
            }
        }
    }
    Ok(entries)
}

fn insert_temporal_entries(
    provider: &dyn LegalArchiveProvider,
    directory: &LegalArchiveIndexDirectory,
    axis: LegalTemporalAxis,
    interval: LegalTimeInterval,
    version: &LegalVersionRef,
    pages: &mut BTreeMap<u16, BTreeSet<LegalArchiveTemporalEntry>>,
) -> Result<(), CanwuError> {
    let domain = match axis {
        LegalTemporalAxis::Effective => "canwu.law.archive-effective-bucket.v1",
        LegalTemporalAxis::Recorded => "canwu.law.archive-recorded-bucket.v1",
    };
    for cell in decompose_legal_time_interval(interval)? {
        let bucket = legal_archive_index_bucket(domain, &cell)?;
        let entries = match pages.entry(bucket) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(load_temporal_page(provider, directory, axis, bucket)?)
            }
        };
        let entry = LegalArchiveTemporalEntry {
            cell,
            version: version.clone(),
            primary_member_commitment: version.content_commitment.clone(),
        };
        if !entries.insert(entry) {
            return Err(invalid("legal temporal archive entry was already indexed"));
        }
    }
    Ok(())
}

fn persist_temporal_pages(
    store: &dyn LegalArchiveStore,
    directory: &mut LegalArchiveIndexDirectory,
    axis: LegalTemporalAxis,
    pages: BTreeMap<u16, BTreeSet<LegalArchiveTemporalEntry>>,
) -> Result<(), CanwuError> {
    for (bucket, entries) in pages {
        let entries = entries.into_iter().collect::<Vec<_>>();
        let mut page_ids =
            Vec::with_capacity(entries.len().div_ceil(MAX_LEGAL_ARCHIVE_PAGE_ENTRIES));
        for (segment, chunk) in entries.chunks(MAX_LEGAL_ARCHIVE_PAGE_ENTRIES).enumerate() {
            let page = LegalArchiveTemporalPage {
                format_version: LEGAL_ARCHIVE_INDEX_FORMAT_VERSION,
                shard: directory.shard.clone(),
                axis,
                bucket,
                segment: u32::try_from(segment)
                    .map_err(|_| invalid("legal temporal segment range is exhausted"))?,
                entries: chunk.to_vec(),
            };
            let page_id = page.page_id()?;
            let _ = store.store_legal_archive_temporal_page(&page)?;
            let loaded = store
                .load_legal_archive_temporal_page(&page_id)?
                .ok_or_else(|| invalid("stored legal temporal page is unavailable"))?;
            if loaded != page || loaded.page_id()? != page_id {
                return Err(invalid("stored legal temporal page failed verification"));
            }
            page_ids.push(page_id);
        }
        match axis {
            LegalTemporalAxis::Effective => {
                directory.effective_pages.insert(bucket, page_ids);
            }
            LegalTemporalAxis::Recorded => {
                directory.recorded_pages.insert(bucket, page_ids);
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalStorageState {
    pub format_version: u32,
    #[serde(default = "default_true")]
    pub archived_membership_materialized: bool,
    #[serde(default)]
    pub directory: LegalDirectoryRecord,
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
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub(crate) compaction_order: BTreeSet<LegalCompactionOrderKey>,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "ordered_map_serde"
    )]
    pub(crate) candidate_accumulators: BTreeMap<LegalShardKey, LegalCandidateAccumulator>,
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
            archived_membership_materialized: true,
            directory: LegalDirectoryRecord {
                schema_version: LEGAL_STORAGE_FORMAT_VERSION,
                ..LegalDirectoryRecord::default()
            },
            heads: BTreeMap::new(),
            membership: BTreeMap::new(),
            compaction_candidates: BTreeMap::new(),
            compaction_order: BTreeSet::new(),
            candidate_accumulators: BTreeMap::new(),
            archive_heads: BTreeMap::new(),
            reachability: BTreeMap::new(),
        }
    }
}

impl LegalStorageState {
    pub(crate) fn shard_states(&self) -> Result<Vec<LegalStorageShardState>, CanwuError> {
        let mut shards = self
            .heads
            .keys()
            .map(|object| object.home_shard.clone())
            .chain(
                self.membership
                    .keys()
                    .map(|version| version.object.home_shard.clone()),
            )
            .chain(
                self.compaction_candidates
                    .keys()
                    .map(|version| version.object.home_shard.clone()),
            )
            .collect::<BTreeSet<_>>();
        shards.extend(self.candidate_accumulators.keys().cloned());
        let mut states = Vec::with_capacity(shards.len());
        for shard in shards {
            let state = LegalStorageShardState {
                format_version: LEGAL_STORAGE_FORMAT_VERSION,
                shard: shard.clone(),
                heads: self
                    .heads
                    .iter()
                    .filter(|(object, _)| object.home_shard == shard)
                    .map(|(object, head)| (object.clone(), head.clone()))
                    .collect(),
                membership: self
                    .membership
                    .iter()
                    .filter(|(version, membership)| {
                        version.object.home_shard == shard
                            && matches!(membership.location, LegalVersionLocation::Hot)
                    })
                    .map(|(version, membership)| (version.clone(), membership.clone()))
                    .collect(),
                compaction_candidates: self
                    .compaction_candidates
                    .iter()
                    .filter(|(version, _)| version.object.home_shard == shard)
                    .map(|(version, candidate)| (version.clone(), candidate.clone()))
                    .collect(),
                compaction_order: self
                    .compaction_order
                    .iter()
                    .filter(|key| key.shard == shard)
                    .cloned()
                    .collect(),
                candidate_accumulator: self.candidate_accumulators.get(&shard).cloned(),
            };
            state.validate()?;
            states.push(state);
        }
        Ok(states)
    }

    pub(crate) fn clear_sharded_hot_indexes(&mut self) {
        self.heads.clear();
        self.membership.clear();
        self.compaction_candidates.clear();
        self.compaction_order.clear();
        self.candidate_accumulators.clear();
    }

    pub(crate) fn install_shard_state(
        &mut self,
        state: LegalStorageShardState,
    ) -> Result<(), CanwuError> {
        state.validate()?;
        if state.heads.keys().any(|key| self.heads.contains_key(key))
            || state
                .membership
                .keys()
                .any(|key| self.membership.contains_key(key))
            || state
                .compaction_candidates
                .keys()
                .any(|key| self.compaction_candidates.contains_key(key))
            || state
                .compaction_order
                .iter()
                .any(|key| self.compaction_order.contains(key))
            || (state.candidate_accumulator.is_some()
                && self.candidate_accumulators.contains_key(&state.shard))
        {
            return Err(invalid("legal storage shard overlaps another shard"));
        }
        self.heads.extend(state.heads);
        self.membership.extend(state.membership);
        self.compaction_candidates
            .extend(state.compaction_candidates);
        self.compaction_order.extend(state.compaction_order);
        if let Some(accumulator) = state.candidate_accumulator {
            self.candidate_accumulators.insert(state.shard, accumulator);
        }
        Ok(())
    }

    #[must_use]
    pub(crate) fn has_compaction_candidates_for_shard(&self, shard: &LegalShardKey) -> bool {
        self.candidate_accumulators
            .get(shard)
            .is_some_and(|accumulator| accumulator.count > 0)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.heads.is_empty()
            && self.membership.is_empty()
            && self.compaction_candidates.is_empty()
            && self.compaction_order.is_empty()
            && self.candidate_accumulators.is_empty()
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
                effective_interval: None,
                recorded_interval: None,
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
        if let Some(existing) = self.compaction_candidates.get(&candidate.version) {
            return if existing == &candidate {
                Ok(())
            } else {
                Err(invalid(
                    "a legal version cannot have conflicting compaction metadata",
                ))
            };
        }
        let order = LegalCompactionOrderKey::from_candidate(&candidate);
        if candidate.is_eligible() && !self.compaction_order.insert(order) {
            return Err(invalid("legal compaction order contains a duplicate key"));
        }
        self.candidate_accumulators
            .entry(candidate.version.object.home_shard.clone())
            .or_default()
            .insert(&candidate)?;
        self.compaction_candidates
            .insert(candidate.version.clone(), candidate);
        Ok(())
    }

    fn remove_compaction_candidate(
        &mut self,
        version: &LegalVersionRef,
    ) -> Result<LegalCompactionCandidate, CanwuError> {
        let candidate = self
            .compaction_candidates
            .remove(version)
            .ok_or_else(|| invalid("legal compaction candidate disappeared"))?;
        if candidate.is_eligible()
            && !self
                .compaction_order
                .remove(&LegalCompactionOrderKey::from_candidate(&candidate))
        {
            return Err(invalid("legal compaction order lost a candidate"));
        }
        let shard = candidate.version.object.home_shard.clone();
        let accumulator = self
            .candidate_accumulators
            .get_mut(&shard)
            .ok_or_else(|| invalid("legal candidate accumulator disappeared"))?;
        accumulator.remove(&candidate)?;
        if accumulator.count == 0 {
            self.candidate_accumulators.remove(&shard);
        }
        Ok(candidate)
    }

    pub(crate) fn replace_compaction_candidate(
        &mut self,
        previous: &LegalVersionRef,
        candidate: LegalCompactionCandidate,
    ) -> Result<(), CanwuError> {
        let previous_candidate = self.remove_compaction_candidate(previous)?;
        if previous_candidate.version.object != candidate.version.object
            || !matches!(
                self.membership.remove(previous),
                Some(LegalArchiveMembership {
                    location: LegalVersionLocation::Hot,
                    ..
                })
            )
        {
            return Err(invalid(
                "legal compaction replacement changed object identity or lost hot membership",
            ));
        }
        self.membership.insert(
            candidate.version.clone(),
            LegalArchiveMembership {
                version: candidate.version.clone(),
                location: LegalVersionLocation::Hot,
                effective_interval: None,
                recorded_interval: None,
            },
        );
        self.mark_compaction_candidate(candidate)
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
        let mut selected = Vec::new();
        let mut source_bytes = 0_u64;
        let mut examined_candidates = 0_u64;
        for order in self
            .compaction_order
            .range(LegalCompactionOrderKey::shard_start(shard)..)
        {
            if order.shard != *shard {
                break;
            }
            if selected.len() == budgets.max_records {
                break;
            }
            examined_candidates = examined_candidates
                .checked_add(1)
                .ok_or_else(|| invalid("legal compaction examined count is exhausted"))?;
            let candidate = self
                .compaction_candidates
                .get(&order.version)
                .ok_or_else(|| invalid("legal compaction order lost its candidate"))?;
            if !candidate.is_eligible()
                || self
                    .heads
                    .get(&candidate.version.object)
                    .is_some_and(|head| head.version == candidate.version)
            {
                return Err(invalid(
                    "ineligible legal candidates must be removed from the ordered queue",
                ));
            }
            let next_bytes = source_bytes
                .checked_add(candidate.encoded_bytes)
                .ok_or_else(|| invalid("legal compaction byte count overflowed"))?;
            if next_bytes > budgets.max_source_bytes {
                if selected.is_empty() {
                    return Err(invalid(
                        "legal compaction byte budget cannot fit the earliest candidate",
                    ));
                }
                break;
            }
            source_bytes = next_bytes;
            selected.push(candidate.clone());
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
                examined_candidates,
            ),
        )?;
        Ok(Some(PreparedLegalCompaction {
            token,
            shard: shard.clone(),
            archive_batch_sequence,
            source_membership_root,
            candidates: selected,
            source_bytes,
            examined_candidates,
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

    pub(crate) fn commit_verified_compaction(
        &mut self,
        prepared: &PreparedLegalCompaction,
        receipts: &[ArchiveObjectReceipt],
        archive_head: &LegalArchiveHead,
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
        if expected != *prepared || receipts.len() != prepared.candidates.len() {
            return Err(invalid("verified legal compaction is stale or incomplete"));
        }
        let previous = self.archive_heads.get(&prepared.shard);
        let previous_count = previous.map_or(0, |head| head.archived_member_count);
        validate_archive_head(archive_head)?;
        if archive_head.shard != prepared.shard
            || archive_head.index_format_version != LEGAL_ARCHIVE_INDEX_FORMAT_VERSION
            || archive_head.committed_batch_count != prepared.archive_batch_sequence
            || archive_head.archived_member_count
                != previous_count
                    .checked_add(receipts.len() as u64)
                    .ok_or_else(|| invalid("legal archive member count is exhausted"))?
            || archive_head.last_content_id.as_ref()
                != receipts.last().map(|receipt| &receipt.object.content_id)
        {
            return Err(invalid("verified legal archive head is inconsistent"));
        }
        for (index, (candidate, receipt)) in prepared.candidates.iter().zip(receipts).enumerate() {
            let member_index = u64::try_from(index)
                .map_err(|_| invalid("legal archive member index exceeds u64"))?;
            if receipt.owner_shard != prepared.shard
                || receipt.archive_batch_sequence != prepared.archive_batch_sequence
                || receipt.member_index != member_index
                || receipt.source_root != prepared.source_membership_root
                || receipt.verified_plan_hash != prepared.token
                || receipt.stored_bytes == 0
                || receipt.decoded_bytes == 0
                || self.reachability.get(&receipt.object)
                    != Some(&ArchiveReachabilityState::DurableIngress)
                || self.compaction_candidates.get(&candidate.version) != Some(candidate)
            {
                return Err(invalid(
                    "verified legal archive receipt disagrees with hot compaction state",
                ));
            }
            validate_archive_receipt(receipt)?;
        }

        for (candidate, receipt) in prepared.candidates.iter().zip(receipts) {
            self.membership.insert(
                candidate.version.clone(),
                LegalArchiveMembership {
                    version: candidate.version.clone(),
                    location: LegalVersionLocation::Archived {
                        receipt: Box::new(receipt.clone()),
                    },
                    effective_interval: Some(point_interval(candidate.closed_at)?),
                    recorded_interval: Some(point_interval(candidate.closed_at)?),
                },
            );
            self.remove_compaction_candidate(&candidate.version)?;
        }
        for object in receipts
            .iter()
            .map(|receipt| receipt.object.clone())
            .collect::<BTreeSet<_>>()
        {
            self.reachability
                .insert(object, ArchiveReachabilityState::Committed);
        }
        self.archive_heads
            .insert(prepared.shard.clone(), archive_head.clone());
        self.archived_membership_materialized = false;
        Ok(())
    }

    fn commit_compaction_in_place(
        &mut self,
        prepared: &PreparedLegalCompaction,
        receipts: Vec<ArchiveObjectReceipt>,
    ) -> Result<(), CanwuError> {
        let previous_head = self.archive_heads.get(&prepared.shard).cloned();
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
                    effective_interval: Some(point_interval(candidate.closed_at)?),
                    recorded_interval: Some(point_interval(candidate.closed_at)?),
                },
            );
            self.remove_compaction_candidate(&candidate.version)?;
        }
        for object in receipts
            .iter()
            .map(|receipt| receipt.object.clone())
            .collect::<BTreeSet<_>>()
        {
            self.reachability
                .insert(object, ArchiveReachabilityState::Committed);
        }

        let (archived_member_count, membership_root, effective_time_root, recorded_time_root) =
            if self.archived_membership_materialized {
                let archived_member_count = self
                    .membership
                    .values()
                    .filter(|membership| {
                        membership.version.object.home_shard == prepared.shard
                            && matches!(membership.location, LegalVersionLocation::Archived { .. })
                    })
                    .count();
                let archived_member_count = u64::try_from(archived_member_count).map_err(|_| {
                    invalid("legal archive member count exceeds the persistent range")
                })?;
                let membership_root = self.archived_membership_root_for_shard(&prepared.shard)?;
                let temporal = self.archived_temporal_index_for_shard(&prepared.shard)?;
                let (effective_time_root, recorded_time_root) = temporal.roots()?;
                (
                    archived_member_count,
                    membership_root,
                    effective_time_root,
                    recorded_time_root,
                )
            } else {
                let archived_member_count = previous_head
                    .as_ref()
                    .map_or(0, |head| head.archived_member_count)
                    .checked_add(receipts.len() as u64)
                    .ok_or_else(|| invalid("legal archive member count is exhausted"))?;
                let (membership_root, effective_time_root, recorded_time_root) =
                    append_archive_roots(previous_head.as_ref(), prepared, &receipts)?;
                (
                    archived_member_count,
                    membership_root,
                    effective_time_root,
                    recorded_time_root,
                )
            };
        let last_content_id = receipts
            .last()
            .map(|receipt| receipt.object.content_id.clone());
        self.archive_heads.insert(
            prepared.shard.clone(),
            LegalArchiveHead {
                shard: prepared.shard.clone(),
                index_format_version: 0,
                committed_batch_count: prepared.archive_batch_sequence,
                archived_member_count,
                membership_root,
                effective_time_root,
                recorded_time_root,
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
        if self.directory.schema_version != LEGAL_STORAGE_FORMAT_VERSION
            || self
                .directory
                .active_shards
                .iter()
                .any(|shard| self.directory.archive_only_shards.contains(shard))
        {
            return Err(invalid("legal shard directory is inconsistent"));
        }
        for shard in self
            .directory
            .active_shards
            .iter()
            .chain(self.directory.archive_only_shards.iter())
        {
            shard.validate()?;
        }
        let known_shards = self
            .directory
            .active_shards
            .iter()
            .chain(self.directory.archive_only_shards.iter())
            .cloned()
            .collect::<BTreeSet<_>>();
        for (object, shard) in &self.directory.object_routes {
            require_identifier(object, "legal routing object ID")?;
            if !known_shards.contains(shard) {
                return Err(invalid("legal object route targets an unknown shard"));
            }
        }
        for shards in self.directory.due_shards.values() {
            if shards.is_empty() || shards.iter().any(|shard| !known_shards.contains(shard)) {
                return Err(invalid("legal due-work route targets an unknown shard"));
            }
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
                let effective = membership.effective_interval.ok_or_else(|| {
                    invalid("archived legal membership lacks an effective interval")
                })?;
                let recorded = membership.recorded_interval.ok_or_else(|| {
                    invalid("archived legal membership lacks a recorded interval")
                })?;
                effective.validate()?;
                recorded.validate()?;
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
            } else if membership.effective_interval.is_some()
                || membership.recorded_interval.is_some()
            {
                return Err(invalid(
                    "hot legal membership cannot claim archived temporal placement",
                ));
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
        let expected_order = self
            .compaction_candidates
            .values()
            .filter(|candidate| candidate.is_eligible())
            .map(LegalCompactionOrderKey::from_candidate)
            .collect::<BTreeSet<_>>();
        if self.compaction_order != expected_order {
            return Err(invalid("legal compaction ordered index is inconsistent"));
        }
        let mut expected_accumulators = BTreeMap::<LegalShardKey, LegalCandidateAccumulator>::new();
        for candidate in self.compaction_candidates.values() {
            expected_accumulators
                .entry(candidate.version.object.home_shard.clone())
                .or_default()
                .insert(candidate)?;
        }
        if self.candidate_accumulators != expected_accumulators {
            return Err(invalid("legal candidate accumulator is inconsistent"));
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
            if !self.archived_membership_materialized {
                let local = self
                    .membership
                    .values()
                    .filter_map(|membership| match &membership.location {
                        LegalVersionLocation::Archived { receipt }
                            if membership.version.object.home_shard == *shard =>
                        {
                            Some(receipt)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if head.archived_member_count < local.len() as u64
                    || local
                        .iter()
                        .any(|receipt| receipt.archive_batch_sequence > head.committed_batch_count)
                {
                    return Err(invalid(
                        "root-only legal archive head omits locally materialized receipts",
                    ));
                }
                continue;
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
                || head.last_content_id.as_ref() != terminal_content_id
            {
                return Err(invalid("legal archive head is inconsistent"));
            }
            if head.index_format_version == 0 {
                let temporal = self.archived_temporal_index_for_shard(shard)?;
                let (effective_time_root, recorded_time_root) = temporal.roots()?;
                if head.membership_root != self.archived_membership_root_for_shard(shard)?
                    || head.effective_time_root != effective_time_root
                    || head.recorded_time_root != recorded_time_root
                {
                    return Err(invalid("legacy legal archive head is inconsistent"));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn source_membership_root_for_shard(
        &self,
        shard: &LegalShardKey,
    ) -> Result<String, CanwuError> {
        // The compaction token authenticates the exact *hot candidate set*,
        // not whichever archived membership pages happen to be materialized in
        // this process. Root-only restore deliberately omits archived
        // membership, so including it here made an otherwise identical next
        // batch acquire a different token after restart.
        canonical_hash(
            SOURCE_MEMBERSHIP_ROOT_DOMAIN,
            &(
                shard,
                self.candidate_accumulators
                    .get(shard)
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
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

    fn archived_temporal_index_for_shard(
        &self,
        shard: &LegalShardKey,
    ) -> Result<LegalBitemporalIndex, CanwuError> {
        let mut index = LegalBitemporalIndex::default();
        for membership in self.membership.values().filter(|membership| {
            membership.version.object.home_shard == *shard
                && matches!(membership.location, LegalVersionLocation::Archived { .. })
        }) {
            index.insert(
                membership.effective_interval.ok_or_else(|| {
                    invalid("archived legal membership lacks an effective interval")
                })?,
                membership.recorded_interval.ok_or_else(|| {
                    invalid("archived legal membership lacks a recorded interval")
                })?,
                &membership.version,
                &membership.version.content_commitment,
            )?;
        }
        Ok(index)
    }

    pub fn archived_versions_at(
        &self,
        shard: &LegalShardKey,
        effective_at: SimTime,
        recorded_at: SimTime,
        budget: LegalTemporalQueryBudget,
    ) -> Result<Vec<LegalVersionRef>, CanwuError> {
        shard.validate()?;
        self.archived_temporal_index_for_shard(shard)?.point_query(
            effective_at,
            recorded_at,
            budget,
        )
    }

    pub fn authenticated_archive_directory(
        &self,
        shard: &LegalShardKey,
        provider: &dyn LegalArchiveProvider,
    ) -> Result<Option<LegalArchiveIndexDirectory>, CanwuError> {
        let Some(head) = self.archive_heads.get(shard) else {
            return Ok(None);
        };
        if head.index_format_version != LEGAL_ARCHIVE_INDEX_FORMAT_VERSION {
            return Err(invalid(
                "legal archive head does not commit an authenticated sparse index",
            ));
        }
        let directory = provider
            .load_legal_archive_index_directory(&head.membership_root)?
            .ok_or_else(|| invalid("committed legal archive index directory is unavailable"))?;
        directory.validate()?;
        if directory.shard != *shard
            || directory.directory_id()? != head.membership_root
            || directory.effective_root()? != head.effective_time_root
            || directory.recorded_root()? != head.recorded_time_root
            || directory.archived_member_count != head.archived_member_count
        {
            return Err(invalid(
                "legal archive index directory disagrees with its committed head",
            ));
        }
        Ok(Some(directory))
    }

    pub fn authenticated_archive_membership(
        &self,
        version: &LegalVersionRef,
        provider: &dyn LegalArchiveProvider,
    ) -> Result<Option<LegalArchiveMembership>, CanwuError> {
        let Some(directory) =
            self.authenticated_archive_directory(&version.object.home_shard, provider)?
        else {
            return Ok(None);
        };
        let bucket = legal_archive_index_bucket("canwu.law.archive-membership-bucket.v1", version)?;
        let members = load_membership_page(provider, &directory, bucket)?;
        Ok(members.get(version).cloned())
    }

    pub(crate) fn authenticated_archive_membership_with_meter(
        &self,
        version: &LegalVersionRef,
        provider: &dyn LegalArchiveProvider,
        budget: LegalTemporalQueryBudget,
        meter: &mut LegalTemporalQueryMeter,
    ) -> Result<Option<LegalArchiveMembership>, CanwuError> {
        let Some(head) = self.archive_heads.get(&version.object.home_shard) else {
            return Ok(None);
        };
        meter.begin_provider_call(budget, false)?;
        let directory = provider
            .load_legal_archive_index_directory(&head.membership_root)?
            .ok_or_else(|| invalid("committed legal archive index directory is unavailable"))?;
        meter.record_decoded(&directory, budget)?;
        directory.validate()?;
        if directory.shard != version.object.home_shard
            || directory.directory_id()? != head.membership_root
            || directory.effective_root()? != head.effective_time_root
            || directory.recorded_root()? != head.recorded_time_root
            || directory.archived_member_count != head.archived_member_count
        {
            return Err(invalid(
                "legal archive index directory disagrees with its committed head",
            ));
        }
        let bucket = legal_archive_index_bucket("canwu.law.archive-membership-bucket.v1", version)?;
        let Some(page_id) = directory.membership_pages.get(&bucket) else {
            return Ok(None);
        };
        meter.begin_provider_call(budget, false)?;
        let page = provider
            .load_legal_archive_membership_page(page_id)?
            .ok_or_else(|| invalid("legal archive membership page is unavailable"))?;
        meter.record_decoded(&page, budget)?;
        page.validate()?;
        if page.shard != directory.shard || page.bucket != bucket || page.page_id()? != *page_id {
            return Err(invalid(
                "legal archive membership page failed directory verification",
            ));
        }
        Ok(page
            .memberships
            .into_iter()
            .find(|membership| membership.version == *version))
    }

    pub fn archived_versions_at_with_provider_usage(
        &self,
        shard: &LegalShardKey,
        effective_at: SimTime,
        recorded_at: SimTime,
        budget: LegalTemporalQueryBudget,
        provider: &dyn LegalArchiveProvider,
    ) -> Result<(Vec<LegalVersionRef>, LegalTemporalQueryUsage), CanwuError> {
        LegalTemporalQueryMeter::validate_budget(budget)?;
        let mut meter = LegalTemporalQueryMeter::default();
        let versions = self.archived_versions_at_with_provider_meter(
            shard,
            effective_at,
            recorded_at,
            budget,
            provider,
            &mut meter,
        )?;
        Ok((versions, meter.usage()))
    }

    pub(crate) fn archived_versions_at_with_provider_meter(
        &self,
        shard: &LegalShardKey,
        effective_at: SimTime,
        recorded_at: SimTime,
        budget: LegalTemporalQueryBudget,
        provider: &dyn LegalArchiveProvider,
        meter: &mut LegalTemporalQueryMeter,
    ) -> Result<Vec<LegalVersionRef>, CanwuError> {
        LegalTemporalQueryMeter::validate_budget(budget)?;
        let Some(head) = self.archive_heads.get(shard) else {
            return Ok(Vec::new());
        };
        if head.index_format_version != LEGAL_ARCHIVE_INDEX_FORMAT_VERSION {
            return Err(invalid(
                "legal archive head does not commit an authenticated sparse index",
            ));
        }
        meter.begin_provider_call(budget, false)?;
        let directory = provider
            .load_legal_archive_index_directory(&head.membership_root)?
            .ok_or_else(|| invalid("committed legal archive index directory is unavailable"))?;
        meter.record_decoded(&directory, budget)?;
        directory.validate()?;
        if directory.shard != *shard
            || directory.directory_id()? != head.membership_root
            || directory.effective_root()? != head.effective_time_root
            || directory.recorded_root()? != head.recorded_time_root
            || directory.archived_member_count != head.archived_member_count
        {
            return Err(invalid(
                "legal archive index directory disagrees with its committed head",
            ));
        }
        let mut load_axis = |axis: LegalTemporalAxis,
                             at: SimTime|
         -> Result<BTreeMap<LegalVersionRef, String>, CanwuError> {
            let encoded = encode_time(at);
            let domain = match axis {
                LegalTemporalAxis::Effective => "canwu.law.archive-effective-bucket.v1",
                LegalTemporalAxis::Recorded => "canwu.law.archive-recorded-bucket.v1",
            };
            let mut by_bucket = BTreeMap::<u16, BTreeSet<LegalDyadicCell>>::new();
            for prefix_length in 0..=LEGAL_TEMPORAL_WIDTH {
                let cell = LegalDyadicCell {
                    prefix_bits: masked_prefix(encoded, prefix_length),
                    prefix_length,
                };
                by_bucket
                    .entry(legal_archive_index_bucket(domain, &cell)?)
                    .or_default()
                    .insert(cell);
            }
            let mut candidates = BTreeMap::new();
            for (bucket, cells) in by_bucket {
                let pages = match axis {
                    LegalTemporalAxis::Effective => &directory.effective_pages,
                    LegalTemporalAxis::Recorded => &directory.recorded_pages,
                };
                let Some(page_ids) = pages.get(&bucket) else {
                    continue;
                };
                for (segment, page_id) in page_ids.iter().enumerate() {
                    meter.begin_provider_call(budget, true)?;
                    let page = provider
                        .load_legal_archive_temporal_page(page_id)?
                        .ok_or_else(|| invalid("legal archive temporal page is unavailable"))?;
                    meter.record_decoded(&page, budget)?;
                    page.validate()?;
                    if page.shard != directory.shard
                        || page.axis != axis
                        || page.bucket != bucket
                        || page.segment != segment as u32
                        || page.page_id()? != *page_id
                    {
                        return Err(invalid(
                            "legal archive temporal page failed directory verification",
                        ));
                    }
                    for entry in page.entries {
                        if !cells.contains(&entry.cell) {
                            continue;
                        }
                        if candidates.len() == budget.max_candidates_per_dimension
                            && !candidates.contains_key(&entry.version)
                        {
                            return Err(query_budget_error(
                                "legal temporal provider query exceeded its candidate budget",
                            ));
                        }
                        if candidates
                            .insert(
                                entry.version.clone(),
                                entry.primary_member_commitment.clone(),
                            )
                            .is_some_and(|previous| previous != entry.primary_member_commitment)
                        {
                            return Err(invalid(
                                "legal temporal provider pages disagree about a primary member",
                            ));
                        }
                    }
                }
            }
            Ok(candidates)
        };
        let effective = load_axis(LegalTemporalAxis::Effective, effective_at)?;
        let recorded = load_axis(LegalTemporalAxis::Recorded, recorded_at)?;
        let mut result = Vec::new();
        for (version, commitment) in effective {
            if recorded.get(&version) == Some(&commitment) {
                if result.len() == budget.max_intersection_members {
                    return Err(query_budget_error(
                        "legal temporal provider query exceeded its intersection budget",
                    ));
                }
                result.push(version);
            }
        }
        Ok(result)
    }

    pub fn archived_versions_at_with_provider(
        &self,
        shard: &LegalShardKey,
        effective_at: SimTime,
        recorded_at: SimTime,
        budget: LegalTemporalQueryBudget,
        provider: &dyn LegalArchiveProvider,
    ) -> Result<Vec<LegalVersionRef>, CanwuError> {
        self.archived_versions_at_with_provider_usage(
            shard,
            effective_at,
            recorded_at,
            budget,
            provider,
        )
        .map(|(versions, _)| versions)
    }

    pub fn reachable_archive_object_ids_with_provider(
        &self,
        provider: &dyn LegalArchiveProvider,
    ) -> Result<BTreeSet<ArchiveObjectId>, CanwuError> {
        Ok(self.archive_reachability_with_provider(provider)?.objects)
    }

    pub fn archive_reachability_with_provider(
        &self,
        provider: &dyn LegalArchiveProvider,
    ) -> Result<LegalArchiveReachability, CanwuError> {
        let mut reachable = LegalArchiveReachability {
            objects: self.leased_archive_object_ids(),
            index_page_ids: BTreeSet::new(),
            directory_ids: BTreeSet::new(),
            membership_page_ids: BTreeSet::new(),
            temporal_page_ids: BTreeSet::new(),
        };
        for shard in self.archive_heads.keys() {
            let Some(directory) = self.authenticated_archive_directory(shard, provider)? else {
                continue;
            };
            let directory_id = directory.directory_id()?;
            reachable.index_page_ids.insert(directory_id.clone());
            reachable.directory_ids.insert(directory_id);
            reachable
                .membership_page_ids
                .extend(directory.membership_pages.values().cloned());
            reachable.temporal_page_ids.extend(
                directory
                    .effective_pages
                    .values()
                    .flatten()
                    .chain(directory.recorded_pages.values().flatten())
                    .cloned(),
            );
            reachable
                .index_page_ids
                .extend(reachable.membership_page_ids.iter().cloned());
            reachable
                .index_page_ids
                .extend(reachable.temporal_page_ids.iter().cloned());
            for bucket in directory.membership_pages.keys().copied() {
                for membership in load_membership_page(provider, &directory, bucket)?.into_values()
                {
                    if let LegalVersionLocation::Archived { receipt } = membership.location {
                        reachable.objects.insert(receipt.object);
                    }
                }
            }
        }
        Ok(reachable)
    }
}

fn point_interval(at: SimTime) -> Result<LegalTimeInterval, CanwuError> {
    Ok(LegalTimeInterval {
        start: at,
        end_exclusive: (at.as_minutes() != i64::MAX)
            .then(|| SimTime::from_minutes(at.as_minutes() + 1)),
    })
}

fn append_archive_roots(
    previous: Option<&LegalArchiveHead>,
    prepared: &PreparedLegalCompaction,
    receipts: &[ArchiveObjectReceipt],
) -> Result<(String, String, String), CanwuError> {
    if receipts.len() != prepared.candidates.len() {
        return Err(invalid(
            "legal archive append roots require complete batch receipts",
        ));
    }
    let genesis = canonical_hash("canwu.law.archive-root-genesis.v1", &prepared.shard)?;
    let previous_membership =
        previous.map_or(genesis.as_str(), |head| head.membership_root.as_str());
    let previous_effective =
        previous.map_or(genesis.as_str(), |head| head.effective_time_root.as_str());
    let previous_recorded =
        previous.map_or(genesis.as_str(), |head| head.recorded_time_root.as_str());
    let members = prepared
        .candidates
        .iter()
        .zip(receipts)
        .map(|(candidate, receipt)| {
            Ok((
                &candidate.version,
                receipt,
                point_interval(candidate.closed_at)?,
            ))
        })
        .collect::<Result<Vec<_>, CanwuError>>()?;
    Ok((
        canonical_hash(
            MEMBERSHIP_APPEND_ROOT_DOMAIN,
            &(
                previous_membership,
                &prepared.shard,
                prepared.archive_batch_sequence,
                &members,
            ),
        )?,
        canonical_hash(
            EFFECTIVE_APPEND_ROOT_DOMAIN,
            &(
                previous_effective,
                &prepared.shard,
                prepared.archive_batch_sequence,
                &members,
            ),
        )?,
        canonical_hash(
            RECORDED_APPEND_ROOT_DOMAIN,
            &(
                previous_recorded,
                &prepared.shard,
                prepared.archive_batch_sequence,
                &members,
            ),
        )?,
    ))
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
    if head.index_format_version > LEGAL_ARCHIVE_INDEX_FORMAT_VERSION
        || head.committed_batch_count == 0
        || head.archived_member_count == 0
    {
        return Err(invalid(
            "legal archive heads must describe committed members",
        ));
    }
    validate_hash(&head.membership_root, "legal archive membership root")?;
    validate_hash(
        &head.effective_time_root,
        "legal archive effective-time root",
    )?;
    validate_hash(&head.recorded_time_root, "legal archive recorded-time root")?;
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
    fn archive_retention_prepare_interlocks_gc_and_survives_restart() {
        let mut state = LegalStorageState::default();
        let old_version = version("retention", 1);
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
        state
            .mark_compaction_candidate(LegalCompactionCandidate {
                version: old_version,
                record_class: "law_version".to_owned(),
                closed_at: SimTime::EPOCH,
                encoded_bytes: 1,
                dependencies_resolved: true,
                current_projection_retained: true,
            })
            .unwrap();
        let compaction = state
            .select_compaction_batch(
                &LegalShardKey::order("order"),
                LegalCompactionBudgets {
                    max_records: 1,
                    max_source_bytes: 1,
                },
            )
            .unwrap()
            .unwrap();
        let batch = PreparedLegalArchiveBatch {
            format_version: LEGAL_ARCHIVE_BLOB_FORMAT_VERSION,
            compaction,
            blobs: Vec::new(),
            receipts: Vec::new(),
            previous_archive_head: None,
        };
        let mut ledger = LegalArchiveRetentionLedger::default();
        let handle = ledger.prepare(&batch).unwrap();
        assert_eq!(
            ledger.begin_gc_epoch().unwrap_err().code,
            ErrorCode::ArchiveNotReady
        );
        let restarted: LegalArchiveRetentionLedger =
            serde_json::from_str(&serde_json::to_string(&ledger).unwrap()).unwrap();
        assert_eq!(restarted, ledger);
        ledger.abandon(&handle).unwrap();
        assert_eq!(ledger.begin_gc_epoch().unwrap(), 1);
    }

    #[test]
    fn archive_retention_root_handoff_retires_superseded_root_and_compacts_handle() {
        fn reachability(object_byte: char, root_byte: char) -> LegalArchiveReachability {
            let root = hash(root_byte);
            LegalArchiveReachability {
                objects: BTreeSet::from([ArchiveObjectId {
                    content_id: hash(object_byte),
                    blob_id: hash(char::from_u32(u32::from(object_byte) + 1).unwrap()),
                }]),
                index_page_ids: BTreeSet::from([root.clone()]),
                directory_ids: BTreeSet::from([root]),
                membership_page_ids: BTreeSet::new(),
                temporal_page_ids: BTreeSet::new(),
            }
        }

        let old_root = hash('1');
        let new_root = hash('2');
        let old_reachability = reachability('a', '1');
        let new_delta = reachability('c', '2');
        let mut committed_reachability = new_delta.clone();
        committed_reachability
            .objects
            .extend(old_reachability.objects.iter().cloned());
        let old_handle_id = hash('3');
        let new_handle_id = hash('4');
        let mut ledger = LegalArchiveRetentionLedger {
            committed_roots: BTreeMap::from([(old_root.clone(), old_reachability.clone())]),
            ..LegalArchiveRetentionLedger::default()
        };
        ledger.handles.insert(
            old_handle_id.clone(),
            LegalArchiveRetentionHandle {
                format_version: LEGAL_ARCHIVE_RETENTION_FORMAT_VERSION,
                handle_id: old_handle_id,
                compaction_token: hash('5'),
                source_root: hash('6'),
                previous_root: None,
                target_root: Some(old_root.clone()),
                reachability: LegalArchiveReachability::default(),
                prepared_epoch: 0,
                phase: LegalArchiveRetentionPhase::Committed,
            },
        );
        ledger.handles.insert(
            new_handle_id.clone(),
            LegalArchiveRetentionHandle {
                format_version: LEGAL_ARCHIVE_RETENTION_FORMAT_VERSION,
                handle_id: new_handle_id.clone(),
                compaction_token: hash('7'),
                source_root: hash('8'),
                previous_root: Some(old_root.clone()),
                target_root: Some(new_root.clone()),
                reachability: new_delta,
                prepared_epoch: 0,
                phase: LegalArchiveRetentionPhase::DurableIngress,
            },
        );

        ledger.commit(&new_handle_id, &new_root).unwrap();
        ledger.commit(&new_handle_id, &new_root).unwrap();

        assert_eq!(
            ledger.committed_roots,
            BTreeMap::from([(new_root.clone(), committed_reachability.clone())])
        );
        assert!(!ledger.reachable().index_page_ids.contains(&old_root));
        assert_eq!(ledger.reachable(), committed_reachability);
        assert!(
            ledger
                .handles
                .values()
                .all(|handle| handle.reachability == LegalArchiveReachability::default())
        );
        let restarted: LegalArchiveRetentionLedger =
            serde_json::from_str(&serde_json::to_string(&ledger).unwrap()).unwrap();
        restarted.validate().unwrap();
        assert_eq!(restarted, ledger);
    }

    #[test]
    fn one_temporal_meter_covers_multiple_shards_membership_and_blob_hydration() {
        let provider = LegalTemporalScaleProvider::default();
        let mut root_only = LegalStorageState {
            archived_membership_materialized: false,
            ..LegalStorageState::default()
        };
        let mut versions = Vec::new();
        for (ordinal, shard) in [
            (1_u64, LegalShardKey::order("first")),
            (2_u64, LegalShardKey::order("second")),
        ] {
            let (candidate, blob) = format8_legal_scale_candidate(ordinal, &shard).unwrap();
            let compaction = PreparedLegalCompaction {
                token: canonical_hash("canwu.test.legal-compaction.v1", &(ordinal, &shard))
                    .unwrap(),
                shard: shard.clone(),
                archive_batch_sequence: 1,
                source_membership_root: canonical_hash(
                    "canwu.test.legal-source-root.v1",
                    &(ordinal, &shard),
                )
                .unwrap(),
                candidates: vec![candidate.clone()],
                source_bytes: candidate.encoded_bytes,
                examined_candidates: 1,
            };
            let receipt = ArchiveObjectReceipt {
                object: ArchiveObjectId {
                    content_id: blob.content_id().unwrap(),
                    blob_id: blob.blob_id().unwrap(),
                },
                owner_shard: shard.clone(),
                archive_batch_sequence: 1,
                member_index: 0,
                codec: "json-canonical-v1".to_owned(),
                stored_bytes: candidate.encoded_bytes,
                decoded_bytes: candidate.encoded_bytes,
                source_root: compaction.source_membership_root.clone(),
                verified_plan_hash: compaction.token.clone(),
            };
            let verified = PreparedLegalArchiveBatch {
                format_version: LEGAL_ARCHIVE_BLOB_FORMAT_VERSION,
                compaction,
                blobs: vec![blob],
                receipts: vec![receipt],
                previous_archive_head: None,
            }
            .store_and_verify(&provider)
            .unwrap();
            root_only.archive_heads.insert(shard, verified.archive_head);
            versions.push(candidate.version);
        }
        let at = SimTime::EPOCH;
        let budget = LegalTemporalQueryBudget {
            max_provider_calls: 3,
            ..LegalTemporalQueryBudget::default()
        };
        let mut meter = LegalTemporalQueryMeter::default();
        root_only
            .archived_versions_at_with_provider_meter(
                &LegalShardKey::order("first"),
                at,
                at,
                budget,
                &provider,
                &mut meter,
            )
            .unwrap();
        assert_eq!(
            root_only
                .archived_versions_at_with_provider_meter(
                    &LegalShardKey::order("second"),
                    SimTime::from_minutes(1),
                    SimTime::from_minutes(1),
                    budget,
                    &provider,
                    &mut meter,
                )
                .unwrap_err()
                .code,
            ErrorCode::QueryBudgetExceeded
        );

        let hydration_budget = LegalTemporalQueryBudget {
            max_provider_calls: 5,
            ..LegalTemporalQueryBudget::default()
        };
        let mut hydration_meter = LegalTemporalQueryMeter::default();
        root_only
            .archived_versions_at_with_provider_meter(
                &LegalShardKey::order("first"),
                at,
                at,
                hydration_budget,
                &provider,
                &mut hydration_meter,
            )
            .unwrap();
        root_only
            .authenticated_archive_membership_with_meter(
                &versions[0],
                &provider,
                hydration_budget,
                &mut hydration_meter,
            )
            .unwrap()
            .expect("authenticated membership");
        assert_eq!(
            hydration_meter
                .begin_provider_call(hydration_budget, false)
                .unwrap_err()
                .code,
            ErrorCode::QueryBudgetExceeded,
            "the next blob hydration call must be rejected before provider I/O"
        );
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

    #[test]
    fn dyadic_time_decomposition_is_minimal_canonical_and_covers_open_end() {
        let interval = LegalTimeInterval {
            start: SimTime::from_minutes(-3),
            end_exclusive: Some(SimTime::from_minutes(9)),
        };
        let cells = decompose_legal_time_interval(interval).unwrap();
        assert!(cells.len() <= MAX_LEGAL_TEMPORAL_CELLS_PER_INTERVAL);
        for minute in -3..9 {
            assert_eq!(
                cells
                    .iter()
                    .filter(|cell| cell.contains(SimTime::from_minutes(minute)))
                    .count(),
                1
            );
        }
        assert!(
            !cells
                .iter()
                .any(|cell| cell.contains(SimTime::from_minutes(9)))
        );

        let final_point = decompose_legal_time_interval(LegalTimeInterval {
            start: SimTime::from_minutes(i64::MAX),
            end_exclusive: None,
        })
        .unwrap();
        assert_eq!(final_point.len(), 1);
        assert!(final_point[0].contains(SimTime::from_minutes(i64::MAX)));
    }

    #[test]
    fn archive_page_buckets_use_the_full_u16_space_and_pages_are_hard_bounded() {
        let high_bucket = (1_u64..10_000)
            .map(|ordinal| version(&format!("bucket-{ordinal}"), ordinal))
            .map(|version| {
                legal_archive_index_bucket("canwu.law.archive-membership-bucket.v1", &version)
                    .unwrap()
            })
            .find(|bucket| *bucket >= 4_096)
            .expect("full-width legal bucket");
        assert!(u32::from(high_bucket) < LEGAL_ARCHIVE_INDEX_BUCKET_COUNT);

        let oversized_membership = LegalArchiveMembershipPage {
            format_version: LEGAL_ARCHIVE_INDEX_FORMAT_VERSION,
            shard: LegalShardKey::order("order"),
            bucket: 0,
            memberships: (0..=MAX_LEGAL_ARCHIVE_PAGE_ENTRIES)
                .map(|ordinal| LegalArchiveMembership {
                    version: version(&format!("membership-{ordinal}"), ordinal as u64 + 1),
                    location: LegalVersionLocation::Hot,
                    effective_interval: None,
                    recorded_interval: None,
                })
                .collect(),
        };
        assert!(oversized_membership.validate().is_err());

        let oversized_temporal = LegalArchiveTemporalPage {
            format_version: LEGAL_ARCHIVE_INDEX_FORMAT_VERSION,
            shard: LegalShardKey::order("order"),
            axis: LegalTemporalAxis::Effective,
            bucket: 0,
            segment: 0,
            entries: (0..=MAX_LEGAL_ARCHIVE_PAGE_ENTRIES)
                .map(|ordinal| LegalArchiveTemporalEntry {
                    cell: LegalDyadicCell {
                        prefix_bits: ordinal as u64,
                        prefix_length: LEGAL_TEMPORAL_WIDTH,
                    },
                    version: version(&format!("temporal-page-{ordinal}"), ordinal as u64 + 1),
                    primary_member_commitment: hash('a'),
                })
                .collect(),
        };
        assert!(oversized_temporal.validate().is_err());
    }

    #[test]
    fn segmented_temporal_pages_survive_root_only_restart_with_dense_time_cells() {
        let shard = LegalShardKey::order("dense-time");
        let provider = LegalTemporalScaleProvider::default();
        let mut storage = LegalStorageState::default();
        storage.directory.active_shards.insert(shard.clone());
        let mut blobs = BTreeMap::<LegalVersionRef, LegalArchiveBlob>::new();
        for ordinal in 1_u64..=130 {
            let (kind, record_class, closed_at, payload) = if ordinal <= 65 {
                (
                    LegalObjectKind::Retirement,
                    "retirement",
                    SimTime::from_minutes(42),
                    serde_json::json!({
                        "id": format!("dense-point-{ordinal}"),
                        "retired_at": SimTime::from_minutes(42),
                    }),
                )
            } else {
                (
                    LegalObjectKind::Source,
                    "source",
                    SimTime::from_minutes(i64::MAX),
                    serde_json::json!({
                        "id": format!("dense-open-{ordinal}"),
                        "adopted_at": SimTime::EPOCH,
                        "effective_at": SimTime::EPOCH,
                        "expires_at": null,
                    }),
                )
            };
            let version = LegalVersionRef {
                object: LegalObjectId {
                    kind,
                    id: format!("dense-time-{ordinal}"),
                    home_shard: shard.clone(),
                    local_discriminator: None,
                },
                version_ordinal: 1,
                content_commitment: legal_archive_content_commitment(record_class, &payload)
                    .expect("content commitment"),
            };
            let blob = LegalArchiveBlob {
                format_version: LEGAL_ARCHIVE_BLOB_FORMAT_VERSION,
                version: version.clone(),
                record_class: record_class.to_owned(),
                payload,
            };
            let encoded_bytes = serde_json::to_vec(&blob).expect("encode blob").len() as u64;
            storage.membership.insert(
                version.clone(),
                LegalArchiveMembership {
                    version: version.clone(),
                    location: LegalVersionLocation::Hot,
                    effective_interval: None,
                    recorded_interval: None,
                },
            );
            storage
                .mark_compaction_candidate(LegalCompactionCandidate {
                    version: version.clone(),
                    record_class: record_class.to_owned(),
                    closed_at,
                    encoded_bytes,
                    dependencies_resolved: true,
                    current_projection_retained: true,
                })
                .expect("mark candidate");
            blobs.insert(version, blob);
        }
        let compaction = storage
            .select_compaction_batch(
                &shard,
                LegalCompactionBudgets {
                    max_records: 130,
                    max_source_bytes: u64::MAX,
                },
            )
            .expect("select batch")
            .expect("dense batch");
        let ordered_blobs = compaction
            .candidates
            .iter()
            .map(|candidate| blobs[&candidate.version].clone())
            .collect::<Vec<_>>();
        let receipts = ordered_blobs
            .iter()
            .enumerate()
            .map(|(member_index, blob)| ArchiveObjectReceipt {
                object: ArchiveObjectId {
                    content_id: blob.content_id().expect("content id"),
                    blob_id: blob.blob_id().expect("blob id"),
                },
                owner_shard: shard.clone(),
                archive_batch_sequence: compaction.archive_batch_sequence,
                member_index: member_index as u64,
                codec: "json-canonical-v1".to_owned(),
                stored_bytes: serde_json::to_vec(blob).expect("encode blob").len() as u64,
                decoded_bytes: serde_json::to_vec(blob).expect("encode blob").len() as u64,
                source_root: compaction.source_membership_root.clone(),
                verified_plan_hash: compaction.token.clone(),
            })
            .collect::<Vec<_>>();
        let verified = PreparedLegalArchiveBatch {
            format_version: LEGAL_ARCHIVE_BLOB_FORMAT_VERSION,
            compaction: compaction.clone(),
            blobs: ordered_blobs,
            receipts,
            previous_archive_head: None,
        }
        .store_and_verify(&provider)
        .expect("store and verify dense temporal batch");
        for receipt in &verified.receipts {
            advance_to_durable(&mut storage, &receipt.object);
        }
        storage
            .commit_compaction(&verified.compaction, verified.receipts.clone())
            .expect("commit dense batch");
        storage
            .archive_heads
            .insert(shard.clone(), verified.archive_head.clone());
        provider
            .mark_legal_archive_retention_durable(&verified.retention_handle_id)
            .expect("mark dense archive ingress durable");
        provider
            .commit_legal_archive_retention(
                &verified.retention_handle_id,
                &verified.archive_head.membership_root,
            )
            .expect("commit dense archive retention");
        provider
            .finish_committed_head(&verified.archive_head)
            .expect("retain committed head");
        storage.archived_membership_materialized = false;
        storage
            .membership
            .retain(|_, membership| matches!(membership.location, LegalVersionLocation::Hot));
        storage
            .reachability
            .retain(|_, state| *state != ArchiveReachabilityState::Committed);
        storage
            .validate()
            .expect("validate root-only restart state");

        let directory = storage
            .authenticated_archive_directory(&shard, &provider)
            .expect("authenticate directory")
            .expect("directory");
        assert!(
            directory
                .effective_pages
                .values()
                .chain(directory.recorded_pages.values())
                .any(|segments| segments.len() >= 2)
        );
        let (at_point, usage) = storage
            .archived_versions_at_with_provider_usage(
                &shard,
                SimTime::from_minutes(42),
                SimTime::from_minutes(42),
                LegalTemporalQueryBudget {
                    max_candidates_per_dimension: 256,
                    max_intersection_members: 256,
                    ..LegalTemporalQueryBudget::default()
                },
                &provider,
            )
            .expect("query identical point and overlapping open intervals");
        assert_eq!(at_point.len(), 130);
        assert!(usage.provider_calls <= LegalTemporalQueryBudget::default().max_provider_calls);
        assert!(usage.segments <= LegalTemporalQueryBudget::default().max_segments);
        assert!(usage.decoded_bytes <= LegalTemporalQueryBudget::default().max_decoded_bytes);
        let after_point = storage
            .archived_versions_at_with_provider(
                &shard,
                SimTime::from_minutes(43),
                SimTime::from_minutes(43),
                LegalTemporalQueryBudget {
                    max_candidates_per_dimension: 256,
                    max_intersection_members: 256,
                    ..LegalTemporalQueryBudget::default()
                },
                &provider,
            )
            .expect("query open intervals after point expiry");
        assert_eq!(after_point.len(), 65);
        let error = storage
            .archived_versions_at_with_provider(
                &shard,
                SimTime::from_minutes(42),
                SimTime::from_minutes(42),
                LegalTemporalQueryBudget {
                    max_candidates_per_dimension: 256,
                    max_intersection_members: 256,
                    max_provider_calls: 1,
                    max_segments: 1,
                    max_decoded_bytes: u64::MAX,
                },
                &provider,
            )
            .expect_err("query must stop before reading an unbudgeted temporal segment");
        assert_eq!(error.code, ErrorCode::QueryBudgetExceeded);
    }

    #[test]
    fn bitemporal_roots_are_permutation_stable_and_queries_are_budgeted() {
        let effective = LegalTimeInterval {
            start: SimTime::from_minutes(10),
            end_exclusive: Some(SimTime::from_minutes(20)),
        };
        let recorded = LegalTimeInterval {
            start: SimTime::from_minutes(12),
            end_exclusive: None,
        };
        let versions = (1..=5)
            .map(|ordinal| version(&format!("temporal-{ordinal}"), ordinal))
            .collect::<Vec<_>>();
        let mut forward = LegalBitemporalIndex::default();
        let mut reverse = LegalBitemporalIndex::default();
        for item in &versions {
            forward
                .insert(effective, recorded, item, &item.content_commitment)
                .unwrap();
        }
        for item in versions.iter().rev() {
            reverse
                .insert(effective, recorded, item, &item.content_commitment)
                .unwrap();
        }
        assert_eq!(forward.roots().unwrap(), reverse.roots().unwrap());
        assert_eq!(
            forward
                .point_query(
                    SimTime::from_minutes(15),
                    SimTime::from_minutes(30),
                    LegalTemporalQueryBudget {
                        max_candidates_per_dimension: 5,
                        max_intersection_members: 5,
                        ..LegalTemporalQueryBudget::default()
                    },
                )
                .unwrap(),
            versions
        );
        let error = forward
            .point_query(
                SimTime::from_minutes(15),
                SimTime::from_minutes(30),
                LegalTemporalQueryBudget {
                    max_candidates_per_dimension: 4,
                    max_intersection_members: 4,
                    ..LegalTemporalQueryBudget::default()
                },
            )
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::QueryBudgetExceeded);
    }

    #[test]
    fn root_only_archive_head_appends_after_restart_without_old_membership() {
        let shard = LegalShardKey::order("order");
        let mut state = LegalStorageState::default();
        let first = version("first-cold", 1);
        state
            .record_hot_head(LegalHeadRef {
                object: first.object.clone(),
                version: first.clone(),
            })
            .unwrap();
        state
            .record_hot_head(LegalHeadRef {
                object: first.object.clone(),
                version: LegalVersionRef {
                    object: first.object.clone(),
                    version_ordinal: 2,
                    content_commitment: hash('b'),
                },
            })
            .unwrap();
        state
            .mark_compaction_candidate(LegalCompactionCandidate {
                version: first,
                record_class: "law_version".to_owned(),
                closed_at: SimTime::from_minutes(1),
                encoded_bytes: 10,
                dependencies_resolved: true,
                current_projection_retained: true,
            })
            .unwrap();
        let prepared = state
            .select_compaction_batch(&shard, LegalCompactionBudgets::default())
            .unwrap()
            .unwrap();
        let first_receipt = receipt(&prepared, 0, 'c');
        advance_to_durable(&mut state, &first_receipt.object);
        state
            .commit_compaction(&prepared, vec![first_receipt])
            .unwrap();
        let first_head = state.archive_heads[&shard].clone();

        state.archived_membership_materialized = false;
        state
            .membership
            .retain(|_, membership| matches!(membership.location, LegalVersionLocation::Hot));
        state
            .reachability
            .retain(|_, reachability| *reachability != ArchiveReachabilityState::Committed);

        let second = version("second-cold", 3);
        state
            .record_hot_head(LegalHeadRef {
                object: second.object.clone(),
                version: second.clone(),
            })
            .unwrap();
        state
            .record_hot_head(LegalHeadRef {
                object: second.object.clone(),
                version: LegalVersionRef {
                    object: second.object.clone(),
                    version_ordinal: 4,
                    content_commitment: hash('d'),
                },
            })
            .unwrap();
        state
            .mark_compaction_candidate(LegalCompactionCandidate {
                version: second,
                record_class: "law_version".to_owned(),
                closed_at: SimTime::from_minutes(2),
                encoded_bytes: 10,
                dependencies_resolved: true,
                current_projection_retained: true,
            })
            .unwrap();
        let prepared = state
            .select_compaction_batch(&shard, LegalCompactionBudgets::default())
            .unwrap()
            .unwrap();
        assert_eq!(prepared.archive_batch_sequence, 2);
        let second_receipt = receipt(&prepared, 0, 'e');
        advance_to_durable(&mut state, &second_receipt.object);
        state
            .commit_compaction(&prepared, vec![second_receipt])
            .unwrap();
        let second_head = &state.archive_heads[&shard];
        assert_eq!(second_head.archived_member_count, 2);
        assert_eq!(second_head.committed_batch_count, 2);
        assert_ne!(second_head.membership_root, first_head.membership_root);
        state.validate().unwrap();
    }
}
