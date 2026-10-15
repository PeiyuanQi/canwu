//! Downstream gap G-53: one operation takes a completion lease whose only
//! participant is the resource plugin from nothing to active.
//!
//! The first fixture runs the single step beside the four steps on equal
//! roots. The activation certificate is the same, and with the four steps at
//! one boundary so is the whole lease. The single step leaves one outcome and
//! one receipt where the four leave four of each, a resend is not a second
//! lease, and what the four steps refuse the single step refuses as one
//! recorded rejection that leaves no lease behind. A lease that names another
//! participant is refused and keeps the four-step path, and the four-step
//! path never resumes a lease the single step created. Identities too long to
//! form an operation key, of the step or of the release that may have to give
//! the lease up, are a recorded rejection, and no request other than a
//! completion operation settles under a completion operation's key.
//!
//! The second fixture counts, in a plugin-owned run with an archive, what a
//! same-place transfer leaves and reads with each path: 27 terminal records
//! and 15 archive lookups with four-step leases, 15 and 9 with single steps.
//! A refusal the simulation view finds settles as a recorded rejection without
//! failing its boundary, a tracked command cannot carry the single step, an
//! archive commit and a single step settle in one boundary, a resend changes
//! nothing before or after its outcome is archived, and a restored run and an
//! exact replay agree. A completion packet whose key a different request
//! holds settles nothing and its boundary commits: a second single step with
//! the same acquisition id, and the host application's abort of a lease whose
//! holder took the abort key first.

#![allow(
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::too_many_lines
)]

use canwu_api::{
    BoundaryRequest, Canwu, CanwuError, CommandAttemptOutcome, CommandEnvelope, CommandRequest,
    CommandRequestId, DomainRecord, DomainRecordClass, DomainRecordKind, DomainRecordLifecycle,
    DomainRecordRef, DomainRecordSchema, DomainRecordType, DomainRecordVersionRef,
    DomainRecordVersionSource, DomainValueKindClass, EntityRef, ErrorCode, Issuer,
    KnowledgeHolderRef, PayloadSchema, PersonId, PluginArchiveObjectProvider, PluginRegistrar,
    Scenario, SimDuration, SimTime, SimulationPlugin,
};
use canwu_resource::*;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

const NAMESPACE: &str = "fixture.resource";
const EVIDENCE_NAMESPACE: &str = "fixture.provider";
const SOURCE_CUSTODIAN: u64 = 1;
const DESTINATION_CUSTODIAN: u64 = 2;
/// A holder with no completion-capacity partition.
const OUTSIDER: u64 = 3;

fn holder(value: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(value))
}

fn minute(value: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::minutes(value)
}

fn key(value: &str) -> ResourceOperationKey {
    ResourceOperationKey::new(value).expect("operation key")
}

fn account(name: &str) -> ResourceAccountId {
    ResourceAccountId::new(format!("fixture:account:{name}")).expect("account")
}

fn acquisition(suffix: &str) -> CompletionLeaseAcquisitionId {
    CompletionLeaseAcquisitionId::new(format!("fixture:lease:{suffix}")).expect("acquisition")
}

fn grant(suffix: &str) -> CompletionCapacityGrantId {
    CompletionCapacityGrantId::new(format!("fixture:grant:{suffix}")).expect("grant")
}

fn evidence(id: &str, version: u64) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new(EVIDENCE_NAMESPACE, "evidence", id),
        version,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

fn evidence_record(version: &DomainRecordVersionRef) -> DomainRecord {
    DomainRecord {
        reference: version.record.clone(),
        owner: "fixture-provider".to_owned(),
        class: DomainRecordClass::Record,
        version: version.version,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::json!({ "evidence": version.record.id }),
        references: Vec::new(),
    }
}

/// Owner of the handover records the transfers cite.
struct EvidenceRecord;

impl DomainRecordType for EvidenceRecord {
    type Payload = serde_json::Value;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = EVIDENCE_NAMESPACE;
    const NAME: &'static str = "evidence";
}

struct EvidencePlugin;

impl SimulationPlugin for EvidencePlugin {
    fn name(&self) -> &'static str {
        "fixture-provider"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000f5"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_record::<EvidenceRecord>();
        schema.payload_schema = PayloadSchema::Any;
        registrar.register_record_schema(schema)
    }
}

fn partition(authority: KnowledgeHolderRef) -> CompletionCapacityPartitionV1 {
    CompletionCapacityPartitionV1 {
        authority,
        operation_namespace: NAMESPACE.to_owned(),
        guaranteed_units: 400_000,
        reserved_pending_slots: 8,
        maximum_burst_units: 100_000,
        request_token_capacity: 16,
        request_token_refill_minutes: 1,
        reacquire_cooldown_minutes: 1,
        root_acquisition_cap_per_sim_time: 4,
        guaranteed_max_wait_boundaries: 4,
    }
}

/// A root with a source and a destination account in one place and room for
/// the leases of both custodians.
fn world() -> ResourceState {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("state");
    state
        .install_run_budget(
            RunBudgetRevisionV1 {
                revision: ResourceRevision::INITIAL,
                total_completion_units: 2_000_000,
                shared_pending_slots: 4,
                partitions: vec![
                    partition(holder(SOURCE_CUSTODIAN)),
                    partition(holder(DESTINATION_CUSTODIAN)),
                ],
                semantic_digest: String::new(),
            }
            .seal()
            .expect("budget"),
        )
        .expect("install budget");
    let unit = ResourceUnitRevisionId::new("fixture:unit:measure:v1").expect("unit");
    state
        .install_unit(ResourceUnitRevision {
            id: unit.clone(),
            revision: ResourceRevision::INITIAL,
            symbol: "u".to_owned(),
            scale_numerator: 1,
            scale_denominator: 1,
            semantic_digest: "0".repeat(64),
        })
        .expect("unit");
    let resource = ResourceDefinitionRevisionId::new("fixture:resource:stock:v1").expect("id");
    state
        .install_definition(ResourceDefinitionRevision {
            id: resource.clone(),
            resource: ResourceDefinitionId::new("fixture:resource:stock").expect("definition"),
            revision: ResourceRevision::INITIAL,
            canonical_unit: unit.clone(),
            quality: ResourceQualityId::new("fixture:quality:standard").expect("quality"),
            scope: ResourceScopeId::new("fixture:scope:market").expect("scope"),
            effective_from: SimTime::EPOCH,
            effective_until: None,
            process_suitability: BTreeSet::new(),
            semantic_digest: "1".repeat(64),
        })
        .expect("definition");
    for (name, custodian, balance) in [
        ("source", SOURCE_CUSTODIAN, 100),
        ("destination", DESTINATION_CUSTODIAN, 0),
    ] {
        state
            .install_opening_account(ResourceAccount {
                id: account(name),
                revision: ResourceRevision::INITIAL,
                custodian: holder(custodian),
                resource_revision: resource.clone(),
                unit_revision: unit.clone(),
                balance,
                capacity: None,
                protected_floor_policy: None,
                closed: false,
                place_scope: Some(ResourceScopeId::new("fixture:place:depot").expect("place")),
            })
            .expect("account");
    }
    state
}

