use crate::model::{canonicalize_options, require_text};
use crate::{
    DecisionAction, DecisionAttemptOutcome, DecisionAttemptRecord, DecisionControllerBinding,
    DecisionError, DecisionErrorCode, DecisionMutation, DecisionOutcome, DecisionPolicy,
    DecisionPolicyIdentity, DecisionTicket, DecisionTicketState, DecisionTrace, PolicyDecision,
};
use canwu_core::{CommandRequestId, DecisionTicketId, DecisionTraceId};
use canwu_time::SimTime;
use im::{OrdMap, OrdSet};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeMap as StdBTreeMap;
use std::fmt::Write as _;
use std::mem::size_of;
use std::ops::Index;
use std::sync::Arc;

mod persistent_decision_map_serde {
    use im::OrdMap;
    use serde::ser::SerializeMap as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    pub fn serialize<S, K, V>(entries: &OrdMap<K, Arc<V>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        K: Clone + Ord + Serialize,
        V: Serialize,
    {
        let mut map = serializer.serialize_map(Some(entries.len()))?;
        for (ordinal, entry) in entries {
            map.serialize_entry(ordinal, entry.as_ref())?;
        }
        map.end()
    }

    pub fn deserialize<'de, D, K, V>(deserializer: D) -> Result<OrdMap<K, Arc<V>>, D::Error>
    where
        D: Deserializer<'de>,
        K: Clone + Deserialize<'de> + Ord,
        V: Deserialize<'de>,
    {
        Ok(BTreeMap::<K, V>::deserialize(deserializer)?
            .into_iter()
            .map(|(ordinal, entry)| (ordinal, Arc::new(entry)))
            .collect())
    }
}

mod decision_archive_receipts_serde {
    use super::{
        CompactDecisionArchiveReceipt, DecisionArchivePageKey, DecisionArchiveReceipt,
        DecisionHistoryKey,
    };
    use im::OrdMap;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(
        buckets: &OrdMap<
            DecisionArchivePageKey,
            OrdMap<DecisionHistoryKey, CompactDecisionArchiveReceipt>,
        >,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        buckets
            .values()
            .flat_map(OrdMap::iter)
            .map(|(key, receipt)| receipt.to_receipt(key))
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<
        OrdMap<DecisionArchivePageKey, OrdMap<DecisionHistoryKey, CompactDecisionArchiveReceipt>>,
        D::Error,
    >
    where
        D: Deserializer<'de>,
    {
        let entries = Vec::<DecisionArchiveReceipt>::deserialize(deserializer)?;
        let mut buckets = OrdMap::<
            DecisionArchivePageKey,
            OrdMap<DecisionHistoryKey, CompactDecisionArchiveReceipt>,
        >::new();
        for receipt in entries {
            let key = receipt.key.clone();
            let bucket = super::decision_history_page_key(&key).map_err(D::Error::custom)?;
            let value =
                CompactDecisionArchiveReceipt::from_receipt(&receipt).map_err(D::Error::custom)?;
            if buckets
                .entry(bucket)
                .or_default()
                .insert(key, value)
                .is_some()
            {
                return Err(D::Error::custom(
                    "decision archive receipt map contains a duplicate key",
                ));
            }
        }
        Ok(buckets)
    }
}

mod decision_archive_page_directory_serde {
    use super::DecisionArchivePageKey;
    use im::OrdMap;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(
        pages: &OrdMap<DecisionArchivePageKey, String>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        pages
            .iter()
            .map(|(key, id)| (*key, id))
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<OrdMap<DecisionArchivePageKey, String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = Vec::<(DecisionArchivePageKey, String)>::deserialize(deserializer)?;
        let mut pages = OrdMap::new();
        let mut previous = None;
        for (key, id) in entries {
            if previous.is_some_and(|previous| previous >= key) || pages.insert(key, id).is_some() {
                return Err(D::Error::custom(
                    "decision archive page directory is not strictly ordered",
                ));
            }
            previous = Some(key);
        }
        Ok(pages)
    }
}

/// Typed identity for decision history. A scalar ID is not enough because
/// tickets, caller-selected requests, and engine-issued traces have different
/// uniqueness and retention rules.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum DecisionHistoryKey {
    Ticket(canwu_core::DecisionTicketId),
    Attempt(canwu_core::DecisionRequestId),
    Trace(canwu_core::DecisionTraceId),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "location")]
pub enum DecisionHistoryLocation {
    Hot,
    Archived {
        locator: String,
    },
    /// The committed locator bucket is cold. Exact membership requires the
    /// archive provider and must not be inferred as absence.
    Unresolved {
        bucket: u16,
        segment: u8,
    },
    Absent,
}

