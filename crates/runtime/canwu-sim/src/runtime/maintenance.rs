use super::{
    CanwuError, CauseRef, DomainRecordChange, DomainRecordMutation, DomainRecordRef, EntityRef,
    ErrorCode, IngressClass, IngressPayload, IngressReceipt, PersistentDomainRecordStore,
    PluginRegistry, SimTime, Simulation, StateVisibility, canonical_hash, canonical_text,
    is_canonical_hash, records, runtime_entity_exists,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::panic::{AssertUnwindSafe, catch_unwind};

pub const OWNER_AUTHORIZED_MAINTENANCE_FORMAT_VERSION: u32 = 1;
pub const MAX_OWNER_AUTHORIZED_PARTICIPANTS: usize = 32;
pub const MAX_OWNER_AUTHORIZED_MUTATIONS: usize = 256;
pub(super) const OWNER_AUTHORIZED_MAINTENANCE_SYSTEM: &str = "owner-authorized-maintenance";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerAuthorizedParticipantRole {
    TargetOwner,
    DependentOwner,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerAuthorizedRecordExpectation {
    pub record: DomainRecordRef,
    pub version: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerAuthorizedMutation {
    pub mutation: DomainRecordMutation,
    pub visibility: StateVisibility,
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerAuthorizedParticipantDraft {
    pub plugin: String,
    pub role: OwnerAuthorizedParticipantRole,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
    pub expected_records: Vec<OwnerAuthorizedRecordExpectation>,
    pub mutations: Vec<OwnerAuthorizedMutation>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerAuthorizedMaintenanceRequest {
    pub request_id: String,
    pub target: OwnerAuthorizedRecordExpectation,
    pub requested_at: SimTime,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerAuthorizedMaintenanceDraft {
    pub request_id: String,
    pub target: OwnerAuthorizedRecordExpectation,
    pub requested_at: SimTime,
    pub participants: Vec<OwnerAuthorizedParticipantDraft>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerAuthorizedParticipantProposal {
    pub plugin: String,
    pub semantic_hash: String,
    pub role: OwnerAuthorizedParticipantRole,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
    pub expected_records: Vec<OwnerAuthorizedRecordExpectation>,
    pub mutations: Vec<OwnerAuthorizedMutation>,
    pub proposal_commitment: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedOwnerAuthorizedMaintenanceCommit {
    format_version: u32,
    request_id: String,
    target: OwnerAuthorizedRecordExpectation,
    requested_at: SimTime,
    source_domain_root: String,
    participants: Vec<OwnerAuthorizedParticipantProposal>,
    token: String,
}

impl VerifiedOwnerAuthorizedMaintenanceCommit {
    #[must_use]
    pub(super) fn token(&self) -> &str {
        &self.token
    }

    pub(super) fn source_root(&self) -> &str {
        &self.source_domain_root
    }
}

impl Simulation {
    /// Asks every mandatory owner callback to author its own proposal and
    /// queues the resulting opaque commit as one canonical maintenance item.
    pub fn schedule_owner_authorized_maintenance(
        &mut self,
        due_at: SimTime,
        priority: i32,
        request: OwnerAuthorizedMaintenanceRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        let commit = self.prepare_owner_authorized_maintenance(request)?;
        self.enqueue_owner_authorized_maintenance(due_at, priority, commit)
    }

    /// Freezes and verifies a bounded set of owner proposals. The kernel fills
    /// semantic hashes and commitments from the active descriptor registry;
    /// callers cannot omit a registered dependent owner or mutate a foreign
    /// schema.
    fn prepare_owner_authorized_maintenance(
        &self,
        request: OwnerAuthorizedMaintenanceRequest,
    ) -> Result<VerifiedOwnerAuthorizedMaintenanceCommit, CanwuError> {
        self.ensure_runtime_ready()?;
        if !canonical_text(&request.request_id) || request.requested_at != self.time() {
            return Err(invalid_maintenance(
                "owner-authorized maintenance identity or request time is invalid",
            ));
        }
        let target_record = self
            .state
            .current
            .domain_records
            .get(&request.target.record)
            .ok_or_else(|| invalid_maintenance("maintenance target record is unavailable"))?;
        if target_record.version != request.target.version || !target_record.is_active() {
            return Err(invalid_maintenance(
                "maintenance target expectation is stale or not active",
            ));
        }
        let (target_owner, _) = self
            .plugins
            .record_schemas
            .get(&request.target.record.kind)
            .ok_or_else(|| invalid_maintenance("maintenance target schema has no owner"))?;
        let mut required = self
            .plugins
            .maintenance_dependency_resolvers
            .get(&request.target.record.kind.namespace)
            .cloned()
            .unwrap_or_default();
        required.insert(target_owner.clone());
        if required.is_empty() || required.len() > MAX_OWNER_AUTHORIZED_PARTICIPANTS {
            return Err(invalid_maintenance(
                "owner-authorized maintenance participant budget is invalid",
            ));
        }

        let mut participant_drafts = Vec::with_capacity(required.len());
        for plugin in &required {
            let descriptor = self.plugins.descriptors.get(plugin).ok_or_else(|| {
                invalid_maintenance("maintenance participant has no plugin descriptor")
            })?;
            if !descriptor.owner_authorized_maintenance_participant {
                return Err(invalid_maintenance(
                    "maintenance participant descriptor lacks an owner callback",
                ));
            }
            let handler = self
                .plugins
                .maintenance_participants
                .get(plugin)
                .copied()
                .ok_or_else(|| {
                    invalid_maintenance("maintenance participant callback is unavailable")
                })?;
            let role = if plugin == target_owner {
                OwnerAuthorizedParticipantRole::TargetOwner
            } else {
                OwnerAuthorizedParticipantRole::DependentOwner
            };
            let reads = descriptor
                .record_schemas
                .iter()
                .map(records::DomainRecordSchema::state_key)
                .collect::<Vec<_>>();
            let proposal = catch_unwind(AssertUnwindSafe(|| {
                handler(&self.plugin_view(plugin, &reads), &request, role)
            }))
            .map_err(|_| {
                CanwuError::new(
                    ErrorCode::PluginPanicked,
                    format!("maintenance participant {plugin} panicked"),
                )
            })??;
            if proposal.plugin != *plugin || proposal.role != role {
                return Err(invalid_maintenance(
                    "maintenance callback returned the wrong participant identity or role",
                ));
            }
            participant_drafts.push(proposal);
        }
        if participant_drafts
            .iter()
            .map(|proposal| proposal.mutations.len())
            .sum::<usize>()
            > MAX_OWNER_AUTHORIZED_MUTATIONS
        {
            return Err(invalid_maintenance(
                "owner-authorized maintenance mutation budget is invalid",
            ));
        }
        let mut draft = OwnerAuthorizedMaintenanceDraft {
            request_id: request.request_id,
            target: request.target,
            requested_at: request.requested_at,
            participants: participant_drafts,
        };
        draft
            .participants
            .sort_by(|left, right| left.plugin.cmp(&right.plugin));

        let mut participants = Vec::with_capacity(draft.participants.len());
        for mut proposal in draft.participants {
            let descriptor = self
                .plugins
                .descriptors
                .get(&proposal.plugin)
                .ok_or_else(|| {
                    invalid_maintenance("maintenance participant has no plugin descriptor")
                })?;
            let expected_role = if proposal.plugin == *target_owner {
                OwnerAuthorizedParticipantRole::TargetOwner
            } else {
                OwnerAuthorizedParticipantRole::DependentOwner
            };
            if proposal.role != expected_role
                || !proposal.accepted
                || proposal.rejection_reason.is_some()
            {
                return Err(invalid_maintenance(
                    "maintenance participant rejected or claimed the wrong owner role",
                ));
            }
            proposal.expected_records.sort();
            proposal.expected_records.dedup();
            if proposal.expected_records.is_empty()
                || proposal
                    .expected_records
                    .iter()
                    .any(|expected| expected.version == 0)
            {
                return Err(invalid_maintenance(
                    "maintenance participant requires exact nonzero record expectations",
                ));
            }
            for expected in &proposal.expected_records {
                let record = self
                    .state
                    .current
                    .domain_records
                    .get(&expected.record)
                    .ok_or_else(|| {
                        invalid_maintenance("maintenance expected record is unavailable")
                    })?;
                if record.version != expected.version {
                    return Err(invalid_maintenance(
                        "maintenance participant record expectation is stale",
                    ));
                }
            }
            for change in &proposal.mutations {
                if !canonical_text(&change.summary) {
                    return Err(invalid_maintenance(
                        "maintenance mutation summary is not canonical",
                    ));
                }
                let owner = self
                    .plugins
                    .record_schemas
                    .get(&change.mutation.target().kind)
                    .map(|(owner, _)| owner)
                    .ok_or_else(|| {
                        invalid_maintenance("maintenance mutation target has no schema owner")
                    })?;
                if owner != &proposal.plugin {
                    return Err(CanwuError::new(
                        ErrorCode::UndeclaredStateWrite,
                        "owner-authorized proposal targets another plugin's schema",
                    ));
                }
            }
            if proposal.role == OwnerAuthorizedParticipantRole::TargetOwner
                && !proposal.mutations.iter().any(|change| {
                    matches!(
                        &change.mutation,
                        DomainRecordMutation::Update {
                            record,
                            expected_version,
                        } if record.reference == draft.target.record
                            && *expected_version == draft.target.version
                    ) || matches!(
                        &change.mutation,
                        DomainRecordMutation::Retire { record, expected_version, .. }
                            if record == &draft.target.record
                                && *expected_version == draft.target.version
                    ) || matches!(
                        &change.mutation,
                        DomainRecordMutation::Delete { record, expected_version }
                            if record == &draft.target.record
                                && *expected_version == draft.target.version
                    )
                })
            {
                return Err(invalid_maintenance(
                    "target owner did not supply an exact owner-defined target mutation",
                ));
            }
            let proposal_commitment =
                participant_commitment(&proposal, &descriptor.semantic_hash, &draft.target)?;
            participants.push(OwnerAuthorizedParticipantProposal {
                plugin: proposal.plugin,
                semantic_hash: descriptor.semantic_hash.clone(),
                role: proposal.role,
                accepted: proposal.accepted,
                rejection_reason: proposal.rejection_reason,
                expected_records: proposal.expected_records,
                mutations: proposal.mutations,
                proposal_commitment,
            });
        }
        let source_domain_root = canonical_hash(
            "canwu.owner-authorized.source-domain-root.v1",
            self.state.current.domain_records.roots(),
        )?;
        let token = canonical_hash(
            "canwu.owner-authorized.maintenance-token.v1",
            &(
                OWNER_AUTHORIZED_MAINTENANCE_FORMAT_VERSION,
                &draft.request_id,
                &draft.target,
                draft.requested_at,
                &source_domain_root,
                &participants,
            ),
        )?;
        let commit = VerifiedOwnerAuthorizedMaintenanceCommit {
            format_version: OWNER_AUTHORIZED_MAINTENANCE_FORMAT_VERSION,
            request_id: draft.request_id,
            target: draft.target,
            requested_at: draft.requested_at,
            source_domain_root,
            participants,
            token,
        };
        let _ = self.apply_owner_authorized_commit_to_root(&commit)?;
        Ok(commit)
    }

    pub(super) fn enqueue_owner_authorized_maintenance(
        &mut self,
        due_at: SimTime,
        priority: i32,
        commit: VerifiedOwnerAuthorizedMaintenanceCommit,
    ) -> Result<IngressReceipt, CanwuError> {
        self.ensure_runtime_ready()?;
        self.ensure_canonical_ingress_can_start()?;
        let _ = self.apply_owner_authorized_commit_to_root(&commit)?;
        for record in &self.state.evidence.ingress {
            let IngressPayload::Maintenance { request } = &record.payload else {
                continue;
            };
            if let super::MaintenanceIngressRequest::OwnerAuthorized { commit: existing } =
                request.as_ref()
                && existing.token() == commit.token()
            {
                if existing == &commit && record.due_at == due_at && record.priority == priority {
                    return Ok(IngressReceipt {
                        ingress_id: record.id,
                        issued_at: record.issued_at,
                        due_at: record.due_at,
                    });
                }
                return Err(CanwuError::new(
                    ErrorCode::IdempotencyConflict,
                    "owner-authorized maintenance token is already queued differently",
                ));
            }
        }
        self.append_ingress(
            due_at,
            IngressClass::ScheduledSystem,
            priority,
            IngressPayload::Maintenance {
                request: Box::new(super::MaintenanceIngressRequest::OwnerAuthorized { commit }),
            },
            Some(CauseRef::System(
                "canwu.core.owner-authorized-maintenance".to_owned(),
            )),
            false,
        )
    }

    pub(super) fn apply_owner_authorized_maintenance(
        &mut self,
        commit: &VerifiedOwnerAuthorizedMaintenanceCommit,
    ) -> Result<Vec<super::DomainRecordChange>, CanwuError> {
        let (next, changes) = self.apply_owner_authorized_commit_to_root(commit)?;
        self.state.current.domain_records = next;
        self.invalidate_commitments(super::CommitmentDomains::DOMAIN_RECORDS);
        Ok(changes)
    }

    fn apply_owner_authorized_commit_to_root(
        &self,
        commit: &VerifiedOwnerAuthorizedMaintenanceCommit,
    ) -> Result<(PersistentDomainRecordStore, Vec<DomainRecordChange>), CanwuError> {
        apply_verified_owner_authorized_commit(
            commit,
            &self.state.current.domain_records,
            &self.plugins,
            self.state.scheduler.now,
            &|entity| runtime_entity_exists(&self.state, entity),
        )
    }
}

pub(super) fn apply_verified_owner_authorized_commit(
    commit: &VerifiedOwnerAuthorizedMaintenanceCommit,
    current: &PersistentDomainRecordStore,
    plugins: &PluginRegistry,
    now: SimTime,
    core_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(PersistentDomainRecordStore, Vec<DomainRecordChange>), CanwuError> {
    validate_verified_commit_authorization(commit, current, plugins)?;
    let source_domain_root = canonical_hash(
        "canwu.owner-authorized.source-domain-root.v1",
        current.roots(),
    )?;
    if source_domain_root != commit.source_domain_root {
        return Err(invalid_maintenance(
            "owner-authorized maintenance source root is stale",
        ));
    }
    let mut mutations = Vec::new();
    for proposal in &commit.participants {
        mutations.extend(
            proposal
                .mutations
                .iter()
                .map(|change| records::DomainMutationRequest {
                    plugin: proposal.plugin.as_str(),
                    system: OWNER_AUTHORIZED_MAINTENANCE_SYSTEM,
                    visibility: change.visibility,
                    mutation: &change.mutation,
                    summary: &change.summary,
                }),
        );
    }
    records::apply_mutation_bundle_cow(
        current,
        &plugins.record_schemas,
        now,
        core_exists,
        mutations,
    )
}

fn validate_verified_commit_authorization(
    commit: &VerifiedOwnerAuthorizedMaintenanceCommit,
    current: &PersistentDomainRecordStore,
    plugins: &PluginRegistry,
) -> Result<(), CanwuError> {
    validate_verified_commit_authorization_structure(commit, plugins)?;
    validate_verified_commit_freshness(commit, current)
}

pub(super) fn validate_verified_commit_authorization_structure(
    commit: &VerifiedOwnerAuthorizedMaintenanceCommit,
    plugins: &PluginRegistry,
) -> Result<(), CanwuError> {
    validate_verified_commit_shape(commit)?;
    if commit.target.version == 0 {
        return Err(invalid_maintenance(
            "maintenance target expectation must use a nonzero version",
        ));
    }
    let (target_owner, _) = plugins
        .record_schemas
        .get(&commit.target.record.kind)
        .ok_or_else(|| invalid_maintenance("maintenance target schema has no owner"))?;
    let mut required = plugins
        .maintenance_dependency_resolvers
        .get(&commit.target.record.kind.namespace)
        .cloned()
        .unwrap_or_default();
    required.insert(target_owner.clone());
    if required.is_empty()
        || required.len() > MAX_OWNER_AUTHORIZED_PARTICIPANTS
        || commit.participants.len() != required.len()
        || !commit
            .participants
            .iter()
            .map(|proposal| &proposal.plugin)
            .eq(required.iter())
    {
        return Err(invalid_maintenance(
            "owner-authorized maintenance participant set is incomplete or noncanonical",
        ));
    }
    let mutation_count = commit
        .participants
        .iter()
        .try_fold(0_usize, |total, proposal| {
            total.checked_add(proposal.mutations.len()).ok_or_else(|| {
                invalid_maintenance("owner-authorized maintenance mutation budget is invalid")
            })
        })?;
    if mutation_count > MAX_OWNER_AUTHORIZED_MUTATIONS {
        return Err(invalid_maintenance(
            "owner-authorized maintenance mutation budget is invalid",
        ));
    }
    for proposal in &commit.participants {
        let descriptor = plugins.descriptors.get(&proposal.plugin).ok_or_else(|| {
            invalid_maintenance("maintenance participant descriptor is unavailable")
        })?;
        let expected_role = if proposal.plugin == *target_owner {
            OwnerAuthorizedParticipantRole::TargetOwner
        } else {
            OwnerAuthorizedParticipantRole::DependentOwner
        };
        if !descriptor.owner_authorized_maintenance_participant
            || descriptor.semantic_hash != proposal.semantic_hash
            || proposal.role != expected_role
            || !proposal.accepted
            || proposal.rejection_reason.is_some()
            || participant_commitment_from_verified(proposal, &commit.target)?
                != proposal.proposal_commitment
        {
            return Err(invalid_maintenance(
                "maintenance participant identity, role, or commitment changed",
            ));
        }
        if proposal.expected_records.is_empty()
            || proposal
                .expected_records
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || proposal
                .expected_records
                .iter()
                .any(|expected| expected.version == 0)
        {
            return Err(invalid_maintenance(
                "maintenance participant expectations are empty or noncanonical",
            ));
        }
        for change in &proposal.mutations {
            if !canonical_text(&change.summary) {
                return Err(invalid_maintenance(
                    "maintenance mutation summary is not canonical",
                ));
            }
            let owner = plugins
                .record_schemas
                .get(&change.mutation.target().kind)
                .map(|(owner, _)| owner)
                .ok_or_else(|| {
                    invalid_maintenance("maintenance mutation target has no schema owner")
                })?;
            if owner != &proposal.plugin {
                return Err(CanwuError::new(
                    ErrorCode::UndeclaredStateWrite,
                    "owner-authorized proposal targets another plugin's schema",
                ));
            }
        }
        if proposal.role == OwnerAuthorizedParticipantRole::TargetOwner
            && !proposal.mutations.iter().any(|change| {
                matches!(
                    &change.mutation,
                    DomainRecordMutation::Update {
                        record,
                        expected_version,
                    } if record.reference == commit.target.record
                        && *expected_version == commit.target.version
                ) || matches!(
                    &change.mutation,
                    DomainRecordMutation::Retire { record, expected_version, .. }
                        if record == &commit.target.record
                            && *expected_version == commit.target.version
                ) || matches!(
                    &change.mutation,
                    DomainRecordMutation::Delete { record, expected_version }
                        if record == &commit.target.record
                            && *expected_version == commit.target.version
                )
            })
        {
            return Err(invalid_maintenance(
                "target owner did not supply an exact owner-defined target mutation",
            ));
        }
    }
    Ok(())
}

fn validate_verified_commit_freshness(
    commit: &VerifiedOwnerAuthorizedMaintenanceCommit,
    current: &PersistentDomainRecordStore,
) -> Result<(), CanwuError> {
    let target_record = current
        .get(&commit.target.record)
        .ok_or_else(|| invalid_maintenance("maintenance target record is unavailable"))?;
    if target_record.version != commit.target.version || !target_record.is_active() {
        return Err(invalid_maintenance(
            "maintenance target expectation is stale or not active",
        ));
    }
    for proposal in &commit.participants {
        for expected in &proposal.expected_records {
            if current
                .get(&expected.record)
                .is_none_or(|record| record.version != expected.version)
            {
                return Err(invalid_maintenance(
                    "owner-authorized maintenance expectation is stale",
                ));
            }
        }
    }
    Ok(())
}

fn participant_commitment(
    proposal: &OwnerAuthorizedParticipantDraft,
    semantic_hash: &str,
    target: &OwnerAuthorizedRecordExpectation,
) -> Result<String, CanwuError> {
    canonical_hash(
        "canwu.owner-authorized.participant.v1",
        &(
            &proposal.plugin,
            semantic_hash,
            proposal.role,
            proposal.accepted,
            &proposal.rejection_reason,
            &proposal.expected_records,
            &proposal.mutations,
            target,
        ),
    )
}

fn participant_commitment_from_verified(
    proposal: &OwnerAuthorizedParticipantProposal,
    target: &OwnerAuthorizedRecordExpectation,
) -> Result<String, CanwuError> {
    canonical_hash(
        "canwu.owner-authorized.participant.v1",
        &(
            &proposal.plugin,
            &proposal.semantic_hash,
            proposal.role,
            proposal.accepted,
            &proposal.rejection_reason,
            &proposal.expected_records,
            &proposal.mutations,
            target,
        ),
    )
}

pub(super) fn validate_verified_commit_shape(
    commit: &VerifiedOwnerAuthorizedMaintenanceCommit,
) -> Result<(), CanwuError> {
    if commit.format_version != OWNER_AUTHORIZED_MAINTENANCE_FORMAT_VERSION
        || !canonical_text(&commit.request_id)
        || !is_canonical_hash(&commit.source_domain_root)
        || !is_canonical_hash(&commit.token)
        || commit.participants.is_empty()
        || commit.participants.len() > MAX_OWNER_AUTHORIZED_PARTICIPANTS
        || commit
            .participants
            .windows(2)
            .any(|pair| pair[0].plugin >= pair[1].plugin)
        || commit.participants.iter().any(|proposal| {
            !proposal.accepted
                || proposal.rejection_reason.is_some()
                || !is_canonical_hash(&proposal.semantic_hash)
                || !is_canonical_hash(&proposal.proposal_commitment)
        })
    {
        return Err(invalid_maintenance(
            "owner-authorized maintenance commit is malformed or non-canonical",
        ));
    }
    let expected = canonical_hash(
        "canwu.owner-authorized.maintenance-token.v1",
        &(
            commit.format_version,
            &commit.request_id,
            &commit.target,
            commit.requested_at,
            &commit.source_domain_root,
            &commit.participants,
        ),
    )?;
    if expected != commit.token {
        return Err(invalid_maintenance(
            "owner-authorized maintenance token is inconsistent",
        ));
    }
    Ok(())
}

fn invalid_maintenance(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}