/// A 10-unit demand of the source custodian on its own account.
fn demand(state: &ResourceState, name: &str, at: SimTime) -> ResourceDemand {
    let source = &state.accounts[&account("source")];
    ResourceDemand {
        id: ResourceDemandId::new(format!("fixture:demand:{name}")).expect("demand"),
        revision: ResourceRevision::INITIAL,
        requester: holder(SOURCE_CUSTODIAN),
        source_policy: ResourceDemandSourcePolicyV1::ExactAccounts(vec![account("source")]),
        resource_revision: source.resource_revision.clone(),
        unit_revision: source.unit_revision.clone(),
        requested: 10,
        fulfilled: 0,
        minimum_useful: 10,
        partial_fulfillment: PartialFulfillmentPolicy::RejectPartial,
        alternative_group: None,
        due_at: at,
        expires_at: at + SimDuration::days(10),
        priority: 1,
        tie_break: ResourceTieBreakKey::new(format!("fixture:tie:{name}")).expect("tie"),
        admitted_sequence: 0,
        protected_floor_policy: None,
        protection_override_class: None,
        status: DemandStatus::Open,
        rejection_reason: None,
    }
}

fn leg_of(state: &ResourceState, name: &str) -> ResourceAllocationLeg {
    let demand = ResourceDemandId::new(format!("fixture:demand:{name}")).expect("demand");
    state
        .allocation_legs
        .values()
        .find(|leg| leg.demand == demand)
        .cloned()
        .expect("allocation leg")
}

/// The exact targets a transfer start locks.
fn start_targets(
    state: &ResourceState,
    leg: &ResourceAllocationLeg,
) -> Vec<CompletionLockedTargetV1> {
    vec![
        CompletionLockedTargetV1::Account {
            id: leg.account.clone(),
            revision: state.accounts[&leg.account].revision,
        },
        CompletionLockedTargetV1::AllocationLeg {
            id: leg.id.clone(),
            revision: leg.revision,
        },
        CompletionLockedTargetV1::Demand {
            id: leg.demand.clone(),
            revision: leg.demand_revision,
        },
    ]
}

