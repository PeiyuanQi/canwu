use crate::residue::{ArchiveClaims, outcome_matches_lease, validate_residue_archive_record};
use crate::{
    MAX_ARCHIVE_BYTES_PER_BATCH, MAX_ARCHIVE_COMMIT_PAGES, MAX_ARCHIVE_OBJECTS_PER_BATCH,
    MAX_ARCHIVE_PAGE_ENTRIES, MAX_ARCHIVE_PREPARE_CANDIDATES, PreparedProductionArchiveBatchV1,
    PreparedProductionArchivePageV1, ProductionArchiveBlobV1, ProductionArchiveIndexDirectoryV1,
    ProductionArchiveMaintenanceReceiptV1, ProductionArchiveMembershipPageV1,
    ProductionArchiveMembershipV1, ProductionArchiveReceiptId, ProductionArchiveRetentionHandleV1,
    ProductionArchiveRetentionPhaseV1, ProductionArchiveTemporalEntryV1,
    ProductionArchiveTemporalPageV1, ProductionExecutionId,
    ProductionFacilityProjectArchiveRecordV1, ProductionResidueArchiveRecordV1, ProductionState,
    ProductionTerminalArchiveKeyV1, ProductionTerminalArchiveRecordV1,
    VerifiedProductionArchiveCommitV1, WorkInProgressId, WorkOrderLifecycle, invalid,
};
use canwu_api::{CanwuError, ErrorCode, SimTime, canonical_hash};
use canwu_resource::{ResourceArchiveLookupEntryV1, ResourceError};
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, BTreeSet};

pub const PRODUCTION_ARCHIVE_BLOB_NAMESPACE: &str = "canwu.production.archive.blob";
pub const PRODUCTION_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE: &str =
    "canwu.production.archive.membership-page";
pub const PRODUCTION_ARCHIVE_TEMPORAL_PAGE_NAMESPACE: &str =
    "canwu.production.archive.temporal-page";
pub const PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE: &str =
    "canwu.production.archive.index-directory";
/// Namespace of the pages of the production archive's cumulative lookup
/// index. A page has the format and content id of
/// [`canwu_resource::ResourceArchiveLookupPageV1`]; only the namespace and
/// the key digests (see [`production_archive_lookup_key_digest`]) are the
/// production archive's own.
pub const PRODUCTION_ARCHIVE_LOOKUP_PAGE_NAMESPACE: &str = "canwu.production.archive.lookup-page";

const PRODUCTION_ARCHIVE_LOOKUP_KEY_DOMAIN: &str = "canwu.production.archive-lookup-key.v1";

/// Format of [`ProductionArchiveBlobV1`]. Version 3 adds residue records and
/// the decision receipts and lease outcomes an execution archives; a blob of
/// any other version is refused.
pub const PRODUCTION_ARCHIVE_BLOB_FORMAT_VERSION: u32 = 3;

/// The host application's store of production archive objects and retention
/// handles.
///
/// A store a run holds, through
/// [`from_production_snapshot_json_with_archives`](crate::from_production_snapshot_json_with_archives)
/// and the other `_with_archives` restores, is passed as
/// `Arc<dyn ProductionArchiveStore + Send + Sync>`, because forks share it and
/// may run on other threads. A store lent for one call as
/// `&dyn ProductionArchiveStore` needs neither bound.
pub trait ProductionArchiveStore {
    fn store_production_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), CanwuError>;

    fn load_production_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError>;

    fn persist_production_archive_retention(
        &self,
        handle: &ProductionArchiveRetentionHandleV1,
    ) -> Result<(), CanwuError>;

    fn finalize_production_archive_retention(
        &self,
        handle_id: &str,
        phase: ProductionArchiveRetentionPhaseV1,
    ) -> Result<(), CanwuError>;
}

/// The absent store: it holds no object and accepts none. It is enough to
/// prepare the first archive commit of a production root, which reads
/// nothing, and it fails every read of a root that already has an archive
/// head.
impl ProductionArchiveStore for () {
    fn store_production_archive_object(
        &self,
        _namespace: &str,
        _object_id: &str,
        _bytes: &[u8],
    ) -> Result<(), CanwuError> {
        Err(invalid("no production archive store is attached"))
    }

    fn load_production_archive_object(
        &self,
        _namespace: &str,
        _object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        Ok(None)
    }

    fn persist_production_archive_retention(
        &self,
        _handle: &ProductionArchiveRetentionHandleV1,
    ) -> Result<(), CanwuError> {
        Err(invalid("no production archive store is attached"))
    }

    fn finalize_production_archive_retention(
        &self,
        _handle_id: &str,
        _phase: ProductionArchiveRetentionPhaseV1,
    ) -> Result<(), CanwuError> {
        Err(invalid("no production archive store is attached"))
    }
}

/// One archived terminal record, as [`archived_production_terminal_record`]
/// returns it.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum ProductionArchivedRecordV1 {
    /// A settled, cancelled, or failed execution with its work.
    Execution(ProductionTerminalArchiveRecordV1),
    /// A completed facility project.
    FacilityProject(ProductionFacilityProjectArchiveRecordV1),
    /// The maintenance receipt of an applied archive commit.
    ArchiveMaintenanceReceipt(ProductionArchiveMaintenanceReceiptV1),
    /// A residue: a work order, completion lease, or outcome no execution or
    /// facility project archived.
    Residue(ProductionResidueArchiveRecordV1),
}

/// The production store seen as a store of lookup-index pages, so that the
/// lookup-index functions of `canwu-resource` read and audit the production
/// index. It serves the resource lookup-page namespace from the production
/// one, refuses every other namespace, and writes nothing.
struct ProductionLookupPages<'a>(&'a dyn ProductionArchiveStore);

impl canwu_resource::ResourceArchiveStore for ProductionLookupPages<'_> {
    fn store_resource_archive_object(
        &self,
        _namespace: &str,
        _object_id: &str,
        _bytes: &[u8],
    ) -> Result<(), ResourceError> {
        Err(ResourceError::InvalidLifecycle(
            "the production lookup-page view is read-only".to_owned(),
        ))
    }

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, ResourceError> {
        if namespace != canwu_resource::RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE {
            return Err(ResourceError::InvalidDefinition(format!(
                "the production lookup-page view serves no {namespace} object"
            )));
        }
        self.0
            .load_production_archive_object(PRODUCTION_ARCHIVE_LOOKUP_PAGE_NAMESPACE, object_id)
            .map_err(|error| {
                ResourceError::InvalidDefinition(format!(
                    "production archive store failed: {error}"
                ))
            })
    }

    fn persist_resource_archive_retention(
        &self,
        _handle: &canwu_resource::ResourceArchiveRetentionHandleV1,
    ) -> Result<(), ResourceError> {
        Err(ResourceError::InvalidLifecycle(
            "the production lookup-page view keeps no retention".to_owned(),
        ))
    }

    fn finalize_resource_archive_retention(
        &self,
        _handle_id: &str,
        _phase: canwu_resource::ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError> {
        Err(ResourceError::InvalidLifecycle(
            "the production lookup-page view keeps no retention".to_owned(),
        ))
    }
}

fn lookup_error(error: &ResourceError) -> CanwuError {
    invalid(format!("production archive lookup index: {error}"))
}

/// Digest under which the production lookup index files one terminal record
/// key: a domain-separated BLAKE3 hash, whose hexadecimal nibbles are the
/// key's path through the index.
pub fn production_archive_lookup_key_digest(
    key: &ProductionTerminalArchiveKeyV1,
) -> Result<String, CanwuError> {
    canonical_hash(PRODUCTION_ARCHIVE_LOOKUP_KEY_DOMAIN, key)
}

