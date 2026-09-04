use crate::{ProductionState, invalid};
use canwu_api::{CanwuError, DomainRecordVersionRef, ErrorCode, KnowledgeHolderRef, SimTime};
use canwu_resource::{
    AbortCompletionLeaseV1, AdmissionEpochV1, CompletionCapacityGrantId, CompletionCapacityGrantV1,
    CompletionGrantStateV1, CompletionLeaseAcquisitionId, CompletionLeaseAcquisitionStateV1,
    CompletionLeaseAcquisitionV1, CompletionLeaseActivationCertificateV1,
    CompletionLeaseReceiptActionV1, CompletionLeaseReceiptV1, CompletionLeaseStatusDtoV1,
    CompletionLockedTargetV1, CompletionPolicyClassV1, ExpireCompletionCapacityV1,
    GrantCompletionCapacityV1, MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL,
    MAX_PENDING_LEASE_ACQUISITIONS_PER_AUTHORITY, PrepareCompletionCapacityV1,
    RequestCompletionLeaseV1, ResourceOperationKey, ResourceRevision, RunBudgetRevisionV1,
    canonical_digest, deterministic_completion_fairness_order,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const PRODUCTION_COMPLETION_OPERATION_NAMESPACE: &str = "canwu.production.execution";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionCompletionAdmissionEpochV1 {
    pub holder: KnowledgeHolderRef,
    pub operation_namespace: String,
    pub epoch: AdmissionEpochV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionCompletionParticipantGrantV1 {
    pub participant: String,
    pub provider_source: DomainRecordVersionRef,
    pub grant: CompletionCapacityGrantV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "completion", content = "request", rename_all = "snake_case")]
pub enum ProductionCompletionIngressV1 {
    GrantLocal(GrantCompletionCapacityV1),
    AcknowledgeParticipantGrant {
        acquisition: CompletionLeaseAcquisitionId,
        expected_acquisition_revision: ResourceRevision,
        participant: String,
        provider_source: DomainRecordVersionRef,
        grant: CompletionCapacityGrantV1,
    },
    PrepareLocal(PrepareCompletionCapacityV1),
    AcknowledgeParticipantPrepared {
        acquisition: CompletionLeaseAcquisitionId,
        expected_acquisition_revision: ResourceRevision,
        participant: String,
        provider_source: DomainRecordVersionRef,
        grant: CompletionCapacityGrantV1,
    },
    Activate {
        acquisition: CompletionLeaseAcquisitionId,
        expected_acquisition_revision: ResourceRevision,
        current_boundary: u64,
    },
    AcknowledgeParticipantConsumed {
        acquisition: CompletionLeaseAcquisitionId,
        expected_acquisition_revision: ResourceRevision,
        participant: String,
        provider_source: DomainRecordVersionRef,
        grant: CompletionCapacityGrantV1,
    },
    AcknowledgeParticipantCompleted {
        acquisition: CompletionLeaseAcquisitionId,
        participant: String,
    },
    AcknowledgeParticipantReleased {
        acquisition: CompletionLeaseAcquisitionId,
        expected_acquisition_revision: ResourceRevision,
        participant: String,
        provider_source: DomainRecordVersionRef,
        grant: CompletionCapacityGrantV1,
    },
    Expire(ExpireCompletionCapacityV1),
}

impl ProductionCompletionIngressV1 {
    #[must_use]
    pub fn acquisition(&self) -> Option<&CompletionLeaseAcquisitionId> {
        match self {
            Self::GrantLocal(value) => Some(&value.acquisition),
            Self::AcknowledgeParticipantGrant { acquisition, .. }
            | Self::AcknowledgeParticipantPrepared { acquisition, .. }
            | Self::Activate { acquisition, .. }
            | Self::AcknowledgeParticipantConsumed { acquisition, .. }
            | Self::AcknowledgeParticipantCompleted { acquisition, .. }
            | Self::AcknowledgeParticipantReleased { acquisition, .. } => Some(acquisition),
            Self::PrepareLocal(value) => Some(&value.acquisition),
            Self::Expire(_) => None,
        }
    }
}

impl ProductionState {
    pub fn request_completion_acquisition(
        &mut self,
        request: RequestCompletionLeaseV1,
    ) -> Result<CompletionLeaseAcquisitionV1, CanwuError> {
        let budget = self.production_budget()?.clone();
        budget.validate().map_err(resource_error)?;
        request
            .eligibility_envelope
            .validate()
            .map_err(resource_error)?;
        request.recipe.validate().map_err(resource_error)?;
        let expected = BTreeSet::from([
            crate::PLUGIN_NAME.to_owned(),
            canwu_resource::PLUGIN_NAME.to_owned(),
        ]);
        if request.expected_participants != expected
            || request.operation_namespace != PRODUCTION_COMPLETION_OPERATION_NAMESPACE
            || request.recipe.receipts != canwu_resource::MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE
            || request.recipe.mutations == 0
            || request.recipe.reports_per_holder == 0
            || request.recipe.holders == 0
            || request.recipe.bytes == 0
        {
            return Err(invalid(
                "production completion acquisition must reserve the full production/resource terminal path",
            ));
        }
        if let Some(existing) = self.completion_acquisitions.get(&request.id) {
            let recipe_digest = request.recipe.digest().map_err(resource_error)?;
            if existing.operation_key == request.operation_key
                && existing.holder == request.holder
                && existing.operation_namespace == request.operation_namespace
                && existing.eligibility_time == request.eligibility_time
                && existing.eligibility_envelope == request.eligibility_envelope
                && existing.recipe_digest == recipe_digest
                && existing.expected_participants == request.expected_participants
                && existing.policy_class == request.policy_class
            {
                return Ok(existing.clone());
            }
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                "production completion acquisition key was reused with changed input",
            ));
        }
        if self
            .completion_acquisitions
            .values()
            .any(|value| value.operation_key == request.operation_key)
        {
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                "production completion operation key already belongs to another acquisition",
            ));
        }
        let partition = budget
            .partitions
            .iter()
            .find(|partition| {
                partition.authority == request.holder
                    && partition.operation_namespace == request.operation_namespace
            })
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "production completion authority has no run-budget partition",
                )
            })?
            .clone();
        let pending_global = self
            .completion_acquisitions
            .values()
            .filter(|value| acquisition_pending(value.state))
            .count();
        let pending_holder = self
            .completion_acquisitions
            .values()
            .filter(|value| value.holder == request.holder && acquisition_pending(value.state))
            .count();
        if pending_global >= MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL
            || pending_holder >= MAX_PENDING_LEASE_ACQUISITIONS_PER_AUTHORITY
        {
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                "production completion pending-admission capacity is exhausted",
            ));
        }
        let units = request.recipe.canonical_units().map_err(resource_error)?;
        if (request.policy_class == CompletionPolicyClassV1::Guaranteed
            && units > partition.guaranteed_units)
            || (request.policy_class == CompletionPolicyClassV1::SharedBurst
                && units > partition.maximum_burst_units)
        {
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                "production completion recipe exceeds its authority partition",
            ));
        }
        let epoch_key = completion_epoch_key(&request.holder, &request.operation_namespace)?;
        let minute = request.eligibility_time.as_minutes();
        let entry = self
            .completion_admission_epochs
            .entry(epoch_key)
            .or_insert_with(|| ProductionCompletionAdmissionEpochV1 {
                holder: request.holder.clone(),
                operation_namespace: request.operation_namespace.clone(),
                epoch: AdmissionEpochV1 {
                    at: request.eligibility_time,
                    token_balance: partition.request_token_capacity,
                    last_refill_minute: minute,
                    next_eligible_minute: minute,
                    root_acquisition_count: 0,
                },
            });
        if entry.holder != request.holder
            || entry.operation_namespace != request.operation_namespace
            || request.eligibility_time < entry.epoch.at
        {
            return Err(invalid(
                "production completion admission epoch is forged or regressed",
            ));
        }
        if request.eligibility_time > entry.epoch.at {
            let elapsed = minute.saturating_sub(entry.epoch.last_refill_minute);
            let refill = elapsed
                / i64::try_from(partition.request_token_refill_minutes)
                    .map_err(|_| invalid("production completion refill overflowed"))?;
            if refill > 0 {
                entry.epoch.token_balance = entry
                    .epoch
                    .token_balance
                    .saturating_add(u16::try_from(refill).unwrap_or(u16::MAX))
                    .min(partition.request_token_capacity);
                entry.epoch.last_refill_minute = minute;
            }
            entry.epoch.at = request.eligibility_time;
            entry.epoch.root_acquisition_count = 0;
        }
        if minute < entry.epoch.next_eligible_minute
            || entry.epoch.token_balance == 0
            || entry.epoch.root_acquisition_count >= partition.root_acquisition_cap_per_sim_time
        {
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                "production completion request rate or cooldown is exhausted",
            ));
        }
        entry.epoch.token_balance -= 1;
        entry.epoch.root_acquisition_count += 1;
        entry.epoch.next_eligible_minute = minute
            .checked_add(
                i64::try_from(partition.reacquire_cooldown_minutes)
                    .map_err(|_| invalid("production completion cooldown overflowed"))?,
            )
            .ok_or_else(|| invalid("production completion cooldown overflowed"))?;
        let admitted_sequence = self.next_completion_sequence()?;
        let recipe_digest = request.recipe.digest().map_err(resource_error)?;
        let acquisition = CompletionLeaseAcquisitionV1 {
            id: request.id.clone(),
            revision: ResourceRevision::INITIAL,
            operation_key: request.operation_key,
            holder: request.holder,
            operation_namespace: request.operation_namespace,
            eligibility_time: request.eligibility_time,
            eligibility_envelope: request.eligibility_envelope,
            recipe: request.recipe,
            recipe_digest,
            expected_participants: request.expected_participants,
            grants: BTreeMap::new(),
            policy_class: request.policy_class,
            admitted_sequence,
            fairness_round: 0,
            state: CompletionLeaseAcquisitionStateV1::Requested,
            blocker: None,
            refunded_units: 0,
        };
        self.completion_acquisitions
            .insert(acquisition.id.clone(), acquisition.clone());
        self.record_completion_receipt(
            acquisition.operation_key.clone(),
            acquisition.id.clone(),
            None,
            CompletionLeaseReceiptActionV1::Requested,
            None,
        )?;
        Ok(acquisition)
    }

    pub fn apply_completion_ingress(
        &mut self,
        operation: &ProductionCompletionIngressV1,
    ) -> Result<Option<CompletionLeaseActivationCertificateV1>, CanwuError> {
        match operation {
            ProductionCompletionIngressV1::GrantLocal(request) => {
                self.grant_local_completion(request.clone())?;
                Ok(None)
            }
            ProductionCompletionIngressV1::AcknowledgeParticipantGrant {
                acquisition,
                expected_acquisition_revision,
                participant,
                provider_source,
                grant,
            } => {
                self.acknowledge_participant_grant(
                    acquisition,
                    *expected_acquisition_revision,
                    participant,
                    provider_source,
                    grant,
                    CompletionGrantStateV1::Held,
                )?;
                Ok(None)
            }
            ProductionCompletionIngressV1::PrepareLocal(request) => {
                self.prepare_local_completion(request.clone())?;
                Ok(None)
            }
            ProductionCompletionIngressV1::AcknowledgeParticipantPrepared {
                acquisition,
                expected_acquisition_revision,
                participant,
                provider_source,
                grant,
            } => {
                self.acknowledge_participant_grant(
                    acquisition,
                    *expected_acquisition_revision,
                    participant,
                    provider_source,
                    grant,
                    CompletionGrantStateV1::Prepared,
                )?;
                Ok(None)
            }
            ProductionCompletionIngressV1::Activate {
                acquisition,
                expected_acquisition_revision,
                current_boundary,
            } => self
                .activate_completion(
                    acquisition,
                    *expected_acquisition_revision,
                    *current_boundary,
                )
                .map(Some),
            ProductionCompletionIngressV1::AcknowledgeParticipantConsumed {
                acquisition,
                expected_acquisition_revision,
                participant,
                provider_source,
                grant,
            } => {
                self.acknowledge_participant_consumed(
                    acquisition,
                    *expected_acquisition_revision,
                    participant,
                    provider_source,
                    grant,
                )?;
                Ok(None)
            }
            ProductionCompletionIngressV1::AcknowledgeParticipantCompleted { .. } => Err(invalid(
                "completed participant acknowledgements require the authoritative provider resolver",
            )),
            ProductionCompletionIngressV1::AcknowledgeParticipantReleased {
                acquisition,
                expected_acquisition_revision,
                participant,
                provider_source,
                grant,
            } => {
                self.acknowledge_participant_terminal(
                    acquisition,
                    *expected_acquisition_revision,
                    participant,
                    provider_source,
                    grant,
                )?;
                Ok(None)
            }
            ProductionCompletionIngressV1::Expire(request) => {
                self.expire_local_completion(request)?;
                Ok(None)
            }
        }
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn abort_completion_acquisition(
        &mut self,
        request: &AbortCompletionLeaseV1,
    ) -> Result<&'static str, CanwuError> {
        let snapshot = self
            .completion_acquisitions
            .get(&request.acquisition)
            .cloned()
            .ok_or_else(|| invalid("production completion acquisition is unavailable"))?;
        if snapshot.holder != request.holder || snapshot.revision != request.expected_revision {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "production completion abort holder or exact revision differs",
            ));
        }
        if snapshot.state == CompletionLeaseAcquisitionStateV1::Activated {
            return Ok("already_activated");
        }
        if matches!(
            snapshot.state,
            CompletionLeaseAcquisitionStateV1::Released
                | CompletionLeaseAcquisitionStateV1::Expired
        ) {
            return Ok("already_terminal");
        }
        if let Some(grant_id) = snapshot.grants.get(crate::PLUGIN_NAME)
            && let Some(grant) = self.production_completion_grants.get_mut(grant_id)
            && matches!(
                grant.state,
                CompletionGrantStateV1::Held | CompletionGrantStateV1::Prepared
            )
        {
            grant.state = CompletionGrantStateV1::Released;
            grant.revision = grant.revision.next().map_err(resource_error)?;
            self.completion_reserved_units = self
                .completion_reserved_units
                .checked_sub(grant.reserved_units)
                .ok_or_else(|| invalid("production completion reserve underflowed"))?;
            remove_completion_locks(&mut self.completion_target_locks, grant);
        }
        let terminal = snapshot
            .grants
            .iter()
            .filter(|(participant, _)| participant.as_str() != crate::PLUGIN_NAME)
            .all(|(participant, _)| {
                self.completion_participant_grants
                    .get(&request.acquisition)
                    .and_then(|values| values.get(participant))
                    .is_some_and(|value| grant_terminal(value.grant.state))
            });
        let (operation_key, acquisition_id) = {
            let acquisition = self
                .completion_acquisitions
                .get_mut(&request.acquisition)
                .expect("production completion acquisition was checked");
            acquisition.state = if terminal {
                CompletionLeaseAcquisitionStateV1::Released
            } else {
                CompletionLeaseAcquisitionStateV1::Aborting
            };
            acquisition.blocker = Some("holder_aborted".to_owned());
            acquisition.revision = acquisition.revision.next().map_err(resource_error)?;
            (acquisition.operation_key.clone(), acquisition.id.clone())
        };
        self.record_completion_receipt(
            operation_key,
            acquisition_id,
            None,
            CompletionLeaseReceiptActionV1::Aborted,
            Some("holder_aborted".to_owned()),
        )?;
        Ok("aborting")
    }

    pub fn completion_status_for(
        &self,
        holder: &KnowledgeHolderRef,
        acquisition_id: &CompletionLeaseAcquisitionId,
    ) -> Result<CompletionLeaseStatusDtoV1, CanwuError> {
        let acquisition = self
            .completion_acquisitions
            .get(acquisition_id)
            .ok_or_else(|| invalid("production completion acquisition is unavailable"))?;
        if &acquisition.holder != holder {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "production completion status is holder-bound",
            ));
        }
        let mut grant_states = BTreeMap::new();
        let mut exact_grant_versions = BTreeMap::new();
        let mut expiry_boundaries = BTreeMap::new();
        let mut activation_deadlines = BTreeMap::new();
        for (participant, grant_id) in &acquisition.grants {
            let grant = if participant == crate::PLUGIN_NAME {
                self.production_completion_grants.get(grant_id)
            } else {
                self.completion_participant_grants
                    .get(acquisition_id)
                    .and_then(|values| values.get(participant))
                    .map(|value| &value.grant)
            }
            .ok_or_else(|| invalid("production completion status lost a participant grant"))?;
            grant_states.insert(participant.clone(), grant.state);
            exact_grant_versions.insert(participant.clone(), grant.revision);
            expiry_boundaries.insert(participant.clone(), grant.expires_after_boundary);
            if let Some(deadline) = grant.activation_deadline_boundary {
                activation_deadlines.insert(participant.clone(), deadline);
            }
        }
        Ok(CompletionLeaseStatusDtoV1 {
            acquisition: acquisition.id.clone(),
            operation_key: acquisition.operation_key.clone(),
            state: acquisition.state,
            grant_states,
            exact_grant_versions,
            eligibility_time: acquisition.eligibility_time,
            expiry_boundaries,
            activation_deadlines,
            blocker: acquisition.blocker.clone(),
            refunded_units: acquisition.refunded_units,
            next_eligible_action: completion_next_action(acquisition.state).to_owned(),
        })
    }

    pub(crate) fn consume_local_completion_grant(
        &mut self,
        acquisition_id: &CompletionLeaseAcquisitionId,
        at: SimTime,
    ) -> Result<
        (
            CompletionLeaseActivationCertificateV1,
            CompletionCapacityGrantId,
        ),
        CanwuError,
    > {
        let certificate = self
            .production_completion_certificates
            .get(acquisition_id)
            .cloned()
            .ok_or_else(|| invalid("production completion certificate is unavailable"))?;
        let acquisition = self
            .completion_acquisitions
            .get(acquisition_id)
            .ok_or_else(|| invalid("production completion acquisition is unavailable"))?;
        let grant_id = acquisition
            .grants
            .get(crate::PLUGIN_NAME)
            .cloned()
            .ok_or_else(|| invalid("production completion acquisition lost its local grant"))?;
        if acquisition.state != CompletionLeaseAcquisitionStateV1::Activated
            || certificate.eligibility_time != at
        {
            return Err(invalid(
                "production completion certificate is not activated at the execution eligibility time",
            ));
        }
        let grant = self
            .production_completion_grants
            .get_mut(&grant_id)
            .ok_or_else(|| invalid("production completion local grant is unavailable"))?;
        if grant.state != CompletionGrantStateV1::Prepared
            || !certificate
                .prepared_grants
                .contains(&(grant.id.clone(), grant.revision))
            || grant.target_versions.iter().any(|target| {
                completion_lock_key(target)
                    .ok()
                    .and_then(|key| self.completion_target_locks.get(&key))
                    != Some(&(target.clone(), grant.id.clone()))
            })
        {
            return Err(invalid(
                "production completion certificate does not bind the exact local prepared grant",
            ));
        }
        grant.state = CompletionGrantStateV1::Consumed;
        grant.revision = grant.revision.next().map_err(resource_error)?;
        Ok((certificate, grant_id))
    }

    pub(crate) fn complete_local_completion_grant(
        &mut self,
        acquisition_id: &CompletionLeaseAcquisitionId,
        grant_id: &CompletionCapacityGrantId,
    ) -> Result<(), CanwuError> {
        let (operation_key, completed_grant) = {
            let grant = self
                .production_completion_grants
                .get_mut(grant_id)
                .ok_or_else(|| invalid("production completion local grant is unavailable"))?;
            if grant.acquisition != *acquisition_id {
                return Err(invalid(
                    "production completion grant names another acquisition",
                ));
            }
            if grant.state == CompletionGrantStateV1::Completed {
                return Ok(());
            }
            if grant.state != CompletionGrantStateV1::Consumed {
                return Err(invalid(
                    "production completion local grant was not consumed",
                ));
            }
            grant.state = CompletionGrantStateV1::Completed;
            grant.revision = grant.revision.next().map_err(resource_error)?;
            self.completion_reserved_units = self
                .completion_reserved_units
                .checked_sub(grant.reserved_units)
                .ok_or_else(|| invalid("production completion reserve underflowed"))?;
            remove_completion_locks(&mut self.completion_target_locks, grant);
            (grant.operation_key.clone(), grant.id.clone())
        };
        self.record_completion_receipt(
            operation_key,
            acquisition_id.clone(),
            Some(completed_grant),
            CompletionLeaseReceiptActionV1::Completed,
            None,
        )?;
        self.release_acquisition_if_terminal(acquisition_id)?;
        Ok(())
    }

    pub(crate) fn acknowledge_completed_participant(
        &mut self,
        acquisition_id: &CompletionLeaseAcquisitionId,
        participant: &str,
        provider_source: &DomainRecordVersionRef,
        grant: &CompletionCapacityGrantV1,
    ) -> Result<(), CanwuError> {
        let expected_revision = self
            .completion_acquisitions
            .get(acquisition_id)
            .ok_or_else(|| invalid("production completion acquisition is unavailable"))?
            .revision;
        self.acknowledge_participant_terminal(
            acquisition_id,
            expected_revision,
            participant,
            provider_source,
            grant,
        )
    }

    pub(crate) fn validate_completion_coordinator(&self) -> Result<(), CanwuError> {
        let Some(budget) = &self.production_run_budget else {
            if self.completion_acquisitions.is_empty()
                && self.production_completion_grants.is_empty()
                && self.completion_participant_grants.is_empty()
                && self.production_completion_certificates.is_empty()
            {
                return Ok(());
            }
            return Err(invalid(
                "production completion coordinator requires a run budget",
            ));
        };
        budget.validate().map_err(resource_error)?;
        let mut reserved = 0_u64;
        for (id, acquisition) in &self.completion_acquisitions {
            acquisition
                .eligibility_envelope
                .validate()
                .map_err(resource_error)?;
            if id != &acquisition.id
                || acquisition.recipe_digest
                    != acquisition.recipe.digest().map_err(resource_error)?
                || acquisition.expected_participants
                    != BTreeSet::from([
                        crate::PLUGIN_NAME.to_owned(),
                        canwu_resource::PLUGIN_NAME.to_owned(),
                    ])
                || acquisition
                    .grants
                    .keys()
                    .any(|participant| !acquisition.expected_participants.contains(participant))
            {
                return Err(invalid(
                    "production completion acquisition closure is invalid",
                ));
            }
            if matches!(
                acquisition.state,
                CompletionLeaseAcquisitionStateV1::FullyGranted
                    | CompletionLeaseAcquisitionStateV1::Preparing
                    | CompletionLeaseAcquisitionStateV1::PreparedAll
                    | CompletionLeaseAcquisitionStateV1::Activated
                    | CompletionLeaseAcquisitionStateV1::Aborting
                    | CompletionLeaseAcquisitionStateV1::Released
            ) && acquisition.grants.len() != acquisition.expected_participants.len()
            {
                return Err(invalid(
                    "production completion acquisition lost a required participant grant",
                ));
            }
            for (participant, grant_id) in &acquisition.grants {
                let exact = if participant == crate::PLUGIN_NAME {
                    self.production_completion_grants
                        .get(grant_id)
                        .is_some_and(|grant| grant.acquisition == *id)
                } else {
                    self.completion_participant_grants
                        .get(id)
                        .and_then(|participants| participants.get(participant))
                        .is_some_and(|value| {
                            value.grant.id == *grant_id && value.grant.acquisition == *id
                        })
                };
                if !exact {
                    return Err(invalid(
                        "production completion acquisition grant map differs from its authoritative participant state",
                    ));
                }
            }
            if matches!(
                acquisition.state,
                CompletionLeaseAcquisitionStateV1::Activated
                    | CompletionLeaseAcquisitionStateV1::Released
            ) != self.production_completion_certificates.contains_key(id)
            {
                return Err(invalid(
                    "production completion acquisition certificate closure is invalid",
                ));
            }
        }
        for (id, grant) in &self.production_completion_grants {
            if id != &grant.id
                || grant.owner_plugin != crate::PLUGIN_NAME
                || grant.run_budget_revision != budget.revision
                || !self
                    .completion_acquisitions
                    .contains_key(&grant.acquisition)
            {
                return Err(invalid("production completion local grant is invalid"));
            }
            if matches!(
                grant.state,
                CompletionGrantStateV1::Held
                    | CompletionGrantStateV1::Prepared
                    | CompletionGrantStateV1::Consumed
            ) {
                reserved = reserved
                    .checked_add(grant.reserved_units)
                    .ok_or_else(|| invalid("production completion reserve overflowed"))?;
            }
        }
        if reserved != self.completion_reserved_units || reserved > budget.total_completion_units {
            return Err(invalid("production completion reserve does not reconcile"));
        }
        for (acquisition, participants) in &self.completion_participant_grants {
            if !self.completion_acquisitions.contains_key(acquisition) {
                return Err(invalid("production participant grant is orphaned"));
            }
            for (participant, value) in participants {
                if participant != &value.participant
                    || participant != &value.grant.owner_plugin
                    || value.grant.acquisition != *acquisition
                {
                    return Err(invalid("production participant grant mirror is invalid"));
                }
            }
        }
        for (acquisition, certificate) in &self.production_completion_certificates {
            let mut detached = certificate.clone();
            let recorded = std::mem::take(&mut detached.semantic_digest);
            if acquisition != &certificate.acquisition
                || recorded
                    != canonical_digest(
                        "canwu.resource.completion-activation-certificate.v1",
                        &detached,
                    )
                    .map_err(resource_error)?
                || self
                    .completion_acquisitions
                    .get(acquisition)
                    .is_none_or(|value| {
                        value.state != CompletionLeaseAcquisitionStateV1::Activated
                            && value.state != CompletionLeaseAcquisitionStateV1::Released
                    })
            {
                return Err(invalid("production completion certificate is forged"));
            }
        }
        for (key, (target, grant_id)) in &self.completion_target_locks {
            if key != &completion_lock_key(target)?
                || self
                    .production_completion_grants
                    .get(grant_id)
                    .is_none_or(|grant| {
                        !grant.target_versions.contains(target)
                            || !matches!(
                                grant.state,
                                CompletionGrantStateV1::Prepared | CompletionGrantStateV1::Consumed
                            )
                    })
            {
                return Err(invalid("production completion target lock is invalid"));
            }
        }
        for receipt in self.completion_receipts.values() {
            let mut detached = receipt.clone();
            let recorded = std::mem::take(&mut detached.semantic_digest);
            if recorded
                != canonical_digest("canwu.resource.completion-lease-receipt.v1", &detached)
                    .map_err(resource_error)?
            {
                return Err(invalid("production completion receipt is forged"));
            }
        }
        Ok(())
    }

    fn production_budget(&self) -> Result<&RunBudgetRevisionV1, CanwuError> {
        self.production_run_budget
            .as_ref()
            .ok_or_else(|| invalid("production run budget is not installed"))
    }

    fn grant_local_completion(
        &mut self,
        request: GrantCompletionCapacityV1,
    ) -> Result<(), CanwuError> {
        let budget = self.production_budget()?.clone();
        let acquisition = self
            .completion_acquisitions
            .get(&request.acquisition)
            .cloned()
            .ok_or_else(|| invalid("production completion acquisition is unavailable"))?;
        if deterministic_completion_fairness_order(self.completion_acquisitions.values().cloned())
            .first()
            != Some(&request.acquisition)
            || acquisition.revision != request.expected_acquisition_revision
            || acquisition.state != CompletionLeaseAcquisitionStateV1::Requested
            || request.owner_plugin != crate::PLUGIN_NAME
            || request.target_versions.is_empty()
            || request
                .target_versions
                .iter()
                .any(|target| !matches!(target, CompletionLockedTargetV1::ExternalRecord { .. }))
        {
            return Err(invalid(
                "production completion local grant is stale, unfairly ordered, or not exactly production-owned",
            ));
        }
        let units = acquisition
            .recipe
            .canonical_units()
            .map_err(resource_error)?;
        if self
            .completion_reserved_units
            .checked_add(units)
            .is_none_or(|value| value > budget.total_completion_units)
        {
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                "production completion local capacity is exhausted",
            ));
        }
        let mut targets = request.target_versions.clone();
        targets.sort();
        targets.dedup();
        if targets != request.target_versions {
            return Err(invalid(
                "production completion local targets are not canonical",
            ));
        }
        let grant = CompletionCapacityGrantV1 {
            id: request.grant_id.clone(),
            revision: ResourceRevision::INITIAL,
            acquisition: acquisition.id.clone(),
            operation_key: acquisition.operation_key.clone(),
            owner_plugin: crate::PLUGIN_NAME.to_owned(),
            run_budget_revision: budget.revision,
            target_versions: request.target_versions,
            recipe_digest: acquisition.recipe_digest.clone(),
            reserved_units: units,
            expires_after_boundary: request
                .current_boundary
                .checked_add(canwu_resource::PREACTIVATION_LEASE_TTL_BOUNDARIES)
                .ok_or_else(|| invalid("production completion expiry overflowed"))?,
            activation_deadline_boundary: None,
            state: CompletionGrantStateV1::Held,
            rejection: None,
        };
        self.completion_reserved_units = self
            .completion_reserved_units
            .checked_add(units)
            .ok_or_else(|| invalid("production completion reserve overflowed"))?;
        self.completion_expiry_due
            .entry(grant.expires_after_boundary)
            .or_default()
            .insert(acquisition.id.clone());
        self.production_completion_grants
            .insert(grant.id.clone(), grant.clone());
        let acquisition_mut = self
            .completion_acquisitions
            .get_mut(&acquisition.id)
            .expect("production completion acquisition was checked");
        acquisition_mut
            .grants
            .insert(crate::PLUGIN_NAME.to_owned(), grant.id.clone());
        acquisition_mut.state = CompletionLeaseAcquisitionStateV1::PartiallyGranted;
        acquisition_mut.revision = acquisition_mut.revision.next().map_err(resource_error)?;
        self.record_completion_receipt(
            acquisition.operation_key,
            acquisition.id,
            Some(grant.id),
            CompletionLeaseReceiptActionV1::Granted,
            None,
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn acknowledge_participant_grant(
        &mut self,
        acquisition_id: &CompletionLeaseAcquisitionId,
        expected_revision: ResourceRevision,
        participant: &str,
        provider_source: &DomainRecordVersionRef,
        grant: &CompletionCapacityGrantV1,
        expected_state: CompletionGrantStateV1,
    ) -> Result<(), CanwuError> {
        if participant == crate::PLUGIN_NAME
            || participant != canwu_resource::PLUGIN_NAME
            || grant.owner_plugin != participant
            || grant.state != expected_state
            || grant.acquisition != *acquisition_id
        {
            return Err(invalid(
                "production completion participant acknowledgement is invalid",
            ));
        }
        let acquisition = self
            .completion_acquisitions
            .get(acquisition_id)
            .cloned()
            .ok_or_else(|| invalid("production completion acquisition is unavailable"))?;
        if acquisition.revision != expected_revision
            || acquisition.operation_key != grant.operation_key
            || acquisition.recipe_digest != grant.recipe_digest
        {
            return Err(CanwuError::new(
                ErrorCode::DomainRecordVersionConflict,
                "production completion participant acknowledgement is stale",
            ));
        }
        let participants = self
            .completion_participant_grants
            .entry(acquisition_id.clone())
            .or_default();
        if let Some(existing) = participants.get(participant) {
            if existing.grant == *grant && existing.provider_source == *provider_source {
                return Ok(());
            }
            let expected_previous_state = if expected_state == CompletionGrantStateV1::Prepared {
                CompletionGrantStateV1::Held
            } else {
                return Err(invalid(
                    "production completion participant grant transition is invalid",
                ));
            };
            if existing.grant.state != expected_previous_state
                || grant.revision != existing.grant.revision.next().map_err(resource_error)?
                || !grant_lineage_matches(&existing.grant, grant)
            {
                return Err(CanwuError::new(
                    ErrorCode::DomainRecordVersionConflict,
                    "production completion participant grant transition is stale",
                ));
            }
        }
        participants.insert(
            participant.to_owned(),
            ProductionCompletionParticipantGrantV1 {
                participant: participant.to_owned(),
                provider_source: provider_source.clone(),
                grant: grant.clone(),
            },
        );
        let acquisition_mut = self
            .completion_acquisitions
            .get_mut(acquisition_id)
            .expect("production completion acquisition was checked");
        acquisition_mut
            .grants
            .insert(participant.to_owned(), grant.id.clone());
        recompute_acquisition_state(
            acquisition_mut,
            &self.production_completion_grants,
            participants,
        );
        acquisition_mut.revision = acquisition_mut.revision.next().map_err(resource_error)?;
        self.record_completion_receipt(
            acquisition.operation_key,
            acquisition.id,
            Some(grant.id.clone()),
            if expected_state == CompletionGrantStateV1::Prepared {
                CompletionLeaseReceiptActionV1::Prepared
            } else {
                CompletionLeaseReceiptActionV1::Granted
            },
            None,
        )?;
        Ok(())
    }

    #[allow(clippy::needless_pass_by_value)]
    fn prepare_local_completion(
        &mut self,
        request: PrepareCompletionCapacityV1,
    ) -> Result<(), CanwuError> {
        let acquisition = self
            .completion_acquisitions
            .get(&request.acquisition)
            .cloned()
            .ok_or_else(|| invalid("production completion acquisition is unavailable"))?;
        let grant = self
            .production_completion_grants
            .get(&request.grant)
            .cloned()
            .ok_or_else(|| invalid("production completion local grant is unavailable"))?;
        if acquisition.revision != request.expected_acquisition_revision
            || !matches!(
                acquisition.state,
                CompletionLeaseAcquisitionStateV1::FullyGranted
                    | CompletionLeaseAcquisitionStateV1::Preparing
            )
            || acquisition.eligibility_envelope.digest != request.eligibility_envelope_digest
            || grant.revision != request.expected_grant_revision
            || grant.state != CompletionGrantStateV1::Held
            || request
                .current_boundary
                .checked_add(canwu_resource::ACTIVATION_GUARD_BOUNDARIES)
                .is_none_or(|guard| guard > grant.expires_after_boundary)
        {
            return Err(invalid("production completion local prepare is stale"));
        }
        for target in &grant.target_versions {
            let key = completion_lock_key(target)?;
            if self.completion_target_locks.contains_key(&key) {
                return Err(CanwuError::new(
                    ErrorCode::DomainRecordVersionConflict,
                    "production completion local target is already locked",
                ));
            }
        }
        for target in &grant.target_versions {
            self.completion_target_locks.insert(
                completion_lock_key(target)?,
                (target.clone(), grant.id.clone()),
            );
        }
        let grant_mut = self
            .production_completion_grants
            .get_mut(&grant.id)
            .expect("production completion grant was checked");
        grant_mut.state = CompletionGrantStateV1::Prepared;
        grant_mut.activation_deadline_boundary = Some(
            grant_mut
                .expires_after_boundary
                .saturating_sub(canwu_resource::ACTIVATION_GUARD_BOUNDARIES - 1),
        );
        grant_mut.revision = grant_mut.revision.next().map_err(resource_error)?;
        let participants = self
            .completion_participant_grants
            .get(&acquisition.id)
            .cloned()
            .unwrap_or_default();
        let acquisition_mut = self
            .completion_acquisitions
            .get_mut(&acquisition.id)
            .expect("production completion acquisition was checked");
        recompute_acquisition_state(
            acquisition_mut,
            &self.production_completion_grants,
            &participants,
        );
        acquisition_mut.revision = acquisition_mut.revision.next().map_err(resource_error)?;
        self.record_completion_receipt(
            acquisition.operation_key,
            acquisition.id,
            Some(grant.id),
            CompletionLeaseReceiptActionV1::Prepared,
            None,
        )?;
        Ok(())
    }

    fn activate_completion(
        &mut self,
        acquisition_id: &CompletionLeaseAcquisitionId,
        expected_revision: ResourceRevision,
        current_boundary: u64,
    ) -> Result<CompletionLeaseActivationCertificateV1, CanwuError> {
        if let Some(existing) = self.production_completion_certificates.get(acquisition_id) {
            return Ok(existing.clone());
        }
        let acquisition = self
            .completion_acquisitions
            .get(acquisition_id)
            .cloned()
            .ok_or_else(|| invalid("production completion acquisition is unavailable"))?;
        if acquisition.revision != expected_revision
            || acquisition.state != CompletionLeaseAcquisitionStateV1::PreparedAll
        {
            return Err(invalid(
                "production completion activation is stale or not prepared",
            ));
        }
        let mut prepared_grants = Vec::new();
        let mut locked_targets = Vec::new();
        let mut earliest_deadline = u64::MAX;
        for (participant, grant_id) in &acquisition.grants {
            let grant = if participant == crate::PLUGIN_NAME {
                self.production_completion_grants.get(grant_id)
            } else {
                self.completion_participant_grants
                    .get(acquisition_id)
                    .and_then(|values| values.get(participant))
                    .map(|value| &value.grant)
            }
            .ok_or_else(|| invalid("production completion activation lost a participant grant"))?;
            if grant.state != CompletionGrantStateV1::Prepared {
                return Err(invalid(
                    "production completion activation requires all grants prepared",
                ));
            }
            earliest_deadline = earliest_deadline.min(
                grant
                    .activation_deadline_boundary
                    .ok_or_else(|| invalid("prepared participant grant lacks a deadline"))?,
            );
            prepared_grants.push((grant.id.clone(), grant.revision));
            locked_targets.extend(grant.target_versions.iter().cloned());
        }
        if current_boundary >= earliest_deadline {
            return Err(invalid(
                "production completion activation window was missed",
            ));
        }
        prepared_grants.sort();
        locked_targets.sort();
        locked_targets.dedup();
        let acquisition_mut = self
            .completion_acquisitions
            .get_mut(acquisition_id)
            .expect("production completion acquisition was checked");
        acquisition_mut.state = CompletionLeaseAcquisitionStateV1::Activated;
        acquisition_mut.revision = acquisition_mut.revision.next().map_err(resource_error)?;
        let mut certificate = CompletionLeaseActivationCertificateV1 {
            acquisition: acquisition.id.clone(),
            acquisition_revision: acquisition_mut.revision,
            operation_key: acquisition.operation_key.clone(),
            prepared_grants,
            locked_target_versions: locked_targets,
            recipe_digest: acquisition.recipe_digest,
            eligibility_time: acquisition.eligibility_time,
            eligibility_envelope_digest: acquisition.eligibility_envelope.digest,
            activation_boundary: current_boundary,
            semantic_digest: String::new(),
        };
        certificate.semantic_digest = canonical_digest(
            "canwu.resource.completion-activation-certificate.v1",
            &certificate,
        )
        .map_err(resource_error)?;
        self.production_completion_certificates
            .insert(acquisition.id.clone(), certificate.clone());
        for values in self.completion_expiry_due.values_mut() {
            values.remove(acquisition_id);
        }
        self.completion_expiry_due
            .retain(|_, values| !values.is_empty());
        self.record_completion_receipt(
            acquisition.operation_key,
            acquisition.id,
            None,
            CompletionLeaseReceiptActionV1::Activated,
            None,
        )?;
        Ok(certificate)
    }

    fn acknowledge_participant_terminal(
        &mut self,
        acquisition_id: &CompletionLeaseAcquisitionId,
        expected_revision: ResourceRevision,
        participant: &str,
        provider_source: &DomainRecordVersionRef,
        grant: &CompletionCapacityGrantV1,
    ) -> Result<(), CanwuError> {
        if !grant_terminal(grant.state) {
            return Err(invalid(
                "production participant terminal acknowledgement is not terminal",
            ));
        }
        let acquisition = self
            .completion_acquisitions
            .get(acquisition_id)
            .cloned()
            .ok_or_else(|| invalid("production completion acquisition is unavailable"))?;
        if acquisition.revision != expected_revision
            || participant == crate::PLUGIN_NAME
            || grant.owner_plugin != participant
            || grant.acquisition != *acquisition_id
        {
            return Err(invalid(
                "production participant terminal acknowledgement is stale",
            ));
        }
        let participants = self
            .completion_participant_grants
            .get(acquisition_id)
            .ok_or_else(|| invalid("production participant terminal mirror is unavailable"))?;
        let existing = participants
            .get(participant)
            .ok_or_else(|| invalid("production participant terminal mirror is unavailable"))?;
        if existing.provider_source == *provider_source && existing.grant == *grant {
            return Ok(());
        }
        let valid_predecessor = match grant.state {
            CompletionGrantStateV1::Completed => {
                existing.grant.state == CompletionGrantStateV1::Consumed
            }
            CompletionGrantStateV1::Released | CompletionGrantStateV1::Expired => {
                matches!(
                    existing.grant.state,
                    CompletionGrantStateV1::Held | CompletionGrantStateV1::Prepared
                )
            }
            _ => false,
        };
        if !valid_predecessor
            || grant.revision != existing.grant.revision.next().map_err(resource_error)?
            || !grant_lineage_matches(&existing.grant, grant)
        {
            return Err(CanwuError::new(
                ErrorCode::DomainRecordVersionConflict,
                "production participant terminal acknowledgement skips a grant transition",
            ));
        }
        self.completion_participant_grants
            .entry(acquisition_id.clone())
            .or_default()
            .insert(
                participant.to_owned(),
                ProductionCompletionParticipantGrantV1 {
                    participant: participant.to_owned(),
                    provider_source: provider_source.clone(),
                    grant: grant.clone(),
                },
            );
        let acquisition_mut = self
            .completion_acquisitions
            .get_mut(acquisition_id)
            .expect("production completion acquisition was checked");
        acquisition_mut.revision = acquisition_mut.revision.next().map_err(resource_error)?;
        self.record_completion_receipt(
            acquisition.operation_key,
            acquisition.id.clone(),
            Some(grant.id.clone()),
            if grant.state == CompletionGrantStateV1::Completed {
                CompletionLeaseReceiptActionV1::Completed
            } else {
                CompletionLeaseReceiptActionV1::Released
            },
            grant.rejection.clone(),
        )?;
        self.release_acquisition_if_terminal(&acquisition.id)?;
        Ok(())
    }

    fn release_acquisition_if_terminal(
        &mut self,
        acquisition_id: &CompletionLeaseAcquisitionId,
    ) -> Result<(), CanwuError> {
        let acquisition = self
            .completion_acquisitions
            .get(acquisition_id)
            .cloned()
            .ok_or_else(|| invalid("production completion acquisition is unavailable"))?;
        let all_terminal = acquisition.grants.iter().all(|(owner, grant_id)| {
            if owner == crate::PLUGIN_NAME {
                self.production_completion_grants
                    .get(grant_id)
                    .is_some_and(|value| grant_terminal(value.state))
            } else {
                self.completion_participant_grants
                    .get(acquisition_id)
                    .and_then(|values| values.get(owner))
                    .is_some_and(|value| grant_terminal(value.grant.state))
            }
        });
        if all_terminal {
            let acquisition = self
                .completion_acquisitions
                .get_mut(acquisition_id)
                .expect("production completion acquisition was checked");
            acquisition.state = CompletionLeaseAcquisitionStateV1::Released;
        }
        Ok(())
    }

    fn acknowledge_participant_consumed(
        &mut self,
        acquisition_id: &CompletionLeaseAcquisitionId,
        expected_revision: ResourceRevision,
        participant: &str,
        provider_source: &DomainRecordVersionRef,
        grant: &CompletionCapacityGrantV1,
    ) -> Result<(), CanwuError> {
        let acquisition = self
            .completion_acquisitions
            .get(acquisition_id)
            .cloned()
            .ok_or_else(|| invalid("production completion acquisition is unavailable"))?;
        if acquisition.revision != expected_revision
            || acquisition.state != CompletionLeaseAcquisitionStateV1::Activated
            || participant != canwu_resource::PLUGIN_NAME
            || grant.owner_plugin != participant
            || grant.acquisition != *acquisition_id
            || grant.operation_key != acquisition.operation_key
            || grant.state != CompletionGrantStateV1::Consumed
        {
            return Err(invalid(
                "production participant consumed acknowledgement is stale or invalid",
            ));
        }
        let participants = self
            .completion_participant_grants
            .get(acquisition_id)
            .ok_or_else(|| invalid("production participant consumed mirror is unavailable"))?;
        let existing = participants
            .get(participant)
            .ok_or_else(|| invalid("production participant consumed mirror is unavailable"))?;
        if existing.provider_source == *provider_source && existing.grant == *grant {
            return Ok(());
        }
        if existing.grant.state != CompletionGrantStateV1::Prepared
            || grant.revision != existing.grant.revision.next().map_err(resource_error)?
            || !grant_lineage_matches(&existing.grant, grant)
        {
            return Err(CanwuError::new(
                ErrorCode::DomainRecordVersionConflict,
                "production participant consumed acknowledgement skips a grant transition",
            ));
        }
        self.completion_participant_grants
            .get_mut(acquisition_id)
            .expect("production participant mirror was checked")
            .insert(
                participant.to_owned(),
                ProductionCompletionParticipantGrantV1 {
                    participant: participant.to_owned(),
                    provider_source: provider_source.clone(),
                    grant: grant.clone(),
                },
            );
        let acquisition_mut = self
            .completion_acquisitions
            .get_mut(acquisition_id)
            .expect("production completion acquisition was checked");
        acquisition_mut.revision = acquisition_mut.revision.next().map_err(resource_error)?;
        self.record_completion_receipt(
            acquisition.operation_key,
            acquisition.id,
            Some(grant.id.clone()),
            CompletionLeaseReceiptActionV1::Consumed,
            None,
        )?;
        Ok(())
    }

    fn expire_local_completion(
        &mut self,
        request: &ExpireCompletionCapacityV1,
    ) -> Result<(), CanwuError> {
        let candidates = self
            .completion_expiry_due
            .range(..=request.current_boundary)
            .flat_map(|(_, values)| values.iter().cloned())
            .take(request.candidate_limit.saturating_add(1))
            .collect::<Vec<_>>();
        if request.candidate_limit == 0 || candidates.len() > request.candidate_limit {
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                "production completion expiry due-work budget was exceeded",
            ));
        }
        for acquisition_id in &candidates {
            let acquisition = self
                .completion_acquisitions
                .get(acquisition_id)
                .cloned()
                .ok_or_else(|| invalid("production completion expiry index is orphaned"))?;
            if request.at < acquisition.eligibility_time
                || acquisition.state == CompletionLeaseAcquisitionStateV1::Activated
            {
                continue;
            }
            if let Some(grant_id) = acquisition.grants.get(crate::PLUGIN_NAME)
                && let Some(grant) = self.production_completion_grants.get_mut(grant_id)
                && matches!(
                    grant.state,
                    CompletionGrantStateV1::Held | CompletionGrantStateV1::Prepared
                )
            {
                grant.state = CompletionGrantStateV1::Expired;
                grant.revision = grant.revision.next().map_err(resource_error)?;
                self.completion_reserved_units = self
                    .completion_reserved_units
                    .checked_sub(grant.reserved_units)
                    .ok_or_else(|| invalid("production completion reserve underflowed"))?;
                remove_completion_locks(&mut self.completion_target_locks, grant);
            }
            let acquisition_mut = self
                .completion_acquisitions
                .get_mut(acquisition_id)
                .expect("production completion acquisition was checked");
            acquisition_mut.state = CompletionLeaseAcquisitionStateV1::Expired;
            acquisition_mut.blocker = Some("preactivation_expired".to_owned());
            acquisition_mut.revision = acquisition_mut.revision.next().map_err(resource_error)?;
        }
        for values in self.completion_expiry_due.values_mut() {
            values.retain(|value| !candidates.contains(value));
        }
        self.completion_expiry_due
            .retain(|_, values| !values.is_empty());
        Ok(())
    }

    fn record_completion_receipt(
        &mut self,
        operation_key: ResourceOperationKey,
        acquisition: CompletionLeaseAcquisitionId,
        grant: Option<CompletionCapacityGrantId>,
        action: CompletionLeaseReceiptActionV1,
        reason: Option<String>,
    ) -> Result<(), CanwuError> {
        let sequence = self.next_completion_sequence()?;
        let state = self
            .completion_acquisitions
            .get(&acquisition)
            .map_or(CompletionLeaseAcquisitionStateV1::Released, |value| {
                value.state
            });
        let reserved_units = grant
            .as_ref()
            .and_then(|id| self.production_completion_grants.get(id))
            .map_or(0, |value| value.reserved_units);
        let refunded_units = self
            .completion_acquisitions
            .get(&acquisition)
            .map_or(0, |value| value.refunded_units);
        let mut receipt = CompletionLeaseReceiptV1 {
            sequence,
            operation_key,
            acquisition,
            grant,
            action,
            state,
            reserved_units,
            refunded_units,
            reason,
            semantic_digest: String::new(),
        };
        receipt.semantic_digest =
            canonical_digest("canwu.resource.completion-lease-receipt.v1", &receipt)
                .map_err(resource_error)?;
        self.completion_receipts.insert(sequence, receipt);
        Ok(())
    }

    fn next_completion_sequence(&mut self) -> Result<u64, CanwuError> {
        let current = self.completion_next_sequence.max(1);
        self.completion_next_sequence = current
            .checked_add(1)
            .ok_or_else(|| invalid("production completion sequence overflowed"))?;
        Ok(current)
    }
}

