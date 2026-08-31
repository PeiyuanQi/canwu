use canwu_api::{KnowledgeHolderRef, PersonId, SimDuration, SimTime};
use canwu_resource::{
    DemandStatus, PartialFulfillmentPolicy, ResourceAccount, ResourceAccountId,
    ResourceAllocationRequestV1, ResourceDefinitionId, ResourceDefinitionRevision,
    ResourceDefinitionRevisionId, ResourceDemand, ResourceDemandId, ResourceLimitsV1,
    ResourceOperationKey, ResourceOperationRequestV1, ResourceQualityId, ResourceRevision,
    ResourceScopeId, ResourceState, ResourceTieBreakKey, ResourceUnitRevision,
    ResourceUnitRevisionId,
};
use std::collections::BTreeSet;

fn digest_placeholder() -> String {
    // Authored catalogs normally provide the real canonical content digest.
    "0".repeat(64)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let custodian = KnowledgeHolderRef::Person(PersonId::new(1));
    let consumer = KnowledgeHolderRef::Person(PersonId::new(2));
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical())?;

    let unit = ResourceUnitRevisionId::new("example:unit:kg:v1")?;
    state.install_unit(ResourceUnitRevision {
        id: unit.clone(),
        revision: ResourceRevision::INITIAL,
        symbol: "kg".to_owned(),
        scale_numerator: 1,
        scale_denominator: 1,
        semantic_digest: digest_placeholder(),
    })?;

    let resource = ResourceDefinitionRevisionId::new("example:resource:grain:v1")?;
    state.install_definition(ResourceDefinitionRevision {
        id: resource.clone(),
        resource: ResourceDefinitionId::new("example:resource:grain")?,
        revision: ResourceRevision::INITIAL,
        canonical_unit: unit.clone(),
        quality: ResourceQualityId::new("example:quality:merchantable")?,
        scope: ResourceScopeId::new("example:scope:granary")?,
        effective_from: SimTime::EPOCH,
        effective_until: None,
        process_suitability: BTreeSet::new(),
        semantic_digest: digest_placeholder(),
    })?;

    let account = ResourceAccountId::new("example:account:granary")?;
    state.install_opening_account(ResourceAccount {
        id: account.clone(),
        revision: ResourceRevision::INITIAL,
        custodian,
        resource_revision: resource.clone(),
        unit_revision: unit.clone(),
        balance: 100,
        capacity: Some(500),
        protected_floor_policy: None,
        closed: false,
    })?;

    state.install_demand(ResourceDemand {
        id: ResourceDemandId::new("example:demand:workshop")?,
        revision: ResourceRevision::INITIAL,
        requester: consumer,
        resource_revision: resource,
        unit_revision: unit,
        requested: 40,
        fulfilled: 0,
        minimum_useful: 20,
        partial_fulfillment: PartialFulfillmentPolicy::AcceptPartial,
        alternative_group: None,
        due_at: SimTime::EPOCH,
        expires_at: SimTime::EPOCH + SimDuration::days(7),
        priority: 10,
        tie_break: ResourceTieBreakKey::new("example:tie:workshop")?,
        admitted_sequence: 0,
        protected_floor_policy: None,
        protection_override_class: None,
        status: DemandStatus::Open,
        rejection_reason: None,
    })?;

    let allocation = state.apply_operation(&ResourceOperationRequestV1::Allocate(
        ResourceAllocationRequestV1 {
            operation_key: ResourceOperationKey::new("example:operation:allocate")?,
            expected_state_revision: state.state_revision,
            at: SimTime::EPOCH,
            candidate_limit: 16,
        },
    ))?;
    state.validate()?;
    let quantities = state.account_quantities(&account)?;
    println!(
        "allocated={}, reserved={}, balance={}, open_remainder={}",
        allocation.quantity,
        quantities.reserved,
        quantities.authoritative_balance,
        allocation.remainder
    );

    // The plugin scenario root is the record returned here.
    let _authoritative_record = state.into_record()?;
    Ok(())
}
