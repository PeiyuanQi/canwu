use crate::{
    ConsumptionStatus, FulfillmentStatus, ResourceError, ResourceOperationKey,
    ResourceOperationStatus, ResourceState, ResourceTransferState, canonical_digest,
};
use canwu_api::DomainRecordVersionRef;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const RESOURCE_ARCHIVE_BLOB_NAMESPACE: &str = "canwu.resource.archive.blob";
pub const RESOURCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE: &str =
    "canwu.resource.archive.membership-page";
pub const RESOURCE_ARCHIVE_TEMPORAL_PAGE_NAMESPACE: &str = "canwu.resource.archive.temporal-page";
pub const RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE: &str =
    "canwu.resource.archive.index-directory";
/// Namespace of the pages of the cumulative lookup index
/// ([`ResourceArchiveLookupPageV1`]).
pub const RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE: &str = "canwu.resource.archive.lookup-page";
/// Terminal records one blob, membership page, or temporal page may hold.
pub const MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES: usize = 256;
/// Blob, membership, and temporal pages one archive commit may carry.
pub const MAX_RESOURCE_ARCHIVE_COMMIT_PAGES: usize = 64;
/// Terminal records one archive commit may carry: every page full. It equals
/// [`crate::ResourceLimitsV1::MAX_ARCHIVE_CANDIDATES`], so one commit can
/// always settle every hot terminal candidate of the resource root.
pub const MAX_RESOURCE_ARCHIVE_COMMIT_RECORDS: usize =
    MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES * MAX_RESOURCE_ARCHIVE_COMMIT_PAGES;
/// Entries a leaf page of the lookup index may hold before it splits into
/// one child per next key-digest nibble. Every commit rewrites the leaf of
/// each key it adds, so a small leaf keeps the bytes a commit writes low;
/// below eight the saving is a few percent and the index gains a level
/// sooner.
pub const MAX_RESOURCE_ARCHIVE_LOOKUP_LEAF_ENTRIES: usize = 8;
/// Longest key-digest prefix a lookup page may sit at: one hexadecimal nibble
/// per level of a 256-bit digest. A cold lookup therefore reads the head
/// directory, at most `MAX_RESOURCE_ARCHIVE_LOOKUP_DEPTH + 1` lookup pages,
/// and one blob on a hit, whatever the archive holds.
pub const MAX_RESOURCE_ARCHIVE_LOOKUP_DEPTH: usize = 64;
/// Encoded size above which a lookup page is refused before it is decoded.
/// The largest well-formed page, a full leaf or a page with sixteen children
/// at a deep prefix, encodes to about 1.3 KB; the bound leaves room for a
/// store that returns the same JSON with other whitespace.
pub const MAX_RESOURCE_ARCHIVE_LOOKUP_PAGE_BYTES: usize = 4 * 1024;