pub const DECISION_ARCHIVE_FORMAT_VERSION: u32 = 1;
pub const MAX_DECISION_ARCHIVE_BATCH_ENTRIES: usize = 4_096;
pub const MAX_DECISION_HISTORY_PAGE_SIZE: usize = 512;
pub const MAX_DECISION_HISTORY_PAGE_BYTES: u64 = 16 * 1024 * 1024;
pub const DECISION_HISTORY_BUCKET_BITS: u8 = 12;
pub const DECISION_HISTORY_BUCKET_COUNT: u16 = 1 << DECISION_HISTORY_BUCKET_BITS;
pub const DECISION_ARCHIVE_BUCKET_PAGE_FORMAT_VERSION: u32 = 1;
pub const MAX_DECISION_ARCHIVE_BUCKET_PAGE_ENTRIES: usize = 64;
pub const MAX_DECISION_ARCHIVE_BUCKET_PAGE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionArchivePageKey {
    pub bucket: u16,
    pub segment: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionHistoryQueryBudget {
    pub max_results: usize,
    pub max_provider_calls: usize,
    pub max_decoded_bytes: u64,
}

impl Default for DecisionHistoryQueryBudget {
    fn default() -> Self {
        Self {
            max_results: 128,
            max_provider_calls: 128,
            max_decoded_bytes: 4 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionHistoryCursor {
    pub archive_root: String,
    pub bucket: u16,
    pub segment: u8,
    pub after: DecisionHistoryKey,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionHistoryPage {
    pub archive_root: String,
    pub records: Vec<DecisionArchiveRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<DecisionHistoryCursor>,
    pub provider_calls: u64,
    pub decoded_bytes: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionArchiveReachability {
    pub bucket_page_ids: OrdSet<String>,
    pub blob_locators: OrdSet<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionArchiveBucketPage {
    pub format_version: u32,
    pub bucket: u16,
    pub segment: u8,
    pub receipts: Vec<DecisionArchiveReceipt>,
}

impl DecisionArchiveBucketPage {
    pub fn validate(&self) -> Result<(), DecisionError> {
        if self.format_version != DECISION_ARCHIVE_BUCKET_PAGE_FORMAT_VERSION
            || self.bucket >= DECISION_HISTORY_BUCKET_COUNT
            || self.receipts.is_empty()
            || self.receipts.len() > MAX_DECISION_ARCHIVE_BUCKET_PAGE_ENTRIES
            || self
                .receipts
                .windows(2)
                .any(|pair| pair[0].key >= pair[1].key)
            || self.receipts.iter().any(|receipt| {
                decision_history_page_key(&receipt.key).ok()
                    != Some(DecisionArchivePageKey {
                        bucket: self.bucket,
                        segment: self.segment,
                    })
                    || CompactDecisionArchiveReceipt::from_receipt(receipt).is_err()
            })
        {
            return Err(archive_error(
                "decision archive bucket page is malformed or non-canonical",
            ));
        }
        let encoded = serde_json::to_vec(self).map_err(|error| {
            archive_error(format!(
                "cannot encode decision archive bucket page: {error}"
            ))
        })?;
        if encoded.len() > MAX_DECISION_ARCHIVE_BUCKET_PAGE_BYTES {
            return Err(archive_error(
                "decision archive bucket page exceeds the hard byte limit",
            ));
        }
        Ok(())
    }

    pub fn state_page_id(&self) -> Result<String, DecisionError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|error| {
            archive_error(format!(
                "cannot encode decision archive bucket page: {error}"
            ))
        })?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"canwu.state-page.v1");
        hasher.update(&[0]);
        hasher.update(&bytes);
        Ok(hasher.finalize().to_hex().to_string())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionLocatorScaleMetrics {
    pub entries: u64,
    pub locator_pages: u64,
    pub max_page_entries: u64,
    pub max_page_encoded_bytes: u64,
    pub archive_batches: u64,
    pub exact_restart_queries: u64,
    pub reachable_blob_locators: u64,
    pub estimated_resident_structural_bytes: u64,
    pub root_hash: String,
}

#[doc(hidden)]
pub struct DecisionLocatorScaleFixture {
    pub state: DecisionState,
    pub archive_blobs: Vec<DecisionArchiveBlob>,
    pub metrics: DecisionLocatorScaleMetrics,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TraceLocatorScaleMetrics {
    pub hot_trace_entries: u64,
    pub indexed_lookup_samples: u64,
    pub archive_commit_entries: u64,
    pub target_archived: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "record", rename_all = "snake_case")]
pub enum DecisionArchiveRecord {
    Ticket { ticket: DecisionTicket },
    Attempt { attempt: DecisionAttemptRecord },
    Trace { trace: DecisionTrace },
}

impl DecisionArchiveRecord {
    #[must_use]
    pub fn key(&self) -> DecisionHistoryKey {
        match self {
            Self::Ticket { ticket } => DecisionHistoryKey::Ticket(ticket.id),
            Self::Attempt { attempt } => DecisionHistoryKey::Attempt(attempt.request_id),
            Self::Trace { trace } => DecisionHistoryKey::Trace(trace.id),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionArchiveBlob {
    pub format_version: u32,
    pub key: DecisionHistoryKey,
    pub record: DecisionArchiveRecord,
}

impl DecisionArchiveBlob {
    pub fn validate(&self) -> Result<(), DecisionError> {
        if self.format_version != DECISION_ARCHIVE_FORMAT_VERSION || self.key != self.record.key() {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "decision archive blob format or typed identity is invalid",
            ));
        }
        Ok(())
    }

    pub fn content_id(&self) -> Result<String, DecisionError> {
        self.validate()?;
        decision_archive_hash("canwu.decision.archive-blob.v1", self)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionArchiveReceipt {
    pub format_version: u32,
    pub key: DecisionHistoryKey,
    pub locator: String,
    pub content_id: String,
    pub encoded_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompactDecisionArchiveReceipt {
    content_id: [u8; 32],
    encoded_bytes: u64,
}

impl CompactDecisionArchiveReceipt {
    fn from_receipt(receipt: &DecisionArchiveReceipt) -> Result<Self, DecisionError> {
        if receipt.format_version != DECISION_ARCHIVE_FORMAT_VERSION
            || receipt.locator != receipt.content_id
            || receipt.encoded_bytes == 0
        {
            return Err(archive_error("decision archive receipt is not canonical"));
        }
        Ok(Self {
            content_id: decode_archive_hash(&receipt.content_id)?,
            encoded_bytes: receipt.encoded_bytes,
        })
    }

    fn to_receipt(&self, key: &DecisionHistoryKey) -> DecisionArchiveReceipt {
        let content_id = encode_archive_hash(self.content_id);
        DecisionArchiveReceipt {
            format_version: DECISION_ARCHIVE_FORMAT_VERSION,
            key: key.clone(),
            locator: content_id.clone(),
            content_id,
            encoded_bytes: self.encoded_bytes,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedDecisionArchive {
    pub source_root: String,
    pub token: String,
    pub blobs: Vec<DecisionArchiveBlob>,
    pub receipts: Vec<DecisionArchiveReceipt>,
}

/// Provider-verified, replay-safe archive transition. Blob bytes remain in the
/// host archive; canonical ingress carries only the exact hot-state source
/// root, token, and compact receipts needed to revalidate the transition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedDecisionArchiveCommit {
    format_version: u32,
    source_root: String,
    token: String,
    receipts: Vec<DecisionArchiveReceipt>,
    page_replacements: Vec<VerifiedDecisionArchivePageReplacement>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct VerifiedDecisionArchivePageReplacement {
    page_key: DecisionArchivePageKey,
    previous_page_id: Option<String>,
    page: DecisionArchiveBucketPage,
}

impl VerifiedDecisionArchiveCommit {
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    #[must_use]
    pub fn source_root(&self) -> &str {
        &self.source_root
    }

    #[must_use]
    pub fn has_current_nonempty_shape(&self) -> bool {
        self.format_version == DECISION_ARCHIVE_FORMAT_VERSION
            && !self.receipts.is_empty()
            && !self.page_replacements.is_empty()
    }

    pub fn archive_locators(&self) -> impl Iterator<Item = &str> {
        self.receipts.iter().map(|receipt| receipt.locator.as_str())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionArchiveStoreOutcome {
    Stored,
    AlreadyStored,
}

pub trait DecisionArchiveProvider {
    fn load_decision_archive(
        &self,
        locator: &str,
    ) -> Result<Option<DecisionArchiveBlob>, DecisionError>;

    /// Loads one content-addressed locator bucket for root-only decision
    /// states. Providers that serve only archive payloads may leave this
    /// unsupported; calls then fail closed as unavailable history.
    fn load_decision_archive_bucket_page(
        &self,
        _page_id: &str,
    ) -> Result<Option<DecisionArchiveBucketPage>, DecisionError> {
        Ok(None)
    }
}

pub trait DecisionArchiveStore: DecisionArchiveProvider {
    fn store_decision_archive(
        &self,
        blob: &DecisionArchiveBlob,
    ) -> Result<DecisionArchiveStoreOutcome, DecisionError>;
}

fn archive_error(message: impl Into<String>) -> DecisionError {
    DecisionError::new(DecisionErrorCode::InvalidDecision, message)
}

fn history_unavailable(message: impl Into<String>) -> DecisionError {
    DecisionError::new(DecisionErrorCode::DecisionHistoryUnavailable, message)
}

fn history_budget_error(message: impl Into<String>) -> DecisionError {
    DecisionError::new(DecisionErrorCode::QueryBudgetExceeded, message)
}

fn decision_archive_hash(domain: &str, value: &impl Serialize) -> Result<String, DecisionError> {
    let encoded = serde_json::to_vec(&(domain, value)).map_err(|error| {
        archive_error(format!(
            "cannot encode canonical decision archive content: {error}"
        ))
    })?;
    Ok(blake3::hash(&encoded).to_hex().to_string())
}

fn canonical_archive_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn decode_archive_hash(value: &str) -> Result<[u8; 32], DecisionError> {
    if !canonical_archive_hash(value) {
        return Err(archive_error(
            "decision archive hash must be lower-case 32-byte hexadecimal",
        ));
    }
    let mut decoded = [0_u8; 32];
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    debug_assert!(remainder.is_empty());
    for (index, pair) in pairs.iter().enumerate() {
        let digit = |byte| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        decoded[index] = digit(pair[0])
            .zip(digit(pair[1]))
            .map(|(high, low)| (high << 4) | low)
            .ok_or_else(|| archive_error("decision archive hash contains invalid hexadecimal"))?;
    }
    Ok(decoded)
}

fn encode_archive_hash(value: [u8; 32]) -> String {
    let mut encoded = String::with_capacity(64);
    for byte in value {
        write!(&mut encoded, "{byte:02x}").expect("writing to a string cannot fail");
    }
    encoded
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionHotState {
    pub ticket_count: u64,
    pub attempt_count: u64,
    pub trace_count: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct DecisionHotHistoryAccumulator {
    count: u64,
    xor: [u8; 32],
    sum: [u8; 32],
}

impl DecisionHotHistoryAccumulator {
    fn insert(
        &mut self,
        key: &DecisionHistoryKey,
        record: &DecisionArchiveRecord,
    ) -> Result<(), DecisionError> {
        let digest = decision_hot_leaf_hash(key, record)?;
        self.count = self
            .count
            .checked_add(1)
            .ok_or_else(|| archive_error("decision hot-history count overflowed"))?;
        for (target, byte) in self.xor.iter_mut().zip(digest) {
            *target ^= byte;
        }
        add_digest_mod_256(&mut self.sum, digest);
        Ok(())
    }

    fn remove(
        &mut self,
        key: &DecisionHistoryKey,
        record: &DecisionArchiveRecord,
    ) -> Result<(), DecisionError> {
        let digest = decision_hot_leaf_hash(key, record)?;
        self.count = self
            .count
            .checked_sub(1)
            .ok_or_else(|| archive_error("decision hot-history count underflowed"))?;
        for (target, byte) in self.xor.iter_mut().zip(digest) {
            *target ^= byte;
        }
        subtract_digest_mod_256(&mut self.sum, digest);
        Ok(())
    }
}

/// Structurally shared append-only decision log.
///
/// Cloning this value shares its immutable tree root and its entry payloads.
/// Appending copies only the affected tree path, so boundary rollback and
/// simulation forks do not clone all retained decision attempts or traces.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
pub struct PersistentDecisionLog<T: Clone> {
    next_ordinal: u64,
    #[serde(with = "persistent_decision_map_serde")]
    entries: OrdMap<u64, Arc<T>>,
}

impl<T: Clone> Default for PersistentDecisionLog<T> {
    fn default() -> Self {
        Self {
            next_ordinal: 1,
            entries: OrdMap::new(),
        }
    }
}

impl<T: Clone> PersistentDecisionLog<T> {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn is_default_empty(&self) -> bool {
        self.entries.is_empty() && self.next_ordinal == 1
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&T> {
        self.entries.values().nth(index).map(Arc::as_ref)
    }

    #[must_use]
    pub fn last(&self) -> Option<&T> {
        self.entries.get_max().map(|(_, value)| value.as_ref())
    }

    #[must_use]
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + ExactSizeIterator {
        self.entries.values().map(Arc::as_ref)
    }

    /// Appends one value and returns its stable ordinal.
    ///
    /// # Panics
    ///
    /// Panics only after the persistent `u64` ordinal space is exhausted.
    pub fn push(&mut self, value: T) -> u64 {
        let ordinal = self.next_ordinal;
        self.next_ordinal = self
            .next_ordinal
            .checked_add(1)
            .expect("decision log ordinal range is exhausted");
        self.entries.insert(ordinal, Arc::new(value));
        ordinal
    }

    #[must_use]
    pub fn get_ordinal(&self, ordinal: u64) -> Option<&T> {
        self.entries.get(&ordinal).map(Arc::as_ref)
    }

    pub fn remove_ordinal(&mut self, ordinal: u64) -> Option<T> {
        self.entries.remove(&ordinal).map(Arc::unwrap_or_clone)
    }

    #[must_use]
    pub fn ordinals(&self) -> impl DoubleEndedIterator<Item = u64> + ExactSizeIterator + '_ {
        self.entries.keys().copied()
    }

    #[must_use]
    pub const fn next_ordinal(&self) -> u64 {
        self.next_ordinal
    }
}

impl<T: Clone> Index<usize> for PersistentDecisionLog<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index)
            .expect("persistent decision log index is out of bounds")
    }
}

pub struct PersistentDecisionLogIter<'a, T: Clone> {
    inner: im::ordmap::Values<'a, u64, Arc<T>>,
}

impl<'a, T: Clone> Iterator for PersistentDecisionLogIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(Arc::as_ref)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<T: Clone> DoubleEndedIterator for PersistentDecisionLogIter<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(Arc::as_ref)
    }
}

impl<T: Clone> ExactSizeIterator for PersistentDecisionLogIter<'_, T> {}

impl<'a, T: Clone> IntoIterator for &'a PersistentDecisionLog<T> {
    type Item = &'a T;
    type IntoIter = PersistentDecisionLogIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        PersistentDecisionLogIter {
            inner: self.entries.values(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct DecisionState {
    #[serde(
        default,
        skip_serializing_if = "OrdMap::is_empty",
        with = "persistent_decision_map_serde"
    )]
    pub controllers: OrdMap<String, Arc<DecisionControllerBinding>>,
    #[serde(
        default,
        skip_serializing_if = "OrdMap::is_empty",
        with = "persistent_decision_map_serde"
    )]
    pub tickets: OrdMap<DecisionTicketId, Arc<DecisionTicket>>,
    #[serde(
        default,
        skip_serializing_if = "PersistentDecisionLog::is_default_empty"
    )]
    pub traces: PersistentDecisionLog<DecisionTrace>,
    #[serde(
        default,
        skip_serializing_if = "PersistentDecisionLog::is_default_empty"
    )]
    attempts: PersistentDecisionLog<DecisionAttemptRecord>,
    #[serde(skip)]
    attempts_by_request: OrdMap<canwu_core::DecisionRequestId, u64>,
    #[serde(skip)]
    deadline_index: OrdMap<SimTime, OrdSet<DecisionTicketId>>,
    #[serde(
        default,
        skip_serializing_if = "OrdMap::is_empty",
        with = "decision_archive_receipts_serde"
    )]
    archive_receipt_buckets:
        OrdMap<DecisionArchivePageKey, OrdMap<DecisionHistoryKey, CompactDecisionArchiveReceipt>>,
    #[serde(
        default,
        skip_serializing_if = "OrdMap::is_empty",
        with = "decision_archive_page_directory_serde"
    )]
    archive_bucket_page_ids: OrdMap<DecisionArchivePageKey, String>,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    archive_receipt_count: u64,
    #[serde(skip)]
    hot_history_accumulator: DecisionHotHistoryAccumulator,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

impl<'de> Deserialize<'de> for DecisionState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct PersistedDecisionState {
            #[serde(default, with = "persistent_decision_map_serde")]
            controllers: OrdMap<String, Arc<DecisionControllerBinding>>,
            #[serde(default, with = "persistent_decision_map_serde")]
            tickets: OrdMap<canwu_core::DecisionTicketId, Arc<DecisionTicket>>,
            #[serde(default)]
            traces: PersistentDecisionLog<DecisionTrace>,
            #[serde(default)]
            attempts: PersistentDecisionLog<DecisionAttemptRecord>,
            #[serde(default, with = "decision_archive_receipts_serde")]
            archive_receipt_buckets: OrdMap<
                DecisionArchivePageKey,
                OrdMap<DecisionHistoryKey, CompactDecisionArchiveReceipt>,
            >,
            #[serde(default, with = "decision_archive_page_directory_serde")]
            archive_bucket_page_ids: OrdMap<DecisionArchivePageKey, String>,
            #[serde(default)]
            archive_receipt_count: u64,
        }

        let persisted = PersistedDecisionState::deserialize(deserializer)?;
        let attempts_by_request = persisted
            .attempts
            .ordinals()
            .zip(persisted.attempts.iter())
            .map(|(ordinal, attempt)| (attempt.request_id, ordinal))
            .collect();
        let mut deadline_index = OrdMap::<SimTime, OrdSet<DecisionTicketId>>::new();
        for ticket in persisted.tickets.values().filter(|ticket| ticket.is_open()) {
            if let Some(deadline) = ticket.deadline {
                deadline_index
                    .entry(deadline)
                    .or_default()
                    .insert(ticket.id);
            }
        }
        Self {
            controllers: persisted.controllers,
            tickets: persisted.tickets,
            traces: persisted.traces,
            attempts: persisted.attempts,
            attempts_by_request,
            deadline_index,
            archive_receipt_buckets: persisted.archive_receipt_buckets,
            archive_bucket_page_ids: persisted.archive_bucket_page_ids,
            archive_receipt_count: persisted.archive_receipt_count,
            hot_history_accumulator: DecisionHotHistoryAccumulator::default(),
        }
        .with_rebuilt_hot_history_accumulator()
        .map_err(serde::de::Error::custom)?
        .with_rebuilt_archive_bucket_pages()
        .map_err(serde::de::Error::custom)
    }
}

impl DecisionState {
    fn with_rebuilt_hot_history_accumulator(mut self) -> Result<Self, DecisionError> {
        self.rebuild_hot_history_accumulator()?;
        Ok(self)
    }

    fn rebuild_hot_history_accumulator(&mut self) -> Result<(), DecisionError> {
        let mut rebuilt = DecisionHotHistoryAccumulator::default();
        for ticket in self.tickets.values() {
            let key = DecisionHistoryKey::Ticket(ticket.id);
            rebuilt.insert(
                &key,
                &DecisionArchiveRecord::Ticket {
                    ticket: ticket.as_ref().clone(),
                },
            )?;
        }
        for attempt in self.attempts.iter() {
            let key = DecisionHistoryKey::Attempt(attempt.request_id);
            rebuilt.insert(
                &key,
                &DecisionArchiveRecord::Attempt {
                    attempt: attempt.clone(),
                },
            )?;
        }
        for trace in self.traces.iter() {
            let key = DecisionHistoryKey::Trace(trace.id);
            rebuilt.insert(
                &key,
                &DecisionArchiveRecord::Trace {
                    trace: trace.clone(),
                },
            )?;
        }
        self.hot_history_accumulator = rebuilt;
        Ok(())
    }

    fn insert_hot_history_record(
        &mut self,
        record: &DecisionArchiveRecord,
    ) -> Result<(), DecisionError> {
        self.hot_history_accumulator.insert(&record.key(), record)
    }

    fn replace_hot_history_record(
        &mut self,
        previous: &DecisionArchiveRecord,
        next: &DecisionArchiveRecord,
    ) -> Result<(), DecisionError> {
        if previous.key() != next.key() {
            return Err(archive_error(
                "decision hot-history replacement changed record identity",
            ));
        }
        self.hot_history_accumulator
            .remove(&previous.key(), previous)?;
        self.hot_history_accumulator.insert(&next.key(), next)
    }

    fn remove_hot_history_record(
        &mut self,
        record: &DecisionArchiveRecord,
    ) -> Result<(), DecisionError> {
        self.hot_history_accumulator.remove(&record.key(), record)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.controllers.is_empty()
            && self.tickets.is_empty()
            && self.traces.is_empty()
            && self.attempts.is_empty()
            && self.archive_receipt_count == 0
    }

    fn compact_archive_receipt(
        &self,
        key: &DecisionHistoryKey,
    ) -> Option<&CompactDecisionArchiveReceipt> {
        let bucket = decision_history_page_key(key).ok()?;
        self.archive_receipt_buckets.get(&bucket)?.get(key)
    }

    fn contains_archived_key(&self, key: &DecisionHistoryKey) -> bool {
        self.compact_archive_receipt(key).is_some()
    }

    fn insert_archive_receipt(
        &mut self,
        receipt: &DecisionArchiveReceipt,
    ) -> Result<Option<CompactDecisionArchiveReceipt>, DecisionError> {
        let bucket = decision_history_page_key(&receipt.key)?;
        let previous = self
            .archive_receipt_buckets
            .entry(bucket)
            .or_default()
            .insert(
                receipt.key.clone(),
                CompactDecisionArchiveReceipt::from_receipt(receipt)?,
            );
        self.refresh_archive_bucket_page_id(bucket)?;
        if previous.is_none() {
            self.archive_receipt_count = self
                .archive_receipt_count
                .checked_add(1)
                .ok_or_else(|| archive_error("decision archive receipt count overflowed"))?;
        }
        Ok(previous)
    }

    fn with_rebuilt_archive_bucket_pages(mut self) -> Result<Self, DecisionError> {
        let resident_count =
            self.archive_receipt_buckets
                .values()
                .try_fold(0_u64, |total, receipts| {
                    total
                        .checked_add(receipts.len() as u64)
                        .ok_or_else(|| archive_error("decision archive receipt count overflowed"))
                })?;
        if resident_count > 0 {
            let buckets = self
                .archive_receipt_buckets
                .keys()
                .copied()
                .collect::<Vec<_>>();
            if self.archive_bucket_page_ids.is_empty() {
                self.archive_receipt_count = resident_count;
                for bucket in buckets {
                    self.refresh_archive_bucket_page_id(bucket)?;
                }
            } else {
                for bucket in buckets {
                    let resident_page_id = self
                        .decision_archive_bucket_page(bucket)?
                        .ok_or_else(|| archive_error("resident decision archive bucket vanished"))?
                        .state_page_id()?;
                    if self.archive_bucket_page_ids.get(&bucket) != Some(&resident_page_id) {
                        return Err(archive_error(
                            "decision archive bucket directory disagrees with resident receipts",
                        ));
                    }
                }
            }
        }
        self.validate_archive_directory_shape()?;
        Ok(self)
    }

    fn refresh_archive_bucket_page_id(
        &mut self,
        bucket: DecisionArchivePageKey,
    ) -> Result<(), DecisionError> {
        let page = self
            .decision_archive_bucket_page(bucket)?
            .ok_or_else(|| archive_error("decision archive bucket unexpectedly disappeared"))?;
        self.archive_bucket_page_ids
            .insert(bucket, page.state_page_id()?);
        Ok(())
    }

    pub fn decision_archive_bucket_page(
        &self,
        bucket: DecisionArchivePageKey,
    ) -> Result<Option<DecisionArchiveBucketPage>, DecisionError> {
        let Some(receipts) = self.archive_receipt_buckets.get(&bucket) else {
            return Ok(None);
        };
        let page = DecisionArchiveBucketPage {
            format_version: DECISION_ARCHIVE_BUCKET_PAGE_FORMAT_VERSION,
            bucket: bucket.bucket,
            segment: bucket.segment,
            receipts: receipts
                .iter()
                .map(|(key, receipt)| receipt.to_receipt(key))
                .collect(),
        };
        page.validate()?;
        Ok(Some(page))
    }

    #[must_use]
    pub fn decision_archive_bucket_page_ids(&self) -> &OrdMap<DecisionArchivePageKey, String> {
        &self.archive_bucket_page_ids
    }

    /// Returns the bounded hot decision state used by a paged checkpoint.
    /// Persistent maps/logs remain structurally shared; archive locator buckets
    /// are represented by the separately committed page directory.
    #[must_use]
    pub fn paged_checkpoint_hot_state(&self) -> Self {
        let mut hot = self.clone();
        hot.archive_receipt_buckets.clear();
        hot.archive_bucket_page_ids.clear();
        hot.archive_receipt_count = 0;
        hot
    }

    #[must_use]
    pub fn archived_history_count(&self) -> usize {
        usize::try_from(self.archive_receipt_count).unwrap_or(usize::MAX)
    }

    #[must_use]
    pub fn controller(&self, id: &str) -> Option<&DecisionControllerBinding> {
        self.controllers.get(id).map(Arc::as_ref)
    }

    #[must_use]
    pub fn ticket(&self, id: DecisionTicketId) -> Option<&DecisionTicket> {
        self.tickets.get(&id).map(Arc::as_ref)
    }

    #[must_use]
    pub fn trace(&self, id: DecisionTraceId) -> Option<&DecisionTrace> {
        self.traces.get_ordinal(id.get())
    }

    #[must_use]
    pub fn attempt(&self, id: canwu_core::DecisionRequestId) -> Option<&DecisionAttemptRecord> {
        self.attempts_by_request
            .get(&id)
            .and_then(|ordinal| self.attempts.get_ordinal(*ordinal))
    }

    #[must_use]
    pub const fn attempts(&self) -> &PersistentDecisionLog<DecisionAttemptRecord> {
        &self.attempts
    }

    /// Returns the current hot/archive location without conflating an
    /// unavailable archive page with an absent record.
    #[must_use]
    pub fn decision_locator(&self, key: &DecisionHistoryKey) -> DecisionHistoryLocation {
        let hot = match key {
            DecisionHistoryKey::Ticket(id) => self.tickets.contains_key(id),
            DecisionHistoryKey::Attempt(id) => self.attempts_by_request.contains_key(id),
            DecisionHistoryKey::Trace(id) => self.traces.get_ordinal(id.get()).is_some(),
        };
        if hot {
            DecisionHistoryLocation::Hot
        } else if let Some(receipt) = self.compact_archive_receipt(key) {
            DecisionHistoryLocation::Archived {
                locator: encode_archive_hash(receipt.content_id),
            }
        } else if let Ok(page) = decision_history_page_key(key)
            && self.archive_bucket_page_ids.contains_key(&page)
        {
            DecisionHistoryLocation::Unresolved {
                bucket: page.bucket,
                segment: page.segment,
            }
        } else {
            DecisionHistoryLocation::Absent
        }
    }

    /// Resolves an exact decision-history location through the committed
    /// state-page directory when the bucket is not resident. Missing provider
    /// data is never reported as an absent key.
    pub fn decision_locator_with_provider(
        &self,
        key: &DecisionHistoryKey,
        provider: &dyn DecisionArchiveProvider,
    ) -> Result<DecisionHistoryLocation, DecisionError> {
        let local = self.decision_locator(key);
        match local {
            DecisionHistoryLocation::Hot
            | DecisionHistoryLocation::Archived { .. }
            | DecisionHistoryLocation::Absent => return Ok(local),
            DecisionHistoryLocation::Unresolved { .. } => {}
        }
        let bucket = decision_history_page_key(key)?;
        let Some(page_id) = self.archive_bucket_page_ids.get(&bucket) else {
            return Ok(DecisionHistoryLocation::Absent);
        };
        let page = provider
            .load_decision_archive_bucket_page(page_id)?
            .ok_or_else(|| history_unavailable("decision locator bucket page is unavailable"))?;
        page.validate()?;
        if page.bucket != bucket.bucket
            || page.segment != bucket.segment
            || page.state_page_id()? != *page_id
        {
            return Err(history_unavailable(
                "decision locator provider returned a mismatched bucket page",
            ));
        }
        match page
            .receipts
            .binary_search_by(|receipt| receipt.key.cmp(key))
        {
            Ok(index) => Ok(DecisionHistoryLocation::Archived {
                locator: page.receipts[index].locator.clone(),
            }),
            Err(_) => Ok(DecisionHistoryLocation::Absent),
        }
    }

    #[must_use]
    pub fn archive_receipt(&self, key: &DecisionHistoryKey) -> Option<DecisionArchiveReceipt> {
        self.compact_archive_receipt(key)
            .map(|receipt| receipt.to_receipt(key))
    }

    fn archive_receipt_with_provider(
        &self,
        key: &DecisionHistoryKey,
        provider: &dyn DecisionArchiveProvider,
    ) -> Result<Option<DecisionArchiveReceipt>, DecisionError> {
        if let Some(receipt) = self.archive_receipt(key) {
            return Ok(Some(receipt));
        }
        let bucket = decision_history_page_key(key)?;
        let Some(page_id) = self.archive_bucket_page_ids.get(&bucket) else {
            return Ok(None);
        };
        let page = provider
            .load_decision_archive_bucket_page(page_id)?
            .ok_or_else(|| history_unavailable("decision locator bucket page is unavailable"))?;
        page.validate()?;
        if page.bucket != bucket.bucket
            || page.segment != bucket.segment
            || page.state_page_id()? != *page_id
        {
            return Err(history_unavailable(
                "decision locator provider returned a mismatched bucket page",
            ));
        }
        Ok(page
            .receipts
            .binary_search_by(|receipt| receipt.key.cmp(key))
            .ok()
            .map(|index| page.receipts[index].clone()))
    }

    pub fn archived_history_keys(&self) -> impl Iterator<Item = &DecisionHistoryKey> {
        self.archive_receipt_buckets.values().flat_map(OrdMap::keys)
    }

    /// Returns the exact locator pages required to validate references from
    /// bounded hot state into archived decision history. The result is bounded
    /// by hot traces, resolved tickets, and accepted attempts rather than by
    /// total archive size.
    pub fn required_archived_dependency_page_keys(
        &self,
    ) -> Result<OrdSet<DecisionArchivePageKey>, DecisionError> {
        let mut pages = OrdSet::new();
        for trace in self.traces.iter() {
            if !self.tickets.contains_key(&trace.ticket_id) {
                pages.insert(decision_history_page_key(&DecisionHistoryKey::Ticket(
                    trace.ticket_id,
                ))?);
            }
        }
        for ticket in self.tickets.values() {
            if let DecisionTicketState::Resolved { trace_id, .. } = ticket.state
                && self.traces.get_ordinal(trace_id.get()).is_none()
            {
                pages.insert(decision_history_page_key(&DecisionHistoryKey::Trace(
                    trace_id,
                ))?);
            }
        }
        for attempt in self.attempts.iter() {
            if let DecisionAttemptOutcome::Accepted {
                trace_id: Some(trace_id),
                ..
            } = &attempt.outcome
                && self.traces.get_ordinal(trace_id.get()).is_none()
            {
                pages.insert(decision_history_page_key(&DecisionHistoryKey::Trace(
                    *trace_id,
                ))?);
            }
        }
        Ok(pages)
    }

    /// Separates bounded hot decision state from stable hash-prefix archive
    /// buckets. A checkpoint can rewrite only buckets touched by this archive
    /// generation instead of serializing one all-history page.
    pub fn paged_checkpoint_parts(
        &self,
    ) -> Result<
        (
            Self,
            std::collections::BTreeMap<DecisionArchivePageKey, Vec<DecisionArchiveReceipt>>,
        ),
        DecisionError,
    > {
        let hot = self.paged_checkpoint_hot_state();
        let mut buckets =
            std::collections::BTreeMap::<DecisionArchivePageKey, Vec<DecisionArchiveReceipt>>::new(
            );
        for (bucket, receipts) in &self.archive_receipt_buckets {
            let decoded = receipts
                .iter()
                .map(|(key, receipt)| receipt.to_receipt(key))
                .collect();
            buckets.insert(*bucket, decoded);
        }
        Ok((hot, buckets))
    }

    pub fn from_paged_checkpoint_parts(
        mut hot: Self,
        buckets: impl IntoIterator<Item = (DecisionArchivePageKey, Vec<DecisionArchiveReceipt>)>,
    ) -> Result<Self, DecisionError> {
        if !hot.archive_receipt_buckets.is_empty() {
            return Err(archive_error(
                "paged decision hot state must not duplicate archive receipts",
            ));
        }
        hot.archive_bucket_page_ids.clear();
        hot.archive_receipt_count = 0;
        for (bucket, receipts) in buckets {
            if bucket.bucket >= DECISION_HISTORY_BUCKET_COUNT
                || receipts.windows(2).any(|pair| pair[0].key >= pair[1].key)
            {
                return Err(archive_error(
                    "paged decision archive bucket is out of range or not strictly ordered",
                ));
            }
            for receipt in receipts {
                if decision_history_page_key(&receipt.key)? != bucket
                    || hot.insert_archive_receipt(&receipt)?.is_some()
                {
                    return Err(archive_error(
                        "paged decision archive bucket contains a misplaced or duplicate receipt",
                    ));
                }
            }
        }
        hot.validate()?;
        Ok(hot)
    }

    /// Restores the bounded hot decision state and authenticated archive-page
    /// directory without decoding any historical locator bucket.
    pub fn from_paged_checkpoint_root(
        hot: Self,
        archive_bucket_page_ids: OrdMap<DecisionArchivePageKey, String>,
        archive_receipt_count: u64,
        archive_receipt_root: &str,
    ) -> Result<Self, DecisionError> {
        Self::from_paged_checkpoint_root_with_resident_pages(
            hot,
            archive_bucket_page_ids,
            archive_receipt_count,
            archive_receipt_root,
            std::iter::empty(),
        )
    }

    /// Restores a root-only checkpoint while retaining only locator pages
    /// needed to validate hot-to-archive references. Every supplied page is
    /// authenticated against the committed directory before its receipts
    /// become resident.
    pub fn from_paged_checkpoint_root_with_resident_pages(
        mut hot: Self,
        archive_bucket_page_ids: OrdMap<DecisionArchivePageKey, String>,
        archive_receipt_count: u64,
        archive_receipt_root: &str,
        resident_pages: impl IntoIterator<Item = DecisionArchiveBucketPage>,
    ) -> Result<Self, DecisionError> {
        if !hot.archive_receipt_buckets.is_empty()
            || !hot.archive_bucket_page_ids.is_empty()
            || hot.archive_receipt_count != 0
        {
            return Err(archive_error(
                "paged decision hot state must not contain archive receipts or archive metadata",
            ));
        }
        hot.archive_bucket_page_ids = archive_bucket_page_ids;
        hot.archive_receipt_count = archive_receipt_count;
        for page in resident_pages {
            page.validate()?;
            let page_key = DecisionArchivePageKey {
                bucket: page.bucket,
                segment: page.segment,
            };
            let page_id = page.state_page_id()?;
            if hot.archive_bucket_page_ids.get(&page_key) != Some(&page_id)
                || hot.archive_receipt_buckets.contains_key(&page_key)
            {
                return Err(archive_error(
                    "resident decision archive page is absent from or disagrees with the committed directory",
                ));
            }
            let mut receipts = OrdMap::new();
            for receipt in page.receipts {
                if receipts
                    .insert(
                        receipt.key.clone(),
                        CompactDecisionArchiveReceipt::from_receipt(&receipt)?,
                    )
                    .is_some()
                {
                    return Err(archive_error(
                        "resident decision archive page contains duplicate receipts",
                    ));
                }
            }
            hot.archive_receipt_buckets.insert(page_key, receipts);
        }
        hot.validate()?;
        if hot.archive_receipt_root()? != archive_receipt_root {
            return Err(archive_error(
                "paged decision archive directory root is inconsistent",
            ));
        }
        Ok(hot)
    }

    pub fn archive_receipt_commitment(&self) -> Result<String, DecisionError> {
        self.archive_receipt_root()
    }

    /// Returns the authoritative decision-state commitment independently of
    /// which authenticated archive locator pages are currently resident.
    pub fn authoritative_commitment(&self) -> Result<String, DecisionError> {
        decision_archive_hash(
            "canwu.commitment.decisions.v2",
            &(
                &self.controllers,
                self.hot_history_root()?,
                self.archive_receipt_root()?,
            ),
        )
    }

    pub fn archived_decision_history_page(
        &self,
        cursor: Option<&DecisionHistoryCursor>,
        budget: DecisionHistoryQueryBudget,
        provider: &dyn DecisionArchiveProvider,
    ) -> Result<DecisionHistoryPage, DecisionError> {
        if budget.max_results == 0
            || budget.max_results > MAX_DECISION_HISTORY_PAGE_SIZE
            || budget.max_provider_calls == 0
            || budget.max_decoded_bytes == 0
            || budget.max_decoded_bytes > MAX_DECISION_HISTORY_PAGE_BYTES
        {
            return Err(history_budget_error(
                "decision history page budget is zero, inconsistent, or exceeds the hard limit",
            ));
        }
        let archive_root = self.archive_receipt_root()?;
        if cursor.is_some_and(|cursor| cursor.archive_root != archive_root) {
            return Err(history_unavailable(
                "decision history cursor belongs to a different archive generation",
            ));
        }
        if cursor.is_some_and(|cursor| {
            cursor.bucket >= DECISION_HISTORY_BUCKET_COUNT
                || decision_history_page_key(&cursor.after).ok()
                    != Some(DecisionArchivePageKey {
                        bucket: cursor.bucket,
                        segment: cursor.segment,
                    })
        }) {
            return Err(history_unavailable(
                "decision history cursor has an invalid bucket position",
            ));
        }
        let start_bucket = cursor.map_or(
            DecisionArchivePageKey {
                bucket: 0,
                segment: 0,
            },
            |cursor| DecisionArchivePageKey {
                bucket: cursor.bucket,
                segment: cursor.segment,
            },
        );
        let mut selected = Vec::<(DecisionArchivePageKey, DecisionArchiveReceipt)>::new();
        let mut provider_calls = 0_usize;
        for (bucket, page_id) in self.archive_bucket_page_ids.range(start_bucket..) {
            let loaded;
            let receipts = if let Some(receipts) = self.archive_receipt_buckets.get(bucket) {
                receipts
                    .iter()
                    .map(|(key, receipt)| receipt.to_receipt(key))
                    .collect::<Vec<_>>()
            } else {
                provider_calls = provider_calls.checked_add(1).ok_or_else(|| {
                    history_budget_error("decision history provider-call count overflowed")
                })?;
                if provider_calls > budget.max_provider_calls {
                    return Err(history_budget_error(
                        "decision history page exceeded its provider-call budget",
                    ));
                }
                loaded = provider
                    .load_decision_archive_bucket_page(page_id)?
                    .ok_or_else(|| {
                        history_unavailable("decision locator bucket page is unavailable")
                    })?;
                loaded.validate()?;
                if loaded.bucket != bucket.bucket
                    || loaded.segment != bucket.segment
                    || loaded.state_page_id()? != *page_id
                {
                    return Err(history_unavailable(
                        "decision locator provider returned a mismatched bucket page",
                    ));
                }
                loaded.receipts
            };
            if let Some(cursor) = cursor
                .filter(|cursor| cursor.bucket == bucket.bucket && cursor.segment == bucket.segment)
            {
                let after = &cursor.after;
                for receipt in receipts.iter().filter(|receipt| receipt.key > *after) {
                    selected.push((*bucket, receipt.clone()));
                    if selected.len() > budget.max_results {
                        break;
                    }
                }
            } else {
                for receipt in receipts {
                    selected.push((*bucket, receipt));
                    if selected.len() > budget.max_results {
                        break;
                    }
                }
            }
            if selected.len() > budget.max_results {
                break;
            }
        }
        let has_more = selected.len() > budget.max_results;
        let selected = &selected[..selected.len().min(budget.max_results)];
        let mut records = Vec::with_capacity(selected.len());
        let mut decoded_bytes = 0_u64;
        for (_, receipt) in selected {
            provider_calls = provider_calls.checked_add(1).ok_or_else(|| {
                history_budget_error("decision history provider-call count overflowed")
            })?;
            if provider_calls > budget.max_provider_calls {
                return Err(history_budget_error(
                    "decision history page exceeded its provider-call budget",
                ));
            }
            let blob = provider
                .load_decision_archive(&receipt.locator)?
                .ok_or_else(|| {
                    history_unavailable("decision archive provider omitted a committed page member")
                })?;
            blob.validate()?;
            if blob.key != receipt.key || blob.content_id()? != receipt.content_id {
                return Err(history_unavailable(
                    "decision archive provider returned a mismatched page member",
                ));
            }
            decoded_bytes = decoded_bytes
                .checked_add(receipt.encoded_bytes)
                .ok_or_else(|| history_budget_error("decision history byte count overflowed"))?;
            if decoded_bytes > budget.max_decoded_bytes {
                return Err(history_budget_error(
                    "decision history page exceeded its decoded-byte budget",
                ));
            }
            records.push(blob.record);
        }
        let next_cursor = if has_more {
            selected
                .last()
                .map(|(bucket, receipt)| DecisionHistoryCursor {
                    archive_root: archive_root.clone(),
                    bucket: bucket.bucket,
                    segment: bucket.segment,
                    after: receipt.key.clone(),
                })
        } else {
            None
        };
        Ok(DecisionHistoryPage {
            archive_root,
            records,
            next_cursor,
            provider_calls: provider_calls as u64,
            decoded_bytes,
        })
    }

    /// Enumerates every archive page and payload locator committed by this
    /// decision root. This is an offline GC operation; ordinary lookups still
    /// load only the exact hash bucket for the requested key.
    pub fn archive_reachability(
        &self,
        provider: &dyn DecisionArchiveProvider,
    ) -> Result<DecisionArchiveReachability, DecisionError> {
        let mut reachable = DecisionArchiveReachability::default();
        let mut receipt_count = 0_u64;
        for (bucket, page_id) in &self.archive_bucket_page_ids {
            let page = provider
                .load_decision_archive_bucket_page(page_id)?
                .ok_or_else(|| {
                    history_unavailable("decision archive bucket page is unavailable")
                })?;
            page.validate()?;
            if page.bucket != bucket.bucket
                || page.segment != bucket.segment
                || page.state_page_id()? != *page_id
            {
                return Err(history_unavailable(
                    "decision archive reachability provider returned a mismatched page",
                ));
            }
            reachable.bucket_page_ids.insert(page_id.clone());
            for receipt in page.receipts {
                receipt_count = receipt_count
                    .checked_add(1)
                    .ok_or_else(|| archive_error("decision archive receipt count overflowed"))?;
                reachable.blob_locators.insert(receipt.locator);
            }
        }
        if receipt_count != self.archive_receipt_count {
            return Err(archive_error(
                "decision archive reachability count disagrees with its directory",
            ));
        }
        Ok(reachable)
    }

    pub fn load_decision_history(
        &self,
        key: &DecisionHistoryKey,
        provider: &dyn DecisionArchiveProvider,
    ) -> Result<Option<DecisionArchiveRecord>, DecisionError> {
        if let Some(record) = self.hot_archive_record(key) {
            return Ok(Some(record));
        }
        let Some(receipt) = self.archive_receipt_with_provider(key, provider)? else {
            return Ok(None);
        };
        let blob = provider
            .load_decision_archive(&receipt.locator)?
            .ok_or_else(|| archive_error("decision archive provider omitted a committed blob"))?;
        blob.validate()?;
        let content_id = blob.content_id()?;
        if blob.key != *key || content_id != receipt.content_id {
            return Err(archive_error(
                "decision archive provider returned content that disagrees with its receipt",
            ));
        }
        Ok(Some(blob.record))
    }

    pub fn prepare_decision_archive(
        &self,
        keys: &[DecisionHistoryKey],
    ) -> Result<PreparedDecisionArchive, DecisionError> {
        if keys.is_empty() || keys.len() > MAX_DECISION_ARCHIVE_BATCH_ENTRIES {
            return Err(archive_error(
                "decision archive batch is empty or exceeds the bounded entry limit",
            ));
        }
        let mut canonical_keys = keys.to_vec();
        canonical_keys.sort();
        canonical_keys.dedup();
        if canonical_keys.len() != keys.len() {
            return Err(archive_error(
                "decision archive batch contains duplicate keys",
            ));
        }
        let source_root = self.hot_history_root()?;
        let mut blobs = Vec::with_capacity(canonical_keys.len());
        let mut receipts = Vec::with_capacity(canonical_keys.len());
        for key in canonical_keys {
            if self.contains_archived_key(&key) {
                return Err(archive_error("decision history is already archived"));
            }
            let record = self
                .hot_archive_record(&key)
                .ok_or_else(|| archive_error("decision archive key is not resident"))?;
            self.validate_archive_eligibility(&record)?;
            let blob = DecisionArchiveBlob {
                format_version: DECISION_ARCHIVE_FORMAT_VERSION,
                key: key.clone(),
                record,
            };
            let content_id = blob.content_id()?;
            let encoded_bytes = u64::try_from(
                serde_json::to_vec(&blob)
                    .map_err(|error| {
                        archive_error(format!("cannot encode decision archive: {error}"))
                    })?
                    .len(),
            )
            .map_err(|_| {
                archive_error("decision archive blob exceeds the persistent byte range")
            })?;
            receipts.push(DecisionArchiveReceipt {
                format_version: DECISION_ARCHIVE_FORMAT_VERSION,
                key,
                locator: content_id.clone(),
                content_id,
                encoded_bytes,
            });
            blobs.push(blob);
        }
        let token = decision_archive_hash(
            "canwu.decision.archive-token.v1",
            &(&source_root, &receipts),
        )?;
        Ok(PreparedDecisionArchive {
            source_root,
            token,
            blobs,
            receipts,
        })
    }

    pub fn commit_decision_archive(
        &self,
        prepared: &PreparedDecisionArchive,
        provider: &dyn DecisionArchiveProvider,
    ) -> Result<Self, DecisionError> {
        let verified = self.verify_decision_archive(prepared, provider)?;
        self.commit_verified_decision_archive(&verified)
    }

    pub fn verify_decision_archive(
        &self,
        prepared: &PreparedDecisionArchive,
        provider: &dyn DecisionArchiveProvider,
    ) -> Result<VerifiedDecisionArchiveCommit, DecisionError> {
        let keys = prepared
            .receipts
            .iter()
            .map(|receipt| receipt.key.clone())
            .collect::<Vec<_>>();
        let expected = self.prepare_decision_archive(&keys)?;
        if expected != *prepared {
            return Err(archive_error(
                "prepared decision archive is stale or altered",
            ));
        }
        for (blob, receipt) in prepared.blobs.iter().zip(&prepared.receipts) {
            let stored = provider
                .load_decision_archive(&receipt.locator)?
                .ok_or_else(|| {
                    archive_error("prepared decision archive is not durably readable")
                })?;
            if stored != *blob || stored.content_id()? != receipt.content_id {
                return Err(archive_error(
                    "stored decision archive blob failed verification",
                ));
            }
        }
        let mut additions = std::collections::BTreeMap::<
            DecisionArchivePageKey,
            Vec<&DecisionArchiveReceipt>,
        >::new();
        for receipt in &prepared.receipts {
            additions
                .entry(decision_history_page_key(&receipt.key)?)
                .or_default()
                .push(receipt);
        }
        let mut page_replacements = Vec::with_capacity(additions.len());
        for (page_key, receipts) in additions {
            let previous_page_id = self.archive_bucket_page_ids.get(&page_key).cloned();
            let mut merged = if let Some(resident) = self.archive_receipt_buckets.get(&page_key) {
                resident
                    .iter()
                    .map(|(key, receipt)| (key.clone(), receipt.to_receipt(key)))
                    .collect::<std::collections::BTreeMap<_, _>>()
            } else if let Some(page_id) = previous_page_id.as_deref() {
                let page = provider
                    .load_decision_archive_bucket_page(page_id)?
                    .ok_or_else(|| {
                        history_unavailable(
                            "decision archive locator page is unavailable for authenticated merge",
                        )
                    })?;
                page.validate()?;
                if page.bucket != page_key.bucket
                    || page.segment != page_key.segment
                    || page.state_page_id()? != page_id
                {
                    return Err(history_unavailable(
                        "decision archive locator provider returned a mismatched merge page",
                    ));
                }
                page.receipts
                    .into_iter()
                    .map(|receipt| (receipt.key.clone(), receipt))
                    .collect::<std::collections::BTreeMap<_, _>>()
            } else {
                std::collections::BTreeMap::new()
            };
            for receipt in receipts {
                if merged
                    .insert(receipt.key.clone(), receipt.clone())
                    .is_some()
                {
                    return Err(archive_error(
                        "decision archive locator merge would replace an existing receipt",
                    ));
                }
            }
            let page = DecisionArchiveBucketPage {
                format_version: DECISION_ARCHIVE_BUCKET_PAGE_FORMAT_VERSION,
                bucket: page_key.bucket,
                segment: page_key.segment,
                receipts: merged.into_values().collect(),
            };
            page.validate()?;
            page_replacements.push(VerifiedDecisionArchivePageReplacement {
                page_key,
                previous_page_id,
                page,
            });
        }
        Ok(VerifiedDecisionArchiveCommit {
            format_version: DECISION_ARCHIVE_FORMAT_VERSION,
            source_root: prepared.source_root.clone(),
            token: prepared.token.clone(),
            receipts: prepared.receipts.clone(),
            page_replacements,
        })
    }

    pub fn commit_verified_decision_archive(
        &self,
        verified: &VerifiedDecisionArchiveCommit,
    ) -> Result<Self, DecisionError> {
        if verified.format_version != DECISION_ARCHIVE_FORMAT_VERSION {
            return Err(archive_error(
                "verified decision archive uses an unsupported format",
            ));
        }
        let mut replacement_ids = std::collections::BTreeMap::new();
        for replacement in &verified.page_replacements {
            replacement.page.validate()?;
            if replacement.page_key.bucket != replacement.page.bucket
                || replacement.page_key.segment != replacement.page.segment
                || replacement_ids
                    .insert(replacement.page_key, replacement.page.state_page_id()?)
                    .is_some()
                || replacement
                    .previous_page_id
                    .as_ref()
                    .is_some_and(|page_id| !canonical_archive_hash(page_id))
            {
                return Err(archive_error(
                    "verified decision archive page replacement is malformed",
                ));
            }
        }
        let mut receipts_by_page = std::collections::BTreeMap::<
            DecisionArchivePageKey,
            Vec<&DecisionArchiveReceipt>,
        >::new();
        for receipt in &verified.receipts {
            receipts_by_page
                .entry(decision_history_page_key(&receipt.key)?)
                .or_default()
                .push(receipt);
        }
        if receipts_by_page.len() != replacement_ids.len()
            || receipts_by_page
                .keys()
                .any(|page_key| !replacement_ids.contains_key(page_key))
        {
            return Err(archive_error(
                "verified decision archive does not replace every touched locator page",
            ));
        }
        let already_applied = verified.receipts.iter().all(|receipt| {
            !matches!(
                self.decision_locator(&receipt.key),
                DecisionHistoryLocation::Hot
            )
        }) && replacement_ids
            .iter()
            .all(|(page_key, page_id)| self.archive_bucket_page_ids.get(page_key) == Some(page_id));
        if already_applied {
            return Ok(self.clone());
        }
        for replacement in &verified.page_replacements {
            if self.archive_bucket_page_ids.get(&replacement.page_key)
                != replacement.previous_page_id.as_ref()
            {
                return Err(archive_error(
                    "verified decision archive locator base changed before commit",
                ));
            }
            let receipts = receipts_by_page.get(&replacement.page_key).ok_or_else(|| {
                archive_error("verified decision archive locator page has no touched receipts")
            })?;
            for receipt in receipts {
                if replacement
                    .page
                    .receipts
                    .binary_search_by(|candidate| candidate.key.cmp(&receipt.key))
                    .ok()
                    .is_none_or(|index| replacement.page.receipts[index] != **receipt)
                {
                    return Err(archive_error(
                        "verified decision archive locator page omits a touched receipt",
                    ));
                }
            }
        }
        let keys = verified
            .receipts
            .iter()
            .map(|receipt| receipt.key.clone())
            .collect::<Vec<_>>();
        let expected = self.prepare_decision_archive(&keys)?;
        if expected.source_root != verified.source_root
            || expected.token != verified.token
            || expected.receipts != verified.receipts
        {
            return Err(archive_error(
                "verified decision archive is stale or altered",
            ));
        }
        let mut next = self.clone();
        for receipt in &verified.receipts {
            let hot_record = next
                .hot_archive_record(&receipt.key)
                .ok_or_else(|| archive_error("decision history disappeared during commit"))?;
            next.remove_hot_history_record(&hot_record)?;
            match receipt.key {
                DecisionHistoryKey::Ticket(id) => {
                    next.tickets.remove(&id);
                }
                DecisionHistoryKey::Attempt(id) => {
                    let ordinal = next.attempts_by_request.remove(&id).ok_or_else(|| {
                        archive_error("decision attempt disappeared during commit")
                    })?;
                    next.attempts.remove_ordinal(ordinal);
                }
                DecisionHistoryKey::Trace(id) => {
                    next.traces.remove_ordinal(id.get());
                }
            }
        }
        next.archive_receipt_count = next
            .archive_receipt_count
            .checked_add(verified.receipts.len() as u64)
            .ok_or_else(|| archive_error("decision archive receipt count overflowed"))?;
        for replacement in &verified.page_replacements {
            let receipts = replacement
                .page
                .receipts
                .iter()
                .map(|receipt| {
                    Ok((
                        receipt.key.clone(),
                        CompactDecisionArchiveReceipt::from_receipt(receipt)?,
                    ))
                })
                .collect::<Result<OrdMap<_, _>, DecisionError>>()?;
            next.archive_receipt_buckets
                .insert(replacement.page_key, receipts);
            next.archive_bucket_page_ids
                .insert(replacement.page_key, replacement.page.state_page_id()?);
        }
        next.validate_archive_directory_shape()?;
        for receipt in &verified.receipts {
            if matches!(
                next.decision_locator(&receipt.key),
                DecisionHistoryLocation::Hot
            ) {
                return Err(archive_error(
                    "decision archive commit left a touched key in hot state",
                ));
            }
        }
        Ok(next)
    }

    fn hot_archive_record(&self, key: &DecisionHistoryKey) -> Option<DecisionArchiveRecord> {
        match key {
            DecisionHistoryKey::Ticket(id) => self
                .tickets
                .get(id)
                .map(|ticket| ticket.as_ref().clone())
                .map(|ticket| DecisionArchiveRecord::Ticket { ticket }),
            DecisionHistoryKey::Attempt(id) => self
                .attempt(*id)
                .cloned()
                .map(|attempt| DecisionArchiveRecord::Attempt { attempt }),
            DecisionHistoryKey::Trace(id) => self
                .traces
                .get_ordinal(id.get())
                .cloned()
                .map(|trace| DecisionArchiveRecord::Trace { trace }),
        }
    }

    fn validate_archive_eligibility(
        &self,
        record: &DecisionArchiveRecord,
    ) -> Result<(), DecisionError> {
        match record {
            DecisionArchiveRecord::Ticket { ticket } if ticket.is_open() => Err(archive_error(
                "open decision tickets cannot leave the hot mutation path",
            )),
            DecisionArchiveRecord::Trace { trace } => {
                let terminal = self
                    .tickets
                    .get(&trace.ticket_id)
                    .is_some_and(|ticket| !ticket.is_open())
                    || self.contains_archived_key(&DecisionHistoryKey::Ticket(trace.ticket_id));
                if terminal {
                    Ok(())
                } else {
                    Err(archive_error(
                        "decision traces can archive only after their ticket is terminal",
                    ))
                }
            }
            DecisionArchiveRecord::Ticket { .. } | DecisionArchiveRecord::Attempt { .. } => Ok(()),
        }
    }

    fn hot_history_root(&self) -> Result<String, DecisionError> {
        let resident_count = (self.tickets.len() as u64)
            .checked_add(self.traces.len() as u64)
            .and_then(|count| count.checked_add(self.attempts.len() as u64))
            .ok_or_else(|| archive_error("decision hot-history count overflowed"))?;
        if resident_count != self.hot_history_accumulator.count {
            return Err(archive_error(
                "decision hot-history accumulator count is inconsistent",
            ));
        }
        decision_archive_hash(
            "canwu.decision.hot-history-root.v2",
            &(
                self.hot_history_accumulator.count,
                self.hot_history_accumulator.xor,
                self.hot_history_accumulator.sum,
            ),
        )
    }

    pub fn hot_history_commitment(&self) -> Result<String, DecisionError> {
        self.hot_history_root()
    }

    fn archive_receipt_root(&self) -> Result<String, DecisionError> {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"canwu.decision.archive-receipt-root.v3");
        hasher.update(&[0]);
        hasher.update(&self.archive_receipt_count.to_be_bytes());
        hasher.update(&(self.archive_bucket_page_ids.len() as u64).to_be_bytes());
        for (bucket, page_id) in &self.archive_bucket_page_ids {
            hasher.update(&bucket.bucket.to_be_bytes());
            hasher.update(&[bucket.segment]);
            hasher.update(&decode_archive_hash(page_id)?);
        }
        Ok(hasher.finalize().to_hex().to_string())
    }

    #[must_use]
    pub fn decision_hot_state(&self) -> DecisionHotState {
        DecisionHotState {
            ticket_count: self.tickets.len() as u64,
            attempt_count: self.attempts.len() as u64,
            trace_count: self.traces.len() as u64,
        }
    }

    pub fn append_attempt(&mut self, attempt: DecisionAttemptRecord) -> Result<(), DecisionError> {
        validate_attempt_shape(self, &attempt)?;
        if self.attempts_by_request.contains_key(&attempt.request_id) {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "decision attempts must use unique request IDs",
            ));
        }
        let request_id = attempt.request_id;
        self.insert_hot_history_record(&DecisionArchiveRecord::Attempt {
            attempt: attempt.clone(),
        })?;
        let ordinal = self.attempts.push(attempt);
        self.attempts_by_request.insert(request_id, ordinal);
        Ok(())
    }

    pub fn open_tickets(&self) -> impl Iterator<Item = &DecisionTicket> {
        self.tickets
            .values()
            .map(Arc::as_ref)
            .filter(|ticket| ticket.is_open())
    }

    pub fn validate(&self) -> Result<(), DecisionError> {
        self.validate_archive_directory_shape()?;
        if self.attempts_by_request.len() != self.attempts.len() {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "decision attempt request index is inconsistent",
            ));
        }
        let mut expected_deadlines = OrdMap::<SimTime, OrdSet<DecisionTicketId>>::new();
        for ticket in self.tickets.values().filter(|ticket| ticket.is_open()) {
            if let Some(deadline) = ticket.deadline {
                expected_deadlines
                    .entry(deadline)
                    .or_default()
                    .insert(ticket.id);
            }
        }
        if expected_deadlines != self.deadline_index {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "decision deadline index is inconsistent",
            ));
        }
        for (id, controller) in &self.controllers {
            if id != &controller.id {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidController,
                    "controller map key does not match its persisted identity",
                ));
            }
            controller.validate()?;
        }
        for (bucket, receipts) in &self.archive_receipt_buckets {
            if bucket.bucket >= DECISION_HISTORY_BUCKET_COUNT || receipts.is_empty() {
                return Err(archive_error("decision archive receipt bucket is invalid"));
            }
            for (key, receipt) in receipts {
                if decision_history_page_key(key)? != *bucket
                    || receipt.encoded_bytes == 0
                    || matches!(self.decision_locator(key), DecisionHistoryLocation::Hot)
                {
                    return Err(archive_error(
                        "decision archive receipt is invalid or overlaps hot history",
                    ));
                }
            }
            let expected_page_id = self
                .decision_archive_bucket_page(*bucket)?
                .ok_or_else(|| archive_error("nonempty archive bucket has no page"))?
                .state_page_id()?;
            if self.archive_bucket_page_ids.get(bucket) != Some(&expected_page_id) {
                return Err(archive_error(
                    "decision archive bucket page commitment is inconsistent",
                ));
            }
        }
        if self.attempts.iter().any(|attempt| {
            attempt.request_commitment.len() != 64
                || !attempt
                    .request_commitment
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }) {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "decision attempts require a canonical request commitment",
            ));
        }
        for (id, ticket) in &self.tickets {
            if id != &ticket.id || !self.controllers.contains_key(&ticket.assigned_controller) {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "ticket identity or assigned controller is invalid",
                ));
            }
            ticket.validate()?;
        }
        for (ordinal, trace) in self.traces.ordinals().zip(self.traces.iter()) {
            if trace.id.get() != ordinal || ordinal >= self.traces.next_ordinal() {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "decision trace identity disagrees with its persistent log ordinal",
                ));
            }
            let ticket_version_valid = self.tickets.get(&trace.ticket_id).is_some_and(|ticket| {
                trace.ticket_version != 0 && trace.ticket_version <= ticket.version
            }) || self
                .contains_archived_key(&DecisionHistoryKey::Ticket(trace.ticket_id));
            if !ticket_version_valid || !self.controllers.contains_key(&trace.controller_id) {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "decision trace ticket, version, or controller is invalid",
                ));
            }
        }
        for ticket in self.tickets.values() {
            if let DecisionTicketState::Resolved { trace_id, .. } = ticket.state {
                let hot_trace_matches = self
                    .traces
                    .get_ordinal(trace_id.get())
                    .is_some_and(|trace| trace.id == trace_id && trace.ticket_id == ticket.id);
                let archived_trace_exists =
                    self.contains_archived_key(&DecisionHistoryKey::Trace(trace_id));
                if !hot_trace_matches && !archived_trace_exists {
                    return Err(DecisionError::new(
                        DecisionErrorCode::InvalidDecision,
                        "resolved ticket does not reference hot or archived trace evidence",
                    ));
                }
            }
        }
        let mut request_ids = std::collections::BTreeSet::new();
        for (ordinal, attempt) in self.attempts.ordinals().zip(self.attempts.iter()) {
            if attempt.request_id.get() == 0 || !request_ids.insert(attempt.request_id) {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "decision attempts must use unique nonzero request IDs",
                ));
            }
            if self.attempts_by_request.get(&attempt.request_id) != Some(&ordinal) {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "decision attempt request index is inconsistent",
                ));
            }
            match &attempt.outcome {
                DecisionAttemptOutcome::Accepted {
                    trace_id,
                    command_request_id,
                } => {
                    if command_request_id.is_some() && trace_id.is_none() {
                        return Err(DecisionError::new(
                            DecisionErrorCode::InvalidDecision,
                            "accepted decision commands require a decision trace",
                        ));
                    }
                    if trace_id.is_some_and(|trace_id| {
                        self.traces.get_ordinal(trace_id.get()).is_none()
                            && !self.contains_archived_key(&DecisionHistoryKey::Trace(trace_id))
                    }) {
                        return Err(DecisionError::new(
                            DecisionErrorCode::InvalidDecision,
                            "accepted decision attempt references unavailable trace evidence",
                        ));
                    }
                }
                DecisionAttemptOutcome::Rejected { message, .. } => {
                    require_text(message, "decision rejection message")?;
                }
            }
        }
        Ok(())
    }

    fn validate_archive_directory_shape(&self) -> Result<(), DecisionError> {
        if self
            .archive_bucket_page_ids
            .iter()
            .any(|(bucket, page_id)| {
                bucket.bucket >= DECISION_HISTORY_BUCKET_COUNT || !canonical_archive_hash(page_id)
            })
        {
            return Err(archive_error(
                "decision archive bucket directory is malformed",
            ));
        }
        if (self.archive_receipt_count == 0) != self.archive_bucket_page_ids.is_empty() {
            return Err(archive_error(
                "decision archive receipt count and page directory disagree",
            ));
        }
        if !self.archive_receipt_buckets.is_empty() {
            let resident_count =
                self.archive_receipt_buckets
                    .values()
                    .try_fold(0_u64, |total, receipts| {
                        total.checked_add(receipts.len() as u64).ok_or_else(|| {
                            archive_error("decision archive receipt count overflowed")
                        })
                    })?;
            if resident_count > self.archive_receipt_count {
                return Err(archive_error(
                    "resident decision archive receipt count exceeds the committed total",
                ));
            }
        }
        Ok(())
    }

    pub fn apply(
        &mut self,
        mutation: DecisionMutation,
        at: SimTime,
        trace_id: Option<DecisionTraceId>,
    ) -> Result<PreparedDecision, DecisionError> {
        let prepared = match mutation {
            DecisionMutation::RegisterController { controller } => {
                controller.validate()?;
                if self.controllers.contains_key(&controller.id) {
                    return Err(DecisionError::new(
                        DecisionErrorCode::DuplicateController,
                        format!(
                            "decision controller {} is already registered",
                            controller.id
                        ),
                    ));
                }
                self.controllers
                    .insert(controller.id.clone(), Arc::new(controller));
                PreparedDecision::default()
            }
            DecisionMutation::Open { mut ticket } => {
                ticket.validate()?;
                if self.tickets.contains_key(&ticket.id) {
                    return Err(DecisionError::new(
                        DecisionErrorCode::DuplicateTicket,
                        format!("decision ticket {} is already present", ticket.id),
                    ));
                }
                if !self.controllers.contains_key(&ticket.assigned_controller) {
                    return Err(DecisionError::new(
                        DecisionErrorCode::InvalidController,
                        format!(
                            "decision ticket {} names unknown controller {}",
                            ticket.id, ticket.assigned_controller
                        ),
                    ));
                }
                if ticket.deadline.is_some_and(|deadline| deadline < at) {
                    return Err(DecisionError::new(
                        DecisionErrorCode::InvalidDecision,
                        "decision deadline precedes its admission time",
                    ));
                }
                let persisted = DecisionTicket {
                    id: ticket.id,
                    definition: ticket.definition,
                    decision_maker: ticket.decision_maker,
                    assigned_controller: ticket.assigned_controller,
                    summary: ticket.summary,
                    context: ticket.context,
                    options: std::mem::take(&mut ticket.options),
                    opened_at: at,
                    updated_at: at,
                    deadline: ticket.deadline,
                    version: 1,
                    state: DecisionTicketState::Open,
                };
                if let Some(deadline) = persisted.deadline {
                    self.deadline_index
                        .entry(deadline)
                        .or_default()
                        .insert(persisted.id);
                }
                self.insert_hot_history_record(&DecisionArchiveRecord::Ticket {
                    ticket: persisted.clone(),
                })?;
                self.tickets.insert(persisted.id, Arc::new(persisted));
                PreparedDecision::default()
            }
            DecisionMutation::ReplaceOptions {
                ticket_id,
                expected_version,
                context,
                mut options,
            } => {
                context.validate()?;
                canonicalize_options(&mut options)?;
                let previous = self
                    .tickets
                    .get(&ticket_id)
                    .map(|ticket| ticket.as_ref().clone())
                    .ok_or_else(|| {
                        DecisionError::new(
                            DecisionErrorCode::TicketNotFound,
                            format!("decision ticket {ticket_id} was not found"),
                        )
                    })?;
                let ticket = self.open_ticket_mut(ticket_id, expected_version, at)?;
                ticket.context = context;
                ticket.options = options;
                ticket.updated_at = at;
                ticket.version = ticket.version.checked_add(1).ok_or_else(|| {
                    DecisionError::new(
                        DecisionErrorCode::InvalidDecision,
                        "decision ticket version is exhausted",
                    )
                })?;
                ticket.validate()?;
                let updated = ticket.clone();
                let _ = ticket;
                self.replace_hot_history_record(
                    &DecisionArchiveRecord::Ticket { ticket: previous },
                    &DecisionArchiveRecord::Ticket { ticket: updated },
                )?;
                PreparedDecision::default()
            }
            DecisionMutation::Resolve {
                ticket_id,
                expected_version,
                controller_id,
                policy,
                decision,
                command_request_id,
            } => self.resolve(
                ticket_id,
                expected_version,
                &controller_id,
                policy,
                decision,
                command_request_id,
                at,
                trace_id.ok_or_else(|| {
                    DecisionError::new(
                        DecisionErrorCode::InvalidDecision,
                        "decision resolution requires a claimed trace ID",
                    )
                })?,
            )?,
            DecisionMutation::Cancel {
                ticket_id,
                expected_version,
                reason,
            } => {
                require_text(&reason, "decision cancellation reason")?;
                let previous = self
                    .tickets
                    .get(&ticket_id)
                    .map(|ticket| ticket.as_ref().clone())
                    .ok_or_else(|| {
                        DecisionError::new(
                            DecisionErrorCode::TicketNotFound,
                            format!("decision ticket {ticket_id} was not found"),
                        )
                    })?;
                let ticket = self.open_ticket_mut(ticket_id, expected_version, at)?;
                ticket.updated_at = at;
                ticket.version = ticket.version.checked_add(1).ok_or_else(|| {
                    DecisionError::new(
                        DecisionErrorCode::InvalidDecision,
                        "decision ticket version is exhausted",
                    )
                })?;
                ticket.state = DecisionTicketState::Cancelled { reason };
                ticket.validate()?;
                let deadline = ticket.deadline;
                let updated = ticket.clone();
                let _ = ticket;
                self.replace_hot_history_record(
                    &DecisionArchiveRecord::Ticket { ticket: previous },
                    &DecisionArchiveRecord::Ticket { ticket: updated },
                )?;
                self.remove_deadline(ticket_id, deadline);
                PreparedDecision::default()
            }
        };
        self.advance_time(at)?;
        Ok(prepared)
    }

    fn open_ticket_mut(
        &mut self,
        ticket_id: DecisionTicketId,
        expected_version: u64,
        at: SimTime,
    ) -> Result<&mut DecisionTicket, DecisionError> {
        let ticket = self.tickets.get_mut(&ticket_id).ok_or_else(|| {
            DecisionError::new(
                DecisionErrorCode::TicketNotFound,
                format!("decision ticket {ticket_id} was not found"),
            )
        })?;
        let ticket = Arc::make_mut(ticket);
        if !ticket.is_open() || ticket.deadline.is_some_and(|deadline| deadline < at) {
            return Err(DecisionError::new(
                DecisionErrorCode::ClosedTicket,
                format!("decision ticket {ticket_id} is not open"),
            ));
        }
        if ticket.version != expected_version {
            return Err(DecisionError::new(
                DecisionErrorCode::VersionConflict,
                format!(
                    "decision ticket {ticket_id} is at version {}, expected {expected_version}",
                    ticket.version
                ),
            ));
        }
        Ok(ticket)
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve(
        &mut self,
        ticket_id: DecisionTicketId,
        expected_version: u64,
        controller_id: &str,
        policy: DecisionPolicyIdentity,
        decision: PolicyDecision,
        command_request_id: Option<CommandRequestId>,
        at: SimTime,
        trace_id: DecisionTraceId,
    ) -> Result<PreparedDecision, DecisionError> {
        let controller = self.controllers.get(controller_id).ok_or_else(|| {
            DecisionError::new(
                DecisionErrorCode::InvalidController,
                format!("decision controller {controller_id} was not found"),
            )
        })?;
        if controller.policy != policy {
            return Err(DecisionError::new(
                DecisionErrorCode::PolicyMismatch,
                "decision resolution policy does not match the persisted controller binding",
            ));
        }
        if (controller.policy.kind == crate::DecisionPolicyKind::Random)
            != decision.random.is_some()
        {
            return Err(DecisionError::new(
                DecisionErrorCode::PolicyMismatch,
                "random decision controllers require random draw evidence, and other controllers reject it",
            ));
        }
        let previous_ticket = self
            .tickets
            .get(&ticket_id)
            .map(|ticket| ticket.as_ref().clone())
            .ok_or_else(|| {
                DecisionError::new(
                    DecisionErrorCode::TicketNotFound,
                    format!("decision ticket {ticket_id} was not found"),
                )
            })?;
        let ticket = self.open_ticket_mut(ticket_id, expected_version, at)?;
        if ticket.assigned_controller != controller_id {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidController,
                "decision resolution came from a controller not assigned to the ticket",
            ));
        }
        decision.validate(ticket)?;
        let action = match &decision.outcome {
            DecisionOutcome::Selected { option_id } => {
                ticket.option(option_id).map(|option| option.action.clone())
            }
            DecisionOutcome::Deferred { .. } => None,
            DecisionOutcome::Pending { .. } => {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "pending policy outcomes are not authoritative decision mutations",
                ));
            }
        };
        if matches!(action, Some(DecisionAction::Command { .. })) != command_request_id.is_some() {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "command actions require exactly one command request ID",
            ));
        }
        let trace = DecisionTrace {
            id: trace_id,
            ticket_id,
            ticket_version: ticket.version,
            controller_id: controller_id.to_owned(),
            policy,
            decided_at: at,
            outcome: decision.outcome.clone(),
            summary: decision.summary,
            evaluations: decision.evaluations,
            external: decision.external,
            random: decision.random,
            command_request_id,
        };
        ticket.updated_at = at;
        ticket.version = ticket.version.checked_add(1).ok_or_else(|| {
            DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "decision ticket version is exhausted",
            )
        })?;
        if let DecisionOutcome::Selected { option_id } = &trace.outcome {
            ticket.state = DecisionTicketState::Resolved {
                option_id: option_id.clone(),
                trace_id,
            };
        }
        ticket.validate()?;
        let deadline = ticket.deadline;
        let updated_ticket = ticket.clone();
        let _ = ticket;
        self.replace_hot_history_record(
            &DecisionArchiveRecord::Ticket {
                ticket: previous_ticket,
            },
            &DecisionArchiveRecord::Ticket {
                ticket: updated_ticket,
            },
        )?;
        self.remove_deadline(ticket_id, deadline);
        self.insert_hot_history_record(&DecisionArchiveRecord::Trace {
            trace: trace.clone(),
        })?;
        let ordinal = self.traces.push(trace.clone());
        debug_assert_eq!(ordinal, trace.id.get());
        Ok(PreparedDecision {
            trace: Some(trace),
            action,
        })
    }

    pub fn advance_time(&mut self, at: SimTime) -> Result<(), DecisionError> {
        let due = self
            .deadline_index
            .range(..at)
            .map(|(deadline, tickets)| (*deadline, tickets.iter().copied().collect::<Vec<_>>()))
            .collect::<Vec<_>>();
        for (deadline, ticket_ids) in due {
            self.deadline_index.remove(&deadline);
            for ticket_id in ticket_ids {
                let previous = self
                    .tickets
                    .get(&ticket_id)
                    .map(|ticket| ticket.as_ref().clone())
                    .ok_or_else(|| {
                        DecisionError::new(
                            DecisionErrorCode::InvalidDecision,
                            "decision ticket index changed during time advancement",
                        )
                    })?;
                let Some(ticket) = self.tickets.get_mut(&ticket_id) else {
                    return Err(DecisionError::new(
                        DecisionErrorCode::InvalidDecision,
                        "decision ticket index changed during time advancement",
                    ));
                };
                let ticket = Arc::make_mut(ticket);
                let updated =
                    if ticket.is_open() && ticket.deadline.is_some_and(|deadline| deadline < at) {
                        ticket.updated_at = at;
                        ticket.version = ticket.version.checked_add(1).ok_or_else(|| {
                            DecisionError::new(
                                DecisionErrorCode::InvalidDecision,
                                "decision ticket version is exhausted",
                            )
                        })?;
                        ticket.state = DecisionTicketState::Expired;
                        ticket.validate()?;
                        Some(ticket.clone())
                    } else {
                        None
                    };
                let _ = ticket;
                if let Some(updated) = updated {
                    self.replace_hot_history_record(
                        &DecisionArchiveRecord::Ticket { ticket: previous },
                        &DecisionArchiveRecord::Ticket { ticket: updated },
                    )?;
                }
            }
        }
        Ok(())
    }

    fn remove_deadline(&mut self, ticket_id: DecisionTicketId, deadline: Option<SimTime>) {
        let Some(deadline) = deadline else {
            return;
        };
        if let Some(mut tickets) = self.deadline_index.get(&deadline).cloned() {
            tickets.remove(&ticket_id);
            if tickets.is_empty() {
                self.deadline_index.remove(&deadline);
            } else {
                self.deadline_index.insert(deadline, tickets);
            }
        }
    }
}