/// The acquisition both paths start from.
fn lease_request(
    lease_holder: u64,
    suffix: &str,
    operation_key: &ResourceOperationKey,
    targets: &[CompletionLockedTargetV1],
    at: SimTime,
) -> RequestCompletionLeaseV1 {
    RequestCompletionLeaseV1 {
        id: acquisition(suffix),
        operation_key: operation_key.clone(),
        holder: holder(lease_holder),
        operation_namespace: NAMESPACE.to_owned(),
        eligibility_time: at,
        eligibility_envelope: EligibilityEnvelopeV1::new(
            targets
                .iter()
                .filter_map(|target| match target {
                    CompletionLockedTargetV1::ExternalRecord { version } => Some(version.clone()),
                    _ => None,
                })
                .collect(),
            BTreeMap::new(),
            BTreeSet::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("envelope"),
        recipe: CompletionCapacityRecipeV1 {
            receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
            mutations: 2,
            reports_per_holder: 0,
            holders: 0,
            bytes: 1_024,
        },
        expected_participants: BTreeSet::from([PLUGIN_NAME.to_owned()]),
        policy_class: CompletionPolicyClassV1::Guaranteed,
    }
}

fn single_step(
    lease_holder: u64,
    suffix: &str,
    operation_key: &ResourceOperationKey,
    targets: &[CompletionLockedTargetV1],
    at: SimTime,
    current_boundary: u64,
) -> SingleStepCompletionLeaseV1 {
    SingleStepCompletionLeaseV1 {
        lease: lease_request(lease_holder, suffix, operation_key, targets, at),
        grant_id: grant(suffix),
        target_versions: targets.to_vec(),
        current_boundary,
    }
}

/// A single step for an operation of its own, named after the lease.
fn spare_lease(
    lease_holder: u64,
    suffix: &str,
    targets: &[CompletionLockedTargetV1],
    at: SimTime,
    current_boundary: u64,
) -> SingleStepCompletionLeaseV1 {
    single_step(
        lease_holder,
        suffix,
        &key(&format!("fixture:operation:{suffix}")),
        targets,
        at,
        current_boundary,
    )
}

fn completion(operation: ResourceCompletionOperationV1) -> ResourceOperationRequestV1 {
    ResourceOperationRequestV1::Completion(operation)
}

fn single_step_request(lease: &SingleStepCompletionLeaseV1) -> ResourceOperationRequestV1 {
    completion(ResourceCompletionOperationV1::SingleStepLease(
        lease.clone(),
    ))
}

fn applied(
    state: &mut ResourceState,
    request: ResourceOperationRequestV1,
) -> ResourceOperationOutcome {
    let outcome = state.apply_operation(&request).expect("operation");
    assert_eq!(
        outcome.status,
        ResourceOperationStatus::Applied,
        "{:?}",
        outcome.rejection_reason
    );
    outcome
}

/// The grant, prepare, and activate steps of an acquisition already
/// requested, each at its own boundary.
fn finish_four_steps(
    state: &mut ResourceState,
    lease: &RequestCompletionLeaseV1,
    grant_id: &CompletionCapacityGrantId,
    targets: &[CompletionLockedTargetV1],
    boundaries: [u64; 3],
) {
    applied(
        state,
        completion(ResourceCompletionOperationV1::Grant(
            GrantCompletionCapacityV1 {
                grant_id: grant_id.clone(),
                acquisition: lease.id.clone(),
                expected_acquisition_revision: state.completion_leases.acquisitions[&lease.id]
                    .revision,
                owner_plugin: PLUGIN_NAME.to_owned(),
                target_versions: targets.to_vec(),
                current_boundary: boundaries[0],
            },
        )),
    );
    applied(
        state,
        completion(ResourceCompletionOperationV1::Prepare(
            PrepareCompletionCapacityV1 {
                acquisition: lease.id.clone(),
                expected_acquisition_revision: state.completion_leases.acquisitions[&lease.id]
                    .revision,
                grant: grant_id.clone(),
                expected_grant_revision: state.completion_leases.grants[grant_id].revision,
                current_boundary: boundaries[1],
                eligibility_envelope_digest: lease.eligibility_envelope.digest.clone(),
            },
        )),
    );
    applied(
        state,
        completion(ResourceCompletionOperationV1::Activate(
            ActivateCompletionLeaseV1 {
                acquisition: lease.id.clone(),
                expected_acquisition_revision: state.completion_leases.acquisitions[&lease.id]
                    .revision,
                grant: grant_id.clone(),
                expected_grant_revision: state.completion_leases.grants[grant_id].revision,
                at: lease.eligibility_time,
                current_boundary: boundaries[2],
                eligibility_envelope_digest: lease.eligibility_envelope.digest.clone(),
            },
        )),
    );
}

/// The four-step path for the same lease the single step names.
fn four_steps(
    state: &mut ResourceState,
    lease: &SingleStepCompletionLeaseV1,
    boundaries: [u64; 3],
) -> CompletionLeaseActivationCertificateV1 {
    applied(
        state,
        completion(ResourceCompletionOperationV1::Acquire(lease.lease.clone())),
    );
    finish_four_steps(
        state,
        &lease.lease,
        &lease.grant_id,
        &lease.target_versions,
        boundaries,
    );
    state.completion_leases.certificates[&lease.lease.id].clone()
}

/// Asserts one recorded rejection that left no trace in the lease book.
fn refused(state: &mut ResourceState, lease: &SingleStepCompletionLeaseV1, code: &str) {
    refused_request(state, &single_step_request(lease), code);
}

/// Asserts that one completion request is a recorded rejection that left no
/// trace in the lease book.
fn refused_request(state: &mut ResourceState, request: &ResourceOperationRequestV1, code: &str) {
    let before = state.clone();
    let outcome = state
        .apply_operation(request)
        .expect("a refused completion operation is a recorded rejection");
    assert_eq!(outcome.status, ResourceOperationStatus::Rejected, "{code}");
    assert_eq!(outcome.kind, ResourceOperationKind::CompletionLease);
    assert_eq!(
        outcome.rejection_code.as_deref(),
        Some(code),
        "{:?}",
        outcome.rejection_reason
    );
    assert_eq!(state.completion_leases, before.completion_leases, "{code}");
    assert_eq!(
        state.completion_report_reservations,
        before.completion_report_reservations
    );
    assert_eq!(
        state.terminal_archive_candidates.len(),
        before.terminal_archive_candidates.len() + 1,
        "a refusal leaves its outcome and nothing else"
    );
    state
        .validate()
        .expect("state after a refused completion operation");
}

#[test]
fn single_step_lease_equals_the_four_steps_and_refuses_what_they_refuse() {
    let at = minute(1);
    let mut root = world();
    let first = demand(&root, "first", SimTime::EPOCH);
    root.install_demand(first).expect("demand");
    let expected_state_revision = root.state_revision;
    applied(
        &mut root,
        ResourceOperationRequestV1::Allocate(ResourceAllocationRequestV1 {
            operation_key: key("fixture:allocate:first"),
            expected_state_revision,
            at: SimTime::EPOCH,
            candidate_limit: 8,
        }),
    );
    let leg = leg_of(&root, "first");
    let targets = start_targets(&root, &leg);
    let start_key = key("fixture:start:first");
    let lease = single_step(SOURCE_CUSTODIAN, "first", &start_key, &targets, at, 12);
    let hot_before = root.terminal_archive_candidates.len();

    // The four steps, at three boundaries and at one.
    let mut spread = root.clone();
    let spread_certificate = four_steps(&mut spread, &lease, [10, 11, 12]);
    let mut four = root.clone();
    let four_certificate = four_steps(&mut four, &lease, [12, 12, 12]);
    assert_eq!(four.terminal_archive_candidates.len() - hot_before, 8);
    assert_eq!(four.completion_leases.receipts.len(), 4);

    // The single step: one outcome, one receipt, the same certificate.
    let mut single = root.clone();
    let outcome = applied(&mut single, single_step_request(&lease));
    assert_eq!(outcome.kind, ResourceOperationKind::CompletionLease);
    assert_eq!(
        outcome.operation_key,
        key("resource:completion:lease:fixture:lease:first")
    );
    assert_eq!(
        outcome.result_ref,
        Some(ResourceRecordRefV1::Lease(lease.lease.id.clone()))
    );
    assert_eq!(single.terminal_archive_candidates.len() - hot_before, 2);
    let receipts: Vec<_> = single.completion_leases.receipts.values().collect();
    assert_eq!(receipts.len(), 1);
    assert_eq!(
        receipts[0].action,
        CompletionLeaseReceiptActionV1::Activated
    );
    assert_eq!(receipts[0].operation_key, outcome.operation_key);
    assert_eq!(receipts[0].grant, Some(lease.grant_id.clone()));
    let certificate = single.completion_leases.certificates[&lease.lease.id].clone();
    assert_eq!(certificate, four_certificate);
    assert_eq!(
        certificate, spread_certificate,
        "the certificate records only the activation boundary"
    );
    assert_eq!(certificate.operation_key, start_key);
    assert_eq!(certificate.activation_boundary, 12);
    // With the four steps at one boundary the whole lease is the same.
    assert_eq!(
        single.completion_leases.acquisitions,
        four.completion_leases.acquisitions
    );
    assert_eq!(
        single.completion_leases.grants,
        four.completion_leases.grants
    );
    assert_eq!(
        single.completion_leases.target_locks,
        four.completion_leases.target_locks
    );
    assert_eq!(
        single.completion_leases.epochs,
        four.completion_leases.epochs
    );
    assert_eq!(
        single.completion_leases.reserved_units,
        four.completion_leases.reserved_units
    );
    assert!(single.completion_leases.expiry_due.is_empty());
    single.validate().expect("state after the single step");

    // A resend is not a second lease.
    let before_resend = single.clone();
    let again = single
        .apply_operation(&single_step_request(&lease))
        .expect("resend");
    assert_eq!(again, outcome);
    assert_eq!(single, before_resend);

    // The four-step path never resumes a lease the single step created. Its
    // acquire has a key of its own, so nothing answers it as a resend: it is
    // a recorded rejection that leaves the active lease as it is.
    refused_request(
        &mut single,
        &completion(ResourceCompletionOperationV1::Acquire(lease.lease.clone())),
        "idempotency_conflict",
    );

    // The same holder is held to the same admission limits: its next
    // acquisition waits out the cooldown, and the refusal spends no token.
    let spare_targets = vec![CompletionLockedTargetV1::Account {
        id: account("destination"),
        revision: ResourceRevision::INITIAL,
    }];
    refused(
        &mut single,
        &spare_lease(SOURCE_CUSTODIAN, "cooldown", &spare_targets, at, 12),
        "limit_exceeded",
    );
    // A target another active lease locks is refused, and unlike a rejected
    // four-step prepare it leaves no aborting acquisition to clean up.
    refused(
        &mut single,
        &spare_lease(DESTINATION_CUSTODIAN, "locked", &targets[..1], at, 12),
        "version_conflict",
    );

    // Both certificates authorize the same debit.
    let transfer = ResourceTransferId::new("fixture:transfer:first").expect("transfer");
    for state in [&mut four, &mut single] {
        let expected_account_revision = state.accounts[&leg.account].revision;
        applied(
            state,
            ResourceOperationRequestV1::BeginTransfer(ResourceTransferStartRequestV1 {
                operation_key: start_key.clone(),
                transfer_id: transfer.clone(),
                allocation: (&leg).into(),
                expected_account_revision,
                destination: Some(account("destination")),
                at,
                completion_certificate: state.completion_leases.certificates[&lease.lease.id]
                    .clone(),
            }),
        );
    }
    assert_eq!(single.transfers, four.transfers);
    assert_eq!(single.accounts, four.accounts);

    // A lease that needs another participant is refused, and the four-step
    // path still admits it.
    let mut shared = spare_lease(SOURCE_CUSTODIAN, "shared", &spare_targets, at, 12);
    shared
        .lease
        .expected_participants
        .insert("fixture-provider".to_owned());
    let mut state = root.clone();
    refused(&mut state, &shared, "invalid_lifecycle");
    applied(
        &mut state,
        completion(ResourceCompletionOperationV1::Acquire(shared.lease.clone())),
    );
    assert_eq!(
        state.completion_leases.acquisitions[&shared.lease.id].state,
        CompletionLeaseAcquisitionStateV1::Requested
    );

    // That pending acquisition sorts first, so a single step may not pass it.
    refused(
        &mut state,
        &spare_lease(DESTINATION_CUSTODIAN, "queued", &spare_targets, at, 12),
        "invalid_lifecycle",
    );

    // A single step never resumes an acquisition the four-step path began,
    // and that path then finishes it as before.
    let mut state = root.clone();
    let begun = spare_lease(DESTINATION_CUSTODIAN, "begun", &spare_targets, at, 12);
    applied(
        &mut state,
        completion(ResourceCompletionOperationV1::Acquire(begun.lease.clone())),
    );
    refused(&mut state, &begun, "idempotency_conflict");
    finish_four_steps(
        &mut state,
        &begun.lease,
        &begun.grant_id,
        &begun.target_versions,
        [12, 12, 12],
    );
    assert!(
        state
            .completion_leases
            .certificates
            .contains_key(&begun.lease.id)
    );

    // The checks of each step apply: authority, exact targets, the recipe.
    let mut state = root.clone();
    refused(
        &mut state,
        &spare_lease(OUTSIDER, "outsider", &spare_targets, at, 12),
        "invalid_authority",
    );
    let stale = vec![CompletionLockedTargetV1::Account {
        id: account("destination"),
        revision: ResourceRevision::new(2).expect("revision"),
    }];
    refused(
        &mut state,
        &spare_lease(SOURCE_CUSTODIAN, "stale", &stale, at, 12),
        "version_conflict",
    );
    let mut short = spare_lease(SOURCE_CUSTODIAN, "short", &spare_targets, at, 12);
    short.lease.recipe.receipts = 4;
    refused(&mut state, &short, "limit_exceeded");
    // Identities that are valid by themselves and too long for the key of
    // the step are a recorded rejection under a key of their own.
    let long = spare_lease(SOURCE_CUSTODIAN, &"x".repeat(140), &spare_targets, at, 12);
    let operation = ResourceCompletionOperationV1::SingleStepLease(long.clone());
    assert!(!operation.has_canonical_operation_key());
    assert!(
        operation
            .operation_key()
            .as_str()
            .starts_with("resource:completion:invalid:")
    );
    refused(&mut state, &long, "invalid_identifier");
    refused_request(
        &mut state,
        &completion(ResourceCompletionOperationV1::Acquire(long.lease)),
        "invalid_identifier",
    );
    // A single step must also be releasable. Its own key names only the
    // acquisition, but the release that may have to give the lease up names
    // the grant too: identities that form no release key are refused, or the
    // lease could never be released.
    let unreleasable = spare_lease(SOURCE_CUSTODIAN, &"y".repeat(60), &spare_targets, at, 12);
    let operation = ResourceCompletionOperationV1::SingleStepLease(unreleasable.clone());
    assert!(
        operation
            .operation_key()
            .as_str()
            .starts_with("resource:completion:lease:")
    );
    assert!(!operation.has_canonical_operation_key());
    refused(&mut state, &unreleasable, "invalid_identifier");
    let mut spaced = spare_lease(SOURCE_CUSTODIAN, "spaced", &spare_targets, at, 12);
    spaced.grant_id = CompletionCapacityGrantId::new("fixture:grant:with a space")
        .expect("a grant identity is free text");
    refused(&mut state, &spaced, "invalid_identifier");
    // A key under the completion prefix belongs to the completion operation
    // that derives it. No other request settles under it, not even as a
    // rejection, which would itself take the key.
    let before = state.clone();
    let taken = ResourceOperationRequestV1::SubmitDemand(ResourceSubmitDemandRequestV1 {
        operation_key: key("resource:completion:lease:fixture:lease:after-refusals"),
        demand: demand(&state, "taker", SimTime::EPOCH),
    });
    assert!(taken.names_reserved_operation_key());
    assert!(matches!(
        state.apply_operation(&taken),
        Err(ResourceError::Authority(_))
    ));
    assert_eq!(state, before);
    // Nothing above spent the holder's admission: the lease is still granted.
    applied(
        &mut state,
        single_step_request(&spare_lease(
            SOURCE_CUSTODIAN,
            "after-refusals",
            &spare_targets,
            at,
            12,
        )),
    );
}

/// Stored archive objects by namespace and object id.
type ArchiveObjects = BTreeMap<(String, String), Vec<u8>>;

/// A content-addressed archive store that counts the archive directories the
/// runtime loads through its archive provider. Every archive lookup of an
/// operation key loads the head directory exactly once.
#[derive(Clone, Default)]
struct CountingArchive {
    objects: Rc<RefCell<ArchiveObjects>>,
    retention: Rc<RefCell<BTreeMap<String, ResourceArchiveRetentionHandleV1>>>,
    lookups: Rc<RefCell<usize>>,
}

impl CountingArchive {
    fn lookups(&self) -> usize {
        *self.lookups.borrow()
    }
}

impl ResourceArchiveStore for CountingArchive {
    fn store_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), ResourceError> {
        self.objects
            .borrow_mut()
            .insert((namespace.to_owned(), object_id.to_owned()), bytes.to_vec());
        Ok(())
    }

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, ResourceError> {
        Ok(self
            .objects
            .borrow()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }

    fn persist_resource_archive_retention(
        &self,
        handle: &ResourceArchiveRetentionHandleV1,
    ) -> Result<(), ResourceError> {
        self.retention
            .borrow_mut()
            .insert(handle.id.clone(), handle.clone());
        Ok(())
    }

    fn finalize_resource_archive_retention(
        &self,
        handle_id: &str,
        phase: ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError> {
        self.retention
            .borrow_mut()
            .get_mut(handle_id)
            .ok_or_else(|| ResourceError::NotFound("retention handle unavailable".to_owned()))?
            .phase = phase;
        Ok(())
    }
}

impl PluginArchiveObjectProvider for CountingArchive {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        if namespace == RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE {
            *self.lookups.borrow_mut() += 1;
        }
        Ok(self
            .objects
            .borrow()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }
}

fn state(canwu: &Canwu) -> ResourceState {
    resource_state(canwu)
        .expect("resource query")
        .expect("resource state")
        .1
}

fn settle(canwu: &mut Canwu, at: SimTime) {
    canwu
        .settle_boundary(BoundaryRequest::at(at))
        .expect("a resource boundary never fails in this fixture");
}

fn next_boundary(canwu: &Canwu) -> u64 {
    canwu
        .boundaries()
        .last()
        .map_or(1, |boundary| boundary.id.get().saturating_add(1))
}

/// A plugin-owned run and the next tracked command request id.
struct Run {
    canwu: Canwu,
    archive: CountingArchive,
    commands: u64,
    /// Every single-step lease operation the run issued, in order.
    single_steps: Vec<ResourceCompletionOperationV1>,
}

impl Run {
    /// Submits one tracked command from `actor` about itself and, when
    /// admitted, settles it. Returns the admission outcome.
    fn command(
        &mut self,
        actor: u64,
        at: SimTime,
        request: ResourceOperationRequestV1,
    ) -> CommandAttemptOutcome {
        self.commands += 1;
        self.canwu
            .enqueue_command(
                at,
                0,
                CommandRequest::new(
                    CommandRequestId::new(self.commands),
                    self.canwu.revision(),
                    CommandEnvelope::new(
                        Issuer::Actor(PersonId::new(actor)),
                        resource_command(&ResourceCommandV1 {
                            subject: holder(actor),
                            request,
                        })
                        .expect("command"),
                    )
                    .at_time(at),
                ),
            )
            .expect("tracked command ingress");
        settle(&mut self.canwu, at);
        let outcome = self
            .canwu
            .command_attempts()
            .last()
            .expect("command attempt")
            .outcome
            .clone();
        if matches!(outcome, CommandAttemptOutcome::Accepted { .. }) {
            settle(&mut self.canwu, at);
        }
        outcome
    }

    /// Settles one admitted command and returns its applied outcome.
    fn applied_command(
        &mut self,
        actor: u64,
        at: SimTime,
        request: ResourceOperationRequestV1,
    ) -> ResourceOperationOutcome {
        let operation_key = request.operation_key();
        let outcome = self.command(actor, at, request);
        assert!(
            matches!(outcome, CommandAttemptOutcome::Accepted { .. }),
            "{outcome:?}"
        );
        let outcome = state(&self.canwu).outcomes[&operation_key].clone();
        assert_eq!(
            outcome.status,
            ResourceOperationStatus::Applied,
            "{:?}",
            outcome.rejection_reason
        );
        outcome
    }

    /// Settles one completion operation through the plugin's canonical ingress.
    fn completion(&mut self, at: SimTime, operation: &ResourceCompletionOperationV1) {
        enqueue_resource_completion_operation(&mut self.canwu, at, operation)
            .expect("completion ingress");
        settle(&mut self.canwu, at);
    }

    /// Activates one lease by the chosen path, each operation in a boundary
    /// of its own, and returns its certificate.
    fn lease(
        &mut self,
        single: bool,
        lease_holder: u64,
        suffix: &str,
        operation_key: &ResourceOperationKey,
        targets: &[CompletionLockedTargetV1],
        at: SimTime,
    ) -> CompletionLeaseActivationCertificateV1 {
        let id = acquisition(suffix);
        if single {
            let boundary = next_boundary(&self.canwu);
            let operation = ResourceCompletionOperationV1::SingleStepLease(single_step(
                lease_holder,
                suffix,
                operation_key,
                targets,
                at,
                boundary,
            ));
            self.completion(at, &operation);
            self.single_steps.push(operation);
        } else {
            let lease = lease_request(lease_holder, suffix, operation_key, targets, at);
            let grant_id = grant(suffix);
            self.completion(at, &ResourceCompletionOperationV1::Acquire(lease.clone()));
            let current = state(&self.canwu);
            let current_boundary = next_boundary(&self.canwu);
            self.completion(
                at,
                &ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
                    grant_id: grant_id.clone(),
                    acquisition: id.clone(),
                    expected_acquisition_revision: current.completion_leases.acquisitions[&id]
                        .revision,
                    owner_plugin: PLUGIN_NAME.to_owned(),
                    target_versions: targets.to_vec(),
                    current_boundary,
                }),
            );
            let current = state(&self.canwu);
            let current_boundary = next_boundary(&self.canwu);
            self.completion(
                at,
                &ResourceCompletionOperationV1::Prepare(PrepareCompletionCapacityV1 {
                    acquisition: id.clone(),
                    expected_acquisition_revision: current.completion_leases.acquisitions[&id]
                        .revision,
                    grant: grant_id.clone(),
                    expected_grant_revision: current.completion_leases.grants[&grant_id].revision,
                    current_boundary,
                    eligibility_envelope_digest: lease.eligibility_envelope.digest.clone(),
                }),
            );
            let current = state(&self.canwu);
            let current_boundary = next_boundary(&self.canwu);
            self.completion(
                at,
                &ResourceCompletionOperationV1::Activate(ActivateCompletionLeaseV1 {
                    acquisition: id.clone(),
                    expected_acquisition_revision: current.completion_leases.acquisitions[&id]
                        .revision,
                    grant: grant_id.clone(),
                    expected_grant_revision: current.completion_leases.grants[&grant_id].revision,
                    at,
                    current_boundary,
                    eligibility_envelope_digest: lease.eligibility_envelope.digest,
                }),
            );
            // The grant started an expiry clock; let its tick settle.
            settle(&mut self.canwu, at);
        }
        state(&self.canwu)
            .completion_leases
            .certificates
            .get(&id)
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "lease {suffix} was not activated: {:?}",
                    state(&self.canwu)
                        .outcomes
                        .values()
                        .max_by_key(|outcome| outcome.sequence)
                )
            })
    }

    /// Archives every hot terminal candidate in one commit and acknowledges it.
    fn archive_all(&mut self) {
        let hot = state(&self.canwu);
        let prepared = hot
            .prepare_resource_archive(hot.terminal_archive_candidates.len(), &self.archive)
            .expect("prepare archive commit");
        let receipt = enqueue_resource_archive(&mut self.canwu, &prepared, &self.archive)
            .expect("enqueue commit");
        let at = self.canwu.time();
        settle(&mut self.canwu, at);
        finalize_resource_archive_retention(&mut self.canwu, &self.archive, &receipt)
            .expect("finalize retention");
        settle(&mut self.canwu, at);
    }

    fn hot_records(&self) -> usize {
        state(&self.canwu).terminal_archive_candidates.len()
    }
}

