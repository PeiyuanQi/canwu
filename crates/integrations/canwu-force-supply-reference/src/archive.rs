//! The package archive: a content-addressed cold store for the terminal
//! records of a reference package, shared by the force-supply and economy
//! reference plugins.
//!
//! Each archive commit is a directory that names one blob of records, its
//! membership and temporal pages, the commit before it, and the root of a
//! cumulative lookup index over every record archived up to and including
//! that commit. The index has the format of the resource archive's
//! ([`ResourceArchiveLookupPageV1`], a 16-way trie over key digests) and is
//! stored under the domain's own namespace, so a lookup reads the head
//! directory, one index page per level, and one blob on a hit, however many
//! commits the archive holds.

use canwu_api::{ArchiveReachabilityManifest, CanwuError, ErrorCode, PluginArchiveObjectProvider};
use canwu_resource::{
    RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE, ResourceArchiveAuditEntry,
    ResourceArchiveLookupEntryV1, ResourceArchiveLookupPageV1, ResourceArchiveRetentionHandleV1,
    ResourceArchiveRetentionPhaseV1, ResourceArchiveStore, ResourceError,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::{BTreeMap, BTreeSet};

pub const FORCE_ARCHIVE_BLOB_NAMESPACE: &str = "canwu.force-supply-reference.archive.blob";
pub const FORCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE: &str =
    "canwu.force-supply-reference.archive.membership-page";
pub const FORCE_ARCHIVE_TEMPORAL_PAGE_NAMESPACE: &str =
    "canwu.force-supply-reference.archive.temporal-page";
pub const FORCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE: &str =
    "canwu.force-supply-reference.archive.index-directory";
/// Namespace of the pages of the force-supply archive's cumulative lookup
/// index.
pub const FORCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE: &str =
    "canwu.force-supply-reference.archive.lookup-page";
pub const MAX_PACKAGE_ARCHIVE_PAGE_ENTRIES: usize = 512;
/// Maintenance receipts one plugin root may hold. Every settled archive
/// commit, applied or stale, leaves one, and receipts never leave the root,
/// so this bounds the archive commits of a run: a root that would hold more
/// does not validate, and the boundary that settles the commit fails. The
/// enqueue helpers refuse a commit that could reach it (see
/// [`package_archive_commit_room`]). Folding acknowledged receipts into a
/// digest chain would lift the bound; that is a follow-up.
pub const MAX_PACKAGE_ARCHIVE_MAINTENANCE_RECEIPTS: usize = 8_192;
/// Retention handles one plugin root may hold until the host acknowledges
/// their commits. A settled commit adds one; a root that would hold more
/// does not validate.
pub const MAX_PACKAGE_ARCHIVE_RETENTION_HANDLES: usize = 64;
/// Encoded size above which a package archive directory is refused before it
/// is decoded. A directory holds a fixed set of ids and counts, about 600
/// bytes; the bound leaves room for a store that returns the same JSON with
/// other whitespace.
pub const MAX_PACKAGE_ARCHIVE_DIRECTORY_BYTES: usize = 4 * 1024;
/// Encoded size above which a package archive blob is refused before it is
/// decoded: 512 records of up to 128 KiB each.
pub const MAX_PACKAGE_ARCHIVE_BLOB_BYTES: usize = 64 * 1024 * 1024;

/// The digest domain and object namespaces of one package archive.
#[derive(Clone, Copy)]
pub struct PackageArchiveDomainV1 {
    pub digest_prefix: &'static str,
    pub blob_namespace: &'static str,
    pub membership_namespace: &'static str,
    pub temporal_namespace: &'static str,
    pub directory_namespace: &'static str,
    /// Namespace of the pages of the archive's cumulative lookup index
    /// ([`ResourceArchiveLookupPageV1`]).
    pub lookup_namespace: &'static str,
}

pub const FORCE_ARCHIVE_DOMAIN: PackageArchiveDomainV1 = PackageArchiveDomainV1 {
    digest_prefix: "canwu.force-supply-reference",
    blob_namespace: FORCE_ARCHIVE_BLOB_NAMESPACE,
    membership_namespace: FORCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE,
    temporal_namespace: FORCE_ARCHIVE_TEMPORAL_PAGE_NAMESPACE,
    directory_namespace: FORCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
    lookup_namespace: FORCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackageArchiveRecordV1<K, P> {
    pub key: K,
    pub terminal_sequence: u64,
    pub payload: P,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackageArchiveBlobV1<K, P> {
    pub format_version: u32,
    pub expected_source_root: String,
    pub records: Vec<PackageArchiveRecordV1<K, P>>,
    pub content_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackageArchiveMembershipV1<K> {
    pub key: K,
    pub blob_id: String,
    pub ordinal: u16,
    pub terminal_sequence: u64,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackageArchiveMembershipPageV1<K> {
    pub id: String,
    pub memberships: Vec<PackageArchiveMembershipV1<K>>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackageArchiveTemporalEntryV1<K> {
    pub terminal_sequence: u64,
    pub key: K,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackageArchiveTemporalPageV1<K> {
    pub id: String,
    pub entries: Vec<PackageArchiveTemporalEntryV1<K>>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackageArchiveHeadStateV1 {
    pub revision: u64,
    pub directory_root: Option<String>,
    pub archived_record_count: u64,
    pub semantic_digest: String,
}

/// One archive commit: its blob and pages, the commit before it, and the
/// root of the cumulative lookup index.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackageArchiveIndexDirectoryV1 {
    pub id: String,
    pub previous_root: Option<String>,
    pub membership_page: String,
    pub temporal_page: String,
    pub blob_id: String,
    pub archived_record_count: u64,
    /// Root [`ResourceArchiveLookupPageV1`] of the index over every record
    /// archived up to and including this commit, stored under the domain's
    /// [`PackageArchiveDomainV1::lookup_namespace`]. A lookup starts here, so
    /// its cost does not depend on how many commits came before.
    pub lookup_root: String,
    pub semantic_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageArchiveRetentionPhaseV1 {
    Prepared,
    Verified,
    DurableIngress,
    Committed,
    RejectedStale,
    Abandoned,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackageArchiveRetentionHandleV1 {
    pub id: String,
    pub phase: PackageArchiveRetentionPhaseV1,
    pub expected_source_root: String,
    pub directory_root: String,
    pub object_ids: BTreeMap<String, BTreeSet<String>>,
    pub semantic_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageArchiveMaintenanceDispositionV1 {
    Applied,
    RejectedStale,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackageArchiveMaintenanceReceiptV1 {
    pub sequence: u64,
    pub retention_handle_id: String,
    pub expected_source_root: String,
    pub directory_root: String,
    pub disposition: PackageArchiveMaintenanceDispositionV1,
    pub archived_records: u32,
    pub semantic_digest: String,
}

/// One archive commit ready to store: its records and pages, the lookup
/// index pages it adds, and its retention handle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreparedPackageArchiveBatchV1<K, P> {
    pub expected_source_root: String,
    pub selected: Vec<K>,
    pub blob: PackageArchiveBlobV1<K, P>,
    pub membership_page: PackageArchiveMembershipPageV1<K>,
    pub temporal_page: PackageArchiveTemporalPageV1<K>,
    pub directory: PackageArchiveIndexDirectoryV1,
    pub retention: PackageArchiveRetentionHandleV1,
    /// The lookup index pages this commit adds, by id: the pages on the
    /// paths of its keys. Every other page of the index is shared with the
    /// previous commit's. The retention handle does not list them; they are
    /// reached through the directory.
    pub lookup_pages: BTreeMap<String, ResourceArchiveLookupPageV1>,
    /// Pages of the previous commit's index that this commit's index no
    /// longer reaches. They may be deleted only after the commit settled as
    /// applied and a durable snapshot or checkpoint taken after it exists,
    /// and deleting them gives up every earlier point of the run (snapshot,
    /// checkpoint, journal, or fork) whose archive head reads one of them.
    /// Consecutive heads share pages, so that reaches back to the commit that
    /// wrote each page. Otherwise objects are deleted by reachability only.
    pub superseded_lookup_pages: BTreeSet<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VerifiedPackageArchiveCommitV1<K> {
    pub expected_source_root: String,
    pub selected: Vec<K>,
    pub directory_root: String,
    pub retention: PackageArchiveRetentionHandleV1,
    pub archived_records: u32,
}

pub trait PackageArchiveStore {
    fn store_package_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), CanwuError>;
    fn load_package_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError>;
    fn persist_package_archive_retention(
        &self,
        handle: &PackageArchiveRetentionHandleV1,
    ) -> Result<(), CanwuError>;
    fn load_package_archive_retention(
        &self,
        handle_id: &str,
    ) -> Result<Option<PackageArchiveRetentionHandleV1>, CanwuError>;
    fn finalize_package_archive_retention(
        &self,
        handle: &PackageArchiveRetentionHandleV1,
    ) -> Result<(), CanwuError>;
}

/// The absent store: it holds no object and accepts none. It is enough to
/// prepare the first archive commit of a root, which reads nothing, and it
/// fails every read of a root that already has an archive head.
impl PackageArchiveStore for () {
    fn store_package_archive_object(&self, _: &str, _: &str, _: &[u8]) -> Result<(), CanwuError> {
        Err(invalid_archive(
            "the absent package archive store accepts no object",
        ))
    }

    fn load_package_archive_object(&self, _: &str, _: &str) -> Result<Option<Vec<u8>>, CanwuError> {
        Ok(None)
    }

    fn persist_package_archive_retention(
        &self,
        _: &PackageArchiveRetentionHandleV1,
    ) -> Result<(), CanwuError> {
        Err(invalid_archive(
            "the absent package archive store keeps no retention",
        ))
    }

    fn load_package_archive_retention(
        &self,
        _: &str,
    ) -> Result<Option<PackageArchiveRetentionHandleV1>, CanwuError> {
        Ok(None)
    }

    fn finalize_package_archive_retention(
        &self,
        _: &PackageArchiveRetentionHandleV1,
    ) -> Result<(), CanwuError> {
        Err(invalid_archive(
            "the absent package archive store keeps no retention",
        ))
    }
}

/// Digest under which the lookup index of `domain` files one record key: a
/// domain-separated BLAKE3 hash whose hexadecimal nibbles are the key's path
/// through the index.
pub fn package_archive_lookup_key_digest<K: Serialize>(
    domain: PackageArchiveDomainV1,
    key: &K,
) -> Result<String, CanwuError> {
    digest(domain, "archive-lookup-key.v1", key)
}

/// A package archive store seen as a store of lookup-index pages, so that
/// the lookup-index functions of `canwu-resource` read the package index. It
/// serves the resource lookup-page namespace from the domain's lookup
/// namespace, refuses every other namespace, and writes nothing.
struct PackageLookupPages<'a> {
    store: &'a dyn PackageArchiveStore,
    namespace: &'static str,
}

impl<'a> PackageLookupPages<'a> {
    fn new(domain: PackageArchiveDomainV1, store: &'a dyn PackageArchiveStore) -> Self {
        Self {
            store,
            namespace: domain.lookup_namespace,
        }
    }
}

impl ResourceArchiveStore for PackageLookupPages<'_> {
    fn store_resource_archive_object(
        &self,
        _namespace: &str,
        _object_id: &str,
        _bytes: &[u8],
    ) -> Result<(), ResourceError> {
        Err(ResourceError::InvalidLifecycle(
            "the package lookup-page view is read-only".to_owned(),
        ))
    }

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, ResourceError> {
        if namespace != RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE {
            return Err(ResourceError::InvalidDefinition(format!(
                "the package lookup-page view serves no {namespace} object"
            )));
        }
        self.store
            .load_package_archive_object(self.namespace, object_id)
            .map_err(|error| {
                ResourceError::InvalidDefinition(format!("package archive store failed: {error}"))
            })
    }

    fn persist_resource_archive_retention(
        &self,
        _handle: &ResourceArchiveRetentionHandleV1,
    ) -> Result<(), ResourceError> {
        Err(ResourceError::InvalidLifecycle(
            "the package lookup-page view keeps no retention".to_owned(),
        ))
    }

    fn finalize_resource_archive_retention(
        &self,
        _handle_id: &str,
        _phase: ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError> {
        Err(ResourceError::InvalidLifecycle(
            "the package lookup-page view keeps no retention".to_owned(),
        ))
    }
}

fn lookup_error(error: &ResourceError) -> CanwuError {
    invalid_archive(format!("package archive lookup index: {error}"))
}

/// Refuses, on the host, an archive commit that could take a plugin root
/// past [`MAX_PACKAGE_ARCHIVE_MAINTENANCE_RECEIPTS`] or
/// [`MAX_PACKAGE_ARCHIVE_RETENTION_HANDLES`] when it settles.
///
/// Every archive commit that settles, applied or stale, adds a maintenance
/// receipt and may add a retention handle; a root above either bound does
/// not validate, so the boundary that settles the commit would fail and could
/// not be stepped past. The check adds to the root's `receipts` and `handles`
/// every commit of `packet_type` still queued for `plugin` in the run, and
/// this commit, and refuses when either sum passes its bound. It counts no
/// queued acknowledgement as freeing a handle, and a stale duplicate as
/// adding one, so it is conservative. It reads the run's ingress queue, not
/// the archive store, and runs on the host, never inside a boundary.
pub fn package_archive_commit_room<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    plugin: &str,
    packet_type: &str,
    receipts: usize,
    handles: usize,
) -> Result<(), CanwuError> {
    let window =
        canwu.boundaries_between(canwu.time(), canwu_api::SimTime::from_minutes(i64::MAX))?;
    let queued = window
        .queued
        .iter()
        .flat_map(|boundary| &boundary.ingress)
        .filter(|count| {
            matches!(
                &count.kind,
                canwu_api::BoundaryIngressKind::Plugin { plugin: owner, packet_type: kind }
                    if owner == plugin && kind == packet_type
            )
        })
        .fold(0_u64, |total, count| total.saturating_add(count.count));
    let settling = usize::try_from(queued)
        .unwrap_or(usize::MAX)
        .saturating_add(1);
    if receipts.saturating_add(settling) > MAX_PACKAGE_ARCHIVE_MAINTENANCE_RECEIPTS
        || handles.saturating_add(settling) > MAX_PACKAGE_ARCHIVE_RETENTION_HANDLES
    {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            "archive_backpressure: the root's maintenance receipts or retention handles, with the commits already queued, leave no room for another archive commit",
        ));
    }
    Ok(())
}

pub fn sealed_archive_retention(
    domain: PackageArchiveDomainV1,
    mut handle: PackageArchiveRetentionHandleV1,
) -> Result<PackageArchiveRetentionHandleV1, CanwuError> {
    handle.semantic_digest.clear();
    handle.semantic_digest = digest(domain, "archive-retention.v1", &handle)?;
    Ok(handle)
}

pub fn sealed_archive_head(
    domain: PackageArchiveDomainV1,
    mut head: PackageArchiveHeadStateV1,
) -> Result<PackageArchiveHeadStateV1, CanwuError> {
    head.semantic_digest.clear();
    head.semantic_digest = digest(domain, "archive-head.v1", &head)?;
    Ok(head)
}

/// Prepares one archive commit of `records` on top of `head`.
///
/// Besides the blob, its pages, and the directory, the commit extends the
/// cumulative lookup index of the head with its keys. That reads, from
/// `store`, the head's directory and the index pages on the paths of the new
/// keys, and prepares their replacements; every other page is shared with
/// the head's index. The first commit of an archive reads nothing, so `()`,
/// the absent store, is enough for it. The result depends only on the head,
/// the records, and the source root, so preparing it again from the same
/// state and store gives an equal batch.
#[allow(clippy::too_many_lines)]
pub fn prepare_package_archive<K, P>(
    domain: PackageArchiveDomainV1,
    expected_source_root: String,
    head: &PackageArchiveHeadStateV1,
    records: Vec<PackageArchiveRecordV1<K, P>>,
    store: &dyn PackageArchiveStore,
) -> Result<PreparedPackageArchiveBatchV1<K, P>, CanwuError>
where
    K: Clone + Eq + Ord + Serialize,
    P: Clone + PartialEq + Serialize,
{
    if records.is_empty() || records.len() > MAX_PACKAGE_ARCHIVE_PAGE_ENTRIES {
        return Err(invalid_archive(
            "package archive candidate budget is empty or excessive",
        ));
    }
    let selected = records
        .iter()
        .map(|record| record.key.clone())
        .collect::<Vec<_>>();
    if selected.iter().collect::<BTreeSet<_>>().len() != selected.len() {
        return Err(invalid_archive("package archive keys are duplicated"));
    }
    let mut blob = PackageArchiveBlobV1 {
        format_version: 1,
        expected_source_root: expected_source_root.clone(),
        records,
        content_id: String::new(),
    };
    blob.content_id = digest(domain, "archive-blob.v1", &blob)?;
    let memberships = blob
        .records
        .iter()
        .enumerate()
        .map(|(ordinal, record)| {
            Ok(PackageArchiveMembershipV1 {
                key: record.key.clone(),
                blob_id: blob.content_id.clone(),
                ordinal: u16::try_from(ordinal)
                    .map_err(|_| invalid_archive("archive ordinal overflow"))?,
                terminal_sequence: record.terminal_sequence,
                semantic_digest: record.semantic_digest.clone(),
            })
        })
        .collect::<Result<Vec<_>, CanwuError>>()?;
    let mut membership_page = PackageArchiveMembershipPageV1 {
        id: String::new(),
        memberships,
        semantic_digest: String::new(),
    };
    membership_page.semantic_digest =
        digest(domain, "archive-membership-page.v1", &membership_page)?;
    membership_page
        .id
        .clone_from(&membership_page.semantic_digest);
    let mut temporal_page = PackageArchiveTemporalPageV1 {
        id: String::new(),
        entries: blob
            .records
            .iter()
            .map(|record| PackageArchiveTemporalEntryV1 {
                terminal_sequence: record.terminal_sequence,
                key: record.key.clone(),
            })
            .collect(),
        semantic_digest: String::new(),
    };
    temporal_page
        .entries
        .sort_by_key(|entry| entry.terminal_sequence);
    temporal_page.semantic_digest = digest(domain, "archive-temporal-page.v1", &temporal_page)?;
    temporal_page.id.clone_from(&temporal_page.semantic_digest);
    let previous_lookup_root = match &head.directory_root {
        Some(root) => Some(load_package_archive_directory(domain, store, root)?.lookup_root),
        None => None,
    };
    let lookup = canwu_resource::prepare_resource_archive_lookup_entries(
        &PackageLookupPages::new(domain, store),
        previous_lookup_root.as_deref(),
        lookup_entries(domain, &membership_page)?,
    )
    .map_err(|error| lookup_error(&error))?;
    let mut directory = PackageArchiveIndexDirectoryV1 {
        id: String::new(),
        previous_root: head.directory_root.clone(),
        membership_page: membership_page.id.clone(),
        temporal_page: temporal_page.id.clone(),
        blob_id: blob.content_id.clone(),
        archived_record_count: head
            .archived_record_count
            .checked_add(
                u64::try_from(selected.len())
                    .map_err(|_| invalid_archive("archive count overflow"))?,
            )
            .ok_or_else(|| invalid_archive("archive count overflow"))?,
        lookup_root: lookup.root,
        semantic_digest: String::new(),
    };
    directory.semantic_digest = digest(domain, "archive-directory.v1", &directory)?;
    directory.id = directory.semantic_digest.clone();
    let mut object_ids = BTreeMap::new();
    object_ids.insert(
        domain.blob_namespace.to_owned(),
        BTreeSet::from([blob.content_id.clone()]),
    );
    object_ids.insert(
        domain.membership_namespace.to_owned(),
        BTreeSet::from([membership_page.id.clone()]),
    );
    object_ids.insert(
        domain.temporal_namespace.to_owned(),
        BTreeSet::from([temporal_page.id.clone()]),
    );
    object_ids.insert(
        domain.directory_namespace.to_owned(),
        BTreeSet::from([directory.id.clone()]),
    );
    let mut retention = PackageArchiveRetentionHandleV1 {
        id: digest(
            domain,
            "archive-retention-id.v1",
            &(&expected_source_root, &directory.id),
        )?,
        phase: PackageArchiveRetentionPhaseV1::Prepared,
        expected_source_root: expected_source_root.clone(),
        directory_root: directory.id.clone(),
        object_ids,
        semantic_digest: String::new(),
    };
    retention.semantic_digest = digest(domain, "archive-retention.v1", &retention)?;
    Ok(PreparedPackageArchiveBatchV1 {
        expected_source_root,
        selected,
        blob,
        membership_page,
        temporal_page,
        directory,
        retention,
        lookup_pages: lookup.pages,
        superseded_lookup_pages: lookup.superseded,
    })
}

/// The lookup-index entries of one commit's memberships, ordered by key
/// digest as the index functions require.
fn lookup_entries<K: Serialize>(
    domain: PackageArchiveDomainV1,
    membership_page: &PackageArchiveMembershipPageV1<K>,
) -> Result<Vec<ResourceArchiveLookupEntryV1>, CanwuError> {
    let mut entries = membership_page
        .memberships
        .iter()
        .map(|membership| {
            Ok(ResourceArchiveLookupEntryV1 {
                key_digest: package_archive_lookup_key_digest(domain, &membership.key)?,
                blob_id: membership.blob_id.clone(),
                ordinal: membership.ordinal,
            })
        })
        .collect::<Result<Vec<_>, CanwuError>>()?;
    entries.sort();
    Ok(entries)
}

impl<K, P> PreparedPackageArchiveBatchV1<K, P>
where
    K: Clone + Eq + Ord + Serialize + DeserializeOwned,
    P: Clone + PartialEq + Serialize + DeserializeOwned,
{
    /// Stores the commit's objects and lookup pages, reads them back, and
    /// checks the commit: the directory and its blob and pages authenticate,
    /// and every record of the commit resolves through the stored lookup
    /// index to its own blob position.
    pub fn store_and_verify(
        &self,
        domain: PackageArchiveDomainV1,
        store: &dyn PackageArchiveStore,
    ) -> Result<VerifiedPackageArchiveCommitV1<K>, CanwuError> {
        if !self.lookup_pages.contains_key(&self.directory.lookup_root) {
            return Err(invalid_archive(
                "prepared package archive batch lacks its lookup root page",
            ));
        }
        for (id, page) in &self.lookup_pages {
            if &page.id().map_err(|error| lookup_error(&error))? != id {
                return Err(invalid_archive(
                    "prepared package archive lookup page is forged",
                ));
            }
        }
        // A lookup refuses a blob above its size bound, so a commit whose
        // blob exceeds it is refused before anything is stored.
        let blob_bytes =
            serde_json::to_vec(&self.blob).map_err(|e| invalid_archive(e.to_string()))?;
        if blob_bytes.len() > MAX_PACKAGE_ARCHIVE_BLOB_BYTES {
            return Err(invalid_archive(
                "package archive blob exceeds its size bound; prepare a smaller commit",
            ));
        }
        store.persist_package_archive_retention(&self.retention)?;
        store.store_package_archive_object(
            domain.blob_namespace,
            &self.blob.content_id,
            &blob_bytes,
        )?;
        store_encoded(
            store,
            domain.membership_namespace,
            &self.membership_page.id,
            &self.membership_page,
        )?;
        store_encoded(
            store,
            domain.temporal_namespace,
            &self.temporal_page.id,
            &self.temporal_page,
        )?;
        for (id, page) in &self.lookup_pages {
            store_encoded(store, domain.lookup_namespace, id, page)?;
        }
        store_encoded(
            store,
            domain.directory_namespace,
            &self.directory.id,
            &self.directory,
        )?;
        let loaded = load_package_archive_directory(domain, store, &self.directory.id)?;
        let membership = authenticate_directory::<K, P>(domain, store, &loaded)?;
        if loaded != self.directory {
            return Err(invalid_archive(
                "stored package archive directory differs from its batch",
            ));
        }
        canwu_resource::verify_resource_archive_lookup_entries(
            &PackageLookupPages::new(domain, store),
            &loaded.lookup_root,
            "",
            &lookup_entries(domain, &membership)?,
        )
        .map_err(|error| lookup_error(&error))?;
        let mut retention = self.retention.clone();
        retention.phase = PackageArchiveRetentionPhaseV1::Verified;
        retention.semantic_digest.clear();
        retention.semantic_digest = digest(domain, "archive-retention.v1", &retention)?;
        store.persist_package_archive_retention(&retention)?;
        Ok(VerifiedPackageArchiveCommitV1 {
            expected_source_root: self.expected_source_root.clone(),
            selected: self.selected.clone(),
            directory_root: self.directory.id.clone(),
            retention,
            archived_records: u32::try_from(self.selected.len())
                .map_err(|_| invalid_archive("archive count overflow"))?,
        })
    }
}

/// Authenticates every archive object a restored root depends on: the
/// directory chain from the head, each commit's blob, records, and pages,
/// the cumulative record counts, the head's lookup index, and the objects of
/// every retention handle and maintenance receipt the root holds.
///
/// The lookup index of the head is rebuilt from the chain (a key archived
/// more than once counts by its newest record) and must be exactly what the
/// store holds: every page present and equal, and the root the head
/// directory names. The cost is linear in the archive, and the audit holds
/// about 40 bytes per archived record while it runs, so this is a restore
/// and audit step, never part of a boundary. It does not audit the index of
/// a commit that is queued and not yet applied; `store_and_verify` checked
/// that index when it was stored.
#[allow(clippy::too_many_lines)]
pub fn validate_package_archive_store<K, P>(
    domain: PackageArchiveDomainV1,
    head: &PackageArchiveHeadStateV1,
    handles: &BTreeMap<String, PackageArchiveRetentionHandleV1>,
    receipts: &BTreeMap<u64, PackageArchiveMaintenanceReceiptV1>,
    store: &dyn PackageArchiveStore,
) -> Result<(), CanwuError>
where
    K: Clone + Eq + Ord + Serialize + DeserializeOwned,
    P: Clone + PartialEq + Serialize + DeserializeOwned,
{
    let mut next = head.directory_root.clone();
    let mut expected_count = head.archived_record_count;
    let mut count = 0_u64;
    let mut lookup_root = None;
    let mut blob_ids = Vec::new();
    let mut entries = Vec::new();
    while let Some(root) = next.take() {
        count = count
            .checked_add(1)
            .ok_or_else(|| invalid_archive("archive chain overflow"))?;
        if count > head.revision {
            return Err(invalid_archive(
                "archive directory chain exceeds its committed revision",
            ));
        }
        let directory = load_package_archive_directory(domain, store, &root)?;
        let membership = authenticate_directory::<K, P>(domain, store, &directory)?;
        if directory.id != root
            || directory.archived_record_count != expected_count
            || membership.memberships.is_empty()
        {
            return Err(invalid_archive(
                "archive directory chain identity or count differs",
            ));
        }
        expected_count = expected_count
            .checked_sub(
                u64::try_from(membership.memberships.len())
                    .map_err(|_| invalid_archive("archive count overflow"))?,
            )
            .ok_or_else(|| invalid_archive("archive directory count does not advance"))?;
        lookup_root.get_or_insert_with(|| directory.lookup_root.clone());
        // Newest commit first, so the first entry kept for a key is the one
        // the index must name.
        let blob = u32::try_from(blob_ids.len())
            .map_err(|_| invalid_archive("archive blob position overflowed"))?;
        blob_ids.push(directory.blob_id.clone());
        for member in &membership.memberships {
            entries.push(
                ResourceArchiveAuditEntry::new(
                    &package_archive_lookup_key_digest(domain, &member.key)?,
                    blob,
                    member.ordinal,
                )
                .map_err(|error| lookup_error(&error))?,
            );
        }
        next.clone_from(&directory.previous_root);
    }
    if count != head.revision
        || expected_count != 0
        || (head.revision == 0) != head.directory_root.is_none()
    {
        return Err(invalid_archive(
            "archive head differs from its durable chain",
        ));
    }
    let rebuilt = canwu_resource::audit_resource_archive_lookup_index(
        &PackageLookupPages::new(domain, store),
        &blob_ids,
        entries,
    )
    .map_err(|error| lookup_error(&error))?;
    if rebuilt != lookup_root {
        return Err(invalid_archive(
            "archive lookup index differs from its directory chain",
        ));
    }
    for handle in handles.values() {
        let stored = store
            .load_package_archive_retention(&handle.id)?
            .ok_or_else(|| invalid_archive("archive retention handle is missing"))?;
        if stored.id != handle.id
            || stored.expected_source_root != handle.expected_source_root
            || stored.directory_root != handle.directory_root
            || stored.object_ids != handle.object_ids
            || !matches!(
                stored.phase,
                PackageArchiveRetentionPhaseV1::DurableIngress
                    | PackageArchiveRetentionPhaseV1::Committed
                    | PackageArchiveRetentionPhaseV1::RejectedStale
            )
        {
            return Err(invalid_archive("archive retention handle closure differs"));
        }
        if sealed_archive_retention(domain, stored.clone())? != stored {
            return Err(invalid_archive("archive retention handle is not sealed"));
        }
        for (namespace, ids) in &handle.object_ids {
            for id in ids {
                load_bytes(store, namespace, id)?;
            }
        }
        let directory: PackageArchiveIndexDirectoryV1 =
            load_required(store, domain.directory_namespace, &handle.directory_root)?;
        authenticate_directory::<K, P>(domain, store, &directory)?;
    }
    for receipt in receipts.values() {
        let stored = store
            .load_package_archive_retention(&receipt.retention_handle_id)?
            .ok_or_else(|| invalid_archive("archive terminal retention handle is missing"))?;
        let pending = handles.contains_key(&stored.id);
        let valid_phase = match receipt.disposition {
            PackageArchiveMaintenanceDispositionV1::Applied if pending => matches!(
                stored.phase,
                PackageArchiveRetentionPhaseV1::DurableIngress
                    | PackageArchiveRetentionPhaseV1::Committed
            ),
            PackageArchiveMaintenanceDispositionV1::Applied => {
                stored.phase == PackageArchiveRetentionPhaseV1::Committed
            }
            PackageArchiveMaintenanceDispositionV1::RejectedStale if pending => matches!(
                stored.phase,
                PackageArchiveRetentionPhaseV1::DurableIngress
                    | PackageArchiveRetentionPhaseV1::RejectedStale
            ),
            PackageArchiveMaintenanceDispositionV1::RejectedStale => {
                stored.phase == PackageArchiveRetentionPhaseV1::RejectedStale
            }
        };
        if !valid_phase
            || stored.id != receipt.retention_handle_id
            || stored.expected_source_root != receipt.expected_source_root
            || stored.directory_root != receipt.directory_root
        {
            return Err(invalid_archive(
                "archive terminal retention phase or receipt binding differs",
            ));
        }
        if sealed_archive_retention(domain, stored.clone())? != stored {
            return Err(invalid_archive(
                "archive terminal retention handle is not sealed",
            ));
        }
        if stored.phase == PackageArchiveRetentionPhaseV1::Committed {
            for (namespace, ids) in &stored.object_ids {
                for id in ids {
                    load_bytes(store, namespace, id)?;
                }
            }
            let directory: PackageArchiveIndexDirectoryV1 =
                load_required(store, domain.directory_namespace, &stored.directory_root)?;
            authenticate_directory::<K, P>(domain, store, &directory)?;
        }
    }
    Ok(())
}

/// Resolves one archived record through the cumulative lookup index of a
/// committed package archive head.
///
/// The lookup reads the head directory and one lookup page per index level,
/// each authenticated by its content id, and nothing else when the key was
/// never archived. Its cost follows the depth of the index, which grows with
/// the logarithm of the archived records and not at all with the number of
/// archive commits. A hit also reads the one blob that holds the record,
/// authenticates that blob by its content id, and checks the record's key
/// and payload digest. A key archived more than once resolves to its newest
/// record.
///
/// What a lookup reads is bounded whatever the provider returns: one
/// directory of at most [`MAX_PACKAGE_ARCHIVE_DIRECTORY_BYTES`], at most
/// [`canwu_resource::MAX_RESOURCE_ARCHIVE_LOOKUP_DEPTH`]` + 1` lookup pages
/// of at most [`canwu_resource::MAX_RESOURCE_ARCHIVE_LOOKUP_PAGE_BYTES`]
/// each, and one blob of at most [`MAX_PACKAGE_ARCHIVE_BLOB_BYTES`], each
/// refused before it is decoded when larger. A missing, forged, oversized, or
/// malformed object is an archive error, never a miss.
///
/// The lookup trusts the head directory to name an index that holds every
/// archived key. The enqueue helpers establish that for each commit they
/// queue, because the batch must equal what the current root and the store
/// derive and `store_and_verify` checks every record against the stored
/// index, and [`validate_package_archive_store`] checks it for the head of a
/// restored root.
pub fn load_package_archive_record<K, P>(
    domain: PackageArchiveDomainV1,
    head: &PackageArchiveHeadStateV1,
    provider: &dyn PluginArchiveObjectProvider,
    key: &K,
) -> Result<Option<PackageArchiveRecordV1<K, P>>, CanwuError>
where
    K: Clone + Eq + Ord + Serialize + DeserializeOwned,
    P: Clone + PartialEq + Serialize + DeserializeOwned,
{
    let Some(root) = &head.directory_root else {
        return Ok(None);
    };
    let store = PluginProviderStore(provider);
    let directory = load_package_archive_directory(domain, &store, root)?;
    let Some(entry) = canwu_resource::find_resource_archive_lookup_entry(
        &PackageLookupPages::new(domain, &store),
        &directory.lookup_root,
        &package_archive_lookup_key_digest(domain, key)?,
    )
    .map_err(|error| lookup_error(&error))?
    else {
        return Ok(None);
    };
    let blob: PackageArchiveBlobV1<K, P> = load_bounded(
        &store,
        domain.blob_namespace,
        &entry.blob_id,
        MAX_PACKAGE_ARCHIVE_BLOB_BYTES,
    )?;
    if blob.content_id != entry.blob_id
        || blob.content_id
            != digest_with_cleared(domain, "archive-blob.v1", &blob, |v| &mut v.content_id)?
    {
        return Err(invalid_archive("archive lookup blob digest is forged"));
    }
    let record = blob
        .records
        .into_iter()
        .nth(usize::from(entry.ordinal))
        .ok_or_else(|| invalid_archive("archive lookup ordinal is outside its blob"))?;
    if &record.key != key
        || record.semantic_digest != digest(domain, "archive-record-payload.v1", &record.payload)?
    {
        return Err(invalid_archive(
            "archive lookup entry differs from its authenticated record",
        ));
    }
    Ok(Some(record))
}

/// Reads an authenticated bounded cold-history window. This is intended for
/// holder reports whose configured fact budget is itself bounded; callers must
/// supply that budget and receive a stable query-budget error instead of an
/// unbounded provider scan.
pub fn load_package_archive_records<K, P>(
    domain: PackageArchiveDomainV1,
    head: &PackageArchiveHeadStateV1,
    provider: &dyn PluginArchiveObjectProvider,
    record_limit: usize,
) -> Result<Vec<PackageArchiveRecordV1<K, P>>, CanwuError>
where
    K: Clone + Eq + Ord + Serialize + DeserializeOwned,
    P: Clone + PartialEq + Serialize + DeserializeOwned,
{
    if record_limit == 0 {
        return Err(CanwuError::new(
            ErrorCode::QueryBudgetExceeded,
            "package archive query budget is zero",
        ));
    }
    let store = PluginProviderStore(provider);
    let mut next = head.directory_root.clone();
    let mut remaining = head.revision;
    let mut records = Vec::new();
    let mut keys = BTreeSet::new();
    while let Some(root) = next {
        if remaining == 0 {
            return Err(invalid_archive(
                "archive directory chain exceeds its committed revision",
            ));
        }
        remaining -= 1;
        let directory: PackageArchiveIndexDirectoryV1 =
            load_required(&store, domain.directory_namespace, &root)?;
        authenticate_directory::<K, P>(domain, &store, &directory)?;
        let blob: PackageArchiveBlobV1<K, P> =
            load_required(&store, domain.blob_namespace, &directory.blob_id)?;
        for record in blob.records {
            if !keys.insert(record.key.clone()) {
                return Err(invalid_archive(
                    "archive contains duplicate cold membership",
                ));
            }
            if records.len() >= record_limit {
                return Err(CanwuError::new(
                    ErrorCode::QueryBudgetExceeded,
                    "package archive history exceeds the requested query budget",
                ));
            }
            records.push(record);
        }
        next = directory.previous_root;
    }
    if remaining != 0 {
        return Err(invalid_archive(
            "archive directory chain is shorter than its committed revision",
        ));
    }
    Ok(records)
}

struct PluginProviderStore<'a>(&'a dyn PluginArchiveObjectProvider);

impl PackageArchiveStore for PluginProviderStore<'_> {
    fn store_package_archive_object(&self, _: &str, _: &str, _: &[u8]) -> Result<(), CanwuError> {
        Err(invalid_archive("archive provider is read-only"))
    }

    fn load_package_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        self.0.load_plugin_archive_object(namespace, object_id)
    }

    fn persist_package_archive_retention(
        &self,
        _: &PackageArchiveRetentionHandleV1,
    ) -> Result<(), CanwuError> {
        Err(invalid_archive("archive provider is read-only"))
    }

    fn load_package_archive_retention(
        &self,
        _: &str,
    ) -> Result<Option<PackageArchiveRetentionHandleV1>, CanwuError> {
        Ok(None)
    }

    fn finalize_package_archive_retention(
        &self,
        _: &PackageArchiveRetentionHandleV1,
    ) -> Result<(), CanwuError> {
        Err(invalid_archive("archive provider is read-only"))
    }
}

/// Marks every package archive object a restore, a replay, a fork, or a
/// queued commit can still read: the commits reachable from the archive
/// head, from `pending_roots` (the directories of unacknowledged retention
/// handles), and from each queued commit, with their blobs, pages, and
/// lookup pages.
///
/// The kernel marks the directory each queued archive commit retains before
/// it calls the participant, so the directories already in `manifest` under
/// the domain's directory namespace are the queued commits. The lookup index
/// of the head and of each queued commit, which may become the head, must be
/// whole; the index of any other directory is marked as far as the store
/// still holds it, so a collection never drops a page that a restore, replay,
/// or fork through that commit could read, and a host application that
/// deleted superseded pages is not refused.
pub fn extend_archive_reachability<K, P>(
    domain: PackageArchiveDomainV1,
    head: Option<&str>,
    pending_roots: impl IntoIterator<Item = String>,
    provider: &dyn PluginArchiveObjectProvider,
    manifest: &mut ArchiveReachabilityManifest,
) -> Result<(), CanwuError>
where
    K: Clone + Eq + Ord + Serialize + DeserializeOwned,
    P: Clone + PartialEq + Serialize + DeserializeOwned,
{
    let store = PluginProviderStore(provider);
    let mut complete = manifest
        .plugin_objects
        .get(domain.directory_namespace)
        .cloned()
        .unwrap_or_default();
    complete.extend(head.map(str::to_owned));
    let mut pending = complete.clone();
    pending.extend(pending_roots);
    let mut seen = BTreeSet::new();
    let mut lookup_roots = Vec::new();
    while let Some(root) = pending.pop_first() {
        if !seen.insert(root.clone()) {
            continue;
        }
        let directory = load_package_archive_directory(domain, &store, &root)?;
        authenticate_directory::<K, P>(domain, &store, &directory)?;
        lookup_roots.push((!complete.contains(&root), directory.lookup_root.clone()));
        manifest
            .plugin_objects
            .entry(domain.directory_namespace.to_owned())
            .or_default()
            .insert(root);
        manifest
            .plugin_objects
            .entry(domain.blob_namespace.to_owned())
            .or_default()
            .insert(directory.blob_id.clone());
        manifest
            .plugin_objects
            .entry(domain.membership_namespace.to_owned())
            .or_default()
            .insert(directory.membership_page.clone());
        manifest
            .plugin_objects
            .entry(domain.temporal_namespace.to_owned())
            .or_default()
            .insert(directory.temporal_page.clone());
        if let Some(previous) = directory.previous_root {
            pending.insert(previous);
        }
    }
    // Complete indexes first: a page they share with another index is then
    // already known to be whole.
    lookup_roots.sort();
    let mut lookup_pages = BTreeSet::new();
    for (earlier, lookup_root) in &lookup_roots {
        canwu_resource::collect_resource_archive_lookup_pages(
            &PackageLookupPages::new(domain, &store),
            lookup_root,
            "",
            !earlier,
            &mut lookup_pages,
        )
        .map_err(|error| lookup_error(&error))?;
    }
    if !lookup_pages.is_empty() {
        manifest
            .plugin_objects
            .entry(domain.lookup_namespace.to_owned())
            .or_default()
            .extend(lookup_pages);
    }
    Ok(())
}

/// Loads one archive directory by id and authenticates its identity: the id
/// is the digest of the directory. Its blob and pages are not read; a lookup
/// trusts the directory, reached from the head the plugin root records, to
/// name a complete index (see [`load_package_archive_record`]).
fn load_package_archive_directory(
    domain: PackageArchiveDomainV1,
    store: &dyn PackageArchiveStore,
    root: &str,
) -> Result<PackageArchiveIndexDirectoryV1, CanwuError> {
    let directory: PackageArchiveIndexDirectoryV1 = load_bounded(
        store,
        domain.directory_namespace,
        root,
        MAX_PACKAGE_ARCHIVE_DIRECTORY_BYTES,
    )?;
    authenticate_directory_identity(domain, &directory)?;
    if directory.id != root {
        return Err(invalid_archive("archive directory identity differs"));
    }
    Ok(directory)
}

fn authenticate_directory_identity(
    domain: PackageArchiveDomainV1,
    directory: &PackageArchiveIndexDirectoryV1,
) -> Result<(), CanwuError> {
    let mut detached = directory.clone();
    let id = std::mem::take(&mut detached.id);
    let semantic = std::mem::take(&mut detached.semantic_digest);
    if id != semantic || semantic != digest(domain, "archive-directory.v1", &detached)? {
        return Err(invalid_archive("archive directory digest is forged"));
    }
    Ok(())
}

/// Authenticates one commit: the directory's identity, its blob and every
/// record payload in it, and its membership and temporal pages, which must
/// cover exactly the blob's records, each membership naming the blob and
/// the record's position in it. Returns the membership page.
fn authenticate_directory<K, P>(
    domain: PackageArchiveDomainV1,
    store: &dyn PackageArchiveStore,
    directory: &PackageArchiveIndexDirectoryV1,
) -> Result<PackageArchiveMembershipPageV1<K>, CanwuError>
where
    K: Clone + Eq + Ord + Serialize + DeserializeOwned,
    P: Clone + PartialEq + Serialize + DeserializeOwned,
{
    authenticate_directory_identity(domain, directory)?;
    let blob: PackageArchiveBlobV1<K, P> = load_bounded(
        store,
        domain.blob_namespace,
        &directory.blob_id,
        MAX_PACKAGE_ARCHIVE_BLOB_BYTES,
    )?;
    let membership: PackageArchiveMembershipPageV1<K> = load_required(
        store,
        domain.membership_namespace,
        &directory.membership_page,
    )?;
    let temporal: PackageArchiveTemporalPageV1<K> =
        load_required(store, domain.temporal_namespace, &directory.temporal_page)?;
    if blob.content_id != directory.blob_id
        || blob.content_id
            != digest_with_cleared(domain, "archive-blob.v1", &blob, |v| &mut v.content_id)?
        || membership.id != directory.membership_page
        || membership.id != membership.semantic_digest
        || membership.semantic_digest
            != digest_with_cleared(domain, "archive-membership-page.v1", &membership, |v| {
                v.id.clear();
                &mut v.semantic_digest
            })?
        || temporal.id != directory.temporal_page
        || temporal.id != temporal.semantic_digest
        || temporal.semantic_digest
            != digest_with_cleared(domain, "archive-temporal-page.v1", &temporal, |v| {
                v.id.clear();
                &mut v.semantic_digest
            })?
    {
        return Err(invalid_archive("archive object digest is forged"));
    }
    if membership
        .memberships
        .iter()
        .enumerate()
        .any(|(position, member)| {
            member.blob_id != blob.content_id || usize::from(member.ordinal) != position
        })
    {
        return Err(invalid_archive(
            "archive membership does not name its blob position",
        ));
    }
    let keys = blob
        .records
        .iter()
        .map(|r| (&r.key, r.terminal_sequence, &r.semantic_digest))
        .collect::<Vec<_>>();
    if blob.records.iter().any(|record| {
        digest(domain, "archive-record-payload.v1", &record.payload)
            .map_or(true, |expected| expected != record.semantic_digest)
    }) {
        return Err(invalid_archive(
            "archive terminal payload closure is forged",
        ));
    }
    let member_keys = membership
        .memberships
        .iter()
        .map(|m| (&m.key, m.terminal_sequence, &m.semantic_digest))
        .collect::<Vec<_>>();
    let temporal_keys = temporal
        .entries
        .iter()
        .map(|e| (&e.key, e.terminal_sequence))
        .collect::<BTreeSet<_>>();
    if keys != member_keys
        || keys
            .iter()
            .map(|(k, s, _)| (*k, *s))
            .collect::<BTreeSet<_>>()
            != temporal_keys
    {
        return Err(invalid_archive("archive pages do not close over the blob"));
    }
    Ok(membership)
}

fn digest<T: Serialize>(
    domain: PackageArchiveDomainV1,
    suffix: &str,
    value: &T,
) -> Result<String, CanwuError> {
    canwu_api::canonical_hash(&format!("{}.{}", domain.digest_prefix, suffix), value)
}
fn digest_with_cleared<T: Clone + Serialize>(
    domain: PackageArchiveDomainV1,
    suffix: &str,
    value: &T,
    clear: impl FnOnce(&mut T) -> &mut String,
) -> Result<String, CanwuError> {
    let mut detached = value.clone();
    clear(&mut detached).clear();
    digest(domain, suffix, &detached)
}
fn store_encoded<T: Serialize>(
    store: &dyn PackageArchiveStore,
    namespace: &str,
    id: &str,
    value: &T,
) -> Result<(), CanwuError> {
    store.store_package_archive_object(
        namespace,
        id,
        &serde_json::to_vec(value).map_err(|e| invalid_archive(e.to_string()))?,
    )
}
fn load_bytes(
    store: &dyn PackageArchiveStore,
    namespace: &str,
    id: &str,
) -> Result<Vec<u8>, CanwuError> {
    store
        .load_package_archive_object(namespace, id)?
        .ok_or_else(|| invalid_archive("archive object is missing"))
}
fn load_required<T: DeserializeOwned>(
    store: &dyn PackageArchiveStore,
    namespace: &str,
    id: &str,
) -> Result<T, CanwuError> {
    serde_json::from_slice(&load_bytes(store, namespace, id)?)
        .map_err(|e| invalid_archive(e.to_string()))
}
/// [`load_required`] that refuses an object above `max_bytes` before
/// decoding it.
fn load_bounded<T: DeserializeOwned>(
    store: &dyn PackageArchiveStore,
    namespace: &str,
    id: &str,
    max_bytes: usize,
) -> Result<T, CanwuError> {
    let bytes = load_bytes(store, namespace, id)?;
    if bytes.len() > max_bytes {
        return Err(invalid_archive(
            "archive object exceeds its size bound before decoding",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|e| invalid_archive(e.to_string()))
}
fn invalid_archive(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidArchive, message)
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", content = "key", rename_all = "snake_case")]
pub enum ForceArchiveKeyV1 {
    TerminalIntent(crate::ForceConsumptionIntentId),
    TerminalSaga(crate::RequisitionSagaId),
    TerminalOperation(canwu_resource::ResourceOperationKey),
    OperationOutcome(crate::ForceOperationId),
    KnowledgePublication(crate::ForceKnowledgePublicationId),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceTerminalLifecycleArchiveV1 {
    pub receipt: crate::ForceTerminalReceiptV1,
    pub acquisition: canwu_resource::CompletionLeaseAcquisitionV1,
    pub local_grants: BTreeMap<
        canwu_resource::CompletionCapacityGrantId,
        canwu_resource::CompletionCapacityGrantV1,
    >,
    pub certificate: canwu_resource::CompletionLeaseActivationCertificateV1,
    pub external_participants:
        BTreeMap<String, canwu_resource::ExternalCompletionParticipantGrantV1>,
    pub lease_receipts: Vec<canwu_resource::CompletionLeaseReceiptV1>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "payload", content = "value", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ForceArchivePayloadV1 {
    TerminalLifecycle(ForceTerminalLifecycleArchiveV1),
    TerminalOperationAlias {
        intent: crate::ForceConsumptionIntentId,
        lifecycle_digest: String,
    },
    TerminalSagaAlias {
        intent: crate::ForceConsumptionIntentId,
        lifecycle_digest: String,
    },
    OperationOutcome(crate::ForceOperationOutcomeV1),
    KnowledgePublication(crate::ForceKnowledgePublicationV1),
}

pub type ForceArchiveBlobV1 = PackageArchiveBlobV1<ForceArchiveKeyV1, ForceArchivePayloadV1>;
pub type ForceArchiveMembershipV1 = PackageArchiveMembershipV1<ForceArchiveKeyV1>;
pub type ForceArchiveIndexDirectoryV1 = PackageArchiveIndexDirectoryV1;
pub type ForceArchiveHeadStateV1 = PackageArchiveHeadStateV1;
pub type ForceArchiveRetentionHandleV1 = PackageArchiveRetentionHandleV1;
pub type ForceArchiveMaintenanceReceiptV1 = PackageArchiveMaintenanceReceiptV1;
pub type PreparedForceArchiveBatchV1 =
    PreparedPackageArchiveBatchV1<ForceArchiveKeyV1, ForceArchivePayloadV1>;
pub type VerifiedForceArchiveCommitV1 = VerifiedPackageArchiveCommitV1<ForceArchiveKeyV1>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct Payload(u64);

    #[derive(Default)]
    struct Store {
        objects: RefCell<BTreeMap<(String, String), Vec<u8>>>,
        handles: RefCell<BTreeMap<String, PackageArchiveRetentionHandleV1>>,
    }

    impl PackageArchiveStore for Store {
        fn store_package_archive_object(
            &self,
            namespace: &str,
            id: &str,
            bytes: &[u8],
        ) -> Result<(), CanwuError> {
            self.objects
                .borrow_mut()
                .insert((namespace.to_owned(), id.to_owned()), bytes.to_vec());
            Ok(())
        }
        fn load_package_archive_object(
            &self,
            namespace: &str,
            id: &str,
        ) -> Result<Option<Vec<u8>>, CanwuError> {
            Ok(self
                .objects
                .borrow()
                .get(&(namespace.to_owned(), id.to_owned()))
                .cloned())
        }
        fn persist_package_archive_retention(
            &self,
            handle: &PackageArchiveRetentionHandleV1,
        ) -> Result<(), CanwuError> {
            self.handles
                .borrow_mut()
                .insert(handle.id.clone(), handle.clone());
            Ok(())
        }
        fn load_package_archive_retention(
            &self,
            id: &str,
        ) -> Result<Option<PackageArchiveRetentionHandleV1>, CanwuError> {
            Ok(self.handles.borrow().get(id).cloned())
        }
        fn finalize_package_archive_retention(
            &self,
            handle: &PackageArchiveRetentionHandleV1,
        ) -> Result<(), CanwuError> {
            self.handles
                .borrow_mut()
                .insert(handle.id.clone(), handle.clone());
            Ok(())
        }
    }

    impl PluginArchiveObjectProvider for Store {
        fn load_plugin_archive_object(
            &self,
            namespace: &str,
            object_id: &str,
        ) -> Result<Option<Vec<u8>>, CanwuError> {
            self.load_package_archive_object(namespace, object_id)
        }
    }

    #[test]
    fn provider_backed_archive_authenticates_payload_closure_and_fails_on_corruption() {
        let store = Store::default();
        let head = sealed_archive_head(FORCE_ARCHIVE_DOMAIN, PackageArchiveHeadStateV1::default())
            .expect("head");
        let payload = Payload(7);
        let record = PackageArchiveRecordV1 {
            key: 1_u64,
            terminal_sequence: 1,
            semantic_digest: digest(FORCE_ARCHIVE_DOMAIN, "archive-record-payload.v1", &payload)
                .expect("digest"),
            payload,
        };
        let prepared = prepare_package_archive(
            FORCE_ARCHIVE_DOMAIN,
            "a".repeat(64),
            &head,
            vec![record],
            &(),
        )
        .expect("prepare");
        let commit = prepared
            .store_and_verify(FORCE_ARCHIVE_DOMAIN, &store)
            .expect("verify");
        let committed_head = sealed_archive_head(
            FORCE_ARCHIVE_DOMAIN,
            PackageArchiveHeadStateV1 {
                revision: 1,
                directory_root: Some(commit.directory_root),
                archived_record_count: 1,
                semantic_digest: String::new(),
            },
        )
        .expect("committed head");
        validate_package_archive_store::<u64, Payload>(
            FORCE_ARCHIVE_DOMAIN,
            &committed_head,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &store,
        )
        .expect("restore");
        let exact = load_package_archive_record::<u64, Payload>(
            FORCE_ARCHIVE_DOMAIN,
            &committed_head,
            &store,
            &1,
        )
        .expect("cold lookup")
        .expect("cold membership");
        assert_eq!(exact.payload, Payload(7));
        assert!(
            load_package_archive_record::<u64, Payload>(
                FORCE_ARCHIVE_DOMAIN,
                &committed_head,
                &store,
                &2,
            )
            .expect("cold miss")
            .is_none()
        );
        store.objects.borrow_mut().insert(
            (
                FORCE_ARCHIVE_BLOB_NAMESPACE.to_owned(),
                prepared.blob.content_id,
            ),
            b"{}".to_vec(),
        );
        assert!(
            validate_package_archive_store::<u64, Payload>(
                FORCE_ARCHIVE_DOMAIN,
                &committed_head,
                &BTreeMap::new(),
                &BTreeMap::new(),
                &store
            )
            .is_err()
        );
    }
}