fn validate_attempt_shape(
    state: &DecisionState,
    attempt: &DecisionAttemptRecord,
) -> Result<(), DecisionError> {
    if attempt.request_id.get() == 0 || !canonical_archive_hash(&attempt.request_commitment) {
        return Err(DecisionError::new(
            DecisionErrorCode::InvalidDecision,
            "decision attempt identity or request commitment is invalid",
        ));
    }
    match &attempt.outcome {
        DecisionAttemptOutcome::Accepted {
            trace_id,
            command_request_id,
        } => {
            if command_request_id.is_some() && trace_id.is_none() {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "accepted decision commands require a decision trace",
                ));
            }
            if trace_id.is_some_and(|trace_id| {
                state.traces.get_ordinal(trace_id.get()).is_none()
                    && !state.contains_archived_key(&DecisionHistoryKey::Trace(trace_id))
            }) {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "accepted decision attempt references unavailable trace evidence",
                ));
            }
        }
        DecisionAttemptOutcome::Rejected { message, .. } => {
            require_text(message, "decision rejection message")?;
        }
    }
    Ok(())
}

fn decision_hot_leaf_hash(
    key: &DecisionHistoryKey,
    record: &DecisionArchiveRecord,
) -> Result<[u8; 32], DecisionError> {
    if record.key() != *key {
        return Err(archive_error(
            "decision hot-history leaf identity is inconsistent",
        ));
    }
    let bytes = serde_json::to_vec(&(key, record)).map_err(|error| {
        archive_error(format!("cannot encode decision hot-history leaf: {error}"))
    })?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"canwu.decision.hot-history-leaf.v2");
    hasher.update(&[0]);
    hasher.update(&bytes);
    Ok(*hasher.finalize().as_bytes())
}