/// What one same-place transfer left and read.
#[derive(Debug, Eq, PartialEq)]
struct TransferCost {
    /// Terminal records its two leases left before any completion.
    lease_records: usize,
    /// Archive lookups its two leases made.
    lease_lookups: usize,
    /// Every terminal record of the transfer.
    records: usize,
    /// Every archive lookup of the transfer.
    lookups: usize,
}

/// Runs one same-place transfer of ten units: demand, allocation, start under
/// the source custodian's lease, and local acceptance under the destination
/// custodian's lease.
fn same_place_transfer(run: &mut Run, name: &str, at: SimTime, single: bool) -> TransferCost {
    let handover = evidence(name, 1);
    let records_before = run.hot_records();
    let lookups_before = run.archive.lookups();
    let mut lease_records = 0;
    let mut lease_lookups = 0;

    let request = demand(&state(&run.canwu), name, at);
    run.applied_command(
        SOURCE_CUSTODIAN,
        at,
        ResourceOperationRequestV1::SubmitDemand(ResourceSubmitDemandRequestV1 {
            operation_key: key(&format!("fixture:submit:{name}")),
            demand: request,
        }),
    );
    let expected_state_revision = state(&run.canwu).state_revision;
    enqueue_resource_allocation(
        &mut run.canwu,
        at,
        &holder(SOURCE_CUSTODIAN),
        &ResourceAllocationRequestV1 {
            operation_key: key(&format!("fixture:allocate:{name}")),
            expected_state_revision,
            at,
            candidate_limit: 8,
        },
    )
    .expect("allocation ingress");
    settle(&mut run.canwu, at);

    let current = state(&run.canwu);
    let leg = leg_of(&current, name);
    let start_key = key(&format!("fixture:start:{name}"));
    let (records, lookups) = (run.hot_records(), run.archive.lookups());
    let start_certificate = run.lease(
        single,
        SOURCE_CUSTODIAN,
        &format!("start-{name}"),
        &start_key,
        &start_targets(&current, &leg),
        at,
    );
    lease_records += run.hot_records() - records;
    lease_lookups += run.archive.lookups() - lookups;
    let transfer = ResourceTransferId::new(format!("fixture:transfer:{name}")).expect("transfer");
    run.applied_command(
        SOURCE_CUSTODIAN,
        at,
        ResourceOperationRequestV1::BeginTransfer(ResourceTransferStartRequestV1 {
            operation_key: start_key,
            transfer_id: transfer.clone(),
            allocation: (&leg).into(),
            expected_account_revision: current.accounts[&leg.account].revision,
            destination: Some(account("destination")),
            at,
            completion_certificate: start_certificate,
        }),
    );

    let current = state(&run.canwu);
    let expected_transfer_revision = current.transfers[&transfer].revision;
    let expected_destination_revision = current.accounts[&account("destination")].revision;
    let accept_key = key(&format!("fixture:accept:{name}"));
    let (records, lookups) = (run.hot_records(), run.archive.lookups());
    let accept_certificate = run.lease(
        single,
        DESTINATION_CUSTODIAN,
        &format!("accept-{name}"),
        &accept_key,
        &[
            CompletionLockedTargetV1::Transfer {
                id: transfer.clone(),
                revision: expected_transfer_revision,
            },
            CompletionLockedTargetV1::Account {
                id: account("destination"),
                revision: expected_destination_revision,
            },
            CompletionLockedTargetV1::ExternalRecord {
                version: handover.clone(),
            },
        ],
        at,
    );
    lease_records += run.hot_records() - records;
    lease_lookups += run.archive.lookups() - lookups;
    let accepted = run.applied_command(
        DESTINATION_CUSTODIAN,
        at,
        ResourceOperationRequestV1::CompleteTransfer(ResourceTransferDispositionRequestV1 {
            operation_key: accept_key,
            transfer: transfer.clone(),
            expected_transfer_revision,
            at,
            disposition: ResourceTransferDispositionV1::AcceptLocal {
                destination: account("destination"),
                expected_destination_revision,
                handover_evidence: handover.clone(),
            },
            exact_transport_evidence: None,
            completion_certificate: accept_certificate,
        }),
    );
    assert_eq!(accepted.exact_evidence, vec![handover]);
    let settled = state(&run.canwu);
    assert_eq!(
        settled.transfers[&transfer].state,
        ResourceTransferState::Accepted
    );
    // Both leases are complete and hold nothing.
    for suffix in [format!("start-{name}"), format!("accept-{name}")] {
        assert_eq!(
            settled.completion_leases.acquisitions[&acquisition(&suffix)].state,
            CompletionLeaseAcquisitionStateV1::Released
        );
    }
    assert!(settled.completion_leases.target_locks.is_empty());
    assert_eq!(settled.completion_leases.reserved_units, 0);
    TransferCost {
        lease_records,
        lease_lookups,
        records: run.hot_records() - records_before,
        lookups: run.archive.lookups() - lookups_before,
    }
}