fn recompute_acquisition_state(
    acquisition: &mut CompletionLeaseAcquisitionV1,
    local_grants: &BTreeMap<CompletionCapacityGrantId, CompletionCapacityGrantV1>,
    participant_grants: &BTreeMap<String, ProductionCompletionParticipantGrantV1>,
) {
    let all_present = acquisition.grants.len() == acquisition.expected_participants.len();
    let all_prepared = all_present
        && acquisition.grants.iter().all(|(participant, grant_id)| {
            if participant == crate::PLUGIN_NAME {
                local_grants
                    .get(grant_id)
                    .is_some_and(|value| value.state == CompletionGrantStateV1::Prepared)
            } else {
                participant_grants
                    .get(participant)
                    .is_some_and(|value| value.grant.state == CompletionGrantStateV1::Prepared)
            }
        });
    let any_prepared = acquisition.grants.iter().any(|(participant, grant_id)| {
        if participant == crate::PLUGIN_NAME {
            local_grants
                .get(grant_id)
                .is_some_and(|value| value.state == CompletionGrantStateV1::Prepared)
        } else {
            participant_grants
                .get(participant)
                .is_some_and(|value| value.grant.state == CompletionGrantStateV1::Prepared)
        }
    });
    acquisition.state = if all_prepared {
        CompletionLeaseAcquisitionStateV1::PreparedAll
    } else if any_prepared {
        CompletionLeaseAcquisitionStateV1::Preparing
    } else if all_present {
        CompletionLeaseAcquisitionStateV1::FullyGranted
    } else if acquisition.grants.is_empty() {
        CompletionLeaseAcquisitionStateV1::Requested
    } else {
        CompletionLeaseAcquisitionStateV1::PartiallyGranted
    };
}

