use canwu_api::{
    DomainRecordRef, DomainRecordVersionRef, DomainRecordVersionSource, KnowledgeHolderRef,
    PersonId, SimTime,
};
use canwu_resource::{
    CompletionCapacityGrantId, CompletionCapacityPartitionV1, CompletionCapacityRecipeV1,
    CompletionLeaseAcquisitionId, CompletionLeaseBookV1, CompletionLockedTargetV1,
    CompletionPolicyClassV1, EligibilityEnvelopeV1, GrantCompletionCapacityV1,
    MIN_REACQUIRE_COOLDOWN_MINUTES, PLUGIN_NAME, PrepareCompletionCapacityV1,
    REQUEST_TOKEN_REFILL_INTERVAL_MINUTES, RequestCompletionLeaseV1, ResourceOperationKey,
    ResourceRevision, RunBudgetRevisionV1,
};
use std::collections::{BTreeMap, BTreeSet};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let holder = KnowledgeHolderRef::Person(PersonId::new(1));
    let budget = RunBudgetRevisionV1 {
        revision: ResourceRevision::INITIAL,
        total_completion_units: 1_000_000,
        shared_pending_slots: 8,
        partitions: vec![CompletionCapacityPartitionV1 {
            authority: holder.clone(),
            operation_namespace: "example.production".to_owned(),
            guaranteed_units: 500_000,
            reserved_pending_slots: 4,
            maximum_burst_units: 100_000,
            request_token_capacity: 4,
            request_token_refill_minutes: REQUEST_TOKEN_REFILL_INTERVAL_MINUTES,
            reacquire_cooldown_minutes: MIN_REACQUIRE_COOLDOWN_MINUTES,
            root_acquisition_cap_per_sim_time: 4,
            guaranteed_max_wait_boundaries: 4,
        }],
        semantic_digest: String::new(),
    }
    .seal()?;

    let mut book = CompletionLeaseBookV1::default();
    let acquisition = book.request_acquisition(
        &budget,
        RequestCompletionLeaseV1 {
            id: CompletionLeaseAcquisitionId::new("example:lease:production")?,
            operation_key: ResourceOperationKey::new("example:operation:production")?,
            holder: holder.clone(),
            operation_namespace: "example.production".to_owned(),
            eligibility_time: SimTime::EPOCH,
            eligibility_envelope: EligibilityEnvelopeV1::new(
                Vec::new(),
                BTreeMap::new(),
                BTreeSet::new(),
                Vec::new(),
                Vec::new(),
            )?,
            recipe: CompletionCapacityRecipeV1 {
                receipts: 2,
                mutations: 4,
                reports_per_holder: 1,
                holders: 1,
                bytes: 4_096,
            },
            expected_participants: BTreeSet::from([PLUGIN_NAME.to_owned()]),
            policy_class: CompletionPolicyClassV1::Guaranteed,
        },
    )?;
    let grant = book.grant_capacity(
        &budget,
        GrantCompletionCapacityV1 {
            grant_id: CompletionCapacityGrantId::new("example:grant:resource")?,
            acquisition: acquisition.id.clone(),
            expected_acquisition_revision: acquisition.revision,
            owner_plugin: PLUGIN_NAME.to_owned(),
            target_versions: vec![CompletionLockedTargetV1::ExternalRecord {
                version: DomainRecordVersionRef {
                    record: DomainRecordRef::new(
                        "example.production",
                        "execution",
                        "production-run",
                    ),
                    version: 1,
                    established_by: DomainRecordVersionSource::InitialScenario,
                },
            }],
            current_boundary: 1,
        },
    )?;
    let prepared = book.prepare_capacity(PrepareCompletionCapacityV1 {
        acquisition: acquisition.id.clone(),
        expected_acquisition_revision: book.acquisitions[&acquisition.id].revision,
        grant: grant.id,
        expected_grant_revision: grant.revision,
        current_boundary: 2,
        eligibility_envelope_digest: book.acquisitions[&acquisition.id]
            .eligibility_envelope
            .digest
            .clone(),
    })?;
    let certificate = book.activate_single_owner(&acquisition.id, &prepared.id, 3)?;
    book.consume_grant(&certificate, &prepared.id)?;
    book.validate(&budget)?;

    let status = book.status_for(&holder, &acquisition.id)?;
    println!("lease state: {:?}", status.state);
    Ok(())
}