#[test]
fn same_place_transfer_leaves_fifteen_records_and_nine_lookups() {
    let handovers = [evidence("four", 1), evidence("single", 1)];
    let resource_plugin =
        ResourcePlugin::new([DomainRecordKind::new(EVIDENCE_NAMESPACE, "evidence")]);
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &EvidencePlugin];
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![
            EntityRef::Person(PersonId::new(SOURCE_CUSTODIAN)),
            EntityRef::Person(PersonId::new(DESTINATION_CUSTODIAN)),
            EntityRef::Person(PersonId::new(OUTSIDER)),
        ],
    )
    .with_domain_records(
        handovers
            .iter()
            .map(evidence_record)
            .chain(world().into_records().expect("resource record"))
            .collect(),
    );
    let archive = CountingArchive::default();
    let mut canwu = Canwu::new_with_plugins(53, scenario, &plugins).expect("runtime");
    canwu.set_plugin_archive_object_provider(Rc::new(archive.clone()));
    let mut run = Run {
        canwu,
        archive,
        commands: 0,
        single_steps: Vec::new(),
    };

    // One archive commit first: from here every operation asks the archive
    // whether its key was used before.
    let at = SimTime::EPOCH;
    let expected_state_revision = state(&run.canwu).state_revision;
    enqueue_resource_allocation(
        &mut run.canwu,
        at,
        &holder(OUTSIDER),
        &ResourceAllocationRequestV1 {
            operation_key: key("fixture:allocate:opening"),
            expected_state_revision,
            at,
            candidate_limit: 1,
        },
    )
    .expect("opening operation");
    settle(&mut run.canwu, at);
    run.archive_all();
    assert!(state(&run.canwu).archive_head.directory_root.is_some());

    // The same transfer with four-step leases and with single steps.
    let four = same_place_transfer(&mut run, "four", minute(1), false);
    let single = same_place_transfer(&mut run, "single", minute(2), true);
    assert_eq!(
        four,
        TransferCost {
            lease_records: 16,
            lease_lookups: 8,
            records: 27,
            lookups: 15,
        }
    );
    assert_eq!(
        single,
        TransferCost {
            lease_records: 4,
            lease_lookups: 2,
            records: 15,
            lookups: 9,
        }
    );
    // A lease also leaves one receipt when its operation completes, on
    // either path: nine records a lease before, three now.
    let settled = state(&run.canwu);
    for (name, expected) in [("four", 5), ("single", 2)] {
        for suffix in [format!("start-{name}"), format!("accept-{name}")] {
            let receipts: Vec<_> = settled
                .completion_leases
                .receipts
                .values()
                .filter(|receipt| receipt.acquisition == acquisition(&suffix))
                .collect();
            assert_eq!(receipts.len(), expected, "{suffix}");
            assert_eq!(
                receipts.last().map(|receipt| receipt.action),
                Some(CompletionLeaseReceiptActionV1::Completed)
            );
        }
    }
    assert_eq!(settled.accounts[&account("destination")].balance, 20);
    assert_eq!(settled.accounts[&account("source")].balance, 80);

    // The four-step path never resumes a lease the single step created, and
    // not a complete one either. The start lease of the settled transfer is
    // released. Its holder's tracked acquire for the same lease is admitted
    // and settles as a recorded rejection, so no receipt is added that would
    // keep the lease in the root after its records are archived.
    let ResourceCompletionOperationV1::SingleStepLease(start) = run.single_steps[0].clone() else {
        unreachable!("only single steps are recorded")
    };
    assert_eq!(
        settled.completion_leases.acquisitions[&start.lease.id].state,
        CompletionLeaseAcquisitionStateV1::Released
    );
    let acquire = completion(ResourceCompletionOperationV1::Acquire(start.lease));
    let admitted = run.command(SOURCE_CUSTODIAN, minute(2), acquire.clone());
    assert!(
        matches!(admitted, CommandAttemptOutcome::Accepted { .. }),
        "{admitted:?}"
    );
    let after = state(&run.canwu);
    let recorded = &after.outcomes[&acquire.operation_key()];
    assert_eq!(recorded.status, ResourceOperationStatus::Rejected);
    assert_eq!(
        recorded.rejection_code.as_deref(),
        Some("idempotency_conflict")
    );
    assert_eq!(after.completion_leases, settled.completion_leases);

    // A tracked command cannot carry the single step: like the grant,
    // prepare, and activate steps it enters through the plugin's own ingress.
    let at = minute(3);
    settle(&mut run.canwu, at);
    let spare_targets = [CompletionLockedTargetV1::Account {
        id: account("source"),
        revision: settled.accounts[&account("source")].revision,
    }];
    let boundary = next_boundary(&run.canwu);
    let spare = spare_lease(SOURCE_CUSTODIAN, "spare", &spare_targets, at, boundary);
    let before = state(&run.canwu);
    let CommandAttemptOutcome::Rejected { error } =
        run.command(SOURCE_CUSTODIAN, at, single_step_request(&spare))
    else {
        panic!("a tracked command carried a single-step lease");
    };
    assert_eq!(error.code, ErrorCode::InvalidAuthority);
    assert_eq!(state(&run.canwu), before);

    // No holder can take the key that single step will derive: a request
    // that is not a completion operation and names a key under the
    // completion prefix is refused at admission, and the adapter ingress
    // refuses to queue one.
    let taken = ResourceOperationRequestV1::SubmitDemand(ResourceSubmitDemandRequestV1 {
        operation_key: ResourceCompletionOperationV1::SingleStepLease(spare.clone())
            .operation_key(),
        demand: demand(&before, "taker", at),
    });
    let CommandAttemptOutcome::Rejected { error } =
        run.command(SOURCE_CUSTODIAN, at, taken.clone())
    else {
        panic!("a holder took a completion operation's key");
    };
    assert_eq!(error.code, ErrorCode::InvalidAuthority);
    assert!(error.message.contains("reserved"), "{}", error.message);
    assert_eq!(state(&run.canwu), before);
    let queued = resource_adapter_ingress(
        at,
        &ResourceAdapterOperationV1 {
            provider_plugin: "fixture-provider".to_owned(),
            provider_source: evidence("four", 1),
            request: taken,
        },
    );
    assert_eq!(
        queued.map(|_| ()).map_err(|error| error.code),
        Err(ErrorCode::InvalidAuthority)
    );

    // Identities too long for an operation key never reach settlement: a
    // tracked acquire is refused at admission and the completion ingress
    // refuses to queue the single step.
    let long = spare_lease(
        SOURCE_CUSTODIAN,
        &"x".repeat(140),
        &spare_targets,
        at,
        boundary,
    );
    let CommandAttemptOutcome::Rejected { error } = run.command(
        SOURCE_CUSTODIAN,
        at,
        completion(ResourceCompletionOperationV1::Acquire(long.lease.clone())),
    ) else {
        panic!("an acquire whose identity forms no operation key was admitted");
    };
    assert_eq!(error.code, ErrorCode::InvalidPayload);
    assert_eq!(
        enqueue_resource_completion_operation(
            &mut run.canwu,
            at,
            &ResourceCompletionOperationV1::SingleStepLease(long),
        )
        .map(|_| ())
        .map_err(|error| error.code),
        Err(ErrorCode::InvalidPayload)
    );
    assert_eq!(state(&run.canwu), before);

    // What the simulation view holds against a single step settles as its
    // recorded rejection. The boundary does not fail and no lease is left.
    let mut late = spare_lease(SOURCE_CUSTODIAN, "late", &spare_targets, at, boundary);
    late.lease.eligibility_time = minute(4);
    let missing_version = [CompletionLockedTargetV1::ExternalRecord {
        version: evidence("four", 2),
    }];
    let unread_kind = [CompletionLockedTargetV1::ExternalRecord {
        version: DomainRecordVersionRef {
            record: DomainRecordRef::new("fixture.unread", "record", "one"),
            version: 1,
            established_by: DomainRecordVersionSource::InitialScenario,
        },
    }];
    for (lease, code) in [
        (late, "invalid_authority"),
        (
            spare_lease(SOURCE_CUSTODIAN, "missing", &missing_version, at, boundary),
            "version_conflict",
        ),
        (
            spare_lease(SOURCE_CUSTODIAN, "unread", &unread_kind, at, boundary),
            "invalid_authority",
        ),
    ] {
        let before = state(&run.canwu);
        let operation = ResourceCompletionOperationV1::SingleStepLease(lease.clone());
        run.completion(at, &operation);
        let after = state(&run.canwu);
        let outcome = &after.outcomes[&operation.operation_key()];
        assert_eq!(outcome.status, ResourceOperationStatus::Rejected);
        assert_eq!(outcome.rejection_code.as_deref(), Some(code));
        assert_eq!(after.completion_leases, before.completion_leases);
    }

    // The same lease is then granted, and a resend changes nothing while its
    // outcome is in the root.
    let operation = ResourceCompletionOperationV1::SingleStepLease(spare.clone());
    run.completion(at, &operation);
    let granted = state(&run.canwu);
    assert_eq!(
        granted.completion_leases.acquisitions[&spare.lease.id].state,
        CompletionLeaseAcquisitionStateV1::Activated
    );
    run.completion(at, &operation);
    let resent = state(&run.canwu);
    assert_eq!(resent.completion_leases, granted.completion_leases);
    assert_eq!(resent.outcomes, granted.outcomes);

    // The key belongs to the request that settled under it. Another single
    // step with the same acquisition id and another body settles nothing:
    // its boundary commits, the first lease stands, and no record of the
    // second is left. The outcome under the key names the first request, so
    // a host application compares digests before reading it as its own.
    let mut other = spare.clone();
    other.current_boundary += 1;
    let other = ResourceCompletionOperationV1::SingleStepLease(other);
    assert_eq!(other.operation_key(), operation.operation_key());
    run.completion(at, &other);
    let contested = state(&run.canwu);
    assert_eq!(contested.completion_leases, granted.completion_leases);
    assert_eq!(contested.outcomes, granted.outcomes);
    let owner = &contested.outcomes[&operation.operation_key()].request_digest;
    assert_eq!(
        owner,
        &completion(operation.clone())
            .request_digest()
            .expect("request digest")
    );
    assert_ne!(
        owner,
        &completion(other).request_digest().expect("request digest")
    );

    // An archive commit and a single step settle in one boundary. The commit
    // settles first, against the root it was prepared from, and the lease is
    // then taken on the root the commit left: everything that was hot is
    // archived, and the root holds the receipt of the commit and the outcome
    // and receipt of the lease.
    let hot = state(&run.canwu);
    let beside = ResourceCompletionOperationV1::SingleStepLease(spare_lease(
        DESTINATION_CUSTODIAN,
        "beside-commit",
        &[CompletionLockedTargetV1::Account {
            id: account("destination"),
            revision: hot.accounts[&account("destination")].revision,
        }],
        at,
        next_boundary(&run.canwu),
    ));
    enqueue_resource_completion_operation(&mut run.canwu, at, &beside).expect("completion ingress");
    let prepared = hot
        .prepare_resource_archive(hot.terminal_archive_candidates.len(), &run.archive)
        .expect("prepare archive commit");
    let receipt = enqueue_resource_archive(&mut run.canwu, &prepared, &run.archive)
        .expect("enqueue commit behind the single step");
    settle(&mut run.canwu, at);
    let settled = state(&run.canwu);
    assert_eq!(
        settled
            .archive_maintenance_receipts
            .values()
            .find(|settled| settled.retention_handle_id == receipt.retention_handle_id)
            .map(|settled| settled.disposition),
        Some(ResourceArchiveMaintenanceDispositionV1::Applied),
        "the commit applied to the root it was prepared from"
    );
    let ResourceCompletionOperationV1::SingleStepLease(leased) = &beside else {
        unreachable!("the operation is a single step")
    };
    assert_eq!(
        settled.outcomes[&beside.operation_key()].status,
        ResourceOperationStatus::Applied
    );
    assert_eq!(
        settled.completion_leases.acquisitions[&leased.lease.id].state,
        CompletionLeaseAcquisitionStateV1::Activated
    );
    assert_eq!(settled.outcomes.len(), 1);
    assert_eq!(settled.terminal_archive_candidates.len(), 3);
    finalize_resource_archive_retention(&mut run.canwu, &run.archive, &receipt)
        .expect("finalize retention");
    settle(&mut run.canwu, at);

    // A resend changes nothing after its outcome is archived either: the
    // start lease of the settled transfer left the root with its records,
    // and its resend is answered from the archive.
    let archived = state(&run.canwu);
    let resend = run.single_steps[0].clone();
    let ResourceCompletionOperationV1::SingleStepLease(start) = &resend else {
        unreachable!("only single steps are recorded")
    };
    assert_eq!(start.lease.id, acquisition("start-single"));
    assert!(
        !archived
            .completion_leases
            .acquisitions
            .contains_key(&start.lease.id)
    );
    assert!(!archived.outcomes.contains_key(&resend.operation_key()));
    run.completion(at, &resend);
    assert_eq!(
        state(&run.canwu),
        archived,
        "an archived single step is not applied again"
    );

    // Two single steps with one acquisition id and different bodies, queued
    // for one boundary: the boundary commits, the first lease stands, and
    // the second settles nothing.
    let at = minute(6);
    settle(&mut run.canwu, at);
    let boundary = next_boundary(&run.canwu);
    let first = spare_lease(
        DESTINATION_CUSTODIAN,
        "twice",
        &[CompletionLockedTargetV1::ExternalRecord {
            version: evidence("four", 1),
        }],
        at,
        boundary,
    );
    let mut second = first.clone();
    second.grant_id = grant("twice-again");
    for lease in [&first, &second] {
        enqueue_resource_completion_operation(
            &mut run.canwu,
            at,
            &ResourceCompletionOperationV1::SingleStepLease(lease.clone()),
        )
        .expect("completion ingress");
    }
    settle(&mut run.canwu, at);
    let current = state(&run.canwu);
    assert_eq!(
        current.completion_leases.acquisitions[&first.lease.id]
            .grants
            .values()
            .collect::<Vec<_>>(),
        [&first.grant_id]
    );
    assert!(
        !current
            .completion_leases
            .grants
            .contains_key(&second.grant_id)
    );
    assert_eq!(
        current.outcomes[&single_step_request(&first).operation_key()].request_digest,
        single_step_request(&first)
            .request_digest()
            .expect("request digest")
    );

    // A holder may still send a tracked acquire, and an abort of a lease it
    // holds. Only that holder: an abort from anyone else is refused at
    // admission, so it leaves nothing under the lease's abort key.
    let stray = lease_request(
        SOURCE_CUSTODIAN,
        "stray",
        &key("fixture:operation:stray"),
        &[],
        at,
    );
    let admitted = run.command(
        SOURCE_CUSTODIAN,
        at,
        completion(ResourceCompletionOperationV1::Acquire(stray.clone())),
    );
    assert!(
        matches!(admitted, CommandAttemptOutcome::Accepted { .. }),
        "{admitted:?}"
    );
    let abort = |sender: u64, expected_revision: u64| {
        ResourceCompletionOperationV1::Abort(AbortCompletionLeaseV1 {
            acquisition: stray.id.clone(),
            expected_revision: ResourceRevision::new(expected_revision).expect("revision"),
            holder: holder(sender),
        })
    };
    let abort_key = abort(SOURCE_CUSTODIAN, 1).operation_key();
    let CommandAttemptOutcome::Rejected { error } = run.command(
        DESTINATION_CUSTODIAN,
        at,
        completion(abort(DESTINATION_CUSTODIAN, 1)),
    ) else {
        panic!("a holder aborted another holder's lease");
    };
    assert_eq!(error.code, ErrorCode::InvalidAuthority);
    assert!(!state(&run.canwu).outcomes.contains_key(&abort_key));

    // The holder's own abort with a stale revision is a recorded rejection
    // that takes the abort key. The host application's abort then settles
    // nothing, and its boundary commits instead of failing for good.
    let admitted = run.command(SOURCE_CUSTODIAN, at, completion(abort(SOURCE_CUSTODIAN, 9)));
    assert!(
        matches!(admitted, CommandAttemptOutcome::Accepted { .. }),
        "{admitted:?}"
    );
    let taken = state(&run.canwu);
    assert_eq!(
        taken.outcomes[&abort_key].rejection_code.as_deref(),
        Some("version_conflict")
    );
    run.completion(at, &abort(SOURCE_CUSTODIAN, 1));
    let current = state(&run.canwu);
    assert_eq!(current, taken);
    assert_eq!(
        current.completion_leases.acquisitions[&stray.id].state,
        CompletionLeaseAcquisitionStateV1::Requested
    );

    // The stray acquisition is then removed the other way: the host
    // application grants it and releases that grant.
    let stray_grant = grant("stray");
    let current_boundary = next_boundary(&run.canwu);
    run.completion(
        at,
        &ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
            grant_id: stray_grant.clone(),
            acquisition: stray.id.clone(),
            expected_acquisition_revision: current.completion_leases.acquisitions[&stray.id]
                .revision,
            owner_plugin: PLUGIN_NAME.to_owned(),
            target_versions: vec![CompletionLockedTargetV1::ExternalRecord {
                version: evidence("single", 1),
            }],
            current_boundary,
        }),
    );
    let current = state(&run.canwu);
    run.completion(
        at,
        &ResourceCompletionOperationV1::Release(ReleaseCompletionCapacityV1 {
            acquisition: stray.id.clone(),
            expected_acquisition_revision: current.completion_leases.acquisitions[&stray.id]
                .revision,
            grant: stray_grant.clone(),
            expected_grant_revision: current.completion_leases.grants[&stray_grant].revision,
            reason: "stray acquisition".to_owned(),
        }),
    );
    // The grant started an expiry clock; let its tick settle.
    settle(&mut run.canwu, at);
    assert_eq!(
        state(&run.canwu).completion_leases.acquisitions[&stray.id].state,
        CompletionLeaseAcquisitionStateV1::Released
    );

    // Restore audits the archive; exact replay settles the same.
    let snapshot = run.canwu.snapshot_json().expect("snapshot");
    from_resource_snapshot_json_with_archive_store(
        &snapshot,
        &plugins,
        Rc::new(run.archive.clone()),
    )
    .expect("restore");
    let replayed = replay_resource_from_journal_with_archive_store(
        &plugins,
        &run.canwu.replay_journal(),
        Rc::new(run.archive.clone()),
    )
    .expect("exact replay");
    assert_eq!(replayed.snapshot_json().expect("replay snapshot"), snapshot);
    assert_eq!(
        replayed.boundary_head_hash(),
        run.canwu.boundary_head_hash()
    );
}