fn completion_epoch_key(
    holder: &KnowledgeHolderRef,
    operation_namespace: &str,
) -> Result<String, CanwuError> {
    canwu_api::canonical_hash(
        "canwu.production.completion-admission-epoch-key.v1",
        &(holder, operation_namespace),
    )
}

fn completion_lock_key(target: &CompletionLockedTargetV1) -> Result<String, CanwuError> {
    canwu_api::canonical_hash("canwu.production.completion-target-lock.v1", target)
}

fn remove_completion_locks(
    locks: &mut BTreeMap<String, (CompletionLockedTargetV1, CompletionCapacityGrantId)>,
    grant: &CompletionCapacityGrantV1,
) {
    locks.retain(|_, (_, grant_id)| grant_id != &grant.id);
}

fn grant_lineage_matches(
    previous: &CompletionCapacityGrantV1,
    next: &CompletionCapacityGrantV1,
) -> bool {
    previous.id == next.id
        && previous.acquisition == next.acquisition
        && previous.operation_key == next.operation_key
        && previous.owner_plugin == next.owner_plugin
        && previous.run_budget_revision == next.run_budget_revision
        && previous.target_versions == next.target_versions
        && previous.recipe_digest == next.recipe_digest
        && previous.reserved_units == next.reserved_units
        && previous.expires_after_boundary == next.expires_after_boundary
}