fn add_digest_mod_256(target: &mut [u8; 32], digest: [u8; 32]) {
    let mut carry = 0_u16;
    for index in (0..target.len()).rev() {
        let value = u16::from(target[index]) + u16::from(digest[index]) + carry;
        target[index] = u8::try_from(value % 256).expect("modulo 256 always fits in u8");
        carry = value >> 8;
    }
}

fn subtract_digest_mod_256(target: &mut [u8; 32], digest: [u8; 32]) {
    let mut borrow = 0_i16;
    for index in (0..target.len()).rev() {
        let value = i16::from(target[index]) - i16::from(digest[index]) - borrow;
        if value < 0 {
            target[index] =
                u8::try_from(value + 256).expect("normalized byte subtraction fits in u8");
            borrow = 1;
        } else {
            target[index] = u8::try_from(value).expect("non-negative byte subtraction fits in u8");
            borrow = 0;
        }
    }
}

pub fn decision_history_bucket(key: &DecisionHistoryKey) -> Result<u16, DecisionError> {
    Ok(decision_history_page_key(key)?.bucket)
}

pub fn decision_history_page_key(
    key: &DecisionHistoryKey,
) -> Result<DecisionArchivePageKey, DecisionError> {
    let encoded = serde_json::to_vec(key)
        .map_err(|error| archive_error(format!("cannot encode decision history key: {error}")))?;
    let digest = blake3::hash(&encoded);
    let bytes = digest.as_bytes();
    Ok(DecisionArchivePageKey {
        bucket: (u16::from(bytes[0]) << 4) | u16::from(bytes[1] >> 4),
        segment: bytes[1] & 0x0f,
    })
}

