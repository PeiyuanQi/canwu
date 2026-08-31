use canwu_api::{ArchiveReachabilityManifest, CanwuError, ErrorCode, PluginArchiveObjectProvider};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::{BTreeMap, BTreeSet};

pub const FORCE_ARCHIVE_BLOB_NAMESPACE: &str = "canwu.force-supply-reference.archive.blob";
pub const FORCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE: &str =
    "canwu.force-supply-reference.archive.membership-page";
pub const FORCE_ARCHIVE_TEMPORAL_PAGE_NAMESPACE: &str =
    "canwu.force-supply-reference.archive.temporal-page";
pub const FORCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE: &str =
    "canwu.force-supply-reference.archive.index-directory";
pub const MAX_PACKAGE_ARCHIVE_PAGE_ENTRIES: usize = 512;

#[derive(Clone, Copy)]
pub struct PackageArchiveDomainV1 {
    pub digest_prefix: &'static str,
    pub blob_namespace: &'static str,
    pub membership_namespace: &'static str,
    pub temporal_namespace: &'static str,
    pub directory_namespace: &'static str,
}

pub const FORCE_ARCHIVE_DOMAIN: PackageArchiveDomainV1 = PackageArchiveDomainV1 {
    digest_prefix: "canwu.force-supply-reference",
    blob_namespace: FORCE_ARCHIVE_BLOB_NAMESPACE,
    membership_namespace: FORCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE,
    temporal_namespace: FORCE_ARCHIVE_TEMPORAL_PAGE_NAMESPACE,
    directory_namespace: FORCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackageArchiveIndexDirectoryV1 {
    pub id: String,
    pub previous_root: Option<String>,
    pub membership_page: String,
    pub temporal_page: String,
    pub blob_id: String,
    pub archived_record_count: u64,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreparedPackageArchiveBatchV1<K, P> {
    pub expected_source_root: String,
    pub selected: Vec<K>,
    pub blob: PackageArchiveBlobV1<K, P>,
    pub membership_page: PackageArchiveMembershipPageV1<K>,
    pub temporal_page: PackageArchiveTemporalPageV1<K>,
    pub directory: PackageArchiveIndexDirectoryV1,
    pub retention: PackageArchiveRetentionHandleV1,
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

#[allow(clippy::too_many_lines)]
pub fn prepare_package_archive<K, P>(
    domain: PackageArchiveDomainV1,
    expected_source_root: String,
    head: &PackageArchiveHeadStateV1,
    records: Vec<PackageArchiveRecordV1<K, P>>,
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
    membership_page.id = membership_page.semantic_digest.clone();
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
    temporal_page.id = temporal_page.semantic_digest.clone();
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
    })
}

impl<K, P> PreparedPackageArchiveBatchV1<K, P>
where
    K: Clone + Eq + Ord + Serialize + DeserializeOwned,
    P: Clone + PartialEq + Serialize + DeserializeOwned,
{
    pub fn store_and_verify(
        &self,
        domain: PackageArchiveDomainV1,
        store: &dyn PackageArchiveStore,
    ) -> Result<VerifiedPackageArchiveCommitV1<K>, CanwuError> {
        store.persist_package_archive_retention(&self.retention)?;
        store_encoded(
            store,
            domain.blob_namespace,
            &self.blob.content_id,
            &self.blob,
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
        store_encoded(
            store,
            domain.directory_namespace,
            &self.directory.id,
            &self.directory,
        )?;
        authenticate_directory::<K, P>(domain, store, &self.directory)?;
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
    while let Some(root) = next.take() {
        count = count
            .checked_add(1)
            .ok_or_else(|| invalid_archive("archive chain overflow"))?;
        let directory: PackageArchiveIndexDirectoryV1 =
            load_required(store, domain.directory_namespace, &root)?;
        authenticate_directory::<K, P>(domain, store, &directory)?;
        if directory.id != root || directory.archived_record_count != expected_count {
            return Err(invalid_archive(
                "archive directory chain identity or count differs",
            ));
        }
        next.clone_from(&directory.previous_root);
        if let Some(previous) = &next {
            let prior: PackageArchiveIndexDirectoryV1 =
                load_required(store, domain.directory_namespace, previous)?;
            if prior.archived_record_count >= directory.archived_record_count {
                return Err(invalid_archive("archive directory count does not advance"));
            }
            expected_count = prior.archived_record_count;
        }
    }
    if count != head.revision || (head.revision == 0) != head.directory_root.is_none() {
        return Err(invalid_archive(
            "archive head differs from its durable chain",
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

/// Resolves one exact cold membership from a committed package archive head.
/// Every traversed directory and the selected membership/blob closure is
/// authenticated before a payload is returned. A missing or corrupt provider
/// object is an archive error, never an apparent cache miss.
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
    let store = PluginProviderStore(provider);
    let mut next = head.directory_root.clone();
    let mut remaining = head.revision;
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
        let membership: PackageArchiveMembershipPageV1<K> = load_required(
            &store,
            domain.membership_namespace,
            &directory.membership_page,
        )?;
        if let Some(member) = membership
            .memberships
            .iter()
            .find(|member| &member.key == key)
        {
            let blob: PackageArchiveBlobV1<K, P> =
                load_required(&store, domain.blob_namespace, &member.blob_id)?;
            let record = blob
                .records
                .get(usize::from(member.ordinal))
                .ok_or_else(|| invalid_archive("archive membership ordinal is unavailable"))?;
            if &record.key != key
                || record.terminal_sequence != member.terminal_sequence
                || record.semantic_digest != member.semantic_digest
            {
                return Err(invalid_archive(
                    "archive membership differs from its authenticated payload",
                ));
            }
            return Ok(Some(record.clone()));
        }
        next = directory.previous_root;
    }
    if remaining != 0 {
        return Err(invalid_archive(
            "archive directory chain is shorter than its committed revision",
        ));
    }
    Ok(None)
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

pub fn extend_archive_reachability<K, P>(
    domain: PackageArchiveDomainV1,
    roots: impl IntoIterator<Item = String>,
    provider: &dyn PluginArchiveObjectProvider,
    manifest: &mut ArchiveReachabilityManifest,
) -> Result<(), CanwuError>
where
    K: Clone + Eq + Ord + Serialize + DeserializeOwned,
    P: Clone + PartialEq + Serialize + DeserializeOwned,
{
    let store = PluginProviderStore(provider);
    let mut pending = roots.into_iter().collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    while let Some(root) = pending.pop_first() {
        if !seen.insert(root.clone()) {
            continue;
        }
        let directory: PackageArchiveIndexDirectoryV1 =
            load_required(&store, domain.directory_namespace, &root)?;
        authenticate_directory::<K, P>(domain, &store, &directory)?;
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
    Ok(())
}

fn authenticate_directory<K, P>(
    domain: PackageArchiveDomainV1,
    store: &dyn PackageArchiveStore,
    directory: &PackageArchiveIndexDirectoryV1,
) -> Result<(), CanwuError>
where
    K: Clone + Eq + Ord + Serialize + DeserializeOwned,
    P: Clone + PartialEq + Serialize + DeserializeOwned,
{
    let blob: PackageArchiveBlobV1<K, P> =
        load_required(store, domain.blob_namespace, &directory.blob_id)?;
    let membership: PackageArchiveMembershipPageV1<K> = load_required(
        store,
        domain.membership_namespace,
        &directory.membership_page,
    )?;
    let temporal: PackageArchiveTemporalPageV1<K> =
        load_required(store, domain.temporal_namespace, &directory.temporal_page)?;
    let mut detached = directory.clone();
    let id = std::mem::take(&mut detached.id);
    let semantic = std::mem::take(&mut detached.semantic_digest);
    if id != semantic
        || semantic != digest(domain, "archive-directory.v1", &detached)?
        || blob.content_id
            != digest_with_cleared(domain, "archive-blob.v1", &blob, |v| &mut v.content_id)?
        || membership.id != membership.semantic_digest
        || temporal.id != temporal.semantic_digest
    {
        return Err(invalid_archive("archive object digest is forged"));
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
    Ok(())
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
        let prepared =
            prepare_package_archive(FORCE_ARCHIVE_DOMAIN, "a".repeat(64), &head, vec![record])
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
