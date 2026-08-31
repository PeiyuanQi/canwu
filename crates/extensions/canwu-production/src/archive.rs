use crate::{
    MAX_ARCHIVE_BYTES_PER_BATCH, MAX_ARCHIVE_OBJECTS_PER_BATCH, MAX_ARCHIVE_PAGE_ENTRIES,
    MAX_ARCHIVE_PREPARE_CANDIDATES, PreparedProductionArchiveBatchV1, ProductionArchiveBlobV1,
    ProductionArchiveIndexDirectoryV1, ProductionArchiveMaintenanceReceiptV1,
    ProductionArchiveMembershipPageV1, ProductionArchiveMembershipV1, ProductionArchiveReceiptId,
    ProductionArchiveRetentionHandleV1, ProductionArchiveRetentionPhaseV1,
    ProductionArchiveTemporalEntryV1, ProductionArchiveTemporalPageV1, ProductionExecutionId,
    ProductionFacilityProjectArchiveRecordV1, ProductionState, ProductionTerminalArchiveKeyV1,
    ProductionTerminalArchiveRecordV1, VerifiedProductionArchiveCommitV1, WorkInProgressId,
    WorkOrderLifecycle, invalid,
};
use canwu_api::{CanwuError, ErrorCode, canonical_hash};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::{BTreeMap, BTreeSet};

pub const PRODUCTION_ARCHIVE_BLOB_NAMESPACE: &str = "canwu.production.archive.blob";
pub const PRODUCTION_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE: &str =
    "canwu.production.archive.membership-page";
pub const PRODUCTION_ARCHIVE_TEMPORAL_PAGE_NAMESPACE: &str =
    "canwu.production.archive.temporal-page";
pub const PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE: &str =
    "canwu.production.archive.index-directory";

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

impl PreparedProductionArchiveBatchV1 {
    pub fn store_and_verify(
        &self,
        store: &dyn ProductionArchiveStore,
    ) -> Result<VerifiedProductionArchiveCommitV1, CanwuError> {
        store.persist_production_archive_retention(&self.retention)?;
        let objects = [
            (
                PRODUCTION_ARCHIVE_BLOB_NAMESPACE,
                self.blob.content_id.as_str(),
                serde_json::to_vec(&self.blob),
            ),
            (
                PRODUCTION_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE,
                self.membership_page.id.as_str(),
                serde_json::to_vec(&self.membership_page),
            ),
            (
                PRODUCTION_ARCHIVE_TEMPORAL_PAGE_NAMESPACE,
                self.temporal_page.id.as_str(),
                serde_json::to_vec(&self.temporal_page),
            ),
            (
                PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
                self.directory.id.as_str(),
                serde_json::to_vec(&self.directory),
            ),
        ];
        if objects.len() > MAX_ARCHIVE_OBJECTS_PER_BATCH {
            return Err(invalid("production archive object budget was exceeded"));
        }
        let mut byte_count = 0_usize;
        for (namespace, object_id, encoded) in objects {
            let bytes = encoded.map_err(|error| {
                invalid(format!(
                    "production archive object could not be encoded: {error}"
                ))
            })?;
            byte_count = byte_count
                .checked_add(bytes.len())
                .ok_or_else(|| invalid("production archive byte count overflowed"))?;
            if byte_count > MAX_ARCHIVE_BYTES_PER_BATCH {
                return Err(invalid("production archive byte budget was exceeded"));
            }
            store.store_production_archive_object(namespace, object_id, &bytes)?;
            let loaded = store
                .load_production_archive_object(namespace, object_id)?
                .ok_or_else(|| invalid("production archive readback object is unavailable"))?;
            if loaded != bytes {
                return Err(invalid(
                    "production archive readback bytes differ from the stored object",
                ));
            }
        }
        authenticate_production_archive_directory(store, &self.directory)?;
        let mut retention = self.retention.clone();
        retention.phase = ProductionArchiveRetentionPhaseV1::Verified;
        retention.semantic_digest.clear();
        retention.semantic_digest =
            canonical_hash("canwu.production.archive-retention.v1", &retention)?;
        store.persist_production_archive_retention(&retention)?;
        Ok(VerifiedProductionArchiveCommitV1 {
            expected_source_root: self.expected_source_root.clone(),
            selected: self.selected.clone(),
            selected_projects: self.selected_projects.clone(),
            directory_root: self.directory.id.clone(),
            retention,
        })
    }
}