#[derive(Default)]
struct DecisionScaleArchive {
    blobs: RefCell<StdBTreeMap<String, DecisionArchiveBlob>>,
    pages: RefCell<StdBTreeMap<String, DecisionArchiveBucketPage>>,
}

impl DecisionArchiveProvider for DecisionScaleArchive {
    fn load_decision_archive(
        &self,
        locator: &str,
    ) -> Result<Option<DecisionArchiveBlob>, DecisionError> {
        Ok(self.blobs.borrow().get(locator).cloned())
    }

    fn load_decision_archive_bucket_page(
        &self,
        page_id: &str,
    ) -> Result<Option<DecisionArchiveBucketPage>, DecisionError> {
        Ok(self.pages.borrow().get(page_id).cloned())
    }
}

impl DecisionArchiveStore for DecisionScaleArchive {
    fn store_decision_archive(
        &self,
        blob: &DecisionArchiveBlob,
    ) -> Result<DecisionArchiveStoreOutcome, DecisionError> {
        let locator = blob.content_id()?;
        let mut blobs = self.blobs.borrow_mut();
        if let Some(existing) = blobs.get(&locator) {
            if existing != blob {
                return Err(archive_error(
                    "decision scale archive locator contains different content",
                ));
            }
            return Ok(DecisionArchiveStoreOutcome::AlreadyStored);
        }
        blobs.insert(locator, blob.clone());
        Ok(DecisionArchiveStoreOutcome::Stored)
    }
}