/// One terminal archive candidate of a production root, in selection order.
enum ArchiveCandidate<'a> {
    Receipt(&'a ProductionArchiveReceiptId),
    Execution(&'a ProductionExecutionId),
    FacilityProject(&'a crate::FacilityProjectId),
    Residue(ProductionTerminalArchiveKeyV1),
}

impl PreparedProductionArchiveBatchV1 {
    /// Every blob of the batch with its membership and temporal page, in
    /// order.
    fn pages(
        &self,
    ) -> impl Iterator<
        Item = (
            &ProductionArchiveBlobV1,
            &ProductionArchiveMembershipPageV1,
            &ProductionArchiveTemporalPageV1,
        ),
    > {
        std::iter::once((&self.blob, &self.membership_page, &self.temporal_page)).chain(
            self.additional_pages
                .iter()
                .map(|page| (&page.blob, &page.membership_page, &page.temporal_page)),
        )
    }

    /// Terminal records the batch archives.
    #[must_use]
    pub fn selected_count(&self) -> usize {
        self.selected_receipts.len()
            + self.selected.len()
            + self.selected_projects.len()
            + self.selected_residues.len()
    }

    /// Checks that the batch is self-consistent before anything is stored:
    /// page shape and bounds, the selection lists against the blobs, the
    /// directory's object lists, the lookup page ids, and the retention
    /// handle. [`Self::store_and_verify`] authenticates the records and the
    /// stored index after writing them.
    pub fn validate(&self) -> Result<(), CanwuError> {
        let mut selected_receipts = Vec::new();
        let mut selected = Vec::new();
        let mut selected_projects = Vec::new();
        let mut selected_residues = Vec::new();
        let mut blob_ids = Vec::new();
        let mut membership_page_ids = Vec::new();
        let mut temporal_page_ids = Vec::new();
        let mut pages_bind_their_blobs = true;
        for (blob, membership_page, temporal_page) in self.pages() {
            let records = blob_record_count(blob);
            selected_residues.extend(blob.residue_records.iter().map(|record| record.key.clone()));
            selected.extend(blob.records.iter().map(|record| match &record.key {
                ProductionTerminalArchiveKeyV1::Execution(id) => Some(id.clone()),
                _ => None,
            }));
            selected_projects.extend(blob.project_records.iter().map(|record| match &record.key {
                ProductionTerminalArchiveKeyV1::FacilityProject(id) => Some(id.clone()),
                _ => None,
            }));
            selected_receipts.extend(blob.receipts.iter().map(|receipt| receipt.id.clone()));
            pages_bind_their_blobs &= records > 0
                && records <= MAX_ARCHIVE_PAGE_ENTRIES
                && blob.expected_source_root == self.expected_source_root
                && membership_page.memberships.len() == records
                && temporal_page.entries.len() == records;
            blob_ids.push(blob.content_id.clone());
            membership_page_ids.push(membership_page.id.clone());
            temporal_page_ids.push(temporal_page.id.clone());
        }
        let selected = selected.into_iter().collect::<Option<Vec<_>>>();
        let selected_projects = selected_projects.into_iter().collect::<Option<Vec<_>>>();
        let expected_object_ids = expected_retention_objects(
            &blob_ids,
            &membership_page_ids,
            &temporal_page_ids,
            &self.directory.id,
        );
        if self.selected_count() == 0
            || self.selected_count() > MAX_ARCHIVE_PREPARE_CANDIDATES
            || self.selected_receipts.len() > 1
            || blob_ids.len() > MAX_ARCHIVE_COMMIT_PAGES
            || selected.as_ref() != Some(&self.selected)
            || selected_projects.as_ref() != Some(&self.selected_projects)
            || selected_receipts != self.selected_receipts
            || selected_residues != self.selected_residues
            || !pages_bind_their_blobs
            || self.expected_source_root.len() != 64
            || self.directory.previous_root.as_ref() == Some(&self.directory.id)
            || self.directory.blob_ids != blob_ids
            || self.directory.membership_pages != membership_page_ids
            || self.directory.temporal_pages != temporal_page_ids
            || !self.lookup_pages.contains_key(&self.directory.lookup_root)
            || self.retention.phase != ProductionArchiveRetentionPhaseV1::Prepared
            || self.retention.source_root != self.expected_source_root
            || self.retention.target_directory_root != self.directory.id
            || self.retention.object_ids != expected_object_ids
            || self.retention.handle_id
                != canonical_hash(
                    "canwu.production.archive-retention-id.v1",
                    &(&self.expected_source_root, &self.directory.id),
                )?
        {
            return Err(invalid(
                "prepared production archive batch is forged or lacks exact closure",
            ));
        }
        for (id, page) in &self.lookup_pages {
            if &page.id().map_err(|error| lookup_error(&error))? != id {
                return Err(invalid("prepared production archive lookup page is forged"));
            }
        }
        let mut detached = self.retention.clone();
        let digest = std::mem::take(&mut detached.semantic_digest);
        if digest != canonical_hash("canwu.production.archive-retention.v1", &detached)? {
            return Err(invalid(
                "prepared production archive retention handle is forged",
            ));
        }
        Ok(())
    }

    /// Stores every object of the batch, reads each back, and authenticates
    /// the commit: its directory, blobs, typed records, and pages bind
    /// exactly the selected records, and each of them resolves through the
    /// stored lookup index to its own blob position.
    ///
    /// The batch is refused before anything is written when it would write
    /// more than [`MAX_ARCHIVE_OBJECTS_PER_BATCH`] objects or
    /// [`MAX_ARCHIVE_BYTES_PER_BATCH`] bytes; prepare a smaller one.
    pub fn store_and_verify(
        &self,
        store: &dyn ProductionArchiveStore,
    ) -> Result<VerifiedProductionArchiveCommitV1, CanwuError> {
        self.validate()?;
        let encode = |value: Result<Vec<u8>, serde_json::Error>| {
            value.map_err(|error| {
                invalid(format!(
                    "production archive object could not be encoded: {error}"
                ))
            })
        };
        let mut objects = Vec::new();
        for (blob, membership_page, temporal_page) in self.pages() {
            objects.push((
                PRODUCTION_ARCHIVE_BLOB_NAMESPACE,
                blob.content_id.as_str(),
                encode(serde_json::to_vec(blob))?,
            ));
            objects.push((
                PRODUCTION_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE,
                membership_page.id.as_str(),
                encode(serde_json::to_vec(membership_page))?,
            ));
            objects.push((
                PRODUCTION_ARCHIVE_TEMPORAL_PAGE_NAMESPACE,
                temporal_page.id.as_str(),
                encode(serde_json::to_vec(temporal_page))?,
            ));
        }
        for (id, page) in &self.lookup_pages {
            objects.push((
                PRODUCTION_ARCHIVE_LOOKUP_PAGE_NAMESPACE,
                id.as_str(),
                encode(serde_json::to_vec(page))?,
            ));
        }
        // The directory goes last: once it is stored, everything it names is.
        objects.push((
            PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            self.directory.id.as_str(),
            encode(serde_json::to_vec(&self.directory))?,
        ));
        let bytes = objects
            .iter()
            .try_fold(0_usize, |total, (_, _, bytes)| {
                total.checked_add(bytes.len())
            })
            .ok_or_else(|| invalid("production archive byte count overflowed"))?;
        if objects.len() > MAX_ARCHIVE_OBJECTS_PER_BATCH {
            return Err(invalid("production archive object budget was exceeded"));
        }
        if bytes > MAX_ARCHIVE_BYTES_PER_BATCH {
            return Err(invalid("production archive byte budget was exceeded"));
        }
        store.persist_production_archive_retention(&self.retention)?;
        for (namespace, object_id, bytes) in &objects {
            store.store_production_archive_object(namespace, object_id, bytes)?;
            let loaded = store
                .load_production_archive_object(namespace, object_id)?
                .ok_or_else(|| invalid("production archive readback object is unavailable"))?;
            if &loaded != bytes {
                return Err(invalid(
                    "production archive readback bytes differ from the stored object",
                ));
            }
        }
        let loaded: ProductionArchiveIndexDirectoryV1 = load_encoded(
            store,
            PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            &self.directory.id,
        )?
        .ok_or_else(|| invalid("production archive directory readback is unavailable"))?;
        if loaded != self.directory {
            return Err(invalid("production archive directory readback differs"));
        }
        let closure = authenticate_production_archive_directory_closure(store, &loaded)?;
        let mut expected = closure
            .records
            .into_iter()
            .map(|(key, blob_id, ordinal)| {
                Ok(ResourceArchiveLookupEntryV1 {
                    key_digest: production_archive_lookup_key_digest(&key)?,
                    blob_id,
                    ordinal,
                })
            })
            .collect::<Result<Vec<_>, CanwuError>>()?;
        expected.sort();
        canwu_resource::verify_resource_archive_lookup_entries(
            &ProductionLookupPages(store),
            &loaded.lookup_root,
            "",
            &expected,
        )
        .map_err(|error| lookup_error(&error))?;
        let mut retention = self.retention.clone();
        retention.phase = ProductionArchiveRetentionPhaseV1::Verified;
        retention.semantic_digest.clear();
        retention.semantic_digest =
            canonical_hash("canwu.production.archive-retention.v1", &retention)?;
        store.persist_production_archive_retention(&retention)?;
        Ok(VerifiedProductionArchiveCommitV1 {
            scope: self.scope.clone(),
            expected_source_root: self.expected_source_root.clone(),
            selected_receipts: self.selected_receipts.clone(),
            selected: self.selected.clone(),
            selected_projects: self.selected_projects.clone(),
            selected_residues: self.selected_residues.clone(),
            directory_root: self.directory.id.clone(),
            lookup_root: self.directory.lookup_root.clone(),
            retention,
        })
    }
}

impl VerifiedProductionArchiveCommitV1 {
    /// Terminal records the commit archives when it applies.
    #[must_use]
    pub fn selected_count(&self) -> usize {
        self.selected_receipts.len()
            + self.selected.len()
            + self.selected_projects.len()
            + self.selected_residues.len()
    }

    /// Checks that the commit is self-consistent: between one and
    /// [`MAX_ARCHIVE_PREPARE_CANDIDATES`] distinct selected records with at
    /// most one receipt, and a verified retention handle bound to the same
    /// source root and directory. It reads no store.
    pub fn validate(&self) -> Result<(), CanwuError> {
        let mut detached = self.retention.clone();
        let digest = std::mem::take(&mut detached.semantic_digest);
        let unique = |count: usize, distinct: usize| count == distinct;
        if self.selected_count() == 0
            || self.selected_count() > MAX_ARCHIVE_PREPARE_CANDIDATES
            || self.selected_receipts.len() > 1
            || !unique(
                self.selected.len(),
                self.selected.iter().collect::<BTreeSet<_>>().len(),
            )
            || !unique(
                self.selected_projects.len(),
                self.selected_projects.iter().collect::<BTreeSet<_>>().len(),
            )
            || !unique(
                self.selected_residues.len(),
                self.selected_residues.iter().collect::<BTreeSet<_>>().len(),
            )
            || self.selected_residues.iter().any(|key| {
                !matches!(
                    key,
                    ProductionTerminalArchiveKeyV1::WorkOrder(_)
                        | ProductionTerminalArchiveKeyV1::CompletionLease(_)
                        | ProductionTerminalArchiveKeyV1::OperationOutcome(_)
                )
            })
            || self.expected_source_root.len() != 64
            || self.directory_root.len() != 64
            || !is_digest(&self.lookup_root)
            || digest != canonical_hash("canwu.production.archive-retention.v1", &detached)?
            || !matches!(
                self.retention.phase,
                ProductionArchiveRetentionPhaseV1::Verified
                    | ProductionArchiveRetentionPhaseV1::DurableIngress
            )
            || self.retention.source_root != self.expected_source_root
            || self.retention.target_directory_root != self.directory_root
            || self.retention.handle_id
                != canonical_hash(
                    "canwu.production.archive-retention-id.v1",
                    &(&self.expected_source_root, &self.directory_root),
                )?
            || self
                .retention
                .object_ids
                .get(PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE)
                != Some(&BTreeSet::from([self.directory_root.clone()]))
        {
            return Err(invalid(
                "production archive commit is forged or incompletely bound",
            ));
        }
        Ok(())
    }
}

fn expected_retention_objects(
    blob_ids: &[String],
    membership_pages: &[String],
    temporal_pages: &[String],
    directory: &str,
) -> BTreeMap<String, BTreeSet<String>> {
    BTreeMap::from([
        (
            PRODUCTION_ARCHIVE_BLOB_NAMESPACE.to_owned(),
            blob_ids.iter().cloned().collect(),
        ),
        (
            PRODUCTION_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE.to_owned(),
            membership_pages.iter().cloned().collect(),
        ),
        (
            PRODUCTION_ARCHIVE_TEMPORAL_PAGE_NAMESPACE.to_owned(),
            temporal_pages.iter().cloned().collect(),
        ),
        (
            PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
            BTreeSet::from([directory.to_owned()]),
        ),
    ])
}

impl ProductionState {
    /// The digest an archive batch is prepared from and applies against:
    /// the runtime revision, which every production transition advances, and
    /// the archive head and candidate indexes. A commit whose source root is
    /// not the root's current one settles as stale.
    pub fn archive_source_root(&self) -> Result<String, CanwuError> {
        canonical_hash(
            "canwu.production.archive-source-root.v2",
            &(
                &self.scope,
                self.revision,
                &self.archive_due_index,
                &self.archive.directory_root,
                self.archive.archived_execution_count,
                self.archive.archived_project_count,
                self.archive.committed_batch_count,
                &self.project_archive_due_index,
                self.archive.archived_receipt_count,
                self.archive.receipt_sequence,
            ),
        )
    }

    /// The hot receipt of an applied commit, which the next commit archives
    /// first: at most one, the receipt of the commit at the archive head.
    fn applied_archive_receipts(&self) -> impl Iterator<Item = &ProductionArchiveReceiptId> {
        self.archive
            .maintenance_receipts
            .values()
            .filter(|receipt| receipt.disposition == ProductionArchiveRetentionPhaseV1::Committed)
            .map(|receipt| &receipt.id)
    }

    /// Prepares one archive commit of the first `candidate_limit` terminal
    /// candidates: the receipt of the commit at the archive head, then
    /// terminal executions, then completed facility projects, each in
    /// identity order.
    ///
    /// The batch is cut into blobs of at most [`MAX_ARCHIVE_PAGE_ENTRIES`]
    /// records, each with its own membership and temporal page, so one commit
    /// carries up to [`MAX_ARCHIVE_PREPARE_CANDIDATES`] records. Its directory
    /// also commits to the cumulative lookup index extended by those records.
    /// Extending the index reads the archive head's directory and the lookup
    /// pages on the paths of the new keys from `store`; nothing is written
    /// until [`PreparedProductionArchiveBatchV1::store_and_verify`]. The
    /// result is a function of this state and the stored index alone.
    ///
    /// The receipt of the head commit is always a candidate, so once the
    /// archive has a head this succeeds even when nothing else is terminal: a
    /// batch whose `selected` and `selected_projects` are empty archives only
    /// that receipt. A host application commits when `archive_due_index` or
    /// `project_archive_due_index` holds records.
    ///
    /// It refuses, as `archive_backpressure`, while the root holds
    /// [`crate::ProductionLimitsV1::max_pending_retention_handles`] retention
    /// handles that the host application has not acknowledged.
    pub fn prepare_production_archive(
        &self,
        candidate_limit: usize,
        store: &dyn ProductionArchiveStore,
    ) -> Result<PreparedProductionArchiveBatchV1, CanwuError> {
        if candidate_limit == 0 || candidate_limit > MAX_ARCHIVE_PREPARE_CANDIDATES {
            return Err(invalid("production archive candidate budget is invalid"));
        }
        if self.archive.pending_handles.len() >= self.limits.max_pending_retention_handles {
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                "archive_backpressure: production archive retention capacity is exhausted",
            ));
        }
        let candidates = self
            .applied_archive_receipts()
            .map(ArchiveCandidate::Receipt)
            .chain(
                self.archive_due_index
                    .iter()
                    .map(ArchiveCandidate::Execution),
            )
            .chain(
                self.project_archive_due_index
                    .iter()
                    .map(ArchiveCandidate::FacilityProject),
            )
            .chain(self.residue_candidates().map(ArchiveCandidate::Residue))
            .take(candidate_limit)
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return Err(CanwuError::new(
                ErrorCode::DomainRecordNotFound,
                "production archive has no eligible terminal record",
            ));
        }
        let expected_source_root = self.archive_source_root()?;
        let mut selected_receipts = Vec::new();
        let mut selected = Vec::new();
        let mut selected_projects = Vec::new();
        let mut selected_residues = Vec::new();
        for candidate in &candidates {
            match candidate {
                ArchiveCandidate::Receipt(id) => selected_receipts.push((*id).clone()),
                ArchiveCandidate::Execution(id) => selected.push((*id).clone()),
                ArchiveCandidate::FacilityProject(id) => selected_projects.push((*id).clone()),
                ArchiveCandidate::Residue(key) => selected_residues.push(key.clone()),
            }
        }
        let pages = self.archive_pages(&expected_source_root, &candidates)?;
        let previous_lookup_root = match &self.archive.directory_root {
            Some(root) => {
                let previous = load_production_archive_directory(store, root)?;
                if previous.archived_execution_count != self.archive.archived_execution_count
                    || previous.archived_project_count != self.archive.archived_project_count
                    || previous.archived_receipt_count != self.archive.archived_receipt_count
                {
                    return Err(invalid(
                        "production archive head directory differs from the head record counts",
                    ));
                }
                Some(previous.lookup_root)
            }
            None => None,
        };
        let entries = pages
            .iter()
            .flat_map(|page| &page.membership_page.memberships)
            .map(|membership| {
                Ok(ResourceArchiveLookupEntryV1 {
                    key_digest: production_archive_lookup_key_digest(&membership.key)?,
                    blob_id: membership.blob_id.clone(),
                    ordinal: membership.ordinal,
                })
            })
            .collect::<Result<Vec<_>, CanwuError>>()?;
        let lookup = canwu_resource::prepare_resource_archive_lookup_entries(
            &ProductionLookupPages(store),
            previous_lookup_root.as_deref(),
            entries,
        )
        .map_err(|error| lookup_error(&error))?;
        let directory = self.archive_directory(&pages, lookup.root)?;
        let handle_id = canonical_hash(
            "canwu.production.archive-retention-id.v1",
            &(&expected_source_root, &directory.id),
        )?;
        let mut retention = ProductionArchiveRetentionHandleV1 {
            handle_id,
            source_root: expected_source_root.clone(),
            target_directory_root: directory.id.clone(),
            object_ids: expected_retention_objects(
                &directory.blob_ids,
                &directory.membership_pages,
                &directory.temporal_pages,
                &directory.id,
            ),
            phase: ProductionArchiveRetentionPhaseV1::Prepared,
            semantic_digest: String::new(),
        };
        retention.semantic_digest =
            canonical_hash("canwu.production.archive-retention.v1", &retention)?;
        let mut pages = pages.into_iter();
        let first = pages
            .next()
            .ok_or_else(|| invalid("production archive batch has no page"))?;
        Ok(PreparedProductionArchiveBatchV1 {
            scope: self.scope.clone(),
            expected_source_root,
            selected_receipts,
            selected,
            selected_projects,
            selected_residues,
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

    /// The blobs, with their membership and temporal pages, of a commit of
    /// `candidates` prepared from `source_root`, cut into pages of
    /// [`MAX_ARCHIVE_PAGE_ENTRIES`] in order.
    fn archive_pages(
        &self,
        source_root: &str,
        candidates: &[ArchiveCandidate<'_>],
    ) -> Result<Vec<PreparedProductionArchivePageV1>, CanwuError> {
        let mut pages = Vec::with_capacity(candidates.len().div_ceil(MAX_ARCHIVE_PAGE_ENTRIES));
        // The root's outcomes and lease receipts are grouped by the records
        // that claim them once for the whole commit.
        let claims = ArchiveClaims::of(self);
        for chunk in candidates.chunks(MAX_ARCHIVE_PAGE_ENTRIES) {
            let mut blob = ProductionArchiveBlobV1 {
                format_version: PRODUCTION_ARCHIVE_BLOB_FORMAT_VERSION,
                expected_source_root: source_root.to_owned(),
                records: Vec::new(),
                project_records: Vec::new(),
                receipts: Vec::new(),
                residue_records: Vec::new(),
                content_id: String::new(),
            };
            for candidate in chunk {
                match candidate {
                    ArchiveCandidate::Receipt(id) => {
                        blob.receipts.push(
                            self.archive
                                .maintenance_receipts
                                .get(*id)
                                .ok_or_else(|| {
                                    invalid("production archive candidate receipt is unavailable")
                                })?
                                .clone(),
                        );
                    }
                    ArchiveCandidate::Execution(id) => {
                        blob.records
                            .push(self.terminal_archive_record(id, &claims)?);
                    }
                    ArchiveCandidate::FacilityProject(id) => {
                        blob.project_records
                            .push(self.facility_project_archive_record(id, &claims)?);
                    }
                    ArchiveCandidate::Residue(key) => {
                        blob.residue_records
                            .push(self.residue_archive_record(key, &claims)?);
                    }
                }
            }
            blob.content_id = canonical_hash("canwu.production.archive-blob.v1", &blob)?;
            pages.push(prepare_archive_page(blob)?);
        }
        Ok(pages)
    }

    /// The directory of a commit of `pages` on top of this root's archive
    /// head, naming the lookup index `lookup_root`.
    fn archive_directory(
        &self,
        pages: &[PreparedProductionArchivePageV1],
        lookup_root: String,
    ) -> Result<ProductionArchiveIndexDirectoryV1, CanwuError> {
        let add = |total: u64, value: usize| {
            u64::try_from(value)
                .ok()
                .and_then(|value| total.checked_add(value))
                .ok_or_else(|| invalid("production archive count overflowed"))
        };
        let (executions, projects, receipts, residues) =
            pages
                .iter()
                .fold((0_usize, 0_usize, 0_usize, 0_usize), |totals, page| {
                    (
                        totals.0 + page.blob.records.len(),
                        totals.1 + page.blob.project_records.len(),
                        totals.2 + page.blob.receipts.len(),
                        totals.3 + page.blob.residue_records.len(),
                    )
                });
        let mut directory = ProductionArchiveIndexDirectoryV1 {
            id: String::new(),
            previous_root: self.archive.directory_root.clone(),
            blob_ids: pages
                .iter()
                .map(|page| page.blob.content_id.clone())
                .collect(),
            membership_pages: pages
                .iter()
                .map(|page| page.membership_page.id.clone())
                .collect(),
            temporal_pages: pages
                .iter()
                .map(|page| page.temporal_page.id.clone())
                .collect(),
            archived_execution_count: add(self.archive.archived_execution_count, executions)?,
            archived_project_count: add(self.archive.archived_project_count, projects)?,
            archived_receipt_count: add(self.archive.archived_receipt_count, receipts)?,
            archived_residue_count: add(self.archive.archived_residue_count, residues)?,
            lookup_root,
            semantic_digest: String::new(),
        };
        directory.semantic_digest =
            canonical_hash("canwu.production.archive-directory.v1", &directory)?;
        directory.id.clone_from(&directory.semantic_digest);
        Ok(directory)
    }

    /// Settles one verified archive commit against this root and records its
    /// maintenance receipt and retention handle.
    ///
    /// A commit prepared from the current root is applied: its records leave
    /// the root and the archive head moves to its directory. Any other commit
    /// is stale and changes nothing else. Neither reads the archive store,
    /// and neither can fail for capacity: an applied commit archives the
    /// receipt of the commit before it, so the root holds one applied
    /// receipt; a stale receipt is not an archive candidate and stays only
    /// until the host application acknowledges its retention handle. The
    /// batch an applied commit names was checked against this root and the
    /// store when it was queued ([`crate::enqueue_production_archive`]).
    pub(crate) fn apply_production_archive_commit(
        &mut self,
        commit: &VerifiedProductionArchiveCommitV1,
        at: SimTime,
    ) -> Result<ProductionArchiveMaintenanceReceiptV1, CanwuError> {
        commit.validate()?;
        if commit.scope != self.scope {
            return Err(invalid(
                "production archive commit names another production root",
            ));
        }
        let applied = self.archive_source_root()? == commit.expected_source_root;
        if applied {
            let head_receipts = self.applied_archive_receipts().cloned().collect::<Vec<_>>();
            if commit.selected_receipts != head_receipts {
                return Err(invalid(
                    "production archive commit does not archive the receipt of the head commit first",
                ));
            }
            // The blobs, pages, and directory of the selection are rebuilt
            // from this root, with the lookup root the commit names, and must
            // be the ones the commit stored. This reads no store.
            let candidates = commit
                .selected_receipts
                .iter()
                .map(ArchiveCandidate::Receipt)
                .chain(commit.selected.iter().map(ArchiveCandidate::Execution))
                .chain(
                    commit
                        .selected_projects
                        .iter()
                        .map(ArchiveCandidate::FacilityProject),
                )
                .chain(
                    commit
                        .selected_residues
                        .iter()
                        .cloned()
                        .map(ArchiveCandidate::Residue),
                )
                .collect::<Vec<_>>();
            let pages = self.archive_pages(&commit.expected_source_root, &candidates)?;
            let directory = self.archive_directory(&pages, commit.lookup_root.clone())?;
            if directory.id != commit.directory_root
                || commit.retention.object_ids
                    != expected_retention_objects(
                        &directory.blob_ids,
                        &directory.membership_pages,
                        &directory.temporal_pages,
                        &directory.id,
                    )
            {
                return Err(invalid(
                    "production archive commit does not bind the exact records of its selection",
                ));
            }
            for receipt in &commit.selected_receipts {
                self.archive.maintenance_receipts.remove(receipt);
            }
            // Each record leaves the root with exactly what its archived
            // record holds.
            for page in &pages {
                for record in &page.blob.records {
                    self.remove_hot_terminal_execution(record)?;
                }
                for record in &page.blob.project_records {
                    self.remove_hot_terminal_project(record)?;
                }
                for record in &page.blob.residue_records {
                    self.remove_residue(record)?;
                }
            }
            self.release_unbound_completion_leases(
                &pages
                    .iter()
                    .flat_map(|page| {
                        page.blob
                            .records
                            .iter()
                            .map(|record| &record.completion_acquisition.id)
                            .chain(
                                page.blob
                                    .project_records
                                    .iter()
                                    .map(|record| &record.completion_acquisition.id),
                            )
                    })
                    .cloned()
                    .collect(),
            );
            let add = |total: u64, value: usize| {
                u64::try_from(value)
                    .ok()
                    .and_then(|value| total.checked_add(value))
                    .ok_or_else(|| invalid("production archive count overflowed"))
            };
            self.archive.directory_root = Some(commit.directory_root.clone());
            self.archive.archived_execution_count =
                add(self.archive.archived_execution_count, commit.selected.len())?;
            self.archive.archived_project_count = add(
                self.archive.archived_project_count,
                commit.selected_projects.len(),
            )?;
            self.archive.archived_receipt_count = add(
                self.archive.archived_receipt_count,
                commit.selected_receipts.len(),
            )?;
            self.archive.archived_residue_count = add(
                self.archive.archived_residue_count,
                commit.selected_residues.len(),
            )?;
            self.archive.committed_batch_count = self
                .archive
                .committed_batch_count
                .checked_add(1)
                .ok_or_else(|| invalid("production archive batch count overflowed"))?;
        }
        let disposition = if applied {
            ProductionArchiveRetentionPhaseV1::Committed
        } else {
            ProductionArchiveRetentionPhaseV1::RejectedStale
        };
        // A stale commit under a handle that already has a stale receipt
        // counts on that receipt: queuing one batch again and again leaves
        // one receipt per handle.
        if !applied
            && let Some(existing) = self
                .archive
                .maintenance_receipts
                .values_mut()
                .find(|receipt| {
                    receipt.disposition == ProductionArchiveRetentionPhaseV1::RejectedStale
                        && receipt.retention_handle_id == commit.retention.handle_id
                })
        {
            existing.duplicate_settlements = existing
                .duplicate_settlements
                .checked_add(1)
                .ok_or_else(|| invalid("production archive duplicate count overflowed"))?;
            existing.canonical_digest.clear();
            existing.canonical_digest = canonical_hash(
                "canwu.production.archive-maintenance-receipt.v1",
                &*existing,
            )?;
            let receipt = existing.clone();
            self.revision = self
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("production runtime revision overflowed"))?;
            self.validate()?;
            return Ok(receipt);
        }
        self.archive.receipt_sequence = self
            .archive
            .receipt_sequence
            .checked_add(1)
            .ok_or_else(|| invalid("production archive receipt sequence overflowed"))?;
        let id = ProductionArchiveReceiptId::new(format!(
            "canwu.production:archive-receipt:{}",
            self.archive.receipt_sequence
        ))?;
        let mut receipt = ProductionArchiveMaintenanceReceiptV1 {
            id: id.clone(),
            retention_handle_id: commit.retention.handle_id.clone(),
            source_root: commit.expected_source_root.clone(),
            directory_root: commit.directory_root.clone(),
            archived_executions: if applied { commit.selected.len() } else { 0 },
            archived_projects: if applied {
                commit.selected_projects.len()
            } else {
                0
            },
            archived_receipts: if applied {
                commit.selected_receipts.len()
            } else {
                0
            },
            archived_residues: if applied {
                commit.selected_residues.len()
            } else {
                0
            },
            disposition,
            settled_at: at,
            duplicate_settlements: 0,
            canonical_digest: String::new(),
        };
        receipt.canonical_digest =
            canonical_hash("canwu.production.archive-maintenance-receipt.v1", &receipt)?;
        self.archive
            .maintenance_receipts
            .insert(id, receipt.clone());
        let mut handle = commit.retention.clone();
        handle.phase = disposition;
        handle.semantic_digest.clear();
        handle.semantic_digest = canonical_hash("canwu.production.archive-retention.v1", &handle)?;
        // The same batch queued twice shares one handle id: its stale
        // duplicate must not overwrite the handle of the applied commit.
        if applied || !self.archive.pending_handles.contains_key(&handle.handle_id) {
            self.archive
                .pending_handles
                .insert(handle.handle_id.clone(), handle);
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("production runtime revision overflowed"))?;
        self.validate()?;
        Ok(receipt)
    }

    /// Retires the recovery metadata of one settled archive commit after the
    /// host application finalized its store-side retention: the retention
    /// handle, and every stale receipt recorded under it.
    ///
    /// The same batch queued twice settles once as applied and once as stale
    /// under one handle id, so stale receipts are retired by handle. A handle
    /// the root does not hold, because the acknowledgement arrived twice or
    /// names nothing, changes nothing and returns `false`. The receipt of an
    /// applied commit is not retired here: the next commit archives it.
    pub fn acknowledge_production_archive_retention(
        &mut self,
        handle_id: &str,
    ) -> Result<bool, CanwuError> {
        if !self.archive.pending_handles.contains_key(handle_id) {
            return Ok(false);
        }
        if self
            .archive
            .pending_handles
            .get(handle_id)
            .is_some_and(|handle| {
                !matches!(
                    handle.phase,
                    ProductionArchiveRetentionPhaseV1::Committed
                        | ProductionArchiveRetentionPhaseV1::RejectedStale
                )
            })
        {
            return Err(invalid(
                "production archive retention is not terminal and cannot be acknowledged",
            ));
        }
        self.archive.pending_handles.remove(handle_id);
        self.archive.maintenance_receipts.retain(|_, receipt| {
            receipt.disposition != ProductionArchiveRetentionPhaseV1::RejectedStale
                || receipt.retention_handle_id != handle_id
        });
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("production runtime revision overflowed"))?;
        self.validate()?;
        Ok(true)
    }

    fn terminal_archive_record(
        &self,
        execution_id: &ProductionExecutionId,
        claims: &ArchiveClaims<'_>,
    ) -> Result<ProductionTerminalArchiveRecordV1, CanwuError> {
        let execution = self
            .executions
            .get(execution_id)
            .ok_or_else(|| invalid("terminal production execution is unavailable"))?;
        if !matches!(
            execution.lifecycle,
            WorkOrderLifecycle::Settled
                | WorkOrderLifecycle::Cancelled
                | WorkOrderLifecycle::Failed
        ) {
            return Err(invalid("production archive candidate is not terminal"));
        }
        let wip_id = WorkInProgressId::new(format!("canwu.production:wip:{execution_id}"))?;
        let wip = self
            .work_in_progress
            .get(&wip_id)
            .ok_or_else(|| invalid("terminal production execution lost its WIP receipt"))?;
        let work_order = self
            .work_orders
            .get(&execution.work_order)
            .ok_or_else(|| invalid("terminal production execution lost its work-order receipt"))?;
        let acquisition = self
            .completion_acquisitions
            .get(&execution.completion_certificate.acquisition)
            .ok_or_else(|| {
                invalid("terminal production execution lost its completion acquisition")
            })?;
        let production_grant = self
            .production_completion_grants
            .get(&execution.production_completion_grant)
            .ok_or_else(|| invalid("terminal production execution lost its production grant"))?;
        let operation_outcomes = claims.execution_outcomes(work_order, execution);
        let decision_receipts = crate::residue::decision_receipts_of(
            self,
            &crate::residue::resolved_tickets(&operation_outcomes),
        );
        let mut participant_grants = self
            .completion_participant_grants
            .get(&acquisition.id)
            .into_iter()
            .flat_map(std::collections::BTreeMap::values)
            .cloned()
            .collect::<Vec<_>>();
        participant_grants.sort_by(|left, right| left.participant.cmp(&right.participant));
        let completion_receipts = claims.lease_receipts(&acquisition.id);
        let mut record = ProductionTerminalArchiveRecordV1 {
            key: ProductionTerminalArchiveKeyV1::Execution(execution_id.clone()),
            work_order: execution.work_order.clone(),
            process: execution.process.clone(),
            site: execution.site.clone(),
            facility: execution.facility.clone(),
            lifecycle: execution.lifecycle,
            completed_units: wip.completed_units,
            total_units: wip.total_units,
            recoverable_input_quantity: wip.recoverable_input_quantity,
            non_recoverable_waste_quantity: wip.non_recoverable_waste_quantity,
            input_consumption_digests: execution
                .inputs
                .iter()
                .map(|input| input.consumption.semantic_digest.clone())
                .collect(),
            technology_digest: execution.technology.semantic_digest.clone(),
            output_outcome_digests: execution
                .output_outcomes
                .iter()
                .map(|outcome| outcome.semantic_digest.clone())
                .collect(),
            output_source: execution.output_source.clone(),
            work_order_record: work_order.clone(),
            execution_record: execution.clone(),
            work_in_progress_record: wip.clone(),
            operation_outcomes,
            completion_acquisition: acquisition.clone(),
            production_completion_grant: production_grant.clone(),
            participant_grants,
            completion_receipts,
            decision_receipts,
            terminal_at: execution.completed_at.unwrap_or(wip.updated_at),
            canonical_digest: String::new(),
        };
        record.canonical_digest =
            canonical_hash("canwu.production.terminal-archive-record.v1", &record)?;
        Ok(record)
    }

    fn facility_project_archive_record(
        &self,
        project_id: &crate::FacilityProjectId,
        claims: &ArchiveClaims<'_>,
    ) -> Result<ProductionFacilityProjectArchiveRecordV1, CanwuError> {
        let project = self
            .facility_projects
            .get(project_id)
            .ok_or_else(|| invalid("terminal facility project is unavailable"))?;
        if project.lifecycle != crate::FacilityProjectLifecycle::Completed
            || !self.project_archive_due_index.contains(project_id)
        {
            return Err(invalid(
                "facility project archive candidate is not terminal",
            ));
        }
        let resulting_asset = project
            .resulting_asset
            .clone()
            .ok_or_else(|| invalid("terminal facility project lost its authoritative result"))?;
        let acquisition = self
            .completion_acquisitions
            .get(&project.completion_certificate.acquisition)
            .ok_or_else(|| invalid("terminal facility project lost its completion acquisition"))?;
        let production_grant = self
            .production_completion_grants
            .get(&project.production_completion_grant)
            .ok_or_else(|| invalid("terminal facility project lost its completion grant"))?;
        let operation_outcomes = claims.project_outcomes(project);
        let mut participant_grants = self
            .completion_participant_grants
            .get(&acquisition.id)
            .into_iter()
            .flat_map(std::collections::BTreeMap::values)
            .cloned()
            .collect::<Vec<_>>();
        participant_grants.sort_by(|left, right| left.participant.cmp(&right.participant));
        if production_grant.state != canwu_resource::CompletionGrantStateV1::Completed
            || participant_grants.len() != 1
            || participant_grants[0].participant != canwu_resource::PLUGIN_NAME
            || participant_grants[0].grant.state
                != canwu_resource::CompletionGrantStateV1::Completed
        {
            return Err(invalid(
                "terminal facility project has not closed both completion grants",
            ));
        }
        let completion_receipts = claims.lease_receipts(&acquisition.id);
        let mut record = ProductionFacilityProjectArchiveRecordV1 {
            key: ProductionTerminalArchiveKeyV1::FacilityProject(project_id.clone()),
            project: project.clone(),
            resulting_asset,
            operation_outcomes,
            completion_acquisition: acquisition.clone(),
            production_completion_grant: production_grant.clone(),
            participant_grants,
            completion_receipts,
            terminal_at: project
                .completed_at
                .ok_or_else(|| invalid("terminal facility project lost its completion time"))?,
            canonical_digest: String::new(),
        };
        record.canonical_digest = canonical_hash(
            "canwu.production.facility-project-archive-record.v1",
            &record,
        )?;
        Ok(record)
    }

    /// Removes the execution `record` archives, with its work order, work in
    /// progress, allocations, local grant, outcomes, and decision receipts.
    /// Its completion lease leaves with it unless another hot record binds
    /// it; see [`Self::release_unbound_completion_leases`].
    fn remove_hot_terminal_execution(
        &mut self,
        record: &ProductionTerminalArchiveRecordV1,
    ) -> Result<(), CanwuError> {
        let execution_id = &record.execution_record.id;
        let execution = self
            .executions
            .remove(execution_id)
            .ok_or_else(|| invalid("production archive commit lost its execution"))?;
        if !self.archive_due_index.remove(execution_id) {
            return Err(invalid(
                "production archive commit names an execution outside the due index",
            ));
        }
        let wip_id = WorkInProgressId::new(format!("canwu.production:wip:{execution_id}"))?;
        self.work_in_progress.remove(&wip_id);
        for allocation in &execution.allocations {
            self.capacity_allocations.remove(allocation);
        }
        self.work_orders.remove(&execution.work_order);
        for outcome in &record.operation_outcomes {
            self.operation_outcomes.remove(&outcome.id);
        }
        for receipt in &record.decision_receipts {
            self.decision_receipts.remove(&receipt.ticket_id);
        }
        self.production_completion_grants
            .remove(&execution.production_completion_grant);
        Ok(())
    }

    fn remove_hot_terminal_project(
        &mut self,
        record: &ProductionFacilityProjectArchiveRecordV1,
    ) -> Result<(), CanwuError> {
        let project_id = &record.project.id;
        let project = self
            .facility_projects
            .get(project_id)
            .ok_or_else(|| invalid("production archive commit lost its facility project"))?
            .clone();
        if project.lifecycle != crate::FacilityProjectLifecycle::Completed
            || !self.project_archive_due_index.remove(project_id)
        {
            return Err(invalid(
                "production archive commit names a non-terminal facility project",
            ));
        }
        self.facility_projects.remove(project_id);
        self.resource_continuation_witnesses.remove(project_id);
        for outcome in &record.operation_outcomes {
            self.operation_outcomes.remove(&outcome.id);
        }
        self.production_completion_grants
            .remove(&project.production_completion_grant);
        Ok(())
    }

    /// Removes each completion lease of `acquisitions` that no hot execution
    /// or facility project binds any longer, with its certificate,
    /// participant grants, receipts, expiry entries, and target locks: the
    /// archived execution or project records them.
    fn release_unbound_completion_leases(
        &mut self,
        acquisitions: &BTreeSet<canwu_resource::CompletionLeaseAcquisitionId>,
    ) {
        if acquisitions.is_empty() {
            return;
        }
        let bound = self
            .executions
            .values()
            .map(|execution| &execution.completion_certificate.acquisition)
            .chain(
                self.facility_projects
                    .values()
                    .map(|project| &project.completion_certificate.acquisition),
            )
            .filter(|acquisition| acquisitions.contains(*acquisition))
            .cloned()
            .collect::<BTreeSet<_>>();
        let released = acquisitions
            .difference(&bound)
            .cloned()
            .collect::<BTreeSet<_>>();
        if released.is_empty() {
            return;
        }
        for acquisition in &released {
            self.production_completion_certificates.remove(acquisition);
            self.completion_participant_grants.remove(acquisition);
            self.completion_acquisitions.remove(acquisition);
        }
        self.completion_receipts
            .retain(|_, receipt| !released.contains(&receipt.acquisition));
        for due in self.completion_expiry_due.values_mut() {
            due.retain(|acquisition| !released.contains(acquisition));
        }
        self.completion_expiry_due
            .retain(|_, acquisitions| !acquisitions.is_empty());
        self.completion_target_locks
            .retain(|_, (_, grant)| self.production_completion_grants.contains_key(grant));
    }
}

/// Records one blob holds: executions, facility projects, receipts, and
/// residues.
fn blob_record_count(blob: &ProductionArchiveBlobV1) -> usize {
    blob.records.len()
        + blob.project_records.len()
        + blob.receipts.len()
        + blob.residue_records.len()
}

fn blob_entries(
    blob: &ProductionArchiveBlobV1,
) -> impl Iterator<Item = (ProductionTerminalArchiveKeyV1, &str, SimTime)> {
    blob.records
        .iter()
        .map(|record| {
            (
                record.key.clone(),
                record.canonical_digest.as_str(),
                record.terminal_at,
            )
        })
        .chain(blob.project_records.iter().map(|record| {
            (
                record.key.clone(),
                record.canonical_digest.as_str(),
                record.terminal_at,
            )
        }))
        .chain(blob.receipts.iter().map(|receipt| {
            (
                ProductionTerminalArchiveKeyV1::ArchiveMaintenanceReceipt(receipt.id.clone()),
                receipt.canonical_digest.as_str(),
                receipt.settled_at,
            )
        }))
        .chain(blob.residue_records.iter().map(|record| {
            (
                record.key.clone(),
                record.canonical_digest.as_str(),
                record.terminal_at,
            )
        }))
}

/// The membership and temporal pages of one prepared blob.
fn prepare_archive_page(
    blob: ProductionArchiveBlobV1,
) -> Result<PreparedProductionArchivePageV1, CanwuError> {
    let mut membership_page = ProductionArchiveMembershipPageV1 {
        id: String::new(),
        memberships: blob_entries(&blob)
            .enumerate()
            .map(|(ordinal, (key, digest, _))| {
                Ok(ProductionArchiveMembershipV1 {
                    key,
                    blob_id: blob.content_id.clone(),
                    ordinal: u16::try_from(ordinal)
                        .map_err(|_| invalid("production archive membership ordinal overflowed"))?,
                    semantic_digest: digest.to_owned(),
                })
            })
            .collect::<Result<Vec<_>, CanwuError>>()?,
        semantic_digest: String::new(),
    };
    membership_page.semantic_digest = canonical_hash(
        "canwu.production.archive-membership-page.v1",
        &membership_page,
    )?;
    membership_page.id = membership_page.semantic_digest.clone();
    let mut temporal_page = ProductionArchiveTemporalPageV1 {
        id: String::new(),
        entries: blob_entries(&blob)
            .map(|(key, _, terminal_at)| ProductionArchiveTemporalEntryV1 { terminal_at, key })
            .collect(),
        semantic_digest: String::new(),
    };
    temporal_page
        .entries
        .sort_by(|left, right| (left.terminal_at, &left.key).cmp(&(right.terminal_at, &right.key)));
    temporal_page.semantic_digest =
        canonical_hash("canwu.production.archive-temporal-page.v1", &temporal_page)?;
    temporal_page.id = temporal_page.semantic_digest.clone();
    Ok(PreparedProductionArchivePageV1 {
        blob,
        membership_page,
        temporal_page,
    })
}

/// What one authenticated archive commit holds.
struct ProductionArchiveCommitClosure {
    /// Every archived key with the blob and ordinal its membership binds.
    records: Vec<(ProductionTerminalArchiveKeyV1, String, u16)>,
    executions: u64,
    projects: u64,
    receipts: u64,
    residues: u64,
}

/// Authenticates one archive commit: the directory's own identity, every
/// blob and typed record it names, and the membership and temporal pages
/// that must cover exactly those records. An archived maintenance receipt
/// must be the receipt of the commit before this one.
///
/// The cumulative lookup index is not read here, only the shape of
/// `lookup_root`: a historical directory stays authentic after the lookup
/// pages that only it reaches are gone. [`validate_production_archive`]
/// audits the index of the head.
pub fn authenticate_production_archive_directory(
    store: &dyn ProductionArchiveStore,
    directory: &ProductionArchiveIndexDirectoryV1,
) -> Result<(), CanwuError> {
    authenticate_production_archive_directory_closure(store, directory).map(|_| ())
}

fn authenticate_production_archive_directory_identity(
    directory: &ProductionArchiveIndexDirectoryV1,
) -> Result<(), CanwuError> {
    let mut detached = directory.clone();
    let id = std::mem::take(&mut detached.id);
    let digest = std::mem::take(&mut detached.semantic_digest);
    let expected = canonical_hash("canwu.production.archive-directory.v1", &detached)?;
    if id != expected || digest != expected || !is_digest(&directory.lookup_root) {
        return Err(invalid("production archive directory root is forged"));
    }
    Ok(())
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Loads one directory by id and authenticates its identity only: the single
/// directory read of a lookup and of an archive preparation.
fn load_production_archive_directory(
    store: &dyn ProductionArchiveStore,
    root: &str,
) -> Result<ProductionArchiveIndexDirectoryV1, CanwuError> {
    let directory: ProductionArchiveIndexDirectoryV1 =
        load_encoded(store, PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, root)?
            .ok_or_else(|| invalid("production archive directory is unavailable"))?;
    authenticate_production_archive_directory_identity(&directory)?;
    if directory.id != root {
        return Err(invalid("production archive directory identity differs"));
    }
    Ok(directory)
}

#[allow(clippy::too_many_lines)]
fn authenticate_production_archive_directory_closure(
    store: &dyn ProductionArchiveStore,
    directory: &ProductionArchiveIndexDirectoryV1,
) -> Result<ProductionArchiveCommitClosure, CanwuError> {
    authenticate_production_archive_directory_identity(directory)?;
    let unique_blobs = directory.blob_ids.iter().collect::<BTreeSet<_>>();
    let unique_memberships = directory.membership_pages.iter().collect::<BTreeSet<_>>();
    let unique_temporal = directory.temporal_pages.iter().collect::<BTreeSet<_>>();
    if directory.blob_ids.is_empty()
        || directory.blob_ids.len() > MAX_ARCHIVE_COMMIT_PAGES
        || directory.membership_pages.len() != directory.blob_ids.len()
        || directory.temporal_pages.len() != directory.blob_ids.len()
        || unique_blobs.len() != directory.blob_ids.len()
        || unique_memberships.len() != directory.membership_pages.len()
        || unique_temporal.len() != directory.temporal_pages.len()
        || directory.previous_root.as_ref() == Some(&directory.id)
    {
        return Err(invalid(
            "production archive directory object lists are empty, duplicated, cyclic, or unbounded",
        ));
    }
    let mut archived_records = BTreeMap::new();
    let mut source_root: Option<String> = None;
    let (mut executions, mut projects, mut receipts, mut residues) = (0_u64, 0_u64, 0_u64, 0_u64);
    for blob_id in &directory.blob_ids {
        let blob: ProductionArchiveBlobV1 =
            load_encoded(store, PRODUCTION_ARCHIVE_BLOB_NAMESPACE, blob_id)?.ok_or_else(|| {
                invalid("production archive blob referenced by the directory is missing")
            })?;
        authenticate_production_archive_blob(&blob, blob_id)?;
        if source_root
            .as_ref()
            .is_some_and(|expected| expected != &blob.expected_source_root)
        {
            return Err(invalid(
                "production archive blobs of one commit differ in their source root",
            ));
        }
        source_root.get_or_insert_with(|| blob.expected_source_root.clone());
        for receipt in &blob.receipts {
            validate_archived_maintenance_receipt(receipt, directory)?;
        }
        executions += u64::try_from(blob.records.len())
            .map_err(|_| invalid("production archive count overflowed"))?;
        projects += u64::try_from(blob.project_records.len())
            .map_err(|_| invalid("production archive count overflowed"))?;
        receipts += u64::try_from(blob.receipts.len())
            .map_err(|_| invalid("production archive count overflowed"))?;
        residues += u64::try_from(blob.residue_records.len())
            .map_err(|_| invalid("production archive count overflowed"))?;
        for (ordinal, (key, digest, terminal_at)) in blob_entries(&blob).enumerate() {
            let ordinal = u16::try_from(ordinal)
                .map_err(|_| invalid("production archive record ordinal overflowed"))?;
            if archived_records
                .insert(
                    key,
                    (blob_id.clone(), ordinal, digest.to_owned(), terminal_at),
                )
                .is_some()
            {
                return Err(invalid(
                    "production archive contains a duplicate terminal key",
                ));
            }
        }
    }
    // A commit archives at most the receipt of the commit before it.
    if receipts > 1 {
        return Err(invalid(
            "production archive commit archives more than one maintenance receipt",
        ));
    }
    let mut membership_keys = BTreeSet::new();
    for page_id in &directory.membership_pages {
        let page: ProductionArchiveMembershipPageV1 =
            load_encoded(store, PRODUCTION_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE, page_id)?
                .ok_or_else(|| invalid("production archive membership page is missing"))?;
        if page.memberships.is_empty() || page.memberships.len() > MAX_ARCHIVE_PAGE_ENTRIES {
            return Err(invalid(
                "production archive membership page is empty or exceeds its cap",
            ));
        }
        let mut detached = page.clone();
        let id = std::mem::take(&mut detached.id);
        let digest = std::mem::take(&mut detached.semantic_digest);
        let expected = canonical_hash("canwu.production.archive-membership-page.v1", &detached)?;
        if id != *page_id || digest != expected || id != expected {
            return Err(invalid("production archive membership page is forged"));
        }
        for membership in &page.memberships {
            let Some((blob_id, ordinal, digest, _)) = archived_records.get(&membership.key) else {
                return Err(invalid(
                    "production archive membership names an unknown terminal key",
                ));
            };
            if &membership.blob_id != blob_id
                || membership.ordinal != *ordinal
                || &membership.semantic_digest != digest
                || !membership_keys.insert(membership.key.clone())
            {
                return Err(invalid(
                    "production archive membership does not bind its exact blob record",
                ));
            }
        }
    }
    let mut temporal_keys = BTreeSet::new();
    for page_id in &directory.temporal_pages {
        let page: ProductionArchiveTemporalPageV1 =
            load_encoded(store, PRODUCTION_ARCHIVE_TEMPORAL_PAGE_NAMESPACE, page_id)?
                .ok_or_else(|| invalid("production archive temporal page is missing"))?;
        if page.entries.is_empty() || page.entries.len() > MAX_ARCHIVE_PAGE_ENTRIES {
            return Err(invalid(
                "production archive temporal page is empty or exceeds its cap",
            ));
        }
        let mut detached = page.clone();
        let id = std::mem::take(&mut detached.id);
        let digest = std::mem::take(&mut detached.semantic_digest);
        let expected = canonical_hash("canwu.production.archive-temporal-page.v1", &detached)?;
        if id != *page_id || digest != expected || id != expected {
            return Err(invalid("production archive temporal page is forged"));
        }
        if page
            .entries
            .windows(2)
            .any(|pair| (pair[0].terminal_at, &pair[0].key) > (pair[1].terminal_at, &pair[1].key))
        {
            return Err(invalid("production archive temporal page is not ordered"));
        }
        for entry in &page.entries {
            let Some((_, _, _, terminal_at)) = archived_records.get(&entry.key) else {
                return Err(invalid(
                    "production archive temporal index names an unknown terminal key",
                ));
            };
            if &entry.terminal_at != terminal_at || !temporal_keys.insert(entry.key.clone()) {
                return Err(invalid(
                    "production archive temporal index does not bind its exact terminal record",
                ));
            }
        }
    }
    if membership_keys.len() != archived_records.len()
        || temporal_keys.len() != archived_records.len()
    {
        return Err(invalid(
            "production archive indexes do not cover the complete terminal blob set",
        ));
    }
    Ok(ProductionArchiveCommitClosure {
        records: archived_records
            .into_iter()
            .map(|(key, (blob_id, ordinal, _, _))| (key, blob_id, ordinal))
            .collect(),
        executions,
        projects,
        receipts,
        residues,
    })
}

/// Authenticates one blob read under `blob_id`: its content id, format, size,
/// and the digest and typed closure of every record in it.
fn authenticate_production_archive_blob(
    blob: &ProductionArchiveBlobV1,
    blob_id: &str,
) -> Result<(), CanwuError> {
    let mut detached = blob.clone();
    detached.content_id.clear();
    let records = blob_record_count(blob);
    if blob.content_id != blob_id
        || blob.content_id != canonical_hash("canwu.production.archive-blob.v1", &detached)?
        || blob.format_version != PRODUCTION_ARCHIVE_BLOB_FORMAT_VERSION
        || records == 0
        || records > MAX_ARCHIVE_PAGE_ENTRIES
        || blob.expected_source_root.len() != 64
    {
        return Err(invalid("production archive blob content ID is forged"));
    }
    for record in &blob.records {
        let mut detached = record.clone();
        let digest = std::mem::take(&mut detached.canonical_digest);
        if digest != canonical_hash("canwu.production.terminal-archive-record.v1", &detached)? {
            return Err(invalid("production archive terminal record is forged"));
        }
        validate_terminal_execution_archive_record(record)?;
    }
    for record in &blob.project_records {
        let mut detached = record.clone();
        let digest = std::mem::take(&mut detached.canonical_digest);
        if digest
            != canonical_hash(
                "canwu.production.facility-project-archive-record.v1",
                &detached,
            )?
        {
            return Err(invalid(
                "production facility-project archive record is forged",
            ));
        }
        validate_facility_project_archive_record(record)?;
    }
    for receipt in &blob.receipts {
        let mut detached = receipt.clone();
        let digest = std::mem::take(&mut detached.canonical_digest);
        if digest != canonical_hash("canwu.production.archive-maintenance-receipt.v1", &detached)? {
            return Err(invalid("production archive maintenance receipt is forged"));
        }
    }
    for record in &blob.residue_records {
        let mut detached = record.clone();
        let digest = std::mem::take(&mut detached.canonical_digest);
        if digest != canonical_hash("canwu.production.residue-archive-record.v1", &detached)? {
            return Err(invalid("production residue archive record is forged"));
        }
        validate_residue_archive_record(record)?;
    }
    Ok(())
}

/// An archived maintenance receipt is the receipt of the applied commit just
/// before the commit `directory` that archived it.
fn validate_archived_maintenance_receipt(
    receipt: &ProductionArchiveMaintenanceReceiptV1,
    directory: &ProductionArchiveIndexDirectoryV1,
) -> Result<(), CanwuError> {
    if receipt.disposition != ProductionArchiveRetentionPhaseV1::Committed
        || directory.previous_root.as_ref() != Some(&receipt.directory_root)
        || !is_digest(&receipt.retention_handle_id)
        || !is_digest(&receipt.source_root)
        || receipt
            .archived_executions
            .saturating_add(receipt.archived_projects)
            .saturating_add(receipt.archived_receipts)
            .saturating_add(receipt.archived_residues)
            == 0
    {
        return Err(invalid(
            "production archived maintenance receipt is not the receipt of the commit before",
        ));
    }
    Ok(())
}

fn validate_terminal_execution_archive_record(
    record: &ProductionTerminalArchiveRecordV1,
) -> Result<(), CanwuError> {
    let execution = &record.execution_record;
    let order = &record.work_order_record;
    let wip = &record.work_in_progress_record;
    let expected_wip = WorkInProgressId::new(format!("canwu.production:wip:{}", execution.id))?;
    let expected_input_digests = execution
        .inputs
        .iter()
        .map(|input| input.consumption.semantic_digest.clone())
        .collect::<Vec<_>>();
    let expected_output_digests = execution
        .output_outcomes
        .iter()
        .map(|outcome| outcome.semantic_digest.clone())
        .collect::<Vec<_>>();
    if record.key != ProductionTerminalArchiveKeyV1::Execution(execution.id.clone())
        || record.work_order != execution.work_order
        || record.process != execution.process
        || record.site != execution.site
        || record.facility != execution.facility
        || record.lifecycle != execution.lifecycle
        || !matches!(
            execution.lifecycle,
            WorkOrderLifecycle::Settled
                | WorkOrderLifecycle::Cancelled
                | WorkOrderLifecycle::Failed
        )
        || order.id != execution.work_order
        || order.process != execution.process
        || order.site != execution.site
        || order.lifecycle != execution.lifecycle
        || order.execution.as_ref() != Some(&execution.id)
        || wip.id != expected_wip
        || wip.execution != execution.id
        || record.completed_units != wip.completed_units
        || record.total_units != wip.total_units
        || record.recoverable_input_quantity != wip.recoverable_input_quantity
        || record.non_recoverable_waste_quantity != wip.non_recoverable_waste_quantity
        || wip.completed_units > wip.total_units
        || wip.consumed_input_evidence != execution.inputs
        || record.input_consumption_digests != expected_input_digests
        || record.technology_digest != execution.technology.semantic_digest
        || record.output_outcome_digests != expected_output_digests
        || record.output_source != execution.output_source
        || record.terminal_at != execution.completed_at.unwrap_or(wip.updated_at)
    {
        return Err(invalid(
            "production execution archive identity, lifecycle, work, or evidence closure is invalid",
        ));
    }
    match execution.lifecycle {
        WorkOrderLifecycle::Settled
            if wip.completed_units == wip.total_units
                && execution.completed_at.is_some()
                && execution.output_settled_at >= execution.completed_at
                && execution.output_outcomes.len() == execution.output_requests.len()
                && !execution.output_outcomes.is_empty()
                && execution.output_source.is_some()
                && execution.output_ack_digest.is_some() =>
        {
            validate_terminal_output_closure(execution)?;
        }
        WorkOrderLifecycle::Cancelled | WorkOrderLifecycle::Failed
            if execution.output_outcomes.is_empty()
                && execution.output_settled_at.is_none()
                && execution.output_source.is_none()
                && execution.output_ack_digest.is_none() => {}
        _ => {
            return Err(invalid(
                "production execution archive terminal result closure is invalid",
            ));
        }
    }
    crate::model::validate_completion_certificate(execution, execution.started_at)?;
    let acquisition = &record.completion_acquisition;
    let local = &record.production_completion_grant;
    let certificate = &execution.completion_certificate;
    let completion_units = acquisition
        .recipe
        .canonical_units()
        .map_err(|error| invalid(error.to_string()))?;
    let expected_grants = BTreeMap::from([
        (
            crate::PLUGIN_NAME.to_owned(),
            execution.production_completion_grant.clone(),
        ),
        (
            canwu_resource::PLUGIN_NAME.to_owned(),
            execution.resource_completion_grant.clone(),
        ),
    ]);
    let prepared_revisions = certificate
        .prepared_grants
        .iter()
        .cloned()
        .collect::<BTreeMap<_, _>>();
    if acquisition.id != certificate.acquisition
        || acquisition.operation_key != execution.completion_certificate.operation_key
        || acquisition.holder != order.holder
        || acquisition.operation_namespace != crate::PRODUCTION_COMPLETION_OPERATION_NAMESPACE
        || acquisition.eligibility_time != execution.started_at
        || acquisition.recipe_digest
            != acquisition
                .recipe
                .digest()
                .map_err(|error| invalid(error.to_string()))?
        || acquisition.state != canwu_resource::CompletionLeaseAcquisitionStateV1::Released
        || acquisition.blocker.is_some()
        || acquisition.refunded_units != 0
        || acquisition.expected_participants
            != BTreeSet::from([
                crate::PLUGIN_NAME.to_owned(),
                canwu_resource::PLUGIN_NAME.to_owned(),
            ])
        || acquisition.grants != expected_grants
        || certificate.acquisition_revision.get() >= acquisition.revision.get()
        || certificate.recipe_digest != acquisition.recipe_digest
        || certificate.eligibility_envelope_digest != acquisition.eligibility_envelope.digest
        || certificate.eligibility_time != acquisition.eligibility_time
        || prepared_revisions.len() != 2
        || local.id != execution.production_completion_grant
        || local.acquisition != acquisition.id
        || local.operation_key != execution.completion_certificate.operation_key
        || local.owner_plugin != crate::PLUGIN_NAME
        || local.state != canwu_resource::CompletionGrantStateV1::Completed
        || local.recipe_digest != acquisition.recipe_digest
        || local.reserved_units != completion_units
        || local.rejection.is_some()
        || prepared_revisions
            .get(&local.id)
            .is_none_or(|prepared| local.revision.get() != prepared.get().saturating_add(2))
        || local.target_versions.is_empty()
        || local
            .target_versions
            .iter()
            .any(|target| !certificate.locked_target_versions.contains(target))
        || record.participant_grants.len() != 1
    {
        return Err(invalid(
            "production execution archive completion acquisition is not closed",
        ));
    }
    let participant = &record.participant_grants[0];
    if participant.participant != canwu_resource::PLUGIN_NAME
        || participant.grant.id != execution.resource_completion_grant
        || participant.grant.acquisition != acquisition.id
        || participant.grant.operation_key != execution.completion_certificate.operation_key
        || participant.grant.owner_plugin != canwu_resource::PLUGIN_NAME
        || participant.grant.state != canwu_resource::CompletionGrantStateV1::Completed
        || participant.grant.recipe_digest != acquisition.recipe_digest
        || participant.grant.reserved_units != completion_units
        || participant.grant.rejection.is_some()
        || prepared_revisions
            .get(&participant.grant.id)
            .is_none_or(|prepared| {
                participant.grant.revision.get() != prepared.get().saturating_add(2)
            })
        || participant.grant.target_versions.is_empty()
        || participant
            .grant
            .target_versions
            .iter()
            .any(|target| !certificate.locked_target_versions.contains(target))
        || !participant
            .provider_source
            .record
            .kind
            .matches_type::<canwu_resource::ResourceRuntimeRecord>()
    {
        return Err(invalid(
            "production execution archive resource participant is not exact and terminal",
        ));
    }
    if record
        .operation_outcomes
        .windows(2)
        .any(|pair| pair[0].id >= pair[1].id)
        || record.operation_outcomes.iter().any(|outcome| {
            validate_archived_operation_outcome(outcome).is_err()
                || !(outcome_matches_execution(outcome, order, execution)
                    || outcome_matches_lease(outcome, &acquisition.id, &order.holder))
                || outcome.settled_at > record.terminal_at
                || match outcome.disposition {
                    crate::ProductionOperationDisposition::Applied
                    | crate::ProductionOperationDisposition::Duplicate => {
                        outcome.rejection_code.is_some() || outcome.rejection_message.is_some()
                    }
                    crate::ProductionOperationDisposition::Rejected => {
                        outcome.rejection_code.is_none() || outcome.rejection_message.is_none()
                    }
                }
        })
    {
        return Err(invalid(
            "production execution archive operation outcomes are not canonical",
        ));
    }
    let tickets = crate::residue::resolved_tickets(&record.operation_outcomes);
    if record
        .decision_receipts
        .windows(2)
        .any(|pair| pair[0].ticket_id >= pair[1].ticket_id)
        || record.decision_receipts.iter().any(|receipt| {
            let mut detached = receipt.clone();
            let recorded = std::mem::take(&mut detached.canonical_digest);
            !tickets.contains(&receipt.ticket_id)
                || canonical_hash("canwu.production.decision-receipt.v1", &detached)
                    .is_ok_and(|digest| digest != recorded)
        })
    {
        return Err(invalid(
            "production execution archive decision receipts are not canonical",
        ));
    }
    let known_grants = BTreeSet::from([
        execution.production_completion_grant.clone(),
        execution.resource_completion_grant.clone(),
    ]);
    let mut receipt_sequences = BTreeSet::new();
    let mut requested = false;
    let mut activated = false;
    let mut granted_grants = BTreeSet::new();
    let mut prepared_grants = BTreeSet::new();
    let mut consumed_grants = BTreeSet::new();
    let mut completed_grants = BTreeSet::new();
    if record.completion_receipts.len() != 9
        || record
            .completion_receipts
            .windows(2)
            .any(|pair| pair[0].sequence >= pair[1].sequence)
    {
        return Err(invalid(
            "production execution archive completion receipts are not canonical",
        ));
    }
    for receipt in &record.completion_receipts {
        let mut detached = receipt.clone();
        let digest = std::mem::take(&mut detached.semantic_digest);
        if !receipt_sequences.insert(receipt.sequence)
            || receipt.acquisition != acquisition.id
            || receipt.operation_key != execution.completion_certificate.operation_key
            || receipt
                .grant
                .as_ref()
                .is_some_and(|grant| !known_grants.contains(grant))
            || digest
                != canwu_resource::canonical_digest(
                    "canwu.resource.completion-lease-receipt.v1",
                    &detached,
                )
                .map_err(|error| invalid(error.to_string()))?
        {
            return Err(invalid(
                "production execution archive completion receipt is forged",
            ));
        }
        match (&receipt.action, &receipt.grant) {
            (canwu_resource::CompletionLeaseReceiptActionV1::Requested, None)
                if receipt.state
                    == canwu_resource::CompletionLeaseAcquisitionStateV1::Requested
                    && receipt.reserved_units == 0
                    && receipt.refunded_units == 0
                    && receipt.reason.is_none() =>
            {
                requested = true;
            }
            (canwu_resource::CompletionLeaseReceiptActionV1::Granted, Some(grant))
                if ((grant == &execution.production_completion_grant
                    && receipt.state
                        == canwu_resource::CompletionLeaseAcquisitionStateV1::PartiallyGranted
                    && receipt.reserved_units == completion_units)
                    || (grant == &execution.resource_completion_grant
                        && receipt.state
                            == canwu_resource::CompletionLeaseAcquisitionStateV1::FullyGranted
                        && receipt.reserved_units == 0))
                    && receipt.refunded_units == 0
                    && receipt.reason.is_none() =>
            {
                granted_grants.insert(grant.clone());
            }
            (canwu_resource::CompletionLeaseReceiptActionV1::Prepared, Some(grant))
                if ((grant == &execution.production_completion_grant
                    && receipt.state
                        == canwu_resource::CompletionLeaseAcquisitionStateV1::Preparing
                    && receipt.reserved_units == completion_units)
                    || (grant == &execution.resource_completion_grant
                        && receipt.state
                            == canwu_resource::CompletionLeaseAcquisitionStateV1::PreparedAll
                        && receipt.reserved_units == 0))
                    && receipt.refunded_units == 0
                    && receipt.reason.is_none() =>
            {
                prepared_grants.insert(grant.clone());
            }
            (canwu_resource::CompletionLeaseReceiptActionV1::Activated, None)
                if receipt.state
                    == canwu_resource::CompletionLeaseAcquisitionStateV1::Activated
                    && receipt.reserved_units == 0
                    && receipt.refunded_units == 0
                    && receipt.reason.is_none() =>
            {
                activated = true;
            }
            (canwu_resource::CompletionLeaseReceiptActionV1::Consumed, Some(grant))
                if grant == &execution.resource_completion_grant
                    && receipt.state
                        == canwu_resource::CompletionLeaseAcquisitionStateV1::Activated
                    && receipt.reserved_units == 0
                    && receipt.refunded_units == 0
                    && receipt.reason.is_none() =>
            {
                consumed_grants.insert(grant.clone());
            }
            (canwu_resource::CompletionLeaseReceiptActionV1::Completed, Some(grant))
                if (grant == &execution.production_completion_grant
                    || grant == &execution.resource_completion_grant)
                    && matches!(
                        receipt.state,
                        canwu_resource::CompletionLeaseAcquisitionStateV1::Activated
                            | canwu_resource::CompletionLeaseAcquisitionStateV1::Released
                    )
                    && (receipt.reserved_units == completion_units
                        || receipt.reserved_units == 0)
                    && receipt.refunded_units == 0
                    && receipt.reason.is_none() =>
            {
                completed_grants.insert(grant.clone());
            }
            _ => {
                return Err(invalid(
                    "production execution archive completion receipt action is not canonical",
                ));
            }
        }
    }
    if !requested
        || !activated
        || granted_grants != known_grants
        || prepared_grants != known_grants
        || consumed_grants != BTreeSet::from([execution.resource_completion_grant.clone()])
        || completed_grants != known_grants
    {
        return Err(invalid(
            "production execution archive completion receipt closure is incomplete",
        ));
    }
    Ok(())
}

pub(crate) fn validate_terminal_output_closure(
    execution: &crate::ProductionExecution,
) -> Result<(), CanwuError> {
    let source = execution
        .output_source
        .as_ref()
        .ok_or_else(|| invalid("settled production archive lost its exact output source"))?;
    // The credits settled at the instant the acknowledgement recorded: the
    // dispatch instant, or the instant a deferred batch came due.
    let at = execution
        .output_settled_at
        .ok_or_else(|| invalid("settled production archive lost its output settlement time"))?;
    let mut operation_keys = BTreeSet::new();
    for (request, outcome) in execution
        .output_requests
        .iter()
        .zip(&execution.output_outcomes)
    {
        let resource_request =
            canwu_resource::ResourceOperationRequestV1::Credit(request.resource_credit_request(
                source.clone(),
                execution.completion_certificate.clone(),
                at,
            ));
        let expected_request_digest = canwu_resource::canonical_digest(
            "canwu.resource.operation-request.v1",
            &resource_request,
        )
        .map_err(|error| invalid(error.to_string()))?;
        let mut detached = outcome.clone();
        let semantic_digest = std::mem::take(&mut detached.semantic_digest);
        if !operation_keys.insert(request.operation_key.clone())
            || outcome.operation_key != request.operation_key
            || outcome.request_digest != expected_request_digest
            || outcome.kind != canwu_resource::ResourceOperationKind::Credit
            || !matches!(
                outcome.status,
                canwu_resource::ResourceOperationStatus::Applied
                    | canwu_resource::ResourceOperationStatus::Duplicate
            )
            || outcome.quantity != request.quantity
            || outcome.remainder != 0
            || outcome.result_ref.is_some()
            || outcome.rejection_code.is_some()
            || outcome.rejection_reason.is_some()
            || outcome.exact_evidence != vec![source.clone()]
            || semantic_digest
                != canwu_resource::canonical_digest(
                    "canwu.resource.operation-outcome.v1",
                    &detached,
                )
                .map_err(|error| invalid(error.to_string()))?
        {
            return Err(invalid(
                "production archive output outcome is not the exact resource-owned settlement",
            ));
        }
    }
    let acknowledgement = crate::ProductionOutputAcknowledgement {
        execution: execution.id.clone(),
        production_source: source.clone(),
        outcomes: execution.output_outcomes.clone(),
        settled_at: at,
    };
    let expected_ack_digest = canonical_hash("canwu.production.output-ack.v1", &acknowledgement)?;
    if execution.output_ack_digest.as_deref() != Some(expected_ack_digest.as_str()) {
        return Err(invalid(
            "production archive output acknowledgement digest is not canonical",
        ));
    }
    Ok(())
}

fn validate_facility_project_archive_record(
    record: &ProductionFacilityProjectArchiveRecordV1,
) -> Result<(), CanwuError> {
    let project = &record.project;
    if record.key != ProductionTerminalArchiveKeyV1::FacilityProject(project.id.clone())
        || project.lifecycle != crate::FacilityProjectLifecycle::Completed
        || project.completed_at != Some(record.terminal_at)
        || project.resulting_asset.as_ref() != Some(&record.resulting_asset)
        || project.result_evidence_digest.as_deref()
            != Some(crate::model::facility_project_result_digest(project)?.as_str())
        || record.resulting_asset.id != project.facility
        || record.resulting_asset.site != project.site
        || record.resulting_asset.generation != project.resulting_generation
        || record.resulting_asset.lifecycle != crate::FacilityLifecycle::Operational
        || record.resulting_asset.condition_per_mille != 1_000
    {
        return Err(invalid(
            "production facility-project archive identity, terminal time, or result is invalid",
        ));
    }
    crate::model::validate_project_completion_certificate(project)?;
    let acquisition = &record.completion_acquisition;
    let local = &record.production_completion_grant;
    let certificate = &project.completion_certificate;
    let completion_units = acquisition
        .recipe
        .canonical_units()
        .map_err(|error| invalid(error.to_string()))?;
    let expected_grants = BTreeMap::from([
        (
            crate::PLUGIN_NAME.to_owned(),
            project.production_completion_grant.clone(),
        ),
        (
            canwu_resource::PLUGIN_NAME.to_owned(),
            project.resource_completion_grant.clone(),
        ),
    ]);
    let prepared_revisions = certificate
        .prepared_grants
        .iter()
        .cloned()
        .collect::<BTreeMap<_, _>>();
    if acquisition.id != project.completion_certificate.acquisition
        || acquisition.operation_key != project.operation_key
        || acquisition.holder != project.holder
        || acquisition.operation_namespace != crate::PRODUCTION_COMPLETION_OPERATION_NAMESPACE
        || acquisition.eligibility_time != project.created_at
        || acquisition.recipe_digest
            != acquisition
                .recipe
                .digest()
                .map_err(|error| invalid(error.to_string()))?
        || acquisition.state != canwu_resource::CompletionLeaseAcquisitionStateV1::Released
        || acquisition.blocker.is_some()
        || acquisition.refunded_units != 0
        || acquisition.expected_participants
            != BTreeSet::from([
                crate::PLUGIN_NAME.to_owned(),
                canwu_resource::PLUGIN_NAME.to_owned(),
            ])
        || acquisition.grants != expected_grants
        || certificate.recipe_digest != acquisition.recipe_digest
        || certificate.eligibility_time != acquisition.eligibility_time
        || certificate.eligibility_envelope_digest != acquisition.eligibility_envelope.digest
        || certificate.acquisition_revision.get() >= acquisition.revision.get()
        || prepared_revisions.len() != 2
        || local.id != project.production_completion_grant
        || local.acquisition != acquisition.id
        || local.operation_key != project.operation_key
        || local.owner_plugin != crate::PLUGIN_NAME
        || local.state != canwu_resource::CompletionGrantStateV1::Completed
        || local.recipe_digest != acquisition.recipe_digest
        || local.reserved_units != completion_units
        || local.rejection.is_some()
        || prepared_revisions
            .get(&local.id)
            .is_none_or(|prepared| local.revision.get() != prepared.get().saturating_add(2))
        || local.target_versions.is_empty()
        || local.target_versions.iter().any(|target| {
            !matches!(
                target,
                canwu_resource::CompletionLockedTargetV1::ExternalRecord { version }
                    if version
                        .record
                        .kind
                        .matches_type::<crate::ProductionRuntimeRecord>()
            ) || !certificate.locked_target_versions.contains(target)
        })
        || record.participant_grants.len() != 1
    {
        return Err(invalid(
            "production facility-project archive completion acquisition is not closed",
        ));
    }
    let participant = &record.participant_grants[0];
    if participant.participant != canwu_resource::PLUGIN_NAME
        || participant.grant.id != project.resource_completion_grant
        || participant.grant.acquisition != acquisition.id
        || participant.grant.operation_key != project.operation_key
        || participant.grant.owner_plugin != canwu_resource::PLUGIN_NAME
        || participant.grant.state != canwu_resource::CompletionGrantStateV1::Completed
        || participant.grant.run_budget_revision != local.run_budget_revision
        || participant.grant.recipe_digest != acquisition.recipe_digest
        || participant.grant.reserved_units != completion_units
        || participant.grant.rejection.is_some()
        || prepared_revisions
            .get(&participant.grant.id)
            .is_none_or(|prepared| {
                participant.grant.revision.get() != prepared.get().saturating_add(2)
            })
        || participant.grant.target_versions.is_empty()
        || participant
            .grant
            .target_versions
            .iter()
            .any(|target| !certificate.locked_target_versions.contains(target))
        || project.inputs.iter().any(|input| {
            let target = canwu_resource::CompletionLockedTargetV1::AllocationLeg {
                id: input.allocation_leg.id.clone(),
                revision: input.allocation_leg.revision,
            };
            !participant.grant.target_versions.contains(&target)
                || !certificate.locked_target_versions.contains(&target)
        })
        || !participant
            .provider_source
            .record
            .kind
            .matches_type::<canwu_resource::ResourceRuntimeRecord>()
    {
        return Err(invalid(
            "production facility-project archive resource participant is not exact and terminal",
        ));
    }
    if record
        .operation_outcomes
        .windows(2)
        .any(|pair| pair[0].id >= pair[1].id)
        || record.operation_outcomes.iter().any(|outcome| {
            validate_archived_operation_outcome(outcome).is_err()
                || !(outcome_matches_project(outcome, project)
                    || outcome_matches_lease(
                        outcome,
                        &project.completion_certificate.acquisition,
                        &project.holder,
                    ))
                || outcome.settled_at > record.terminal_at
                || match outcome.disposition {
                    crate::ProductionOperationDisposition::Applied
                    | crate::ProductionOperationDisposition::Duplicate => {
                        outcome.rejection_code.is_some() || outcome.rejection_message.is_some()
                    }
                    crate::ProductionOperationDisposition::Rejected => {
                        outcome.rejection_code.is_none() || outcome.rejection_message.is_none()
                    }
                }
        })
    {
        return Err(invalid(
            "production facility-project archive operation outcomes are not canonical",
        ));
    }
    let mut receipt_sequences = BTreeSet::new();
    let mut requested = false;
    let mut activated = false;
    let mut consumed = false;
    let mut completed_grants = BTreeSet::new();
    let known_grants = BTreeSet::from([
        project.production_completion_grant.clone(),
        project.resource_completion_grant.clone(),
    ]);
    if record
        .completion_receipts
        .windows(2)
        .any(|pair| pair[0].sequence >= pair[1].sequence)
    {
        return Err(invalid(
            "production facility-project archive completion receipts are not canonical",
        ));
    }
    for receipt in &record.completion_receipts {
        let mut detached = receipt.clone();
        let digest = std::mem::take(&mut detached.semantic_digest);
        if !receipt_sequences.insert(receipt.sequence)
            || receipt.acquisition != acquisition.id
            || receipt.operation_key != project.operation_key
            || receipt
                .grant
                .as_ref()
                .is_some_and(|grant| !known_grants.contains(grant))
            || (receipt.action == canwu_resource::CompletionLeaseReceiptActionV1::Requested
                && receipt.grant.is_some())
            || digest
                != canwu_resource::canonical_digest(
                    "canwu.resource.completion-lease-receipt.v1",
                    &detached,
                )
                .map_err(|error| invalid(error.to_string()))?
        {
            return Err(invalid(
                "production facility-project archive completion receipt is forged",
            ));
        }
        requested |= receipt.action == canwu_resource::CompletionLeaseReceiptActionV1::Requested;
        activated |= receipt.action == canwu_resource::CompletionLeaseReceiptActionV1::Activated;
        consumed |= receipt.action == canwu_resource::CompletionLeaseReceiptActionV1::Consumed;
        if receipt.action == canwu_resource::CompletionLeaseReceiptActionV1::Completed
            && let Some(grant) = &receipt.grant
        {
            completed_grants.insert(grant.clone());
        }
    }
    if !requested || !activated || !consumed || completed_grants != known_grants {
        return Err(invalid(
            "production facility-project archive completion receipt closure is incomplete",
        ));
    }
    Ok(())
}

pub(crate) fn validate_archived_operation_outcome(
    outcome: &crate::ProductionOperationOutcome,
) -> Result<(), CanwuError> {
    if outcome.command.operation_id != outcome.id
        || outcome.canonical_input_hash
            != canonical_hash("canwu.production.operation-input.v1", &outcome.command)?
    {
        return Err(invalid(
            "production archive operation outcome is not bound to its exact canonical command",
        ));
    }
    Ok(())
}

pub(crate) fn outcome_matches_execution(
    outcome: &crate::ProductionOperationOutcome,
    order: &crate::WorkOrder,
    execution: &crate::ProductionExecution,
) -> bool {
    if outcome.command.holder != order.holder || outcome.project.is_some() {
        return false;
    }
    match &outcome.command.operation {
        crate::ProductionOperation::CreateWorkOrder { work_order } => {
            work_order.id == order.id
                && work_order.holder == order.holder
                && work_order.process == order.process
                && work_order.site == order.site
                && outcome.work_order.as_ref() == Some(&order.id)
                && outcome.execution.is_none()
        }
        crate::ProductionOperation::AuthorizeWorkOrder { work_order }
        | crate::ProductionOperation::CancelWorkOrder { work_order }
            if work_order == &order.id =>
        {
            outcome.work_order.as_ref() == Some(&order.id) && outcome.execution.is_none()
        }
        crate::ProductionOperation::ResolveDegradedFacility { work_order, .. }
            if work_order == &order.id =>
        {
            outcome.work_order.as_ref() == Some(&order.id) && outcome.execution.is_none()
        }
        crate::ProductionOperation::StartExecution {
            execution: started, ..
        } => {
            started.id == execution.id
                && started.work_order == order.id
                && started.process == execution.process
                && started.site == execution.site
                && started.facility == execution.facility
                && outcome.work_order.as_ref() == Some(&order.id)
                && outcome.execution.as_ref() == Some(&execution.id)
        }
        crate::ProductionOperation::AdvanceExecution {
            execution: advanced,
            ..
        }
        | crate::ProductionOperation::CompleteExecution {
            execution: advanced,
            ..
        } if advanced == &execution.id => {
            outcome.work_order.is_none() && outcome.execution.as_ref() == Some(&execution.id)
        }
        _ => false,
    }
}

pub(crate) fn outcome_matches_project(
    outcome: &crate::ProductionOperationOutcome,
    project: &crate::FacilityProject,
) -> bool {
    if outcome.command.holder != project.holder
        || outcome.work_order.is_some()
        || outcome.execution.is_some()
        || outcome.project.as_ref() != Some(&project.id)
    {
        return false;
    }
    match &outcome.command.operation {
        crate::ProductionOperation::CreateFacilityProject { project: created } => {
            created.id == project.id
                && created.holder == project.holder
                && created.site == project.site
                && created.facility == project.facility
                && created.kind == project.kind
                && created.process == project.process
                && created.base_generation == project.base_generation
                && created.resulting_generation == project.resulting_generation
                && created.created_at == project.created_at
        }
        crate::ProductionOperation::AuthorizeFacilityProject { project: id }
        | crate::ProductionOperation::AdvanceFacilityProject { project: id, .. }
        | crate::ProductionOperation::AcceptFacilityCommissioning { project: id } => {
            id == &project.id
        }
        _ => false,
    }
}

/// Authenticates every archive object reachable from the persisted production
/// archive head and every unacknowledged retention handle. Restore callers
/// must provide the durable store that owns those objects; a snapshot alone
/// is not enough to trust archive-backed state.
///
/// The directory chain is walked to its first commit, bounded by the root's
/// committed commit count, and each commit's blobs, typed records, and pages
/// are authenticated; the cumulative record counts must close. The lookup
/// index of the head is then rebuilt from the chain's records (a key
/// archived more than once counts by its newest record) and must be exactly
/// what the store holds: every page present and equal, and the same root.
/// The cost is linear in the archive, and the audit holds about 40 bytes per
/// archived record while it runs, so this is a restore and audit step, never
/// part of a boundary.
pub fn validate_production_archive(
    store: &dyn ProductionArchiveStore,
    state: &ProductionState,
) -> Result<(), CanwuError> {
    let mut next = state.archive.directory_root.clone();
    let mut expected = (
        state.archive.archived_execution_count,
        state.archive.archived_project_count,
        state.archive.archived_receipt_count,
        state.archive.archived_residue_count,
    );
    let mut visited = BTreeSet::new();
    let mut directories = 0_u64;
    let mut lookup_root = None;
    let mut blob_ids = Vec::new();
    let mut entries = Vec::new();
    while let Some(root) = next {
        if !visited.insert(root.clone()) {
            return Err(invalid("production archive directory chain is cyclic"));
        }
        directories = directories
            .checked_add(1)
            .ok_or_else(|| invalid("production archive directory count overflowed"))?;
        if directories > state.archive.committed_batch_count {
            return Err(invalid(
                "production archive directory chain exceeds its committed batch bound",
            ));
        }
        let directory: ProductionArchiveIndexDirectoryV1 =
            load_encoded(store, PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, &root)?
                .ok_or_else(|| invalid("production archive head directory is unavailable"))?;
        let closure = authenticate_production_archive_directory_closure(store, &directory)?;
        if directory.id != root
            || (
                directory.archived_execution_count,
                directory.archived_project_count,
                directory.archived_receipt_count,
                directory.archived_residue_count,
            ) != expected
        {
            return Err(invalid(
                "production archive directory chain count differs from its authoritative closure",
            ));
        }
        // The first commit archives no receipt, and every later one archives
        // the receipt of the commit before it.
        if closure.receipts != u64::from(directory.previous_root.is_some()) {
            return Err(invalid(
                "production archive commit does not archive the receipt of the commit before it",
            ));
        }
        lookup_root.get_or_insert_with(|| directory.lookup_root.clone());
        // Newest commit first, so the first entry kept for a key is the one
        // the index must name.
        let first_blob = blob_ids.len();
        blob_ids.extend(directory.blob_ids.iter().cloned());
        for (key, blob_id, ordinal) in closure.records {
            let blob = directory
                .blob_ids
                .iter()
                .position(|candidate| candidate == &blob_id)
                .and_then(|offset| first_blob.checked_add(offset))
                .and_then(|index| u32::try_from(index).ok())
                .ok_or_else(|| invalid("production archive blob position overflowed"))?;
            entries.push(
                canwu_resource::ResourceArchiveAuditEntry::new(
                    &production_archive_lookup_key_digest(&key)?,
                    blob,
                    ordinal,
                )
                .map_err(|error| lookup_error(&error))?,
            );
        }
        expected = (
            expected
                .0
                .checked_sub(closure.executions)
                .ok_or_else(|| invalid("production archive execution count underflowed"))?,
            expected
                .1
                .checked_sub(closure.projects)
                .ok_or_else(|| invalid("production archive project count underflowed"))?,
            expected
                .2
                .checked_sub(closure.receipts)
                .ok_or_else(|| invalid("production archive receipt count underflowed"))?,
            expected
                .3
                .checked_sub(closure.residues)
                .ok_or_else(|| invalid("production archive residue count underflowed"))?,
        );
        next = directory.previous_root;
    }
    if expected != (0, 0, 0, 0) || directories != state.archive.committed_batch_count {
        return Err(invalid(
            "production archive directory chain does not close its authoritative counts",
        ));
    }
    let rebuilt = canwu_resource::audit_resource_archive_lookup_index(
        &ProductionLookupPages(store),
        &blob_ids,
        entries,
    )
    .map_err(|error| lookup_error(&error))?;
    if rebuilt != lookup_root {
        return Err(invalid(
            "production archive lookup index differs from its directory chain",
        ));
    }
    for handle in state.archive.pending_handles.values() {
        for (namespace, object_ids) in &handle.object_ids {
            for object_id in object_ids {
                if store
                    .load_production_archive_object(namespace, object_id)?
                    .is_none()
                {
                    return Err(invalid(
                        "production pending retention handle references a missing archive object",
                    ));
                }
            }
        }
        let target: ProductionArchiveIndexDirectoryV1 = load_encoded(
            store,
            PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            &handle.target_directory_root,
        )?
        .ok_or_else(|| invalid("production pending retention directory is unavailable"))?;
        authenticate_production_archive_directory(store, &target)?;
        let expected_objects = expected_retention_objects(
            &target.blob_ids,
            &target.membership_pages,
            &target.temporal_pages,
            &target.id,
        );
        if handle.object_ids != expected_objects || handle.target_directory_root != target.id {
            return Err(invalid(
                "production pending retention handle does not close its exact archive objects and prior root",
            ));
        }
    }
    Ok(())
}

/// Resolves one archived terminal record through the cumulative lookup index
/// of the production archive head.
///
/// The lookup reads the head directory and one lookup page per index level,
/// each authenticated by its content id, and nothing else when the key was
/// never archived. Its cost follows the depth of the index, which grows with
/// the logarithm of the archived records and not at all with the number of
/// archive commits. A hit also reads the one blob that holds the record,
/// authenticates that blob, and validates the record's digest and typed
/// closure. A key archived more than once resolves to its newest record.
///
/// The number of objects read is bounded whatever the store returns: one
/// directory, at most
/// [`canwu_resource::MAX_RESOURCE_ARCHIVE_LOOKUP_DEPTH`]` + 1` lookup pages,
/// and one blob. A missing, forged, oversized, or malformed object is an
/// error, never a miss.
///
/// The lookup trusts the head directory to name an index that holds every
/// archived key. [`crate::enqueue_production_archive`] establishes that for
/// each commit it queues, and [`validate_production_archive`] checks it for
/// the head of a restored root.
pub fn archived_production_terminal_record(
    state: &ProductionState,
    store: &dyn ProductionArchiveStore,
    key: &ProductionTerminalArchiveKeyV1,
) -> Result<Option<ProductionArchivedRecordV1>, CanwuError> {
    let Some(root) = &state.archive.directory_root else {
        return Ok(None);
    };
    let directory = load_production_archive_directory(store, root)?;
    let Some(entry) = canwu_resource::find_resource_archive_lookup_entry(
        &ProductionLookupPages(store),
        &directory.lookup_root,
        &production_archive_lookup_key_digest(key)?,
    )
    .map_err(|error| lookup_error(&error))?
    else {
        return Ok(None);
    };
    let blob: ProductionArchiveBlobV1 =
        load_encoded(store, PRODUCTION_ARCHIVE_BLOB_NAMESPACE, &entry.blob_id)?
            .ok_or_else(|| invalid("production archive lookup blob is missing"))?;
    authenticate_production_archive_blob(&blob, &entry.blob_id)?;
    let ordinal = usize::from(entry.ordinal);
    let (executions, projects, receipts) = (
        blob.records.len(),
        blob.project_records.len(),
        blob.receipts.len(),
    );
    let record = if ordinal < executions {
        blob.records
            .into_iter()
            .nth(ordinal)
            .map(ProductionArchivedRecordV1::Execution)
    } else if ordinal - executions < projects {
        blob.project_records
            .into_iter()
            .nth(ordinal - executions)
            .map(ProductionArchivedRecordV1::FacilityProject)
    } else if ordinal - executions - projects < receipts {
        blob.receipts
            .into_iter()
            .nth(ordinal - executions - projects)
            .map(ProductionArchivedRecordV1::ArchiveMaintenanceReceipt)
    } else {
        blob.residue_records
            .into_iter()
            .nth(ordinal - executions - projects - receipts)
            .map(ProductionArchivedRecordV1::Residue)
    }
    .ok_or_else(|| invalid("production archive lookup ordinal is outside its blob"))?;
    let found = match &record {
        ProductionArchivedRecordV1::Execution(record) => record.key.clone(),
        ProductionArchivedRecordV1::FacilityProject(record) => record.key.clone(),
        ProductionArchivedRecordV1::ArchiveMaintenanceReceipt(receipt) => {
            ProductionTerminalArchiveKeyV1::ArchiveMaintenanceReceipt(receipt.id.clone())
        }
        ProductionArchivedRecordV1::Residue(record) => record.key.clone(),
    };
    if &found != key {
        return Err(invalid(
            "production archive lookup entry differs from its typed record",
        ));
    }
    Ok(Some(record))
}

/// The lookup pages a live run reads: the pages reachable from the index of
/// the archive head and of each named queued commit.
///
/// A commit is queued when its store-side retention handle is
/// `DurableIngress` and the root holds no retention handle with its id. A
/// queued commit may become the head, so its pages are live; one the caller
/// does not name is not covered. Every page of a named index must be
/// present: a missing one is an error. A host application that lost the
/// `superseded_lookup_pages` of a batch uses this to recompute what is live;
/// whether a page outside the result may be deleted is still decided by the
/// retention rule of
/// [`PreparedProductionArchiveBatchV1::superseded_lookup_pages`].
pub fn live_production_archive_lookup_pages<'a>(
    state: &ProductionState,
    store: &dyn ProductionArchiveStore,
    queued_directory_roots: impl IntoIterator<Item = &'a str>,
) -> Result<BTreeSet<String>, CanwuError> {
    let mut pages = BTreeSet::new();
    let mut collect = |root: &str| {
        let directory = load_production_archive_directory(store, root)?;
        canwu_resource::collect_resource_archive_lookup_pages(
            &ProductionLookupPages(store),
            &directory.lookup_root,
            "",
            true,
            &mut pages,
        )
        .map_err(|error| lookup_error(&error))
    };
    if let Some(root) = &state.archive.directory_root {
        collect(root)?;
    }
    for root in queued_directory_roots {
        collect(root)?;
    }
    Ok(pages)
}

/// Every production archive object reachable from `roots` (the head, the
/// directories of unacknowledged retention handles, and the directories of
/// queued commits) and from the commits before them, for an offline
/// collection's mark set: directories, blobs, membership and temporal pages,
/// and lookup pages.
///
/// The index of each root in `complete_roots` (the head and every queued
/// commit, which may become the head) must be whole. The index of any other
/// directory is marked as far as the store still holds it, so a collection
/// never drops a page that a restore, replay, or fork through that commit
/// could read, and a host application that deleted superseded pages is not
/// refused.
pub(crate) fn reachable_production_archive_objects(
    store: &dyn ProductionArchiveStore,
    mut roots: BTreeSet<String>,
    complete_roots: &BTreeSet<String>,
    directory_bound: usize,
) -> Result<BTreeMap<&'static str, BTreeSet<String>>, CanwuError> {
    let mut objects = BTreeMap::<&'static str, BTreeSet<String>>::new();
    let mut lookup_roots = Vec::new();
    let mut visited = BTreeSet::new();
    while let Some(root) = roots.pop_first() {
        if !visited.insert(root.clone()) {
            continue;
        }
        if visited.len() > directory_bound {
            return Err(invalid(
                "production archive directory graph exceeds its committed bound",
            ));
        }
        let directory: ProductionArchiveIndexDirectoryV1 =
            load_encoded(store, PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, &root)?
                .ok_or_else(|| invalid("production archive directory is unavailable"))?;
        if directory.id != root {
            return Err(invalid("production archive directory identity differs"));
        }
        authenticate_production_archive_directory(store, &directory)?;
        lookup_roots.push((
            !complete_roots.contains(&root),
            directory.lookup_root.clone(),
        ));
        objects
            .entry(PRODUCTION_ARCHIVE_BLOB_NAMESPACE)
            .or_default()
            .extend(directory.blob_ids.iter().cloned());
        objects
            .entry(PRODUCTION_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE)
            .or_default()
            .extend(directory.membership_pages.iter().cloned());
        objects
            .entry(PRODUCTION_ARCHIVE_TEMPORAL_PAGE_NAMESPACE)
            .or_default()
            .extend(directory.temporal_pages.iter().cloned());
        objects
            .entry(PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE)
            .or_default()
            .insert(root);
        if let Some(previous) = directory.previous_root {
            roots.insert(previous);
        }
    }
    // Complete indexes first: a page they share with another index is then
    // already known to be whole.
    lookup_roots.sort();
    let mut lookup_pages = BTreeSet::new();
    for (earlier, lookup_root) in &lookup_roots {
        canwu_resource::collect_resource_archive_lookup_pages(
            &ProductionLookupPages(store),
            lookup_root,
            "",
            !earlier,
            &mut lookup_pages,
        )
        .map_err(|error| lookup_error(&error))?;
    }
    objects.insert(PRODUCTION_ARCHIVE_LOOKUP_PAGE_NAMESPACE, lookup_pages);
    Ok(objects)
}

fn load_encoded<T: DeserializeOwned>(
    store: &dyn ProductionArchiveStore,
    namespace: &str,
    object_id: &str,
) -> Result<Option<T>, CanwuError> {
    store
        .load_production_archive_object(namespace, object_id)?
        .map(|bytes| {
            serde_json::from_slice(&bytes).map_err(|error| {
                invalid(format!(
                    "production archive object could not be decoded: {error}"
                ))
            })
        })
        .transpose()
}