const RESOURCE_ARCHIVE_LOOKUP_KEY_DOMAIN: &str = "canwu.resource.archive-lookup-key.v1";
const RESOURCE_ARCHIVE_LOOKUP_PAGE_DOMAIN: &str = "canwu.resource.archive-lookup-page.v1";

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum ResourceTerminalRecordKeyV1 {
    Outcome(ResourceOperationKey),
    Demand(crate::ResourceDemandId),
    Reservation(crate::ResourceReservationId),
    AllocationLeg(crate::ResourceAllocationLegId),
    Consumption(crate::ResourceConsumptionId),
    Fulfillment(crate::ResourceFulfillmentId),
    Loss(crate::ResourceLossId),
    Transfer(crate::ResourceTransferId),
    ExternalCompletionParticipant(crate::CompletionLeaseAcquisitionId),
    LeaseReceipt(u64),
    ArchiveMaintenanceReceipt(u64),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "payload", content = "value", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ResourceTerminalArchivePayloadV1 {
    Outcome(crate::ResourceOperationOutcome),
    Demand(crate::ResourceDemand),
    Reservation(crate::ResourceReservation),
    AllocationLeg(crate::ResourceAllocationLeg),
    Consumption(crate::ResourceConsumption),
    Fulfillment(crate::ResourceFulfillment),
    Loss(crate::ResourceLoss),
    Transfer(crate::ResourceTransfer),
    ExternalCompletionParticipant(crate::ExternalCompletionParticipantGrantV1),
    LeaseReceipt(crate::CompletionLeaseReceiptV1),
    ArchiveMaintenanceReceipt(ResourceArchiveMaintenanceReceiptV1),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceTerminalArchiveRecordV1 {
    pub key: ResourceTerminalRecordKeyV1,
    pub operation_key: ResourceOperationKey,
    pub quantity: u64,
    pub remainder: u64,
    pub exact_evidence: Vec<DomainRecordVersionRef>,
    pub semantic_digest: String,
    pub terminal_sequence: u64,
    pub payload: ResourceTerminalArchivePayloadV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceArchiveBlobV1 {
    pub format_version: u32,
    pub expected_source_root: String,
    pub records: Vec<ResourceTerminalArchiveRecordV1>,
    pub content_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceArchiveMembershipV1 {
    pub key: ResourceTerminalRecordKeyV1,
    pub blob_id: String,
    pub ordinal: u16,
    pub terminal_sequence: u64,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceArchiveMembershipPageV1 {
    pub id: String,
    pub memberships: Vec<ResourceArchiveMembershipV1>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceArchiveTemporalEntryV1 {
    pub terminal_sequence: u64,
    pub key: ResourceTerminalRecordKeyV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceArchiveTemporalPageV1 {
    pub id: String,
    pub entries: Vec<ResourceArchiveTemporalEntryV1>,
    pub semantic_digest: String,
}

/// One archive commit: the blobs and per-commit pages it added, the commit
/// before it, and the root of the cumulative lookup index as of this commit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceArchiveIndexDirectoryV1 {
    pub id: String,
    pub previous_root: Option<String>,
    /// One membership page per blob, in blob order.
    pub membership_pages: Vec<String>,
    /// One temporal page per blob, in blob order.
    pub temporal_pages: Vec<String>,
    /// At most [`MAX_RESOURCE_ARCHIVE_COMMIT_PAGES`] blobs of at most
    /// [`MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES`] records each.
    pub blob_ids: Vec<String>,
    pub archived_record_count: u64,
    /// Root [`ResourceArchiveLookupPageV1`] of the index over every terminal
    /// record archived up to and including this commit. A cold lookup starts
    /// here, so its cost does not depend on how many commits came before.
    pub lookup_root: String,
    pub semantic_digest: String,
}

/// Where one archived terminal record lives: the entry a cold lookup ends at.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ResourceArchiveLookupEntryV1 {
    /// [`resource_archive_lookup_key_digest`] of the terminal record key.
    #[serde(rename = "k")]
    pub key_digest: String,
    /// Blob that holds the newest archived record with this key.
    #[serde(rename = "b")]
    pub blob_id: String,
    /// Position of the record in that blob.
    #[serde(rename = "o")]
    pub ordinal: u16,
}

/// One content-addressed page of the cumulative lookup index: a 16-way trie
/// over the hexadecimal nibbles of each key digest.
///
/// A page at `prefix` covers every archived key whose digest starts with
/// `prefix`. It is a leaf while it covers at most
/// [`MAX_RESOURCE_ARCHIVE_LOOKUP_LEAF_ENTRIES`] keys and otherwise names one
/// child per next nibble that has a key, so the index is a function of its
/// set of entries: the same entries (key digest, blob id, ordinal) give the
/// same root in whatever order and grouping they were added. A blob id
/// commits to the root it was prepared from, so two runs that batch the same
/// records differently hold different entries and different roots; what the
/// restore audit relies on is that the newest entry of every key in a
/// directory chain determines the index. Its object id is the canonical
/// digest of the page ([`Self::id`]); a commit writes new pages only along
/// the paths of the keys it adds.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceArchiveLookupPageV1 {
    /// Lowercase hexadecimal key-digest prefix this page covers.
    pub prefix: String,
    /// Leaf entries, ordered by key digest. Empty on an interior page.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<ResourceArchiveLookupEntryV1>,
    /// Child page id by next nibble. Empty on a leaf page.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub children: BTreeMap<String, String>,
}

impl ResourceArchiveLookupPageV1 {
    /// Content id under which the page is stored in
    /// [`RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE`].
    pub fn id(&self) -> Result<String, ResourceError> {
        canonical_digest(RESOURCE_ARCHIVE_LOOKUP_PAGE_DOMAIN, self)
    }

    /// Checks the page shape against the prefix it was reached by: bounded,
    /// ordered, and holding only keys and children under that prefix.
    fn validate_at(&self, prefix: &str) -> Result<(), ResourceError> {
        let leaf = self.children.is_empty();
        let depth = prefix.len();
        if self.prefix != prefix
            || depth > MAX_RESOURCE_ARCHIVE_LOOKUP_DEPTH
            || (leaf == self.entries.is_empty())
            || self.entries.len() > MAX_RESOURCE_ARCHIVE_LOOKUP_LEAF_ENTRIES
            || (!leaf && depth == MAX_RESOURCE_ARCHIVE_LOOKUP_DEPTH)
            || self.entries.iter().any(|entry| {
                !is_lookup_digest(&entry.key_digest)
                    || !entry.key_digest.starts_with(prefix)
                    || !is_lookup_digest(&entry.blob_id)
            })
            || !self
                .entries
                .windows(2)
                .all(|pair| pair[0].key_digest < pair[1].key_digest)
            || self.children.iter().any(|(nibble, child)| {
                nibble.len() != 1 || !is_lookup_digest_prefix(nibble) || !is_lookup_digest(child)
            })
        {
            return Err(ResourceError::InvalidDefinition(
                "resource archive lookup page is malformed, unordered, or outside its prefix"
                    .to_owned(),
            ));
        }
        Ok(())
    }
}

/// The pages a commit adds to the cumulative lookup index, and its new root.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreparedResourceArchiveLookupV1 {
    /// Root page id after the extension.
    pub root: String,
    /// New pages by id. Pages of the previous index that the extension did
    /// not touch are shared, not repeated.
    pub pages: BTreeMap<String, ResourceArchiveLookupPageV1>,
    /// Pages of the previous index that `root` no longer reaches.
    pub superseded: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceArchiveHeadStateV1 {
    pub revision: u64,
    pub directory_root: Option<String>,
    pub archived_record_count: u64,
    pub semantic_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceArchiveRetentionPhaseV1 {
    Prepared,
    Verified,
    DurableIngress,
    Committed,
    RejectedStale,
    Abandoned,
}

/// The store-side protection of one prepared archive batch, from preparation
/// until its commit settles and the host application finalizes it.
///
/// Releasing a handle removes its protection and never deletes an object.
/// Objects are shared between handles: two batches prepared from one root
/// share their leading blobs, pages, and lookup pages, so an object named by
/// a rejected or abandoned handle may belong to an applied commit. Deletion
/// is by reachability only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceArchiveRetentionHandleV1 {
    pub id: String,
    pub phase: ResourceArchiveRetentionPhaseV1,
    pub expected_source_root: String,
    pub directory_root: String,
    /// The blobs, membership pages, temporal pages, and directory of the
    /// batch, by namespace. Lookup pages are reached through the directory.
    pub object_ids: BTreeMap<String, BTreeSet<String>>,
    pub semantic_digest: String,
}

/// How one archive commit settled against the resource root.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceArchiveMaintenanceDispositionV1 {
    /// The commit was prepared from the root it met: its records left the
    /// root and the archive head moved to its directory.
    Applied,
    /// Another commit or an operation settled first, so the commit changed
    /// nothing. It is a terminal no-op and needs no candidate or receipt
    /// budget.
    RejectedStale,
}

/// The terminal record of one settled archive commit.
///
/// The receipt of an applied commit is a terminal archive candidate and is
/// archived by a later commit. The receipt of a stale commit is not a
/// candidate: it stays in the root until the host application acknowledges
/// the commit's retention and is then removed, so it is never archived.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceArchiveMaintenanceReceiptV1 {
    pub sequence: u64,
    pub retention_handle_id: String,
    pub expected_source_root: String,
    pub directory_root: String,
    pub disposition: ResourceArchiveMaintenanceDispositionV1,
    pub archived_records: u32,
    pub semantic_digest: String,
}

/// One blob of a prepared archive batch with its membership and temporal
/// pages.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreparedResourceArchivePageV1 {
    pub blob: ResourceArchiveBlobV1,
    pub membership_page: ResourceArchiveMembershipPageV1,
    pub temporal_page: ResourceArchiveTemporalPageV1,
}

/// One archive commit prepared from the hot terminal candidates of a resource
/// root, ready for [`Self::store_and_verify`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreparedResourceArchiveBatchV1 {
    pub expected_source_root: String,
    /// Every selected candidate, in terminal sequence order.
    pub selected: Vec<ResourceTerminalRecordKeyV1>,
    /// First blob: the first [`MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES`] selected
    /// records.
    pub blob: ResourceArchiveBlobV1,
    pub membership_page: ResourceArchiveMembershipPageV1,
    pub temporal_page: ResourceArchiveTemporalPageV1,
    /// The following blobs and their pages, in order, when the batch holds
    /// more records than one page. Empty for a single-page batch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_pages: Vec<PreparedResourceArchivePageV1>,
    /// New pages of the cumulative lookup index, by page id.
    pub lookup_pages: BTreeMap<String, ResourceArchiveLookupPageV1>,
    /// Lookup pages of the previous head's index that this batch's index no
    /// longer reaches.
    ///
    /// The list may be used only after the batch's commit settled as
    /// `Applied`. A batch whose commit was rejected as stale carries a list
    /// too, and that list names pages of the live head. Deleting the pages of
    /// an applied commit is safe only after a durable snapshot or checkpoint
    /// taken after that commit exists, and it gives up every earlier point of
    /// the run that reads one of those pages: a snapshot or checkpoint taken
    /// before the commit no longer restores, a journal no longer replays
    /// across it, and a fork whose head precedes it no longer continues. This
    /// reaches back as far as the commit that wrote each page, not only to
    /// the commit before, because consecutive heads share pages. A host
    /// application that needs any of those keeps the pages.
    /// [`live_resource_archive_lookup_pages`] recomputes what a live run
    /// reads when a list was lost.
    pub superseded_lookup_pages: BTreeSet<String>,
    pub directory: ResourceArchiveIndexDirectoryV1,
    pub retention: ResourceArchiveRetentionHandleV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VerifiedResourceArchiveCommitV1 {
    pub expected_source_root: String,
    pub selected: Vec<ResourceTerminalRecordKeyV1>,
    pub directory_root: String,
    pub retention: ResourceArchiveRetentionHandleV1,
    pub archived_records: u32,
}

impl VerifiedResourceArchiveCommitV1 {
    /// Checks that the commit is self-consistent: between one and
    /// [`MAX_RESOURCE_ARCHIVE_COMMIT_RECORDS`] distinct selected keys, and a
    /// verified retention handle bound to the same source root and directory.
    pub fn validate(&self) -> Result<(), ResourceError> {
        let selected_count =
            u32::try_from(self.selected.len()).map_err(|_| ResourceError::Overflow)?;
        let mut unique = BTreeSet::new();
        let mut detached = self.retention.clone();
        let retention_digest = std::mem::take(&mut detached.semantic_digest);
        if self.selected.is_empty()
            || self.selected.len() > MAX_RESOURCE_ARCHIVE_COMMIT_RECORDS
            || self.selected.iter().any(|key| !unique.insert(key))
            || self.archived_records != selected_count
            || self.expected_source_root.len() != 64
            || self.directory_root.len() != 64
            || self.retention.phase != ResourceArchiveRetentionPhaseV1::Verified
            || self.retention.expected_source_root != self.expected_source_root
            || self.retention.directory_root != self.directory_root
            || self.retention.id
                != canonical_digest(
                    "canwu.resource.archive-retention-id.v1",
                    &(&self.expected_source_root, &self.directory_root),
                )?
            || retention_digest
                != canonical_digest("canwu.resource.archive-retention.v1", &detached)?
            || self
                .retention
                .object_ids
                .get(RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE)
                != Some(&BTreeSet::from([self.directory_root.clone()]))
        {
            return Err(ResourceError::InvalidDefinition(
                "resource verified archive commit is forged or non-canonical".to_owned(),
            ));
        }
        Ok(())
    }
}

/// The host application's store of resource archive objects and retention
/// handles.
///
/// A store a run holds, through
/// [`from_resource_snapshot_json_with_archive_store`](crate::from_resource_snapshot_json_with_archive_store)
/// and the other `_with_archive_store` restores, is passed as
/// `Arc<dyn ResourceArchiveStore + Send + Sync>`, because forks share it and
/// may run on other threads. A store lent for one call as
/// `&dyn ResourceArchiveStore` needs neither bound.
pub trait ResourceArchiveStore {
    fn store_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), ResourceError>;

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, ResourceError>;

    fn persist_resource_archive_retention(
        &self,
        handle: &ResourceArchiveRetentionHandleV1,
    ) -> Result<(), ResourceError>;

    fn finalize_resource_archive_retention(
        &self,
        handle_id: &str,
        phase: ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError>;
}

/// The absent store: it holds no object and accepts none. It is enough to
/// prepare the first archive commit of a resource root, which reads nothing,
/// and it fails every read of a root that already has an archive head.
impl ResourceArchiveStore for () {
    fn store_resource_archive_object(
        &self,
        _namespace: &str,
        _object_id: &str,
        _bytes: &[u8],
    ) -> Result<(), ResourceError> {
        Err(ResourceError::InvalidLifecycle(
            "no resource archive store is attached".to_owned(),
        ))
    }

    fn load_resource_archive_object(
        &self,
        _namespace: &str,
        _object_id: &str,
    ) -> Result<Option<Vec<u8>>, ResourceError> {
        Ok(None)
    }

    fn persist_resource_archive_retention(
        &self,
        _handle: &ResourceArchiveRetentionHandleV1,
    ) -> Result<(), ResourceError> {
        Err(ResourceError::InvalidLifecycle(
            "no resource archive store is attached".to_owned(),
        ))
    }

    fn finalize_resource_archive_retention(
        &self,
        _handle_id: &str,
        _phase: ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError> {
        Err(ResourceError::InvalidLifecycle(
            "no resource archive store is attached".to_owned(),
        ))
    }
}

impl PreparedResourceArchiveBatchV1 {
    /// Every blob of the batch with its membership and temporal page, in
    /// terminal sequence order.
    fn pages(
        &self,
    ) -> impl Iterator<
        Item = (
            &ResourceArchiveBlobV1,
            &ResourceArchiveMembershipPageV1,
            &ResourceArchiveTemporalPageV1,
        ),
    > {
        std::iter::once((&self.blob, &self.membership_page, &self.temporal_page)).chain(
            self.additional_pages
                .iter()
                .map(|page| (&page.blob, &page.membership_page, &page.temporal_page)),
        )
    }

    /// Stores every object of the batch, reads the directory back, and
    /// authenticates its closure: the blobs and pages bind exactly the
    /// selected records, and each of them resolves through the stored lookup
    /// index to its own blob position.
    pub fn store_and_verify(
        &self,
        store: &dyn ResourceArchiveStore,
    ) -> Result<VerifiedResourceArchiveCommitV1, ResourceError> {
        self.validate()?;
        store.persist_resource_archive_retention(&self.retention)?;
        for (blob, membership_page, temporal_page) in self.pages() {
            store_encoded(
                store,
                RESOURCE_ARCHIVE_BLOB_NAMESPACE,
                &blob.content_id,
                blob,
            )?;
            store_encoded(
                store,
                RESOURCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE,
                &membership_page.id,
                membership_page,
            )?;
            store_encoded(
                store,
                RESOURCE_ARCHIVE_TEMPORAL_PAGE_NAMESPACE,
                &temporal_page.id,
                temporal_page,
            )?;
        }
        for (id, page) in &self.lookup_pages {
            store_encoded(store, RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE, id, page)?;
        }
        store_encoded(
            store,
            RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            &self.directory.id,
            &self.directory,
        )?;
        let loaded: ResourceArchiveIndexDirectoryV1 = load_encoded(
            store,
            RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            &self.directory.id,
        )?
        .ok_or_else(|| {
            ResourceError::NotFound("resource archive directory readback is unavailable".to_owned())
        })?;
        let archived = authenticate_resource_archive_directory_closure(store, &loaded)?;
        if loaded != self.directory {
            return Err(ResourceError::InvalidDefinition(
                "resource archive directory readback differs".to_owned(),
            ));
        }
        let mut expected = archived
            .into_iter()
            .map(|(key, blob_id, ordinal)| {
                Ok(ResourceArchiveLookupEntryV1 {
                    key_digest: resource_archive_lookup_key_digest(&key)?,
                    blob_id,
                    ordinal,
                })
            })
            .collect::<Result<Vec<_>, ResourceError>>()?;
        expected.sort();
        verify_resource_archive_lookup_entries(store, &loaded.lookup_root, "", &expected)?;
        let mut retention = self.retention.clone();
        retention.phase = ResourceArchiveRetentionPhaseV1::Verified;
        retention.semantic_digest.clear();
        retention.semantic_digest =
            canonical_digest("canwu.resource.archive-retention.v1", &retention)?;
        store.persist_resource_archive_retention(&retention)?;
        Ok(VerifiedResourceArchiveCommitV1 {
            expected_source_root: self.expected_source_root.clone(),
            selected: self.selected.clone(),
            directory_root: self.directory.id.clone(),
            retention,
            archived_records: u32::try_from(self.selected.len())
                .map_err(|_| ResourceError::Overflow)?,
        })
    }

    /// Checks that the batch is self-consistent before anything is stored:
    /// typed record closure, page shape, the directory's object lists, the
    /// lookup page ids, and the retention handle.
    pub fn validate(&self) -> Result<(), ResourceError> {
        let mut selected_keys = Vec::with_capacity(self.selected.len());
        let mut blob_ids = Vec::new();
        let mut membership_page_ids = Vec::new();
        let mut temporal_page_ids = Vec::new();
        let mut pages_bind_their_blobs = true;
        for (blob, membership_page, temporal_page) in self.pages() {
            for record in &blob.records {
                validate_resource_terminal_archive_record(record)?;
                selected_keys.push(record.key.clone());
            }
            validate_archived_consumption_links(&blob.records)?;
            pages_bind_their_blobs &= !blob.records.is_empty()
                && blob.records.len() <= MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES
                && blob.expected_source_root == self.expected_source_root
                && membership_page.memberships.len() == blob.records.len()
                && temporal_page.entries.len() == blob.records.len();
            blob_ids.push(blob.content_id.clone());
            membership_page_ids.push(membership_page.id.clone());
            temporal_page_ids.push(temporal_page.id.clone());
        }
        let expected_object_ids = BTreeMap::from([
            (
                RESOURCE_ARCHIVE_BLOB_NAMESPACE.to_owned(),
                blob_ids.iter().cloned().collect::<BTreeSet<_>>(),
            ),
            (
                RESOURCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE.to_owned(),
                membership_page_ids.iter().cloned().collect(),
            ),
            (
                RESOURCE_ARCHIVE_TEMPORAL_PAGE_NAMESPACE.to_owned(),
                temporal_page_ids.iter().cloned().collect(),
            ),
            (
                RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
                BTreeSet::from([self.directory.id.clone()]),
            ),
        ]);
        if self.selected.is_empty()
            || self.selected.len() > MAX_RESOURCE_ARCHIVE_COMMIT_RECORDS
            || blob_ids.len() > MAX_RESOURCE_ARCHIVE_COMMIT_PAGES
            || self.selected != selected_keys
            || !pages_bind_their_blobs
            || self.directory.previous_root == Some(self.directory.id.clone())
            || self.directory.membership_pages != membership_page_ids
            || self.directory.temporal_pages != temporal_page_ids
            || self.directory.blob_ids != blob_ids
            || !self.lookup_pages.contains_key(&self.directory.lookup_root)
            || self.retention.phase != ResourceArchiveRetentionPhaseV1::Prepared
            || self.retention.expected_source_root != self.expected_source_root
            || self.retention.directory_root != self.directory.id
            || self.retention.object_ids != expected_object_ids
        {
            return Err(ResourceError::InvalidDefinition(
                "prepared resource archive batch is forged or lacks exact closure".to_owned(),
            ));
        }
        for (id, page) in &self.lookup_pages {
            if &page.id()? != id {
                return Err(ResourceError::InvalidDefinition(
                    "prepared resource archive lookup page is forged".to_owned(),
                ));
            }
        }
        let mut detached = self.retention.clone();
        let retention_digest = std::mem::take(&mut detached.semantic_digest);
        if self.retention.id
            != canonical_digest(
                "canwu.resource.archive-retention-id.v1",
                &(&self.expected_source_root, &self.directory.id),
            )?
            || retention_digest
                != canonical_digest("canwu.resource.archive-retention.v1", &detached)?
        {
            return Err(ResourceError::InvalidDefinition(
                "prepared resource archive retention handle is forged".to_owned(),
            ));
        }
        Ok(())
    }
}

impl ResourceState {
    /// The digest that names the root an archive commit is prepared from.
    /// A commit applies only to the root it names, so any change of the root
    /// in between makes it stale.
    ///
    /// It commits to what the live root alone holds: the state revision and
    /// the admitted sequence, which every change of the root advances; the
    /// number of hot terminal records, which an applied commit lowers within
    /// the boundary that applies it; and the archive head, which it moves.
    /// The candidates themselves need not be read: the same revision of one
    /// run's root always holds the same ones.
    pub fn archive_source_root(&self) -> Result<String, ResourceError> {
        canonical_digest(
            "canwu.resource.archive-source-root.v2",
            &(
                self.state_revision,
                self.next_admitted_sequence,
                self.hot_terminal.records,
                &self.archive_head,
            ),
        )
    }

    /// Prepares one archive commit from the first `candidate_limit` hot
    /// terminal candidates, in terminal sequence order.
    ///
    /// The batch is cut into blobs of at most
    /// [`MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES`] records, each with its own
    /// membership and temporal page, so one commit carries up to
    /// [`MAX_RESOURCE_ARCHIVE_COMMIT_RECORDS`] records. Its directory also
    /// commits to the cumulative lookup index extended by those records.
    /// Extending the index reads the previous commit's directory and the
    /// lookup pages on the paths of the new keys from `store`; nothing is
    /// written until [`PreparedResourceArchiveBatchV1::store_and_verify`].
    /// The result is a function of this state and the stored index alone.
    pub fn prepare_resource_archive(
        &self,
        candidate_limit: usize,
        store: &dyn ResourceArchiveStore,
    ) -> Result<PreparedResourceArchiveBatchV1, ResourceError> {
        if candidate_limit == 0 || candidate_limit > self.limits.max_archive_candidates {
            return Err(ResourceError::LimitExceeded(
                "resource archive candidate budget is invalid".to_owned(),
            ));
        }
        if !self.hot_terminal_records_attached() {
            return Err(ResourceError::InvalidDefinition(
                "a resource archive commit is prepared from the whole root".to_owned(),
            ));
        }
        let selected_entries: Vec<_> = self
            .terminal_archive_candidates
            .iter()
            .take(candidate_limit.min(MAX_RESOURCE_ARCHIVE_COMMIT_RECORDS))
            .map(|(sequence, key)| (*sequence, key.clone()))
            .collect();
        if selected_entries.is_empty() {
            return Err(ResourceError::NotFound(
                "resource archive has no eligible terminal candidates".to_owned(),
            ));
        }
        let selected: Vec<_> = selected_entries
            .iter()
            .map(|(_, key)| key.clone())
            .collect();
        let expected_source_root = self.archive_source_root()?;
        let mut pages = Vec::with_capacity(
            selected_entries
                .len()
                .div_ceil(MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES),
        );
        for page_entries in selected_entries.chunks(MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES) {
            let mut records = Vec::with_capacity(page_entries.len());
            for (sequence, key) in page_entries {
                records.push(self.terminal_archive_record(key, *sequence)?);
            }
            let mut blob = ResourceArchiveBlobV1 {
                format_version: 1,
                expected_source_root: expected_source_root.clone(),
                records,
                content_id: String::new(),
            };
            blob.content_id = canonical_digest("canwu.resource.archive-blob.v1", &blob)?;
            let memberships = blob
                .records
                .iter()
                .enumerate()
                .map(|(ordinal, record)| {
                    Ok(ResourceArchiveMembershipV1 {
                        key: record.key.clone(),
                        blob_id: blob.content_id.clone(),
                        ordinal: u16::try_from(ordinal).map_err(|_| ResourceError::Overflow)?,
                        terminal_sequence: record.terminal_sequence,
                        semantic_digest: record.semantic_digest.clone(),
                    })
                })
                .collect::<Result<Vec<_>, ResourceError>>()?;
            let mut membership_page = ResourceArchiveMembershipPageV1 {
                id: String::new(),
                memberships,
                semantic_digest: String::new(),
            };
            membership_page.semantic_digest = canonical_digest(
                "canwu.resource.archive-membership-page.v1",
                &membership_page,
            )?;
            membership_page.id = membership_page.semantic_digest.clone();
            let mut temporal_page = ResourceArchiveTemporalPageV1 {
                id: String::new(),
                entries: blob
                    .records
                    .iter()
                    .map(|record| ResourceArchiveTemporalEntryV1 {
                        terminal_sequence: record.terminal_sequence,
                        key: record.key.clone(),
                    })
                    .collect(),
                semantic_digest: String::new(),
            };
            temporal_page
                .entries
                .sort_by_key(|entry| entry.terminal_sequence);
            temporal_page.semantic_digest =
                canonical_digest("canwu.resource.archive-temporal-page.v1", &temporal_page)?;
            temporal_page.id = temporal_page.semantic_digest.clone();
            pages.push(PreparedResourceArchivePageV1 {
                blob,
                membership_page,
                temporal_page,
            });
        }
        let previous_lookup_root = match &self.archive_head.directory_root {
            Some(root) => {
                let previous = load_resource_archive_directory(store, root)?;
                if previous.archived_record_count != self.archive_head.archived_record_count {
                    return Err(ResourceError::InvalidDefinition(
                        "resource archive head directory differs from the head record count"
                            .to_owned(),
                    ));
                }
                Some(previous.lookup_root)
            }
            None => None,
        };
        let lookup = prepare_resource_archive_lookup(
            store,
            previous_lookup_root.as_deref(),
            pages
                .iter()
                .flat_map(|page| &page.membership_page.memberships),
        )?;
        let mut directory = ResourceArchiveIndexDirectoryV1 {
            id: String::new(),
            previous_root: self.archive_head.directory_root.clone(),
            membership_pages: pages
                .iter()
                .map(|page| page.membership_page.id.clone())
                .collect(),
            temporal_pages: pages
                .iter()
                .map(|page| page.temporal_page.id.clone())
                .collect(),
            blob_ids: pages
                .iter()
                .map(|page| page.blob.content_id.clone())
                .collect(),
            archived_record_count: self
                .archive_head
                .archived_record_count
                .checked_add(u64::try_from(selected.len()).map_err(|_| ResourceError::Overflow)?)
                .ok_or(ResourceError::Overflow)?,
            lookup_root: lookup.root,
            semantic_digest: String::new(),
        };
        directory.semantic_digest =
            canonical_digest("canwu.resource.archive-directory.v1", &directory)?;
        directory.id = directory.semantic_digest.clone();
        let handle_id = canonical_digest(
            "canwu.resource.archive-retention-id.v1",
            &(&expected_source_root, &directory.id),
        )?;
        let mut retention = ResourceArchiveRetentionHandleV1 {
            id: handle_id,
            phase: ResourceArchiveRetentionPhaseV1::Prepared,
            expected_source_root: expected_source_root.clone(),
            directory_root: directory.id.clone(),
            object_ids: BTreeMap::from([
                (
                    RESOURCE_ARCHIVE_BLOB_NAMESPACE.to_owned(),
                    directory.blob_ids.iter().cloned().collect(),
                ),
                (
                    RESOURCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE.to_owned(),
                    directory.membership_pages.iter().cloned().collect(),
                ),
                (
                    RESOURCE_ARCHIVE_TEMPORAL_PAGE_NAMESPACE.to_owned(),
                    directory.temporal_pages.iter().cloned().collect(),
                ),
                (
                    RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
                    BTreeSet::from([directory.id.clone()]),
                ),
            ]),
            semantic_digest: String::new(),
        };
        retention.semantic_digest =
            canonical_digest("canwu.resource.archive-retention.v1", &retention)?;
        let mut pages = pages.into_iter();
        let first = pages.next().ok_or_else(|| {
            ResourceError::NotFound(
                "resource archive has no eligible terminal candidates".to_owned(),
            )
        })?;
        Ok(PreparedResourceArchiveBatchV1 {
            expected_source_root,
            selected,
            blob: first.blob,
            membership_page: first.membership_page,
            temporal_page: first.temporal_page,
            additional_pages: pages.collect(),
            lookup_pages: lookup.pages,
            superseded_lookup_pages: lookup.superseded,
            directory,
            retention,
        })
    }

    /// Records how one verified archive commit settled against this root:
    /// its maintenance receipt and retention handle, and, for a commit
    /// prepared from this root (`applied`), the archive head it moves to.
    /// The resource plugin has already taken the selected records out of the
    /// root ([`crate::ResourceRootWork`]); a stale commit changes nothing
    /// else.
    ///
    /// Neither disposition can fail for capacity. An applied commit archives
    /// at least one candidate before its own receipt becomes one, so the
    /// candidate budget holds. The receipt of a stale commit is not an
    /// archive candidate: it stays in the root, outside the candidate and
    /// receipt budgets, until the host application acknowledges the commit's
    /// retention ([`Self::acknowledge_archive_retention`]). A stale commit
    /// therefore always terminates, and a commit queued beside it against a
    /// full root still applies.
    pub(crate) fn record_archive_commit(
        &mut self,
        commit: &VerifiedResourceArchiveCommitV1,
        applied: bool,
    ) -> Result<ResourceArchiveMaintenanceReceiptV1, ResourceError> {
        let disposition = if applied {
            self.archive_head.revision = self
                .archive_head
                .revision
                .checked_add(1)
                .ok_or(ResourceError::Overflow)?;
            self.archive_head.directory_root = Some(commit.directory_root.clone());
            self.archive_head.archived_record_count = self
                .archive_head
                .archived_record_count
                .checked_add(u64::from(commit.archived_records))
                .ok_or(ResourceError::Overflow)?;
            self.archive_head.semantic_digest.clear();
            self.archive_head.semantic_digest =
                canonical_digest("canwu.resource.archive-head.v1", &self.archive_head)?;
            ResourceArchiveMaintenanceDispositionV1::Applied
        } else {
            ResourceArchiveMaintenanceDispositionV1::RejectedStale
        };
        let applied = disposition == ResourceArchiveMaintenanceDispositionV1::Applied;
        let sequence = self.next_admitted_sequence;
        self.next_admitted_sequence = sequence.checked_add(1).ok_or(ResourceError::Overflow)?;
        let mut receipt = ResourceArchiveMaintenanceReceiptV1 {
            sequence,
            retention_handle_id: commit.retention.id.clone(),
            expected_source_root: commit.expected_source_root.clone(),
            directory_root: commit.directory_root.clone(),
            disposition,
            archived_records: if applied { commit.archived_records } else { 0 },
            semantic_digest: String::new(),
        };
        receipt.semantic_digest =
            canonical_digest("canwu.resource.archive-maintenance-receipt.v1", &receipt)?;
        self.archive_maintenance_receipts
            .insert(sequence, receipt.clone());
        if applied {
            self.add_hot_candidate(
                sequence,
                ResourceTerminalRecordKeyV1::ArchiveMaintenanceReceipt(sequence),
            )?;
        }
        let mut handle = commit.retention.clone();
        handle.phase = if applied {
            ResourceArchiveRetentionPhaseV1::Committed
        } else {
            ResourceArchiveRetentionPhaseV1::RejectedStale
        };
        handle.semantic_digest.clear();
        handle.semantic_digest = canonical_digest("canwu.resource.archive-retention.v1", &handle)?;
        // The same batch queued twice shares one handle id: its stale
        // duplicate must not overwrite the handle of the applied commit.
        if applied || !self.archive_retention_handles.contains_key(&handle.id) {
            self.archive_retention_handles
                .insert(handle.id.clone(), handle);
        }
        self.refresh_continuation();
        Ok(receipt)
    }

    /// Retires the recovery metadata of one settled archive commit after the
    /// host application finalized its store-side retention: the retention
    /// handle, and every stale receipt recorded under that handle.
    ///
    /// The same batch queued twice settles once as applied and once as stale
    /// under one handle id, and the acknowledgement names the applied
    /// receipt, so stale receipts are retired by handle, not by sequence.
    ///
    /// The receipt must be the persisted one while it is still in the root.
    /// It may already have left: the receipt of an applied commit is archived
    /// by a later commit, and a stale receipt goes with its first
    /// acknowledgement. The acknowledgement then only retires the handle, so
    /// a repeated or late acknowledgement settles as a no-op.
    pub(crate) fn acknowledge_archive_retention(
        &mut self,
        receipt: &ResourceArchiveMaintenanceReceiptV1,
    ) -> Result<(), ResourceError> {
        if self
            .archive_maintenance_receipts
            .get(&receipt.sequence)
            .is_some_and(|persisted| persisted != receipt)
            || self
                .archive_retention_handles
                .get(&receipt.retention_handle_id)
                .is_some_and(|handle| handle.directory_root != receipt.directory_root)
        {
            return Err(ResourceError::InvalidDefinition(
                "resource archive acknowledgement receipt is forged".to_owned(),
            ));
        }
        self.archive_retention_handles
            .remove(&receipt.retention_handle_id);
        self.archive_maintenance_receipts.retain(|_, hot| {
            hot.disposition != ResourceArchiveMaintenanceDispositionV1::RejectedStale
                || hot.retention_handle_id != receipt.retention_handle_id
        });
        Ok(())
    }

    /// Retires the recovery metadata of one settled archive commit by its
    /// retention handle, when the commit's receipt has already left the root:
    /// the receipt of an applied commit is archived by a later commit, while
    /// the state-side handle stays until it is acknowledged and records how
    /// the commit settled.
    ///
    /// The handle, when the root still holds it, must name `directory_root`.
    /// It is removed with every stale receipt recorded under it. A handle the
    /// root no longer holds was acknowledged before, so this settles as a
    /// no-op, like a repeated acknowledgement by receipt.
    pub(crate) fn acknowledge_archive_retention_handle(
        &mut self,
        retention_handle_id: &str,
        directory_root: &str,
    ) -> Result<(), ResourceError> {
        if self
            .archive_retention_handles
            .get(retention_handle_id)
            .is_some_and(|handle| handle.directory_root != directory_root)
        {
            return Err(ResourceError::InvalidDefinition(
                "resource archive acknowledgement handle is forged".to_owned(),
            ));
        }
        self.archive_retention_handles.remove(retention_handle_id);
        self.archive_maintenance_receipts.retain(|_, hot| {
            hot.disposition != ResourceArchiveMaintenanceDispositionV1::RejectedStale
                || hot.retention_handle_id != retention_handle_id
        });
        Ok(())
    }

    /// Checks that maintenance receipts and archive candidates agree: the
    /// receipt of an applied commit is a candidate under its own sequence,
    /// and the receipt of a stale commit is not a candidate.
    pub(crate) fn validate_archive_maintenance_candidacy(&self) -> Result<(), ResourceError> {
        for (sequence, receipt) in &self.archive_maintenance_receipts {
            let applied = receipt.disposition == ResourceArchiveMaintenanceDispositionV1::Applied;
            let candidate = self.terminal_archive_candidates.get(sequence)
                == Some(&ResourceTerminalRecordKeyV1::ArchiveMaintenanceReceipt(
                    *sequence,
                ));
            if applied != candidate {
                return Err(ResourceError::InvalidDefinition(
                    "resource archive maintenance receipt and archive candidates disagree"
                        .to_owned(),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn terminal_archive_record(
        &self,
        key: &ResourceTerminalRecordKeyV1,
        archive_sequence: u64,
    ) -> Result<ResourceTerminalArchiveRecordV1, ResourceError> {
        let (operation_key, quantity, remainder, evidence, digest, record_sequence, payload) =
            match key {
                ResourceTerminalRecordKeyV1::Outcome(operation_key) => {
                    let value = self.outcomes.get(operation_key).ok_or_else(|| {
                        ResourceError::NotFound(
                            "terminal resource outcome is unavailable".to_owned(),
                        )
                    })?;
                    if !matches!(
                        value.status,
                        ResourceOperationStatus::Applied | ResourceOperationStatus::Rejected
                    ) {
                        return Err(ResourceError::InvalidLifecycle(
                            "resource outcome is not terminal".to_owned(),
                        ));
                    }
                    (
                        value.operation_key.clone(),
                        value.quantity,
                        value.remainder,
                        value.exact_evidence.clone(),
                        value.semantic_digest.clone(),
                        value.sequence,
                        ResourceTerminalArchivePayloadV1::Outcome(value.clone()),
                    )
                }
                ResourceTerminalRecordKeyV1::Demand(id) => {
                    let value = self.demands.get(id).ok_or_else(|| {
                        ResourceError::NotFound(
                            "terminal resource demand is unavailable".to_owned(),
                        )
                    })?;
                    if matches!(
                        value.status,
                        crate::DemandStatus::Open | crate::DemandStatus::PartiallyFulfilled
                    ) {
                        return Err(ResourceError::InvalidLifecycle(
                            "resource demand is not terminal".to_owned(),
                        ));
                    }
                    let digest = canonical_digest("canwu.resource.demand.v1", value)?;
                    (
                        ResourceOperationKey::new(format!("resource:demand-terminal:{digest}"))?,
                        value.fulfilled,
                        value.remainder(),
                        Vec::new(),
                        digest,
                        archive_sequence,
                        ResourceTerminalArchivePayloadV1::Demand(value.clone()),
                    )
                }
                ResourceTerminalRecordKeyV1::Reservation(id) => {
                    let value = self.reservations.get(id).ok_or_else(|| {
                        ResourceError::NotFound(
                            "terminal resource reservation is unavailable".to_owned(),
                        )
                    })?;
                    if value.status == crate::ReservationStatus::Active {
                        return Err(ResourceError::InvalidLifecycle(
                            "resource reservation is not terminal".to_owned(),
                        ));
                    }
                    (
                        value.operation_key.clone(),
                        value.quantity,
                        0,
                        Vec::new(),
                        canonical_digest("canwu.resource.reservation.v1", value)?,
                        archive_sequence,
                        ResourceTerminalArchivePayloadV1::Reservation(value.clone()),
                    )
                }
                ResourceTerminalRecordKeyV1::AllocationLeg(id) => {
                    let value = self.allocation_legs.get(id).ok_or_else(|| {
                        ResourceError::NotFound(
                            "terminal resource allocation leg is unavailable".to_owned(),
                        )
                    })?;
                    if value.status == crate::AllocationLegStatus::Reserved {
                        return Err(ResourceError::InvalidLifecycle(
                            "resource allocation leg is not terminal".to_owned(),
                        ));
                    }
                    (
                        value.operation_key.clone(),
                        value.quantity,
                        0,
                        Vec::new(),
                        value.semantic_digest.clone(),
                        archive_sequence,
                        ResourceTerminalArchivePayloadV1::AllocationLeg(value.clone()),
                    )
                }
                ResourceTerminalRecordKeyV1::Consumption(id) => {
                    let value = self.consumptions.get(id).ok_or_else(|| {
                        ResourceError::NotFound("terminal consumption is unavailable".to_owned())
                    })?;
                    if value.status != ConsumptionStatus::Settled {
                        return Err(ResourceError::InvalidLifecycle(
                            "resource consumption is not terminal".to_owned(),
                        ));
                    }
                    (
                        value.operation_key.clone(),
                        value.quantity,
                        0,
                        vec![value.consumer_evidence.clone()],
                        value.semantic_digest.clone(),
                        value.terminal_sequence,
                        ResourceTerminalArchivePayloadV1::Consumption(value.clone()),
                    )
                }
                ResourceTerminalRecordKeyV1::Fulfillment(id) => {
                    let value = self.fulfillments.get(id).ok_or_else(|| {
                        ResourceError::NotFound("terminal fulfillment is unavailable".to_owned())
                    })?;
                    if !matches!(
                        value.status,
                        FulfillmentStatus::Partial
                            | FulfillmentStatus::Complete
                            | FulfillmentStatus::RejectedMinimum
                            | FulfillmentStatus::Cancelled
                            | FulfillmentStatus::Expired
                    ) {
                        return Err(ResourceError::InvalidLifecycle(
                            "resource fulfillment is not terminal".to_owned(),
                        ));
                    }
                    (
                        value.operation_key.clone(),
                        value.consumed_quantity,
                        value.remainder,
                        Vec::new(),
                        value.semantic_digest.clone(),
                        value.terminal_sequence,
                        ResourceTerminalArchivePayloadV1::Fulfillment(value.clone()),
                    )
                }
                ResourceTerminalRecordKeyV1::Loss(id) => {
                    let value = self.losses.get(id).ok_or_else(|| {
                        ResourceError::NotFound("terminal loss is unavailable".to_owned())
                    })?;
                    (
                        value.operation_key.clone(),
                        value.quantity,
                        0,
                        match &value.cause {
                            canwu_api::EvidenceRef::DomainRecordVersion(version) => {
                                vec![version.clone()]
                            }
                            _ => Vec::new(),
                        },
                        canonical_digest("canwu.resource.loss.v1", value)?,
                        value.terminal_sequence,
                        ResourceTerminalArchivePayloadV1::Loss(value.clone()),
                    )
                }
                ResourceTerminalRecordKeyV1::Transfer(id) => {
                    let value = self.transfers.get(id).ok_or_else(|| {
                        ResourceError::NotFound("terminal transfer is unavailable".to_owned())
                    })?;
                    if !matches!(
                        value.state,
                        ResourceTransferState::Accepted
                            | ResourceTransferState::Lost
                            | ResourceTransferState::ExternalOutflowSettled
                            | ResourceTransferState::Cancelled
                            | ResourceTransferState::Returned
                    ) {
                        return Err(ResourceError::InvalidLifecycle(
                            "resource transfer is not terminal".to_owned(),
                        ));
                    }
                    (
                        value.operation_key.clone(),
                        value.quantity,
                        0,
                        value.exact_evidence.clone(),
                        canonical_digest("canwu.resource.transfer.v1", value)?,
                        value.terminal_sequence,
                        ResourceTerminalArchivePayloadV1::Transfer(value.clone()),
                    )
                }
                ResourceTerminalRecordKeyV1::ExternalCompletionParticipant(acquisition) => {
                    let value = self
                        .external_completion_participants
                        .terminal_grants
                        .get(acquisition)
                        .ok_or_else(|| {
                            ResourceError::NotFound(
                                "terminal external completion participant is unavailable"
                                    .to_owned(),
                            )
                        })?;
                    if value.grant.state != crate::CompletionGrantStateV1::Completed {
                        return Err(ResourceError::InvalidLifecycle(
                            "external completion participant archive candidate is not completed"
                                .to_owned(),
                        ));
                    }
                    (
                        value.grant.operation_key.clone(),
                        value.grant.reserved_units,
                        0,
                        vec![value.coordinator_source.clone()],
                        canonical_digest(
                            "canwu.resource.external-completion-participant.v1",
                            value,
                        )?,
                        archive_sequence,
                        ResourceTerminalArchivePayloadV1::ExternalCompletionParticipant(
                            value.clone(),
                        ),
                    )
                }
                ResourceTerminalRecordKeyV1::LeaseReceipt(sequence) => {
                    let value = self
                        .completion_leases
                        .receipts
                        .get(sequence)
                        .ok_or_else(|| {
                            ResourceError::NotFound(
                                "terminal completion receipt is unavailable".to_owned(),
                            )
                        })?;
                    (
                        value.operation_key.clone(),
                        value.reserved_units,
                        0,
                        Vec::new(),
                        value.semantic_digest.clone(),
                        *sequence,
                        ResourceTerminalArchivePayloadV1::LeaseReceipt(value.clone()),
                    )
                }
                ResourceTerminalRecordKeyV1::ArchiveMaintenanceReceipt(sequence) => {
                    let value =
                        self.archive_maintenance_receipts
                            .get(sequence)
                            .ok_or_else(|| {
                                ResourceError::NotFound(
                                    "terminal resource archive maintenance receipt is unavailable"
                                        .to_owned(),
                                )
                            })?;
                    (
                        ResourceOperationKey::new(format!(
                            "resource:archive-maintenance:{sequence}"
                        ))?,
                        u64::from(value.archived_records),
                        0,
                        Vec::new(),
                        value.semantic_digest.clone(),
                        *sequence,
                        ResourceTerminalArchivePayloadV1::ArchiveMaintenanceReceipt(value.clone()),
                    )
                }
            };
        if !matches!(
            key,
            ResourceTerminalRecordKeyV1::LeaseReceipt(_)
                | ResourceTerminalRecordKeyV1::ArchiveMaintenanceReceipt(_)
        ) && record_sequence != archive_sequence
        {
            return Err(ResourceError::InvalidDefinition(
                "resource terminal archive candidate sequence differs from its hot record"
                    .to_owned(),
            ));
        }
        Ok(ResourceTerminalArchiveRecordV1 {
            key: key.clone(),
            operation_key,
            quantity,
            remainder,
            exact_evidence: evidence,
            semantic_digest: digest,
            terminal_sequence: archive_sequence,
            payload,
        })
    }

    /// Archives the attached hot terminal record `key`: takes it out of the
    /// ledger and the root, with what archiving it does to the live root. The
    /// caller removes its candidate entry.
    pub(crate) fn remove_hot_terminal(
        &mut self,
        key: &ResourceTerminalRecordKeyV1,
    ) -> Result<(), ResourceError> {
        self.drop_hot_candidate(key)?;
        match key {
            ResourceTerminalRecordKeyV1::Outcome(id) => {
                self.outcomes.remove(id);
            }
            ResourceTerminalRecordKeyV1::Demand(id) => {
                self.demands.remove(id);
            }
            ResourceTerminalRecordKeyV1::Reservation(id) => {
                if let Some(reservation) = self.reservations.remove(id)
                    && let Some(ids) = self.reservation_by_demand.get_mut(&reservation.demand)
                {
                    ids.remove(id);
                    if ids.is_empty() {
                        self.reservation_by_demand.remove(&reservation.demand);
                    }
                }
            }
            ResourceTerminalRecordKeyV1::AllocationLeg(id) => {
                self.allocation_legs.remove(id);
            }
            ResourceTerminalRecordKeyV1::Consumption(id) => {
                self.consumptions.remove(id);
            }
            ResourceTerminalRecordKeyV1::Fulfillment(id) => {
                self.fulfillments.remove(id);
            }
            ResourceTerminalRecordKeyV1::Loss(id) => {
                self.losses.remove(id);
            }
            ResourceTerminalRecordKeyV1::Transfer(id) => {
                self.transfers.remove(id);
                self.active_transfers.remove(id);
            }
            ResourceTerminalRecordKeyV1::ExternalCompletionParticipant(acquisition) => {
                self.external_completion_participants
                    .terminal_grants
                    .remove(acquisition);
            }
            // A finished lease leaves the root when it finishes
            // (`ResourceState::retire_finished_leases`), not with its receipts.
            ResourceTerminalRecordKeyV1::LeaseReceipt(sequence) => {
                self.completion_leases.receipts.remove(sequence);
            }
            ResourceTerminalRecordKeyV1::ArchiveMaintenanceReceipt(sequence) => {
                self.archive_maintenance_receipts.remove(sequence);
            }
        }
        Ok(())
    }
}

/// Authenticates one archive commit: the directory's own identity, every blob
/// and typed record it names, and the membership and temporal pages that must
/// cover exactly those records.
///
/// The cumulative lookup index is not read here, only the shape of
/// `lookup_root`: a historical directory stays authentic after the lookup
/// pages that only it reaches are gone. [`validate_resource_archive_store`]
/// audits the index of the head.
pub fn authenticate_resource_archive_directory(
    store: &dyn ResourceArchiveStore,
    directory: &ResourceArchiveIndexDirectoryV1,
) -> Result<(), ResourceError> {
    authenticate_resource_archive_directory_closure(store, directory).map(|_| ())
}

/// [`authenticate_resource_archive_directory`], returning every archived key
/// of the commit with the blob and ordinal its membership binds.
fn authenticate_resource_archive_directory_closure(
    store: &dyn ResourceArchiveStore,
    directory: &ResourceArchiveIndexDirectoryV1,
) -> Result<Vec<(ResourceTerminalRecordKeyV1, String, u16)>, ResourceError> {
    authenticate_resource_archive_directory_identity(directory)?;
    let unique_blobs = directory.blob_ids.iter().collect::<BTreeSet<_>>();
    let unique_membership_pages = directory.membership_pages.iter().collect::<BTreeSet<_>>();
    let unique_temporal_pages = directory.temporal_pages.iter().collect::<BTreeSet<_>>();
    if directory.blob_ids.is_empty()
        || directory.blob_ids.len() > MAX_RESOURCE_ARCHIVE_COMMIT_PAGES
        || directory.membership_pages.len() != directory.blob_ids.len()
        || directory.temporal_pages.len() != directory.blob_ids.len()
        || unique_blobs.len() != directory.blob_ids.len()
        || unique_membership_pages.len() != directory.membership_pages.len()
        || unique_temporal_pages.len() != directory.temporal_pages.len()
        || directory.previous_root.as_ref() == Some(&directory.id)
    {
        return Err(ResourceError::InvalidDefinition(
            "resource archive directory object lists are empty, duplicated, cyclic, or unbounded"
                .to_owned(),
        ));
    }
    let mut archived_records = BTreeMap::new();
    let mut source_root: Option<String> = None;
    for blob_id in &directory.blob_ids {
        let blob: ResourceArchiveBlobV1 =
            load_encoded(store, RESOURCE_ARCHIVE_BLOB_NAMESPACE, blob_id)?.ok_or_else(|| {
                ResourceError::NotFound("resource archive blob is missing".to_owned())
            })?;
        let mut detached = blob.clone();
        detached.content_id.clear();
        if blob.content_id != *blob_id
            || blob.content_id != canonical_digest("canwu.resource.archive-blob.v1", &detached)?
            || blob.format_version != 1
            || blob.records.is_empty()
            || blob.records.len() > MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES
            || blob.expected_source_root.len() != 64
            || source_root
                .as_ref()
                .is_some_and(|expected| expected != &blob.expected_source_root)
        {
            return Err(ResourceError::InvalidDefinition(
                "resource archive blob is forged, unbounded, or source-inconsistent".to_owned(),
            ));
        }
        source_root.get_or_insert_with(|| blob.expected_source_root.clone());
        validate_archived_consumption_links(&blob.records)?;
        for (ordinal, record) in blob.records.iter().enumerate() {
            validate_resource_terminal_archive_record(record)?;
            let ordinal = u16::try_from(ordinal).map_err(|_| ResourceError::Overflow)?;
            if record.semantic_digest.len() != 64
                || archived_records
                    .insert(
                        record.key.clone(),
                        (
                            blob_id.clone(),
                            ordinal,
                            record.terminal_sequence,
                            record.semantic_digest.clone(),
                        ),
                    )
                    .is_some()
            {
                return Err(ResourceError::InvalidDefinition(
                    "resource archive contains an invalid or duplicate terminal record".to_owned(),
                ));
            }
        }
    }
    let mut membership_keys = BTreeSet::new();
    for page_id in &directory.membership_pages {
        let page: ResourceArchiveMembershipPageV1 = load_encoded(
            store,
            RESOURCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE,
            page_id,
        )?
        .ok_or_else(|| {
            ResourceError::NotFound("resource archive membership page is missing".to_owned())
        })?;
        let mut detached = page.clone();
        detached.id.clear();
        detached.semantic_digest.clear();
        let digest = canonical_digest("canwu.resource.archive-membership-page.v1", &detached)?;
        if page.id != *page_id
            || page.id != digest
            || page.semantic_digest != digest
            || page.memberships.is_empty()
            || page.memberships.len() > MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES
        {
            return Err(ResourceError::InvalidDefinition(
                "resource archive membership page is forged, empty, or exceeds its bound"
                    .to_owned(),
            ));
        }
        for membership in &page.memberships {
            let Some((blob_id, ordinal, sequence, digest)) = archived_records.get(&membership.key)
            else {
                return Err(ResourceError::InvalidDefinition(
                    "resource archive membership names an unknown terminal record".to_owned(),
                ));
            };
            if &membership.blob_id != blob_id
                || membership.ordinal != *ordinal
                || membership.terminal_sequence != *sequence
                || &membership.semantic_digest != digest
                || !membership_keys.insert(membership.key.clone())
            {
                return Err(ResourceError::InvalidDefinition(
                    "resource archive membership does not bind its exact terminal record"
                        .to_owned(),
                ));
            }
        }
    }
    let mut temporal_keys = BTreeSet::new();
    for page_id in &directory.temporal_pages {
        let page: ResourceArchiveTemporalPageV1 =
            load_encoded(store, RESOURCE_ARCHIVE_TEMPORAL_PAGE_NAMESPACE, page_id)?.ok_or_else(
                || ResourceError::NotFound("resource archive temporal page is missing".to_owned()),
            )?;
        let mut detached = page.clone();
        detached.id.clear();
        detached.semantic_digest.clear();
        let digest = canonical_digest("canwu.resource.archive-temporal-page.v1", &detached)?;
        if page.id != *page_id
            || page.id != digest
            || page.semantic_digest != digest
            || page.entries.is_empty()
            || page.entries.len() > MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES
            || !page
                .entries
                .windows(2)
                .all(|pair| pair[0].terminal_sequence < pair[1].terminal_sequence)
        {
            return Err(ResourceError::InvalidDefinition(
                "resource archive temporal page is forged, empty, unordered, or exceeds its bound"
                    .to_owned(),
            ));
        }
        for entry in &page.entries {
            let Some((_, _, sequence, _)) = archived_records.get(&entry.key) else {
                return Err(ResourceError::InvalidDefinition(
                    "resource archive temporal index names an unknown terminal record".to_owned(),
                ));
            };
            if entry.terminal_sequence != *sequence || !temporal_keys.insert(entry.key.clone()) {
                return Err(ResourceError::InvalidDefinition(
                    "resource archive temporal index does not bind its exact terminal record"
                        .to_owned(),
                ));
            }
        }
    }
    if membership_keys.len() != archived_records.len()
        || temporal_keys.len() != archived_records.len()
    {
        return Err(ResourceError::InvalidDefinition(
            "resource archive indexes do not cover the complete terminal blob set".to_owned(),
        ));
    }
    Ok(archived_records
        .into_iter()
        .map(|(key, (blob_id, ordinal, _, _))| (key, blob_id, ordinal))
        .collect())
}

/// Checks that a decoded directory is the object its id commits to and that
/// it names a lookup root. Reads nothing.
fn authenticate_resource_archive_directory_identity(
    directory: &ResourceArchiveIndexDirectoryV1,
) -> Result<(), ResourceError> {
    let mut detached = directory.clone();
    let id = std::mem::take(&mut detached.id);
    let semantic_digest = std::mem::take(&mut detached.semantic_digest);
    let expected = canonical_digest("canwu.resource.archive-directory.v1", &detached)?;
    if id != expected || semantic_digest != expected || !is_lookup_digest(&directory.lookup_root) {
        return Err(ResourceError::InvalidDefinition(
            "resource archive directory root is forged".to_owned(),
        ));
    }
    Ok(())
}

/// Loads one directory by id and authenticates its identity only: the single
/// directory read of a cold lookup and of an archive preparation.
fn load_resource_archive_directory(
    store: &dyn ResourceArchiveStore,
    root: &str,
) -> Result<ResourceArchiveIndexDirectoryV1, ResourceError> {
    let directory: ResourceArchiveIndexDirectoryV1 =
        load_encoded(store, RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, root)?.ok_or_else(
            || ResourceError::NotFound("resource archive lookup directory is missing".to_owned()),
        )?;
    authenticate_resource_archive_directory_identity(&directory)?;
    if directory.id != root {
        return Err(ResourceError::InvalidDefinition(
            "resource archive lookup directory identity differs".to_owned(),
        ));
    }
    Ok(directory)
}

/// Digest under which the cumulative lookup index files one terminal record
/// key. Its hexadecimal nibbles are the key's path through the index.
pub fn resource_archive_lookup_key_digest(
    key: &ResourceTerminalRecordKeyV1,
) -> Result<String, ResourceError> {
    canonical_digest(RESOURCE_ARCHIVE_LOOKUP_KEY_DOMAIN, key)
}

fn is_lookup_digest_prefix(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_lookup_digest(value: &str) -> bool {
    value.len() == 64 && is_lookup_digest_prefix(value)
}

/// Loads one lookup page by id and authenticates it: bounded size, the id is
/// the digest of the page, and the page is well formed at `prefix`.
fn load_resource_archive_lookup_page(
    store: &dyn ResourceArchiveStore,
    id: &str,
    prefix: &str,
) -> Result<ResourceArchiveLookupPageV1, ResourceError> {
    find_resource_archive_lookup_page(store, id, prefix)?.ok_or_else(|| {
        ResourceError::NotFound("resource archive lookup page is missing".to_owned())
    })
}

fn find_resource_archive_lookup_page(
    store: &dyn ResourceArchiveStore,
    id: &str,
    prefix: &str,
) -> Result<Option<ResourceArchiveLookupPageV1>, ResourceError> {
    let Some(bytes) =
        store.load_resource_archive_object(RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE, id)?
    else {
        return Ok(None);
    };
    if bytes.len() > MAX_RESOURCE_ARCHIVE_LOOKUP_PAGE_BYTES {
        return Err(ResourceError::InvalidDefinition(
            "resource archive lookup page exceeds its size bound".to_owned(),
        ));
    }
    let page: ResourceArchiveLookupPageV1 = serde_json::from_slice(&bytes).map_err(|error| {
        ResourceError::InvalidDefinition(format!(
            "resource archive object decoding failed: {error}"
        ))
    })?;
    if page.id()? != id {
        return Err(ResourceError::InvalidDefinition(
            "resource archive lookup page is forged".to_owned(),
        ));
    }
    page.validate_at(prefix)?;
    Ok(Some(page))
}

/// Extends the cumulative lookup index `previous_root` with one commit's
/// memberships and returns the new root, the pages to store, and the previous
/// pages the new root no longer reaches.
///
/// Only the pages on the paths of the new keys are read from `store` and
/// rewritten; every other page is shared with the previous index. A key that
/// is already indexed is pointed at its new record, so a lookup returns the
/// newest archived record of a key. The result depends only on the previous
/// index and the memberships, never on their order.
pub fn prepare_resource_archive_lookup<'a>(
    store: &dyn ResourceArchiveStore,
    previous_root: Option<&str>,
    memberships: impl IntoIterator<Item = &'a ResourceArchiveMembershipV1>,
) -> Result<PreparedResourceArchiveLookupV1, ResourceError> {
    let entries = memberships
        .into_iter()
        .map(|membership| {
            Ok(ResourceArchiveLookupEntryV1 {
                key_digest: resource_archive_lookup_key_digest(&membership.key)?,
                blob_id: membership.blob_id.clone(),
                ordinal: membership.ordinal,
            })
        })
        .collect::<Result<Vec<_>, ResourceError>>()?;
    prepare_resource_archive_lookup_entries(store, previous_root, entries)
}

/// [`prepare_resource_archive_lookup`] for entries whose key digests the
/// caller computed: extends the index `previous_root` with `entries`, in any
/// order, and returns the new root, the pages to store, and the previous
/// pages the new root no longer reaches.
///
/// The index format does not depend on what a key is, only on its digest, so
/// another archive can keep its own cumulative index in this format: it
/// digests its keys under its own domain, and its store serves the pages
/// under [`RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE`]. Each key digest must be
/// a 64-character lowercase hexadecimal digest and appear once, and each
/// blob id must be one too.
pub fn prepare_resource_archive_lookup_entries(
    store: &dyn ResourceArchiveStore,
    previous_root: Option<&str>,
    mut entries: Vec<ResourceArchiveLookupEntryV1>,
) -> Result<PreparedResourceArchiveLookupV1, ResourceError> {
    entries.sort();
    if entries.is_empty()
        || entries
            .windows(2)
            .any(|pair| pair[0].key_digest == pair[1].key_digest)
        || entries
            .iter()
            .any(|entry| !is_lookup_digest(&entry.key_digest) || !is_lookup_digest(&entry.blob_id))
    {
        return Err(ResourceError::InvalidDefinition(
            "resource archive lookup extension is empty, repeats a key, or names no blob"
                .to_owned(),
        ));
    }
    let mut extension = ResourceArchiveLookupExtension {
        store,
        pages: BTreeMap::new(),
        replaced: BTreeSet::new(),
    };
    let root = extension.insert(previous_root, "", &entries)?;
    let ResourceArchiveLookupExtension {
        pages,
        mut replaced,
        ..
    } = extension;
    replaced.retain(|id| !pages.contains_key(id));
    Ok(PreparedResourceArchiveLookupV1 {
        root,
        pages,
        superseded: replaced,
    })
}

struct ResourceArchiveLookupExtension<'a> {
    store: &'a dyn ResourceArchiveStore,
    pages: BTreeMap<String, ResourceArchiveLookupPageV1>,
    replaced: BTreeSet<String>,
}

impl ResourceArchiveLookupExtension<'_> {
    /// Adds `entries`, all under `prefix` and ordered by key digest, to the
    /// page `page_id` (or to nothing) and returns the id of its replacement.
    fn insert(
        &mut self,
        page_id: Option<&str>,
        prefix: &str,
        entries: &[ResourceArchiveLookupEntryV1],
    ) -> Result<String, ResourceError> {
        let page = match page_id {
            Some(id) => {
                self.replaced.insert(id.to_owned());
                Some(load_resource_archive_lookup_page(self.store, id, prefix)?)
            }
            None => None,
        };
        let page = page.unwrap_or_default();
        if page.children.is_empty() {
            return self.build(prefix, &merge_lookup_entries(page.entries, entries));
        }
        let mut children = page.children;
        for group in lookup_nibble_groups(entries, prefix.len()) {
            let nibble = &group[0].key_digest[prefix.len()..=prefix.len()];
            let child = self.insert(
                children.get(nibble).map(String::as_str),
                &format!("{prefix}{nibble}"),
                group,
            )?;
            children.insert(nibble.to_owned(), child);
        }
        self.emit(ResourceArchiveLookupPageV1 {
            prefix: prefix.to_owned(),
            entries: Vec::new(),
            children,
        })
    }

    /// Writes the canonical pages for `entries`, the complete ordered key set
    /// under `prefix`: one leaf while they fit, otherwise one child per
    /// nibble.
    fn build(
        &mut self,
        prefix: &str,
        entries: &[ResourceArchiveLookupEntryV1],
    ) -> Result<String, ResourceError> {
        if entries.len() <= MAX_RESOURCE_ARCHIVE_LOOKUP_LEAF_ENTRIES {
            return self.emit(ResourceArchiveLookupPageV1 {
                prefix: prefix.to_owned(),
                entries: entries.to_vec(),
                children: BTreeMap::new(),
            });
        }
        if prefix.len() >= MAX_RESOURCE_ARCHIVE_LOOKUP_DEPTH {
            return Err(ResourceError::InvalidDefinition(
                "resource archive lookup keys share one complete digest".to_owned(),
            ));
        }
        let mut children = BTreeMap::new();
        for group in lookup_nibble_groups(entries, prefix.len()) {
            let nibble = &group[0].key_digest[prefix.len()..=prefix.len()];
            let child = self.build(&format!("{prefix}{nibble}"), group)?;
            children.insert(nibble.to_owned(), child);
        }
        self.emit(ResourceArchiveLookupPageV1 {
            prefix: prefix.to_owned(),
            entries: Vec::new(),
            children,
        })
    }

    fn emit(&mut self, page: ResourceArchiveLookupPageV1) -> Result<String, ResourceError> {
        let id = page.id()?;
        self.pages.insert(id.clone(), page);
        Ok(id)
    }
}

/// Splits entries ordered by key digest into runs that share the nibble at
/// `depth`.
fn lookup_nibble_groups(
    entries: &[ResourceArchiveLookupEntryV1],
    depth: usize,
) -> impl Iterator<Item = &[ResourceArchiveLookupEntryV1]> {
    entries.chunk_by(move |left, right| {
        left.key_digest.as_bytes()[depth] == right.key_digest.as_bytes()[depth]
    })
}

/// Merges two entry lists ordered by key digest. An added entry replaces an
/// existing one with the same key digest.
fn merge_lookup_entries(
    existing: Vec<ResourceArchiveLookupEntryV1>,
    added: &[ResourceArchiveLookupEntryV1],
) -> Vec<ResourceArchiveLookupEntryV1> {
    let mut merged = Vec::with_capacity(existing.len() + added.len());
    let mut existing = existing.into_iter().peekable();
    for entry in added {
        while existing
            .peek()
            .is_some_and(|current| current.key_digest < entry.key_digest)
        {
            merged.extend(existing.next());
        }
        if existing
            .peek()
            .is_some_and(|current| current.key_digest == entry.key_digest)
        {
            existing.next();
        }
        merged.push(entry.clone());
    }
    merged.extend(existing);
    merged
}

/// Walks the lookup index from `root` to the entry for `key_digest`, reading
/// one page per level, each authenticated by its id and checked against the
/// prefix it was reached by.
///
/// It reads at most [`MAX_RESOURCE_ARCHIVE_LOOKUP_DEPTH`]` + 1` pages. A key
/// the index does not hold is `None`; a missing, forged, oversized, or
/// malformed page is an error, never a miss. `key_digest` must be a
/// 64-character lowercase hexadecimal digest.
pub fn find_resource_archive_lookup_entry(
    store: &dyn ResourceArchiveStore,
    root: &str,
    key_digest: &str,
) -> Result<Option<ResourceArchiveLookupEntryV1>, ResourceError> {
    if !is_lookup_digest(key_digest) {
        return Err(ResourceError::InvalidDefinition(
            "resource archive lookup key digest is malformed".to_owned(),
        ));
    }
    let mut page_id = root.to_owned();
    for depth in 0..=MAX_RESOURCE_ARCHIVE_LOOKUP_DEPTH {
        let page = load_resource_archive_lookup_page(store, &page_id, &key_digest[..depth])?;
        if page.children.is_empty() {
            return Ok(page
                .entries
                .into_iter()
                .find(|entry| entry.key_digest == key_digest));
        }
        let Some(child) = page.children.get(&key_digest[depth..=depth]) else {
            return Ok(None);
        };
        page_id.clone_from(child);
    }
    // Unreachable: a page at the deepest prefix is authenticated as a leaf.
    Err(ResourceError::InvalidDefinition(
        "resource cold membership lookup exceeded its page-read bound".to_owned(),
    ))
}

/// Checks that every entry of `expected`, ordered by key digest and all under
/// `prefix`, is exactly what the lookup index below `page_id` holds for its
/// key, reading each page on their paths once. Call it with the root page and
/// the empty prefix to check that a stored index resolves every record of a
/// commit.
pub fn verify_resource_archive_lookup_entries(
    store: &dyn ResourceArchiveStore,
    page_id: &str,
    prefix: &str,
    expected: &[ResourceArchiveLookupEntryV1],
) -> Result<(), ResourceError> {
    if !expected.windows(2).all(|pair| pair[0] < pair[1])
        || expected.iter().any(|entry| {
            !is_lookup_digest(&entry.key_digest) || !entry.key_digest.starts_with(prefix)
        })
    {
        return Err(ResourceError::InvalidDefinition(
            "resource archive lookup entries are unordered, malformed, or outside their prefix"
                .to_owned(),
        ));
    }
    let unresolved = || {
        ResourceError::InvalidDefinition(
            "resource archive lookup index does not resolve every record of its commit".to_owned(),
        )
    };
    let page = load_resource_archive_lookup_page(store, page_id, prefix)?;
    if page.children.is_empty() {
        return expected
            .iter()
            .all(|entry| page.entries.binary_search(entry).is_ok())
            .then_some(())
            .ok_or_else(unresolved);
    }
    for group in lookup_nibble_groups(expected, prefix.len()) {
        let nibble = &group[0].key_digest[prefix.len()..=prefix.len()];
        let child = page.children.get(nibble).ok_or_else(unresolved)?;
        verify_resource_archive_lookup_entries(store, child, &format!("{prefix}{nibble}"), group)?;
    }
    Ok(())
}

/// One archived record as the restore audit holds it while it walks the
/// directory chain: the key digest, the index of its blob among the blobs
/// seen so far, and its ordinal. About 40 bytes, so an audit can hold one per
/// archived record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceArchiveAuditEntry {
    key_digest: [u8; 32],
    blob: u32,
    ordinal: u16,
}

impl ResourceArchiveAuditEntry {
    /// An entry for the record at `ordinal` of the blob at position `blob` of
    /// the blob list passed to [`audit_resource_archive_lookup_index`].
    /// `key_digest` is the 64-character lowercase hexadecimal digest the
    /// index files the record's key under.
    pub fn new(key_digest: &str, blob: u32, ordinal: u16) -> Result<Self, ResourceError> {
        if !is_lookup_digest(key_digest) {
            return Err(ResourceError::InvalidDefinition(
                "resource archive audit key digest is malformed".to_owned(),
            ));
        }
        let key_digest = blake3::Hash::from_hex(key_digest)
            .map_err(|error| ResourceError::InvalidDefinition(error.to_string()))?;
        Ok(Self {
            key_digest: *key_digest.as_bytes(),
            blob,
            ordinal,
        })
    }

    fn nibble(&self, depth: usize) -> u8 {
        let byte = self.key_digest[depth / 2];
        if depth & 1 == 0 {
            byte >> 4
        } else {
            byte & 0x0f
        }
    }
}

/// Rebuilds the canonical lookup page for `entries`, the complete key set
/// under `prefix` ordered by key digest, and requires the store to hold
/// exactly that page under its id. Returns the id.
///
/// Because the index is a function of its key set, a stored index agrees
/// with the directory chain exactly when this reaches the head's lookup root.
fn audit_resource_archive_lookup_page(
    store: &dyn ResourceArchiveStore,
    blob_ids: &[String],
    prefix: &mut String,
    entries: &[ResourceArchiveAuditEntry],
) -> Result<String, ResourceError> {
    let depth = prefix.len();
    let mut page = ResourceArchiveLookupPageV1 {
        prefix: prefix.clone(),
        ..ResourceArchiveLookupPageV1::default()
    };
    if entries.len() <= MAX_RESOURCE_ARCHIVE_LOOKUP_LEAF_ENTRIES {
        page.entries = entries
            .iter()
            .map(|entry| ResourceArchiveLookupEntryV1 {
                key_digest: blake3::Hash::from_bytes(entry.key_digest)
                    .to_hex()
                    .to_string(),
                blob_id: blob_ids[entry.blob as usize].clone(),
                ordinal: entry.ordinal,
            })
            .collect();
    } else {
        if depth >= MAX_RESOURCE_ARCHIVE_LOOKUP_DEPTH {
            return Err(ResourceError::InvalidDefinition(
                "resource archive lookup keys share one complete digest".to_owned(),
            ));
        }
        for group in entries.chunk_by(|left, right| left.nibble(depth) == right.nibble(depth)) {
            let nibble = char::from_digit(u32::from(group[0].nibble(depth)), 16)
                .ok_or(ResourceError::Overflow)?;
            prefix.push(nibble);
            let child = audit_resource_archive_lookup_page(store, blob_ids, prefix, group)?;
            prefix.pop();
            page.children.insert(nibble.to_string(), child);
        }
    }
    let id = page.id()?;
    let stored = find_resource_archive_lookup_page(store, &id, prefix)?.ok_or_else(|| {
        ResourceError::NotFound(
            "resource archive lookup index differs from its directory chain or lacks a page"
                .to_owned(),
        )
    })?;
    if stored != page {
        return Err(ResourceError::InvalidDefinition(
            "resource archive lookup page is forged".to_owned(),
        ));
    }
    Ok(id)
}

/// Rebuilds the canonical lookup index over `entries` and requires `store`
/// to hold every page of it. Returns its root page id, or `None` when there
/// are no entries.
///
/// `entries` lists every archived record of a directory chain, newest commit
/// first; the first entry of each key digest is the one the index must name,
/// as a key archived again resolves to its newest record. `blob_ids` holds
/// the blob ids the entries' `blob` positions refer to. Because the index is
/// a function of its entries, a stored index agrees with the chain exactly
/// when the returned root is the root the head directory names. The cost is
/// linear in the entries, and the pages are rebuilt and compared one at a
/// time, so the audit holds the entries and one path of pages.
pub fn audit_resource_archive_lookup_index(
    store: &dyn ResourceArchiveStore,
    blob_ids: &[String],
    mut entries: Vec<ResourceArchiveAuditEntry>,
) -> Result<Option<String>, ResourceError> {
    if entries.is_empty() {
        return Ok(None);
    }
    if entries.iter().any(|entry| {
        usize::try_from(entry.blob)
            .ok()
            .is_none_or(|blob| blob >= blob_ids.len())
    }) {
        return Err(ResourceError::InvalidDefinition(
            "resource archive audit entry names a blob outside its list".to_owned(),
        ));
    }
    entries.sort_by_key(|entry| entry.key_digest);
    entries.dedup_by(|later, earlier| later.key_digest == earlier.key_digest);
    audit_resource_archive_lookup_page(store, blob_ids, &mut String::new(), &entries).map(Some)
}

/// Adds to `pages` the id of every lookup page reachable from `page_id`.
///
/// With `complete`, a missing page is an error: the index of the head, or of
/// a commit still in flight, must be whole. Without it, a missing page ends
/// that branch: the index of an earlier commit may have been pruned by a host
/// application that no longer restores or replays through it. A page already
/// in `pages` is not walked again, so indexes that share pages are walked
/// once. Call it with the root page and the empty prefix.
pub fn collect_resource_archive_lookup_pages(
    store: &dyn ResourceArchiveStore,
    page_id: &str,
    prefix: &str,
    complete: bool,
    pages: &mut BTreeSet<String>,
) -> Result<(), ResourceError> {
    if pages.contains(page_id) {
        return Ok(());
    }
    let page = match find_resource_archive_lookup_page(store, page_id, prefix)? {
        Some(page) => page,
        None if complete => {
            return Err(ResourceError::NotFound(
                "resource archive lookup page is missing".to_owned(),
            ));
        }
        None => return Ok(()),
    };
    pages.insert(page_id.to_owned());
    for (nibble, child) in &page.children {
        collect_resource_archive_lookup_pages(
            store,
            child,
            &format!("{prefix}{nibble}"),
            complete,
            pages,
        )?;
    }
    Ok(())
}

/// Checks, within one archive blob, that every consumption whose
/// fulfillment the blob also holds names the fulfillment it recorded
/// ([`crate::consumption_names_its_fulfillment`]).
fn validate_archived_consumption_links(
    records: &[ResourceTerminalArchiveRecordV1],
) -> Result<(), ResourceError> {
    let fulfillments: BTreeMap<_, _> = records
        .iter()
        .filter_map(|record| match &record.payload {
            ResourceTerminalArchivePayloadV1::Fulfillment(value) => Some((&value.id, value)),
            _ => None,
        })
        .collect();
    for record in records {
        if let ResourceTerminalArchivePayloadV1::Consumption(consumption) = &record.payload
            && let Some(fulfillment) = fulfillments.get(&consumption.fulfillment)
            && !crate::consumption_names_its_fulfillment(consumption, fulfillment)
        {
            return Err(ResourceError::InvalidDefinition(
                "archived resource consumption names a fulfillment of another consumption"
                    .to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_resource_terminal_archive_record(
    record: &ResourceTerminalArchiveRecordV1,
) -> Result<(), ResourceError> {
    match (&record.key, &record.payload) {
        (
            ResourceTerminalRecordKeyV1::Outcome(key),
            ResourceTerminalArchivePayloadV1::Outcome(value),
        ) => {
            value.validate()?;
            let mut detached = value.clone();
            let digest = std::mem::take(&mut detached.semantic_digest);
            if key != &value.operation_key
                || record.operation_key != value.operation_key
                || record.quantity != value.quantity
                || record.remainder != value.remainder
                || record.exact_evidence != value.exact_evidence
                || record.semantic_digest != digest
                || record.terminal_sequence != value.sequence
                || digest != canonical_digest("canwu.resource.operation-outcome.v1", &detached)?
            {
                return Err(ResourceError::InvalidDefinition(
                    "archived resource operation outcome closure is forged".to_owned(),
                ));
            }
        }
        (
            ResourceTerminalRecordKeyV1::Demand(id),
            ResourceTerminalArchivePayloadV1::Demand(value),
        ) => {
            value.source_policy.validate_shape()?;
            let digest = canonical_digest("canwu.resource.demand.v1", value)?;
            if id != &value.id
                || record.operation_key
                    != ResourceOperationKey::new(format!("resource:demand-terminal:{digest}"))?
                || record.quantity != value.fulfilled
                || record.remainder != value.remainder()
                || !record.exact_evidence.is_empty()
                || record.semantic_digest != digest
                || matches!(
                    value.status,
                    crate::DemandStatus::Open | crate::DemandStatus::PartiallyFulfilled
                )
            {
                return Err(ResourceError::InvalidDefinition(
                    "archived resource demand closure is forged".to_owned(),
                ));
            }
        }
        (
            ResourceTerminalRecordKeyV1::Reservation(id),
            ResourceTerminalArchivePayloadV1::Reservation(value),
        ) => {
            let digest = canonical_digest("canwu.resource.reservation.v1", value)?;
            if id != &value.id
                || record.operation_key != value.operation_key
                || record.quantity != value.quantity
                || record.remainder != 0
                || !record.exact_evidence.is_empty()
                || record.semantic_digest != digest
                || value.status == crate::ReservationStatus::Active
            {
                return Err(ResourceError::InvalidDefinition(
                    "archived resource reservation closure is forged".to_owned(),
                ));
            }
        }
        (
            ResourceTerminalRecordKeyV1::AllocationLeg(id),
            ResourceTerminalArchivePayloadV1::AllocationLeg(value),
        ) => {
            let mut detached = value.clone();
            let digest = std::mem::take(&mut detached.semantic_digest);
            if id != &value.id
                || record.operation_key != value.operation_key
                || record.quantity != value.quantity
                || record.remainder != 0
                || !record.exact_evidence.is_empty()
                || record.semantic_digest != digest
                || value.status == crate::AllocationLegStatus::Reserved
                || digest != canonical_digest("canwu.resource.allocation-leg.v1", &detached)?
            {
                return Err(ResourceError::InvalidDefinition(
                    "archived resource allocation leg closure is forged".to_owned(),
                ));
            }
        }
        (
            ResourceTerminalRecordKeyV1::Consumption(id),
            ResourceTerminalArchivePayloadV1::Consumption(value),
        ) => {
            let mut detached = value.clone();
            let digest = std::mem::take(&mut detached.semantic_digest);
            if id != &value.id
                || record.operation_key != value.operation_key
                || record.quantity != value.quantity
                || record.remainder != 0
                || record.exact_evidence != [value.consumer_evidence.clone()]
                || record.semantic_digest != digest
                || record.terminal_sequence != value.terminal_sequence
                || value.status != ConsumptionStatus::Settled
                || digest != canonical_digest("canwu.resource.consumption.v1", &detached)?
            {
                return Err(ResourceError::InvalidDefinition(
                    "archived resource consumption closure is forged".to_owned(),
                ));
            }
        }
        (
            ResourceTerminalRecordKeyV1::Fulfillment(id),
            ResourceTerminalArchivePayloadV1::Fulfillment(value),
        ) => {
            let mut detached = value.clone();
            let digest = std::mem::take(&mut detached.semantic_digest);
            if id != &value.id
                || record.operation_key != value.operation_key
                || record.quantity != value.consumed_quantity
                || record.remainder != value.remainder
                || !record.exact_evidence.is_empty()
                || record.semantic_digest != digest
                || record.terminal_sequence != value.terminal_sequence
                || !matches!(
                    value.status,
                    FulfillmentStatus::Partial
                        | FulfillmentStatus::Complete
                        | FulfillmentStatus::RejectedMinimum
                        | FulfillmentStatus::Cancelled
                        | FulfillmentStatus::Expired
                )
                || digest != canonical_digest("canwu.resource.fulfillment.v1", &detached)?
            {
                return Err(ResourceError::InvalidDefinition(
                    "archived resource fulfillment closure is forged".to_owned(),
                ));
            }
        }
        (ResourceTerminalRecordKeyV1::Loss(id), ResourceTerminalArchivePayloadV1::Loss(value)) => {
            let evidence = match &value.cause {
                canwu_api::EvidenceRef::DomainRecordVersion(version) => vec![version.clone()],
                _ => Vec::new(),
            };
            let digest = canonical_digest("canwu.resource.loss.v1", value)?;
            if id != &value.id
                || value.quantity == 0
                || value.account.is_some() == value.transfer.is_some()
                || record.operation_key != value.operation_key
                || record.quantity != value.quantity
                || record.remainder != 0
                || record.exact_evidence != evidence
                || record.semantic_digest != digest
                || record.terminal_sequence != value.terminal_sequence
            {
                return Err(ResourceError::InvalidDefinition(
                    "archived resource loss closure is forged".to_owned(),
                ));
            }
        }
        (
            ResourceTerminalRecordKeyV1::Transfer(id),
            ResourceTerminalArchivePayloadV1::Transfer(value),
        ) => {
            let digest = canonical_digest("canwu.resource.transfer.v1", value)?;
            // A terminal transfer's quantity is what it accepted, lost
            // (partial losses included), returned, and let out, and its link
            // history is one its advances could record.
            let settled = u128::from(value.accepted)
                + u128::from(value.lost)
                + u128::from(value.returned)
                + u128::from(value.external_outflow);
            if id != &value.id
                || record.operation_key != value.operation_key
                || record.quantity != value.quantity
                || record.remainder != 0
                || record.exact_evidence != value.exact_evidence
                || record.semantic_digest != digest
                || record.terminal_sequence != value.terminal_sequence
                || !matches!(
                    value.state,
                    ResourceTransferState::Accepted
                        | ResourceTransferState::Lost
                        | ResourceTransferState::ExternalOutflowSettled
                        | ResourceTransferState::Cancelled
                        | ResourceTransferState::Returned
                )
                || value.escrow != 0
                || settled != u128::from(value.quantity)
                || !crate::transfer_link_history_is_valid(value)
            {
                return Err(ResourceError::InvalidDefinition(
                    "archived resource transfer closure is forged".to_owned(),
                ));
            }
        }
        (
            ResourceTerminalRecordKeyV1::ExternalCompletionParticipant(acquisition),
            ResourceTerminalArchivePayloadV1::ExternalCompletionParticipant(participant),
        ) => {
            let certificate = participant.certificate.as_ref().ok_or_else(|| {
                ResourceError::InvalidDefinition(
                    "archived external completion participant certificate is missing".to_owned(),
                )
            })?;
            let mut detached_certificate = certificate.clone();
            let certificate_digest = std::mem::take(&mut detached_certificate.semantic_digest);
            let mut targets = participant.grant.target_versions.clone();
            targets.sort();
            targets.dedup();
            let prepared_revision =
                certificate
                    .prepared_grants
                    .iter()
                    .find_map(|(grant, revision)| {
                        (grant == &participant.grant.id).then_some(*revision)
                    });
            if &participant.grant.acquisition != acquisition
                || participant.grant.operation_key != record.operation_key
                || participant.grant.state != crate::CompletionGrantStateV1::Completed
                || participant.grant.owner_plugin != crate::PLUGIN_NAME
                || participant.grant.rejection.is_some()
                || participant.coordinator_plugin.is_empty()
                || participant.operation_namespace.is_empty()
                || participant.eligibility_envelope_digest.is_empty()
                || participant.grant.target_versions.is_empty()
                || targets != participant.grant.target_versions
                || participant.recipe.validate().is_err()
                || participant.grant.recipe_digest != participant.recipe.digest()?
                || certificate.acquisition != *acquisition
                || certificate.operation_key != participant.grant.operation_key
                || certificate.recipe_digest != participant.grant.recipe_digest
                || certificate.eligibility_time != participant.eligibility_time
                || certificate.eligibility_envelope_digest
                    != participant.eligibility_envelope_digest
                || certificate_digest
                    != canonical_digest(
                        "canwu.resource.completion-activation-certificate.v1",
                        &detached_certificate,
                    )?
                || prepared_revision.is_none_or(|revision| {
                    participant.grant.revision.get() != revision.get().saturating_add(2)
                })
                || participant
                    .grant
                    .target_versions
                    .iter()
                    .any(|target| !certificate.locked_target_versions.contains(target))
                || record.quantity != participant.grant.reserved_units
                || record.remainder != 0
                || record.exact_evidence != [participant.coordinator_source.clone()]
                || record.semantic_digest
                    != canonical_digest(
                        "canwu.resource.external-completion-participant.v1",
                        participant,
                    )?
            {
                return Err(ResourceError::InvalidDefinition(
                    "archived external completion participant closure is forged".to_owned(),
                ));
            }
        }
        (
            ResourceTerminalRecordKeyV1::LeaseReceipt(sequence),
            ResourceTerminalArchivePayloadV1::LeaseReceipt(value),
        ) => {
            let mut detached = value.clone();
            let digest = std::mem::take(&mut detached.semantic_digest);
            if sequence != &value.sequence
                || record.operation_key != value.operation_key
                || record.quantity != value.reserved_units
                || record.remainder != 0
                || !record.exact_evidence.is_empty()
                || record.semantic_digest != digest
                || record.terminal_sequence == 0
                || digest
                    != canonical_digest("canwu.resource.completion-lease-receipt.v1", &detached)?
            {
                return Err(ResourceError::InvalidDefinition(
                    "archived resource completion receipt closure is forged".to_owned(),
                ));
            }
        }
        (
            ResourceTerminalRecordKeyV1::ArchiveMaintenanceReceipt(sequence),
            ResourceTerminalArchivePayloadV1::ArchiveMaintenanceReceipt(value),
        ) => {
            let mut detached = value.clone();
            let digest = std::mem::take(&mut detached.semantic_digest);
            if sequence != &value.sequence
                || record.operation_key
                    != ResourceOperationKey::new(format!(
                        "resource:archive-maintenance:{sequence}"
                    ))?
                || record.quantity != u64::from(value.archived_records)
                || record.remainder != 0
                || !record.exact_evidence.is_empty()
                || record.semantic_digest != digest
                || record.terminal_sequence == 0
                || digest
                    != canonical_digest("canwu.resource.archive-maintenance-receipt.v1", &detached)?
            {
                return Err(ResourceError::InvalidDefinition(
                    "archived resource archive-maintenance receipt closure is forged".to_owned(),
                ));
            }
        }
        _ => {
            return Err(ResourceError::InvalidDefinition(
                "resource archive key and typed terminal payload differ".to_owned(),
            ));
        }
    }
    Ok(())
}

/// Returns the id of every lookup page a live run reads: the pages the
/// index of the archive head reaches, and the pages the index of each queued
/// archive commit reaches.
///
/// `queued_directory_roots` names the directory of every queued archive
/// commit. A commit is queued when its store-side retention handle is in
/// [`ResourceArchiveRetentionPhaseV1::DurableIngress`] and
/// `archive_retention_handles` of this root holds no handle with its id: it
/// was enqueued and its boundary has not run. A store-side handle that is
/// still `DurableIngress` while the root holds a handle with its id belongs
/// to a settled commit that was never finalized; it is not queued, and its
/// index may be incomplete. A queued commit may become the head, so its pages
/// are live. A queued commit the caller does not name is not covered. Every
/// page of a named commit must be present: a missing one is an error.
///
/// A host application that lost the `superseded_lookup_pages` of a batch
/// uses this to recompute what is live. A lookup page the store holds that
/// is not in the result is read by no live index; whether it may be deleted
/// is still decided by the retention rule of
/// [`PreparedResourceArchiveBatchV1::superseded_lookup_pages`].
pub fn live_resource_archive_lookup_pages<'a>(
    state: &ResourceState,
    store: &dyn ResourceArchiveStore,
    queued_directory_roots: impl IntoIterator<Item = &'a str>,
) -> Result<BTreeSet<String>, ResourceError> {
    let mut pages = BTreeSet::new();
    let mut collect = |root: &str| {
        let directory = load_resource_archive_directory(store, root)?;
        collect_resource_archive_lookup_pages(store, &directory.lookup_root, "", true, &mut pages)
    };
    if let Some(root) = &state.archive_head.directory_root {
        collect(root)?;
    }
    for root in queued_directory_roots {
        collect(root)?;
    }
    Ok(pages)
}

/// Authenticates every archive object reachable from the persisted resource
/// head and every in-flight retention handle. Restore callers must provide the
/// same durable store that owns those objects; a snapshot alone is not enough
/// to trust archive-backed state.
///
/// The directory chain is walked to its first commit and each commit's blobs,
/// records, and pages are authenticated. The cumulative lookup index of the
/// head is then rebuilt from the chain's records (a key archived more than
/// once counts by its newest record) and must be exactly what the store
/// holds: every page present and equal, and the same root. The index of a
/// commit that is queued and not yet applied is not audited; if one of its
/// pages is missing once it is the head, a lookup that needs the page fails
/// its boundary and is never taken for a miss. The cost is proportional to
/// the archive, and the audit holds about 40 bytes per archived record while
/// it runs, so this is a restore and audit step, never part of a boundary.
pub fn validate_resource_archive_store(
    state: &ResourceState,
    store: &dyn ResourceArchiveStore,
) -> Result<(), ResourceError> {
    let mut next = state.archive_head.directory_root.clone();
    let mut expected_count = state.archive_head.archived_record_count;
    let mut seen = BTreeSet::new();
    let mut directory_count = 0_u64;
    let mut lookup_root: Option<String> = None;
    let mut blob_ids = Vec::new();
    let mut archived = Vec::new();
    while let Some(root) = next {
        if !seen.insert(root.clone()) {
            return Err(ResourceError::InvalidDefinition(
                "resource archive directory chain is cyclic".to_owned(),
            ));
        }
        directory_count = directory_count
            .checked_add(1)
            .ok_or(ResourceError::Overflow)?;
        if directory_count > state.archive_head.revision {
            return Err(ResourceError::LimitExceeded(
                "resource archive directory chain exceeds the committed head revision".to_owned(),
            ));
        }
        let directory: ResourceArchiveIndexDirectoryV1 =
            load_encoded(store, RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, &root)?.ok_or_else(
                || ResourceError::NotFound("resource archive head directory is missing".to_owned()),
            )?;
        let records = authenticate_resource_archive_directory_closure(store, &directory)?;
        if directory.id != root || directory.archived_record_count != expected_count {
            return Err(ResourceError::InvalidDefinition(
                "resource archive directory chain count or identity differs from its head"
                    .to_owned(),
            ));
        }
        lookup_root.get_or_insert_with(|| directory.lookup_root.clone());
        // Newest commit first, so the first entry kept for a key is the one
        // the index must name.
        let first_blob = blob_ids.len();
        blob_ids.extend(directory.blob_ids.iter().cloned());
        for (key, blob_id, ordinal) in records {
            let blob = directory
                .blob_ids
                .iter()
                .position(|candidate| candidate == &blob_id)
                .and_then(|offset| first_blob.checked_add(offset))
                .and_then(|index| u32::try_from(index).ok())
                .ok_or(ResourceError::Overflow)?;
            let key_digest = blake3::Hash::from_hex(resource_archive_lookup_key_digest(&key)?)
                .map_err(|error| ResourceError::InvalidDefinition(error.to_string()))?;
            archived.push(ResourceArchiveAuditEntry {
                key_digest: *key_digest.as_bytes(),
                blob,
                ordinal,
            });
        }
        next = directory.previous_root.clone();
        if let Some(previous) = &next {
            let prior: ResourceArchiveIndexDirectoryV1 =
                load_encoded(store, RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, previous)?
                    .ok_or_else(|| {
                        ResourceError::NotFound(
                            "resource archive previous directory is missing".to_owned(),
                        )
                    })?;
            if prior.archived_record_count >= directory.archived_record_count {
                return Err(ResourceError::InvalidDefinition(
                    "resource archive directory chain does not advance its record count".to_owned(),
                ));
            }
            expected_count = prior.archived_record_count;
        }
    }
    if directory_count != state.archive_head.revision
        || (state.archive_head.revision == 0) != (state.archive_head.directory_root.is_none())
    {
        return Err(ResourceError::InvalidDefinition(
            "resource archive head revision differs from its durable directory chain".to_owned(),
        ));
    }
    if let Some(lookup_root) = &lookup_root {
        archived.sort_by_key(|entry| entry.key_digest);
        archived.dedup_by(|later, earlier| later.key_digest == earlier.key_digest);
        let rebuilt =
            audit_resource_archive_lookup_page(store, &blob_ids, &mut String::new(), &archived)?;
        if &rebuilt != lookup_root {
            return Err(ResourceError::InvalidDefinition(
                "resource archive lookup index differs from its directory chain".to_owned(),
            ));
        }
    }
    for handle in state.archive_retention_handles.values() {
        for (namespace, ids) in &handle.object_ids {
            for id in ids {
                if store.load_resource_archive_object(namespace, id)?.is_none() {
                    return Err(ResourceError::NotFound(
                        "resource archive retention object is missing".to_owned(),
                    ));
                }
            }
        }
        let directory: ResourceArchiveIndexDirectoryV1 = load_encoded(
            store,
            RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            &handle.directory_root,
        )?
        .ok_or_else(|| {
            ResourceError::NotFound("resource archive retention directory is missing".to_owned())
        })?;
        authenticate_resource_archive_directory(store, &directory)?;
    }
    Ok(())
}

/// Authenticates one historical directory as an ancestor of the current
/// authoritative resource archive head.
pub fn authenticate_reachable_resource_archive_directory(
    state: &ResourceState,
    store: &dyn ResourceArchiveStore,
    directory_root: &str,
    archived_record_count: u64,
) -> Result<(), ResourceError> {
    validate_resource_archive_store(state, store)?;
    let mut next = state.archive_head.directory_root.clone();
    while let Some(root) = next {
        let directory: ResourceArchiveIndexDirectoryV1 =
            load_encoded(store, RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, &root)?.ok_or_else(
                || ResourceError::NotFound("resource archive directory is missing".to_owned()),
            )?;
        if root == directory_root {
            if directory.archived_record_count != archived_record_count {
                return Err(ResourceError::InvalidDefinition(
                    "resource archive witness count differs from its reachable directory"
                        .to_owned(),
                ));
            }
            return Ok(());
        }
        next = directory.previous_root;
    }
    Err(ResourceError::InvalidDefinition(
        "resource archive witness directory is not reachable from the current head".to_owned(),
    ))
}

/// Resolves one exact cold terminal record through the cumulative lookup
/// index of the archive head.
///
/// The lookup reads the head directory and one lookup page per index level,
/// each authenticated by its content id, and nothing else when the key was
/// never archived. Its cost follows the depth of the index, which grows with
/// the logarithm of the archived records and not at all with the number of
/// archive commits. A hit also reads the one blob that holds the record,
/// authenticates that blob, and validates the record's typed closure. A key
/// archived more than once resolves to its newest record.
///
/// The number of objects read is bounded whatever the store returns: one
/// directory, at most [`MAX_RESOURCE_ARCHIVE_LOOKUP_DEPTH`]` + 1` lookup
/// pages, and one blob. A lookup page is also bounded in bytes
/// ([`MAX_RESOURCE_ARCHIVE_LOOKUP_PAGE_BYTES`]); the directory and the blob
/// are not, as before. A missing, forged, oversized, or malformed object is
/// an error, never a miss. Every error means the archive could not be read;
/// none is a property of persisted state, so a boundary system fails its
/// boundary on one and never records it as a durable rejection.
///
/// The lookup trusts the head directory to name an index that holds every
/// archived key. [`crate::enqueue_resource_archive`] establishes that for
/// each commit it queues, and [`validate_resource_archive_store`] checks it
/// for the head of a restored root. A root assembled by hand, or restored
/// without the checked wrappers, is not checked.
pub fn archived_resource_terminal_record(
    state: &ResourceState,
    store: &dyn ResourceArchiveStore,
    key: &ResourceTerminalRecordKeyV1,
) -> Result<Option<ResourceTerminalArchiveRecordV1>, ResourceError> {
    let Some(root) = &state.archive_head.directory_root else {
        return Ok(None);
    };
    let directory = load_resource_archive_directory(store, root)?;
    let key_digest = resource_archive_lookup_key_digest(key)?;
    let Some(entry) =
        find_resource_archive_lookup_entry(store, &directory.lookup_root, &key_digest)?
    else {
        return Ok(None);
    };
    let blob: ResourceArchiveBlobV1 =
        load_encoded(store, RESOURCE_ARCHIVE_BLOB_NAMESPACE, &entry.blob_id)?.ok_or_else(|| {
            ResourceError::NotFound("resource archive lookup blob is missing".to_owned())
        })?;
    let mut detached = blob.clone();
    detached.content_id.clear();
    if blob.content_id != entry.blob_id
        || blob.content_id != canonical_digest("canwu.resource.archive-blob.v1", &detached)?
        || blob.format_version != 1
        || blob.records.len() > MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES
    {
        return Err(ResourceError::InvalidDefinition(
            "resource archive lookup blob is forged or unbounded".to_owned(),
        ));
    }
    let record = blob
        .records
        .into_iter()
        .nth(usize::from(entry.ordinal))
        .ok_or_else(|| {
            ResourceError::InvalidDefinition(
                "resource archive lookup ordinal is outside its blob".to_owned(),
            )
        })?;
    if &record.key != key {
        return Err(ResourceError::InvalidDefinition(
            "resource archive lookup entry differs from its typed record".to_owned(),
        ));
    }
    validate_resource_terminal_archive_record(&record)?;
    Ok(Some(record))
}

/// Resolves the archived outcome of one operation key, if that key was ever
/// archived. See [`archived_resource_terminal_record`] for the read bound.
pub fn archived_resource_operation_outcome(
    state: &ResourceState,
    store: &dyn ResourceArchiveStore,
    operation_key: &ResourceOperationKey,
) -> Result<Option<crate::ResourceOperationOutcome>, ResourceError> {
    archived_resource_terminal_record(
        state,
        store,
        &ResourceTerminalRecordKeyV1::Outcome(operation_key.clone()),
    )?
    .map(|record| match record.payload {
        ResourceTerminalArchivePayloadV1::Outcome(outcome) => Ok(outcome),
        _ => Err(ResourceError::InvalidDefinition(
            "resource archived operation membership has the wrong typed payload".to_owned(),
        )),
    })
    .transpose()
}

fn store_encoded<T: Serialize>(
    store: &dyn ResourceArchiveStore,
    namespace: &str,
    id: &str,
    value: &T,
) -> Result<(), ResourceError> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        ResourceError::InvalidDefinition(format!("resource archive encoding failed: {error}"))
    })?;
    store.store_resource_archive_object(namespace, id, &bytes)
}

fn load_encoded<T: serde::de::DeserializeOwned>(
    store: &dyn ResourceArchiveStore,
    namespace: &str,
    id: &str,
) -> Result<Option<T>, ResourceError> {
    store
        .load_resource_archive_object(namespace, id)?
        .map(|bytes| {
            serde_json::from_slice(&bytes).map_err(|error| {
                ResourceError::InvalidDefinition(format!(
                    "resource archive object decoding failed: {error}"
                ))
            })
        })
        .transpose()
}