/// Runs the production decision archive lifecycle in bounded batches: append
/// terminal attempts, prepare, store, read-back verify, commit/hot-release,
/// persist root-only locator pages, restart, exact-query, and GC reachability.
#[doc(hidden)]
pub fn format8_decision_locator_scale_fixture(
    key_count: usize,
) -> Result<DecisionLocatorScaleFixture, DecisionError> {
    let mut state = DecisionState::default();
    let archive = DecisionScaleArchive::default();
    let mut archive_batches = 0_u64;
    let mut next_ordinal = 1_usize;
    while next_ordinal <= key_count {
        let batch_end = next_ordinal
            .saturating_add(MAX_DECISION_ARCHIVE_BATCH_ENTRIES - 1)
            .min(key_count);
        let mut keys = Vec::with_capacity(batch_end - next_ordinal + 1);
        for ordinal in next_ordinal..=batch_end {
            let ordinal = u64::try_from(ordinal)
                .map_err(|_| archive_error("decision locator scale key exceeds u64"))?;
            let request_id = canwu_core::DecisionRequestId::new(ordinal);
            let request_commitment = blake3::hash(&ordinal.to_be_bytes()).to_hex().to_string();
            state.append_attempt(DecisionAttemptRecord {
                request_id,
                request_commitment,
                at: SimTime::from_minutes(
                    i64::try_from(ordinal)
                        .map_err(|_| archive_error("decision scale time exceeds i64"))?,
                ),
                revision_before: ordinal - 1,
                expected_revision: ordinal - 1,
                outcome: DecisionAttemptOutcome::Rejected {
                    code: crate::DecisionAttemptErrorCode::InvalidDecision,
                    message: "format8 production-path scale attempt".to_owned(),
                },
            })?;
            keys.push(DecisionHistoryKey::Attempt(request_id));
        }
        let prepared = state.prepare_decision_archive(&keys)?;
        for blob in &prepared.blobs {
            let _ = archive.store_decision_archive(blob)?;
        }
        let verified = state.verify_decision_archive(&prepared, &archive)?;
        state = state.commit_verified_decision_archive(&verified)?;
        let touched_pages = keys
            .iter()
            .map(decision_history_page_key)
            .collect::<Result<OrdSet<_>, _>>()?;
        for page_key in touched_pages {
            let page = state
                .decision_archive_bucket_page(page_key)?
                .ok_or_else(|| archive_error("committed decision locator page is missing"))?;
            let page_id = page.state_page_id()?;
            archive.pages.borrow_mut().insert(page_id, page);
        }
        archive_batches = archive_batches
            .checked_add(1)
            .ok_or_else(|| archive_error("decision archive batch count overflowed"))?;
        next_ordinal = batch_end.saturating_add(1);
    }

    let archive_root = state.archive_receipt_root()?;
    let hot_bytes = serde_json::to_vec(&state.paged_checkpoint_hot_state()).map_err(|error| {
        archive_error(format!(
            "cannot persist root-only decision hot state: {error}"
        ))
    })?;
    let hot = serde_json::from_slice::<DecisionState>(&hot_bytes).map_err(|error| {
        archive_error(format!(
            "cannot restart root-only decision hot state: {error}"
        ))
    })?;
    let restarted = DecisionState::from_paged_checkpoint_root(
        hot,
        state.archive_bucket_page_ids.clone(),
        state.archive_receipt_count,
        &archive_root,
    )?;
    let samples: OrdSet<usize> = [1_usize, key_count.saturating_div(2).max(1), key_count]
        .into_iter()
        .filter(|ordinal| *ordinal <= key_count)
        .collect::<OrdSet<_>>();
    for ordinal in &samples {
        let key = DecisionHistoryKey::Attempt(canwu_core::DecisionRequestId::new(
            u64::try_from(*ordinal)
                .map_err(|_| archive_error("decision scale sample exceeds u64"))?,
        ));
        if restarted.load_decision_history(&key, &archive)?.is_none() {
            return Err(archive_error(
                "root-only decision restart lost exact archived history",
            ));
        }
    }
    let reachable = restarted.archive_reachability(&archive)?;
    let mut max_page_entries = 0_u64;
    let mut max_page_encoded_bytes = 0_u64;
    for page_id in restarted.archive_bucket_page_ids.values() {
        let page = archive
            .load_decision_archive_bucket_page(page_id)?
            .ok_or_else(|| archive_error("decision scale locator page is unavailable"))?;
        max_page_entries = max_page_entries.max(page.receipts.len() as u64);
        max_page_encoded_bytes = max_page_encoded_bytes.max(
            serde_json::to_vec(&page)
                .map_err(|error| archive_error(format!("cannot size locator page: {error}")))?
                .len() as u64,
        );
    }
    let entry_bytes = size_of::<DecisionHistoryKey>()
        .saturating_add(size_of::<CompactDecisionArchiveReceipt>())
        .saturating_add(48);
    let metrics = DecisionLocatorScaleMetrics {
        entries: restarted.archived_history_count() as u64,
        locator_pages: restarted.archive_bucket_page_ids.len() as u64,
        max_page_entries,
        max_page_encoded_bytes,
        archive_batches,
        exact_restart_queries: samples.len() as u64,
        reachable_blob_locators: reachable.blob_locators.len() as u64,
        estimated_resident_structural_bytes: (restarted.archived_history_count() as u64)
            .saturating_mul(entry_bytes as u64),
        root_hash: archive_root,
    };
    Ok(DecisionLocatorScaleFixture {
        state,
        archive_blobs: archive.blobs.into_inner().into_values().collect(),
        metrics,
    })
}