impl ProductionState {
    pub fn archive_source_root(&self) -> Result<String, CanwuError> {
        canonical_hash(
            "canwu.production.archive-source-root.v1",
            &(
                self.revision,
                &self.archive_due_index,
                &self.archive.directory_root,
                self.archive.archived_execution_count,
                self.archive.archived_project_count,
                self.archive.committed_batch_count,
                &self.project_archive_due_index,
            ),
        )
    }

    pub fn prepare_production_archive(
        &self,
        candidate_limit: usize,
    ) -> Result<PreparedProductionArchiveBatchV1, CanwuError> {
        if candidate_limit == 0 || candidate_limit > MAX_ARCHIVE_PREPARE_CANDIDATES {
            return Err(invalid("production archive candidate budget is invalid"));
        }
        if self.archive.pending_handles.len()
            >= crate::ProductionLimitsV1::canonical().max_pending_retention_handles
            || self.archive.maintenance_receipts.len()
                >= crate::ProductionLimitsV1::canonical().max_operation_outcomes
        {
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                "archive_backpressure: production archive retention capacity is exhausted",
            ));
        }
        let selected = self
            .archive_due_index
            .iter()
            .take(candidate_limit)
            .cloned()
            .collect::<Vec<_>>();
        let remaining = candidate_limit.saturating_sub(selected.len());
        let selected_projects = self
            .project_archive_due_index
            .iter()
            .take(remaining)
            .cloned()
            .collect::<Vec<_>>();
        if selected.is_empty() && selected_projects.is_empty() {
            return Err(CanwuError::new(
                ErrorCode::DomainRecordNotFound,
                "production archive has no eligible terminal execution or facility project",
            ));
        }
        if selected
            .len()
            .checked_add(selected_projects.len())
            .is_none_or(|count| count > candidate_limit || count > MAX_ARCHIVE_PAGE_ENTRIES)
        {
            return Err(invalid("production archive due-work budget was exceeded"));
        }
        let expected_source_root = self.archive_source_root()?;
        let records = selected
            .iter()
            .map(|execution| self.terminal_archive_record(execution))
            .collect::<Result<Vec<_>, _>>()?;
        let project_records = selected_projects
            .iter()
            .map(|project| self.facility_project_archive_record(project))
            .collect::<Result<Vec<_>, _>>()?;
        let mut blob = ProductionArchiveBlobV1 {
            format_version: 1,
            expected_source_root: expected_source_root.clone(),
            records,
            project_records,
            content_id: String::new(),
        };
        blob.content_id = canonical_hash("canwu.production.archive-blob.v1", &blob)?;
        let mut membership_page = ProductionArchiveMembershipPageV1 {
            id: String::new(),
            memberships: blob
                .records
                .iter()
                .map(|record| (&record.key, &record.canonical_digest))
                .chain(
                    blob.project_records
                        .iter()
                        .map(|record| (&record.key, &record.canonical_digest)),
                )
                .enumerate()
                .map(|(ordinal, (key, digest))| {
                    Ok(ProductionArchiveMembershipV1 {
                        key: key.clone(),
                        blob_id: blob.content_id.clone(),
                        ordinal: u16::try_from(ordinal).map_err(|_| {
                            invalid("production archive membership ordinal overflowed")
                        })?,
                        semantic_digest: digest.clone(),
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
            entries: blob
                .records
                .iter()
                .map(|record| (record.terminal_at, record.key.clone()))
                .chain(
                    blob.project_records
                        .iter()
                        .map(|record| (record.terminal_at, record.key.clone())),
                )
                .map(|(terminal_at, key)| ProductionArchiveTemporalEntryV1 { terminal_at, key })
                .collect(),
            semantic_digest: String::new(),
        };
        temporal_page
            .entries
            .sort_by_key(|entry| (entry.terminal_at, entry.key.clone()));
        temporal_page.semantic_digest =
            canonical_hash("canwu.production.archive-temporal-page.v1", &temporal_page)?;
        temporal_page.id = temporal_page.semantic_digest.clone();
        let archived_execution_count = self
            .archive
            .archived_execution_count
            .checked_add(
                u64::try_from(selected.len())
                    .map_err(|_| invalid("production archive count overflowed"))?,
            )
            .ok_or_else(|| invalid("production archive count overflowed"))?;
        let archived_project_count = self
            .archive
            .archived_project_count
            .checked_add(
                u64::try_from(selected_projects.len())
                    .map_err(|_| invalid("production project archive count overflowed"))?,
            )
            .ok_or_else(|| invalid("production project archive count overflowed"))?;
        let mut directory = ProductionArchiveIndexDirectoryV1 {
            id: String::new(),
            previous_root: self.archive.directory_root.clone(),
            blob_ids: vec![blob.content_id.clone()],
            membership_pages: vec![membership_page.id.clone()],
            temporal_pages: vec![temporal_page.id.clone()],
            archived_execution_count,
            archived_project_count,
            semantic_digest: String::new(),
        };
        directory.semantic_digest =
            canonical_hash("canwu.production.archive-directory.v1", &directory)?;
        directory.id = directory.semantic_digest.clone();
        let handle_id = canonical_hash(
            "canwu.production.archive-retention-id.v1",
            &(&expected_source_root, &directory.id),
        )?;
        let mut retention = ProductionArchiveRetentionHandleV1 {
            handle_id,
            source_root: expected_source_root.clone(),
            target_directory_root: directory.id.clone(),
            object_ids: BTreeMap::from([
                (
                    PRODUCTION_ARCHIVE_BLOB_NAMESPACE.to_owned(),
                    BTreeSet::from([blob.content_id.clone()]),
                ),
                (
                    PRODUCTION_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE.to_owned(),
                    BTreeSet::from([membership_page.id.clone()]),
                ),
                (
                    PRODUCTION_ARCHIVE_TEMPORAL_PAGE_NAMESPACE.to_owned(),
                    BTreeSet::from([temporal_page.id.clone()]),
                ),
                (
                    PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
                    BTreeSet::from([directory.id.clone()]),
                ),
            ]),
            phase: ProductionArchiveRetentionPhaseV1::Prepared,
            semantic_digest: String::new(),
        };
        retention.semantic_digest =
            canonical_hash("canwu.production.archive-retention.v1", &retention)?;
        Ok(PreparedProductionArchiveBatchV1 {
            expected_source_root,
            selected,
            selected_projects,
            blob,
            membership_page,
            temporal_page,
            directory,
            retention,
        })
    }

    pub(crate) fn apply_production_archive_commit(
        &mut self,
        commit: &VerifiedProductionArchiveCommitV1,
    ) -> Result<ProductionArchiveMaintenanceReceiptV1, CanwuError> {
        let mut detached_retention = commit.retention.clone();
        let retention_digest = std::mem::take(&mut detached_retention.semantic_digest);
        if retention_digest
            != canonical_hash("canwu.production.archive-retention.v1", &detached_retention)?
            || !matches!(
                commit.retention.phase,
                ProductionArchiveRetentionPhaseV1::Verified
                    | ProductionArchiveRetentionPhaseV1::DurableIngress
            )
            || commit.retention.source_root != commit.expected_source_root
            || commit.retention.target_directory_root != commit.directory_root
            || commit
                .retention
                .object_ids
                .get(PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE)
                != Some(&BTreeSet::from([commit.directory_root.clone()]))
        {
            return Err(invalid(
                "production archive commit retention handle is forged or incompletely bound",
            ));
        }
        let selected_count = commit
            .selected
            .len()
            .checked_add(commit.selected_projects.len())
            .ok_or_else(|| invalid("production archive selection count overflowed"))?;
        let applied = self.archive_source_root()? == commit.expected_source_root;
        if applied {
            if selected_count == 0 || selected_count > MAX_ARCHIVE_PREPARE_CANDIDATES {
                return Err(invalid(
                    "production archive commit selection is empty or exceeds its bound",
                ));
            }
            let expected = self.prepare_production_archive(selected_count)?;
            if expected.selected != commit.selected
                || expected.selected_projects != commit.selected_projects
                || expected.directory.id != commit.directory_root
                || expected.retention.handle_id != commit.retention.handle_id
                || expected.retention.source_root != commit.retention.source_root
                || expected.retention.target_directory_root
                    != commit.retention.target_directory_root
                || expected.retention.object_ids != commit.retention.object_ids
            {
                return Err(invalid(
                    "production archive commit does not bind the exact selected blob, pages, directory, and retention objects",
                ));
            }
            for execution_id in &commit.selected {
                self.remove_hot_terminal_execution(execution_id)?;
            }
            for project_id in &commit.selected_projects {
                self.remove_hot_terminal_project(project_id)?;
            }
            self.archive.directory_root = Some(commit.directory_root.clone());
            self.archive.archived_execution_count = self
                .archive
                .archived_execution_count
                .checked_add(
                    u64::try_from(commit.selected.len())
                        .map_err(|_| invalid("production archive count overflowed"))?,
                )
                .ok_or_else(|| invalid("production archive count overflowed"))?;
            self.archive.archived_project_count = self
                .archive
                .archived_project_count
                .checked_add(
                    u64::try_from(commit.selected_projects.len())
                        .map_err(|_| invalid("production project archive count overflowed"))?,
                )
                .ok_or_else(|| invalid("production project archive count overflowed"))?;
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
        let receipt_sequence = self
            .archive
            .maintenance_receipts
            .len()
            .checked_add(1)
            .ok_or_else(|| invalid("production archive receipt sequence overflowed"))?;
        let id = ProductionArchiveReceiptId::new(format!(
            "canwu.production:archive-receipt:{receipt_sequence}"
        ))?;
        let mut receipt = ProductionArchiveMaintenanceReceiptV1 {
            id: id.clone(),
            source_root: commit.expected_source_root.clone(),
            directory_root: commit.directory_root.clone(),
            archived_executions: if applied { commit.selected.len() } else { 0 },
            archived_projects: if applied {
                commit.selected_projects.len()
            } else {
                0
            },
            disposition,
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
        self.archive
            .pending_handles
            .insert(handle.handle_id.clone(), handle);
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("production runtime revision overflowed"))?;
        self.validate()?;
        Ok(receipt)
    }

    pub fn acknowledge_production_archive_retention(
        &mut self,
        handle_id: &str,
    ) -> Result<(), CanwuError> {
        let handle = self
            .archive
            .pending_handles
            .get(handle_id)
            .ok_or_else(|| invalid("production archive retention handle is unavailable"))?;
        if !matches!(
            handle.phase,
            ProductionArchiveRetentionPhaseV1::Committed
                | ProductionArchiveRetentionPhaseV1::RejectedStale
                | ProductionArchiveRetentionPhaseV1::Abandoned
        ) {
            return Err(invalid(
                "production archive retention is not terminal and cannot be acknowledged",
            ));
        }
        self.archive.pending_handles.remove(handle_id);
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("production runtime revision overflowed"))?;
        self.validate()
    }

    fn terminal_archive_record(
        &self,
        execution_id: &ProductionExecutionId,
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
        let mut operation_outcomes = self
            .operation_outcomes
            .values()
            .filter(|outcome| {
                outcome.execution.as_ref() == Some(execution_id)
                    || outcome.work_order.as_ref() == Some(&execution.work_order)
            })
            .cloned()
            .collect::<Vec<_>>();
        operation_outcomes.sort_by(|left, right| left.id.cmp(&right.id));
        let mut participant_grants = self
            .completion_participant_grants
            .get(&acquisition.id)
            .into_iter()
            .flat_map(std::collections::BTreeMap::values)
            .cloned()
            .collect::<Vec<_>>();
        participant_grants.sort_by(|left, right| left.participant.cmp(&right.participant));
        let completion_receipts = self
            .completion_receipts
            .values()
            .filter(|receipt| receipt.acquisition == acquisition.id)
            .cloned()
            .collect::<Vec<_>>();
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
        let mut operation_outcomes = self
            .operation_outcomes
            .values()
            .filter(|outcome| outcome.project.as_ref() == Some(project_id))
            .cloned()
            .collect::<Vec<_>>();
        operation_outcomes.sort_by(|left, right| left.id.cmp(&right.id));
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
        let completion_receipts = self
            .completion_receipts
            .values()
            .filter(|receipt| receipt.acquisition == acquisition.id)
            .cloned()
            .collect::<Vec<_>>();
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

    fn remove_hot_terminal_execution(
        &mut self,
        execution_id: &ProductionExecutionId,
    ) -> Result<(), CanwuError> {
        let execution = self
            .executions
            .get(execution_id)
            .ok_or_else(|| invalid("production archive commit lost its execution"))?
            .clone();
        if !self.archive_due_index.contains(execution_id) {
            return Err(invalid(
                "production archive commit names an execution outside the due index",
            ));
        }
        let wip_id = WorkInProgressId::new(format!("canwu.production:wip:{execution_id}"))?;
        self.executions.remove(execution_id);
        self.work_in_progress.remove(&wip_id);
        for allocation in &execution.allocations {
            self.capacity_allocations.remove(allocation);
        }
        self.work_orders.remove(&execution.work_order);
        self.operation_outcomes.retain(|_, outcome| {
            outcome.execution.as_ref() != Some(execution_id)
                && outcome.work_order.as_ref() != Some(&execution.work_order)
        });
        self.production_completion_grants
            .remove(&execution.production_completion_grant);
        let acquisition = execution.completion_certificate.acquisition;
        if !self
            .executions
            .values()
            .any(|other| other.completion_certificate.acquisition == acquisition)
        {
            self.production_completion_certificates.remove(&acquisition);
            self.completion_participant_grants.remove(&acquisition);
            self.completion_acquisitions.remove(&acquisition);
            self.completion_receipts
                .retain(|_, receipt| receipt.acquisition != acquisition);
            for acquisitions in self.completion_expiry_due.values_mut() {
                acquisitions.remove(&acquisition);
            }
            self.completion_expiry_due
                .retain(|_, acquisitions| !acquisitions.is_empty());
            self.completion_target_locks
                .retain(|_, (_, grant)| self.production_completion_grants.contains_key(grant));
        }
        self.archive_due_index.remove(execution_id);
        Ok(())
    }

    fn remove_hot_terminal_project(
        &mut self,
        project_id: &crate::FacilityProjectId,
    ) -> Result<(), CanwuError> {
        let project = self
            .facility_projects
            .get(project_id)
            .ok_or_else(|| invalid("production archive commit lost its facility project"))?
            .clone();
        if project.lifecycle != crate::FacilityProjectLifecycle::Completed
            || !self.project_archive_due_index.contains(project_id)
        {
            return Err(invalid(
                "production archive commit names a non-terminal facility project",
            ));
        }
        self.facility_projects.remove(project_id);
        self.resource_continuation_witnesses.remove(project_id);
        self.operation_outcomes
            .retain(|_, outcome| outcome.project.as_ref() != Some(project_id));
        self.production_completion_grants
            .remove(&project.production_completion_grant);
        let acquisition = project.completion_certificate.acquisition;
        if !self
            .executions
            .values()
            .any(|execution| execution.completion_certificate.acquisition == acquisition)
            && !self
                .facility_projects
                .values()
                .any(|other| other.completion_certificate.acquisition == acquisition)
        {
            self.production_completion_certificates.remove(&acquisition);
            self.completion_participant_grants.remove(&acquisition);
            self.completion_acquisitions.remove(&acquisition);
            self.completion_receipts
                .retain(|_, receipt| receipt.acquisition != acquisition);
            for acquisitions in self.completion_expiry_due.values_mut() {
                acquisitions.remove(&acquisition);
            }
            self.completion_expiry_due
                .retain(|_, acquisitions| !acquisitions.is_empty());
            self.completion_target_locks
                .retain(|_, (_, grant)| self.production_completion_grants.contains_key(grant));
        }
        self.project_archive_due_index.remove(project_id);
        Ok(())
    }
}

pub fn authenticate_production_archive_directory(
    store: &dyn ProductionArchiveStore,
    directory: &ProductionArchiveIndexDirectoryV1,
) -> Result<(), CanwuError> {
    let mut detached = directory.clone();
    let id = std::mem::take(&mut detached.id);
    let digest = std::mem::take(&mut detached.semantic_digest);
    let expected = canonical_hash("canwu.production.archive-directory.v1", &detached)?;
    let unique_blobs = directory.blob_ids.iter().collect::<BTreeSet<_>>();
    let unique_memberships = directory.membership_pages.iter().collect::<BTreeSet<_>>();
    let unique_temporal = directory.temporal_pages.iter().collect::<BTreeSet<_>>();
    if id != expected
        || digest != expected
        || directory.blob_ids.is_empty()
        || directory.membership_pages.is_empty()
        || directory.temporal_pages.is_empty()
        || directory.blob_ids.len() > MAX_ARCHIVE_PAGE_ENTRIES
        || directory.membership_pages.len() > MAX_ARCHIVE_PAGE_ENTRIES
        || directory.temporal_pages.len() > MAX_ARCHIVE_PAGE_ENTRIES
        || unique_blobs.len() != directory.blob_ids.len()
        || unique_memberships.len() != directory.membership_pages.len()
        || unique_temporal.len() != directory.temporal_pages.len()
        || directory.previous_root.as_ref() == Some(&directory.id)
    {
        return Err(invalid("production archive directory root is forged"));
    }
    let mut archived_records = BTreeMap::new();
    let mut source_root: Option<String> = None;
    for blob_id in &directory.blob_ids {
        let blob: ProductionArchiveBlobV1 =
            load_encoded(store, PRODUCTION_ARCHIVE_BLOB_NAMESPACE, blob_id)?.ok_or_else(|| {
                invalid("production archive blob referenced by the directory is missing")
            })?;
        let mut detached = blob.clone();
        detached.content_id.clear();
        if blob.content_id != *blob_id
            || blob.content_id != canonical_hash("canwu.production.archive-blob.v1", &detached)?
            || blob.format_version != 1
            || blob
                .records
                .len()
                .saturating_add(blob.project_records.len())
                == 0
            || blob
                .records
                .len()
                .saturating_add(blob.project_records.len())
                > MAX_ARCHIVE_PAGE_ENTRIES
            || blob.expected_source_root.len() != 64
            || source_root
                .as_ref()
                .is_some_and(|expected| expected != &blob.expected_source_root)
        {
            return Err(invalid("production archive blob content ID is forged"));
        }
        source_root.get_or_insert_with(|| blob.expected_source_root.clone());
        for (ordinal, record) in blob.records.iter().enumerate() {
            let mut detached = record.clone();
            let digest = std::mem::take(&mut detached.canonical_digest);
            if digest != canonical_hash("canwu.production.terminal-archive-record.v1", &detached)? {
                return Err(invalid("production archive terminal record is forged"));
            }
            validate_terminal_execution_archive_record(record)?;
            let ordinal = u16::try_from(ordinal)
                .map_err(|_| invalid("production archive record ordinal overflowed"))?;
            if archived_records
                .insert(
                    record.key.clone(),
                    (
                        blob_id.clone(),
                        ordinal,
                        record.canonical_digest.clone(),
                        record.terminal_at,
                    ),
                )
                .is_some()
            {
                return Err(invalid(
                    "production archive contains a duplicate terminal key",
                ));
            }
        }
        let ordinal_offset = blob.records.len();
        for (index, record) in blob.project_records.iter().enumerate() {
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
            let ordinal = ordinal_offset
                .checked_add(index)
                .and_then(|value| u16::try_from(value).ok())
                .ok_or_else(|| invalid("production archive record ordinal overflowed"))?;
            if archived_records
                .insert(
                    record.key.clone(),
                    (
                        blob_id.clone(),
                        ordinal,
                        record.canonical_digest.clone(),
                        record.terminal_at,
                    ),
                )
                .is_some()
            {
                return Err(invalid(
                    "production archive contains a duplicate terminal key",
                ));
            }
        }
    }
    let mut membership_keys = BTreeSet::new();
    for page_id in &directory.membership_pages {
        let page: ProductionArchiveMembershipPageV1 =
            load_encoded(store, PRODUCTION_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE, page_id)?
                .ok_or_else(|| invalid("production archive membership page is missing"))?;
        if page.memberships.len() > MAX_ARCHIVE_PAGE_ENTRIES {
            return Err(invalid(
                "production archive membership page exceeds its cap",
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
        if page.entries.len() > MAX_ARCHIVE_PAGE_ENTRIES {
            return Err(invalid("production archive temporal page exceeds its cap"));
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
    let archived_keys = archived_records.keys().cloned().collect::<BTreeSet<_>>();
    if membership_keys != archived_keys || temporal_keys != archived_keys {
        return Err(invalid(
            "production archive indexes do not cover the complete terminal blob set",
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
                && execution.output_outcomes.len() == execution.output_requests.len()
                && !execution.output_outcomes.is_empty()
                && execution.output_source.is_some()
                && execution.output_ack_digest.is_some() =>
        {
            validate_terminal_output_closure(execution)?;
        }
        WorkOrderLifecycle::Cancelled | WorkOrderLifecycle::Failed
            if execution.output_outcomes.is_empty()
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
                || !outcome_matches_execution(outcome, order, execution)
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

fn validate_terminal_output_closure(
    execution: &crate::ProductionExecution,
) -> Result<(), CanwuError> {
    let source = execution
        .output_source
        .as_ref()
        .ok_or_else(|| invalid("settled production archive lost its exact output source"))?;
    let at = execution
        .completed_at
        .ok_or_else(|| invalid("settled production archive lost its completion time"))?;
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
                || !outcome_matches_project(outcome, project)
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

fn validate_archived_operation_outcome(
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

fn outcome_matches_execution(
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
        } if advanced == &execution.id => {
            outcome.work_order.is_none() && outcome.execution.as_ref() == Some(&execution.id)
        }
        _ => false,
    }
}

fn outcome_matches_project(
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

pub fn validate_production_archive(
    store: &dyn ProductionArchiveStore,
    state: &ProductionState,
) -> Result<(), CanwuError> {
    let mut next = state.archive.directory_root.clone();
    let mut expected_execution_count = state.archive.archived_execution_count;
    let mut expected_project_count = state.archive.archived_project_count;
    let mut visited = BTreeSet::new();
    let mut directories = 0_u64;
    while let Some(root) = next {
        if !visited.insert(root.clone()) {
            return Err(invalid("production archive directory chain is cyclic"));
        }
        let directory: ProductionArchiveIndexDirectoryV1 =
            load_encoded(store, PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, &root)?
                .ok_or_else(|| invalid("production archive head directory is unavailable"))?;
        authenticate_production_archive_directory(store, &directory)?;
        if directory.id != root
            || directory.archived_execution_count != expected_execution_count
            || directory.archived_project_count != expected_project_count
        {
            return Err(invalid(
                "production archive directory chain count differs from its authoritative closure",
            ));
        }
        let batch_execution_count =
            directory
                .blob_ids
                .iter()
                .try_fold(0_u64, |total, blob_id| {
                    let blob: ProductionArchiveBlobV1 =
                        load_encoded(store, PRODUCTION_ARCHIVE_BLOB_NAMESPACE, blob_id)?
                            .ok_or_else(|| invalid("production archive blob is unavailable"))?;
                    total
                        .checked_add(u64::try_from(blob.records.len()).map_err(|_| {
                            invalid("production archive execution count overflowed")
                        })?)
                        .ok_or_else(|| invalid("production archive execution count overflowed"))
                })?;
        let batch_project_count = directory
            .blob_ids
            .iter()
            .try_fold(0_u64, |total, blob_id| {
                let blob: ProductionArchiveBlobV1 =
                    load_encoded(store, PRODUCTION_ARCHIVE_BLOB_NAMESPACE, blob_id)?
                        .ok_or_else(|| invalid("production archive blob is unavailable"))?;
                total
                    .checked_add(
                        u64::try_from(blob.project_records.len())
                            .map_err(|_| invalid("production archive project count overflowed"))?,
                    )
                    .ok_or_else(|| invalid("production archive project count overflowed"))
            })?;
        expected_execution_count = expected_execution_count
            .checked_sub(batch_execution_count)
            .ok_or_else(|| invalid("production archive execution count underflowed"))?;
        expected_project_count = expected_project_count
            .checked_sub(batch_project_count)
            .ok_or_else(|| invalid("production archive project count underflowed"))?;
        next = directory.previous_root;
        directories = directories
            .checked_add(1)
            .ok_or_else(|| invalid("production archive directory count overflowed"))?;
        if directories > state.archive.committed_batch_count {
            return Err(invalid(
                "production archive directory chain exceeds its committed batch bound",
            ));
        }
    }
    if expected_execution_count != 0
        || expected_project_count != 0
        || directories != state.archive.committed_batch_count
    {
        return Err(invalid(
            "production archive directory chain does not close its authoritative counts",
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
        let expected_objects = BTreeMap::from([
            (
                PRODUCTION_ARCHIVE_BLOB_NAMESPACE.to_owned(),
                target.blob_ids.iter().cloned().collect(),
            ),
            (
                PRODUCTION_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE.to_owned(),
                target.membership_pages.iter().cloned().collect(),
            ),
            (
                PRODUCTION_ARCHIVE_TEMPORAL_PAGE_NAMESPACE.to_owned(),
                target.temporal_pages.iter().cloned().collect(),
            ),
            (
                PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
                BTreeSet::from([target.id.clone()]),
            ),
        ]);
        if handle.object_ids != expected_objects || handle.target_directory_root != target.id {
            return Err(invalid(
                "production pending retention handle does not close its exact archive objects and prior root",
            ));
        }
    }
    Ok(())
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

#[allow(dead_code)]
fn encoded_size<T: Serialize>(value: &T) -> Result<usize, CanwuError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|error| invalid(format!("production archive sizing failed: {error}")))
}