const fn grant_terminal(state: CompletionGrantStateV1) -> bool {
    matches!(
        state,
        CompletionGrantStateV1::Completed
            | CompletionGrantStateV1::Released
            | CompletionGrantStateV1::Rejected
            | CompletionGrantStateV1::Expired
    )
}

const fn acquisition_pending(state: CompletionLeaseAcquisitionStateV1) -> bool {
    matches!(
        state,
        CompletionLeaseAcquisitionStateV1::Requested
            | CompletionLeaseAcquisitionStateV1::PartiallyGranted
            | CompletionLeaseAcquisitionStateV1::FullyGranted
            | CompletionLeaseAcquisitionStateV1::Preparing
            | CompletionLeaseAcquisitionStateV1::PreparedAll
            | CompletionLeaseAcquisitionStateV1::Aborting
    )
}

const fn completion_next_action(state: CompletionLeaseAcquisitionStateV1) -> &'static str {
    match state {
        CompletionLeaseAcquisitionStateV1::Requested
        | CompletionLeaseAcquisitionStateV1::PartiallyGranted => "await_grants",
        CompletionLeaseAcquisitionStateV1::FullyGranted => "prepare",
        CompletionLeaseAcquisitionStateV1::Preparing => "await_prepare",
        CompletionLeaseAcquisitionStateV1::PreparedAll => "activate",
        CompletionLeaseAcquisitionStateV1::Activated => "consume_or_complete",
        CompletionLeaseAcquisitionStateV1::Aborting => "release",
        CompletionLeaseAcquisitionStateV1::Released
        | CompletionLeaseAcquisitionStateV1::Expired => "none",
    }
}

#[allow(clippy::needless_pass_by_value)]
fn resource_error(error: canwu_resource::ResourceError) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, error.to_string())
}