/// Runs the production decision archive lifecycle and reports its bounded
/// locator metrics without exposing the scale fixture.
pub fn format8_decision_locator_scale_probe(
    key_count: usize,
) -> Result<DecisionLocatorScaleMetrics, DecisionError> {
    Ok(format8_decision_locator_scale_fixture(key_count)?.metrics)
}

/// Exercises the trace locator and archive-commit path with a large retained
/// hot trace log. The target is the final ordinal, which made the former
/// linear scan hit its worst case.
pub fn format8_trace_locator_scale_probe(
    trace_count: usize,
) -> Result<TraceLocatorScaleMetrics, DecisionError> {
    if trace_count == 0 {
        return Err(archive_error(
            "trace locator scale probe requires at least one trace",
        ));
    }
    let mut state = DecisionState::default();
    let controller_id = "format8-trace-controller".to_owned();
    let policy =
        DecisionPolicyIdentity::new(crate::DecisionPolicyKind::Rule, "format8-trace-policy", "1");
    state.controllers.insert(
        controller_id.clone(),
        Arc::new(DecisionControllerBinding::new(
            controller_id.clone(),
            policy.clone(),
            crate::DecisionAuthority::NoResponsibleActor {
                reason: "Format-8 trace scale fixture".to_owned(),
            },
        )),
    );
    let ticket_id = DecisionTicketId::new(1);
    let ticket = DecisionTicket {
        id: ticket_id,
        definition: "format8-trace-scale".to_owned(),
        decision_maker: canwu_core::EntityRef::Person(canwu_core::PersonId::new(1)),
        assigned_controller: controller_id.clone(),
        summary: "Format-8 trace scale ticket".to_owned(),
        context: crate::DecisionContext::new("format8-trace-scale", serde_json::json!({})),
        options: vec![crate::DecisionOption::new("defer", "Defer")],
        opened_at: SimTime::EPOCH,
        updated_at: SimTime::EPOCH,
        deadline: None,
        version: 1,
        state: DecisionTicketState::Cancelled {
            reason: "Scale fixture is terminal".to_owned(),
        },
    };
    ticket.validate()?;
    state.insert_hot_history_record(&DecisionArchiveRecord::Ticket {
        ticket: ticket.clone(),
    })?;
    state.tickets.insert(ticket_id, Arc::new(ticket));
    for ordinal in 1..=trace_count {
        let ordinal = u64::try_from(ordinal)
            .map_err(|_| archive_error("trace locator scale ordinal exceeds u64"))?;
        let trace = DecisionTrace {
            id: DecisionTraceId::new(ordinal),
            ticket_id,
            ticket_version: 1,
            controller_id: controller_id.clone(),
            policy: policy.clone(),
            decided_at: SimTime::from_minutes(
                i64::try_from(ordinal)
                    .map_err(|_| archive_error("trace locator scale time exceeds i64"))?,
            ),
            outcome: DecisionOutcome::Deferred {
                reason: "trace-scale".to_owned(),
            },
            summary: "trace-scale".to_owned(),
            evaluations: Vec::new(),
            external: None,
            random: None,
            command_request_id: None,
        };
        state.insert_hot_history_record(&DecisionArchiveRecord::Trace {
            trace: trace.clone(),
        })?;
        let inserted = state.traces.push(trace);
        if inserted != ordinal {
            return Err(archive_error(
                "trace locator scale ordinal insertion is inconsistent",
            ));
        }
    }
    state.validate()?;
    let samples: OrdSet<usize> = [1_usize, trace_count.saturating_div(2).max(1), trace_count]
        .into_iter()
        .filter(|ordinal| *ordinal <= trace_count)
        .collect::<OrdSet<_>>();
    for ordinal in &samples {
        let key = DecisionHistoryKey::Trace(DecisionTraceId::new(
            u64::try_from(*ordinal)
                .map_err(|_| archive_error("trace locator scale sample exceeds u64"))?,
        ));
        if state.decision_locator(&key) != DecisionHistoryLocation::Hot {
            return Err(archive_error(
                "trace locator ordinal index lost a retained hot trace",
            ));
        }
    }
    let target = DecisionHistoryKey::Trace(DecisionTraceId::new(
        u64::try_from(trace_count)
            .map_err(|_| archive_error("trace locator scale target exceeds u64"))?,
    ));
    let archive = DecisionScaleArchive::default();
    let prepared = state.prepare_decision_archive(std::slice::from_ref(&target))?;
    for blob in &prepared.blobs {
        let _ = archive.store_decision_archive(blob)?;
    }
    let verified = state.verify_decision_archive(&prepared, &archive)?;
    let committed = state.commit_verified_decision_archive(&verified)?;
    let target_archived = matches!(
        committed.decision_locator(&target),
        DecisionHistoryLocation::Archived { .. }
    );
    if !target_archived {
        return Err(archive_error(
            "trace locator scale archive commit did not release its target",
        ));
    }
    Ok(TraceLocatorScaleMetrics {
        hot_trace_entries: trace_count as u64,
        indexed_lookup_samples: samples.len() as u64,
        archive_commit_entries: verified.receipts.len() as u64,
        target_archived,
    })
}

