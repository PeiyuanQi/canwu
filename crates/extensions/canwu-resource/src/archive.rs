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
pub const MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES: usize = 256;
pub const MAX_RESOURCE_ARCHIVE_LOOKUP_DIRECTORIES: usize = 2_048;

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceArchiveIndexDirectoryV1 {
    pub id: String,
    pub previous_root: Option<String>,
    pub membership_pages: Vec<String>,
    pub temporal_pages: Vec<String>,
    pub blob_ids: Vec<String>,
    pub archived_record_count: u64,
    pub semantic_digest: String,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceArchiveRetentionHandleV1 {
    pub id: String,
    pub phase: ResourceArchiveRetentionPhaseV1,
    pub expected_source_root: String,
    pub directory_root: String,
    pub object_ids: BTreeMap<String, BTreeSet<String>>,
    pub semantic_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceArchiveMaintenanceDispositionV1 {
    Applied,
    RejectedStale,
}

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreparedResourceArchiveBatchV1 {
    pub expected_source_root: String,
    pub selected: Vec<ResourceTerminalRecordKeyV1>,
    pub blob: ResourceArchiveBlobV1,
    pub membership_page: ResourceArchiveMembershipPageV1,
    pub temporal_page: ResourceArchiveTemporalPageV1,
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
    pub fn validate(&self) -> Result<(), ResourceError> {
        let selected_count =
            u32::try_from(self.selected.len()).map_err(|_| ResourceError::Overflow)?;
        let mut unique = BTreeSet::new();
        let mut detached = self.retention.clone();
        let retention_digest = std::mem::take(&mut detached.semantic_digest);
        if self.selected.is_empty()
            || self.selected.len() > MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES
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

impl PreparedResourceArchiveBatchV1 {
    pub fn store_and_verify(
        &self,
        store: &dyn ResourceArchiveStore,
    ) -> Result<VerifiedResourceArchiveCommitV1, ResourceError> {
        self.validate()?;
        store.persist_resource_archive_retention(&self.retention)?;
        store_encoded(
            store,
            RESOURCE_ARCHIVE_BLOB_NAMESPACE,
            &self.blob.content_id,
            &self.blob,
        )?;
        store_encoded(
            store,
            RESOURCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE,
            &self.membership_page.id,
            &self.membership_page,
        )?;
        store_encoded(
            store,
            RESOURCE_ARCHIVE_TEMPORAL_PAGE_NAMESPACE,
            &self.temporal_page.id,
            &self.temporal_page,
        )?;
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
        authenticate_resource_archive_directory(store, &loaded)?;
        if loaded != self.directory {
            return Err(ResourceError::InvalidDefinition(
                "resource archive directory readback differs".to_owned(),
            ));
        }
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

    pub fn validate(&self) -> Result<(), ResourceError> {
        for record in &self.blob.records {
            validate_resource_terminal_archive_record(record)?;
        }
        let expected_object_ids = BTreeMap::from([
            (
                RESOURCE_ARCHIVE_BLOB_NAMESPACE.to_owned(),
                BTreeSet::from([self.blob.content_id.clone()]),
            ),
            (
                RESOURCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE.to_owned(),
                BTreeSet::from([self.membership_page.id.clone()]),
            ),
            (
                RESOURCE_ARCHIVE_TEMPORAL_PAGE_NAMESPACE.to_owned(),
                BTreeSet::from([self.temporal_page.id.clone()]),
            ),
            (
                RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
                BTreeSet::from([self.directory.id.clone()]),
            ),
        ]);
        let selected_keys = self
            .blob
            .records
            .iter()
            .map(|record| record.key.clone())
            .collect::<Vec<_>>();
        if self.selected.is_empty()
            || self.selected.len() > MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES
            || self.selected != selected_keys
            || self.blob.expected_source_root != self.expected_source_root
            || self.membership_page.memberships.len() != self.selected.len()
            || self.temporal_page.entries.len() != self.selected.len()
            || self.directory.previous_root == Some(self.directory.id.clone())
            || self.directory.membership_pages != [self.membership_page.id.clone()]
            || self.directory.temporal_pages != [self.temporal_page.id.clone()]
            || self.directory.blob_ids != [self.blob.content_id.clone()]
            || self.retention.phase != ResourceArchiveRetentionPhaseV1::Prepared
            || self.retention.expected_source_root != self.expected_source_root
            || self.retention.directory_root != self.directory.id
            || self.retention.object_ids != expected_object_ids
        {
            return Err(ResourceError::InvalidDefinition(
                "prepared resource archive batch is forged or lacks exact closure".to_owned(),
            ));
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
    pub fn archive_source_root(&self) -> Result<String, ResourceError> {
        canonical_digest(
            "canwu.resource.archive-source-root.v1",
            &(
                self.state_revision,
                &self.terminal_archive_candidates,
                &self.archive_head,
            ),
        )
    }

    pub fn prepare_resource_archive(
        &self,
        candidate_limit: usize,
    ) -> Result<PreparedResourceArchiveBatchV1, ResourceError> {
        if candidate_limit == 0 || candidate_limit > self.limits.max_archive_candidates {
            return Err(ResourceError::LimitExceeded(
                "resource archive candidate budget is invalid".to_owned(),
            ));
        }
        let selected_entries: Vec<_> = self
            .terminal_archive_candidates
            .iter()
            .take(candidate_limit.min(MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES))
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
        let mut records = Vec::with_capacity(selected.len());
        for (sequence, key) in &selected_entries {
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
        let mut directory = ResourceArchiveIndexDirectoryV1 {
            id: String::new(),
            previous_root: self.archive_head.directory_root.clone(),
            membership_pages: vec![membership_page.id.clone()],
            temporal_pages: vec![temporal_page.id.clone()],
            blob_ids: vec![blob.content_id.clone()],
            archived_record_count: self
                .archive_head
                .archived_record_count
                .checked_add(u64::try_from(selected.len()).map_err(|_| ResourceError::Overflow)?)
                .ok_or(ResourceError::Overflow)?,
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
                    BTreeSet::from([blob.content_id.clone()]),
                ),
                (
                    RESOURCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE.to_owned(),
                    BTreeSet::from([membership_page.id.clone()]),
                ),
                (
                    RESOURCE_ARCHIVE_TEMPORAL_PAGE_NAMESPACE.to_owned(),
                    BTreeSet::from([temporal_page.id.clone()]),
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
        Ok(PreparedResourceArchiveBatchV1 {
            expected_source_root,
            selected,
            blob,
            membership_page,
            temporal_page,
            directory,
            retention,
        })
    }

    pub(crate) fn apply_archive_commit(
        &mut self,
        commit: &VerifiedResourceArchiveCommitV1,
    ) -> Result<ResourceArchiveMaintenanceReceiptV1, ResourceError> {
        commit.validate()?;
        let source_matches = self.archive_source_root()? == commit.expected_source_root;
        let selected_receipts = if source_matches {
            commit
                .selected
                .iter()
                .filter(|key| {
                    matches!(
                        key,
                        ResourceTerminalRecordKeyV1::ArchiveMaintenanceReceipt(_)
                    )
                })
                .count()
        } else {
            0
        };
        let projected_receipts = self
            .archive_maintenance_receipts
            .len()
            .checked_sub(selected_receipts)
            .and_then(|value| value.checked_add(1))
            .ok_or(ResourceError::Overflow)?;
        let projected_candidates = if source_matches {
            self.terminal_archive_candidates
                .len()
                .checked_sub(commit.selected.len())
                .and_then(|value| value.checked_add(1))
        } else {
            self.terminal_archive_candidates.len().checked_add(1)
        }
        .ok_or(ResourceError::Overflow)?;
        if projected_receipts > self.limits.max_archive_maintenance_receipts {
            return Err(ResourceError::LimitExceeded(
                "resource archive maintenance receipt capacity is exhausted".to_owned(),
            ));
        }
        if projected_candidates > self.limits.max_archive_candidates {
            return Err(ResourceError::LimitExceeded(
                "resource terminal archive backpressure cannot retain the maintenance receipt"
                    .to_owned(),
            ));
        }
        let disposition = if source_matches {
            for key in &commit.selected {
                self.remove_hot_terminal(key)?;
            }
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
        let sequence = self.next_admitted_sequence;
        self.next_admitted_sequence = sequence.checked_add(1).ok_or(ResourceError::Overflow)?;
        let mut receipt = ResourceArchiveMaintenanceReceiptV1 {
            sequence,
            retention_handle_id: commit.retention.id.clone(),
            expected_source_root: commit.expected_source_root.clone(),
            directory_root: commit.directory_root.clone(),
            disposition,
            archived_records: if disposition == ResourceArchiveMaintenanceDispositionV1::Applied {
                commit.archived_records
            } else {
                0
            },
            semantic_digest: String::new(),
        };
        receipt.semantic_digest =
            canonical_digest("canwu.resource.archive-maintenance-receipt.v1", &receipt)?;
        self.archive_maintenance_receipts
            .insert(sequence, receipt.clone());
        self.terminal_archive_candidates.insert(
            sequence,
            ResourceTerminalRecordKeyV1::ArchiveMaintenanceReceipt(sequence),
        );
        let mut handle = commit.retention.clone();
        handle.phase = if disposition == ResourceArchiveMaintenanceDispositionV1::Applied {
            ResourceArchiveRetentionPhaseV1::Committed
        } else {
            ResourceArchiveRetentionPhaseV1::RejectedStale
        };
        handle.semantic_digest.clear();
        handle.semantic_digest = canonical_digest("canwu.resource.archive-retention.v1", &handle)?;
        self.archive_retention_handles
            .insert(handle.id.clone(), handle);
        self.refresh_continuation();
        Ok(receipt)
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

    fn remove_hot_terminal(
        &mut self,
        key: &ResourceTerminalRecordKeyV1,
    ) -> Result<(), ResourceError> {
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
            ResourceTerminalRecordKeyV1::LeaseReceipt(sequence) => {
                let removed = self.completion_leases.receipts.remove(sequence);
                if let Some(receipt) = removed
                    && matches!(
                        receipt.action,
                        crate::CompletionLeaseReceiptActionV1::Completed
                            | crate::CompletionLeaseReceiptActionV1::Released
                            | crate::CompletionLeaseReceiptActionV1::Expired
                            | crate::CompletionLeaseReceiptActionV1::Aborted
                    )
                    && !self
                        .completion_leases
                        .receipts
                        .values()
                        .any(|other| other.acquisition == receipt.acquisition)
                    && self
                        .completion_leases
                        .acquisitions
                        .get(&receipt.acquisition)
                        .is_some_and(|acquisition| {
                            matches!(
                                acquisition.state,
                                crate::CompletionLeaseAcquisitionStateV1::Released
                                    | crate::CompletionLeaseAcquisitionStateV1::Expired
                            )
                        })
                {
                    let grant_ids = self
                        .completion_leases
                        .acquisitions
                        .get(&receipt.acquisition)
                        .map(|acquisition| acquisition.grants.values().cloned().collect::<Vec<_>>())
                        .unwrap_or_default();
                    for grant_id in grant_ids {
                        if let Some(grant) = self.completion_leases.grants.remove(&grant_id) {
                            for target in grant.target_versions {
                                self.completion_leases.target_locks.remove(&target);
                            }
                        }
                        for grants in self.completion_leases.expiry_due.values_mut() {
                            grants.remove(&grant_id);
                        }
                    }
                    self.completion_leases
                        .expiry_due
                        .retain(|_, grants| !grants.is_empty());
                    self.completion_leases
                        .certificates
                        .remove(&receipt.acquisition);
                    self.completion_leases
                        .acquisitions
                        .remove(&receipt.acquisition);
                }
            }
            ResourceTerminalRecordKeyV1::ArchiveMaintenanceReceipt(sequence) => {
                self.archive_maintenance_receipts.remove(sequence);
            }
        }
        self.terminal_archive_candidates
            .retain(|_, candidate| candidate != key);
        Ok(())
    }
}

pub fn authenticate_resource_archive_directory(
    store: &dyn ResourceArchiveStore,
    directory: &ResourceArchiveIndexDirectoryV1,
) -> Result<(), ResourceError> {
    let mut detached = directory.clone();
    let id = std::mem::take(&mut detached.id);
    let semantic_digest = std::mem::take(&mut detached.semantic_digest);
    let expected = canonical_digest("canwu.resource.archive-directory.v1", &detached)?;
    if id != expected || semantic_digest != expected {
        return Err(ResourceError::InvalidDefinition(
            "resource archive directory root is forged".to_owned(),
        ));
    }
    let unique_blobs = directory.blob_ids.iter().collect::<BTreeSet<_>>();
    let unique_membership_pages = directory.membership_pages.iter().collect::<BTreeSet<_>>();
    let unique_temporal_pages = directory.temporal_pages.iter().collect::<BTreeSet<_>>();
    if directory.blob_ids.is_empty()
        || directory.membership_pages.is_empty()
        || directory.temporal_pages.is_empty()
        || directory.blob_ids.len() > MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES
        || directory.membership_pages.len() > MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES
        || directory.temporal_pages.len() > MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES
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
    if archived_records.len() > MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES {
        return Err(ResourceError::LimitExceeded(
            "resource archive directory exceeds its terminal record verification budget".to_owned(),
        ));
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
    let archived_keys = archived_records.keys().cloned().collect::<BTreeSet<_>>();
    if membership_keys != archived_keys || temporal_keys != archived_keys {
        return Err(ResourceError::InvalidDefinition(
            "resource archive indexes do not cover the complete terminal blob set".to_owned(),
        ));
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

/// Authenticates every archive object reachable from the persisted resource
/// head and every in-flight retention handle. Restore callers must provide the
/// same durable store that owns those objects; a snapshot alone is not enough
/// to trust archive-backed state.
pub fn validate_resource_archive_store(
    state: &ResourceState,
    store: &dyn ResourceArchiveStore,
) -> Result<(), ResourceError> {
    let mut next = state.archive_head.directory_root.clone();
    let mut expected_count = state.archive_head.archived_record_count;
    let mut seen = BTreeSet::new();
    let mut directory_count = 0_u64;
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
        authenticate_resource_archive_directory(store, &directory)?;
        if directory.id != root || directory.archived_record_count != expected_count {
            return Err(ResourceError::InvalidDefinition(
                "resource archive directory chain count or identity differs from its head"
                    .to_owned(),
            ));
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

/// Resolves one exact cold terminal member through the authenticated archive
/// chain. The read is capped even if a hostile or unsupported archive chain is
/// supplied, so ordinary admission never performs an unbounded history scan.
pub fn archived_resource_terminal_record(
    state: &ResourceState,
    store: &dyn ResourceArchiveStore,
    key: &ResourceTerminalRecordKeyV1,
) -> Result<Option<ResourceTerminalArchiveRecordV1>, ResourceError> {
    let mut next = state.archive_head.directory_root.clone();
    let mut visited = BTreeSet::new();
    let mut reads = 0_usize;
    while let Some(root) = next {
        reads = reads.checked_add(1).ok_or(ResourceError::Overflow)?;
        if reads > MAX_RESOURCE_ARCHIVE_LOOKUP_DIRECTORIES {
            return Err(ResourceError::LimitExceeded(
                "resource cold membership lookup exceeded its directory-read bound".to_owned(),
            ));
        }
        if !visited.insert(root.clone()) {
            return Err(ResourceError::InvalidDefinition(
                "resource archive lookup found a cyclic directory chain".to_owned(),
            ));
        }
        let directory: ResourceArchiveIndexDirectoryV1 = load_encoded(
            store,
            RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            &root,
        )?
        .ok_or_else(|| {
            ResourceError::NotFound("resource archive lookup directory is missing".to_owned())
        })?;
        authenticate_resource_archive_directory(store, &directory)?;
        if directory.id != root {
            return Err(ResourceError::InvalidDefinition(
                "resource archive lookup directory identity differs".to_owned(),
            ));
        }
        for page_id in &directory.membership_pages {
            let page: ResourceArchiveMembershipPageV1 =
                load_encoded(store, RESOURCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE, page_id)?
                    .ok_or_else(|| {
                        ResourceError::NotFound(
                            "resource archive lookup membership page is missing".to_owned(),
                        )
                    })?;
            let Some(member) = page.memberships.iter().find(|member| &member.key == key) else {
                continue;
            };
            let blob: ResourceArchiveBlobV1 = load_encoded(
                store,
                RESOURCE_ARCHIVE_BLOB_NAMESPACE,
                &member.blob_id,
            )?
            .ok_or_else(|| {
                ResourceError::NotFound("resource archive lookup blob is missing".to_owned())
            })?;
            let record = blob
                .records
                .get(usize::from(member.ordinal))
                .ok_or_else(|| {
                    ResourceError::InvalidDefinition(
                        "resource archive lookup ordinal is outside its blob".to_owned(),
                    )
                })?
                .clone();
            if &record.key != key
                || record.semantic_digest != member.semantic_digest
                || record.terminal_sequence != member.terminal_sequence
            {
                return Err(ResourceError::InvalidDefinition(
                    "resource archive lookup membership differs from its typed record".to_owned(),
                ));
            }
            validate_resource_terminal_archive_record(&record)?;
            return Ok(Some(record));
        }
        next = directory.previous_root;
    }
    Ok(None)
}

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