#[cfg(test)]
mod archive_restart_tests {
    use super::*;

    fn linked_decision_state(resolved_ticket: bool, accepted_attempt: bool) -> DecisionState {
        let mut state = DecisionState::default();
        let controller_id = "restart-controller".to_owned();
        let policy =
            DecisionPolicyIdentity::new(crate::DecisionPolicyKind::Rule, "restart-policy", "1");
        state.controllers.insert(
            controller_id.clone(),
            Arc::new(DecisionControllerBinding::new(
                controller_id.clone(),
                policy.clone(),
                crate::DecisionAuthority::NoResponsibleActor {
                    reason: "restart dependency fixture".to_owned(),
                },
            )),
        );
        let ticket_id = DecisionTicketId::new(91);
        let trace_id = DecisionTraceId::new(1);
        let ticket = DecisionTicket {
            id: ticket_id,
            definition: "restart-dependency".to_owned(),
            decision_maker: canwu_core::EntityRef::Person(canwu_core::PersonId::new(9)),
            assigned_controller: controller_id.clone(),
            summary: "Restart dependency fixture".to_owned(),
            context: crate::DecisionContext::new("restart-dependency", serde_json::json!({})),
            options: vec![crate::DecisionOption::new("accept", "Accept")],
            opened_at: SimTime::EPOCH,
            updated_at: SimTime::from_minutes(1),
            deadline: None,
            version: 2,
            state: if resolved_ticket {
                DecisionTicketState::Resolved {
                    option_id: "accept".to_owned(),
                    trace_id,
                }
            } else {
                DecisionTicketState::Cancelled {
                    reason: "terminal fixture".to_owned(),
                }
            },
        };
        state
            .insert_hot_history_record(&DecisionArchiveRecord::Ticket {
                ticket: ticket.clone(),
            })
            .expect("insert fixture ticket history");
        state.tickets.insert(ticket_id, Arc::new(ticket));
        let trace = DecisionTrace {
            id: trace_id,
            ticket_id,
            ticket_version: 1,
            controller_id,
            policy,
            decided_at: SimTime::from_minutes(1),
            outcome: if resolved_ticket {
                DecisionOutcome::Selected {
                    option_id: "accept".to_owned(),
                }
            } else {
                DecisionOutcome::Deferred {
                    reason: "terminal fixture".to_owned(),
                }
            },
            summary: "Restart dependency trace".to_owned(),
            evaluations: Vec::new(),
            external: None,
            random: None,
            command_request_id: None,
        };
        state
            .insert_hot_history_record(&DecisionArchiveRecord::Trace {
                trace: trace.clone(),
            })
            .expect("insert fixture trace history");
        assert_eq!(state.traces.push(trace), trace_id.get());
        if accepted_attempt {
            state
                .append_attempt(DecisionAttemptRecord {
                    request_id: canwu_core::DecisionRequestId::new(92),
                    request_commitment: "a".repeat(64),
                    at: SimTime::from_minutes(1),
                    revision_before: 1,
                    expected_revision: 1,
                    outcome: DecisionAttemptOutcome::Accepted {
                        trace_id: Some(trace_id),
                        command_request_id: None,
                    },
                })
                .expect("append fixture attempt");
        }
        state.validate().expect("fixture state validates");
        state
    }

    fn restart_with_exact_dependency_pages(state: &DecisionState) -> DecisionState {
        let hot_bytes = serde_json::to_vec(&state.paged_checkpoint_hot_state())
            .expect("encode paged hot state");
        let hot: DecisionState =
            serde_json::from_slice(&hot_bytes).expect("decode paged hot state");
        let required_pages = hot
            .required_archived_dependency_page_keys()
            .expect("derive exact dependency pages");
        assert!(!required_pages.is_empty());
        let resident_pages = required_pages
            .iter()
            .map(|page_key| {
                state
                    .decision_archive_bucket_page(*page_key)
                    .expect("encode dependency page")
                    .expect("dependency page is resident")
            })
            .collect::<Vec<_>>();
        let restarted = DecisionState::from_paged_checkpoint_root_with_resident_pages(
            hot,
            state.archive_bucket_page_ids.clone(),
            state.archive_receipt_count,
            &state.archive_receipt_root().expect("archive root"),
            resident_pages,
        )
        .expect("restart with exact dependency pages");
        let encoded = serde_json::to_vec(&restarted).expect("encode sparse restarted state");
        serde_json::from_slice(&encoded).expect("sparse restarted state remains restartable")
    }

    #[test]
    fn paged_checkpoint_root_rejects_hot_archive_metadata() {
        let hot = DecisionState {
            archive_receipt_count: 1,
            ..DecisionState::default()
        };
        let error = DecisionState::from_paged_checkpoint_root(
            hot,
            OrdMap::new(),
            0,
            &decision_archive_hash(
                "canwu.decision.archive-receipts.v3",
                &(0_u64, OrdMap::<DecisionArchivePageKey, String>::new()),
            )
            .expect("empty archive root"),
        )
        .expect_err("hot archive metadata must be rejected");
        assert_eq!(error.code, DecisionErrorCode::InvalidDecision);
    }

    fn append_terminal_attempt(state: &mut DecisionState, ordinal: u64) {
        state
            .append_attempt(DecisionAttemptRecord {
                request_id: canwu_core::DecisionRequestId::new(ordinal),
                request_commitment: blake3::hash(&ordinal.to_be_bytes()).to_hex().to_string(),
                at: SimTime::from_minutes(i64::try_from(ordinal).expect("test time fits i64")),
                revision_before: ordinal.saturating_sub(1),
                expected_revision: ordinal.saturating_sub(1),
                outcome: DecisionAttemptOutcome::Rejected {
                    code: crate::DecisionAttemptErrorCode::InvalidDecision,
                    message: "terminal test attempt".to_owned(),
                },
            })
            .expect("append terminal attempt");
    }

    fn archive_keys(
        state: &DecisionState,
        archive: &DecisionScaleArchive,
        keys: &[DecisionHistoryKey],
    ) -> (DecisionState, VerifiedDecisionArchiveCommit) {
        let prepared = state
            .prepare_decision_archive(keys)
            .expect("prepare archive");
        for blob in &prepared.blobs {
            archive.store_decision_archive(blob).expect("store blob");
        }
        let verified = state
            .verify_decision_archive(&prepared, archive)
            .expect("verify archive");
        let next = state
            .commit_verified_decision_archive(&verified)
            .expect("commit archive");
        for page_key in keys
            .iter()
            .map(decision_history_page_key)
            .collect::<Result<OrdSet<_>, _>>()
            .expect("derive touched pages")
        {
            let page = next
                .decision_archive_bucket_page(page_key)
                .expect("build page")
                .expect("page is resident");
            archive
                .pages
                .borrow_mut()
                .insert(page.state_page_id().expect("page id"), page);
        }
        (next, verified)
    }

    #[test]
    fn root_only_restart_can_archive_again_into_the_same_segment() {
        let first = 1_u64;
        let first_key = DecisionHistoryKey::Attempt(canwu_core::DecisionRequestId::new(first));
        let page_key = decision_history_page_key(&first_key).expect("first page key");
        let second = (2_u64..1_000_000)
            .find(|ordinal| {
                decision_history_page_key(&DecisionHistoryKey::Attempt(
                    canwu_core::DecisionRequestId::new(*ordinal),
                ))
                .ok()
                    == Some(page_key)
            })
            .expect("find same-segment request id");
        let second_key = DecisionHistoryKey::Attempt(canwu_core::DecisionRequestId::new(second));
        let archive = DecisionScaleArchive::default();
        let mut state = DecisionState::default();
        append_terminal_attempt(&mut state, first);
        let (state, _) = archive_keys(&state, &archive, std::slice::from_ref(&first_key));
        let archive_root = state.archive_receipt_root().expect("archive root");
        let hot = state.paged_checkpoint_hot_state();
        assert!(hot.archive_receipt_buckets.is_empty());
        assert!(hot.archive_bucket_page_ids.is_empty());
        assert_eq!(hot.archive_receipt_count, 0);
        let mut restarted = DecisionState::from_paged_checkpoint_root(
            hot,
            state.archive_bucket_page_ids.clone(),
            state.archive_receipt_count,
            &archive_root,
        )
        .expect("root-only restart");
        append_terminal_attempt(&mut restarted, second);
        let (restarted, verified) =
            archive_keys(&restarted, &archive, std::slice::from_ref(&second_key));

        assert_eq!(restarted.archived_history_count(), 2);
        assert!(
            restarted
                .load_decision_history(&first_key, &archive)
                .expect("load first")
                .is_some()
        );
        assert!(
            restarted
                .load_decision_history(&second_key, &archive)
                .expect("load second")
                .is_some()
        );
        assert_eq!(
            restarted
                .archive_reachability(&archive)
                .expect("enumerate reachability")
                .blob_locators
                .len(),
            2
        );
        assert_eq!(
            restarted
                .commit_verified_decision_archive(&verified)
                .expect("replay commit"),
            restarted
        );
    }

    #[test]
    fn paged_restart_loads_archived_ticket_referenced_by_hot_trace() {
        let archive = DecisionScaleArchive::default();
        let state = linked_decision_state(true, false);
        let ticket_id = DecisionTicketId::new(91);
        let (archived, _) =
            archive_keys(&state, &archive, &[DecisionHistoryKey::Ticket(ticket_id)]);
        let restarted = restart_with_exact_dependency_pages(&archived);

        assert!(restarted.trace(DecisionTraceId::new(1)).is_some());
        assert!(matches!(
            restarted.decision_locator(&DecisionHistoryKey::Ticket(ticket_id)),
            DecisionHistoryLocation::Archived { .. }
        ));
        restarted
            .validate()
            .expect("hot trace dependency validates");
    }

    #[test]
    fn paged_restart_loads_archived_trace_referenced_by_resolved_hot_ticket() {
        let archive = DecisionScaleArchive::default();
        let state = linked_decision_state(true, false);
        let trace_id = DecisionTraceId::new(1);
        let (archived, _) = archive_keys(&state, &archive, &[DecisionHistoryKey::Trace(trace_id)]);
        let restarted = restart_with_exact_dependency_pages(&archived);

        assert!(restarted.ticket(DecisionTicketId::new(91)).is_some());
        assert!(matches!(
            restarted.decision_locator(&DecisionHistoryKey::Trace(trace_id)),
            DecisionHistoryLocation::Archived { .. }
        ));
        restarted
            .validate()
            .expect("resolved hot ticket dependency validates");
    }

    #[test]
    fn paged_restart_loads_archived_trace_referenced_by_accepted_hot_attempt() {
        let archive = DecisionScaleArchive::default();
        let state = linked_decision_state(false, true);
        let trace_id = DecisionTraceId::new(1);
        let (archived, _) = archive_keys(&state, &archive, &[DecisionHistoryKey::Trace(trace_id)]);
        let restarted = restart_with_exact_dependency_pages(&archived);

        assert!(
            restarted
                .attempt(canwu_core::DecisionRequestId::new(92))
                .is_some()
        );
        assert!(matches!(
            restarted.decision_locator(&DecisionHistoryKey::Trace(trace_id)),
            DecisionHistoryLocation::Archived { .. }
        ));
        restarted
            .validate()
            .expect("accepted hot attempt dependency validates");
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PreparedDecision {
    pub trace: Option<DecisionTrace>,
    pub action: Option<DecisionAction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControllerDecision {
    Authoritative {
        decision: PolicyDecision,
        action: Option<DecisionAction>,
    },
    Pending(PolicyDecision),
}

pub struct DecisionController;

impl DecisionController {
    pub fn evaluate(
        ticket: &DecisionTicket,
        controller: &DecisionControllerBinding,
        policy: &dyn DecisionPolicy,
    ) -> Result<ControllerDecision, DecisionError> {
        if !ticket.is_open() {
            return Err(DecisionError::new(
                DecisionErrorCode::ClosedTicket,
                "only open tickets can be evaluated",
            ));
        }
        if ticket.assigned_controller != controller.id || policy.identity() != &controller.policy {
            return Err(DecisionError::new(
                DecisionErrorCode::PolicyMismatch,
                "runtime policy identity does not match the ticket controller binding",
            ));
        }
        let decision = policy.decide(ticket)?;
        decision.validate(ticket)?;
        if matches!(decision.outcome, DecisionOutcome::Pending { .. }) {
            return Ok(ControllerDecision::Pending(decision));
        }
        let action = match &decision.outcome {
            DecisionOutcome::Selected { option_id } => {
                ticket.option(option_id).map(|option| option.action.clone())
            }
            DecisionOutcome::Deferred { .. } | DecisionOutcome::Pending { .. } => None,
        };
        Ok(ControllerDecision::Authoritative { action, decision })
    }
}
